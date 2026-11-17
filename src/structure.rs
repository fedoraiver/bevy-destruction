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
mod tests {
    use super::*;
    use crate::asset::{AnchorRecord, BondRecord, ChunkRecord};

    fn fixture() -> Manifest {
        serde_json::from_str(include_str!(
            "../assets/buildings/rectangle.destruction.json"
        ))
        .unwrap()
    }

    fn chunk(id: &str, mass: f32, position: Vec3) -> ChunkRecord {
        let mut record = fixture().chunks.remove(0);
        record.id = id.into();
        record.mass = mass;
        record.position = position.to_array();
        record.rotation = Quat::IDENTITY.to_array();
        record.center_of_mass = [0.0; 3];
        record.inertia = Mat3::IDENTITY.to_cols_array_2d();
        record
    }

    fn bond(id: &str, a: &str, b: &str) -> BondRecord {
        let mut record = fixture().bonds.remove(0);
        record.id = id.into();
        record.a = a.into();
        record.b = b.into();
        record
    }

    fn graph() -> Manifest {
        let mut manifest = fixture();
        manifest.chunks = ["a", "b", "c", "d"]
            .map(|id| chunk(id, 1.0, Vec3::ZERO))
            .into();
        manifest.bonds = vec![
            bond("ab", "a", "b"),
            bond("bc", "b", "c"),
            bond("ac", "a", "c"),
        ];
        manifest.anchors = ["a", "c"]
            .map(|id| AnchorRecord {
                chunk: id.into(),
                kind: "fixed".into(),
                position: [0.0; 3],
            })
            .into();
        manifest
    }

    #[test]
    fn alternate_path_and_remaining_anchor_preserve_support() {
        let manifest = graph();
        let mut alive: BTreeSet<String> = ["a", "b", "c", "d"].map(String::from).into();
        let mut bonds: BTreeMap<String, bool> =
            ["ab", "bc", "ac"].map(|id| (id.into(), true)).into();
        bonds.insert("ab".into(), false);
        let intact = components(&manifest, &alive, &bonds);
        assert_eq!(
            intact,
            vec![
                Component {
                    chunks: vec!["a".into(), "b".into(), "c".into()],
                    anchored: true
                },
                Component {
                    chunks: vec!["d".into()],
                    anchored: false
                },
            ]
        );
        alive.remove("a");
        assert!(components(&manifest, &alive, &bonds)[0].anchored);
        alive.remove("c");
        assert!(
            components(&manifest, &alive, &bonds)
                .iter()
                .all(|part| !part.anchored)
        );
        alive.clear();
        assert!(components(&manifest, &alive, &bonds).is_empty());
    }

    #[test]
    fn graph_and_mass_do_not_depend_on_manifest_or_chunk_order() {
        let mut manifest = graph();
        let alive = ["a", "b", "c", "d"].map(String::from).into();
        let bonds = ["ab", "bc", "ac"].map(|id| (id.into(), true)).into();
        let expected = components(&manifest, &alive, &bonds);
        manifest.bonds.reverse();
        manifest.chunks.reverse();
        manifest.anchors.reverse();
        assert_eq!(components(&manifest, &alive, &bonds), expected);
        let first = cluster_mass(&manifest, &["b".into(), "a".into()]);
        let second = cluster_mass(&manifest, &["a".into(), "b".into()]);
        assert_eq!(first.mass, second.mass);
        assert_eq!(first.center, second.center);
        assert_eq!(first.inertia, second.inertia);
    }

    #[test]
    fn rotates_local_center_and_asymmetric_inertia() {
        let mut manifest = fixture();
        let mut record = chunk("a", 2.0, Vec3::new(3.0, 4.0, 5.0));
        record.rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2).to_array();
        record.center_of_mass = [1.0, 0.0, 0.0];
        record.inertia = Mat3::from_diagonal(Vec3::new(2.0, 3.0, 4.0)).to_cols_array_2d();
        manifest.chunks = vec![record];
        let mass = cluster_mass(&manifest, &["a".into()]);
        assert!((mass.center - Vec3::new(3.0, 5.0, 5.0)).length() < 1e-5);
        assert!(
            mass.inertia
                .abs_diff_eq(Mat3::from_diagonal(Vec3::new(3.0, 2.0, 4.0)), 1e-5)
        );
    }

    #[test]
    fn separated_masses_include_parallel_axis_inertia() {
        let mut manifest = fixture();
        manifest.chunks = vec![chunk("a", 2.0, Vec3::NEG_X), chunk("b", 2.0, Vec3::X)];
        let mass = cluster_mass(&manifest, &["a".into(), "b".into()]);
        assert_eq!(mass.mass, 4.0);
        assert_eq!(mass.center, Vec3::ZERO);
        assert!(
            mass.inertia
                .abs_diff_eq(Mat3::from_diagonal(Vec3::new(2.0, 6.0, 6.0)), 1e-5)
        );
    }

    #[test]
    fn rotating_split_preserves_total_momentum_in_world_frame() {
        let mut manifest = fixture();
        manifest.chunks = vec![
            chunk("a", 2.0, Vec3::new(-1.0, 0.0, 0.0)),
            chunk("b", 3.0, Vec3::new(2.0, 1.0, 0.0)),
            chunk("c", 4.0, Vec3::new(0.0, 0.0, 3.0)),
        ];
        let all = ["a", "b", "c"].map(String::from);
        let parent = cluster_mass(&manifest, &all);
        let rotation = Mat3::from_quat(Quat::from_euler(EulerRot::YXZ, 0.7, -0.4, 0.2));
        let origin = Vec3::new(10.0, -2.0, 4.0);
        let parent_center = origin + rotation * parent.center;
        let linear = Vec3::new(3.0, -4.0, 1.0);
        let angular = Vec3::new(0.3, -0.8, 1.1);
        let mut momentum = Vec3::ZERO;
        let mut angular_momentum = Vec3::ZERO;
        for group in [&all[..2], &all[2..]] {
            let child = cluster_mass(&manifest, group);
            let center = origin + rotation * child.center;
            let velocity = inherited_velocity(linear, angular, parent_center, center);
            momentum += child.mass * velocity;
            angular_momentum += rotation * child.inertia * rotation.transpose() * angular
                + (center - parent_center).cross(child.mass * velocity);
        }
        assert!((momentum - parent.mass * linear).length() < 2e-5);
        let expected = rotation * parent.inertia * rotation.transpose() * angular;
        assert!((angular_momentum - expected).length() < 3e-5);
    }
}
