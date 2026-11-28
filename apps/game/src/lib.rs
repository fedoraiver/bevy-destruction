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
#[path = "../../../src/settings.rs"]
mod settings;
#[path = "../../../src/stress/mod.rs"]
mod stress;
#[path = "../../../src/stress_runtime/mod.rs"]
mod stress_runtime;
#[path = "../../../src/structure/mod.rs"]
mod structure;
#[path = "../../../src/vfx/mod.rs"]
mod vfx;

use std::path::{Path, PathBuf};

use bevy::prelude::*;

#[cfg(test)]
#[path = "../../../src/test_fixtures.rs"]
mod test_fixtures;

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
    settings::{DemoSettings, SETTINGS_APP_NAME},
    stress_runtime::{StressRuntimePlugin, StressSettings},
    vfx::{VfxPlugin, VfxSettings},
};

pub const DEFAULT_VIEWPORT: UVec2 = UVec2::new(1920, 1080);

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
        PhysicsPlugins::default(),
        DestructibleAssetPlugin,
        BuildingMaterialPlugin,
        DustMaterialPlugin,
        FractureDebugPlugin,
        ForceDebugPlugin,
        DemoScenePlugin,
        DestructionPhysicsPlugin,
        FreeCameraPlugin,
        InteractionPlugin,
        VfxPlugin,
        StressRuntimePlugin,
        ProjectilePlugin,
        PauseMenuPlugin,
    ));
    app
}
