use super::*;
use crate::{
    camera::CameraControl,
    physics::{Cluster, DestructionPhysicsPlugin},
    scene::spawn_bonds,
    stress_smoke::{BEAM, LEFT_BOND, RIGHT_BOND, spawn_beam},
};
use bevy::{
    ecs::system::{InMut, InRef, RunSystemOnce},
    time::TimeUpdateStrategy,
};

fn chunk<'a>(world: &'a World, building: Entity, id: &str) -> &'a ChunkInstance {
    let entity = world.get::<BuildingInstance>(building).unwrap().chunks[id];
    world.get::<ChunkInstance>(entity).unwrap()
}

fn body(world: &World, building: Entity, id: &str) -> Entity {
    let entity = world.get::<BuildingInstance>(building).unwrap().chunks[id];
    world.get::<ColliderOf>(entity).unwrap().body
}

fn bond_states(world: &World, building: Entity) -> BTreeMap<String, bool> {
    world
        .get::<BuildingInstance>(building)
        .unwrap()
        .bond_states(|entity| world.get::<BondInstance>(entity).unwrap().active)
}

fn break_bond(world: &mut World, building: Entity, id: &str) {
    let mut instance = world.get_mut::<BuildingInstance>(building).unwrap();
    instance.topology_dirty = true;
    let entity = instance.bonds[id];
    world.get_mut::<BondInstance>(entity).unwrap().active = false;
}

fn evaluate_dynamic_impact(
    world: &mut World,
    state: &mut ImpactState,
) -> Option<DynamicImpactReport> {
    world
        .run_system_once_with(
            |InMut(state): InMut<ImpactState>,
             structure: ImpactStructure,
             assets: Res<Assets<DestructibleAsset>>,
             capture: Option<Res<ForceCapture>>,
             time: Res<Time<Fixed>>| {
                process_dynamic_impact(
                    &structure,
                    &assets,
                    state,
                    capture.is_some_and(|capture| capture.enabled),
                    time.elapsed_secs_f64(),
                )
            },
            state,
        )
        .unwrap()
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, TransformPlugin, PhysicsPlugins::default()))
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .init_resource::<Assets<DestructibleAsset>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<CameraControl>()
        .insert_resource(DemoOptions {
            smoke_output: Some("output/projectile-unit".into()),
            ..default()
        })
        .insert_resource(Time::<Fixed>::from_hz(60.0))
        .insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f64(1.0 / 60.0),
        ))
        .insert_resource(Gravity(Vec3::ZERO))
        .add_plugins((DestructionPhysicsPlugin, ProjectilePlugin));
    app.finish();
    app.cleanup();
    app.update();
    app
}

fn launch(app: &mut App, shape: ProjectileShape) -> Entity {
    app.world_mut()
        .resource_mut::<LaunchRequests>()
        .queue
        .push_back(LaunchRequest {
            origin: Vec3::new(0.0, 1.1, 5.0),
            direction: Dir3::NEG_Z,
            shape,
        });
    app.update();
    app.world_mut()
        .query_filtered::<Entity, With<RigidProjectile>>()
        .single(app.world())
        .unwrap()
}

fn computed_mass(world: &World, entity: Entity) -> f32 {
    world.get::<ComputedMass>(entity).unwrap().value()
}

#[test]
fn sphere_and_cube_real_contacts_break_bonds_and_preserve_the_rigid_impactor() {
    for shape in [ProjectileShape::Sphere, ProjectileShape::Cube] {
        let mut app = app();
        let building = spawn_beam(app.world_mut(), Vec3::ZERO);
        app.update();
        let projectile = launch(&mut app, shape);
        for _ in 0..30 {
            app.update();
        }
        let stats = app.world().resource::<ImpactStats>();
        assert_eq!(stats.launched, 1);
        assert!(
            stats.contacts >= 1,
            "The projectile did not contact actual geometry"
        );
        assert!(stats.max_impulse > 1000.0);
        assert_eq!(stats.broken_bonds, 2);
        let world = app.world();
        let instance = world.get::<BuildingInstance>(building).unwrap();
        let bonds = bond_states(world, building);
        assert!(!bonds[LEFT_BOND]);
        assert!(!bonds[RIGHT_BOND]);
        assert!(
            instance
                .chunks
                .values()
                .all(|&entity| world.get::<ChunkInstance>(entity).unwrap().alive)
        );
        let beam_body = body(world, building, BEAM);
        assert!(!app.world().get::<Cluster>(beam_body).unwrap().anchored);
        assert_eq!(
            app.world().get::<RigidBody>(beam_body),
            Some(&RigidBody::Dynamic)
        );
        let mass = computed_mass(app.world(), projectile);
        assert!((mass - PROJECTILE_MASS).abs() < 1e-3);
        assert!(app.world().get::<RigidProjectile>(projectile).is_some());
        let collider = app.world().get::<Collider>(projectile).unwrap();
        let expected_inertia = match shape {
            ProjectileShape::Sphere => {
                assert_eq!(collider.shape().as_ball().unwrap().radius, 0.65);
                PROJECTILE_MASS * 0.4 * 0.65 * 0.65
            }
            ProjectileShape::Cube => {
                let size = collider.shape().as_cuboid().unwrap().half_extents;
                assert_eq!([size.x, size.y, size.z], [0.6; 3]);
                PROJECTILE_MASS * 1.2 * 1.2 / 6.0
            }
        };
        let inertia = app
            .world()
            .get::<ComputedAngularInertia>(projectile)
            .unwrap();
        let inertia_error = Mat3::from(inertia.value()) - Mat3::IDENTITY * expected_inertia;
        assert!(
            inertia_error
                .to_cols_array()
                .iter()
                .all(|error| error.abs() < 1e-3)
        );
        assert_eq!(
            app.world()
                .get::<ComputedCenterOfMass>(projectile)
                .unwrap()
                .0,
            Vec3::ZERO
        );
        assert!(app.world().get::<ColliderDensity>(projectile).unwrap().0 > 0.0);
        assert!(app.world().get::<Position>(projectile).unwrap().is_finite());
        let velocity = app.world().get::<LinearVelocity>(projectile).unwrap().0;
        assert!(velocity.is_finite());
        assert!(velocity.distance(Vec3::NEG_Z * PROJECTILE_SPEED) > 1.0);
        let before = (stats.contacts, stats.broken_bonds);
        for _ in 0..120 {
            app.update();
        }
        let after = app.world().resource::<ImpactStats>();
        assert_eq!((after.contacts, after.broken_bonds), before);
        assert_eq!(computed_mass(app.world(), projectile), mass);
    }
}

#[test]
fn subthreshold_speed_contacts_do_not_break_bonds_or_accumulate_resting_damage() {
    let mut app = app();
    let building = spawn_beam(app.world_mut(), Vec3::ZERO);
    app.update();
    let projectile = launch(&mut app, ProjectileShape::Sphere);
    app.world_mut()
        .entity_mut(projectile)
        .insert((LinearVelocity(Vec3::NEG_Z), SleepingDisabled));
    let mut saw_contact = false;
    for _ in 0..300 {
        app.update();
        saw_contact |= app
            .world()
            .resource::<ContactGraph>()
            .iter_active_touching()
            .any(|pair| pair.collider1 == projectile || pair.collider2 == projectile);
    }
    assert!(saw_contact, "The slow projectile never reached its target");
    let stats = app.world().resource::<ImpactStats>();
    assert_eq!(stats.contacts, 0);
    assert_eq!(stats.broken_bonds, 0);
    let bonds = bond_states(app.world(), building);
    assert!(bonds[LEFT_BOND]);
    assert!(bonds[RIGHT_BOND]);
    assert!(
        app.world()
            .get::<Cluster>(body(app.world(), building, BEAM))
            .unwrap()
            .anchored
    );
    assert!((computed_mass(app.world(), projectile) - PROJECTILE_MASS).abs() < 1e-3);
}

fn spawn_dropped_tower(app: &mut App) -> (Entity, Vec<String>) {
    spawn_dropped_tower_at_height(app, None)
}

fn spawn_dropped_tower_at_height(
    app: &mut App,
    ground_clearance: Option<f32>,
) -> (Entity, Vec<String>) {
    use crate::{
        asset::Manifest,
        scene::{BuildingBounds, chunk_collider},
    };
    let contents = std::env::var_os("DESTRUCTION_LANDING_ASSET")
        .map(|path| std::fs::read_to_string(path).unwrap())
        .unwrap_or_else(|| {
            include_str!("../../assets/buildings/highrise_slab.destruction.json").into()
        });
    let manifest: Manifest = serde_json::from_str(&contents).unwrap();
    let offset = ground_clearance.map_or(0.0, |height| {
        let lowest = manifest
            .chunks
            .iter()
            .filter(|record| {
                record
                    .transform()
                    .transform_point(record.center_of_mass.into())
                    .y
                    >= 16.0
            })
            .flat_map(|record| {
                record.colliders.iter().flat_map(move |proxy| {
                    proxy
                        .vertices
                        .iter()
                        .map(move |&point| record.transform().transform_point(point.into()).y)
                })
            })
            .fold(f32::INFINITY, f32::min);
        height - lowest
    });
    let asset = app
        .world_mut()
        .resource_mut::<Assets<DestructibleAsset>>()
        .add(DestructibleAsset {
            manifest: manifest.clone(),
            gltf: Handle::default(),
        });
    let root = app
        .world_mut()
        .spawn((Transform::from_xyz(0.0, offset, 0.0), Visibility::Visible))
        .id();
    let mut instance = BuildingInstance::pending(asset);
    instance.bounds = Some(BuildingBounds {
        min: Vec3::new(0.0, 0.0, -8.0),
        max: Vec3::new(14.0, 32.24, 0.0),
    });
    instance.topology_dirty = true;
    let mut upper = Vec::new();
    for record in &manifest.chunks {
        let local = record.transform();
        let alive = local.transform_point(record.center_of_mass.into()).y >= 16.0;
        let entity = app
            .world_mut()
            .spawn((
                local,
                GlobalTransform::from(local),
                Visibility::Inherited,
                ChildOf(root),
                ChunkIdentity {
                    building: root,
                    id: record.id.clone(),
                },
            ))
            .id();
        let mass_properties = if alive {
            let collider = chunk_collider(record).unwrap();
            let density = manifest.materials[&record.material].density;
            let properties = collider.mass_properties(density);
            app.world_mut()
                .entity_mut(entity)
                .insert((collider, ColliderDensity(density)));
            upper.push(record.id.clone());
            properties
        } else {
            MassProperties3d::ZERO
        };
        app.world_mut().entity_mut(entity).insert(ChunkInstance {
            primitives: Vec::new(),
            alive,
            triangles: Vec::new(),
            transform: local,
            mass_properties,
            material: record.material.clone(),
        });
        instance.chunks.insert(record.id.clone(), entity);
    }
    let world = app.world_mut();
    spawn_bonds(&mut world.commands(), root, &mut instance, &manifest.bonds);
    world.flush();
    for bond in &manifest.bonds {
        let entity = instance.bonds[&bond.id];
        let endpoints = world.get::<BondInstance>(entity).unwrap().endpoints;
        if !endpoints
            .into_iter()
            .all(|endpoint| world.get::<ChunkInstance>(endpoint).unwrap().alive)
        {
            world.get_mut::<BondInstance>(entity).unwrap().active = false;
        }
    }
    world.entity_mut(root).insert(instance);
    (root, upper)
}

#[test]
fn dropped_upper_tower_fractures_on_ground_contact() {
    let mut app = app();
    app.insert_resource(Gravity(Vec3::NEG_Y * 9.81));
    let (building, upper) = spawn_dropped_tower(&mut app);
    let trace = std::env::var_os("DESTRUCTION_LANDING_TRACE").is_some();
    if trace {
        app.insert_resource(ForceCapture { enabled: true })
            .init_resource::<ForceSamples>()
            .add_systems(
                PhysicsSchedule,
                trace_landing_contact_speeds
                    .after(PhysicsStepSystems::NarrowPhase)
                    .before(PhysicsStepSystems::Solver),
            );
    }
    app.update();
    let first_body = body(app.world(), building, &upper[0]);
    assert_eq!(
        app.world().get::<Cluster>(first_body).unwrap().chunks.len(),
        upper.len()
    );
    let mut saw_fast_contact = false;
    let mut max_impulse = 0.0_f32;
    let mut max_speed = 0.0_f32;
    let mut trace_at = -1.0;
    let mut trace_released = 0;
    let mut previous_bonds = bond_states(app.world(), building);
    for tick in 0..300 {
        app.update();
        if trace {
            trace_landing_step(
                app.world(),
                building,
                tick,
                &mut trace_at,
                &mut trace_released,
                &mut previous_bonds,
            );
        }
        let graph = app.world().resource::<ContactGraph>();
        for pair in graph.iter_active_touching() {
            if [pair.body1, pair.body2].contains(&Some(first_body)) {
                let speed = pair
                    .manifolds
                    .iter()
                    .flat_map(|m| &m.points)
                    .map(|p| -p.normal_speed)
                    .fold(0.0_f32, f32::max);
                let impulse = pair.total_normal_impulse_magnitude();
                max_speed = max_speed.max(speed);
                max_impulse = max_impulse.max(impulse);
                saw_fast_contact |= speed > 2.0;
            }
        }
        if tick < 90 {
            let stats = app.world().resource::<ImpactStats>();
            assert_eq!(stats.broken_bonds, 0, "Free fall fractured before contact");
            assert_eq!(stats.structural_solves, 0);
        }
        for body in &app
            .world()
            .get::<BuildingInstance>(building)
            .unwrap()
            .clusters
        {
            assert!(app.world().get::<Position>(*body).unwrap().0.is_finite());
            assert!(app.world().get::<Rotation>(*body).unwrap().0.is_finite());
            assert!(
                app.world()
                    .get::<LinearVelocity>(*body)
                    .unwrap()
                    .0
                    .is_finite()
            );
            assert!(
                app.world()
                    .get::<AngularVelocity>(*body)
                    .unwrap()
                    .0
                    .is_finite()
            );
        }
    }
    let stats = app.world().resource::<ImpactStats>();
    let sizes = app
        .world()
        .get::<BuildingInstance>(building)
        .unwrap()
        .clusters
        .iter()
        .map(|body| app.world().get::<Cluster>(*body).unwrap().chunks.len())
        .collect::<Vec<_>>();
    eprintln!(
        "upper-tower landing: broken={} dynamic={} solves={} released={} invalidated={} pending={} peak_utilization={:.3} speed={max_speed:.3} impulse={max_impulse:.1} sizes={sizes:?}",
        stats.broken_bonds,
        stats.structural_broken_bonds,
        stats.structural_solves,
        stats.released_structural_events,
        stats.invalidated_structural_events,
        stats.pending_structural,
        stats.max_structural_utilization,
    );
    assert!(
        saw_fast_contact,
        "Drop did not reach an actual fast ground contact"
    );
    assert!(
        stats.broken_bonds > 0,
        "Upper tower landed without collision-driven structural fracture"
    );
    assert!(max_speed > 10.0 && max_impulse > 1e6);
    assert!(stats.structural_solves > 0);
    assert_eq!(stats.structural_failures, 0);
    assert!(stats.max_force_balance_ratio < 1e-6);
    assert!(stats.max_moment_balance_ratio < 1e-6);
    assert!(stats.max_gauge_reaction_ratio < 1e-5);
    assert_eq!(
        stats.pending_structural, 0,
        "Landing event work did not finish"
    );
    assert!(stats.released_structural_events > 0);
    assert!(
        sizes.iter().max().unwrap() + 4 < upper.len(),
        "Landing left an almost intact tower: {} broken bonds ({} dynamic), {} solves, {} released events, {} pending, peak utilization {}, sizes {sizes:?}",
        stats.broken_bonds,
        stats.structural_broken_bonds,
        stats.structural_solves,
        stats.released_structural_events,
        stats.pending_structural,
        stats.max_structural_utilization,
    );
    assert!(
        sizes.iter().any(|&size| size > 1),
        "Landing atomized all {} chunks: {} broken bonds, {} solves, sizes {sizes:?}",
        upper.len(),
        stats.broken_bonds,
        stats.structural_solves,
    );
    assert!(
        app.world()
            .get::<BuildingInstance>(building)
            .unwrap()
            .clusters
            .len()
            > 1,
        "Broken landing bonds did not split the compound"
    );
    let instance = app.world().get::<BuildingInstance>(building).unwrap();
    let bonds = bond_states(app.world(), building);
    let manifest = &app
        .world()
        .resource::<Assets<DestructibleAsset>>()
        .get(&instance.asset)
        .unwrap()
        .manifest;
    let records = manifest
        .chunks
        .iter()
        .filter(|record| upper.contains(&record.id))
        .map(|record| (record.id.as_str(), record))
        .collect::<BTreeMap<_, _>>();
    let source_y = |record: &crate::asset::ChunkRecord| {
        record
            .transform()
            .transform_point(record.center_of_mass.into())
            .y
    };
    let first_floor = records
        .values()
        .filter(|record| record.kind == "floor")
        .min_by(|a, b| source_y(a).total_cmp(&source_y(b)))
        .unwrap();
    let first_level = &first_floor.part_id;
    let next_floor_y = records
        .values()
        .filter(|record| record.kind == "floor" && &record.part_id != first_level)
        .map(|record| source_y(record))
        .min_by(f32::total_cmp)
        .unwrap();
    let roof_y = records
        .values()
        .filter(|record| record.kind == "roof")
        .map(|record| source_y(record))
        .min_by(f32::total_cmp)
        .unwrap();
    let main_body = *instance
        .clusters
        .iter()
        .max_by_key(|&&body| app.world().get::<Cluster>(body).unwrap().chunks.len())
        .unwrap();
    assert!(
        instance.clusters.iter().any(|&body| {
            let cluster = app.world().get::<Cluster>(body).unwrap();
            let has_lower_kind = |kind| {
                cluster.chunks.iter().any(|id| {
                    let record = records[id.as_str()];
                    record.kind == kind && source_y(record) < next_floor_y
                })
            };
            body != main_body
                && cluster.chunks.len() > 1
                && has_lower_kind("floor")
                && has_lower_kind("wall")
        }),
        "Landing did not separate a connected lower-storey floor-and-wall section"
    );
    let broken_y = manifest
        .bonds
        .iter()
        .filter(|bond| upper.contains(&bond.a) && upper.contains(&bond.b) && !bonds[&bond.id])
        .map(|bond| bond.center[1])
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), y| {
            (min.min(y), max.max(y))
        });
    eprintln!("broken bond source_y={:.3}..{:.3}", broken_y.0, broken_y.1);
    let expected_mass = upper
        .iter()
        .map(|id| f64::from(chunk(app.world(), building, id).mass_properties.mass))
        .sum::<f64>();
    let actual_mass = instance
        .clusters
        .iter()
        .map(|body| f64::from(computed_mass(app.world(), *body)))
        .sum::<f64>();
    assert!((actual_mass - expected_mass).abs() < expected_mass * 1e-5);
    assert!(
        manifest.bonds.iter().any(|bond| upper.contains(&bond.a)
            && upper.contains(&bond.b)
            && !bonds[&bond.id]
            && bond.center[1] > 20.0
            && bond.center[1] < roof_y
            && records[bond.a.as_str()].kind != "roof"
            && records[bond.b.as_str()].kind != "roof"),
        "Landing only chipped its directly contacting floor; no load reached internal upper-storey bonds"
    );
}

fn trace_landing_step(
    world: &World,
    building: Entity,
    tick: usize,
    last_capture: &mut f64,
    last_released: &mut u64,
    previous_bonds: &mut BTreeMap<String, bool>,
) {
    let instance = world.get::<BuildingInstance>(building).unwrap();
    let manifest = &world
        .resource::<Assets<DestructibleAsset>>()
        .get(&instance.asset)
        .unwrap()
        .manifest;
    let bonds = bond_states(world, building);
    let newly_broken = bonds
        .iter()
        .filter(|(id, active)| !**active && previous_bonds.get(*id) == Some(&true))
        .map(|(id, _)| id.as_str())
        .collect::<Vec<_>>();
    let sample = world
        .resource::<ForceSamples>()
        .dynamic
        .iter()
        .max_by(|a, b| a.captured_at.total_cmp(&b.captured_at))
        .filter(|sample| sample.captured_at > *last_capture);
    let stats = world.resource::<ImpactStats>();
    if newly_broken.is_empty()
        && sample.is_none()
        && stats.released_structural_events == *last_released
    {
        return;
    }
    let records = manifest
        .chunks
        .iter()
        .map(|record| (record.id.as_str(), record))
        .collect::<BTreeMap<_, _>>();
    *last_released = stats.released_structural_events;
    let groups = instance
        .clusters
        .iter()
        .map(|body| {
            let cluster = world.get::<Cluster>(*body).unwrap();
            (
                cluster.chunks.len(),
                computed_mass(world, *body),
                world.get::<LinearVelocity>(*body).unwrap().y,
                world.get::<AngularVelocity>(*body).unwrap().length(),
                (cluster.chunks.len() <= 4).then(|| cluster.chunks.clone()),
            )
        })
        .collect::<Vec<_>>();
    eprintln!(
        "TRACE tick={tick} broken={} released={} fresh_rebound={} groups={groups:?}",
        stats.broken_bonds, stats.released_structural_events, stats.rebound_fresh_structural_events,
    );
    for id in newly_broken {
        let bond = manifest.bonds.iter().find(|bond| bond.id == id).unwrap();
        let a = records[bond.a.as_str()];
        let b = records[bond.b.as_str()];
        eprintln!(
            "BREAK {id} y={} area={} facets={} a={}({},{:.1}kg) b={}({},{:.1}kg)",
            bond.center[1],
            bond.area,
            bond.patches.len(),
            a.id,
            a.kind,
            a.mass,
            b.id,
            b.kind,
            b.mass
        );
    }
    if let Some(sample) = sample {
        *last_capture = sample.captured_at;
        let mut ranked = sample.bonds.iter().collect::<Vec<_>>();
        ranked.sort_by(|a, b| b.1.utilization.total_cmp(&a.1.utilization));
        for (id, stress) in ranked.into_iter().take(6) {
            let bond = manifest.bonds.iter().find(|bond| &bond.id == id).unwrap();
            eprintln!(
                "FIELD {id} U={:.6e} t={:.4e} c={:.4e} s={:.4e} F={:.4e} M={:.4e} area={:.4e} facets={:?} y={} a={} b={}",
                stress.utilization,
                stress.tension,
                stress.compression,
                stress.shear,
                DVec3::from_array(stress.force).length(),
                DVec3::from_array(stress.moment).length(),
                bond.area,
                bond.patches
                    .iter()
                    .map(|patch| patch.area)
                    .collect::<Vec<_>>(),
                bond.center[1],
                bond.a,
                bond.b
            );
        }
        let shocks = world
            .resource::<ImpactState>()
            .shocks
            .iter()
            .map(|shock| {
                (
                    shock.chunks.len(),
                    shock.prepared.is_some(),
                    shock
                        .loads
                        .iter()
                        .map(|(id, impulse)| (id.clone(), impulse.linear.length()))
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        eprintln!("QUEUED {shocks:?}");
    }
    *previous_bonds = bonds;
}

fn trace_landing_contact_speeds(world: &mut World) {
    let state = world.resource::<ImpactState>();
    let point_velocity = |body: Option<Entity>, point: Vec3| {
        body.map_or(Vec3::ZERO, |body| {
            let linear = world
                .get::<LinearVelocity>(body)
                .map_or(Vec3::ZERO, |v| v.0);
            let angular = world
                .get::<AngularVelocity>(body)
                .map_or(Vec3::ZERO, |v| v.0);
            let position = world.get::<Position>(body).map_or(Vec3::ZERO, |p| p.0);
            let rotation = world.get::<Rotation>(body).map_or(Quat::IDENTITY, |r| r.0);
            let local_center = world
                .get::<ComputedCenterOfMass>(body)
                .map_or(Vec3::ZERO, |c| c.0);
            linear + angular.cross(point - position - rotation * local_center)
        })
    };
    for pair in world.resource::<ContactGraph>().iter_active_touching() {
        let structural = [pair.collider1, pair.collider2]
            .into_iter()
            .filter_map(|collider| {
                world
                    .get::<ChunkIdentity>(collider)
                    .map(|chunk| chunk.id.as_str())
            })
            .collect::<Vec<_>>();
        if structural.is_empty() {
            continue;
        }
        let mut speed = 0.0_f32;
        for manifold in &pair.manifolds {
            for point in &manifold.points {
                speed = speed.max(
                    -(point_velocity(pair.body2, point.point)
                        - point_velocity(pair.body1, point.point))
                    .dot(manifold.normal),
                );
            }
        }
        if speed < IMPACT_MIN_SPEED {
            continue;
        }
        let key = if pair.collider1 < pair.collider2 {
            (pair.collider1, pair.collider2)
        } else {
            (pair.collider2, pair.collider1)
        };
        let stored = pair
            .manifolds
            .iter()
            .flat_map(|manifold| &manifold.points)
            .map(|point| -point.normal_speed)
            .fold(0.0_f32, f32::max);
        let motions = [pair.body1, pair.body2].map(|body| {
            body.and_then(|body| world.get::<LinearVelocity>(body).map(|v| (body, v.0)))
        });
        eprintln!(
            "PRE_CONTACT time={:.6} suppressed={} actual_speed={speed:.6} stored_speed={stored:.6} chunks={structural:?} linear={motions:?}",
            world.resource::<Time<Fixed>>().elapsed_secs_f64(),
            state.touching.contains(&key)
        );
    }
}

#[test]
fn tower_landing_fracture_responds_to_drop_height() {
    let mut outcomes = Vec::new();
    for height in [0.5, 16.0] {
        let mut app = app();
        app.insert_resource(Gravity(Vec3::NEG_Y * 9.81));
        let (building, upper) = spawn_dropped_tower_at_height(&mut app, Some(height));
        let mut max_speed = 0.0_f32;
        for _ in 0..420 {
            app.update();
            for pair in app
                .world()
                .resource::<ContactGraph>()
                .iter_active_touching()
            {
                if [pair.collider1, pair.collider2].iter().any(|&entity| {
                    app.world()
                        .get::<ChunkIdentity>(entity)
                        .is_some_and(|identity| identity.building == building)
                }) {
                    max_speed = pair
                        .manifolds
                        .iter()
                        .flat_map(|manifold| &manifold.points)
                        .map(|point| -point.normal_speed)
                        .fold(max_speed, f32::max);
                }
            }
        }
        let instance = app.world().get::<BuildingInstance>(building).unwrap();
        let mut sizes = instance
            .clusters
            .iter()
            .map(|&body| app.world().get::<Cluster>(body).unwrap().chunks.len())
            .collect::<Vec<_>>();
        sizes.sort_unstable_by(|a, b| b.cmp(a));
        let stats = app.world().resource::<ImpactStats>();
        eprintln!(
            "height={height}m speed={max_speed:.3}m/s broken={} dynamic={} solves={} released={} pending={} peak_utilization={:.3} sizes={sizes:?}",
            stats.broken_bonds,
            stats.structural_broken_bonds,
            stats.structural_solves,
            stats.released_structural_events,
            stats.pending_structural,
            stats.max_structural_utilization,
        );
        assert!(
            max_speed > IMPACT_MIN_SPEED,
            "Drop did not reach an eligible real impact"
        );
        assert_eq!(sizes.iter().sum::<usize>(), upper.len());
        assert!(
            sizes.iter().any(|&size| size > 1),
            "Drop atomized the entire tower"
        );
        assert_eq!(stats.pending_structural, 0);
        assert_eq!(stats.structural_failures, 0);
        outcomes.push((stats.broken_bonds, sizes[0], max_speed));
    }
    assert!(outcomes[1].2 > outcomes[0].2 * 3.0);
    assert!(
        outcomes[1].0 > outcomes[0].0,
        "A much faster landing caused no extra fracture: {outcomes:?}"
    );
    assert!(outcomes[1].1 < outcomes[0].1);
}

#[test]
fn a_split_releases_the_parent_shock_without_reusing_its_contact_impulse() {
    let mut app = app();
    let (building, upper) = spawn_dropped_tower(&mut app);
    app.update();
    let mut state = ImpactState::default();
    state.shocks.push_back(ClusterImpact {
        building,
        chunks: upper.clone(),
        loads: BTreeMap::from([(
            upper[0].clone(),
            ImpulseLoad {
                linear: DVec3::Y * 1e7,
                angular: DVec3::ZERO,
            },
        )]),
        prepared: None,
    });
    let initial = evaluate_dynamic_impact(app.world_mut(), &mut state).unwrap();
    assert!(initial.valid);
    assert!(initial.fracture.is_some());
    state.shocks.push_back(initial.continuation.unwrap());
    let detached = upper.last().unwrap();
    let ids = {
        let instance = app.world().get::<BuildingInstance>(building).unwrap();
        app.world()
            .resource::<Assets<DestructibleAsset>>()
            .get(&instance.asset)
            .unwrap()
            .manifest
            .bonds
            .iter()
            .filter(|bond| bond.a == *detached || bond.b == *detached)
            .map(|bond| bond.id.clone())
            .collect::<Vec<_>>()
    };
    for id in ids {
        break_bond(app.world_mut(), building, &id);
    }
    crate::physics::rebuild_clusters(app.world_mut());
    let before = bond_states(app.world(), building);
    assert!(
        app.world()
            .get::<BuildingInstance>(building)
            .unwrap()
            .clusters
            .len()
            > 1
    );
    assert!(evaluate_dynamic_impact(app.world_mut(), &mut state).is_none());
    assert_eq!(state.released, 1);
    assert_eq!(state.invalidated, 0);
    assert!(state.shocks.is_empty());
    assert!(state.pending.is_empty());
    assert_eq!(bond_states(app.world(), building), before);
}

#[test]
fn force_capture_preserves_the_solved_impact_and_fracture_choice() {
    let mut app = app();
    let (building, upper) = spawn_dropped_tower(&mut app);
    app.update();
    let state_for = || {
        let mut state = ImpactState::default();
        state.shocks.push_back(ClusterImpact {
            building,
            chunks: upper.clone(),
            loads: BTreeMap::from([(
                upper[0].clone(),
                ImpulseLoad {
                    linear: DVec3::Y * 1e7,
                    angular: DVec3::ZERO,
                },
            )]),
            prepared: None,
        });
        state
    };
    let baseline = evaluate_dynamic_impact(app.world_mut(), &mut state_for()).unwrap();
    assert!(baseline.valid);
    assert!(baseline.force_sample.is_none());
    app.insert_resource(ForceCapture { enabled: true });
    let captured = evaluate_dynamic_impact(app.world_mut(), &mut state_for()).unwrap();
    assert_eq!(captured.fracture, baseline.fracture);
    assert_eq!(captured.max_utilization, baseline.max_utilization);
    assert_eq!(captured.gauge_reaction_ratio, baseline.gauge_reaction_ratio);
    let sample = captured.force_sample.unwrap();
    assert_eq!(sample.chunks, upper);
    assert!(!sample.bonds.is_empty());
    assert!(
        sample
            .bonds
            .values()
            .any(|bond| DVec3::from_array(bond.force).length() > 0.0)
    );
    assert!(sample.bonds.values().all(|bond| {
        bond.force
            .into_iter()
            .chain(bond.moment)
            .all(f64::is_finite)
    }));
}

fn split_free_beam_for_queued_hit() -> (App, Entity, Vec<String>, Vec<String>) {
    let mut app = app();
    let building = spawn_beam(app.world_mut(), Vec3::ZERO);
    let asset = app
        .world()
        .get::<BuildingInstance>(building)
        .unwrap()
        .asset
        .clone();
    app.world_mut()
        .resource_mut::<Assets<DestructibleAsset>>()
        .get_mut(&asset)
        .unwrap()
        .manifest
        .anchors
        .clear();
    app.update();
    let original = app
        .world()
        .get::<BuildingInstance>(building)
        .unwrap()
        .chunks
        .keys()
        .cloned()
        .collect();
    break_bond(app.world_mut(), building, RIGHT_BOND);
    crate::physics::rebuild_clusters(app.world_mut());
    let body = body(app.world(), building, BEAM);
    let current = app.world().get::<Cluster>(body).unwrap().chunks.clone();
    assert_eq!(current.len(), 2);
    (app, building, original, current)
}

#[test]
fn fresh_independent_hit_binds_once_to_its_current_child_without_reapplying_impulse() {
    let (mut app, building, original, current) = split_free_beam_for_queued_hit();
    let loads = BTreeMap::from([(
        BEAM.into(),
        ImpulseLoad {
            linear: DVec3::Y * 1e6,
            angular: DVec3::ZERO,
        },
    )]);
    let state_for = |chunks| ImpactState {
        shocks: VecDeque::from([ClusterImpact {
            building,
            chunks,
            loads: loads.clone(),
            prepared: None,
        }]),
        ..default()
    };
    let body = body(app.world(), building, BEAM);
    let velocity = app.world().get::<LinearVelocity>(body).unwrap().0;
    let mut state = state_for(original);
    let mut rebound = evaluate_dynamic_impact(app.world_mut(), &mut state).unwrap();
    let direct = evaluate_dynamic_impact(app.world_mut(), &mut state_for(current.clone())).unwrap();
    assert!(rebound.valid && direct.valid);
    assert_eq!(rebound.fracture, direct.fracture);
    assert_eq!(rebound.max_utilization, direct.max_utilization);
    assert_eq!(rebound.force_balance_ratio, direct.force_balance_ratio);
    assert_eq!(rebound.moment_balance_ratio, direct.moment_balance_ratio);
    assert!(rebound.gauge_reaction_ratio < 1e-5);
    assert_eq!(state.rebound_fresh, 1);
    assert_eq!(app.world().get::<LinearVelocity>(body).unwrap().0, velocity);
    let continuation = rebound.continuation.take().unwrap();
    assert_eq!(continuation.chunks, current);
    assert!(continuation.prepared.is_some());
    state.shocks.push_back(continuation);
    break_bond(app.world_mut(), building, LEFT_BOND);
    crate::physics::rebuild_clusters(app.world_mut());
    assert!(evaluate_dynamic_impact(app.world_mut(), &mut state).is_none());
    assert_eq!(state.released, 1);
    assert_eq!(state.rebound_fresh, 1);
    assert!(state.shocks.is_empty() && state.pending.is_empty());
}

#[test]
fn fresh_hit_with_sources_on_both_sides_of_a_split_is_released() {
    let (mut app, building, original, _) = split_free_beam_for_queued_hit();
    let before = bond_states(app.world(), building);
    let mut state = ImpactState {
        shocks: VecDeque::from([ClusterImpact {
            building,
            chunks: original,
            loads: [BEAM, crate::stress_smoke::RIGHT_SUPPORT]
                .map(|id| {
                    (
                        id.into(),
                        ImpulseLoad {
                            linear: DVec3::Y * 1e6,
                            angular: DVec3::ZERO,
                        },
                    )
                })
                .into(),
            prepared: None,
        }]),
        ..default()
    };
    assert!(evaluate_dynamic_impact(app.world_mut(), &mut state).is_none());
    assert_eq!(state.released, 1);
    assert_eq!(state.rebound_fresh, 0);
    assert!(state.shocks.is_empty() && state.pending.is_empty());
    assert_eq!(bond_states(app.world(), building), before);
}

fn offset_impact_fixture() -> (Manifest, BTreeMap<String, ChunkInstance>) {
    let mut manifest = crate::stress_smoke::beam_manifest();
    let mut chunks = BTreeMap::new();
    for (index, record) in manifest.chunks.iter_mut().enumerate() {
        let rotation = Quat::from_euler(EulerRot::XYZ, 0.2, -0.4, 0.3 * index as f32);
        record.rotation = rotation.to_array();
        let collider = Collider::compound(vec![
            (
                Vec3::new(-0.1, 0.2, 0.3),
                Quat::IDENTITY,
                Collider::cuboid(0.4, 0.1, 0.2),
            ),
            (
                Vec3::new(0.4, 0.3, 0.2),
                Quat::from_rotation_z(0.7),
                Collider::cuboid(0.2, 0.4, 0.3),
            ),
        ]);
        let mass_properties = collider.mass_properties(700.0 + 100.0 * index as f32);
        chunks.insert(
            record.id.clone(),
            ChunkInstance {
                primitives: Vec::new(),
                alive: true,
                triangles: Vec::new(),
                transform: record.transform(),
                mass_properties,
                material: record.material.clone(),
            },
        );
    }
    (manifest, chunks)
}

#[test]
fn inertial_impact_loads_balance_and_do_not_depend_on_a_numerical_gauge() {
    use crate::stress_smoke::{LEFT_SUPPORT, RIGHT_SUPPORT};
    let (manifest, chunk_states) = offset_impact_fixture();
    let chunks = manifest
        .chunks
        .iter()
        .map(|chunk| chunk.id.clone())
        .collect::<Vec<_>>();
    let active = manifest
        .bonds
        .iter()
        .map(|bond| (bond.id.clone(), true))
        .collect();
    let impulses = BTreeMap::from([(
        LEFT_SUPPORT.into(),
        ImpulseLoad {
            linear: DVec3::new(2000.0, 6000.0, 300.0),
            angular: DVec3::new(0.0, 500.0, -200.0),
        },
    )]);
    let case = inertial_loads(&manifest, |id| chunk_states.get(id), &chunks, &impulses).unwrap();
    assert!(case.force_balance_ratio < 1e-12);
    assert!(case.moment_balance_ratio < 1e-12);
    let left = stress::solve_with_gauge(
        &manifest,
        &chunks,
        &active,
        &case.loads,
        &SolverSettings::default(),
        LEFT_SUPPORT,
    )
    .unwrap();
    let right = stress::solve_with_gauge(
        &manifest,
        &chunks,
        &active,
        &case.loads,
        &SolverSettings::default(),
        RIGHT_SUPPORT,
    )
    .unwrap();
    for (id, bond) in &left.bonds {
        let other = &right.bonds[id];
        let force = DVec3::from_array(bond.force);
        let moment = DVec3::from_array(bond.moment);
        assert!((force - DVec3::from_array(other.force)).length() < force.length() * 1e-7 + 1e-4);
        assert!(
            (moment - DVec3::from_array(other.moment)).length() < moment.length() * 1e-7 + 1e-4
        );
    }
    for (result, gauge) in [(&left, LEFT_SUPPORT), (&right, RIGHT_SUPPORT)] {
        let reaction = &result.reactions[gauge];
        assert!(DVec3::from_array(reaction.force).length() / case.force_scale < 1e-6);
        assert!(DVec3::from_array(reaction.moment).length() / case.moment_scale < 1e-6);
    }
}

#[test]
fn a_rigid_velocity_change_has_no_internal_impact_load() {
    let (manifest, chunk_states) = offset_impact_fixture();
    let chunks = manifest
        .chunks
        .iter()
        .map(|chunk| chunk.id.clone())
        .collect::<Vec<_>>();
    let total_mass = chunk_states
        .values()
        .map(|chunk| f64::from(chunk.mass_properties.mass))
        .sum::<f64>();
    let center = chunk_states
        .values()
        .map(|chunk| chunk.center_in_building().as_dvec3() * f64::from(chunk.mass_properties.mass))
        .sum::<DVec3>()
        / total_mass;
    let linear = DVec3::new(2.0, -5.0, 3.0);
    let angular = DVec3::new(0.0, 0.0, 0.7);
    let impulses = chunk_states
        .iter()
        .map(|(id, chunk)| {
            let r = chunk.center_in_building().as_dvec3() - center;
            let inertia = Mat3::from(
                chunk
                    .mass_properties
                    .global_angular_inertia_tensor(chunk.transform.rotation),
            );
            (
                id.clone(),
                ImpulseLoad {
                    linear: (linear + angular.cross(r)) * f64::from(chunk.mass_properties.mass),
                    angular: inertia.as_dmat3() * angular,
                },
            )
        })
        .collect();
    let case = inertial_loads(&manifest, |id| chunk_states.get(id), &chunks, &impulses).unwrap();
    for load in case.loads.values() {
        assert!(DVec3::from_array(load.force).length() < 1e-7);
        assert!(DVec3::from_array(load.moment).length() < 1e-7);
    }
}

#[test]
fn local_impacts_use_facet_directions_and_are_invariant_to_planar_subdivision() {
    let rectangle = |normal: Vec3, center: Vec3, half_height: f32| {
        let tangent = Vec3::new(-normal.y, normal.x, 0.0);
        let half_width = 0.05_f32;
        let area = 4.0 * half_width * half_height;
        crate::asset::BondPatchRecord {
            area,
            center: center.to_array(),
            normal: normal.to_array(),
            area_covariance: std::array::from_fn(|row| {
                std::array::from_fn(|column| {
                    area / 3.0
                        * (half_width.powi(2) * tangent[row] * tangent[column]
                            + half_height.powi(2) * Vec3::Z[row] * Vec3::Z[column])
                })
            }),
            vertices: [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                .into_iter()
                .map(|(u, v)| {
                    (center + tangent * (u * half_width) + Vec3::Z * (v * half_height)).to_array()
                })
                .collect(),
        }
    };
    let mut planar = crate::stress_smoke::beam_manifest();
    for bond in &mut planar.bonds {
        let patch = rectangle(Vec3::X, Vec3::from_array(bond.center), 0.05);
        bond.normal = patch.normal;
        bond.area = patch.area;
        bond.area_covariance = patch.area_covariance;
        bond.patches = vec![patch];
    }
    let total_area = planar.bonds.iter().map(|bond| bond.area).sum::<f32>();
    let intensity = planar.materials["concrete"].shear_strength * 1.2 / 0.8;
    let impulse = Vec3::NEG_X * (intensity * total_area * IMPACT_DURATION);
    let evaluate = |manifest: &Manifest| {
        let mut world = World::new();
        let building = world.spawn(Name::new("Local impact fixture")).id();
        let mut instance = BuildingInstance::pending(Handle::default());
        for record in &manifest.chunks {
            let entity = world
                .spawn((
                    Name::new(record.id.clone()),
                    ChildOf(building),
                    ChunkInstance {
                        primitives: Vec::new(),
                        alive: true,
                        triangles: Vec::new(),
                        transform: record.transform(),
                        mass_properties: MassProperties3d::ZERO,
                        material: record.material.clone(),
                    },
                ))
                .id();
            instance.chunks.insert(record.id.clone(), entity);
        }
        spawn_bonds(
            &mut world.commands(),
            building,
            &mut instance,
            &manifest.bonds,
        );
        world.flush();
        world.entity_mut(building).insert(instance);
        world
            .run_system_once_with(
                move |InRef(manifest): InRef<Manifest>, structure: ImpactStructure| {
                    let instance = structure.buildings.get(building).unwrap();
                    let mut pending = BTreeMap::new();
                    queue_local_damage(
                        local_impact_bonds(
                            &structure,
                            manifest,
                            instance,
                            BEAM,
                            instance.chunks[BEAM],
                        ),
                        building,
                        impulse,
                        &mut pending,
                    );
                    pending
                        .into_iter()
                        .map(|((_, id), utilization)| (id, utilization))
                        .collect::<BTreeMap<_, _>>()
                },
                manifest,
            )
            .unwrap()
    };
    assert!(
        evaluate(&planar).is_empty(),
        "The average plane should see only subcritical compression"
    );

    let mut folded = planar.clone();
    let bond = folded
        .bonds
        .iter_mut()
        .find(|bond| bond.id == LEFT_BOND)
        .unwrap();
    bond.patches = [-1.0, 1.0]
        .into_iter()
        .map(|side| {
            rectangle(
                Vec3::new(0.6, side * 0.8, 0.0),
                Vec3::from_array(bond.center) + Vec3::Y * (side * 0.03),
                0.025,
            )
        })
        .collect();
    let pending = evaluate(&folded);
    assert_eq!(pending.len(), 1);
    assert!(
        (pending[LEFT_BOND] - 1.2).abs() < 1e-5,
        "The folded facets must see shear hidden by their average plane"
    );

    let mut subdivided = folded.clone();
    let bond = subdivided
        .bonds
        .iter_mut()
        .find(|bond| bond.id == LEFT_BOND)
        .unwrap();
    bond.patches = bond
        .patches
        .iter()
        .flat_map(|patch| {
            [-1.0, 1.0].into_iter().map(move |side| {
                rectangle(
                    Vec3::from_array(patch.normal),
                    Vec3::from_array(patch.center) + Vec3::Z * (side * 0.0125),
                    0.0125,
                )
            })
        })
        .collect();
    let second = evaluate(&subdivided);
    assert_eq!(
        pending.keys().collect::<Vec<_>>(),
        second.keys().collect::<Vec<_>>()
    );
    for (key, value) in pending {
        assert!((value - second[&key]).abs() < 1e-6);
    }
}
