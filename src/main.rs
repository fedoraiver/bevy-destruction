mod asset;
mod debug;
mod interaction;
mod material;
mod scene;
mod smoke;
mod vfx;

use avian3d::prelude::*;
use bevy::prelude::*;

#[derive(Resource)]
pub struct DemoOptions {
    pub asset: String,
    pub smoke_output: Option<std::path::PathBuf>,
}

fn options() -> Result<DemoOptions, String> {
    let mut options = DemoOptions {
        asset: "buildings/rectangle.destruction.json".into(),
        smoke_output: None,
    };
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
                    "bevy-destruction [--asset buildings/rectangle.destruction.json] [--smoke-test output/demo]\nLeft click: remove chunks | 0 / numpad 0: fracture edges | Esc: exit"
                );
                std::process::exit(0);
            }
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
    let mut app = App::new();
    app.insert_resource(options)
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
            debug::FractureDebugPlugin,
            scene::DemoScenePlugin,
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
    app.run()
}

fn close_on_escape(keys: Res<ButtonInput<KeyCode>>, mut exit: MessageWriter<AppExit>) {
    if keys.just_pressed(KeyCode::Escape) {
        exit.write(AppExit::Success);
    }
}
