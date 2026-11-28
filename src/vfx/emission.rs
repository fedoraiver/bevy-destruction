use std::time::Instant;

use bevy::{light::NotShadowCaster, mesh::MeshTag, prelude::*, render::storage::ShaderBuffer};

use crate::{dust_material::dust_settings, scene::DemoCamera};

use super::{
    BreakBurst, EmissionSurface, LAYER_LIMITS, Layer, PARTICLE_LIMIT, Particle, VfxAssets,
    VfxSettings, VfxStats, random,
};

const BURST_COUNTS: [usize; 4] = [8, 8, 24, 24];

fn material_palette(material: &str) -> usize {
    match material {
        "steel" => 1,
        "wood" | "timber" => 2,
        "brick" | "masonry" => 3,
        _ => 0,
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

pub(super) fn spawn_particle(
    burst: &BreakBurst,
    layer: Layer,
    rng: &mut u64,
) -> (Particle, Transform) {
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

pub(super) fn screen_footprint(
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
pub(super) fn spawn_bursts(
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
