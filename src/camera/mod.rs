mod flight;
mod framing;

use bevy::prelude::*;

pub(crate) use flight::move_camera;
pub(crate) use framing::initialize_camera;

#[cfg(test)]
use crate::scene::{BuildingBounds, BuildingInstance, DemoCamera};
#[cfg(test)]
use bevy::{
    input::mouse::AccumulatedMouseMotion,
    window::{CursorGrabMode, CursorOptions},
};
#[cfg(test)]
use flight::{FLIGHT_SPEED, LOOK_SENSITIVITY, PITCH_LIMIT, apply_flight};
#[cfg(test)]
use framing::initial_position;

pub const FIELD_OF_VIEW: f32 = std::f32::consts::FRAC_PI_3;

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
#[cfg_attr(feature = "brp", derive(Reflect), reflect(Component))]
pub struct FreeCamera {
    initialized: bool,
    yaw: f32,
    pitch: f32,
}

#[derive(Component)]
pub struct Crosshair;

#[derive(Resource)]
#[cfg_attr(feature = "brp", derive(Reflect), reflect(Resource))]
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

#[cfg(test)]
mod tests;
