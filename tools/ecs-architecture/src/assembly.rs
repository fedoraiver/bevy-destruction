use bevy::prelude::*;

use crate::{DestructionPlugin, asset_plugin, desktop_window};

pub fn build_app() -> App {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(asset_plugin()).set(desktop_window()))
        .add_plugins(DestructionPlugin);
    app
}
