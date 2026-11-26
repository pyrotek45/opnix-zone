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
const POST_MATCH: f32 = 9.0;
pub const MAX_TEAM: u8 = 6;

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
                if let Some(def) = crate::killstreak::STREAKS.iter().find(|s| s.kills == player.streak) {
                    player.rewards |= def.bit;
                    fx.write(FxEvent::Streak { player: id, reward: def.bit });
                }
            }
            // Scavengers pull spare rounds off whoever they drop.
            if matches!(crate::player::perk(&content, &player), Some(oz_core::content::PerkEffect::Scavenger)) {
                let index = arsenal.current as usize;
                if let (Some(state), Some(def)) = (arsenal.weapons.get_mut(index), content.weapons.get(index))
                    && !def.infinite_reserve {
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
) {
    let wanted = state.team_size.max(1) as usize;
    // Ids handed out this frame are not in the query yet, so track them too.
    let mut taken: Vec<u8> = players.iter().map(|(_, player, _)| player.id).collect();
    for team in 0..2u8 {
        let members: Vec<_> = players.iter().filter(|(_, player, _)| player.team == team).collect();
        if members.len() < wanted {
            if let Some(id) = (FIRST_BOT_ID..=254).find(|id| !taken.contains(id)) {
                taken.push(id);
                spawn_bot(&mut commands, &mut ids, &content, &rules, &map, &mut rng, id, team);
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
    mut players: Query<(&mut Player, &mut Health, &mut Transform, &mut Arsenal, &mut PlayerInput, Has<Bot>)>,
    mut fx: EventWriter<FxEvent>,
) {
    let dt = sim_dt(&time);
    state.daylight = 1.0;
    for order in orders.read() {
        let PlayerCommand::Lobby(action) = order.command else { continue };
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
                let humans_on = |team: u8, players: &Query<(&mut Player, &mut Health, &mut Transform, &mut Arsenal, &mut PlayerInput, Has<Bot>)>| {
                    players.iter().filter(|(p, .., bot)| p.team == team && !bot).count()
                };
                if let Some(current) = players.iter().find(|(p, ..)| p.id == order.player).map(|(p, ..)| p.team) {
                    let other = 1 - current.min(1);
                    if humans_on(other, &players) < state.team_size as usize
                        && let Some((mut player, _, mut transform, ..)) = players.iter_mut().find(|(p, ..)| p.id == order.player) {
                            player.team = other;
                            transform.translation = crate::player::spawn_point(&map, player.id, Some(other));
                        }
                }
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
                for (mut player, mut health, mut transform, mut arsenal, mut input, _) in &mut players {
                    player.kills = 0;
                    player.deaths = 0;
                    player.ready = false;
                    respawn(&content, &map, &rules, &state, &mut player, &mut health, &mut transform, &mut arsenal, &mut input);
                }
                fx.write(FxEvent::notice("FIGHT"));
                fx.write(FxEvent::Sound { sfx: Sfx::Horn, at: None });
            }
        }
        Phase::Live => {
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

