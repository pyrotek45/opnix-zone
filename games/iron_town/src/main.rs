//! Iron Town client. Launch shortcuts are described in `oz_client::launch`.

// Release builds on Windows open no console window behind the game.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use bevy::prelude::*;
use iron_town::IronTownPlugin;
use oz_client::{ClientPlugin, launch::launch_from_args};
use oz_sim::prelude::*;

fn main() {
    App::new()
        .add_plugins((
            ClientPlugin {
                title: "IRON TOWN".into(),
            },
            SimPlugin,
            IronTownPlugin,
        ))
        .add_systems(Startup, launch_from_args)
        .run();
}
