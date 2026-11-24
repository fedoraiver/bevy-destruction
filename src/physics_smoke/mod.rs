use std::collections::BTreeMap;

use avian3d::prelude::*;
use bevy::{prelude::*, render::view::screenshot::save_to_disk};

use crate::{
    DemoOptions,
    asset::{DestructibleAsset, Manifest},
    physics::Cluster,
    scene::{BondInstance, BuildingInstance, ChunkInstance, DemoCamera},
};

pub struct PhysicsSmokePlugin;

impl Plugin for PhysicsSmokePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PhysicsSmokeState>()
            .add_systems(
                Update,
                drive
                    .after(crate::camera::move_camera)
                    .after(crate::scene::instantiate_pending),
            )
            .add_systems(
                FixedPostUpdate,
                verify_split
                    .after(crate::physics::rebuild_clusters)
                    .before(PhysicsSystems::First),
            );
    }
}

#[derive(Resource, Default)]
struct PhysicsSmokeState {
    waiting: u32,
    warmup: u32,
    frame: u32,
    selected: Vec<String>,
    cleared_chunks: Vec<String>,
    pair_bonds: Vec<String>,
    initial_centers: BTreeMap<String, Vec3>,
    split_predictions: BTreeMap<String, SplitPrediction>,
    verify_split_pending: bool,
    split_continuity: bool,
    errors: Vec<String>,
    single_cluster: bool,
    split_clusters: bool,
    fell: bool,
    slept: bool,
    non_finite: bool,
    expected_mass: f32,
    settled_centers: BTreeMap<String, Vec3>,
}

#[derive(Clone, Copy)]
struct SplitPrediction {
    center: Vec3,
    rotation: Quat,
    linear: Vec3,
    angular: Vec3,
}

fn roof_pair(manifest: &Manifest) -> Option<Vec<String>> {
    let roof = manifest
        .chunks
        .iter()
        .filter(|chunk| chunk.kind == "roof")
        .filter(|chunk| !manifest.anchors.iter().any(|a| a.chunk == chunk.id))
        .map(|chunk| (chunk.id.as_str(), chunk))
        .collect::<BTreeMap<_, _>>();
    let center = roof
        .values()
        .map(|chunk| Vec3::from(chunk.position))
        .sum::<Vec3>()
        / roof.len().max(1) as f32;
    manifest
        .bonds
        .iter()
        .filter_map(|bond| {
            let a = roof.get(bond.a.as_str())?;
            let b = roof.get(bond.b.as_str())?;
            let midpoint = (Vec3::from(a.position) + Vec3::from(b.position)) * 0.5;
            Some((midpoint.distance_squared(center), &bond.a, &bond.b))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(b.1)))
        .map(|(_, a, b)| vec![a.clone(), b.clone()])
}

fn capture(
    commands: &mut Commands,
    options: &DemoOptions,
    directory: &std::path::Path,
    name: &str,
) {
    commands
        .spawn((
            Name::new("Physics smoke screenshot"),
            options.smoke_screenshot(),
        ))
        .observe(save_to_disk(directory.join(name)));
}

fn finish(
    state: &PhysicsSmokeState,
    options: &DemoOptions,
    directory: &std::path::Path,
    exit: &mut MessageWriter<AppExit>,
) {
    let report = serde_json::json!({
        "asset": options.asset,
        "selected_chunks": state.selected,
        "clearance_chunks": state.cleared_chunks,
        "expected_mass": state.expected_mass,
        "initial_centers": state.initial_centers.iter().map(|(id, center)| (id, center.to_array())).collect::<BTreeMap<_, _>>(),
        "settled_centers": state.settled_centers.iter().map(|(id, center)| (id, center.to_array())).collect::<BTreeMap<_, _>>(),
        "released_as_one_cluster": state.single_cluster,
        "split_into_two_clusters": state.split_clusters,
        "split_pose_and_velocity_preserved": state.split_continuity,
        "fell": state.fell,
        "slept": state.slept,
        "simulated_frames": state.frame,
        "errors": state.errors,
    });
    std::fs::create_dir_all(directory).expect("Create physics smoke output");
    std::fs::write(
        directory.join("report.json"),
        serde_json::to_string_pretty(&report).unwrap(),
    )
    .expect("Write physics smoke report");
    if state.errors.is_empty() {
        info!("Physics smoke test passed: {report}");
        exit.write(AppExit::Success);
    } else {
        error!("Physics smoke test failed: {report}");
        exit.write(AppExit::error());
    }
}

#[allow(clippy::type_complexity)]
fn verify_split(
    mut state: ResMut<PhysicsSmokeState>,
    bodies: Query<(
        &Cluster,
        &Position,
        &Rotation,
        &LinearVelocity,
        &AngularVelocity,
        &ComputedCenterOfMass,
    )>,
) {
    if !state.verify_split_pending {
        return;
    }
    state.verify_split_pending = false;
    let mut matched = 0;
    let mut continuous = true;
    for (cluster, position, rotation, linear, angular, center) in &bodies {
        if cluster.chunks.len() != 1 {
            continue;
        }
        let id = &cluster.chunks[0];
        let Some(&expected) = state.split_predictions.get(id) else {
            continue;
        };
        matched += 1;
        if (position.0 + rotation.0 * center.0).distance(expected.center) > 0.0001
            || !rotation.0.abs_diff_eq(expected.rotation, 0.00001)
        {
            continuous = false;
            state
                .errors
                .push(format!("Split changed world pose for {id}"));
        }
        if linear.0.distance(expected.linear) > 0.0001
            || angular.0.distance(expected.angular) > 0.0001
        {
            continuous = false;
            state
                .errors
                .push(format!("Split lost inherited velocity for {id}"));
        }
    }
    state.split_continuity = continuous && matched == state.selected.len();
    if matched != state.selected.len() {
        state
            .errors
            .push("Split lost stable chunk identities before physics".into());
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn drive(
    mut commands: Commands,
    options: Res<DemoOptions>,
    assets: Res<Assets<DestructibleAsset>>,
    mut state: ResMut<PhysicsSmokeState>,
    mut instances: Query<(Entity, &mut BuildingInstance)>,
    mut chunks: Query<&mut ChunkInstance>,
    mut bonds: Query<&mut BondInstance>,
    bodies: Query<(
        Entity,
        &Cluster,
        &RigidBody,
        &Position,
        &Rotation,
        &LinearVelocity,
        &AngularVelocity,
        &ComputedCenterOfMass,
        &ComputedMass,
        Has<Sleeping>,
        &ChildOf,
    )>,
    mut cameras: Query<&mut Transform, With<DemoCamera>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(directory) = &options.physics_smoke_output else {
        return;
    };
    state.waiting += 1;
    let Some((building, mut instance)) = instances.iter_mut().next() else {
        if state.waiting == 1800 {
            state.errors.push("Building did not appear".into());
            finish(&state, &options, directory, &mut exit);
        }
        return;
    };
    if let Some(error) = &instance.error {
        state.errors.push(format!("Building load failed: {error}"));
        finish(&state, &options, directory, &mut exit);
        return;
    }
    let Some(bounds) = instance.bounds else {
        if state.waiting == 1800 {
            state.errors.push("Building did not finish loading".into());
            finish(&state, &options, directory, &mut exit);
        }
        return;
    };
    let Some(asset) = assets.get(&instance.asset) else {
        return;
    };
    if instance.clusters.is_empty() {
        if state.waiting == 1800 {
            state
                .errors
                .push("Building has no physical clusters".into());
            finish(&state, &options, directory, &mut exit);
        }
        return;
    }
    let size = bounds.max - bounds.min;
    let center = bounds.center();
    for mut camera in &mut cameras {
        *camera = Transform::from_translation(Vec3::new(
            center.x + size.x * 0.8,
            bounds.max.y + size.y * 0.6,
            bounds.max.z + size.z * 0.9,
        ))
        .looking_at(center, Vec3::Y);
    }
    state.warmup += 1;
    if state.warmup <= 150 {
        return;
    }
    state.frame += 1;

    let building_bodies = || {
        bodies
            .iter()
            .filter(|(.., parent)| parent.parent() == building)
    };
    let selected_bodies = building_bodies()
        .filter(|(_, cluster, ..)| cluster.chunks.iter().any(|id| state.selected.contains(id)))
        .collect::<Vec<_>>();
    if !state.non_finite
        && building_bodies().any(|(_, _, _, p, r, v, w, c, m, ..)| {
            !p.0.is_finite()
                || !r.0.is_finite()
                || !v.0.is_finite()
                || !w.0.is_finite()
                || !c.0.is_finite()
                || !m.value().is_finite()
        })
    {
        state.non_finite = true;
        state
            .errors
            .push("A cluster developed non-finite physics state".into());
    }

    match state.frame {
        10 => {
            std::fs::create_dir_all(directory).expect("Create physics smoke output");
            capture(&mut commands, &options, directory, "01-supported.png");
        }
        20 => {
            let Some(selected) = roof_pair(&asset.manifest) else {
                state
                    .errors
                    .push("Asset has no bonded nonanchored roof pair".into());
                finish(&state, &options, directory, &mut exit);
                return;
            };
            state.expected_mass = selected
                .iter()
                .map(|id| {
                    chunks
                        .get(instance.chunks[id])
                        .unwrap()
                        .mass_properties
                        .mass
                })
                .sum();
            info!(
                "Physics smoke: releasing roof pair {selected:?}; existing clusters={:?}",
                building_bodies()
                    .map(|body| (body.0, body.1.anchored, body.1.chunks.len()))
                    .collect::<Vec<_>>()
            );
            for id in &selected {
                let chunk = chunks.get(instance.chunks[id]).unwrap();
                state
                    .initial_centers
                    .insert(id.clone(), chunk.center_in_building());
            }
            state.cleared_chunks = asset
                .manifest
                .bonds
                .iter()
                .filter_map(|bond| {
                    let a = selected.contains(&bond.a);
                    let b = selected.contains(&bond.b);
                    (a != b).then_some(if a { &bond.b } else { &bond.a })
                })
                .filter(|id| {
                    !asset
                        .manifest
                        .anchors
                        .iter()
                        .any(|anchor| anchor.chunk == **id)
                })
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            for id in &state.cleared_chunks {
                let entity = instance.chunks[id];
                chunks.get_mut(entity).unwrap().alive = false;
                commands
                    .entity(entity)
                    .insert((crate::scene::RemovedChunk, Visibility::Hidden));
            }
            for bond in &asset.manifest.bonds {
                let a = selected.contains(&bond.a);
                let b = selected.contains(&bond.b);
                if a != b
                    || state.cleared_chunks.contains(&bond.a)
                    || state.cleared_chunks.contains(&bond.b)
                {
                    bonds.get_mut(instance.bonds[&bond.id]).unwrap().active = false;
                } else if a {
                    state.pair_bonds.push(bond.id.clone());
                }
            }
            instance
                .detail_dirty
                .extend(state.cleared_chunks.iter().cloned());
            instance.topology_dirty = true;
            state.selected = selected;
        }
        23 => {
            state.single_cluster = selected_bodies.len() == 1
                && !selected_bodies[0].1.anchored
                && *selected_bodies[0].2 == RigidBody::Dynamic
                && selected_bodies[0].1.chunks.len() == 2;
            info!(
                "Physics smoke: released pair valid={}, bodies={:?}",
                state.single_cluster,
                selected_bodies
                    .iter()
                    .map(|body| body.0)
                    .collect::<Vec<_>>()
            );
            if !state.single_cluster {
                state
                    .errors
                    .push("Unsupported roof pair did not become one dynamic cluster".into());
            }
            let mass: f32 = selected_bodies.iter().map(|body| body.8.value()).sum();
            if (mass - state.expected_mass).abs() > state.expected_mass * 0.0001 {
                state
                    .errors
                    .push("Released cluster has incorrect authored mass".into());
            }
        }
        38 => {
            state.fell = selected_bodies.iter().any(|body| {
                let center = body.3.0 + body.4.0 * body.7.0;
                let original = state.initial_centers.values().map(|p| p.y).sum::<f32>()
                    / state.initial_centers.len() as f32;
                center.y < original - 0.10
            });
            if !state.fell {
                state
                    .errors
                    .push("Unsupported cluster did not fall under gravity".into());
            }
            capture(&mut commands, &options, directory, "02-falling.png");
        }
        40 => {
            info!(
                "Physics smoke: splitting the falling pair; existing clusters={:?}",
                building_bodies()
                    .map(|body| (body.0, body.1.anchored, body.1.chunks.len()))
                    .collect::<Vec<_>>()
            );
            for (_, cluster, _, position, rotation, linear, angular, center, ..) in &selected_bodies
            {
                let parent_center = position.0 + rotation.0 * center.0;
                for id in &cluster.chunks {
                    if state.selected.contains(id) {
                        let local = chunks
                            .get(instance.chunks[id])
                            .unwrap()
                            .center_in_building();
                        let point = position.0 + rotation.0 * local;
                        let velocity = linear.0 + angular.0.cross(point - parent_center);
                        state.split_predictions.insert(
                            id.clone(),
                            SplitPrediction {
                                center: point,
                                rotation: rotation.0,
                                linear: velocity,
                                angular: angular.0,
                            },
                        );
                    }
                }
            }
            for bond in &state.pair_bonds {
                bonds.get_mut(instance.bonds[bond]).unwrap().active = false;
            }
            instance.topology_dirty = true;
            state.verify_split_pending = true;
        }
        42 => {
            state.split_clusters = selected_bodies.len() == 2
                && selected_bodies.iter().all(|body| {
                    !body.1.anchored && *body.2 == RigidBody::Dynamic && body.1.chunks.len() == 1
                });
            if !state.split_clusters {
                state
                    .errors
                    .push("Falling pair did not split into two dynamic clusters".into());
            }
            if state.verify_split_pending || !state.split_continuity {
                state
                    .errors
                    .push("Split continuity was not verified before physics".into());
            }
            let mass: f32 = selected_bodies.iter().map(|body| body.8.value()).sum();
            if (mass - state.expected_mass).abs() > state.expected_mass * 0.0001 {
                state
                    .errors
                    .push("Splitting changed surviving authored mass".into());
            }
        }
        900 => {
            state.slept = selected_bodies.len() == 2 && selected_bodies.iter().all(|body| body.9);
            if !state.slept {
                state
                    .errors
                    .push("Detached chunks did not reach physical sleep".into());
            }
            for body in &selected_bodies {
                let center = body.3.0 + body.4.0 * body.7.0;
                if center.y < -0.1 || body.5.0.length() > 0.1 || body.6.0.length() > 0.1 {
                    state
                        .errors
                        .push("Detached chunk did not settle above the ground".into());
                }
                if let Some(id) = body.1.chunks.first() {
                    state.settled_centers.insert(id.clone(), center);
                }
            }
            for (_, cluster, kind, position, rotation, ..) in building_bodies() {
                if cluster.anchored
                    && (*kind != RigidBody::Static
                        || position.0.length() > 0.0001
                        || rotation.0.angle_between(Quat::IDENTITY) > 0.0001)
                {
                    state
                        .errors
                        .push("Supported building moved while debris fell".into());
                }
            }
            capture(&mut commands, &options, directory, "03-settled.png");
        }
        930 => {
            for name in ["01-supported.png", "02-falling.png", "03-settled.png"] {
                if !directory.join(name).is_file() {
                    state
                        .errors
                        .push(format!("Missing physics screenshot {name}"));
                }
            }
            finish(&state, &options, directory, &mut exit);
        }
        _ => {}
    }
}
