//! Content definitions and the [`Catalog`] that a game hands to the engine.
//!
//! The engine crates never hard-code a weapon, enemy or building. A game
//! builds a `Catalog` and both the simulation and the client read from it.

use crate::{
    arsenal::{AttachmentTable, ModKind, SLOT_COUNT, WeaponDef, carried_weight},
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
    /// Thrown frag or mine.
    Grenade = 3,
    Knife = 4,
    /// Smoke and flash canisters.
    Canister = 5,
    /// A placed proximity mine.
    Mine = 6,
    /// A killstreak attack drone.
    Drone = 7,
    /// The airstrike jet.
    Jet = 8,
    /// The recon UAV: a radar plane circling the map.
    RadarPlane = 9,
    /// A bomb falling from the jet.
    Bomb = 10,
    /// An artillery shell coming down.
    Shell = 11,
}

impl ShotLook {
    pub fn from_index(i: u8) -> Self {
        [ShotLook::Rocket, ShotLook::Acid, ShotLook::Bolt, ShotLook::Grenade, ShotLook::Knife, ShotLook::Canister, ShotLook::Mine, ShotLook::Drone, ShotLook::Jet, ShotLook::RadarPlane, ShotLook::Bomb, ShotLook::Shell][(i as usize).min(11)]
    }

    pub fn paint(self) -> Paint {
        [Paint::Glow, Paint::Green, Paint::Red, Paint::Iron, Paint::Metal, Paint::Concrete, Paint::Red, Paint::Dark, Paint::Dark, Paint::Steel, Paint::Iron, Paint::Iron][self as usize]
    }

    /// Killstreak aircraft and what they drop: drawn as full models, not a dart.
    pub fn is_air(self) -> bool {
        matches!(self, ShotLook::Drone | ShotLook::Jet | ShotLook::RadarPlane | ShotLook::Bomb | ShotLook::Shell)
    }

    /// Size of the projectile's mesh.
    pub fn scale(self) -> [f32; 3] {
        match self {
            ShotLook::Grenade => [0.13, 0.13, 0.15],
            ShotLook::Knife => [0.04, 0.05, 0.32],
            ShotLook::Canister => [0.1, 0.1, 0.2],
            ShotLook::Mine => [0.22, 0.06, 0.22],
            ShotLook::Drone => [0.7, 0.16, 0.7],
            _ => [0.16, 0.16, 0.34],
        }
    }
}

/// A passive bonus chosen with a loadout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PerkEffect {
    /// Movement speed multiplier.
    Speed(f32),
    /// Extra maximum health.
    Health(f32),
    /// Reload time multiplier.
    Reload(f32),
    /// Spread multiplier.
    Spread(f32),
    /// Kills refill a magazine's worth of reserve ammo.
    Scavenger,
    /// Never shows on enemy radar, even when firing loud.
    Ghost,
    /// Explosive damage multiplier.
    FlakJacket(f32),
    /// Aim-down-sights and swap speed multiplier.
    Handling(f32),
    /// Hip-fire spread multiplier.
    Hipfire(f32),
    /// Enemies glow hot while you aim, through smoke and darkness.
    Thermal,
    /// A night-vision visor you can switch on.
    NightVision,
}

impl PerkEffect {
    /// Kilograms the perk adds to the load: armour plates are heavy.
    pub fn weight(self) -> f32 {
        match self {
            PerkEffect::Health(hp) => hp * 0.1,
            _ => 0.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PerkDef {
    pub name: &'static str,
    pub blurb: &'static str,
    pub effect: PerkEffect,
}

/// Throwable or usable equipment chosen with a loadout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GearKind {
    /// Thrown explosive.
    Frag { damage: f32, radius: f32 },
    /// Instant self-heal.
    Stim { heal: f32 },
    /// Thrown canister: a cloud that blocks sight for a while.
    Smoke { radius: f32, seconds: f32 },
    /// Thrown stun: blinds and deafens anyone facing it.
    Flash { radius: f32, seconds: f32 },
    /// Thrown proximity mine: arms, then blows when an enemy comes close.
    Mine { damage: f32, radius: f32 },
    /// Thrown blade: fast, silent, lethal on a direct hit.
    Knife { damage: f32 },
    /// Refills reserves for you and teammates close by.
    Ammo { radius: f32 },
}

impl GearKind {
    /// Thrown gear flies as a projectile; the rest takes effect at once.
    pub fn thrown(&self) -> bool {
        !matches!(self, GearKind::Stim { .. } | GearKind::Ammo { .. })
    }
}

#[derive(Clone, Debug)]
pub struct GearDef {
    pub name: &'static str,
    pub blurb: &'static str,
    pub kind: GearKind,
    /// Uses per life.
    pub count: u8,
    /// Kilograms per item carried.
    pub weight: f32,
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
    pub perks: Vec<PerkDef>,
    pub gear: Vec<GearDef>,
    /// Model shown for the objective / beacon core.
    pub objective_model: BlockModel,
}

impl Catalog {
    /// Kilograms carried: guns (catalog index and fitted parts), the perk's armour and
    /// `gear_count` pieces of gear. Feeds [`crate::arsenal::load_speed`].
    pub fn load(&self, guns: &[(usize, [u8; SLOT_COUNT])], perk: u8, gear: u8, gear_count: u8) -> f32 {
        let guns: f32 = guns.iter().filter_map(|(index, parts)| self.weapons.get(*index).map(|def| carried_weight(def, parts, &self.attachments))).sum();
        let perk = self.perks.get(perk as usize).map_or(0.0, |p| p.effect.weight());
        let gear = self.gear.get(gear as usize).map_or(0.0, |g| g.weight * gear_count as f32);
        guns + perk + gear
    }

    /// Multiplier for an effect given the team's upgrade levels: `1 + sum(level * per_level)`.
    pub fn bonus(&self, levels: &[u8], effect: UpgradeEffect) -> f32 {
        1.0 + self.upgrades.iter().zip(levels).filter(|(u, _)| u.effect == effect).map(|(u, level)| u.per_level * *level as f32).sum::<f32>()
    }
}
