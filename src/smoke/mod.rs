use avian3d::prelude::Collider;
use bevy::{ecs::system::SystemParam, prelude::*, render::view::screenshot::save_to_disk};

use crate::{
    DemoOptions,
    camera::{FlightInput, LOOK_SENSITIVITY, ScriptedFlight},
    debug::{DebugOverlay, FractureEdges},
    diagnostics::RunMetrics,
    dust_material::DustMaterial,
    interaction::{BREAK_LIMIT, DamageRequests, ScriptedClick},
    scene::{
        BondInstance, BuildingInstance, ChunkInstance, DamageDetail, DemoCamera, DetailStats,
        PatchInstance, RemovedChunk,
    },
    vfx::{PARTICLE_LIMIT, Particle, VfxStats},
};

const SETTLE_START: u32 = 760;
// The longest burst particle (including its spawn delay) expires within 3 simulated seconds.
const VFX_QUIET_FRAMES: u32 = 180;
const SETTLE_DEADLINE: u32 = 1200;
const SCREENSHOT_READBACK_FRAMES: u32 = 30;

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
    observed_bursts: u64,
    last_burst_frame: u32,
    settled_frame: Option<u32>,
    camera_origin: Option<Transform>,
    camera_checks: bool,
    replay_frames: u32,
    rim_focus: Option<Vec3>,
}

#[derive(SystemParam)]
struct SmokeResources<'w, 's> {
    options: Res<'w, DemoOptions>,
    chunks: Query<'w, 's, &'static ChunkInstance>,
    bonds: Query<'w, 's, &'static BondInstance>,
    patches: Query<'w, 's, &'static PatchInstance>,
    vfx: Res<'w, VfxStats>,
    detail: Res<'w, DetailStats>,
    metrics: ResMut<'w, RunMetrics>,
    details: Query<'w, 's, &'static DamageDetail>,
    dust_materials: Res<'w, Assets<DustMaterial>>,
    meshes: Res<'w, Assets<Mesh>>,
    adapter: Res<'w, bevy::render::renderer::RenderAdapterInfo>,
}

#[derive(Component)]
struct LoadedAssetProbe;

fn capture(
    commands: &mut Commands,
    pending: &mut Option<Entity>,
    options: &DemoOptions,
    directory: &std::path::Path,
    name: &str,
) {
    if options.profile {
        return;
    }
    let entity = *pending.get_or_insert_with(|| {
        commands
            .spawn((
                Name::new("Destruction smoke screenshot"),
                options.smoke_screenshot(),
            ))
            .id()
    });
    commands
        .entity(entity)
        .observe(save_to_disk(directory.join(name)));
}

#[allow(clippy::too_many_arguments)]
fn drive(
    mut commands: Commands,
    resources: SmokeResources,
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
    mut flight: ResMut<ScriptedFlight>,
) {
    let SmokeResources {
        options,
        chunks,
        bonds,
        patches,
        vfx,
        detail,
        mut metrics,
        details,
        dust_materials,
        meshes,
        adapter,
    } = resources;
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
    metrics.phase = state.frames;
    if vfx.received != state.observed_bursts {
        state.observed_bursts = vfx.received;
        state.last_burst_frame = state.frames;
        if state.frames < SETTLE_DEADLINE {
            state.settled_frame = None;
        }
    }
    let mut pending_capture = None;
    if options.capture_replay
        && (150..=390).contains(&state.frames)
        && state.frames.is_multiple_of(2)
    {
        let name = format!("replay/frame-{:04}.png", state.replay_frames);
        capture(
            &mut commands,
            &mut pending_capture,
            &options,
            directory,
            &name,
        );
        state.replay_frames += 1;
    }
    state.peak_particles = state.peak_particles.max(particles.iter().len());
    if particles.iter().len() > PARTICLE_LIMIT {
        state.errors.push("Particle budget exceeded".into());
    }
    let target = matches!(state.frames, 148 | 150)
        .then(|| {
            instance
                .chunks
                .values()
                .filter_map(|&entity| chunks.get(entity).ok())
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
    if (392..=452).contains(&state.frames)
        && let Some(focus) = state.rim_focus
    {
        let transform = cameras.single().expect("Smoke camera").2;
        let destination = focus + Vec3::new(0.38, 0.18, 1.0);
        let travel = destination - transform.translation;
        if travel.length() > 0.11 {
            let basis = Mat3::from_cols(*transform.right(), Vec3::Y, *transform.back());
            flight.input = Some(FlightInput {
                movement: basis.inverse() * travel.normalize(),
                ..default()
            });
        }
    }
    match state.frames {
        1 => {
            std::fs::create_dir_all(directory).expect("Create smoke output");
            if options.capture_replay && !options.profile {
                std::fs::create_dir_all(directory.join("replay")).expect("Create replay output");
            }
        }
        10 => {
            commands.spawn_scene(bsn! { Name::new("Already-loaded asset probe") Transform::IDENTITY Visibility::Hidden })
                .insert((BuildingInstance::pending(instance.asset.clone()), LoadedAssetProbe));
        }
        20 => state.camera_origin = Some(*cameras.single().expect("Smoke camera").2),
        21 => {
            flight.input = Some(FlightInput {
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
            flight.input = Some(FlightInput {
                movement: Vec3::new(-1.0, -1.0, 1.0),
                ..default()
            });
        }
        25 => {
            flight.input = Some(FlightInput {
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
            flight.input = Some(FlightInput {
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
            flight.input = Some(FlightInput {
                movement: Vec3::X,
                ..default()
            })
        }
        90 => {
            if !details.is_empty() {
                state
                    .errors
                    .push("Intact building shows damage detail".into());
            }
            capture(
                &mut commands,
                &mut pending_capture,
                &options,
                directory,
                "01-grid.png",
            );
        }
        100 | 500 => {
            let (window, _) = windows.single().expect("Smoke window");
            commands.write_message(bevy::input::mouse::MouseButtonInput {
                button: MouseButton::Left,
                state: bevy::input::ButtonState::Pressed,
                window,
            });
        }
        101 | 501 => {
            let (window, _) = windows.single().expect("Smoke window");
            commands.write_message(bevy::input::mouse::MouseButtonInput {
                button: MouseButton::Left,
                state: bevy::input::ButtonState::Released,
                window,
            });
        }
        110 => debug.enabled = true,
        130 => capture(
            &mut commands,
            &mut pending_capture,
            &options,
            directory,
            "02-fracture-edges.png",
        ),
        145 => debug.enabled = false,
        148 => {
            if let Some(point) = target {
                let transform = cameras.single().expect("Smoke camera").2;
                let aimed =
                    Transform::from_translation(transform.translation).looking_at(point, Vec3::Y);
                let (yaw, pitch, _) = transform.rotation.to_euler(EulerRot::YXZ);
                let (target_yaw, target_pitch, _) = aimed.rotation.to_euler(EulerRot::YXZ);
                flight.input = Some(FlightInput {
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
                scripted.cursor = Some(cursor);
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
            let expected = if options.no_vfx { 0 } else { 64 };
            if state.first_particles != expected {
                let count = state.first_particles;
                state.errors.push(format!(
                    "Expected {expected} scheduled particles from one click, got {count}"
                ));
            }
            if !options.no_vfx && (vfx.active[0] == 0 || vfx.scheduled[2] == 0) {
                state
                    .errors
                    .push("Impact and delayed smoke stages were not separated".into());
            }
            if !options.no_detail && details.is_empty() {
                state
                    .errors
                    .push("Exposed break has no geometric rim detail".into());
            }
            let hit = state.first_point.unwrap();
            state.rim_focus = instance
                .patches
                .iter()
                .filter_map(|&entity| patches.get(entity).ok().map(PatchInstance::patch))
                .filter(|patch| {
                    patch
                        .exposed_survivor(|id| chunks.get(instance.chunks[id]).unwrap().alive)
                        .is_some()
                })
                .flat_map(|patch| &patch.rims)
                .map(|rim| (rim.points[0] + rim.points[1]) * 0.5)
                .filter(|point| point.z >= bounds.max.z - 0.3)
                .min_by(|a, b| a.distance_squared(hit).total_cmp(&b.distance_squared(hit)));
            let deleted = instance
                .chunks
                .values()
                .filter(|&&entity| chunks.get(entity).is_ok_and(|chunk| !chunk.alive))
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
            for (entity, chunk) in instance
                .chunks
                .values()
                .filter_map(|&entity| chunks.get(entity).ok().map(|chunk| (entity, chunk)))
                .filter(|(_, chunk)| !chunk.alive)
            {
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
                if instance
                    .bonds
                    .values()
                    .filter_map(|&entity| bonds.get(entity).ok())
                    .any(|bond| bond.active && bond.endpoints.contains(&entity))
                {
                    state
                        .errors
                        .push("Incident bond survived chunk removal".into());
                }
            }
        }
        153 => capture(
            &mut commands,
            &mut pending_capture,
            &options,
            directory,
            "06-impact-005s.png",
        ),
        159 => capture(
            &mut commands,
            &mut pending_capture,
            &options,
            directory,
            "07-debris-015s.png",
        ),
        174 => capture(
            &mut commands,
            &mut pending_capture,
            &options,
            directory,
            "08-dust-040s.png",
        ),
        180 => capture(
            &mut commands,
            &mut pending_capture,
            &options,
            directory,
            "03-damage-and-dust.png",
        ),
        210 => capture(
            &mut commands,
            &mut pending_capture,
            &options,
            directory,
            "09-dust-100s.png",
        ),
        330 => capture(
            &mut commands,
            &mut pending_capture,
            &options,
            directory,
            "10-fallout-300s.png",
        ),
        390 => capture(
            &mut commands,
            &mut pending_capture,
            &options,
            directory,
            "11-rim-without-dust.png",
        ),
        454 => {
            if let Some(point) = state.rim_focus {
                let transform = cameras.single().expect("Smoke camera").2;
                let aimed =
                    Transform::from_translation(transform.translation).looking_at(point, Vec3::Y);
                let (yaw, pitch, _) = transform.rotation.to_euler(EulerRot::YXZ);
                let (target_yaw, target_pitch, _) = aimed.rotation.to_euler(EulerRot::YXZ);
                flight.input = Some(FlightInput {
                    mouse: Vec2::new(yaw - target_yaw, pitch - target_pitch) / LOOK_SENSITIVITY,
                    ..default()
                });
            }
        }
        460 => capture(
            &mut commands,
            &mut pending_capture,
            &options,
            directory,
            "12-rim-closeup.png",
        ),
        480 => {
            state.stress_time = time.elapsed_secs();
            for point in instance
                .chunks
                .values()
                .filter_map(|&entity| chunks.get(entity).ok())
                .filter(|chunk| chunk.alive)
                .flat_map(|chunk| &chunk.triangles)
                .filter(|t| (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero().z > 0.95)
                .map(|t| (t[0] + t[1] + t[2]) / 3.0)
                .filter(|p| p.z > bounds.max.z - 0.05)
                .take(48)
            {
                requests
                    .rays
                    .push_back(Ray3d::new(point + Vec3::Z * 20.0, Dir3::NEG_Z));
            }
        }
        490 => capture(
            &mut commands,
            &mut pending_capture,
            &options,
            directory,
            "04-budget-stress.png",
        ),
        frame
            if frame >= SETTLE_START
                && state.settled_frame.is_none()
                && (frame - state.last_burst_frame >= VFX_QUIET_FRAMES
                    || frame >= SETTLE_DEADLINE) =>
        {
            if frame - state.last_burst_frame < VFX_QUIET_FRAMES {
                state
                    .errors
                    .push("Fracture effects were still emitting before the settle deadline".into());
            } else if !particles.is_empty() {
                state
                    .errors
                    .push("Particles outlived their maximum lifetime".into());
            }
            capture(
                &mut commands,
                &mut pending_capture,
                &options,
                directory,
                "05-settled.png",
            );
            state.settled_frame = Some(frame);
        }
        frame
            if state
                .settled_frame
                .is_some_and(|settled| frame == settled + SCREENSHOT_READBACK_FRAMES) =>
        {
            if state.last_burst_frame > state.settled_frame.unwrap() {
                state
                    .errors
                    .push("New fracture effects appeared after the settle deadline".into());
            }
            for name in [
                "01-grid.png",
                "02-fracture-edges.png",
                "03-damage-and-dust.png",
                "04-budget-stress.png",
                "05-settled.png",
                "06-impact-005s.png",
                "07-debris-015s.png",
                "08-dust-040s.png",
                "09-dust-100s.png",
                "10-fallout-300s.png",
                "11-rim-without-dust.png",
                "12-rim-closeup.png",
            ] {
                if !options.profile && !directory.join(name).is_file() {
                    state.errors.push(format!("Missing screenshot {name}"));
                }
            }
            if options.capture_replay && !options.profile {
                for frame in 0..state.replay_frames {
                    if !directory
                        .join(format!("replay/frame-{frame:04}.png"))
                        .is_file()
                    {
                        state.errors.push(format!("Missing replay frame {frame}"));
                    }
                }
            }
            if detail.triangles > crate::scene::DETAIL_TRIANGLE_LIMIT {
                state.errors.push("Rim triangle budget exceeded".into());
            }
            if details.iter().map(|detail| detail.triangles).sum::<usize>() != detail.triangles {
                state
                    .errors
                    .push("Rim geometry count disagrees with instance state".into());
            }
            if !options.no_vfx && vfx.feedback_bursts != vfx.received {
                state
                    .errors
                    .push("A new hit lost immediate feedback under saturation".into());
            }
            let primitives = instance
                .chunks
                .values()
                .filter_map(|&entity| chunks.get(entity).ok())
                .map(|chunk| chunk.primitives.len())
                .sum::<usize>();
            let report = serde_json::json!({ "asset": options.asset, "chunks": instance.chunks.len(), "primitives": primitives,
                "renderer":{"adapter":adapter.name,"backend":format!("{:?}",adapter.backend),"physical_resolution":windows.single().ok().map(|(_,window)|[window.physical_width(),window.physical_height()])},
                "first_click_removed": state.first_removed, "first_click_particles": state.first_particles, "peak_particles": state.peak_particles, "particle_limit": PARTICLE_LIMIT,
                "remaining_particles": particles.iter().len(), "camera_flight_checks": state.camera_checks, "stress_to_end_seconds": time.elapsed_secs() - state.stress_time, "last_burst_frame": state.last_burst_frame, "settled_frame": state.settled_frame,
                "remaining_particle_ages": particles.iter().take(6).map(|p| [p.age, p.lifetime]).collect::<Vec<_>>(),
                "vfx": {"spawned_by_layer":vfx.spawned,"dropped_by_layer":vfx.dropped,"retired":vfx.retired,"received":vfx.received,"feedback_bursts":vfx.feedback_bursts,"collision_queries":vfx.collision_queries,"collisions":vfx.collisions,"max_update_cpu_ms":vfx.max_update_cpu_ms,"max_emit_cpu_ms":vfx.max_emit_cpu_ms,"estimated_coverage":vfx.estimated_coverage,"peak_estimated_coverage":vfx.peak_estimated_coverage},
                "detail":{"cached_patches":detail.patches,"cached_meshes":detail.cached_meshes,"cached_triangles":detail.cached_triangles,"active_meshes":detail.active_meshes,"triangles":detail.triangles,"cache_build_ms":detail.cache_build_ms,"max_update_ms":detail.max_update_ms},
                "asset_counts":{"dust_materials":dust_materials.len(),"meshes":meshes.len()},
                "metrics":metrics.json(),"replay_frames":state.replay_frames,"no_vfx":options.no_vfx,"no_detail":options.no_detail,"no_soft_particles":options.no_soft_particles,"profile":options.profile,"errors": state.errors });
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
