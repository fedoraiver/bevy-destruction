use std::{
    collections::BTreeMap,
    hash::{Hash, Hasher},
};

use avian3d::prelude::*;
use bevy::{ecs::change_detection::Tick, math::DVec3, prelude::*};

use crate::{
    asset::DestructibleAsset,
    interaction::build_bond_feedback,
    physics,
    scene::{BondInstance, BuildingInstance, ChunkInstance, PatchInstance},
    stress::{self, BondStress, NodeLoad, SolverSettings, node_center},
    structure,
    vfx::BreakBurst,
};

#[cfg(test)]
mod tests;

pub struct StressRuntimePlugin;

impl Plugin for StressRuntimePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<StressSettings>()
            .add_message::<BreakBurst>()
            .add_systems(
                FixedPostUpdate,
                update_stress.before(physics::rebuild_clusters),
            );
    }
}

#[derive(Resource)]
pub struct StressSettings {
    pub enabled: bool,
    pub damage_seconds: f64,
    pub immediate_utilization: f64,
    pub max_breaks_per_step: usize,
    pub solver: SolverSettings,
}

impl Default for StressSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            damage_seconds: 0.5,
            immediate_utilization: 4.0,
            max_breaks_per_step: 4,
            solver: SolverSettings::default(),
        }
    }
}

/// Applied loads on this building, in building coordinates and about the authored structural nodes.
#[derive(Component, Default)]
pub struct StructuralLoads {
    pub by_chunk: BTreeMap<String, NodeLoad>,
}

pub struct BondLoadState {
    pub stress: BondStress,
    pub damage: f64,
    pub valid: bool,
}

#[derive(Clone)]
#[cfg_attr(not(any(test, feature = "acceptance-test")), expect(dead_code))]
pub struct StressComponentReport {
    pub chunks: usize,
    pub valid: bool,
    pub iterations: usize,
    pub relative_residual: f64,
    pub error: Option<String>,
    pub force_balance_error: f64,
    pub moment_balance_error: f64,
    pub max_displacement: f64,
}

#[derive(Component, Default)]
pub struct StressState {
    pub bonds: BTreeMap<String, BondLoadState>,
    pub reports: Vec<StressComponentReport>,
    pub solves: u64,
    pub numerical_solves: u64,
    pub failures: u64,
    pub broken_total: u64,
    pub last_broken: Vec<String>,
    pub max_utilization: f64,
    pub max_solve_ms: f64,
    pub skipped_dynamic: usize,
    cache: Option<StressCache>,
    requires_damage_update: bool,
    static_components: BTreeMap<Vec<String>, StaticComponentCache>,
}

struct StaticComponentCache {
    key: u64,
    report: StressComponentReport,
    bonds: Vec<String>,
    max_utilization: f64,
}

#[derive(Clone, Copy)]
struct StressCache {
    key: u64,
    source_tick: u32,
    loads_tick: Option<Tick>,
    settings_tick: u32,
    gravity: Vec3,
    retry_at: f64,
}

impl StressState {
    pub fn inputs_current(
        &self,
        source_tick: u32,
        loads_tick: Option<Tick>,
        settings_tick: u32,
        gravity: Vec3,
    ) -> bool {
        self.cache.is_some_and(|cache| {
            cache.source_tick == source_tick
                && cache.loads_tick == loads_tick
                && cache.settings_tick == settings_tick
                && cache.gravity == gravity
        })
    }
}

fn hash_weight(chunk: &ChunkInstance, hash: &mut impl Hasher) {
    chunk.mass_properties.mass.to_bits().hash(hash);
    chunk
        .center_in_building()
        .to_array()
        .map(f32::to_bits)
        .hash(hash);
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn update_stress(
    mut commands: Commands,
    mut buildings: Query<(
        Entity,
        &mut BuildingInstance,
        Option<&mut StressState>,
        Option<Ref<StructuralLoads>>,
    )>,
    chunks: Query<(&ChunkInstance, Option<&ColliderOf>)>,
    mut bonds: Query<&mut BondInstance>,
    patches: Query<&PatchInstance>,
    settings: Res<StressSettings>,
    time: Res<Time<Fixed>>,
    gravity: Res<Gravity>,
    assets: Res<Assets<DestructibleAsset>>,
    transforms: TransformHelper,
    poses: Query<(&Position, &Rotation, &ChildOf), With<physics::Cluster>>,
    mut bursts: MessageWriter<BreakBurst>,
) {
    if !settings.enabled {
        return;
    }
    let dt = time.delta_secs_f64();
    let now = time.elapsed_secs_f64();
    let settings_tick = settings.last_changed().get();
    let empty_loads = BTreeMap::new();
    for (building, mut instance, mut state, loads) in &mut buildings {
        if instance.bounds.is_none() || instance.error.is_some() {
            continue;
        }
        let mut initial_state = None;
        let state = match state.as_deref_mut() {
            Some(state) => state,
            None => initial_state.insert(StressState::default()),
        };
        state.last_broken.clear();
        let source_tick = instance.last_changed().get();
        let Some(asset) = assets.get(&instance.asset) else {
            if let Some(state) = initial_state {
                commands.entity(building).insert(state);
            }
            continue;
        };
        let root = transforms
            .compute_global_transform(building)
            .expect("A loaded building has a valid transform hierarchy")
            .compute_transform();
        let local_gravity = root.rotation.conjugate() * gravity.0;
        // An absent component has a distinct version so removal invalidates only this building.
        let loads_tick = loads.as_ref().map(|loads| loads.last_changed());
        let loads = loads
            .as_deref()
            .map_or(&empty_loads, |loads| &loads.by_chunk);
        let chunk_at = |entity| chunks.get(entity).expect("Indexed building chunk exists").0;
        let bond_active = |entity| {
            bonds
                .get(entity)
                .expect("Indexed building bond exists")
                .active
        };
        let mut bond_states = None;
        let unchanged = state.inputs_current(source_tick, loads_tick, settings_tick, local_gravity);
        let key = if unchanged {
            state.cache.unwrap().key
        } else {
            let mut hash = std::hash::DefaultHasher::new();
            instance.asset.id().hash(&mut hash);
            for (id, &entity) in &instance.chunks {
                let chunk = chunk_at(entity);
                id.hash(&mut hash);
                chunk.alive.hash(&mut hash);
                if chunk.alive {
                    hash_weight(chunk, &mut hash);
                }
            }
            bond_states
                .insert(instance.bond_states(bond_active))
                .hash(&mut hash);
            for value in local_gravity.to_array() {
                value.to_bits().hash(&mut hash);
            }
            settings.solver.max_iterations.hash(&mut hash);
            settings.solver.relative_tolerance.to_bits().hash(&mut hash);
            for (id, load) in loads {
                id.hash(&mut hash);
                for value in load.force.into_iter().chain(load.moment) {
                    value.to_bits().hash(&mut hash);
                }
            }
            hash.finish()
        };
        let retry = state.reports.iter().any(|report| !report.valid)
            && state.cache.is_some_and(|cache| now >= cache.retry_at);
        let recompute = state.cache.is_none_or(|cache| cache.key != key) || retry;
        if recompute {
            let bond_states = bond_states.get_or_insert_with(|| instance.bond_states(bond_active));
            let started = bevy::platform::time::Instant::now();
            state.solves += 1;
            state.reports.clear();
            state.skipped_dynamic = 0;
            state.max_utilization = 0.0;
            for bond in state.bonds.values_mut() {
                bond.valid = false;
            }
            let alive = instance
                .chunks
                .iter()
                .filter(|(_, entity)| chunk_at(**entity).alive)
                .map(|(id, _)| id.clone())
                .collect();
            let components = structure::components(&asset.manifest, &alive, bond_states);
            let records = asset
                .manifest
                .chunks
                .iter()
                .map(|record| (record.id.as_str(), record))
                .collect::<BTreeMap<_, _>>();
            let mut previous_components = std::mem::take(&mut state.static_components);
            for component in components {
                if !component.anchored {
                    state.skipped_dynamic += 1;
                    continue;
                }
                let mut component_hash = std::hash::DefaultHasher::new();
                instance.asset.id().hash(&mut component_hash);
                component.chunks.hash(&mut component_hash);
                for value in local_gravity.to_array() {
                    value.to_bits().hash(&mut component_hash);
                }
                settings.solver.max_iterations.hash(&mut component_hash);
                settings
                    .solver
                    .relative_tolerance
                    .to_bits()
                    .hash(&mut component_hash);
                for bond in &asset.manifest.bonds {
                    if component.chunks.binary_search(&bond.a).is_ok()
                        && component.chunks.binary_search(&bond.b).is_ok()
                    {
                        bond.id.hash(&mut component_hash);
                        bond_states[&bond.id].hash(&mut component_hash);
                    }
                }
                for id in &component.chunks {
                    hash_weight(chunk_at(instance.chunks[id]), &mut component_hash);
                    if let Some(load) = loads.get(id) {
                        id.hash(&mut component_hash);
                        for value in load.force.into_iter().chain(load.moment) {
                            value.to_bits().hash(&mut component_hash);
                        }
                    }
                }
                let component_key = component_hash.finish();
                if let Some(cached) = previous_components
                    .remove(&component.chunks)
                    .filter(|cached| cached.key == component_key)
                {
                    for id in &cached.bonds {
                        state.bonds.get_mut(id).unwrap().valid = true;
                    }
                    state.reports.push(cached.report.clone());
                    state.max_utilization = state.max_utilization.max(cached.max_utilization);
                    state.static_components.insert(component.chunks, cached);
                    continue;
                }
                state.numerical_solves += 1;
                let applied_loads = component
                    .chunks
                    .iter()
                    .filter_map(|id| {
                        let record = records.get(id.as_str())?;
                        let chunk = chunk_at(instance.chunks[id]);
                        let center = chunk.center_in_building().as_dvec3();
                        let weight =
                            local_gravity.as_dvec3() * f64::from(chunk.mass_properties.mass);
                        let weight_load = NodeLoad {
                            force: weight.to_array(),
                            ..default()
                        }
                        .at_reference(center - node_center(record));
                        let external = loads.get(id).copied().unwrap_or_default();
                        Some((
                            id.clone(),
                            NodeLoad {
                                force: (weight + DVec3::from_array(external.force)).to_array(),
                                moment: (DVec3::from_array(weight_load.moment)
                                    + DVec3::from_array(external.moment))
                                .to_array(),
                            },
                        ))
                    })
                    .collect::<BTreeMap<_, _>>();
                match stress::solve(
                    &asset.manifest,
                    &component.chunks,
                    bond_states,
                    &applied_loads,
                    &settings.solver,
                ) {
                    Ok(solution) => {
                        let mut force = DVec3::ZERO;
                        let mut moment = DVec3::ZERO;
                        for (id, applied) in &applied_loads {
                            let center = node_center(records[id.as_str()]);
                            let reaction = solution.reactions.get(id).copied().unwrap_or_default();
                            let net_force = DVec3::from_array(applied.force)
                                + DVec3::from_array(reaction.force);
                            force += net_force;
                            moment += center.cross(net_force)
                                + DVec3::from_array(applied.moment)
                                + DVec3::from_array(reaction.moment);
                        }
                        let max_displacement = solution
                            .displacements
                            .values()
                            .map(|d| DVec3::new(d[0], d[1], d[2]).length())
                            .fold(0.0_f64, f64::max);
                        let report = StressComponentReport {
                            chunks: component.chunks.len(),
                            valid: true,
                            iterations: solution.iterations,
                            relative_residual: solution.relative_residual,
                            error: None,
                            force_balance_error: force.length(),
                            moment_balance_error: moment.length(),
                            max_displacement,
                        };
                        let cached = StaticComponentCache {
                            key: component_key,
                            report: report.clone(),
                            bonds: solution.bonds.keys().cloned().collect(),
                            max_utilization: solution
                                .bonds
                                .values()
                                .map(|bond| bond.utilization)
                                .fold(0.0_f64, f64::max),
                        };
                        state.static_components.insert(component.chunks, cached);
                        state.reports.push(report);
                        for (id, stress) in solution.bonds {
                            state.max_utilization = state.max_utilization.max(stress.utilization);
                            let damage = state.bonds.get(&id).map_or(0.0, |bond| bond.damage);
                            state.bonds.insert(
                                id,
                                BondLoadState {
                                    stress,
                                    damage,
                                    valid: true,
                                },
                            );
                        }
                    }
                    Err(error) => {
                        state.failures += 1;
                        warn!(
                            "Structural solve failed for {building:?} ({} chunks): {error}",
                            component.chunks.len()
                        );
                        state.reports.push(StressComponentReport {
                            chunks: component.chunks.len(),
                            valid: false,
                            iterations: 0,
                            relative_residual: f64::INFINITY,
                            error: Some(error.to_string()),
                            force_balance_error: f64::INFINITY,
                            moment_balance_error: f64::INFINITY,
                            max_displacement: f64::INFINITY,
                        });
                    }
                }
            }
            state.max_solve_ms = state
                .max_solve_ms
                .max(started.elapsed().as_secs_f64() * 1000.0);
        }
        if recompute
            || state
                .cache
                .is_none_or(|cache| cache.settings_tick != settings_tick)
        {
            state.requires_damage_update = state.bonds.iter().any(|(id, bond)| {
                bond.valid
                    && instance.bonds.get(id).copied().is_some_and(bond_active)
                    && (bond.stress.utilization > 1.0
                        || bond.stress.utilization >= settings.immediate_utilization
                        || bond.damage >= 1.0)
            });
        }
        state.cache = Some(StressCache {
            key,
            source_tick,
            loads_tick,
            settings_tick,
            gravity: local_gravity,
            retry_at: if recompute {
                now + 1.0
            } else {
                state.cache.unwrap().retry_at
            },
        });
        if state.requires_damage_update {
            let mut candidates = Vec::new();
            for (id, bond) in &mut state.bonds {
                if !bond.valid || !instance.bonds.get(id).copied().is_some_and(bond_active) {
                    continue;
                }
                let utilization = bond.stress.utilization;
                bond.damage += dt * (utilization - 1.0).max(0.0) / settings.damage_seconds;
                if utilization >= settings.immediate_utilization || bond.damage >= 1.0 {
                    candidates.push((id.clone(), utilization));
                }
            }
            candidates.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
            candidates.truncate(settings.max_breaks_per_step);
            if !candidates.is_empty() {
                for (id, _) in candidates {
                    bonds.get_mut(instance.bonds[&id]).unwrap().active = false;
                    state.broken_total += 1;
                    state.last_broken.push(id);
                }
                instance.topology_dirty = true;

                // 在刚体簇重建前，从断裂瞬间的位姿生成反馈。
                if let Some(burst) = build_bond_feedback(
                    &instance,
                    &asset.manifest,
                    &state.last_broken,
                    |entity| {
                        let (chunk, collider) = chunks.get(entity).unwrap();
                        (chunk, collider.map(|collider| collider.body))
                    },
                    instance
                        .patches
                        .iter()
                        .map(|&entity| patches.get(entity).unwrap().patch()),
                    |body| {
                        body.and_then(|body| poses.get(body).ok())
                            .filter(|(_, _, parent)| parent.parent() == building)
                            .map_or(root, |(position, rotation, _)| {
                                Transform::from_translation(position.0).with_rotation(rotation.0)
                            })
                    },
                ) {
                    bursts.write(burst);
                }
            }
        }
        if let Some(state) = initial_state {
            commands.entity(building).insert(state);
        }
    }
}
