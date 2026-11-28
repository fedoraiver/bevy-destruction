//! Particle effect contracts and system registration.

mod assets;
mod emission;
mod simulation;

use bevy::{prelude::*, render::storage::ShaderBuffer};

use crate::dust_material::DustMaterial;
use assets::{prewarm_pipelines, setup};
use emission::spawn_bursts;
use simulation::animate_particles;

#[cfg(test)]
use {
    crate::{
        dust_material::DustSettings,
        scene::{ChunkInstance, DemoCamera, RemovedChunk},
    },
    assets::PipelineWarmup,
    avian3d::prelude::{Collider, MassProperties3d, PhysicsPlugins, RigidBody},
    bevy::mesh::MeshTag,
    emission::{screen_footprint, spawn_particle},
};

#[cfg(test)]
mod tests;

pub const PARTICLE_LIMIT: usize = 640;
const LAYER_LIMITS: [usize; 4] = [96, 96, 320, 128];
const DUST_POOL_SIZE: usize = PARTICLE_LIMIT - LAYER_LIMITS[1];
const DEBRIS_FADE_STEPS: usize = 16;

pub struct VfxPlugin;

impl Plugin for VfxPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<BreakBurst>()
            .init_resource::<Assets<ShaderBuffer>>()
            .init_resource::<VfxStats>()
            .init_resource::<VfxSettings>()
            .add_systems(Startup, setup)
            .add_systems(
                Update,
                (
                    spawn_bursts.after(crate::interaction::apply_damage),
                    animate_particles.after(crate::camera::move_camera),
                    prewarm_pipelines,
                )
                    .chain(),
            );
    }
}

#[derive(Clone, Debug)]
pub struct EmissionSurface {
    pub triangle: [Vec3; 3],
    pub normal: Vec3,
}

#[derive(Message, Clone)]
pub struct BreakBurst {
    pub point: Vec3,
    pub normal: Vec3,
    pub seed: u64,
    pub material: String,
    pub removed_volume: f32,
    pub exposed_area: f32,
    pub surfaces: Vec<EmissionSurface>,
}

#[derive(Resource)]
pub struct VfxSettings {
    pub enabled: bool,
    pub density: f32,
}

impl Default for VfxSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            density: 1.0,
        }
    }
}

#[derive(Resource, Default, Debug)]
#[cfg_attr(feature = "brp", derive(Reflect), reflect(Resource))]
pub struct VfxStats {
    pub active: [usize; 4],
    pub scheduled: [usize; 4],
    pub peak: usize,
    pub received: u64,
    pub spawned: [u64; 4],
    pub dropped: [u64; 4],
    pub retired: u64,
    pub first_feedbacks: u64,
    pub feedback_bursts: u64,
    pub collision_queries: u64,
    pub collisions: u64,
    pub update_cpu_ms: f32,
    pub max_update_cpu_ms: f32,
    pub emit_cpu_ms: f32,
    pub max_emit_cpu_ms: f32,
    pub estimated_coverage: f32,
    pub peak_estimated_coverage: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Layer {
    Impact,
    Debris,
    Smoke,
    Fallout,
}

impl Layer {
    fn index(self) -> usize {
        self as usize
    }
    fn from_index(index: usize) -> Self {
        [Self::Impact, Self::Debris, Self::Smoke, Self::Fallout][index]
    }
}

#[derive(Component)]
pub struct Particle {
    pub age: f32,
    pub lifetime: f32,
    layer: Layer,
    velocity: Vec3,
    size: f32,
    spin: Vec3,
    roll: f32,
    variant: u32,
    opacity: f32,
    palette: usize,
    dust_slot: Option<usize>,
    bounces: u8,
    settled: bool,
    sweep_origin: Vec3,
    support: Option<Entity>,
}

#[derive(Resource)]
struct VfxAssets {
    root: Entity,
    billboard: Handle<Mesh>,
    debris: Vec<Handle<Mesh>>,
    solids: Vec<Vec<Handle<StandardMaterial>>>,
    dust: Handle<DustMaterial>,
    dust_instances: Handle<ShaderBuffer>,
    free_dust: Vec<usize>,
}

fn random(state: &mut u64) -> f32 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    (*state >> 40) as f32 / (1_u32 << 24) as f32
}
