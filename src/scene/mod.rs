//! Scene plugin registration and shared ECS types.

mod colliders;
mod details;
mod entities;
mod geometry_cache;
mod instance;
mod setup;

pub(crate) use colliders::{chunk_collider, chunk_mass_properties};
#[cfg(test)]
pub(crate) use entities::BondOfBuilding;
pub(crate) use entities::{
    BondPatches, ChunkPrimitive, PatchOfBond, chunk_primitives, spawn_bonds, spawn_patches,
};
pub use entities::{BuildingBounds, BuildingInstance, PatchInstance, StructuralPatch};
pub(crate) use instance::instantiate_pending;

use avian3d::prelude::MassProperties3d;
use bevy::prelude::*;

use crate::material::SurfaceMeshCache;
use details::{
    DetailMeshCache, RimMaterials, detail_lod, maintain_rim_warmup, update_details,
    warm_rim_pipeline,
};
use geometry_cache::{GeometryCache, release_geometry_cache};
use setup::{setup, update_hint};

#[cfg(test)]
use {
    crate::{
        asset::{BondRecord, DestructibleAsset, mesh_triangles},
        fracture_detail::ContactPatch,
        material::BuildingMaterial,
        settings::DemoSettings,
    },
    avian3d::prelude::Collider,
    bevy::{asset::AssetEvent, render::storage::ShaderBuffer},
    entities::BuildingBonds,
    geometry_cache::SurfaceMasks,
    std::{collections::BTreeMap, sync::Arc},
};

#[cfg(test)]
mod tests;

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
#[cfg_attr(feature = "brp", derive(Reflect))]
#[cfg_attr(feature = "brp", reflect(Component))]
pub struct DemoCamera;

#[derive(Component)]
pub(crate) struct DemoRoot;

#[derive(Component)]
#[cfg_attr(feature = "brp", derive(Reflect), reflect(Component))]
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
#[cfg_attr(feature = "brp", derive(Reflect), reflect(Component))]
pub struct BondInstance {
    pub id: String,
    pub endpoints: [Entity; 2],
    pub active: bool,
}

#[derive(Component)]
#[cfg_attr(feature = "brp", derive(Reflect), reflect(Component))]
pub struct ChunkIdentity {
    pub building: Entity,
    pub id: String,
}

#[derive(Component)]
#[cfg_attr(feature = "brp", derive(Reflect), reflect(Component))]
pub struct RemovedChunk;

#[derive(Component)]
#[cfg_attr(feature = "brp", derive(Reflect), reflect(Component))]
pub struct DamageDetail {
    pub patch_ids: Vec<String>,
    #[cfg(feature = "brp")]
    pub triangles: usize,
}

#[derive(Component)]
pub(crate) struct RefreshDamageDetails;

#[derive(SystemSet, Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) struct UpdateDamageDetails;

#[derive(Resource, Default)]
#[cfg_attr(feature = "brp", derive(Reflect), reflect(Resource))]
pub struct DetailStats {
    pub patches: usize,
    pub active_meshes: usize,
    pub triangles: usize,
    pub max_update_ms: f64,
    pub cache_build_ms: f64,
    pub cached_meshes: usize,
    pub cached_triangles: usize,
}
