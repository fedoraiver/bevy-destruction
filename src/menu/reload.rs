use std::{fs, io::ErrorKind};

use bevy::{
    core_pipeline::prepass::DepthPrepass, prelude::*, settings::SettingsPlugin, time::Virtual,
};

use crate::{
    camera::{CameraControl, FreeCamera},
    projectiles::{ResetProjectiles, RigidProjectile},
    scene::{BuildingInstance, DemoCamera},
    settings::{DemoSettings, SETTINGS_APP_NAME},
    stress_runtime::StressSettings,
    vfx::{Particle, VfxSettings},
};

use super::{PauseMenu, input::resume};

#[derive(Component)]
pub(crate) struct PendingReload {
    pub settings: DemoSettings,
}

fn read_reload_settings() -> Result<DemoSettings, String> {
    if let Some(directory) = bevy::platform::dirs::preferences_dir() {
        let path = directory.join(SETTINGS_APP_NAME).join("settings.toml");
        match fs::read_to_string(path) {
            Ok(source) => {
                // SettingsPlugin uses defaults on a parse error. Keep the
                // running scene when the user's edited file is invalid.
                source
                    .parse::<toml::Table>()
                    .map_err(|error| format!("Invalid settings.toml: {error}"))?;
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Could not read settings.toml: {error}")),
        }
    }

    let mut reader = App::new();
    reader
        .register_type::<DemoSettings>()
        .add_plugins(SettingsPlugin::new(SETTINGS_APP_NAME));
    Ok(reader
        .world_mut()
        .remove_resource::<DemoSettings>()
        .unwrap_or_default())
}

pub(super) fn begin_reload(
    commands: &mut Commands,
    server: &AssetServer,
    root: Entity,
    menu: &mut PauseMenu,
) {
    let settings = match read_reload_settings() {
        Ok(settings) => settings,
        Err(error) => {
            menu.status = error;
            return;
        }
    };
    let path = match settings.scene_path() {
        Ok(path) => path,
        Err(error) => {
            menu.status = error.into();
            return;
        }
    };
    let asset = server.load(path.to_owned());
    menu.candidate = Some(
        commands
            .spawn((
                Name::new("BuildingRoot"),
                Transform::IDENTITY,
                Visibility::Hidden,
                BuildingInstance::pending(asset),
                ChildOf(root),
                PendingReload { settings },
            ))
            .id(),
    );
    menu.status = "Loading scene…".into();
}

#[allow(clippy::too_many_arguments)]
pub(super) fn complete_reload(
    mut commands: Commands,
    mut menu: ResMut<PauseMenu>,
    mut time: ResMut<Time<Virtual>>,
    mut control: ResMut<CameraControl>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut settings: ResMut<DemoSettings>,
    mut vfx: ResMut<VfxSettings>,
    mut stress: ResMut<StressSettings>,
    candidates: Query<(&BuildingInstance, &PendingReload)>,
    previous: Query<Entity, (With<BuildingInstance>, Without<PendingReload>)>,
    projectiles: Query<Entity, With<RigidProjectile>>,
    mut cameras: Query<(Entity, &mut FreeCamera), With<DemoCamera>>,
    mut particles: Query<&mut Particle>,
) {
    let Some(candidate) = menu.candidate else {
        return;
    };
    let Ok((building, pending)) = candidates.get(candidate) else {
        return;
    };
    if let Some(error) = &building.error {
        menu.status.clone_from(error);
        menu.candidate = None;
        commands.entity(candidate).despawn();
        return;
    }
    if building.bounds.is_none() {
        return;
    }

    for entity in previous.iter().chain(projectiles.iter()) {
        commands.entity(entity).despawn();
    }
    for mut particle in &mut particles {
        particle.age = particle.lifetime;
    }
    commands.trigger(ResetProjectiles);

    *settings = pending.settings.clone();
    vfx.enabled = settings.vfx;
    stress.enabled = settings.stress;
    for (entity, mut camera) in &mut cameras {
        *camera = FreeCamera::default();
        if settings.soft_particles {
            commands.entity(entity).insert(DepthPrepass);
        } else {
            commands.entity(entity).remove::<DepthPrepass>();
        }
    }
    commands
        .entity(candidate)
        .remove::<PendingReload>()
        .insert(Visibility::Visible);
    menu.candidate = None;
    resume(&mut menu, &mut time, &mut control, &mut mouse);
}
