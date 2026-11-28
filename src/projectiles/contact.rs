use std::collections::{BTreeMap, BTreeSet};

use avian3d::{collision::contact_types::ContactGraph, prelude::*};
use bevy::{ecs::system::SystemParam, prelude::*, transform::helper::TransformHelper};

use crate::{
    asset::DestructibleAsset,
    force_samples::{ForceCapture, ForceSamples, ImpulseSample, MAX_IMPULSE_SAMPLES},
    interaction::build_bond_feedback,
    physics::{Cluster, NeedsClusterRebuild},
    scene::{
        BondInstance, BondPatches, BuildingInstance, ChunkIdentity, ChunkInstance, PatchInstance,
        StructuralPatch,
    },
    vfx::BreakBurst,
};

#[cfg(feature = "brp")]
use super::RigidProjectile;
use super::{
    ClusterImpact, IMPACT_BREAK_LIMIT, IMPACT_MIN_SPEED, ImpactState, ImpactStats,
    dynamic_impact::process_dynamic_impact,
    local_impact::{local_impact_bonds, queue_local_damage},
};

#[derive(SystemParam)]
pub(super) struct ImpactStructure<'w, 's> {
    pub(super) buildings: Query<'w, 's, &'static BuildingInstance>,
    pub(super) chunks: Query<'w, 's, &'static ChunkInstance>,
    pub(super) bonds: Query<'w, 's, &'static mut BondInstance>,
    pub(super) bond_patches: Query<'w, 's, &'static BondPatches>,
    pub(super) structural_patches: Query<'w, 's, &'static StructuralPatch>,
    pub(super) collider_owners: Query<'w, 's, &'static ColliderOf>,
    pub(super) clusters: Query<'w, 's, (&'static Cluster, &'static RigidBody, &'static ChildOf)>,
    poses: Query<'w, 's, (&'static Position, &'static Rotation)>,
}

impl ImpactStructure<'_, '_> {
    fn cluster_for(
        &self,
        collider: Entity,
        building: Entity,
    ) -> Option<(Entity, &Cluster, &RigidBody)> {
        let body = self.collider_owners.get(collider).ok()?.body;
        let (cluster, kind, parent) = self.clusters.get(body).ok()?;
        (parent.parent() == building).then_some((body, cluster, kind))
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn impact_damage(
    mut commands: Commands,
    graph: Res<ContactGraph>,
    assets: Res<Assets<DestructibleAsset>>,
    time: Res<Time<Fixed>>,
    mut state: ResMut<ImpactState>,
    mut stats: ResMut<ImpactStats>,
    capture: Option<Res<ForceCapture>>,
    mut samples: Option<ResMut<ForceSamples>>,
    mut structure: ImpactStructure,
    identities: Query<&ChunkIdentity>,
    patches: Query<&PatchInstance>,
    transforms: TransformHelper,
    mut bursts: MessageWriter<BreakBurst>,
    #[cfg(feature = "brp")] projectiles: Query<&Collider, With<RigidProjectile>>,
) {
    let capture = capture.is_some_and(|capture| capture.enabled);
    let captured_at = time.elapsed_secs_f64();
    let mut impulse_samples = Vec::new();
    let mut touching = BTreeSet::new();
    let mut hits = BTreeMap::<(Entity, String), Vec3>::new();
    let mut shocks = BTreeMap::<(Entity, (Entity, Entity)), ClusterImpact>::new();
    let mut contact_count = 0;
    #[cfg(feature = "brp")]
    let (mut sphere_contacts, mut cube_contacts) = (0, 0);
    let mut max_impulse = 0.0_f32;
    for pair in graph.iter_active_touching() {
        let key = if pair.collider1 < pair.collider2 {
            (pair.collider1, pair.collider2)
        } else {
            (pair.collider2, pair.collider1)
        };
        if state.touching.contains(&key) {
            touching.insert(key);
            continue;
        }
        let speed = pair
            .manifolds
            .iter()
            .flat_map(|manifold| &manifold.points)
            .map(|point| -point.normal_speed)
            .fold(0.0_f32, f32::max);
        if speed < IMPACT_MIN_SPEED {
            continue;
        }
        let impulse = pair.total_normal_impulse();
        let magnitude = impulse.length();
        if magnitude <= 0.0 {
            continue;
        }
        let mut structural_contact = false;
        for (collider, impulse, sign) in [
            (pair.collider1, -impulse, -1.0),
            (pair.collider2, impulse, 1.0),
        ] {
            if let Ok(identity) = identities.get(collider) {
                if !structure.buildings.contains(identity.building) {
                    continue;
                }
                let Ok(chunk) = structure.chunks.get(collider) else {
                    continue;
                };
                if !chunk.alive {
                    continue;
                }
                let Some((body, cluster, kind)) =
                    structure.cluster_for(collider, identity.building)
                else {
                    continue;
                };
                if capture {
                    for manifold in &pair.manifolds {
                        for point in &manifold.points {
                            if impulse_samples.len() < MAX_IMPULSE_SAMPLES
                                && point.normal_impulse > 0.0
                            {
                                impulse_samples.push(ImpulseSample {
                                    building: identity.building,
                                    chunk: identity.id.clone(),
                                    impulse: manifold.normal * (point.normal_impulse * sign),
                                    captured_at,
                                });
                            }
                        }
                    }
                }
                if *kind == RigidBody::Dynamic && cluster.chunks.len() > 1 {
                    let center = chunk.center_in_building().as_dvec3();
                    let (position, rotation) = structure.poses.get(body).unwrap();
                    let origin = position.0;
                    let inverse_rotation = rotation.0.conjugate();
                    // 同一接触对的逐点冲量共享一次工况。其他接触有独立的受载来源。
                    let shock = shocks.entry((body, key)).or_insert_with(|| ClusterImpact {
                        building: identity.building,
                        chunks: cluster.chunks.clone(),
                        loads: BTreeMap::new(),
                        prepared: None,
                    });
                    let load = shock.loads.entry(identity.id.clone()).or_default();
                    for manifold in &pair.manifolds {
                        for point in &manifold.points {
                            let impulse = (inverse_rotation
                                * manifold.normal
                                * (point.normal_impulse * sign))
                                .as_dvec3();
                            let contact = (inverse_rotation * (point.point - origin)).as_dvec3();
                            load.linear += impulse;
                            load.angular += (contact - center).cross(impulse);
                        }
                    }
                } else {
                    *hits
                        .entry((identity.building, identity.id.clone()))
                        .or_default() += impulse;
                }
                structural_contact = true;
            }
        }
        if structural_contact {
            touching.insert(key);
            contact_count += 1;
            #[cfg(feature = "brp")]
            for entity in [pair.collider1, pair.collider2] {
                if let Ok(collider) = projectiles.get(entity) {
                    let shape = collider.shape();
                    sphere_contacts += u64::from(shape.as_ball().is_some());
                    cube_contacts += u64::from(shape.as_cuboid().is_some());
                }
            }
            max_impulse = max_impulse.max(magnitude);
        }
    }
    state.touching = touching;
    state.shocks.extend(shocks.into_values());
    if let Some(samples) = &mut samples {
        for sample in impulse_samples {
            samples.record_impulse(sample);
        }
    }
    for ((building, id), impulse) in hits {
        let Ok(instance) = structure.buildings.get(building) else {
            continue;
        };
        let Some(asset) = assets.get(&instance.asset) else {
            continue;
        };
        let hit = instance.chunks[&id];
        let chunk = structure.chunks.get(hit).unwrap();
        if !chunk.alive {
            continue;
        }
        let Some((body, _, _)) = structure.cluster_for(hit, building) else {
            continue;
        };
        let (_, rotation) = structure.poses.get(body).unwrap();
        let impulse = rotation.0.conjugate() * impulse;
        queue_local_damage(
            local_impact_bonds(&structure, &asset.manifest, instance, &id, hit),
            building,
            impulse,
            &mut state.pending,
        );
    }
    let mut structural =
        process_dynamic_impact(&structure, &assets, &mut state, capture, captured_at);
    let mut broken = BTreeMap::<Entity, Vec<String>>::new();
    let mut budget = IMPACT_BREAK_LIMIT;
    let mut structural_broken = 0;
    if let Some(report) = &mut structural
        && let Some((building, bonds)) = report.fracture.take()
        && let Ok(instance) = structure.buildings.get(building)
    {
        for id in bonds {
            if let Some(&entity) = instance.bonds.get(&id)
                && let Ok(mut bond) = structure.bonds.get_mut(entity)
                && bond.active
            {
                bond.active = false;
                broken.entry(building).or_default().push(id);
                structural_broken += 1;
                budget -= 1;
            }
        }
        if structural_broken > 0
            && let Some(shock) = report.continuation.take()
        {
            state.shocks.push_back(shock);
        }
    }
    let mut pending = state
        .pending
        .iter()
        .map(|(key, &utilization)| (key.clone(), utilization))
        .collect::<Vec<_>>();
    pending.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    for ((building, id), _) in pending {
        let Ok(instance) = structure.buildings.get(building) else {
            state.pending.remove(&(building, id));
            continue;
        };
        let Some(mut bond) = instance
            .bonds
            .get(&id)
            .and_then(|&entity| structure.bonds.get_mut(entity).ok())
            .filter(|bond| bond.active)
        else {
            state.pending.remove(&(building, id));
            continue;
        };
        if budget == 0 {
            break;
        }
        bond.active = false;
        broken.entry(building).or_default().push(id.clone());
        state.pending.remove(&(building, id));
        budget -= 1;
    }
    for (&building, ids) in &broken {
        commands.entity(building).insert(NeedsClusterRebuild);
        let instance = structure.buildings.get(building).unwrap();
        let asset = assets.get(&instance.asset).unwrap();
        if let Some(burst) = build_bond_feedback(
            instance,
            &asset.manifest,
            ids,
            |entity| {
                let chunk = structure.chunks.get(entity).unwrap();
                let owner = structure
                    .cluster_for(entity, building)
                    .map(|(body, _, _)| body);
                (chunk, owner)
            },
            instance
                .patches
                .iter()
                .map(|&entity| patches.get(entity).unwrap().patch()),
            |body| match body {
                Some(body) => {
                    let (position, rotation) = structure.poses.get(body).unwrap();
                    Transform::from_translation(position.0).with_rotation(rotation.0)
                }
                None => transforms
                    .compute_global_transform(building)
                    .expect("A building has a valid transform hierarchy")
                    .compute_transform(),
            },
        ) {
            bursts.write(burst);
        }
    }
    if let Some(sample) = structural
        .as_mut()
        .and_then(|report| report.force_sample.take())
        && let Some(samples) = &mut samples
    {
        samples.record_force(sample);
    }
    stats.contacts += contact_count;
    #[cfg(feature = "brp")]
    {
        stats.sphere_contacts += sphere_contacts;
        stats.cube_contacts += cube_contacts;
    }
    stats.last_impulse = max_impulse;
    stats.max_impulse = stats.max_impulse.max(max_impulse);
    if let Some(report) = structural {
        stats.structural_solves += 1;
        stats.structural_failures += u64::from(!report.valid);
        stats.max_structural_solve_ms = stats.max_structural_solve_ms.max(report.elapsed_ms);
        stats.max_force_balance_ratio = stats
            .max_force_balance_ratio
            .max(report.force_balance_ratio);
        stats.max_moment_balance_ratio = stats
            .max_moment_balance_ratio
            .max(report.moment_balance_ratio);
        stats.max_gauge_reaction_ratio = stats
            .max_gauge_reaction_ratio
            .max(report.gauge_reaction_ratio);
        stats.max_structural_utilization =
            stats.max_structural_utilization.max(report.max_utilization);
    }
    stats.pending_structural = state.shocks.len();
    stats.structural_broken_bonds += structural_broken;
    stats.released_structural_events = state.released;
    stats.rebound_fresh_structural_events = state.rebound_fresh;
    stats.invalidated_structural_events = state.invalidated;
    stats.broken_bonds += broken.values().map(|ids| ids.len() as u64).sum::<u64>();
}
