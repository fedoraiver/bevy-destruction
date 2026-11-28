use bevy::{app::AppExit, settings::SettingsPlugin};
use bevy_destruction::{DemoSettings, SETTINGS_APP_NAME, StressSettings, VfxSettings, build_app};

fn main() -> AppExit {
    let mut app = build_app();
    app.register_type::<DemoSettings>()
        .add_plugins(SettingsPlugin::new(SETTINGS_APP_NAME));
    let settings = app.world().resource::<DemoSettings>();
    let (stress, vfx) = (settings.stress, settings.vfx);
    app.world_mut().resource_mut::<StressSettings>().enabled = stress;
    app.world_mut().resource_mut::<VfxSettings>().enabled = vfx;
    app.run()
}
