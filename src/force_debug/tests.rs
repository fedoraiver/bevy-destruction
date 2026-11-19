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
            GlobalTransform::IDENTITY,
            Cluster {
                building,
                chunks: vec!["front".into()],
                anchored: true,
            },
        ))
        .id();
    let mut instance = BuildingInstance::pending(Handle::default());
    instance.chunks.insert(
        "front".into(),
        crate::scene::ChunkInstance {
            entity: world.spawn(InheritedVisibility::VISIBLE).id(),
            primitives: vec![],
            alive: true,
            triangles: vec![[
                Vec3::new(-1.0, -1.0, 0.0),
                Vec3::new(1.0, -1.0, 0.0),
                Vec3::Y,
            ]],
            transform: Transform::IDENTITY,
            mass_properties: avian3d::prelude::MassProperties3d::ZERO,
            material: "concrete".into(),
            detail: None,
            cluster: Some(body),
        },
    );
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
    let body = world.spawn_empty().id();
    let other = world.spawn_empty().id();
    let mut instance = BuildingInstance::pending(Handle::default());
    for id in ["a", "b"] {
        instance.chunks.insert(
            id.into(),
            crate::scene::ChunkInstance {
                entity: world.spawn_empty().id(),
                primitives: Vec::new(),
                alive: true,
                triangles: Vec::new(),
                transform: Transform::IDENTITY,
                mass_properties: avian3d::prelude::MassProperties3d::ZERO,
                material: "concrete".into(),
                detail: None,
                cluster: Some(body),
            },
        );
    }
    let mut sample = DynamicForceSample {
        building,
        body,
        chunks: vec!["a".into(), "b".into()],
        bonds: BTreeMap::new(),
        bond_signature: bond_signature(&instance.bonds),
        captured_at: 0.0,
    };
    let mut cluster = Cluster {
        building,
        chunks: sample.chunks.clone(),
        anchored: false,
    };
    assert!(sample_members_current(&sample, &instance, &cluster));
    instance.chunks.get_mut("b").unwrap().alive = false;
    assert!(!sample_members_current(&sample, &instance, &cluster));
    instance.chunks.get_mut("b").unwrap().alive = true;
    instance.chunks.get_mut("b").unwrap().cluster = Some(other);
    assert!(!sample_members_current(&sample, &instance, &cluster));
    instance.chunks.get_mut("b").unwrap().cluster = Some(body);
    cluster.chunks.pop();
    assert!(!sample_members_current(&sample, &instance, &cluster));
    cluster.chunks.push("b".into());
    instance.bonds.insert("a-b".into(), true);
    sample.bond_signature = bond_signature(&instance.bonds);
    assert!(sample_members_current(&sample, &instance, &cluster));
    instance.bonds.insert("a-b".into(), false);
    assert!(!sample_members_current(&sample, &instance, &cluster));
}
