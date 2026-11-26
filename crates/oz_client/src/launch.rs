//! Command-line shortcuts past the menu, shared by every game binary.
//!
//! ```text
//! game                      open the main menu
//! game --solo [--mode N]    jump straight into an offline match
//! game --host [--mode N]    host a match on the default port
//! game --join ADDRESS       join a host
//! ```
//! `--seed N` fixes the map and `--teams N` sets the team size in versus modes.

use crate::settings::Settings;
use bevy::prelude::*;
use oz_net::{DEFAULT_PORT, normalize_address};
use oz_sim::{net, prelude::*};

/// Add as a `Startup` system.
pub fn launch_from_args(mut commands: Commands, modes: Res<Modes>, settings: Res<Settings>, mut pending: ResMut<PendingMatch>) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let seed = value("--seed").and_then(|s| s.parse().ok()).unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(7, |d| d.as_nanos() as u64));
    let rules = || {
        let index = value("--mode").and_then(|m| m.parse::<usize>().ok()).unwrap_or(0);
        let mode = modes.0.get(index).filter(|mode| mode.playable)?;
        let mut rules = (mode.rules)(seed);
        rules.mode = index as u8;
        if let Some(size) = value("--teams").and_then(|t| t.parse::<u8>().ok()) {
            rules.team_size = size.clamp(1, 6);
        }
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
