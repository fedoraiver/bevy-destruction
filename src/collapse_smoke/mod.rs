use std::collections::{BTreeMap, BTreeSet};

use avian3d::{collision::contact_types::ContactGraph, prelude::*};
use bevy::{prelude::*, render::view::screenshot::save_to_disk};

use crate::{
    DemoOptions,
    asset::{DestructibleAsset, Manifest},
    debug::FractureEdges,
    diagnostics::RunMetrics,
    physics::Cluster,
    scene::{BuildingInstance, DemoCamera, RemovedChunk},
    stress_runtime::StressState,
};

pub struct CollapseSmokePlugin;

#[cfg(test)]
mod tests;

impl Plugin for CollapseSmokePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CollapseState>().add_systems(
            Update,
            drive
                .after(crate::camera::move_camera)
                .after(crate::scene::instantiate_pending),
        );
    }
}

const RECTANGLE: &str = "buildings/rectangle.destruction.json";
const CAPTURES: [&str; 4] = [
    "01-intact-rectangle.png",
    "02-narrow-support.png",
    "03-bulk-tipping.png",
    "04-final-collapse.png",
];

fn contacted_static_support<'a>(
    bulk: Entity,
    supports: &[(&'a str, Entity, Entity)],
    contacts: impl IntoIterator<Item = ([Option<Entity>; 2], [Entity; 2], f32)>,
) -> Option<&'a str> {
    contacts
        .into_iter()
        .filter(|(_, _, impulse)| *impulse > 0.0)
        .find_map(|(bodies, colliders, _)| {
            if !bodies.contains(&Some(bulk)) {
                return None;
            }
            supports.iter().find_map(|&(id, collider, body)| {
                (bodies.contains(&Some(body)) && colliders.contains(&collider)).then_some(id)
            })
        })
}

pub(crate) struct CollapseSelection {
    pub removed: Vec<String>,
    pub support_path: Vec<String>,
    pub static_stub: String,
    pub upper_floor_y: f32,
}

pub(crate) fn select_collapse_support(
    manifest: &Manifest,
    mass: impl Fn(&str) -> f32,
) -> Result<CollapseSelection, String> {
    let records = manifest
        .chunks
        .iter()
        .map(|chunk| (chunk.id.as_str(), chunk))
        .collect::<BTreeMap<_, _>>();
    let bounds = manifest
        .chunks
        .iter()
        .map(|chunk| {
            let transform = chunk.transform();
            let mut min = Vec3::splat(f32::INFINITY);
            let mut max = Vec3::splat(f32::NEG_INFINITY);
            for point in chunk.colliders.iter().flat_map(|proxy| &proxy.vertices) {
                let point = transform.transform_point((*point).into());
                min = min.min(point);
                max = max.max(point);
            }
            (chunk.id.as_str(), (min, max))
        })
        .collect::<BTreeMap<_, _>>();
    let bottom = manifest
        .chunks
        .iter()
        .filter(|chunk| chunk.kind == "floor")
        .map(|chunk| bounds[chunk.id.as_str()].0.y)
        .min_by(f32::total_cmp)
        .ok_or("No foundation floor geometry")?;
    let foundation = manifest
        .chunks
        .iter()
        .filter(|chunk| {
            chunk.kind == "floor" && (bounds[chunk.id.as_str()].0.y - bottom).abs() < 1e-4
        })
        .map(|chunk| chunk.id.as_str())
        .collect::<BTreeSet<_>>();
    let foundation_top = foundation
        .iter()
        .map(|id| bounds[id].1.y)
        .max_by(f32::total_cmp)
        .unwrap();
    let upper_floor_y = manifest
        .chunks
        .iter()
        .filter(|chunk| {
            chunk.kind == "floor" && bounds[chunk.id.as_str()].0.y > foundation_top + 1e-4
        })
        .map(|chunk| bounds[chunk.id.as_str()].0.y)
        .min_by(f32::total_cmp)
        .ok_or("No upper storey floor geometry")?;
    let supports = manifest
        .chunks
        .iter()
        .filter(|chunk| {
            let (min, max) = bounds[chunk.id.as_str()];
            chunk.kind != "floor"
                && chunk.kind != "roof"
                && min.y < upper_floor_y - 1e-4
                && max.y > foundation_top + 1e-4
        })
        .map(|chunk| chunk.id.as_str())
        .collect::<BTreeSet<_>>();
    let upper_floor = manifest
        .chunks
        .iter()
        .filter(|chunk| {
            chunk.kind == "floor" && (bounds[chunk.id.as_str()].0.y - upper_floor_y).abs() < 1e-4
        })
        .map(|chunk| chunk.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut adjacent = records
        .keys()
        .map(|&id| (id, BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    for bond in &manifest.bonds {
        adjacent
            .get_mut(bond.a.as_str())
            .unwrap()
            .insert(bond.b.as_str());
        adjacent
            .get_mut(bond.b.as_str())
            .unwrap()
            .insert(bond.a.as_str());
    }
    let mut grounded = manifest
        .anchors
        .iter()
        .filter(|anchor| anchor.kind == "fixed" && foundation.contains(anchor.chunk.as_str()))
        .map(|anchor| anchor.chunk.as_str())
        .collect::<BTreeSet<_>>();
    let mut pending = grounded.iter().copied().collect::<Vec<_>>();
    while let Some(id) = pending.pop() {
        for &neighbor in adjacent[id].intersection(&foundation) {
            if grounded.insert(neighbor) {
                pending.push(neighbor);
            }
        }
    }
    let starts = supports
        .iter()
        .copied()
        .filter(|id| {
            adjacent[id]
                .iter()
                .any(|neighbor| grounded.contains(neighbor))
                && !adjacent[id]
                    .iter()
                    .any(|neighbor| upper_floor.contains(neighbor))
        })
        .collect::<Vec<_>>();
    let ends = supports
        .iter()
        .copied()
        .filter(|id| {
            adjacent[id]
                .iter()
                .any(|neighbor| upper_floor.contains(neighbor))
        })
        .collect::<BTreeSet<_>>();
    let mut low = Vec3::splat(f32::INFINITY);
    let mut high = Vec3::splat(f32::NEG_INFINITY);
    for &(min, max) in bounds.values() {
        low = low.min(min);
        high = high.max(max);
    }
    let target = (low + high) * 0.5 - Vec3::Z * 0.5;
    let mut candidates = Vec::new();
    for start in starts {
        let mut remaining = supports.clone();
        let mut costs = BTreeMap::from([(start, f64::from(mass(start)))]);
        let mut previous = BTreeMap::new();
        while let Some(id) = remaining
            .iter()
            .copied()
            .filter(|id| costs.contains_key(id))
            .min_by(|a, b| costs[a].total_cmp(&costs[b]).then(a.cmp(b)))
        {
            remaining.remove(id);
            if ends.contains(id) {
                let mut path = vec![id.to_owned()];
                let mut cursor = id;
                while cursor != start {
                    cursor = previous[cursor];
                    path.push(cursor.to_owned());
                }
                path.reverse();
                if path.len() >= 2 {
                    let (min, max) = bounds[start];
                    let interior = min.x > low.x + 1e-4
                        && max.x < high.x - 1e-4
                        && min.z > low.z + 1e-4
                        && max.z < high.z - 1e-4;
                    let center = records[start]
                        .transform()
                        .transform_point(records[start].center_of_mass.into());
                    let distance = (center.x - target.x).powi(2) + (center.z - target.z).powi(2);
                    candidates.push((!interior, distance, costs[id], path));
                }
                break;
            }
            for &neighbor in adjacent[id].intersection(&remaining) {
                let cost = costs[id] + f64::from(mass(neighbor));
                if costs.get(neighbor).is_none_or(|old| cost < *old) {
                    costs.insert(neighbor, cost);
                    previous.insert(neighbor, id);
                }
            }
        }
    }
    candidates.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.total_cmp(&b.1))
            .then(a.2.total_cmp(&b.2))
            .then(a.3.cmp(&b.3))
    });
    let (_, _, _, support_path) = candidates
        .into_iter()
        .next()
        .ok_or("No multi-piece foundation-to-upperfloor support path")?;
    let retained = support_path
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    Ok(CollapseSelection {
        removed: supports
            .into_iter()
            .filter(|id| !retained.contains(id))
            .map(str::to_owned)
            .collect(),
        static_stub: support_path[0].clone(),
        support_path,
        upper_floor_y,
    })
}

#[derive(Resource, Default)]
struct CollapseState {
    initialized: bool,
    finished: bool,
    waiting: u32,
    warmup: u32,
    frame: u32,
    upper: BTreeMap<String, (Vec3, f32)>,
    upper_mass: f32,
    support_path: Vec<String>,
    static_stub: String,
    removed: Vec<String>,
    initial_pose: Transform,
    expected_mass: f64,
    final_mass: f64,
    max_mass_error: f64,
    finite: bool,
    before_broken: u64,
    strength_breaks: u64,
    stress_failures: u64,
    peak_invalid_reports: usize,
    final_invalid_reports: usize,
    final_stress_reports: Vec<serde_json::Value>,
    first_release: Option<u32>,
    first_tipping: Option<u32>,
    largest_released_upper: usize,
    max_drop: f32,
    max_tilt: f32,
    max_downward_speed: f32,
    bulk_fell: bool,
    residual_support_contact: bool,
    tipping_sample: Option<serde_json::Value>,
    errors: Vec<String>,
}

fn capture(commands: &mut Commands, options: &DemoOptions, name: &str) {
    if !options.profile {
        commands
            .spawn((
                Name::new("Collapse smoke screenshot"),
                options.smoke_screenshot(),
            ))
            .observe(save_to_disk(
                options.collapse_smoke_output.as_ref().unwrap().join(name),
            ));
    }
}

fn finish(
    state: &mut CollapseState,
    options: &DemoOptions,
    metrics: &RunMetrics,
    exit: &mut MessageWriter<AppExit>,
) {
    let directory = options.collapse_smoke_output.as_ref().unwrap();
    let report = serde_json::json!({
        "asset":options.asset,
        "scenario":"Eccentric support: remove other first-storey supports while retaining a geometry-selected multi-piece load path",
        "selected_support_path":state.support_path,
        "removed_chunks":state.removed.len(),
        "original_upper_chunks":state.upper.len(),
        "expected_retained_mass_kg":state.expected_mass,
        "final_retained_mass_kg":state.final_mass,
        "max_mass_error_kg":state.max_mass_error,
        "finite_body_state":state.finite,
        "static_strength_broken_bonds":state.strength_breaks,
        "stress_failures":state.stress_failures,
        "peak_invalid_stress_reports":state.peak_invalid_reports,
        "final_invalid_stress_reports":state.final_invalid_reports,
        "final_stress_reports":state.final_stress_reports,
        "first_bulk_release_frame":state.first_release,
        "first_bulk_tipping_frame":state.first_tipping,
        "largest_released_upper_chunk_count":state.largest_released_upper,
        "max_bulk_drop_m":state.max_drop,
        "max_bulk_tilt_rad":state.max_tilt,
        "max_bulk_downward_speed_m_per_s":state.max_downward_speed,
        "bulk_fell":state.bulk_fell,
        "residual_support_contact":state.residual_support_contact,
        "residual_support_chunk":state.static_stub,
        "tipping_sample":state.tipping_sample,
        "simulated_frames_after_warmup":state.frame,
        "profile":options.profile,
        "metrics":metrics.json(),
        "errors":state.errors,
    });
    std::fs::write(
        directory.join("report.json"),
        serde_json::to_string_pretty(&report).unwrap(),
    )
    .expect("Write collapse report");
    state.finished = true;
    if state.errors.is_empty() {
        info!("Collapse smoke passed: {report}");
        exit.write(AppExit::Success);
    } else {
        error!("Collapse smoke failed: {report}");
        exit.write(AppExit::error());
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn drive(
    mut commands: Commands,
    options: Res<DemoOptions>,
    assets: Res<Assets<DestructibleAsset>>,
    mut state: ResMut<CollapseState>,
    mut instances: Query<(
        Entity,
        &mut BuildingInstance,
        &GlobalTransform,
        Option<&StressState>,
    )>,
    bodies: Query<(
        &Cluster,
        &RigidBody,
        &Position,
        &Rotation,
        &LinearVelocity,
        &AngularVelocity,
        &ComputedMass,
        &ChildOf,
    )>,
    mut cameras: Query<&mut Transform, With<DemoCamera>>,
    mut metrics: ResMut<RunMetrics>,
    contacts: Res<ContactGraph>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(directory) = &options.collapse_smoke_output else {
        return;
    };
    if state.finished {
        return;
    }
    if !state.initialized {
        std::fs::create_dir_all(directory).expect("Create collapse output");
        state.initialized = true;
        state.finite = true;
        if options.asset != RECTANGLE {
            state
                .errors
                .push("Collapse validation requires --scene rectangle".into());
            finish(&mut state, &options, &metrics, &mut exit);
            return;
        }
    }
    state.waiting += 1;
    if state.waiting > 1800 && state.warmup < 150 {
        state
            .errors
            .push("Rectangle or initial static solution did not finish loading".into());
        finish(&mut state, &options, &metrics, &mut exit);
        return;
    }
    let Some((building, mut instance, root_global, stress)) = instances.iter_mut().next() else {
        return;
    };
    if let Some(error) = &instance.error {
        state.errors.push(format!("Rectangle load failed: {error}"));
        finish(&mut state, &options, &metrics, &mut exit);
        return;
    }
    let (Some(bounds), Some(stress)) = (instance.bounds, stress) else {
        return;
    };
    if instance.clusters.is_empty() || stress.solves == 0 {
        return;
    }
    let Some(asset) = assets.get(&instance.asset) else {
        return;
    };
    if state.warmup < 150 {
        state.warmup += 1;
        return;
    }
    state.frame += 1;
    metrics.phase = state.frame;
    for mut camera in &mut cameras {
        let center = bounds.center();
        *camera = Transform::from_translation(Vec3::new(
            bounds.min.x - 7.0,
            bounds.max.y * 0.65,
            bounds.max.z + 10.0,
        ))
        .looking_at(Vec3::new(center.x, bounds.max.y * 0.4, center.z), Vec3::Y);
    }
    state.stress_failures = stress.failures;
    state.final_invalid_reports = stress.reports.iter().filter(|report| !report.valid).count();
    state.peak_invalid_reports = state.peak_invalid_reports.max(state.final_invalid_reports);
    state.final_stress_reports = stress
        .reports
        .iter()
        .map(|report| {
            serde_json::json!({
                "chunks":report.chunks,"valid":report.valid,"iterations":report.iterations,
                "relative_residual":report.relative_residual,"error":report.error,
                "force_balance_error_n":report.force_balance_error,
                "moment_balance_error_nm":report.moment_balance_error,
                "max_displacement_m":report.max_displacement,
            })
        })
        .collect();
    if state.frame > 20 && !instance.topology_dirty {
        state.strength_breaks = stress.broken_total.saturating_sub(state.before_broken);
        let mut total_mass = 0.0_f64;
        for (cluster, kind, position, rotation, linear, angular, mass, parent) in &bodies {
            if parent.parent() != building {
                continue;
            }
            state.finite &= position.0.is_finite()
                && rotation.0.is_finite()
                && linear.0.is_finite()
                && angular.0.is_finite()
                && mass.value().is_finite();
            total_mass += f64::from(mass.value());
            if cluster.anchored || *kind != RigidBody::Dynamic {
                continue;
            }
            let mut upper_chunks = 0;
            let mut upper_mass = 0.0_f32;
            let mut weighted_center = Vec3::ZERO;
            for id in &cluster.chunks {
                if let Some(&(center, mass)) = state.upper.get(id) {
                    upper_chunks += 1;
                    upper_mass += mass;
                    weighted_center += center * mass;
                }
            }
            if upper_chunks < 2 || upper_mass < state.upper_mass * 0.25 {
                continue;
            }
            let frame = state.frame;
            state.first_release.get_or_insert(frame);
            state.largest_released_upper = state.largest_released_upper.max(upper_chunks);
            let body = instance.chunks[&cluster.chunks[0]].cluster.unwrap();
            let static_supports = state
                .support_path
                .iter()
                .filter_map(|id| {
                    let chunk = &instance.chunks[id.as_str()];
                    if !chunk.alive {
                        return None;
                    }
                    let support = chunk.cluster?;
                    bodies
                        .get(support)
                        .is_ok_and(|entry| *entry.1 == RigidBody::Static)
                        .then_some((id.as_str(), chunk.entity, support))
                })
                .collect::<Vec<_>>();
            if let Some(stub) = contacted_static_support(
                body,
                &static_supports,
                contacts.iter_active_touching().map(|pair| {
                    (
                        [pair.body1, pair.body2],
                        [pair.collider1, pair.collider2],
                        pair.total_normal_impulse_magnitude(),
                    )
                }),
            ) {
                let stub = stub.to_owned();
                state.residual_support_contact = true;
                state.static_stub = stub;
            }
            let rest_center = weighted_center / upper_mass;
            let center = position.0 + rotation.0 * rest_center;
            let original_center = state.initial_pose.transform_point(rest_center);
            let drop = original_center.y - center.y;
            let tilt = (rotation.0 * Vec3::Y)
                .dot(state.initial_pose.rotation * Vec3::Y)
                .clamp(-1.0, 1.0)
                .acos();
            state.max_drop = state.max_drop.max(drop);
            state.max_tilt = state.max_tilt.max(tilt);
            state.max_downward_speed = state.max_downward_speed.max(-linear.0.y);
            state.bulk_fell |= drop > 0.5 && linear.0.y < -0.25;
            if state.first_tipping.is_none() && tilt > 0.1 && drop > 0.1 {
                state.first_tipping = Some(state.frame);
                state.tipping_sample = Some(serde_json::json!({
                    "frame":state.frame,"cluster_chunks":cluster.chunks.len(),
                    "upper_chunks":upper_chunks,"upper_mass_kg":upper_mass,
                    "original_upper_center":original_center.to_array(),
                    "upper_center":center.to_array(),"drop_m":drop,"tilt_rad":tilt,
                    "linear_velocity":linear.0.to_array(),"angular_velocity":angular.0.to_array(),
                }));
                capture(&mut commands, &options, CAPTURES[2]);
            }
        }
        state.final_mass = total_mass;
        state.max_mass_error = state
            .max_mass_error
            .max((total_mass - state.expected_mass).abs());
    }
    match state.frame {
        10 => {
            if stress.broken_total != 0 || state.final_invalid_reports != 0 {
                state
                    .errors
                    .push("Intact rectangle was unstable before scripted removal".into());
            }
            capture(&mut commands, &options, CAPTURES[0]);
        }
        20 => {
            let selection = match select_collapse_support(&asset.manifest, |id| {
                instance.chunks[id].mass_properties.mass
            }) {
                Ok(selection) => selection,
                Err(error) => {
                    state.errors.push(error);
                    finish(&mut state, &options, &metrics, &mut exit);
                    return;
                }
            };
            state.initial_pose = root_global.compute_transform();
            state.before_broken = stress.broken_total;
            state.upper = asset
                .manifest
                .chunks
                .iter()
                .filter(|record| {
                    record
                        .transform()
                        .transform_point(record.center_of_mass.into())
                        .y
                        >= selection.upper_floor_y
                })
                .map(|record| {
                    let chunk = &instance.chunks[&record.id];
                    (
                        record.id.clone(),
                        (chunk.center_in_building(), chunk.mass_properties.mass),
                    )
                })
                .collect();
            state.upper_mass = state.upper.values().map(|(_, mass)| mass).sum();
            state.removed = selection.removed;
            state.support_path = selection.support_path;
            state.static_stub = selection.static_stub;
            state.expected_mass = instance
                .chunks
                .iter()
                .filter(|(id, chunk)| chunk.alive && !state.removed.contains(*id))
                .map(|(_, chunk)| f64::from(chunk.mass_properties.mass))
                .sum();
            for id in &state.removed {
                let chunk = instance.chunks.get_mut(id).unwrap();
                chunk.alive = false;
                commands
                    .entity(chunk.entity)
                    .insert((RemovedChunk, Visibility::Hidden))
                    .remove::<FractureEdges>();
                for &primitive in &chunk.primitives {
                    commands.entity(primitive).despawn();
                }
            }
            for bond in &asset.manifest.bonds {
                if state.removed.contains(&bond.a) || state.removed.contains(&bond.b) {
                    instance.bonds.insert(bond.id.clone(), false);
                }
            }
            instance.topology_dirty = true;
            let damage = serde_json::json!({
                "asset":options.asset,"frame_after_warmup":state.frame,
                "predicate":"foundation-adjacent interior start, minimum-mass multi-piece support path to the next floor",
                "retained_support_path":state.support_path,"static_stub_candidate":state.static_stub,
                "removed_ids":state.removed,"retained_chunks":instance.chunks.len()-state.removed.len(),
                "expected_retained_mass_kg":state.expected_mass,
            });
            std::fs::write(
                directory.join("damage.json"),
                serde_json::to_string_pretty(&damage).unwrap(),
            )
            .expect("Write collapse damage replay");
            capture(&mut commands, &options, CAPTURES[1]);
        }
        330 if state.first_tipping.is_none() => {
            capture(&mut commands, &options, CAPTURES[2]);
        }
        360 => capture(&mut commands, &options, CAPTURES[3]),
        390 => {
            if state.strength_breaks == 0 {
                state
                    .errors
                    .push("Narrow support did not fail from calculated static strength".into());
            }
            if state.first_release.is_none() || !state.bulk_fell {
                state.errors.push(
                    "No coherent released body with at least a quarter of the original upper mass fell".into(),
                );
            }
            if state.first_tipping.is_none() {
                state.errors.push(
                    "Bulk upper structure did not tilt more than 0.1 radians while falling".into(),
                );
            }
            if !state.residual_support_contact {
                state.errors.push("Bulk upper structure never exchanged contact impulse with its surviving static support".into());
            }
            if state.final_invalid_reports != 0 || state.final_stress_reports.is_empty() {
                state
                    .errors
                    .push("Final anchored structure has no valid static stress solution".into());
            }
            if !state.finite {
                state
                    .errors
                    .push("Collapse produced a non-finite body state".into());
            }
            if state.max_mass_error > state.expected_mass * 1e-5 {
                state
                    .errors
                    .push("Collapse changed total surviving rigid-body mass".into());
            }
            for name in CAPTURES {
                if !options.profile && !directory.join(name).is_file() {
                    state.errors.push(format!("Missing screenshot {name}"));
                }
            }
            finish(&mut state, &options, &metrics, &mut exit);
        }
        _ => {}
    }
}
