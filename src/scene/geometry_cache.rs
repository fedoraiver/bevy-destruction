use std::{collections::BTreeMap, sync::Arc};

use bevy::{asset::AssetEvent, prelude::*, render::storage::ShaderBuffer};

use crate::{
    asset::DestructibleAsset, contact_mask::ContactMaskBuilder, fracture_detail::ContactPatch,
    material::surface_coordinates,
};

#[derive(Resource, Default)]
pub(crate) struct GeometryCache {
    pub(super) patches: BTreeMap<AssetId<DestructibleAsset>, Arc<Vec<ContactPatch>>>,
    pub(super) surface_masks: BTreeMap<AssetId<DestructibleAsset>, SurfaceMasks>,
}

pub(super) struct SurfaceMasks {
    pub(super) meshes: BTreeMap<(usize, AssetId<Mesh>), Handle<Mesh>>,
    pub(super) buffer: Handle<ShaderBuffer>,
}

impl GeometryCache {
    pub(super) fn prepare_surface_masks(
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

pub(super) fn release_geometry_cache(
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
