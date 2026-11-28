use bevy::{prelude::*, time::Virtual};

use crate::{camera::CameraControl, scene::DemoRoot};

use super::{MenuAction, PauseMenu, reload::begin_reload};

pub(super) fn resume(
    menu: &mut PauseMenu,
    time: &mut Time<Virtual>,
    control: &mut CameraControl,
    mouse: &mut ButtonInput<MouseButton>,
) {
    menu.open = false;
    menu.status.clear();
    time.unpause();
    control.captured = menu.resume_capture;
    mouse.clear_just_pressed(MouseButton::Left);
}

#[allow(clippy::too_many_arguments)]
pub(super) fn read_menu_input(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut menu: ResMut<PauseMenu>,
    mut time: ResMut<Time<Virtual>>,
    mut control: ResMut<CameraControl>,
    server: Res<AssetServer>,
    root: Single<Entity, With<DemoRoot>>,
    mut actions: MessageReader<MenuAction>,
    mut exit: MessageWriter<AppExit>,
) {
    let clicked = actions.read().last().copied();
    let action = if keys.just_pressed(KeyCode::Escape) {
        if !menu.open {
            menu.open = true;
            menu.resume_capture = control.captured;
            time.pause();
            return;
        }
        Some(MenuAction::Resume)
    } else if menu.open {
        clicked
    } else {
        None
    };

    if matches!(action, Some(MenuAction::Quit)) {
        exit.write(AppExit::Success);
        return;
    }
    if menu.candidate.is_some() {
        return;
    }
    match action {
        Some(MenuAction::Resume) => resume(&mut menu, &mut time, &mut control, &mut mouse),
        Some(MenuAction::Reload) => begin_reload(&mut commands, &server, *root, &mut menu),
        None | Some(MenuAction::Quit) => {}
    }
}
