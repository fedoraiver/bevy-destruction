use super::*;

fn quad(min: Vec2, max: Vec2, z: f32) -> Vec<[Vec3; 3]> {
    let a = min.extend(z);
    let b = Vec3::new(max.x, min.y, z);
    let c = max.extend(z);
    let d = Vec3::new(min.x, max.y, z);
    vec![[a, b, c], [a, c, d]]
}

fn chunk<'a>(building: Entity, name: &str, triangles: &'a [[Vec3; 3]]) -> VisibilityChunk<'a> {
    VisibilityChunk {
        key: (building, name.to_owned()),
        asset: AssetId::default(),
        triangles,
        world_from_rest: Affine3A::IDENTITY,
        visible: true,
    }
}

fn ray(origin: Vec3, target: Vec3) -> Ray3d {
    Ray3d::new(origin, Dir3::new(target - origin).unwrap())
}

#[test]
fn nearest_source_surface_wins_independently_of_chunk_or_building_order() {
    let mut world = World::new();
    let a = world.spawn_empty().id();
    let b = world.spawn_empty().id();
    let front = quad(Vec2::splat(-1.0), Vec2::splat(1.0), 0.0);
    let rear = quad(Vec2::splat(-1.0), Vec2::splat(1.0), -1.0);
    let mut chunks = [chunk(a, "rear", &rear), chunk(b, "front", &front)];
    let forward = ray(Vec3::Z * 5.0, Vec3::ZERO);
    let mut cache = ForceVisibilityCache::default();
    let hit = cache.pick(forward, 20.0, &chunks, &[]).unwrap();
    assert_eq!(hit.key, chunks[1].key);
    assert!(hit.point.distance(Vec3::ZERO) < 1e-5);
    chunks.reverse();
    assert_eq!(
        cache.pick(forward, 20.0, &chunks, &[]).unwrap().key,
        hit.key
    );
    assert_eq!(
        cache
            .pick(ray(Vec3::NEG_Z * 5.0, Vec3::ZERO), 20.0, &chunks, &[])
            .unwrap()
            .key,
        chunks[1].key
    );
}

#[test]
fn window_hole_selects_the_real_rear_surface_instead_of_the_front_bounds() {
    let mut world = World::new();
    let building = world.spawn_empty().id();
    let mut frame = Vec::new();
    for (min, max) in [
        (Vec2::new(-2.0, -2.0), Vec2::new(-0.7, 2.0)),
        (Vec2::new(0.7, -2.0), Vec2::new(2.0, 2.0)),
        (Vec2::new(-0.7, -2.0), Vec2::new(0.7, -0.7)),
        (Vec2::new(-0.7, 0.7), Vec2::new(0.7, 2.0)),
    ] {
        frame.extend(quad(min, max, 0.0));
    }
    let rear = quad(Vec2::splat(-0.4), Vec2::splat(0.4), -1.0);
    let chunks = [
        chunk(building, "frame", &frame),
        chunk(building, "rear", &rear),
    ];
    let mut cache = ForceVisibilityCache::default();
    let hit = cache
        .pick(ray(Vec3::Z * 5.0, Vec3::ZERO), 20.0, &chunks, &[])
        .unwrap();
    assert_eq!(hit.key, chunks[1].key);
    assert!((hit.point.z + 1.0).abs() < 1e-5);
    let hit = cache
        .pick(ray(Vec3::new(1.0, 0.0, 5.0), Vec3::X), 20.0, &chunks, &[])
        .unwrap();
    assert_eq!(hit.key, chunks[0].key);
}

#[test]
fn rendered_affine_immediately_updates_selection_and_converts_nonuniform_scale_distances() {
    let mut world = World::new();
    let building = world.spawn_empty().id();
    let surface = quad(Vec2::splat(-1.0), Vec2::splat(1.0), -1.0);
    let front = quad(Vec2::splat(-1.0), Vec2::splat(1.0), -0.5);
    let mut chunks = [
        chunk(building, "scaled", &surface),
        chunk(building, "front", &front),
    ];
    chunks[0].world_from_rest = Affine3A::from_scale(Vec3::new(1.0, 1.0, 2.0));
    let forward = ray(Vec3::Z * 5.0, Vec3::ZERO);
    let mut cache = ForceVisibilityCache::default();
    assert_eq!(
        cache.pick(forward, 20.0, &chunks, &[]).unwrap().key,
        chunks[1].key
    );
    chunks[1].world_from_rest = Affine3A::from_translation(Vec3::X * 20.0);
    let hit = cache.pick(forward, 20.0, &chunks, &[]).unwrap();
    assert_eq!(hit.key, chunks[0].key);
    assert!(hit.point.distance(Vec3::new(0.0, 0.0, -2.0)) < 1e-5);
    let pose =
        Affine3A::from_rotation_translation(Quat::from_rotation_y(0.7), Vec3::new(17.0, 4.0, -3.0));
    chunks[0].world_from_rest = pose * chunks[0].world_from_rest;
    chunks[1].visible = false;
    let moved_ray = Ray3d::new(
        pose.transform_point3(forward.origin),
        Dir3::new(pose.transform_vector3(*forward.direction)).unwrap(),
    );
    let moved = cache.pick(moved_ray, 20.0, &chunks, &[]).unwrap();
    assert!(moved.point.distance(pose.transform_point3(hit.point)) < 1e-4);
}

#[test]
fn foreground_and_equal_depth_opaque_colliders_block_selection_but_rear_ones_do_not() {
    let mut world = World::new();
    let building = world.spawn_empty().id();
    let surface = quad(Vec2::splat(-1.0), Vec2::splat(1.0), 0.0);
    let chunks = [chunk(building, "surface", &surface)];
    let shape = Collider::cuboid(2.0, 2.0, 0.2);
    let forward = ray(Vec3::Z * 5.0, Vec3::ZERO);
    let mut cache = ForceVisibilityCache::default();
    for z in [1.0, -0.1] {
        assert!(
            cache
                .pick(
                    forward,
                    20.0,
                    &chunks,
                    &[VisibilityOccluder {
                        collider: &shape,
                        world_from_local: Affine3A::from_translation(Vec3::Z * z),
                    }]
                )
                .is_none()
        );
    }
    assert!(
        cache
            .pick(
                forward,
                20.0,
                &chunks,
                &[VisibilityOccluder {
                    collider: &shape,
                    world_from_local: Affine3A::from_translation(Vec3::NEG_Z),
                }]
            )
            .is_some()
    );
}

#[test]
fn camera_inside_an_opaque_shape_does_not_get_a_fictitious_zero_distance_blocker() {
    let mut world = World::new();
    let building = world.spawn_empty().id();
    let surface = quad(Vec2::splat(-1.0), Vec2::splat(1.0), -1.0);
    let chunks = [chunk(building, "inside", &surface)];
    let enclosure = Collider::cuboid(10.0, 10.0, 10.0);
    let hit = ForceVisibilityCache::default()
        .pick(
            ray(Vec3::ZERO, Vec3::NEG_Z),
            20.0,
            &chunks,
            &[VisibilityOccluder {
                collider: &enclosure,
                world_from_local: Affine3A::IDENTITY,
            }],
        )
        .unwrap();
    assert_eq!(hit.key, chunks[0].key);
}

#[test]
fn chunk_ties_are_conservative_but_a_later_strictly_nearer_surface_clears_the_tie() {
    let mut world = World::new();
    let building = world.spawn_empty().id();
    let same = quad(Vec2::splat(-1.0), Vec2::splat(1.0), 0.0);
    let nearer = quad(Vec2::splat(-1.0), Vec2::splat(1.0), 1.0);
    let mut chunks = vec![chunk(building, "a", &same), chunk(building, "b", &same)];
    let forward = ray(Vec3::Z * 5.0, Vec3::ZERO);
    let mut cache = ForceVisibilityCache::default();
    assert!(cache.pick(forward, 20.0, &chunks, &[]).is_none());
    chunks.push(chunk(building, "nearer", &nearer));
    assert_eq!(
        cache.pick(forward, 20.0, &chunks, &[]).unwrap().key,
        chunks[2].key
    );
    chunks.reverse();
    assert_eq!(
        cache.pick(forward, 20.0, &chunks, &[]).unwrap().key,
        chunks[0].key
    );
}

#[test]
fn almost_coplanar_tie_chains_do_not_depend_on_input_order() {
    let mut world = World::new();
    let building = world.spawn_empty().id();
    let a = quad(Vec2::splat(-1.0), Vec2::splat(1.0), 0.0);
    let b = quad(Vec2::splat(-1.0), Vec2::splat(1.0), 3e-6);
    let c = quad(Vec2::splat(-1.0), Vec2::splat(1.0), 6e-6);
    let sources = [("a", &a), ("b", &b), ("c", &c)];
    let forward = ray(Vec3::Z * 5.0, Vec3::ZERO);
    let mut cache = ForceVisibilityCache::default();
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let chunks = order.map(|index| chunk(building, sources[index].0, sources[index].1));
        assert!(cache.pick(forward, 20.0, &chunks, &[]).is_none());
    }
}

#[test]
fn instances_of_one_asset_share_rest_bvh_and_keep_independent_render_poses() {
    let mut world = World::new();
    let near_building = world.spawn_empty().id();
    let far_building = world.spawn_empty().id();
    let source = quad(Vec2::splat(-1.0), Vec2::splat(1.0), 0.0);
    let mut chunks = [
        chunk(far_building, "shared", &source),
        chunk(near_building, "shared", &source),
    ];
    chunks[0].world_from_rest = Affine3A::from_translation(Vec3::NEG_Z * 2.0);
    let forward = ray(Vec3::Z * 5.0, Vec3::ZERO);
    let mut cache = ForceVisibilityCache::default();
    assert_eq!(
        cache.pick(forward, 20.0, &chunks, &[]).unwrap().key,
        chunks[1].key
    );
    assert_eq!(cache.geometry.len(), 1);
    chunks[1].world_from_rest = Affine3A::from_translation(Vec3::X * 20.0);
    let hit = cache.pick(forward, 20.0, &chunks, &[]).unwrap();
    assert_eq!(hit.key, chunks[0].key);
    assert!((hit.point.z + 2.0).abs() < 1e-5);
}

#[test]
fn hidden_chunks_misses_and_distance_limits_never_select_a_stale_surface() {
    let mut world = World::new();
    let building = world.spawn_empty().id();
    let surface = quad(Vec2::splat(-1.0), Vec2::splat(1.0), 0.0);
    let mut chunks = [chunk(building, "target", &surface)];
    let forward = ray(Vec3::Z * 5.0, Vec3::ZERO);
    let mut cache = ForceVisibilityCache::default();
    assert!(cache.pick(forward, 4.9, &chunks, &[]).is_none());
    assert!(cache.pick(forward, f32::NAN, &chunks, &[]).is_none());
    assert!(cache.pick(forward, 0.0, &chunks, &[]).is_none());
    assert!(cache.pick(forward, 5.1, &chunks, &[]).is_some());
    chunks[0].visible = false;
    assert!(cache.pick(forward, 20.0, &chunks, &[]).is_none());
    chunks[0].visible = true;
    assert!(
        cache
            .pick(
                Ray3d::new(Vec3::new(3.0, 0.0, 5.0), Dir3::NEG_Z),
                20.0,
                &chunks,
                &[]
            )
            .is_none()
    );
}

#[test]
fn asset_invalidation_replaces_source_geometry_while_off_ray_meshes_do_not_build_bvhs() {
    let mut world = World::new();
    let building = world.spawn_empty().id();
    let old = quad(Vec2::splat(-1.0), Vec2::splat(1.0), 0.0);
    let changed = quad(Vec2::splat(-1.0), Vec2::splat(1.0), -2.0);
    let side = quad(Vec2::new(10.0, -1.0), Vec2::new(12.0, 1.0), 0.0);
    let forward = ray(Vec3::Z * 5.0, Vec3::ZERO);
    let mut cache = ForceVisibilityCache::default();
    let chunks = [
        chunk(building, "target", &old),
        chunk(building, "side", &side),
    ];
    assert!(
        cache
            .pick(forward, 20.0, &chunks, &[])
            .unwrap()
            .point
            .z
            .abs()
            < 1e-5
    );
    assert!(
        cache.geometry[&(AssetId::default(), "side".into())]
            .as_ref()
            .unwrap()
            .collider
            .is_none()
    );
    cache.invalidate(AssetId::default());
    assert!(cache.geometry.is_empty());
    let hit = cache
        .pick(forward, 20.0, &[chunk(building, "target", &changed)], &[])
        .unwrap();
    assert!((hit.point.z + 2.0).abs() < 1e-5);
}

#[test]
fn malformed_visible_assets_and_singular_poses_cannot_pick_through_to_a_rear_chunk() {
    let mut world = World::new();
    let building = world.spawn_empty().id();
    let malformed = [[Vec3::NAN, Vec3::X, Vec3::Y]];
    let rear = quad(Vec2::splat(-1.0), Vec2::splat(1.0), -1.0);
    let forward = ray(Vec3::Z * 5.0, Vec3::ZERO);
    let mut chunks = [
        chunk(building, "invalid", &malformed),
        chunk(building, "rear", &rear),
    ];
    let mut cache = ForceVisibilityCache::default();
    assert!(cache.pick(forward, 20.0, &chunks, &[]).is_none());
    chunks[0].visible = false;
    assert_eq!(
        cache.pick(forward, 20.0, &chunks, &[]).unwrap().key,
        chunks[1].key
    );
    chunks[1].world_from_rest = Affine3A::from_scale(Vec3::new(1.0, 1.0, 0.0));
    assert!(cache.pick(forward, 20.0, &chunks, &[]).is_none());
}

#[test]
fn transformed_extra_occluders_use_render_pose_and_world_ray_distance() {
    let mut world = World::new();
    let building = world.spawn_empty().id();
    let surface = quad(Vec2::splat(-0.5), Vec2::splat(0.5), -2.0);
    let chunks = [chunk(building, "target", &surface)];
    let shape = Collider::cuboid(2.0, 2.0, 0.2);
    let forward = ray(Vec3::Z * 5.0, Vec3::ZERO);
    let mut cache = ForceVisibilityCache::default();
    let scaled_behind = Affine3A::from_scale_rotation_translation(
        Vec3::new(1.0, 1.0, 2.0),
        Quat::IDENTITY,
        Vec3::Z * -3.0,
    );
    assert!(
        cache
            .pick(
                forward,
                20.0,
                &chunks,
                &[VisibilityOccluder {
                    collider: &shape,
                    world_from_local: scaled_behind,
                }]
            )
            .is_some()
    );
    assert!(
        cache
            .pick(
                forward,
                20.0,
                &chunks,
                &[VisibilityOccluder {
                    collider: &shape,
                    world_from_local: Affine3A::from_rotation_translation(
                        Quat::from_rotation_y(0.7),
                        Vec3::ZERO
                    ),
                }]
            )
            .is_none()
    );
}
