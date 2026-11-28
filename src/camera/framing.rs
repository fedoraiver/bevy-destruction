use bevy::prelude::*;

use crate::scene::{BuildingBounds, BuildingInstance, DemoCamera};

use super::{FIELD_OF_VIEW, FreeCamera};

pub(super) fn initial_position(bounds: BuildingBounds, aspect: f32) -> Vec3 {
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
