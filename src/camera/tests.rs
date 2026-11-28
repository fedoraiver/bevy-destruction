use super::*;

fn controller_app() -> (App, Entity, Entity, Entity) {
    let mut app = App::new();
    app.init_resource::<Time>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<AccumulatedMouseMotion>()
        .init_resource::<CameraControl>()
        .init_resource::<ScriptedFlight>()
        .init_resource::<ProbeOptions>()
        .add_systems(Update, move_camera);
    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(std::time::Duration::from_secs_f32(0.5));
    let window = app
        .world_mut()
        .spawn((
            Window {
                focused: true,
                ..default()
            },
            CursorOptions::default(),
        ))
        .id();
    let camera = app
        .world_mut()
        .spawn((
            DemoCamera,
            FreeCamera {
                initialized: true,
                ..default()
            },
            Transform::default(),
        ))
        .id();
    let crosshair = app.world_mut().spawn((Crosshair, Visibility::Visible)).id();
    (app, window, camera, crosshair)
}

fn controller_frame(app: &mut App) {
    app.update();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .clear();
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .clear();
    app.world_mut()
        .resource_mut::<AccumulatedMouseMotion>()
        .delta = Vec2::ZERO;
}

#[test]
fn focus_loss_stops_flight_and_recapture_consumes_the_click_and_motion() {
    let (mut app, window, camera, crosshair) = controller_app();
    controller_frame(&mut app);
    assert_eq!(
        app.world().get::<CursorOptions>(window).unwrap().grab_mode,
        CursorGrabMode::Locked
    );
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyW);
    app.world_mut()
        .resource_mut::<AccumulatedMouseMotion>()
        .delta = Vec2::new(50.0, 30.0);
    app.world_mut().get_mut::<Window>(window).unwrap().focused = false;
    controller_frame(&mut app);
    assert_eq!(
        *app.world().get::<Transform>(camera).unwrap(),
        Transform::default()
    );
    assert!(!app.world().resource::<CameraControl>().captured);
    let cursor = app.world().get::<CursorOptions>(window).unwrap();
    assert!(cursor.visible);
    assert_eq!(cursor.grab_mode, CursorGrabMode::None);
    assert_eq!(
        *app.world().get::<Visibility>(crosshair).unwrap(),
        Visibility::Hidden
    );

    app.world_mut().get_mut::<Window>(window).unwrap().focused = true;
    controller_frame(&mut app);
    assert!(!app.world().resource::<CameraControl>().captured);
    assert_eq!(
        *app.world().get::<Transform>(camera).unwrap(),
        Transform::default()
    );

    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    app.world_mut()
        .resource_mut::<AccumulatedMouseMotion>()
        .delta = Vec2::new(200.0, 150.0);
    controller_frame(&mut app);
    let control = app.world().resource::<CameraControl>();
    assert!(control.captured && control.suppress_click);
    assert_eq!(
        *app.world().get::<Transform>(camera).unwrap(),
        Transform::default()
    );
    assert_eq!(
        *app.world().get::<Visibility>(crosshair).unwrap(),
        Visibility::Visible
    );
    assert!(!app.world().get::<CursorOptions>(window).unwrap().visible);

    app.world_mut()
        .resource_mut::<AccumulatedMouseMotion>()
        .delta = Vec2::new(20.0, 0.0);
    controller_frame(&mut app);
    let transform = app.world().get::<Transform>(camera).unwrap();
    assert!((transform.translation.length() - FLIGHT_SPEED * 0.5).abs() < 0.001);
    assert!(transform.rotation.angle_between(Quat::IDENTITY) > 0.01);
    assert!(!app.world().resource::<CameraControl>().suppress_click);
}

#[test]
fn tab_releases_and_recaptures_without_moving_on_the_transition() {
    let (mut app, window, camera, _) = controller_app();
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyW);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Tab);
    controller_frame(&mut app);
    assert!(!app.world().resource::<CameraControl>().captured);
    assert_eq!(
        *app.world().get::<Transform>(camera).unwrap(),
        Transform::default()
    );
    assert_eq!(
        app.world().get::<CursorOptions>(window).unwrap().grab_mode,
        CursorGrabMode::None
    );
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::Tab);
    controller_frame(&mut app);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Tab);
    controller_frame(&mut app);
    assert!(app.world().resource::<CameraControl>().captured);
    assert!(app.world().resource::<CameraControl>().suppress_click);
    assert_eq!(
        *app.world().get::<Transform>(camera).unwrap(),
        Transform::default()
    );
    controller_frame(&mut app);
    assert!(app.world().get::<Transform>(camera).unwrap().translation.z < -2.9);
}

#[test]
fn resize_and_late_instances_preserve_the_initialized_camera_pose() {
    let mut app = App::new();
    app.add_systems(Update, initialize_camera);
    let window = app.world_mut().spawn(Window::default()).id();
    let camera = app
        .world_mut()
        .spawn((DemoCamera, FreeCamera::default(), Transform::default()))
        .id();
    let bounds = BuildingBounds {
        min: Vec3::ZERO,
        max: Vec3::new(8.0, 6.4, 6.0),
    };
    let mut instance = BuildingInstance::pending(Handle::default());
    instance.bounds = Some(bounds);
    app.world_mut().spawn(instance);
    app.update();
    assert!(app.world().get::<FreeCamera>(camera).unwrap().initialized);
    let window_data = app.world().get::<Window>(window).unwrap();
    assert!(
        app.world()
            .get::<Transform>(camera)
            .unwrap()
            .translation
            .distance(initial_position(
                bounds,
                window_data.width() / window_data.height()
            ))
            < 0.001
    );

    let pose = Transform::from_xyz(-4.0, 10.0, -8.0).with_rotation(Quat::from_euler(
        EulerRot::YXZ,
        0.8,
        -0.3,
        0.0,
    ));
    *app.world_mut().get_mut::<Transform>(camera).unwrap() = pose;
    {
        let mut state = app.world_mut().get_mut::<FreeCamera>(camera).unwrap();
        state.yaw = 0.8;
        state.pitch = -0.3;
    }
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .resolution
        .set(300.0, 900.0);
    let mut later_instance = BuildingInstance::pending(Handle::default());
    later_instance.bounds = Some(BuildingBounds {
        min: Vec3::splat(-50.0),
        max: Vec3::splat(100.0),
    });
    app.world_mut().spawn(later_instance);
    app.update();
    assert_eq!(*app.world().get::<Transform>(camera).unwrap(), pose);
    let state = app.world().get::<FreeCamera>(camera).unwrap();
    assert_eq!(state.yaw, 0.8);
    assert_eq!(state.pitch, -0.3);
}

#[test]
fn flight_follows_view_keeps_vertical_world_aligned_and_normalizes_diagonals() {
    let mut transform = Transform::default();
    let mut camera = FreeCamera::default();
    apply_flight(
        &mut transform,
        &mut camera,
        FlightInput {
            mouse: Vec2::new(-std::f32::consts::FRAC_PI_2 / LOOK_SENSITIVITY, 0.0),
            ..default()
        },
        0.0,
    );
    apply_flight(
        &mut transform,
        &mut camera,
        FlightInput {
            movement: Vec3::NEG_Z,
            ..default()
        },
        1.0,
    );
    assert!(transform.translation.distance(Vec3::NEG_X * FLIGHT_SPEED) < 0.001);
    let start = transform.translation;
    apply_flight(
        &mut transform,
        &mut camera,
        FlightInput {
            movement: Vec3::Y,
            ..default()
        },
        1.0,
    );
    assert!(
        transform
            .translation
            .distance(start + Vec3::Y * FLIGHT_SPEED)
            < 0.001
    );
    let start = transform.translation;
    apply_flight(
        &mut transform,
        &mut camera,
        FlightInput {
            movement: Vec3::new(1.0, 1.0, -1.0),
            ..default()
        },
        1.0,
    );
    assert!(((transform.translation - start).length() - FLIGHT_SPEED).abs() < 0.001);
}

#[test]
fn flight_speed_is_frame_independent_and_look_does_not_flip() {
    let simulate = |steps: usize| {
        let mut transform = Transform::default();
        let mut camera = FreeCamera::default();
        for _ in 0..steps {
            apply_flight(
                &mut transform,
                &mut camera,
                FlightInput {
                    movement: Vec3::NEG_Z,
                    ..default()
                },
                1.0 / steps as f32,
            );
        }
        transform
    };
    assert!(simulate(30).translation.distance(simulate(120).translation) < 0.001);
    let mut transform = Transform::default();
    let mut camera = FreeCamera::default();
    apply_flight(
        &mut transform,
        &mut camera,
        FlightInput {
            mouse: Vec2::new(100.0, -10000.0),
            ..default()
        },
        0.0,
    );
    assert!((camera.yaw + 0.2).abs() < 0.001);
    assert_eq!(camera.pitch, PITCH_LIMIT);
    assert!(transform.right().y.abs() < 0.001);
}

#[test]
fn initial_perspective_framing_covers_front_for_wide_and_tall_windows() {
    let bounds = BuildingBounds {
        min: Vec3::ZERO,
        max: Vec3::new(8.0, 6.4, 6.0),
    };
    for aspect in [0.5, 1.0, 16.0 / 9.0, 3.0] {
        let position = initial_position(bounds, aspect);
        let half_height = (position.z - bounds.max.z) * (FIELD_OF_VIEW * 0.5).tan();
        assert!(half_height * 2.0 >= 6.4 * 1.15 - 0.001);
        assert!(half_height * 2.0 * aspect >= 8.0 * 1.15 - 0.001);
    }
}
