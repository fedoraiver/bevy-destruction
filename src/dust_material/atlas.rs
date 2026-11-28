use bevy::{
    asset::RenderAssetUsages,
    image::{Image, ImageSampler},
    prelude::Vec2,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};

use super::{DUST_FRAMES, DUST_VARIANTS};

pub(super) const TILE_SIZE: u32 = 64;

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
