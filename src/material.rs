use bevy::{
    gltf::GltfMaterial,
    pbr::{ExtendedMaterial, MaterialExtension, gltf::standard_material_from_gltf_material},
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderType},
    shader::ShaderRef,
};

pub type BuildingMaterial = ExtendedMaterial<StandardMaterial, BuildingSurface>;

pub struct BuildingMaterialPlugin;

impl Plugin for BuildingMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<BuildingMaterial>::default());
    }
}

#[derive(Clone, Debug, ShaderType, Reflect)]
pub struct SurfaceSettings {
    pub building_from_world: Mat4,
    pub grid: Vec4,
    pub fracture: Vec4,
    pub fracture_color: Vec4,
}

impl Default for SurfaceSettings {
    fn default() -> Self {
        Self {
            building_from_world: Mat4::IDENTITY,
            grid: Vec4::new(0.2, 1.0, 0.8, 1.8),
            fracture: Vec4::new(0.0, 0.0022, 30.0, 180.0),
            fracture_color: Vec4::new(0.54, 0.50, 0.43, 1.0),
        }
    }
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
pub struct BuildingSurface {
    #[uniform(100)]
    pub settings: SurfaceSettings,
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
    if fracture {
        settings.fracture.x = 1.0;
        let (color, bump, roughness) = match material_id {
            "steel" => (Vec3::new(0.31, 0.34, 0.37), 0.0007, 0.72),
            "wood" | "timber" => (Vec3::new(0.43, 0.27, 0.12), 0.0016, 0.94),
            "brick" | "masonry" => (Vec3::new(0.52, 0.27, 0.18), 0.0025, 0.98),
            _ => (Vec3::new(0.54, 0.50, 0.43), 0.0022, 0.98),
        };
        settings.fracture.y = bump;
        settings.fracture_color = color.extend(1.0);
        base.perceptual_roughness = roughness;
        base.metallic = if material_id == "steel" { 0.65 } else { 0.0 };
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
        extension: BuildingSurface { settings },
    }
}

pub fn rim_material(material_id: &str) -> BuildingMaterial {
    let mut material = building_material(None, true, material_id);
    material.extension.settings.fracture.x = 2.0;
    material.extension.settings.fracture.y *= 1.25;
    material
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn material_keeps_gltf_double_sided_and_uses_material_class_for_fracture() {
        let source = GltfMaterial {
            double_sided: true,
            ..default()
        };
        let steel = building_material(Some(&source), true, "steel");
        let concrete = building_material(Some(&source), true, "concrete");
        assert!(steel.base.double_sided);
        assert!(steel.base.metallic > concrete.base.metallic);
        assert!(concrete.extension.settings.fracture.y > 0.0);
        assert_ne!(
            steel.extension.settings.fracture_color,
            concrete.extension.settings.fracture_color
        );
    }
}
