use super::*;
use crate::{
    asset::{AnchorRecord, BondRecord, ColliderRecord, Manifest},
    scene::{
        BuildingBounds, ChunkIdentity, chunk_collider, chunk_mass_properties, spawn_bonds,
        spawn_patches,
    },
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

fn box_proxy(center: Vec3, size: Vec3) -> ColliderRecord {
    ColliderRecord {
        kind: "convex_hull".into(),
        vertices: [-0.5, 0.5]
            .into_iter()
            .flat_map(|x| {
                [-0.5, 0.5].into_iter().flat_map(move |y| {
                    [-0.5, 0.5].map(|z| (center + size * Vec3::new(x, y, z)).to_array())
                })
            })
            .collect(),
    }
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
            chunk.colliders = vec![box_proxy(Vec3::ZERO, Vec3::ONE)];
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
    for record in manifest.chunks {
        let local = record.transform();
        let collider = chunk_collider(&record).unwrap();
        let density = manifest.materials[&record.material].density;
        let mass_properties = chunk_mass_properties(&record.id, &collider, density).unwrap();
        let entity = app
            .world_mut()
            .spawn((
                local,
                GlobalTransform::from(pose) * local,
                Visibility::Inherited,
                ChildOf(root),
                collider,
                ColliderDensity(density),
                ChunkIdentity {
                    building: root,
                    id: record.id.clone(),
                },
                ChunkInstance {
                    primitives: Vec::new(),
                    alive: true,
                    triangles: Vec::new(),
                    transform: local,
                    mass_properties,
                    material: record.material,
                },
            ))
            .id();
        instance.chunks.insert(record.id, entity);
    }
    let world = app.world_mut();
    spawn_bonds(&mut world.commands(), root, &mut instance, &manifest.bonds);
    world.flush();
    world.entity_mut(root).insert(instance);
    root
}

fn asset(app: &mut App) -> Handle<DestructibleAsset> {
    add_asset(app, manifest())
}

fn add_asset(app: &mut App, manifest: Manifest) -> Handle<DestructibleAsset> {
    app.world_mut()
        .resource_mut::<Assets<DestructibleAsset>>()
        .add(DestructibleAsset {
            manifest,
            gltf: Handle::default(),
        })
}

fn remove(world: &mut World, building: Entity, id: &str) -> Entity {
    let mut instance = world.get_mut::<BuildingInstance>(building).unwrap();
    let entity = instance.chunks[id];
    instance.topology_dirty = true;
    let bonds = instance.bonds.values().copied().collect::<Vec<_>>();
    world.get_mut::<ChunkInstance>(entity).unwrap().alive = false;
    for bond in bonds {
        let mut bond = world.get_mut::<BondInstance>(bond).unwrap();
        if bond.endpoints.contains(&entity) {
            bond.active = false;
        }
    }
    entity
}

fn body(world: &World, building: Entity, id: &str) -> Entity {
    let entity = world.get::<BuildingInstance>(building).unwrap().chunks[id];
    world.get::<ColliderOf>(entity).unwrap().body
}

fn chunk<'a>(world: &'a World, building: Entity, id: &str) -> &'a ChunkInstance {
    let entity = world.get::<BuildingInstance>(building).unwrap().chunks[id];
    world.get::<ChunkInstance>(entity).unwrap()
}

fn break_bond(world: &mut World, building: Entity, id: &str) {
    let mut instance = world.get_mut::<BuildingInstance>(building).unwrap();
    instance.topology_dirty = true;
    let entity = instance.bonds[id];
    world.get_mut::<BondInstance>(entity).unwrap().active = false;
}

fn mass(world: &World, body: Entity) -> f32 {
    world.get::<ComputedMass>(body).unwrap().value()
}

fn inertia(world: &World, body: Entity) -> Mat3 {
    world
        .get::<ComputedAngularInertia>(body)
        .unwrap()
        .tensor()
        .into()
}

fn assert_mass(world: &World, body: Entity, expected: f32) {
    let actual = mass(world, body);
    assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
}

fn assert_vec(actual: Vec3, expected: Vec3) {
    assert!(
        actual.distance(expected) < 1e-4,
        "{actual:?} != {expected:?}"
    );
}

#[test]
fn first_cluster_uses_building_pose_before_transform_propagation() {
    let mut app = app();
    let asset = asset(&mut app);
    let local = Transform::from_xyz(1.0, 2.0, 3.0).with_rotation(Quat::from_rotation_x(0.25));
    let building = spawn_building(&mut app, asset, local);
    let world = app.world_mut();
    let parent = world
        .spawn((Name::new("Building pose parent"), Transform::IDENTITY))
        .id();
    world.entity_mut(building).insert(ChildOf(parent));
    let parent_pose =
        Transform::from_xyz(9.0, 1.0, -4.0).with_rotation(Quat::from_rotation_y(0.55));
    *world.get_mut::<Transform>(parent).unwrap() = parent_pose;
    let expected = (GlobalTransform::from(parent_pose) * local).compute_transform();
    let stale = world
        .get::<GlobalTransform>(building)
        .unwrap()
        .translation();
    assert!(stale.distance(expected.translation) > 1.0);

    rebuild_clusters(world);
    let cluster = body(world, building, "a");
    assert_vec(
        world.get::<Position>(cluster).unwrap().0,
        expected.translation,
    );
    assert!(
        world
            .get::<Rotation>(cluster)
            .unwrap()
            .0
            .angle_between(expected.rotation)
            < 1e-4
    );
}

#[test]
fn support_removal_reparents_live_colliders_and_preserves_other_instances() {
    let mut app = app();
    let asset = asset(&mut app);
    let damaged = spawn_building(&mut app, asset.clone(), Transform::from_xyz(3.0, 0.0, 1.0));
    let intact = spawn_building(&mut app, asset, Transform::from_xyz(-3.0, 0.0, 1.0));
    let support = app.world().get::<BuildingInstance>(damaged).unwrap().chunks["a"];
    assert!(app.world().get::<ColliderOf>(support).is_none());
    assert!(building_clusters(app.world(), damaged).next().is_none());
    rebuild_clusters(app.world_mut());
    let old = body(app.world(), damaged, "a");
    let untouched = body(app.world(), intact, "a");
    assert!(building_clusters(app.world(), damaged).eq([old]));
    assert_eq!(app.world().get::<ChildOf>(old).unwrap().parent(), damaged);
    assert_eq!(
        app.world().get::<ChildOf>(untouched).unwrap().parent(),
        intact
    );
    assert_eq!(app.world().get::<RigidBody>(old), Some(&RigidBody::Static));
    assert_mass(app.world(), old, 14.0);
    assert_eq!(app.world().get::<RigidBodyColliders>(old).unwrap().len(), 4);
    let removed = remove(app.world_mut(), damaged, "a");
    assert_eq!(app.world().get::<ColliderOf>(removed).unwrap().body, old);
    let surviving_ids =
        ["b", "c", "d"].map(|id| app.world().get::<BuildingInstance>(damaged).unwrap().chunks[id]);
    rebuild_clusters(app.world_mut());
    let detached = body(app.world(), damaged, "b");
    assert_ne!(old, detached);
    assert!(app.world().get_entity(old).is_err());
    assert_eq!(
        app.world().get::<ChildOf>(detached).unwrap().parent(),
        damaged
    );
    assert_eq!(
        app.world().get::<RigidBody>(detached),
        Some(&RigidBody::Dynamic)
    );
    assert_mass(app.world(), detached, 12.0);
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
    assert!(app.world().get::<ColliderOf>(removed).is_none());
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
    assert!(building_clusters(app.world(), intact).eq([untouched]));
    assert!(building_clusters(app.world(), damaged).eq([detached]));
    assert_mass(app.world(), untouched, 14.0);
    app.world_mut()
        .get_mut::<BuildingInstance>(damaged)
        .unwrap()
        .topology_dirty = true;
    rebuild_clusters(app.world_mut());
    assert_eq!(body(app.world(), damaged, "b"), detached);
    assert!(building_clusters(app.world(), damaged).eq([detached]));
    app.update();
    assert!(app.world().get::<ColliderOf>(removed).is_none());
    for entity in surviving_ids {
        assert_eq!(
            app.world().get::<ColliderOf>(entity).unwrap().body,
            detached
        );
    }
    assert_mass(app.world(), detached, 12.0);
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
    let center = position + rotation * app.world().get::<ComputedCenterOfMass>(parent).unwrap().0;
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
        building_clusters(app.world(), building).collect::<BTreeSet<_>>(),
        BTreeSet::from([
            body(app.world(), building, "b"),
            body(app.world(), building, "d")
        ]),
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
        assert_mass(app.world(), child, mass);
        assert_vec(
            app.world().get::<ComputedCenterOfMass>(child).unwrap().0,
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
            .chunks[id];
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
fn rotated_density_compounds_preserve_momentum_on_first_step_after_split() {
    let mut app = app();
    let mut manifest = manifest();
    manifest
        .chunks
        .retain(|record| ["b", "c"].contains(&record.id.as_str()));
    manifest.bonds.retain(|bond| bond.id == "bc");
    manifest.anchors.clear();
    let boxes = [
        [
            (Vec3::new(0.4, 0.2, -0.1), Vec3::new(1.0, 1.0, 2.0)),
            (Vec3::new(2.2, 0.2, -0.1), Vec3::new(0.5, 2.0, 1.0)),
        ],
        [
            (Vec3::new(-0.2, 0.3, 0.5), Vec3::ONE),
            (Vec3::new(-0.2, 1.8, 0.5), Vec3::new(2.0, 0.5, 0.5)),
        ],
    ];
    let mut parts = Vec::new();
    for ((record, shapes), angle) in manifest.chunks.iter_mut().zip(boxes).zip([0.4, -0.7]) {
        record.rotation = Quat::from_rotation_z(angle).to_array();
        record.center_of_mass = [0.4, -0.2, 0.1];
        record.colliders = shapes.map(|(center, size)| box_proxy(center, size)).into();
        let transform = record.transform();
        let rotation = Mat3::from_quat(transform.rotation);
        let density = manifest.materials[&record.material].density;
        for (center, size) in shapes {
            let mass = size.element_product() * density;
            let square = size * size;
            let inertia =
                Mat3::from_diagonal((Vec3::splat(square.element_sum()) - square) * (mass / 12.0));
            parts.push((
                transform.transform_point(center),
                mass,
                rotation * inertia * rotation.transpose(),
            ));
        }
    }
    let expected_mass = parts.iter().map(|(_, mass, _)| mass).sum::<f32>();
    let expected_center = parts
        .iter()
        .map(|&(center, mass, _)| mass * center)
        .sum::<Vec3>()
        / expected_mass;
    let expected_inertia = parts
        .iter()
        .map(|&(center, mass, inertia)| {
            let r = center - expected_center;
            let outer = Mat3::from_cols(r * r.x, r * r.y, r * r.z);
            inertia + mass * (Mat3::IDENTITY * r.length_squared() - outer)
        })
        .sum::<Mat3>();
    assert!(expected_inertia.x_axis.y.abs() > 0.1);

    let asset = add_asset(&mut app, manifest);
    let building = spawn_building(&mut app, asset, Transform::IDENTITY);
    rebuild_clusters(app.world_mut());
    let parent = body(app.world(), building, "b");
    assert_eq!(body(app.world(), building, "c"), parent);
    assert_mass(app.world(), parent, expected_mass);
    assert_vec(
        app.world().get::<ComputedCenterOfMass>(parent).unwrap().0,
        expected_center,
    );
    assert!(inertia(app.world(), parent).abs_diff_eq(expected_inertia, 2e-4));

    let position = Vec3::new(4.0, 8.0, 6.0);
    let rotation = Quat::from_euler(EulerRot::YXZ, 0.7, -0.3, 0.5);
    let world_rotation = Mat3::from_quat(rotation);
    let linear = Vec3::new(2.0, -1.0, 3.0);
    let angular = Vec3::new(0.2, -0.3, 0.7);
    let parent_center = position + rotation * expected_center;
    app.world_mut().entity_mut(parent).insert((
        Position(position),
        Rotation(rotation),
        LinearVelocity(linear),
        AngularVelocity(angular),
    ));
    break_bond(app.world_mut(), building, "bc");
    rebuild_clusters(app.world_mut());

    let world = app.world();
    assert!(world.get_entity(parent).is_err());
    let mut total_mass = 0.0;
    let mut linear_momentum = Vec3::ZERO;
    let mut angular_momentum = Vec3::ZERO;
    for id in ["b", "c"] {
        let child = body(world, building, id);
        let member = chunk(world, building, id);
        let child_center = world.get::<ComputedCenterOfMass>(child).unwrap().0;
        let child_mass = mass(world, child);
        let center = position + rotation * child_center;
        let velocity = world.get::<LinearVelocity>(child).unwrap().0;
        let spin = world.get::<AngularVelocity>(child).unwrap().0;
        assert_vec(child_center, member.center_in_building());
        assert_vec(velocity, linear + angular.cross(center - parent_center));
        assert_vec(spin, angular);
        total_mass += child_mass;
        let momentum = child_mass * velocity;
        linear_momentum += momentum;
        angular_momentum +=
            world_rotation * inertia(world, child) * world_rotation.transpose() * spin
                + (center - parent_center).cross(momentum);
    }
    assert!((total_mass - expected_mass).abs() < 1e-5);
    assert_vec(linear_momentum, expected_mass * linear);
    assert_vec(
        angular_momentum,
        world_rotation * expected_inertia * world_rotation.transpose() * angular,
    );
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
    assert_mass(app.world(), detached, 12.0);
    app.world_mut()
        .get_mut::<BuildingInstance>(building)
        .unwrap()
        .topology_dirty = true;
    rebuild_clusters(app.world_mut());
    assert_eq!(body(app.world(), building, "b"), detached);
    assert!(!app.world().get::<Cluster>(detached).unwrap().anchored);

    break_bond(app.world_mut(), building, "bc");
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
        let world = app.world_mut();
        let mut instance = world
            .entity_mut(building)
            .take::<BuildingInstance>()
            .unwrap();
        for (index, id) in ["b", "c", "d"].into_iter().enumerate() {
            let bottom = index as f32 + 1.0;
            let [a, b, c, d] = [
                Vec3::new(-0.5, bottom, 0.5),
                Vec3::new(0.5, bottom, 0.5),
                Vec3::new(0.5, bottom + 1.0, 0.5),
                Vec3::new(-0.5, bottom + 1.0, 0.5),
            ];
            world
                .get_mut::<ChunkInstance>(instance.chunks[id])
                .unwrap()
                .triangles = vec![[a, b, c], [a, c, d]];
        }
        let patches = std::sync::Arc::new(vec![ContactPatch {
            id: "cd patch".into(),
            bond_id: "cd".into(),
            a: "c".into(),
            b: "d".into(),
            normal: Vec3::Y,
            triangles: vec![patch_triangle],
            area: 0.5,
            rims: Vec::new(),
        }]);
        spawn_patches(&mut world.commands(), building, &mut instance, patches);
        world.flush();
        world.entity_mut(building).insert(instance);
    }
    app.update();
    let local_hit = Vec3::new(0.0, 1.95, 0.5);
    let world_hit = pose.transform_point(local_hit);
    let normal = rotation * Vec3::Z;
    app.world_mut()
        .resource_mut::<DamageRequests>()
        .rays
        .push_back(Ray3d::new(
            world_hit + normal * 3.0,
            Dir3::new(-normal).unwrap(),
        ));
    let mut cursor = bevy::ecs::message::MessageCursor::<BreakBurst>::default();
    app.update();
    assert!(
        !chunk(app.world(), building, "b").alive,
        "Moved collider was not picked"
    );
    assert!(
        !chunk(app.world(), building, "c").alive,
        "Neighbor face was tested in stale rest coordinates"
    );
    assert!(chunk(app.world(), building, "d").alive);
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
    break_bond(app.world_mut(), building, "bc");
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
