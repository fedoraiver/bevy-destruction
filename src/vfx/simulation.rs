use std::time::Instant;

use avian3d::prelude::*;
use bevy::{prelude::*, render::storage::ShaderBuffer};

use crate::{
    dust_material::DustSettings,
    scene::{ChunkInstance, DemoCamera, RemovedChunk},
};

use super::{DEBRIS_FADE_STEPS, Layer, Particle, VfxAssets, VfxSettings, VfxStats};

const MAX_COLLISION_QUERIES: usize = 48;

fn curl(position: Vec3, age: f32) -> Vec3 {
    let p = position * 1.7 + Vec3::new(age * 0.3, age * 0.2, -age * 0.25);
    Vec3::new(
        p.y.cos() - p.z.sin(),
        p.z.cos() - p.x.sin(),
        p.x.cos() - p.y.sin(),
    ) * 0.12
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn animate_particles(
    mut commands: Commands,
    time: Res<Time>,
    settings: Res<VfxSettings>,
    mut assets: ResMut<VfxAssets>,
    mut dust_buffers: ResMut<Assets<ShaderBuffer>>,
    mut stats: ResMut<VfxStats>,
    mut collision_cursor: Local<usize>,
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
        let start = *collision_cursor % collision_candidates.len();
        collision_candidates.rotate_left(start);
        *collision_cursor = start + MAX_COLLISION_QUERIES;
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
