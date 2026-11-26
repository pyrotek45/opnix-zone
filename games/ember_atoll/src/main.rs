//! Ember Atoll client.
//!
//! ```text
//! ember_atoll                      open the main menu
//! ember_atoll --solo [--mode N]    jump straight into a solo match
//! ember_atoll --host [--mode N]    host a match on the default port
//! ember_atoll --join ADDRESS       join a host
//! ```

use bevy::prelude::*;
use ember_atoll::EmberAtollPlugin;
use oz_client::{ClientPlugin, settings::Settings};
use oz_net::{DEFAULT_PORT, normalize_address};
use oz_sim::{net, prelude::*};

fn main() {
    App::new()
        .add_plugins((ClientPlugin { title: "EMBER ATOLL".into() }, SimPlugin, EmberAtollPlugin))
        .add_systems(Startup, launch_from_args)
        .run();
}

/// Optional command-line shortcuts past the menu.
fn launch_from_args(mut commands: Commands, modes: Res<Modes>, settings: Res<Settings>, mut pending: ResMut<PendingMatch>) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let seed = value("--seed").and_then(|s| s.parse().ok()).unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(7, |d| d.as_nanos() as u64));
    let rules = || {
        let index = value("--mode").and_then(|m| m.parse::<usize>().ok()).unwrap_or(0);
        let mode = modes.0.get(index).filter(|mode| mode.playable)?;
        let mut rules = (mode.rules)(seed);
        rules.mode = index as u8;
        Some(rules)
    };
    if let Some(address) = value("--join") {
        net::join(&mut commands, &normalize_address(&address), settings.callsign.clone());
    } else if has("--host") {
        match rules().map(|rules| net::host_setup(rules, settings.callsign.clone(), &format!("0.0.0.0:{DEFAULT_PORT}"), false)) {
            Some(Ok(setup)) => pending.0 = Some(setup),
            Some(Err(error)) => error!("could not host: {error}"),
            None => error!("that mode is not playable"),
        }
    } else if has("--solo") {
        if let Some(rules) = rules() {
            pending.0 = Some(MatchSetup::offline(rules, settings.callsign.clone()));
        }
    }
}
