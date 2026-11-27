//! Team-versus plumbing any mode can switch on with `Rules::pvp`:
//! a pre-match lobby, team balancing, bot backfill, the score limit, and
//! returning to the lobby for a rematch. Also keeps [`MatchState::roster`]
//! up to date for every mode, which is where names and scoreboards come from.

use crate::{
    bots::{Bot, FIRST_BOT_ID, spawn_bot},
    net::NetIds,
    player::{Arsenal, Health, Player, PlayerInput, respawn},
    session::{Content, Map, Modes, Relaunch},
    sim_dt,
    state::{CommandEvent, FxEvent, LobbyAction, MatchState, Phase, PlayerCommand, RosterEntry, Rules, SimRng},
};
use bevy::prelude::*;
use oz_core::{
    synth::Sfx,
    worldgen::{Biome, Layout, Theme, Weather, WorldRecipe},
};

const COUNTDOWN: f32 = 5.0;
const POST_MATCH: f32 = 14.0;
pub const MAX_TEAM: u8 = 6;
/// A full day and night, in seconds.
const DAY_CYCLE: f32 = 480.0;
pub const TIME_MODES: [&str; 4] = ["DAY", "DUSK", "NIGHT", "DAY / NIGHT CYCLE"];
pub const WEATHER_MODES: [&str; 6] = ["MAP'S OWN", "CLEAR", "OVERCAST", "RAIN", "FOG", "RANDOM FRONTS"];
/// Lobby setting steps. A score limit of 0 means none (a time limit or nothing ends the match).
pub const SCORE_LIMITS: [u32; 10] = [0, 10, 15, 20, 25, 30, 40, 50, 75, 100];
/// Time limits in seconds; 0 = none.
pub const TIME_LIMITS: [f32; 7] = [0.0, 180.0, 300.0, 480.0, 600.0, 900.0, 1200.0];
pub const RESPAWNS: [f32; 7] = [1.0, 2.0, 3.0, 4.0, 5.5, 8.0, 10.0];
/// Bot skill: a name and the `Rules::difficulty` it sets.
pub const BOT_SKILLS: [(&str, f32); 4] = [("RECRUIT", 0.75), ("STANDARD", 1.0), ("VETERAN", 1.35), ("NIGHTMARE", 1.8)];
/// How long the host's mode / map pick has to sit still before the lobby reloads on it.
pub const SETTLE: f32 = 0.9;
/// Longest player name.
pub const NAME_MAX: usize = 14;

/// Versus map choices: the random town (any theme), the town in each theme, then the fixed maps.
pub const MAPS: u8 = 1 + Theme::ALL.len() as u8 + (Layout::VERSUS.len() - 1) as u8;

/// Which of the [`MAPS`] these rules play.
pub fn map_choice(rules: &Rules) -> u8 {
    let themes = Theme::ALL.len() as u8;
    match rules.recipe.layout {
        Layout::Town => rules.map.min(themes),
        layout => Layout::VERSUS.iter().position(|l| *l == layout).map_or(0, |i| themes + i as u8),
    }
}

pub fn map_title(choice: u8) -> &'static str {
    let themes = Theme::ALL.len() as u8;
    match choice {
        0 => "RANDOM TOWN",
        c if c <= themes => Theme::ALL[(c - 1) as usize].title(),
        c => Layout::VERSUS[((c - themes) as usize).min(Layout::VERSUS.len() - 1)].title(),
    }
}

/// The world for a map choice. Towns roll their streets from `seed`; fixed maps only take
/// their weather from it.
pub fn map_recipe(choice: u8, seed: u64, biome: Biome) -> WorldRecipe {
    let themes = Theme::ALL.len() as u8;
    // Kept well below the top so the themed search can never run off the end.
    let seed = seed & 0xFFFF_FFFF_FFFF;
    match choice {
        0 => WorldRecipe::town(seed, biome),
        c if c <= themes => WorldRecipe::themed(seed, biome, Theme::ALL[(c - 1) as usize]),
        c => WorldRecipe::preset(Layout::VERSUS[((c - themes) as usize).min(Layout::VERSUS.len() - 1)], biome, Weather::from_seed(seed)),
    }
}

/// The name of the bot skill closest to `difficulty`.
pub fn skill_name(difficulty: f32) -> &'static str {
    BOT_SKILLS[skill_index(difficulty)].0
}

fn skill_index(difficulty: f32) -> usize {
    (0..BOT_SKILLS.len()).min_by(|a, b| (BOT_SKILLS[*a].1 - difficulty).abs().total_cmp(&(BOT_SKILLS[*b].1 - difficulty).abs())).unwrap_or(1)
}

/// Steps along a list of settings from the entry nearest `current`, stopping at the ends.
fn step_value<T: Copy + Into<f64>>(list: &[T], current: T, step: i8) -> T {
    let at = (0..list.len()).min_by(|a, b| (list[*a].into() - current.into()).abs().total_cmp(&(list[*b].into() - current.into()).abs())).unwrap_or(0);
    list[(at as i32 + step as i32).clamp(0, list.len() as i32 - 1) as usize]
}

/// A name as everyone will see it: upper case, letters, digits and a little punctuation,
/// single spaces, at most [`NAME_MAX`] characters.
pub fn clean_name(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.chars().map(|c| c.to_ascii_uppercase()) {
        if out.len() >= NAME_MAX {
            break;
        }
        if c.is_ascii_alphanumeric() || "-_.!?'#".contains(c) {
            out.push(c);
        } else if c.is_whitespace() && !out.is_empty() && !out.ends_with(' ') {
            out.push(' ');
        }
    }
    out.trim_end().to_string()
}

/// `name`, or with a number on the end if someone already has it.
pub fn unique_name(name: &str, taken: &[&str]) -> String {
    if !taken.contains(&name) {
        return name.to_string();
    }
    for n in 2..100 {
        let suffix = n.to_string();
        let base: String = name.chars().take(NAME_MAX - suffix.len()).collect();
        let candidate = format!("{}{suffix}", base.trim_end());
        if !taken.contains(&candidate.as_str()) {
            return candidate;
        }
    }
    name.to_string()
}

/// The modes a versus lobby can switch between.
pub fn versus_modes(modes: &Modes) -> Vec<u8> {
    modes.0.iter().enumerate().filter(|(_, m)| m.playable && (m.rules)(0).pvp).map(|(i, _)| i as u8).collect()
}

/// The rules for switching the lobby to `mode`: that mode's own defaults, on the same map,
/// keeping the host's bot, time-limit choices, with room for every human already here.
pub fn rules_for_mode(modes: &Modes, mode: u8, current: &Rules, humans: [usize; 2]) -> Option<Rules> {
    let def = modes.0.get(mode as usize).filter(|m| m.playable)?;
    let mut rules = (def.rules)(current.recipe.seed);
    rules.mode = mode;
    rules.recipe = current.recipe;
    rules.map = current.map;
    rules.difficulty = current.difficulty;
    rules.time_limit = current.time_limit;
    rules.no_bots = current.no_bots;
    let needed = if rules.hunt { humans[0] + humans[1] } else { humans[0].max(humans[1]) };
    rules.team_size = rules.team_size.max(needed.min(MAX_TEAM as usize) as u8);
    Some(rules)
}

/// Terrorist hunt: how many hostiles hold the map for a squad of `squad`.
pub fn hunt_hostiles(rules: &Rules, squad: u8) -> usize {
    ((14.0 + 6.0 * squad as f32) * rules.difficulty).round().clamp(8.0, 50.0) as usize
}

/// Terrorist hunt: the squad's base is a safe zone this wide (metres from its first
/// spawn). Hostiles never post, wander or chase into it and cannot target anyone in it,
/// so they cannot camp the respawn.
pub const HUNT_SAFE: f32 = 26.0;

/// The middle of the squad's safe zone.
pub fn hunt_base(map: &crate::session::Map) -> Vec2 {
    map.team_spawns[0].first().copied().unwrap_or(map.spawn)
}

/// Respawns the squad shares in a terrorist hunt.
pub const HUNT_LIVES: u16 = 5;

/// A guard post for a hostile: somewhere to stand on the far side of the map from the
/// squad's base - in the streets, in a building, or up on an upper floor.
pub fn hunt_post(map: &crate::session::Map, statics: &oz_core::collide::StaticWorld, rng: &mut SimRng) -> Vec3 {
    let base = map.team_spawns[0].first().copied().unwrap_or(map.spawn);
    let mut fallback = None;
    for _ in 0..80 {
        // Indoors, posts are near the map's waypoints (the rest of the ground is walled off).
        let (x, z) = if let Some(p) = rng.0.pick(&map.waypoints).copied() {
            (p.x + rng.0.range(-1.5, 1.5), p.z + rng.0.range(-1.5, 1.5))
        } else {
            let reach = map.arena * 0.95;
            (rng.0.range(-reach, reach), rng.0.range(-reach, reach))
        };
        if !map.in_bounds(x, z) {
            continue;
        }
        let ground = map.height(x, z);
        let level = rng.0.below(3) as f32;
        let y = crate::player::ground_under(map, statics, x, z, ground + level * 3.2 + 1.0);
        if y < map.water || statics.blocked(x, z, y, 1.8, 0.4) {
            continue;
        }
        let at = Vec3::new(x, y, z);
        if Vec2::new(x, z).distance(base) > HUNT_SAFE + 8.0 {
            return at;
        }
        fallback.get_or_insert(at);
    }
    fallback.unwrap_or_else(|| crate::player::spawn_point(map, 0, Some(1)))
}

pub fn versus(rules: Option<Res<Rules>>) -> bool {
    rules.is_some_and(|rules| rules.pvp)
}

/// Authority: rebuilds the roster and hands out kill credits. Runs in every mode.
pub fn update_roster(
    content: Res<Content>,
    rules: Option<Res<Rules>>,
    mut state: ResMut<MatchState>,
    mut players: Query<(&mut Player, &Health, &PlayerInput, &mut Arsenal, Has<Bot>)>,
    mut fx: EventWriter<FxEvent>,
) {
    let pvp = rules.is_some_and(|r| r.pvp);
    for id in std::mem::take(&mut state.credits) {
        if let Some((mut player, _, _, mut arsenal, _)) = players.iter_mut().find(|(p, ..)| p.id == id) {
            player.kills += 1;
            if pvp {
                player.streak += 1;
                let chosen = player.streaks;
                if let Some(def) = crate::killstreak::STREAKS.iter().find(|s| s.kills == player.streak && chosen.contains(&s.id)) {
                    crate::killstreak::push(&mut player.rewards, def.id);
                    fx.write(FxEvent::Streak { player: id, reward: def.id });
                }
            }
            // Scavengers pull spare rounds off whoever they drop.
            if matches!(crate::player::perk(&content, &player), Some(oz_core::content::PerkEffect::Scavenger)) {
                let index = arsenal.current as usize;
                if let (Some(state), Some(def)) = (arsenal.weapons.get_mut(index), content.weapons.get(index))
                    && !def.infinite_reserve
                {
                    state.reserve = state.reserve.saturating_add(def.mag.max(1));
                }
            }
        }
    }
    let mut roster: Vec<RosterEntry> = players
        .iter()
        .map(|(p, health, input, arsenal, bot)| RosterEntry {
            id: p.id,
            name: p.name.clone(),
            team: p.team,
            bot,
            ready: p.ready || bot,
            alive: health.alive(),
            kills: p.kills,
            deaths: p.deaths,
            ping: p.ping,
            weapon: input.weapon,
            parts: arsenal.weapons.get(input.weapon as usize).map_or([0; oz_core::arsenal::SLOT_COUNT], |w| w.equipped),
        })
        .collect();
    roster.sort_by_key(|entry| (entry.team, entry.bot, entry.id));
    if state.roster != roster {
        state.roster = roster;
    }
}

/// Picks a team for a newly joining human: whichever has fewer humans.
pub fn team_for_newcomer(players: impl Iterator<Item = (u8, bool)>) -> u8 {
    let mut humans = [0; 2];
    for (team, bot) in players {
        if !bot {
            humans[(team as usize).min(1)] += 1;
        }
    }
    if humans[1] < humans[0] { 1 } else { 0 }
}

/// Authority: keeps each team at `team_size` by adding or removing bots.
#[allow(clippy::too_many_arguments)]
pub fn fill_teams(
    rules: Res<Rules>,
    content: Res<Content>,
    map: Res<Map>,
    state: Res<MatchState>,
    mut rng: ResMut<SimRng>,
    mut ids: ResMut<NetIds>,
    mut commands: Commands,
    players: Query<(Entity, &Player, Has<Bot>)>,
    statics: Res<crate::session::Statics>,
) {
    let squad = state.team_size.max(1);
    // Ids handed out this frame are not in the query yet, so track them too.
    let mut taken: Vec<u8> = players.iter().map(|(_, player, _)| player.id).collect();
    for team in 0..2u8 {
        let members: Vec<_> = players.iter().filter(|(_, player, _)| player.team == team).collect();
        let humans = members.iter().filter(|(.., bot)| !bot).count();
        let wanted = if rules.hunt && team == 1 {
            hunt_hostiles(&rules, squad)
        } else if rules.no_bots {
            humans
        } else {
            squad as usize
        };
        if members.len() < wanted {
            if let Some(id) = (FIRST_BOT_ID..=254).find(|id| !taken.contains(id)) {
                taken.push(id);
                let bot = spawn_bot(&mut commands, &mut ids, &content, &rules, &map, &mut rng, id, team);
                if rules.hunt && team == 1 {
                    // A hostile: a sentry at its own post.
                    let post = hunt_post(&map, &statics, &mut rng);
                    let yaw = rng.0.range(0.0, std::f32::consts::TAU);
                    let brain = Bot::sentry(post);
                    commands.entity(bot).insert((brain, Transform::from_translation(post).with_rotation(Quat::from_rotation_y(yaw))));
                }
            }
        } else if members.len() > wanted {
            // Too many: release a bot (humans are never kicked to make room).
            if let Some((bot, ..)) = members.iter().rev().find(|(_, _, is_bot)| *is_bot) {
                commands.entity(*bot).despawn();
            }
        }
    }
}

/// Authority: lobby commands, the countdown, the score limit and the rematch.
#[allow(clippy::too_many_arguments)]
pub fn run_versus(
    time: Res<Time>,
    mut rules: ResMut<Rules>,
    content: Res<Content>,
    map: Res<Map>,
    mut state: ResMut<MatchState>,
    mut orders: EventReader<CommandEvent>,
    mut players: Query<(&mut Player, &mut Health, &mut Transform, &mut Arsenal, &mut PlayerInput, Option<&Bot>)>,
    mut fx: EventWriter<FxEvent>,
    (modes, mut rng, mut relaunch): (Res<Modes>, ResMut<SimRng>, ResMut<Relaunch>),
) {
    let dt = sim_dt(&time);
    // Time of day: fixed, or a cycle that runs through the match.
    state.clock += dt;
    state.daylight = match state.time_mode {
        0 => 1.0,
        1 => 0.45,
        2 => 0.0,
        _ => (0.5 + 0.6 * (state.clock / DAY_CYCLE * std::f32::consts::TAU).cos()).clamp(0.0, 1.0),
    };
    for order in orders.read() {
        let PlayerCommand::Lobby(action) = order.command else {
            continue;
        };
        let host = order.player == 0;
        match action {
            LobbyAction::ToggleReady => {
                if let Some((mut player, ..)) = players.iter_mut().find(|(p, ..)| p.id == order.player) {
                    player.ready = !player.ready;
                }
            }
            LobbyAction::Start if host && state.phase == Phase::Lobby && state.countdown <= 0.0 && state.change_in <= 0.0 && relaunch.0.is_none() => {
                state.countdown = COUNTDOWN;
                fx.write(FxEvent::Sound { sfx: Sfx::UiAccept, at: None });
            }
            LobbyAction::SwitchTeam if state.phase == Phase::Lobby => {
                let humans_on = |team: u8, players: &Query<(&mut Player, &mut Health, &mut Transform, &mut Arsenal, &mut PlayerInput, Option<&Bot>)>| {
                    players.iter().filter(|(p, .., bot)| p.team == team && bot.is_none()).count()
                };
                if let Some(current) = players.iter().find(|(p, ..)| p.id == order.player).map(|(p, ..)| p.team).filter(|_| !rules.hunt) {
                    let other = 1 - current.min(1);
                    if humans_on(other, &players) < state.team_size as usize
                        && let Some((mut player, _, mut transform, ..)) = players.iter_mut().find(|(p, ..)| p.id == order.player)
                    {
                        player.team = other;
                        transform.translation = crate::player::spawn_point(&map, player.id, Some(other));
                    }
                }
            }
            LobbyAction::TimeOfDay(step) if host && state.phase == Phase::Lobby => {
                state.time_mode = (state.time_mode as i32 + step as i32).rem_euclid(TIME_MODES.len() as i32) as u8;
                state.clock = 0.0;
            }
            LobbyAction::Weather(step) if host && state.phase == Phase::Lobby => {
                state.weather_mode = (state.weather_mode as i32 + step as i32).rem_euclid(WEATHER_MODES.len() as i32) as u8;
                state.weather = match state.weather_mode {
                    0 | 5 => rules.recipe.weather,
                    n => oz_core::worldgen::Weather::ALL[(n - 1) as usize],
                };
            }
            LobbyAction::TeamSize(step) if host && state.phase == Phase::Lobby => {
                let size = (state.team_size as i32 + step as i32).clamp(1, MAX_TEAM as i32) as u8;
                state.team_size = size;
                rules.team_size = size;
            }
            // Mode and map: the pick shows at once, and the lobby reloads on it once it settles.
            LobbyAction::Mode(step) if host && state.phase == Phase::Lobby && state.countdown <= 0.0 => {
                let choices = versus_modes(&modes);
                if !choices.is_empty() {
                    let at = choices.iter().position(|m| *m == state.next_mode).unwrap_or(0);
                    state.next_mode = choices[(at as i32 + step as i32).rem_euclid(choices.len() as i32) as usize];
                    state.change_in = SETTLE;
                }
            }
            LobbyAction::Map(step) if host && state.phase == Phase::Lobby && state.countdown <= 0.0 => {
                state.next_map = (state.next_map as i32 + step as i32).rem_euclid(MAPS as i32) as u8;
                state.change_in = SETTLE;
            }
            LobbyAction::NewMap if host && state.phase == Phase::Lobby && state.countdown <= 0.0 => {
                state.reroll = true;
                state.change_in = state.change_in.max(0.2);
            }
            LobbyAction::ScoreLimit(step) if host && state.phase == Phase::Lobby => {
                rules.score_limit = step_value(&SCORE_LIMITS, rules.score_limit, step);
            }
            LobbyAction::TimeLimit(step) if host && state.phase == Phase::Lobby => {
                rules.time_limit = step_value(&TIME_LIMITS, rules.time_limit, step);
            }
            LobbyAction::BotSkill(step) if host && state.phase == Phase::Lobby => {
                let at = (skill_index(rules.difficulty) as i32 + step as i32).clamp(0, BOT_SKILLS.len() as i32 - 1);
                rules.difficulty = BOT_SKILLS[at as usize].1;
            }
            LobbyAction::Bots if host && state.phase == Phase::Lobby => rules.no_bots = !rules.no_bots,
            LobbyAction::Respawn(step) if host && state.phase == Phase::Lobby => {
                rules.respawn_seconds = step_value(&RESPAWNS, rules.respawn_seconds, step);
            }
            _ => {}
        }
    }

    // The host's mode / map pick has settled: reload the lobby on it.
    if state.phase == Phase::Lobby && state.change_in > 0.0 {
        state.change_in -= dt;
        if state.change_in <= 0.0 {
            state.change_in = 0.0;
            let mut humans = [0; 2];
            for (player, .., bot) in &players {
                if bot.is_none() {
                    humans[(player.team as usize).min(1)] += 1;
                }
            }
            let mut next = if state.next_mode != rules.mode { rules_for_mode(&modes, state.next_mode, &rules, humans) } else { Some(rules.clone()) };
            if let Some(next) = &mut next
                && (state.next_map != map_choice(&rules) || state.reroll)
            {
                let seed = rng.0.next_u64();
                // A fresh roll of a fixed map gets new colours instead of new streets.
                let fixed = state.next_map > Theme::ALL.len() as u8;
                let biome = if fixed && state.next_map == map_choice(&rules) { Biome::ALL[(seed % Biome::ALL.len() as u64) as usize] } else { next.recipe.biome };
                next.recipe = map_recipe(state.next_map, seed, biome);
                next.map = if fixed { 0 } else { state.next_map };
            }
            state.reroll = false;
            match next {
                Some(next) if next != *rules => relaunch.0 = Some(next),
                _ => {
                    state.next_mode = rules.mode;
                    state.next_map = map_choice(&rules);
                }
            }
        }
    }

    match state.phase {
        Phase::Lobby if rules.hunt && state.countdown <= 0.0 => {
            state.hostiles = players.iter().filter(|(p, ..)| p.team == 1).count() as u16;
            state.lives = HUNT_LIVES;
        }
        Phase::Lobby if state.countdown > 0.0 => {
            let before = state.countdown.ceil();
            state.countdown -= dt;
            if state.countdown.ceil() < before && state.countdown > 0.0 {
                fx.write(FxEvent::Sound { sfx: Sfx::UiMove, at: None });
            }
            if state.countdown <= 0.0 {
                state.countdown = 0.0;
                state.phase = Phase::Live;
                state.scores = [0, 0];
                state.kills = 0;
                state.clock = 0.0;
                state.timer = 60.0;
                state.lives = HUNT_LIVES;
                state.time_left = rules.time_limit;
                for (mut player, mut health, mut transform, mut arsenal, mut input, bot) in &mut players {
                    player.kills = 0;
                    player.deaths = 0;
                    player.ready = false;
                    respawn(&content, &map, &rules, &state, &mut player, &mut health, &mut transform, &mut arsenal, &mut input);
                    // Hostiles start back at their posts.
                    if let Some(bot) = bot.filter(|b| b.sentry) {
                        transform.translation = bot.post;
                    }
                }
                fx.write(FxEvent::notice(if rules.hunt { "CLEAR THE MAP" } else { "FIGHT" }));
                fx.write(FxEvent::Sound { sfx: Sfx::Horn, at: None });
            }
        }
        Phase::Live if rules.hunt => {
            let hostiles = players.iter().filter(|(p, h, ..)| p.team == 1 && h.alive()).count();
            let posted = players.iter().filter(|(p, ..)| p.team == 1).count();
            let squad_up = players.iter().any(|(p, h, ..)| p.team == 0 && h.alive());
            state.hostiles = hostiles as u16;
            let out_of_time = rules.time_limit > 0.0 && {
                state.time_left -= dt;
                state.time_left <= 0.0
            };
            let outcome = if posted > 0 && hostiles == 0 {
                Some((0, "MAP CLEARED"))
            } else if state.lives == 0 && !squad_up {
                Some((1, "SQUAD LOST"))
            } else if out_of_time {
                Some((1, "OUT OF TIME"))
            } else {
                None
            };
            if let Some((winner, line)) = outcome {
                state.phase = Phase::Won;
                state.winner = winner;
                state.timer = POST_MATCH;
                fx.write(FxEvent::notice(line));
                fx.write(FxEvent::Sound { sfx: if winner == 0 { Sfx::Dawn } else { Sfx::Alarm }, at: None });
            }
        }
        Phase::Live => {
            // Random fronts: every couple of minutes the weather may turn.
            if state.weather_mode == 5 {
                state.timer -= dt;
                if state.timer <= 0.0 {
                    let seed = (state.clock * 1000.0) as u64 ^ (state.scores[0] as u64 * 31) ^ (state.scores[1] as u64 * 97);
                    let next = oz_core::worldgen::Weather::ALL[(seed % 4) as usize];
                    state.timer = 70.0 + (seed % 80) as f32;
                    if next != state.weather {
                        state.weather = next;
                        let line = match next {
                            oz_core::worldgen::Weather::Clear => "THE SKY IS CLEARING",
                            oz_core::worldgen::Weather::Overcast => "CLOUDS ROLLING IN",
                            oz_core::worldgen::Weather::Rain => "RAIN IS COMING DOWN",
                            oz_core::worldgen::Weather::Fog => "FOG IS SETTLING IN",
                        };
                        fx.write(FxEvent::notice(line));
                    }
                }
            }
            // Time up: the higher score wins; level scores are a draw (winner 2).
            if rules.time_limit > 0.0 {
                state.time_left -= dt;
                if state.time_left <= 0.0 {
                    state.time_left = 0.0;
                    state.phase = Phase::Won;
                    state.timer = POST_MATCH;
                    let [teal, coral] = state.scores;
                    state.winner = if teal > coral { 0 } else if coral > teal { 1 } else { 2 };
                    fx.write(FxEvent::notice(match state.winner {
                        0 => "TIME - TEAM TEAL WINS",
                        1 => "TIME - TEAM CORAL WINS",
                        _ => "TIME - IT'S A DRAW",
                    }));
                    fx.write(FxEvent::Sound { sfx: Sfx::Dawn, at: None });
                }
            }
            for team in 0..2 {
                if state.phase != Phase::Live {
                    break;
                }
                if rules.score_limit > 0 && state.scores[team] >= rules.score_limit {
                    state.phase = Phase::Won;
                    state.winner = team as u8;
                    state.timer = POST_MATCH;
                    let name = ["TEAL", "CORAL"][team];
                    fx.write(FxEvent::notice(format!("TEAM {name} WINS")));
                    fx.write(FxEvent::Sound { sfx: Sfx::Dawn, at: None });
                    break;
                }
            }
        }
        Phase::Won => {
            state.timer -= dt;
            if state.timer <= 0.0 && relaunch.0.is_none() {
                // Back to the lobby: the same rules on a freshly built copy of the map, so
                // nothing of the last match (scores, streaks, wreckage, loot, bots) carries over.
                relaunch.0 = Some(rules.clone());
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newcomers_balance_the_humans() {
        assert_eq!(team_for_newcomer([(0, false)].into_iter()), 1);
        assert_eq!(team_for_newcomer([(0, false), (1, false)].into_iter()), 0);
        // Bots do not count: they make room.
        assert_eq!(team_for_newcomer([(0, false), (1, true), (1, true)].into_iter()), 1);
    }
}
