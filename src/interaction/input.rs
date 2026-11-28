use bevy::prelude::*;

use crate::{camera::CameraControl, scene::DemoCamera};

use super::DamageRequest;

#[allow(clippy::too_many_arguments)]
pub(crate) fn cursor_damage(
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    cameras: Query<(Entity, &Camera), With<DemoCamera>>,
    transforms: TransformHelper,
    mut requests: MessageWriter<DamageRequest>,
    control: Res<CameraControl>,
) {
    if !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    let (Ok(window), Ok((entity, camera))) = (windows.single(), cameras.single()) else {
        return;
    };
    if !control.captured || control.suppress_click || !window.focused {
        return;
    }
    let Ok(transform) = transforms.compute_global_transform(entity) else {
        return;
    };
    if let Ok(ray) = camera.viewport_to_world(&transform, window.size() * 0.5) {
        requests.write(DamageRequest { ray });
    }
}
