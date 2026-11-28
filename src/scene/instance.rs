use std::{collections::BTreeMap, sync::Arc};

use avian3d::prelude::ColliderDensity;
use bevy::{asset::LoadState, gltf::*, mesh::MeshTag, prelude::*, render::storage::ShaderBuffer};

use crate::{
    asset::{DestructibleAsset, SurfaceRole, mesh_triangles, resolve_building},
    debug::fracture_edges,
    fracture_detail::{SurfaceGeometry, build_patches},
    material::{BuildingMaterial, SurfaceMeshCache, building_material},
    menu::PendingReload,
    physics::NeedsClusterRebuild,
    settings::DemoSettings,
};

use super::{
    BuildingBounds, BuildingInstance, ChunkIdentity, ChunkInstance, ChunkPrimitive, DetailStats,
    chunk_collider, chunk_mass_properties, details::RimMaterials, geometry_cache::GeometryCache,
    spawn_bonds, spawn_patches,
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn instantiate_pending(
    mut commands: Commands,
    settings: Res<DemoSettings>,
    server: Res<AssetServer>,
    assets: Res<Assets<DestructibleAsset>>,
    gltfs: Res<Assets<Gltf>>,
    nodes: Res<Assets<GltfNode>>,
    gltf_meshes: Res<Assets<GltfMesh>>,
    mut meshes: ResMut<Assets<Mesh>>,
    gltf_materials: Res<Assets<GltfMaterial>>,
    mut materials: ResMut<Assets<BuildingMaterial>>,
    mut instances: Query<(Entity, &mut BuildingInstance, Option<&PendingReload>)>,
    mut cache: ResMut<GeometryCache>,
    mut rim_materials: ResMut<RimMaterials>,
    mut stats: ResMut<DetailStats>,
    mut surface_meshes: ResMut<SurfaceMeshCache>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
) {
    for (root, mut instance, reload) in &mut instances {
        let settings = reload.map_or(&*settings, |reload| &reload.settings);
        if instance.bounds.is_some() || instance.error.is_some() {
            continue;
        }
        if let Some(LoadState::Failed(error)) = server.get_load_state(instance.asset.id()) {
            instance.error = Some(
                "Asset could not load. Check the asset path or re-export the building.".into(),
            );
            error!("Building load failed: {error}");
            continue;
        }
        if let Some(bevy::asset::RecursiveDependencyLoadState::Failed(error)) =
            server.get_recursive_dependency_load_state(instance.asset.id())
        {
            instance.error = Some("Building mesh could not load. Re-export the building.".into());
            error!("Building dependency failed: {error}");
            continue;
        }
        if !server.is_loaded_with_dependencies(instance.asset.id()) {
            continue;
        }
        let Some(asset) = assets.get(&instance.asset) else {
            continue;
        };
        let prepared = (|| -> Result<_, String> {
            let building = resolve_building(
                asset,
                &gltfs,
                &nodes,
                &gltf_meshes,
                &meshes,
                &gltf_materials,
            )?;
            let mut chunks = Vec::new();
            let mut geometry = BTreeMap::new();
            for chunk in building.chunks {
                let record = &asset.manifest.chunks[chunk.record_index];
                if !cache.patches.contains_key(&instance.asset.id()) {
                    let transform = record.transform();
                    let exterior = chunk
                        .primitives
                        .iter()
                        .filter(|p| p.surface_role == SurfaceRole::Exterior)
                        .flat_map(|p| mesh_triangles(meshes.get(&p.mesh).unwrap()).unwrap())
                        .map(|t| t.map(|p| transform.transform_point(p)))
                        .collect();
                    geometry.insert(
                        record.id.clone(),
                        SurfaceGeometry {
                            triangles: chunk
                                .triangles
                                .iter()
                                .map(|t| t.map(|p| transform.transform_point(p)))
                                .collect(),
                            exterior,
                        },
                    );
                }
                let collider = chunk_collider(record)?;
                let density = asset.manifest.materials[&record.material].density;
                let mass_properties = chunk_mass_properties(&record.id, &collider, density)?;
                let edges = fracture_edges(
                    chunk
                        .primitives
                        .iter()
                        .map(|primitive| meshes.get(&primitive.mesh).unwrap()),
                )?;
                chunks.push((chunk, collider, density, mass_properties, edges));
            }
            let patches = if let Some(patches) = cache.patches.get(&instance.asset.id()) {
                patches.clone()
            } else {
                let start = bevy::platform::time::Instant::now();
                let patches = Arc::new(build_patches(&asset.manifest, &geometry)?);
                stats.cache_build_ms += start.elapsed().as_secs_f64() * 1000.0;
                cache.patches.insert(instance.asset.id(), patches.clone());
                patches
            };
            Ok((
                chunks,
                BuildingBounds {
                    min: building.bounds_min,
                    max: building.bounds_max,
                },
                patches,
            ))
        })();
        let (prepared, bounds, patches) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                error!("Invalid building: {error}");
                instance.error =
                    Some("Building data and mesh do not match. Re-export the building.".into());
                continue;
            }
        };
        let mask_started = bevy::platform::time::Instant::now();
        let sources = prepared.iter().flat_map(|(chunk, ..)| {
            let record = &asset.manifest.chunks[chunk.record_index];
            chunk
                .primitives
                .iter()
                .filter(|primitive| primitive.surface_role == SurfaceRole::Exterior)
                .map(move |primitive| {
                    (
                        chunk.record_index,
                        primitive.mesh.clone(),
                        record.transform(),
                        record.id.clone(),
                    )
                })
        });
        let masks = match cache.prepare_surface_masks(
            instance.asset.id(),
            sources,
            &patches,
            &mut meshes,
            &mut buffers,
        ) {
            Ok(masks) => masks,
            Err(error) => {
                error!("Invalid contact surface mask: {error}");
                instance.error = Some("Building contact surface could not be prepared.".into());
                continue;
            }
        };
        stats.cache_build_ms += mask_started.elapsed().as_secs_f64() * 1000.0;
        let mut ready = BuildingInstance::pending(instance.asset.clone());
        let mut material_cache = BTreeMap::new();
        let mut primitive_count = 0;
        for (chunk, collider, density, mass_properties, edges) in prepared {
            let record = &asset.manifest.chunks[chunk.record_index];
            if settings.detail {
                rim_materials.get_or_create(&record.material, &mut materials);
            }
            let transform = record.transform();
            let triangles: Vec<[Vec3; 3]> = chunk
                .triangles
                .iter()
                .map(|triangle| triangle.map(|p| transform.transform_point(p)))
                .collect();
            let chunk_name = record.id.clone();
            let entity = commands.spawn_scene(bsn! { Name::new(chunk_name) Transform { translation: {transform.translation}, rotation: {transform.rotation} } Visibility::Visible })
                .insert((
                    ChunkIdentity { building: root, id: record.id.clone() },
                    ChildOf(root),
                    collider,
                    ColliderDensity(density),
                    edges,
                ))
                .id();
            for (i, primitive) in chunk.primitives.iter().enumerate() {
                let fracture = primitive.surface_role == crate::asset::SurfaceRole::Fracture;
                let key = (
                    record.material.clone(),
                    primitive.material.as_ref().map(|m| m.id()),
                    fracture,
                );
                let material = material_cache
                    .entry(key)
                    .or_insert_with(|| {
                        let mut material = building_material(
                            primitive
                                .material
                                .as_ref()
                                .and_then(|h| gltf_materials.get(h)),
                            fracture,
                            &record.material,
                        );
                        material.extension.contacts = masks.buffer.clone();
                        materials.add(material)
                    })
                    .clone();
                let mesh = if fracture {
                    surface_meshes.get_or_create(&primitive.mesh, transform, &mut meshes)
                } else {
                    masks.meshes[&(chunk.record_index, primitive.mesh.id())].clone()
                };
                let primitive_name = format!("{} primitive {i}", record.id);
                commands.spawn_scene(bsn! {
                    Name::new(primitive_name) Mesh3d(mesh) MeshMaterial3d::<BuildingMaterial>(material) Transform::IDENTITY
                }).insert((ChildOf(entity), ChunkPrimitive, MeshTag::new(0)));
                primitive_count += 1;
            }
            commands.entity(entity).insert(ChunkInstance {
                alive: true,
                triangles,
                transform,
                mass_properties,
                material: record.material.clone(),
            });
            ready.chunks.insert(record.id.clone(), entity);
        }
        spawn_bonds(&mut commands, root, &mut ready, &asset.manifest.bonds);
        spawn_patches(&mut commands, root, &mut ready, patches);
        ready.bounds = Some(bounds);
        stats.patches = ready.patches.len();
        info!(
            "Instantiated {}: {} chunks, {} primitives, {} bonds",
            asset.manifest.name,
            ready.chunks.len(),
            primitive_count,
            ready.bonds.len()
        );
        // Publish the indexes only after the entities they name have been spawned.
        commands.entity(root).insert((ready, NeedsClusterRebuild));
    }
}
