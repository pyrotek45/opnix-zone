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
use glam::Vec3;
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

    /// How hard this kind of gun kicks, on top of the gun's own recoil figure: pistols and
    /// SMGs are light (SMGs lightest per round, they fire so many), rifles moderate, LMGs,
    /// shotguns and sniper rifles strong.
    pub fn kick(self) -> f32 {
        match self {
            GunClass::Pistol => 0.75,
            GunClass::Smg => 0.85,
            GunClass::Rifle => 1.25,
            GunClass::Lmg => 1.8,
            GunClass::Shotgun => 1.6,
            GunClass::Sniper => 1.7,
            GunClass::Launcher => 1.3,
            GunClass::Tool => 0.0,
        }
    }
}

/// Aim climb per unit of [`shot_kick`], in radians. The camera's visible kick is drawn from
/// the same units, so what you see the gun do is what it does to your aim.
pub const RECOIL_RADIANS: f32 = 0.0062;
/// Aiming down the sights braces the gun against the shoulder.
pub const ADS_RECOIL: f32 = 0.78;
/// The hardest single kick any gun can give (in units), so even the heaviest stays usable.
pub const MAX_KICK: f32 = 4.5;

/// One shot's kick, in recoil units: the gun's recoil after attachments (grips, stocks,
/// compensators...), times its class, times `brace` (the stance: crouched and prone kick
/// less), times [`ADS_RECOIL`] when aiming. Multiply by [`RECOIL_RADIANS`] for the aim climb.
pub fn shot_kick(class: GunClass, recoil: f32, brace: f32, aiming: bool) -> f32 {
    (recoil * class.kick() * brace * if aiming { ADS_RECOIL } else { 1.0 }).clamp(0.0, MAX_KICK)
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
    /// Banana magazine that curves towards the muzzle.
    Curved,
    Drum,
    /// Shotgun tube under the barrel.
    Tube,
    /// Ammunition box under the receiver with the belt running into its side.
    Belt,
    /// Energy cell.
    Cell,
    /// Big launcher canister.
    Canister,
    /// Lies flat along the top of the receiver, feeding down (personal defence weapons).
    Top,
    /// A revolving cylinder around the bore, in an open frame.
    Cylinder,
    /// Fixed internal magazine: only a floorplate shows.
    Internal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StockShape {
    None,
    Fixed,
    Skeleton,
    Heavy,
    /// Triangular polymer stock on a hinge.
    Folding,
    /// A sweeping stock with a hole for the thumb behind the grip.
    Thumbhole,
    /// A classic wooden stock that drops away from the bore line.
    Sloped,
    /// Adjustable stock on a buffer tube.
    Tube,
    /// A solid chassis stock: flat comb, cheek piece and a tall, square butt.
    Precision,
}

/// Where the action sits relative to the trigger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GunLayout {
    Conventional,
    /// Action and magazine behind the grip, in the butt: a full barrel in a short gun.
    Bullpup,
}

/// The cross-section and outline of the receiver (and the handguard that follows it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiverShape {
    Box,
    /// Stepped upper with sloped nose and tail, and a wedge handguard.
    Angular,
    /// A tube, like old stamped and machined guns.
    Round,
    /// Tall, with a deep lower and a raked front.
    Slab,
    /// Smooth hump and swept nose.
    Sleek,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarrelShape {
    Plain,
    /// Vented cooling jacket.
    Shroud,
    /// Thick barrel with grooves.
    Fluted,
    /// Six barrels round a spindle.
    Rotary,
    /// Twin rails with glowing coils.
    Rails,
    /// Long and thin, ending in a ported brake.
    Brake,
    /// Revolver barrel with a full underlug and top rib.
    Lug,
    /// Two barrels side by side under a flat rib (double-barrel shotguns).
    Twin,
    /// Vented rib along the top and a bead at the muzzle.
    Rib,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GripShape {
    Pistol,
    /// Steeply raked with a rounded butt.
    Raked,
    /// No pistol grip: the stock's wrist runs down from the receiver.
    Wrist,
    /// A wrist with a lever loop under it (moves with the action).
    Lever,
    /// A grip joined to the stock below, leaving a hole for the thumb.
    Thumbhole,
}

/// Built-in furniture under the barrel; hidden when an underbarrel part is fitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Foregrip {
    None,
    Vertical,
    Angled,
    /// A small hand stop at the front of the handguard.
    Stop,
}

/// The shape of a gun. Attachments read these numbers to find their mounts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GunFrame {
    pub body: Paint,
    pub accent: Paint,
    pub furniture: Paint,
    /// Receiver width, height and length in metres (the part ahead of the grip on a bullpup).
    pub width: f32,
    pub height: f32,
    pub receiver: f32,
    /// Barrel length beyond the receiver, and its diameter.
    pub barrel: f32,
    pub bore: f32,
    pub mag: MagShape,
    /// Magazine length multiplier (stick magazines, flush ones).
    pub mag_len: f32,
    pub stock: StockShape,
    pub handguard: bool,
    pub layout: GunLayout,
    pub shape: ReceiverShape,
    pub barrel_shape: BarrelShape,
    pub grip: GripShape,
    pub fore: Foregrip,
    /// A carry handle over the receiver; sights and optics sit on top of it.
    pub carry: bool,
    /// A folded-down bipod near the muzzle.
    pub bipod: bool,
    /// Accessory rails along the handguard.
    pub rails: bool,
}

impl GunFrame {
    /// A plain conventional rifle frame to build others from with struct-update syntax.
    pub const RIFLE: GunFrame = GunFrame {
        body: Paint::Steel, accent: Paint::Teal, furniture: Paint::Dark, width: 0.08, height: 0.13, receiver: 0.34, barrel: 0.3, bore: 0.04,
        mag: MagShape::Box, mag_len: 1.0, stock: StockShape::Fixed, handguard: true, layout: GunLayout::Conventional, shape: ReceiverShape::Box,
        barrel_shape: BarrelShape::Plain, grip: GripShape::Pistol, fore: Foregrip::None, carry: false, bipod: false, rails: false,
    };
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
    /// Bare weight in kilograms; attachments add to it (see [`carried_weight`]).
    pub weight: f32,
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
    /// Kilograms the part adds to the gun (negative when it replaces something heavier).
    pub weight: f32,
    /// Hides the shooter from enemy radar and muffles the report.
    pub quiet: bool,
    /// 1 = magnified sight overlay, 2 = full sniper scope, 3 = thermal scope (enemies glow).
    pub scope: u8,
}

impl StatMods {
    pub const NONE: StatMods = StatMods {
        damage: 1.0, capacity: 1.0, reload: 1.0, recoil: 1.0, spread: 1.0, hipfire: 1.0, range: 1.0, fov: 0.0, handling: 1.0, weight: 0.0,
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
    pub purchased: [u16; SLOT_COUNT],
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
        variant == 0 || self.purchased.get(slot).is_some_and(|mask| mask & (1 << variant.min(15)) != 0)
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
    /// Kilograms, with the fitted parts.
    pub weight: f32,
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
            m.weight += part.mods.weight;
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
        weight: (def.weight + m.weight).max(0.1),
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

/// Kilograms a soldier carries at the normal pace: a rifle, a sidearm and a couple of grenades.
pub const LOAD_REFERENCE: f32 = 5.5;
/// Pace lost (or gained) per kilogram away from [`LOAD_REFERENCE`].
pub const LOAD_PER_KG: f32 = 0.025;
/// The pace limits: a pistol and knives at the top, a rotary cannon at the bottom.
pub const LOAD_FASTEST: f32 = 1.05;
pub const LOAD_SLOWEST: f32 = 0.78;

/// Movement speed multiplier for carrying `kg` (walk, sprint and aim alike).
pub fn load_speed(kg: f32) -> f32 {
    (1.0 - (kg - LOAD_REFERENCE) * LOAD_PER_KG).clamp(LOAD_SLOWEST, LOAD_FASTEST)
}

/// Kilograms of one gun with these parts fitted.
pub fn carried_weight(def: &WeaponDef, equipped: &[u8; SLOT_COUNT], table: &AttachmentTable) -> f32 {
    let mut kg = def.weight;
    if def.frame.is_some() {
        for (slot, variants) in table.iter().enumerate().take(slot::AMMO) {
            kg += variants.get(equipped[slot] as usize).map_or(0.0, |part| part.mods.weight);
        }
    }
    kg.max(0.1)
}

/// The shared attachment catalogue. Works for every framed gun. New parts go at the
/// end of their slot so saved classes keep their picks.
pub fn standard_attachments() -> AttachmentTable {
    use GunClass::*;
    const B: StatMods = StatMods::NONE;
    let part = |name, blurb, cost, mods, classes| Attachment { name, blurb, cost, mods, classes };
    // Long glass belongs on rifles (marksman rifles included) and sniper rifles.
    let long = only(&[Rifle, Sniper]);
    vec![
        vec![
            part("IRON SIGHTS", "Clean and quick", 0, B, GUNS),
            part("REFLEX", "Red dot. Faster on target", 85, StatMods { fov: 60.0, handling: 1.06, spread: 0.94, weight: 0.15, ..B }, except(&[Launcher])),
            part("HOLOGRAPHIC", "Wide window, slight zoom", 110, StatMods { fov: 54.0, spread: 0.9, weight: 0.3, ..B }, GUNS),
            part("ACOG 3X", "Magnified mid-range sight", 160, StatMods { fov: 34.0, spread: 0.82, handling: 0.9, scope: 1, weight: 0.45, ..B }, except(&[Pistol, Launcher])),
            part("SNIPER SCOPE 8X", "Long glass for long shots", 230, StatMods { fov: 13.0, spread: 0.6, handling: 0.78, scope: 2, weight: 0.8, ..B }, long),
            part("MARKSMAN 6X", "Mid-long glass, quicker to settle than the 8x", 190, StatMods { fov: 19.0, spread: 0.68, handling: 0.84, scope: 2, weight: 0.6, ..B }, only(&[Rifle, Sniper, Lmg])),
            part("VARIABLE 12X", "Huge zoom for the longest lanes; slow and heavy", 280, StatMods { fov: 8.0, spread: 0.52, handling: 0.7, scope: 2, weight: 1.15, ..B }, long),
            part("THERMAL 4X", "Enemies glow hot through smoke and shadow", 260, StatMods { fov: 28.0, spread: 0.84, handling: 0.84, scope: 3, weight: 0.95, ..B }, only(&[Rifle, Sniper, Lmg])),
            part("HYBRID 4X", "Magnified glass with a canted red dot for close work", 210, StatMods { fov: 30.0, spread: 0.84, hipfire: 0.9, handling: 0.88, scope: 1, weight: 0.65, ..B }, except(&[Pistol, Launcher])),
        ],
        vec![
            part("NONE", "Bare muzzle", 0, B, GUNS),
            part("COMPENSATOR", "Tames vertical kick", 90, StatMods { recoil: 0.74, weight: 0.12, ..B }, except(&[Launcher])),
            part("SUPPRESSOR", "Off the radar, quieter, less range", 140, StatMods { range: 0.88, quiet: true, weight: 0.35, ..B }, except(&[Launcher])),
            part("MUZZLE BRAKE", "Less recoil, tighter groups", 110, StatMods { recoil: 0.84, spread: 0.92, weight: 0.18, ..B }, except(&[Launcher])),
            part("FLASH HIDER", "Steadier follow-up shots", 70, StatMods { spread: 0.9, recoil: 0.94, weight: 0.08, ..B }, except(&[Launcher])),
            part("HEAVY SUPPRESSOR", "Silent with full range, but long and heavy", 200, StatMods { quiet: true, recoil: 0.88, handling: 0.88, weight: 0.7, ..B }, only(&[Rifle, Sniper, Lmg])),
            part("CHOKE", "Squeezes the pellet spread for reach", 80, StatMods { spread: 0.7, range: 1.22, weight: 0.1, ..B }, only(&[Shotgun])),
        ],
        vec![
            part("STANDARD BARREL", "Factory length", 0, B, GUNS),
            part("LONG BARREL", "Range and punch, slower to aim", 160, StatMods { damage: 1.1, range: 1.35, spread: 0.88, handling: 0.88, weight: 0.5, ..B }, GUNS),
            part("SHORT BARREL", "Snappy handling, less range", 90, StatMods { range: 0.78, damage: 0.95, handling: 1.15, weight: -0.3, ..B }, GUNS),
            part("HEAVY BARREL", "Stable and accurate, heavy", 150, StatMods { recoil: 0.8, range: 1.15, spread: 0.9, handling: 0.86, weight: 0.9, ..B }, except(&[Pistol])),
            part("MATCH BARREL", "Precision bore: tight groups, a little more kick", 190, StatMods { spread: 0.75, range: 1.2, recoil: 1.05, handling: 0.95, weight: 0.3, ..B }, only(&[Pistol, Rifle, Sniper])),
            part("SAWN-OFF", "Brutal up close, wild spread", 60, StatMods { spread: 1.3, range: 0.7, handling: 1.25, weight: -0.6, ..B }, only(&[Shotgun])),
        ],
        vec![
            part("NONE", "Nothing under the barrel", 0, B, GUNS),
            part("VERTICAL GRIP", "Strong recoil control", 80, StatMods { recoil: 0.76, handling: 0.96, weight: 0.15, ..B }, except(&[Pistol, Launcher])),
            part("ANGLED GRIP", "Faster aim, some control", 95, StatMods { recoil: 0.9, handling: 1.12, weight: 0.12, ..B }, except(&[Pistol, Launcher])),
            part("BIPOD", "Steady from cover and prone", 120, StatMods { recoil: 0.82, spread: 0.82, handling: 0.9, weight: 0.45, ..B }, only(&[Rifle, Sniper, Lmg])),
            part("TACTICAL LASER", "Tight hip fire", 100, StatMods { hipfire: 0.65, weight: 0.1, ..B }, except(&[Launcher])),
            part("GRIP POD", "A grip that unfolds into short legs", 140, StatMods { recoil: 0.8, spread: 0.9, handling: 0.93, weight: 0.35, ..B }, only(&[Rifle, Sniper, Lmg])),
            part("HAND STOP", "A light stop: a little control, quicker hands", 60, StatMods { recoil: 0.94, handling: 1.06, weight: 0.05, ..B }, except(&[Pistol, Launcher])),
        ],
        vec![
            part("STANDARD MAG", "Factory capacity", 0, B, GUNS),
            part("EXTENDED MAG", "+50% rounds, slower reload", 95, StatMods { capacity: 1.5, reload: 1.15, weight: 0.3, ..B }, GUNS),
            part("DRUM MAG", "Double capacity, heavy", 180, StatMods { capacity: 2.0, reload: 1.35, recoil: 1.06, handling: 0.88, weight: 1.0, ..B }, only(&[Pistol, Smg, Rifle, Lmg])),
            part("FAST MAG", "Quick reloads, fewer rounds", 130, StatMods { capacity: 0.9, reload: 0.62, weight: 0.1, ..B }, GUNS),
            part("QUAD STACK", "+75% rounds in a fat box, slow to seat", 150, StatMods { capacity: 1.75, reload: 1.25, handling: 0.95, weight: 0.6, ..B }, only(&[Smg, Rifle])),
        ],
        vec![
            part("STANDARD STOCK", "Balanced", 0, B, GUNS),
            part("NO STOCK", "Light and fast, kicks hard", 60, StatMods { recoil: 1.25, handling: 1.2, weight: -0.6, ..B }, except(&[Pistol, Launcher])),
            part("PADDED STOCK", "Soaks up recoil", 80, StatMods { recoil: 0.85, weight: 0.2, ..B }, except(&[Pistol])),
            part("TACTICAL STOCK", "Faster aim, light frame", 130, StatMods { handling: 1.1, recoil: 0.95, weight: -0.2, ..B }, except(&[Pistol])),
            part("MARKSMAN STOCK", "Cheek riser and rear monopod: rock steady, slow", 170, StatMods { recoil: 0.78, spread: 0.86, handling: 0.86, weight: 0.7, ..B }, long),
        ],
    ]
}

const BACK: f32 = 0.08;
/// Length of a bullpup's butt, which holds the action and magazine behind the grip.
const BUTT: f32 = 0.26;

fn barrel_length(frame: &GunFrame, class: GunClass, equipped: &[u8; SLOT_COUNT]) -> f32 {
    frame.barrel * stretch(class) * [1.0, 1.6, 0.6, 1.1, 1.25, 0.45][(equipped[slot::BARREL] as usize).min(5)]
}

/// How much longer than their frames say guns are drawn ahead of the grip (receiver and
/// barrel; the stock stays put): seen from behind, a long gun should reach well out in
/// front of the shooter, not look stubby against its own stock.
pub fn stretch(class: GunClass) -> f32 {
    match class {
        GunClass::Pistol => 1.1,
        _ => 1.32,
    }
}

/// Height of the surface sights and optics mount on: the receiver top, a carry handle, or the rail over a top magazine.
fn deck_height(frame: &GunFrame) -> f32 {
    frame.height * 0.5 + if frame.carry { 0.06 } else if frame.mag == MagShape::Top { 0.045 } else { 0.0 }
}

/// Grip to muzzle, including any muzzle device.
pub fn muzzle_distance(def: &WeaponDef, equipped: &[u8; SLOT_COUNT]) -> f32 {
    let Some(frame) = def.frame else { return def.length };
    let device = [0.0, 0.08, 0.26, 0.07, 0.1, 0.36, 0.06][(equipped[slot::MUZZLE] as usize).min(6)];
    frame.receiver * stretch(def.class) - BACK + barrel_length(&frame, def.class, equipped) + device
}

/// Height of the sight line above the grip.
pub fn sight_height(def: &WeaponDef, equipped: &[u8; SLOT_COUNT]) -> f32 {
    sight_line(def, equipped).aim.y
}

/// Where each optic's aim mark sits, as (height above the optic base `deck + 0.016`, distance
/// along the gun from the optic's middle `BACK - receiver * 0.45`, and where its back end is
/// along the gun): the reflex and holographic dots, and the centre of each scope's objective
/// lens (its tube's axis). Must match [`assemble`]; a test checks it does.
const OPTIC_MARK: [(f32, f32, f32); 9] = [
    (0.0, 0.0, 0.0),
    (0.045, -0.024, 0.0),
    (0.037, -0.04, 0.045),
    (0.05, -0.101, 0.08),
    (0.06, -0.247, 0.175),
    (0.056, -0.187, 0.14),
    (0.066, -0.372, 0.235),
    (0.058, -0.151, 0.112),
    (0.05, -0.085, 0.065),
];

/// A gun's sight picture in its model space (+Y up, -Z forward, origin at the grip).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SightLine {
    /// Where the eye looks from: the top of the rear notch, or the back end of the optic, on its axis.
    pub rear: Vec3,
    /// What sits on the target: the tip of the front post, the red dot, or the centre of the lens.
    pub aim: Vec3,
    /// An optic (true) or iron sights.
    pub optic: bool,
}

impl SightLine {
    /// The direction the sights look along (unit length).
    pub fn dir(&self) -> Vec3 {
        (self.aim - self.rear).normalize_or(Vec3::NEG_Z)
    }
}

/// The sight line of the gun as [`assemble`] builds it with these parts: the rear notch and
/// front post with iron sights, or the fitted optic's aim mark and axis.
pub fn sight_line(def: &WeaponDef, equipped: &[u8; SLOT_COUNT]) -> SightLine {
    let Some(f) = def.frame else {
        return SightLine { rear: Vec3::new(0.0, 0.19, 0.0), aim: Vec3::new(0.0, 0.19, -def.length.max(0.2)), optic: false };
    };
    let deck = deck_height(&f);
    let r = f.receiver * stretch(def.class);
    match equipped[slot::OPTIC] as usize {
        // The notch is cut down to `deck + 0.055` and the post comes up to the same line.
        0 => {
            let front = BACK - r;
            let zf = front - barrel_length(&f, def.class, equipped) * 0.82;
            let y = deck + 0.055;
            SightLine { rear: Vec3::new(0.0, y, BACK - 0.04), aim: Vec3::new(0.0, y, zf), optic: false }
        }
        i => {
            let (dy, dz, back) = OPTIC_MARK[i.min(OPTIC_MARK.len() - 1)];
            let (y, z0) = (deck + 0.016 + dy, BACK - r * 0.45);
            SightLine { rear: Vec3::new(0.0, y, z0 + back), aim: Vec3::new(0.0, y, z0 + dz), optic: true }
        }
    }
}

/// Builds the gun as a block model, fitted with its parts. Magazines are
/// tagged [`Anim::Mag`] and moving actions [`Anim::Slide`] so viewmodels can
/// animate them separately.
pub fn assemble(def: &WeaponDef, equipped: &[u8; SLOT_COUNT]) -> BlockModel {
    let Some(f) = def.frame else { return def.model.clone() };
    let v = |s: usize| equipped[s] as usize;
    let (w, h, r) = (f.width, f.height, f.receiver * stretch(def.class));
    let (front, mid, top) = (BACK - r, BACK - r * 0.5, h * 0.5);
    let bore_y = h * 0.12;
    let barrel = barrel_length(&f, def.class, equipped);
    let tip = front - barrel;
    let bore = f.bore * if v(slot::BARREL) == 3 { 1.35 } else { 1.0 };
    let deck = deck_height(&f);
    let bullpup = f.layout == GunLayout::Bullpup;
    let revolver = f.mag == MagShape::Cylinder;
    let slide = def.class == GunClass::Pistol && !revolver;
    let guard = (f.barrel * stretch(def.class) * 0.7).min(barrel - 0.02).max(0.05);
    let (zu, yu) = (front - (guard * 0.55).max(0.06), bore_y - h * 0.36);

    // Receiver.
    let mut m = BlockModel::new();
    m = match (revolver, f.shape) {
        // An open frame round the cylinder: top strap, lower frame, recoil shield and a hump down to the grip.
        (true, _) => m
            .cube(f.body, [0.0, top - h * 0.1, mid], [w * 0.8, h * 0.2, r])
            .cube(f.body, [0.0, -top + h * 0.17, mid], [w * 0.8, h * 0.34, r])
            .cube(f.body, [0.0, 0.0, BACK - r * 0.09], [w * 0.9, h, r * 0.18])
            .plate(f.body, 0.0, [top - 0.015, BACK - 0.02], [-top * 0.4, BACK + 0.035], w * 0.7, 0.03),
        (_, ReceiverShape::Box) => m.cube(f.body, [0.0, 0.0, mid], [w, h, r]).cube(Paint::Dark, [0.0, top + 0.008, mid + r * 0.05], [w * 0.55, 0.016, r * 0.8]),
        (_, ReceiverShape::Angular) => m
            .cube(f.body, [0.0, -h * 0.15, mid], [w, h * 0.7, r])
            .cube(f.body, [0.0, h * 0.2, mid + r * 0.12], [w * 0.94, h * 0.6, r * 0.76])
            .plate(f.body, 0.0, [top - 0.012, mid - r * 0.26], [h * 0.2 - 0.012, front + 0.012], w * 0.94, 0.024)
            .plate(f.accent, 0.0, [top - 0.01, BACK - 0.012], [h * 0.02, BACK + 0.028], w * 0.8, 0.02)
            .cube(Paint::Dark, [0.0, top + 0.006, mid + r * 0.12], [w * 0.5, 0.012, r * 0.7]),
        (_, ReceiverShape::Round) => m
            .cylinder(f.body, [0.0, h * 0.08, mid], [w, r, h * 0.84])
            .rotated([90.0, 0.0, 0.0])
            .cube(f.body, [0.0, -h * 0.26, mid + r * 0.1], [w * 0.8, h * 0.48, r * 0.7])
            .cylinder(Paint::Dark, [0.0, h * 0.08, BACK - 0.008], [w * 1.08, 0.02, h * 0.9])
            .rotated([90.0, 0.0, 0.0]),
        (_, ReceiverShape::Slab) => m
            .cube(f.body, [0.0, 0.0, mid], [w, h, r])
            .cube(f.body, [0.0, -top - h * 0.12, mid + r * 0.1], [w * 0.9, h * 0.26, r * 0.62])
            .plate(f.body, 0.0, [-top - h * 0.24, mid - r * 0.21], [-top + 0.01, front + 0.01], w * 0.9, 0.02)
            .cube(Paint::Dark, [0.0, top + 0.008, mid], [w * 0.6, 0.016, r * 0.9])
            .cylinder(Paint::Metal, [w * 0.5 + 0.012, h * 0.3, mid - r * 0.1], [0.022, r * 0.7, 0.022])
            .rotated([90.0, 0.0, 0.0]),
        (_, ReceiverShape::Sleek) => m
            .cube(f.body, [0.0, -h * 0.08, mid], [w, h * 0.84, r])
            .cylinder(f.body, [0.0, h * 0.2, mid + r * 0.05], [w * 0.96, r * 0.9, h * 0.6])
            .rotated([90.0, 0.0, 0.0])
            .plate(f.body, 0.0, [h * 0.3, front + r * 0.14], [-h * 0.32, front - 0.025], w * 0.92, 0.03)
            .plate(f.accent, 0.0, [-top + 0.01, front + 0.03], [-top - 0.025, mid + r * 0.15], w * 0.7, 0.02),
    };
    if !revolver {
        m = m
            .cube(f.accent, [w * 0.5 + 0.004, h * 0.1, mid], [0.01, h * 0.26, r * 0.62])
            .cube(f.accent, [-w * 0.5 - 0.004, h * 0.1, mid], [0.01, h * 0.26, r * 0.62])
            .cube(Paint::Dark, [w * 0.5 + 0.003, h * 0.2, mid + r * 0.18], [0.008, h * 0.22, r * 0.22]);
    }
    // A bullpup's butt holds the action: cheek comb, chamfered heel and pad.
    if bullpup {
        m = m
            .cube(f.body, [0.0, -h * 0.075, BACK + BUTT * 0.5], [w, h * 1.15, BUTT])
            .plate(f.accent, 0.0, [top + 0.004, BACK + 0.01], [top + 0.018, BACK + BUTT * 0.85], w * 0.7, 0.018)
            .plate(f.body, 0.0, [-h * 0.64, BACK + BUTT * 0.6], [-h * 0.25, BACK + BUTT + 0.015], w * 0.96, 0.03)
            .cube(Paint::Dark, [w * 0.5 + 0.003, h * 0.05, BACK + BUTT * 0.3], [0.008, h * 0.3, BUTT * 0.3]);
        if v(slot::STOCK) != 1 {
            m = m.cube(Paint::Dark, [0.0, -h * 0.075, BACK + BUTT + 0.01], [w * 1.05, h * 1.2, 0.02]);
        }
    }

    // Grip and trigger guard.
    let gy = -top - 0.085;
    m = match f.grip {
        // A thin seat flush under the receiver, fused with the tilted grip: the grip meets
        // the frame along a clean edge instead of a notch.
        GripShape::Pistol => m
            .cube(f.furniture, [0.0, gy, 0.02], [w * 0.75, 0.19, 0.1])
            .rotated([-14.0, 0.0, 0.0])
            .cube(f.furniture, [0.0, -top - 0.004, -0.002], [w * 0.75, 0.008, 0.11])
            .hull(2)
            .cube(Paint::Dark, [0.0, gy + 0.008, 0.07], [w * 0.6, 0.15, 0.012])
            .rotated([-14.0, 0.0, 0.0]),
        GripShape::Raked => m
            .cube(f.furniture, [0.0, gy + 0.005, 0.035], [w * 0.75, 0.18, 0.09])
            .rotated([-26.0, 0.0, 0.0])
            .cylinder(f.furniture, [0.0, gy - 0.07, 0.075], [0.09, w * 0.78, 0.09])
            .rotated([0.0, 0.0, 90.0])
            .cube(f.furniture, [0.0, -top - 0.004, 0.0], [w * 0.75, 0.008, 0.11])
            .hull(3),
        GripShape::Wrist | GripShape::Lever => m.plate(f.furniture, 0.0, [-top + 0.025, BACK - 0.04], [-top - 0.12, 0.13], w * 0.7, 0.065),
        GripShape::Thumbhole => {
            m = m
                .cube(f.furniture, [0.0, gy + 0.005, 0.02], [w * 0.75, 0.17, 0.09])
                .rotated([-8.0, 0.0, 0.0])
                .plate(f.furniture, 0.0, [gy - 0.075, 0.0], [-top - 0.01, -0.13], w * 0.65, 0.026);
            if bullpup {
                // A flat frame under the butt, closing the loop round the thumb.
                m.plate(f.furniture, 0.0, [gy - 0.075, 0.03], [gy - 0.065, BACK + BUTT * 0.75], w * 0.7, 0.03)
                    .plate(f.furniture, 0.0, [gy - 0.065, BACK + BUTT * 0.75], [-h * 0.6, BACK + BUTT * 0.95], w * 0.7, 0.03)
            } else {
                m.plate(f.furniture, 0.0, [gy - 0.075, 0.03], [-h * 0.95, BACK + 0.17], w * 0.7, 0.03)
            }
        }
    };
    if f.grip == GripShape::Lever {
        for (from, to) in [([-top - 0.01, -0.085], [-top - 0.09, -0.07]), ([-top - 0.09, -0.07], [-top - 0.1, 0.06]), ([-top - 0.1, 0.06], [-top - 0.035, 0.065])] {
            m = m.plate(Paint::Metal, 0.0, from, to, 0.014, 0.014).anim(Anim::Slide);
        }
    } else if f.grip != GripShape::Thumbhole {
        m = m.cube(Paint::Dark, [0.0, -top - 0.02, -0.07], [w * 0.3, 0.022, 0.12]).plate(Paint::Dark, 0.0, [-top - 0.025, -0.125], [-top + 0.005, -0.14], w * 0.3, 0.016);
    }

    // Barrel.
    let barrel_mid = front - barrel * 0.5;
    m = match f.barrel_shape {
        BarrelShape::Rotary => {
            let ring = bore * 1.5;
            for i in 0..6 {
                let a = i as f32 * std::f32::consts::FRAC_PI_3;
                m = m.cylinder(Paint::Metal, [a.cos() * ring, bore_y + a.sin() * ring, barrel_mid], [bore * 0.8, barrel, bore * 0.8]).rotated([90.0, 0.0, 0.0]);
            }
            for (z, len, size) in [(front - 0.03, 0.06, 3.0), (front - barrel * 0.55, 0.02, 2.7), (tip + 0.02, 0.04, 2.8)] {
                m = m.cylinder(Paint::Dark, [0.0, bore_y, z], [ring * size, len, ring * size]).rotated([90.0, 0.0, 0.0]);
            }
            m.cylinder(Paint::Steel, [0.0, bore_y, barrel_mid], [bore * 0.9, barrel, bore * 0.9]).rotated([90.0, 0.0, 0.0])
        }
        BarrelShape::Rails => {
            let gap = bore * 0.85;
            m = m
                .cube(Paint::Steel, [0.0, bore_y + gap, barrel_mid], [bore * 1.4, bore * 0.5, barrel])
                .cube(Paint::Steel, [0.0, bore_y - gap, barrel_mid], [bore * 1.4, bore * 0.5, barrel])
                .cube(Paint::Blue, [0.0, bore_y, barrel_mid], [bore * 0.3, bore * 0.3, barrel * 0.95]);
            for i in 0..3 {
                m = m.cylinder(f.accent, [0.0, bore_y, front - barrel * (0.2 + 0.28 * i as f32)], [bore * 2.6, 0.02, bore * 2.6]).rotated([90.0, 0.0, 0.0]);
            }
            m.cube(Paint::Dark, [0.0, bore_y, tip + 0.015], [bore * 1.8, gap * 2.6, 0.03])
        }
        BarrelShape::Brake => m
            .cylinder(Paint::Metal, [0.0, bore_y, barrel_mid], [bore * 0.85, barrel, bore * 0.85])
            .rotated([90.0, 0.0, 0.0])
            .cylinder(Paint::Metal, [0.0, bore_y, front - barrel * 0.12], [bore * 1.3, barrel * 0.24, bore * 1.3])
            .rotated([90.0, 0.0, 0.0])
            .cube(Paint::Dark, [0.0, bore_y, tip + 0.045], [bore * 2.4, bore * 1.7, 0.09])
            .cube(Paint::Metal, [0.0, bore_y, tip + 0.06], [bore * 2.6, bore * 0.5, 0.025])
            .cube(Paint::Metal, [0.0, bore_y, tip + 0.025], [bore * 2.6, bore * 0.5, 0.025]),
        BarrelShape::Twin => {
            for side in [-1.0, 1.0] {
                m = m.cylinder(Paint::Metal, [side * bore * 0.52, bore_y, barrel_mid], [bore, barrel, bore]).rotated([90.0, 0.0, 0.0]);
            }
            m
        }
        _ => m.cylinder(Paint::Metal, [0.0, bore_y, barrel_mid], [bore, barrel, bore]).rotated([90.0, 0.0, 0.0]),
    };
    m = match f.barrel_shape {
        BarrelShape::Shroud => {
            let jacket = barrel * 0.85;
            m = m
                .cylinder(f.body, [0.0, bore_y, front - jacket * 0.5], [bore * 2.1, jacket, bore * 2.1])
                .rotated([90.0, 0.0, 0.0])
                .cylinder(Paint::Dark, [0.0, bore_y, front - jacket], [bore * 2.3, 0.02, bore * 2.3])
                .rotated([90.0, 0.0, 0.0]);
            for i in 0..4 {
                let z = front - jacket * (0.16 + 0.22 * i as f32);
                m = m.cube(Paint::Dark, [0.0, bore_y, z], [bore * 2.16, bore * 0.7, jacket * 0.1]).cube(Paint::Dark, [0.0, bore_y + bore * 1.03, z], [bore * 0.5, 0.006, jacket * 0.1]);
            }
            m
        }
        BarrelShape::Fluted => m
            .cylinder(Paint::Metal, [0.0, bore_y, front - barrel * 0.4], [bore * 1.35, barrel * 0.8, bore * 1.35])
            .rotated([90.0, 0.0, 0.0])
            .cube(Paint::Dark, [0.0, bore_y, front - barrel * 0.4], [bore * 1.4, bore * 0.25, barrel * 0.7])
            .cube(Paint::Dark, [0.0, bore_y, front - barrel * 0.4], [bore * 0.25, bore * 1.4, barrel * 0.7]),
        BarrelShape::Lug => m
            .cube(f.body, [0.0, bore_y - bore * 0.75, barrel_mid], [bore * 0.9, bore * 1.1, barrel])
            .cube(f.body, [0.0, bore_y + bore * 0.55, barrel_mid], [bore * 0.5, bore * 0.35, barrel])
            .cube(Paint::Dark, [0.0, bore_y - bore * 0.75, barrel_mid], [bore * 0.95, bore * 0.2, barrel * 0.8]),
        BarrelShape::Rib => m
            .cube(Paint::Dark, [0.0, bore_y + bore * 0.62, barrel_mid], [bore * 0.4, bore * 0.25, barrel])
            .cube(Paint::Gold, [0.0, bore_y + bore * 0.85, tip + 0.012], [0.012, 0.012, 0.012]),
        // A flat rib over the pair, a web between them, a bead at the muzzle, and a splinter
        // forend under the breech whose tapered nose is fused on so the two meet without a seam.
        BarrelShape::Twin => m
            .cube(Paint::Dark, [0.0, bore_y + bore * 0.5, barrel_mid], [bore * 0.5, bore * 0.2, barrel])
            .cube(Paint::Dark, [0.0, bore_y - bore * 0.42, barrel_mid], [bore * 0.4, bore * 0.2, barrel * 0.9])
            .cube(Paint::Gold, [0.0, bore_y + bore * 0.66, tip + 0.012], [0.012, 0.012, 0.012])
            .cube(f.furniture, [0.0, bore_y - bore * 0.95, front - barrel * 0.2], [bore * 2.0, bore * 0.9, barrel * 0.36])
            .plate(f.furniture, 0.0, [bore_y - bore * 1.3, front - barrel * 0.38], [bore_y - bore * 0.65, front - barrel * 0.46], bore * 1.6, 0.02)
            .hull(2),
        _ => m,
    };
    let single = !matches!(f.barrel_shape, BarrelShape::Rotary | BarrelShape::Twin);
    if v(slot::BARREL) == 3 && single {
        for i in 0..3 {
            m = m.cylinder(Paint::Dark, [0.0, bore_y, front - barrel * (0.25 + 0.25 * i as f32)], [bore * 1.3, 0.02, bore * 1.3]).rotated([90.0, 0.0, 0.0]);
        }
    }
    // A match barrel: a gold thread ring at the crown and a dark band at the chamber.
    if v(slot::BARREL) == 4 && single {
        m = m
            .cylinder(Paint::Gold, [0.0, bore_y, tip + 0.012], [bore * 1.15, 0.024, bore * 1.15])
            .rotated([90.0, 0.0, 0.0])
            .cylinder(Paint::Dark, [0.0, bore_y, front - 0.03], [bore * 1.25, 0.06, bore * 1.25])
            .rotated([90.0, 0.0, 0.0]);
    }
    // Sawn off: a rough, flat cut at the muzzle.
    if v(slot::BARREL) == 5 {
        m = m.cube(Paint::Rust, [0.0, bore_y, tip + 0.006], [bore * if f.barrel_shape == BarrelShape::Twin { 2.2 } else { 1.15 }, bore * 1.15, 0.012]);
    }

    // Handguard, following the receiver's lines.
    if f.handguard && def.action != Action::Pump {
        let zg = front - guard * 0.5;
        m = match f.shape {
            ReceiverShape::Angular => m
                .cube(f.furniture, [0.0, bore_y - h * 0.12, zg], [w * 0.92, h * 0.5, guard])
                .plate(f.furniture, 0.0, [bore_y + h * 0.3, front], [bore_y + bore * 0.7, front - guard], w * 0.9, 0.024),
            ReceiverShape::Slab => m
                .cube(f.furniture, [0.0, bore_y - 0.005, zg], [w * 0.95, h * 0.82, guard])
                .plate(Paint::Dark, 0.0, [bore_y + h * 0.4, front - guard], [bore_y - h * 0.42, front - guard + 0.045], w * 0.9, 0.018),
            ReceiverShape::Round | ReceiverShape::Sleek => m
                .cylinder(f.furniture, [0.0, bore_y - h * 0.04, zg], [w * 0.9, guard, h * 0.66])
                .rotated([90.0, 0.0, 0.0])
                .plate(f.furniture, 0.0, [bore_y - h * 0.36, front], [bore_y - h * 0.22, front - guard], w * 0.6, 0.02),
            ReceiverShape::Box => m.cube(f.furniture, [0.0, bore_y - 0.005, zg], [w * 0.92, h * 0.72, guard]),
        };
        m = m
            .cube(Paint::Dark, [w * 0.465, bore_y, zg], [0.006, h * 0.18, guard * 0.7])
            .cube(Paint::Dark, [-w * 0.465, bore_y, zg], [0.006, h * 0.18, guard * 0.7]);
        if f.rails {
            let y_top = bore_y + h * 0.36 + 0.007;
            m = m
                .cube(Paint::Dark, [0.0, y_top, zg], [w * 0.42, 0.014, guard])
                .cube(Paint::Dark, [w * 0.5, bore_y, zg], [0.014, w * 0.36, guard * 0.85])
                .cube(Paint::Dark, [-w * 0.5, bore_y, zg], [0.014, w * 0.36, guard * 0.85])
                .cube(Paint::Dark, [0.0, bore_y - h * 0.36 - 0.007, zg], [w * 0.42, 0.014, guard * 0.85]);
            for i in 0..4 {
                m = m.cube(f.accent, [0.0, y_top + 0.004, front - guard * (0.15 + 0.23 * i as f32)], [w * 0.44, 0.008, 0.008]);
            }
        }
    }

    // Built-in furniture under the barrel, unless a part replaces it.
    if v(slot::UNDER) == 0 {
        m = match f.fore {
            Foregrip::Vertical => m.cube(f.furniture, [0.0, yu - 0.07, zu], [0.045, 0.14, 0.05]).rotated([-6.0, 0.0, 0.0]).cube(Paint::Dark, [0.0, yu - 0.142, zu + 0.008], [0.05, 0.012, 0.055]),
            Foregrip::Angled => m.plate(f.furniture, 0.0, [yu - 0.005, zu + 0.05], [yu - 0.06, zu - 0.06], 0.04, 0.035),
            Foregrip::Stop => m.plate(Paint::Dark, 0.0, [yu, zu - guard * 0.3], [yu - 0.035, zu - guard * 0.3 - 0.02], 0.03, 0.022),
            Foregrip::None => m,
        };
    }
    if f.bipod && !matches!(v(slot::UNDER), 3 | 5) {
        let zb = front - guard * 0.92;
        m = m
            .cube(Paint::Dark, [0.0, yu + 0.005, zb], [0.05, 0.025, 0.04])
            .segment(Paint::Dark, [0.02, yu, zb], [0.06, yu - 0.2, zb - 0.07], 0.014)
            .segment(Paint::Dark, [-0.02, yu, zb], [-0.06, yu - 0.2, zb - 0.07], 0.014)
            .cube(Paint::Rust, [0.06, yu - 0.2, zb - 0.07], [0.022, 0.012, 0.022])
            .cube(Paint::Rust, [-0.06, yu - 0.2, zb - 0.07], [0.022, 0.012, 0.022]);
    }
    if f.carry {
        m = m
            .cube(f.body, [0.0, top + 0.022, BACK - 0.05], [w * 0.4, 0.044, 0.04])
            .plate(f.body, 0.0, [top - 0.005, front + 0.05], [deck - 0.012, front + 0.1], w * 0.4, 0.03)
            .cube(f.body, [0.0, deck - 0.008, mid + 0.01], [w * 0.42, 0.016, r * 0.72])
            .cube(Paint::Dark, [0.0, deck - 0.018, mid + 0.01], [w * 0.44, 0.006, r * 0.6]);
    }

    // The moving action.
    m = match (def.action, def.class) {
        _ if revolver => m.plate(Paint::Dark, 0.0, [top - 0.015, BACK - 0.012], [top + 0.025, BACK + 0.022], 0.016, 0.02).anim(Anim::Slide),
        (Action::Pump, _) => {
            let len = (barrel * 0.35).max(0.1);
            let zp = front - barrel * 0.3;
            m = m
                .cube(f.furniture, [0.0, bore_y - bore * 0.95, zp], [w * 0.88, h * 0.55, len])
                .anim(Anim::Slide)
                .cube(Paint::Dark, [0.0, bore_y - bore * 0.95 - h * 0.28, zp], [w * 0.7, 0.012, len * 0.9])
                .anim(Anim::Slide);
            for i in 0..3 {
                m = m.cube(Paint::Dark, [0.0, bore_y - bore * 0.95, zp + len * (i as f32 - 1.0) * 0.28], [w * 0.9, h * 0.5, 0.01]).anim(Anim::Slide);
            }
            m
        }
        (Action::Bolt, _) if f.grip == GripShape::Lever => m.cylinder(Paint::Dark, [w * 0.5 + 0.004, h * 0.22, BACK - r * 0.3], [0.012, 0.02, 0.012]).rotated([0.0, 0.0, 90.0]).anim(Anim::Slide),
        (Action::Bolt, _) => m
            .cylinder(Paint::Metal, [w * 0.5 + 0.035, h * 0.2, BACK - r * 0.25], [0.018, 0.07, 0.018])
            .rotated([0.0, 0.0, 90.0])
            .anim(Anim::Slide)
            .shard(Paint::Dark, [w * 0.5 + 0.075, h * 0.2, BACK - r * 0.25], [0.028, 0.028, 0.028])
            .anim(Anim::Slide),
        _ if slide => {
            m = m
                .cube(Paint::Steel, [0.0, top - h * 0.16, mid - 0.01], [w * 1.05, h * 0.4, r * 1.02])
                .anim(Anim::Slide)
                .cube(Paint::Dark, [w * 0.53, top - h * 0.14, mid + r * 0.1], [0.006, h * 0.18, r * 0.25])
                .anim(Anim::Slide);
            for i in 0..3 {
                m = m.cube(Paint::Dark, [0.0, top - h * 0.16, BACK - 0.02 - 0.014 * i as f32], [w * 1.08, h * 0.3, 0.005]).anim(Anim::Slide);
            }
            if f.shape == ReceiverShape::Angular {
                m = m.plate(Paint::Steel, 0.0, [top + 0.002, front + 0.04], [top - h * 0.32, front - 0.012], w * 1.04, 0.02).anim(Anim::Slide);
            }
            m
        }
        _ => m.cube(Paint::Dark, [w * 0.5 + 0.015, h * 0.22, BACK - r * 0.32], [0.03, 0.025, 0.05]).anim(Anim::Slide),
    };

    // Optic.
    let (z0, yb) = (BACK - r * 0.45, deck + 0.016);
    m = match v(slot::OPTIC) {
        0 => {
            let zf = front - barrel * 0.82;
            let post = deck + 0.055 - bore_y;
            m = m
                // A tight rear notch and a slim front post: a precise sight picture.
                .cube(Paint::Dark, [0.014, deck + 0.03, BACK - 0.04], [0.012, 0.05, 0.02])
                .cube(Paint::Dark, [-0.014, deck + 0.03, BACK - 0.04], [0.012, 0.05, 0.02])
                .cube(Paint::Glow, [0.0, deck + 0.052, zf], [0.006, 0.006, 0.008]);
            if f.carry {
                m.plate(Paint::Dark, 0.0, [bore_y + bore * 0.4, zf + 0.02], [deck + 0.045, zf], 0.016, 0.02)
            } else {
                m.cube(Paint::Dark, [0.0, bore_y + post * 0.5, zf], [0.008, post, 0.016])
            }
        }
        1 => m
            .cube(Paint::Dark, [0.0, yb + 0.01, z0], [0.05, 0.02, 0.07])
            .cube(Paint::Dark, [0.026, yb + 0.045, z0 - 0.02], [0.008, 0.05, 0.012])
            .cube(Paint::Dark, [-0.026, yb + 0.045, z0 - 0.02], [0.008, 0.05, 0.012])
            .cube(Paint::Dark, [0.0, yb + 0.072, z0 - 0.02], [0.06, 0.008, 0.012])
            .cube(Paint::Red, [0.0, yb + 0.045, z0 - 0.024], [0.0035, 0.0035, 0.002]),
        2 => m
            .cube(Paint::Dark, [0.0, yb + 0.006, z0], [0.075, 0.012, 0.09])
            .cube(Paint::Dark, [0.034, yb + 0.037, z0], [0.008, 0.062, 0.09])
            .cube(Paint::Dark, [-0.034, yb + 0.037, z0], [0.008, 0.062, 0.09])
            .cube(Paint::Dark, [0.0, yb + 0.07, z0], [0.076, 0.008, 0.09])
            .cube(Paint::Red, [0.0, yb + 0.037, z0 - 0.04], [0.003, 0.003, 0.002]),
        3 => m
            .cube(Paint::Dark, [0.0, yb + 0.008, z0], [0.04, 0.016, 0.08])
            .cylinder(Paint::Dark, [0.0, yb + 0.05, z0], [0.06, 0.16, 0.06])
            .rotated([90.0, 0.0, 0.0])
            .cylinder(Paint::Steel, [0.0, yb + 0.05, z0 - 0.085], [0.072, 0.03, 0.072])
            .rotated([90.0, 0.0, 0.0])
            .cylinder(Paint::Blue, [0.0, yb + 0.05, z0 - 0.101], [0.056, 0.004, 0.056])
            .rotated([90.0, 0.0, 0.0]),
        4 => m
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
        // Marksman 6x: a medium tube in two rings with a modest bell.
        5 => m
            .cube(Paint::Metal, [0.0, yb + 0.018, z0 + 0.04], [0.028, 0.036, 0.018])
            .cube(Paint::Metal, [0.0, yb + 0.018, z0 - 0.08], [0.028, 0.036, 0.018])
            .cylinder(Paint::Dark, [0.0, yb + 0.056, z0 - 0.02], [0.058, 0.26, 0.058])
            .rotated([90.0, 0.0, 0.0])
            .cylinder(Paint::Dark, [0.0, yb + 0.056, z0 - 0.16], [0.08, 0.05, 0.08])
            .rotated([90.0, 0.0, 0.0])
            .cylinder(Paint::Dark, [0.0, yb + 0.056, z0 + 0.12], [0.068, 0.04, 0.068])
            .rotated([90.0, 0.0, 0.0])
            .cylinder(Paint::Metal, [0.0, yb + 0.092, z0 - 0.02], [0.026, 0.03, 0.026])
            .cylinder(Paint::Blue, [0.0, yb + 0.056, z0 - 0.187], [0.07, 0.004, 0.07])
            .rotated([90.0, 0.0, 0.0]),
        // Variable 12x: a long tube, a big objective with a sunshade, tall turrets and a gold zoom ring.
        6 => m
            .cube(Paint::Metal, [0.0, yb + 0.022, z0 + 0.07], [0.032, 0.044, 0.022])
            .cube(Paint::Metal, [0.0, yb + 0.022, z0 - 0.12], [0.032, 0.044, 0.022])
            .cylinder(Paint::Dark, [0.0, yb + 0.066, z0 - 0.04], [0.064, 0.42, 0.064])
            .rotated([90.0, 0.0, 0.0])
            .cone(Paint::Dark, [0.0, yb + 0.066, z0 - 0.27], [0.11, 0.06, 0.11])
            .rotated([90.0, 0.0, 0.0])
            .cylinder(Paint::Dark, [0.0, yb + 0.066, z0 - 0.33], [0.112, 0.08, 0.112])
            .rotated([90.0, 0.0, 0.0])
            .cylinder(Paint::Dark, [0.0, yb + 0.066, z0 + 0.2], [0.082, 0.07, 0.082])
            .rotated([90.0, 0.0, 0.0])
            .cylinder(Paint::Gold, [0.0, yb + 0.066, z0 + 0.155], [0.075, 0.016, 0.075])
            .rotated([90.0, 0.0, 0.0])
            .cylinder(Paint::Metal, [0.0, yb + 0.112, z0 - 0.03], [0.034, 0.05, 0.034])
            .cylinder(Paint::Metal, [0.046, yb + 0.066, z0 - 0.03], [0.034, 0.04, 0.034])
            .rotated([0.0, 0.0, 90.0])
            .cylinder(Paint::Metal, [-0.044, yb + 0.066, z0 - 0.03], [0.044, 0.02, 0.044])
            .rotated([0.0, 0.0, 90.0])
            .cylinder(Paint::Blue, [0.0, yb + 0.066, z0 - 0.372], [0.1, 0.004, 0.1])
            .rotated([90.0, 0.0, 0.0]),
        // Thermal: a boxy housing with a big germanium lens, a battery pod and a glowing rear screen.
        7 => m
            .cube(Paint::Dark, [0.0, yb + 0.01, z0], [0.04, 0.02, 0.1])
            .cube(Paint::Steel, [0.0, yb + 0.058, z0 - 0.01], [0.072, 0.078, 0.2])
            .cylinder(Paint::Dark, [0.0, yb + 0.058, z0 - 0.125], [0.09, 0.05, 0.09])
            .rotated([90.0, 0.0, 0.0])
            .cylinder(Paint::Violet, [0.0, yb + 0.058, z0 - 0.151], [0.07, 0.004, 0.07])
            .rotated([90.0, 0.0, 0.0])
            .cube(Paint::Dark, [0.044, yb + 0.07, z0 + 0.02], [0.018, 0.05, 0.1])
            .cube(Paint::Dark, [0.0, yb + 0.058, z0 + 0.1], [0.06, 0.06, 0.02])
            .cube(Paint::Aether, [0.0, yb + 0.058, z0 + 0.111], [0.044, 0.036, 0.003]),
        // Hybrid: a short magnified tube with a red dot canted forty-five degrees to the right.
        8 => m
            .cube(Paint::Dark, [0.0, yb + 0.008, z0], [0.04, 0.016, 0.1])
            .cylinder(Paint::Dark, [0.0, yb + 0.05, z0], [0.058, 0.13, 0.058])
            .rotated([90.0, 0.0, 0.0])
            .cylinder(Paint::Steel, [0.0, yb + 0.05, z0 - 0.072], [0.07, 0.024, 0.07])
            .rotated([90.0, 0.0, 0.0])
            .cylinder(Paint::Blue, [0.0, yb + 0.05, z0 - 0.085], [0.054, 0.004, 0.054])
            .rotated([90.0, 0.0, 0.0])
            .cube(Paint::Dark, [0.045, yb + 0.07, z0 + 0.02], [0.04, 0.014, 0.05])
            .rotated([0.0, 0.0, -45.0])
            .cube(Paint::Dark, [0.06, yb + 0.087, z0 + 0.012], [0.034, 0.03, 0.008])
            .rotated([0.0, 0.0, -45.0])
            .cube(Paint::Red, [0.059, yb + 0.087, z0 + 0.008], [0.005, 0.005, 0.003]),
        _ => m,
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
        // A long, fat can with a vented collar.
        5 => m
            .cylinder(Paint::Dark, [0.0, bore_y, tip - 0.18], [bore * 2.6, 0.36, bore * 2.6])
            .rotated([90.0, 0.0, 0.0])
            .cylinder(Paint::Steel, [0.0, bore_y, tip - 0.03], [bore * 2.7, 0.04, bore * 2.7])
            .rotated([90.0, 0.0, 0.0])
            .cube(Paint::Metal, [0.0, bore_y, tip - 0.33], [bore * 2.75, bore * 0.5, 0.05]),
        6 => m
            .cylinder(Paint::Steel, [0.0, bore_y, tip - 0.03], [bore * 1.3, 0.06, bore * 1.3])
            .rotated([90.0, 0.0, 0.0])
            .cylinder(Paint::Gold, [0.0, bore_y, tip - 0.055], [bore * 1.4, 0.012, bore * 1.4])
            .rotated([90.0, 0.0, 0.0]),
        _ => m,
    };

    // Underbarrel.
    m = match v(slot::UNDER) {
        1 => m.cube(Paint::Dark, [0.0, yu - 0.07, zu], [0.045, 0.14, 0.05]).cube(Paint::Rust, [0.0, yu - 0.145, zu], [0.05, 0.012, 0.055]),
        2 => m.cube(Paint::Dark, [0.0, yu - 0.03, zu + 0.02], [0.04, 0.045, 0.11]).rotated([-28.0, 0.0, 0.0]),
        3 => m
            .cube(Paint::Dark, [0.0, yu - 0.012, zu + 0.04], [0.05, 0.024, 0.04])
            .cube(Paint::Metal, [0.025, yu - 0.016, zu - 0.06], [0.014, 0.014, 0.2])
            .cube(Paint::Metal, [-0.025, yu - 0.016, zu - 0.06], [0.014, 0.014, 0.2]),
        4 => m.cube(Paint::Dark, [0.0, yu - 0.025, zu], [0.04, 0.045, 0.1]).cube(Paint::Red, [0.0, yu - 0.025, zu - 0.051], [0.02, 0.02, 0.004]),
        // A fat grip with its two legs folded along it.
        5 => m
            .cube(Paint::Dark, [0.0, yu - 0.075, zu], [0.05, 0.15, 0.055])
            .cube(Paint::Metal, [0.031, yu - 0.08, zu - 0.01], [0.01, 0.15, 0.014])
            .cube(Paint::Metal, [-0.031, yu - 0.08, zu - 0.01], [0.01, 0.15, 0.014])
            .cube(Paint::Rust, [0.0, yu - 0.158, zu], [0.08, 0.014, 0.06]),
        6 => m.plate(Paint::Dark, 0.0, [yu, zu + 0.02], [yu - 0.03, zu - 0.005], 0.028, 0.02),
        _ => m,
    };

    // Magazine: in the grip on a pistol, behind the grip on a bullpup, ahead of it otherwise.
    let zm = if bullpup { BACK + BUTT * 0.42 } else { BACK - r * 0.62 };
    let mag_top = if bullpup {
        -h * 0.64
    } else if f.shape == ReceiverShape::Slab && !revolver {
        -top - h * 0.22
    } else {
        -top
    };
    let size = f.mag_len
        * match v(slot::MAGAZINE) {
            1 => 1.5,
            3 => 0.9,
            4 => 1.3,
            _ => 1.0,
        };
    // A quad stack is a wider box.
    let fat = if v(slot::MAGAZINE) == 4 { 1.45 } else { 1.0 };
    let plate = if v(slot::MAGAZINE) == 3 { Paint::Gold } else { Paint::Dark };
    let shape = if v(slot::MAGAZINE) == 2 && matches!(f.mag, MagShape::Box | MagShape::Curved) { MagShape::Drum } else { f.mag };
    m = match shape {
        MagShape::Box if slide => {
            // Up the grip, along its rake; longer magazines stick out of the bottom.
            let (dy, dz) = (-(14f32.to_radians().cos()), 14f32.to_radians().sin());
            let (ty, tz) = (gy + 0.092, -0.003);
            let len = 0.19 * size + 0.008;
            m.cube(Paint::Metal, [0.0, ty + dy * len * 0.5, tz + dz * len * 0.5], [w * 0.55 * fat, len, 0.07])
                .rotated([-14.0, 0.0, 0.0])
                .anim(Anim::Mag)
                .cube(plate, [0.0, ty + dy * (len + 0.006), tz + dz * (len + 0.006)], [w * 0.8, 0.016, 0.1])
                .rotated([-14.0, 0.0, 0.0])
                .anim(Anim::Mag)
        }
        MagShape::Box => {
            let len = 0.15 * size;
            m.cube(Paint::Metal, [0.0, mag_top - len * 0.5 + 0.01, zm], [w * 0.55 * fat, len, 0.085])
                .rotated([3.0, 0.0, 0.0])
                .anim(Anim::Mag)
                .cube(Paint::Dark, [w * 0.28 * fat, mag_top - len * 0.5 + 0.01, zm], [0.004, len * 0.7, 0.05])
                .anim(Anim::Mag)
                .cube(plate, [0.0, mag_top - len + 0.005, zm - len * 0.05], [w * 0.62 * fat, 0.018, 0.098])
                .rotated([3.0, 0.0, 0.0])
                .anim(Anim::Mag)
        }
        MagShape::Curved => {
            // A banana: three pieces, each raked further towards the muzzle.
            let seg = 0.16 * size / 3.0;
            let (mut y, mut z) = (mag_top + 0.01, zm);
            for i in 0..3 {
                let a = (8.0 + 11.0 * i as f32).to_radians();
                let (dy, dz) = (-a.cos(), -a.sin());
                m = m.cube(Paint::Metal, [0.0, y + dy * seg * 0.5, z + dz * seg * 0.5], [w * 0.55 * fat, seg + 0.008, 0.085]).rotated([a.to_degrees(), 0.0, 0.0]).anim(Anim::Mag);
                y += dy * seg;
                z += dz * seg;
            }
            m.cube(plate, [0.0, y, z], [w * 0.6 * fat, 0.018, 0.095]).rotated([30.0, 0.0, 0.0]).anim(Anim::Mag)
        }
        MagShape::Drum => {
            let (yd, zd) = if slide { (gy - 0.1, 0.04) } else { (mag_top - 0.12, zm) };
            m.cube(Paint::Metal, [0.0, yd + 0.09, zd], [w * 0.5, 0.06, 0.08])
                .anim(Anim::Mag)
                .cylinder(Paint::Metal, [0.0, yd, zd], [0.2, 0.09, 0.2])
                .rotated([0.0, 0.0, 90.0])
                .anim(Anim::Mag)
                .cylinder(plate, [0.0, yd, zd], [0.08, 0.095, 0.08])
                .rotated([0.0, 0.0, 90.0])
                .anim(Anim::Mag)
                .cylinder(Paint::Dark, [0.0, yd, zd], [0.205, 0.02, 0.205])
                .rotated([0.0, 0.0, 90.0])
                .anim(Anim::Mag)
        }
        MagShape::Tube => {
            let len = (barrel * 0.85 * size).min(barrel);
            m.cylinder(Paint::Metal, [0.0, bore_y - bore * 1.25, front - len * 0.5], [bore * 0.9, len, bore * 0.9])
                .rotated([90.0, 0.0, 0.0])
                .cylinder(Paint::Dark, [0.0, bore_y - bore * 1.25, front - len], [bore, 0.02, bore])
                .rotated([90.0, 0.0, 0.0])
        }
        MagShape::Belt => {
            // An ammunition box slung under the receiver, belt running up into its side.
            let tall = 0.13 * size;
            m.cube(Paint::Steel, [-w * 0.2, mag_top - tall * 0.5 - 0.01, zm], [0.1, tall, 0.17])
                .anim(Anim::Mag)
                .cube(Paint::Dark, [-w * 0.2, mag_top - 0.012, zm], [0.104, 0.012, 0.174])
                .anim(Anim::Mag)
                .segment(Paint::Gold, [-w * 0.2 - 0.04, mag_top - 0.01, zm + 0.02], [-w * 0.5 - 0.012, h * 0.1, zm + 0.03], 0.03)
                .anim(Anim::Mag)
        }
        MagShape::Cell => m
            .cube(Paint::Dark, [0.0, mag_top - 0.05 * size, zm], [w * 0.6, 0.1 * size, 0.09])
            .anim(Anim::Mag)
            .cube(Paint::Aether, [w * 0.31, mag_top - 0.05 * size, zm], [0.006, 0.07 * size, 0.06])
            .anim(Anim::Mag)
            .cube(Paint::Aether, [-w * 0.31, mag_top - 0.05 * size, zm], [0.006, 0.07 * size, 0.06])
            .anim(Anim::Mag),
        MagShape::Canister => m.cylinder(Paint::Teal, [0.0, mag_top - 0.06 * size, zm], [0.16, 0.12 * size, 0.16]).anim(Anim::Mag),
        MagShape::Top => {
            // Flat along the top from the muzzle end to over the butt, with ears holding the sight rail above it.
            let (z1, z2) = (front + 0.015, if bullpup { BACK + BUTT * 0.8 } else { BACK - 0.02 });
            let thick = 0.036 * (1.0 + (size - 1.0) * 0.6);
            m.cube(Paint::Metal, [0.0, top + thick * 0.5, (z1 + z2) * 0.5], [w * 0.92, thick, z2 - z1])
                .anim(Anim::Mag)
                .cube(Paint::Gold, [w * 0.465, top + thick * 0.5, (z1 + z2) * 0.5], [0.004, thick * 0.4, (z2 - z1) * 0.8])
                .anim(Anim::Mag)
                .plate(f.body, w * 0.47, [top, z0 + 0.06], [deck, z0 + 0.02], 0.012, 0.03)
                .plate(f.body, -w * 0.47, [top, z0 + 0.06], [deck, z0 + 0.02], 0.012, 0.03)
                .cube(Paint::Dark, [0.0, deck - 0.006, z0], [w * 0.95, 0.012, 0.12])
        }
        MagShape::Cylinder => {
            // The open frame window holds the cylinder; the rounds at its back face come out with a reload.
            // Sized by the frame's own bore: a heavy barrel must not swell it up into the sight line.
            let d = (h * 0.95).max(f.bore * 2.6);
            let (zc, len) = (mid - r * 0.05, r * 0.62);
            let yc = bore_y - d * 0.12;
            m = m.cylinder(Paint::Steel, [0.0, yc, zc], [d, len, d]).rotated([90.0, 0.0, 0.0]);
            for i in 0..3 {
                m = m.cube(Paint::Dark, [0.0, yc, zc], [d * 1.01, d * 0.14, len * 0.7]).rotated([0.0, 0.0, i as f32 * 60.0 + 30.0]);
            }
            m.cylinder(Paint::Gold, [0.0, yc, zc + len * 0.5 + 0.006], [d * 0.8, 0.012, d * 0.8]).rotated([90.0, 0.0, 0.0]).anim(Anim::Mag)
        }
        MagShape::Internal => m.cube(Paint::Dark, [0.0, mag_top - 0.012, zm], [w * 0.7, 0.024 * size, r * 0.35 * size.min(1.4)]).anim(Anim::Mag),
    };

    // Stock (a bullpup's butt is its receiver).
    if bullpup {
        if v(slot::STOCK) == 2 {
            m = m.cube(Paint::Rust, [0.0, -h * 0.075, BACK + BUTT + 0.03], [w * 1.08, h * 1.22, 0.03]);
        }
        if v(slot::STOCK) == 3 {
            m = m.plate(Paint::Dark, 0.0, [top + 0.02, BACK + 0.06], [top + 0.035, BACK + BUTT * 0.75], w * 0.6, 0.02);
        }
        if v(slot::STOCK) == 4 {
            m = marksman_stock(m, f.furniture, w, top + 0.03, BACK + BUTT * 0.55, -h * 0.66, BACK + BUTT * 0.85);
        }
        return m;
    }
    let stock = match v(slot::STOCK) {
        1 => StockShape::None,
        // A gun that already wears a skeleton stock gets a tube stock instead.
        3 if f.stock == StockShape::Skeleton => StockShape::Tube,
        3 => StockShape::Skeleton,
        // A padded or marksman stock on a stockless gun brings a stock to put the pad on.
        2 | 4 if f.stock == StockShape::None => StockShape::Fixed,
        _ => f.stock,
    };
    // Where a padded recoil pad goes: (height of its centre, distance behind BACK, its height).
    let pad = match stock {
        StockShape::Fixed => {
            // A straight comb over a toe that flares down to the butt.
            m = m
                .cube(f.furniture, [0.0, h * 0.22, BACK + 0.14], [w * 0.8, h * 0.5, 0.28])
                .plate(f.furniture, 0.0, [-h * 0.08, BACK], [-h * 0.5, BACK + 0.28], w * 0.8, h * 0.62)
                .hull(2)
                .cube(Paint::Dark, [0.0, -h * 0.2, BACK + 0.285], [w * 0.85, h * 1.4, 0.012]);
            Some((-h * 0.2, 0.31, h * 1.4))
        }
        StockShape::Skeleton => {
            m = m
                .cube(Paint::Dark, [0.0, top * 0.4, BACK + 0.14], [0.03, 0.03, 0.28])
                .segment(Paint::Dark, [0.0, -top * 0.6, BACK], [0.0, -h * 0.9, BACK + 0.27], 0.028)
                .cube(Paint::Dark, [0.0, -h * 0.25, BACK + 0.28], [w * 0.7, h * 1.3, 0.025]);
            Some((-h * 0.25, 0.3, h * 1.35))
        }
        StockShape::Heavy => {
            // A deep comb with a raked toe and a tall butt.
            m = m
                .cube(f.furniture, [0.0, h * 0.12, BACK + 0.15], [w * 0.9, h * 0.6, 0.3])
                .plate(f.furniture, 0.0, [-top + 0.02, BACK + 0.01], [-h * 1.0, BACK + 0.27], w * 0.85, h * 0.42)
                .cube(f.furniture, [0.0, -h * 0.25, BACK + 0.3], [w * 0.95, h * 1.45, 0.06])
                .hull(3)
                .cube(Paint::Dark, [0.0, h * 0.45, BACK + 0.17], [w * 0.6, 0.03, 0.18])
                .cube(Paint::Dark, [0.0, -h * 0.25, BACK + 0.335], [w, h * 1.5, 0.012]);
            Some((-h * 0.25, 0.355, h * 1.5))
        }
        StockShape::Folding => {
            m = m
                .cube(Paint::Dark, [0.0, 0.0, BACK + 0.015], [w * 0.9, h * 0.5, 0.03])
                .plate(f.furniture, 0.0, [h * 0.25, BACK + 0.02], [h * 0.2, BACK + 0.26], w * 0.5, 0.025)
                .plate(f.furniture, 0.0, [-h * 0.2, BACK + 0.02], [-h * 0.75, BACK + 0.26], w * 0.5, 0.025)
                .cube(f.furniture, [0.0, -h * 0.28, BACK + 0.27], [w * 0.75, h * 1.15, 0.03]);
            Some((-h * 0.28, 0.29, h * 1.15))
        }
        StockShape::Thumbhole => {
            m = m
                .plate(f.furniture, 0.0, [top - 0.015, BACK - 0.01], [top - 0.03, BACK + 0.3], w * 0.8, 0.04)
                .plate(f.furniture, 0.0, [-top - 0.03, BACK + 0.02], [-h * 1.05, BACK + 0.26], w * 0.8, 0.05)
                .cube(f.furniture, [0.0, -h * 0.3, BACK + 0.28], [w * 0.85, h * 1.6, 0.08])
                .hull(2)
                .cube(Paint::Dark, [0.0, -h * 0.3, BACK + 0.325], [w * 0.9, h * 1.65, 0.012]);
            Some((-h * 0.3, 0.345, h * 1.65))
        }
        StockShape::Sloped => {
            m = m
                .plate(f.furniture, 0.0, [h * 0.05, BACK - 0.01], [-h * 0.35, BACK + 0.34], w * 0.85, h * 0.9)
                .plate(f.furniture, 0.0, [-top, BACK + 0.02], [-h * 1.15, BACK + 0.33], w * 0.8, h * 0.5)
                .hull(2)
                .cube(Paint::Dark, [0.0, -h * 0.62, BACK + 0.345], [w * 0.88, h * 1.45, 0.015]);
            Some((-h * 0.62, 0.365, h * 1.45))
        }
        StockShape::Tube => {
            m = m
                .cylinder(Paint::Dark, [0.0, h * 0.12, BACK + 0.11], [0.035, 0.22, 0.035])
                .rotated([90.0, 0.0, 0.0])
                .cube(f.furniture, [0.0, h * 0.02, BACK + 0.2], [w * 0.8, h * 0.55, 0.12])
                .plate(f.furniture, 0.0, [h * 0.32, BACK + 0.14], [h * 0.42, BACK + 0.27], w * 0.75, 0.03)
                .plate(f.furniture, 0.0, [-h * 0.2, BACK + 0.15], [-h * 0.68, BACK + 0.27], w * 0.75, 0.04)
                .hull(3)
                .cube(Paint::Dark, [0.0, -h * 0.1, BACK + 0.28], [w * 0.85, h * 1.2, 0.025]);
            Some((-h * 0.1, 0.3, h * 1.2))
        }
        StockShape::Precision => {
            m = m
                .cube(f.furniture, [0.0, h * 0.2, BACK + 0.15], [w * 0.75, h * 0.45, 0.3])
                .plate(f.furniture, 0.0, [-h * 0.1, BACK + 0.01], [-h * 0.78, BACK + 0.2], w * 0.7, h * 0.3)
                .cube(f.furniture, [0.0, -h * 0.25, BACK + 0.29], [w * 0.8, h * 1.4, 0.05])
                .hull(3)
                .cube(Paint::Dark, [0.0, h * 0.47, BACK + 0.17], [w * 0.55, 0.025, 0.14])
                .plate(Paint::Dark, 0.0, [-h * 0.95, BACK + 0.29], [-h * 1.1, BACK + 0.24], w * 0.5, 0.02)
                .cube(Paint::Dark, [0.0, -h * 0.25, BACK + 0.32], [w * 0.85, h * 1.45, 0.012]);
            Some((-h * 0.25, 0.34, h * 1.45))
        }
        StockShape::None => None,
    };
    if let (2, Some((y, z, tall))) = (v(slot::STOCK), pad) {
        m = m.cube(Paint::Rust, [0.0, y, BACK + z], [w * 0.88, tall, 0.035]);
    }
    if let (4, Some((y, z, tall))) = (v(slot::STOCK), pad) {
        m = marksman_stock(m, f.furniture, w, y + tall * 0.5 + 0.015, BACK + z - 0.13, y - tall * 0.5, BACK + z - 0.06);
    }
    m
}

/// A cheek riser on the comb and a short rear monopod under the butt.
fn marksman_stock(m: BlockModel, furniture: Paint, w: f32, comb_y: f32, comb_z: f32, foot_y: f32, foot_z: f32) -> BlockModel {
    m.cube(furniture, [0.0, comb_y, comb_z], [w * 0.62, 0.03, 0.15])
        .cube(Paint::Dark, [w * 0.33, comb_y - 0.012, comb_z], [0.008, 0.016, 0.1])
        .segment(Paint::Dark, [0.0, foot_y, foot_z], [0.0, foot_y - 0.09, foot_z + 0.01], 0.018)
        .cube(Paint::Rust, [0.0, foot_y - 0.095, foot_z + 0.01], [0.03, 0.012, 0.03])
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
            frame: Some(GunFrame { mag: MagShape::Curved, ..GunFrame::RIFLE }),
            defaults: [0; SLOT_COUNT], model: BlockModel::new(), length: 0.6, weight: 3.5,
        }
    }

    #[test]
    fn recoil_scales_with_class_parts_stance_and_aim() {
        use crate::kit::standard_guns;
        let guns = standard_guns();
        let table = standard_attachments();
        // Average aim climb per shot (radians) for each class, standing, from the hip.
        let average = |class: GunClass| {
            let kicks: Vec<f32> = guns.iter().filter(|g| g.class == class && g.frame.is_some()).map(|g| shot_kick(g.class, g.recoil, 1.0, false) * RECOIL_RADIANS).collect();
            assert!(!kicks.is_empty(), "no {class:?}");
            kicks.iter().sum::<f32>() / kicks.len() as f32
        };
        let (smg, pistol, rifles, lmg, shotgun, sniper) =
            (average(GunClass::Smg), average(GunClass::Pistol), average(GunClass::Rifle), average(GunClass::Lmg), average(GunClass::Shotgun), average(GunClass::Sniper));
        // Light SMGs, then pistols, rifles; LMGs heavier; shotguns and snipers kick hardest.
        assert!(smg < pistol && pistol < rifles && rifles < lmg && lmg < shotgun && shotgun < sniper, "{smg} {pistol} {rifles} {lmg} {shotgun} {sniper}");
        // A clear difference: a sniper round kicks several times an SMG round.
        assert!(sniper > smg * 6.0, "{sniper} vs {smg}");
        // Noticeable but sane per shot: between a twentieth of a degree and two degrees.
        for g in guns.iter().filter(|g| g.frame.is_some()) {
            let k = shot_kick(g.class, g.recoil, 1.0, false) * RECOIL_RADIANS;
            assert!(k > 0.05f32.to_radians() && k < 2.0f32.to_radians(), "{}: {k}", g.name);
        }
        // Controllable: no automatic climbs more than ten degrees a second held from the hip
        // (the pattern adds up to 35% late in a burst), and aimed and crouched it is far less.
        for g in guns.iter().filter(|g| g.frame.is_some() && g.action == Action::Auto) {
            let per_second = shot_kick(g.class, g.recoil, 1.0, false) * RECOIL_RADIANS * 1.35 / g.interval;
            assert!(per_second < 10f32.to_radians(), "{} climbs {} deg/s", g.name, per_second.to_degrees());
            let braced = shot_kick(g.class, g.recoil, 0.72, true) * RECOIL_RADIANS * 1.35 / g.interval;
            assert!(braced < per_second * 0.6);
        }
        // Parts: a vertical grip and a compensator each tame it; no stock makes it worse.
        let def = rifle();
        let mut state = WeaponState::fresh(&def);
        let base = shot_kick(def.class, resolve(&def, &state, &table).recoil, 1.0, false);
        let find = |slot_index: usize, name: &str| table[slot_index].iter().position(|p| p.name == name).unwrap_or_else(|| panic!("{name}")) as u8;
        state.equipped[slot::UNDER] = find(slot::UNDER, "VERTICAL GRIP");
        let gripped = shot_kick(def.class, resolve(&def, &state, &table).recoil, 1.0, false);
        state.equipped[slot::MUZZLE] = find(slot::MUZZLE, "COMPENSATOR");
        let both = shot_kick(def.class, resolve(&def, &state, &table).recoil, 1.0, false);
        assert!(gripped < base * 0.8 && both < gripped * 0.8, "{base} {gripped} {both}");
        let mut bare = WeaponState::fresh(&def);
        bare.equipped[slot::STOCK] = find(slot::STOCK, "NO STOCK");
        assert!(shot_kick(def.class, resolve(&def, &bare, &table).recoil, 1.0, false) > base);
        // Stance and aiming: crouched < standing, prone < crouched, aimed < hip.
        assert!(shot_kick(def.class, 1.0, 0.72, false) < base && shot_kick(def.class, 1.0, 0.45, false) < shot_kick(def.class, 1.0, 0.72, false));
        assert!(shot_kick(def.class, 1.0, 1.0, true) < base);
        // The class sets the scale for the same recoil figure.
        assert!(shot_kick(GunClass::Smg, 1.0, 1.0, false) < shot_kick(GunClass::Rifle, 1.0, 1.0, false));
        assert!(shot_kick(GunClass::Sniper, 1.0, 1.0, false) > shot_kick(GunClass::Rifle, 1.0, 1.0, false));
        // Capped, so the heaviest gun is still usable.
        assert_eq!(shot_kick(GunClass::Sniper, 9.0, 1.0, false), MAX_KICK);
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
