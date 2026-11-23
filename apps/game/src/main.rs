use bevy::prelude::*;
use bevy_destruction::{DestructionPlugin, asset_plugin, desktop_window, parse_options};

fn main() -> AppExit {
    let options = parse_options(std::env::args().skip(1)).unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(2);
    });

    App::new()
        .add_plugins(DefaultPlugins.set(asset_plugin()).set(desktop_window()))
        .insert_resource(options)
        .add_plugins(DestructionPlugin)
        .run()
}
