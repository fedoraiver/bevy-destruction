use super::*;
use avian3d::collider_tree::ColliderTrees;

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        bevy::scene::ScenePlugin,
    ))
    .init_resource::<Assets<Mesh>>()
    .init_resource::<Assets<Image>>()
    .init_resource::<Assets<StandardMaterial>>()
    .init_resource::<Assets<DustMaterial>>()
    .init_resource::<ColliderTrees>()
    .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f64(1.0 / 60.0),
    ))
    .add_plugins(VfxPlugin);
    app.update();
    app
}

fn burst(seed: u64) -> BreakBurst {
    BreakBurst {
        point: Vec3::ZERO,
        normal: Vec3::Z,
        seed,
        material: "concrete".into(),
        removed_volume: 0.8,
        exposed_area: 1.5,
        surfaces: vec![EmissionSurface {
            triangle: [Vec3::ZERO, Vec3::X, Vec3::Y],
            normal: Vec3::Z,
        }],
    }
}

fn check_dust_instances(world: &mut World) -> usize {
    let mut query = world.query::<(
        &Particle,
        &DustSettings,
        &MeshTag,
        &MeshMaterial3d<DustMaterial>,
    )>();
    let shared = world.resource::<VfxAssets>();
    let buffer = world
        .resource::<Assets<ShaderBuffer>>()
        .get(&shared.dust_instances)
        .unwrap();
    let packed = buffer.cast_slice::<[[f32; 4]; 4]>().unwrap();
    let mut tags = std::collections::BTreeSet::new();
    for (particle, settings, tag, material) in query.iter(world) {
        assert_ne!(particle.layer, Layer::Debris);
        assert_eq!(material.0, shared.dust);
        assert!(tags.insert(tag.value));
        assert_eq!(tag.value as usize, particle.dust_slot.unwrap());
        assert_eq!(packed[tag.value as usize], settings.packed());
    }
    assert_eq!(world.resource::<Assets<DustMaterial>>().len(), 1);
    tags.len()
}

#[test]
fn shared_dust_material_preserves_instance_data_across_reuse_and_removal() {
    let mut app = app();
    for (seed, material) in [(1, "steel"), (2, "wood")] {
        let mut event = burst(seed);
        event.material = material.into();
        app.world_mut().write_message(event);
    }
    app.update();
    assert!(check_dust_instances(app.world_mut()) > 0);
    for _ in 0..30 {
        app.update();
    }
    for seed in 3..7 {
        app.world_mut().write_message(burst(seed));
    }
    app.update();
    assert!(check_dust_instances(app.world_mut()) > 0);
    app.world_mut().resource_mut::<VfxSettings>().enabled = false;
    app.update();
    assert_eq!(check_dust_instances(app.world_mut()), 0);
    app.world_mut().resource_mut::<VfxSettings>().enabled = true;
    app.world_mut().write_message(burst(7));
    app.update();
    assert!(check_dust_instances(app.world_mut()) > 0);
}

#[test]
fn sequenced_layers_emit_with_deterministic_outward_surface_motion() {
    let mut a = 67;
    let mut b = 67;
    for layer in [Layer::Impact, Layer::Debris, Layer::Smoke, Layer::Fallout] {
        let (one, pose_one) = spawn_particle(&burst(1), layer, &mut a);
        let (two, pose_two) = spawn_particle(&burst(1), layer, &mut b);
        assert_eq!(one.age, two.age);
        assert_eq!(one.velocity, two.velocity);
        assert_eq!(pose_one, pose_two);
        assert!(one.velocity.z > 0.0);
        if layer == Layer::Impact {
            assert_eq!(one.age, 0.0);
        } else {
            assert!(one.age < 0.0);
        }
    }
    let mut app = app();
    app.world_mut().write_message(burst(1));
    app.update();
    let stats = app.world().resource::<VfxStats>();
    assert_eq!(stats.active[0], 8);
    assert_eq!(stats.scheduled, [0, 8, 24, 24]);
    assert_eq!(stats.peak, 64);
    for _ in 0..20 {
        app.update();
    }
    assert!(app.world().resource::<VfxStats>().active[2] > 0);
}

#[test]
fn saturated_bursts_keep_feedback_share_assets_and_eventually_expire() {
    let mut app = app();
    let material_count = app.world().resource::<Assets<DustMaterial>>().len();
    let mesh_count = app.world().resource::<Assets<Mesh>>().len();
    for seed in 1..=16 {
        app.world_mut().write_message(burst(seed));
    }
    app.update();
    assert_eq!(app.world().resource::<VfxStats>().peak, PARTICLE_LIMIT);
    for seed in 1..=40 {
        let mut event = burst(seed);
        event.point.x = seed as f32 * 0.2;
        app.world_mut().write_message(event);
    }
    app.update();
    let count = app
        .world_mut()
        .query::<&Particle>()
        .iter(app.world())
        .count();
    assert!(count <= PARTICLE_LIMIT);
    let stats = app.world().resource::<VfxStats>();
    assert_eq!(stats.received, 56);
    assert_eq!(stats.first_feedbacks, 96 + 40 * 2);
    assert_eq!(stats.feedback_bursts, 56);
    assert!(stats.retired > 0);
    assert!(stats.dropped.iter().sum::<u64>() > 0);
    let feedback_positions = app
        .world_mut()
        .query::<(&Particle, &Transform)>()
        .iter(app.world())
        .filter(|(particle, _)| particle.layer == Layer::Impact)
        .map(|(_, transform)| (transform.translation.x * 5.0).round() as usize)
        .collect::<std::collections::BTreeSet<_>>();
    assert!((1..=40).all(|position| feedback_positions.contains(&position)));
    for _ in 0..300 {
        app.update();
    }
    assert_eq!(
        app.world_mut()
            .query::<&Particle>()
            .iter(app.world())
            .count(),
        0
    );
    assert_eq!(
        app.world().resource::<Assets<DustMaterial>>().len(),
        material_count
    );
    assert_eq!(app.world().resource::<Assets<Mesh>>().len(), mesh_count);
    assert_eq!(
        app.world().resource::<VfxAssets>().free_dust.len(),
        DUST_POOL_SIZE
    );
}

#[test]
fn billboards_follow_camera_and_disabling_vfx_releases_the_pool() {
    let mut app = app();
    let rotation = Quat::from_euler(EulerRot::YXZ, 0.8, -0.3, 0.0);
    app.world_mut()
        .spawn((DemoCamera, Transform::from_rotation(rotation)));
    app.world_mut().write_message(burst(1));
    app.update();
    assert!(
        app.world_mut()
            .query::<(&Particle, &Transform)>()
            .iter(app.world())
            .filter(|(p, _)| p.layer != Layer::Debris && p.age >= 0.0)
            .all(|(_, t)| t.rotation.angle_between(rotation) < 0.001)
    );
    app.world_mut().resource_mut::<VfxSettings>().enabled = false;
    app.update();
    assert_eq!(
        app.world_mut()
            .query::<&Particle>()
            .iter(app.world())
            .count(),
        0
    );
    assert_eq!(
        app.world().resource::<VfxAssets>().free_dust.len(),
        DUST_POOL_SIZE
    );
    assert_eq!(app.world().resource::<VfxStats>().peak, 64);
    for _ in 0..90 {
        app.update();
    }
    assert_eq!(
        app.world_mut()
            .query::<&PipelineWarmup>()
            .iter(app.world())
            .count(),
        0
    );
    assert_eq!(
        app.world().resource::<VfxAssets>().free_dust.len(),
        DUST_POOL_SIZE
    );
}

#[test]
fn screen_footprint_and_distance_drive_lod() {
    let camera = Transform::IDENTITY;
    let projection = Projection::Perspective(PerspectiveProjection {
        fov: crate::camera::FIELD_OF_VIEW,
        aspect_ratio: 16.0 / 9.0,
        ..default()
    });
    let footprint = |position| screen_footprint(&camera, Some(&projection), position, 1.0);
    let close = footprint(Vec3::new(0.0, 0.0, -5.0));
    let far = footprint(Vec3::new(0.0, 0.0, -10.0));
    assert!((close / far - 4.0).abs() < 0.001);
    assert_eq!(footprint(Vec3::new(0.0, 0.0, 5.0)), 0.0);
    assert_eq!(footprint(Vec3::new(100.0, 0.0, -5.0)), 0.0);

    let mut near_app = app();
    near_app.world_mut().write_message(burst(1));
    near_app.update();
    let mut far_app = app();
    far_app
        .world_mut()
        .spawn((DemoCamera, Transform::from_xyz(0.0, 0.0, 60.0)));
    far_app.world_mut().write_message(burst(1));
    far_app.update();
    assert_eq!(
        near_app.world().resource::<VfxStats>().spawned,
        [8, 8, 24, 24]
    );
    assert_eq!(far_app.world().resource::<VfxStats>().spawned, [4, 2, 6, 6]);

    let mut saturated_app = app();
    let (mut smoke, mut transform) = spawn_particle(&burst(1), Layer::Smoke, &mut 89);
    smoke.age = 0.2;
    transform.scale = Vec3::splat(12.0);
    saturated_app
        .world_mut()
        .spawn((smoke, transform, Visibility::Visible));
    saturated_app.world_mut().write_message(burst(1));
    saturated_app.update();
    let stats = saturated_app.world().resource::<VfxStats>();
    assert!(stats.estimated_coverage > 0.45);
    assert_eq!(stats.spawned[2], 8);
}

#[test]
fn heavy_debris_bounces_and_releases_when_support_moves_or_is_removed() {
    let mut app = app();
    app.add_plugins((TransformPlugin, PhysicsPlugins::default()));
    app.finish();
    app.cleanup();
    let building = app
        .world_mut()
        .spawn(BuildingInstance::pending(Handle::default()))
        .id();
    let floor_transform = Transform::from_xyz(0.0, 0.5, 0.0);
    let floor = app
        .world_mut()
        .spawn((
            RigidBody::Static,
            Collider::cuboid(3.0, 0.2, 3.0),
            floor_transform,
            ChunkIdentity {
                building,
                id: "floor".into(),
            },
        ))
        .id();
    app.world_mut()
        .get_mut::<BuildingInstance>(building)
        .unwrap()
        .chunks
        .insert(
            "floor".into(),
            crate::scene::ChunkInstance {
                entity: floor,
                primitives: Vec::new(),
                alive: true,
                triangles: Vec::new(),
                transform: floor_transform,
                material: "concrete".into(),
                detail: None,
                cluster: None,
            },
        );
    let mut seed = 71;
    let (mut particle, mut transform) = spawn_particle(&burst(1), Layer::Debris, &mut seed);
    particle.age = 0.0;
    particle.lifetime = 3.0;
    particle.velocity = Vec3::new(0.0, -2.0, 0.0);
    transform.translation = Vec3::new(0.0, 1.5, 0.0);
    particle.sweep_origin = transform.translation;
    let entity = app
        .world_mut()
        .spawn((particle, transform, Visibility::Visible))
        .id();
    for _ in 0..60 {
        app.update();
    }
    assert!(app.world().resource::<VfxStats>().collisions >= 2);
    assert!(app.world().get::<Particle>(entity).unwrap().settled);
    assert!(app.world().get::<Transform>(entity).unwrap().translation.y > 0.6);
    app.world_mut().entity_mut(floor).insert(RigidBody::Dynamic);
    app.update();
    assert!(!app.world().get::<Particle>(entity).unwrap().settled);
    let collisions = app.world().resource::<VfxStats>().collisions;
    app.world_mut()
        .get_mut::<BuildingInstance>(building)
        .unwrap()
        .chunks
        .get_mut("floor")
        .unwrap()
        .alive = false;
    app.world_mut().entity_mut(floor).insert(RemovedChunk);
    for _ in 0..40 {
        app.update();
    }
    assert_eq!(app.world().resource::<VfxStats>().collisions, collisions);
    assert!(app.world().get::<Transform>(entity).unwrap().translation.y < 0.0);
}
