mod gltf;
mod loader;
mod manifest;
mod patches;

#[cfg(test)]
mod tests;

use bevy::{gltf::Gltf, prelude::*};
use thiserror::Error;

pub use self::{
    gltf::{SurfaceRole, mesh_triangles, resolve_building},
    manifest::{BondPatchRecord, BondRecord, ChunkRecord, Manifest, PhysicalMaterial},
};

#[cfg(test)]
use self::{gltf::surface_role, loader::validate_mesh_hash, patches::patch_metrics};
#[cfg(test)]
pub use manifest::{AnchorRecord, ColliderRecord, Coordinates};
#[cfg(test)]
use {
    bevy::mesh::PrimitiveTopology,
    sha2::{Digest, Sha256},
    std::collections::BTreeMap,
};

pub struct DestructibleAssetPlugin;

impl Plugin for DestructibleAssetPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<DestructibleAsset>()
            .init_asset_loader::<DestructibleAssetLoader>();
    }
}

#[derive(Asset, TypePath, Debug)]
pub struct DestructibleAsset {
    pub manifest: Manifest,
    #[dependency]
    pub gltf: Handle<Gltf>,
}

#[derive(Default, TypePath)]
pub struct DestructibleAssetLoader;

#[derive(Debug, Error)]
pub enum DestructibleAssetError {
    #[error("读取建筑资产失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("建筑清单 JSON 无效：{0}")]
    Json(#[from] serde_json::Error),
    #[error("建筑资产无效：{0}")]
    Invalid(String),
}
