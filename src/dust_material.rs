use bevy::{
    asset::RenderAssetUsages,
    image::{Image, ImageSampler},
    mesh::MeshVertexBufferLayoutRef,
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    render::render_resource::{
        AsBindGroup, Extent3d, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
        TextureDimension, TextureFormat,
    },
    shader::ShaderRef,
};

pub const DUST_VARIANTS: u32 = 4;
pub const DUST_FRAMES: u32 = 8;
const TILE_SIZE: u32 = 64;

pub struct DustMaterialPlugin;

impl Plugin for DustMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<DustMaterial>::default());
    }
}

#[derive(Clone, Debug, ShaderType)]
pub struct DustSettings {
    pub color: Vec4,
    pub time: Vec4,
    pub shape: Vec4,
    pub light: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct DustMaterial {
    #[uniform(0)]
    pub settings: DustSettings,
    #[texture(1)]
    #[sampler(2)]
    pub atlas: Handle<Image>,
}

impl DustMaterial {
    pub fn update(&mut self, age: f32, lifetime: f32, variant: u32, opacity: f32, roll: f32) {
        self.settings.time = Vec4::new(age, lifetime, (variant % DUST_VARIANTS) as f32, opacity);
        self.settings.shape.x = roll;
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

pub fn dust_material(atlas: Handle<Image>, material_id: &str) -> DustMaterial {
    let color = match material_id {
        "steel" => Color::srgb(0.47, 0.49, 0.51),
        "wood" | "timber" => Color::srgb(0.65, 0.47, 0.27),
        "brick" | "masonry" => Color::srgb(0.66, 0.38, 0.27),
        _ => Color::srgb(0.68, 0.64, 0.55),
    };
    DustMaterial {
        settings: DustSettings {
            color: color.to_linear().to_vec4(),
            time: Vec4::new(0.0, 1.0, 0.0, 0.0),
            shape: Vec4::new(0.0, 0.12, 0.08, 0.3),
            light: Vec4::new(-0.48, 0.67, 0.57, 0.55),
        },
        atlas,
    }
}

fn hash(mut value: u32) -> f32 {
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846c_a68b);
    value ^= value >> 16;
    (value >> 8) as f32 / 16_777_216.0
}

fn density(point: Vec2, frame: f32, variant: u32) -> f32 {
    let mut sum = 0.0;
    for lobe in 0..13_u32 {
        let seed = variant * 97 + lobe * 11 + 1;
        let angle = hash(seed) * std::f32::consts::TAU;
        let radius = (0.09 + hash(seed + 1) * 0.38) * (0.65 + frame * 0.4);
        let wobble = Vec2::new((frame * 3.0 + angle).sin(), (frame * 2.0 - angle).cos())
            * (0.015 + frame * 0.035);
        let center = Vec2::from_angle(angle) * radius + wobble;
        let width = 0.12 + hash(seed + 2) * 0.13 + frame * 0.035;
        let delta = (point - center) / width;
        sum += (-delta.length_squared() * 1.8).exp() * (0.3 + hash(seed + 3) * 0.45);
    }
    let curl = ((point.x * 18.0 + frame * 2.0).sin() * (point.y * 15.0 - frame * 1.5).cos()) * 0.1;
    let edge = (1.0 - point.length_squared()).max(0.0).powi(2);
    ((sum + curl) * edge).clamp(0.0, 1.0)
}

/// Four deterministic evolving dust shapes. RG stores a pseudo-normal, B density,
/// and A opacity; the texture is linear data rather than sRGB colour.
pub fn dust_atlas() -> Image {
    let width = TILE_SIZE * DUST_FRAMES;
    let height = TILE_SIZE * DUST_VARIANTS;
    let mut pixels = vec![0_u8; (width * height * 4) as usize];
    let epsilon = 2.0 / TILE_SIZE as f32;
    for variant in 0..DUST_VARIANTS {
        for frame in 0..DUST_FRAMES {
            let progress = frame as f32 / (DUST_FRAMES - 1) as f32;
            for y in 0..TILE_SIZE {
                for x in 0..TILE_SIZE {
                    let point = Vec2::new(
                        (x as f32 + 0.5) * epsilon - 1.0,
                        (y as f32 + 0.5) * epsilon - 1.0,
                    );
                    let opacity = density(point, progress, variant);
                    let gradient = Vec2::new(
                        density(point + Vec2::X * epsilon, progress, variant)
                            - density(point - Vec2::X * epsilon, progress, variant),
                        density(point + Vec2::Y * epsilon, progress, variant)
                            - density(point - Vec2::Y * epsilon, progress, variant),
                    ) / (2.0 * epsilon);
                    let normal = (-gradient * 0.28).extend(1.0).normalize();
                    let index =
                        (((variant * TILE_SIZE + y) * width + frame * TILE_SIZE + x) * 4) as usize;
                    pixels[index..index + 4].copy_from_slice(&[
                        ((normal.x * 0.5 + 0.5) * 255.0) as u8,
                        ((normal.y * 0.5 + 0.5) * 255.0) as u8,
                        (opacity * 255.0) as u8,
                        (opacity * 255.0) as u8,
                    ]);
                }
            }
        }
    }
    let mut image = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels,
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::default(),
    );
    image.sampler = ImageSampler::linear();
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_has_distinct_animated_shapes_and_transparent_borders() {
        let image = dust_atlas();
        let data = image.data.as_ref().unwrap();
        let width = (TILE_SIZE * DUST_FRAMES) as usize;
        for row in 0..DUST_VARIANTS as usize {
            for frame in 0..DUST_FRAMES as usize {
                for (x, y) in [(0, 0), (63, 0), (0, 63), (63, 63)] {
                    assert_eq!(data[((row * 64 + y) * width + frame * 64 + x) * 4 + 3], 0);
                }
            }
        }
        let sample = |frame: usize, variant: usize| {
            (0..64)
                .flat_map(|y| {
                    (0..64).map(move |x| ((variant * 64 + y) * width + frame * 64 + x) * 4 + 3)
                })
                .map(|index| data[index])
                .collect::<Vec<_>>()
        };
        assert_ne!(sample(0, 0), sample(7, 0));
        assert_ne!(sample(3, 0), sample(3, 1));
    }
}
