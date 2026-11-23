use std::time::Duration;

use bevy::{
    app::ScheduleRunnerPlugin, prelude::*, render::RenderPlugin, ui::IsDefaultUiCamera,
    winit::WinitPlugin,
};
use bevy_acceptance_test::{AcceptanceCamera, AcceptancePlugin, OFFSCREEN_IMAGE};
use bevy_destruction::{
    DEFAULT_VIEWPORT,
    acceptance::{AcceptanceScenario, DemoCamera},
    asset_plugin,
};

fn main() -> AppExit {
    let acceptance = match AcceptancePlugin::from_env() {
        Ok(acceptance) => acceptance.with_size(DEFAULT_VIEWPORT),
        Err(error) => {
            eprintln!("{error}");
            return AppExit::error();
        }
    };
    let scenario = match AcceptanceScenario::from_env(OFFSCREEN_IMAGE) {
        Ok(scenario) => scenario,
        Err(error) => {
            eprintln!("{error}");
            return AppExit::error();
        }
    };

    let window = acceptance.input_window_plugin();
    let runner = if scenario.profile() {
        ScheduleRunnerPlugin::run_loop(Duration::ZERO)
    } else {
        acceptance.runner_plugin()
    };
    let mut app = App::new();
    app.register_required_components_with::<DemoCamera, AcceptanceCamera>(|| AcceptanceCamera)
        .register_required_components::<DemoCamera, IsDefaultUiCamera>()
        .add_plugins(acceptance)
        .add_plugins(
            DefaultPlugins
                .build()
                .disable::<WinitPlugin>()
                .set(asset_plugin())
                .set(window)
                .set(RenderPlugin {
                    // Fixed-frame captures need pipelines ready on the frame they request.
                    synchronous_pipeline_compilation: scenario.synchronous_pipelines(),
                    ..default()
                }),
        )
        .add_plugins(runner);
    scenario.install(&mut app);
    app.run()
}
