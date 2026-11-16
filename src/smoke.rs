use avian3d::prelude::Collider;
use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
};

use crate::{
    DemoOptions,
    camera::{FlightInput, LOOK_SENSITIVITY, ScriptedFlight},
    debug::{DebugOverlay, FractureEdges},
    interaction::{BREAK_LIMIT, DamageRequests, ScriptedClick},
    scene::{BuildingInstance, DemoCamera, RemovedChunk},
    vfx::{PARTICLE_LIMIT, Particle},
};

pub struct SmokePlugin;

impl Plugin for SmokePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SmokeState>()
            .add_systems(Update, drive.after(crate::interaction::apply_damage));
    }
}

#[derive(Resource, Default)]
struct SmokeState {
    frames: u32,
    first_removed: usize,
    errors: Vec<String>,
    peak_particles: usize,
    first_point: Option<Vec3>,
    first_particles: usize,
    stress_time: f32,
    camera_origin: Option<Transform>,
    camera_checks: bool,
}

#[derive(Component)]
struct LoadedAssetProbe;

fn capture(commands: &mut Commands, directory: &std::path::Path, name: &str) {
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(directory.join(name)));
}

#[allow(clippy::too_many_arguments)]
fn drive(
    mut commands: Commands,
    options: Res<DemoOptions>,
    time: Res<Time>,
    mut state: ResMut<SmokeState>,
    instances: Query<&BuildingInstance, Without<LoadedAssetProbe>>,
    probes: Query<(Entity, &BuildingInstance), With<LoadedAssetProbe>>,
    removed: Query<(Has<Collider>, Has<FractureEdges>), With<RemovedChunk>>,
    particles: Query<&Particle>,
    mut requests: ResMut<DamageRequests>,
    mut debug: ResMut<DebugOverlay>,
    mut exit: MessageWriter<AppExit>,
    mut windows: Query<(Entity, &mut Window)>,
    cameras: Query<(&Camera, &GlobalTransform, &Transform), With<DemoCamera>>,
    mut scripted: ResMut<ScriptedClick>,
    assets: Res<Assets<crate::asset::DestructibleAsset>>,
    mut flight: ResMut<ScriptedFlight>,
) {
    let Some(directory) = &options.smoke_output else {
        return;
    };
    let Some(instance) = instances.iter().next() else {
        return;
    };
    if let Some(error) = &instance.error {
        error!("Smoke test load failed: {error}");
        std::fs::create_dir_all(directory).expect("Create smoke output");
        std::fs::write(directory.join("report.json"), serde_json::to_string_pretty(&serde_json::json!({ "load_failed": true, "chunks": instance.chunks.len(), "errors": [error] })).unwrap()).expect("Write failed load report");
        exit.write(AppExit::error());
        return;
    }
    let Some(bounds) = instance.bounds else {
        if time.elapsed_secs() > 60.0 {
            error!("Smoke test timed out loading asset");
            exit.write(AppExit::error());
        }
        return;
    };
    state.frames += 1;
    state.peak_particles = state.peak_particles.max(particles.iter().len());
    if particles.iter().len() > PARTICLE_LIMIT {
        state.errors.push("Particle budget exceeded".into());
    }
    let target = matches!(state.frames, 148 | 150)
        .then(|| {
            instance
                .chunks
                .values()
                .filter(|chunk| chunk.alive)
                .flat_map(|chunk| &chunk.triangles)
                .filter(|t| (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero().z > 0.95)
                .map(|t| (t[0] + t[1] + t[2]) / 3.0)
                .filter(|p| p.z > bounds.max.z - 0.05)
                .min_by(|a, b| {
                    a.distance_squared(bounds.center())
                        .total_cmp(&b.distance_squared(bounds.center()))
                })
        })
        .flatten();
    match state.frames {
        1 => {
            std::fs::create_dir_all(directory).expect("Create smoke output");
        }
        10 => {
            commands.spawn_scene(bsn! { Name::new("Already-loaded asset probe") Transform::IDENTITY Visibility::Hidden })
                .insert((BuildingInstance::pending(instance.asset.clone()), LoadedAssetProbe));
        }
        20 => state.camera_origin = Some(*cameras.single().expect("Smoke camera").2),
        21 => {
            flight.0 = Some(FlightInput {
                movement: Vec3::new(1.0, 1.0, -1.0),
                ..default()
            })
        }
        23 => {
            let actual = cameras.single().expect("Smoke camera").2;
            let expected = state.camera_origin.unwrap().translation
                + Vec3::new(1.0, 1.0, -1.0).normalize() * 0.1;
            if actual.translation.distance(expected) > 0.001 {
                state
                    .errors
                    .push("Free flight did not move at normalized 6m/s".into());
            }
            flight.0 = Some(FlightInput {
                movement: Vec3::new(-1.0, -1.0, 1.0),
                ..default()
            });
        }
        25 => {
            flight.0 = Some(FlightInput {
                mouse: Vec2::new(125.0, -75.0),
                ..default()
            })
        }
        27 => {
            let actual = cameras.single().expect("Smoke camera").2;
            let expected = Quat::from_euler(EulerRot::YXZ, -0.25, 0.15, 0.0);
            if actual.rotation.angle_between(expected) > 0.001 {
                state
                    .errors
                    .push("Mouse look did not apply yaw and pitch".into());
            }
            flight.0 = Some(FlightInput {
                mouse: Vec2::new(-125.0, 75.0),
                ..default()
            });
        }
        29 => {
            let actual = cameras.single().expect("Smoke camera").2;
            let original = state.camera_origin.unwrap();
            state.camera_checks = actual.translation.distance(original.translation) < 0.001
                && actual.rotation.angle_between(original.rotation) < 0.001;
            if !state.camera_checks {
                state
                    .errors
                    .push("Free camera pose was reset or movement failed to reverse".into());
            }
        }
        60 => {
            if probes.is_empty() {
                state
                    .errors
                    .push("Missing already-loaded instance probe".into());
            }
            for (entity, probe) in &probes {
                if probe.bounds.is_none() || probe.chunks.len() != instance.chunks.len() {
                    state
                        .errors
                        .push("Already-loaded asset did not instantiate".into());
                }
                commands.entity(entity).despawn();
            }
        }
        65..=69 => {
            flight.0 = Some(FlightInput {
                movement: Vec3::X,
                ..default()
            })
        }
        90 => capture(&mut commands, directory, "01-grid.png"),
        100 | 300 => {
            let (window, _) = windows.single().expect("Smoke window");
            commands.write_message(bevy::input::mouse::MouseButtonInput {
                button: MouseButton::Left,
                state: bevy::input::ButtonState::Pressed,
                window,
            });
        }
        101 | 301 => {
            let (window, _) = windows.single().expect("Smoke window");
            commands.write_message(bevy::input::mouse::MouseButtonInput {
                button: MouseButton::Left,
                state: bevy::input::ButtonState::Released,
                window,
            });
        }
        110 => debug.0 = true,
        130 => capture(&mut commands, directory, "02-fracture-edges.png"),
        145 => debug.0 = false,
        148 => {
            if let Some(point) = target {
                let transform = cameras.single().expect("Smoke camera").2;
                let aimed =
                    Transform::from_translation(transform.translation).looking_at(point, Vec3::Y);
                let (yaw, pitch, _) = transform.rotation.to_euler(EulerRot::YXZ);
                let (target_yaw, target_pitch, _) = aimed.rotation.to_euler(EulerRot::YXZ);
                flight.0 = Some(FlightInput {
                    mouse: Vec2::new(yaw - target_yaw, pitch - target_pitch) / LOOK_SENSITIVITY,
                    ..default()
                });
            }
        }
        150 => {
            if let Some(point) = target {
                state.first_point = Some(point);
                let (camera, transform, _) = cameras.single().expect("Smoke camera");
                let projected = camera
                    .world_to_viewport(transform, point)
                    .expect("Project test click");
                let (window, mut data) = windows.single_mut().expect("Smoke window");
                let cursor = data.size() * 0.5;
                if projected.distance(cursor) > 0.01 {
                    state
                        .errors
                        .push("Mouse look did not aim the centre ray at the target".into());
                }
                data.set_cursor_position(Some(cursor));
                scripted.0 = Some(cursor);
                commands.write_message(bevy::input::mouse::MouseButtonInput {
                    button: MouseButton::Left,
                    state: bevy::input::ButtonState::Pressed,
                    window,
                });
            } else {
                state.errors.push("No front face for test click".into());
            }
        }
        151 => {
            let (window, _) = windows.single().expect("Smoke window");
            commands.write_message(bevy::input::mouse::MouseButtonInput {
                button: MouseButton::Left,
                state: bevy::input::ButtonState::Released,
                window,
            });
        }
        154 => {
            state.first_particles = particles.iter().len();
            info!(
                "First click at {:.3}s: {} particles",
                time.elapsed_secs(),
                state.first_particles
            );
            if state.first_particles != 64 {
                let count = state.first_particles;
                state
                    .errors
                    .push(format!("Expected 64 particles from one click, got {count}"));
            }
            let deleted = instance
                .chunks
                .values()
                .filter(|chunk| !chunk.alive)
                .count();
            state.first_removed = deleted;
            if !(1..=BREAK_LIMIT).contains(&deleted) {
                state
                    .errors
                    .push(format!("First click removed {deleted} chunks"));
            }
            for (has_collider, has_edges) in &removed {
                if has_collider {
                    state
                        .errors
                        .push("Removed chunk still has a collider after fixed boundary".into());
                }
                if has_edges {
                    state.errors.push("Removed chunk kept debug edges".into());
                }
            }
            let manifest = &assets.get(&instance.asset).unwrap().manifest;
            for (id, chunk) in instance.chunks.iter().filter(|(_, chunk)| !chunk.alive) {
                if chunk
                    .primitives
                    .iter()
                    .any(|entity| commands.get_entity(*entity).is_ok())
                {
                    state
                        .errors
                        .push("Removed chunk kept a render primitive".into());
                }
                if crate::interaction::surface_distance(
                    &chunk.triangles,
                    state.first_point.unwrap(),
                    Vec3::Z,
                )
                .is_none()
                {
                    state
                        .errors
                        .push("First click penetrated the front layer".into());
                }
                if manifest
                    .bonds
                    .iter()
                    .filter(|bond| &bond.a == id || &bond.b == id)
                    .any(|bond| instance.bonds[&bond.id])
                {
                    state
                        .errors
                        .push("Incident bond survived chunk removal".into());
                }
            }
        }
        180 => capture(&mut commands, directory, "03-damage-and-dust.png"),
        200 => {
            state.stress_time = time.elapsed_secs();
            for point in instance
                .chunks
                .values()
                .filter(|chunk| chunk.alive)
                .flat_map(|chunk| &chunk.triangles)
                .filter(|t| (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero().z > 0.95)
                .map(|t| (t[0] + t[1] + t[2]) / 3.0)
                .filter(|p| p.z > bounds.max.z - 0.05)
                .take(48)
            {
                requests
                    .0
                    .push_back(Ray3d::new(point + Vec3::Z * 20.0, Dir3::NEG_Z));
            }
        }
        210 => capture(&mut commands, directory, "04-budget-stress.png"),
        400 => {
            if particles.iter().len() != 0 {
                state
                    .errors
                    .push("Particles outlived their maximum lifetime".into());
            }
            capture(&mut commands, directory, "05-settled.png");
        }
        430 => {
            for name in [
                "01-grid.png",
                "02-fracture-edges.png",
                "03-damage-and-dust.png",
                "04-budget-stress.png",
                "05-settled.png",
            ] {
                if !directory.join(name).is_file() {
                    state.errors.push(format!("Missing screenshot {name}"));
                }
            }
            let report = serde_json::json!({ "asset": options.asset, "chunks": instance.chunks.len(), "primitives": instance.chunks.values().map(|c| c.primitives.len()).sum::<usize>(),
                "first_click_removed": state.first_removed, "first_click_particles": state.first_particles, "peak_particles": state.peak_particles, "particle_limit": PARTICLE_LIMIT,
                "remaining_particles": particles.iter().len(), "camera_flight_checks": state.camera_checks, "stress_to_end_seconds": time.elapsed_secs() - state.stress_time,
                "remaining_particle_ages": particles.iter().take(6).map(|p| [p.age, p.lifetime]).collect::<Vec<_>>(), "errors": state.errors });
            std::fs::write(
                directory.join("report.json"),
                serde_json::to_string_pretty(&report).unwrap(),
            )
            .expect("Write smoke report");
            if state.errors.is_empty() {
                info!("Smoke test passed: {report}");
                exit.write(AppExit::Success);
            } else {
                error!("Smoke test failed: {report}");
                exit.write(AppExit::error());
            }
        }
        _ => {}
    }
}
