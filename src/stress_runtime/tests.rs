use super::*;
use crate::{
    physics::{Cluster, DestructionPhysicsPlugin},
    stress_smoke::{BEAM, RIGHT_BOND, beam_manifest, remove_left_support, spawn_beam},
};
use bevy::time::TimeUpdateStrategy;

fn app(hz: f64) -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, TransformPlugin, PhysicsPlugins::default()))
        .init_resource::<Assets<DestructibleAsset>>()
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .insert_resource(Time::<Fixed>::from_hz(hz))
        .insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f64(1.0 / hz),
        ))
        .add_plugins((DestructionPhysicsPlugin, StressRuntimePlugin));
    app.finish();
    app.cleanup();
    app.update();
    app
}

fn tick(app: &mut App, count: usize) {
    for _ in 0..count {
        app.update();
    }
}

fn body(world: &World, root: Entity) -> Entity {
    world.get::<BuildingInstance>(root).unwrap().chunks[BEAM]
        .cluster
        .unwrap()
}

#[test]
fn overload_releases_a_still_connected_beam_and_keeps_shared_asset_instance_stable() {
    let mut app = app(60.0);
    app.world_mut()
        .resource_mut::<StressSettings>()
        .immediate_utilization = 100.0;
    app.world_mut()
        .resource_mut::<StressSettings>()
        .damage_seconds = 2.0;
    let damaged = spawn_beam(app.world_mut(), Vec3::ZERO);
    let intact = spawn_beam(app.world_mut(), Vec3::new(8.0, 0.0, 0.0));
    let shared_asset = app
        .world()
        .get::<BuildingInstance>(damaged)
        .unwrap()
        .asset
        .clone();
    app.world_mut()
        .get_mut::<BuildingInstance>(intact)
        .unwrap()
        .asset = shared_asset;
    tick(&mut app, 5);
    let original = body(app.world(), damaged);
    assert_eq!(
        app.world().get::<RigidBody>(original),
        Some(&RigidBody::Static)
    );
    for root in [damaged, intact] {
        let state = app.world().get::<StressState>(root).unwrap();
        assert!(state.reports.iter().all(|report| report.valid));
        assert!(state.max_utilization < 1.0);
        assert_eq!(state.broken_total, 0);
        assert!(
            state
                .reports
                .iter()
                .all(|report| report.force_balance_error < 0.05
                    && report.moment_balance_error < 0.1
                    && report.max_displacement.is_finite())
        );
        assert!(state.bonds.values().all(|bond| bond.stress.section_exact));
    }
    remove_left_support(app.world_mut(), damaged);
    tick(&mut app, 2);
    let instance = app.world().get::<BuildingInstance>(damaged).unwrap();
    assert!(
        instance.bonds[RIGHT_BOND],
        "Fixture lost support before stress-driven fracture"
    );
    let state = app.world().get::<StressState>(damaged).unwrap();
    let bond = &state.bonds[RIGHT_BOND];
    assert!(bond.valid && bond.stress.utilization > 1.0);
    let weight = 288.0 * 9.81;
    assert!((bond.stress.force[1].abs() - weight).abs() < 0.01);
    assert!((bond.stress.moment[2].abs() - weight).abs() < 0.01);
    assert!(bond.damage > 0.0 && bond.damage < 1.0);
    assert_eq!(
        app.world().get::<RigidBody>(body(app.world(), damaged)),
        Some(&RigidBody::Static)
    );
    tick(&mut app, 90);
    let state = app.world().get::<StressState>(damaged).unwrap();
    assert_eq!(state.broken_total, 1);
    assert!(!app.world().get::<BuildingInstance>(damaged).unwrap().bonds[RIGHT_BOND]);
    let released = body(app.world(), damaged);
    assert_ne!(released, original);
    assert_eq!(
        app.world().get::<RigidBody>(released),
        Some(&RigidBody::Dynamic)
    );
    assert!(!app.world().get::<Cluster>(released).unwrap().anchored);
    let intact_state = app.world().get::<StressState>(intact).unwrap();
    assert_eq!(intact_state.broken_total, 0);
    assert!(intact_state.max_utilization < 1.0);
    assert!(
        app.world()
            .get::<BuildingInstance>(intact)
            .unwrap()
            .bonds
            .values()
            .all(|active| *active)
    );
}

fn rupture_time(hz: f64) -> f64 {
    let mut app = app(hz);
    {
        let mut settings = app.world_mut().resource_mut::<StressSettings>();
        settings.immediate_utilization = 100.0;
        settings.damage_seconds = 2.0;
    }
    let root = spawn_beam(app.world_mut(), Vec3::ZERO);
    tick(&mut app, 5);
    remove_left_support(app.world_mut(), root);
    for tick_index in 1..=120 {
        app.update();
        if app.world().get::<StressState>(root).unwrap().broken_total > 0 {
            return tick_index as f64 / hz;
        }
    }
    panic!("Progressive overload did not fracture at {hz}Hz");
}

#[test]
fn initial_overload_can_fracture_before_the_first_cluster_exists() {
    let mut app = app(60.0);
    app.world_mut()
        .resource_mut::<StressSettings>()
        .immediate_utilization = 2.0;
    let root = spawn_beam(app.world_mut(), Vec3::ZERO);
    remove_left_support(app.world_mut(), root);
    assert!(
        app.world()
            .get::<BuildingInstance>(root)
            .unwrap()
            .clusters
            .is_empty()
    );
    tick(&mut app, 3);
    assert_eq!(
        app.world().get::<StressState>(root).unwrap().broken_total,
        1
    );
    let released = body(app.world(), root);
    assert_eq!(
        app.world().get::<RigidBody>(released),
        Some(&RigidBody::Dynamic)
    );
    assert!(!app.world().get::<Cluster>(released).unwrap().anchored);
}

#[test]
fn self_weight_uses_collider_mass_and_shifts_moment_to_the_structural_node() {
    let mut app = app(60.0);
    app.insert_resource(Gravity(Vec3::NEG_Y * 10.0));
    let root = spawn_beam(app.world_mut(), Vec3::ZERO);
    remove_left_support(app.world_mut(), root);

    let collider = Collider::compound(vec![(
        Vec3::X * 0.25,
        Quat::IDENTITY,
        Collider::cuboid(3.0, 0.2, 0.1),
    )]);
    let density = 2400.0;
    let properties = collider.mass_properties(density);
    let entity = {
        let mut instance = app.world_mut().get_mut::<BuildingInstance>(root).unwrap();
        let chunk = instance.chunks.get_mut(BEAM).unwrap();
        chunk.mass_properties = properties;
        chunk.entity
    };
    app.world_mut()
        .entity_mut(entity)
        .insert((collider, ColliderDensity(density)));

    update_stress(app.world_mut());
    let stress = app.world().get::<StressState>(root).unwrap();
    let bond = &stress.bonds[RIGHT_BOND].stress;
    assert!((bond.force[1].abs() - 1440.0).abs() < 1e-3);
    assert!((bond.moment[2].abs() - 1080.0).abs() < 1e-3);
    assert!(stress.reports[0].force_balance_error < 1e-6);
    assert!(stress.reports[0].moment_balance_error < 1e-6);
}

#[test]
fn progressive_failure_uses_elapsed_fixed_time() {
    let at_30 = rupture_time(30.0);
    let at_120 = rupture_time(120.0);
    assert!(
        (at_30 - at_120).abs() <= 1.0 / 30.0 + 1e-6,
        "Failure time depends on tick rate: {at_30} vs {at_120}"
    );
}

#[test]
fn healthy_cache_tracks_load_gravity_settings_and_topology_changes() {
    let mut app = app(60.0);
    app.world_mut()
        .resource_mut::<StressSettings>()
        .immediate_utilization = 1000.0;
    app.world_mut()
        .resource_mut::<StressSettings>()
        .damage_seconds = 1000.0;
    let root = spawn_beam(app.world_mut(), Vec3::ZERO);
    tick(&mut app, 5);
    let initial = app.world().get::<StressState>(root).unwrap();
    let solves = initial.solves;
    let initial_force = initial.bonds[RIGHT_BOND].stress.force[1].abs();
    tick(&mut app, 120);
    let unchanged = app.world().get::<StressState>(root).unwrap();
    assert_eq!(unchanged.solves, solves);
    assert_eq!(
        unchanged.bonds[RIGHT_BOND].stress.force[1].abs(),
        initial_force
    );

    app.world_mut()
        .resource_mut::<StructuralLoads>()
        .by_building
        .entry(root)
        .or_default()
        .insert(
            BEAM.into(),
            NodeLoad {
                force: [0.0, -1000.0, 0.0],
                moment: [0.0; 3],
            },
        );
    tick(&mut app, 3);
    let loaded = app.world().get::<StressState>(root).unwrap();
    assert_eq!(loaded.solves, solves + 1);
    let loaded_force = loaded.bonds[RIGHT_BOND].stress.force[1].abs();
    assert!(loaded_force > initial_force + 400.0);
    app.world_mut().resource_mut::<Gravity>().0 *= 2.0;
    tick(&mut app, 3);
    let heavier = app.world().get::<StressState>(root).unwrap();
    assert_eq!(heavier.solves, solves + 2);
    assert!(heavier.bonds[RIGHT_BOND].stress.force[1].abs() > loaded_force + 1000.0);

    app.world_mut()
        .resource_mut::<StressSettings>()
        .solver
        .relative_tolerance = 1e-7;
    tick(&mut app, 3);
    assert_eq!(
        app.world().get::<StressState>(root).unwrap().solves,
        solves + 3
    );
    remove_left_support(app.world_mut(), root);
    tick(&mut app, 3);
    let cantilever = app.world().get::<StressState>(root).unwrap();
    assert_eq!(cantilever.solves, solves + 4);
    assert!(cantilever.bonds[RIGHT_BOND].stress.moment[2].abs() > 5000.0);
    assert_eq!(cantilever.broken_total, 0);
}

#[test]
fn immediate_threshold_below_one_applies_before_and_after_cache_warmup() {
    let mut warm = app(60.0);
    let root = spawn_beam(warm.world_mut(), Vec3::ZERO);
    tick(&mut warm, 5);
    let state = warm.world().get::<StressState>(root).unwrap();
    assert_eq!(state.broken_total, 0);
    assert!(state.max_utilization > 0.0 && state.max_utilization < 1.0);
    let threshold = state.max_utilization * 0.5;
    let numerical_solves = state.numerical_solves;
    warm.world_mut()
        .resource_mut::<StressSettings>()
        .immediate_utilization = threshold;
    tick(&mut warm, 1);
    let state = warm.world().get::<StressState>(root).unwrap();
    assert!(state.broken_total > 0);
    assert_eq!(state.numerical_solves, numerical_solves);
    assert!(
        warm.world()
            .get::<BuildingInstance>(root)
            .unwrap()
            .bonds
            .values()
            .any(|active| !active)
    );

    let mut cold = app(60.0);
    cold.world_mut()
        .resource_mut::<StressSettings>()
        .immediate_utilization = threshold;
    let root = spawn_beam(cold.world_mut(), Vec3::ZERO);
    tick(&mut cold, 1);
    let state = cold.world().get::<StressState>(root).unwrap();
    assert!(state.broken_total > 0);
    assert_eq!(state.numerical_solves, 1);
    assert!(state.max_utilization < 1.0);
}

#[test]
fn detached_debris_changes_reuse_the_unchanged_anchored_solution() {
    use crate::scene::{ChunkIdentity, ChunkInstance};
    let mut app = app(60.0);
    let root = spawn_beam(app.world_mut(), Vec3::ZERO);
    let asset = app
        .world()
        .get::<BuildingInstance>(root)
        .unwrap()
        .asset
        .clone();
    let mut record = app
        .world()
        .resource::<Assets<DestructibleAsset>>()
        .get(&asset)
        .unwrap()
        .manifest
        .chunks
        .iter()
        .find(|chunk| chunk.id == BEAM)
        .unwrap()
        .clone();
    record.id = "floating".into();
    record.node = "floating".into();
    record.position = [8.0, 6.0, 0.0];
    let transform = record.transform();
    let collider = Collider::cuboid(3.0, 0.2, 0.2);
    let density = app
        .world()
        .resource::<Assets<DestructibleAsset>>()
        .get(&asset)
        .unwrap()
        .manifest
        .materials[&record.material]
        .density;
    let mass_properties = collider.mass_properties(density);
    let entity = app
        .world_mut()
        .spawn((
            transform,
            ChildOf(root),
            collider,
            ColliderDensity(density),
            ChunkIdentity {
                building: root,
                id: "floating".into(),
            },
        ))
        .id();
    app.world_mut()
        .resource_mut::<Assets<DestructibleAsset>>()
        .get_mut(&asset)
        .unwrap()
        .manifest
        .chunks
        .push(record);
    app.world_mut()
        .get_mut::<BuildingInstance>(root)
        .unwrap()
        .chunks
        .insert(
            "floating".into(),
            ChunkInstance {
                entity,
                primitives: Vec::new(),
                alive: true,
                triangles: Vec::new(),
                transform,
                mass_properties,
                material: "concrete".into(),
                detail: None,
                cluster: None,
            },
        );
    tick(&mut app, 5);
    let state = app.world().get::<StressState>(root).unwrap();
    let solves = state.numerical_solves;
    let force = state.bonds[RIGHT_BOND].stress.force;
    let supported = body(app.world(), root);
    {
        let mut instance = app.world_mut().get_mut::<BuildingInstance>(root).unwrap();
        instance.chunks.get_mut("floating").unwrap().alive = false;
        instance.topology_dirty = true;
    }
    tick(&mut app, 3);
    let state = app.world().get::<StressState>(root).unwrap();
    assert_eq!(state.numerical_solves, solves);
    assert_eq!(state.bonds[RIGHT_BOND].stress.force, force);
    assert_eq!(body(app.world(), root), supported);
    assert!(state.reports.iter().all(|report| report.valid));
    assert!(app.world().get::<Collider>(entity).is_none());
    app.world_mut()
        .resource_mut::<StructuralLoads>()
        .by_building
        .entry(root)
        .or_default()
        .insert(
            BEAM.into(),
            NodeLoad {
                force: [0.0, -1000.0, 0.0],
                moment: [0.0; 3],
            },
        );
    tick(&mut app, 3);
    let state = app.world().get::<StressState>(root).unwrap();
    assert_eq!(state.numerical_solves, solves + 1);
    assert!(state.bonds[RIGHT_BOND].stress.force[1].abs() > force[1].abs() + 400.0);
}

#[test]
fn failed_solve_does_not_break_bonds_and_recovers_after_settings_change() {
    let mut app = app(60.0);
    app.world_mut()
        .resource_mut::<StressSettings>()
        .solver
        .max_iterations = 0;
    let root = spawn_beam(app.world_mut(), Vec3::ZERO);
    tick(&mut app, 4);
    let state = app.world().get::<StressState>(root).unwrap();
    assert!(state.failures > 0);
    assert!(
        state
            .reports
            .iter()
            .any(|report| !report.valid && report.error.is_some())
    );
    assert_eq!(state.broken_total, 0);
    assert!(
        app.world()
            .get::<BuildingInstance>(root)
            .unwrap()
            .bonds
            .values()
            .all(|active| *active)
    );
    app.world_mut()
        .resource_mut::<StressSettings>()
        .solver
        .max_iterations = 3000;
    tick(&mut app, 3);
    assert!(
        app.world()
            .get::<StressState>(root)
            .unwrap()
            .reports
            .iter()
            .all(|report| report.valid)
    );
    assert_eq!(
        app.world().get::<StressState>(root).unwrap().broken_total,
        0
    );
}

#[test]
fn free_fall_has_no_static_selfweight_fracture_or_damage() {
    let mut app = app(60.0);
    let root = spawn_beam(app.world_mut(), Vec3::new(0.0, 8.0, 0.0));
    {
        let mut instance = app.world_mut().get_mut::<BuildingInstance>(root).unwrap();
        let asset = instance.asset.clone();
        instance.topology_dirty = true;
        app.world_mut()
            .resource_mut::<Assets<DestructibleAsset>>()
            .get_mut(&asset)
            .unwrap()
            .manifest
            .anchors
            .clear();
    }
    tick(&mut app, 30);
    let state = app.world().get::<StressState>(root).unwrap();
    assert_eq!(state.broken_total, 0);
    assert!(state.skipped_dynamic > 0);
    assert!(
        state
            .bonds
            .values()
            .all(|bond| !bond.valid && bond.damage == 0.0)
    );
    let cluster = body(app.world(), root);
    assert_eq!(
        app.world().get::<RigidBody>(cluster),
        Some(&RigidBody::Dynamic)
    );
    assert!(app.world().get::<LinearVelocity>(cluster).unwrap().y < -1.0);
}

#[test]
fn acceptance_fixture_uses_valid_si_mass_and_section_data() {
    let manifest = beam_manifest();
    manifest.validate().unwrap();
    assert!(
        (manifest
            .chunks
            .iter()
            .find(|chunk| chunk.id == BEAM)
            .unwrap()
            .mass
            - 288.0)
            .abs()
            < 1e-3
    );
}

mod collapse {
    #[test]
    fn retained_failure_graph_uses_current_schema_and_real_interfaces() {
        let manifest: crate::asset::Manifest = serde_json::from_str(include_str!(
            "../stress/fixtures/damaged-support.destruction.json"
        ))
        .unwrap();
        assert_eq!(manifest.schema_version, 2);
        assert!(manifest.bonds.iter().all(|bond| bond.patches.len() == 1));
        manifest.validate().unwrap();
    }

    use super::*;
    use crate::{
        asset::{ChunkRecord, Manifest},
        scene::{BuildingBounds, ChunkIdentity, ChunkInstance, chunk_collider},
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
            "../stress/fixtures/damaged-support.destruction.json"
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
            let collider = chunk_collider(record).unwrap();
            let density = manifest.materials[&record.material].density;
            let mass_properties = collider.mass_properties(density);
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
                app.world_mut()
                    .entity_mut(entity)
                    .insert((collider, ColliderDensity(density)));
            }
            instance.chunks.insert(
                record.id.clone(),
                ChunkInstance {
                    entity,
                    primitives: Vec::new(),
                    alive,
                    triangles: Vec::new(),
                    transform: local,
                    mass_properties,
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
        let instance = app.world().get::<BuildingInstance>(root).unwrap();
        let expected_mass = instance
            .chunks
            .values()
            .filter(|chunk| chunk.alive)
            .map(|chunk| f64::from(chunk.mass_properties.mass))
            .sum::<f64>();
        let upper = instance
            .chunks
            .iter()
            .filter_map(|(id, chunk)| {
                let center = chunk.center_in_building();
                (center.y >= 3.0).then_some((id.clone(), (center, chunk.mass_properties.mass)))
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
                total_mass += f64::from(app.world().get::<ComputedMass>(body).unwrap().value());
                let chunks = &app.world().get::<physics::Cluster>(body).unwrap().chunks;
                let tracked = chunks
                    .iter()
                    .filter_map(|id| upper.get(id))
                    .collect::<Vec<_>>();
                if tracked.len() < 20
                    || app.world().get::<RigidBody>(body) != Some(&RigidBody::Dynamic)
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
            motion.largest_released_upper >= 20
                && motion.max_drop > 0.5
                && motion.strength_breaks > 0,
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
}
