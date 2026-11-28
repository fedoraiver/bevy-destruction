use std::time::Duration;

use bevy::{
    app::ScheduleRunnerPlugin, prelude::*, settings::SettingsPlugin, ui::IsDefaultUiCamera,
    winit::WinitPlugin,
};
use bevy_acceptance_test::{AcceptanceCamera, AcceptancePlugin};
use bevy_destruction::{
    DEFAULT_VIEWPORT, DemoCamera, DemoSettings, DestructionPlugin, SETTINGS_APP_NAME,
};

fn main() -> AppExit {
    let acceptance = match AcceptancePlugin::from_env() {
        Ok(acceptance) => acceptance.with_size(DEFAULT_VIEWPORT),
        Err(error) => {
            eprintln!("{error}");
            return AppExit::error();
        }
    };
    let window = acceptance.input_window_plugin();
    let runner = if std::env::var("BEVY_DESTRUCTION_ACCEPTANCE_UNCAPPED").as_deref() == Ok("1") {
        ScheduleRunnerPlugin::run_loop(Duration::ZERO)
    } else {
        acceptance.runner_plugin()
    };

    let mut app = App::new();
    app.add_plugins((acceptance, runner))
        .add_plugins(
            DefaultPlugins
                .build()
                .disable::<WinitPlugin>()
                .set(AssetPlugin {
                    file_path: concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets").into(),
                    ..default()
                })
                .set(window),
        )
        .register_required_components_with::<DemoCamera, AcceptanceCamera>(|| AcceptanceCamera)
        .register_required_components::<DemoCamera, IsDefaultUiCamera>()
        .register_type::<DemoSettings>()
        .add_plugins(SettingsPlugin::new(SETTINGS_APP_NAME))
        .add_plugins(DestructionPlugin);
    app.run()
}
