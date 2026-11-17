use super::*;
use crate::{
    camera::CameraControl,
    physics::{Cluster, DestructionPhysicsPlugin},
    stress_smoke::{BEAM, LEFT_BOND, RIGHT_BOND, spawn_beam},
};
use bevy::time::TimeUpdateStrategy;

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
        .0
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
        let instance = app.world().get::<BuildingInstance>(building).unwrap();
        assert!(!instance.bonds[LEFT_BOND]);
        assert!(!instance.bonds[RIGHT_BOND]);
        assert!(instance.chunks.values().all(|chunk| chunk.alive));
        let beam_body = instance.chunks[BEAM].cluster.unwrap();
        assert!(!app.world().get::<Cluster>(beam_body).unwrap().anchored);
        assert_eq!(
            app.world().get::<RigidBody>(beam_body),
            Some(&RigidBody::Dynamic)
        );
        assert_eq!(
            app.world().get::<Mass>(projectile).unwrap().0,
            PROJECTILE_MASS
        );
        assert!(app.world().get::<RigidProjectile>(projectile).is_some());
        let collider = app.world().get::<Collider>(projectile).unwrap();
        match shape {
            ProjectileShape::Sphere => assert_eq!(collider.shape().as_ball().unwrap().radius, 0.65),
            ProjectileShape::Cube => {
                let size = collider.shape().as_cuboid().unwrap().half_extents;
                assert_eq!([size.x, size.y, size.z], [0.6; 3]);
            }
        }
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
        assert_eq!(
            app.world().get::<Mass>(projectile).unwrap().0,
            PROJECTILE_MASS
        );
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
    let instance = app.world().get::<BuildingInstance>(building).unwrap();
    assert!(instance.bonds[LEFT_BOND]);
    assert!(instance.bonds[RIGHT_BOND]);
    assert!(
        app.world()
            .get::<Cluster>(instance.chunks[BEAM].cluster.unwrap())
            .unwrap()
            .anchored
    );
    assert_eq!(
        app.world().get::<Mass>(projectile).unwrap().0,
        PROJECTILE_MASS
    );
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
        scene::{BuildingBounds, ChunkInstance},
    };
    let manifest: Manifest = serde_json::from_str(include_str!(
        "../../assets/buildings/highrise_slab.destruction.json"
    ))
    .unwrap();
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
    instance.bonds = manifest
        .bonds
        .iter()
        .map(|bond| (bond.id.clone(), true))
        .collect();
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
                .collect::<Vec<_>>();
            app.world_mut()
                .entity_mut(entity)
                .insert((Collider::compound(shapes), ColliderDensity(0.0)));
            upper.push(record.id.clone());
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
    (root, upper)
}

#[test]
fn dropped_upper_tower_fractures_on_ground_contact() {
    let mut app = app();
    app.insert_resource(Gravity(Vec3::NEG_Y * 9.81));
    let (building, upper) = spawn_dropped_tower(&mut app);
    app.update();
    let first_body = app
        .world()
        .get::<BuildingInstance>(building)
        .unwrap()
        .chunks[&upper[0]]
        .cluster
        .unwrap();
    assert_eq!(
        app.world().get::<Cluster>(first_body).unwrap().chunks.len(),
        upper.len()
    );
    let mut saw_fast_contact = false;
    let mut max_impulse = 0.0_f32;
    let mut max_speed = 0.0_f32;
    for tick in 0..300 {
        app.update();
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
    let first_level = first_floor.part_id.rsplit_once('-').unwrap().0;
    let next_floor_y = records
        .values()
        .filter(|record| {
            record.kind == "floor" && record.part_id.rsplit_once('-').unwrap().0 != first_level
        })
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
        .filter(|bond| {
            upper.contains(&bond.a) && upper.contains(&bond.b) && !instance.bonds[&bond.id]
        })
        .map(|bond| bond.center[1])
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), y| {
            (min.min(y), max.max(y))
        });
    eprintln!("broken bond source_y={:.3}..{:.3}", broken_y.0, broken_y.1);
    let expected_mass = manifest
        .chunks
        .iter()
        .filter(|record| upper.contains(&record.id))
        .map(|record| f64::from(record.mass))
        .sum::<f64>();
    let actual_mass = instance
        .clusters
        .iter()
        .map(|body| f64::from(app.world().get::<Mass>(*body).unwrap().0))
        .sum::<f64>();
    assert!((actual_mass - expected_mass).abs() < expected_mass * 1e-5);
    assert!(
        manifest.bonds.iter().any(|bond| upper.contains(&bond.a)
            && upper.contains(&bond.b)
            && !instance.bonds[&bond.id]
            && bond.center[1] > 20.0
            && bond.center[1] < roof_y
            && records[bond.a.as_str()].kind != "roof"
            && records[bond.b.as_str()].kind != "roof"),
        "Landing only chipped its directly contacting floor; no load reached internal upper-storey bonds"
    );
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
    let initial = process_dynamic_impact(app.world(), &mut state).unwrap();
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
    {
        let mut instance = app
            .world_mut()
            .get_mut::<BuildingInstance>(building)
            .unwrap();
        for id in ids {
            instance.bonds.insert(id, false);
        }
        instance.topology_dirty = true;
    }
    crate::physics::rebuild_clusters(app.world_mut());
    let before = app
        .world()
        .get::<BuildingInstance>(building)
        .unwrap()
        .bonds
        .clone();
    assert!(
        app.world()
            .get::<BuildingInstance>(building)
            .unwrap()
            .clusters
            .len()
            > 1
    );
    assert!(process_dynamic_impact(app.world(), &mut state).is_none());
    assert_eq!(state.released, 1);
    assert_eq!(state.invalidated, 0);
    assert!(state.shocks.is_empty());
    assert!(state.pending.is_empty());
    assert_eq!(
        app.world().get::<BuildingInstance>(building).unwrap().bonds,
        before
    );
}

#[test]
fn inertial_impact_loads_balance_and_do_not_depend_on_a_numerical_gauge() {
    use crate::stress_smoke::{LEFT_SUPPORT, RIGHT_SUPPORT, beam_manifest};
    let manifest = beam_manifest();
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
    let case = inertial_loads(&manifest, &chunks, &impulses).unwrap();
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
    let manifest = crate::stress_smoke::beam_manifest();
    let chunks = manifest
        .chunks
        .iter()
        .map(|chunk| chunk.id.clone())
        .collect::<Vec<_>>();
    let total_mass = manifest
        .chunks
        .iter()
        .map(|chunk| f64::from(chunk.mass))
        .sum::<f64>();
    let center = manifest
        .chunks
        .iter()
        .map(|record| {
            record
                .transform()
                .transform_point(record.center_of_mass.into())
                .as_dvec3()
                * f64::from(record.mass)
        })
        .sum::<DVec3>()
        / total_mass;
    let linear = DVec3::new(2.0, -5.0, 3.0);
    let angular = DVec3::new(0.0, 0.0, 0.7);
    let impulses = manifest
        .chunks
        .iter()
        .map(|record| {
            let r = record
                .transform()
                .transform_point(record.center_of_mass.into())
                .as_dvec3()
                - center;
            let inertia =
                DMat3::from_cols_array_2d(&record.inertia.map(|column| column.map(f64::from)));
            (
                record.id.clone(),
                ImpulseLoad {
                    linear: (linear + angular.cross(r)) * f64::from(record.mass),
                    angular: inertia * angular,
                },
            )
        })
        .collect();
    let case = inertial_loads(&manifest, &chunks, &impulses).unwrap();
    for load in case.loads.values() {
        assert!(DVec3::from_array(load.force).length() < 1e-7);
        assert!(DVec3::from_array(load.moment).length() < 1e-7);
    }
}

#[test]
fn local_impacts_use_facet_directions_and_are_invariant_to_planar_subdivision() {
    let mut app = app();
    let building = spawn_beam(app.world_mut(), Vec3::ZERO);
    let instance = app.world().get::<BuildingInstance>(building).unwrap();
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
    let mut pending = BTreeMap::new();
    queue_local_damage(&planar, instance, building, BEAM, impulse, &mut pending);
    assert!(
        pending.is_empty(),
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
    queue_local_damage(&folded, instance, building, BEAM, impulse, &mut pending);
    assert_eq!(pending.len(), 1);
    assert!(
        (pending[&(building, LEFT_BOND.to_string())] - 1.2).abs() < 1e-5,
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
    let mut second = BTreeMap::new();
    queue_local_damage(&subdivided, instance, building, BEAM, impulse, &mut second);
    assert_eq!(
        pending.keys().collect::<Vec<_>>(),
        second.keys().collect::<Vec<_>>()
    );
    for (key, value) in pending {
        assert!((value - second[&key]).abs() < 1e-6);
    }
}
