mod drawing;
mod hud;
mod picking;

use bevy::{
    asset::AssetEventSystems, camera::visibility::VisibilitySystems,
    gizmos::config::GizmoConfigGroup, prelude::*,
};

use crate::{
    force_samples::{ForceCapture, ForceSamples},
    force_visibility::ChunkKey,
};

use drawing::draw;
use hud::{setup, update_hud};
use picking::{Aim, pick_target};

#[cfg(test)]
use {
    crate::{
        asset::DestructibleAsset,
        camera::CameraControl,
        force_samples::{DynamicForceSample, bond_signature},
        material::{HEAT_AMBER_TAG, HEAT_RED_TAG},
        physics::Cluster,
        scene::{BondInstance, BuildingInstance, ChunkInstance, DemoCamera},
        stress_runtime::{StressState, StructuralLoads},
    },
    avian3d::prelude::{ColliderOf, MassProperties3d, PhysicsPlugins, RigidBody},
    bevy::window::PrimaryWindow,
    drawing::{
        InspectionBond, direction_on_chunk, heat_tag, order_inspection_bonds,
        sample_members_current,
    },
    hud::inspection_text,
    std::collections::BTreeMap,
};

#[derive(Default, Reflect, GizmoConfigGroup)]
struct ForceGizmos;

#[derive(Resource, Default, Reflect)]
#[reflect(Resource)]
pub struct ForceViewStats {
    pub static_bonds: usize,
    pub dynamic_bonds: usize,
    pub max_force_n: f64,
    pub tinted_chunks: usize,
    pub inspected: Option<ForceInspection>,
    pub force_arrows: usize,
    pub pick_queries: usize,
}

#[derive(Reflect)]
pub struct ForceInspection {
    pub chunk: ChunkKey,
    pub bonds: Vec<ForceBondReadout>,
    pub impulse_ns: Option<f64>,
}

#[derive(Reflect)]
pub struct ForceBondReadout {
    pub id: String,
    pub force_n: f64,
    pub moment_nm: f64,
    pub utilization: f64,
    pub impact: bool,
}

pub struct ForceDebugPlugin;

impl Plugin for ForceDebugPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ForceCapture>()
            .init_resource::<ForceSamples>()
            .init_resource::<ForceViewStats>()
            .init_resource::<Aim>()
            .init_gizmo_group::<ForceGizmos>()
            .add_systems(Startup, setup)
            .add_systems(Update, toggle)
            .add_systems(
                PostUpdate,
                (
                    pick_target
                        .after(AssetEventSystems)
                        .after(VisibilitySystems::CheckVisibility),
                    draw,
                    update_hud,
                )
                    .chain(),
            );
    }
}

fn toggle(keys: Res<ButtonInput<KeyCode>>, mut capture: ResMut<ForceCapture>) {
    if keys.just_pressed(KeyCode::Digit9) || keys.just_pressed(KeyCode::Numpad9) {
        capture.enabled = !capture.enabled;
    }
}

#[cfg(test)]
mod tests;
