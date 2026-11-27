//! Iron Town's horde mode content: the guns (stock and reforged), the husks, and the rules.
//! The round director, points, perks and the rest live in `oz_sim::zombies`.

use oz_core::arsenal::{Caliber, slot};
use oz_core::kit::gun;
use oz_sim::prelude::*;

/// Reforged names for the kit guns, by kit index; anything newer gets a generic one.
fn reforged_name(index: usize, name: &'static str) -> &'static str {
    match index {
        gun::PISTOL => "SUPERNOVA PISTOL",
        gun::SMG => "RIPTIDE SMG",
        gun::SHOTGUN => "TSUNAMI SHOTGUN",
        gun::CARBINE => "PATHFINDER CARBINE",
        gun::LANCE => "THUNDER LANCE",
        gun::LAUNCHER => "SOLAR FLARE",
        gun::MINIGUN => "HURRICANE LMG",
        gun::SNIPER => "FARSHOT RIFLE",
        gun::BURST => "POSEIDON BURST",
        gun::REVOLVER => "HARBOR MASTER",
        gun::DMR => "WATCHTOWER DMR",
        gun::BREACHER => "BATTERING RAM",
        gun::PDW => "SWARM PDW",
        gun::BATTLE_RIFLE => "BASTION RIFLE",
        gun::BULLPUP => "WARLORD BULLPUP",
        gun::MACHINE_PISTOL => "KILLER BEE",
        gun::DRUM_SMG => "TRENCH STORM",
        gun::CQB => "PHANTOM CQB",
        gun::LEVER => "FRONTIER JUSTICE",
        gun::ANTI_MATERIEL => "TITAN RIFLE",
        gun::ROTARY => "MAELSTROM ROTARY",
        _ => Box::leak(format!("{name} MK II").into_boxed_str()),
    }
}

/// The hottest load a caliber takes: the reforge's tracer and kick.
fn hot_load(caliber: Caliber) -> u8 {
    oz_core::arsenal::ammo_for(caliber).len().saturating_sub(1) as u8
}

fn husk(name: &'static str, hp: f32, speed: f32, radius: f32, damage: f32, interval: f32, gibs: Paint) -> EnemyDef {
    EnemyDef {
        name,
        hp,
        speed,
        radius,
        damage,
        attack_interval: interval,
        reward: 0,
        threat: 1,
        first_wave: 1,
        brain: Brain::Melee,
        prefers: Prefers::Players,
        boss: false,
        gibs,
        // Drawn by the client's own husk rig (see `oz_client::zombies_hud`).
        model: BlockModel::new(),
    }
}

pub fn catalog(_: &Rules) -> Catalog {
    let mut weapons = oz_core::kit::standard_guns();
    for (i, weapon) in weapons.iter_mut().enumerate() {
        weapon.price = Cost::FREE;
        weapon.reserve = (weapon.mag * 5).clamp(24, 360);
        if i == gun::PISTOL {
            weapon.infinite_reserve = false;
            weapon.reserve = 80;
        }
    }
    let base = weapons.len();
    for i in 0..base {
        let mut w = weapons[i].clone();
        w.name = reforged_name(i, w.name);
        w.damage *= if i == gun::PISTOL { 3.0 } else { 2.2 };
        w.mag = ((w.mag as f32 * 1.5).ceil() as u16).max(w.mag + 1);
        w.reserve = (w.reserve * 2).min(600);
        w.interval *= 0.9;
        w.reload *= 0.85;
        if i == gun::PISTOL {
            w.splash = w.splash.max(1.6);
        }
        w.defaults[slot::AMMO] = hot_load(w.caliber);
        if let Some(frame) = w.frame.as_mut() {
            frame.body = Paint::Dark;
            frame.accent = Paint::Violet;
            frame.furniture = Paint::Purple;
        }
        for part in &mut w.model.parts {
            if !part.paint.unlit() {
                part.paint = if part.paint == Paint::Dark { Paint::Purple } else { Paint::Dark };
            } else {
                part.paint = Paint::Violet;
            }
        }
        weapons.push(w);
    }
    Catalog {
        weapons,
        attachments: standard_attachments(),
        enemies: vec![
            husk("HUSK", 150.0, 1.3, 0.42, 55.0, 1.0, Paint::Rust),
            husk("SLAG BRUTE", 150.0, 2.6, 0.6, 90.0, 1.4, Paint::Iron),
            husk("CRAWLER", 150.0, 1.0, 0.4, 35.0, 1.1, Paint::Rust),
        ],
        perks: vec![
            PerkDef { name: "NONE", blurb: "", effect: PerkEffect::Speed(1.0) },
            PerkDef { name: "FLEETFOOT", blurb: "Run and walk faster", effect: PerkEffect::Speed(1.15) },
        ],
        gear: vec![GearDef { name: "FRAG GRENADE", blurb: "Two a round", kind: GearKind::Frag { damage: 400.0, radius: 6.5 }, count: 2, weight: 0.4 }],
        ..Default::default()
    }
}

/// Horde: you, friends and bots against endless rounds of husks.
pub fn horde_rules(_seed: u64) -> Rules {
    Rules {
        // Its place in Iron Town's mode list (after domination and search and destroy).
        mode: 6,
        recipe: WorldRecipe::horde(oz_core::worldgen::Layout::CinderWorks),
        day_seconds: 0.0,
        objective_hp: 0.0,
        mining: false,
        difficulty: 1.0,
        start_stock: Stock::default(),
        starter_weapons: 1 << gun::PISTOL,
        // Nobody respawns on a timer: the fallen are back at the next round.
        respawn_seconds: 1.0e6,
        pvp: false,
        team_size: 2,
        score_limit: 0,
        hunt: false,
        ranked: false,
        loot: false,
        horde: true,
        time_limit: 0.0,
        no_bots: false,
        map: 0,
        domination: false,
        bomb: false,
    }
}
