use std::time::Duration;

use bevy::{
    app::ScheduleRunnerPlugin, prelude::*, settings::SettingsPlugin, ui::IsDefaultUiCamera,
    winit::WinitPlugin,
};
use bevy_acceptance_test::{AcceptanceCamera, AcceptancePlugin};
use bevy_destruction::{
    BuildingMaterialPlugin, DEFAULT_VIEWPORT, DemoCamera, DemoScenePlugin, DemoSettings,
    DestructibleAssetPlugin, DestructionPhysicsPlugin, DustMaterialPlugin, ForceDebugPlugin,
    FractureDebugPlugin, FreeCameraPlugin, InteractionPlugin, PauseMenuPlugin, PhysicsPlugins,
    ProjectilePlugin, SETTINGS_APP_NAME, StressRuntimePlugin, StressSettings, VfxPlugin,
    VfxSettings,
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
                    file_path: std::path::absolute("assets")
                        .expect("Could not resolve the assets directory")
                        .to_string_lossy()
                        .into_owned(),
                    ..default()
                })
                .set(window),
        )
        .register_required_components_with::<DemoCamera, AcceptanceCamera>(|| AcceptanceCamera)
        .register_required_components::<DemoCamera, IsDefaultUiCamera>()
        .register_type::<DemoSettings>()
        .add_plugins(SettingsPlugin::new(SETTINGS_APP_NAME))
        .init_resource::<DemoSettings>();

    let settings = app.world().resource::<DemoSettings>();
    let stress = StressSettings {
        enabled: settings.stress,
        ..default()
    };
    let vfx = VfxSettings {
        enabled: settings.vfx,
        ..default()
    };
    app.insert_resource(stress)
        .insert_resource(vfx)
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
    app.run()
}
