use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

use super::*;

#[test]
fn demo_settings_load_from_toml() {
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let directory = asset_root().with_file_name("tmp").join(format!(
        "demo-settings-test-{}-{}",
        std::process::id(),
        timestamp.as_nanos()
    ));
    fs::create_dir_all(&directory).unwrap();
    let settings_file = directory.join("settings.toml");
    // Bevy joins the app name to the preferences directory. Use an absolute
    // test directory so the loader never reads the user's settings.
    let load = || {
        let mut app = App::new();
        app.init_resource::<ProbeOptions>();
        load_demo_settings(&mut app, directory.to_str().unwrap());
        app
    };

    let missing = load();
    fs::write(
        &settings_file,
        r#"[demo_options]
vfx = false
"#,
    )
    .unwrap();
    let partial = load();
    fs::write(
        &settings_file,
        r#"[demo_options]
asset = "buildings/highrise_slab.destruction.json"
stress = false
vfx = false
detail = false
soft_particles = false
smoke_output = "output"
profile = true
"#,
    )
    .unwrap();
    let configured = load();
    fs::remove_file(settings_file).unwrap();
    fs::remove_dir(directory).unwrap();

    let options = missing.world().resource::<DemoOptions>();
    assert_eq!(options.asset, "buildings/rectangle.destruction.json");
    assert!(options.stress && options.vfx && options.detail && options.soft_particles);

    let options = partial.world().resource::<DemoOptions>();
    assert_eq!(options.asset, "buildings/rectangle.destruction.json");
    assert!(options.stress && !options.vfx && options.detail && options.soft_particles);

    let options = configured.world().resource::<DemoOptions>();
    assert_eq!(options.asset, "buildings/highrise_slab.destruction.json");
    assert!(!options.stress && !options.vfx && !options.detail && !options.soft_particles);
    let probe = configured.world().resource::<ProbeOptions>();
    assert!(!probe.scripted() && !probe.profile);
}
