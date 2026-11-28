mod surface_mesh;

use bevy::{
    gltf::GltfMaterial,
    mesh::MeshVertexBufferLayoutRef,
    pbr::{
        ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline,
        gltf::standard_material_from_gltf_material,
    },
    prelude::*,
    render::{
        render_resource::{
            AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
        },
        storage::ShaderBuffer,
    },
    shader::{ShaderDefVal, ShaderRef},
};

#[cfg(test)]
use bevy::mesh::VertexAttributeValues;

use surface_mesh::release_surface_meshes;
pub(crate) use surface_mesh::{SurfaceMeshCache, surface_coordinates};

pub type BuildingMaterial = ExtendedMaterial<StandardMaterial, BuildingSurface>;

/// Per-primitive MeshTag values; zero or an absent tag leaves the surface unchanged.
pub const HEAT_AMBER_TAG: u32 = 1;
pub const HEAT_RED_TAG: u32 = 2;

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

impl MaterialExtension for BuildingSurface {
    fn fragment_shader() -> ShaderRef {
        "shaders/building_surface.wesl".into()
    }

    fn deferred_fragment_shader() -> ShaderRef {
        "shaders/building_surface.wesl".into()
    }

    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(fragment) = &mut descriptor.fragment {
            fragment.shader_defs.extend([
                ShaderDefVal::UInt("HEAT_AMBER_TAG".into(), HEAT_AMBER_TAG),
                ShaderDefVal::UInt("HEAT_RED_TAG".into(), HEAT_RED_TAG),
            ]);
        }
        Ok(())
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
