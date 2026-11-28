use bevy::{
    asset::RenderAssetUsages,
    camera::visibility::NoFrustumCulling,
    light::NotShadowCaster,
    mesh::{MeshTag, PrimitiveTopology},
    prelude::*,
    render::storage::ShaderBuffer,
};

use crate::{
    dust_material::{DustMaterial, dust_atlas, dust_material, dust_settings},
    scene::DemoCamera,
};

use super::{DEBRIS_FADE_STEPS, DUST_POOL_SIZE, VfxAssets, random};

#[derive(Component)]
pub(super) struct PipelineWarmup {
    frames_remaining: u16,
}

fn palette_color(index: usize) -> [f32; 3] {
    [
        [0.43, 0.40, 0.34],
        [0.28, 0.31, 0.34],
        [0.42, 0.26, 0.12],
        [0.51, 0.25, 0.15],
    ][index]
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

pub(super) fn setup(
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

pub(super) fn prewarm_pipelines(
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
