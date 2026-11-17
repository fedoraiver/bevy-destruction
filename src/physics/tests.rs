use super::*;
use crate::{
    asset::{AnchorRecord, BondRecord, ColliderRecord, Manifest},
    scene::{BuildingBounds, ChunkIdentity, ChunkInstance},
};
use bevy::time::TimeUpdateStrategy;

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, TransformPlugin, PhysicsPlugins::default()))
        .init_resource::<Assets<DestructibleAsset>>()
        .insert_resource(Time::<Fixed>::from_hz(60.0))
        .insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f64(1.0 / 60.0),
        ));
    app.finish();
    app.cleanup();
    app.update();
    app
}

fn manifest() -> Manifest {
    let mut manifest = crate::stress_smoke::beam_manifest();
    let physical = manifest.materials["concrete"].clone();
    manifest.materials = (2..=5)
        .map(|mass| {
            let mut material = physical.clone();
            material.density = mass as f32;
            (format!("mass-{mass}"), material)
        })
        .collect();
    let template = manifest.chunks[0].clone();
    manifest.chunks = ["a", "b", "c", "d"]
        .into_iter()
        .enumerate()
        .map(|(index, id)| {
            let mut chunk = template.clone();
            chunk.id = id.into();
            chunk.node = id.into();
            chunk.part_id = id.into();
            chunk.mass = index as f32 + 2.0;
            chunk.material = format!("mass-{}", index + 2);
            chunk.volume = 1.0;
            chunk.position = [0.0, index as f32 + 0.5, 0.0];
            chunk.rotation = Quat::IDENTITY.to_array();
            chunk.center_of_mass = [0.0; 3];
            chunk.inertia = Mat3::from_diagonal(Vec3::splat(chunk.mass / 6.0)).to_cols_array_2d();
            chunk.colliders = vec![ColliderRecord {
                kind: "convex_hull".into(),
                vertices: [-0.5, 0.5]
                    .into_iter()
                    .flat_map(|x| {
                        [-0.5, 0.5]
                            .into_iter()
                            .flat_map(move |y| [-0.5, 0.5].map(|z| [x, y, z]))
                    })
                    .collect(),
            }];
            chunk
        })
        .collect();
    let bond_template = manifest.bonds[0].clone();
    manifest.bonds = [("ab", "a", "b"), ("bc", "b", "c"), ("cd", "c", "d")]
        .map(|(id, a, b)| {
            let y = match id {
                "ab" => 1.0,
                "bc" => 2.0,
                _ => 3.0,
            };
            let q = [
                [1.0 / 12.0, 0.0, 0.0],
                [0.0, 0.0, 0.0],
                [0.0, 0.0, 1.0 / 12.0],
            ];
            BondRecord {
                id: id.into(),
                a: a.into(),
                b: b.into(),
                center: [0.0, y, 0.0],
                area: 1.0,
                area_covariance: q,
                patches: vec![crate::asset::BondPatchRecord {
                    area: 1.0,
                    center: [0.0, y, 0.0],
                    normal: [0.0, 1.0, 0.0],
                    area_covariance: q,
                    vertices: vec![
                        [-0.5, y, -0.5],
                        [-0.5, y, 0.5],
                        [0.5, y, 0.5],
                        [0.5, y, -0.5],
                    ],
                }],
                ..bond_template.clone()
            }
        })
        .into();
    manifest.anchors = vec![AnchorRecord {
        chunk: "a".into(),
        kind: "fixed".into(),
        position: [0.0; 3],
    }];
    manifest.validate().unwrap();
    manifest
}

fn spawn_building(app: &mut App, asset: Handle<DestructibleAsset>, pose: Transform) -> Entity {
    let manifest = app
        .world()
        .resource::<Assets<DestructibleAsset>>()
        .get(&asset)
        .unwrap()
        .manifest
        .clone();
    let root = app.world_mut().spawn((pose, Visibility::Visible)).id();
    let mut instance = BuildingInstance::pending(asset);
    instance.bounds = Some(BuildingBounds {
        min: Vec3::new(-0.5, 0.0, -0.5),
        max: Vec3::new(0.5, 4.0, 0.5),
    });
    instance.topology_dirty = true;
    instance.bonds = manifest
        .bonds
        .iter()
        .map(|bond| (bond.id.clone(), true))
        .collect();
    for record in manifest.chunks {
        let local = record.transform();
        let points: Vec<_> = record.colliders[0]
            .vertices
            .iter()
            .copied()
            .map(Vec3::from_array)
            .collect();
        let entity = app
            .world_mut()
            .spawn((
                local,
                GlobalTransform::from(pose) * local,
                Visibility::Inherited,
                ChildOf(root),
                Collider::convex_hull(&points).unwrap(),
                ColliderDensity(0.0),
                Mass(record.mass),
                AngularInertia::from_mat3_unchecked(Mat3::from_cols_array_2d(&record.inertia)),
                CenterOfMass(Vec3::from_array(record.center_of_mass)),
                NoAutoMass,
                NoAutoAngularInertia,
                NoAutoCenterOfMass,
                ChunkIdentity {
                    building: root,
                    id: record.id.clone(),
                },
            ))
            .id();
        instance.chunks.insert(
            record.id,
            ChunkInstance {
                entity,
                primitives: Vec::new(),
                alive: true,
                triangles: Vec::new(),
                transform: local,
                material: record.material,
                detail: None,
                cluster: None,
            },
        );
    }
    app.world_mut().entity_mut(root).insert(instance);
    root
}

fn asset(app: &mut App) -> Handle<DestructibleAsset> {
    app.world_mut()
        .resource_mut::<Assets<DestructibleAsset>>()
        .add(DestructibleAsset {
            manifest: manifest(),
            gltf: Handle::default(),
        })
}

fn remove(world: &mut World, building: Entity, id: &str) -> Entity {
    let mut instance = world.get_mut::<BuildingInstance>(building).unwrap();
    let entity = instance.chunks[id].entity;
    instance.chunks.get_mut(id).unwrap().alive = false;
    instance.topology_dirty = true;
    for (bond_id, active) in &mut instance.bonds {
        if bond_id.contains(id) {
            *active = false;
        }
    }
    entity
}

fn body(world: &World, building: Entity, id: &str) -> Entity {
    world.get::<BuildingInstance>(building).unwrap().chunks[id]
        .cluster
        .unwrap()
}

fn assert_vec(actual: Vec3, expected: Vec3) {
    assert!(
        actual.distance(expected) < 1e-4,
        "{actual:?} != {expected:?}"
    );
}

#[test]
fn support_removal_reparents_live_colliders_and_preserves_other_instances() {
    let mut app = app();
    let asset = asset(&mut app);
    let damaged = spawn_building(&mut app, asset.clone(), Transform::from_xyz(3.0, 0.0, 1.0));
    let intact = spawn_building(&mut app, asset, Transform::from_xyz(-3.0, 0.0, 1.0));
    rebuild_clusters(app.world_mut());
    let old = body(app.world(), damaged, "a");
    let untouched = body(app.world(), intact, "a");
    assert_eq!(app.world().get::<RigidBody>(old), Some(&RigidBody::Static));
    assert_eq!(app.world().get::<Mass>(old).unwrap().0, 14.0);
    assert_eq!(app.world().get::<RigidBodyColliders>(old).unwrap().len(), 4);
    let removed = remove(app.world_mut(), damaged, "a");
    let surviving_ids = ["b", "c", "d"]
        .map(|id| app.world().get::<BuildingInstance>(damaged).unwrap().chunks[id].entity);
    rebuild_clusters(app.world_mut());
    let detached = body(app.world(), damaged, "b");
    assert_ne!(old, detached);
    assert!(app.world().get_entity(old).is_err());
    assert_eq!(
        app.world().get::<RigidBody>(detached),
        Some(&RigidBody::Dynamic)
    );
    assert_eq!(app.world().get::<Mass>(detached).unwrap().0, 12.0);
    assert_eq!(
        app.world()
            .get::<RigidBodyColliders>(detached)
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        app.world().get::<ChildOf>(removed).unwrap().parent(),
        damaged
    );
    assert!(app.world().get::<Collider>(removed).is_none());
    assert!(app.world().get::<RigidBody>(removed).is_none());
    assert!(app.world().get::<ChunkIdentity>(removed).is_some());
    for entity in surviving_ids {
        assert!(app.world().get::<Collider>(entity).is_some());
        assert_eq!(
            app.world().get::<ColliderOf>(entity).unwrap().body,
            detached
        );
        assert_eq!(
            app.world().get::<ChildOf>(entity).unwrap().parent(),
            detached
        );
    }
    assert_eq!(body(app.world(), intact, "a"), untouched);
    assert_eq!(app.world().get::<Mass>(untouched).unwrap().0, 14.0);
    app.world_mut()
        .get_mut::<BuildingInstance>(damaged)
        .unwrap()
        .topology_dirty = true;
    rebuild_clusters(app.world_mut());
    assert_eq!(body(app.world(), damaged, "b"), detached);
    app.update();
    for entity in surviving_ids {
        assert_eq!(
            app.world().get::<ColliderOf>(entity).unwrap().body,
            detached
        );
    }
    assert_eq!(app.world().get::<Mass>(detached).unwrap().0, 12.0);
}

#[test]
fn moving_split_uses_physics_pose_and_inherits_angular_velocity_field() {
    let mut app = app();
    let asset = asset(&mut app);
    let building = spawn_building(&mut app, asset, Transform::from_xyz(8.0, 0.0, -2.0));
    rebuild_clusters(app.world_mut());
    remove(app.world_mut(), building, "a");
    rebuild_clusters(app.world_mut());
    let parent = body(app.world(), building, "b");
    let position = Vec3::new(4.0, 8.0, 6.0);
    let rotation = Quat::from_euler(EulerRot::YXZ, 0.7, -0.3, 0.5);
    let linear = Vec3::new(2.0, -1.0, 3.0);
    let angular = Vec3::new(0.2, -0.3, 0.7);
    let center = position + rotation * app.world().get::<CenterOfMass>(parent).unwrap().0;
    app.world_mut().entity_mut(parent).insert((
        Position(position),
        Rotation(rotation),
        LinearVelocity(linear),
        AngularVelocity(angular),
        Transform::from_xyz(-100.0, -100.0, -100.0),
    ));
    let removed = remove(app.world_mut(), building, "c");
    rebuild_clusters(app.world_mut());
    assert!(app.world().get_entity(parent).is_err());
    assert!(app.world().get_entity(removed).is_ok());
    assert!(app.world().get::<Collider>(removed).is_none());
    assert_eq!(
        app.world()
            .get::<BuildingInstance>(building)
            .unwrap()
            .clusters
            .len(),
        2
    );
    for (id, mass, local_center) in [
        ("b", 3.0, Vec3::new(0.0, 1.5, 0.0)),
        ("d", 5.0, Vec3::new(0.0, 3.5, 0.0)),
    ] {
        let child = body(app.world(), building, id);
        assert_vec(app.world().get::<Position>(child).unwrap().0, position);
        assert!(
            app.world()
                .get::<Rotation>(child)
                .unwrap()
                .0
                .angle_between(rotation)
                < 1e-4
        );
        assert_eq!(app.world().get::<Mass>(child).unwrap().0, mass);
        assert_vec(
            app.world().get::<CenterOfMass>(child).unwrap().0,
            local_center,
        );
        assert_vec(
            app.world().get::<AngularVelocity>(child).unwrap().0,
            angular,
        );
        let world_center = position + rotation * local_center;
        assert_vec(
            app.world().get::<LinearVelocity>(child).unwrap().0,
            linear + angular.cross(world_center - center),
        );
        let entity = app
            .world()
            .get::<BuildingInstance>(building)
            .unwrap()
            .chunks[id]
            .entity;
        assert_vec(
            app.world()
                .get::<GlobalTransform>(entity)
                .unwrap()
                .translation(),
            world_center,
        );
        assert_vec(
            app.world()
                .get::<ColliderTransform>(entity)
                .unwrap()
                .translation,
            local_center,
        );
        assert_eq!(app.world().get::<ColliderOf>(entity).unwrap().body, child);
    }
}

#[test]
fn detached_cluster_falls_contacts_ground_and_sleeps_without_becoming_anchored() {
    let mut app = app();
    app.world_mut().spawn((
        RigidBody::Static,
        Collider::cuboid(20.0, 0.2, 20.0),
        Transform::from_xyz(0.0, -0.1, 0.0),
        Friction::new(0.7),
        Restitution::new(0.0),
    ));
    let asset = asset(&mut app);
    let building = spawn_building(&mut app, asset, Transform::IDENTITY);
    rebuild_clusters(app.world_mut());
    remove(app.world_mut(), building, "a");
    rebuild_clusters(app.world_mut());
    let detached = body(app.world(), building, "b");
    for _ in 0..12 {
        app.update();
    }
    assert!(app.world().get::<Position>(detached).unwrap().y < -0.05);
    assert!(app.world().get::<LinearVelocity>(detached).unwrap().y < -1.0);
    for _ in 0..420 {
        app.update();
    }
    let y = app.world().get::<Position>(detached).unwrap().y;
    assert!(
        (-1.08..-0.9).contains(&y),
        "Ground did not support detached compound: y={y}"
    );
    assert!(app.world().get::<Sleeping>(detached).is_some());
    assert_eq!(
        app.world().get::<RigidBody>(detached),
        Some(&RigidBody::Dynamic)
    );
    assert!(!app.world().get::<Cluster>(detached).unwrap().anchored);
    assert_eq!(app.world().get::<Mass>(detached).unwrap().0, 12.0);
    app.world_mut()
        .get_mut::<BuildingInstance>(building)
        .unwrap()
        .topology_dirty = true;
    rebuild_clusters(app.world_mut());
    assert_eq!(body(app.world(), building, "b"), detached);
    assert!(!app.world().get::<Cluster>(detached).unwrap().anchored);

    {
        let mut instance = app
            .world_mut()
            .get_mut::<BuildingInstance>(building)
            .unwrap();
        instance.bonds.insert("bc".into(), false);
        instance.topology_dirty = true;
    }
    rebuild_clusters(app.world_mut());
    let lower = body(app.world(), building, "b");
    let upper = body(app.world(), building, "c");
    assert_ne!(lower, upper);
    assert!(app.world().get_entity(detached).is_err());
    for _ in 0..180 {
        app.update();
    }
    for cluster in [lower, upper] {
        assert_eq!(
            app.world().get::<RigidBody>(cluster),
            Some(&RigidBody::Dynamic)
        );
        assert!(app.world().get::<Position>(cluster).unwrap().is_finite());
        assert!(!app.world().get::<Cluster>(cluster).unwrap().anchored);
    }
}

#[test]
fn damage_targets_moved_chunk_and_transforms_neighbor_faces_and_emission_surfaces() {
    use crate::{fracture_detail::ContactPatch, interaction::DamageRequests, vfx::BreakBurst};

    let mut app = app();
    app.insert_resource(Gravity(Vec3::ZERO))
        .add_message::<BreakBurst>()
        .init_resource::<DamageRequests>()
        .add_systems(Update, crate::interaction::apply_damage);
    let asset = asset(&mut app);
    let building = spawn_building(&mut app, asset, Transform::IDENTITY);
    rebuild_clusters(app.world_mut());
    remove(app.world_mut(), building, "a");
    rebuild_clusters(app.world_mut());
    let body = body(app.world(), building, "b");
    let position = Vec3::new(8.0, 5.0, -4.0);
    let rotation = Quat::from_euler(EulerRot::YXZ, 0.8, -0.3, 0.2);
    let pose = Transform::from_translation(position).with_rotation(rotation);
    app.world_mut().entity_mut(body).insert((
        Position(position),
        Rotation(rotation),
        pose,
        GlobalTransform::from(pose),
    ));
    let patch_triangle = [
        Vec3::new(-0.5, 3.0, -0.5),
        Vec3::new(-0.5, 3.0, 0.5),
        Vec3::new(0.5, 3.0, 0.5),
    ];
    {
        let mut instance = app
            .world_mut()
            .get_mut::<BuildingInstance>(building)
            .unwrap();
        for (index, id) in ["b", "c", "d"].into_iter().enumerate() {
            let bottom = index as f32 + 1.0;
            let [a, b, c, d] = [
                Vec3::new(-0.5, bottom, 0.5),
                Vec3::new(0.5, bottom, 0.5),
                Vec3::new(0.5, bottom + 1.0, 0.5),
                Vec3::new(-0.5, bottom + 1.0, 0.5),
            ];
            instance.chunks.get_mut(id).unwrap().triangles = vec![[a, b, c], [a, c, d]];
        }
        instance.patches = std::sync::Arc::new(vec![ContactPatch {
            id: "cd patch".into(),
            bond_id: "cd".into(),
            a: "c".into(),
            b: "d".into(),
            normal: Vec3::Y,
            triangles: vec![patch_triangle],
            area: 0.5,
            rims: Vec::new(),
        }]);
    }
    app.update();
    let local_hit = Vec3::new(0.0, 1.95, 0.5);
    let world_hit = pose.transform_point(local_hit);
    let normal = rotation * Vec3::Z;
    app.world_mut()
        .resource_mut::<DamageRequests>()
        .0
        .push_back(Ray3d::new(
            world_hit + normal * 3.0,
            Dir3::new(-normal).unwrap(),
        ));
    let mut cursor = bevy::ecs::message::MessageCursor::<BreakBurst>::default();
    app.update();
    let instance = app.world().get::<BuildingInstance>(building).unwrap();
    assert!(!instance.chunks["b"].alive, "Moved collider was not picked");
    assert!(
        !instance.chunks["c"].alive,
        "Neighbor face was tested in stale rest coordinates"
    );
    assert!(instance.chunks["d"].alive);
    let messages = app.world().resource::<Messages<BreakBurst>>();
    let bursts = cursor.read(messages).collect::<Vec<_>>();
    assert_eq!(bursts.len(), 1);
    assert_vec(bursts[0].point, world_hit);
    assert_vec(bursts[0].normal, normal);
    assert_eq!(bursts[0].surfaces.len(), 1);
    assert_eq!(bursts[0].removed_volume, 2.0);
    for (actual, local) in bursts[0].surfaces[0]
        .triangle
        .into_iter()
        .zip(patch_triangle)
    {
        assert_vec(actual, pose.transform_point(local));
    }
    assert_vec(bursts[0].surfaces[0].normal, rotation * Vec3::NEG_Y);
}

#[test]
fn splitting_awake_compound_rebinds_cached_contacts_with_adjacent_static_building() {
    use avian3d::collision::contact_types::ContactGraph;

    let mut app = app();
    app.insert_resource(Gravity(Vec3::new(9.81, 0.0, 0.0)));
    let asset = asset(&mut app);
    let building = spawn_building(&mut app, asset.clone(), Transform::IDENTITY);
    let wall = spawn_building(&mut app, asset, Transform::from_xyz(1.0, 0.0, 0.0));
    rebuild_clusters(app.world_mut());
    remove(app.world_mut(), building, "a");
    rebuild_clusters(app.world_mut());
    let old = body(app.world(), building, "b");
    let wall_body = body(app.world(), wall, "a");
    app.world_mut().entity_mut(old).insert(SleepingDisabled);
    for _ in 0..30 {
        app.update();
    }
    assert!(
        app.world()
            .resource::<ContactGraph>()
            .iter_active_touching()
            .any(|pair| {
                [pair.body1, pair.body2].contains(&Some(old))
                    && [pair.body1, pair.body2].contains(&Some(wall_body))
            }),
        "Fixture must cache live dynamic-static face contacts before the split"
    );
    {
        let mut instance = app
            .world_mut()
            .get_mut::<BuildingInstance>(building)
            .unwrap();
        instance.bonds.insert("bc".into(), false);
        instance.topology_dirty = true;
    }
    rebuild_clusters(app.world_mut());
    let lower = body(app.world(), building, "b");
    let upper = body(app.world(), building, "c");
    assert_ne!(lower, upper);
    assert!(app.world().get_entity(old).is_err());
    for _ in 0..30 {
        app.update();
    }
    let graph = app.world().resource::<ContactGraph>();
    assert!(
        graph
            .iter_active()
            .chain(graph.iter_sleeping())
            .all(|pair| pair.body1 != Some(old) && pair.body2 != Some(old))
    );
    assert!(
        graph
            .iter_active_touching()
            .chain(graph.iter_sleeping_touching())
            .any(|pair| {
                [pair.body1, pair.body2].contains(&Some(wall_body))
                    && [pair.body1, pair.body2]
                        .iter()
                        .any(|owner| *owner == Some(lower) || *owner == Some(upper))
            }),
        "New compounds lost persistent contact with the static building"
    );
    assert_eq!(body(app.world(), wall, "a"), wall_body);
}
