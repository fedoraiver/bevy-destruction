use super::*;
use bevy::camera::CameraProjection;

fn camera(eye: Vec3, target: Vec3) -> VisibilityCamera {
    let pose = Transform::from_translation(eye)
        .looking_at(target, Vec3::Y)
        .compute_affine();
    let view_from_world = pose.inverse();
    VisibilityCamera {
        clip_from_world: PerspectiveProjection {
            fov: std::f32::consts::FRAC_PI_3,
            aspect_ratio: 1.0,
            near: 0.05,
            far: 100.0,
            ..default()
        }
        .get_clip_from_view()
            * Mat4::from(view_from_world),
        view_from_world,
        near: 0.05,
        far: 100.0,
    }
}

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

#[test]
fn opaque_front_surface_hides_rear_and_camera_orbit_reverses_selection() {
    let mut world = World::new();
    let building = world.spawn_empty().id();
    let front = quad(Vec2::splat(-1.0), Vec2::splat(1.0), 0.0);
    let rear = quad(Vec2::splat(-0.8), Vec2::splat(0.8), -1.0);
    let chunks = [
        chunk(building, "front", &front),
        chunk(building, "rear", &rear),
    ];
    let mut cache = ForceVisibilityCache::default();
    let result = cache.update(true, &camera(Vec3::Z * 5.0, Vec3::ZERO), &chunks, &[]);
    assert_eq!(result.visible, BTreeSet::from([chunks[0].key.clone()]));
    assert!(result.stats.rays > 1);
    let result = cache.update(true, &camera(Vec3::NEG_Z * 5.0, Vec3::ZERO), &chunks, &[]);
    assert_eq!(result.visible, BTreeSet::from([chunks[1].key.clone()]));
    assert_eq!(result.stats.cache_hits, 0);
}

#[test]
fn actual_window_hole_reveals_rear_surface_without_hull_occlusion() {
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
        chunk(building, "window", &frame),
        chunk(building, "rear", &rear),
    ];
    let result = ForceVisibilityCache::default().update(
        true,
        &camera(Vec3::Z * 5.0, Vec3::ZERO),
        &chunks,
        &[],
    );
    assert_eq!(result.visible.len(), 2);
    assert!(result.visible.contains(&chunks[1].key));
}

#[test]
fn edge_witness_survives_when_both_triangle_centers_are_occluded() {
    let mut world = World::new();
    let building = world.spawn_empty().id();
    let blocker = world.spawn_empty().id();
    let surface = quad(Vec2::splat(-2.0), Vec2::splat(2.0), -1.0);
    let chunks = [chunk(building, "edge", &surface)];
    let occluder = Collider::cuboid(2.0, 2.0, 0.2);
    let extras = [VisibilityOccluder {
        entity: blocker,
        collider: &occluder,
        world_from_local: Affine3A::IDENTITY,
        revision: 0,
    }];
    let result = ForceVisibilityCache::default().update(
        true,
        &camera(Vec3::Z * 5.0, Vec3::ZERO),
        &chunks,
        &extras,
    );
    assert!(result.visible.contains(&chunks[0].key));
    assert!(
        result.stats.rays >= 5,
        "centers alone must not be the visibility test"
    );
}

#[test]
fn clipping_finds_real_surface_when_original_centroid_is_outside_and_rejects_outside_chunks() {
    let mut world = World::new();
    let building = world.spawn_empty().id();
    let crossing = [[
        Vec3::new(-30.0, -30.0, 0.0),
        Vec3::new(30.0, -30.0, 0.0),
        Vec3::new(0.0, 30.0, 0.0),
    ]];
    let side = quad(Vec2::new(20.0, -1.0), Vec2::new(21.0, 1.0), 0.0);
    let behind = quad(Vec2::splat(-1.0), Vec2::splat(1.0), 10.0);
    let far = quad(Vec2::splat(-1.0), Vec2::splat(1.0), -150.0);
    let chunks = [
        chunk(building, "crossing", &crossing),
        chunk(building, "side", &side),
        chunk(building, "behind", &behind),
        chunk(building, "far", &far),
    ];
    let result = ForceVisibilityCache::default().update(
        true,
        &camera(Vec3::Z * 5.0, Vec3::ZERO),
        &chunks,
        &[],
    );
    assert_eq!(result.visible, BTreeSet::from([chunks[0].key.clone()]));
    assert_eq!(result.stats.frustum_rejected, 3);
}

#[test]
fn render_pose_and_visibility_changes_immediately_discard_cached_selection() {
    let mut world = World::new();
    let building = world.spawn_empty().id();
    let surface = quad(Vec2::splat(-1.0), Vec2::splat(1.0), 0.0);
    let mut chunks = [chunk(building, "moving", &surface)];
    let camera = camera(Vec3::Z * 5.0, Vec3::ZERO);
    let mut cache = ForceVisibilityCache::default();
    assert_eq!(cache.update(true, &camera, &chunks, &[]).visible.len(), 1);
    let reused = cache.update(true, &camera, &chunks, &[]);
    assert_eq!(reused.stats.rays, 0);
    assert_eq!(reused.stats.cache_hits, 1);
    chunks[0].world_from_rest = Affine3A::from_translation(Vec3::X * 20.0);
    assert!(cache.update(true, &camera, &chunks, &[]).visible.is_empty());
    chunks[0].world_from_rest = Affine3A::IDENTITY;
    assert_eq!(cache.update(true, &camera, &chunks, &[]).visible.len(), 1);
    chunks[0].visible = false;
    assert!(cache.update(true, &camera, &chunks, &[]).visible.is_empty());
}

#[test]
fn local_ray_distance_is_converted_to_world_distance_for_nonuniform_render_scale() {
    let mut world = World::new();
    let building = world.spawn_empty().id();
    let blocker = world.spawn_empty().id();
    let surface = quad(Vec2::splat(-0.5), Vec2::splat(0.5), -1.0);
    let mut chunks = [chunk(building, "scaled", &surface)];
    chunks[0].world_from_rest = Affine3A::from_scale(Vec3::new(1.0, 1.0, 2.0));
    let shape = Collider::cuboid(2.0, 2.0, 0.2);
    let extras = [VisibilityOccluder {
        entity: blocker,
        collider: &shape,
        world_from_local: Affine3A::IDENTITY,
        revision: 0,
    }];
    let result = ForceVisibilityCache::default().update(
        true,
        &camera(Vec3::Z * 5.0, Vec3::ZERO),
        &chunks,
        &extras,
    );
    assert!(result.visible.is_empty());
    assert_eq!(result.stats.pending_chunks, 0);
}

#[test]
fn camera_inside_source_uses_boundary_hit_and_not_solid_zero_distance() {
    let mut world = World::new();
    let building = world.spawn_empty().id();
    let outer = quad(Vec2::splat(-2.0), Vec2::splat(2.0), -2.0);
    let inner = quad(Vec2::splat(-2.0), Vec2::splat(2.0), -1.0);
    let chunks = [
        chunk(building, "outer", &outer),
        chunk(building, "inner", &inner),
    ];
    let result = ForceVisibilityCache::default().update(
        true,
        &camera(Vec3::ZERO, Vec3::NEG_Z),
        &chunks,
        &[],
    );
    assert_eq!(result.visible, BTreeSet::from([chunks[1].key.clone()]));
    let enclosure = Collider::cuboid(10.0, 10.0, 10.0);
    let only_inner = [chunk(building, "inside", &inner)];
    let result = ForceVisibilityCache::default().update(
        true,
        &camera(Vec3::ZERO, Vec3::NEG_Z),
        &only_inner,
        &[VisibilityOccluder {
            entity: world.spawn_empty().id(),
            collider: &enclosure,
            world_from_local: Affine3A::IDENTITY,
            revision: 0,
        }],
    );
    assert_eq!(
        result.visible.len(),
        1,
        "the enclosing solid must not report a fictitious hit at the camera"
    );
}

#[test]
fn budget_counts_shape_queries_and_pending_work_resumes_without_stale_visible_bits() {
    let mut world = World::new();
    let building = world.spawn_empty().id();
    let front = quad(Vec2::splat(-1.0), Vec2::splat(1.0), 0.0);
    let rear = quad(Vec2::splat(-0.8), Vec2::splat(0.8), -1.0);
    let chunks = [
        chunk(building, "front", &front),
        chunk(building, "rear", &rear),
    ];
    let camera = camera(Vec3::Z * 5.0, Vec3::ZERO);
    let mut cache = ForceVisibilityCache {
        max_rays: 2,
        ..default()
    };
    let first = cache.update(true, &camera, &chunks, &[]);
    assert_eq!(first.stats.rays, 2);
    assert_eq!(first.stats.pending_chunks, 1);
    assert_eq!(first.visible, BTreeSet::from([chunks[0].key.clone()]));
    // The same small budget eventually checks every hidden-surface witness.
    let mut complete = first.clone();
    for _ in 0..64 {
        complete = cache.update(true, &camera, &chunks, &[]);
        assert!(complete.stats.rays <= 2);
        if complete.stats.pending_chunks == 0 {
            break;
        }
    }
    assert_eq!(complete.stats.pending_chunks, 0);
    assert_eq!(complete.visible, first.visible);
    let reused = cache.update(true, &camera, &chunks, &[]);
    assert_eq!(reused.stats.rays, 0);
    assert_eq!(reused.stats.cache_hits, 2);
    cache.max_rays = 0;
    let unknown = cache.update(true, &camera, &chunks, &[]);
    assert_eq!(unknown.stats.rays, 0);
    assert_eq!(unknown.stats.pending_chunks, 2);
    assert!(unknown.visible.is_empty());
}

#[test]
fn asset_invalidation_rebuilds_source_bvh_and_disabled_mode_does_no_work() {
    let mut world = World::new();
    let building = world.spawn_empty().id();
    let old = quad(Vec2::new(-1.2, -0.2), Vec2::new(-0.8, 0.2), 0.0);
    let new = quad(Vec2::new(0.8, -0.2), Vec2::new(1.2, 0.2), 0.0);
    let camera = camera(Vec3::Z * 5.0, Vec3::ZERO);
    let mut cache = ForceVisibilityCache::default();
    let disabled = cache.update(false, &camera, &[chunk(building, "edited", &old)], &[]);
    assert_eq!(disabled.stats.rays, 0);
    assert!(cache.geometry.is_empty());
    let first = cache.update(true, &camera, &[chunk(building, "edited", &old)], &[]);
    assert_eq!(first.stats.geometry_builds, 1);
    cache.invalidate(AssetId::default());
    assert!(cache.geometry.is_empty());
    let edited = cache.update(true, &camera, &[chunk(building, "edited", &new)], &[]);
    assert_eq!(edited.stats.geometry_builds, 1);
    assert_eq!(edited.stats.cache_hits, 0);
    assert_eq!(edited.visible.len(), 1);
    assert_eq!(cache.update(false, &camera, &[], &[]).stats.rays, 0);
    let resumed = cache.update(true, &camera, &[chunk(building, "edited", &new)], &[]);
    assert_eq!(resumed.stats.rays, 0);
    assert_eq!(resumed.stats.cache_hits, 1);
}

#[test]
fn independent_opaque_collider_revision_invalidates_visibility_without_changing_chunk_pose() {
    let mut world = World::new();
    let building = world.spawn_empty().id();
    let blocker = world.spawn_empty().id();
    let surface = quad(Vec2::splat(-0.5), Vec2::splat(0.5), -1.0);
    let chunks = [chunk(building, "target", &surface)];
    let camera = camera(Vec3::Z * 5.0, Vec3::ZERO);
    let wide = Collider::cuboid(2.0, 2.0, 0.2);
    let tiny = Collider::cuboid(0.01, 0.01, 0.01);
    let mut cache = ForceVisibilityCache::default();
    let first = cache.update(
        true,
        &camera,
        &chunks,
        &[VisibilityOccluder {
            entity: blocker,
            collider: &wide,
            world_from_local: Affine3A::IDENTITY,
            revision: 1,
        }],
    );
    assert!(first.visible.is_empty());
    let changed = cache.update(
        true,
        &camera,
        &chunks,
        &[VisibilityOccluder {
            entity: blocker,
            collider: &tiny,
            world_from_local: Affine3A::IDENTITY,
            revision: 2,
        }],
    );
    assert_eq!(changed.stats.cache_hits, 0);
    assert_eq!(changed.visible.len(), 1);
}
