#[path = "../../../src/asset/mod.rs"]
mod asset;
#[path = "../../../src/camera/mod.rs"]
mod camera;
#[cfg(test)]
#[path = "../../../src/catalog/mod.rs"]
mod catalog;
#[path = "../../../src/contact_mask/mod.rs"]
mod contact_mask;
#[path = "../../../src/debug/mod.rs"]
mod debug;
#[path = "../../../src/dust_material/mod.rs"]
mod dust_material;
#[path = "../../../src/force_debug/mod.rs"]
mod force_debug;
#[path = "../../../src/force_samples/mod.rs"]
mod force_samples;
#[path = "../../../src/force_visibility/mod.rs"]
mod force_visibility;
#[path = "../../../src/fracture_detail/mod.rs"]
mod fracture_detail;
#[path = "../../../src/interaction/mod.rs"]
mod interaction;
#[path = "../../../src/material/mod.rs"]
mod material;
#[path = "../../../src/menu/mod.rs"]
mod menu;
#[path = "../../../src/physics/mod.rs"]
mod physics;
#[path = "../../../src/projectiles/mod.rs"]
mod projectiles;
#[path = "../../../src/rim_profile/mod.rs"]
mod rim_profile;
#[path = "../../../src/scene/mod.rs"]
mod scene;
#[path = "../../../src/stress/mod.rs"]
mod stress;
#[path = "../../../src/stress_runtime/mod.rs"]
mod stress_runtime;
#[path = "../../../src/structure/mod.rs"]
mod structure;
#[path = "../../../src/vfx/mod.rs"]
mod vfx;

use std::path::{Path, PathBuf};

use bevy::{
    prelude::*,
    settings::{ReflectSettingsGroup, SettingsGroup},
};

#[cfg(test)]
#[path = "../../../src/test_fixtures.rs"]
mod test_fixtures;

#[cfg(feature = "acceptance-test")]
pub use {
    asset::DestructibleAssetPlugin,
    avian3d::prelude::PhysicsPlugins,
    camera::FreeCameraPlugin,
    debug::FractureDebugPlugin,
    dust_material::DustMaterialPlugin,
    force_debug::ForceDebugPlugin,
    interaction::InteractionPlugin,
    material::BuildingMaterialPlugin,
    menu::PauseMenuPlugin,
    physics::DestructionPhysicsPlugin,
    projectiles::ProjectilePlugin,
    scene::{DemoCamera, DemoScenePlugin},
    stress_runtime::StressRuntimePlugin,
    vfx::VfxPlugin,
};
pub use {stress_runtime::StressSettings, vfx::VfxSettings};

pub const SETTINGS_APP_NAME: &str = "io.github.fedoraiver.bevy-destruction";

pub const DEFAULT_VIEWPORT: UVec2 = UVec2::new(1920, 1080);

#[derive(Clone, Reflect, Resource, SettingsGroup)]
#[reflect(Resource, Default, SettingsGroup)]
#[settings_group(group = "demo_settings")]
pub struct DemoSettings {
    scene: String,
    pub stress: bool,
    pub vfx: bool,
    detail: bool,
    soft_particles: bool,
}

impl Default for DemoSettings {
    fn default() -> Self {
        Self {
            scene: "buildings/rectangle.destruction.json".into(),
            stress: true,
            vfx: true,
            detail: true,
            soft_particles: true,
        }
    }
}

impl DemoSettings {
    fn scene_path(&self) -> Result<&str, &'static str> {
        use std::path::Component;

        if !self.scene.ends_with(".destruction.json")
            || self.scene.contains([':', '#'])
            || !Path::new(&self.scene)
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
        {
            return Err("Scene must be a .destruction.json path relative to assets/.");
        }
        Ok(&self.scene)
    }
}

fn asset_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("Package must be two levels below the workspace root")
        .join("assets")
}

/// 按默认设置装配桌面应用，供桌面入口和架构导出器使用。
pub fn build_app() -> App {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(AssetPlugin {
                file_path: asset_root().to_string_lossy().into_owned(),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Bevy Destruction · Physical fracture".into(),
                    resolution: (DEFAULT_VIEWPORT.x, DEFAULT_VIEWPORT.y).into(),
                    ..default()
                }),
                ..default()
            }),
    )
    .init_resource::<DemoSettings>()
    .insert_resource(ClearColor(Color::srgb(0.075, 0.09, 0.12)))
    .insert_resource(Time::<Fixed>::from_hz(60.0))
    .add_plugins((
        avian3d::prelude::PhysicsPlugins::default(),
        asset::DestructibleAssetPlugin,
        material::BuildingMaterialPlugin,
        dust_material::DustMaterialPlugin,
        debug::FractureDebugPlugin,
        force_debug::ForceDebugPlugin,
        scene::DemoScenePlugin,
        physics::DestructionPhysicsPlugin,
        camera::FreeCameraPlugin,
        interaction::InteractionPlugin,
        vfx::VfxPlugin,
        stress_runtime::StressRuntimePlugin,
        projectiles::ProjectilePlugin,
        menu::PauseMenuPlugin,
    ));
    app
}
