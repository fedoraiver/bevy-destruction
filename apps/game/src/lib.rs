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

use avian3d::prelude::*;
use bevy::{
    prelude::*,
    settings::{ReflectSettingsGroup, SettingsGroup, SettingsPlugin},
};

#[cfg(test)]
#[path = "../../../src/test_fixtures.rs"]
mod test_fixtures;
#[cfg(test)]
mod tests;

#[cfg(feature = "acceptance-test")]
pub use scene::DemoCamera;

pub const SETTINGS_APP_NAME: &str = "io.github.fedoraiver.bevy-destruction";

pub const DEFAULT_VIEWPORT: UVec2 = UVec2::new(1920, 1080);

#[derive(Clone, Reflect, Resource, SettingsGroup)]
#[reflect(Resource, Default, SettingsGroup)]
#[settings_group(group = "demo_settings")]
pub struct DemoSettings {
    scene: String,
    stress: bool,
    vfx: bool,
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

pub fn asset_plugin() -> AssetPlugin {
    AssetPlugin {
        file_path: asset_root().to_string_lossy().into_owned(),
        ..default()
    }
}

pub fn desktop_window() -> WindowPlugin {
    WindowPlugin {
        primary_window: Some(Window {
            title: "Bevy Destruction · Physical fracture".into(),
            resolution: (DEFAULT_VIEWPORT.x, DEFAULT_VIEWPORT.y).into(),
            ..default()
        }),
        ..default()
    }
}

/// 按默认选项装配桌面应用，供架构导出器读取调度。
pub fn build_app() -> App {
    prepare_app(None)
}

/// 从用户设置文件加载演示选项并运行桌面应用。
pub fn run() -> AppExit {
    prepare_app(Some(SETTINGS_APP_NAME)).run()
}

fn prepare_app(settings_app_name: Option<&str>) -> App {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(asset_plugin()).set(desktop_window()));
    if let Some(app_name) = settings_app_name {
        load_demo_settings(&mut app, app_name);
    }
    app.add_plugins(DestructionPlugin);
    app
}

fn load_demo_settings(app: &mut App, app_name: &str) {
    app.register_type::<DemoSettings>()
        .add_plugins(SettingsPlugin::new(app_name));
}

pub struct DestructionPlugin;

impl Plugin for DestructionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DemoSettings>();
        let settings = app.world().resource::<DemoSettings>();
        let vfx_settings = vfx::VfxSettings {
            enabled: settings.vfx,
            ..default()
        };
        let stress_settings = stress_runtime::StressSettings {
            enabled: settings.stress,
            ..default()
        };
        app.insert_resource(vfx_settings)
            .insert_resource(stress_settings)
            .insert_resource(ClearColor(Color::srgb(0.075, 0.09, 0.12)))
            .insert_resource(Time::<Fixed>::from_hz(60.0))
            .add_plugins((
                PhysicsPlugins::default(),
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
    }
}
