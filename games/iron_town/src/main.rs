//! Iron Town client. Launch shortcuts are described in `oz_client::launch`.

use bevy::prelude::*;
use iron_town::IronTownPlugin;
use oz_client::{ClientPlugin, launch::launch_from_args};
use oz_sim::prelude::*;

fn main() {
    App::new()
        .add_plugins((ClientPlugin { title: "IRON TOWN".into() }, SimPlugin, IronTownPlugin))
        .add_systems(Startup, launch_from_args)
        .run();
}
