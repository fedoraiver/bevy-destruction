use std::collections::BTreeMap;

use avian3d::prelude::RigidBody;
use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured},
};

use crate::{
    DemoOptions,
    asset::DestructibleAsset,
    force_debug::ForceViewStats,
    force_samples::{ForceCapture, ForceSamples},
    projectiles::{ImpactStats, LaunchRequest, LaunchRequests, ProjectileShape},
    scene::{BuildingInstance, DemoCamera},
    stress_runtime::StressState,
};

#[cfg(test)]
mod tests;

const NAMES: [&str; 5] = [
    "01-overlay-off.png",
    "02-structural-forces.png",
    "03-broken-connection-hidden.png",
    "04-contact-impulse.png",
    "05-overlay-cleared.png",
];

pub struct ForceSmokePlugin;

impl Plugin for ForceSmokePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ForceSmokeState>().add_systems(
            Update,
            drive
                .after(crate::scene::instantiate_pending)
                .after(crate::camera::move_camera),
        );
    }
}

struct Capture {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

#[derive(Resource, Default)]
struct ForceSmokeState {
    frames: u32,
    phase: usize,
    applied: Option<usize>,
    hold: u32,
    pending: bool,
    finished: bool,
    building: Option<Entity>,
    asset_id: String,
    schema_version: u32,
    chunk_count: usize,
    proxy_count: usize,
    before_contacts: u64,
    broken: Option<(Entity, String)>,
    captures: BTreeMap<usize, Capture>,
    scene_pixels: BTreeMap<usize, usize>,
    rejected_readbacks: [u32; 5],
    peak_static_bonds: usize,
    peak_force_pairs: usize,
    peak_force_n: f64,
    peak_impulse_ns: f64,
    saw_broken_hidden: bool,
    saw_cleared: bool,
    errors: Vec<String>,
}

fn drive(world: &mut World) {
    let Some(directory) = world.resource::<DemoOptions>().force_smoke_output.clone() else {
        return;
    };
    let mut state = world.remove_resource::<ForceSmokeState>().unwrap();
    tick(world, &mut state, &directory);
    world.insert_resource(state);
}

fn tick(world: &mut World, state: &mut ForceSmokeState, directory: &std::path::Path) {
    if state.finished {
        return;
    }
    state.frames += 1;
    if state.frames == 1 {
        std::fs::create_dir_all(directory).expect("Create force view probe directory");
    }
    if state.frames > 1800 {
        state
            .errors
            .push("Force view timed out waiting for actual geometry/contact/readback".into());
        finish(world, state, directory);
        return;
    }
    if state.building.is_none() {
        if let Some(error) = world
            .query::<&BuildingInstance>()
            .iter(world)
            .find_map(|instance| instance.error.clone())
        {
            state
                .errors
                .push(format!("Force probe asset failed: {error}"));
            finish(world, state, directory);
            return;
        }
        state.building = world
            .query::<(Entity, &BuildingInstance, &StressState)>()
            .iter(world)
            .find(|(_, instance, stress)| {
                instance.bounds.is_some()
                    && instance.error.is_none()
                    && stress.reports.iter().any(|report| report.valid)
            })
            .map(|(entity, _, _)| entity);
        if state.building.is_none() {
            return;
        }
        let instance = world
            .get::<BuildingInstance>(state.building.unwrap())
            .unwrap();
        let manifest = &world
            .resource::<Assets<DestructibleAsset>>()
            .get(&instance.asset)
            .unwrap()
            .manifest;
        state.asset_id = manifest.asset_id.clone();
        state.schema_version = manifest.schema_version;
        state.chunk_count = manifest.chunks.len();
        state.proxy_count = manifest
            .chunks
            .iter()
            .map(|chunk| chunk.colliders.len())
            .sum();
    }
    if state.phase == NAMES.len() {
        finish(world, state, directory);
        return;
    }
    if state.applied != Some(state.phase) {
        world.resource_mut::<ForceCapture>().enabled = matches!(state.phase, 1..=3);
        if state.phase == 3 {
            state.before_contacts = world.resource::<ImpactStats>().contacts;
            let eye = world
                .query_filtered::<&GlobalTransform, With<DemoCamera>>()
                .single(world)
                .unwrap()
                .translation();
            let building = state.building.unwrap();
            let instance = world.get::<BuildingInstance>(building).unwrap();
            let bounds = instance.bounds.as_ref().unwrap();
            let asset = world
                .resource::<Assets<DestructibleAsset>>()
                .get(&instance.asset)
                .unwrap();
            let target = asset
                .manifest
                .chunks
                .iter()
                .filter(|record| instance.chunks[&record.id].alive)
                .filter(|record| {
                    instance.chunks[&record.id]
                        .cluster
                        .and_then(|body| world.get::<RigidBody>(body))
                        .is_some_and(|body| *body == RigidBody::Static)
                })
                .filter(|record| {
                    let y = record
                        .transform()
                        .transform_point(record.center_of_mass.into())
                        .y;
                    y > bounds.min.y + (bounds.max.y - bounds.min.y) * 0.1
                        && y < bounds.min.y + (bounds.max.y - bounds.min.y) * 0.8
                })
                .filter_map(|record| {
                    record.colliders.first().and_then(|proxy| {
                        let center = proxy
                            .vertices
                            .iter()
                            .map(|&v| Vec3::from_array(v))
                            .sum::<Vec3>()
                            / proxy.vertices.len() as f32;
                        world
                            .get::<GlobalTransform>(instance.chunks[&record.id].entity)
                            .map(|global| global.transform_point(center))
                    })
                })
                .min_by(|a, b| a.distance_squared(eye).total_cmp(&b.distance_squared(eye)));
            if let Some(target) = target {
                let direction = Dir3::new(target - eye).unwrap();
                let (yaw, pitch, _) = Transform::from_translation(eye)
                    .looking_at(target, Vec3::Y)
                    .rotation
                    .to_euler(EulerRot::YXZ);
                world.resource_mut::<crate::camera::ScriptedFlight>().0 =
                    Some(crate::camera::FlightInput {
                        movement: Vec3::ZERO,
                        mouse: Vec2::new(-yaw, -pitch) / crate::camera::LOOK_SENSITIVITY,
                    });
                world
                    .resource_mut::<LaunchRequests>()
                    .0
                    .push_back(LaunchRequest {
                        origin: eye + *direction * 1.5,
                        direction,
                        shape: ProjectileShape::Sphere,
                    });
            } else {
                state
                    .errors
                    .push("No live convex target for actual impulse sample".into());
                state.phase = 5;
                return;
            }
        }
        if state.phase == 2 {
            state.broken = world
                .resource::<ForceViewStats>()
                .drawn_bonds
                .first()
                .cloned();
            if let Some((building, id)) = &state.broken {
                let mut instance = world.get_mut::<BuildingInstance>(*building).unwrap();
                instance.bonds.insert(id.clone(), false);
                instance.topology_dirty = true;
            } else {
                state
                    .errors
                    .push("No displayed live connection for stale-vector check".into());
            }
        }
        state.applied = Some(state.phase);
        state.hold = 0;
    }
    state.hold += 1;
    let stats = world.resource::<ForceViewStats>().clone();
    if state.phase == 1 {
        state.peak_static_bonds = state.peak_static_bonds.max(stats.static_bonds);
        state.peak_force_pairs = state.peak_force_pairs.max(stats.force_pairs);
        state.peak_force_n = state.peak_force_n.max(stats.max_force_n);
    }
    if state.phase == 3 {
        state.peak_impulse_ns = state.peak_impulse_ns.max(stats.max_impulse_ns);
        if world.resource::<ImpactStats>().contacts <= state.before_contacts || stats.impulses == 0
        {
            return;
        }
    }
    if state.phase == 2 {
        state.saw_broken_hidden = state
            .broken
            .as_ref()
            .is_some_and(|broken| !stats.drawn_bonds.contains(broken));
    }
    if state.phase == 4 {
        let samples = world.resource::<ForceSamples>();
        state.saw_cleared = stats.force_pairs == 0
            && stats.impulses == 0
            && samples.dynamic.is_empty()
            && samples.impulses.is_empty();
    }
    let wait = if state.phase == 0 {
        140
    } else if state.phase == 3 {
        2
    } else {
        25
    };
    if state.pending || state.hold < wait {
        return;
    }
    let phase = state.phase;
    let path = directory.join(NAMES[phase]);
    let retry_hold = wait.saturating_sub(10);
    world.spawn(Screenshot::primary_window()).observe(
        move |capture: On<ScreenshotCaptured>, mut state: ResMut<ForceSmokeState>| {
            state.pending = false;
            match capture.image.clone().try_into_dynamic() {
                Ok(image) => {
                    let width = image.width() as usize;
                    let height = image.height() as usize;
                    let candidate = Capture {
                        width,
                        height,
                        pixels: image.to_rgba8().into_raw(),
                    };
                    if accept_readback(&mut state, phase, candidate) {
                        if let Err(error) = image.to_rgb8().save(&path) {
                            state.errors.push(format!("Force screenshot save: {error}"));
                            state.phase = NAMES.len();
                        }
                    } else {
                        state.hold = retry_hold;
                    }
                }
                Err(error) => {
                    state
                        .errors
                        .push(format!("Force screenshot readback: {error}"));
                    state.phase = 5;
                }
            }
        },
    );
    state.pending = true;
}

fn scene_surface_pixels(capture: &Capture) -> usize {
    let background = [19_u8, 23, 31];
    let radius = (capture.width.min(capture.height) / 100).max(8);
    let pixel = |x: usize, y: usize| {
        let index = (y * capture.width + x) * 4;
        &capture.pixels[index..index + 3]
    };
    let mut count = 0;
    for y in (capture.height / 4).max(2)
        ..(capture.height * 48 / 100).min(capture.height.saturating_sub(2))
    {
        for x in (capture.width * 15 / 100).max(2)
            ..(capture.width * 85 / 100).min(capture.width.saturating_sub(2))
        {
            if x.abs_diff(capture.width / 2) <= radius && y.abs_diff(capture.height / 2) <= radius {
                continue;
            }
            let center = pixel(x, y);
            let low = *center.iter().min().unwrap();
            let high = *center.iter().max().unwrap();
            if low < 43
                || high > 234
                || high - low > 35
                || center
                    .iter()
                    .zip(background)
                    .map(|(&a, b)| u32::from(a.abs_diff(b)))
                    .sum::<u32>()
                    <= 48
            {
                continue;
            }
            if [(x - 2, y), (x + 2, y), (x, y - 2), (x, y + 2)]
                .into_iter()
                .all(|(x, y)| {
                    pixel(x, y)
                        .iter()
                        .zip(center)
                        .all(|(&a, &b)| a.abs_diff(b) <= 12)
                })
            {
                count += 1;
            }
        }
    }
    count
}

fn accept_readback(state: &mut ForceSmokeState, phase: usize, candidate: Capture) -> bool {
    let scene_pixels = scene_surface_pixels(&candidate);
    if scene_pixels < (candidate.width * candidate.height / 500).max(256) {
        state.rejected_readbacks[phase] += 1;
        return false;
    }
    state.scene_pixels.insert(phase, scene_pixels);
    state.captures.insert(phase, candidate);
    state.phase = phase + 1;
    true
}

fn center_pixels(capture: &Capture) -> impl Iterator<Item = (usize, &[u8])> {
    capture
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .enumerate()
        .filter(|(index, _)| {
            let y = index / capture.width;
            y >= capture.height / 4 && y < capture.height * 85 / 100
        })
        .map(|(index, pixel)| (index, pixel.as_slice()))
}

fn finish(world: &mut World, state: &mut ForceSmokeState, directory: &std::path::Path) {
    let changed = state
        .captures
        .get(&0)
        .zip(state.captures.get(&1))
        .filter(|(a, b)| a.width == b.width && a.height == b.height)
        .map_or(0, |(a, b)| {
            center_pixels(b)
                .filter(|(index, pixel)| {
                    let old = &a.pixels[index * 4..index * 4 + 4];
                    (0..3)
                        .map(|channel| old[channel].abs_diff(pixel[channel]) as u32)
                        .sum::<u32>()
                        > 75
                })
                .count()
        });
    let purple = state.captures.get(&3).map_or(0, |capture| {
        center_pixels(capture)
            .filter(|(_, p)| {
                u16::from(p[0]) > u16::from(p[1]) + 30
                    && u16::from(p[2]) > u16::from(p[1]) + 30
                    && p[2] > 140
            })
            .count()
    });
    if state.peak_static_bonds == 0 || state.peak_force_pairs == 0 || state.peak_force_n <= 0.0 {
        state
            .errors
            .push("No real valid selfweight forces were displayed".into());
    }
    if changed < 80 {
        state.errors.push(format!(
            "Force overlay changed only {changed} non-HUD pixels"
        ));
    }
    if state.peak_impulse_ns <= 0.0 || purple < 5 {
        state.errors.push(format!(
            "Actual contact impulse was not visible: {} Ns, {purple} purple pixels",
            state.peak_impulse_ns
        ));
    }
    if !state.saw_broken_hidden {
        state
            .errors
            .push("Broken connection retained a force glyph".into());
    }
    if !state.saw_cleared {
        state
            .errors
            .push("Disabled force overlay retained data or glyphs".into());
    }
    for name in NAMES {
        if !directory.join(name).is_file() {
            state.errors.push(format!("Missing screenshot {name}"));
        }
    }
    let report = serde_json::json!({"asset":world.resource::<DemoOptions>().asset,"asset_id":state.asset_id,"schema_version":state.schema_version,"chunk_count":state.chunk_count,"proxy_count":state.proxy_count,"static_bonds":state.peak_static_bonds,"force_pairs":state.peak_force_pairs,"peak_force_n":state.peak_force_n,"peak_actual_impulse_ns":state.peak_impulse_ns,"force_overlay_changed_non_hud_pixels":changed,"contact_impulse_purple_pixels":purple,"broken_connection_hidden":state.saw_broken_hidden,"overlay_cleared":state.saw_cleared,"screenshots":state.captures.len(),"scene_surface_pixels":state.scene_pixels,"rejected_readbacks":state.rejected_readbacks,"errors":state.errors});
    std::fs::write(
        directory.join("report.json"),
        serde_json::to_string_pretty(&report).unwrap(),
    )
    .expect("Write force probe report");
    state.finished = true;
    if state.errors.is_empty() {
        info!("Force view probe passed: {report}");
        world.write_message(AppExit::Success);
    } else {
        error!("Force view probe failed: {report}");
        world.write_message(AppExit::error());
    }
}
