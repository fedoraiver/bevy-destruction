use super::*;
use crate::asset::{BondRecord, Coordinates};
use bevy::mesh::VertexAttributeValues;

#[test]
fn tiny_rounded_interface_checks_its_overlap_on_the_actual_source_plane() {
    let normal = DVec3::new(1.0, 0.001, 0.0).normalize();
    let plane = Plane::new(normal, DVec3::ZERO);
    let vertices = [
        (-0.00004, -0.0045),
        (0.00004, -0.0045),
        (0.00004, 0.0045),
        (-0.00004, 0.0045),
    ]
    .map(|(y, z)| DVec3::new(-0.001 * y, y, z));
    let polygon = vertices.map(|point| plane.project(point));
    let expected = polygon_area(&polygon).abs();
    let triangle = [
        Vec3::new(0.0, -1.0, -1.0),
        Vec3::new(0.0, 1.0, -1.0),
        Vec3::new(0.0, 0.0, 1.0),
    ];
    assert!(
        triangle
            .iter()
            .any(|point| plane.normal.dot(point.as_dvec3()).abs() > CONTACT_TOLERANCE)
    );
    for orientation in [1.0, -1.0] {
        let mut triangle = triangle;
        if orientation < 0.0 {
            triangle.swap(1, 2);
        }
        let source = SurfaceGeometry {
            triangles: vec![triangle],
            exterior: Vec::new(),
        };
        let covered = mesh_patch_coverage(&plane, &polygon, &source, orientation).unwrap();
        assert!(
            (covered - expected).abs() < 1e-12,
            "{covered} != {expected}"
        );
    }
    let displaced = SurfaceGeometry {
        triangles: vec![triangle.map(|point| point + Vec3::X * 0.0001)],
        exterior: Vec::new(),
    };
    assert_eq!(
        mesh_patch_coverage(&plane, &polygon, &displaced, 1.0).unwrap(),
        0.0
    );
}

#[test]
fn contact_surface_masks_keep_exact_planes_area_and_both_endpoint_windings() {
    let first = [Vec3::ZERO, Vec3::Z, Vec3::Y];
    let second = [
        Vec3::new(2.0, 0.0, 1.0),
        Vec3::new(3.0, 0.0, 1.0),
        Vec3::new(2.0, 1.0, 1.0),
    ];
    let patches = [
        ContactPatch {
            id: "first".into(),
            bond_id: "ab".into(),
            a: "a".into(),
            b: "b".into(),
            normal: Vec3::X,
            triangles: vec![first],
            area: 0.5,
            rims: Vec::new(),
        },
        ContactPatch {
            id: "second".into(),
            bond_id: "ca".into(),
            a: "c".into(),
            b: "a".into(),
            normal: Vec3::Z,
            triangles: vec![second],
            area: 0.5,
            rims: Vec::new(),
        },
    ];
    let mesh = contact_surface_mesh(&patches, "a").unwrap();
    let triangles = crate::asset::mesh_triangles(&mesh).unwrap();
    let VertexAttributeValues::Float32x3(normals) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL).unwrap()
    else {
        panic!()
    };
    assert_eq!(triangles.len(), 2);
    let area: f32 = triangles
        .iter()
        .map(|t| (t[1] - t[0]).cross(t[2] - t[0]).length() * 0.5)
        .sum();
    assert_eq!(area, 1.0);
    for (index, source) in [first, second].iter().enumerate() {
        assert!(triangles[index].iter().all(|point| source.contains(point)));
        let expected = if index == 0 { Vec3::X } else { Vec3::NEG_Z };
        assert_eq!(normals[index * 3..index * 3 + 3], [expected.to_array(); 3]);
        assert!(
            (triangles[index][1] - triangles[index][0])
                .cross(triangles[index][2] - triangles[index][0])
                .dot(expected)
                > 0.0
        );
    }
    let other = contact_surface_mesh(&patches, "b").unwrap();
    let other = crate::asset::mesh_triangles(&other).unwrap();
    assert_eq!(other.len(), 1);
    assert!(other[0].iter().all(|point| first.contains(point)));
    assert!(
        (other[0][1] - other[0][0])
            .cross(other[0][2] - other[0][0])
            .dot(Vec3::NEG_X)
            > 0.0
    );
    assert!(contact_surface_mesh(&patches, "unrelated").is_none());
}

fn cube(min: Vec3, max: Vec3, hidden_axis: usize, hidden_positive: bool) -> SurfaceGeometry {
    let mut geometry = SurfaceGeometry::default();
    for axis in 0..3 {
        for positive in [false, true] {
            let u = (axis + 1) % 3;
            let v = (axis + 2) % 3;
            let mut points = [min; 4];
            for (i, point) in points.iter_mut().enumerate() {
                point[axis] = if positive { max[axis] } else { min[axis] };
                point[u] = if i == 1 || i == 2 { max[u] } else { min[u] };
                point[v] = if i >= 2 { max[v] } else { min[v] };
            }
            if !positive {
                points.reverse();
            }
            let triangles = [
                [points[0], points[1], points[2]],
                [points[0], points[2], points[3]],
            ];
            geometry.triangles.extend(triangles);
            if axis != hidden_axis || positive != hidden_positive {
                geometry.exterior.extend(triangles);
            }
        }
    }
    geometry
}

fn manifest(area: f32) -> Manifest {
    Manifest {
        schema: "bevy_destruction_asset".into(),
        schema_version: 2,
        name: "test".into(),
        asset_id: String::new(),
        coordinates: Coordinates {
            units: "meters".into(),
            up: "+Y".into(),
            handedness: "right".into(),
        },
        materials: BTreeMap::new(),
        chunks: Vec::new(),
        bonds: vec![BondRecord {
            id: "bond".into(),
            a: "a".into(),
            b: "b".into(),
            area,
            center: [0.0, 0.5, area * 0.5],
            normal: [1.0, 0.0, 0.0],
            area_covariance: [
                [0.0; 3],
                [0.0, area / 12.0, 0.0],
                [0.0, 0.0, area.powi(3) / 12.0],
            ],
            patches: vec![rectangular_patch([
                [0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 1.0, area],
                [0.0, 0.0, area],
            ])],
            state: "rigid".into(),
        }],
        anchors: Vec::new(),
        mesh_file: String::new(),
        mesh_sha256: String::new(),
    }
}

fn adjacent(thickness: f32) -> BTreeMap<String, SurfaceGeometry> {
    BTreeMap::from([
        (
            "a".into(),
            cube(
                Vec3::new(-1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, thickness),
                0,
                true,
            ),
        ),
        (
            "b".into(),
            cube(Vec3::ZERO, Vec3::new(1.0, 1.0, thickness), 0, false),
        ),
    ])
}

#[test]
fn baked_contact_keeps_the_real_rim_and_exposure_sides() {
    let patches = build_patches(&manifest(1.0), &adjacent(1.0)).unwrap();
    assert_eq!(patches.len(), 1);
    assert!((patches[0].area - 1.0).abs() < 1e-6);
    assert_eq!(patches[0].triangles.len(), 2);
    assert_eq!(patches[0].rims.len(), 4);
    assert!(patches[0].normal.dot(Vec3::X) > 0.999);
    assert!(patches[0].exposed_survivor(|_| true).is_none());
    assert_eq!(patches[0].exposed_survivor(|id| id == "a"), Some("a"));
    assert_eq!(patches[0].exposed_survivor(|id| id == "b"), Some("b"));
    assert!(patches[0].exposed_survivor(|_| false).is_none());
}

fn rectangular_patch(vertices: [[f32; 3]; 4]) -> crate::asset::BondPatchRecord {
    let points = vertices.map(Vec3::from_array);
    let first = points[1] - points[0];
    let second = points[3] - points[0];
    crate::asset::BondPatchRecord {
        area: first.cross(second).length(),
        center: ((points[0] + points[2]) * 0.5).to_array(),
        normal: first.cross(second).normalize().to_array(),
        area_covariance: std::array::from_fn(|row| {
            std::array::from_fn(|column| {
                first.cross(second).length() / 12.0
                    * (first[row] * first[column] + second[row] * second[column])
            })
        }),
        vertices: vertices.into(),
    }
}

#[test]
fn baked_coplanar_facets_merge_without_visual_internal_seams() {
    let mut manifest = manifest(1.0);
    manifest.schema_version = 2;
    manifest.bonds[0].patches = [0.0, 0.5]
        .into_iter()
        .map(|y| {
            rectangular_patch([
                [0.0, y, 0.0],
                [0.0, y + 0.5, 0.0],
                [0.0, y + 0.5, 1.0],
                [0.0, y, 1.0],
            ])
        })
        .collect();
    let patches = build_patches(&manifest, &adjacent(1.0)).unwrap();
    assert_eq!(patches.len(), 1);
    assert_eq!(patches[0].rims.len(), 4);
    assert!((patches[0].area - 1.0).abs() < 1e-6);
    let original = patches[0].triangles.clone();
    manifest.bonds[0].patches.reverse();
    assert_eq!(
        build_patches(&manifest, &adjacent(1.0)).unwrap()[0].triangles,
        original
    );
}

#[test]
fn baked_folded_interfaces_use_actual_faces_and_reject_fabricated_coverage() {
    let mut manifest = manifest(1.0);
    manifest.schema_version = 2;
    manifest.bonds[0].patches = vec![
        rectangular_patch([
            [0.0, 0.0, 0.0],
            [0.0, 0.5, 0.0],
            [0.0, 0.5, 1.0],
            [0.0, 0.0, 1.0],
        ]),
        rectangular_patch([
            [0.0, 0.5, 0.0],
            [0.1, 1.0, 0.0],
            [0.1, 1.0, 1.0],
            [0.0, 0.5, 1.0],
        ]),
    ];
    let mut triangles = Vec::new();
    for patch in &manifest.bonds[0].patches {
        let p = patch
            .vertices
            .iter()
            .copied()
            .map(Vec3::from_array)
            .collect::<Vec<_>>();
        triangles.extend([[p[0], p[1], p[2]], [p[0], p[2], p[3]]]);
    }
    let mut geometry = BTreeMap::from([
        (
            "a".into(),
            SurfaceGeometry {
                triangles: triangles.clone(),
                exterior: Vec::new(),
            },
        ),
        (
            "b".into(),
            SurfaceGeometry {
                triangles: triangles.iter().map(|t| [t[0], t[2], t[1]]).collect(),
                exterior: Vec::new(),
            },
        ),
    ]);
    let patches = build_patches(&manifest, &geometry).unwrap();
    assert_eq!(patches.len(), 2);
    assert!(patches[0].normal.dot(patches[1].normal) < 0.999);
    assert!(patches.iter().all(|patch| patch.rims.is_empty()));
    geometry.get_mut("b").unwrap().triangles.truncate(2);
    assert!(build_patches(&manifest, &geometry).is_err());
    geometry.get_mut("b").unwrap().triangles =
        vec![[triangles[0][0], triangles[0][2], triangles[0][1]]; 2];
    assert!(build_patches(&manifest, &geometry).is_err());
}

#[test]
fn a_baked_facet_below_area_tolerance_still_needs_actual_mesh_coverage() {
    let mut manifest = manifest(1.0);
    manifest.schema_version = 2;
    manifest.bonds[0].patches = vec![rectangular_patch([
        [0.0, 0.0, 0.0],
        [0.0, 1e-5, 0.0],
        [0.0, 1e-5, 1e-5],
        [0.0, 0.0, 1e-5],
    ])];
    let geometry = BTreeMap::from([
        ("a".into(), SurfaceGeometry::default()),
        ("b".into(), SurfaceGeometry::default()),
    ]);
    assert!(build_patches(&manifest, &geometry).is_err());
}

#[test]
fn baked_coplanar_merge_does_not_fill_a_resolvable_small_notch() {
    let mut manifest = manifest(1.0);
    manifest.schema_version = 2;
    manifest.bonds[0].patches = vec![
        rectangular_patch([
            [0.0, 0.0, 0.0],
            [0.0, 0.5, 0.0],
            [0.0, 0.5, 1.0],
            [0.0, 0.0, 1.0],
        ]),
        rectangular_patch([
            [0.0, 0.5, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.99999],
            [0.0, 0.5, 0.99999],
        ]),
    ];
    let triangles = manifest.bonds[0]
        .patches
        .iter()
        .flat_map(|patch| {
            let p = patch
                .vertices
                .iter()
                .copied()
                .map(Vec3::from_array)
                .collect::<Vec<_>>();
            [[p[0], p[1], p[2]], [p[0], p[2], p[3]]]
        })
        .collect::<Vec<_>>();
    let geometry = BTreeMap::from([
        (
            "a".into(),
            SurfaceGeometry {
                triangles: triangles.clone(),
                exterior: Vec::new(),
            },
        ),
        (
            "b".into(),
            SurfaceGeometry {
                triangles: triangles.iter().map(|p| [p[0], p[2], p[1]]).collect(),
                exterior: Vec::new(),
            },
        ),
    ]);
    let patches = build_patches(&manifest, &geometry).unwrap();
    assert_eq!(patches.len(), 2);
    let actual = patches.iter().map(|patch| patch.area).sum::<f32>();
    let expected = manifest.bonds[0]
        .patches
        .iter()
        .map(|patch| patch.area)
        .sum::<f32>();
    assert!((actual - expected).abs() < 1e-7);
}

#[test]
fn partial_contact_does_not_mark_uncovered_larger_face_edges() {
    let mut geometry = adjacent(1.0);
    geometry.insert(
        "b".into(),
        cube(
            Vec3::new(0.0, 0.25, 0.25),
            Vec3::new(1.0, 0.75, 0.75),
            0,
            false,
        ),
    );
    let mut manifest = manifest(0.25);
    manifest.bonds[0].patches = vec![rectangular_patch([
        [0.0, 0.25, 0.25],
        [0.0, 0.75, 0.25],
        [0.0, 0.75, 0.75],
        [0.0, 0.25, 0.75],
    ])];
    let patches = build_patches(&manifest, &geometry).unwrap();
    assert_eq!(patches.len(), 1);
    assert!((patches[0].area - 0.25).abs() < 1e-6);
    assert!(
        patches[0]
            .rims
            .iter()
            .all(|rim| rim.exterior_a.is_none() && rim.exterior_b.is_some())
    );
    assert!(detail_mesh(&patches[0], "a").is_none());
}

#[test]
fn disjoint_coplanar_polygons_keep_the_gap() {
    let first = vec![DVec2::ZERO, DVec2::X, DVec2::Y];
    let second: Vec<_> = first.iter().map(|p| *p + DVec2::new(3.0, 0.0)).collect();
    let components = connected_polygons(&[first, second]).unwrap();
    assert_eq!(components.len(), 2);
}

#[test]
fn hull_welds_close_vertices_even_when_lexical_sort_interleaves_another_corner() {
    let points = vec![
        DVec2::ZERO,
        DVec2::new(1.0, 0.0),
        DVec2::new(1.0 + 1e-12, 1.0),
        DVec2::new(1.0 + 2e-12, 1e-12),
        DVec2::Y,
    ];
    let hull = convex_hull(points);
    assert_eq!(hull.len(), 4);
    assert!((polygon_area(&hull) - 1.0).abs() < 1e-8);
}

#[test]
fn hull_keeps_extreme_corner_with_almost_equal_coordinates() {
    let hull = convex_hull(vec![
        DVec2::new(-0.22 - 1e-17, -4.06),
        DVec2::new(-0.22, -5.02),
        DVec2::new(-0.22, -4.64),
        DVec2::new(0.0, -5.02),
        DVec2::new(0.0, -4.06),
    ]);
    assert_eq!(hull.len(), 4);
    assert!((polygon_area(&hull) - 0.2112).abs() < 1e-8);
}

#[test]
fn lip_geometry_is_closed_deterministic_and_thin_wall_bounded() {
    let patches = build_patches(&manifest(0.02), &adjacent(0.02)).unwrap();
    let mesh = detail_mesh(&patches[0], "a").unwrap();
    let VertexAttributeValues::Float32x3(points) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
    else {
        panic!()
    };
    assert!(
        points
            .iter()
            .all(|p| p.iter().all(|v| v.is_finite()) && p[0] <= 0.0017)
    );
    let mut edges = BTreeMap::new();
    let key = |point: [f32; 3]| point.map(|p| (p * 1000000.0).round() as i32);
    for triangle in points.as_chunks::<3>().0 {
        let p = triangle.map(Vec3::from_array);
        assert!((p[1] - p[0]).cross(p[2] - p[0]).length_squared() > 1e-18);
        for (a, b) in [
            (triangle[0], triangle[1]),
            (triangle[1], triangle[2]),
            (triangle[2], triangle[0]),
        ] {
            let (a, b) = (key(a), key(b));
            *edges.entry((a.min(b), a.max(b))).or_insert(0) += 1;
        }
    }
    assert!(edges.values().all(|count| *count == 2));
    let again = detail_mesh(&patches[0], "a").unwrap();
    assert_eq!(
        mesh.attribute(Mesh::ATTRIBUTE_POSITION),
        again.attribute(Mesh::ATTRIBUTE_POSITION)
    );
    assert!(detail_mesh(&patches[0], "unrelated").is_none());
}

#[test]
fn oblique_lips_keep_embedded_roots_on_the_survivor_side() {
    for obliqueness in [-0.96_f32, 0.96] {
        let exterior = Vec3::new(obliqueness, 0.0, (1.0 - obliqueness.powi(2)).sqrt());
        let patch = ContactPatch {
            id: "oblique".into(),
            bond_id: "bond".into(),
            a: "a".into(),
            b: "b".into(),
            normal: Vec3::X,
            triangles: vec![],
            area: 1.0,
            rims: vec![RimEdge {
                points: [Vec3::ZERO, Vec3::Y],
                exterior_a: Some(exterior),
                exterior_b: None,
                thickness_a: 0.02,
                thickness_b: 0.02,
            }],
        };
        let mesh = detail_mesh(&patch, "a").unwrap();
        let VertexAttributeValues::Float32x3(points) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
        else {
            panic!()
        };
        let VertexAttributeValues::Float32x4(colors) =
            mesh.attribute(Mesh::ATTRIBUTE_COLOR).unwrap()
        else {
            panic!()
        };
        for (point, color) in points.iter().zip(colors) {
            if color[0] == 1.0 {
                assert!(point[0] < 0.0, "embedded root left the survivor");
            }
        }
    }
}

fn fixture_geometry(manifest: &Manifest, bytes: &[u8]) -> BTreeMap<String, SurfaceGeometry> {
    let json_length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let document: serde_json::Value = serde_json::from_slice(&bytes[20..20 + json_length]).unwrap();
    let binary = &bytes[28 + json_length..];
    let nodes: BTreeMap<_, _> = document["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| (node["name"].as_str().unwrap(), node))
        .collect();
    let accessor = |index: usize, element_size: usize| {
        let accessor = &document["accessors"][index];
        let view = &document["bufferViews"][accessor["bufferView"].as_u64().unwrap() as usize];
        let offset = accessor["byteOffset"].as_u64().unwrap_or(0) as usize
            + view["byteOffset"].as_u64().unwrap_or(0) as usize;
        (
            offset,
            view["byteStride"]
                .as_u64()
                .map_or(element_size, |value| value as usize),
            accessor["count"].as_u64().unwrap() as usize,
        )
    };
    let mut result = BTreeMap::new();
    for chunk in &manifest.chunks {
        let node = nodes[chunk.node.as_str()];
        let primitives = document["meshes"][node["mesh"].as_u64().unwrap() as usize]["primitives"]
            .as_array()
            .unwrap();
        let mut geometry = SurfaceGeometry::default();
        for primitive in primitives {
            let (offset, stride, count) = accessor(
                primitive["attributes"]["POSITION"].as_u64().unwrap() as usize,
                12,
            );
            let positions: Vec<_> = (0..count)
                .map(|index| {
                    let offset = offset + index * stride;
                    Vec3::from_array(std::array::from_fn(|axis| {
                        f32::from_le_bytes(
                            binary[offset + axis * 4..offset + axis * 4 + 4]
                                .try_into()
                                .unwrap(),
                        )
                    }))
                })
                .collect();
            let indices: Vec<usize> = if let Some(index) = primitive["indices"].as_u64() {
                let component = document["accessors"][index as usize]["componentType"]
                    .as_u64()
                    .unwrap();
                let size = match component {
                    5121 => 1,
                    5123 => 2,
                    5125 => 4,
                    _ => panic!("Unsupported fixture index"),
                };
                let (offset, stride, count) = accessor(index as usize, size);
                (0..count)
                    .map(|index| {
                        let offset = offset + index * stride;
                        match component {
                            5121 => usize::from(binary[offset]),
                            5123 => usize::from(u16::from_le_bytes(
                                binary[offset..offset + 2].try_into().unwrap(),
                            )),
                            _ => u32::from_le_bytes(binary[offset..offset + 4].try_into().unwrap())
                                as usize,
                        }
                    })
                    .collect()
            } else {
                (0..positions.len()).collect()
            };
            let exterior = document["materials"][primitive["material"].as_u64().unwrap() as usize]
                ["extras"]["bd_surface_role"]
                == "exterior";
            for indices in indices.as_chunks::<3>().0 {
                let triangle =
                    indices.map(|index| chunk.transform().transform_point(positions[index]));
                geometry.triangles.push(triangle);
                if exterior {
                    geometry.exterior.push(triangle);
                }
            }
        }
        result.insert(chunk.id.clone(), geometry);
    }
    result
}

#[test]
fn bundled_contact_caches_match_every_bond_and_cache_only_real_exterior_rims() {
    let directory = crate::asset_root().join("buildings");
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
        let bytes = std::fs::read(directory.join(&manifest.mesh_file)).unwrap();
        let geometry = fixture_geometry(&manifest, &bytes);
        let start = std::time::Instant::now();
        let patches =
            build_patches(&manifest, &geometry).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert!(
            patches.len()
                >= manifest
                    .bonds
                    .iter()
                    .filter(|bond| bond.area >= MIN_VISUAL_CONTACT_AREA)
                    .count()
        );
        let mut maximum_error = 0.0_f32;
        for bond in &manifest.bonds {
            let area: f32 = patches
                .iter()
                .filter(|patch| patch.bond_id == bond.id)
                .map(|patch| patch.area)
                .sum();
            maximum_error = maximum_error.max((area - bond.area).abs());
            assert!((area - bond.area).abs() <= (bond.area * 0.01).max(0.00002));
        }
        let mut triangles = 0;
        let excluded: Vec<_> = manifest
            .bonds
            .iter()
            .filter(|bond| bond.area < MIN_VISUAL_CONTACT_AREA)
            .collect();
        for patch in &patches {
            for survivor in [&patch.a, &patch.b] {
                if let Some(mesh) = detail_mesh(patch, survivor) {
                    let VertexAttributeValues::Float32x3(points) =
                        mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
                    else {
                        panic!()
                    };
                    assert!(points.iter().flatten().all(|value| value.is_finite()));
                    for triangle in points.as_chunks::<3>().0 {
                        let [a, b, c] = triangle.map(Vec3::from_array);
                        assert!((b - a).cross(c - a).length_squared() > 1e-18);
                    }
                    triangles += mesh.count_vertices() / 3;
                }
            }
        }
        println!(
            "{name}: {} patches, {} rims, all-side upper-bound {triangles} detail triangles, maximum area error {maximum_error:.8}m2, excluded {} visual-only slivers totaling {:.12}m2, cache+geometry {:.2}ms",
            patches.len(),
            patches.iter().map(|patch| patch.rims.len()).sum::<usize>(),
            excluded.len(),
            excluded.iter().map(|bond| bond.area as f64).sum::<f64>(),
            start.elapsed().as_secs_f64() * 1000.0
        );
    }
}
