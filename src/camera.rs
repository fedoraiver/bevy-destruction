use bevy::{
    input::mouse::AccumulatedMouseMotion,
    prelude::*,
    window::{CursorGrabMode, CursorOptions},
};

use crate::{
    DemoOptions,
    scene::{BuildingBounds, BuildingInstance, DemoCamera},
};

pub const FIELD_OF_VIEW: f32 = std::f32::consts::FRAC_PI_3;
const FLIGHT_SPEED: f32 = 6.0;
pub(crate) const LOOK_SENSITIVITY: f32 = 0.002;
const PITCH_LIMIT: f32 = 89.0_f32.to_radians();

pub struct FreeCameraPlugin;

impl Plugin for FreeCameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraControl>()
            .init_resource::<ScriptedFlight>()
            .add_systems(
                Update,
                (
                    initialize_camera.after(crate::scene::instantiate_pending),
                    move_camera,
                )
                    .chain()
                    .before(crate::interaction::cursor_damage),
            );
    }
}

#[derive(Component, Default)]
pub struct FreeCamera {
    initialized: bool,
    yaw: f32,
    pitch: f32,
}

#[derive(Component)]
pub struct Crosshair;

#[derive(Resource)]
pub struct CameraControl {
    pub captured: bool,
    pub suppress_click: bool,
}

impl Default for CameraControl {
    fn default() -> Self {
        Self {
            captured: true,
            suppress_click: false,
        }
    }
}

#[derive(Default)]
pub struct FlightInput {
    pub movement: Vec3,
    pub mouse: Vec2,
}

#[derive(Resource, Default)]
pub struct ScriptedFlight(pub Option<FlightInput>);

fn initial_position(bounds: BuildingBounds, aspect: f32) -> Vec3 {
    let center = bounds.center();
    let size = bounds.max - bounds.min;
    let distance =
        (size.y * 0.5).max(size.x * 0.5 / aspect.max(0.01)) * 1.15 / (FIELD_OF_VIEW * 0.5).tan();
    Vec3::new(center.x, center.y, bounds.max.z + distance.max(1.0))
}

fn initialize_camera(
    instances: Query<&BuildingInstance>,
    windows: Query<&Window>,
    mut cameras: Query<(&mut Transform, &mut FreeCamera), With<DemoCamera>>,
) {
    let Some(bounds) = instances.iter().find_map(|instance| instance.bounds) else {
        return;
    };
    let Ok(window) = windows.single() else {
        return;
    };
    for (mut transform, mut camera) in &mut cameras {
        if camera.initialized {
            continue;
        }
        transform.translation = initial_position(bounds, window.width() / window.height());
        transform.rotation = Quat::IDENTITY;
        camera.initialized = true;
    }
}

fn keyboard_movement(keys: &ButtonInput<KeyCode>) -> Vec3 {
    let value = |positive: bool, negative: bool| positive as u8 as f32 - negative as u8 as f32;
    Vec3::new(
        value(keys.pressed(KeyCode::KeyD), keys.pressed(KeyCode::KeyA)),
        value(
            keys.pressed(KeyCode::Space),
            keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight),
        ),
        value(keys.pressed(KeyCode::KeyS), keys.pressed(KeyCode::KeyW)),
    )
}

fn apply_flight(transform: &mut Transform, camera: &mut FreeCamera, input: FlightInput, dt: f32) {
    camera.yaw -= input.mouse.x * LOOK_SENSITIVITY;
    camera.pitch =
        (camera.pitch - input.mouse.y * LOOK_SENSITIVITY).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    transform.rotation = Quat::from_euler(EulerRot::YXZ, camera.yaw, camera.pitch, 0.0);
    let direction = transform.rotation * Vec3::new(input.movement.x, 0.0, input.movement.z)
        + Vec3::Y * input.movement.y;
    transform.translation += direction.normalize_or_zero() * FLIGHT_SPEED * dt;
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn move_camera(
    time: Res<Time>,
    options: Res<DemoOptions>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    mouse: Res<AccumulatedMouseMotion>,
    mut control: ResMut<CameraControl>,
    mut scripted: ResMut<ScriptedFlight>,
    mut windows: Query<(&Window, &mut CursorOptions)>,
    mut cameras: Query<(&mut Transform, &mut FreeCamera), With<DemoCamera>>,
    mut crosshairs: Query<&mut Visibility, With<Crosshair>>,
) {
    let Ok((window, mut cursor)) = windows.single_mut() else {
        return;
    };
    control.suppress_click = false;
    let smoke = options.smoke_output.is_some() || options.physics_smoke_output.is_some();
    if !smoke {
        if !window.focused {
            control.captured = false;
        } else if keys.just_pressed(KeyCode::Tab) {
            control.captured = !control.captured;
            control.suppress_click = true;
        } else if !control.captured && buttons.just_pressed(MouseButton::Left) {
            control.captured = true;
            control.suppress_click = true;
        }
    }
    cursor.grab_mode = if control.captured && !smoke {
        CursorGrabMode::Locked
    } else {
        CursorGrabMode::None
    };
    cursor.visible = !control.captured || smoke;
    for mut visibility in &mut crosshairs {
        *visibility = if control.captured {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
    if !control.captured || control.suppress_click {
        return;
    }
    let input = if smoke {
        scripted.0.take().unwrap_or_default()
    } else {
        FlightInput {
            movement: keyboard_movement(&keys),
            mouse: mouse.delta,
        }
    };
    for (mut transform, mut camera) in &mut cameras {
        if camera.initialized {
            apply_flight(
                &mut transform,
                &mut camera,
                FlightInput {
                    movement: input.movement,
                    mouse: input.mouse,
                },
                time.delta_secs(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn controller_app() -> (App, Entity, Entity, Entity) {
        let mut app = App::new();
        app.init_resource::<Time>()
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<AccumulatedMouseMotion>()
            .init_resource::<CameraControl>()
            .init_resource::<ScriptedFlight>()
            .insert_resource(DemoOptions {
                asset: "buildings/rectangle.destruction.json".into(),
                smoke_output: None,
                ..default()
            })
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
}
