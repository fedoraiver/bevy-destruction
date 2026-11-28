use std::path::{Component, Path};

use bevy::{
    prelude::*,
    settings::{ReflectSettingsGroup, SettingsGroup},
};

pub const SETTINGS_APP_NAME: &str = "io.github.fedoraiver.bevy-destruction";

#[derive(Clone, Reflect, Resource, SettingsGroup)]
#[reflect(Resource, Default, SettingsGroup)]
// BRP clients use the public resource path.
#[type_path = "bevy_destruction"]
#[settings_group(group = "demo_settings")]
pub struct DemoSettings {
    scene: String,
    pub stress: bool,
    pub vfx: bool,
    pub(crate) detail: bool,
    pub(crate) soft_particles: bool,
}

impl Default for DemoSettings {
    fn default() -> Self {
        Self {
            scene: "buildings/rectangle.destruction.json".into(),
            stress: true,
            vfx: true,
            detail: true,
            soft_particles: true,
        }
    }
}

impl DemoSettings {
    pub(crate) fn scene_path(&self) -> Result<&str, &'static str> {
        if !self.scene.ends_with(".destruction.json")
            || self.scene.contains([':', '#'])
            || !Path::new(&self.scene)
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
        {
            return Err("Scene must be a .destruction.json path relative to assets/.");
        }
        Ok(&self.scene)
    }
}
