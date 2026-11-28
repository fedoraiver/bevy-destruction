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
        load_demo_settings(&mut app, directory.to_str().unwrap());
        app
    };

    let missing = load();
    fs::write(
        &settings_file,
        r#"[demo_settings]
vfx = false
"#,
    )
    .unwrap();
    let partial = load();
    fs::write(
        &settings_file,
        r#"[demo_settings]
scene = "buildings/highrise_slab.destruction.json"
stress = false
vfx = false
detail = false
soft_particles = false
"#,
    )
    .unwrap();
    let configured = load();
    fs::remove_file(settings_file).unwrap();
    fs::remove_dir(directory).unwrap();

    let settings = missing.world().resource::<DemoSettings>();
    assert_eq!(settings.scene, "buildings/rectangle.destruction.json");
    assert!(settings.stress && settings.vfx && settings.detail && settings.soft_particles);

    let settings = partial.world().resource::<DemoSettings>();
    assert_eq!(settings.scene, "buildings/rectangle.destruction.json");
    assert!(settings.stress && !settings.vfx && settings.detail && settings.soft_particles);

    let settings = configured.world().resource::<DemoSettings>();
    assert_eq!(settings.scene, "buildings/highrise_slab.destruction.json");
    assert!(!settings.stress && !settings.vfx && !settings.detail && !settings.soft_particles);
}

#[test]
fn scene_paths_must_name_an_asset_relative_manifest() {
    for scene in [
        "rectangle",
        "../buildings/rectangle.destruction.json",
        "/buildings/rectangle.destruction.json",
        "C:/buildings/rectangle.destruction.json",
        "buildings/rectangle.glb",
        "source://buildings/rectangle.destruction.json",
    ] {
        let settings = DemoSettings {
            scene: scene.into(),
            ..default()
        };
        assert!(settings.scene_path().is_err(), "accepted {scene}");
    }
    let settings = DemoSettings::default();
    assert_eq!(
        settings.scene_path(),
        Ok("buildings/rectangle.destruction.json")
    );
}
