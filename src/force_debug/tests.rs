use super::*;

#[test]
fn force_directions_rotate_without_translation_and_obey_action_reaction() {
    let pose = GlobalTransform::from(
        Transform::from_xyz(17.0, -9.0, 3.0)
            .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
    );
    let a = direction_on_chunk(&pose, [100.0, 0.0, 0.0], true).unwrap();
    let b = direction_on_chunk(&pose, [100.0, 0.0, 0.0], false).unwrap();
    assert!(a.distance(Vec3::Y) < 1e-6);
    assert!((a + b).length() < 1e-7);
    assert!(direction_on_chunk(&pose, [0.0; 3], true).is_none());
}

#[test]
fn heat_levels_use_inclusive_capacity_thresholds() {
    assert_eq!(heat_tag(0.499), 0);
    assert_eq!(heat_tag(0.5), HEAT_AMBER_TAG);
    assert_eq!(heat_tag(0.999), HEAT_AMBER_TAG);
    assert_eq!(heat_tag(1.0), HEAT_RED_TAG);
}

#[test]
fn building_load_changes_hide_only_its_stale_force_results_until_the_next_fixed_step() {
    use std::time::Duration;

    use crate::{
        physics::DestructionPhysicsPlugin,
        stress::NodeLoad,
        stress_runtime::StressRuntimePlugin,
        stress_smoke::{BEAM, RIGHT_BOND, spawn_beam},
    };
    use bevy::{gizmos::GizmoAsset, time::TimeUpdateStrategy};

    let mut app = App::new();
    let step = Duration::from_secs_f64(1.0 / 60.0);
    app.add_plugins((MinimalPlugins, TransformPlugin, PhysicsPlugins::default()))
        .init_resource::<Assets<DestructibleAsset>>()
        .init_resource::<Assets<GizmoAsset>>()
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .init_resource::<Aim>()
        .init_resource::<ForceSamples>()
        .init_resource::<ForceViewStats>()
        .insert_resource(DemoOptions {
            force_smoke_output: Some(default()),
            ..default()
        })
        .insert_resource(ForceCapture { enabled: true })
        .insert_resource(Time::<Fixed>::from_hz(60.0))
        .insert_resource(TimeUpdateStrategy::ManualDuration(step))
        .init_gizmo_group::<ForceGizmos>()
        .add_plugins((DestructionPhysicsPlugin, StressRuntimePlugin))
        .add_systems(PostUpdate, draw.after(TransformSystems::Propagate));
    app.finish();
    app.cleanup();
    app.update();
    let building = spawn_beam(app.world_mut(), Vec3::ZERO);
    let other = spawn_beam(app.world_mut(), Vec3::X * 8.0);
    for _ in 0..5 {
        app.update();
    }

    let visible_bonds = |app: &App| {
        let stats = app.world().resource::<ForceViewStats>();
        [building, other].map(|building| {
            stats
                .valid_bonds
                .iter()
                .filter(|(owner, _)| *owner == building)
                .count()
        })
    };
    let force = |app: &App| {
        app.world().get::<StressState>(building).unwrap().bonds[RIGHT_BOND]
            .stress
            .force[1]
            .abs()
    };
    let draw_without_fixed_step = |app: &mut App| {
        let now = app.world().resource::<Time<Fixed>>().elapsed();
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
        app.update();
        assert_eq!(app.world().resource::<Time<Fixed>>().elapsed(), now);
        assert_eq!(visible_bonds(app), [0, 2]);
    };
    let resume_fixed_step = |app: &mut App| {
        app.insert_resource(TimeUpdateStrategy::ManualDuration(step));
        app.update();
        assert_eq!(visible_bonds(app), [2, 2]);
    };
    assert_eq!(visible_bonds(&app), [2, 2]);
    let original_force = force(&app);
    let other_solves = app
        .world()
        .get::<StressState>(other)
        .unwrap()
        .numerical_solves;

    app.world_mut()
        .entity_mut(building)
        .insert(StructuralLoads {
            by_chunk: BTreeMap::from([(
                BEAM.into(),
                NodeLoad {
                    force: [0.0, -1000.0, 0.0],
                    moment: [0.0; 3],
                },
            )]),
        });
    draw_without_fixed_step(&mut app);
    resume_fixed_step(&mut app);
    let loaded_force = force(&app);
    assert!(loaded_force > original_force + 400.0);

    app.world_mut()
        .get_mut::<StructuralLoads>(building)
        .unwrap()
        .by_chunk
        .get_mut(BEAM)
        .unwrap()
        .force[1] *= 2.0;
    draw_without_fixed_step(&mut app);
    resume_fixed_step(&mut app);
    assert!(force(&app) > loaded_force + 400.0);

    app.world_mut()
        .entity_mut(building)
        .remove::<StructuralLoads>();
    draw_without_fixed_step(&mut app);
    resume_fixed_step(&mut app);
    assert_eq!(force(&app), original_force);
    assert_eq!(
        app.world()
            .get::<StressState>(other)
            .unwrap()
            .numerical_solves,
        other_solves
    );
}

#[test]
fn inspector_prioritizes_utilization_over_force_and_shows_at_most_three_bonds() {
    let mut bonds = [
        ("c", 0.7, 60.0),
        ("b", 0.7, 100.0),
        ("a", 1.0, 20.0),
        ("d", 0.1, 300.0),
    ]
    .into_iter()
    .map(|(id, utilization, force_n)| InspectionBond {
        direction: Some(Vec3::Y),
        readout: ForceBondReadout {
            id: id.into(),
            force_n,
            moment_nm: 3.0,
            utilization,
            impact: id == "a",
        },
    })
    .collect::<Vec<_>>();
    order_inspection_bonds(&mut bonds);
    assert_eq!(
        bonds
            .iter()
            .map(|bond| bond.readout.id.as_str())
            .collect::<Vec<_>>(),
        ["a", "b", "c"],
    );
    let text = inspection_text(&ForceInspection {
        chunk: (Entity::PLACEHOLDER, "test".into()),
        bonds: bonds.into_iter().map(|bond| bond.readout).collect(),
        impulse_ns: Some(5.0),
    });
    assert!(text.contains("U 100.0%  Impact J/10 ms"));
    assert!(text.contains("F 20.00 N | M 3.00 N*m"));
    assert!(text.contains("Contact J 5.00 N*s"));
}

#[test]
fn inspector_clears_on_release_focus_loss_removed_target_and_numpad_toggle() {
    use bevy::camera::{CameraProjection, RenderTargetInfo};

    let mut app = App::new();
    app.init_resource::<DemoOptions>()
        .init_resource::<CameraControl>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ScriptedForceInspection>()
        .init_resource::<ForceVisibilityCache>()
        .init_resource::<Aim>()
        .insert_resource(ForceCapture { enabled: true })
        .add_systems(Update, (toggle, pick_target).chain());
    let world = app.world_mut();
    let window = world
        .spawn((
            Window {
                focused: true,
                ..default()
            },
            PrimaryWindow,
        ))
        .id();
    let projection = PerspectiveProjection {
        aspect_ratio: 1.0,
        ..default()
    };
    let mut camera = Camera::default();
    camera.computed.target_info = Some(RenderTargetInfo {
        physical_size: UVec2::splat(500),
        ..default()
    });
    camera.computed.clip_from_view = projection.get_clip_from_view();
    world.spawn((
        camera,
        Projection::Perspective(projection),
        GlobalTransform::from_xyz(0.0, 0.0, 5.0),
        DemoCamera,
    ));
    let building = world.spawn_empty().id();
    let body = world
        .spawn((
            Name::new("Force inspection body"),
            GlobalTransform::IDENTITY,
            RigidBody::Static,
            ChildOf(building),
            Cluster {
                chunks: vec!["front".into()],
                anchored: true,
            },
        ))
        .id();
    let front = world
        .spawn((
            Name::new("Front force inspection target"),
            ChildOf(body),
            GlobalTransform::IDENTITY,
            InheritedVisibility::VISIBLE,
            ChunkInstance {
                alive: true,
                triangles: vec![[
                    Vec3::new(-1.0, -1.0, 0.0),
                    Vec3::new(1.0, -1.0, 0.0),
                    Vec3::Y,
                ]],
                transform: Transform::IDENTITY,
                mass_properties: avian3d::prelude::MassProperties3d::ZERO,
                material: "concrete".into(),
            },
        ))
        .id();
    world.entity_mut(front).insert(ColliderOf { body });
    let mut instance = BuildingInstance::pending(Handle::default());
    instance.chunks.insert("front".into(), front);
    world.entity_mut(building).insert(instance);

    app.update();
    assert!(!app.world().resource::<Aim>().queried);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::AltLeft);
    app.update();
    assert_eq!(
        app.world().resource::<Aim>().hit.as_ref().unwrap().key,
        (building, "front".into())
    );
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::AltLeft);
    app.update();
    assert!(app.world().resource::<Aim>().hit.is_none());

    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::AltRight);
    app.update();
    assert!(app.world().resource::<Aim>().hit.is_some());
    app.world_mut().get_mut::<Window>(window).unwrap().focused = false;
    app.update();
    assert!(!app.world().resource::<Aim>().queried);
    assert!(app.world().resource::<Aim>().hit.is_none());
    app.world_mut().get_mut::<Window>(window).unwrap().focused = true;
    app.update();
    assert!(app.world().resource::<Aim>().hit.is_some());
    app.world_mut().despawn(building);
    app.update();
    assert!(app.world().resource::<Aim>().hit.is_none());

    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Numpad9);
    app.update();
    assert!(!app.world().resource::<ForceCapture>().enabled);
    assert!(!app.world().resource::<Aim>().queried);
}

#[test]
fn dead_or_split_members_cannot_reuse_a_dynamic_force_frame() {
    let mut world = World::new();
    let building = world.spawn_empty().id();
    let body = world
        .spawn((
            Name::new("Original force body"),
            GlobalTransform::IDENTITY,
            RigidBody::Dynamic,
        ))
        .id();
    let other = world
        .spawn((
            Name::new("Other force body"),
            GlobalTransform::IDENTITY,
            RigidBody::Dynamic,
        ))
        .id();
    let mut instance = BuildingInstance::pending(Handle::default());
    for id in ["a", "b"] {
        let entity = world
            .spawn((
                Name::new(format!("Force member {id}")),
                GlobalTransform::IDENTITY,
                ChunkInstance {
                    alive: true,
                    triangles: Vec::new(),
                    transform: Transform::IDENTITY,
                    mass_properties: MassProperties3d::ZERO,
                    material: "concrete".into(),
                },
            ))
            .id();
        world.entity_mut(entity).insert(ColliderOf { body });
        instance.chunks.insert(id.into(), entity);
    }
    let a = instance.chunks["a"];
    let b = instance.chunks["b"];
    let bond = world
        .spawn((
            Name::new("Force test bond"),
            crate::scene::BondOfBuilding { building },
            BondInstance {
                id: "a-b".into(),
                endpoints: [a, b],
                active: true,
            },
        ))
        .id();
    instance.bonds.insert("a-b".into(), bond);
    let sample = DynamicForceSample {
        building,
        body,
        chunks: vec!["a".into(), "b".into()],
        bonds: BTreeMap::new(),
        bond_signature: bond_signature([("a-b", true)]),
        captured_at: 0.0,
    };
    let mut cluster = Cluster {
        chunks: sample.chunks.clone(),
        anchored: false,
    };
    let parent = ChildOf(building);
    let current = |world: &World, cluster: &Cluster, parent: &ChildOf| {
        sample_members_current(
            &sample,
            &instance,
            cluster,
            parent,
            |entity| {
                world
                    .get::<ChunkInstance>(entity)
                    .map(|chunk| (chunk, world.get::<ColliderOf>(entity)))
            },
            || bond_signature([("a-b", world.get::<BondInstance>(bond).unwrap().active)]),
        )
    };
    assert!(current(&world, &cluster, &parent));
    assert!(!current(&world, &cluster, &ChildOf(other)));
    world.get_mut::<ChunkInstance>(b).unwrap().alive = false;
    assert!(!current(&world, &cluster, &parent));
    world.get_mut::<ChunkInstance>(b).unwrap().alive = true;
    world.entity_mut(b).insert(ColliderOf { body: other });
    assert!(!current(&world, &cluster, &parent));
    world.entity_mut(b).remove::<ColliderOf>();
    assert!(!current(&world, &cluster, &parent));
    world.entity_mut(b).insert(ColliderOf { body });
    cluster.chunks.pop();
    assert!(!current(&world, &cluster, &parent));
    cluster.chunks.push("b".into());
    assert!(current(&world, &cluster, &parent));
    world.get_mut::<BondInstance>(bond).unwrap().active = false;
    assert!(!current(&world, &cluster, &parent));
}
