use std::collections::BTreeMap;

use avian3d::prelude::*;
use bevy::{asset::LoadState, gltf::*, prelude::*};

use crate::{
    DemoOptions,
    asset::{DestructibleAsset, resolve_building},
    camera::{Crosshair, FreeCamera},
    debug::fracture_edges,
    material::{BuildingMaterial, building_material},
};

pub struct DemoScenePlugin;

impl Plugin for DemoScenePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup)
            .add_systems(Update, (instantiate_pending, update_hint).chain());
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
}

#[derive(Component)]
pub struct BuildingInstance {
    pub asset: Handle<DestructibleAsset>,
    pub chunks: BTreeMap<String, ChunkInstance>,
    pub bonds: BTreeMap<String, bool>,
    pub bounds: Option<BuildingBounds>,
    pub error: Option<String>,
}

impl BuildingInstance {
    pub fn pending(asset: Handle<DestructibleAsset>) -> Self {
        Self {
            asset,
            chunks: BTreeMap::new(),
            bonds: BTreeMap::new(),
            bounds: None,
            error: None,
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

fn setup(mut commands: Commands, server: Res<AssetServer>, options: Res<DemoOptions>) {
    let root = commands
        .spawn_scene(bsn! { Name::new("DemoRoot") Transform::IDENTITY Visibility::Visible })
        .id();
    commands.spawn_scene(bsn! {
        Name::new("Free flight camera")
        Camera3d::default()
        Projection::from(PerspectiveProjection { fov: crate::camera::FIELD_OF_VIEW, near: 0.05, far: 500.0, ..default() })
        Transform::from_xyz(0.0, 3.0, 30.0)
        AmbientLight { brightness: 280.0 }
    }).insert((DemoCamera, FreeCamera::default(), ChildOf(root)));
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn instantiate_pending(
    mut commands: Commands,
    server: Res<AssetServer>,
    assets: Res<Assets<DestructibleAsset>>,
    gltfs: Res<Assets<Gltf>>,
    nodes: Res<Assets<GltfNode>>,
    gltf_meshes: Res<Assets<GltfMesh>>,
    meshes: Res<Assets<Mesh>>,
    gltf_materials: Res<Assets<GltfMaterial>>,
    mut materials: ResMut<Assets<BuildingMaterial>>,
    mut instances: Query<(Entity, &mut BuildingInstance)>,
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
            for chunk in building.chunks {
                let record = &asset.manifest.chunks[chunk.record_index];
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
            Ok((
                chunks,
                BuildingBounds {
                    min: building.bounds_min,
                    max: building.bounds_max,
                },
            ))
        })();
        let (prepared, bounds) = match prepared {
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
            let transform = record.transform();
            let triangles: Vec<[Vec3; 3]> = chunk
                .triangles
                .iter()
                .map(|triangle| triangle.map(|p| transform.transform_point(p)))
                .collect();
            let chunk_name = record.id.clone();
            let entity = commands.spawn_scene(bsn! { Name::new(chunk_name) Transform { translation: {transform.translation}, rotation: {transform.rotation} } Visibility::Visible })
                .insert((ChunkIdentity { building: root, id: record.id.clone() }, ChildOf(root), RigidBody::Static, collider,
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
