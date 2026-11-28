use std::collections::BTreeMap;

use avian3d::prelude::{
    AngularInertia, CenterOfMass, Collider, ColliderOf, ComputeMassProperties3d, GravityScale,
    Mass, NoAutoAngularInertia, NoAutoCenterOfMass, NoAutoMass, Position, RigidBody, Rotation,
};
use bevy::{
    asset::RenderAssetUsages,
    core_pipeline::prepass::DepthPrepass,
    mesh::{MeshTag, PrimitiveTopology},
    prelude::*,
    render::view::{DebandDither, Tonemapping, screenshot::ScreenshotCaptured},
};

use crate::{
    DemoSettings, ProbeOptions,
    asset::{BondPatchRecord, ColliderRecord, DestructibleAsset},
    force_debug::{ForceViewStats, ScriptedForceInspection},
    force_samples::{
        DynamicForceSample, FORCE_SAMPLE_SECONDS, ForceCapture, ForceSamples, bond_signature,
    },
    material::{
        BuildingMaterial, HEAT_AMBER_TAG, HEAT_RED_TAG, building_material, surface_coordinates,
    },
    physics::{Cluster, NeedsClusterRebuild},
    projectiles::{ImpactStats, LaunchRequest, LaunchRequests, ProjectileShape},
    scene::{
        BondInstance, BuildingBounds, BuildingInstance, ChunkIdentity, ChunkInstance,
        ChunkPrimitive, DemoCamera, chunk_primitives, spawn_bonds,
    },
    stress::BondStress,
    stress_runtime::StressState,
};

#[cfg(test)]
mod tests;

const NAMES: [&str; 11] = [
    "01-overlay-off.png",
    "02-structural-heat.png",
    "03-broken-connection-hidden.png",
    "04-actual-contact-inspection.png",
    "05-overlay-cleared.png",
    "06-visibility-source-ids.png",
    "07-building-neutral.png",
    "08-heat-without-alt.png",
    "09-alt-window-inspection.png",
    "10-alt-released.png",
    "11-heat-off-restored.png",
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
    rejected_readbacks: [u32; 11],
    peak_static_bonds: usize,
    peak_tinted_chunks: usize,
    peak_force_n: f64,
    peak_impulse_ns: f64,
    saw_broken_hidden: bool,
    saw_cleared: bool,
    saw_real_impulse_readout: bool,
    saw_normal_without_arrows: bool,
    saw_neutral_inspection: bool,
    fixture: Option<VisibilityFixture>,
    fixture_stats: BTreeMap<usize, ForceViewStats>,
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
    twin: Entity,
    primitives: Vec<FixturePrimitive>,
}

struct FixturePrimitive {
    entity: Entity,
    source_id: Handle<StandardMaterial>,
    building: Handle<BuildingMaterial>,
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
    let mut primitives = Vec::new();
    let mut surface = building_material(None, false, "concrete");
    surface.base.double_sided = true;
    surface.base.cull_mode = None;
    let surface = world
        .resource_mut::<Assets<BuildingMaterial>>()
        .add(surface);
    let mut fracture_surface = building_material(None, true, "concrete");
    fracture_surface.base.double_sided = true;
    fracture_surface.base.cull_mode = None;
    let fracture_surface = world
        .resource_mut::<Assets<BuildingMaterial>>()
        .add(fracture_surface);
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
        let mass_properties = Collider::compound(
            shape
                .boxes
                .iter()
                .map(|&(box_center, size)| {
                    (
                        box_center - center,
                        Quat::IDENTITY,
                        Collider::cuboid(size.x, size.y, size.z),
                    )
                })
                .collect(),
        )
        .mass_properties(manifest.materials[&record.material].density);
        record.mass = mass_properties.mass;
        record.inertia = Mat3::from(mass_properties.angular_inertia_tensor()).to_cols_array_2d();
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
            .add(surface_coordinates(
                triangle_mesh(&triangles),
                Transform::IDENTITY,
            ));
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
                Name::new(format!("{} force fixture primitive", shape.id)),
                Mesh3d(mesh),
                MeshMaterial3d(material.clone()),
                MeshTag::new(0),
                ChunkPrimitive,
                Transform::IDENTITY,
                Visibility::Inherited,
                ChildOf(entity),
            ))
            .id();
        let source_surface = if shape.id == "partial-b" {
            fracture_surface.clone()
        } else {
            surface.clone()
        };
        primitives.push(FixturePrimitive {
            entity: primitive,
            source_id: material,
            building: source_surface,
        });
        world.entity_mut(entity).insert(ChunkInstance {
            alive: true,
            triangles,
            transform: record.transform(),
            mass_properties,
            material: "concrete".into(),
        });
        instance.chunks.insert(shape.id, entity);
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
        // These deterministic diagnostic forces test visibility only. The first
        // five phases still require the authored selfweight and a real contact J.
        let utilization = [0.499, 1.4, 0.5, 1.0, 1.2][index];
        force_bonds.insert(
            bond.id.clone(),
            BondStress {
                force: [2000.0, 0.0, 0.0],
                moment: [0.0; 3],
                tension: 0.0,
                compression: 0.0,
                shear: 0.0,
                utilization,
                strain_energy: 0.0,
                section_exact: true,
            },
        );
        manifest.bonds.push(bond);
    }
    let chunks = instance.chunks.keys().cloned().collect::<Vec<_>>();
    world.entity_mut(body).insert(Cluster {
        chunks: chunks.clone(),
        anchored: false,
    });
    let twin = world
        .spawn((
            Name::new("Same-asset neutral instance"),
            Transform::IDENTITY,
            Visibility::Visible,
        ))
        .id();
    let twin_origin = FIXTURE_ORIGIN + Vec3::Y * 4.5;
    let twin_body = world
        .spawn((
            Name::new("Frozen same-asset body"),
            RigidBody::Dynamic,
            Position(twin_origin),
            Rotation::IDENTITY,
            GravityScale(0.0),
            Mass(100.0),
            CenterOfMass(Vec3::ZERO),
            AngularInertia::from_mat3_unchecked(Mat3::IDENTITY),
            Transform::from_translation(twin_origin),
            Visibility::Visible,
            ChildOf(twin),
            NoAutoMass,
            NoAutoAngularInertia,
            NoAutoCenterOfMass,
        ))
        .id();
    let mut second = BuildingInstance::pending(Handle::default());
    second.bounds = instance.bounds;
    for (id, &original_entity) in &instance.chunks {
        let chunk = world.get::<ChunkInstance>(original_entity).unwrap();
        let original = chunk_primitives(world.get::<Children>(original_entity), |entity| {
            world.get::<ChunkPrimitive>(entity).is_some()
        })
        .next()
        .expect("Force fixture chunk has an original mesh primitive");
        let chunk = ChunkInstance {
            alive: true,
            triangles: chunk.triangles.clone(),
            transform: chunk.transform,
            mass_properties: chunk.mass_properties,
            material: chunk.material.clone(),
        };
        let entity = world
            .spawn((
                Name::new(format!("Neutral twin {id}")),
                Transform::IDENTITY,
                Visibility::Inherited,
                ChildOf(twin_body),
                ChunkIdentity {
                    building: twin,
                    id: id.clone(),
                },
                chunk,
            ))
            .id();
        let mesh = world.get::<Mesh3d>(original).unwrap().0.clone();
        let original_material = primitives
            .iter()
            .find(|entry| entry.entity == original)
            .unwrap();
        let source = original_material.source_id.clone();
        let source_surface = original_material.building.clone();
        let primitive = world
            .spawn((
                Name::new(format!("Neutral twin {id} primitive")),
                Mesh3d(mesh),
                MeshMaterial3d(source.clone()),
                MeshTag::new(0),
                ChunkPrimitive,
                Transform::IDENTITY,
                Visibility::Inherited,
                ChildOf(entity),
            ))
            .id();
        primitives.push(FixturePrimitive {
            entity: primitive,
            source_id: source,
            building: source_surface,
        });
        second.chunks.insert(id.clone(), entity);
    }
    world.entity_mut(twin_body).insert(Cluster {
        chunks: chunks.clone(),
        anchored: false,
    });
    // These visual fixtures model body ownership and use only meshes for geometry.
    for (owner, origin, instance) in [
        (body, FIXTURE_ORIGIN, &instance),
        (twin_body, twin_origin, &second),
    ] {
        let global = GlobalTransform::from_translation(origin);
        world.entity_mut(owner).insert(global);
        for &entity in instance.chunks.values() {
            world
                .entity_mut(entity)
                .insert((global, ColliderOf { body: owner }));
        }
    }
    let mut commands = world.commands();
    spawn_bonds(&mut commands, building, &mut instance, &manifest.bonds);
    spawn_bonds(&mut commands, twin, &mut second, &manifest.bonds);
    world.flush();
    let signature = bond_signature(instance.bonds.iter().map(|(id, &entity)| {
        (
            id.as_str(),
            world.get::<BondInstance>(entity).unwrap().active,
        )
    }));
    let asset = world
        .resource_mut::<Assets<DestructibleAsset>>()
        .add(DestructibleAsset {
            manifest,
            gltf: Handle::default(),
        });
    instance.asset = asset.clone();
    second.asset = asset;
    world.entity_mut(twin).insert(second);
    world.entity_mut(building).insert(instance);
    VisibilityFixture {
        building,
        body,
        chunks,
        bonds: force_bonds,
        signature,
        twin,
        primitives,
    }
}

fn hold_visibility_fixture(
    world: &mut World,
    fixture: &VisibilityFixture,
    phase: usize,
    hold: u32,
) {
    let camera = world
        .query_filtered::<Entity, With<DemoCamera>>()
        .single(world)
        .unwrap();
    let eye = FIXTURE_ORIGIN + Vec3::new(0.0, 2.25, FIXTURE_DISTANCE);
    let pose = if phase == 8 {
        Transform::from_translation(eye).looking_at(
            FIXTURE_ORIGIN + Vec3::new(1.4, if hold < 8 { 4.5 } else { 0.0 }, 0.08),
            Vec3::Y,
        )
    } else {
        Transform::from_translation(eye)
    };
    world.entity_mut(camera).insert((
        pose,
        Projection::Perspective(PerspectiveProjection {
            fov: std::f32::consts::FRAC_PI_3,
            near: 0.05,
            far: 100.0,
            ..default()
        }),
        Tonemapping::None,
        DebandDither::Disabled,
        DepthPrepass,
    ));
    if phase != 5 {
        for primitive in &fixture.primitives {
            if world
                .get::<MeshMaterial3d<BuildingMaterial>>(primitive.entity)
                .is_none()
            {
                world
                    .entity_mut(primitive.entity)
                    .remove::<MeshMaterial3d<StandardMaterial>>()
                    .insert(MeshMaterial3d(primitive.building.clone()));
            }
        }
    }
    if matches!(phase, 6..=9) {
        let now = world.resource::<Time<Fixed>>().elapsed_secs_f64();
        world
            .resource_mut::<ForceSamples>()
            .record_force(DynamicForceSample {
                building: fixture.building,
                body: fixture.body,
                chunks: fixture.chunks.clone(),
                bonds: fixture.bonds.clone(),
                bond_signature: fixture.signature,
                captured_at: if phase == 6 {
                    now - FORCE_SAMPLE_SECONDS - 0.1
                } else {
                    now
                },
            });
    }
}

fn aim_at_real_impulse_chunk(world: &mut World) {
    let Some((building, id)) = world
        .resource::<ForceSamples>()
        .impulses
        .iter()
        .rev()
        .map(|sample| (sample.building, sample.chunk.clone()))
        .find(|(building, id)| {
            world
                .get::<BuildingInstance>(*building)
                .and_then(|instance| instance.chunks.get(id))
                .and_then(|&entity| world.get::<ChunkInstance>(entity))
                .is_some_and(|chunk| chunk.alive)
        })
    else {
        return;
    };
    let camera = world
        .query_filtered::<Entity, With<DemoCamera>>()
        .single(world)
        .unwrap();
    let eye = world.get::<GlobalTransform>(camera).unwrap().translation();
    let instance = world.get::<BuildingInstance>(building).unwrap();
    let entity = instance.chunks[&id];
    let chunk = world.get::<ChunkInstance>(entity).unwrap();
    let Some(pose) = world
        .get::<ColliderOf>(entity)
        .and_then(|owner| world.get::<GlobalTransform>(owner.body))
    else {
        return;
    };
    let target = chunk
        .triangles
        .iter()
        .filter_map(|triangle| {
            let [a, b, c] = triangle.map(|point| pose.transform_point(point));
            let normal = (b - a).cross(c - a).try_normalize()?;
            let center = (a + b + c) / 3.0;
            (normal.dot(eye - center) > 0.0).then_some((
                center,
                normal,
                center.distance_squared(eye),
            ))
        })
        .min_by(|a, b| a.2.total_cmp(&b.2));
    if let Some((center, normal, _)) = target {
        *world.get_mut::<Transform>(camera).unwrap() =
            Transform::from_translation(center + normal * 1.2).looking_at(center, Vec3::Y);
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
    let Some(directory) = world.resource::<ProbeOptions>().force_smoke_output.clone() else {
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
        world.resource_mut::<ForceCapture>().enabled = matches!(state.phase, 1..=3 | 6..=9);
        world.resource_mut::<ScriptedForceInspection>().held = matches!(state.phase, 3 | 8);
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
                .filter(|record| {
                    let entity = instance.chunks[&record.id];
                    let chunk = world.get::<ChunkInstance>(entity).unwrap();
                    chunk.alive
                        && world
                            .get::<ColliderOf>(entity)
                            .and_then(|owner| world.get::<RigidBody>(owner.body))
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
                            .get::<GlobalTransform>(instance.chunks[&record.id])
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
                world.resource_mut::<crate::camera::ScriptedFlight>().input =
                    Some(crate::camera::FlightInput {
                        movement: Vec3::ZERO,
                        mouse: Vec2::new(-yaw, -pitch) / crate::camera::LOOK_SENSITIVITY,
                    });
                world
                    .resource_mut::<LaunchRequests>()
                    .queue
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
                .valid_bonds
                .first()
                .cloned();
            if let Some((building, id)) = &state.broken {
                let bond = world.get::<BuildingInstance>(*building).unwrap().bonds[id];
                world.get_mut::<BondInstance>(bond).unwrap().active = false;
                world.entity_mut(*building).insert(NeedsClusterRebuild);
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
        hold_visibility_fixture(world, fixture, state.phase, state.hold);
    } else if state.phase == 3 {
        aim_at_real_impulse_chunk(world);
    }
    state.hold += 1;
    let stats = world.resource::<ForceViewStats>().clone();
    if state.phase == 1 {
        state.peak_static_bonds = state.peak_static_bonds.max(stats.static_bonds);
        state.peak_tinted_chunks = state.peak_tinted_chunks.max(stats.tinted_chunks.len());
        state.peak_force_n = state.peak_force_n.max(stats.max_force_n);
        state.saw_normal_without_arrows =
            stats.force_arrows == 0 && stats.pick_queries == 0 && stats.inspected.is_none();
    }
    if state.phase == 3 {
        if let Some(inspection) = &stats.inspected
            && let Some(impulse) = inspection.impulse_ns
        {
            let genuine = world
                .resource::<ForceSamples>()
                .impulses
                .iter()
                .any(|sample| {
                    (sample.building, sample.chunk.as_str())
                        == (inspection.chunk.0, inspection.chunk.1.as_str())
                        && (sample.impulse.as_dvec3().length() - impulse).abs()
                            <= 1e-6 * impulse.max(1.0)
                });
            state.saw_real_impulse_readout |= genuine;
            if genuine {
                state.peak_impulse_ns = state.peak_impulse_ns.max(impulse);
            }
        }
        if world.resource::<ImpactStats>().contacts <= state.before_contacts
            || !state.saw_real_impulse_readout
        {
            return;
        }
    }
    if state.phase == 2 {
        state.saw_broken_hidden = state
            .broken
            .as_ref()
            .is_some_and(|broken| !stats.valid_bonds.contains(broken));
    }
    if state.phase == 4 {
        let samples = world.resource::<ForceSamples>();
        state.saw_cleared = stats.force_arrows == 0
            && stats.tinted_chunks.is_empty()
            && stats.inspected.is_none()
            && samples.dynamic.is_empty()
            && samples.impulses.is_empty();
    }
    if state.phase >= 6 {
        state.fixture_stats.insert(state.phase, stats.clone());
        if state.phase == 8
            && stats.inspected.as_ref().is_some_and(|readout| {
                state
                    .fixture
                    .as_ref()
                    .is_some_and(|fixture| readout.chunk == (fixture.twin, "window-a".into()))
                    && readout.bonds.is_empty()
                    && readout.impulse_ns.is_none()
            })
        {
            state.saw_neutral_inspection |= stats.force_arrows == 0 && stats.pick_queries > 0;
        }
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
    let screenshot = (
        Name::new("Force smoke screenshot"),
        world.resource::<ProbeOptions>().smoke_screenshot(),
    );
    world.spawn(screenshot).observe(
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
    let scene_pixels = if phase == 5 {
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
    } else if phase > 5 {
        let pixels = state
            .captures
            .get(&5)
            .and_then(|reference| building_readback_pixels(reference, &candidate, phase));
        let Some(pixels) = pixels else {
            state.rejected_readbacks[phase] += 1;
            return false;
        };
        pixels
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

fn building_readback_pixels(reference: &Capture, capture: &Capture, phase: usize) -> Option<usize> {
    if (reference.width, reference.height) != (capture.width, capture.height) {
        return None;
    }
    if phase == 8 {
        let mut center = Vec::new();
        for y in capture.height / 2 - 10..capture.height / 2 + 10 {
            for x in capture.width / 2 - 10..capture.width / 2 + 10 {
                center.push((y * capture.width + x) * 4);
            }
        }
        let amber = heat_hues(capture, &center).0;
        let cyan = center_pixels(capture)
            .filter(|(_, p)| {
                p[1] > p[0].saturating_add(30) && p[2] > p[0].saturating_add(30) && p[2] > 140
            })
            .count();
        return (amber * 10 >= center.len() * 5 && cyan >= 5).then_some(amber);
    }
    let mut ready = 0;
    for twin in [false, true] {
        for target in [0, 1] {
            let indices = id_indices(reference, target, twin);
            if indices.len() < 16 {
                return None;
            }
            let solid = indices
                .iter()
                .filter(|&&i| {
                    let channels = &capture.pixels[i..i + 3];
                    let low = *channels.iter().min().unwrap();
                    let high = *channels.iter().max().unwrap();
                    low >= 43 && high <= 245 && high - low <= 45
                })
                .count();
            if solid * 10 < indices.len() * 7 {
                return None;
            }
            ready += solid;
        }
    }
    Some(ready)
}

fn id_indices(capture: &Capture, target: usize, twin: bool) -> Vec<usize> {
    let color = TARGET_COLORS[target];
    capture
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .enumerate()
        .filter(|(index, pixel)| {
            let upper = index / capture.width < capture.height / 2;
            upper == twin && (0..3).all(|c| pixel[c].abs_diff(color[c]) <= 2)
        })
        .map(|(index, _)| index * 4)
        .collect()
}

fn changed_indices(a: &Capture, b: &Capture, indices: &[usize]) -> usize {
    if (a.width, a.height) != (b.width, b.height) {
        return usize::MAX;
    }
    indices
        .iter()
        .filter(|&&i| {
            (0..3)
                .map(|c| u32::from(a.pixels[i + c].abs_diff(b.pixels[i + c])))
                .sum::<u32>()
                > 24
        })
        .count()
}

fn heat_hues(capture: &Capture, indices: &[usize]) -> (usize, usize) {
    let mut amber = 0;
    let mut red = 0;
    for &i in indices {
        let [r, g, b] = [
            capture.pixels[i],
            capture.pixels[i + 1],
            capture.pixels[i + 2],
        ]
        .map(f32::from);
        if r >= 0.9 * g && g > 1.4 * b && g - b > 18.0 {
            amber += 1;
        } else if r > 1.35 * g.max(b) && r - g > 20.0 {
            red += 1;
        }
    }
    (amber, red)
}

fn covered_rear_indices(capture: &Capture) -> Vec<usize> {
    let scale =
        capture.height as f32 / (2.0 * std::f32::consts::FRAC_PI_6.tan() * FIXTURE_DISTANCE);
    let center = Vec2::new(
        capture.width as f32 * 0.5 + TARGETS[1].1 * scale,
        capture.height as f32 * 0.5 + 2.25 * scale,
    );
    (0..capture.width * capture.height)
        .filter(|&index| {
            let d = Vec2::new(
                (index % capture.width) as f32 + 0.5,
                (index / capture.width) as f32 + 0.5,
            ) - center;
            d.x.abs() < 0.9 * scale && d.y.abs() < 0.6 * scale
        })
        .map(|index| index * 4)
        .collect()
}

fn heat_report(state: &mut ForceSmokeState) -> serde_json::Value {
    let Some(fixture) = &state.fixture else {
        state
            .errors
            .push("Missing shared-asset heat fixture".into());
        return serde_json::Value::Null;
    };
    let Some(stats) = state.fixture_stats.get(&7) else {
        state
            .errors
            .push("Missing actual heat draw statistics".into());
        return serde_json::Value::Null;
    };
    let Some(ids) = state.captures.get(&5) else {
        state.errors.push("Missing source-ID reference".into());
        return serde_json::Value::Null;
    };
    let Some(neutral) = state.captures.get(&6) else {
        state
            .errors
            .push("Missing neutral BuildingMaterial reference".into());
        return serde_json::Value::Null;
    };
    let Some(heat) = state.captures.get(&7) else {
        state.errors.push("Missing heat GPU capture".into());
        return serde_json::Value::Null;
    };
    let Some(released) = state.captures.get(&9) else {
        state.errors.push("Missing Alt-release GPU capture".into());
        return serde_json::Value::Null;
    };
    let Some(restored) = state.captures.get(&10) else {
        state
            .errors
            .push("Missing heat-off restored GPU capture".into());
        return serde_json::Value::Null;
    };
    let mut targets = Vec::new();
    let mut all_indices = Vec::new();
    for twin in [false, true] {
        let building = if twin { fixture.twin } else { fixture.building };
        for index in 0..10 {
            let id = target_id(index);
            let indices = id_indices(ids, index, twin);
            let expected_visible = expected_target_visible(index);
            if (expected_visible && indices.len() < if index == 7 { 16 } else { 64 })
                || (!expected_visible && !indices.is_empty())
            {
                state.errors.push(format!(
                    "Source-ID {} {id}: {} visible pixels",
                    if twin { "neutral twin" } else { "loaded" },
                    indices.len()
                ));
            }
            let expected_tag = if twin || index < 2 {
                0
            } else if index / 2 == 2 {
                HEAT_AMBER_TAG
            } else {
                HEAT_RED_TAG
            };
            let actual_tag = stats
                .tinted_chunks
                .get(&(building, id.clone()))
                .copied()
                .unwrap_or(0);
            if actual_tag != expected_tag {
                state.errors.push(format!(
                    "Heat tag for {id} twin={twin}: expected {expected_tag}, actual {actual_tag}"
                ));
            }
            let changed = changed_indices(neutral, heat, &indices);
            let restored_pixels = changed_indices(neutral, restored, &indices);
            let released_pixels = changed_indices(heat, released, &indices);
            let (amber, red) = heat_hues(heat, &indices);
            if expected_visible && expected_tag == 0 && changed != 0 {
                state.errors.push(format!(
                    "Neutral instance/chunk {id} twin={twin} inherited heat: {changed} pixels"
                ));
            }
            if expected_visible && expected_tag != 0 {
                let matching = if expected_tag == HEAT_AMBER_TAG {
                    amber
                } else {
                    red
                };
                if matching * 10 < indices.len() * 7 {
                    state.errors.push(format!(
                        "Heat hue did not cover {id}: {matching}/{} expected tag {expected_tag}",
                        indices.len()
                    ));
                }
            }
            if restored_pixels != 0 || released_pixels != 0 {
                state.errors.push(format!("Heat restoration {id} twin={twin}: off={restored_pixels}, release={released_pixels}"));
            }
            targets.push(serde_json::json!({"chunk":id,"neutral_same_asset_instance":twin,"source_id_pixels":indices.len(),"expected_tag":expected_tag,"actual_tag":actual_tag,"changed_pixels":changed,"amber_pixels":amber,"red_pixels":red,"off_restored_pixels":restored_pixels,"alt_released_pixels":released_pixels}));
            all_indices.extend(indices);
        }
    }
    let rear_changed = changed_indices(neutral, heat, &covered_rear_indices(neutral));
    if rear_changed != 0 {
        state.errors.push(format!(
            "Hidden red chunks leaked through opaque foreground: {rear_changed} pixels"
        ));
    }
    for phase in [6, 7, 9, 10] {
        if state
            .fixture_stats
            .get(&phase)
            .is_none_or(|s| s.inspected.is_some() || s.force_arrows != 0 || s.pick_queries != 0)
        {
            state.errors.push(format!(
                "Phase {phase} performed inspection or retained arrows without Alt"
            ));
        }
    }
    let inspection = state
        .fixture_stats
        .get(&8)
        .and_then(|s| s.inspected.as_ref());
    let selected_ok = inspection.is_some_and(|selected| {
        selected.chunk == (fixture.building, "window-a".into())
            && selected.bonds.len() == 1
            && selected.bonds[0].id == "visibility-window"
            && (selected.bonds[0].force_n - 2000.0).abs() < 1e-6
            && (selected.bonds[0].utilization - 0.5).abs() < 1e-12
            && selected.bonds[0].impact
    });
    let inspection_stats = state.fixture_stats.get(&8);
    if !state.saw_neutral_inspection {
        state.errors.push("Alt on same-asset chunk without valid force data did not report unavailable without an arrow".into());
    }
    let expired = state.fixture_stats.get(&6).is_some_and(|s| {
        !s.tinted_chunks
            .keys()
            .any(|(building, _)| *building == fixture.building)
            && !s
                .valid_bonds
                .iter()
                .any(|(building, _)| *building == fixture.building)
    });
    let recovered = stats
        .valid_bonds
        .iter()
        .filter(|(building, _)| *building == fixture.building)
        .count()
        == 5;
    if !expired || !recovered {
        state
            .errors
            .push("Expired dynamic forces did not reset before fresh fixture recovery".into());
    }
    if !selected_ok || inspection_stats.is_none_or(|s| s.force_arrows != 1 || s.pick_queries == 0) {
        state.errors.push(
            "Alt did not inspect the warm chunk through the real window with one local force arrow"
                .into(),
        );
    }
    let cyan = state.captures.get(&8).map_or(0, |capture| {
        center_pixels(capture)
            .filter(|(_, p)| {
                p[1] > p[0].saturating_add(30) && p[2] > p[0].saturating_add(30) && p[2] > 140
            })
            .count()
    });
    if cyan < 5 {
        state
            .errors
            .push(format!("Alt inspection has only {cyan} cyan arrow pixels"));
    }
    let final_stats = state.fixture_stats.get(&10);
    if final_stats.is_none_or(|s| !s.tinted_chunks.is_empty()) {
        state
            .errors
            .push("Disabling heat left nonzero chunk tags".into());
    }
    serde_json::json!({"shared_asset_mesh_material_instances":true,"source_triangles_match_render_mesh":true,"depth_prepass":true,"expired_sample_reset":expired,"fresh_sample_recovered":recovered,"unavailable_inspection_without_arrow":state.saw_neutral_inspection,"ordinary_heat_has_no_arrows_or_picking":true,"hidden_red_changed_pixels":rear_changed,"alt_window_selected":selected_ok,"alt_cyan_arrow_pixels":cyan,"alt_force_arrows":inspection_stats.map(|s|s.force_arrows),"targets":targets,"restored_source_pixels":all_indices.len()})
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
    if state.peak_static_bonds == 0 || state.peak_force_n <= 0.0 {
        state
            .errors
            .push("No real current selfweight force data reached the overview".into());
    }
    if !state.saw_normal_without_arrows {
        state
            .errors
            .push("Normal overview performed picking or retained arrows".into());
    }
    if state.peak_impulse_ns <= 0.0 || !state.saw_real_impulse_readout {
        state
            .errors
            .push("Inspector did not read a real matching physical contact impulse".into());
    }
    if !state.saw_broken_hidden {
        state
            .errors
            .push("Broken connection retained valid force data".into());
    }
    if !state.saw_cleared {
        state
            .errors
            .push("Disabled force heat retained data, tags or inspection".into());
    }
    for name in NAMES {
        if !directory.join(name).is_file() {
            state.errors.push(format!("Missing screenshot {name}"));
        }
    }
    let heat = heat_report(state);
    let report = serde_json::json!({"asset":world.resource::<DemoSettings>().asset,"asset_id":state.asset_id,"schema_version":state.schema_version,"chunk_count":state.chunk_count,"proxy_count":state.proxy_count,"static_bonds":state.peak_static_bonds,"overview_tinted_chunks":state.peak_tinted_chunks,"peak_force_n":state.peak_force_n,"peak_actual_impulse_ns":state.peak_impulse_ns,"actual_impulse_matches_physical_sample":state.saw_real_impulse_readout,"normal_overview_without_arrows":state.saw_normal_without_arrows,"broken_connection_hidden":state.saw_broken_hidden,"overlay_cleared":state.saw_cleared,"screenshots":state.captures.len(),"scene_surface_pixels":state.scene_pixels,"rejected_readbacks":state.rejected_readbacks,"heat":heat,"errors":state.errors});
    std::fs::write(
        directory.join("report.json"),
        serde_json::to_string_pretty(&report).unwrap(),
    )
    .expect("Write force probe report");
    state.finished = true;
    world.write_message(if state.errors.is_empty() {
        AppExit::Success
    } else {
        AppExit::error()
    });
}
