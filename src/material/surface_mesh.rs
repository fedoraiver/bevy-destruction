use std::collections::BTreeMap;

use bevy::{asset::AssetId, mesh::VertexAttributeValues, prelude::*};

#[derive(Resource, Default)]
pub(crate) struct SurfaceMeshCache {
    pub(super) meshes: BTreeMap<(AssetId<Mesh>, [u32; 16]), Handle<Mesh>>,
}

impl SurfaceMeshCache {
    pub(crate) fn get_or_create(
        &mut self,
        source: &Handle<Mesh>,
        rest_from_mesh: Transform,
        meshes: &mut Assets<Mesh>,
    ) -> Handle<Mesh> {
        let key = (
            source.id(),
            rest_from_mesh.to_matrix().to_cols_array().map(f32::to_bits),
        );
        self.meshes
            .entry(key)
            .or_insert_with(|| {
                let mesh = meshes.get(source).expect("Validated building mesh").clone();
                meshes.add(surface_coordinates(mesh, rest_from_mesh))
            })
            .clone()
    }
}

pub(super) fn release_surface_meshes(
    mut events: MessageReader<AssetEvent<Mesh>>,
    mut cache: ResMut<SurfaceMeshCache>,
) {
    for event in events.read() {
        if let AssetEvent::Removed { id } = event {
            cache.meshes.retain(|(source, _), _| source != id);
        }
    }
}

pub(crate) fn surface_coordinates(mut mesh: Mesh, rest_from_mesh: Transform) -> Mesh {
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        panic!("Building surface requires float3 positions");
    };
    let mut xy = Vec::with_capacity(positions.len());
    let mut z = Vec::with_capacity(positions.len());
    for &position in positions {
        let rest = rest_from_mesh.transform_point(Vec3::from_array(position));
        xy.push([rest.x, rest.y]);
        z.push([rest.z, 0.0]);
    }
    // Both UV channels are immutable metre coordinates, carried by Bevy's stock vertex passes.
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, xy);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, z);
    mesh
}
