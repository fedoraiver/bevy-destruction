mod contact;
mod dynamic_impact;
mod launch;
mod local_impact;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use avian3d::prelude::PhysicsSystems;
use bevy::{math::DVec3, prelude::*};

use crate::vfx::BreakBurst;

use self::{
    contact::impact_damage,
    dynamic_impact::InertialLoadCase,
    launch::{fire, reset, retire_projectiles, spawn_projectiles},
};

#[cfg(test)]
use self::{
    contact::ImpactStructure,
    dynamic_impact::{DynamicImpactReport, inertial_loads, process_dynamic_impact},
    local_impact::{local_impact_bonds, queue_local_damage},
};
#[cfg(test)]
use {
    crate::{
        asset::{DestructibleAsset, Manifest},
        force_samples::{ForceCapture, ForceSamples},
        physics::NeedsClusterRebuild,
        scene::{BondInstance, BuildingInstance, ChunkIdentity, ChunkInstance, DemoCamera},
        stress::{self, SolverSettings},
    },
    avian3d::{collision::contact_types::ContactGraph, prelude::*},
};

pub struct ProjectilePlugin;

impl Plugin for ProjectilePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LaunchRequests>()
            .init_resource::<ImpactStats>()
            .init_resource::<ImpactState>()
            .add_message::<BreakBurst>()
            .add_observer(reset)
            .add_systems(Update, fire.after(crate::camera::move_camera))
            .add_systems(FixedUpdate, (spawn_projectiles, retire_projectiles))
            .add_systems(FixedPostUpdate, impact_damage.after(PhysicsSystems::Last));
    }
}

pub const PROJECTILE_MASS: f32 = 8000.0;
pub const PROJECTILE_SPEED: f32 = 55.0;
const IMPACT_DURATION: f32 = 0.01;
const IMPACT_MIN_SPEED: f32 = 2.0;
const IMPACT_BREAK_LIMIT: usize = 8;
// 为同一步的局部接触保留一半断裂预算。
const DYNAMIC_IMPACT_BREAK_LIMIT: usize = IMPACT_BREAK_LIMIT / 2;

#[derive(Clone, Copy)]
#[cfg_attr(feature = "brp", derive(Reflect))]
pub enum ProjectileShape {
    Sphere,
    Cube,
}

pub struct LaunchRequest {
    pub origin: Vec3,
    pub direction: Dir3,
    pub shape: ProjectileShape,
}

#[derive(Resource, Default)]
pub struct LaunchRequests {
    pub queue: VecDeque<LaunchRequest>,
}

#[derive(Component)]
#[cfg_attr(feature = "brp", derive(Reflect), reflect(Component))]
pub struct RigidProjectile {
    age: f32,
    #[cfg(feature = "brp")]
    shape: ProjectileShape,
}

#[derive(Resource, Default, Clone)]
#[cfg_attr(feature = "brp", derive(Reflect), reflect(Resource))]
pub struct ImpactStats {
    pub launched: u64,
    pub contacts: u64,
    #[cfg(feature = "brp")]
    pub sphere_contacts: u64,
    #[cfg(feature = "brp")]
    pub cube_contacts: u64,
    pub broken_bonds: u64,
    pub last_impulse: f32,
    pub max_impulse: f32,
    pub structural_solves: u64,
    pub structural_failures: u64,
    pub pending_structural: usize,
    pub max_structural_solve_ms: f64,
    pub max_force_balance_ratio: f64,
    pub max_moment_balance_ratio: f64,
    pub max_gauge_reaction_ratio: f64,
    pub structural_broken_bonds: u64,
    pub released_structural_events: u64,
    pub rebound_fresh_structural_events: u64,
    pub invalidated_structural_events: u64,
    pub max_structural_utilization: f64,
}

#[derive(Clone, Copy, Default)]
struct ImpulseLoad {
    linear: DVec3,
    angular: DVec3,
}

struct ClusterImpact {
    building: Entity,
    chunks: Vec<String>,
    loads: BTreeMap<String, ImpulseLoad>,
    prepared: Option<InertialLoadCase>,
}

#[derive(Resource, Default)]
struct ImpactState {
    touching: BTreeSet<(Entity, Entity)>,
    pending: BTreeMap<(Entity, String), f32>,
    shocks: VecDeque<ClusterImpact>,
    released: u64,
    rebound_fresh: u64,
    invalidated: u64,
}

#[derive(Event)]
pub(crate) struct ResetProjectiles;
