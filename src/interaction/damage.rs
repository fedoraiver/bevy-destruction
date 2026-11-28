use avian3d::prelude::*;
use bevy::prelude::*;

use crate::{
    asset::DestructibleAsset,
    debug::FractureEdges,
    physics::NeedsClusterRebuild,
    scene::{
        BondInstance, BuildingInstance, ChunkIdentity, ChunkInstance, ChunkPrimitive,
        PatchInstance, PatchOfBond, RefreshDamageDetails, RemovedChunk, chunk_primitives,
    },
    vfx::{BreakBurst, EmissionSurface},
};

use super::{BREAK_LIMIT, BREAK_RADIUS, DamageRequest, selection::surface_distance};

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_damage(
    mut commands: Commands,
    mut requests: MessageReader<DamageRequest>,
    spatial: SpatialQuery,
    identities: Query<&ChunkIdentity>,
    poses: Query<(&Position, &Rotation)>,
    instances: Query<&BuildingInstance>,
    mut chunks: Query<&mut ChunkInstance>,
    children: Query<&Children>,
    primitive_meshes: Query<(), With<ChunkPrimitive>>,
    mut bonds: Query<&mut BondInstance>,
    patches: Query<(&PatchInstance, &PatchOfBond)>,
    assets: Res<Assets<DestructibleAsset>>,
    mut bursts: MessageWriter<BreakBurst>,
) {
    for request in requests.read() {
        let ray = &request.ray;
        let hit = spatial.cast_ray_predicate(
            ray.origin,
            ray.direction,
            1000.0,
            true,
            &SpatialQueryFilter::default(),
            &|entity| chunks.get(entity).is_ok_and(|chunk| chunk.alive),
        );
        let Some(hit) = hit else {
            continue;
        };
        let Ok(identity) = identities.get(hit.entity) else {
            continue;
        };
        let Ok(instance) = instances.get(identity.building) else {
            continue;
        };
        let Some(asset) = assets.get(&instance.asset) else {
            continue;
        };
        let point = ray.get_point(hit.distance);
        let normal = hit.normal;
        let mut candidates = Vec::new();
        for &entity in instance.bonds.values() {
            let bond = bonds.get(entity).unwrap();
            if !bond.active {
                continue;
            }
            debug_assert_eq!(instance.bonds[&bond.id], entity);
            let [a, b] = bond.endpoints;
            let neighbor = if hit.entity == a {
                b
            } else if hit.entity == b {
                a
            } else {
                continue;
            };
            if let Ok(chunk) = chunks.get(neighbor)
                && chunk.alive
                && let Some(distance) = {
                    let rest_from_world = world_from_rest(neighbor, chunk, &poses).inverse();
                    surface_distance(
                        &chunk.triangles,
                        rest_from_world.transform_point3(point),
                        rest_from_world.transform_vector3(normal),
                    )
                }
                && distance <= BREAK_RADIUS
            {
                candidates.push((identities.get(neighbor).unwrap().id.clone(), distance));
            }
        }
        candidates.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
        let mut selected = vec![identity.id.clone()];
        selected.extend(
            candidates
                .into_iter()
                .take(BREAK_LIMIT - 1)
                .map(|(id, _)| id),
        );
        let material = asset
            .manifest
            .chunks
            .iter()
            .find(|chunk| chunk.id == identity.id)
            .unwrap()
            .material
            .clone();
        let removed_volume = asset
            .manifest
            .chunks
            .iter()
            .filter(|chunk| selected.contains(&chunk.id))
            .map(|chunk| chunk.volume)
            .sum();
        let mut surfaces = Vec::new();
        let mut exposed_area = 0.0;
        for &entity in &instance.patches {
            let (runtime_patch, owner) = patches.get(entity).unwrap();
            if !bonds.get(owner.bond).unwrap().active {
                continue;
            }
            let patch = runtime_patch.patch();
            let removed_a = selected.contains(&patch.a);
            let removed_b = selected.contains(&patch.b);
            if removed_a == removed_b {
                continue;
            }
            let survivor = if removed_a { &patch.b } else { &patch.a };
            let survivor_entity = instance.chunks[survivor];
            let chunk = chunks.get(survivor_entity).unwrap();
            if !chunk.alive {
                continue;
            }
            exposed_area += patch.area;
            let normal = if removed_a {
                -patch.normal
            } else {
                patch.normal
            };
            let world_from_rest = world_from_rest(survivor_entity, chunk, &poses);
            surfaces.extend(patch.triangles.iter().map(|triangle| EmissionSurface {
                triangle: triangle.map(|point| world_from_rest.transform_point3(point)),
                normal: world_from_rest.transform_vector3(normal).normalize(),
            }));
        }
        surfaces.sort_by(|a, b| {
            let center =
                |s: &EmissionSurface| (s.triangle[0] + s.triangle[1] + s.triangle[2]) / 3.0;
            center(a)
                .distance_squared(point)
                .total_cmp(&center(b).distance_squared(point))
        });
        surfaces.truncate(256);
        for id in &selected {
            let entity = instance.chunks[id];
            chunks.get_mut(entity).unwrap().alive = false;
            commands
                .entity(entity)
                .insert((RemovedChunk, Visibility::Hidden))
                .remove::<FractureEdges>();
            for primitive in chunk_primitives(children.get(entity).ok(), |entity| {
                primitive_meshes.contains(entity)
            }) {
                commands.entity(primitive).despawn();
            }
        }
        for &entity in instance.bonds.values() {
            let mut bond = bonds.get_mut(entity).unwrap();
            if bond.active
                && selected
                    .iter()
                    .any(|id| bond.endpoints.contains(&instance.chunks[id]))
            {
                bond.active = false;
            }
        }
        commands
            .entity(identity.building)
            .insert((NeedsClusterRebuild, RefreshDamageDetails));
        bursts.write(BreakBurst {
            point,
            normal,
            seed: asset
                .manifest
                .name
                .bytes()
                .chain([0])
                .chain(identity.id.bytes())
                .fold(0xcbf29ce484222325_u64, |hash, byte| {
                    (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
                }),
            material,
            removed_volume,
            exposed_area,
            surfaces,
        });
        info!("Removed {} chunks at {point:?}", selected.len());
    }
}

fn world_from_rest(
    entity: Entity,
    chunk: &ChunkInstance,
    poses: &Query<(&Position, &Rotation)>,
) -> bevy::math::Affine3A {
    let (position, rotation) = poses.get(entity).expect("Chunk has a physics pose");
    Transform::from_translation(position.0)
        .with_rotation(rotation.0)
        .compute_affine()
        * chunk.transform.compute_affine().inverse()
}
