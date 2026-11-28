use std::{collections::BTreeMap, sync::Arc};

use bevy::prelude::*;

use crate::{
    asset::{BondRecord, DestructibleAsset},
    fracture_detail::ContactPatch,
};

use super::BondInstance;

#[derive(Clone, Copy, Debug)]
pub struct BuildingBounds {
    pub min: Vec3,
    pub max: Vec3,
}

impl BuildingBounds {
    pub fn center(self) -> Vec3 {
        (self.min + self.max) * 0.5
    }
}

#[derive(Component)]
pub struct BuildingInstance {
    pub asset: Handle<DestructibleAsset>,
    pub chunks: BTreeMap<String, Entity>,
    pub bonds: BTreeMap<String, Entity>,
    pub bounds: Option<BuildingBounds>,
    pub error: Option<String>,
    pub patches: Vec<Entity>,
}

impl BuildingInstance {
    pub fn pending(asset: Handle<DestructibleAsset>) -> Self {
        Self {
            asset,
            chunks: BTreeMap::new(),
            bonds: BTreeMap::new(),
            bounds: None,
            error: None,
            patches: Vec::new(),
        }
    }

    pub(crate) fn bond_states(
        &self,
        mut is_active: impl FnMut(Entity) -> bool,
    ) -> BTreeMap<String, bool> {
        self.bonds
            .iter()
            .map(|(id, &entity)| (id.clone(), is_active(entity)))
            .collect()
    }
}

#[derive(Component)]
#[relationship(relationship_target = BuildingBonds)]
pub(crate) struct BondOfBuilding {
    pub building: Entity,
}

#[derive(Component)]
#[relationship_target(relationship = BondOfBuilding, linked_spawn)]
pub(crate) struct BuildingBonds {
    bonds: Vec<Entity>,
}

#[derive(Component)]
#[relationship(relationship_target = BondPatches)]
pub(crate) struct PatchOfBond {
    pub bond: Entity,
}

#[derive(Component)]
#[relationship_target(relationship = PatchOfBond, linked_spawn)]
pub(crate) struct BondPatches {
    patches: Vec<Entity>,
}

#[derive(Component)]
pub struct StructuralPatch {
    pub index: usize,
}

#[derive(Component)]
pub struct PatchInstance {
    pub index: usize,
    geometry: Arc<Vec<ContactPatch>>,
}

impl PatchInstance {
    pub fn patch(&self) -> &ContactPatch {
        &self.geometry[self.index]
    }
}

pub(crate) fn spawn_bonds(
    commands: &mut Commands,
    building: Entity,
    instance: &mut BuildingInstance,
    bonds: &[BondRecord],
) {
    for record in bonds {
        let entity = commands
            .spawn((
                Name::new(format!("Bond {}", record.id)),
                BondOfBuilding { building },
                BondInstance {
                    id: record.id.clone(),
                    endpoints: [instance.chunks[&record.a], instance.chunks[&record.b]],
                    active: true,
                },
            ))
            .id();
        instance.bonds.insert(record.id.clone(), entity);
        for index in 0..record.patches.len() {
            commands.spawn((
                Name::new(format!("Structural patch {}:{index}", record.id)),
                PatchOfBond { bond: entity },
                StructuralPatch { index },
            ));
        }
    }
}

pub(crate) fn spawn_patches(
    commands: &mut Commands,
    building: Entity,
    instance: &mut BuildingInstance,
    patches: Arc<Vec<ContactPatch>>,
) {
    for (index, patch) in patches.iter().enumerate() {
        let Some(&bond) = instance.bonds.get(&patch.bond_id) else {
            panic!("Surface patch {} has no bond in {building:?}", patch.id);
        };
        let entity = commands
            .spawn((
                Name::new(format!("Surface patch {}", patch.id)),
                PatchOfBond { bond },
                PatchInstance {
                    index,
                    geometry: patches.clone(),
                },
            ))
            .id();
        instance.patches.push(entity);
    }
}

#[derive(Component)]
pub(crate) struct ChunkPrimitive;

pub(crate) fn chunk_primitives<'a>(
    children: Option<&'a Children>,
    mut is_primitive: impl FnMut(Entity) -> bool + 'a,
) -> impl Iterator<Item = Entity> + 'a {
    children
        .into_iter()
        .flatten()
        .copied()
        .filter(move |&entity| is_primitive(entity))
}
