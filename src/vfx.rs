use bevy::{
    asset::RenderAssetUsages,
    image::Image,
    light::NotShadowCaster,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};

use crate::scene::DemoCamera;

pub const PARTICLE_LIMIT: usize = 640;
const PARTICLES_PER_BURST: usize = 64;
const FADE_STEPS: usize = 16;

pub struct VfxPlugin;

impl Plugin for VfxPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<BreakBurst>()
            .add_systems(Startup, setup)
            .add_systems(
                Update,
                (
                    spawn_bursts,
                    animate_particles.after(crate::camera::move_camera),
                )
                    .chain(),
            );
    }
}

#[derive(Message)]
pub struct BreakBurst {
    pub point: Vec3,
    pub normal: Vec3,
    pub seed: u64,
    pub exposed_points: Vec<Vec3>,
}

#[derive(Component)]
pub struct Particle {
    pub age: f32,
    pub lifetime: f32,
    velocity: Vec3,
    size: f32,
    kind: usize,
    spin: f32,
}

#[derive(Resource)]
struct VfxAssets {
    root: Entity,
    billboard: Handle<Mesh>,
    debris: Handle<Mesh>,
    fades: Vec<Handle<StandardMaterial>>,
    solid: Handle<StandardMaterial>,
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let root = commands
        .spawn_scene(bsn! { Name::new("VfxRoot") Transform::IDENTITY Visibility::Visible })
        .id();
    let mut pixels = Vec::new();
    for y in 0..64 {
        for x in 0..64 {
            let radius =
                Vec2::new((x as f32 + 0.5) / 32.0 - 1.0, (y as f32 + 0.5) / 32.0 - 1.0).length();
            let alpha = (1.0 - radius).max(0.0).powf(1.8);
            pixels.extend_from_slice(&[255, 255, 255, (alpha * 255.0) as u8]);
        }
    }
    let texture = images.add(Image::new(
        Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    ));
    let fades = (0..FADE_STEPS)
        .map(|step| {
            materials.add(StandardMaterial {
                base_color: Color::srgba(
                    0.63,
                    0.59,
                    0.52,
                    (1.0 - step as f32 / FADE_STEPS as f32) * 0.48,
                ),
                base_color_texture: Some(texture.clone()),
                alpha_mode: AlphaMode::Blend,
                unlit: true,
                cull_mode: None,
                ..default()
            })
        })
        .collect();
    commands.insert_resource(VfxAssets {
        root,
        billboard: meshes.add(Rectangle::new(1.0, 1.0)),
        debris: meshes.add(Cuboid::new(1.0, 0.7, 0.6)),
        fades,
        solid: materials.add(StandardMaterial {
            base_color: Color::srgb(0.39, 0.36, 0.31),
            perceptual_roughness: 1.0,
            ..default()
        }),
    });
}

fn random(state: &mut u64) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    (*state >> 40) as f32 / (1_u32 << 24) as f32
}

fn spawn_bursts(
    mut commands: Commands,
    mut bursts: MessageReader<BreakBurst>,
    assets: Res<VfxAssets>,
    particles: Query<(), With<Particle>>,
) {
    let mut available = PARTICLE_LIMIT.saturating_sub(particles.iter().len());
    for burst in bursts.read() {
        let count = available.min(PARTICLES_PER_BURST);
        available -= count;
        let mut seed = burst.seed.max(1);
        for i in 0..count {
            let kind = if i < 16 {
                0
            } else if i < 44 {
                1
            } else {
                2
            };
            let jitter = Vec3::new(
                random(&mut seed) * 2.0 - 1.0,
                random(&mut seed) * 2.0 - 1.0,
                random(&mut seed) * 2.0 - 1.0,
            );
            let (speed, size, lifetime) = match kind {
                0 => (
                    2.7,
                    0.035 + random(&mut seed) * 0.055,
                    0.7 + random(&mut seed) * 0.6,
                ),
                1 => (
                    0.55,
                    0.22 + random(&mut seed) * 0.35,
                    1.8 + random(&mut seed) * 0.7,
                ),
                _ => (
                    1.4,
                    0.06 + random(&mut seed) * 0.11,
                    0.35 + random(&mut seed) * 0.45,
                ),
            };
            let velocity =
                (burst.normal * (0.35 + random(&mut seed)) + jitter * 0.7 + Vec3::Y * 0.4) * speed;
            let origin = if kind == 1 && i % 2 == 0 && !burst.exposed_points.is_empty() {
                burst.exposed_points[(i / 2) % burst.exposed_points.len()]
            } else {
                burst.point
            };
            commands.spawn((
                Name::new("Transient fracture particle"),
                ChildOf(assets.root),
                NotShadowCaster,
                Mesh3d(if kind == 0 {
                    assets.debris.clone()
                } else {
                    assets.billboard.clone()
                }),
                MeshMaterial3d(if kind == 0 {
                    assets.solid.clone()
                } else {
                    assets.fades[0].clone()
                }),
                Transform::from_translation(origin + burst.normal * 0.035 + jitter * 0.08)
                    .with_scale(Vec3::splat(size)),
                Particle {
                    age: 0.0,
                    lifetime,
                    velocity,
                    size,
                    kind,
                    spin: random(&mut seed) * 8.0 - 4.0,
                },
            ));
        }
    }
}

fn animate_particles(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<VfxAssets>,
    cameras: Query<&Transform, With<DemoCamera>>,
    mut particles: Query<
        (
            Entity,
            &mut Particle,
            &mut Transform,
            &mut MeshMaterial3d<StandardMaterial>,
        ),
        Without<DemoCamera>,
    >,
) {
    let dt = time.delta_secs();
    let billboard_rotation = cameras
        .single()
        .map_or(Quat::IDENTITY, |camera| camera.rotation);
    for (entity, mut particle, mut transform, mut material) in &mut particles {
        particle.age += dt;
        if particle.age >= particle.lifetime {
            commands.entity(entity).despawn();
            continue;
        }
        let fraction = particle.age / particle.lifetime;
        transform.translation += particle.velocity * dt;
        if particle.kind == 0 {
            particle.velocity.y -= 5.0 * dt;
            transform.rotate_local_x(particle.spin * dt);
            transform.rotate_local_y(particle.spin * dt * 0.7);
            transform.scale = Vec3::splat(particle.size * (1.0 - fraction * fraction));
        } else {
            transform.rotation = billboard_rotation;
            particle.velocity *= (-1.2 * dt).exp();
            particle.velocity.y += 0.45 * dt;
            transform.scale = Vec3::splat(particle.size * (1.0 + fraction * 3.5));
            material.0 =
                assets.fades[((fraction * FADE_STEPS as f32) as usize).min(FADE_STEPS - 1)].clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_bursts_share_assets_respect_budget_and_expire() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            bevy::scene::ScenePlugin,
        ))
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<Image>>()
        .init_resource::<Assets<StandardMaterial>>()
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f64(1.0 / 60.0),
        ))
        .add_plugins(VfxPlugin);
        app.update();
        let rotation = Quat::from_euler(EulerRot::YXZ, 0.8, -0.3, 0.0);
        let camera = app
            .world_mut()
            .spawn((DemoCamera, Transform::from_rotation(rotation)))
            .id();
        let materials = app.world().resource::<Assets<StandardMaterial>>().len();
        for _ in 0..3 {
            for seed in 1..=24 {
                app.world_mut().write_message(BreakBurst {
                    point: Vec3::ZERO,
                    normal: Vec3::Z,
                    seed,
                    exposed_points: vec![Vec3::X],
                });
            }
            app.update();
            let count = app
                .world_mut()
                .query::<&Particle>()
                .iter(app.world())
                .count();
            assert_eq!(count, PARTICLE_LIMIT);
            assert!(
                app.world_mut()
                    .query::<(&Particle, &Transform)>()
                    .iter(app.world())
                    .filter(|(p, _)| p.kind != 0)
                    .all(|(_, t)| t
                        .rotation
                        .angle_between(app.world().get::<Transform>(camera).unwrap().rotation)
                        < 0.001)
            );
            assert_eq!(
                app.world().resource::<Assets<StandardMaterial>>().len(),
                materials
            );
        }
        app.world_mut()
            .get_mut::<Transform>(camera)
            .unwrap()
            .rotation = Quat::from_rotation_y(-0.6);
        app.update();
        assert!(
            app.world_mut()
                .query::<(&Particle, &Transform)>()
                .iter(app.world())
                .filter(|(p, _)| p.kind != 0)
                .all(|(_, t)| t.rotation.angle_between(Quat::from_rotation_y(-0.6)) < 0.001)
        );
        for _ in 0..180 {
            app.update();
        }
        assert_eq!(
            app.world_mut()
                .query::<&Particle>()
                .iter(app.world())
                .count(),
            0
        );
        assert_eq!(
            app.world().resource::<Assets<StandardMaterial>>().len(),
            materials
        );
    }
}
