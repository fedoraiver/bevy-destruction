use std::collections::BTreeMap;

use avian3d::prelude::{
    AngularInertia, CenterOfMass, GravityScale, Mass, NoAutoAngularInertia, NoAutoCenterOfMass,
    NoAutoMass, Position, RigidBody, Rotation,
};
use bevy::{
    asset::RenderAssetUsages,
    mesh::PrimitiveTopology,
    prelude::*,
    render::view::{
        DebandDither, Tonemapping,
        screenshot::{Screenshot, ScreenshotCaptured},
    },
};

use crate::{
    DemoOptions,
    asset::{BondPatchRecord, ColliderRecord, DestructibleAsset},
    force_debug::ForceViewStats,
    force_samples::{DynamicForceSample, ForceCapture, ForceSamples, bond_signature},
    physics::Cluster,
    projectiles::{ImpactStats, LaunchRequest, LaunchRequests, ProjectileShape},
    scene::{BuildingBounds, BuildingInstance, ChunkIdentity, ChunkInstance, DemoCamera},
    stress::BondStress,
    stress_runtime::StressState,
};

#[cfg(test)]
mod tests;

const NAMES: [&str; 8] = [
    "01-overlay-off.png",
    "02-structural-forces.png",
    "03-broken-connection-hidden.png",
    "04-contact-impulse.png",
    "05-overlay-cleared.png",
    "06-visibility-source-ids.png",
    "07-visibility-filtered-forces.png",
    "08-visibility-cleared-repeat.png",
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
    rejected_readbacks: [u32; 8],
    peak_static_bonds: usize,
    peak_force_pairs: usize,
    peak_force_n: f64,
    peak_impulse_ns: f64,
    saw_broken_hidden: bool,
    saw_cleared: bool,
    fixture: Option<VisibilityFixture>,
    visibility_stats: Option<ForceViewStats>,
    visibility_peak_rays: usize,
    errors: Vec<String>,
}

const FIXTURE_ORIGIN: Vec3 = Vec3::new(100.0, 40.0, 0.0);
const FIXTURE_DISTANCE: f32 = 16.0;
const TARGETS: [(&str, f32); 5] = [
    ("front", -8.0),
    ("rear", -3.0),
    ("window", 2.0),
    ("partial", 7.0),
    ("offscreen", 30.0),
];
const TARGET_COLORS: [[u8; 3]; 10] = [
    [190, 65, 65],
    [160, 40, 40],
    [65, 190, 65],
    [40, 160, 40],
    [65, 65, 190],
    [40, 40, 160],
    [190, 190, 65],
    [160, 160, 40],
    [190, 65, 190],
    [160, 40, 160],
];

struct VisibilityFixture {
    building: Entity,
    body: Entity,
    chunks: Vec<String>,
    bonds: BTreeMap<String, BondStress>,
    signature: u64,
}

struct FixtureShape {
    id: String,
    boxes: Vec<(Vec3, Vec3)>,
    color: [u8; 3],
}

fn target_id(index: usize) -> String {
    format!(
        "{}-{}",
        TARGETS[index / 2].0,
        if index.is_multiple_of(2) { "a" } else { "b" }
    )
}

fn expected_target_visible(index: usize) -> bool {
    matches!(index, 0 | 1 | 4 | 5 | 7)
}

fn visibility_shapes() -> Vec<FixtureShape> {
    let mut shapes = Vec::new();
    for (tile, &(_, x)) in TARGETS.iter().enumerate() {
        for side in 0..2 {
            shapes.push(FixtureShape {
                id: target_id(tile * 2 + side),
                boxes: vec![(
                    Vec3::new(x + if side == 0 { -0.6 } else { 0.6 }, 0.0, 0.0),
                    Vec3::new(1.2, 3.2, 0.16),
                )],
                color: TARGET_COLORS[tile * 2 + side],
            });
        }
    }
    // Foreground rectangles are specified in the target plane, then projected
    // onto z=2 along the same perspective rays. The partial mask covers BOTH COMs.
    let foreground = |x: f32, y: f32, width: f32, height: f32| {
        let k = (FIXTURE_DISTANCE - 2.0) / FIXTURE_DISTANCE;
        (
            Vec3::new(x * k, y * k, 2.0),
            Vec3::new(width * k, height * k, 0.08),
        )
    };
    for (id, boxes) in [
        ("rear-occluder", vec![foreground(-3.0, 0.0, 3.2, 4.0)]),
        (
            "window-frame",
            vec![
                foreground(2.0 - 1.2, 0.0, 0.7, 4.0),
                foreground(2.0 + 1.2, 0.0, 0.7, 4.0),
                foreground(2.0, -1.55, 1.7, 0.9),
                foreground(2.0, 1.55, 1.7, 0.9),
            ],
        ),
        (
            "partial-occluder",
            vec![foreground(7.0 - 0.275, 0.0, 2.25, 4.0)],
        ),
    ] {
        shapes.push(FixtureShape {
            id: id.into(),
            boxes,
            color: [112, 115, 119],
        });
    }
    shapes
}

fn box_triangles(center: Vec3, size: Vec3) -> Vec<[Vec3; 3]> {
    let half = size * 0.5;
    let corners = [
        Vec3::new(-1.0, -1.0, -1.0),
        Vec3::new(1.0, -1.0, -1.0),
        Vec3::new(1.0, 1.0, -1.0),
        Vec3::new(-1.0, 1.0, -1.0),
        Vec3::new(-1.0, -1.0, 1.0),
        Vec3::new(1.0, -1.0, 1.0),
        Vec3::new(1.0, 1.0, 1.0),
        Vec3::new(-1.0, 1.0, 1.0),
    ]
    .map(|v| center + v * half);
    [
        [0, 3, 2, 1],
        [4, 5, 6, 7],
        [0, 1, 5, 4],
        [3, 7, 6, 2],
        [0, 4, 7, 3],
        [1, 2, 6, 5],
    ]
    .into_iter()
    .flat_map(|[a, b, c, d]| {
        [
            [corners[a], corners[b], corners[c]],
            [corners[a], corners[c], corners[d]],
        ]
    })
    .collect()
}

fn triangle_mesh(triangles: &[[Vec3; 3]]) -> Mesh {
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        triangles
            .iter()
            .flatten()
            .map(|v| v.to_array())
            .collect::<Vec<_>>(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        triangles
            .iter()
            .flat_map(|t| {
                let normal = (t[1] - t[0]).cross(t[2] - t[0]).normalize().to_array();
                [normal; 3]
            })
            .collect::<Vec<_>>(),
    )
}

fn spawn_visibility_fixture(world: &mut World) -> VisibilityFixture {
    world
        .resource_mut::<crate::camera::CameraControl>()
        .captured = false;
    let previous = world
        .query::<(Entity, &BuildingInstance)>()
        .iter(world)
        .map(|(entity, _)| entity)
        .collect::<Vec<_>>();
    for entity in previous {
        world.entity_mut(entity).insert(Visibility::Hidden);
    }
    let building = world
        .spawn((
            Name::new("Force visibility fixture"),
            Transform::IDENTITY,
            Visibility::Visible,
        ))
        .id();
    let body = world
        .spawn((
            Name::new("Frozen diagnostic force body"),
            RigidBody::Dynamic,
            Position(FIXTURE_ORIGIN),
            Rotation::IDENTITY,
            GravityScale(0.0),
            Mass(100.0),
            CenterOfMass(Vec3::ZERO),
            AngularInertia::from_mat3_unchecked(Mat3::IDENTITY),
            Transform::from_translation(FIXTURE_ORIGIN),
            Visibility::Visible,
            ChildOf(building),
            NoAutoMass,
            NoAutoAngularInertia,
            NoAutoCenterOfMass,
        ))
        .id();
    let mut manifest = crate::stress_smoke::beam_manifest();
    manifest.asset_id = "f".repeat(64);
    manifest.name = "Frozen source-triangle force visibility fixture".into();
    manifest.anchors.clear();
    let chunk_template = manifest.chunks[0].clone();
    let bond_template = manifest.bonds[0].clone();
    manifest.chunks.clear();
    manifest.bonds.clear();
    let mut instance = BuildingInstance::pending(Handle::default());
    instance.bounds = Some(BuildingBounds {
        min: Vec3::new(-10.0, -2.0, -0.08),
        max: Vec3::new(32.0, 2.0, 2.08),
    });
    for shape in visibility_shapes() {
        let triangles = shape
            .boxes
            .iter()
            .flat_map(|&(center, size)| box_triangles(center, size))
            .collect::<Vec<_>>();
        let volume = shape
            .boxes
            .iter()
            .map(|(_, size)| size.element_product())
            .sum::<f32>();
        let center = shape
            .boxes
            .iter()
            .map(|(center, size)| *center * size.element_product())
            .sum::<Vec3>()
            / volume;
        let mut record = chunk_template.clone();
        record.id = shape.id.clone();
        record.node = shape.id.clone();
        record.part_id = shape.id.clone();
        record.position = center.to_array();
        record.rotation = Quat::IDENTITY.to_array();
        record.center_of_mass = [0.0; 3];
        record.volume = volume;
        record.mass = volume * 2400.0;
        let mut inertia = Mat3::ZERO;
        for &(box_center, size) in &shape.boxes {
            let mass = size.element_product() * 2400.0;
            let offset = box_center - center;
            inertia += Mat3::from_diagonal(
                Vec3::new(
                    size.y * size.y + size.z * size.z,
                    size.x * size.x + size.z * size.z,
                    size.x * size.x + size.y * size.y,
                ) * (mass / 12.0),
            ) + (Mat3::IDENTITY * offset.length_squared()
                - Mat3::from_cols(offset * offset.x, offset * offset.y, offset * offset.z))
                * mass;
        }
        record.inertia = inertia.to_cols_array_2d();
        record.colliders = shape
            .boxes
            .iter()
            .map(|&(box_center, size)| ColliderRecord {
                kind: "convex_hull".into(),
                vertices: box_triangles(box_center - center, size)
                    .into_iter()
                    .flatten()
                    .map(|point| point.to_array())
                    .collect(),
            })
            .collect();
        let entity = world
            .spawn((
                Name::new(shape.id.clone()),
                Transform::IDENTITY,
                Visibility::Inherited,
                ChildOf(body),
                ChunkIdentity {
                    building,
                    id: shape.id.clone(),
                },
            ))
            .id();
        let mesh = world
            .resource_mut::<Assets<Mesh>>()
            .add(triangle_mesh(&triangles));
        let color = shape.color.map(|value| f32::from(value) / 255.0);
        let material = world
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                base_color: Color::srgb(color[0], color[1], color[2]),
                unlit: true,
                double_sided: true,
                cull_mode: None,
                ..default()
            });
        let primitive = world
            .spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform::IDENTITY,
                Visibility::Inherited,
                ChildOf(entity),
            ))
            .id();
        instance.chunks.insert(
            shape.id,
            ChunkInstance {
                entity,
                primitives: vec![primitive],
                alive: true,
                triangles,
                transform: record.transform(),
                material: "concrete".into(),
                detail: None,
                cluster: Some(body),
            },
        );
        manifest.chunks.push(record);
    }
    let mut force_bonds = BTreeMap::new();
    for (index, &(name, x)) in TARGETS.iter().enumerate() {
        let mut bond = bond_template.clone();
        bond.id = format!("visibility-{name}");
        bond.a = target_id(index * 2);
        bond.b = target_id(index * 2 + 1);
        bond.center = [x, 0.0, 0.0];
        bond.normal = Vec3::X.to_array();
        bond.area = 0.01;
        bond.area_covariance =
            Mat3::from_diagonal(Vec3::new(0.0, 1e-4 / 12.0, 1e-4 / 12.0)).to_cols_array_2d();
        bond.patches = vec![BondPatchRecord {
            area: bond.area,
            center: bond.center,
            normal: bond.normal,
            area_covariance: bond.area_covariance,
            vertices: vec![
                [x, -0.05, -0.05],
                [x, 0.05, -0.05],
                [x, 0.05, 0.05],
                [x, -0.05, 0.05],
            ],
        }];
        instance.bonds.insert(bond.id.clone(), true);
        // These deterministic diagnostic forces test visibility only. The first
        // five phases still require the authored selfweight and a real contact J.
        force_bonds.insert(
            bond.id.clone(),
            BondStress {
                force: [2000.0, 0.0, 0.0],
                moment: [0.0; 3],
                tension: 0.0,
                compression: 0.0,
                shear: 0.0,
                utilization: 0.7,
                section_exact: true,
            },
        );
        manifest.bonds.push(bond);
    }
    let asset = world
        .resource_mut::<Assets<DestructibleAsset>>()
        .add(DestructibleAsset {
            manifest,
            gltf: Handle::default(),
        });
    instance.asset = asset;
    instance.clusters = vec![body];
    let chunks = instance.chunks.keys().cloned().collect::<Vec<_>>();
    let signature = bond_signature(&instance.bonds);
    world.entity_mut(body).insert(Cluster {
        building,
        chunks: chunks.clone(),
        anchored: false,
    });
    world.entity_mut(building).insert(instance);
    VisibilityFixture {
        building,
        body,
        chunks,
        bonds: force_bonds,
        signature,
    }
}

fn hold_visibility_fixture(world: &mut World, fixture: &VisibilityFixture, enabled: bool) {
    let camera = world
        .query_filtered::<Entity, With<DemoCamera>>()
        .single(world)
        .unwrap();
    world.entity_mut(camera).insert((
        Transform::from_translation(FIXTURE_ORIGIN + Vec3::Z * FIXTURE_DISTANCE),
        Projection::Perspective(PerspectiveProjection {
            fov: std::f32::consts::FRAC_PI_3,
            near: 0.05,
            far: 100.0,
            ..default()
        }),
        Tonemapping::None,
        DebandDither::Disabled,
    ));
    if enabled {
        let now = world.resource::<Time<Fixed>>().elapsed_secs_f64();
        world
            .resource_mut::<ForceSamples>()
            .record_force(DynamicForceSample {
                building: fixture.building,
                body: fixture.body,
                chunks: fixture.chunks.clone(),
                bonds: fixture.bonds.clone(),
                bond_signature: fixture.signature,
                captured_at: now,
            });
    }
}

fn source_id_pixels(capture: &Capture, index: usize) -> usize {
    let color = TARGET_COLORS[index];
    capture
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| (0..3).all(|c| pixel[c].abs_diff(color[c]) <= 2))
        .count()
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
        world.resource_mut::<ForceCapture>().enabled = matches!(state.phase, 1..=3 | 6);
        if state.phase == 5 {
            state.fixture = Some(spawn_visibility_fixture(world));
        }
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
                state.phase = NAMES.len();
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
    if let Some(fixture) = &state.fixture {
        hold_visibility_fixture(world, fixture, state.phase == 6);
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
    if state.phase == 6 {
        state.visibility_peak_rays = state.visibility_peak_rays.max(stats.visibility_rays);
        state.visibility_stats = Some(stats.clone());
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
                    state.phase = NAMES.len();
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
    let scene_pixels = if phase >= 5 {
        let counts = (0..10)
            .map(|index| source_id_pixels(&candidate, index))
            .collect::<Vec<_>>();
        if counts
            .iter()
            .enumerate()
            .any(|(index, &pixels)| expected_target_visible(index) && pixels < 16)
        {
            state.rejected_readbacks[phase] += 1;
            return false;
        }
        counts.iter().sum()
    } else {
        scene_surface_pixels(&candidate)
    };
    if phase < 5 && scene_pixels < (candidate.width * candidate.height / 500).max(256) {
        state.rejected_readbacks[phase] += 1;
        return false;
    }
    state.scene_pixels.insert(phase, scene_pixels);
    state.captures.insert(phase, candidate);
    state.phase = phase + 1;
    true
}

fn fixture_changed_pixels(a: &Capture, b: &Capture, tile: usize, exclude_center: bool) -> usize {
    if a.width != b.width || a.height != b.height {
        return usize::MAX;
    }
    let scale = a.height as f32 / (2.0 * (std::f32::consts::FRAC_PI_6).tan() * FIXTURE_DISTANCE);
    let center = Vec2::new(
        a.width as f32 * 0.5 + TARGETS[tile].1 * scale,
        a.height as f32 * 0.5,
    );
    let half = Vec2::new(scale, scale * 0.6);
    let mut count = 0;
    for y in 0..a.height {
        for x in 0..a.width {
            let delta = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - center;
            if delta.x.abs() > half.x
                || delta.y.abs() > half.y
                || (exclude_center && delta.length_squared() < 64.0)
            {
                continue;
            }
            let i = (y * a.width + x) * 4;
            let difference = (0..3)
                .map(|c| u32::from(a.pixels[i + c].abs_diff(b.pixels[i + c])))
                .sum::<u32>();
            count += usize::from(difference > 24);
        }
    }
    count
}

fn visibility_report(state: &mut ForceSmokeState) -> serde_json::Value {
    let Some(fixture) = &state.fixture else {
        state
            .errors
            .push("Missing source-triangle visibility fixture".into());
        return serde_json::Value::Null;
    };
    let Some(stats) = &state.visibility_stats else {
        state
            .errors
            .push("Missing actual force visibility draw statistics".into());
        return serde_json::Value::Null;
    };
    let visible = stats
        .visible_chunks
        .iter()
        .filter(|(building, _)| *building == fixture.building)
        .map(|(_, id)| id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let drawn = stats
        .drawn_bonds
        .iter()
        .filter(|(building, _)| *building == fixture.building)
        .map(|(_, id)| id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let graph = stats
        .drawn_graph_bonds
        .iter()
        .filter(|(building, _)| *building == fixture.building)
        .map(|(_, id)| id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let mut targets = Vec::new();
    if let (Some(off), Some(on), Some(repeat)) = (
        state.captures.get(&5),
        state.captures.get(&6),
        state.captures.get(&7),
    ) {
        for index in 0..10 {
            let id = target_id(index);
            let pixels = source_id_pixels(off, index);
            let expected = expected_target_visible(index);
            let minimum = if index == 7 { 16 } else { 64 };
            if (expected && pixels < minimum) || (!expected && pixels != 0) {
                state.errors.push(format!("Visibility source-ID reference {id}: {pixels} pixels, expected visible={expected}"));
            }
            if visible.contains(&id) != expected {
                state.errors.push(format!(
                    "Actual visibility set disagrees for {id}: expected visible={expected}"
                ));
            }
            targets.push(serde_json::json!({"chunk":id,"source_id_pixels":pixels,"expected_visible":expected,"actual_visible":visible.contains(&id)}));
        }
        for (tile, &(name, _)) in TARGETS.iter().enumerate() {
            let id = format!("visibility-{name}");
            let expected = matches!(tile, 0 | 2 | 3);
            if drawn.contains(&id) != expected || graph.contains(&id) != expected {
                state.errors.push(format!(
                    "Hidden/visible target {name} has incorrect force/graph bond submission"
                ));
            }
            if tile == 4 {
                continue;
            }
            let changed = fixture_changed_pixels(off, on, tile, tile == 1);
            let repeat_changed = fixture_changed_pixels(off, repeat, tile, tile == 1);
            if (expected && changed < 8) || (!expected && changed != 0) {
                state.errors.push(format!("Visibility target {name}: {changed} changed force pixels, expected visible={expected}"));
            }
            if repeat_changed != 0 {
                state.errors.push(format!(
                    "Visibility cleared target {name}: {repeat_changed} residual force pixels"
                ));
            }
            targets.push(serde_json::json!({"bond":id,"expected_drawn":expected,"force_pixels":changed,"cleared_repeat_pixels":repeat_changed}));
        }
    } else {
        state
            .errors
            .push("Missing off/on/repeat visibility GPU captures".into());
    }
    if stats.force_pairs != 2 || stats.force_arrows != 5 {
        state.errors.push(format!(
            "Visible endpoint force counts disagree: {} pairs, {} arrows; expected 2/5",
            stats.force_pairs, stats.force_arrows
        ));
    }
    if state.visibility_peak_rays == 0 || stats.visibility_untested != 0 {
        state.errors.push(format!(
            "Visibility fixture was not fully tested: {} rays, {} untested",
            state.visibility_peak_rays, stats.visibility_untested
        ));
    }
    serde_json::json!({"source_triangles_match_render_mesh":true,"partial_centers_are_occluded":true,"synthetic_forces_only_in_fixture":true,"visible_chunks":visible,"drawn_force_bonds":drawn,"drawn_graph_bonds":graph,"force_pairs":stats.force_pairs,"force_arrows":stats.force_arrows,"visibility_peak_actual_rays":state.visibility_peak_rays,"visibility_capture_frame_rays":stats.visibility_rays,"visibility_untested":stats.visibility_untested,"targets":targets})
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
    let visibility = visibility_report(state);
    let report = serde_json::json!({"asset":world.resource::<DemoOptions>().asset,"asset_id":state.asset_id,"schema_version":state.schema_version,"chunk_count":state.chunk_count,"proxy_count":state.proxy_count,"static_bonds":state.peak_static_bonds,"force_pairs":state.peak_force_pairs,"peak_force_n":state.peak_force_n,"peak_actual_impulse_ns":state.peak_impulse_ns,"force_overlay_changed_non_hud_pixels":changed,"contact_impulse_purple_pixels":purple,"broken_connection_hidden":state.saw_broken_hidden,"overlay_cleared":state.saw_cleared,"screenshots":state.captures.len(),"scene_surface_pixels":state.scene_pixels,"rejected_readbacks":state.rejected_readbacks,"visibility":visibility,"errors":state.errors});
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
