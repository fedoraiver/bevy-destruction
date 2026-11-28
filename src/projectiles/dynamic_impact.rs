use std::collections::{BTreeMap, BTreeSet};

use avian3d::prelude::*;
use bevy::{
    math::{DMat3, DVec3},
    prelude::*,
};

use crate::{
    asset::{DestructibleAsset, Manifest},
    force_samples::{DynamicForceSample, bond_signature},
    scene::ChunkInstance,
    stress::{self, NodeLoad, SolverSettings},
};

use super::{
    ClusterImpact, DYNAMIC_IMPACT_BREAK_LIMIT, IMPACT_DURATION, ImpactState, ImpulseLoad,
    contact::ImpactStructure,
    local_impact::{local_impact_bonds, queue_local_damage},
};

pub(super) struct InertialLoadCase {
    pub(super) loads: BTreeMap<String, NodeLoad>,
    pub(super) force_scale: f64,
    pub(super) moment_scale: f64,
    pub(super) force_balance_ratio: f64,
    pub(super) moment_balance_ratio: f64,
}

struct ImpactChunk<'a> {
    id: &'a str,
    mass: f64,
    center: DVec3,
    node: DVec3,
    inertia: DMat3,
}

pub(super) fn inertial_loads<'a>(
    manifest: &Manifest,
    get_chunk: impl Fn(&str) -> Option<&'a ChunkInstance>,
    chunks: &[String],
    impulses: &BTreeMap<String, ImpulseLoad>,
) -> Option<InertialLoadCase> {
    let selected = chunks.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let members = manifest
        .chunks
        .iter()
        .filter(|record| selected.contains(record.id.as_str()))
        .map(|record| {
            let chunk = get_chunk(&record.id)?;
            let properties = &chunk.mass_properties;
            let inertia = properties.global_angular_inertia_tensor(chunk.transform.rotation);
            Some(ImpactChunk {
                id: &record.id,
                mass: f64::from(properties.mass),
                center: chunk.center_in_building().as_dvec3(),
                node: stress::node_center(record),
                inertia: Mat3::from(inertia).as_dmat3(),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    if members.len() != selected.len() {
        return None;
    }
    let mass = members.iter().map(|member| member.mass).sum::<f64>();
    let center = members
        .iter()
        .map(|member| member.center * member.mass)
        .sum::<DVec3>()
        / mass;
    let mut inertia = DMat3::ZERO;
    let mut linear = DVec3::ZERO;
    let mut angular = DVec3::ZERO;
    let mut force_scale = 0.0;
    let mut moment_scale = 0.0;
    for member in &members {
        let r = member.center - center;
        let outer = DMat3::from_cols(r * r.x, r * r.y, r * r.z);
        inertia += member.inertia + (DMat3::IDENTITY * r.length_squared() - outer) * member.mass;
        let impulse = impulses.get(member.id).copied().unwrap_or_default();
        linear += impulse.linear;
        angular += impulse.angular + r.cross(impulse.linear);
        force_scale += impulse.linear.length();
        moment_scale += impulse.angular.length() + r.cross(impulse.linear).length();
    }
    if !inertia.is_finite() || inertia.determinant() <= 0.0 {
        return None;
    }
    let delta_linear = linear / mass;
    let delta_angular = inertia.inverse() * angular;
    let duration = f64::from(IMPACT_DURATION);
    force_scale = (force_scale / duration).max(1.0);
    let length = (inertia.x_axis.x + inertia.y_axis.y + inertia.z_axis.z).sqrt() / mass.sqrt();
    moment_scale = (moment_scale / duration).max(force_scale * length).max(1.0);
    let mut loads = BTreeMap::new();
    let mut force_residual = DVec3::ZERO;
    let mut moment_residual = DVec3::ZERO;
    for member in members {
        let r = member.center - center;
        let impulse = impulses.get(member.id).copied().unwrap_or_default();
        let force =
            (impulse.linear - (delta_linear + delta_angular.cross(r)) * member.mass) / duration;
        let moment = (impulse.angular - member.inertia * delta_angular) / duration;
        let load = NodeLoad {
            force: force.to_array(),
            moment: moment.to_array(),
        }
        .at_reference(member.center - member.node);
        force_residual += force;
        moment_residual += (member.node - center).cross(force) + DVec3::from_array(load.moment);
        loads.insert(member.id.to_owned(), load);
    }
    Some(InertialLoadCase {
        loads,
        force_scale,
        moment_scale,
        force_balance_ratio: force_residual.length() / force_scale,
        moment_balance_ratio: moment_residual.length() / moment_scale,
    })
}

pub(super) struct DynamicImpactReport {
    pub(super) valid: bool,
    pub(super) elapsed_ms: f64,
    pub(super) force_balance_ratio: f64,
    pub(super) moment_balance_ratio: f64,
    pub(super) gauge_reaction_ratio: f64,
    pub(super) max_utilization: f64,
    pub(super) fracture: Option<(Entity, Vec<String>)>,
    pub(super) continuation: Option<ClusterImpact>,
    pub(super) force_sample: Option<DynamicForceSample>,
}

/// 从同一载荷解中选择可以同步起裂的连接，共用碎块的候选须等待下一次重算。
fn simultaneous_fractures(manifest: &Manifest, solution: &stress::StressSolution) -> Vec<String> {
    let mut candidates = manifest
        .bonds
        .iter()
        .filter_map(|bond| {
            let stress = solution.bonds.get(&bond.id)?;
            (stress.utilization >= 1.0).then_some((bond, stress.strain_energy))
        })
        .collect::<Vec<_>>();
    candidates.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then(a.0.id.cmp(&b.0.id)));

    let mut selected_chunks = BTreeSet::new();
    let mut fractures = Vec::new();
    for (bond, _) in candidates {
        if selected_chunks.contains(&bond.a) || selected_chunks.contains(&bond.b) {
            continue;
        }
        selected_chunks.insert(&bond.a);
        selected_chunks.insert(&bond.b);
        fractures.push(bond.id.clone());
        if fractures.len() == DYNAMIC_IMPACT_BREAK_LIMIT {
            break;
        }
    }
    fractures
}

pub(super) fn process_dynamic_impact(
    structure: &ImpactStructure,
    assets: &Assets<DestructibleAsset>,
    state: &mut ImpactState,
    capture: bool,
    captured_at: f64,
) -> Option<DynamicImpactReport> {
    while let Some(mut shock) = state.shocks.pop_front() {
        let Ok(instance) = structure.buildings.get(shock.building) else {
            continue;
        };
        let Some(asset) = assets.get(&instance.asset) else {
            continue;
        };
        let get_chunk = |id: &str| {
            instance
                .chunks
                .get(id)
                .and_then(|&entity| structure.chunks.get(entity).ok())
        };
        let live_body = |id: &str| {
            let &entity = instance.chunks.get(id)?;
            structure
                .chunks
                .get(entity)
                .ok()
                .filter(|chunk| chunk.alive)?;
            structure
                .collider_owners
                .get(entity)
                .ok()
                .map(|owner| owner.body)
        };
        if shock.prepared.is_none() {
            let source_bodies = shock
                .loads
                .keys()
                .map(|id| live_body(id))
                .collect::<Option<Vec<_>>>();
            let Some(source_bodies) = source_bodies.filter(|bodies| !bodies.is_empty()) else {
                state.invalidated += 1;
                continue;
            };
            let body = source_bodies[0];
            if source_bodies.iter().any(|&source| source != body) {
                state.released += 1;
                continue;
            }
            let Some((cluster, _, _)) =
                structure
                    .clusters
                    .get(body)
                    .ok()
                    .filter(|(cluster, kind, parent)| {
                        !cluster.anchored
                            && parent.parent() == shock.building
                            && **kind == RigidBody::Dynamic
                    })
            else {
                state.released += 1;
                continue;
            };
            if shock.loads.keys().any(|id| !cluster.chunks.contains(id)) {
                state.invalidated += 1;
                continue;
            }
            if cluster.chunks != shock.chunks {
                let original = shock.chunks.iter().collect::<BTreeSet<_>>();
                if cluster.chunks.iter().any(|id| !original.contains(id)) {
                    state.invalidated += 1;
                    continue;
                }
                // An independent hit has not been solved yet. Bind its unchanged
                // contact loads to the one current child that contains every source.
                shock.chunks = cluster.chunks.clone();
                state.rebound_fresh += 1;
            }
        }
        let mut groups = BTreeMap::<Entity, Vec<String>>::new();
        for id in &shock.chunks {
            if let Some(body) = live_body(id) {
                groups.entry(body).or_default().push(id.clone());
            }
        }
        if groups.values().map(Vec::len).sum::<usize>() != shock.chunks.len() {
            state.invalidated += 1;
            continue;
        }
        if groups.len() != 1 {
            // A solved parent response ends at separation. Only an independent,
            // never-solved hit may bind once to its current source-containing child.
            state.released += 1;
            continue;
        }
        let body = *groups.keys().next().unwrap();
        if !structure
            .clusters
            .get(body)
            .is_ok_and(|(_, _, parent)| parent.parent() == shock.building)
        {
            state.released += 1;
            continue;
        }
        if shock.chunks.len() < 2 {
            continue;
        }
        let started = bevy::platform::time::Instant::now();
        // Continuations retain these inputs until the membership checks above release the event.
        if shock.prepared.is_none() {
            let Some(case) =
                inertial_loads(&asset.manifest, get_chunk, &shock.chunks, &shock.loads)
            else {
                continue;
            };
            shock.prepared = Some(case);
        }
        let case = shock.prepared.as_ref().unwrap();
        let mut report = DynamicImpactReport {
            valid: false,
            elapsed_ms: 0.0,
            force_balance_ratio: case.force_balance_ratio,
            moment_balance_ratio: case.moment_balance_ratio,
            gauge_reaction_ratio: f64::INFINITY,
            max_utilization: 0.0,
            fracture: None,
            continuation: None,
            force_sample: None,
        };
        let bond_states =
            instance.bond_states(|entity| structure.bonds.get(entity).unwrap().active);
        let result = if case.force_balance_ratio <= 1e-6 && case.moment_balance_ratio <= 1e-6 {
            stress::solve_with_gauge(
                &asset.manifest,
                &shock.chunks,
                &bond_states,
                &case.loads,
                &SolverSettings::default(),
                &shock.chunks[0],
            )
        } else {
            Err(stress::SolveError::InvalidModel(
                "Unbalanced inertial impact loads".into(),
            ))
        };
        match result {
            Ok(solution) => {
                let reaction = &solution.reactions[&shock.chunks[0]];
                report.gauge_reaction_ratio = (DVec3::from_array(reaction.force).length()
                    / case.force_scale)
                    .max(DVec3::from_array(reaction.moment).length() / case.moment_scale);
                if report.gauge_reaction_ratio <= 1e-5 {
                    report.valid = true;
                    report.max_utilization = solution
                        .bonds
                        .values()
                        .map(|bond| bond.utilization)
                        .fold(0.0, f64::max);
                    let fractures = simultaneous_fractures(&asset.manifest, &solution);
                    if !fractures.is_empty() {
                        report.fracture = Some((shock.building, fractures));
                    }
                    if capture {
                        report.force_sample = Some(DynamicForceSample {
                            building: shock.building,
                            body,
                            chunks: shock.chunks.clone(),
                            bonds: solution.bonds,
                            bond_signature: bond_signature(
                                bond_states
                                    .iter()
                                    .map(|(id, &active)| (id.as_str(), active)),
                            ),
                            captured_at,
                        });
                    }
                }
            }
            Err(error) => warn!(
                "Dynamic impact stress failed for {:?}: {error}",
                shock.building
            ),
        }
        if !report.valid {
            for (id, load) in &shock.loads {
                queue_local_damage(
                    local_impact_bonds(
                        structure,
                        &asset.manifest,
                        instance,
                        id,
                        instance.chunks[id],
                    ),
                    shock.building,
                    load.linear.as_vec3(),
                    &mut state.pending,
                );
            }
        }
        if report.fracture.is_some() {
            // 下一批断裂须等待拓扑重建并重新求解。
            report.continuation = Some(shock);
        }
        report.elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        return Some(report);
    }
    None
}
