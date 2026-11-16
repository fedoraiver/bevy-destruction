use std::collections::VecDeque;

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::{
    DemoOptions,
    asset::DestructibleAsset,
    camera::CameraControl,
    debug::FractureEdges,
    scene::{BuildingInstance, ChunkIdentity, DemoCamera, RemovedChunk},
    vfx::BreakBurst,
};

pub const BREAK_RADIUS: f32 = 0.4;
pub const BREAK_LIMIT: usize = 6;
const SURFACE_DEPTH: f32 = 0.08;

pub struct InteractionPlugin;

impl Plugin for InteractionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DamageRequests>()
            .init_resource::<ScriptedClick>()
            .init_resource::<PendingColliderRemoval>()
            .add_systems(Update, (cursor_damage, apply_damage).chain())
            .add_systems(FixedUpdate, remove_colliders);
    }
}

#[derive(Resource, Default)]
pub struct DamageRequests(pub VecDeque<Ray3d>);

#[derive(Resource, Default)]
pub struct ScriptedClick(pub Option<Vec2>);

#[derive(Resource, Default)]
pub(crate) struct PendingColliderRemoval(Vec<Entity>);

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
    let cursor = if options.smoke_output.is_some() {
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
    mut instances: Query<&mut BuildingInstance>,
    assets: Res<Assets<DestructibleAsset>>,
    mut pending: ResMut<PendingColliderRemoval>,
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
                && let Some(distance) = surface_distance(&chunk.triangles, point, normal)
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
            pending.0.push(chunk.entity);
        }
        let mut exposed_points = Vec::new();
        for bond in &asset.manifest.bonds {
            if selected.contains(&bond.a) || selected.contains(&bond.b) {
                if selected.contains(&bond.a) != selected.contains(&bond.b)
                    && instance.bonds.get(&bond.id).copied().unwrap_or(false)
                {
                    exposed_points.push(Vec3::from(bond.center));
                }
                instance.bonds.insert(bond.id.clone(), false);
            }
        }
        exposed_points.sort_by(|a, b| {
            a.distance_squared(point)
                .total_cmp(&b.distance_squared(point))
        });
        exposed_points.truncate(12);
        bursts.write(BreakBurst {
            point,
            normal,
            seed: hit.entity.to_bits(),
            exposed_points,
        });
        info!("Removed {} chunks at {point:?}", selected.len());
    }
}

fn remove_colliders(mut commands: Commands, mut pending: ResMut<PendingColliderRemoval>) {
    for entity in pending.0.drain(..) {
        commands.entity(entity).remove::<(Collider, RigidBody)>();
    }
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
}
