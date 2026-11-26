//! Ember Atoll client. Run with `--help`-style shortcuts described in `oz_client::launch`.

use bevy::prelude::*;
use ember_atoll::EmberAtollPlugin;
use oz_client::{ClientPlugin, launch::launch_from_args};
use oz_sim::prelude::*;

fn main() {
    App::new()
        .add_plugins((ClientPlugin { title: "EMBER ATOLL".into() }, SimPlugin, EmberAtollPlugin))
        .add_systems(Startup, launch_from_args)
        .run();
}
