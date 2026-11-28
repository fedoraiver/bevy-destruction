use bevy::asset::{AssetLoader, LoadContext, io::Reader};
use sha2::{Digest, Sha256};

use super::{DestructibleAsset, DestructibleAssetError, DestructibleAssetLoader, Manifest};

impl AssetLoader for DestructibleAssetLoader {
    type Asset = DestructibleAsset;
    type Settings = ();
    type Error = DestructibleAssetError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &(),
        load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let manifest: Manifest = serde_json::from_slice(&bytes)?;
        manifest
            .validate()
            .map_err(DestructibleAssetError::Invalid)?;
        let mesh_path = load_context
            .path()
            .resolve_embed_str(&manifest.mesh_file)
            .map_err(|error| DestructibleAssetError::Invalid(error.to_string()))?;
        let mesh_bytes = load_context
            .read_asset_bytes(mesh_path.clone())
            .await
            .map_err(|error| DestructibleAssetError::Invalid(error.to_string()))?;
        validate_mesh_hash(&mesh_bytes, &manifest.mesh_sha256)
            .map_err(DestructibleAssetError::Invalid)?;
        let gltf = load_context.load(mesh_path);
        Ok(DestructibleAsset { manifest, gltf })
    }

    fn extensions(&self) -> &[&str] {
        &["destruction.json"]
    }
}

pub(super) fn validate_mesh_hash(bytes: &[u8], expected: &str) -> Result<(), String> {
    let actual = format!("{:x}", Sha256::digest(bytes));
    if actual != expected.to_ascii_lowercase() {
        return Err("GLB 的 SHA256 与清单不匹配".into());
    }
    Ok(())
}
