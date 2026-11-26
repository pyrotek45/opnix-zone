//! Shared match state, rules, commands and presentation events.

use bevy::prelude::*;
use oz_core::{arsenal::ModKind, economy::Stock, model::Paint, rng::Rng, synth::Sfx, worldgen::WorldRecipe};
use serde::{Deserialize, Serialize};

/// Who owns the simulation on this machine.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NetRole {
    /// Single player: authority, no sockets.
    #[default]
    Offline,
    /// Authority that also accepts remote players.
    Host,
    /// Remote player: receives snapshots, sends input.
    Client,
}

#[derive(States, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MatchPhase {
    #[default]
    None,
    Active,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    /// Calm: build, mine, shop.
    #[default]
    Day,
    /// Under attack.
    Night,
    Won,
    Lost,
    /// Versus: gathering in the lobby before a match.
    Lobby,
    /// Versus: the match is being played.
    Live,
}

/// Match configuration. Chosen in the menu, sent to joining clients verbatim.
#[derive(Resource, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rules {
    /// Index into the registered [`crate::session::Modes`].
    pub mode: u8,
    pub recipe: WorldRecipe,
    /// Length of the calm phase; 0 disables it (pure horde).
    pub day_seconds: f32,
    /// Hit points of the defended objective; 0 = no objective, only players can lose.
    pub objective_hp: f32,
    /// Spawn ore deposits and allow mining.
    pub mining: bool,
    /// Enemy health / damage / count multiplier.
    pub difficulty: f32,
    pub start_stock: Stock,
    /// Bitmask of catalog weapons every player starts with.
    pub starter_weapons: u32,
    pub respawn_seconds: f32,
    /// Team-versus-team: players damage the other team, bots fill empty slots.
    pub pvp: bool,
    /// Players per team in versus modes (humans first, bots for the rest).
    pub team_size: u8,
    /// Team score that wins a versus match.
    pub score_limit: u32,
}

/// The replicated heart of a match. Small enough to send with every snapshot.
#[derive(Resource, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MatchState {
    pub phase: Phase,
    /// 1-based day / wave counter.
    pub day: u32,
    /// Seconds left in the day, or seconds elapsed in the night.
    pub timer: f32,
    /// 1 = full daylight, 0 = deep night. Smoothed by the director; drives lighting.
    pub daylight: f32,
    pub enemies_left: u32,
    pub objective_hp: f32,
    pub objective_max: f32,
    pub stock: Stock,
    /// One level per `Catalog::upgrades` entry.
    pub upgrades: Vec<u8>,
    pub kills: u32,
    /// Team scores in versus modes.
    pub scores: [u32; 2],
    /// Winning team once `phase` is `Won`.
    pub winner: u8,
    /// Everyone in the match, for lobbies, scoreboards and names.
    pub roster: Vec<RosterEntry>,
    /// Versus: players per team (humans first, bots fill the rest).
    pub team_size: u8,
    /// Seconds until the lobby launches the match; 0 = not counting down.
    pub countdown: f32,
    /// Player ids owed a kill this frame. Authority bookkeeping, never sent.
    #[serde(skip)]
    pub credits: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RosterEntry {
    pub id: u8,
    pub name: String,
    pub team: u8,
    pub bot: bool,
    pub ready: bool,
    pub alive: bool,
    pub kills: u16,
    pub deaths: u16,
    /// Round trip to the host in ms (0 for the host and bots).
    pub ping: u16,
    /// What they are holding, so everyone sees their real attachments.
    pub weapon: u8,
    pub parts: [u8; oz_core::arsenal::SLOT_COUNT],
}

impl MatchState {
    pub fn over(&self) -> bool {
        matches!(self.phase, Phase::Won | Phase::Lost)
    }

    pub fn damage_objective(&mut self, amount: f32) {
        self.objective_hp = (self.objective_hp - amount).max(0.0);
    }
}

/// Gameplay randomness (authority only). Presentation uses its own RNG.
#[derive(Resource)]
pub struct SimRng(pub Rng);

/// Everything a player can ask the authority to do. Validated in [`crate::orders`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PlayerCommand {
    BuyWeapon(u8),
    BuyAmmo(u8),
    /// Buys (if needed) and equips an attachment.
    EquipPart { weapon: u8, slot: u8, variant: u8 },
    BuyMod { weapon: u8, kind: ModKind },
    Build { kind: u16, x: f32, z: f32, yaw: f32 },
    Upgrade(u8),
    /// Versus modes: choose weapons, perk and gear for the next spawn.
    Loadout(crate::player::Loadout),
    /// Mode-specific action, interpreted by the mode's director (e.g. "call nightfall").
    Mode(u8),
    /// Versus lobby controls.
    Lobby(LobbyAction),
    /// A line of text chat.
    Chat(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LobbyAction {
    ToggleReady,
    /// Host only: launch the match.
    Start,
    SwitchTeam,
    /// Host only: change players per team.
    TeamSize(i8),
}

#[derive(Event, Clone, Debug)]
pub struct CommandEvent {
    pub player: u8,
    pub command: PlayerCommand,
}

/// One-shot presentation cues. Emitted by the authority, replayed on every client.
#[derive(Event, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum FxEvent {
    /// A shot or beam. `by` is a player id, or 255 for turrets.
    /// `local` marks a shot the client predicted itself; the host never sets it.
    Shot { by: u8, sound: u8, from: Vec3, to: Vec3, beam: bool, tracer: Paint, quiet: bool, local: bool },
    /// `victim` was taken down by `by` (a player id).
    Killed { victim: u8, by: u8 },
    /// A line of chat from a player.
    Chat { name: String, text: String },
    Explosion { at: Vec3, radius: f32 },
    EnemyDied { at: Vec3, kind: u16 },
    StructureLost { at: Vec3 },
    Arc { from: Vec3, to: Vec3 },
    Mined { at: Vec3, ore: u8 },
    HitMarker { player: u8, kill: bool },
    Hurt { player: u8 },
    Sound { sfx: Sfx, at: Option<Vec3> },
    /// A UI sound for one player only.
    Cue { to: u8, sfx: Sfx },
    /// `to` = one player id, or 255 for everyone.
    Notice { to: u8, text: String },
}

impl FxEvent {
    pub fn notice(text: impl Into<String>) -> Self {
        FxEvent::Notice { to: 255, text: text.into() }
    }

    pub fn tell(player: u8, text: impl Into<String>) -> Self {
        FxEvent::Notice { to: player, text: text.into() }
    }
}
