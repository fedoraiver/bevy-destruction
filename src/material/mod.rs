use std::collections::BTreeMap;

use bevy::{
    asset::AssetId,
    gltf::GltfMaterial,
    mesh::VertexAttributeValues,
    pbr::{ExtendedMaterial, MaterialExtension, gltf::standard_material_from_gltf_material},
    prelude::*,
    render::{
        render_resource::{AsBindGroup, ShaderType},
        storage::ShaderBuffer,
    },
    shader::ShaderRef,
};

pub type BuildingMaterial = ExtendedMaterial<StandardMaterial, BuildingSurface>;

const EMPTY_CONTACT_MASK: Handle<ShaderBuffer> =
    bevy::asset::uuid_handle!("d5c5e270-46af-40d0-9904-9220b6a4603b");

pub struct BuildingMaterialPlugin;

impl Plugin for BuildingMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Assets<ShaderBuffer>>()
            .world_mut()
            .resource_mut::<Assets<ShaderBuffer>>()
            .insert(
                EMPTY_CONTACT_MASK.id(),
                ShaderBuffer::from(vec![Vec4::ZERO]),
            )
            .expect("Reserved contact-mask buffer ID");
        app.init_resource::<SurfaceMeshCache>()
            .add_plugins(MaterialPlugin::<BuildingMaterial>::default())
            .add_systems(Update, release_surface_meshes);
    }
}

#[derive(Clone, Debug, ShaderType, Reflect)]
pub struct SurfaceSettings {
    pub grid: Vec4,
    pub fracture: Vec4,
    pub fracture_color: Vec4,
    pub fracture_surface: Vec4,
}

impl Default for SurfaceSettings {
    fn default() -> Self {
        Self {
            grid: Vec4::new(0.2, 1.0, 0.8, 1.8),
            fracture: Vec4::new(0.0, 0.0022, 30.0, 180.0),
            fracture_color: Vec4::new(0.54, 0.50, 0.43, 1.0),
            fracture_surface: Vec4::new(0.98, 0.0, 0.0, 0.0),
        }
    }
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct BuildingSurface {
    #[uniform(100)]
    pub settings: SurfaceSettings,
    #[storage(101, read_only)]
    pub contacts: Handle<ShaderBuffer>,
}

impl Default for BuildingSurface {
    fn default() -> Self {
        Self {
            settings: SurfaceSettings::default(),
            contacts: EMPTY_CONTACT_MASK,
        }
    }
}

#[derive(Resource, Default)]
pub(crate) struct SurfaceMeshCache(BTreeMap<(AssetId<Mesh>, [u32; 16]), Handle<Mesh>>);

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
        self.0
            .entry(key)
            .or_insert_with(|| {
                let mesh = meshes.get(source).expect("Validated building mesh").clone();
                meshes.add(surface_coordinates(mesh, rest_from_mesh))
            })
            .clone()
    }
}

fn release_surface_meshes(
    mut events: MessageReader<AssetEvent<Mesh>>,
    mut cache: ResMut<SurfaceMeshCache>,
) {
    for event in events.read() {
        if let AssetEvent::Removed { id } = event {
            cache.0.retain(|(source, _), _| source != id);
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

impl MaterialExtension for BuildingSurface {
    fn fragment_shader() -> ShaderRef {
        "shaders/building_surface.wesl".into()
    }

    fn deferred_fragment_shader() -> ShaderRef {
        "shaders/building_surface.wesl".into()
    }
}

pub fn building_material(
    source: Option<&GltfMaterial>,
    fracture: bool,
    material_id: &str,
) -> BuildingMaterial {
    let mut base = source
        .map(standard_material_from_gltf_material)
        .unwrap_or_default();
    let mut settings = SurfaceSettings::default();
    let (color, bump, roughness) = match material_id {
        "steel" => (Vec3::new(0.31, 0.34, 0.37), 0.0007, 0.72),
        "wood" | "timber" => (Vec3::new(0.43, 0.27, 0.12), 0.0016, 0.94),
        "brick" | "masonry" => (Vec3::new(0.52, 0.27, 0.18), 0.0025, 0.98),
        _ => (Vec3::new(0.54, 0.50, 0.43), 0.0022, 0.98),
    };
    let metallic = if material_id == "steel" { 0.65 } else { 0.0 };
    settings.fracture.y = bump;
    settings.fracture_color = color.extend(1.0);
    settings.fracture_surface = Vec4::new(roughness, metallic, 0.0, 0.0);
    if fracture {
        settings.fracture.x = 1.0;
        base.perceptual_roughness = roughness;
        base.metallic = metallic;
    } else {
        base.base_color = Color::srgb(0.78, 0.80, 0.81);
        base.perceptual_roughness = 0.88;
        base.metallic = 0.0;
    }
    // Imported assets have no UVs; all detail is generated in building metres.
    base.base_color_texture = None;
    base.normal_map_texture = None;
    base.metallic_roughness_texture = None;
    BuildingMaterial {
        base,
        extension: BuildingSurface {
            settings,
            ..default()
        },
    }
}

pub fn rim_material(material_id: &str) -> BuildingMaterial {
    let mut material = building_material(None, true, material_id);
    material.extension.settings.fracture.x = 2.0;
    material.extension.settings.fracture.y *= 1.25;
    material
}

#[cfg(test)]
mod tests;
