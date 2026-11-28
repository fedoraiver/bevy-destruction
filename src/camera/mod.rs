use bevy::{
    input::mouse::AccumulatedMouseMotion,
    prelude::*,
    window::{CursorGrabMode, CursorOptions},
};

use crate::{
    menu::PauseMenu,
    scene::{BuildingBounds, BuildingInstance, DemoCamera},
};

pub const FIELD_OF_VIEW: f32 = std::f32::consts::FRAC_PI_3;
const FLIGHT_SPEED: f32 = 6.0;
pub(crate) const LOOK_SENSITIVITY: f32 = 0.002;
const PITCH_LIMIT: f32 = 89.0_f32.to_radians();

pub struct FreeCameraPlugin;

impl Plugin for FreeCameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraControl>().add_systems(
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
#[cfg_attr(feature = "acceptance-test", derive(Reflect), reflect(Component))]
pub struct FreeCamera {
    initialized: bool,
    yaw: f32,
    pitch: f32,
}

#[derive(Component)]
pub struct Crosshair;

#[derive(Resource)]
#[cfg_attr(feature = "acceptance-test", derive(Reflect), reflect(Resource))]
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

fn initial_position(bounds: BuildingBounds, aspect: f32) -> Vec3 {
    let center = bounds.center();
    let size = bounds.max - bounds.min;
    let distance =
        (size.y * 0.5).max(size.x * 0.5 / aspect.max(0.01)) * 1.15 / (FIELD_OF_VIEW * 0.5).tan();
    Vec3::new(center.x, center.y, bounds.max.z + distance.max(1.0))
}

pub(crate) fn initialize_camera(
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

#[cfg(test)]
mod tests;
