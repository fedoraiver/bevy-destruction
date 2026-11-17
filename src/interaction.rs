use std::collections::VecDeque;

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::{
    DemoOptions,
    asset::DestructibleAsset,
    camera::CameraControl,
    debug::FractureEdges,
    scene::{BuildingInstance, ChunkIdentity, ChunkInstance, DemoCamera, RemovedChunk},
    vfx::{BreakBurst, EmissionSurface},
};

pub const BREAK_RADIUS: f32 = 0.4;
pub const BREAK_LIMIT: usize = 6;
const SURFACE_DEPTH: f32 = 0.08;

pub struct InteractionPlugin;

impl Plugin for InteractionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DamageRequests>()
            .init_resource::<ScriptedClick>()
            .add_systems(Update, (cursor_damage, apply_damage).chain());
    }
}

#[derive(Resource, Default)]
pub struct DamageRequests(pub VecDeque<Ray3d>);

#[derive(Resource, Default)]
pub struct ScriptedClick(pub Option<Vec2>);

pub(crate) fn bond_feedback(world: &mut World, building: Entity, broken: &[String]) {
    let instance = world.get::<BuildingInstance>(building).unwrap();
    let asset = world
        .resource::<Assets<DestructibleAsset>>()
        .get(&instance.asset)
        .unwrap();
    let Some(bond) = asset
        .manifest
        .bonds
        .iter()
        .find(|bond| broken.contains(&bond.id))
    else {
        return;
    };
    let chunk = &instance.chunks[&bond.a];
    let pose_for = |id: &str| {
        if let Some(body) = instance.chunks[id].cluster {
            Transform::from_translation(world.get::<Position>(body).unwrap().0)
                .with_rotation(world.get::<Rotation>(body).unwrap().0)
        } else {
            crate::physics::global_transform(world, building).compute_transform()
        }
    };
    let pose = pose_for(&bond.a);
    let mut surfaces = Vec::new();
    let mut area = 0.0;
    for patch in instance
        .patches
        .iter()
        .filter(|patch| broken.contains(&patch.bond_id))
    {
        let pose = pose_for(&patch.a);
        area += patch.area;
        surfaces.extend(patch.triangles.iter().map(|triangle| EmissionSurface {
            triangle: triangle.map(|point| pose.transform_point(point)),
            normal: pose.rotation * patch.normal,
        }));
    }
    surfaces.truncate(256);
    let burst = BreakBurst {
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
    };
    world.write_message(burst);
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn cursor_damage(
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    cameras: Query<(Entity, &Camera), With<DemoCamera>>,
    transforms: TransformHelper,
    mut requests: ResMut<DamageRequests>,
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
        scripted.0.take()
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
        requests.0.push_back(ray);
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
pub fn apply_damage(
    mut commands: Commands,
    mut requests: ResMut<DamageRequests>,
    spatial: SpatialQuery,
    identities: Query<&ChunkIdentity>,
    poses: Query<(&Position, &Rotation)>,
    mut instances: Query<&mut BuildingInstance>,
    assets: Res<Assets<DestructibleAsset>>,
    mut bursts: MessageWriter<BreakBurst>,
) {
    while let Some(ray) = requests.0.pop_front() {
        let hit = spatial.cast_ray_predicate(
            ray.origin,
            ray.direction,
            1000.0,
            true,
            &SpatialQueryFilter::default(),
            &|entity| {
                let Ok(identity) = identities.get(entity) else {
                    return false;
                };
                instances
                    .get(identity.building)
                    .ok()
                    .and_then(|instance| instance.chunks.get(&identity.id))
                    .is_some_and(|chunk| chunk.alive)
            },
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
        for bond in &asset.manifest.bonds {
            if !instance.bonds.get(&bond.id).copied().unwrap_or(false) {
                continue;
            }
            let neighbor = if bond.a == identity.id {
                &bond.b
            } else if bond.b == identity.id {
                &bond.a
            } else {
                continue;
            };
            if let Some(chunk) = instance.chunks.get(neighbor).filter(|chunk| chunk.alive)
                && let Some(distance) = {
                    let rest_from_world = world_from_rest(chunk, &poses).inverse();
                    surface_distance(
                        &chunk.triangles,
                        rest_from_world.transform_point3(point),
                        rest_from_world.transform_vector3(normal),
                    )
                }
                && distance <= BREAK_RADIUS
            {
                candidates.push((neighbor.clone(), distance));
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
        let mut dirty = selected
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        for patch in instance.patches.iter() {
            if !instance.bonds[&patch.bond_id] {
                continue;
            }
            let removed_a = selected.contains(&patch.a);
            let removed_b = selected.contains(&patch.b);
            if removed_a == removed_b {
                continue;
            }
            let survivor = if removed_a { &patch.b } else { &patch.a };
            if !instance.chunks[survivor].alive {
                continue;
            }
            dirty.insert(survivor.clone());
            exposed_area += patch.area;
            let normal = if removed_a {
                -patch.normal
            } else {
                patch.normal
            };
            let world_from_rest = world_from_rest(&instance.chunks[survivor], &poses);
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
        instance.detail_dirty.extend(dirty);
        for id in &selected {
            let chunk = instance.chunks.get_mut(id).unwrap();
            chunk.alive = false;
            commands
                .entity(chunk.entity)
                .insert((RemovedChunk, Visibility::Hidden))
                .remove::<FractureEdges>();
            for &primitive in &chunk.primitives {
                commands.entity(primitive).despawn();
            }
        }
        for bond in &asset.manifest.bonds {
            if selected.contains(&bond.a) || selected.contains(&bond.b) {
                instance.bonds.insert(bond.id.clone(), false);
            }
        }
        instance.topology_dirty = true;
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
    chunk: &ChunkInstance,
    poses: &Query<(&Position, &Rotation)>,
) -> bevy::math::Affine3A {
    let (position, rotation) = poses.get(chunk.entity).expect("Chunk has a physics pose");
    Transform::from_translation(position.0)
        .with_rotation(rotation.0)
        .compute_affine()
        * chunk.transform.compute_affine().inverse()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn face(z: f32) -> [[Vec3; 3]; 1] {
        [[
            Vec3::new(0.0, 0.0, z),
            Vec3::new(10.0, 0.0, z),
            Vec3::new(0.0, 10.0, z),
        ]]
    }
    #[test]
    fn selection_uses_surface_distance_and_rejects_back_wall() {
        let point = Vec3::new(0.05, 0.05, 0.0);
        assert!(surface_distance(&face(0.0), point, Vec3::Z).unwrap() < 0.001);
        assert!(surface_distance(&face(-0.2), point, Vec3::Z).is_none());
        assert!(surface_distance(&face(0.0), point, -Vec3::Z).is_none());
        assert!(
            surface_distance(&face(0.0), Vec3::new(-1.0, 0.0, 0.0), Vec3::Z).unwrap()
                > BREAK_RADIUS
        );
    }

    #[test]
    fn fracture_feedback_uses_each_moving_cluster_pose() {
        use crate::{asset::Manifest, fracture_detail::ContactPatch};
        let mut world = World::new();
        world.init_resource::<Assets<DestructibleAsset>>();
        world.init_resource::<Messages<BreakBurst>>();
        let mut manifest: Manifest = serde_json::from_str(include_str!(
            "../assets/buildings/rectangle.destruction.json"
        ))
        .unwrap();
        manifest.schema_version = 1;
        let template = manifest.bonds[0].clone();
        manifest.bonds = [("ab", "a", "b"), ("cd", "c", "d")]
            .map(|(id, a, b)| crate::asset::BondRecord {
                id: id.into(),
                a: a.into(),
                b: b.into(),
                center: [0.0; 3],
                normal: Vec3::Y.to_array(),
                patches: Vec::new(),
                ..template.clone()
            })
            .into();
        let asset = world
            .resource_mut::<Assets<DestructibleAsset>>()
            .add(DestructibleAsset {
                manifest,
                gltf: Handle::default(),
            });
        let root = world
            .spawn((Transform::IDENTITY, BuildingInstance::pending(asset)))
            .id();
        let poses = [
            Transform::from_xyz(4.0, 2.0, 1.0).with_rotation(Quat::from_rotation_z(0.3)),
            Transform::from_xyz(-8.0, 3.0, -2.0).with_rotation(Quat::from_rotation_y(0.7)),
        ];
        let triangle = [Vec3::ZERO, Vec3::X, Vec3::Z];
        let mut patches = Vec::new();
        for ((id, a, b), pose) in [("ab", "a", "b"), ("cd", "c", "d")].into_iter().zip(poses) {
            let body = world
                .spawn((Position(pose.translation), Rotation(pose.rotation)))
                .id();
            let entity = world.spawn_empty().id();
            world
                .get_mut::<BuildingInstance>(root)
                .unwrap()
                .chunks
                .insert(
                    a.into(),
                    ChunkInstance {
                        entity,
                        primitives: Vec::new(),
                        alive: true,
                        triangles: Vec::new(),
                        transform: Transform::IDENTITY,
                        material: "concrete".into(),
                        detail: None,
                        cluster: Some(body),
                    },
                );
            patches.push(ContactPatch {
                id: id.into(),
                bond_id: id.into(),
                a: a.into(),
                b: b.into(),
                normal: Vec3::Y,
                triangles: vec![triangle],
                area: 0.5,
                rims: Vec::new(),
            });
        }
        world.get_mut::<BuildingInstance>(root).unwrap().patches = std::sync::Arc::new(patches);
        bond_feedback(&mut world, root, &["ab".into(), "cd".into()]);
        let burst = world
            .resource_mut::<Messages<BreakBurst>>()
            .drain()
            .next()
            .unwrap();
        assert_eq!(burst.surfaces.len(), 2);
        for (surface, pose) in burst.surfaces.iter().zip(poses) {
            for (actual, local) in surface.triangle.iter().zip(triangle) {
                assert!(actual.distance(pose.transform_point(local)) < 1e-5);
            }
            assert!(surface.normal.distance(pose.rotation * Vec3::Y) < 1e-5);
        }
    }
}
