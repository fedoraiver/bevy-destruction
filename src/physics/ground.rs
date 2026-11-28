use avian3d::prelude::*;
use bevy::prelude::*;

pub(super) fn ground(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Name::new("Permanent ground"),
        RigidBody::Static,
        Collider::cuboid(400.0, 0.2, 400.0),
        Mesh3d(meshes.add(Cuboid::new(400.0, 0.2, 400.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.12, 0.14, 0.16),
            perceptual_roughness: 1.0,
            ..default()
        })),
        Transform::from_xyz(0.0, -0.1, 0.0),
        Friction::new(0.7),
        Restitution::new(0.0),
    ));
}
