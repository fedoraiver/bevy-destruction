mod overload;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use bevy::{ecs::change_detection::Tick, prelude::*};

use crate::{
    physics,
    stress::{BondStress, NodeLoad, SolverSettings},
    vfx::BreakBurst,
};

pub(crate) use overload::update_stress;

#[cfg(test)]
use {
    crate::{
        asset::DestructibleAsset,
        scene::{BondInstance, BuildingInstance, ChunkInstance},
    },
    avian3d::prelude::*,
};

pub struct StressRuntimePlugin;

impl Plugin for StressRuntimePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<StressSettings>()
            .add_message::<BreakBurst>()
            .add_systems(
                FixedPostUpdate,
                update_stress.before(physics::rebuild_clusters),
            );
    }
}

#[derive(Resource)]
#[cfg_attr(feature = "brp", derive(Reflect), reflect(Resource))]
pub struct StressSettings {
    pub enabled: bool,
    pub damage_seconds: f64,
    pub immediate_utilization: f64,
    pub max_breaks_per_step: usize,
    pub solver: SolverSettings,
}

impl Default for StressSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            damage_seconds: 0.5,
            immediate_utilization: 4.0,
            max_breaks_per_step: 4,
            solver: SolverSettings::default(),
        }
    }
}

/// Applied loads on this building, in building coordinates and about the authored structural nodes.
#[derive(Component, Default)]
#[cfg_attr(feature = "brp", derive(Reflect), reflect(Component))]
pub struct StructuralLoads {
    pub by_chunk: BTreeMap<String, NodeLoad>,
}

#[cfg_attr(feature = "brp", derive(Reflect))]
pub struct BondLoadState {
    pub stress: BondStress,
    pub damage: f64,
    pub valid: bool,
}

#[derive(Clone)]
#[cfg_attr(not(feature = "brp"), expect(dead_code))]
#[cfg_attr(feature = "brp", derive(Reflect))]
pub struct StressComponentReport {
    pub chunks: usize,
    pub valid: bool,
    pub iterations: usize,
    pub relative_residual: f64,
    pub error: Option<String>,
    pub force_balance_error: f64,
    pub moment_balance_error: f64,
    pub max_displacement: f64,
}

#[derive(Component, Default)]
#[cfg_attr(feature = "brp", derive(Reflect), reflect(Component))]
pub struct StressState {
    pub bonds: BTreeMap<String, BondLoadState>,
    pub reports: Vec<StressComponentReport>,
    pub solves: u64,
    pub numerical_solves: u64,
    pub failures: u64,
    pub broken_total: u64,
    pub last_broken: Vec<String>,
    pub max_utilization: f64,
    pub max_solve_ms: f64,
    pub skipped_dynamic: usize,
    cache: Option<StressCache>,
    requires_damage_update: bool,
    static_components: BTreeMap<String, StaticComponentCache>,
}

#[cfg_attr(feature = "brp", derive(Reflect))]
struct StaticComponentCache {
    key: u64,
    chunks: Vec<String>,
    report: StressComponentReport,
    bonds: Vec<String>,
    max_utilization: f64,
}

#[derive(Clone, Copy)]
#[cfg_attr(feature = "brp", derive(Reflect))]
struct StressCache {
    key: u64,
    source_tick: u32,
    loads_tick: Option<Tick>,
    settings_tick: u32,
    gravity: Vec3,
    retry_at: f64,
}

impl StressState {
    pub fn inputs_current(
        &self,
        source_tick: u32,
        loads_tick: Option<Tick>,
        settings_tick: u32,
        gravity: Vec3,
    ) -> bool {
        self.cache.is_some_and(|cache| {
            cache.source_tick == source_tick
                && cache.loads_tick == loads_tick
                && cache.settings_tick == settings_tick
                && cache.gravity == gravity
        })
    }
}
