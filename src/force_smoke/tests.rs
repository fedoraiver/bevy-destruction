use super::*;

fn readback_fixture() -> Capture {
    Capture {
        width: 120,
        height: 100,
        pixels: [19_u8, 23, 31, 255].repeat(120 * 100),
    }
}

fn paint(capture: &mut Capture, x: usize, y: usize, color: [u8; 4]) {
    let index = (y * capture.width + x) * 4;
    capture.pixels[index..index + 4].copy_from_slice(&color);
}

#[test]
fn hud_and_crosshair_readback_cannot_advance_or_replace_the_scene_baseline() {
    let mut state = ForceSmokeState::default();
    let mut empty = readback_fixture();
    for y in 0..25 {
        for x in 0..empty.width {
            paint(&mut empty, x, y, [180, 180, 180, 255]);
        }
    }
    for x in 50..70 {
        paint(&mut empty, x, 50, [240, 180, 60, 255]);
    }
    assert!(!accept_readback(&mut state, 0, empty));
    assert_eq!(state.phase, 0);
    assert!(state.captures.is_empty() && state.scene_pixels.is_empty());
    assert_eq!(state.rejected_readbacks[0], 1);

    let mut ready = readback_fixture();
    for y in 27..47 {
        for x in 24..96 {
            paint(&mut ready, x, y, [155, 158, 160, 255]);
        }
    }
    assert!(accept_readback(&mut state, 0, ready));
    assert_eq!(state.phase, 1);
    assert!(state.scene_pixels[&0] > 256);
    assert_eq!(state.captures.len(), 1);
}

#[test]
fn thin_debug_lines_without_solid_scene_surfaces_are_not_a_ready_readback() {
    let mut lines = readback_fixture();
    for y in (27..47).step_by(6) {
        for x in 18..102 {
            paint(&mut lines, x, y, [155, 158, 160, 255]);
        }
    }
    let mut state = ForceSmokeState {
        phase: 1,
        ..default()
    };
    assert!(!accept_readback(&mut state, 1, lines));
    assert_eq!(state.phase, 1);
    assert!(state.captures.is_empty());
}

#[test]
fn rejected_asset_writes_its_failure_report_without_waiting_for_geometry() {
    let directory = std::env::temp_dir().join(format!(
        "bevy-destruction-force-rejected-{}",
        std::process::id()
    ));
    let mut world = World::new();
    world.insert_resource(DemoOptions::default());
    world.init_resource::<Messages<AppExit>>();
    let mut instance = BuildingInstance::pending(Handle::default());
    instance.error = Some("fixture: missing required contact facets".into());
    world.spawn(instance);
    let mut state = ForceSmokeState::default();

    tick(&mut world, &mut state, &directory);

    assert!(state.finished);
    assert_eq!(state.frames, 1);
    assert!(state.captures.is_empty());
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("report.json")).unwrap()).unwrap();
    assert!(report["errors"].as_array().unwrap().iter().any(|error| {
        error
            .as_str()
            .unwrap()
            .contains("fixture: missing required contact facets")
    }));
    std::fs::remove_file(directory.join("report.json")).unwrap();
    std::fs::remove_dir(directory).unwrap();
}
