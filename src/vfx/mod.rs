use std::time::Instant;

use avian3d::prelude::*;
use bevy::{
    asset::RenderAssetUsages,
    camera::visibility::NoFrustumCulling,
    light::NotShadowCaster,
    mesh::{MeshTag, PrimitiveTopology},
    prelude::*,
    render::storage::ShaderBuffer,
};

use crate::{
    dust_material::{DustMaterial, DustSettings, dust_atlas, dust_material, dust_settings},
    scene::{ChunkInstance, DemoCamera, RemovedChunk},
};

pub const PARTICLE_LIMIT: usize = 640;
const LAYER_LIMITS: [usize; 4] = [96, 96, 320, 128];
const BURST_COUNTS: [usize; 4] = [8, 8, 24, 24];
const DUST_POOL_SIZE: usize = PARTICLE_LIMIT - LAYER_LIMITS[1];
const MAX_COLLISION_QUERIES: usize = 48;
const DEBRIS_FADE_STEPS: usize = 16;

pub struct VfxPlugin;

impl Plugin for VfxPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<BreakBurst>()
            .init_resource::<Assets<ShaderBuffer>>()
            .init_resource::<VfxStats>()
            .init_resource::<VfxSettings>()
            .add_systems(Startup, setup)
            .add_systems(
                Update,
                (
                    spawn_bursts.after(crate::interaction::apply_damage),
                    animate_particles.after(crate::camera::move_camera),
                    prewarm_pipelines,
                )
                    .chain(),
            );
    }
}

#[derive(Clone, Debug)]
pub struct EmissionSurface {
    pub triangle: [Vec3; 3],
    pub normal: Vec3,
}

#[derive(Message, Clone)]
pub struct BreakBurst {
    pub point: Vec3,
    pub normal: Vec3,
    pub seed: u64,
    pub material: String,
    pub removed_volume: f32,
    pub exposed_area: f32,
    pub surfaces: Vec<EmissionSurface>,
}

#[derive(Resource)]
pub struct VfxSettings {
    pub enabled: bool,
    pub density: f32,
}

impl Default for VfxSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            density: 1.0,
        }
    }
}

#[derive(Resource, Default, Debug)]
pub struct VfxStats {
    pub active: [usize; 4],
    pub scheduled: [usize; 4],
    pub peak: usize,
    pub received: u64,
    pub spawned: [u64; 4],
    pub dropped: [u64; 4],
    pub retired: u64,
    pub first_feedbacks: u64,
    pub feedback_bursts: u64,
    pub collision_queries: u64,
    pub collisions: u64,
    pub update_cpu_ms: f32,
    pub max_update_cpu_ms: f32,
    pub emit_cpu_ms: f32,
    pub max_emit_cpu_ms: f32,
    pub estimated_coverage: f32,
    pub peak_estimated_coverage: f32,
    collision_cursor: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Layer {
    Impact,
    Debris,
    Smoke,
    Fallout,
}

impl Layer {
    fn index(self) -> usize {
        self as usize
    }
    fn from_index(index: usize) -> Self {
        [Self::Impact, Self::Debris, Self::Smoke, Self::Fallout][index]
    }
}

#[derive(Component)]
pub struct Particle {
    pub age: f32,
    pub lifetime: f32,
    layer: Layer,
    velocity: Vec3,
    size: f32,
    spin: Vec3,
    roll: f32,
    variant: u32,
    opacity: f32,
    palette: usize,
    dust_slot: Option<usize>,
    bounces: u8,
    settled: bool,
    sweep_origin: Vec3,
    support: Option<Entity>,
}

#[derive(Resource)]
struct VfxAssets {
    root: Entity,
    billboard: Handle<Mesh>,
    debris: Vec<Handle<Mesh>>,
    solids: Vec<Vec<Handle<StandardMaterial>>>,
    dust: Handle<DustMaterial>,
    dust_instances: Handle<ShaderBuffer>,
    free_dust: Vec<usize>,
}

#[derive(Component)]
struct PipelineWarmup {
    frames_remaining: u16,
}

fn material_palette(material: &str) -> usize {
    match material {
        "steel" => 1,
        "wood" | "timber" => 2,
        "brick" | "masonry" => 3,
        _ => 0,
    }
}

fn palette_color(index: usize) -> [f32; 3] {
    [
        [0.43, 0.40, 0.34],
        [0.28, 0.31, 0.34],
        [0.42, 0.26, 0.12],
        [0.51, 0.25, 0.15],
    ][index]
}

fn random(state: &mut u64) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    (*state >> 40) as f32 / (1_u32 << 24) as f32
}

fn debris_mesh(seed: u64) -> Mesh {
    let mut rng = seed;
    let mut vertices = [
        Vec3::X,
        Vec3::NEG_X,
        Vec3::Y,
        Vec3::NEG_Y,
        Vec3::Z,
        Vec3::NEG_Z,
    ];
    for vertex in &mut vertices {
        *vertex *= 0.38 + random(&mut rng) * 0.25;
        *vertex += Vec3::new(
            random(&mut rng) - 0.5,
            random(&mut rng) - 0.5,
            random(&mut rng) - 0.5,
        ) * 0.12;
        vertex.y *= 0.65;
    }
    let min = vertices.iter().copied().reduce(Vec3::min).unwrap();
    let max = vertices.iter().copied().reduce(Vec3::max).unwrap();
    let extent = (max - min).max_element();
    for vertex in &mut vertices {
        *vertex /= extent;
    }
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    for face in [
        [2, 0, 4],
        [2, 4, 1],
        [2, 1, 5],
        [2, 5, 0],
        [3, 4, 0],
        [3, 1, 4],
        [3, 5, 1],
        [3, 0, 5],
    ] {
        let mut triangle = face.map(|i| vertices[i]);
        let mut normal = (triangle[1] - triangle[0])
            .cross(triangle[2] - triangle[0])
            .normalize();
        if normal.dot(triangle.iter().copied().sum::<Vec3>()) < 0.0 {
            triangle.swap(1, 2);
            normal = -normal;
        }
        positions.extend(triangle.map(|point| point.to_array()));
        normals.extend([normal.to_array(); 3]);
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut dust_materials: ResMut<Assets<DustMaterial>>,
    mut dust_buffers: ResMut<Assets<ShaderBuffer>>,
) {
    let root = commands
        .spawn_scene(bsn! { Name::new("VfxRoot") Transform::IDENTITY Visibility::Visible })
        .id();
    let atlas = images.add(dust_atlas());
    let dust_instances = dust_buffers.add(ShaderBuffer::from(vec![
        dust_settings("concrete")
            .packed();
        DUST_POOL_SIZE + 1
    ]));
    let dust = dust_materials.add(dust_material(atlas, dust_instances.clone()));
    let solids: Vec<Vec<Handle<StandardMaterial>>> = (0..4)
        .map(|palette| {
            let [r, g, b] = palette_color(palette);
            (0..DEBRIS_FADE_STEPS)
                .map(|step| {
                    materials.add(StandardMaterial {
                        base_color: Color::srgba(
                            r,
                            g,
                            b,
                            1.0 - step as f32 / DEBRIS_FADE_STEPS as f32,
                        ),
                        metallic: if palette == 1 { 0.65 } else { 0.0 },
                        perceptual_roughness: if palette == 1 { 0.72 } else { 0.98 },
                        alpha_mode: if step == 0 {
                            AlphaMode::Opaque
                        } else {
                            AlphaMode::Blend
                        },
                        ..default()
                    })
                })
                .collect()
        })
        .collect();
    let billboard = meshes.add(Rectangle::new(1.0, 1.0));
    let debris = (1..=8)
        .map(|seed| meshes.add(debris_mesh(seed * 104729)))
        .collect::<Vec<_>>();
    let mut warmup_settings = dust_settings("concrete");
    warmup_settings.update(0.0, 1.0, 0, 0.0, 0.0);
    commands.spawn((
        Name::new("Dust pipeline warmup"),
        PipelineWarmup {
            frames_remaining: 90,
        },
        NoFrustumCulling,
        NotShadowCaster,
        Mesh3d(billboard.clone()),
        MeshMaterial3d(dust.clone()),
        warmup_settings,
        MeshTag::new(DUST_POOL_SIZE as u32),
        Transform::from_xyz(0.0, 0.0, -1.0).with_scale(Vec3::splat(0.00001)),
        Visibility::Visible,
    ));
    for material in solids[0].iter().take(2) {
        commands.spawn((
            Name::new("Debris pipeline warmup"),
            PipelineWarmup {
                frames_remaining: 90,
            },
            NoFrustumCulling,
            NotShadowCaster,
            Mesh3d(debris[0].clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_xyz(0.0, 0.0, -1.0).with_scale(Vec3::splat(0.00001)),
            Visibility::Visible,
        ));
    }
    commands.insert_resource(VfxAssets {
        root,
        billboard,
        debris,
        solids,
        dust,
        dust_instances,
        free_dust: (0..DUST_POOL_SIZE).rev().collect(),
    });
}

fn prewarm_pipelines(
    mut commands: Commands,
    cameras: Query<&Transform, With<DemoCamera>>,
    mut warmups: Query<(Entity, &mut PipelineWarmup, &mut Transform), Without<DemoCamera>>,
) {
    for (entity, mut warmup, mut transform) in &mut warmups {
        warmup.frames_remaining -= 1;
        if warmup.frames_remaining == 0 {
            commands.entity(entity).despawn();
        } else if let Ok(camera) = cameras.single() {
            transform.translation =
                camera.translation + camera.rotation * Vec3::new(0.35, -0.2, -1.0);
        }
    }
}

struct Resident {
    entity: Entity,
    layer: Layer,
    age: f32,
    dust_slot: Option<usize>,
    retired: bool,
}

fn retire(
    resident: &mut Resident,
    commands: &mut Commands,
    assets: &mut VfxAssets,
    counts: &mut [usize; 4],
    stats: &mut VfxStats,
) {
    resident.retired = true;
    commands.entity(resident.entity).despawn();
    counts[resident.layer.index()] -= 1;
    if let Some(slot) = resident.dust_slot {
        assets.free_dust.push(slot);
    }
    stats.retired += 1;
}

fn sample_surface(surfaces: &[EmissionSurface], rng: &mut u64) -> Option<(Vec3, Vec3)> {
    let area = |s: &EmissionSurface| {
        (s.triangle[1] - s.triangle[0])
            .cross(s.triangle[2] - s.triangle[0])
            .length()
            * 0.5
    };
    let mut target = random(rng) * surfaces.iter().map(area).sum::<f32>();
    for surface in surfaces {
        let area = area(surface);
        target -= area;
        if target <= 0.0 && area > 0.0 {
            let u = random(rng).sqrt();
            let v = random(rng);
            return Some((
                surface.triangle[0] * (1.0 - u)
                    + surface.triangle[1] * (u * (1.0 - v))
                    + surface.triangle[2] * (u * v),
                surface.normal,
            ));
        }
    }
    None
}

fn spawn_particle(burst: &BreakBurst, layer: Layer, rng: &mut u64) -> (Particle, Transform) {
    let surface = if matches!(layer, Layer::Smoke | Layer::Fallout) {
        sample_surface(&burst.surfaces, rng)
    } else {
        None
    };
    let (point, normal) = surface.unwrap_or((burst.point, burst.normal));
    let normal = normal.normalize_or(Vec3::Y);
    let tangent = normal.any_orthonormal_vector();
    let bitangent = normal.cross(tangent);
    let jitter = tangent * (random(rng) * 2.0 - 1.0) + bitangent * (random(rng) * 2.0 - 1.0);
    let strength = (burst.removed_volume.max(0.0).cbrt() * 0.6
        + burst.exposed_area.max(0.0).sqrt() * 0.12)
        .clamp(0.7, 1.5);
    let (delay, lifetime, size, speed, opacity) = match layer {
        Layer::Impact => (
            0.0,
            0.16 + random(rng) * 0.12,
            0.10 + random(rng) * 0.16,
            1.6 + random(rng) * 2.0,
            0.8,
        ),
        Layer::Debris => (
            0.04 + random(rng) * 0.12,
            2.0 + random(rng) * 0.7,
            0.05 + random(rng) * 0.20,
            1.5 + random(rng) * 2.7,
            1.0,
        ),
        Layer::Smoke => (
            0.08 + random(rng) * 0.18,
            1.4 + random(rng) * 1.1,
            0.20 + random(rng) * 0.35,
            0.25 + random(rng) * 0.55,
            0.40,
        ),
        Layer::Fallout => (
            0.3 + random(rng) * 0.7,
            0.7 + random(rng) * 0.8,
            0.025 + random(rng) * 0.06,
            0.15 + random(rng) * 0.4,
            0.26,
        ),
    };
    let velocity = match layer {
        Layer::Debris => {
            (normal * (0.6 + random(rng) * 0.5) + jitter * 0.65 + Vec3::Y * 0.35) * speed
        }
        Layer::Fallout => normal * speed + jitter * 0.18 - Vec3::Y * 0.25,
        _ => normal * speed + jitter * speed * 0.45 + Vec3::Y * 0.12,
    };
    let particle = Particle {
        age: -delay,
        lifetime,
        layer,
        velocity,
        size: if layer == Layer::Debris {
            (size * strength).clamp(0.05, 0.25)
        } else {
            size * strength
        },
        spin: Vec3::new(
            random(rng) * 8.0 - 4.0,
            random(rng) * 8.0 - 4.0,
            random(rng) * 8.0 - 4.0,
        ),
        roll: random(rng) * std::f32::consts::TAU,
        variant: (random(rng) * 4.0) as u32,
        opacity,
        palette: material_palette(&burst.material),
        dust_slot: None,
        bounces: 0,
        settled: false,
        sweep_origin: point + normal * 0.045 + jitter * 0.025,
        support: None,
    };
    let transform = Transform::from_translation(point + normal * 0.045 + jitter * 0.025)
        .with_scale(Vec3::splat(particle.size));
    (particle, transform)
}

fn screen_footprint(
    camera: &Transform,
    projection: Option<&Projection>,
    position: Vec3,
    size: f32,
) -> f32 {
    let view = camera.rotation.inverse() * (position - camera.translation);
    if view.z >= 0.0 {
        return 0.0;
    }
    let (scale_x, scale_y) = match projection {
        Some(Projection::Orthographic(p)) => (1.0 / p.area.width(), 1.0 / p.area.height()),
        _ => {
            let (fov, aspect, near) = match projection {
                Some(Projection::Perspective(p)) => (p.fov, p.aspect_ratio, p.near),
                _ => (crate::camera::FIELD_OF_VIEW, 16.0 / 9.0, 0.05),
            };
            let scale_y = 1.0 / (2.0 * (fov * 0.5).tan() * (-view.z).max(near));
            (scale_y / aspect, scale_y)
        }
    };
    let half_size = size * 0.5;
    if (view.x.abs() - half_size) * scale_x > 0.5 || (view.y.abs() - half_size) * scale_y > 0.5 {
        return 0.0;
    }
    (size * size * scale_x * scale_y).min(1.0)
}

#[allow(clippy::too_many_arguments)]
fn spawn_bursts(
    mut commands: Commands,
    mut bursts: MessageReader<BreakBurst>,
    mut assets: ResMut<VfxAssets>,
    mut dust_buffers: ResMut<Assets<ShaderBuffer>>,
    settings: Res<VfxSettings>,
    mut stats: ResMut<VfxStats>,
    particles: Query<(Entity, &Particle, &Transform), Without<DemoCamera>>,
    cameras: Query<(&Transform, Option<&Projection>), With<DemoCamera>>,
) {
    let started = Instant::now();
    let mut residents = particles
        .iter()
        .map(|(entity, p, _)| Resident {
            entity,
            layer: p.layer,
            age: p.age,
            dust_slot: p.dust_slot,
            retired: false,
        })
        .collect::<Vec<_>>();
    let mut counts = [0; 4];
    for resident in &residents {
        counts[resident.layer.index()] += 1;
    }
    let fallback_camera = Transform::from_xyz(0.0, 0.0, 8.0);
    let (camera, projection) = cameras.single().unwrap_or((&fallback_camera, None));
    let camera_position = camera.translation;
    let coverage = particles
        .iter()
        .filter(|(_, p, _)| p.layer == Layer::Smoke && p.age >= 0.0)
        .map(|(_, _, t)| screen_footprint(camera, projection, t.translation, t.scale.max_element()))
        .sum::<f32>();
    stats.estimated_coverage = coverage;
    stats.peak_estimated_coverage = stats.peak_estimated_coverage.max(coverage);
    let incoming = bursts.read().collect::<Vec<_>>();
    let feedback_share = (LAYER_LIMITS[0] / incoming.len().max(1)).max(1);
    for burst in incoming {
        stats.received += 1;
        if !settings.enabled {
            continue;
        }
        let distance = camera_position.distance(burst.point);
        let distance_lod: f32 = if distance > 45.0 {
            0.25
        } else if distance > 24.0 {
            0.5
        } else {
            1.0
        };
        let smoke_lod = if coverage > 0.45 {
            0.35
        } else if coverage > 0.25 {
            0.65
        } else {
            1.0
        };
        let mut seed = burst.seed.max(1);
        let mut has_feedback = false;
        for (index, base_count) in BURST_COUNTS.into_iter().enumerate() {
            let layer = Layer::from_index(index);
            let lod = if layer == Layer::Impact {
                distance_lod.max(0.5)
            } else {
                distance_lod
            } * settings.density.clamp(0.1, 1.0);
            let requested = (base_count as f32
                * lod
                * if layer == Layer::Smoke {
                    smoke_lod
                } else {
                    1.0
                })
            .round()
            .max(1.0) as usize;
            let requested = if layer == Layer::Impact {
                requested.min(feedback_share)
            } else {
                requested
            };
            for _ in 0..requested {
                if layer == Layer::Impact {
                    while counts[0] >= LAYER_LIMITS[0]
                        || counts.iter().sum::<usize>() >= PARTICLE_LIMIT
                        || assets.free_dust.is_empty()
                    {
                        let same_layer_full = counts[0] >= LAYER_LIMITS[0];
                        let candidate = residents
                            .iter_mut()
                            .filter(|r| {
                                !r.retired && (!same_layer_full || r.layer == Layer::Impact)
                            })
                            .max_by(|a, b| {
                                let rank = |layer| match layer {
                                    Layer::Smoke => 3,
                                    Layer::Fallout => 2,
                                    Layer::Impact => 1,
                                    Layer::Debris => 0,
                                };
                                rank(a.layer)
                                    .cmp(&rank(b.layer))
                                    .then(a.age.total_cmp(&b.age))
                                    .then(a.entity.to_bits().cmp(&b.entity.to_bits()))
                            });
                        let Some(candidate) = candidate else {
                            break;
                        };
                        retire(
                            candidate,
                            &mut commands,
                            &mut assets,
                            &mut counts,
                            &mut stats,
                        );
                    }
                }
                if counts[index] >= LAYER_LIMITS[index]
                    || counts.iter().sum::<usize>() >= PARTICLE_LIMIT
                    || (layer != Layer::Debris && assets.free_dust.is_empty())
                {
                    stats.dropped[index] += 1;
                    continue;
                }
                let (mut particle, transform) = spawn_particle(burst, layer, &mut seed);
                let mut entity = commands.spawn((
                    Name::new("Transient fracture particle"),
                    ChildOf(assets.root),
                    NotShadowCaster,
                    transform,
                    if particle.age < 0.0 {
                        Visibility::Hidden
                    } else {
                        Visibility::Visible
                    },
                ));
                if layer == Layer::Debris {
                    let mesh_index = (random(&mut seed) * assets.debris.len() as f32) as usize;
                    entity.insert((
                        Mesh3d(assets.debris[mesh_index].clone()),
                        MeshMaterial3d(assets.solids[particle.palette][0].clone()),
                    ));
                } else {
                    let slot = assets.free_dust.pop().unwrap();
                    particle.dust_slot = Some(slot);
                    let mut dust = dust_settings(&burst.material);
                    dust.update(
                        0.0,
                        particle.lifetime,
                        particle.variant,
                        particle.opacity,
                        particle.roll,
                    );
                    dust_buffers
                        .get_mut(&assets.dust_instances)
                        .unwrap()
                        .cast_slice_mut::<[[f32; 4]; 4]>()
                        .unwrap()[slot] = dust.packed();
                    entity.insert((
                        Mesh3d(assets.billboard.clone()),
                        MeshMaterial3d(assets.dust.clone()),
                        dust,
                        MeshTag::new(slot as u32),
                    ));
                }
                let resident = Resident {
                    entity: entity.id(),
                    layer,
                    age: particle.age,
                    dust_slot: particle.dust_slot,
                    retired: false,
                };
                entity.insert(particle);
                residents.push(resident);
                counts[index] += 1;
                stats.spawned[index] += 1;
                if layer == Layer::Impact {
                    stats.first_feedbacks += 1;
                    has_feedback = true;
                }
            }
        }
        if has_feedback {
            stats.feedback_bursts += 1;
        }
    }
    stats.peak = stats.peak.max(counts.iter().sum());
    stats.emit_cpu_ms = started.elapsed().as_secs_f32() * 1000.0;
    stats.max_emit_cpu_ms = stats.max_emit_cpu_ms.max(stats.emit_cpu_ms);
}

fn curl(position: Vec3, age: f32) -> Vec3 {
    let p = position * 1.7 + Vec3::new(age * 0.3, age * 0.2, -age * 0.25);
    Vec3::new(
        p.y.cos() - p.z.sin(),
        p.z.cos() - p.x.sin(),
        p.x.cos() - p.y.sin(),
    ) * 0.12
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn animate_particles(
    mut commands: Commands,
    time: Res<Time>,
    settings: Res<VfxSettings>,
    mut assets: ResMut<VfxAssets>,
    mut dust_buffers: ResMut<Assets<ShaderBuffer>>,
    mut stats: ResMut<VfxStats>,
    spatial: SpatialQuery,
    cameras: Query<&Transform, With<DemoCamera>>,
    chunks: Query<&ChunkInstance, Without<RemovedChunk>>,
    colliders: Query<&ColliderOf>,
    bodies: Query<&RigidBody>,
    mut particles: Query<
        (
            Entity,
            &mut Particle,
            &mut Transform,
            &mut Visibility,
            Option<&mut MeshMaterial3d<StandardMaterial>>,
            Option<&mut DustSettings>,
        ),
        Without<DemoCamera>,
    >,
) {
    let started = Instant::now();
    let dt = time.delta_secs();
    let billboard_rotation = cameras
        .single()
        .map_or(Quat::IDENTITY, |camera| camera.rotation);
    let mut active = [0; 4];
    let mut scheduled = [0; 4];
    let mut collision_queries = 0;
    let mut collision_candidates = particles
        .iter()
        .filter(|(_, p, _, _, _, _)| p.layer == Layer::Debris && !p.settled && p.age + dt >= 0.0)
        .map(|(entity, _, _, _, _, _)| entity)
        .collect::<Vec<_>>();
    collision_candidates.sort_by_key(|entity| entity.to_bits());
    if !collision_candidates.is_empty() {
        let start = stats.collision_cursor % collision_candidates.len();
        collision_candidates.rotate_left(start);
        stats.collision_cursor = start + MAX_COLLISION_QUERIES;
        collision_candidates.truncate(MAX_COLLISION_QUERIES);
    }
    let live_collider = |entity| chunks.get(entity).is_ok_and(|chunk| chunk.alive);
    let static_collider = |entity| {
        colliders
            .get(entity)
            .ok()
            .and_then(|collider| bodies.get(collider.body).ok())
            .is_some_and(|body| *body == RigidBody::Static)
    };
    let has_dust = particles
        .iter()
        .any(|(_, particle, _, _, _, _)| particle.dust_slot.is_some());
    let mut buffer = has_dust.then(|| dust_buffers.get_mut(&assets.dust_instances).unwrap());
    let mut packed = buffer
        .as_mut()
        .map(|buffer| buffer.cast_slice_mut::<[[f32; 4]; 4]>().unwrap());
    for (entity, mut particle, mut transform, mut visibility, solid, dust) in &mut particles {
        let old_age = particle.age;
        particle.age += dt;
        if !settings.enabled || particle.age >= particle.lifetime {
            if let Some(slot) = particle.dust_slot {
                assets.free_dust.push(slot);
            }
            commands.entity(entity).despawn();
            continue;
        }
        if particle.age < 0.0 {
            scheduled[particle.layer.index()] += 1;
            continue;
        }
        active[particle.layer.index()] += 1;
        *visibility = Visibility::Visible;
        let step = if old_age < 0.0 {
            particle.age.min(dt)
        } else {
            dt
        };
        let fraction = particle.age / particle.lifetime;
        if particle.layer == Layer::Debris {
            if particle.settled
                && particle
                    .support
                    .is_some_and(|entity| !live_collider(entity) || !static_collider(entity))
            {
                particle.settled = false;
                particle.sweep_origin = transform.translation;
            }
            if !particle.settled {
                let displacement = particle.velocity * step - Vec3::Y * (4.905 * step * step);
                particle.velocity.y -= 9.81 * step;
                transform.translation += displacement;
                let swept_displacement = transform.translation - particle.sweep_origin;
                if collision_candidates.contains(&entity)
                    && swept_displacement.length_squared() > 1e-10
                {
                    collision_queries += 1;
                    let direction = Dir3::new(swept_displacement).unwrap();
                    let config = ShapeCastConfig {
                        max_distance: swept_displacement.length(),
                        ignore_origin_penetration: true,
                        ..default()
                    };
                    let shape = Collider::sphere(particle.size * 0.45);
                    let hit = spatial.cast_shape_predicate(
                        &shape,
                        particle.sweep_origin,
                        Quat::IDENTITY,
                        direction,
                        &config,
                        &SpatialQueryFilter::default(),
                        &live_collider,
                    );
                    if let Some(hit) = hit {
                        let normal = hit.normal1;
                        let speed = particle.velocity.dot(normal);
                        if speed < 0.0 {
                            transform.translation =
                                particle.sweep_origin + *direction * hit.distance + normal * 0.003;
                            let tangent = particle.velocity - normal * speed;
                            particle.velocity = tangent * 0.58 - normal * speed * 0.28;
                            particle.bounces += 1;
                            stats.collisions += 1;
                            if normal.y > 0.5
                                && static_collider(hit.entity)
                                && (particle.velocity.length_squared() < 0.25
                                    || particle.bounces >= 2)
                            {
                                particle.settled = true;
                                particle.velocity = Vec3::ZERO;
                                particle.support = Some(hit.entity);
                            }
                        }
                    }
                    particle.sweep_origin = transform.translation;
                }
                transform.rotate(Quat::from_scaled_axis(particle.spin * step));
            }
            if let Some(mut solid) = solid {
                let fade = ((fraction - 0.82) / 0.18).clamp(0.0, 0.999);
                solid.0 = assets.solids[particle.palette]
                    [(fade * DEBRIS_FADE_STEPS as f32) as usize]
                    .clone();
            }
        } else {
            transform.rotation = billboard_rotation;
            if particle.layer == Layer::Fallout {
                particle.velocity.y -= 1.8 * step;
            } else {
                particle.velocity *= (-1.4 * step).exp();
                let flow = curl(transform.translation, particle.age);
                particle.velocity += (flow + Vec3::Y * 0.18) * step;
            }
            transform.translation += particle.velocity * step;
            let growth = match particle.layer {
                Layer::Impact => 1.0 + fraction * 3.0,
                Layer::Smoke => 1.0 + (1.0 - (-fraction * 5.0).exp()) * 2.5,
                _ => 1.0 + fraction * 1.4,
            };
            transform.scale = Vec3::splat(particle.size * growth);
            if let Some(mut dust) = dust {
                dust.update(
                    particle.age,
                    particle.lifetime,
                    particle.variant,
                    particle.opacity,
                    particle.roll,
                );
                if let Some(packed) = packed.as_mut() {
                    packed[particle.dust_slot.unwrap()] = dust.packed();
                }
            }
        }
    }
    stats.active = active;
    stats.scheduled = scheduled;
    stats.collision_queries += collision_queries as u64;
    stats.peak = stats
        .peak
        .max(active.iter().sum::<usize>() + scheduled.iter().sum::<usize>());
    stats.update_cpu_ms = started.elapsed().as_secs_f32() * 1000.0;
    stats.max_update_cpu_ms = stats.max_update_cpu_ms.max(stats.update_cpu_ms);
}

#[cfg(test)]
mod tests;
