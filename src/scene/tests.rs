use super::*;
use crate::{asset::ChunkRecord, fracture_detail::RimEdge};
use bevy::{ecs::relationship::RelationshipTarget, mesh::VertexAttributeValues};

#[test]
fn catalog_convex_proxies_build_runtime_colliders() {
    let root = crate::asset_root();
    for (name, path) in crate::catalog::SCENES {
        let manifest: crate::asset::Manifest =
            serde_json::from_slice(&std::fs::read(root.join(path)).unwrap()).unwrap();
        for record in &manifest.chunks {
            let collider = chunk_collider(record).unwrap_or_else(|error| panic!("{name}: {error}"));
            let density = manifest.materials[&record.material].density;
            chunk_mass_properties(&record.id, &collider, density)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
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

#[test]
fn invalid_mass_properties_identify_the_chunk() {
    let collider = Collider::cuboid(1.0, 1.0, 1.0);
    let error = chunk_mass_properties("wall-brick-6", &collider, 0.0).unwrap_err();
    assert!(error.contains("wall-brick-6"));
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
        .insert_resource(DemoSettings::default())
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
    for id in ["a", "b", "c"] {
        let entity = world
            .spawn((
                Transform::IDENTITY,
                Visibility::Visible,
                ChildOf(root),
                ChunkInstance {
                    alive: true,
                    triangles: Vec::new(),
                    transform: Transform::IDENTITY,
                    mass_properties: MassProperties3d::ZERO,
                    material: "concrete".into(),
                },
            ))
            .id();
        instance.chunks.insert(id.into(), entity);
    }
    let template = crate::test_fixtures::beam_manifest().bonds.remove(0);
    let bonds = patches
        .iter()
        .map(|patch| BondRecord {
            id: patch.bond_id.clone(),
            a: patch.a.clone(),
            b: patch.b.clone(),
            ..template.clone()
        })
        .collect::<Vec<_>>();
    let mut commands = world.commands();
    spawn_bonds(&mut commands, root, &mut instance, &bonds);
    spawn_patches(&mut commands, root, &mut instance, patches);
    world.flush();
    world
        .entity_mut(root)
        .insert((instance, RefreshDamageDetails));
    root
}

fn remove_chunk(world: &mut World, root: Entity, id: &str) {
    let entity = world.get::<BuildingInstance>(root).unwrap().chunks[id];
    world.get_mut::<ChunkInstance>(entity).unwrap().alive = false;
    world.entity_mut(root).insert(RefreshDamageDetails);
}

fn detail_entity(world: &World, chunk: Entity) -> Option<Entity> {
    world
        .get::<Children>(chunk)?
        .iter()
        .find(|&entity| world.get::<DamageDetail>(entity).is_some())
}

fn detail_snapshot(world: &World, root: Entity) -> Vec<(String, Vec<String>, Vec<[f32; 3]>)> {
    let instance = world.get::<BuildingInstance>(root).unwrap();
    instance
        .chunks
        .iter()
        .filter_map(|(id, &chunk_entity)| {
            let entity = detail_entity(world, chunk_entity)?;
            assert_eq!(world.get::<ChildOf>(entity).unwrap().parent(), chunk_entity);
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

    let building_tick = |world: &World, entity: Entity| {
        world
            .entity(entity)
            .get_ref::<BuildingInstance>()
            .unwrap()
            .last_changed()
    };
    let chunk = app.world().get::<BuildingInstance>(damaged).unwrap().chunks["a"];
    let chunk_tick = |world: &World| {
        world
            .entity(chunk)
            .get_ref::<ChunkInstance>()
            .unwrap()
            .last_changed()
    };
    let root_ticks = [intact, damaged].map(|entity| building_tick(app.world(), entity));
    let surviving_chunk_tick = chunk_tick(app.world());
    remove_chunk(app.world_mut(), damaged, "b");
    app.update();
    assert_eq!(
        [intact, damaged].map(|entity| building_tick(app.world(), entity)),
        root_ticks
    );
    assert_eq!(chunk_tick(app.world()), surviving_chunk_tick);
    assert!(detail_snapshot(app.world(), intact).is_empty());
    let snapshot = detail_snapshot(app.world(), damaged);
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].0, "a");
    assert_eq!(snapshot[0].1, ["ab"]);
    assert!(!snapshot[0].2.is_empty());
    assert!(snapshot[0].2.iter().flatten().all(|p| p.is_finite()));
    let detail = detail_entity(app.world(), chunk).unwrap();
    let mesh = app.world().get::<Mesh3d>(detail).unwrap().0.clone();
    let mesh_count = app.world().resource::<Assets<Mesh>>().len();
    app.world_mut()
        .entity_mut(damaged)
        .insert(RefreshDamageDetails);
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
    let bond = app.world().get::<BuildingInstance>(detached).unwrap().bonds["ab"];
    app.world_mut()
        .get_mut::<BondInstance>(bond)
        .unwrap()
        .active = false;
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

    let chunk = app.world().get::<BuildingInstance>(first).unwrap().chunks["a"];
    let old_detail = detail_entity(app.world(), chunk).unwrap();
    remove_chunk(app.world_mut(), first, "a");
    app.update();
    assert!(detail_snapshot(app.world(), first).is_empty());
    assert!(app.world().get_entity(old_detail).is_err());
    assert_eq!(detail_snapshot(app.world(), second), expected);
    assert_eq!(app.world().resource::<DetailStats>().active_meshes, 1);
}

#[test]
fn despawning_one_building_releases_its_graph_entities_and_keeps_shared_geometry() {
    let mut world = World::new();
    let geometry = shared_patches();
    let removed = spawn_instance(&mut world, geometry.clone());
    let retained = spawn_instance(&mut world, geometry);
    let first = world.get::<BuildingInstance>(removed).unwrap();
    let other = world.get::<BuildingInstance>(retained).unwrap();
    let removed_entities = [first.chunks["a"], first.bonds["ab"], first.patches[0]];
    let retained_entities = [other.chunks["a"], other.bonds["ab"], other.patches[0]];
    let [chunk, bond, patch] = removed_entities;
    let bond_state = world.get::<BondInstance>(bond).unwrap();
    assert_eq!(bond_state.endpoints, [chunk, first.chunks["b"]]);
    assert_eq!(world.get::<BondOfBuilding>(bond).unwrap().building, removed);
    assert!(
        world
            .get::<BuildingBonds>(removed)
            .unwrap()
            .iter()
            .any(|entity| entity == bond)
    );
    assert!(world.get::<ChildOf>(bond).is_none());
    assert_eq!(world.get::<PatchOfBond>(patch).unwrap().bond, bond);
    assert!(world.get::<ChildOf>(patch).is_none());
    let related_patches = world.get::<BondPatches>(bond).unwrap();
    assert!(related_patches.iter().any(|entity| entity == patch));
    let facet = related_patches
        .iter()
        .find(|&entity| world.get::<StructuralPatch>(entity).is_some())
        .unwrap();
    assert_eq!(world.get::<StructuralPatch>(facet).unwrap().index, 0);
    assert_eq!(world.get::<PatchOfBond>(facet).unwrap().bond, bond);
    assert!(std::ptr::eq(
        world.get::<PatchInstance>(patch).unwrap().patch(),
        world
            .get::<PatchInstance>(retained_entities[2])
            .unwrap()
            .patch(),
    ));

    world.despawn(removed);
    for entity in removed_entities.into_iter().chain([facet]) {
        assert!(world.get_entity(entity).is_err());
    }
    for entity in retained_entities {
        assert!(world.get_entity(entity).is_ok());
    }
}
