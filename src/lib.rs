#[cfg(feature = "acceptance-test")]
pub mod acceptance;
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

use avian3d::prelude::*;
use bevy::{app::PluginGroupBuilder, prelude::*};

const DEFAULT_VIEWPORT: UVec2 = UVec2::new(1920, 1080);

#[derive(Resource)]
struct DemoOptions {
    pub asset: String,
    #[cfg(any(test, feature = "acceptance-test"))]
    pub smoke_output: Option<std::path::PathBuf>,
    #[cfg(any(test, feature = "acceptance-test"))]
    pub physics_smoke_output: Option<std::path::PathBuf>,
    #[cfg(any(test, feature = "acceptance-test"))]
    pub stress_smoke_output: Option<std::path::PathBuf>,
    #[cfg(any(test, feature = "acceptance-test"))]
    pub material_smoke_output: Option<std::path::PathBuf>,
    #[cfg(any(test, feature = "acceptance-test"))]
    pub contact_smoke_output: Option<std::path::PathBuf>,
    #[cfg(any(test, feature = "acceptance-test"))]
    pub force_smoke_output: Option<std::path::PathBuf>,
    #[cfg(any(test, feature = "acceptance-test"))]
    pub landing_smoke_output: Option<std::path::PathBuf>,
    #[cfg(any(test, feature = "acceptance-test"))]
    pub collapse_smoke_output: Option<std::path::PathBuf>,
    pub no_vfx: bool,
    pub no_stress: bool,
    pub no_detail: bool,
    pub no_soft_particles: bool,
    #[cfg(feature = "acceptance-test")]
    pub capture_replay: bool,
    #[cfg(any(test, feature = "acceptance-test"))]
    smoke_target: Option<Handle<Image>>,
    #[cfg(any(test, feature = "acceptance-test"))]
    pub profile: bool,
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

fn options(mut args: impl Iterator<Item = String>) -> Result<DemoOptions, String> {
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

pub fn run(args: impl Iterator<Item = String>) -> AppExit {
    let options = options(args).unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(2);
    });
    build_app_with_options(options).run()
}

pub fn build_app() -> App {
    build_app_with_options(DemoOptions::default())
}

fn build_app_with_options(options: DemoOptions) -> App {
    let mut app = App::new();
    app.add_plugins(engine_plugins().set(WindowPlugin {
        primary_window: Some(Window {
            title: "Bevy Destruction · Physical fracture".into(),
            resolution: (DEFAULT_VIEWPORT.x, DEFAULT_VIEWPORT.y).into(),
            ..default()
        }),
        ..default()
    }));
    install_game(&mut app, options);
    app
}

fn engine_plugins() -> PluginGroupBuilder {
    DefaultPlugins.build().set(AssetPlugin {
        file_path: format!("{}/assets", env!("CARGO_MANIFEST_DIR")),
        ..default()
    })
}

fn install_game(app: &mut App, options: DemoOptions) {
    let vfx_settings = vfx::VfxSettings {
        enabled: !options.no_vfx,
        ..default()
    };
    let stress_settings = stress_runtime::StressSettings {
        enabled: !options.no_stress,
        ..default()
    };
    app.insert_resource(options)
        .insert_resource(vfx_settings)
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

fn close_on_escape(keys: Res<ButtonInput<KeyCode>>, mut exit: MessageWriter<AppExit>) {
    if keys.just_pressed(KeyCode::Escape) {
        exit.write(AppExit::Success);
    }
}
