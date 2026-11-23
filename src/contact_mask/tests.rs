use super::*;
use crate::material::surface_coordinates;
use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, MeshVertexAttribute},
    render::render_resource::VertexFormat,
};

fn source_quad() -> Mesh {
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [-1.0, -1.0, 0.0],
            [1.0, -1.0, 0.0],
            [1.0, 1.0, 0.0],
            [-1.0, 1.0, 0.0],
        ],
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; 4])
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.1, 0.2, 0.3, 1.0]; 4])
    .with_inserted_attribute(
        MeshVertexAttribute::new("SourceVertexId", 100_001, VertexFormat::Uint32),
        vec![11_u32, 12, 13, 14],
    )
    .with_inserted_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]))
}

fn patch(id: &str, polygon: &[Vec3], normal: Vec3) -> ContactPatch {
    ContactPatch {
        id: id.into(),
        bond_id: format!("bond-{id}"),
        a: "a".into(),
        b: "b".into(),
        normal,
        triangles: (1..polygon.len() - 1)
            .map(|i| [polygon[0], polygon[i], polygon[i + 1]])
            .collect(),
        area: 0.0,
        rims: Vec::new(),
    }
}

fn partial() -> ContactPatch {
    patch(
        "partial",
        &[
            Vec3::new(0.0, -0.8, 0.0),
            Vec3::new(0.8, -0.8, 0.0),
            Vec3::new(0.8, 0.8, 0.0),
            Vec3::new(0.0, 0.8, 0.0),
        ],
        Vec3::Z,
    )
}

fn headers(mesh: &Mesh) -> Vec<u32> {
    let VertexAttributeValues::Float32x2(uv) = mesh.attribute(Mesh::ATTRIBUTE_UV_1).unwrap() else {
        panic!()
    };
    let indices: Vec<_> = mesh
        .indices()
        .map_or_else(|| (0..uv.len()).collect(), |i| i.iter().collect());
    indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|triangle| {
            let value = uv[triangle[0]][1];
            assert!(triangle.iter().all(|&i| uv[i][1] == value));
            value.round() as u32
        })
        .collect()
}

fn contains(data: &[Vec4], header: u32, point: Vec3) -> bool {
    let header = header as usize;
    (0..data[header].x.round() as usize).any(|candidate| {
        let offset = data[header + 1 + candidate].x.round() as usize;
        let origin = data[offset].truncate();
        (0..data[offset].w.round() as usize).all(|edge| {
            let halfspace = data[offset + 1 + edge];
            halfspace.truncate().dot(point - origin) + halfspace.w >= -5e-7
        })
    })
}

#[test]
fn partial_contact_preserves_source_attributes_and_deduplicates_gpu_lists() {
    let source = surface_coordinates(source_quad(), Transform::IDENTITY);
    let mut builder = ContactMaskBuilder::default();
    let patches = [partial()];
    let masked = builder.mask_mesh(source.clone(), &patches, "a").unwrap();
    assert!(masked.indices().is_none());
    assert_eq!(masked.count_vertices(), 6);
    let mut expected = source.clone();
    expected.duplicate_vertices();
    for (attribute, values) in expected.attributes() {
        if attribute.id != Mesh::ATTRIBUTE_UV_1.id {
            assert_eq!(
                masked.attribute(attribute.id).unwrap().get_bytes(),
                values.get_bytes()
            );
        }
    }
    let VertexAttributeValues::Float32x2(old_uv) =
        expected.attribute(Mesh::ATTRIBUTE_UV_1).unwrap()
    else {
        panic!()
    };
    let VertexAttributeValues::Float32x2(new_uv) = masked.attribute(Mesh::ATTRIBUTE_UV_1).unwrap()
    else {
        panic!()
    };
    assert!(old_uv.iter().zip(new_uv).all(|(a, b)| a[0] == b[0]));
    let h = headers(&masked);
    assert_eq!(h[0], h[1]);
    assert_ne!(h[0], 0);
    assert!(contains(builder.data(), h[0], Vec3::new(0.4, 0.0, 0.0)));
    assert!(!contains(builder.data(), h[0], Vec3::new(-0.4, 0.0, 0.0)));
    assert!(!contains(builder.data(), h[0], Vec3::new(0.9, 0.0, 0.0)));
    let rows = builder.data().to_vec();
    let second = builder.mask_mesh(source, &patches, "a").unwrap();
    assert_eq!(headers(&second), h);
    assert_eq!(builder.data(), rows);
    assert_eq!(builder.data()[0], Vec4::ZERO);
}

#[test]
fn rest_rotation_and_b_side_use_the_same_finite_contact_records() {
    let rest = Transform::from_xyz(23.37, 8.13, -4.4)
        .with_rotation(Quat::from_rotation_y(0.73) * Quat::from_rotation_x(0.21));
    let mut p = partial();
    p.triangles = p
        .triangles
        .iter()
        .map(|t| t.map(|v| rest.transform_point(v)))
        .collect();
    p.normal = rest.rotation * Vec3::Z;
    let patches = [p];
    let source = surface_coordinates(source_quad(), rest);
    let mut builder = ContactMaskBuilder::default();
    let masked = builder.mask_mesh(source.clone(), &patches, "a").unwrap();
    let h = headers(&masked)[0];
    assert!(contains(
        builder.data(),
        h,
        rest.transform_point(Vec3::new(0.4, 0.0, 0.0))
    ));
    assert!(!contains(
        builder.data(),
        h,
        rest.transform_point(Vec3::new(-0.4, 0.0, 0.0))
    ));
    let mut other = source;
    other.insert_indices(Indices::U32(vec![2, 1, 0, 3, 2, 0]));
    let rows = builder.data().len();
    let backside = builder.mask_mesh(other.clone(), &patches, "b").unwrap();
    assert_eq!(headers(&backside), headers(&masked));
    assert_eq!(builder.data().len(), rows);
    let wrong_side = builder.mask_mesh(other, &patches, "a").unwrap();
    assert_eq!(headers(&wrong_side), [0, 0]);
    assert_eq!(wrong_side.count_vertices(), 4);
    assert!(wrong_side.indices().is_some());
}

#[test]
fn disconnected_and_holed_triangle_unions_never_become_a_filled_convex_hull() {
    let outer = [
        Vec3::new(-0.9, -0.9, 0.0),
        Vec3::new(0.9, -0.9, 0.0),
        Vec3::new(0.9, 0.9, 0.0),
        Vec3::new(-0.9, 0.9, 0.0),
    ];
    let inner = outer.map(|p| p * 0.4);
    let mut ring = patch("ring", &outer, Vec3::Z);
    ring.triangles.clear();
    for i in 0..4 {
        let j = (i + 1) % 4;
        ring.triangles.push([outer[i], outer[j], inner[j]]);
        ring.triangles.push([outer[i], inner[j], inner[i]]);
    }
    let mut builder = ContactMaskBuilder::default();
    let mesh = builder
        .mask_mesh(
            surface_coordinates(source_quad(), Transform::IDENTITY),
            &[ring],
            "a",
        )
        .unwrap();
    let h = headers(&mesh);
    assert!(!contains(builder.data(), h[0], Vec3::ZERO));
    assert!(!contains(builder.data(), h[1], Vec3::ZERO));
    assert!(contains(builder.data(), h[0], Vec3::new(0.7, 0.0, 0.0)));
    assert!(contains(builder.data(), h[1], Vec3::new(-0.7, 0.0, 0.0)));
    assert!(!contains(builder.data(), h[0], Vec3::new(0.95, 0.0, 0.0)));
}

#[test]
fn local_overlap_accepts_rounded_source_faces_but_rejects_separated_planes() {
    let normal = Vec3::new(1.0, 0.001, 0.0).normalize();
    let polygon = [
        (-0.00004, -0.0045),
        (0.00004, -0.0045),
        (0.00004, 0.0045),
        (-0.00004, 0.0045),
    ]
    .map(|(y, z)| Vec3::new(-0.001 * y, y, z));
    let patches = [patch("rounded", &polygon, normal)];
    let triangle = [
        Vec3::new(0.0, -1.0, -1.0),
        Vec3::new(0.0, 1.0, -1.0),
        Vec3::new(0.0, 0.0, 1.0),
    ];
    assert!(
        triangle
            .iter()
            .any(|p| normal.dot(*p).abs() > PLANE_TOLERANCE as f32)
    );
    let mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        triangle.map(|point| point.to_array()).to_vec(),
    );
    let mut builder = ContactMaskBuilder::default();
    let masked = builder
        .mask_mesh(
            surface_coordinates(mesh.clone(), Transform::IDENTITY),
            &patches,
            "a",
        )
        .unwrap();
    assert_ne!(headers(&masked)[0], 0);
    assert!(contains(builder.data(), headers(&masked)[0], Vec3::ZERO));
    let shifted = builder
        .mask_mesh(
            surface_coordinates(mesh, Transform::from_xyz(0.0001, 0.0, 0.0)),
            &patches,
            "a",
        )
        .unwrap();
    assert_eq!(headers(&shifted), [0]);
}

#[test]
fn no_finite_contact_keeps_original_indices_and_allocates_only_the_sentinel() {
    let source = surface_coordinates(source_quad(), Transform::IDENTITY);
    let mut far = partial();
    far.triangles = far
        .triangles
        .iter()
        .map(|t| t.map(|p| p + Vec3::X * 10.0))
        .collect();
    let mut builder = ContactMaskBuilder::default();
    let mesh = builder.mask_mesh(source.clone(), &[far], "a").unwrap();
    assert_eq!(mesh.count_vertices(), 4);
    assert_eq!(headers(&mesh), [0, 0]);
    assert_eq!(
        mesh.indices().unwrap().iter().collect::<Vec<_>>(),
        source.indices().unwrap().iter().collect::<Vec<_>>()
    );
    assert_eq!(builder.data(), [Vec4::ZERO]);
    for (attribute, values) in source.attributes() {
        assert_eq!(
            mesh.attribute(attribute.id).unwrap().get_bytes(),
            values.get_bytes()
        );
    }
}

#[test]
fn actual_glb_rounding_fixture_maps_the_contact_on_the_source_surface() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../tools/blender/tests/fixtures/glb-contact-rounding.json"
    ))
    .unwrap();
    let mut vertices: Vec<Vec3> =
        serde_json::from_value::<Vec<[f32; 3]>>(fixture["patch"]["vertices"].clone())
            .unwrap()
            .into_iter()
            .map(Vec3::from_array)
            .collect();
    // The production footprint correction moves only this corner by one ULP.
    vertices[2].y = f32::from_bits(vertices[2].y.to_bits() - 1);
    let normal =
        Vec3::from_array(serde_json::from_value(fixture["patch"]["normal"].clone()).unwrap());
    let source: Vec<[f32; 3]> = serde_json::from_value::<Vec<Vec<[f32; 3]>>>(fixture["a"].clone())
        .unwrap()
        .into_iter()
        .flatten()
        .collect();
    let mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, source.clone())
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        vec![normal.to_array(); source.len()],
    );
    let mut builder = ContactMaskBuilder::default();
    let masked = builder
        .mask_mesh(
            surface_coordinates(mesh, Transform::IDENTITY),
            &[patch("actual-rounded", &vertices, normal)],
            "a",
        )
        .unwrap();
    let header = headers(&masked)[0];
    assert_ne!(header, 0);
    assert!(contains(
        builder.data(),
        header,
        vertices.iter().copied().sum::<Vec3>() / 3.0
    ));
    assert!(!contains(
        builder.data(),
        header,
        Vec3::new(4.78, 8.2, -3.78)
    ));
    let VertexAttributeValues::Float32x3(output) =
        masked.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
    else {
        panic!()
    };
    assert_eq!(output, &source);
}

#[test]
fn bundled_assets_keep_source_geometry_and_report_shared_buffer_costs() {
    use crate::{
        asset::Manifest,
        fracture_detail::{SurfaceGeometry, build_patches},
    };
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
        let json_length = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let document: serde_json::Value =
            serde_json::from_slice(&bytes[20..20 + json_length]).unwrap();
        let binary = &bytes[28 + json_length..];
        let vec3_accessor = |index: usize| {
            let accessor = &document["accessors"][index];
            let view = &document["bufferViews"][accessor["bufferView"].as_u64().unwrap() as usize];
            let base = view["byteOffset"].as_u64().unwrap_or(0) as usize
                + accessor["byteOffset"].as_u64().unwrap_or(0) as usize;
            let stride = view["byteStride"].as_u64().unwrap_or(12) as usize;
            (0..accessor["count"].as_u64().unwrap() as usize)
                .map(|i| {
                    std::array::from_fn(|axis| {
                        f32::from_le_bytes(
                            binary[base + i * stride + axis * 4..base + i * stride + axis * 4 + 4]
                                .try_into()
                                .unwrap(),
                        )
                    })
                })
                .collect::<Vec<[f32; 3]>>()
        };
        let mut geometry = BTreeMap::<String, SurfaceGeometry>::new();
        let mut primitives = Vec::new();
        for node in document["nodes"].as_array().unwrap() {
            let id = node["name"].as_str().unwrap();
            let chunk = manifest.chunks.iter().find(|c| c.id == id).unwrap();
            let surface = geometry.entry(id.into()).or_default();
            for primitive in
                document["meshes"][node["mesh"].as_u64().unwrap() as usize]["primitives"]
                    .as_array()
                    .unwrap()
            {
                let positions =
                    vec3_accessor(primitive["attributes"]["POSITION"].as_u64().unwrap() as usize);
                let normals =
                    vec3_accessor(primitive["attributes"]["NORMAL"].as_u64().unwrap() as usize);
                let accessor =
                    &document["accessors"][primitive["indices"].as_u64().unwrap() as usize];
                let view =
                    &document["bufferViews"][accessor["bufferView"].as_u64().unwrap() as usize];
                let base = view["byteOffset"].as_u64().unwrap_or(0) as usize
                    + accessor["byteOffset"].as_u64().unwrap_or(0) as usize;
                let component = accessor["componentType"].as_u64().unwrap();
                let width = match component {
                    5121 => 1,
                    5123 => 2,
                    5125 => 4,
                    _ => panic!("Invalid fixture indices"),
                };
                let stride = view["byteStride"].as_u64().unwrap_or(width as u64) as usize;
                let indices: Vec<u32> = (0..accessor["count"].as_u64().unwrap() as usize)
                    .map(|i| {
                        let offset = base + i * stride;
                        match component {
                            5121 => u32::from(binary[offset]),
                            5123 => u32::from(u16::from_le_bytes(
                                binary[offset..offset + 2].try_into().unwrap(),
                            )),
                            _ => u32::from_le_bytes(binary[offset..offset + 4].try_into().unwrap()),
                        }
                    })
                    .collect();
                let fracture = document["materials"]
                    [primitive["material"].as_u64().unwrap() as usize]["extras"]["bd_surface_role"]
                    == "fracture";
                for triangle in indices.as_chunks::<3>().0 {
                    let triangle = triangle.map(|i| {
                        chunk
                            .transform()
                            .transform_point(Vec3::from_array(positions[i as usize]))
                    });
                    surface.triangles.push(triangle);
                    if !fracture {
                        surface.exterior.push(triangle);
                    }
                }
                let mesh = Mesh::new(
                    PrimitiveTopology::TriangleList,
                    RenderAssetUsages::default(),
                )
                .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
                .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
                .with_inserted_indices(Indices::U32(indices));
                primitives.push((id.to_owned(), chunk.transform(), mesh, fracture));
            }
        }
        let patches = build_patches(&manifest, &geometry).unwrap();
        let mut builder = ContactMaskBuilder::default();
        let (mut additional_vertices, mut triangles, mut candidates, mut maximum) = (0, 0, 0, 0);
        for (id, transform, mesh, fracture) in primitives {
            if fracture {
                continue;
            }
            let source = surface_coordinates(mesh, transform);
            let original_vertices = source.count_vertices();
            let mut expected = source.clone();
            expected.duplicate_vertices();
            let masked = builder.mask_mesh(source, &patches, &id).unwrap();
            let list = headers(&masked);
            for header in &list {
                assert!((*header as usize) < builder.data().len());
                let count = builder.data()[*header as usize].x.round() as usize;
                assert!(*header as usize + count < builder.data().len());
                candidates += count;
                maximum = maximum.max(count);
            }
            triangles += list.len();
            additional_vertices += masked.count_vertices() - original_vertices;
            let mut actual = masked;
            actual.duplicate_vertices();
            for attribute in [
                Mesh::ATTRIBUTE_POSITION,
                Mesh::ATTRIBUTE_NORMAL,
                Mesh::ATTRIBUTE_UV_0,
            ] {
                assert_eq!(
                    actual.attribute(attribute).unwrap().get_bytes(),
                    expected.attribute(attribute).unwrap().get_bytes()
                );
            }
        }
        assert!(builder.data().len() > 1);
        assert!(builder.data().len() < MAX_DATA_ROWS);
        assert!(builder.data().iter().all(|row| row.is_finite()));
        println!(
            "CONTACT_MASK_COST {name}: rows={} bytes={} additional_vertices={additional_vertices} vertex_bytes={} exterior_triangles={triangles} mean_candidates={:.3} max_candidates={maximum}",
            builder.data().len(),
            builder.data().len() * 16,
            additional_vertices * 40,
            candidates as f64 / triangles as f64
        );
    }
}
