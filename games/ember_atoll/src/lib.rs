//! Ember Atoll - defend the beacon from robotic sea creatures by night, mine
//! and grow an automated mining empire across the islands by day.
//!
//! This crate is a thin game built from the Opnix Zone parts: it supplies
//! content ([`content`]), a mode director ([`director`]) and the list of
//! modes. Everything else - movement, weapons, AI, networking, rendering,
//! menus - comes from `oz_sim` and `oz_client`.

pub mod content;
pub mod director;

use bevy::prelude::*;
use oz_sim::prelude::*;

/// Indexes into the registered [`Modes`].
pub mod mode {
    pub const FRONTIER: u8 = 0;
    pub const HORDE: u8 = 1;
}

/// Day-mining / night-defense campaign. The flagship mode.
pub fn frontier_rules(seed: u64) -> Rules {
    Rules {
        mode: mode::FRONTIER,
        recipe: WorldRecipe::archipelago(seed, Biome::EmberAtoll),
        day_seconds: 210.0,
        objective_hp: 600.0,
        mining: true,
        difficulty: 1.0,
        start_stock: Stock { credits: 220, ore: [30, 0, 0] },
        starter_weapons: 0b11,
        respawn_seconds: 10.0,
        pvp: false,
        team_size: 0,
        score_limit: 0,
        hunt: false,
    }
}

/// Back-to-back waves with no objective and no mining: last squad standing.
pub fn horde_rules(seed: u64) -> Rules {
    Rules {
        mode: mode::HORDE,
        recipe: WorldRecipe { islets: 2, ..WorldRecipe::archipelago(seed, Biome::SunkenReach) },
        day_seconds: 0.0,
        objective_hp: 0.0,
        mining: false,
        difficulty: 1.0,
        start_stock: Stock { credits: 400, ore: [0; 3] },
        starter_weapons: 0b110,
        respawn_seconds: 20.0,
        pvp: false,
        team_size: 0,
        score_limit: 0,
        hunt: false,
    }
}

/// Headless half of the game: registers modes and the director.
/// Used by the client, the dedicated server and the tests alike.
pub struct EmberAtollPlugin;

impl Plugin for EmberAtollPlugin {
    fn build(&self, app: &mut App) {
        let planned = |name, blurb| ModeDef { name, blurb, playable: false, rules: frontier_rules, catalog: content::catalog };
        app.insert_resource(Modes(vec![
            ModeDef { name: "FRONTIER", blurb: "Mine by day. Hold the beacon by night. Automate the islands.", playable: true, rules: frontier_rules, catalog: content::catalog },
            ModeDef { name: "HORDE", blurb: "Endless waves, no beacon, no mining. Last squad standing.", playable: true, rules: horde_rules, catalog: content::catalog },
            planned("OUTBREAK", "PLANNED - round-based undead survival with wall buys and mystery crates."),
            planned("DEATHMATCH", "PLANNED - fast team and free-for-all firefights with loadouts."),
            planned("CONQUEST", "PLANNED - large maps, capture points, vehicles and squads."),
            planned("BASTION", "PLANNED - tower-defense build-up along fixed attack lanes."),
        ]))
        .add_plugins(director::SiegePlugin);
    }
}
