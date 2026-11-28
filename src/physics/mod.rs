mod ground;
mod rebuild;

#[cfg(test)]
mod collapse_tests;
#[cfg(test)]
mod tests;

use avian3d::prelude::PhysicsSystems;
use bevy::{
    ecs::{lifecycle::HookContext, world::DeferredWorld},
    prelude::*,
};

use crate::scene::BuildingInstance;

use ground::ground;
pub(crate) use rebuild::rebuild_clusters;

#[cfg(test)]
use {
    crate::{
        asset::DestructibleAsset,
        scene::{BondInstance, ChunkInstance},
    },
    avian3d::prelude::*,
    std::collections::BTreeSet,
};

pub struct DestructionPhysicsPlugin;

impl Plugin for DestructionPhysicsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, ground).add_systems(
            FixedPostUpdate,
            rebuild_clusters.before(PhysicsSystems::First),
        );
    }
}

#[derive(Component, Clone)]
#[cfg_attr(feature = "brp", derive(Reflect), reflect(Component))]
pub struct Cluster {
    pub chunks: Vec<String>,
    pub anchored: bool,
}

#[derive(Component)]
#[component(storage = "SparseSet", on_insert = mark_topology_changed)]
#[cfg_attr(feature = "brp", derive(Reflect), reflect(Component))]
pub(crate) struct NeedsClusterRebuild;

fn mark_topology_changed(mut world: DeferredWorld, context: HookContext) {
    // Stress and force diagnostics share the building's version of the structure inputs.
    if let Some(mut instance) = world.get_mut::<BuildingInstance>(context.entity) {
        instance.set_changed();
    }
}

pub(crate) fn building_clusters(
    world: &World,
    building: Entity,
) -> impl Iterator<Item = Entity> + '_ {
    world
        .get::<Children>(building)
        .into_iter()
        .flatten()
        .copied()
        .filter(move |&entity| world.get::<Cluster>(entity).is_some())
}

#[cfg(test)]
pub(crate) fn rebuild_clusters_for_test(world: &mut World) {
    use bevy::ecs::system::RunSystemOnce;

    world
        .run_system_once(rebuild_clusters)
        .expect("Rebuild clusters without advancing physics");
}
