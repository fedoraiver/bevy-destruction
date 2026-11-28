use std::{collections::BTreeMap, sync::Arc};

use bevy::{
    asset::RenderAssetUsages, camera::visibility::NoFrustumCulling, mesh::PrimitiveTopology,
    prelude::*,
};

use crate::{
    asset::DestructibleAsset,
    fracture_detail::detail_mesh,
    material::{BuildingMaterial, rim_material, surface_coordinates},
    settings::DemoSettings,
};

use super::{
    BondInstance, BuildingInstance, ChunkInstance, DamageDetail, DemoCamera, DetailStats,
    PatchInstance, PatchOfBond, RefreshDamageDetails,
};

const DETAIL_TRIANGLE_LIMIT: usize = 180_000;

#[derive(Resource, Default)]
pub(crate) struct RimMaterials {
    by_material: BTreeMap<String, Handle<BuildingMaterial>>,
}

impl RimMaterials {
    pub(super) fn get_or_create(
        &mut self,
        material_id: &str,
        materials: &mut Assets<BuildingMaterial>,
    ) -> Handle<BuildingMaterial> {
        self.by_material
            .entry(material_id.into())
            .or_insert_with(|| materials.add(rim_material(material_id)))
            .clone()
    }
}

#[derive(Component)]
pub(super) struct RimPipelineWarmup {
    frames_remaining: u32,
}

struct CachedDetailMesh {
    mesh: Mesh,
    triangles: usize,
}

#[derive(Resource, Default)]
pub(super) struct DetailMeshCache {
    by_patch: BTreeMap<(AssetId<DestructibleAsset>, usize, bool), Option<Arc<CachedDetailMesh>>>,
}

pub(super) fn warm_rim_pipeline(
    mut commands: Commands,
    settings: Res<DemoSettings>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<BuildingMaterial>>,
    mut rim_materials: ResMut<RimMaterials>,
) {
    if !settings.detail {
        return;
    }
    // Match the real rim vertex layout, including attached surface coordinates and its prepass.
    let mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[-0.5, -0.5, 0.0], [0.5, -0.5, 0.0], [0.0, 0.5, 0.0]],
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; 3])
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0, 0.5, 1.0, 1.0]; 3]);
    let mesh = surface_coordinates(mesh, Transform::IDENTITY);
    commands.spawn((
        Name::new("Fracture rim pipeline warmup"),
        RimPipelineWarmup {
            frames_remaining: 90,
        },
        NoFrustumCulling,
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(rim_materials.get_or_create("concrete", &mut materials)),
        Transform::from_xyz(0.0, 0.0, -1.0).with_scale(Vec3::splat(0.00001)),
        Visibility::Visible,
    ));
}

pub(super) fn maintain_rim_warmup(
    mut commands: Commands,
    cameras: Query<&Transform, With<DemoCamera>>,
    mut warmups: Query<(Entity, &mut RimPipelineWarmup, &mut Transform), Without<DemoCamera>>,
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
#[allow(clippy::too_many_arguments)]
pub(super) fn update_details(
    mut commands: Commands,
    settings: Res<DemoSettings>,
    instances: Query<(Entity, &BuildingInstance)>,
    dirty: Query<Entity, (With<BuildingInstance>, With<RefreshDamageDetails>)>,
    chunks: Query<(&ChunkInstance, Option<&Children>)>,
    bonds: Query<&BondInstance>,
    patches: Query<(&PatchInstance, &PatchOfBond)>,
    details: Query<&DamageDetail>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<BuildingMaterial>>,
    mut rim_materials: ResMut<RimMaterials>,
    mut detail_cache: ResMut<DetailMeshCache>,
    mut stats: ResMut<DetailStats>,
) {
    if !settings.detail || dirty.is_empty() {
        return;
    }
    for building in &dirty {
        commands.entity(building).remove::<RefreshDamageDetails>();
    }
    let start = bevy::platform::time::Instant::now();
    let mut triangle_count = 0;
    let mut mesh_count = 0;
    let mut instance_ids = instances
        .iter()
        .map(|(entity, _)| entity)
        .collect::<Vec<_>>();
    instance_ids.sort_by_key(|entity| entity.to_bits());
    for instance_id in instance_ids {
        let (_, instance) = instances.get(instance_id).unwrap();
        let mut exposed = BTreeMap::<String, Vec<&PatchInstance>>::new();
        let alive = |id: &str| chunks.get(instance.chunks[id]).unwrap().0.alive;
        for &entity in &instance.patches {
            let (runtime_patch, owner) = patches.get(entity).unwrap();
            let patch = runtime_patch.patch();
            if let Some(survivor) = patch.exposed_survivor(alive) {
                exposed
                    .entry(survivor.to_owned())
                    .or_default()
                    .push(runtime_patch);
            } else if alive(&patch.a) && alive(&patch.b) && !bonds.get(owner.bond).unwrap().active {
                for id in [&patch.a, &patch.b] {
                    exposed.entry(id.clone()).or_default().push(runtime_patch);
                }
            }
        }
        for (id, &chunk_entity) in &instance.chunks {
            let (chunk, children) = chunks.get(chunk_entity).unwrap();
            let mut selected = Vec::new();
            let mut patch_ids = Vec::new();
            for &runtime_patch in exposed.get(id).into_iter().flatten() {
                let patch = runtime_patch.patch();
                let cached = detail_cache
                    .by_patch
                    .entry((instance.asset.id(), runtime_patch.index, patch.a == *id))
                    .or_insert_with(|| {
                        detail_mesh(patch, id).map(|mesh| {
                            let triangles = mesh
                                .indices()
                                .map_or(mesh.count_vertices(), |indices| indices.len())
                                / 3;
                            Arc::new(CachedDetailMesh { mesh, triangles })
                        })
                    });
                if let Some(cached) = cached {
                    if triangle_count + cached.triangles > DETAIL_TRIANGLE_LIMIT {
                        continue;
                    }
                    triangle_count += cached.triangles;
                    selected.push(cached.clone());
                    patch_ids.push(patch.id.clone());
                }
            }
            if let Some((entity, old)) = children
                .into_iter()
                .flatten()
                .find_map(|&entity| details.get(entity).ok().map(|detail| (entity, detail)))
            {
                if old.patch_ids == patch_ids {
                    mesh_count += 1;
                    continue;
                }
                commands.entity(entity).despawn();
            }
            let mut selected = selected.into_iter();
            let combined = selected.next().map(|first| {
                let mut mesh = first.mesh.clone();
                for part in selected {
                    mesh.merge(&part.mesh).expect("Consistent rim mesh layout");
                }
                mesh
            });
            if let Some(mesh) = combined {
                #[cfg(feature = "brp")]
                let triangles = mesh
                    .indices()
                    .map_or(mesh.count_vertices(), |indices| indices.len())
                    / 3;
                let material = rim_materials.get_or_create(&chunk.material, &mut materials);
                let local = Transform::from_matrix(chunk.transform.to_matrix().inverse());
                mesh_count += 1;
                commands.spawn((
                    Name::new("Exposed jagged fracture rim"),
                    ChildOf(chunk_entity),
                    Mesh3d(
                        meshes.add(
                            surface_coordinates(mesh, Transform::IDENTITY).transformed_by(local),
                        ),
                    ),
                    MeshMaterial3d(material),
                    Transform::IDENTITY,
                    DamageDetail {
                        patch_ids,
                        #[cfg(feature = "brp")]
                        triangles,
                    },
                ));
            }
        }
    }
    stats.active_meshes = mesh_count;
    stats.triangles = triangle_count;
    stats.cached_meshes = detail_cache
        .by_patch
        .values()
        .filter(|mesh| mesh.is_some())
        .count();
    stats.cached_triangles = detail_cache
        .by_patch
        .values()
        .flatten()
        .map(|mesh| mesh.triangles)
        .sum();
    stats.max_update_ms = stats
        .max_update_ms
        .max(start.elapsed().as_secs_f64() * 1000.0);
}

pub(super) fn detail_lod(
    cameras: Query<&GlobalTransform, With<DemoCamera>>,
    mut details: Query<(&GlobalTransform, &mut Visibility), With<DamageDetail>>,
) {
    let Ok(camera) = cameras.single() else {
        return;
    };
    for (transform, mut visibility) in &mut details {
        let distance = transform.translation().distance(camera.translation());
        if distance > 28.0 {
            *visibility = Visibility::Hidden;
        } else if distance < 24.0 {
            *visibility = Visibility::Inherited;
        }
    }
}
