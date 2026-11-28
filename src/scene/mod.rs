use std::{collections::BTreeMap, sync::Arc};

use avian3d::{
    parry::{shape::SharedShape, transformation::try_convex_hull},
    prelude::*,
};
use bevy::{
    asset::{AssetEvent, AssetId, LoadState, RenderAssetUsages},
    camera::visibility::NoFrustumCulling,
    core_pipeline::prepass::DepthPrepass,
    gltf::*,
    mesh::{MeshTag, PrimitiveTopology},
    prelude::*,
    render::storage::ShaderBuffer,
};

use crate::{
    DemoSettings,
    asset::{
        BondRecord, ChunkRecord, DestructibleAsset, SurfaceRole, mesh_triangles, resolve_building,
    },
    camera::{Crosshair, FreeCamera},
    contact_mask::ContactMaskBuilder,
    debug::fracture_edges,
    fracture_detail::{ContactPatch, SurfaceGeometry, build_patches, detail_mesh},
    material::{
        BuildingMaterial, SurfaceMeshCache, building_material, rim_material, surface_coordinates,
    },
    menu::PendingReload,
    physics::NeedsClusterRebuild,
};

pub struct DemoScenePlugin;

impl Plugin for DemoScenePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GeometryCache>()
            .init_resource::<SurfaceMeshCache>()
            .init_resource::<DetailStats>()
            .init_resource::<DetailMeshCache>()
            .init_resource::<RimMaterials>()
            .add_systems(Startup, (setup, warm_rim_pipeline))
            .add_systems(Update, (instantiate_pending, update_hint).chain())
            .add_systems(Update, release_geometry_cache.before(instantiate_pending))
            .add_systems(
                Update,
                maintain_rim_warmup.after(crate::camera::move_camera),
            )
            .add_systems(
                Update,
                update_details
                    .in_set(UpdateDamageDetails)
                    .after(crate::interaction::apply_damage),
            )
            .add_systems(PostUpdate, detail_lod.after(TransformSystems::Propagate));
    }
}

#[derive(Component)]
#[cfg_attr(feature = "acceptance-test", derive(Reflect))]
#[cfg_attr(feature = "acceptance-test", reflect(Component))]
pub struct DemoCamera;

#[derive(Component)]
struct Hint;

#[derive(Component)]
pub(crate) struct DemoRoot;

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

#[derive(Component)]
pub struct ChunkInstance {
    pub alive: bool,
    pub triangles: Vec<[Vec3; 3]>,
    pub transform: Transform,
    pub mass_properties: MassProperties3d,
    pub material: String,
}

impl ChunkInstance {
    pub fn center_in_building(&self) -> Vec3 {
        self.transform
            .transform_point(self.mass_properties.center_of_mass)
    }
}

#[derive(Component)]
pub struct BuildingInstance {
    pub asset: Handle<DestructibleAsset>,
    pub chunks: BTreeMap<String, Entity>,
    pub bonds: BTreeMap<String, Entity>,
    pub bounds: Option<BuildingBounds>,
    pub error: Option<String>,
    pub patches: Vec<Entity>,
}

impl BuildingInstance {
    pub fn pending(asset: Handle<DestructibleAsset>) -> Self {
        Self {
            asset,
            chunks: BTreeMap::new(),
            bonds: BTreeMap::new(),
            bounds: None,
            error: None,
            patches: Vec::new(),
        }
    }

    pub(crate) fn bond_states(
        &self,
        mut is_active: impl FnMut(Entity) -> bool,
    ) -> BTreeMap<String, bool> {
        self.bonds
            .iter()
            .map(|(id, &entity)| (id.clone(), is_active(entity)))
            .collect()
    }
}

#[derive(Component)]
pub struct BondInstance {
    pub id: String,
    pub endpoints: [Entity; 2],
    pub active: bool,
}

#[derive(Component)]
#[relationship(relationship_target = BuildingBonds)]
pub(crate) struct BondOfBuilding {
    pub building: Entity,
}

#[derive(Component)]
#[relationship_target(relationship = BondOfBuilding, linked_spawn)]
pub(crate) struct BuildingBonds {
    bonds: Vec<Entity>,
}

#[derive(Component)]
#[relationship(relationship_target = BondPatches)]
pub(crate) struct PatchOfBond {
    pub bond: Entity,
}

#[derive(Component)]
#[relationship_target(relationship = PatchOfBond, linked_spawn)]
pub(crate) struct BondPatches {
    patches: Vec<Entity>,
}

#[derive(Component)]
pub struct StructuralPatch {
    pub index: usize,
}

#[derive(Component)]
pub struct PatchInstance {
    pub index: usize,
    geometry: Arc<Vec<ContactPatch>>,
}

impl PatchInstance {
    pub fn patch(&self) -> &ContactPatch {
        &self.geometry[self.index]
    }
}

pub(crate) fn spawn_bonds(
    commands: &mut Commands,
    building: Entity,
    instance: &mut BuildingInstance,
    bonds: &[BondRecord],
) {
    for record in bonds {
        let entity = commands
            .spawn((
                Name::new(format!("Bond {}", record.id)),
                BondOfBuilding { building },
                BondInstance {
                    id: record.id.clone(),
                    endpoints: [instance.chunks[&record.a], instance.chunks[&record.b]],
                    active: true,
                },
            ))
            .id();
        instance.bonds.insert(record.id.clone(), entity);
        for index in 0..record.patches.len() {
            commands.spawn((
                Name::new(format!("Structural patch {}:{index}", record.id)),
                PatchOfBond { bond: entity },
                StructuralPatch { index },
            ));
        }
    }
}

pub(crate) fn spawn_patches(
    commands: &mut Commands,
    building: Entity,
    instance: &mut BuildingInstance,
    patches: Arc<Vec<ContactPatch>>,
) {
    for (index, patch) in patches.iter().enumerate() {
        let Some(&bond) = instance.bonds.get(&patch.bond_id) else {
            panic!("Surface patch {} has no bond in {building:?}", patch.id);
        };
        let entity = commands
            .spawn((
                Name::new(format!("Surface patch {}", patch.id)),
                PatchOfBond { bond },
                PatchInstance {
                    index,
                    geometry: patches.clone(),
                },
            ))
            .id();
        instance.patches.push(entity);
    }
}

#[derive(Component)]
pub struct ChunkIdentity {
    pub building: Entity,
    pub id: String,
}

#[derive(Component)]
pub struct RemovedChunk;

#[derive(Component)]
pub(crate) struct ChunkPrimitive;

pub(crate) fn chunk_primitives<'a>(
    children: Option<&'a Children>,
    mut is_primitive: impl FnMut(Entity) -> bool + 'a,
) -> impl Iterator<Item = Entity> + 'a {
    children
        .into_iter()
        .flatten()
        .copied()
        .filter(move |&entity| is_primitive(entity))
}

#[derive(Resource, Default)]
pub(crate) struct GeometryCache {
    patches: BTreeMap<AssetId<DestructibleAsset>, Arc<Vec<ContactPatch>>>,
    surface_masks: BTreeMap<AssetId<DestructibleAsset>, SurfaceMasks>,
}

struct SurfaceMasks {
    meshes: BTreeMap<(usize, AssetId<Mesh>), Handle<Mesh>>,
    buffer: Handle<ShaderBuffer>,
}

impl GeometryCache {
    fn prepare_surface_masks(
        &mut self,
        asset: AssetId<DestructibleAsset>,
        sources: impl IntoIterator<Item = (usize, Handle<Mesh>, Transform, String)>,
        patches: &[ContactPatch],
        meshes: &mut Assets<Mesh>,
        buffers: &mut Assets<ShaderBuffer>,
    ) -> Result<&SurfaceMasks, String> {
        if let std::collections::btree_map::Entry::Vacant(entry) = self.surface_masks.entry(asset) {
            let mut builder = ContactMaskBuilder::default();
            let mut prepared = BTreeMap::new();
            for (index, source, transform, chunk) in sources {
                let mesh = surface_coordinates(meshes.get(&source).unwrap().clone(), transform);
                let masked = builder.mask_mesh(mesh, patches, &chunk)?;
                prepared.insert((index, source.id()), meshes.add(masked));
            }
            entry.insert(SurfaceMasks {
                meshes: prepared,
                buffer: buffers.add(ShaderBuffer::from(builder.data().to_vec())),
            });
        }
        Ok(&self.surface_masks[&asset])
    }
}

fn release_geometry_cache(
    mut events: MessageReader<AssetEvent<DestructibleAsset>>,
    mut cache: ResMut<GeometryCache>,
) {
    for event in events.read() {
        if let AssetEvent::Removed { id } | AssetEvent::Modified { id } = event {
            cache.patches.remove(id);
            cache.surface_masks.remove(id);
        }
    }
}

#[derive(Resource, Default)]
pub(crate) struct RimMaterials {
    by_material: BTreeMap<String, Handle<BuildingMaterial>>,
}

impl RimMaterials {
    fn get_or_create(
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
struct RimPipelineWarmup {
    frames_remaining: u32,
}

struct CachedDetailMesh {
    mesh: Mesh,
    triangles: usize,
}

#[derive(Resource, Default)]
struct DetailMeshCache {
    by_patch: BTreeMap<(AssetId<DestructibleAsset>, usize, bool), Option<Arc<CachedDetailMesh>>>,
}

#[derive(Component)]
pub struct DamageDetail {
    pub patch_ids: Vec<String>,
    #[cfg(feature = "acceptance-test")]
    pub triangles: usize,
}

#[derive(Component)]
pub(crate) struct RefreshDamageDetails;

#[derive(SystemSet, Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) struct UpdateDamageDetails;

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

fn setup(mut commands: Commands, server: Res<AssetServer>, settings: Res<DemoSettings>) {
    let root = commands
        .spawn_scene(bsn! { Name::new("DemoRoot") Transform::IDENTITY Visibility::Visible })
        .insert(DemoRoot)
        .id();
    let camera = commands.spawn_scene(bsn! {
        Name::new("Free flight camera")
        Camera3d::default()
        Projection::from(PerspectiveProjection { fov: crate::camera::FIELD_OF_VIEW, near: 0.05, far: 500.0, ..default() })
        Transform::from_xyz(0.0, 3.0, 30.0)
        AmbientLight { brightness: 280.0 }
    }).insert((DemoCamera, FreeCamera::default(), Msaa::Off, ChildOf(root))).id();
    if settings.soft_particles {
        commands.entity(camera).insert(DepthPrepass);
    }
    commands.spawn_scene(bsn! {
        Name::new("Crosshair")
        Node { position_type: PositionType::Absolute, width: percent(100.0), height: percent(100.0), justify_content: JustifyContent::Center, align_items: AlignItems::Center }
        Visibility::Visible
        Children [
            Name::new("Crosshair glyph")
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
    let building = match settings.scene_path() {
        Ok(path) => BuildingInstance::pending(server.load(path.to_owned())),
        Err(error) => {
            let mut building = BuildingInstance::pending(Handle::default());
            building.error = Some(error.into());
            building
        }
    };
    commands
        .spawn_scene(bsn! { Name::new("BuildingRoot") Transform::IDENTITY Visibility::Visible })
        .insert((building, ChildOf(root)));
    commands
        .spawn_scene(bsn! {
            Name::new("Building status")
            Text::new("Loading building…")
            TextFont { font_size: px(19.0) }
            TextColor(Color::srgb(0.88, 0.9, 0.94))
            Node { position_type: PositionType::Absolute, left: px(22.0), bottom: px(18.0) }
        })
        .insert(Hint);
}

fn warm_rim_pipeline(
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

fn maintain_rim_warmup(
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

pub(crate) fn chunk_collider(record: &ChunkRecord) -> Result<Collider, String> {
    let shapes = record
        .colliders
        .iter()
        .enumerate()
        .map(|(index, proxy)| {
            let points = proxy
                .vertices
                .iter()
                .copied()
                .map(Vec3::from)
                .collect::<Vec<_>>();
            Collider::convex_hull(&points)
                .or_else(|| conditioned_convex_collider(&points))
                .map(|shape| (Vec3::ZERO, Quat::IDENTITY, shape))
                .ok_or_else(|| {
                    format!(
                        "Invalid convex collider: {} (proxy {})",
                        record.id,
                        index + 1
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Collider::compound(shapes))
}

pub(crate) fn chunk_mass_properties(
    id: &str,
    collider: &Collider,
    density: f32,
) -> Result<MassProperties3d, String> {
    let properties = collider.mass_properties(density);
    let inertia = properties.principal_angular_inertia;
    if !properties.mass.is_finite()
        || properties.mass <= 0.0
        || !properties.center_of_mass.is_finite()
        || !properties.local_inertial_frame.is_finite()
        || !inertia.is_finite()
        || inertia.min_element() <= 0.0
    {
        return Err(format!("Invalid collider mass properties: {id}"));
    }
    Ok(properties)
}

fn conditioned_convex_collider(points: &[Vec3]) -> Option<Collider> {
    let first = *points.first()?;
    let (min, max) = points.iter().fold((first, first), |(min, max), &point| {
        (min.min(point), max.max(point))
    });
    let extent = max - min;
    if !extent.is_finite() || extent.min_element() <= 0.0 {
        return None;
    }

    // Parry can classify a thin solid as a line after uniform normalization.
    // Find the faces in better-scaled coordinates, then build from the original vertices.
    let normalized: Vec<_> = points.iter().map(|&point| (point - min) / extent).collect();
    let (vertices, faces) = try_convex_hull(&normalized).ok()?;
    if vertices.len() < 4 || faces.len() < 4 {
        return None;
    }
    let origin = vertices[0].as_dvec3();
    let signed_volume: f64 = faces
        .iter()
        .map(|face| {
            let [a, b, c] = face.map(|index| vertices[index as usize].as_dvec3() - origin);
            a.dot(b.cross(c))
        })
        .sum();
    if signed_volume <= f64::EPSILON {
        return None;
    }
    let vertices = vertices
        .iter()
        .map(|vertex| {
            normalized
                .iter()
                .position(|point| point == vertex)
                .map(|index| points[index])
        })
        .collect::<Option<Vec<_>>>()?;
    SharedShape::convex_mesh(vertices, &faces).map(Collider::from)
}

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

fn update_hint(
    instances: Query<
        (
            &BuildingInstance,
            Option<&crate::stress_runtime::StressState>,
        ),
        Without<PendingReload>,
    >,
    chunks: Query<&ChunkInstance>,
    mut hints: Query<&mut Text, With<Hint>>,
) {
    let Some((instance, stress)) = instances.iter().next() else {
        return;
    };
    let mut text = if let Some(error) = &instance.error {
        format!("Building error: {error}")
    } else if instance.bounds.is_some() {
        format!(
            "WASD: move | Mouse: look | Space/Ctrl: up/down | Tab: cursor\nLeft: break | Right/R: rigid ball/cube | F1-F6: scenes | 0: edges | 9: heat | Esc: exit     {}/{} chunks",
            instance
                .chunks
                .values()
                .filter(|&&entity| chunks.get(entity).is_ok_and(|chunk| chunk.alive))
                .count(),
            instance.chunks.len()
        )
    } else {
        "Loading building…".into()
    };
    if let Some(stress) = stress {
        let invalid = stress.reports.iter().filter(|report| !report.valid).count();
        let max_displacement = stress
            .reports
            .iter()
            .filter(|report| report.valid)
            .map(|report| report.max_displacement)
            .fold(0.0_f64, f64::max);
        if invalid > 0 && invalid == stress.reports.len() {
            text.push_str(&format!(
                "\nStress: unavailable (retrying) | {} overload breaks | {invalid} unsolved components",
                stress.broken_total
            ));
        } else {
            text.push_str(&format!("\nStress: {:.2} capacity | {} overload breaks | {} unsolved components | {:.3} mm elastic estimate",
                stress.max_utilization, stress.broken_total, invalid, max_displacement * 1000.0));
        }
    }
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
                #[cfg(feature = "acceptance-test")]
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
                        #[cfg(feature = "acceptance-test")]
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
mod tests;
