use super::*;
use crate::{asset::ChunkRecord, fracture_detail::RimEdge};
use bevy::mesh::VertexAttributeValues;

#[test]
fn catalog_convex_proxies_build_runtime_colliders() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
    for (name, path) in crate::catalog::SCENES {
        let manifest: crate::asset::Manifest =
            serde_json::from_slice(&std::fs::read(root.join(path)).unwrap()).unwrap();
        for record in &manifest.chunks {
            let collider = chunk_collider(record).unwrap_or_else(|error| panic!("{name}: {error}"));
            let shapes = collider.shape().as_compound().unwrap().shapes();
            assert_eq!(
                shapes.len(),
                record.colliders.len(),
                "{name}: {}",
                record.id
            );
            if record.id == "floor-02:natural:19:0007" && name == "l_shape" {
                let hull = shapes[15].1.as_convex_polyhedron().unwrap();
                let points = &record.colliders[15].vertices;
                assert_eq!(hull.points().len(), points.len());
                assert!(
                    points
                        .iter()
                        .all(|&v| hull.points().contains(&Vec3::from(v)))
                );
            }
        }
    }
}

fn mask_record() -> ChunkRecord {
    ChunkRecord {
        id: "a".into(),
        part_id: "wall".into(),
        kind: "wall".into(),
        material: "concrete".into(),
        node: "a".into(),
        position: [2.0, 3.0, -1.0],
        rotation: Quat::from_rotation_y(0.7).to_array(),
        volume: 1.0,
        mass: 1.0,
        center_of_mass: [0.0; 3],
        inertia: Mat3::IDENTITY.to_cols_array_2d(),
        colliders: Vec::new(),
    }
}

fn mask_asset(value: u128) -> AssetId<DestructibleAsset> {
    AssetId::Uuid {
        uuid: bevy::asset::uuid::Uuid::from_u128(value),
    }
}

#[test]
fn source_masks_share_asset_buffers_and_keep_exact_source_geometry() {
    let patches = shared_patches();
    let record = mask_record();
    let mut cache = GeometryCache::default();
    let mut meshes = Assets::<Mesh>::default();
    let mut buffers = Assets::<ShaderBuffer>::default();
    let local = Transform::from_matrix(record.transform().to_matrix().inverse());
    let source = meshes.add(
        crate::fracture_detail::contact_surface_mesh(&patches, "a")
            .unwrap()
            .transformed_by(local),
    );
    let original = mesh_triangles(meshes.get(&source).unwrap()).unwrap();
    let sources = || [(0, source.clone(), record.transform(), record.id.clone())];
    let prepared = cache
        .prepare_surface_masks(
            mask_asset(1),
            sources(),
            &patches,
            &mut meshes,
            &mut buffers,
        )
        .unwrap();
    let first = (
        prepared.meshes[&(0, source.id())].clone(),
        prepared.buffer.clone(),
    );
    let prepared = cache
        .prepare_surface_masks(
            mask_asset(1),
            sources(),
            &patches,
            &mut meshes,
            &mut buffers,
        )
        .unwrap();
    assert_eq!(first.0, prepared.meshes[&(0, source.id())]);
    assert_eq!(first.1, prepared.buffer);
    let prepared = cache
        .prepare_surface_masks(
            mask_asset(2),
            sources(),
            &patches,
            &mut meshes,
            &mut buffers,
        )
        .unwrap();
    assert_ne!(first.0, prepared.meshes[&(0, source.id())]);
    assert_ne!(first.1, prepared.buffer);
    assert_eq!((meshes.len(), buffers.len()), (3, 2));
    let mesh = meshes.get(&first.0).unwrap();
    assert_eq!(mesh_triangles(mesh).unwrap(), original);
    let VertexAttributeValues::Float32x2(z) = mesh.attribute(Mesh::ATTRIBUTE_UV_1).unwrap() else {
        panic!()
    };
    assert!(z.iter().all(|uv| uv[1] > 0.0));
    for triangle in z.as_chunks::<3>().0 {
        assert_eq!(triangle[0][1], triangle[1][1]);
        assert_eq!(triangle[0][1], triangle[2][1]);
    }
}

#[test]
fn contact_geometry_cache_evicts_only_removed_or_modified_asset_domains() {
    let mut app = App::new();
    app.init_resource::<GeometryCache>()
        .add_message::<AssetEvent<DestructibleAsset>>()
        .add_systems(Update, release_geometry_cache);
    {
        let mut cache = app.world_mut().resource_mut::<GeometryCache>();
        for asset in [mask_asset(1), mask_asset(2)] {
            cache.patches.insert(asset, shared_patches());
            cache.surface_masks.insert(
                asset,
                SurfaceMasks {
                    meshes: BTreeMap::new(),
                    buffer: Handle::default(),
                },
            );
        }
    }
    app.world_mut()
        .write_message(AssetEvent::Removed { id: mask_asset(1) });
    app.update();
    let cache = app.world().resource::<GeometryCache>();
    assert!(!cache.patches.contains_key(&mask_asset(1)));
    assert!(!cache.surface_masks.contains_key(&mask_asset(1)));
    assert!(cache.surface_masks.contains_key(&mask_asset(2)));
    app.world_mut()
        .write_message(AssetEvent::Modified { id: mask_asset(2) });
    app.update();
    let cache = app.world().resource::<GeometryCache>();
    assert!(cache.patches.is_empty() && cache.surface_masks.is_empty());
}

fn detail_app() -> App {
    let mut app = App::new();
    app.init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<BuildingMaterial>>()
        .init_resource::<RimMaterials>()
        .init_resource::<DetailMeshCache>()
        .init_resource::<DetailStats>()
        .insert_resource(DemoOptions::default())
        .add_systems(Update, update_details);
    app
}

fn shared_patches() -> Arc<Vec<ContactPatch>> {
    Arc::new(
        [("ab", "b", 0.0), ("ac", "c", -1.0)]
            .map(|(id, neighbor, x)| {
                let start = Vec3::new(x, 0.0, 0.0);
                ContactPatch {
                    id: id.into(),
                    bond_id: id.into(),
                    a: "a".into(),
                    b: neighbor.into(),
                    normal: Vec3::X,
                    triangles: vec![[start, start + Vec3::NEG_Z, start + Vec3::Y]],
                    area: 0.5,
                    rims: vec![RimEdge {
                        points: [start, start + Vec3::Y],
                        exterior_a: Some(Vec3::Z),
                        exterior_b: Some(Vec3::Z),
                        thickness_a: 0.2,
                        thickness_b: 0.2,
                    }],
                }
            })
            .into(),
    )
}

fn spawn_instance(world: &mut World, patches: Arc<Vec<ContactPatch>>) -> Entity {
    let root = world.spawn((Transform::IDENTITY, Visibility::Visible)).id();
    let mut instance = BuildingInstance::pending(Handle::default());
    instance.patches = patches;
    for id in ["a", "b", "c"] {
        let entity = world
            .spawn((Transform::IDENTITY, Visibility::Visible, ChildOf(root)))
            .id();
        instance.chunks.insert(
            id.into(),
            ChunkInstance {
                entity,
                primitives: Vec::new(),
                alive: true,
                triangles: Vec::new(),
                transform: Transform::IDENTITY,
                material: "concrete".into(),
                detail: None,
                cluster: None,
            },
        );
    }
    instance.detail_dirty.insert("a".into());
    world.entity_mut(root).insert(instance);
    root
}

fn remove_chunk(world: &mut World, root: Entity, id: &str) {
    let mut instance = world.get_mut::<BuildingInstance>(root).unwrap();
    instance.chunks.get_mut(id).unwrap().alive = false;
    instance.detail_dirty.insert(id.into());
}

fn detail_snapshot(world: &World, root: Entity) -> Vec<(String, Vec<String>, Vec<[f32; 3]>)> {
    let instance = world.get::<BuildingInstance>(root).unwrap();
    instance
        .chunks
        .iter()
        .filter_map(|(id, chunk)| {
            let entity = chunk.detail?;
            assert_eq!(world.get::<ChildOf>(entity).unwrap().parent(), chunk.entity);
            let detail = world.get::<DamageDetail>(entity).unwrap();
            let mesh = world
                .resource::<Assets<Mesh>>()
                .get(&world.get::<Mesh3d>(entity).unwrap().0)
                .unwrap();
            let Some(VertexAttributeValues::Float32x3(positions)) =
                mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            else {
                panic!("Rim mesh has no positions");
            };
            Some((id.clone(), detail.patch_ids.clone(), positions.clone()))
        })
        .collect()
}

#[test]
fn shared_contact_cache_keeps_damage_local_and_reuses_unchanged_mesh() {
    let mut app = detail_app();
    let patches = shared_patches();
    let intact = spawn_instance(app.world_mut(), patches.clone());
    let damaged = spawn_instance(app.world_mut(), patches);
    app.update();
    assert!(detail_snapshot(app.world(), intact).is_empty());
    assert!(detail_snapshot(app.world(), damaged).is_empty());

    remove_chunk(app.world_mut(), damaged, "b");
    app.update();
    assert!(detail_snapshot(app.world(), intact).is_empty());
    let snapshot = detail_snapshot(app.world(), damaged);
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].0, "a");
    assert_eq!(snapshot[0].1, ["ab"]);
    assert!(!snapshot[0].2.is_empty());
    assert!(snapshot[0].2.iter().flatten().all(|p| p.is_finite()));
    let detail = app.world().get::<BuildingInstance>(damaged).unwrap().chunks["a"]
        .detail
        .unwrap();
    let mesh = app.world().get::<Mesh3d>(detail).unwrap().0.clone();
    let mesh_count = app.world().resource::<Assets<Mesh>>().len();
    app.world_mut()
        .get_mut::<BuildingInstance>(damaged)
        .unwrap()
        .detail_dirty
        .insert("b".into());
    app.update();
    assert_eq!(app.world().get::<Mesh3d>(detail).unwrap().0, mesh);
    assert_eq!(app.world().resource::<Assets<Mesh>>().len(), mesh_count);
    let stats = app.world().resource::<DetailStats>();
    assert_eq!(stats.active_meshes, 1);
    assert_eq!(stats.cached_meshes, 1);
    assert_eq!(stats.triangles, stats.cached_triangles);
}

#[test]
fn severed_bond_exposes_both_live_sides_only_in_the_damaged_instance() {
    let mut app = detail_app();
    let patches = shared_patches();
    let intact = spawn_instance(app.world_mut(), patches.clone());
    let detached = spawn_instance(app.world_mut(), patches);
    app.world_mut()
        .get_mut::<BuildingInstance>(detached)
        .unwrap()
        .bonds
        .insert("ab".into(), false);
    app.update();
    assert!(detail_snapshot(app.world(), intact).is_empty());
    let snapshot = detail_snapshot(app.world(), detached);
    assert_eq!(snapshot.len(), 2);
    assert_eq!(snapshot[0].0, "a");
    assert_eq!(snapshot[1].0, "b");
    for (_, patch_ids, positions) in snapshot {
        assert_eq!(patch_ids, ["ab"]);
        assert!(!positions.is_empty());
        assert!(positions.iter().flatten().all(|value| value.is_finite()));
    }
}

#[test]
fn removal_order_preserves_geometry_and_removing_survivor_cleans_detail() {
    let mut app = detail_app();
    let patches = shared_patches();
    let first = spawn_instance(app.world_mut(), patches.clone());
    let second = spawn_instance(app.world_mut(), patches);
    for (root, id) in [(first, "b"), (first, "c"), (second, "c"), (second, "b")] {
        remove_chunk(app.world_mut(), root, id);
        app.update();
    }
    let expected = detail_snapshot(app.world(), second);
    assert_eq!(expected.len(), 1);
    assert_eq!(expected[0].1, ["ab", "ac"]);
    assert_eq!(detail_snapshot(app.world(), first), expected);
    assert_eq!(app.world().resource::<DetailStats>().cached_meshes, 2);

    let old_detail = app.world().get::<BuildingInstance>(first).unwrap().chunks["a"]
        .detail
        .unwrap();
    remove_chunk(app.world_mut(), first, "a");
    app.update();
    assert!(detail_snapshot(app.world(), first).is_empty());
    assert!(app.world().get_entity(old_detail).is_err());
    assert_eq!(detail_snapshot(app.world(), second), expected);
    assert_eq!(app.world().resource::<DetailStats>().active_meshes, 1);
}
