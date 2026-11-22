mod asset;
mod camera;
mod catalog;
mod collapse_smoke;
mod contact_mask;
mod contact_smoke;
mod debug;
mod diagnostics;
mod dust_material;
mod force_debug;
mod force_samples;
mod force_smoke;
mod force_visibility;
mod fracture_detail;
mod interaction;
mod landing_smoke;
mod material;
mod material_smoke;
mod physics;
mod physics_smoke;
mod projectiles;
mod rim_profile;
mod scene;
mod smoke;
mod stress;
mod stress_runtime;
mod stress_smoke;
mod structure;
mod vfx;

#[cfg(feature = "debugdump")]
use std::path::PathBuf;

use avian3d::prelude::*;
use bevy::prelude::*;

#[derive(Resource)]
pub struct DemoOptions {
    pub asset: String,
    pub smoke_output: Option<std::path::PathBuf>,
    pub physics_smoke_output: Option<std::path::PathBuf>,
    pub stress_smoke_output: Option<std::path::PathBuf>,
    pub material_smoke_output: Option<std::path::PathBuf>,
    pub contact_smoke_output: Option<std::path::PathBuf>,
    pub force_smoke_output: Option<std::path::PathBuf>,
    pub landing_smoke_output: Option<std::path::PathBuf>,
    pub collapse_smoke_output: Option<std::path::PathBuf>,
    pub no_vfx: bool,
    pub no_stress: bool,
    pub no_detail: bool,
    pub no_soft_particles: bool,
    pub capture_replay: bool,
    pub profile: bool,
}

impl Default for DemoOptions {
    fn default() -> Self {
        Self {
            asset: "buildings/rectangle.destruction.json".into(),
            smoke_output: None,
            physics_smoke_output: None,
            stress_smoke_output: None,
            material_smoke_output: None,
            contact_smoke_output: None,
            force_smoke_output: None,
            landing_smoke_output: None,
            collapse_smoke_output: None,
            no_vfx: false,
            no_stress: false,
            no_detail: false,
            no_soft_particles: false,
            capture_replay: false,
            profile: false,
        }
    }
}

impl DemoOptions {
    pub fn scripted(&self) -> bool {
        self.smoke_output.is_some()
            || self.physics_smoke_output.is_some()
            || self.stress_smoke_output.is_some()
            || self.material_smoke_output.is_some()
            || self.contact_smoke_output.is_some()
            || self.force_smoke_output.is_some()
            || self.landing_smoke_output.is_some()
            || self.collapse_smoke_output.is_some()
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
            "--smoke-test" => {
                options.smoke_output = Some(
                    args.next()
                        .ok_or("--smoke-test requires an output directory")?
                        .into(),
                )
            }
            "--physics-smoke-test" => {
                options.physics_smoke_output = Some(
                    args.next()
                        .ok_or("--physics-smoke-test requires an output directory")?
                        .into(),
                )
            }
            "--stress-smoke-test" => {
                options.stress_smoke_output = Some(
                    args.next()
                        .ok_or("--stress-smoke-test requires an output directory")?
                        .into(),
                )
            }
            "--material-smoke-test" => {
                options.material_smoke_output = Some(
                    args.next()
                        .ok_or("--material-smoke-test requires an output directory")?
                        .into(),
                );
            }
            "--force-smoke-test" => {
                options.force_smoke_output = Some(
                    args.next()
                        .ok_or("--force-smoke-test requires an output directory")?
                        .into(),
                );
            }
            "--contact-smoke-test" => {
                options.contact_smoke_output = Some(
                    args.next()
                        .ok_or("--contact-smoke-test requires an output directory")?
                        .into(),
                );
            }
            "--landing-smoke-test" => {
                options.landing_smoke_output = Some(
                    args.next()
                        .ok_or("--landing-smoke-test requires an output directory")?
                        .into(),
                );
            }
            "--collapse-smoke-test" => {
                options.collapse_smoke_output = Some(
                    args.next()
                        .ok_or("--collapse-smoke-test requires an output directory")?
                        .into(),
                );
            }
            "--help" | "-h" => {
                println!(
                    "bevy-destruction [--scene rectangle|highrise_slab|highrise_l|bridge|courtyard|l_shape] [--asset path]\nArchitecture: --dump-schedules DIR (requires the debugdump feature)\nValidation: --smoke-test DIR | --physics-smoke-test DIR | --stress-smoke-test DIR | --material-smoke-test DIR | --contact-smoke-test DIR | --landing-smoke-test DIR | --collapse-smoke-test DIR | --force-smoke-test DIR\nOptions: --no-stress --no-vfx --no-detail --no-soft-particles --capture-replay --profile\nWASD: fly | Mouse: look | Space/Ctrl: up/down | Tab: cursor\nLeft: break | Right: rigid ball | R: rigid cube | F1-F6: scenes | 0: edges | 9: load heat | Alt: inspect loads | Esc: exit"
                );
                std::process::exit(0);
            }
            "--no-vfx" => options.no_vfx = true,
            "--no-stress" => options.no_stress = true,
            "--no-detail" => options.no_detail = true,
            "--no-soft-particles" => options.no_soft_particles = true,
            "--capture-replay" => options.capture_replay = true,
            "--profile" => options.profile = true,
            _ => return Err(format!("Unknown option: {arg}")),
        }
    }
    if [
        options.smoke_output.is_some(),
        options.physics_smoke_output.is_some(),
        options.stress_smoke_output.is_some(),
        options.material_smoke_output.is_some(),
        options.contact_smoke_output.is_some(),
        options.force_smoke_output.is_some(),
        options.landing_smoke_output.is_some(),
        options.collapse_smoke_output.is_some(),
    ]
    .into_iter()
    .filter(|enabled| *enabled)
    .count()
        > 1
    {
        return Err("Choose one smoke-test mode".into());
    }
    Ok(options)
}

fn main() -> AppExit {
    let mut args = std::env::args().skip(1).peekable();
    if args.peek().is_some_and(|arg| arg == "--dump-schedules") {
        return export_schedules(args.skip(1));
    }
    let options = options(args).unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(2);
    });
    build_app(options).run()
}

fn build_app(options: DemoOptions) -> App {
    let smoke_test = options.scripted();
    let profile = options.profile;
    let vfx_settings = vfx::VfxSettings {
        enabled: !options.no_vfx,
        ..default()
    };
    let stress_settings = stress_runtime::StressSettings {
        enabled: !options.no_stress,
        ..default()
    };
    let mut app = App::new();
    app.insert_resource(options)
        .insert_resource(vfx_settings)
        .insert_resource(stress_settings)
        .insert_resource(ClearColor(Color::srgb(0.075, 0.09, 0.12)))
        .insert_resource(Time::<Fixed>::from_hz(60.0))
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    file_path: format!("{}/assets", env!("CARGO_MANIFEST_DIR")),
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Bevy Destruction · Physical fracture".into(),
                        resolution: (1280, 900).into(),
                        present_mode: if profile {
                            bevy::window::PresentMode::AutoNoVsync
                        } else {
                            default()
                        },
                        ..default()
                    }),
                    ..default()
                }),
        )
        .add_plugins((
            PhysicsPlugins::default(),
            asset::DestructibleAssetPlugin,
            material::BuildingMaterialPlugin,
            dust_material::DustMaterialPlugin,
            diagnostics::MetricsPlugin,
            debug::FractureDebugPlugin,
            force_debug::ForceDebugPlugin,
            scene::DemoScenePlugin,
            physics::DestructionPhysicsPlugin,
            camera::FreeCameraPlugin,
            interaction::InteractionPlugin,
            vfx::VfxPlugin,
            smoke::SmokePlugin,
            physics_smoke::PhysicsSmokePlugin,
        ))
        .add_plugins((
            stress_runtime::StressRuntimePlugin,
            projectiles::ProjectilePlugin,
            stress_smoke::StressSmokePlugin,
            catalog::CatalogPlugin,
            material_smoke::MaterialSmokePlugin,
            contact_smoke::ContactSmokePlugin,
            landing_smoke::LandingSmokePlugin,
            collapse_smoke::CollapseSmokePlugin,
            force_smoke::ForceSmokePlugin,
        ))
        .add_systems(Update, close_on_escape);
    if smoke_test {
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f64(1.0 / 60.0),
        ));
    }
    if profile {
        app.insert_resource(bevy::winit::WinitSettings::continuous());
        app.init_resource::<bevy::diagnostic::DiagnosticsStore>()
            .add_plugins(bevy::render::diagnostic::RenderDiagnosticsPlugin);
    }
    app
}

#[cfg(feature = "debugdump")]
fn export_schedules(mut args: impl Iterator<Item = String>) -> AppExit {
    let Some(directory) = args.next() else {
        eprintln!("--dump-schedules requires an output directory");
        return AppExit::error();
    };
    if directory.starts_with('-') || args.next().is_some() {
        eprintln!("Usage: bevy-destruction --dump-schedules DIR");
        return AppExit::error();
    }
    let mut app = build_app(DemoOptions::default());
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    match bevy_schedule_export::dump_schedules(&mut app, &PathBuf::from(directory), root) {
        Ok(_) => AppExit::Success,
        Err(error) => {
            eprintln!("Schedule export failed: {error}");
            AppExit::error()
        }
    }
}

#[cfg(not(feature = "debugdump"))]
fn export_schedules(_: impl Iterator<Item = String>) -> AppExit {
    eprintln!("--dump-schedules requires the debugdump feature");
    AppExit::error()
}

fn close_on_escape(keys: Res<ButtonInput<KeyCode>>, mut exit: MessageWriter<AppExit>) {
    if keys.just_pressed(KeyCode::Escape) {
        exit.write(AppExit::Success);
    }
}
