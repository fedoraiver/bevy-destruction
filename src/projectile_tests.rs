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
