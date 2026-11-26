//! Iron Town - close-quarters team deathmatch in a procedural town.
//!
//! 1v1 up to 3v3 (or more), with bots filling every empty slot. Players pick
//! a class - two weapons with free attachments, a perk and a piece of gear -
//! and fight to a score limit.
//!
//! The whole game is this one file: a catalog, rules, and a small director.
//! Movement, weapons, bots, netcode, rendering and menus all come from the
//! shared Opnix Zone crates.

use bevy::prelude::*;
use oz_sim::{bots::{FIRST_BOT_ID, spawn_bot}, net::NetIds, prelude::*};

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
        ],
        gear: vec![
            GearDef { name: "FRAG GRENADE", blurb: "Thrown explosive on a short fuse", kind: GearKind::Frag { damage: 150.0, radius: 6.5 }, count: 2 },
            GearDef { name: "STIM", blurb: "Instantly restores 70 health", kind: GearKind::Stim { heal: 70.0 }, count: 2 },
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
        starter_weapons: 0b1001,
        respawn_seconds: 3.0,
        pvp: true,
        team_size: 2,
        score_limit: 25,
    }
}

pub const TEAM_NAMES: [&str; 2] = ["TEAL", "CORAL"];

pub struct IronTownPlugin;

impl Plugin for IronTownPlugin {
    fn build(&self, app: &mut App) {
        let planned = |name, blurb| ModeDef { name, blurb, playable: false, rules: deathmatch_rules, catalog };
        app.insert_resource(Modes(vec![
            ModeDef { name: "TEAM DEATHMATCH", blurb: "Two squads, one town. Bots fill empty slots. First to the score limit wins.", playable: true, rules: deathmatch_rules, catalog },
            planned("DOMINATION", "PLANNED - hold three points around the town to score."),
            planned("SEARCH AND DESTROY", "PLANNED - one life per round, plant or defuse."),
        ]))
        .add_systems(Update, run_match.in_set(SimSet::Logic));
    }
}

/// Keeps both teams full with bots and ends the match at the score limit.
fn run_match(
    rules: Res<Rules>,
    content: Res<Content>,
    map: Res<Map>,
    mut state: ResMut<MatchState>,
    mut rng: ResMut<SimRng>,
    mut ids: ResMut<NetIds>,
    mut commands: Commands,
    players: Query<(Entity, &Player, Has<Bot>)>,
    mut fx: EventWriter<FxEvent>,
) {
    if !rules.pvp || state.over() {
        return;
    }
    state.daylight = 1.0;
    for team in 0..2u8 {
        let members: Vec<_> = players.iter().filter(|(_, player, _)| player.team == team).collect();
        let wanted = rules.team_size.max(1) as usize;
        if members.len() < wanted {
            // Bot ids start high and keep the team's parity (teams alternate by id).
            if let Some(id) = (FIRST_BOT_ID..=253).filter(|id| id % 2 == team).find(|id| !players.iter().any(|(_, player, _)| player.id == *id)) {
                spawn_bot(&mut commands, &mut ids, &content, &rules, &map, &mut rng, id);
            }
        } else if members.len() > wanted {
            // A human joined: release one bot from this team.
            if let Some((bot, ..)) = members.iter().find(|(_, _, is_bot)| *is_bot) {
                commands.entity(*bot).despawn();
            }
        }
        if state.scores[team as usize] >= rules.score_limit && rules.score_limit > 0 {
            state.phase = Phase::Won;
            state.winner = team;
            fx.write(FxEvent::notice(format!("TEAM {} WINS", TEAM_NAMES[team as usize])));
            fx.write(FxEvent::Sound { sfx: Sfx::Dawn, at: None });
        }
    }
}
