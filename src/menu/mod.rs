mod input;
mod reload;
mod view;

use bevy::prelude::*;

use crate::{camera, scene};

use input::read_menu_input;
pub(crate) use reload::PendingReload;
use reload::complete_reload;
use view::{spawn_menu, update_menu};

pub struct PauseMenuPlugin;

impl Plugin for PauseMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PauseMenu>()
            .add_message::<MenuAction>()
            .add_systems(Startup, spawn_menu)
            .add_systems(
                Update,
                (complete_reload, read_menu_input, update_menu)
                    .chain()
                    .after(scene::instantiate_pending)
                    .before(camera::initialize_camera),
            );
    }
}

#[derive(Resource, Default)]
pub(crate) struct PauseMenu {
    pub open: bool,
    resume_capture: bool,
    candidate: Option<Entity>,
    status: String,
}

#[derive(Clone, Copy, Message)]
enum MenuAction {
    Resume,
    Reload,
    Quit,
}
