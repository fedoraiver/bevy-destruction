use super::*;

#[test]
fn force_directions_rotate_without_translation_and_obey_action_reaction() {
    let pose = Transform::from_xyz(17.0, -9.0, 3.0)
        .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2));
    let (a, b, magnitude) = world_force_pair(pose, [100.0, 0.0, 0.0]).unwrap();
    assert!(a.distance(Vec3::Y) < 1e-6);
    assert!((a + b).length() < 1e-7);
    assert_eq!(magnitude, 100.0);
    assert_eq!(pose.transform_point(Vec3::ZERO), pose.translation);
}

#[test]
fn linear_arrow_scale_keeps_zero_load_zero_and_uses_its_own_reference() {
    assert_eq!(scaled_length(0.0, 100.0), 0.0);
    assert!((scaled_length(50.0, 100.0) - 2.0 * scaled_length(25.0, 100.0)).abs() < 1e-7);
    assert_eq!(scaled_length(100.0, 100.0), ARROW_LENGTH);
    assert_eq!(scaled_length(100.0, 1000.0), ARROW_LENGTH * 0.1);
    assert_eq!(scaled_length(f64::NAN, 100.0), 0.0);
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
