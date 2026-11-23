#[cfg(feature = "acceptance-test")]
pub mod acceptance;
#[cfg(ecs_architecture)]
#[path = "../tools/ecs-architecture/src/assembly.rs"]
mod architecture;
mod asset;
mod camera;
mod catalog;
#[cfg(any(test, feature = "acceptance-test"))]
#[cfg_attr(all(test, not(feature = "acceptance-test")), allow(dead_code))]
mod collapse_smoke;
mod contact_mask;
#[cfg(any(test, feature = "acceptance-test"))]
#[cfg_attr(all(test, not(feature = "acceptance-test")), allow(dead_code))]
mod contact_smoke;
mod debug;
#[cfg(any(test, feature = "acceptance-test"))]
#[cfg_attr(all(test, not(feature = "acceptance-test")), allow(dead_code))]
mod diagnostics;
mod dust_material;
#[cfg(ecs_architecture)]
pub use architecture::build_app;
mod force_debug;
mod force_samples;
#[cfg(any(test, feature = "acceptance-test"))]
#[cfg_attr(all(test, not(feature = "acceptance-test")), allow(dead_code))]
mod force_smoke;
mod force_visibility;
mod fracture_detail;
mod interaction;
#[cfg(feature = "acceptance-test")]
mod landing_smoke;
mod material;
#[cfg(feature = "acceptance-test")]
mod material_smoke;
mod physics;
#[cfg(feature = "acceptance-test")]
mod physics_smoke;
mod projectiles;
mod rim_profile;
mod scene;
#[cfg(feature = "acceptance-test")]
mod smoke;
mod stress;
mod stress_runtime;
#[cfg(any(test, feature = "acceptance-test"))]
#[cfg_attr(all(test, not(feature = "acceptance-test")), allow(dead_code))]
mod stress_smoke;
mod structure;
mod vfx;

use std::path::{Path, PathBuf};

use avian3d::prelude::*;
use bevy::prelude::*;

pub const DEFAULT_VIEWPORT: UVec2 = UVec2::new(1920, 1080);

#[derive(Resource)]
pub struct DemoOptions {
    asset: String,
    #[cfg(any(test, feature = "acceptance-test"))]
    smoke_output: Option<std::path::PathBuf>,
    #[cfg(any(test, feature = "acceptance-test"))]
    physics_smoke_output: Option<std::path::PathBuf>,
    #[cfg(any(test, feature = "acceptance-test"))]
    stress_smoke_output: Option<std::path::PathBuf>,
    #[cfg(any(test, feature = "acceptance-test"))]
    material_smoke_output: Option<std::path::PathBuf>,
    #[cfg(any(test, feature = "acceptance-test"))]
    contact_smoke_output: Option<std::path::PathBuf>,
    #[cfg(any(test, feature = "acceptance-test"))]
    force_smoke_output: Option<std::path::PathBuf>,
    #[cfg(any(test, feature = "acceptance-test"))]
    landing_smoke_output: Option<std::path::PathBuf>,
    #[cfg(any(test, feature = "acceptance-test"))]
    collapse_smoke_output: Option<std::path::PathBuf>,
    no_vfx: bool,
    no_stress: bool,
    no_detail: bool,
    no_soft_particles: bool,
    #[cfg(feature = "acceptance-test")]
    capture_replay: bool,
    #[cfg(any(test, feature = "acceptance-test"))]
    smoke_target: Option<Handle<Image>>,
    #[cfg(any(test, feature = "acceptance-test"))]
    profile: bool,
}

impl Default for DemoOptions {
    fn default() -> Self {
        Self {
            asset: "buildings/rectangle.destruction.json".into(),
            #[cfg(any(test, feature = "acceptance-test"))]
            smoke_output: None,
            #[cfg(any(test, feature = "acceptance-test"))]
            physics_smoke_output: None,
            #[cfg(any(test, feature = "acceptance-test"))]
            stress_smoke_output: None,
            #[cfg(any(test, feature = "acceptance-test"))]
            material_smoke_output: None,
            #[cfg(any(test, feature = "acceptance-test"))]
            contact_smoke_output: None,
            #[cfg(any(test, feature = "acceptance-test"))]
            force_smoke_output: None,
            #[cfg(any(test, feature = "acceptance-test"))]
            landing_smoke_output: None,
            #[cfg(any(test, feature = "acceptance-test"))]
            collapse_smoke_output: None,
            no_vfx: false,
            no_stress: false,
            no_detail: false,
            no_soft_particles: false,
            #[cfg(feature = "acceptance-test")]
            capture_replay: false,
            #[cfg(any(test, feature = "acceptance-test"))]
            smoke_target: None,
            #[cfg(any(test, feature = "acceptance-test"))]
            profile: false,
        }
    }
}

impl DemoOptions {
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

pub fn parse_options(mut args: impl Iterator<Item = String>) -> Result<DemoOptions, String> {
    let mut options = DemoOptions::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--asset" => options.asset = args.next().ok_or("--asset requires an asset path")?,
            "--scene" => {
                let name = args.next().ok_or("--scene requires a scene name")?;
                options.asset = catalog::SCENES
                    .iter()
                    .find(|scene| scene.0 == name)
                    .ok_or_else(|| format!("Unknown scene: {name}"))?
                    .1
                    .into();
            }
            "--help" | "-h" => {
                println!(
                    "bevy-destruction [--scene rectangle|highrise_slab|highrise_l|bridge|courtyard|l_shape] [--asset path]\nOptions: --no-stress --no-vfx --no-detail --no-soft-particles\nWASD: fly | Mouse: look | Space/Ctrl: up/down | Tab: cursor\nLeft: break | Right: rigid ball | R: rigid cube | F1-F6: scenes | 0: edges | 9: load heat | Alt: inspect loads | Esc: exit"
                );
                std::process::exit(0);
            }
            "--no-vfx" => options.no_vfx = true,
            "--no-stress" => options.no_stress = true,
            "--no-detail" => options.no_detail = true,
            "--no-soft-particles" => options.no_soft_particles = true,
            _ => return Err(format!("Unknown option: {arg}")),
        }
    }
    Ok(options)
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

pub struct DestructionPlugin;

impl Plugin for DestructionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DemoOptions>();
        let options = app.world().resource::<DemoOptions>();
        let vfx_settings = vfx::VfxSettings {
            enabled: !options.no_vfx,
            ..default()
        };
        let stress_settings = stress_runtime::StressSettings {
            enabled: !options.no_stress,
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
