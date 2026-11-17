use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;

use crate::asset::Manifest;

#[derive(Debug, PartialEq, Eq)]
pub struct Component {
    pub chunks: Vec<String>,
    pub anchored: bool,
}

pub fn components(
    manifest: &Manifest,
    alive: &BTreeSet<String>,
    bonds: &BTreeMap<String, bool>,
) -> Vec<Component> {
    let mut neighbors: BTreeMap<&str, BTreeSet<&str>> = alive
        .iter()
        .map(|id| (id.as_str(), BTreeSet::new()))
        .collect();
    for bond in &manifest.bonds {
        if bonds.get(&bond.id) == Some(&true) && alive.contains(&bond.a) && alive.contains(&bond.b)
        {
            neighbors.get_mut(bond.a.as_str()).unwrap().insert(&bond.b);
            neighbors.get_mut(bond.b.as_str()).unwrap().insert(&bond.a);
        }
    }
    let anchors: BTreeSet<&str> = manifest
        .anchors
        .iter()
        .filter(|anchor| anchor.kind == "fixed")
        .map(|anchor| anchor.chunk.as_str())
        .collect();
    let mut remaining: BTreeSet<&str> = alive.iter().map(String::as_str).collect();
    let mut components = Vec::new();
    while let Some(first) = remaining.pop_first() {
        let mut pending = vec![first];
        let mut chunks = Vec::new();
        let mut anchored = false;
        while let Some(id) = pending.pop() {
            chunks.push(id.to_owned());
            anchored |= anchors.contains(id);
            for &neighbor in &neighbors[id] {
                if remaining.remove(neighbor) {
                    pending.push(neighbor);
                }
            }
        }
        chunks.sort();
        components.push(Component { chunks, anchored });
    }
    components
}

#[derive(Debug)]
pub struct ClusterMass {
    pub mass: f32,
    pub center: Vec3,
    pub inertia: Mat3,
}

pub fn cluster_mass(manifest: &Manifest, chunks: &[String]) -> ClusterMass {
    let records: BTreeMap<_, _> = manifest
        .chunks
        .iter()
        .map(|chunk| (chunk.id.as_str(), chunk))
        .collect();
    let chunks: BTreeSet<_> = chunks.iter().map(String::as_str).collect();
    assert!(
        !chunks.is_empty(),
        "A cluster must contain at least one chunk"
    );
    let mass = chunks.iter().map(|id| records[id].mass).sum::<f32>();
    let center = chunks
        .iter()
        .map(|id| {
            let chunk = records[id];
            chunk
                .transform()
                .transform_point(chunk.center_of_mass.into())
                * chunk.mass
        })
        .sum::<Vec3>()
        / mass;
    let mut inertia = Mat3::ZERO;
    for id in chunks {
        let chunk = records[id];
        let rotation = Mat3::from_quat(chunk.transform().rotation);
        let local_inertia = Mat3::from_cols_array_2d(&chunk.inertia);
        let offset = chunk
            .transform()
            .transform_point(chunk.center_of_mass.into())
            - center;
        let outer = Mat3::from_cols(offset * offset.x, offset * offset.y, offset * offset.z);
        inertia += rotation * local_inertia * rotation.transpose()
            + (Mat3::IDENTITY * offset.length_squared() - outer) * chunk.mass;
    }
    ClusterMass {
        mass,
        center,
        inertia,
    }
}

pub fn inherited_velocity(
    linear: Vec3,
    angular: Vec3,
    parent_center: Vec3,
    child_center: Vec3,
) -> Vec3 {
    linear + angular.cross(child_center - parent_center)
}

#[cfg(test)]
mod tests;
