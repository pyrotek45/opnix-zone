//! `oz_sim` - the authoritative, headless game simulation.
//!
//! This crate knows nothing about rendering, windows or audio. It runs the
//! same way inside a single-player game, a listen server ("host"), a
//! dedicated server, or a unit test. Clients run it too, but with
//! [`NetRole::Client`]: the authority systems switch off and entities are
//! driven by snapshots instead.
//!
//! Games compose a shooter out of these parts:
//!
//! * [`bots`] - AI players that drive the very same controller as humans.
//! * [`player`] - input, first-person movement, health and respawn.
//! * [`weapons`] - data-driven firing: hitscan, projectiles, beams, mods.
//! * [`combat`] - hits, splash, projectiles and the spatial grid.
//! * [`enemy`] - catalog-driven AI (melee, ranged, flyers).
//! * [`structures`] - turrets, walls, drills and ore deposits.
//! * [`orders`] - every purchase / build / upgrade is a validated [`state::PlayerCommand`].
//! * [`lagcomp`] - host-side rewind so remote players hit what they saw.
//! * [`nav`] - walkability grid and A* pathfinding for AI.
//! * [`versus`] - lobby, teams, bot backfill, scoring and rematches.
//! * [`net`] - host/client replication on top of `oz_net`.
//! * [`session`] - starting and tearing down a match.
//!
//! A game mode is "a catalog + rules + a director plugin" (see `games/ember_atoll`).

pub mod bots;
pub mod combat;
pub mod enemy;
pub mod lagcomp;
pub mod nav;
pub mod net;
pub mod orders;
pub mod player;
pub mod session;
pub mod state;
pub mod structures;
pub mod versus;
pub mod weapons;

use bevy::prelude::*;

pub mod prelude {
    pub use crate::combat::{Grid, Hit, Mine, Projectile, SmokeCloud, Solid, smoke_between, smoke_scale};
    pub use crate::enemy::{Enemy, spawn_enemy};
    pub use crate::net::{NetId, NetLink};
    pub use crate::bots::Bot;
    pub use crate::player::{Arsenal, EYE, Health, Loadout, LocalPlayer, Mover, Player, PlayerInput, buttons, flags, stance};
    pub use crate::session::{Content, Map, MatchEntity, MatchSetup, ModeDef, Modes, PendingMatch, Station, Statics, end_match};
    pub use crate::state::{CommandEvent, FxEvent, LobbyAction, MatchPhase, MatchState, NetRole, Phase, PlayerCommand, RosterEntry, Rules, SimRng};
    pub use crate::structures::{OreNode, Structure};
    pub use crate::{SimPlugin, SimSet, authority, sim_dt};
    pub use oz_core::{
        arsenal::*, content::*, economy::*, model::*, rng::Rng, synth::Sfx,
        worldgen::{Biome, World as GameMap, WorldRecipe},
    };
}

/// Frame ordering for everything that touches the simulation.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum SimSet {
    /// Receive network traffic.
    Receive,
    /// Authority: process player commands (purchases, builds).
    Orders,
    /// Authority: mode directors, AI, weapons, structures.
    Logic,
    /// Authority: apply hits, deaths, rewards.
    Resolve,
    /// Send snapshots / inputs.
    Send,
}

/// Run condition: true on machines that own the simulation (offline, host, dedicated).
pub fn authority(role: Res<state::NetRole>) -> bool {
    *role != state::NetRole::Client
}

/// Clamped frame delta so a hitch never tunnels entities through the world.
pub fn sim_dt(time: &Time) -> f32 {
    time.delta_secs().min(0.05)
}

pub struct SimPlugin;

impl Plugin for SimPlugin {
    fn build(&self, app: &mut App) {
        use state::MatchPhase::Active;
        app.init_state::<state::MatchPhase>()
            .init_resource::<state::NetRole>()
            .init_resource::<session::Modes>()
            .init_resource::<session::PendingMatch>()
            .init_resource::<net::NetLink>()
            .init_resource::<nav::NavScratch>()
            .init_resource::<lagcomp::LagHistory>()
            .init_resource::<net::ServerClock>()
            .init_resource::<net::Ping>()
            .init_resource::<nav::NavBudget>()
            .add_event::<state::FxEvent>()
            .add_event::<state::CommandEvent>()
            .add_event::<combat::Hit>()
            .configure_sets(Update, (SimSet::Receive, SimSet::Orders, SimSet::Logic, SimSet::Resolve, SimSet::Send).chain())
            .configure_sets(Update, (SimSet::Orders, SimSet::Logic, SimSet::Resolve).run_if(in_state(Active)).run_if(authority))
            .add_systems(Update, (net::poll_joining, session::start_pending_match).chain().before(SimSet::Receive))
            .add_systems(Update, (net::host_receive, net::client_receive).in_set(SimSet::Receive).run_if(in_state(Active)))
            .add_systems(Update, orders::process_commands.in_set(SimSet::Orders))
            .add_systems(Update, (versus::run_versus, versus::fill_teams).chain().in_set(SimSet::Orders).after(orders::process_commands).run_if(versus::versus))
            .add_systems(
                Update,
                (
                    combat::rebuild_grid,
                    lagcomp::record_history,
                    nav::refill_budget,
                    player::vitals,
                    bots::drive_bots,
                    weapons::fire_weapons,
                    enemy::think,
                    enemy::act,
                    structures::run_structures,
                    combat::fly_projectiles,
                    combat::run_mines,
                )
                    .chain()
                    .in_set(SimSet::Logic),
            )
            .add_systems(Update, (combat::resolve_hits, player::presentation_flags, versus::update_roster).chain().in_set(SimSet::Resolve))
            .add_systems(Update, (player::move_players, net::smooth_remote).after(SimSet::Receive).before(SimSet::Orders).run_if(in_state(Active)))
            .add_systems(Update, (net::host_send, net::client_send).in_set(SimSet::Send).run_if(in_state(Active)))
            .add_systems(Update, combat::tick_smoke.run_if(in_state(Active)))
            .add_systems(OnExit(Active), session::cleanup_match);
    }
}
