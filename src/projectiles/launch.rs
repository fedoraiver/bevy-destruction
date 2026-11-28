use avian3d::prelude::*;
use bevy::{prelude::*, transform::helper::TransformHelper};

use crate::{camera::CameraControl, scene::DemoCamera};

use super::{
    ImpactState, ImpactStats, LaunchRequest, LaunchRequests, PROJECTILE_MASS, PROJECTILE_SPEED,
    ProjectileShape, ResetProjectiles, RigidProjectile,
};

pub(super) fn reset(
    _: On<ResetProjectiles>,
    mut requests: ResMut<LaunchRequests>,
    mut state: ResMut<ImpactState>,
    mut stats: ResMut<ImpactStats>,
) {
    requests.queue.clear();
    *state = ImpactState::default();
    *stats = ImpactStats::default();
}

pub(super) fn fire(
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    control: Res<CameraControl>,
    windows: Query<&Window>,
    camera: (Query<Entity, With<DemoCamera>>, TransformHelper),
    mut requests: ResMut<LaunchRequests>,
) {
    if !control.captured || control.suppress_click || !windows.iter().any(|window| window.focused) {
        return;
    }
    let shape = if buttons.just_pressed(MouseButton::Right) {
        ProjectileShape::Sphere
    } else if keys.just_pressed(KeyCode::KeyR) {
        ProjectileShape::Cube
    } else {
        return;
    };
    let (cameras, transforms) = camera;
    if let Ok(entity) = cameras.single()
        && let Ok(camera) = transforms.compute_global_transform(entity)
    {
        requests.queue.push_back(LaunchRequest {
            origin: camera.translation() + camera.forward() * 1.5,
            direction: camera.forward(),
            shape,
        });
    }
}

pub(super) fn spawn_projectiles(
    mut commands: Commands,
    mut requests: ResMut<LaunchRequests>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    projectiles: Query<Entity, With<RigidProjectile>>,
    mut stats: ResMut<ImpactStats>,
) {
    let mut count = projectiles.iter().len();
    for request in requests.queue.drain(..) {
        if count >= 12 {
            continue;
        }
        let (mesh, collider) = match request.shape {
            ProjectileShape::Sphere => (Mesh::from(Sphere::new(0.65)), Collider::sphere(0.65)),
            ProjectileShape::Cube => (
                Mesh::from(Cuboid::from_length(1.2)),
                Collider::cuboid(1.2, 1.2, 1.2),
            ),
        };
        let density = ColliderDensity(PROJECTILE_MASS / collider.mass(1.0));
        commands
            .spawn((
                Name::new("Unbreakable rigid impactor"),
                RigidProjectile {
                    age: 0.0,
                    #[cfg(feature = "brp")]
                    shape: request.shape,
                },
                RigidBody::Dynamic,
                collider,
                density,
                Transform::from_translation(request.origin),
                LinearVelocity(*request.direction * PROJECTILE_SPEED),
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: Color::srgb(0.9, 0.24, 0.06),
                    metallic: 0.75,
                    perceptual_roughness: 0.25,
                    ..default()
                })),
            ))
            .insert((
                Friction::new(0.4),
                Restitution::new(0.1),
                TransformInterpolation,
                SweptCcd {
                    filter: CcdFilter::ALL,
                    ..default()
                },
            ));
        count += 1;
        stats.launched += 1;
    }
}

pub(super) fn retire_projectiles(
    mut commands: Commands,
    time: Res<Time<Fixed>>,
    mut projectiles: Query<(Entity, &mut RigidProjectile, &Position)>,
) {
    for (entity, mut projectile, position) in &mut projectiles {
        projectile.age += time.delta_secs();
        if projectile.age > 20.0 || position.y < -20.0 {
            commands.entity(entity).despawn();
        }
    }
}
