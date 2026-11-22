use bevy::prelude::*;

use crate::{
    DemoOptions, camera::FreeCamera, projectiles::RigidProjectile, scene::BuildingInstance,
    stress_runtime::StructuralLoads,
};

pub const SCENES: [(&str, &str); 6] = [
    ("rectangle", "buildings/rectangle.destruction.json"),
    ("highrise_slab", "buildings/highrise_slab.destruction.json"),
    ("highrise_l", "buildings/highrise_l.destruction.json"),
    ("bridge", "buildings/bridge.destruction.json"),
    ("courtyard", "buildings/courtyard.destruction.json"),
    ("l_shape", "buildings/l_shape.destruction.json"),
];

pub struct CatalogPlugin;

impl Plugin for CatalogPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            switch_scene.before(crate::scene::instantiate_pending),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn switch_scene(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut options: ResMut<DemoOptions>,
    server: Res<AssetServer>,
    instances: Query<Entity, With<BuildingInstance>>,
    projectiles: Query<Entity, With<RigidProjectile>>,
    mut cameras: Query<&mut FreeCamera>,
    mut loads: ResMut<StructuralLoads>,
    mut particles: Query<&mut crate::vfx::Particle>,
) {
    if options.scripted() {
        return;
    }
    let Some(index) = [
        KeyCode::F1,
        KeyCode::F2,
        KeyCode::F3,
        KeyCode::F4,
        KeyCode::F5,
        KeyCode::F6,
    ]
    .iter()
    .position(|key| keys.just_pressed(*key)) else {
        return;
    };
    for entity in instances.iter().chain(projectiles.iter()) {
        commands.entity(entity).despawn();
    }
    loads.by_building.clear();
    for mut particle in &mut particles {
        particle.age = particle.lifetime;
    }
    commands.queue(crate::projectiles::reset);
    for mut camera in &mut cameras {
        *camera = FreeCamera::default();
    }
    options.asset = SCENES[index].1.into();
    commands.spawn((
        Name::new(SCENES[index].0),
        Transform::IDENTITY,
        Visibility::Visible,
        BuildingInstance::pending(server.load(options.asset.clone())),
    ));
}
