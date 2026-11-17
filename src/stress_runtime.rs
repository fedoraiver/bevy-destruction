use std::{
    collections::BTreeMap,
    hash::{Hash, Hasher},
};

use avian3d::prelude::*;
use bevy::{math::DVec3, prelude::*};

use crate::{
    asset::DestructibleAsset,
    physics,
    scene::BuildingInstance,
    stress::{self, BondStress, NodeLoad, SolverSettings},
    structure,
};

#[cfg(test)]
#[path = "stress_runtime_tests.rs"]
mod tests;

pub struct StressRuntimePlugin;

impl Plugin for StressRuntimePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<StressSettings>()
            .init_resource::<StructuralLoads>()
            .add_message::<crate::vfx::BreakBurst>()
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

#[derive(Resource, Default)]
pub struct StructuralLoads(pub BTreeMap<Entity, BTreeMap<String, NodeLoad>>);

pub struct BondLoadState {
    pub stress: BondStress,
    pub damage: f64,
    pub valid: bool,
}

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
    pub failures: u64,
    pub broken_total: u64,
    pub last_broken: Vec<String>,
    pub max_utilization: f64,
    pub max_solve_ms: f64,
    pub skipped_dynamic: usize,
    cache: Option<(u64, f64)>,
}

pub(crate) fn update_stress(world: &mut World) {
    if !world.resource::<StressSettings>().enabled {
        return;
    }
    let dt = world.resource::<Time<Fixed>>().delta_secs_f64();
    let now = world.resource::<Time<Fixed>>().elapsed_secs_f64();
    let gravity = world.resource::<Gravity>().0;
    let buildings = world
        .query::<(Entity, &BuildingInstance)>()
        .iter(world)
        .filter(|(_, instance)| instance.bounds.is_some() && instance.error.is_none())
        .map(|(entity, _)| entity)
        .collect::<Vec<_>>();
    for building in buildings {
        let mut state = world
            .entity_mut(building)
            .take::<StressState>()
            .unwrap_or_default();
        state.last_broken.clear();
        let instance = world.get::<BuildingInstance>(building).unwrap();
        let Some(asset) = world
            .resource::<Assets<DestructibleAsset>>()
            .get(&instance.asset)
        else {
            world.entity_mut(building).insert(state);
            continue;
        };
        let settings = world.resource::<StressSettings>();
        let root = physics::global_transform(world, building).compute_transform();
        let local_gravity = root.rotation.conjugate() * gravity;
        let empty_loads = BTreeMap::new();
        let loads = world
            .resource::<StructuralLoads>()
            .0
            .get(&building)
            .unwrap_or(&empty_loads);
        let mut hash = std::hash::DefaultHasher::new();
        instance.asset.id().hash(&mut hash);
        for (id, chunk) in &instance.chunks {
            id.hash(&mut hash);
            chunk.alive.hash(&mut hash);
        }
        instance.bonds.hash(&mut hash);
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
        let key = hash.finish();
        let retry = state.reports.iter().any(|report| !report.valid)
            && state.cache.is_some_and(|(_, retry_at)| now >= retry_at);
        if state.cache.is_none_or(|(old, _)| old != key) || retry {
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
                .filter(|(_, chunk)| chunk.alive)
                .map(|(id, _)| id.clone())
                .collect();
            let components = structure::components(&asset.manifest, &alive, &instance.bonds);
            let mut sections = BTreeMap::<String, Vec<[f64; 3]>>::new();
            for patch in instance.patches.iter() {
                sections.entry(patch.bond_id.clone()).or_default().extend(
                    patch
                        .triangles
                        .iter()
                        .flat_map(|triangle| triangle.iter())
                        .map(|point| point.to_array().map(f64::from)),
                );
            }
            for component in components {
                if !component.anchored {
                    state.skipped_dynamic += 1;
                    continue;
                }
                match stress::solve(
                    &asset.manifest,
                    &component.chunks,
                    &instance.bonds,
                    local_gravity,
                    loads,
                    &settings.solver,
                    Some(&sections),
                ) {
                    Ok(solution) => {
                        let mut force = [0.0_f64; 3];
                        let mut moment = [0.0_f64; 3];
                        for record in asset
                            .manifest
                            .chunks
                            .iter()
                            .filter(|record| component.chunks.contains(&record.id))
                        {
                            let center = record
                                .transform()
                                .transform_point(record.center_of_mass.into())
                                .as_dvec3();
                            let applied = loads.get(&record.id).copied().unwrap_or_default();
                            let reaction = solution
                                .reactions
                                .get(&record.id)
                                .copied()
                                .unwrap_or_default();
                            let net_force = local_gravity.as_dvec3() * f64::from(record.mass)
                                + DVec3::from_array(applied.force)
                                + DVec3::from_array(reaction.force);
                            let net_moment = center.cross(net_force)
                                + DVec3::from_array(applied.moment)
                                + DVec3::from_array(reaction.moment);
                            for axis in 0..3 {
                                force[axis] += net_force[axis];
                                moment[axis] += net_moment[axis];
                            }
                        }
                        let max_displacement = solution
                            .displacements
                            .values()
                            .map(|d| DVec3::new(d[0], d[1], d[2]).length())
                            .fold(0.0_f64, f64::max);
                        state.reports.push(StressComponentReport {
                            chunks: component.chunks.len(),
                            valid: true,
                            iterations: solution.iterations,
                            relative_residual: solution.relative_residual,
                            error: None,
                            force_balance_error: DVec3::from_array(force).length(),
                            moment_balance_error: DVec3::from_array(moment).length(),
                            max_displacement,
                        });
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
            state.cache = Some((key, now + 1.0));
            state.max_solve_ms = state
                .max_solve_ms
                .max(started.elapsed().as_secs_f64() * 1000.0);
        }
        let mut candidates = Vec::new();
        for (id, bond) in &mut state.bonds {
            if !bond.valid || !instance.bonds.get(id).copied().unwrap_or(false) {
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
            let mut instance = world.get_mut::<BuildingInstance>(building).unwrap();
            for (id, _) in candidates {
                instance.bonds.insert(id.clone(), false);
                state.broken_total += 1;
                state.last_broken.push(id);
            }
            instance.topology_dirty = true;
        }
        if !state.last_broken.is_empty() {
            crate::interaction::bond_feedback(world, building, &state.last_broken);
        }
        world.entity_mut(building).insert(state);
    }
}
