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
        .0
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
    let entity = app
        .world_mut()
        .spawn((
            transform,
            ChildOf(root),
            Collider::cuboid(3.0, 0.2, 0.2),
            ColliderDensity(0.0),
            Mass(record.mass),
            CenterOfMass(Vec3::ZERO),
            AngularInertia::from_mat3_unchecked(Mat3::from_cols_array_2d(&record.inertia)),
            NoAutoMass,
            NoAutoAngularInertia,
            NoAutoCenterOfMass,
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
        .0
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
