mod asset;
mod camera;
mod debug;
mod diagnostics;
mod dust_material;
mod fracture_detail;
mod interaction;
mod material;
mod rim_profile;
mod scene;
mod smoke;
mod vfx;

use avian3d::prelude::*;
use bevy::prelude::*;

#[derive(Resource)]
pub struct DemoOptions {
    pub asset: String,
    pub smoke_output: Option<std::path::PathBuf>,
    pub no_vfx: bool,
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
            no_vfx: false,
            no_detail: false,
            no_soft_particles: false,
            capture_replay: false,
            profile: false,
        }
    }
}

fn options() -> Result<DemoOptions, String> {
    let mut options = DemoOptions::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--asset" => options.asset = args.next().ok_or("--asset requires an asset path")?,
            "--smoke-test" => {
                options.smoke_output = Some(
                    args.next()
                        .ok_or("--smoke-test requires an output directory")?
                        .into(),
                )
            }
            "--help" | "-h" => {
                println!(
                    "bevy-destruction [--asset buildings/rectangle.destruction.json] [--smoke-test output/demo]\nValidation: --no-vfx --no-detail --no-soft-particles --capture-replay --profile\nWASD: fly | Mouse: look | Space/Ctrl: up/down | Tab: cursor | Left click: break | 0: edges | Esc: exit"
                );
                std::process::exit(0);
            }
            "--no-vfx" => options.no_vfx = true,
            "--no-detail" => options.no_detail = true,
            "--no-soft-particles" => options.no_soft_particles = true,
            "--capture-replay" => options.capture_replay = true,
            "--profile" => options.profile = true,
            _ => return Err(format!("Unknown option: {arg}")),
        }
    }
    Ok(options)
}

fn main() -> AppExit {
    let options = options().unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(2);
    });
    let smoke_test = options.smoke_output.is_some();
    let profile = options.profile;
    let vfx_settings = vfx::VfxSettings {
        enabled: !options.no_vfx,
        ..default()
    };
    let mut app = App::new();
    app.insert_resource(options)
        .insert_resource(vfx_settings)
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
                        title: "Bevy Destruction · ADR 0007".into(),
                        resolution: (1280, 900).into(),
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
            scene::DemoScenePlugin,
            camera::FreeCameraPlugin,
            interaction::InteractionPlugin,
            vfx::VfxPlugin,
            smoke::SmokePlugin,
        ))
        .add_systems(Update, close_on_escape);
    if smoke_test {
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f64(1.0 / 60.0),
        ));
    }
    if profile {
        app.init_resource::<bevy::diagnostic::DiagnosticsStore>()
            .add_plugins(bevy::render::diagnostic::RenderDiagnosticsPlugin);
    }
    app.run()
}

fn close_on_escape(keys: Res<ButtonInput<KeyCode>>, mut exit: MessageWriter<AppExit>) {
    if keys.just_pressed(KeyCode::Escape) {
        exit.write(AppExit::Success);
    }
}
