mod atlas;

#[cfg(test)]
use atlas::TILE_SIZE;
pub use atlas::dust_atlas;

use bevy::{
    mesh::MeshVertexBufferLayoutRef,
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    render::render_resource::{
        AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
    },
    render::storage::ShaderBuffer,
    shader::ShaderRef,
};

pub const DUST_VARIANTS: u32 = 4;
pub const DUST_FRAMES: u32 = 8;

pub struct DustMaterialPlugin;

impl Plugin for DustMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<DustMaterial>::default());
    }
}

#[derive(Component, Clone, Debug, ShaderType)]
pub struct DustSettings {
    pub color: Vec4,
    pub time: Vec4,
    pub shape: Vec4,
    pub light: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct DustMaterial {
    #[storage(0, read_only)]
    pub instances: Handle<ShaderBuffer>,
    #[texture(1)]
    #[sampler(2)]
    pub atlas: Handle<Image>,
}

impl DustSettings {
    pub fn update(&mut self, age: f32, lifetime: f32, variant: u32, opacity: f32, roll: f32) {
        self.time = Vec4::new(age, lifetime, (variant % DUST_VARIANTS) as f32, opacity);
        self.shape.x = roll;
    }

    pub fn packed(&self) -> [[f32; 4]; 4] {
        [
            self.color.to_array(),
            self.time.to_array(),
            self.shape.to_array(),
            self.light.to_array(),
        ]
    }
}

impl Material for DustMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/dust.wesl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

pub fn dust_material(atlas: Handle<Image>, instances: Handle<ShaderBuffer>) -> DustMaterial {
    DustMaterial { instances, atlas }
}

pub fn dust_settings(material_id: &str) -> DustSettings {
    let color = match material_id {
        "steel" => Color::srgb(0.47, 0.49, 0.51),
        "wood" | "timber" => Color::srgb(0.65, 0.47, 0.27),
        "brick" | "masonry" => Color::srgb(0.66, 0.38, 0.27),
        _ => Color::srgb(0.68, 0.64, 0.55),
    };
    DustSettings {
        color: color.to_linear().to_vec4(),
        time: Vec4::new(0.0, 1.0, 0.0, 0.0),
        shape: Vec4::new(0.0, 0.12, 0.08, 0.3),
        light: Vec4::new(-0.48, 0.67, 0.57, 0.55),
    }
}

#[cfg(test)]
mod tests;
