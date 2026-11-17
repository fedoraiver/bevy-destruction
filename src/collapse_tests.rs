use super::*;
use crate::{
    asset::{ChunkRecord, Manifest},
    scene::{BuildingBounds, ChunkIdentity, ChunkInstance},
};
use avian3d::collision::contact_types::ContactGraph;
use bevy::time::TimeUpdateStrategy;

fn supported_rectangle(keep_support: impl Fn(&ChunkRecord) -> bool) -> (App, Entity) {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, TransformPlugin, PhysicsPlugins::default()))
        .init_resource::<Assets<DestructibleAsset>>()
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .insert_resource(Time::<Fixed>::from_hz(60.0))
        .insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f64(1.0 / 60.0),
        ))
        .add_plugins((physics::DestructionPhysicsPlugin, StressRuntimePlugin));
    app.finish();
    app.cleanup();
    app.update();
    let manifest: Manifest = serde_json::from_str(include_str!(
        "../tests/fixtures/rectangle-v1.destruction.json"
    ))
    .unwrap();
    let asset = app
        .world_mut()
        .resource_mut::<Assets<DestructibleAsset>>()
        .add(DestructibleAsset {
            manifest: manifest.clone(),
            gltf: Handle::default(),
        });
    let root = app
        .world_mut()
        .spawn((Transform::IDENTITY, Visibility::Visible))
        .id();
    let mut instance = BuildingInstance::pending(asset);
    instance.bounds = Some(BuildingBounds {
        min: Vec3::new(0.0, 0.0, -6.0),
        max: Vec3::new(8.0, 6.24, 0.0),
    });
    instance.topology_dirty = true;
    instance.bonds = manifest
        .bonds
        .iter()
        .map(|bond| (bond.id.clone(), true))
        .collect();
    for record in &manifest.chunks {
        let alive = !record.part_id.contains("-l00") || keep_support(record);
        let local = record.transform();
        let entity = app
            .world_mut()
            .spawn((
                local,
                GlobalTransform::from(local),
                ChildOf(root),
                ChunkIdentity {
                    building: root,
                    id: record.id.clone(),
                },
            ))
            .id();
        if alive {
            let shapes = record
                .colliders
                .iter()
                .map(|proxy| {
                    let points = proxy
                        .vertices
                        .iter()
                        .copied()
                        .map(Vec3::from_array)
                        .collect::<Vec<_>>();
                    (
                        Vec3::ZERO,
                        Quat::IDENTITY,
                        Collider::convex_hull(&points).unwrap(),
                    )
                })
                .collect();
            app.world_mut()
                .entity_mut(entity)
                .insert((Collider::compound(shapes), ColliderDensity(0.0)));
        }
        instance.chunks.insert(
            record.id.clone(),
            ChunkInstance {
                entity,
                primitives: Vec::new(),
                alive,
                triangles: Vec::new(),
                transform: local,
                material: record.material.clone(),
                detail: None,
                cluster: None,
            },
        );
    }
    for bond in &manifest.bonds {
        if !instance.chunks[&bond.a].alive || !instance.chunks[&bond.b].alive {
            instance.bonds.insert(bond.id.clone(), false);
        }
    }
    app.world_mut().entity_mut(root).insert(instance);
    (app, root)
}

#[derive(Debug, Default)]
struct BulkMotion {
    largest_released_upper: usize,
    max_drop: f32,
    max_tilt: f32,
    strength_breaks: u64,
    first_stub_contact: Option<(usize, f32)>,
}

fn simulate_collapse(
    app: &mut App,
    root: Entity,
    frames: usize,
    static_stub: Option<&str>,
) -> BulkMotion {
    let manifest = app
        .world()
        .resource::<Assets<DestructibleAsset>>()
        .get(&app.world().get::<BuildingInstance>(root).unwrap().asset)
        .unwrap()
        .manifest
        .clone();
    let expected_mass = manifest
        .chunks
        .iter()
        .filter(|record| {
            app.world().get::<BuildingInstance>(root).unwrap().chunks[&record.id].alive
        })
        .map(|record| f64::from(record.mass))
        .sum::<f64>();
    let upper = manifest
        .chunks
        .iter()
        .filter_map(|record| {
            let center = record
                .transform()
                .transform_point(record.center_of_mass.into());
            (center.y >= 3.0).then_some((record.id.clone(), (center, record.mass)))
        })
        .collect::<BTreeMap<_, _>>();
    let mut motion = BulkMotion::default();
    for frame in 0..frames {
        app.update();
        let instance = app.world().get::<BuildingInstance>(root).unwrap();
        let stub = static_stub.map(|id| {
            let stub = &instance.chunks[id];
            let body = stub.cluster.unwrap();
            assert!(stub.alive);
            assert!(app.world().get::<physics::Cluster>(body).unwrap().anchored);
            assert_eq!(app.world().get::<RigidBody>(body), Some(&RigidBody::Static));
            stub.entity
        });
        let mut total_mass = 0.0_f64;
        for &body in &instance.clusters {
            let rotation = app.world().get::<Rotation>(body).unwrap().0;
            let position = app.world().get::<Position>(body).unwrap().0;
            assert!(rotation.is_finite() && position.is_finite());
            assert!(
                app.world()
                    .get::<LinearVelocity>(body)
                    .unwrap()
                    .0
                    .is_finite()
            );
            assert!(
                app.world()
                    .get::<AngularVelocity>(body)
                    .unwrap()
                    .0
                    .is_finite()
            );
            total_mass += f64::from(app.world().get::<Mass>(body).unwrap().0);
            let chunks = &app.world().get::<physics::Cluster>(body).unwrap().chunks;
            let tracked = chunks
                .iter()
                .filter_map(|id| upper.get(id))
                .collect::<Vec<_>>();
            if tracked.len() < 20 || app.world().get::<RigidBody>(body) != Some(&RigidBody::Dynamic)
            {
                continue;
            }
            motion.largest_released_upper = motion.largest_released_upper.max(tracked.len());
            let initial = tracked
                .iter()
                .map(|(center, mass)| *center * *mass)
                .sum::<Vec3>()
                / tracked.iter().map(|(_, mass)| *mass).sum::<f32>();
            motion.max_tilt = motion
                .max_tilt
                .max((rotation * Vec3::Y).angle_between(Vec3::Y));
            let center = rotation * initial + position;
            motion.max_drop = motion.max_drop.max(initial.y - center.y);
            if let Some(stub) = stub {
                for pair in app
                    .world()
                    .resource::<ContactGraph>()
                    .iter_active_touching()
                {
                    if [pair.collider1, pair.collider2].contains(&stub)
                        && [pair.body1, pair.body2].contains(&Some(body))
                        && pair.total_normal_impulse_magnitude() > 0.0
                    {
                        motion
                            .first_stub_contact
                            .get_or_insert((frame, pair.total_normal_impulse_magnitude()));
                    }
                }
            }
        }
        assert!((total_mass - expected_mass).abs() < expected_mass * 1e-5);
    }
    let state = app.world().get::<StressState>(root).unwrap();
    motion.strength_breaks = state.broken_total;
    assert_eq!(
        state.failures, 0,
        "Valid damaged building must remain solvable"
    );
    assert!(state.reports.iter().all(|report| report.valid));
    motion
}

#[test]
fn overloaded_narrow_wall_releases_and_drops_the_real_upper_building() {
    let (mut app, root) =
        supported_rectangle(|chunk| chunk.part_id == "wall-inner-0-l00-p0000-z00");
    let motion = simulate_collapse(&mut app, root, 240, None);
    assert!(
        motion.largest_released_upper >= 20 && motion.max_drop > 0.5 && motion.strength_breaks > 0,
        "Upper building did not release and fall: {motion:?}"
    );
}

#[test]
fn asymmetric_static_wall_stub_naturally_tips_the_released_upper_building() {
    let (mut app, root) =
        supported_rectangle(|chunk| chunk.part_id == "wall-inner-0-l00-p0006-z00");
    let motion = simulate_collapse(
        &mut app,
        root,
        240,
        Some("wall-inner-0-l00-p0006-z00:voronoi:7:0001"),
    );
    assert!(
        motion.largest_released_upper >= 20
            && motion.max_drop > 0.5
            && motion.max_tilt > 0.1
            && motion.strength_breaks > 0
            && motion.first_stub_contact.is_some(),
        "Released upper structure did not tip against the retained static wall stub: {motion:?}"
    );
}

#[test]
fn partially_removed_first_storey_remains_supported_with_a_valid_solution() {
    let (mut app, root) = supported_rectangle(|chunk| chunk.position[0] >= 4.04);
    assert_eq!(
        app.world()
            .get::<BuildingInstance>(root)
            .unwrap()
            .chunks
            .values()
            .filter(|chunk| chunk.alive)
            .count(),
        217
    );
    let motion = simulate_collapse(&mut app, root, 120, None);
    assert_eq!(motion.strength_breaks, 0);
    assert_eq!(motion.largest_released_upper, 0);
}
