//! Iron Town - close-quarters team deathmatch in a procedural town.
//!
//! 1v1 up to 3v3 (or more), with bots filling every empty slot. Players pick
//! a class - two weapons with free attachments, a perk and a piece of gear -
//! and fight to a score limit.
//!
//! The whole game is this one file: a catalog and rules. The lobby, team
//! balancing, bots, scoring, movement, weapons, netcode, rendering and menus
//! all come from the shared Opnix Zone crates.

use bevy::prelude::*;
use oz_sim::prelude::*;

pub fn catalog(_: &Rules) -> Catalog {
    let mut weapons = oz_core::kit::standard_guns();
    for weapon in &mut weapons {
        weapon.price = Cost::FREE;
        // Versus balance: generous reserves, no shopping.
        weapon.reserve = weapon.mag * 4;
    }
    weapons[0].reserve = 0;
    Catalog {
        weapons,
        attachments: standard_attachments(),
        perks: vec![
            PerkDef { name: "LIGHTWEIGHT", blurb: "Move 12% faster", effect: PerkEffect::Speed(1.12) },
            PerkDef { name: "PLATE CARRIER", blurb: "+30 health", effect: PerkEffect::Health(30.0) },
            PerkDef { name: "FAST HANDS", blurb: "Reload 35% faster", effect: PerkEffect::Reload(0.65) },
            PerkDef { name: "DEADEYE", blurb: "30% tighter spread", effect: PerkEffect::Spread(0.7) },
            PerkDef { name: "SCAVENGER", blurb: "Kills refill a magazine of reserve ammo", effect: PerkEffect::Scavenger },
            PerkDef { name: "GHOST", blurb: "Never shows on enemy radar", effect: PerkEffect::Ghost },
            PerkDef { name: "FLAK JACKET", blurb: "Take 55% less explosive damage", effect: PerkEffect::FlakJacket(0.45) },
            PerkDef { name: "QUICK DRAW", blurb: "Aim and swap 30% faster", effect: PerkEffect::Handling(1.3) },
            PerkDef { name: "STEADY HANDS", blurb: "40% tighter hip fire", effect: PerkEffect::Hipfire(0.6) },
            PerkDef { name: "THERMAL OPTICS", blurb: "Aiming shows enemies glowing hot, even through smoke", effect: PerkEffect::Thermal },
            PerkDef { name: "NIGHT VISION", blurb: "A visor for night maps - toggle with N", effect: PerkEffect::NightVision },
        ],
        gear: vec![
            GearDef { name: "FRAG GRENADE", blurb: "Thrown explosive on a short fuse", kind: GearKind::Frag { damage: 170.0, radius: 10.0 }, count: 2 },
            GearDef { name: "STIM", blurb: "Instantly restores 70 health and puts out fire", kind: GearKind::Stim { heal: 70.0 }, count: 2 },
            GearDef { name: "SMOKE", blurb: "A thick cloud that blocks sight for 12 seconds", kind: GearKind::Smoke { radius: 5.5, seconds: 12.0 }, count: 2 },
            GearDef { name: "FLASHBANG", blurb: "Blinds and deafens anyone facing the blast", kind: GearKind::Flash { radius: 12.0, seconds: 3.5 }, count: 2 },
            GearDef { name: "PROXIMITY MINE", blurb: "Arms after a second, blows when an enemy comes close", kind: GearKind::Mine { damage: 170.0, radius: 5.5 }, count: 2 },
            GearDef { name: "THROWING KNIFE", blurb: "Silent, fast, lethal on a direct hit", kind: GearKind::Knife { damage: 160.0 }, count: 3 },
            GearDef { name: "AMMO PACK", blurb: "Refills reserves for you and teammates nearby", kind: GearKind::Ammo { radius: 6.0 }, count: 1 },
        ],
        ..default()
    }
}

/// Team deathmatch. `team_size` and the biome can be changed from the menu.
pub fn deathmatch_rules(seed: u64) -> Rules {
    Rules {
        mode: 0,
        recipe: WorldRecipe::town(seed, Biome::IronShoals),
        day_seconds: 0.0,
        objective_hp: 0.0,
        mining: false,
        difficulty: 1.0,
        start_stock: Stock::default(),
        // Pistol and carbine until the player picks a class.
        starter_weapons: (1 << oz_core::kit::gun::PISTOL) | (1 << oz_core::kit::gun::CARBINE),
        // Long enough to watch the kill-cam replay of how you went down.
        respawn_seconds: 5.5,
        pvp: true,
        team_size: 2,
        score_limit: 25,
        hunt: false,
        ranked: false,
        loot: false,
    }
}

/// Terrorist hunt: your squad (you, friends, and bots for any empty seats) clears a town
/// held by a crowd of hostiles that hold posts, patrol, and come running at noise.
/// Hostiles never respawn; the squad shares a few lives.
pub fn hunt_rules(seed: u64) -> Rules {
    Rules { hunt: true, team_size: 1, score_limit: 0, ..deathmatch_rules(seed) }
}

/// Ranked team deathmatch: the same fight, for rank points.
pub fn ranked_rules(seed: u64) -> Rules {
    Rules { ranked: true, team_size: 4, ..deathmatch_rules(seed) }
}

/// Scavenge: team deathmatch where you spawn with a pistol and loot the town for the rest.
pub fn scavenge_rules(seed: u64) -> Rules {
    Rules { loot: true, team_size: 3, score_limit: 30, ..deathmatch_rules(seed) }
}

pub struct IronTownPlugin;

impl Plugin for IronTownPlugin {
    fn build(&self, app: &mut App) {
        let planned = |name, blurb| ModeDef { name, blurb, playable: false, rules: deathmatch_rules, catalog };
        app.insert_resource(Modes(vec![
            ModeDef { name: "TEAM DEATHMATCH", blurb: "Two squads, one town. Bots fill empty slots. First to the score limit wins.", playable: true, rules: deathmatch_rules, catalog },
            ModeDef { name: "SCAVENGE", blurb: "Spawn with a pistol. Guns, ammo, gear and armour lie around town; the fallen drop theirs. Keep fighting to stay armed.", playable: true, rules: scavenge_rules, catalog },
            ModeDef { name: "RANKED TEAM DEATHMATCH", blurb: "Team deathmatch for rank: wins climb from bronze to diamond, losses cost points.", playable: true, rules: ranked_rules, catalog },
            ModeDef {
                name: "TERRORIST HUNT",
                blurb: "Clear the town of hostiles. They hear gunfire and footsteps: suppressors and a light step keep you hidden.",
                playable: true,
                rules: hunt_rules,
                catalog,
            },
            planned("DOMINATION", "PLANNED - hold three points around the town to score."),
            planned("SEARCH AND DESTROY", "PLANNED - one life per round, plant or defuse."),
        ]));
    }
}
