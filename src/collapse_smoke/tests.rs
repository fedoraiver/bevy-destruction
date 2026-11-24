use super::*;

#[test]
fn upper_static_support_contact_is_detected_when_the_lower_member_has_no_impulse() {
    let mut world = World::new();
    let bulk = world.spawn_empty().id();
    let static_body = world.spawn_empty().id();
    let other_body = world.spawn_empty().id();
    let lower = world.spawn_empty().id();
    let upper = world.spawn_empty().id();
    let bulk_collider = world.spawn_empty().id();
    let supports = [("0004", lower, static_body), ("0005", upper, static_body)];
    let contact = ([Some(bulk), Some(static_body)], [bulk_collider, upper], 3.0);
    assert_eq!(
        contacted_static_support(bulk, &supports, [contact]),
        Some("0005")
    );
    assert_eq!(
        contacted_static_support(bulk, &supports, [(contact.0, contact.1, 0.0)]),
        None
    );
    assert_eq!(
        contacted_static_support(
            bulk,
            &supports,
            [([Some(other_body), Some(static_body)], contact.1, 3.0)]
        ),
        None
    );
    assert_eq!(
        contacted_static_support(
            bulk,
            &supports,
            [([Some(bulk), Some(other_body)], contact.1, 3.0)]
        ),
        None
    );
}
use crate::{
    physics::DestructionPhysicsPlugin,
    scene::{BuildingBounds, ChunkIdentity, chunk_collider, spawn_bonds},
    stress_runtime::StressRuntimePlugin,
};
use bevy::time::TimeUpdateStrategy;

fn production_manifest() -> Manifest {
    serde_json::from_str(include_str!(
        "../../assets/buildings/rectangle.destruction.json"
    ))
    .unwrap()
}

#[test]
fn production_support_selection_uses_geometry_and_is_independent_of_record_order() {
    let mut manifest = production_manifest();
    let masses = manifest
        .chunks
        .iter()
        .map(|record| {
            let collider = chunk_collider(record).unwrap();
            (
                record.id.clone(),
                collider.mass(manifest.materials[&record.material].density),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let selected = select_collapse_support(&manifest, |id| masses[id]).unwrap();
    assert!(selected.support_path.len() >= 2 && !selected.removed.is_empty());
    for pair in selected.support_path.windows(2) {
        assert!(manifest.bonds.iter().any(|bond| {
            [bond.a.as_str(), bond.b.as_str()].contains(&pair[0].as_str())
                && [bond.a.as_str(), bond.b.as_str()].contains(&pair[1].as_str())
        }));
    }
    manifest.chunks.reverse();
    manifest.bonds.reverse();
    manifest.anchors.reverse();
    let repeated = select_collapse_support(&manifest, |id| masses[id]).unwrap();
    assert_eq!(selected.support_path, repeated.support_path);
    assert_eq!(selected.removed, repeated.removed);
}

#[test]
fn production_geometric_support_path_fails_and_the_bulk_tips_against_a_static_stub() {
    let manifest = production_manifest();
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, TransformPlugin, PhysicsPlugins::default()))
        .init_resource::<Assets<DestructibleAsset>>()
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .insert_resource(Time::<Fixed>::from_hz(60.0))
        .insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f64(1.0 / 60.0),
        ))
        .insert_resource(Gravity(Vec3::NEG_Y * 9.81))
        .add_plugins((DestructionPhysicsPlugin, StressRuntimePlugin));
    app.finish();
    app.cleanup();
    app.update();
    let asset = app
        .world_mut()
        .resource_mut::<Assets<DestructibleAsset>>()
        .add(DestructibleAsset {
            manifest: manifest.clone(),
            gltf: Handle::default(),
        });
    let building = app
        .world_mut()
        .spawn((Transform::IDENTITY, Visibility::Visible))
        .id();
    let mut instance = BuildingInstance::pending(asset);
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for record in &manifest.chunks {
        let local = record.transform();
        for &point in record.colliders.iter().flat_map(|proxy| &proxy.vertices) {
            let point = local.transform_point(Vec3::from_array(point));
            min = min.min(point);
            max = max.max(point);
        }
        let collider = chunk_collider(record).unwrap();
        let density = manifest.materials[&record.material].density;
        let mass_properties = collider.mass_properties(density);
        let entity = app
            .world_mut()
            .spawn((
                local,
                GlobalTransform::from(local),
                ChildOf(building),
                ChunkIdentity {
                    building,
                    id: record.id.clone(),
                },
                collider,
                ColliderDensity(density),
            ))
            .id();
        app.world_mut().entity_mut(entity).insert(ChunkInstance {
            alive: true,
            triangles: Vec::new(),
            transform: local,
            mass_properties,
            material: record.material.clone(),
        });
        instance.chunks.insert(record.id.clone(), entity);
    }
    instance.bounds = Some(BuildingBounds { min, max });
    spawn_bonds(
        &mut app.world_mut().commands(),
        building,
        &mut instance,
        &manifest.bonds,
    );
    app.world_mut().flush();
    let chunk_at = |id: &str| {
        app.world()
            .get::<ChunkInstance>(instance.chunks[id])
            .unwrap()
    };
    let selected =
        select_collapse_support(&manifest, |id| chunk_at(id).mass_properties.mass).unwrap();
    let upper = manifest
        .chunks
        .iter()
        .filter(|record| {
            record
                .transform()
                .transform_point(record.center_of_mass.into())
                .y
                >= selected.upper_floor_y
        })
        .map(|record| {
            let chunk = chunk_at(&record.id);
            (
                record.id.clone(),
                (chunk.center_in_building(), chunk.mass_properties.mass),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let upper_mass = upper.values().map(|(_, mass)| mass).sum::<f32>();
    let expected_mass = instance
        .chunks
        .iter()
        .filter(|(id, _)| !selected.removed.contains(*id))
        .map(|(id, _)| f64::from(chunk_at(id).mass_properties.mass))
        .sum::<f64>();
    app.world_mut()
        .entity_mut(building)
        .insert((instance, NeedsClusterRebuild));
    for _ in 0..5 {
        app.update();
    }
    assert_eq!(
        app.world()
            .get::<StressState>(building)
            .unwrap()
            .broken_total,
        0,
        "Intact production rectangle is unstable"
    );
    {
        let world = app.world_mut();
        for id in &selected.removed {
            let entity = world.get::<BuildingInstance>(building).unwrap().chunks[id];
            world.get_mut::<ChunkInstance>(entity).unwrap().alive = false;
        }
        for bond in &manifest.bonds {
            if selected.removed.contains(&bond.a) || selected.removed.contains(&bond.b) {
                let entity = world.get::<BuildingInstance>(building).unwrap().bonds[&bond.id];
                world.get_mut::<BondInstance>(entity).unwrap().active = false;
            }
        }
        world.entity_mut(building).insert(NeedsClusterRebuild);
    }
    let (mut released_mass, mut drop, mut tilt, mut support_contact) =
        (0.0_f32, 0.0_f32, 0.0_f32, false);
    for _ in 0..270 {
        app.update();
        let instance = app.world().get::<BuildingInstance>(building).unwrap();
        let static_stubs = selected
            .support_path
            .iter()
            .filter_map(|id| {
                let entity = instance.chunks[id];
                let body = app.world().get::<ColliderOf>(entity)?.body;
                (app.world().get::<RigidBody>(body) == Some(&RigidBody::Static))
                    .then_some((entity, body))
            })
            .collect::<Vec<_>>();
        let mut mass = 0.0_f64;
        for body in crate::physics::building_clusters(app.world(), building) {
            let position = app.world().get::<Position>(body).unwrap().0;
            let rotation = app.world().get::<Rotation>(body).unwrap().0;
            assert!(position.is_finite() && rotation.is_finite());
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
            mass += f64::from(app.world().get::<ComputedMass>(body).unwrap().value());
            if app.world().get::<RigidBody>(body) != Some(&RigidBody::Dynamic) {
                continue;
            }
            let tracked = app
                .world()
                .get::<Cluster>(body)
                .unwrap()
                .chunks
                .iter()
                .filter_map(|id| upper.get(id))
                .collect::<Vec<_>>();
            let current_mass = tracked.iter().map(|(_, mass)| *mass).sum::<f32>();
            if tracked.len() < 2 || current_mass < upper_mass * 0.25 {
                continue;
            }
            released_mass = released_mass.max(current_mass);
            let center = tracked
                .iter()
                .map(|(point, mass)| *point * *mass)
                .sum::<Vec3>()
                / current_mass;
            drop = drop.max(center.y - (position + rotation * center).y);
            tilt = tilt.max((rotation * Vec3::Y).angle_between(Vec3::Y));
            support_contact |= app
                .world()
                .resource::<ContactGraph>()
                .iter_active_touching()
                .any(|pair| {
                    static_stubs.iter().any(|&(entity, stub)| {
                        [pair.collider1, pair.collider2].contains(&entity)
                            && [pair.body1, pair.body2].contains(&Some(body))
                            && [pair.body1, pair.body2].contains(&Some(stub))
                            && pair.total_normal_impulse_magnitude() > 0.0
                    })
                });
        }
        assert!((mass - expected_mass).abs() < expected_mass * 1e-5);
    }
    let stress = app.world().get::<StressState>(building).unwrap();
    eprintln!(
        "production collapse schema={} path={:?} removed={} upper_mass={upper_mass} released_mass={released_mass} drop={drop} tilt={tilt} stub_contact={support_contact} strength_breaks={} failures={}",
        manifest.schema_version,
        selected.support_path,
        selected.removed.len(),
        stress.broken_total,
        stress.failures
    );
    assert_eq!(stress.failures, 0);
    assert!(stress.reports.iter().all(|report| report.valid));
    assert!(
        stress.broken_total > 0
            && released_mass >= upper_mass * 0.25
            && drop > 0.5
            && tilt > 0.1
            && support_contact,
        "Geometry-selected support did not produce coherent tipping collapse"
    );
}
