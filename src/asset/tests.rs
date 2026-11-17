use super::*;

fn fixture() -> Manifest {
    serde_json::from_value(serde_json::json!({
            "schema": "bevy_destruction_asset", "schema_version": 2,
            "name": "test", "asset_id": "a".repeat(64),
            "coordinates": { "units": "meters", "up": "+Y", "handedness": "right" },
            "materials": { "concrete": { "density": 2400.0, "color": [0.5,0.5,0.5,1.0],
                "tension_strength": 3e6, "compression_strength": 30e6,
                "shear_strength": 4e6, "young_modulus": 30e9, "poisson_ratio": 0.2 } },
            "chunks": [{ "id": "a", "part_id": "wall", "kind": "wall", "material": "concrete", "node": "a",
                "position": [1.0,2.0,3.0], "rotation": [0.0,0.0,0.0,1.0], "volume": 1.0, "mass": 2400.0,
                "center_of_mass": [0.0,0.0,0.0], "inertia": [[1.0,0.0,0.0],[0.0,1.0,0.0],[0.0,0.0,1.0]],
                "colliders": [{ "type": "convex_hull", "vertices": [[0.0,0.0,0.0],[1.0,0.0,0.0],[0.0,1.0,0.0],[0.0,0.0,1.0]] }] }],
            "bonds": [], "anchors": [{ "chunk": "a", "kind": "fixed", "position": [1.0,0.0,3.0] }],
            "mesh_file": "test.glb", "mesh_sha256": "b".repeat(64)
        })).unwrap()
}

#[test]
fn validates_units_ids_references_and_geometry() {
    let mut asset = fixture();
    assert!(asset.validate().is_ok());
    asset.coordinates.up = "+Z".into();
    assert!(asset.validate().is_err());
    asset = fixture();
    asset.chunks.push(asset.chunks[0].clone());
    assert!(asset.validate().is_err());
    asset = fixture();
    asset.anchors[0].chunk = "missing".into();
    assert!(asset.validate().is_err());
    asset = fixture();
    asset.chunks[0].colliders[0].vertices[3] = [0.5, 0.5, 0.0];
    assert!(asset.validate().is_err());
}

#[test]
fn rejects_bad_mass_quaternions_paths_and_tensors() {
    for mutation in [
        |asset: &mut Manifest| asset.chunks[0].mass = 2.0,
        |asset: &mut Manifest| asset.chunks[0].rotation = [0.0; 4],
        |asset: &mut Manifest| asset.chunks[0].inertia[0][1] = 2.0,
        |asset: &mut Manifest| asset.mesh_file = "../escape.glb".into(),
        |asset: &mut Manifest| asset.chunks[0].position[0] = f32::NAN,
    ] {
        let mut asset = fixture();
        mutation(&mut asset);
        assert!(asset.validate().is_err());
    }
}

fn multipatch_fixture() -> Manifest {
    let mut asset = fixture();
    asset.schema_version = 2;
    let mut a = asset.chunks[0].clone();
    a.position = [0.0; 3];
    a.colliders[0].vertices = [-1.0, 0.0]
        .into_iter()
        .flat_map(|x| {
            [0.0, 1.0]
                .into_iter()
                .flat_map(move |y| [0.0, 1.0].into_iter().map(move |z| [x, y, z]))
        })
        .collect();
    let mut b = a.clone();
    b.id = "b".into();
    b.node = "b".into();
    for point in &mut b.colliders[0].vertices {
        point[0] += 1.0;
    }
    asset.chunks = vec![a, b];
    asset.bonds = vec![BondRecord {
        id: "ab".into(),
        a: "a".into(),
        b: "b".into(),
        area: 1.0,
        center: [0.0, 0.5, 0.5],
        normal: [1.0, 0.0, 0.0],
        area_covariance: [
            [0.0, 0.0, 0.0],
            [0.0, 1.0 / 12.0, 0.0],
            [0.0, 0.0, 1.0 / 12.0],
        ],
        patches: [0.0, 0.5]
            .into_iter()
            .map(|y| BondPatchRecord {
                area: 0.5,
                center: [0.0, y + 0.25, 0.5],
                normal: [1.0, 0.0, 0.0],
                area_covariance: [
                    [0.0, 0.0, 0.0],
                    [0.0, 1.0 / 96.0, 0.0],
                    [0.0, 0.0, 1.0 / 24.0],
                ],
                vertices: vec![
                    [0.0, y, 0.0],
                    [0.0, y + 0.5, 0.0],
                    [0.0, y + 0.5, 1.0],
                    [0.0, y, 1.0],
                ],
            })
            .collect(),
        state: "rigid".into(),
    }];
    asset
}

#[test]
fn v2_patch_metrics_and_parent_parallel_axis_aggregation_are_validated() {
    let asset = multipatch_fixture();
    asset.validate().unwrap();
    for mutate in [
        |asset: &mut Manifest| asset.bonds[0].patches.clear(),
        |asset: &mut Manifest| asset.bonds[0].patches[0].area *= 2.0,
        |asset: &mut Manifest| asset.bonds[0].patches[0].center[1] += 0.1,
        |asset: &mut Manifest| asset.bonds[0].patches[0].area_covariance[1][1] *= 2.0,
        |asset: &mut Manifest| asset.bonds[0].area *= 1.5,
        |asset: &mut Manifest| asset.bonds[0].center[1] += 0.1,
        |asset: &mut Manifest| asset.bonds[0].area_covariance[1][1] *= 2.0,
        |asset: &mut Manifest| asset.bonds[0].normal = [0.0, 1.0, 0.0],
    ] {
        let mut asset = asset.clone();
        mutate(&mut asset);
        assert!(asset.validate().is_err());
    }
    let mut unsupported = fixture();
    unsupported.schema_version = 1;
    assert!(unsupported.validate().is_err());
}

#[test]
fn bond_polygons_are_required_during_deserialization() {
    let missing = serde_json::json!({
        "id":"ab", "a":"a", "b":"b", "area":1.0,
        "center":[0.0,0.0,0.0], "normal":[1.0,0.0,0.0],
        "area_covariance":[[0.0,0.0,0.0],[0.0,1.0,0.0],[0.0,0.0,1.0]],
        "state":"rigid"
    });
    assert!(serde_json::from_value::<BondRecord>(missing).is_err());
}

#[test]
fn rejects_nonplanar_reversed_concave_duplicated_and_out_of_bounds_patches() {
    for mutate in [
        |asset: &mut Manifest| asset.bonds[0].patches[0].vertices[0][0] = 0.1,
        |asset: &mut Manifest| asset.bonds[0].patches[0].vertices.reverse(),
        |asset: &mut Manifest| asset.bonds[0].patches[0].vertices[1] = [0.0, 0.1, 0.5],
        |asset: &mut Manifest| {
            asset.bonds[0].patches[1] = asset.bonds[0].patches[0].clone();
        },
        |asset: &mut Manifest| {
            for patch in &mut asset.bonds[0].patches {
                patch.center[0] += 2.0;
                for vertex in &mut patch.vertices {
                    vertex[0] += 2.0;
                }
            }
            asset.bonds[0].center[0] += 2.0;
        },
    ] {
        let mut asset = multipatch_fixture();
        mutate(&mut asset);
        assert!(asset.validate().is_err());
    }
}

#[test]
fn small_patch_cannot_hide_an_incorrect_normal_inside_position_tolerance() {
    let patch = BondPatchRecord {
        area: 1.6e-8,
        center: [0.0, 1e-5, 0.0005],
        normal: [0.8, 0.6, 0.0],
        area_covariance: [
            [0.0, 0.0, 0.0],
            [0.0, 1.6e-8 * 4e-10 / 12.0, 0.0],
            [0.0, 0.0, 1.6e-8 * 1e-6 / 12.0],
        ],
        vertices: vec![
            [0.0, 0.0, 0.0],
            [0.0, 2e-5, 0.0],
            [0.0, 2e-5, 0.001],
            [0.0, 0.0, 0.001],
        ],
    };
    assert!(patch_metrics(&patch).is_err());
}

#[test]
fn rejects_mesh_hash_mismatch() {
    let bytes = b"test GLB contents";
    let digest = format!("{:x}", Sha256::digest(bytes));
    assert!(validate_mesh_hash(bytes, &digest).is_ok());
    assert!(validate_mesh_hash(b"modified", &digest).is_err());
}

#[test]
fn authored_manifest_and_mesh_hash_are_valid() {
    let path = std::env::var_os("DESTRUCTION_ASSET_CHECK")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("assets/buildings/rectangle.destruction.json")
        });
    let manifest: Manifest = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    manifest.validate().unwrap();
    let mesh = std::fs::read(path.parent().unwrap().join(&manifest.mesh_file)).unwrap();
    validate_mesh_hash(&mesh, &manifest.mesh_sha256).unwrap();
}

#[test]
fn surface_roles_require_explicit_material_metadata() {
    assert_eq!(
        surface_role(Some(r#"{"bd_surface_role":"fracture"}"#)).unwrap(),
        SurfaceRole::Fracture
    );
    assert_eq!(
        surface_role(Some(r#"{"bd_surface_role":"exterior"}"#)).unwrap(),
        SurfaceRole::Exterior
    );
    assert!(surface_role(None).is_err());
    assert!(surface_role(Some("{}")).is_err());
    assert!(surface_role(Some(r#"{"bd_surface_role":"unknown"}"#)).is_err());
}

#[test]
fn triangles_support_indexed_and_nonindexed_primitives() {
    use bevy::{asset::RenderAssetUsages, mesh::Indices};
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
    );
    assert_eq!(mesh_triangles(&mesh).unwrap().len(), 1);
    mesh.insert_indices(Indices::U16(vec![2, 1, 0]));
    assert_eq!(mesh_triangles(&mesh).unwrap()[0][0], Vec3::Y);
    mesh.insert_indices(Indices::U16(vec![0, 1, 9]));
    assert!(mesh_triangles(&mesh).is_err());
}

#[test]
fn bundled_buildings_match_manifests_and_all_material_primitives() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/buildings");
    for name in [
        "rectangle",
        "l_shape",
        "courtyard",
        "highrise_slab",
        "highrise_l",
        "bridge",
    ] {
        let manifest: Manifest = serde_json::from_slice(
            &std::fs::read(directory.join(format!("{name}.destruction.json"))).unwrap(),
        )
        .unwrap();
        manifest
            .validate()
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        let bytes = std::fs::read(directory.join(&manifest.mesh_file)).unwrap();
        validate_mesh_hash(&bytes, &manifest.mesh_sha256).unwrap();
        let word =
            |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        assert_eq!(word(0), 0x46546c67);
        assert_eq!(word(4), 2);
        assert_eq!(word(8) as usize, bytes.len());
        assert_eq!(word(16), 0x4e4f534a);
        let document: serde_json::Value =
            serde_json::from_slice(&bytes[20..20 + word(12) as usize]).unwrap();
        let nodes = document["nodes"].as_array().unwrap();
        assert_eq!(nodes.len(), manifest.chunks.len());
        let nodes: BTreeMap<_, _> = nodes
            .iter()
            .map(|node| (node["name"].as_str().unwrap(), node))
            .collect();
        assert_eq!(nodes.len(), manifest.chunks.len());
        let mut primitive_count = 0;
        for chunk in &manifest.chunks {
            let node = nodes[chunk.node.as_str()];
            assert_eq!(node["extras"]["bd_chunk_id"], chunk.id);
            assert_eq!(node["extras"]["bd_part_id"], chunk.part_id);
            let position: [f32; 3] = serde_json::from_value(node["translation"].clone()).unwrap();
            assert!(Vec3::from_array(position).distance(Vec3::from_array(chunk.position)) < 2e-4);
            let mesh = &document["meshes"][node["mesh"].as_u64().unwrap() as usize];
            let primitives = mesh["primitives"].as_array().unwrap();
            assert!(!primitives.is_empty());
            primitive_count += primitives.len();
            for primitive in primitives {
                assert!(primitive["attributes"]["POSITION"].is_u64());
                assert!(primitive["attributes"]["NORMAL"].is_u64());
                let material =
                    &document["materials"][primitive["material"].as_u64().unwrap() as usize];
                let role = material["extras"]["bd_surface_role"].as_str().unwrap();
                assert!(matches!(role, "exterior" | "fracture"));
            }
        }
        assert!(primitive_count > manifest.chunks.len());
        assert!(!manifest.anchors.is_empty());
        assert!(!manifest.bonds.is_empty());
    }
}
