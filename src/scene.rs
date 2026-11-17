use std::{collections::BTreeMap, sync::Arc};

use avian3d::prelude::*;
use bevy::{
    asset::{AssetId, LoadState, RenderAssetUsages},
    camera::visibility::NoFrustumCulling,
    core_pipeline::prepass::DepthPrepass,
    gltf::*,
    mesh::PrimitiveTopology,
    prelude::*,
};

use crate::{
    DemoOptions,
    asset::{DestructibleAsset, SurfaceRole, mesh_triangles, resolve_building},
    camera::{Crosshair, FreeCamera},
    debug::fracture_edges,
    fracture_detail::{ContactPatch, SurfaceGeometry, build_patches, detail_mesh},
    material::{BuildingMaterial, building_material, rim_material},
};

pub struct DemoScenePlugin;

impl Plugin for DemoScenePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GeometryCache>()
            .init_resource::<DetailStats>()
            .init_resource::<DetailMeshCache>()
            .init_resource::<RimMaterials>()
            .add_systems(Startup, (setup, warm_rim_pipeline))
            .add_systems(Update, (instantiate_pending, update_hint).chain())
            .add_systems(
                Update,
                maintain_rim_warmup.after(crate::camera::move_camera),
            )
            .add_systems(
                Update,
                update_details.after(crate::interaction::apply_damage),
            )
            .add_systems(PostUpdate, detail_lod.after(TransformSystems::Propagate));
    }
}

#[derive(Component)]
pub struct DemoCamera;

#[derive(Component)]
struct Hint;

#[derive(Clone, Copy, Debug)]
pub struct BuildingBounds {
    pub min: Vec3,
    pub max: Vec3,
}

impl BuildingBounds {
    pub fn center(self) -> Vec3 {
        (self.min + self.max) * 0.5
    }
}

pub struct ChunkInstance {
    pub entity: Entity,
    pub primitives: Vec<Entity>,
    pub alive: bool,
    pub triangles: Vec<[Vec3; 3]>,
    pub transform: Transform,
    pub material: String,
    pub detail: Option<Entity>,
    pub cluster: Option<Entity>,
}

#[derive(Component)]
pub struct BuildingInstance {
    pub asset: Handle<DestructibleAsset>,
    pub chunks: BTreeMap<String, ChunkInstance>,
    pub bonds: BTreeMap<String, bool>,
    pub bounds: Option<BuildingBounds>,
    pub error: Option<String>,
    pub patches: Arc<Vec<ContactPatch>>,
    pub detail_dirty: std::collections::BTreeSet<String>,
    pub clusters: Vec<Entity>,
    pub topology_dirty: bool,
}

impl BuildingInstance {
    pub fn pending(asset: Handle<DestructibleAsset>) -> Self {
        Self {
            asset,
            chunks: BTreeMap::new(),
            bonds: BTreeMap::new(),
            bounds: None,
            error: None,
            patches: Arc::new(Vec::new()),
            detail_dirty: default(),
            clusters: Vec::new(),
            topology_dirty: false,
        }
    }
}

#[derive(Component)]
pub struct ChunkIdentity {
    pub building: Entity,
    pub id: String,
}

#[derive(Component)]
pub struct RemovedChunk;

#[derive(Resource, Default)]
pub(crate) struct GeometryCache(BTreeMap<AssetId<DestructibleAsset>, Arc<Vec<ContactPatch>>>);

#[derive(Resource, Default)]
pub(crate) struct RimMaterials(BTreeMap<String, Handle<BuildingMaterial>>);

impl RimMaterials {
    fn get_or_create(
        &mut self,
        material_id: &str,
        materials: &mut Assets<BuildingMaterial>,
    ) -> Handle<BuildingMaterial> {
        self.0
            .entry(material_id.into())
            .or_insert_with(|| materials.add(rim_material(material_id)))
            .clone()
    }
}

#[derive(Component)]
struct RimPipelineWarmup(u32);

struct CachedDetailMesh {
    mesh: Mesh,
    triangles: usize,
}

#[derive(Resource, Default)]
struct DetailMeshCache(
    BTreeMap<(AssetId<DestructibleAsset>, usize, bool), Option<Arc<CachedDetailMesh>>>,
);

#[derive(Component)]
pub struct DamageDetail {
    pub patch_ids: Vec<String>,
    pub triangles: usize,
}

#[derive(Resource, Default)]
pub struct DetailStats {
    pub patches: usize,
    pub active_meshes: usize,
    pub triangles: usize,
    pub max_update_ms: f64,
    pub cache_build_ms: f64,
    pub cached_meshes: usize,
    pub cached_triangles: usize,
}

fn setup(mut commands: Commands, server: Res<AssetServer>, options: Res<DemoOptions>) {
    let root = commands
        .spawn_scene(bsn! { Name::new("DemoRoot") Transform::IDENTITY Visibility::Visible })
        .id();
    let camera = commands.spawn_scene(bsn! {
        Name::new("Free flight camera")
        Camera3d::default()
        Projection::from(PerspectiveProjection { fov: crate::camera::FIELD_OF_VIEW, near: 0.05, far: 500.0, ..default() })
        Transform::from_xyz(0.0, 3.0, 30.0)
        AmbientLight { brightness: 280.0 }
    }).insert((DemoCamera, FreeCamera::default(), Msaa::Off, ChildOf(root))).id();
    if !options.no_soft_particles {
        commands.entity(camera).insert(DepthPrepass);
    }
    commands.spawn_scene(bsn! {
        Name::new("Crosshair")
        Node { position_type: PositionType::Absolute, width: percent(100.0), height: percent(100.0), justify_content: JustifyContent::Center, align_items: AlignItems::Center }
        Visibility::Visible
        Children [
            Text::new("+")
            TextFont { font_size: px(24.0) }
            TextColor(Color::srgb(0.95, 0.7, 0.3))
        ]
    }).insert(Crosshair);
    commands
        .spawn_scene(bsn! {
            Name::new("Key light")
            DirectionalLight { illuminance: 6500.0, shadow_maps_enabled: true }
            Transform::from_xyz(-10.0, 14.0, 12.0).looking_at(Vec3::ZERO, Vec3::Y)
        })
        .insert(ChildOf(root));
    commands
        .spawn_scene(bsn! { Name::new("BuildingRoot") Transform::IDENTITY Visibility::Visible })
        .insert((
            BuildingInstance::pending(server.load(options.asset.clone())),
            ChildOf(root),
        ));
    commands
        .spawn_scene(bsn! {
            Text::new("Loading building…")
            TextFont { font_size: px(19.0) }
            TextColor(Color::srgb(0.88, 0.9, 0.94))
            Node { position_type: PositionType::Absolute, left: px(22.0), bottom: px(18.0) }
        })
        .insert(Hint);
}

fn warm_rim_pipeline(
    mut commands: Commands,
    options: Res<DemoOptions>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<BuildingMaterial>>,
    mut rim_materials: ResMut<RimMaterials>,
) {
    if options.no_detail {
        return;
    }
    // Match the real non-indexed POSITION/NORMAL/COLOR layout, including its prepass.
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
    commands.spawn((
        Name::new("Fracture rim pipeline warmup"),
        RimPipelineWarmup(90),
        NoFrustumCulling,
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(rim_materials.get_or_create("concrete", &mut materials)),
        Transform::from_xyz(0.0, 0.0, -1.0).with_scale(Vec3::splat(0.00001)),
        Visibility::Visible,
    ));
}

fn maintain_rim_warmup(
    mut commands: Commands,
    cameras: Query<&Transform, With<DemoCamera>>,
    mut warmups: Query<(Entity, &mut RimPipelineWarmup, &mut Transform), Without<DemoCamera>>,
) {
    for (entity, mut warmup, mut transform) in &mut warmups {
        warmup.0 -= 1;
        if warmup.0 == 0 {
            commands.entity(entity).despawn();
        } else if let Ok(camera) = cameras.single() {
            transform.translation =
                camera.translation + camera.rotation * Vec3::new(0.35, -0.2, -1.0);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn instantiate_pending(
    mut commands: Commands,
    options: Res<DemoOptions>,
    server: Res<AssetServer>,
    assets: Res<Assets<DestructibleAsset>>,
    gltfs: Res<Assets<Gltf>>,
    nodes: Res<Assets<GltfNode>>,
    gltf_meshes: Res<Assets<GltfMesh>>,
    meshes: Res<Assets<Mesh>>,
    gltf_materials: Res<Assets<GltfMaterial>>,
    mut materials: ResMut<Assets<BuildingMaterial>>,
    mut instances: Query<(Entity, &mut BuildingInstance)>,
    mut cache: ResMut<GeometryCache>,
    mut rim_materials: ResMut<RimMaterials>,
    mut stats: ResMut<DetailStats>,
) {
    for (root, mut instance) in &mut instances {
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
                if !cache.0.contains_key(&instance.asset.id()) {
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
                let shapes = record
                    .colliders
                    .iter()
                    .map(|proxy| {
                        let points = proxy
                            .vertices
                            .iter()
                            .copied()
                            .map(Vec3::from)
                            .collect::<Vec<_>>();
                        Collider::convex_hull(&points)
                            .map(|shape| (Vec3::ZERO, Quat::IDENTITY, shape))
                            .ok_or_else(|| format!("Invalid convex collider: {}", record.id))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let edges = fracture_edges(
                    chunk
                        .primitives
                        .iter()
                        .map(|primitive| meshes.get(&primitive.mesh).unwrap()),
                )?;
                chunks.push((chunk, Collider::compound(shapes), edges));
            }
            let patches = if let Some(patches) = cache.0.get(&instance.asset.id()) {
                patches.clone()
            } else {
                let start = bevy::platform::time::Instant::now();
                let patches = Arc::new(build_patches(&asset.manifest, &geometry)?);
                stats.cache_build_ms += start.elapsed().as_secs_f64() * 1000.0;
                cache.0.insert(instance.asset.id(), patches.clone());
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
        let mut material_cache = BTreeMap::new();
        for (chunk, collider, edges) in prepared {
            let record = &asset.manifest.chunks[chunk.record_index];
            if !options.no_detail {
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
                .insert((ChunkIdentity { building: root, id: record.id.clone() }, ChildOf(root), collider,
                    ColliderDensity(0.0), Mass(record.mass), AngularInertia::from_mat3_unchecked(Mat3::from_cols_array_2d(&record.inertia)),
                    CenterOfMass(Vec3::from(record.center_of_mass)), NoAutoMass, NoAutoAngularInertia, NoAutoCenterOfMass, edges)).id();
            let mut primitives = Vec::new();
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
                        materials.add(building_material(
                            primitive
                                .material
                                .as_ref()
                                .and_then(|h| gltf_materials.get(h)),
                            fracture,
                            &record.material,
                        ))
                    })
                    .clone();
                let mesh = primitive.mesh.clone();
                let primitive_name = format!("{} primitive {i}", record.id);
                let primitive_entity = commands.spawn_scene(bsn! {
                    Name::new(primitive_name) Mesh3d(mesh) MeshMaterial3d::<BuildingMaterial>(material) Transform::IDENTITY
                }).insert(ChildOf(entity)).id();
                primitives.push(primitive_entity);
            }
            instance.chunks.insert(
                record.id.clone(),
                ChunkInstance {
                    entity,
                    primitives,
                    alive: true,
                    triangles,
                    transform,
                    material: record.material.clone(),
                    detail: None,
                    cluster: None,
                },
            );
        }
        instance.bonds = asset
            .manifest
            .bonds
            .iter()
            .map(|bond| (bond.id.clone(), true))
            .collect();
        instance.bounds = Some(bounds);
        instance.topology_dirty = true;
        instance.patches = patches;
        stats.patches = instance.patches.len();
        info!(
            "Instantiated {}: {} chunks, {} primitives, {} bonds",
            asset.manifest.name,
            instance.chunks.len(),
            instance
                .chunks
                .values()
                .map(|c| c.primitives.len())
                .sum::<usize>(),
            instance.bonds.len()
        );
    }
}

fn update_hint(instances: Query<&BuildingInstance>, mut hints: Query<&mut Text, With<Hint>>) {
    let Some(instance) = instances.iter().next() else {
        return;
    };
    let text = if let Some(error) = &instance.error {
        format!("Building error: {error}")
    } else if instance.bounds.is_some() {
        format!(
            "WASD: move | Mouse: look | Space/Ctrl: up/down | Tab: cursor\nLeft click: break | 0: edges | Esc: exit     {}/{} chunks",
            instance.chunks.values().filter(|chunk| chunk.alive).count(),
            instance.chunks.len()
        )
    } else {
        "Loading building…".into()
    };
    for mut hint in &mut hints {
        if hint.0 != text {
            hint.0 = text.clone();
        }
    }
}

pub const DETAIL_TRIANGLE_LIMIT: usize = 180_000;

#[allow(clippy::too_many_arguments)]
fn update_details(
    mut commands: Commands,
    options: Res<DemoOptions>,
    mut instances: Query<(Entity, &mut BuildingInstance)>,
    details: Query<&DamageDetail>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<BuildingMaterial>>,
    mut rim_materials: ResMut<RimMaterials>,
    mut detail_cache: ResMut<DetailMeshCache>,
    mut stats: ResMut<DetailStats>,
) {
    if options.no_detail
        || !instances
            .iter()
            .any(|(_, instance)| !instance.detail_dirty.is_empty())
    {
        return;
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
        let (_, mut instance) = instances.get_mut(instance_id).unwrap();
        instance.detail_dirty.clear();
        let mut exposed = BTreeMap::<String, Vec<usize>>::new();
        for (patch_index, patch) in instance.patches.iter().enumerate() {
            if let Some(survivor) = patch.exposed_survivor(|id| instance.chunks[id].alive) {
                exposed
                    .entry(survivor.to_owned())
                    .or_default()
                    .push(patch_index);
            } else if instance.chunks[&patch.a].alive
                && instance.chunks[&patch.b].alive
                && !instance.bonds.get(&patch.bond_id).copied().unwrap_or(true)
            {
                for id in [&patch.a, &patch.b] {
                    exposed.entry(id.clone()).or_default().push(patch_index);
                }
            }
        }
        let chunk_ids = instance.chunks.keys().cloned().collect::<Vec<_>>();
        for id in chunk_ids {
            let chunk = &instance.chunks[&id];
            let mut selected = Vec::new();
            let mut patch_ids = Vec::new();
            for &patch_index in exposed.get(&id).into_iter().flatten() {
                let patch = &instance.patches[patch_index];
                let cached = detail_cache
                    .0
                    .entry((instance.asset.id(), patch_index, patch.a == id))
                    .or_insert_with(|| {
                        detail_mesh(patch, &id).map(|mesh| {
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
            if let Some(entity) = chunk.detail {
                if let Ok(old) = details.get(entity)
                    && old.patch_ids == patch_ids
                {
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
            let entity = if let Some(mesh) = combined {
                let triangles = mesh
                    .indices()
                    .map_or(mesh.count_vertices(), |indices| indices.len())
                    / 3;
                let material = rim_materials.get_or_create(&chunk.material, &mut materials);
                let local = Transform::from_matrix(chunk.transform.to_matrix().inverse());
                mesh_count += 1;
                Some(
                    commands
                        .spawn((
                            Name::new("Exposed jagged fracture rim"),
                            ChildOf(chunk.entity),
                            Mesh3d(meshes.add(mesh.transformed_by(local))),
                            MeshMaterial3d(material),
                            Transform::IDENTITY,
                            DamageDetail {
                                patch_ids,
                                triangles,
                            },
                        ))
                        .id(),
                )
            } else {
                None
            };
            instance.chunks.get_mut(&id).unwrap().detail = entity;
        }
    }
    stats.active_meshes = mesh_count;
    stats.triangles = triangle_count;
    stats.cached_meshes = detail_cache
        .0
        .values()
        .filter(|mesh| mesh.is_some())
        .count();
    stats.cached_triangles = detail_cache
        .0
        .values()
        .flatten()
        .map(|mesh| mesh.triangles)
        .sum();
    stats.max_update_ms = stats
        .max_update_ms
        .max(start.elapsed().as_secs_f64() * 1000.0);
}

fn detail_lod(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fracture_detail::RimEdge;
    use bevy::mesh::VertexAttributeValues;

    fn detail_app() -> App {
        let mut app = App::new();
        app.init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<BuildingMaterial>>()
            .init_resource::<RimMaterials>()
            .init_resource::<DetailMeshCache>()
            .init_resource::<DetailStats>()
            .insert_resource(DemoOptions::default())
            .add_systems(Update, update_details);
        app
    }

    fn shared_patches() -> Arc<Vec<ContactPatch>> {
        Arc::new(
            [("ab", "b", 0.0), ("ac", "c", -1.0)]
                .map(|(id, neighbor, x)| {
                    let start = Vec3::new(x, 0.0, 0.0);
                    ContactPatch {
                        id: id.into(),
                        bond_id: id.into(),
                        a: "a".into(),
                        b: neighbor.into(),
                        normal: Vec3::X,
                        triangles: vec![[start, start + Vec3::NEG_Z, start + Vec3::Y]],
                        area: 0.5,
                        rims: vec![RimEdge {
                            points: [start, start + Vec3::Y],
                            exterior_a: Some(Vec3::Z),
                            exterior_b: Some(Vec3::Z),
                            thickness_a: 0.2,
                            thickness_b: 0.2,
                        }],
                    }
                })
                .into(),
        )
    }

    fn spawn_instance(world: &mut World, patches: Arc<Vec<ContactPatch>>) -> Entity {
        let root = world.spawn((Transform::IDENTITY, Visibility::Visible)).id();
        let mut instance = BuildingInstance::pending(Handle::default());
        instance.patches = patches;
        for id in ["a", "b", "c"] {
            let entity = world
                .spawn((Transform::IDENTITY, Visibility::Visible, ChildOf(root)))
                .id();
            instance.chunks.insert(
                id.into(),
                ChunkInstance {
                    entity,
                    primitives: Vec::new(),
                    alive: true,
                    triangles: Vec::new(),
                    transform: Transform::IDENTITY,
                    material: "concrete".into(),
                    detail: None,
                    cluster: None,
                },
            );
        }
        instance.detail_dirty.insert("a".into());
        world.entity_mut(root).insert(instance);
        root
    }

    fn remove_chunk(world: &mut World, root: Entity, id: &str) {
        let mut instance = world.get_mut::<BuildingInstance>(root).unwrap();
        instance.chunks.get_mut(id).unwrap().alive = false;
        instance.detail_dirty.insert(id.into());
    }

    fn detail_snapshot(world: &World, root: Entity) -> Vec<(String, Vec<String>, Vec<[f32; 3]>)> {
        let instance = world.get::<BuildingInstance>(root).unwrap();
        instance
            .chunks
            .iter()
            .filter_map(|(id, chunk)| {
                let entity = chunk.detail?;
                assert_eq!(world.get::<ChildOf>(entity).unwrap().parent(), chunk.entity);
                let detail = world.get::<DamageDetail>(entity).unwrap();
                let mesh = world
                    .resource::<Assets<Mesh>>()
                    .get(&world.get::<Mesh3d>(entity).unwrap().0)
                    .unwrap();
                let Some(VertexAttributeValues::Float32x3(positions)) =
                    mesh.attribute(Mesh::ATTRIBUTE_POSITION)
                else {
                    panic!("Rim mesh has no positions");
                };
                Some((id.clone(), detail.patch_ids.clone(), positions.clone()))
            })
            .collect()
    }

    #[test]
    fn shared_contact_cache_keeps_damage_local_and_reuses_unchanged_mesh() {
        let mut app = detail_app();
        let patches = shared_patches();
        let intact = spawn_instance(app.world_mut(), patches.clone());
        let damaged = spawn_instance(app.world_mut(), patches);
        app.update();
        assert!(detail_snapshot(app.world(), intact).is_empty());
        assert!(detail_snapshot(app.world(), damaged).is_empty());

        remove_chunk(app.world_mut(), damaged, "b");
        app.update();
        assert!(detail_snapshot(app.world(), intact).is_empty());
        let snapshot = detail_snapshot(app.world(), damaged);
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].0, "a");
        assert_eq!(snapshot[0].1, ["ab"]);
        assert!(!snapshot[0].2.is_empty());
        assert!(snapshot[0].2.iter().flatten().all(|p| p.is_finite()));
        let detail = app.world().get::<BuildingInstance>(damaged).unwrap().chunks["a"]
            .detail
            .unwrap();
        let mesh = app.world().get::<Mesh3d>(detail).unwrap().0.clone();
        let mesh_count = app.world().resource::<Assets<Mesh>>().len();
        app.world_mut()
            .get_mut::<BuildingInstance>(damaged)
            .unwrap()
            .detail_dirty
            .insert("b".into());
        app.update();
        assert_eq!(app.world().get::<Mesh3d>(detail).unwrap().0, mesh);
        assert_eq!(app.world().resource::<Assets<Mesh>>().len(), mesh_count);
        let stats = app.world().resource::<DetailStats>();
        assert_eq!(stats.active_meshes, 1);
        assert_eq!(stats.cached_meshes, 1);
        assert_eq!(stats.triangles, stats.cached_triangles);
    }

    #[test]
    fn severed_bond_exposes_both_live_sides_only_in_the_damaged_instance() {
        let mut app = detail_app();
        let patches = shared_patches();
        let intact = spawn_instance(app.world_mut(), patches.clone());
        let detached = spawn_instance(app.world_mut(), patches);
        app.world_mut()
            .get_mut::<BuildingInstance>(detached)
            .unwrap()
            .bonds
            .insert("ab".into(), false);
        app.update();
        assert!(detail_snapshot(app.world(), intact).is_empty());
        let snapshot = detail_snapshot(app.world(), detached);
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].0, "a");
        assert_eq!(snapshot[1].0, "b");
        for (_, patch_ids, positions) in snapshot {
            assert_eq!(patch_ids, ["ab"]);
            assert!(!positions.is_empty());
            assert!(positions.iter().flatten().all(|value| value.is_finite()));
        }
    }

    #[test]
    fn removal_order_preserves_geometry_and_removing_survivor_cleans_detail() {
        let mut app = detail_app();
        let patches = shared_patches();
        let first = spawn_instance(app.world_mut(), patches.clone());
        let second = spawn_instance(app.world_mut(), patches);
        for (root, id) in [(first, "b"), (first, "c"), (second, "c"), (second, "b")] {
            remove_chunk(app.world_mut(), root, id);
            app.update();
        }
        let expected = detail_snapshot(app.world(), second);
        assert_eq!(expected.len(), 1);
        assert_eq!(expected[0].1, ["ab", "ac"]);
        assert_eq!(detail_snapshot(app.world(), first), expected);
        assert_eq!(app.world().resource::<DetailStats>().cached_meshes, 2);

        let old_detail = app.world().get::<BuildingInstance>(first).unwrap().chunks["a"]
            .detail
            .unwrap();
        remove_chunk(app.world_mut(), first, "a");
        app.update();
        assert!(detail_snapshot(app.world(), first).is_empty());
        assert!(app.world().get_entity(old_detail).is_err());
        assert_eq!(detail_snapshot(app.world(), second), expected);
        assert_eq!(app.world().resource::<DetailStats>().active_meshes, 1);
    }
}
