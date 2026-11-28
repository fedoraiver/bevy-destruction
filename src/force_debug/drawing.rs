use std::collections::BTreeMap;

use avian3d::prelude::{ColliderOf, Gravity, RigidBody};
use bevy::{mesh::MeshTag, prelude::*};

use crate::{
    force_samples::{DynamicForceSample, ForceCapture, ForceSamples, bond_signature},
    material::{HEAT_AMBER_TAG, HEAT_RED_TAG},
    physics::Cluster,
    scene::{
        BondInstance, BuildingInstance, ChunkIdentity, ChunkInstance, ChunkPrimitive,
        chunk_primitives,
    },
    stress::BondStress,
    stress_runtime::{StressSettings, StressState, StructuralLoads},
};

use super::{ForceBondReadout, ForceGizmos, ForceInspection, ForceViewStats, picking::Aim};

const MAX_INSPECTED_BONDS: usize = 3;

pub(super) fn sample_members_current<'a>(
    sample: &DynamicForceSample,
    instance: &BuildingInstance,
    cluster: &Cluster,
    parent: &ChildOf,
    chunk: impl Fn(Entity) -> Option<(&'a ChunkInstance, Option<&'a ColliderOf>)>,
    current_bond_signature: impl FnOnce() -> u64,
) -> bool {
    parent.parent() == sample.building
        && !cluster.anchored
        && cluster.chunks.len() == sample.chunks.len()
        && sample.chunks.iter().all(|id| {
            instance
                .chunks
                .get(id)
                .and_then(|&entity| chunk(entity))
                .is_some_and(|(chunk, collider)| {
                    chunk.alive && collider.is_some_and(|collider| collider.body == sample.body)
                })
        })
        && sample.bond_signature == current_bond_signature()
}

pub(super) fn heat_tag(utilization: f64) -> u32 {
    if utilization >= 1.0 {
        HEAT_RED_TAG
    } else if utilization >= 0.5 {
        HEAT_AMBER_TAG
    } else {
        0
    }
}

pub(super) fn direction_on_chunk(
    pose: &GlobalTransform,
    force: [f64; 3],
    is_a: bool,
) -> Option<Vec3> {
    let direction = bevy::math::DVec3::from_array(force)
        .try_normalize()?
        .as_vec3();
    // The restoring force is +traction on a and -traction on b.
    let direction = if is_a { direction } else { -direction };
    Some(pose.rotation() * direction)
}

pub(super) struct InspectionBond {
    pub(super) readout: ForceBondReadout,
    pub(super) direction: Option<Vec3>,
}

pub(super) fn order_inspection_bonds(bonds: &mut Vec<InspectionBond>) {
    bonds.sort_by(|a, b| {
        b.readout
            .utilization
            .total_cmp(&a.readout.utilization)
            .then_with(|| a.readout.id.cmp(&b.readout.id))
    });
    bonds.truncate(MAX_INSPECTED_BONDS);
}

fn update_heat_tags(
    desired: BTreeMap<Entity, u32>,
    painted: &mut BTreeMap<Entity, u32>,
    tags: &mut Query<&mut MeshTag, With<ChunkPrimitive>>,
) {
    for &entity in painted
        .keys()
        .filter(|entity| !desired.contains_key(entity))
    {
        if let Ok(mut tag) = tags.get_mut(entity) {
            tag.set_if_neq(MeshTag::new(0));
        }
    }
    for (&entity, &value) in &desired {
        if painted.get(&entity) != Some(&value)
            && let Ok(mut tag) = tags.get_mut(entity)
        {
            tag.set_if_neq(MeshTag::new(value));
        }
    }
    *painted = desired;
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn draw(
    capture: Res<ForceCapture>,
    aim: Res<Aim>,
    mut samples: ResMut<ForceSamples>,
    mut stats: ResMut<ForceViewStats>,
    time: Res<Time<Fixed>>,
    gravity: Res<Gravity>,
    settings: Res<StressSettings>,
    instances: Query<(
        Entity,
        Ref<BuildingInstance>,
        Option<&StressState>,
        Option<Ref<StructuralLoads>>,
        &GlobalTransform,
    )>,
    chunks: Query<(&ChunkIdentity, &ChunkInstance, Option<&ColliderOf>)>,
    children: Query<&Children>,
    bonds: Query<&BondInstance>,
    bodies: Query<(&GlobalTransform, &RigidBody, &Cluster, &ChildOf)>,
    mut tags: Query<&mut MeshTag, With<ChunkPrimitive>>,
    mut painted: Local<BTreeMap<Entity, u32>>,
    mut gizmos: Gizmos<ForceGizmos>,
) {
    *stats = ForceViewStats::default();
    if !capture.enabled {
        samples.clear();
        update_heat_tags(BTreeMap::new(), &mut painted, &mut tags);
        return;
    }
    stats.pick_queries = usize::from(aim.queried);
    let now = time.elapsed_secs_f64();
    let mut signatures = BTreeMap::new();
    samples.prune(
        now,
        |sample| {
            instances
                .get(sample.building)
                .ok()
                .zip(bodies.get(sample.body).ok())
                .is_some_and(|((_, instance, _, _, _), (_, kind, cluster, parent))| {
                    *kind == RigidBody::Dynamic
                        && sample_members_current(
                            sample,
                            &instance,
                            cluster,
                            parent,
                            |entity| {
                                chunks
                                    .get(entity)
                                    .ok()
                                    .map(|(_, chunk, collider)| (chunk, collider))
                            },
                            || {
                                *signatures.entry(sample.building).or_insert_with(|| {
                                    bond_signature(instance.bonds.iter().map(|(id, &entity)| {
                                        (
                                            id.as_str(),
                                            bonds.get(entity).is_ok_and(|bond| bond.active),
                                        )
                                    }))
                                })
                            },
                        )
                })
        },
        |sample| {
            instances
                .get(sample.building)
                .is_ok_and(|(_, instance, _, _, _)| {
                    instance
                        .chunks
                        .get(&sample.chunk)
                        .and_then(|&entity| chunks.get(entity).ok())
                        .is_some_and(|(_, chunk, _)| chunk.alive)
                })
        },
    );

    let mut desired = BTreeMap::new();
    let mut inspection_bonds = Vec::new();
    for (building, instance, state, loads, root) in &instances {
        let selected = aim.hit.as_ref().filter(|hit| hit.key.0 == building);
        let current_static = state.filter(|state| {
            settings.enabled
                && state.inputs_current(
                    instance.last_changed().get(),
                    loads.as_ref().map(|loads| loads.last_changed()),
                    settings.last_changed().get(),
                    root.compute_transform().rotation.conjugate() * gravity.0,
                )
        });
        let mut heat = BTreeMap::<Entity, u32>::new();
        for &entity in instance.bonds.values() {
            let Ok(bond) = bonds.get(entity) else {
                continue;
            };
            if !bond.active {
                continue;
            }
            let Ok([(a_id, a, Some(a_collider)), (b_id, b, Some(b_collider))]) =
                chunks.get_many(bond.endpoints)
            else {
                continue;
            };
            let a_id = a_id.id.as_str();
            let b_id = b_id.id.as_str();
            if !a.alive || !b.alive || a_collider.body != b_collider.body {
                continue;
            }
            let body = a_collider.body;
            let Ok((pose, kind, _, parent)) = bodies.get(body) else {
                continue;
            };
            if parent.parent() != building {
                continue;
            }
            let impact = *kind == RigidBody::Dynamic;
            let stress = match kind {
                RigidBody::Static => current_static
                    .and_then(|state| state.bonds.get(&bond.id))
                    .filter(|load| load.valid)
                    .map(|load| &load.stress),
                RigidBody::Dynamic => samples
                    .dynamic
                    .iter()
                    .find(|sample| sample.building == building && sample.body == body)
                    .and_then(|sample| sample.bonds.get(&bond.id)),
                _ => None,
            };
            let Some(stress) = stress else {
                continue;
            };
            let Some((force_n, moment_nm)) = finite_load(stress) else {
                continue;
            };

            if impact {
                stats.dynamic_bonds += 1;
            } else {
                stats.static_bonds += 1;
            }
            stats.max_force_n = stats.max_force_n.max(force_n);

            let tag = heat_tag(stress.utilization);
            if tag != 0 {
                for endpoint in bond.endpoints {
                    heat.entry(endpoint)
                        .and_modify(|old| *old = (*old).max(tag))
                        .or_insert(tag);
                }
            }
            if let Some(hit) = selected
                && (hit.key.1 == a_id || hit.key.1 == b_id)
            {
                inspection_bonds.push(InspectionBond {
                    direction: direction_on_chunk(pose, stress.force, hit.key.1 == a_id),
                    readout: ForceBondReadout {
                        id: bond.id.clone(),
                        force_n,
                        moment_nm,
                        utilization: stress.utilization,
                        impact,
                    },
                });
            }
        }
        stats.tinted_chunks += heat.len();
        for (entity, tag) in heat {
            desired.extend(
                chunk_primitives(children.get(entity).ok(), |entity| tags.contains(entity))
                    .map(|primitive| (primitive, tag)),
            );
        }
    }
    update_heat_tags(desired, &mut painted, &mut tags);

    if let Some(hit) = &aim.hit {
        order_inspection_bonds(&mut inspection_bonds);
        if let Some(direction) = inspection_bonds.first().and_then(|bond| bond.direction)
            && aim.arrow_length > 0.0001
        {
            gizmos.arrow(
                hit.point,
                hit.point + direction * aim.arrow_length,
                Color::srgb(0.24, 0.90, 1.0),
            );
            stats.force_arrows = 1;
        }
        let impulse_ns = samples
            .impulses
            .iter()
            .filter(|sample| sample.building == hit.key.0 && sample.chunk == hit.key.1)
            .map(|sample| sample.impulse.as_dvec3().length())
            .filter(|value| value.is_finite() && *value > 0.0)
            .max_by(f64::total_cmp);
        stats.inspected = Some(ForceInspection {
            chunk: hit.key.clone(),
            bonds: inspection_bonds
                .into_iter()
                .map(|bond| bond.readout)
                .collect(),
            impulse_ns,
        });
    }
}

fn finite_load(stress: &BondStress) -> Option<(f64, f64)> {
    let force_n = bevy::math::DVec3::from_array(stress.force).length();
    let moment_nm = bevy::math::DVec3::from_array(stress.moment).length();
    (force_n.is_finite() && moment_nm.is_finite() && stress.utilization.is_finite())
        .then_some((force_n, moment_nm))
}
