use super::*;
use crate::asset::{AnchorRecord, BondRecord, ChunkRecord};

fn fixture() -> Manifest {
    crate::stress_smoke::beam_manifest()
}

fn chunk(id: &str, mass: f32, position: Vec3) -> ChunkRecord {
    let mut record = fixture().chunks.remove(0);
    record.id = id.into();
    record.node = id.into();
    record.part_id = id.into();
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
    let mut bonds: BTreeMap<String, bool> = ["ab", "bc", "ac"].map(|id| (id.into(), true)).into();
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
fn graph_does_not_depend_on_manifest_order() {
    let mut manifest = graph();
    let alive = ["a", "b", "c", "d"].map(String::from).into();
    let bonds = ["ab", "bc", "ac"].map(|id| (id.into(), true)).into();
    let expected = components(&manifest, &alive, &bonds);
    manifest.bonds.reverse();
    manifest.chunks.reverse();
    manifest.anchors.reverse();
    assert_eq!(components(&manifest, &alive, &bonds), expected);
}
