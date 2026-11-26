//! Weapons, attachments, mods, and the pipeline that turns them into final stats.
//!
//! Definitions ([`WeaponDef`], [`Attachment`]) are static content owned by a
//! game's catalog. [`WeaponState`] is the small serialisable per-player record
//! of what is owned and equipped. [`resolve`] combines both into [`Resolved`],
//! which is the only thing the firing code ever reads.

use crate::{economy::Cost, model::{BlockModel, Paint}};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FireMode {
    /// Instant ray.
    Hitscan,
    /// Spawns a simulated projectile (rockets, grenades, plasma).
    Projectile { speed: f32, gravity: f32 },
    /// Continuous short-range beam; also the mining tool.
    Beam,
}

#[derive(Clone, Debug)]
pub struct WeaponDef {
    pub name: &'static str,
    pub blurb: &'static str,
    pub price: Cost,
    pub mag: u16,
    /// Reserve ammo granted on purchase. Ignored when `infinite_reserve`.
    pub reserve: u16,
    pub infinite_reserve: bool,
    /// Credits for one refill pack of `mag * 2` rounds.
    pub ammo_price: u32,
    pub damage: f32,
    /// Seconds between shots.
    pub interval: f32,
    pub reload: f32,
    pub pellets: u8,
    /// Cone half-angle in radians.
    pub spread: f32,
    pub range: f32,
    pub splash: f32,
    pub recoil: f32,
    pub auto: bool,
    pub mode: FireMode,
    pub ads_fov: f32,
    /// Ore units per second when aimed at a deposit. 0 = cannot mine.
    pub mining: f32,
    /// Barrel length in metres; places the muzzle flash and barrel attachments.
    pub length: f32,
    /// Index into the shot sound bank.
    pub sound: u8,
    pub model: BlockModel,
}

pub const SLOTS: [&str; 5] = ["OPTIC", "MAGAZINE", "GRIP", "BARREL", "STOCK"];

/// Multipliers applied by one attachment. `fov` overrides when non-zero; `length` adds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StatMods {
    pub damage: f32,
    pub capacity: f32,
    pub reload: f32,
    pub recoil: f32,
    pub spread: f32,
    pub range: f32,
    pub fov: f32,
    pub length: f32,
}

impl StatMods {
    pub const NONE: StatMods = StatMods { damage: 1.0, capacity: 1.0, reload: 1.0, recoil: 1.0, spread: 1.0, range: 1.0, fov: 0.0, length: 0.0 };
}

#[derive(Clone, Debug)]
pub struct Attachment {
    pub name: &'static str,
    pub cost: u32,
    pub mods: StatMods,
}

/// `table[slot][variant]`; variant 0 is always the free standard part.
pub type AttachmentTable = Vec<Vec<Attachment>>;

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

/// What one player owns and has equipped on one weapon.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WeaponState {
    pub owned: bool,
    pub ammo: u16,
    pub reserve: u16,
    /// Equipped variant per slot (0 = standard).
    pub equipped: [u8; 5],
    /// Bitmask of purchased variants per slot.
    pub purchased: [u8; 5],
    pub mods: [Option<(ModKind, u8)>; MOD_SLOTS],
}

impl WeaponState {
    pub fn has_part(&self, slot: usize, variant: usize) -> bool {
        variant == 0 || self.purchased[slot] & (1 << variant) != 0
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

/// Final numbers for one weapon after attachments and mods.
#[derive(Clone, Copy, Debug)]
pub struct Resolved {
    pub damage: f32,
    pub mag: u16,
    pub interval: f32,
    pub reload: f32,
    pub recoil: f32,
    pub spread: f32,
    pub range: f32,
    pub splash: f32,
    pub ads_fov: f32,
    pub length: f32,
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
    for (slot, variants) in table.iter().enumerate().take(5) {
        let Some(part) = variants.get(state.equipped[slot] as usize) else { continue };
        m.damage *= part.mods.damage;
        m.capacity *= part.mods.capacity;
        m.reload *= part.mods.reload;
        m.recoil *= part.mods.recoil;
        m.spread *= part.mods.spread;
        m.range *= part.mods.range;
        m.length += part.mods.length;
        if part.mods.fov > 0.0 {
            m.fov = part.mods.fov;
        }
    }
    let level = |kind| state.mod_level(kind) as f32;
    Resolved {
        damage: def.damage * m.damage * (1.0 + 0.2 * level(ModKind::Heavy)),
        mag: ((def.mag as f32 * m.capacity).ceil() as u16).max(1),
        interval: def.interval / (1.0 + 0.15 * level(ModKind::Capacitor)),
        reload: def.reload * m.reload,
        recoil: def.recoil * m.recoil,
        spread: def.spread * m.spread,
        range: def.range * m.range,
        splash: def.splash + 1.3 * level(ModKind::Split),
        ads_fov: if m.fov > 0.0 { m.fov } else { def.ads_fov },
        length: def.length + m.length,
        arc: 0.3 * level(ModKind::Arc),
        frost: 0.9 * level(ModKind::Frost),
        venom: 5.0 * level(ModKind::Venom),
        scavenge: 1.5 * level(ModKind::Scavenger),
        repair: 0.8 * level(ModKind::Repair),
    }
}

/// The classic five-slot table from the Ember Atoll prototype; reusable by any game.
pub fn standard_attachments() -> AttachmentTable {
    const B: StatMods = StatMods::NONE;
    let part = |name, cost, mods| Attachment { name, cost, mods };
    vec![
        vec![
            part("IRON SIGHTS", 0, B),
            part("REFLEX", 85, StatMods { fov: 58.0, spread: 0.90, ..B }),
            part("PRISM", 150, StatMods { fov: 42.0, spread: 0.82, ..B }),
            part("LONG SCOPE", 230, StatMods { fov: 27.0, spread: 0.72, ..B }),
        ],
        vec![
            part("STANDARD MAG", 0, B),
            part("EXTENDED", 95, StatMods { capacity: 1.5, reload: 1.15, ..B }),
            part("DRUM", 180, StatMods { capacity: 2.0, reload: 1.35, recoil: 1.08, ..B }),
            part("QUICKLOAD", 130, StatMods { capacity: 0.9, reload: 0.65, ..B }),
        ],
        vec![
            part("STANDARD GRIP", 0, B),
            part("VERTICAL", 80, StatMods { recoil: 0.76, ..B }),
            part("ANGLED", 105, StatMods { spread: 0.78, ..B }),
            part("GYRO", 200, StatMods { recoil: 0.69, spread: 0.85, ..B }),
        ],
        vec![
            part("STANDARD BARREL", 0, B),
            part("COMPENSATOR", 110, StatMods { recoil: 0.72, length: 0.10, ..B }),
            part("LONG BARREL", 160, StatMods { damage: 1.12, range: 1.3, spread: 0.86, length: 0.24, ..B }),
            part("SHORT BARREL", 90, StatMods { range: 0.78, damage: 0.94, reload: 0.87, length: 0.04, ..B }),
        ],
        vec![
            part("STANDARD STOCK", 0, B),
            part("PADDED", 80, StatMods { recoil: 0.85, ..B }),
            part("PRECISION", 145, StatMods { spread: 0.8, recoil: 0.87, ..B }),
            part("LIGHT FRAME", 130, StatMods { reload: 0.84, recoil: 1.08, ..B }),
        ],
    ]
}

/// Geometry for an equipped attachment, in weapon space (grip at the origin, barrel along -Z).
/// Variant 0 (standard) has no extra geometry.
pub fn part_model(slot: usize, variant: usize, length: f32) -> BlockModel {
    let model = BlockModel::new();
    if variant == 0 {
        return model;
    }
    let v = variant as f32;
    match slot {
        // Optic: a tube that grows with magnification, with a glowing lens.
        0 => {
            let tube = 0.08 + 0.09 * v;
            model
                .cube(Paint::Dark, [0.0, 0.19, -0.12], [0.07, 0.07, tube])
                .cube(Paint::Blue, [0.0, 0.19, -0.12 - tube * 0.5], [0.05, 0.05, 0.012])
                .cube(Paint::Metal, [0.0, 0.14, -0.12], [0.04, 0.04, 0.06])
        }
        // Magazine: extended, drum, quickload.
        1 => match variant {
            1 => model.cube(Paint::Metal, [0.0, -0.24, -0.12], [0.09, 0.30, 0.13]),
            2 => model.cylinder(Paint::Metal, [0.0, -0.22, -0.12], [0.24, 0.10, 0.24]).rotated([0.0, 0.0, 90.0]),
            _ => model.cube(Paint::Gold, [0.0, -0.18, -0.12], [0.09, 0.16, 0.13]),
        },
        // Grip under the barrel.
        2 => model.cube(if variant == 3 { Paint::Teal } else { Paint::Dark }, [0.0, -0.10, -0.34], [0.06, 0.16 - 0.03 * v, 0.07 + 0.03 * v]),
        // Barrel extension past the muzzle.
        3 => {
            let extra = [0.0, 0.10, 0.24, 0.04][variant.min(3)];
            model.cube(if variant == 1 { Paint::Dark } else { Paint::Metal }, [0.0, 0.02, -length - extra * 0.5], [0.07 + 0.02 * (variant == 1) as u8 as f32, 0.07, extra])
        }
        // Stock behind the grip.
        _ => model.cube(if variant == 2 { Paint::Gold } else { Paint::Dark }, [0.0, -0.02, 0.30], [0.08, 0.12 + 0.02 * v, 0.22]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rifle() -> WeaponDef {
        WeaponDef {
            name: "TEST", blurb: "", price: Cost::FREE, mag: 20, reserve: 60, infinite_reserve: false, ammo_price: 10, damage: 10.0,
            interval: 0.1, reload: 1.0, pellets: 1, spread: 0.01, range: 50.0, splash: 0.0, recoil: 1.0, auto: true,
            mode: FireMode::Hitscan, ads_fov: 55.0, mining: 0.0, length: 0.6, sound: 0, model: BlockModel::new(),
        }
    }

    #[test]
    fn attachments_and_mods_stack() {
        let table = standard_attachments();
        let mut state = WeaponState { owned: true, ..Default::default() };
        let base = resolve(&rifle(), &state, &table);
        assert_eq!((base.mag, base.damage, base.ads_fov), (20, 10.0, 55.0));
        state.equipped = [3, 2, 0, 2, 0];
        assert!(state.add_mod(ModKind::Heavy) && state.add_mod(ModKind::Heavy));
        let tuned = resolve(&rifle(), &state, &table);
        assert_eq!(tuned.mag, 40);
        assert_eq!(tuned.ads_fov, 27.0);
        assert!((tuned.damage - 10.0 * 1.12 * 1.4).abs() < 1e-4);
        assert!(tuned.range > base.range && tuned.reload > base.reload);
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
