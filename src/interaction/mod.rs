use avian3d::prelude::*;
use bevy::prelude::*;

use crate::{
    DemoOptions,
    asset::{DestructibleAsset, Manifest},
    camera::CameraControl,
    debug::FractureEdges,
    fracture_detail::ContactPatch,
    scene::{
        BondInstance, BuildingInstance, ChunkIdentity, ChunkInstance, DemoCamera, PatchInstance,
        PatchOfBond, RefreshDamageDetails, RemovedChunk,
    },
    vfx::{BreakBurst, EmissionSurface},
};

pub const BREAK_RADIUS: f32 = 0.4;
pub const BREAK_LIMIT: usize = 6;
const SURFACE_DEPTH: f32 = 0.08;

pub struct InteractionPlugin;

impl Plugin for InteractionPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<DamageRequest>()
            .init_resource::<ScriptedClick>()
            .add_systems(Update, (cursor_damage, apply_damage).chain());
    }
}

#[derive(Message)]
pub(crate) struct DamageRequest {
    pub ray: Ray3d,
}

#[derive(Resource, Default)]
pub struct ScriptedClick {
    pub cursor: Option<Vec2>,
}

pub(crate) fn build_bond_feedback<'a>(
    instance: &BuildingInstance,
    manifest: &Manifest,
    broken: &[String],
    chunk_at: impl Fn(Entity) -> (&'a ChunkInstance, Option<Entity>),
    patches: impl Iterator<Item = &'a ContactPatch>,
    world_pose: impl Fn(Option<Entity>) -> Transform,
) -> Option<BreakBurst> {
    let bond = manifest
        .bonds
        .iter()
        .find(|bond| broken.contains(&bond.id))?;
    let (chunk, body) = chunk_at(instance.chunks[&bond.a]);
    let pose = world_pose(body);
    let mut surfaces = Vec::new();
    let mut area = 0.0;
    for patch in patches.filter(|patch| broken.contains(&patch.bond_id)) {
        let (_, body) = chunk_at(instance.chunks[&patch.a]);
        let pose = world_pose(body);
        area += patch.area;
        surfaces.extend(patch.triangles.iter().map(|triangle| EmissionSurface {
            triangle: triangle.map(|point| pose.transform_point(point)),
            normal: pose.rotation * patch.normal,
        }));
    }
    surfaces.truncate(256);
    Some(BreakBurst {
        point: pose.transform_point(bond.center.into()),
        normal: pose.rotation * Vec3::from(bond.normal),
        seed: broken
            .iter()
            .flat_map(|id| id.bytes())
            .fold(0xcbf29ce484222325u64, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
            }),
        material: chunk.material.clone(),
        removed_volume: 0.0,
        exposed_area: area,
        surfaces,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn cursor_damage(
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    cameras: Query<(Entity, &Camera), With<DemoCamera>>,
    transforms: TransformHelper,
    mut requests: MessageWriter<DamageRequest>,
    options: Res<DemoOptions>,
    mut scripted: ResMut<ScriptedClick>,
    control: Res<CameraControl>,
) {
    if !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    let (Ok(window), Ok((entity, camera))) = (windows.single(), cameras.single()) else {
        return;
    };
    let cursor = if options.scripted() {
        scripted.cursor.take()
    } else {
        if !control.captured || control.suppress_click || !window.focused {
            return;
        }
        Some(window.size() * 0.5)
    };
    let Ok(transform) = transforms.compute_global_transform(entity) else {
        return;
    };
    if let Some(cursor) = cursor
        && let Ok(ray) = camera.viewport_to_world(&transform, cursor)
    {
        requests.write(DamageRequest { ray });
    }
}

pub fn surface_distance(triangles: &[[Vec3; 3]], point: Vec3, normal: Vec3) -> Option<f32> {
    triangles
        .iter()
        .filter_map(|triangle| {
            let geometric_normal = (triangle[1] - triangle[0])
                .cross(triangle[2] - triangle[0])
                .normalize_or_zero();
            if geometric_normal.dot(normal) < 0.7 {
                return None;
            }
            let closest = closest_on_triangle(*triangle, point, geometric_normal);
            if (closest - point).dot(normal).abs() > SURFACE_DEPTH {
                return None;
            }
            Some(closest.distance(point))
        })
        .min_by(f32::total_cmp)
}

fn closest_on_triangle([a, b, c]: [Vec3; 3], point: Vec3, normal: Vec3) -> Vec3 {
    let projected = point - normal * (point - a).dot(normal);
    if [(a, b), (b, c), (c, a)]
        .iter()
        .all(|(start, end)| (*end - *start).cross(projected - *start).dot(normal) >= -1e-7)
    {
        return projected;
    }
    [(a, b), (b, c), (c, a)]
        .map(|(start, end)| {
            let edge = end - start;
            start + edge * ((point - start).dot(edge) / edge.length_squared()).clamp(0.0, 1.0)
        })
        .into_iter()
        .min_by(|a, b| {
            a.distance_squared(point)
                .total_cmp(&b.distance_squared(point))
        })
        .unwrap()
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_damage(
    mut commands: Commands,
    mut requests: MessageReader<DamageRequest>,
    spatial: SpatialQuery,
    identities: Query<&ChunkIdentity>,
    poses: Query<(&Position, &Rotation)>,
    mut instances: Query<&mut BuildingInstance>,
    mut chunks: Query<&mut ChunkInstance>,
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
        let Ok(mut instance) = instances.get_mut(identity.building) else {
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
            let mut chunk = chunks.get_mut(entity).unwrap();
            chunk.alive = false;
            commands
                .entity(entity)
                .insert((RemovedChunk, Visibility::Hidden))
                .remove::<FractureEdges>();
            for &primitive in &chunk.primitives {
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
        instance.topology_dirty = true;
        commands
            .entity(identity.building)
            .insert(RefreshDamageDetails);
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

#[cfg(test)]
mod tests;
