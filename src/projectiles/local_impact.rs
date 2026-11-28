use std::collections::BTreeMap;

use bevy::{ecs::relationship::RelationshipTarget, prelude::*};

use crate::{
    asset::{BondPatchRecord, Manifest, PhysicalMaterial},
    scene::{BondPatches, BuildingInstance},
};

use super::{IMPACT_DURATION, contact::ImpactStructure};

pub(super) struct LocalImpactBond<'a> {
    id: &'a str,
    hit_first: bool,
    materials: [&'a PhysicalMaterial; 2],
    patches: Vec<&'a BondPatchRecord>,
}

pub(super) fn local_impact_bonds<'a>(
    structure: &'a ImpactStructure,
    manifest: &'a Manifest,
    instance: &'a BuildingInstance,
    hit_id: &'a str,
    hit: Entity,
) -> impl Iterator<Item = LocalImpactBond<'a>> {
    manifest.bonds.iter().filter_map(move |record| {
        if record.a != hit_id && record.b != hit_id {
            return None;
        }
        let entity = *instance.bonds.get(&record.id)?;
        let bond = structure.bonds.get(entity).ok()?;
        if !bond.active || !bond.endpoints.contains(&hit) {
            return None;
        }
        let mut patch_indices = structure
            .bond_patches
            .get(entity)
            .into_iter()
            .flat_map(BondPatches::iter)
            .filter_map(|entity| structure.structural_patches.get(entity).ok())
            .map(|patch| patch.index)
            .collect::<Vec<_>>();
        // Preserve the asset's facet order when summing contact areas.
        patch_indices.sort_unstable();
        Some(LocalImpactBond {
            id: &bond.id,
            hit_first: bond.endpoints[0] == hit,
            materials: bond.endpoints.map(|endpoint| {
                let chunk = structure.chunks.get(endpoint).unwrap();
                &manifest.materials[&chunk.material]
            }),
            patches: patch_indices
                .into_iter()
                .map(|index| &record.patches[index])
                .collect(),
        })
    })
}

pub(super) fn queue_local_damage<'a>(
    bonds: impl Iterator<Item = LocalImpactBond<'a>>,
    building: Entity,
    impulse: Vec3,
    pending: &mut BTreeMap<(Entity, String), f32>,
) {
    let bonds = bonds.collect::<Vec<_>>();
    let total_area = bonds
        .iter()
        .flat_map(|bond| &bond.patches)
        .map(|patch| patch.area)
        .sum::<f32>();
    for bond in bonds {
        let [a, b] = bond.materials;
        let utilization = |normal: Vec3| {
            let projected = impulse.dot(normal);
            let tension = projected * if bond.hit_first { -1.0 } else { 1.0 };
            let normal_strength = if tension > 0.0 {
                a.tension_strength.min(b.tension_strength)
            } else {
                a.compression_strength.min(b.compression_strength)
            };
            let normal_utilization =
                tension.abs() / (total_area * IMPACT_DURATION * normal_strength);
            let shear_utilization = (impulse - normal * projected).length()
                / (total_area * IMPACT_DURATION * a.shear_strength.min(b.shear_strength));
            normal_utilization.max(shear_utilization)
        };
        // Area-weighted facet impulses share the same traction; preserve each facet's direction.
        let utilization = bond
            .patches
            .iter()
            .map(|patch| utilization(Vec3::from(patch.normal)))
            .fold(0.0_f32, f32::max);
        if utilization >= 1.0 {
            let prior = pending.entry((building, bond.id.to_owned())).or_default();
            *prior = prior.max(utilization);
        }
    }
}
