//! Team-versus plumbing any mode can switch on with `Rules::pvp`:
//! a pre-match lobby, team balancing, bot backfill, the score limit, and
//! returning to the lobby for a rematch. Also keeps [`MatchState::roster`]
//! up to date for every mode, which is where names and scoreboards come from.

use crate::{
    bots::{Bot, FIRST_BOT_ID, spawn_bot},
    net::NetIds,
    player::{Arsenal, Health, Player, PlayerInput, respawn},
    session::{Content, Map},
    sim_dt,
    state::{CommandEvent, FxEvent, LobbyAction, MatchState, Phase, PlayerCommand, RosterEntry, Rules, SimRng},
};
use bevy::prelude::*;
use oz_core::synth::Sfx;

const COUNTDOWN: f32 = 5.0;
const POST_MATCH: f32 = 14.0;
pub const MAX_TEAM: u8 = 6;
/// A full day and night, in seconds.
const DAY_CYCLE: f32 = 480.0;
pub const TIME_MODES: [&str; 4] = ["DAY", "DUSK", "NIGHT", "DAY / NIGHT CYCLE"];
pub const WEATHER_MODES: [&str; 6] = ["MAP'S OWN", "CLEAR", "OVERCAST", "RAIN", "FOG", "RANDOM FRONTS"];

/// Terrorist hunt: how many hostiles hold the map for a squad of `squad`.
pub fn hunt_hostiles(rules: &Rules, squad: u8) -> usize {
    ((14.0 + 6.0 * squad as f32) * rules.difficulty).round().clamp(8.0, 50.0) as usize
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
        if Vec2::new(x, z).distance(base) > 32.0 {
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
        let wanted = if rules.hunt && team == 1 { hunt_hostiles(&rules, squad) } else { squad as usize };
        let members: Vec<_> = players.iter().filter(|(_, player, _)| player.team == team).collect();
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
            LobbyAction::Start if host && state.phase == Phase::Lobby && state.countdown <= 0.0 => {
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
            _ => {}
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
            let outcome = if posted > 0 && hostiles == 0 {
                Some((0, "MAP CLEARED"))
            } else if state.lives == 0 && !squad_up {
                Some((1, "SQUAD LOST"))
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
            for team in 0..2 {
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
            if state.timer <= 0.0 {
                // Back to the lobby with everyone alive at their base.
                state.phase = Phase::Lobby;
                for (mut player, mut health, mut transform, mut arsenal, mut input, _) in &mut players {
                    respawn(&content, &map, &rules, &state, &mut player, &mut health, &mut transform, &mut arsenal, &mut input);
                }
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
