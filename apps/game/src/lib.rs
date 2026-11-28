#[cfg(feature = "acceptance-test")]
#[path = "../../../src/acceptance/mod.rs"]
pub mod acceptance;
#[path = "../../../src/asset/mod.rs"]
mod asset;
#[path = "../../../src/camera/mod.rs"]
mod camera;
#[path = "../../../src/catalog/mod.rs"]
mod catalog;
#[cfg(any(test, feature = "acceptance-test"))]
#[cfg_attr(all(test, not(feature = "acceptance-test")), allow(dead_code))]
#[path = "../../../src/collapse_smoke/mod.rs"]
mod collapse_smoke;
#[path = "../../../src/contact_mask/mod.rs"]
mod contact_mask;
#[cfg(any(test, feature = "acceptance-test"))]
#[cfg_attr(all(test, not(feature = "acceptance-test")), allow(dead_code))]
#[path = "../../../src/contact_smoke/mod.rs"]
mod contact_smoke;
#[path = "../../../src/debug/mod.rs"]
mod debug;
#[cfg(any(test, feature = "acceptance-test"))]
#[cfg_attr(all(test, not(feature = "acceptance-test")), allow(dead_code))]
#[path = "../../../src/diagnostics/mod.rs"]
mod diagnostics;
#[path = "../../../src/dust_material/mod.rs"]
mod dust_material;
#[path = "../../../src/force_debug/mod.rs"]
mod force_debug;
#[path = "../../../src/force_samples/mod.rs"]
mod force_samples;
#[cfg(any(test, feature = "acceptance-test"))]
#[cfg_attr(all(test, not(feature = "acceptance-test")), allow(dead_code))]
#[path = "../../../src/force_smoke/mod.rs"]
mod force_smoke;
#[path = "../../../src/force_visibility/mod.rs"]
mod force_visibility;
#[path = "../../../src/fracture_detail/mod.rs"]
mod fracture_detail;
#[path = "../../../src/interaction/mod.rs"]
mod interaction;
#[cfg(feature = "acceptance-test")]
#[path = "../../../src/landing_smoke/mod.rs"]
mod landing_smoke;
#[path = "../../../src/material/mod.rs"]
mod material;
#[cfg(feature = "acceptance-test")]
#[path = "../../../src/material_smoke/mod.rs"]
mod material_smoke;
#[path = "../../../src/physics/mod.rs"]
mod physics;
#[cfg(feature = "acceptance-test")]
#[path = "../../../src/physics_smoke/mod.rs"]
mod physics_smoke;
#[path = "../../../src/projectiles/mod.rs"]
mod projectiles;
#[path = "../../../src/rim_profile/mod.rs"]
mod rim_profile;
#[path = "../../../src/scene/mod.rs"]
mod scene;
#[cfg(feature = "acceptance-test")]
#[path = "../../../src/smoke/mod.rs"]
mod smoke;
#[path = "../../../src/stress/mod.rs"]
mod stress;
#[path = "../../../src/stress_runtime/mod.rs"]
mod stress_runtime;
#[cfg(any(test, feature = "acceptance-test"))]
#[cfg_attr(all(test, not(feature = "acceptance-test")), allow(dead_code))]
#[path = "../../../src/stress_smoke/mod.rs"]
mod stress_smoke;
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
mod tests;

const DESKTOP_SETTINGS_APP_NAME: &str = "io.github.fedoraiver.bevy-destruction";

pub const DEFAULT_VIEWPORT: UVec2 = UVec2::new(1920, 1080);

#[derive(Reflect, Resource, SettingsGroup)]
#[reflect(Resource, Default, SettingsGroup)]
#[settings_group(group = "demo_options")]
pub struct DemoOptions {
    asset: String,
    stress: bool,
    vfx: bool,
    detail: bool,
    soft_particles: bool,
}

impl Default for DemoOptions {
    fn default() -> Self {
        Self {
            asset: "buildings/rectangle.destruction.json".into(),
            stress: true,
            vfx: true,
            detail: true,
            soft_particles: true,
        }
    }
}

#[derive(Default, Resource)]
struct ProbeOptions {
    #[cfg(any(test, feature = "acceptance-test"))]
    smoke_output: Option<PathBuf>,
    #[cfg(any(test, feature = "acceptance-test"))]
    physics_smoke_output: Option<PathBuf>,
    #[cfg(any(test, feature = "acceptance-test"))]
    stress_smoke_output: Option<PathBuf>,
    #[cfg(any(test, feature = "acceptance-test"))]
    material_smoke_output: Option<PathBuf>,
    #[cfg(any(test, feature = "acceptance-test"))]
    contact_smoke_output: Option<PathBuf>,
    #[cfg(any(test, feature = "acceptance-test"))]
    force_smoke_output: Option<PathBuf>,
    #[cfg(any(test, feature = "acceptance-test"))]
    landing_smoke_output: Option<PathBuf>,
    #[cfg(any(test, feature = "acceptance-test"))]
    collapse_smoke_output: Option<PathBuf>,
    #[cfg(feature = "acceptance-test")]
    capture_replay: bool,
    #[cfg(any(test, feature = "acceptance-test"))]
    smoke_target: Option<Handle<Image>>,
    #[cfg(any(test, feature = "acceptance-test"))]
    profile: bool,
}

impl ProbeOptions {
    pub fn scripted(&self) -> bool {
        #[cfg(any(test, feature = "acceptance-test"))]
        {
            self.smoke_output.is_some()
                || self.physics_smoke_output.is_some()
                || self.stress_smoke_output.is_some()
                || self.material_smoke_output.is_some()
                || self.contact_smoke_output.is_some()
                || self.force_smoke_output.is_some()
                || self.landing_smoke_output.is_some()
                || self.collapse_smoke_output.is_some()
        }
        #[cfg(not(any(test, feature = "acceptance-test")))]
        false
    }

    fn force_probe(&self) -> bool {
        #[cfg(any(test, feature = "acceptance-test"))]
        {
            self.force_smoke_output.is_some()
        }
        #[cfg(not(any(test, feature = "acceptance-test")))]
        false
    }

    #[cfg(any(test, feature = "acceptance-test"))]
    fn smoke_screenshot(&self) -> bevy::render::view::screenshot::Screenshot {
        bevy::render::view::screenshot::Screenshot::image(
            self.smoke_target
                .clone()
                .expect("Smoke probe requires an offscreen target"),
        )
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
    prepare_app(Some(DESKTOP_SETTINGS_APP_NAME)).run()
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
    app.register_type::<DemoOptions>()
        .add_plugins(SettingsPlugin::new(app_name));
}

pub struct DestructionPlugin;

impl Plugin for DestructionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DemoOptions>()
            .init_resource::<ProbeOptions>();
        let options = app.world().resource::<DemoOptions>();
        let vfx_settings = vfx::VfxSettings {
            enabled: options.vfx,
            ..default()
        };
        let stress_settings = stress_runtime::StressSettings {
            enabled: options.stress,
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
                catalog::CatalogPlugin,
            ))
            .add_systems(Update, close_on_escape);
    }
}

fn close_on_escape(keys: Res<ButtonInput<KeyCode>>, mut exit: MessageWriter<AppExit>) {
    if keys.just_pressed(KeyCode::Escape) {
        exit.write(AppExit::Success);
    }
}
