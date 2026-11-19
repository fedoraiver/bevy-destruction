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
