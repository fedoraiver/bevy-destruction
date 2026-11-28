use bevy::{
    input::mouse::AccumulatedMouseMotion,
    prelude::*,
    window::{CursorGrabMode, CursorOptions},
};

use crate::{menu::PauseMenu, scene::DemoCamera};

use super::{CameraControl, Crosshair, FlightInput, FreeCamera};

pub(super) const FLIGHT_SPEED: f32 = 6.0;
pub(super) const LOOK_SENSITIVITY: f32 = 0.002;
pub(super) const PITCH_LIMIT: f32 = 89.0_f32.to_radians();

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

pub(super) fn apply_flight(
    transform: &mut Transform,
    camera: &mut FreeCamera,
    input: FlightInput,
    dt: f32,
) {
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
    menu: Option<Res<PauseMenu>>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    mouse: Res<AccumulatedMouseMotion>,
    mut control: ResMut<CameraControl>,
    mut windows: Query<(&Window, &mut CursorOptions)>,
    mut cameras: Query<(&mut Transform, &mut FreeCamera), With<DemoCamera>>,
    mut crosshairs: Query<&mut Visibility, With<Crosshair>>,
) {
    let Ok((window, mut cursor)) = windows.single_mut() else {
        return;
    };
    control.suppress_click = false;
    if menu.is_some_and(|menu| menu.open) {
        control.captured = false;
        control.suppress_click = true;
    } else if !window.focused {
        control.captured = false;
    } else if keys.just_pressed(KeyCode::Tab) {
        control.captured = !control.captured;
        control.suppress_click = true;
    } else if !control.captured && buttons.just_pressed(MouseButton::Left) {
        control.captured = true;
        control.suppress_click = true;
    }
    cursor.grab_mode = if control.captured {
        CursorGrabMode::Locked
    } else {
        CursorGrabMode::None
    };
    cursor.visible = !control.captured;
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
    let input = FlightInput {
        movement: keyboard_movement(&keys),
        mouse: mouse.delta,
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
