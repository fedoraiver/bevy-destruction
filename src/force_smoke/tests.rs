use super::*;
use avian3d::prelude::RigidBodyColliders;

use crate::{physics::building_clusters, scene::BondOfBuilding};

#[test]
fn same_asset_visibility_fixtures_own_independent_bonds() {
    let mut world = World::new();
    world.init_resource::<crate::camera::CameraControl>();
    world.init_resource::<Assets<Mesh>>();
    world.init_resource::<Assets<StandardMaterial>>();
    world.init_resource::<Assets<BuildingMaterial>>();
    world.init_resource::<Assets<DestructibleAsset>>();
    let fixture = spawn_visibility_fixture(&mut world);

    let original = world.get::<BuildingInstance>(fixture.building).unwrap();
    let twin = world.get::<BuildingInstance>(fixture.twin).unwrap();
    assert_eq!(original.asset, twin.asset);
    assert_eq!(original.bonds.len(), TARGETS.len());
    assert_eq!(twin.bonds.len(), TARGETS.len());
    assert!(building_clusters(&world, fixture.building).eq([fixture.body]));
    let twin_body = {
        let mut bodies = building_clusters(&world, fixture.twin);
        let body = bodies.next().expect("The twin building has a cluster");
        assert!(bodies.next().is_none());
        body
    };
    for (id, &original_bond) in &original.bonds {
        let twin_bond = twin.bonds[id];
        assert_ne!(original_bond, twin_bond);
        for (building, body, entity) in [
            (fixture.building, fixture.body, original_bond),
            (fixture.twin, twin_body, twin_bond),
        ] {
            let bond = world.get::<BondInstance>(entity).unwrap();
            assert_eq!(
                world.get::<BondOfBuilding>(entity).unwrap().building,
                building
            );
            assert!(bond.active);
            for endpoint in bond.endpoints {
                assert_eq!(
                    world.get::<ChunkIdentity>(endpoint).unwrap().building,
                    building
                );
                assert_eq!(world.get::<ColliderOf>(endpoint).unwrap().body, body);
                assert!(world.get::<Collider>(endpoint).is_none());
                assert!(
                    world
                        .get::<RigidBodyColliders>(body)
                        .unwrap()
                        .contains(&endpoint)
                );
            }
        }
    }
    let original_bond = original.bonds["visibility-window"];
    let twin_bond = twin.bonds["visibility-window"];
    world.get_mut::<BondInstance>(original_bond).unwrap().active = false;
    assert!(world.get::<BondInstance>(twin_bond).unwrap().active);
}

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

fn first_fixture_hit(point: Vec3) -> String {
    let origin = Vec3::Z * FIXTURE_DISTANCE;
    let direction = (point - origin).normalize();
    visibility_shapes()
        .into_iter()
        .flat_map(|shape| {
            shape
                .boxes
                .into_iter()
                .flat_map(|(center, size)| box_triangles(center, size))
                .filter_map(move |[a, b, c]| {
                    let edge1 = b - a;
                    let edge2 = c - a;
                    let cross = direction.cross(edge2);
                    let det = edge1.dot(cross);
                    if det.abs() < 1e-7 {
                        return None;
                    }
                    let relative = origin - a;
                    let u = relative.dot(cross) / det;
                    let v = direction.dot(relative.cross(edge1)) / det;
                    let t = edge2.dot(relative.cross(edge1)) / det;
                    (u >= -1e-6 && v >= -1e-6 && u + v <= 1.0 + 1e-6 && t > 0.0)
                        .then_some((t, shape.id.clone()))
                })
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .unwrap()
        .1
}

#[test]
fn visibility_fixture_has_real_window_and_partial_edge_with_both_centers_hidden() {
    assert_eq!(first_fixture_hit(Vec3::new(-8.6, 0.0, 0.0)), "front-a");
    assert_eq!(
        first_fixture_hit(Vec3::new(-3.6, 0.0, 0.0)),
        "rear-occluder"
    );
    assert_eq!(first_fixture_hit(Vec3::new(1.4, 0.0, 0.0)), "window-a");
    assert_eq!(first_fixture_hit(Vec3::new(2.6, 0.0, 0.0)), "window-b");
    assert_eq!(
        first_fixture_hit(Vec3::new(6.4, 0.0, 0.0)),
        "partial-occluder"
    );
    assert_eq!(
        first_fixture_hit(Vec3::new(7.6, 0.0, 0.0)),
        "partial-occluder"
    );
    assert_eq!(first_fixture_hit(Vec3::new(8.1, 0.0, 0.0)), "partial-b");
}

#[test]
fn fixture_source_triangles_are_closed_boxes_with_the_declared_volume() {
    for shape in visibility_shapes() {
        for (center, size) in shape.boxes {
            let volume = box_triangles(center, size)
                .into_iter()
                .map(|[a, b, c]| {
                    (a - center)
                        .as_dvec3()
                        .dot((b - center).as_dvec3().cross((c - center).as_dvec3()))
                        / 6.0
                })
                .sum::<f64>();
            assert!((volume - f64::from(size.element_product())).abs() < 1e-5);
        }
    }
}

#[test]
fn visibility_readback_requires_all_five_actually_visible_source_ids() {
    let mut state = ForceSmokeState {
        phase: 5,
        ..default()
    };
    let mut capture = readback_fixture();
    assert!(!accept_readback(&mut state, 5, readback_fixture()));
    for (row, index) in [0, 1, 4, 5, 7].into_iter().enumerate() {
        for x in 30..50 {
            let [r, g, b] = TARGET_COLORS[index];
            paint(&mut capture, x, 30 + row, [r, g, b, 255]);
        }
    }
    assert!(accept_readback(&mut state, 5, capture));
    assert_eq!(state.phase, 6);
    assert_eq!(source_id_pixels(&state.captures[&5], 2), 0);
}

#[test]
fn source_id_masks_separate_instances_that_share_the_same_mesh_and_color() {
    let mut capture = readback_fixture();
    let [r, g, b] = TARGET_COLORS[4];
    paint(&mut capture, 40, 30, [r, g, b, 255]);
    paint(&mut capture, 40, 70, [r, g, b, 255]);
    assert_eq!(id_indices(&capture, 4, true), vec![(30 * 120 + 40) * 4]);
    assert_eq!(id_indices(&capture, 4, false), vec![(70 * 120 + 40) * 4]);
}

#[test]
fn heat_pixel_gate_distinguishes_warm_red_neutral_and_exact_restoration() {
    let mut neutral = readback_fixture();
    for x in 30..33 {
        paint(&mut neutral, x, 60, [130, 130, 130, 255]);
    }
    let indices = (30..33).map(|x| (60 * 120 + x) * 4).collect::<Vec<_>>();
    assert_eq!(heat_hues(&neutral, &indices), (0, 0));
    assert_eq!(changed_indices(&neutral, &neutral, &indices), 0);
    let mut heat = readback_fixture();
    paint(&mut heat, 30, 60, [210, 130, 20, 255]);
    paint(&mut heat, 31, 60, [210, 20, 20, 255]);
    paint(&mut heat, 32, 60, [130, 130, 130, 255]);
    assert_eq!(heat_hues(&heat, &indices), (1, 1));
    assert_eq!(changed_indices(&neutral, &heat, &indices), 2);
}

#[test]
fn building_readback_rejects_background_ground_and_depth_stripes_inside_source_masks() {
    let mut reference = readback_fixture();
    for (row, target) in [(30, 0), (32, 1), (70, 0), (72, 1)] {
        for x in 30..50 {
            let [r, g, b] = TARGET_COLORS[target];
            paint(&mut reference, x, row, [r, g, b, 255]);
        }
    }
    let mut state = ForceSmokeState {
        phase: 6,
        ..default()
    };
    state.captures.insert(5, reference);
    let mut depth_only = readback_fixture();
    // A bright ground outside the known source silhouette cannot stand in for
    // the forward material; this reproduces the first GPU warmup failure.
    for y in 78..85 {
        for x in 0..120 {
            paint(&mut depth_only, x, y, [112, 115, 119, 255]);
        }
    }
    assert!(!accept_readback(
        &mut state,
        6,
        Capture {
            width: 120,
            height: 100,
            pixels: depth_only.pixels.clone()
        }
    ));
    for row in [30, 32, 70, 72] {
        for x in (30..50).step_by(5) {
            paint(&mut depth_only, x, row, [155, 160, 162, 255]);
        }
    }
    assert!(!accept_readback(
        &mut state,
        6,
        Capture {
            width: 120,
            height: 100,
            pixels: depth_only.pixels.clone()
        }
    ));
    assert_eq!(state.phase, 6);
    for row in [30, 32, 70, 72] {
        for x in 30..50 {
            paint(&mut depth_only, x, row, [155, 160, 162, 255]);
        }
    }
    assert!(accept_readback(&mut state, 6, depth_only));
    assert_eq!(state.phase, 7);
    assert_eq!(state.rejected_readbacks[6], 2);
}

#[test]
fn alt_readback_requires_a_warm_visible_surface_in_addition_to_the_arrow() {
    let reference = readback_fixture();
    let mut capture = readback_fixture();
    for x in 30..40 {
        paint(&mut capture, x, 65, [61, 230, 255, 255]);
    }
    assert!(building_readback_pixels(&reference, &capture, 8).is_none());
    for y in 40..60 {
        for x in 50..70 {
            paint(&mut capture, x, y, [210, 130, 20, 255]);
        }
    }
    assert!(building_readback_pixels(&reference, &capture, 8).is_some());
}
