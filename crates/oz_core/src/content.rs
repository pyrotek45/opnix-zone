//! Content definitions and the [`Catalog`] that a game hands to the engine.
//!
//! The engine crates never hard-code a weapon, enemy or building. A game
//! builds a `Catalog` and both the simulation and the client read from it.

use crate::{
    arsenal::{AttachmentTable, ModKind, WeaponDef},
    economy::Cost,
    model::{BlockModel, Paint},
};
use glam::Vec3;

/// How an enemy fights.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Brain {
    /// Walks up and hits things.
    Melee,
    /// Keeps its distance and lobs projectiles.
    Ranged { range: f32, shot_speed: f32 },
    /// Hovers at `altitude` and strafes.
    Flyer { altitude: f32, range: f32, shot_speed: f32 },
}

/// What an enemy goes for first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prefers {
    Objective,
    Players,
    Structures,
}

#[derive(Clone, Debug)]
pub struct EnemyDef {
    pub name: &'static str,
    pub hp: f32,
    pub speed: f32,
    /// Hit sphere radius; its centre sits `radius` above the feet.
    pub radius: f32,
    pub damage: f32,
    pub attack_interval: f32,
    pub reward: u32,
    /// Spawn budget cost; also gates how early the enemy can appear.
    pub threat: u32,
    pub first_wave: u32,
    pub brain: Brain,
    pub prefers: Prefers,
    pub boss: bool,
    pub gibs: Paint,
    pub model: BlockModel,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StructureKind {
    Turret { range: f32, damage: f32, interval: f32 },
    /// Extracts ore per second from the deposit it is built on.
    Drill { rate: f32 },
    Wall,
}

#[derive(Clone, Debug)]
pub struct StructureDef {
    pub name: &'static str,
    pub blurb: &'static str,
    pub cost: Cost,
    pub hp: f32,
    /// Collision half-extents.
    pub half: Vec3,
    pub kind: StructureKind,
    pub model: BlockModel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpgradeEffect {
    DrillRate,
    MiningPower,
    ObjectiveHull,
    TurretDamage,
    PlayerArmor,
}

#[derive(Clone, Debug)]
pub struct UpgradeDef {
    pub name: &'static str,
    pub blurb: &'static str,
    pub cost: Cost,
    pub max_level: u8,
    pub effect: UpgradeEffect,
    /// Added to the effect's multiplier for every level.
    pub per_level: f32,
}

/// One tab in a station window. The client knows how to draw each kind from the catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StationPage {
    Weapons,
    Attachments,
    Mods,
    Build,
    Upgrades,
    Mission,
}

impl StationPage {
    pub fn title(self) -> &'static str {
        match self {
            StationPage::Weapons => "WEAPONS", StationPage::Attachments => "ATTACHMENTS", StationPage::Mods => "MODS",
            StationPage::Build => "BUILD", StationPage::Upgrades => "UPGRADES", StationPage::Mission => "MISSION",
        }
    }
}

/// A walk-up terminal: stand next to it, press interact, a window opens.
#[derive(Clone, Debug)]
pub struct StationDef {
    pub name: &'static str,
    pub blurb: &'static str,
    pub accent: Paint,
    pub pages: Vec<StationPage>,
    pub model: BlockModel,
}

/// A mode-specific button on a station's mission page (e.g. "call nightfall").
/// Sent to the authority as `PlayerCommand::Mode(index)`; the mode's director applies it.
#[derive(Clone, Debug)]
pub struct ModeAction {
    pub name: &'static str,
    pub blurb: &'static str,
    pub cost: Cost,
}

/// Visual flavour of a simulated projectile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShotLook {
    Rocket = 0,
    Acid = 1,
    Bolt = 2,
}

impl ShotLook {
    pub fn from_index(i: u8) -> Self {
        [ShotLook::Rocket, ShotLook::Acid, ShotLook::Bolt][(i as usize).min(2)]
    }

    pub fn paint(self) -> Paint {
        [Paint::Glow, Paint::Green, Paint::Red][self as usize]
    }
}

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub weapons: Vec<WeaponDef>,
    pub attachments: AttachmentTable,
    pub mods: Vec<ModKind>,
    pub enemies: Vec<EnemyDef>,
    pub structures: Vec<StructureDef>,
    pub upgrades: Vec<UpgradeDef>,
    pub stations: Vec<StationDef>,
    pub mode_actions: Vec<ModeAction>,
    /// Model shown for the objective / beacon core.
    pub objective_model: BlockModel,
}

impl Catalog {
    /// Multiplier for an effect given the team's upgrade levels: `1 + sum(level * per_level)`.
    pub fn bonus(&self, levels: &[u8], effect: UpgradeEffect) -> f32 {
        1.0 + self.upgrades.iter().zip(levels).filter(|(u, _)| u.effect == effect).map(|(u, level)| u.per_level * *level as f32).sum::<f32>()
    }
}
