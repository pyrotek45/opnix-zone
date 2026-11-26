//! Weapons as kits of parts.
//!
//! A gun is a [`WeaponDef`] - stats, handling, and a [`GunFrame`] describing
//! its shape - plus a [`WeaponState`] recording what is fitted. Seven slots
//! (optic, muzzle, barrel, underbarrel, magazine, stock and ammunition) each
//! change both the numbers ([`resolve`]) and the geometry ([`assemble`]).
//! Any frame accepts any part its [`GunClass`] allows, so a new gun is one
//! struct and every attachment already fits it.

use crate::{
    economy::Cost,
    model::{Anim, BlockModel, Paint},
};
use serde::{Deserialize, Serialize};

pub const SLOT_COUNT: usize = 7;
pub const SLOTS: [&str; SLOT_COUNT] = ["OPTIC", "MUZZLE", "BARREL", "UNDERBARREL", "MAGAZINE", "STOCK", "AMMO"];

/// Slot indices.
pub mod slot {
    pub const OPTIC: usize = 0;
    pub const MUZZLE: usize = 1;
    pub const BARREL: usize = 2;
    pub const UNDER: usize = 3;
    pub const MAGAZINE: usize = 4;
    pub const STOCK: usize = 5;
    pub const AMMO: usize = 6;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GunClass {
    Pistol,
    Smg,
    Rifle,
    Shotgun,
    Sniper,
    Lmg,
    Launcher,
    /// Tools such as the mining laser; takes no attachments.
    Tool,
}

impl GunClass {
    pub const fn bit(self) -> u16 {
        1 << self as u16
    }

    pub fn name(self) -> &'static str {
        match self {
            GunClass::Pistol => "PISTOL", GunClass::Smg => "SMG", GunClass::Rifle => "RIFLE", GunClass::Shotgun => "SHOTGUN",
            GunClass::Sniper => "SNIPER", GunClass::Lmg => "LMG", GunClass::Launcher => "LAUNCHER", GunClass::Tool => "TOOL",
        }
    }
}

/// Every class that takes attachments.
pub const GUNS: u16 = 0x7f;

/// Bitmask of the listed classes.
pub const fn only(classes: &[GunClass]) -> u16 {
    let mut mask = 0;
    let mut i = 0;
    while i < classes.len() {
        mask |= classes[i].bit();
        i += 1;
    }
    mask
}

/// Every class except the listed ones (tools never take parts).
pub const fn except(classes: &[GunClass]) -> u16 {
    GUNS & !only(classes)
}

/// Decides which ammunition a weapon can load.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Caliber {
    Pistol,
    Rifle,
    Shell,
    Magnum,
    Rocket,
    Energy,
    None,
}

/// How the trigger and the action behave. Also picks the firing animation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Auto,
    Semi,
    /// Fires this many rounds per trigger pull.
    Burst(u8),
    /// Bolt worked by hand after every shot.
    Bolt,
    /// Pump worked by hand after every shot.
    Pump,
}

/// How the weapon reloads. Also picks the reload animation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReloadStyle {
    /// Swap the whole magazine (`reload` is the full time).
    Magazine,
    /// One round at a time (`reload` is per round); firing interrupts it.
    Single,
    /// Open the lid and lay in a belt.
    Belt,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FireMode {
    /// Instant ray.
    Hitscan,
    /// Spawns a simulated projectile (rockets, grenades, plasma).
    Projectile { speed: f32, gravity: f32 },
    /// Continuous short-range beam; also the mining tool.
    Beam,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MagShape {
    Box,
    Curved,
    Drum,
    /// Shotgun tube under the barrel.
    Tube,
    Belt,
    /// Energy cell.
    Cell,
    /// Big launcher canister.
    Canister,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StockShape {
    None,
    Fixed,
    Skeleton,
    Heavy,
}

/// The shape of a gun. Attachments read these numbers to find their mounts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GunFrame {
    pub body: Paint,
    pub accent: Paint,
    pub furniture: Paint,
    /// Receiver width, height and length in metres.
    pub width: f32,
    pub height: f32,
    pub receiver: f32,
    /// Barrel length beyond the receiver, and its diameter.
    pub barrel: f32,
    pub bore: f32,
    pub mag: MagShape,
    pub stock: StockShape,
    pub handguard: bool,
}

#[derive(Clone, Debug)]
pub struct WeaponDef {
    pub name: &'static str,
    pub blurb: &'static str,
    pub price: Cost,
    pub class: GunClass,
    pub caliber: Caliber,
    pub action: Action,
    pub reload_style: ReloadStyle,
    pub mag: u16,
    /// Reserve ammo granted on purchase. Ignored when `infinite_reserve`.
    pub reserve: u16,
    pub infinite_reserve: bool,
    /// Credits for one refill pack of `mag * 2` rounds.
    pub ammo_price: u32,
    pub damage: f32,
    /// Seconds between shots (between rounds inside a burst).
    pub interval: f32,
    pub reload: f32,
    pub pellets: u8,
    /// Hip-fire cone half-angle in radians.
    pub spread: f32,
    /// Spread multiplier while aiming down sights.
    pub ads_spread: f32,
    pub range: f32,
    pub splash: f32,
    pub recoil: f32,
    pub mode: FireMode,
    pub ads_fov: f32,
    /// Ore units per second when aimed at a deposit. 0 = cannot mine.
    pub mining: f32,
    /// Index into the shot sound bank.
    pub sound: u8,
    /// Shape for modular guns. `None` uses `model` as-is (tools, oddities).
    pub frame: Option<GunFrame>,
    /// Parts fitted when the weapon is first acquired (e.g. a sniper's scope).
    pub defaults: [u8; SLOT_COUNT],
    pub model: BlockModel,
    /// Muzzle distance for frameless weapons.
    pub length: f32,
}

impl WeaponDef {
    pub fn auto(&self) -> bool {
        self.action == Action::Auto
    }

    /// Does this attachment (or ammo) fit this gun?
    pub fn accepts(&self, classes: u16) -> bool {
        self.frame.is_some() && classes & self.class.bit() != 0
    }
}

/// What one attachment does. Multipliers default to 1; `fov` overrides when non-zero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatMods {
    pub damage: f32,
    pub capacity: f32,
    pub reload: f32,
    pub recoil: f32,
    /// Spread in every stance.
    pub spread: f32,
    /// Extra spread multiplier for hip fire only.
    pub hipfire: f32,
    pub range: f32,
    pub fov: f32,
    /// Aim-down-sights and swap speed.
    pub handling: f32,
    /// Movement speed.
    pub mobility: f32,
    /// Hides the shooter from enemy radar and muffles the report.
    pub quiet: bool,
    /// 1 = magnified sight overlay, 2 = full sniper scope.
    pub scope: u8,
}

impl StatMods {
    pub const NONE: StatMods = StatMods {
        damage: 1.0, capacity: 1.0, reload: 1.0, recoil: 1.0, spread: 1.0, hipfire: 1.0, range: 1.0, fov: 0.0, handling: 1.0, mobility: 1.0,
        quiet: false, scope: 0,
    };
}

#[derive(Clone, Debug)]
pub struct Attachment {
    pub name: &'static str,
    pub blurb: &'static str,
    pub cost: u32,
    pub mods: StatMods,
    /// Classes that accept it (see [`only`] / [`except`]).
    pub classes: u16,
}

/// `table[slot][variant]` for the six hardware slots; variant 0 is always the standard part.
pub type AttachmentTable = Vec<Vec<Attachment>>;

/// A kind of ammunition. Picked per weapon from its caliber's list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AmmoDef {
    pub name: &'static str,
    pub blurb: &'static str,
    pub cost: u32,
    pub damage: f32,
    pub range: f32,
    pub spread: f32,
    pub splash: f32,
    /// Overrides the pellet count (slugs).
    pub pellets: Option<u8>,
    /// Punches through one thin wall at reduced damage.
    pub pierce: bool,
    /// Burn damage per second for a few seconds.
    pub burn: f32,
    pub tracer: Paint,
}

const STANDARD: AmmoDef = AmmoDef {
    name: "STANDARD", blurb: "Reliable all-round rounds", cost: 0, damage: 1.0, range: 1.0, spread: 1.0, splash: 0.0, pellets: None, pierce: false,
    burn: 0.0, tracer: Paint::Glow,
};
const HOLLOW: AmmoDef = AmmoDef { name: "HOLLOW POINT", blurb: "+20% damage, shorter range", cost: 70, damage: 1.2, range: 0.75, tracer: Paint::Spark, ..STANDARD };
const PIERCING: AmmoDef = AmmoDef { name: "ARMOR PIERCING", blurb: "Punches through thin walls", cost: 90, damage: 0.95, pierce: true, tracer: Paint::Blue, ..STANDARD };
const INCENDIARY: AmmoDef = AmmoDef { name: "INCENDIARY", blurb: "Sets targets burning", cost: 90, damage: 0.9, burn: 9.0, tracer: Paint::Red, ..STANDARD };
const EXPLOSIVE: AmmoDef = AmmoDef { name: "EXPLOSIVE", blurb: "Small blast on impact", cost: 120, damage: 0.85, splash: 1.6, tracer: Paint::Glow, ..STANDARD };

const PISTOL_AMMO: [AmmoDef; 4] = [AmmoDef { name: "FMJ", ..STANDARD }, HOLLOW, PIERCING, INCENDIARY];
const RIFLE_AMMO: [AmmoDef; 5] = [AmmoDef { name: "FMJ", ..STANDARD }, HOLLOW, PIERCING, INCENDIARY, EXPLOSIVE];
const MAGNUM_AMMO: [AmmoDef; 4] = [AmmoDef { name: "MATCH GRADE", blurb: "Flat and true", ..STANDARD }, PIERCING, INCENDIARY, EXPLOSIVE];
const SHELL_AMMO: [AmmoDef; 4] = [
    AmmoDef { name: "BUCKSHOT", blurb: "A wide spray of pellets", ..STANDARD },
    AmmoDef { name: "SLUG", blurb: "One heavy, accurate projectile", cost: 80, damage: 5.0, range: 2.2, spread: 0.15, pellets: Some(1), ..STANDARD },
    AmmoDef { name: "DRAGON'S BREATH", blurb: "Burning pellets, short range", cost: 110, damage: 0.8, range: 0.7, burn: 7.0, tracer: Paint::Red, ..STANDARD },
    AmmoDef { name: "FLECHETTE", blurb: "Darts that pierce cover", cost: 100, damage: 0.85, spread: 0.8, pierce: true, tracer: Paint::Blue, ..STANDARD },
];
const ROCKET_AMMO: [AmmoDef; 3] = [
    AmmoDef { name: "HIGH EXPLOSIVE", blurb: "The classic", ..STANDARD },
    AmmoDef { name: "THERMOBARIC", blurb: "Bigger blast, less punch", cost: 140, damage: 0.8, splash: 1.4, ..STANDARD },
    AmmoDef { name: "NAPALM", blurb: "Leaves everything burning", cost: 140, damage: 0.75, burn: 14.0, tracer: Paint::Red, ..STANDARD },
];
const ENERGY_AMMO: [AmmoDef; 2] = [
    AmmoDef { name: "STANDARD CELL", blurb: "Clean discharge", tracer: Paint::Blue, ..STANDARD },
    AmmoDef { name: "OVERCHARGE", blurb: "+30% damage, arcs through cover", cost: 150, damage: 1.3, pierce: true, tracer: Paint::Violet, ..STANDARD },
];
const NO_AMMO: [AmmoDef; 1] = [STANDARD];

/// Ammunition a caliber can load. Index 0 is the default.
pub fn ammo_for(caliber: Caliber) -> &'static [AmmoDef] {
    match caliber {
        Caliber::Pistol => &PISTOL_AMMO,
        Caliber::Rifle => &RIFLE_AMMO,
        Caliber::Magnum => &MAGNUM_AMMO,
        Caliber::Shell => &SHELL_AMMO,
        Caliber::Rocket => &ROCKET_AMMO,
        Caliber::Energy => &ENERGY_AMMO,
        Caliber::None => &NO_AMMO,
    }
}

/// A slot's options for one weapon: (variant index, name, blurb, cost), only those that fit.
pub fn options(def: &WeaponDef, table: &AttachmentTable, slot: usize) -> Vec<(usize, &'static str, &'static str, u32)> {
    if def.frame.is_none() {
        return Vec::new();
    }
    if slot == slot::AMMO {
        return ammo_for(def.caliber).iter().enumerate().map(|(i, a)| (i, a.name, a.blurb, a.cost)).collect();
    }
    table.get(slot).map_or(Vec::new(), |variants| {
        variants.iter().enumerate().filter(|(i, a)| *i == 0 || def.accepts(a.classes)).map(|(i, a)| (i, a.name, a.blurb, a.cost)).collect()
    })
}

/// Price of a part, and whether it fits at all.
pub fn part_cost(def: &WeaponDef, table: &AttachmentTable, slot: usize, variant: usize) -> Option<u32> {
    options(def, table, slot).into_iter().find(|(i, ..)| *i == variant).map(|(_, _, _, cost)| cost)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModKind {
    Arc,
    Frost,
    Venom,
    Scavenger,
    Repair,
    Heavy,
    Split,
    Capacitor,
}

pub const MOD_SLOTS: usize = 3;
pub const MOD_MAX_LEVEL: u8 = 3;

impl ModKind {
    pub const ALL: [ModKind; 8] = [ModKind::Arc, ModKind::Frost, ModKind::Venom, ModKind::Scavenger, ModKind::Repair, ModKind::Heavy, ModKind::Split, ModKind::Capacitor];

    pub fn name(self) -> &'static str {
        match self {
            ModKind::Arc => "ARC", ModKind::Frost => "FROST", ModKind::Venom => "VENOM", ModKind::Scavenger => "SCAVENGER",
            ModKind::Repair => "REPAIR", ModKind::Heavy => "HEAVY", ModKind::Split => "SPLIT", ModKind::Capacitor => "CAPACITOR",
        }
    }

    pub fn blurb(self) -> &'static str {
        match self {
            ModKind::Arc => "Hits chain to a nearby target", ModKind::Frost => "Hits slow the target",
            ModKind::Venom => "Hits deal damage over time", ModKind::Scavenger => "Hits pay bonus credits",
            ModKind::Repair => "Hits repair the objective", ModKind::Heavy => "More damage per shot",
            ModKind::Split => "Impacts splash nearby targets", ModKind::Capacitor => "Faster rate of fire",
        }
    }

    pub fn paint(self) -> Paint {
        match self {
            ModKind::Arc => Paint::Blue, ModKind::Frost => Paint::White, ModKind::Venom => Paint::Green, ModKind::Scavenger => Paint::Glow,
            ModKind::Repair => Paint::Aether, ModKind::Heavy => Paint::Red, ModKind::Split => Paint::Spark, ModKind::Capacitor => Paint::Violet,
        }
    }

    /// Price of installing (level 0 -> 1) or upgrading to `level + 1`.
    pub fn cost(self, level: u8) -> Cost {
        Cost::new(140, 0, 6, 4).scaled(level as u32)
    }
}

/// What one player owns and has fitted on one weapon.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WeaponState {
    pub owned: bool,
    pub ammo: u16,
    pub reserve: u16,
    /// Fitted variant per slot.
    pub equipped: [u8; SLOT_COUNT],
    /// Bitmask of purchased variants per slot (the standard part is always owned).
    pub purchased: [u8; SLOT_COUNT],
    pub mods: [Option<(ModKind, u8)>; MOD_SLOTS],
}

impl WeaponState {
    /// A not-yet-owned weapon with its default parts fitted.
    pub fn fresh(def: &WeaponDef) -> Self {
        let mut state = WeaponState { equipped: def.defaults, ..default_state() };
        for (slot, variant) in def.defaults.iter().enumerate() {
            state.purchased[slot] |= 1 << variant;
        }
        state
    }

    pub fn has_part(&self, slot: usize, variant: usize) -> bool {
        variant == 0 || self.purchased.get(slot).is_some_and(|mask| mask & (1 << variant.min(7)) != 0)
    }

    pub fn mod_level(&self, kind: ModKind) -> u8 {
        self.mods.iter().flatten().find(|(k, _)| *k == kind).map_or(0, |(_, level)| *level)
    }

    /// Installs or upgrades a mod. Fails when maxed or when all slots hold other mods.
    pub fn add_mod(&mut self, kind: ModKind) -> bool {
        if let Some((_, level)) = self.mods.iter_mut().flatten().find(|(k, _)| *k == kind) {
            if *level >= MOD_MAX_LEVEL {
                return false;
            }
            *level += 1;
            return true;
        }
        match self.mods.iter_mut().find(|slot| slot.is_none()) {
            Some(slot) => {
                *slot = Some((kind, 1));
                true
            }
            None => false,
        }
    }
}

fn default_state() -> WeaponState {
    WeaponState { owned: false, ammo: 0, reserve: 0, equipped: [0; SLOT_COUNT], purchased: [0; SLOT_COUNT], mods: [None; MOD_SLOTS] }
}

/// Final numbers for one weapon after attachments, ammunition and mods.
#[derive(Clone, Copy, Debug)]
pub struct Resolved {
    pub damage: f32,
    pub pellets: u8,
    pub mag: u16,
    pub interval: f32,
    pub reload: f32,
    pub recoil: f32,
    /// Hip-fire spread.
    pub spread: f32,
    /// Spread multiplier while aiming.
    pub ads_spread: f32,
    pub range: f32,
    pub splash: f32,
    pub ads_fov: f32,
    /// Grip to muzzle, in metres.
    pub length: f32,
    /// Height of the sight line above the grip, for lining up aim-down-sights.
    pub sight: f32,
    pub handling: f32,
    pub mobility: f32,
    pub quiet: bool,
    pub scope: u8,
    pub pierce: bool,
    pub burn: f32,
    pub tracer: Paint,
    /// Fraction of damage chained to a second target.
    pub arc: f32,
    /// Seconds of slow applied per hit.
    pub frost: f32,
    /// Damage per second of poison applied per hit.
    pub venom: f32,
    /// Bonus credits per hit.
    pub scavenge: f32,
    /// Objective repair per hit.
    pub repair: f32,
}

pub fn resolve(def: &WeaponDef, state: &WeaponState, table: &AttachmentTable) -> Resolved {
    let mut m = StatMods::NONE;
    if def.frame.is_some() {
        for (slot, variants) in table.iter().enumerate().take(slot::AMMO) {
            let Some(part) = variants.get(state.equipped[slot] as usize) else { continue };
            m.damage *= part.mods.damage;
            m.capacity *= part.mods.capacity;
            m.reload *= part.mods.reload;
            m.recoil *= part.mods.recoil;
            m.spread *= part.mods.spread;
            m.hipfire *= part.mods.hipfire;
            m.range *= part.mods.range;
            m.handling *= part.mods.handling;
            m.mobility *= part.mods.mobility;
            m.quiet |= part.mods.quiet;
            m.scope = m.scope.max(part.mods.scope);
            if part.mods.fov > 0.0 {
                m.fov = part.mods.fov;
            }
        }
    }
    let ammo = ammo_for(def.caliber).get(state.equipped[slot::AMMO] as usize).copied().unwrap_or(STANDARD);
    let level = |kind| state.mod_level(kind) as f32;
    Resolved {
        damage: def.damage * m.damage * ammo.damage * (1.0 + 0.2 * level(ModKind::Heavy)),
        pellets: ammo.pellets.unwrap_or(def.pellets).max(1),
        mag: ((def.mag as f32 * m.capacity).ceil() as u16).max(if def.mag == 0 { 0 } else { 1 }),
        interval: def.interval / (1.0 + 0.15 * level(ModKind::Capacitor)),
        reload: def.reload * m.reload,
        recoil: def.recoil * m.recoil,
        spread: def.spread * m.spread * m.hipfire * ammo.spread,
        ads_spread: def.ads_spread / m.hipfire.max(0.1),
        range: def.range * m.range * ammo.range,
        // Explosive weapons scale their blast; bullets gain a small one.
        splash: if def.splash > 0.0 { def.splash * if ammo.splash > 0.0 { ammo.splash } else { 1.0 } } else { ammo.splash } + 1.3 * level(ModKind::Split),
        ads_fov: if m.fov > 0.0 { m.fov } else { def.ads_fov },
        length: muzzle_distance(def, &state.equipped),
        sight: sight_height(def, &state.equipped),
        handling: m.handling,
        mobility: m.mobility,
        quiet: m.quiet,
        scope: m.scope,
        pierce: ammo.pierce,
        burn: ammo.burn,
        tracer: ammo.tracer,
        arc: 0.3 * level(ModKind::Arc),
        frost: 0.9 * level(ModKind::Frost),
        venom: 5.0 * level(ModKind::Venom),
        scavenge: 1.5 * level(ModKind::Scavenger),
        repair: 0.8 * level(ModKind::Repair),
    }
}

/// The shared attachment catalogue. Works for every framed gun.
pub fn standard_attachments() -> AttachmentTable {
    use GunClass::*;
    const B: StatMods = StatMods::NONE;
    let part = |name, blurb, cost, mods, classes| Attachment { name, blurb, cost, mods, classes };
    vec![
        vec![
            part("IRON SIGHTS", "Clean and quick", 0, B, GUNS),
            part("REFLEX", "Red dot. Faster on target", 85, StatMods { fov: 60.0, handling: 1.06, spread: 0.94, ..B }, except(&[Launcher])),
            part("HOLOGRAPHIC", "Wide window, slight zoom", 110, StatMods { fov: 54.0, spread: 0.9, ..B }, GUNS),
            part("ACOG 3X", "Magnified mid-range sight", 160, StatMods { fov: 34.0, spread: 0.82, handling: 0.9, scope: 1, ..B }, except(&[Pistol, Launcher])),
            part("SNIPER SCOPE 8X", "Long glass for long shots", 230, StatMods { fov: 13.0, spread: 0.6, handling: 0.78, scope: 2, ..B }, only(&[Rifle, Sniper, Lmg])),
        ],
        vec![
            part("NONE", "Bare muzzle", 0, B, GUNS),
            part("COMPENSATOR", "Tames vertical kick", 90, StatMods { recoil: 0.74, ..B }, except(&[Launcher])),
            part("SUPPRESSOR", "Off the radar, quieter, less range", 140, StatMods { range: 0.88, quiet: true, ..B }, except(&[Launcher])),
            part("MUZZLE BRAKE", "Less recoil, tighter groups", 110, StatMods { recoil: 0.84, spread: 0.92, ..B }, except(&[Launcher])),
            part("FLASH HIDER", "Steadier follow-up shots", 70, StatMods { spread: 0.9, recoil: 0.94, ..B }, except(&[Launcher])),
        ],
        vec![
            part("STANDARD BARREL", "Factory length", 0, B, GUNS),
            part("LONG BARREL", "Range and punch, slower to aim", 160, StatMods { damage: 1.1, range: 1.35, spread: 0.88, handling: 0.88, mobility: 0.96, ..B }, GUNS),
            part("SHORT BARREL", "Snappy handling, less range", 90, StatMods { range: 0.78, damage: 0.95, handling: 1.15, mobility: 1.04, ..B }, GUNS),
            part("HEAVY BARREL", "Stable and accurate, heavy", 150, StatMods { recoil: 0.8, range: 1.15, spread: 0.9, handling: 0.86, mobility: 0.95, ..B }, except(&[Pistol])),
        ],
        vec![
            part("NONE", "Nothing under the barrel", 0, B, GUNS),
            part("VERTICAL GRIP", "Strong recoil control", 80, StatMods { recoil: 0.76, handling: 0.96, ..B }, except(&[Pistol, Launcher])),
            part("ANGLED GRIP", "Faster aim, some control", 95, StatMods { recoil: 0.9, handling: 1.12, ..B }, except(&[Pistol, Launcher])),
            part("BIPOD", "Steady from cover and prone", 120, StatMods { recoil: 0.82, spread: 0.82, handling: 0.9, ..B }, only(&[Rifle, Sniper, Lmg])),
            part("TACTICAL LASER", "Tight hip fire", 100, StatMods { hipfire: 0.65, ..B }, except(&[Launcher])),
        ],
        vec![
            part("STANDARD MAG", "Factory capacity", 0, B, GUNS),
            part("EXTENDED MAG", "+50% rounds, slower reload", 95, StatMods { capacity: 1.5, reload: 1.15, mobility: 0.98, ..B }, GUNS),
            part("DRUM MAG", "Double capacity, heavy", 180, StatMods { capacity: 2.0, reload: 1.35, recoil: 1.06, handling: 0.88, mobility: 0.95, ..B }, only(&[Pistol, Smg, Rifle, Lmg])),
            part("FAST MAG", "Quick reloads, fewer rounds", 130, StatMods { capacity: 0.9, reload: 0.62, ..B }, GUNS),
        ],
        vec![
            part("STANDARD STOCK", "Balanced", 0, B, GUNS),
            part("NO STOCK", "Light and fast, kicks hard", 60, StatMods { recoil: 1.25, handling: 1.2, mobility: 1.06, ..B }, except(&[Pistol, Launcher])),
            part("PADDED STOCK", "Soaks up recoil", 80, StatMods { recoil: 0.85, ..B }, except(&[Pistol])),
            part("TACTICAL STOCK", "Faster aim, light frame", 130, StatMods { handling: 1.1, recoil: 0.95, mobility: 1.02, ..B }, except(&[Pistol])),
        ],
    ]
}

const BACK: f32 = 0.08;

fn barrel_length(frame: &GunFrame, equipped: &[u8; SLOT_COUNT]) -> f32 {
    frame.barrel * [1.0, 1.6, 0.6, 1.1][(equipped[slot::BARREL] as usize).min(3)]
}

/// Grip to muzzle, including any muzzle device.
pub fn muzzle_distance(def: &WeaponDef, equipped: &[u8; SLOT_COUNT]) -> f32 {
    let Some(frame) = def.frame else { return def.length };
    let device = [0.0, 0.08, 0.26, 0.07, 0.1][(equipped[slot::MUZZLE] as usize).min(4)];
    frame.receiver - BACK + barrel_length(&frame, equipped) + device
}

/// Height of the sight line above the grip.
pub fn sight_height(def: &WeaponDef, equipped: &[u8; SLOT_COUNT]) -> f32 {
    let Some(frame) = def.frame else { return 0.19 };
    let base = frame.height * 0.5 + 0.016;
    match equipped[slot::OPTIC] {
        0 => frame.height * 0.5 + 0.055,
        1 => base + 0.045,
        2 => base + 0.037,
        3 => base + 0.05,
        _ => base + 0.06,
    }
}

/// Builds the gun as a block model, fitted with its parts. Magazines are
/// tagged [`Anim::Mag`] and moving actions [`Anim::Slide`] so viewmodels can
/// animate them separately.
pub fn assemble(def: &WeaponDef, equipped: &[u8; SLOT_COUNT]) -> BlockModel {
    let Some(f) = def.frame else { return def.model.clone() };
    let v = |s: usize| equipped[s] as usize;
    let (w, h, r) = (f.width, f.height, f.receiver);
    let (front, mid, top) = (BACK - r, BACK - r * 0.5, h * 0.5);
    let bore_y = h * 0.12;
    let barrel = barrel_length(&f, equipped);
    let tip = front - barrel;
    let bore = f.bore * if v(slot::BARREL) == 3 { 1.35 } else { 1.0 };

    let mut m = BlockModel::new()
        .cube(f.body, [0.0, 0.0, mid], [w, h, r])
        .cube(Paint::Dark, [0.0, top + 0.008, mid + r * 0.05], [w * 0.55, 0.016, r * 0.8])
        .cube(f.accent, [w * 0.5 + 0.004, h * 0.1, mid], [0.01, h * 0.26, r * 0.62])
        .cube(f.accent, [-w * 0.5 - 0.004, h * 0.1, mid], [0.01, h * 0.26, r * 0.62])
        .cube(Paint::Dark, [w * 0.5 + 0.003, h * 0.2, mid + r * 0.18], [0.008, h * 0.22, r * 0.22])
        .cube(f.furniture, [0.0, -top - 0.085, 0.02], [w * 0.75, 0.19, 0.1])
        .rotated([-14.0, 0.0, 0.0])
        .cube(Paint::Dark, [0.0, -top - 0.02, -0.07], [w * 0.3, 0.022, 0.12])
        .cylinder(Paint::Metal, [0.0, bore_y, front - barrel * 0.5], [bore, barrel, bore])
        .rotated([90.0, 0.0, 0.0]);
    if v(slot::BARREL) == 3 {
        for i in 0..3 {
            m = m.cylinder(Paint::Dark, [0.0, bore_y, front - barrel * (0.25 + 0.25 * i as f32)], [bore * 1.3, 0.02, bore * 1.3]).rotated([90.0, 0.0, 0.0]);
        }
    }
    let guard = (f.barrel * 0.7).min(barrel - 0.02).max(0.05);
    if f.handguard && def.action != Action::Pump {
        m = m
            .cube(f.furniture, [0.0, bore_y - 0.005, front - guard * 0.5], [w * 0.92, h * 0.72, guard])
            .cube(Paint::Dark, [w * 0.465, bore_y, front - guard * 0.5], [0.006, h * 0.18, guard * 0.7])
            .cube(Paint::Dark, [-w * 0.465, bore_y, front - guard * 0.5], [0.006, h * 0.18, guard * 0.7]);
    }

    // The moving action.
    m = match (def.action, def.class) {
        (Action::Pump, _) => {
            let len = (barrel * 0.35).max(0.1);
            m.cube(f.furniture, [0.0, bore_y - bore * 0.95, front - barrel * 0.3], [w * 0.88, h * 0.55, len])
                .anim(Anim::Slide)
                .cube(Paint::Dark, [0.0, bore_y - bore * 0.95 - h * 0.28, front - barrel * 0.3], [w * 0.7, 0.012, len * 0.9])
                .anim(Anim::Slide)
        }
        (Action::Bolt, _) => m
            .cylinder(Paint::Metal, [w * 0.5 + 0.035, h * 0.2, BACK - r * 0.25], [0.018, 0.07, 0.018])
            .rotated([0.0, 0.0, 90.0])
            .anim(Anim::Slide)
            .shard(Paint::Dark, [w * 0.5 + 0.075, h * 0.2, BACK - r * 0.25], [0.028, 0.028, 0.028])
            .anim(Anim::Slide),
        (_, GunClass::Pistol) => m
            .cube(Paint::Steel, [0.0, top - h * 0.16, mid - 0.01], [w * 1.05, h * 0.4, r * 1.02])
            .anim(Anim::Slide)
            .cube(Paint::Dark, [w * 0.53, top - h * 0.14, mid + r * 0.1], [0.006, h * 0.18, r * 0.25])
            .anim(Anim::Slide),
        _ => m.cube(Paint::Dark, [w * 0.5 + 0.015, h * 0.22, BACK - r * 0.32], [0.03, 0.025, 0.05]).anim(Anim::Slide),
    };

    // Optic.
    let (z0, yb) = (BACK - r * 0.45, top + 0.016);
    m = match v(slot::OPTIC) {
        0 => {
            let zf = front - barrel * 0.82;
            let post = top + 0.055 - bore_y;
            m.cube(Paint::Dark, [0.022, top + 0.03, BACK - 0.04], [0.016, 0.05, 0.025])
                .cube(Paint::Dark, [-0.022, top + 0.03, BACK - 0.04], [0.016, 0.05, 0.025])
                .cube(Paint::Dark, [0.0, bore_y + post * 0.5, zf], [0.016, post, 0.02])
                .cube(Paint::Glow, [0.0, top + 0.05, zf], [0.014, 0.01, 0.016])
        }
        1 => m
            .cube(Paint::Dark, [0.0, yb + 0.01, z0], [0.05, 0.02, 0.07])
            .cube(Paint::Dark, [0.026, yb + 0.045, z0 - 0.02], [0.008, 0.05, 0.012])
            .cube(Paint::Dark, [-0.026, yb + 0.045, z0 - 0.02], [0.008, 0.05, 0.012])
            .cube(Paint::Dark, [0.0, yb + 0.072, z0 - 0.02], [0.06, 0.008, 0.012])
            .cube(Paint::Red, [0.0, yb + 0.045, z0 - 0.024], [0.006, 0.006, 0.003]),
        2 => m
            .cube(Paint::Dark, [0.0, yb + 0.006, z0], [0.075, 0.012, 0.09])
            .cube(Paint::Dark, [0.034, yb + 0.037, z0], [0.008, 0.062, 0.09])
            .cube(Paint::Dark, [-0.034, yb + 0.037, z0], [0.008, 0.062, 0.09])
            .cube(Paint::Dark, [0.0, yb + 0.07, z0], [0.076, 0.008, 0.09])
            .cube(Paint::Red, [0.0, yb + 0.037, z0 - 0.04], [0.005, 0.005, 0.003]),
        3 => m
            .cube(Paint::Dark, [0.0, yb + 0.008, z0], [0.04, 0.016, 0.08])
            .cylinder(Paint::Dark, [0.0, yb + 0.05, z0], [0.06, 0.16, 0.06])
            .rotated([90.0, 0.0, 0.0])
            .cylinder(Paint::Steel, [0.0, yb + 0.05, z0 - 0.085], [0.072, 0.03, 0.072])
            .rotated([90.0, 0.0, 0.0])
            .cylinder(Paint::Blue, [0.0, yb + 0.05, z0 - 0.101], [0.056, 0.004, 0.056])
            .rotated([90.0, 0.0, 0.0]),
        _ => m
            .cube(Paint::Metal, [0.0, yb + 0.02, z0 + 0.06], [0.03, 0.04, 0.02])
            .cube(Paint::Metal, [0.0, yb + 0.02, z0 - 0.1], [0.03, 0.04, 0.02])
            .cylinder(Paint::Dark, [0.0, yb + 0.06, z0 - 0.03], [0.068, 0.34, 0.068])
            .rotated([90.0, 0.0, 0.0])
            .cylinder(Paint::Dark, [0.0, yb + 0.06, z0 - 0.21], [0.095, 0.07, 0.095])
            .rotated([90.0, 0.0, 0.0])
            .cylinder(Paint::Dark, [0.0, yb + 0.06, z0 + 0.15], [0.08, 0.05, 0.08])
            .rotated([90.0, 0.0, 0.0])
            .cylinder(Paint::Metal, [0.0, yb + 0.1, z0 - 0.03], [0.03, 0.04, 0.03])
            .cylinder(Paint::Metal, [0.04, yb + 0.06, z0 - 0.03], [0.03, 0.03, 0.03])
            .rotated([0.0, 0.0, 90.0])
            .cylinder(Paint::Blue, [0.0, yb + 0.06, z0 - 0.247], [0.08, 0.004, 0.08])
            .rotated([90.0, 0.0, 0.0]),
    };

    // Muzzle device.
    m = match v(slot::MUZZLE) {
        1 => m
            .cube(Paint::Dark, [0.0, bore_y, tip - 0.04], [bore * 1.7, bore * 1.7, 0.08])
            .cube(Paint::Metal, [0.0, bore_y + bore * 0.86, tip - 0.04], [bore * 0.9, 0.006, 0.05]),
        2 => m
            .cylinder(Paint::Dark, [0.0, bore_y, tip - 0.13], [bore * 2.1, 0.26, bore * 2.1])
            .rotated([90.0, 0.0, 0.0])
            .cylinder(Paint::Metal, [0.0, bore_y, tip - 0.02], [bore * 2.2, 0.015, bore * 2.2])
            .rotated([90.0, 0.0, 0.0]),
        3 => m.cube(Paint::Metal, [0.0, bore_y, tip - 0.035], [bore * 2.6, bore * 1.4, 0.07]).cube(Paint::Dark, [0.0, bore_y, tip - 0.035], [bore * 2.7, bore * 0.4, 0.02]),
        4 => m.cone(Paint::Dark, [0.0, bore_y, tip - 0.05], [bore * 1.9, 0.1, bore * 1.9]).rotated([90.0, 0.0, 0.0]),
        _ => m,
    };

    // Underbarrel.
    let (zu, yu) = (front - (guard * 0.55).max(0.06), bore_y - h * 0.36);
    m = match v(slot::UNDER) {
        1 => m.cube(Paint::Dark, [0.0, yu - 0.07, zu], [0.045, 0.14, 0.05]).cube(Paint::Rust, [0.0, yu - 0.145, zu], [0.05, 0.012, 0.055]),
        2 => m.cube(Paint::Dark, [0.0, yu - 0.03, zu + 0.02], [0.04, 0.045, 0.11]).rotated([-28.0, 0.0, 0.0]),
        3 => m
            .cube(Paint::Dark, [0.0, yu - 0.012, zu + 0.04], [0.05, 0.024, 0.04])
            .cube(Paint::Metal, [0.025, yu - 0.016, zu - 0.06], [0.014, 0.014, 0.2])
            .cube(Paint::Metal, [-0.025, yu - 0.016, zu - 0.06], [0.014, 0.014, 0.2]),
        4 => m.cube(Paint::Dark, [0.0, yu - 0.025, zu], [0.04, 0.045, 0.1]).cube(Paint::Red, [0.0, yu - 0.025, zu - 0.051], [0.02, 0.02, 0.004]),
        _ => m,
    };

    // Magazine.
    let zm = BACK - r * 0.62;
    let size = match v(slot::MAGAZINE) {
        1 => 1.5,
        3 => 0.9,
        _ => 1.0,
    };
    let plate = if v(slot::MAGAZINE) == 3 { Paint::Gold } else { Paint::Dark };
    let shape = if v(slot::MAGAZINE) == 2 && matches!(f.mag, MagShape::Box | MagShape::Curved) { MagShape::Drum } else { f.mag };
    m = match shape {
        MagShape::Box | MagShape::Curved => {
            let len = 0.15 * size;
            let tilt = if shape == MagShape::Curved { 14.0 } else { 0.0 };
            let shift = if shape == MagShape::Curved { -len * 0.18 } else { 0.0 };
            m.cube(Paint::Metal, [0.0, -top - len * 0.5 + 0.01, zm + shift], [w * 0.55, len, 0.085])
                .rotated([tilt, 0.0, 0.0])
                .anim(Anim::Mag)
                .cube(plate, [0.0, -top - len + 0.005, zm + shift * 2.0], [w * 0.6, 0.018, 0.095])
                .rotated([tilt, 0.0, 0.0])
                .anim(Anim::Mag)
        }
        MagShape::Drum => m
            .cube(Paint::Metal, [0.0, -top - 0.03, zm], [w * 0.5, 0.06, 0.08])
            .anim(Anim::Mag)
            .cylinder(Paint::Metal, [0.0, -top - 0.12, zm], [0.2, 0.09, 0.2])
            .rotated([0.0, 0.0, 90.0])
            .anim(Anim::Mag)
            .cylinder(plate, [0.0, -top - 0.12, zm], [0.08, 0.095, 0.08])
            .rotated([0.0, 0.0, 90.0])
            .anim(Anim::Mag),
        MagShape::Tube => {
            let len = (barrel * 0.85 * size).min(barrel);
            m.cylinder(Paint::Metal, [0.0, bore_y - bore * 1.25, front - len * 0.5], [bore * 0.9, len, bore * 0.9]).rotated([90.0, 0.0, 0.0])
        }
        MagShape::Belt => m
            .cube(Paint::Steel, [-w * 0.5 - 0.06, -0.03, zm], [0.12, 0.14 * size, 0.17])
            .anim(Anim::Mag)
            .cube(Paint::Gold, [-w * 0.5 - 0.01, 0.02, zm], [0.04, 0.02, 0.05])
            .anim(Anim::Mag),
        MagShape::Cell => m
            .cube(Paint::Dark, [0.0, -top - 0.05 * size, zm], [w * 0.6, 0.1 * size, 0.09])
            .anim(Anim::Mag)
            .cube(Paint::Aether, [w * 0.31, -top - 0.05 * size, zm], [0.006, 0.07 * size, 0.06])
            .anim(Anim::Mag),
        MagShape::Canister => m.cylinder(Paint::Teal, [0.0, -top - 0.06 * size, zm], [0.16, 0.12 * size, 0.16]).anim(Anim::Mag),
    };

    // Stock.
    let stock = match v(slot::STOCK) {
        1 => StockShape::None,
        3 => StockShape::Skeleton,
        _ => f.stock,
    };
    m = match stock {
        StockShape::Fixed => m.cube(f.furniture, [0.0, -0.03, BACK + 0.14], [w * 0.8, h * 1.05, 0.28]).cube(Paint::Dark, [0.0, -0.03, BACK + 0.285], [w * 0.85, h * 1.1, 0.012]),
        StockShape::Skeleton => m
            .cube(Paint::Dark, [0.0, top * 0.4, BACK + 0.14], [0.03, 0.03, 0.28])
            .segment(Paint::Dark, [0.0, -top * 0.6, BACK], [0.0, -h * 0.9, BACK + 0.27], 0.028)
            .cube(Paint::Dark, [0.0, -h * 0.25, BACK + 0.28], [w * 0.7, h * 1.3, 0.025]),
        StockShape::Heavy => m.cube(f.furniture, [0.0, -0.02, BACK + 0.16], [w * 0.95, h * 1.25, 0.32]).cube(Paint::Dark, [0.0, top * 0.75, BACK + 0.12], [w * 0.6, 0.03, 0.16]),
        StockShape::None => m,
    };
    if v(slot::STOCK) == 2 && stock != StockShape::None {
        m = m.cube(Paint::Rust, [0.0, -0.03, BACK + 0.31], [w * 0.88, h * 1.15, 0.035]);
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rifle() -> WeaponDef {
        WeaponDef {
            name: "TEST", blurb: "", price: Cost::FREE, class: GunClass::Rifle, caliber: Caliber::Rifle, action: Action::Auto,
            reload_style: ReloadStyle::Magazine, mag: 20, reserve: 60, infinite_reserve: false, ammo_price: 10, damage: 10.0, interval: 0.1,
            reload: 1.0, pellets: 1, spread: 0.01, ads_spread: 0.35, range: 50.0, splash: 0.0, recoil: 1.0, mode: FireMode::Hitscan,
            ads_fov: 55.0, mining: 0.0, sound: 0,
            frame: Some(GunFrame {
                body: Paint::Steel, accent: Paint::Teal, furniture: Paint::Dark, width: 0.08, height: 0.13, receiver: 0.34, barrel: 0.3, bore: 0.04,
                mag: MagShape::Curved, stock: StockShape::Fixed, handguard: true,
            }),
            defaults: [0; SLOT_COUNT], model: BlockModel::new(), length: 0.6,
        }
    }

    #[test]
    fn attachments_ammo_and_mods_stack() {
        let table = standard_attachments();
        let def = rifle();
        let mut state = WeaponState::fresh(&def);
        let base = resolve(&def, &state, &table);
        assert_eq!((base.mag, base.damage, base.ads_fov, base.scope), (20, 10.0, 55.0, 0));
        // Sniper scope, long barrel, drum, armor-piercing.
        state.equipped = [4, 0, 1, 0, 2, 0, 2];
        assert!(state.add_mod(ModKind::Heavy) && state.add_mod(ModKind::Heavy));
        let tuned = resolve(&def, &state, &table);
        assert_eq!((tuned.mag, tuned.ads_fov, tuned.scope), (40, 13.0, 2));
        assert!((tuned.damage - 10.0 * 1.1 * 0.95 * 1.4).abs() < 1e-4);
        assert!(tuned.pierce && tuned.range > base.range && tuned.length > base.length && tuned.sight > base.sight);
    }

    #[test]
    fn parts_respect_classes_and_change_geometry() {
        let table = standard_attachments();
        let mut def = rifle();
        let all = options(&def, &table, slot::OPTIC).len();
        def.class = GunClass::Pistol;
        assert!(options(&def, &table, slot::OPTIC).len() < all, "pistols cannot mount sniper scopes");
        assert_eq!(part_cost(&def, &table, slot::OPTIC, 4), None);
        assert_eq!(part_cost(&def, &table, slot::OPTIC, 1), Some(85));
        def.class = GunClass::Rifle;
        let plain = assemble(&def, &[0; SLOT_COUNT]);
        let kitted = assemble(&def, &[4, 2, 1, 1, 2, 2, 0]);
        assert!(kitted.parts.len() > plain.parts.len());
        assert!(plain.parts.iter().any(|p| p.anim == Anim::Mag) && plain.parts.iter().any(|p| p.anim == Anim::Slide));
        assert_eq!(options(&def, &table, slot::AMMO).len(), ammo_for(Caliber::Rifle).len());
    }

    #[test]
    fn mod_slots_are_limited() {
        let mut state = WeaponState::default();
        for kind in [ModKind::Arc, ModKind::Frost, ModKind::Venom] {
            assert!(state.add_mod(kind));
        }
        assert!(!state.add_mod(ModKind::Heavy), "fourth distinct mod must not fit");
        assert!(state.add_mod(ModKind::Arc) && state.add_mod(ModKind::Arc));
        assert!(!state.add_mod(ModKind::Arc), "level cap");
        assert_eq!(state.mod_level(ModKind::Arc), MOD_MAX_LEVEL);
    }
}
