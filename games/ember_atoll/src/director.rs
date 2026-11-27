//! The siege director: the day/night loop that turns the engine's parts into a game.
//!
//! By day the island is quiet and players mine, build and shop. At nightfall
//! machines rise from the sea in waves sized to the day number, the
//! difficulty and the player count. Holding the beacon until every attacker
//! is scrap brings the dawn.
//!
//! The same director runs the pure horde mode: with `day_seconds == 0` the
//! "day" shrinks to a short breather, and with no objective the match is lost
//! only when every player is down at once.

use crate::content::action;
use bevy::prelude::*;
use oz_sim::{net::NetIds, prelude::*};

/// Breather between waves when the mode has no real day phase.
const INTERMISSION: f32 = 12.0;

#[derive(Resource, Default)]
pub struct Siege {
    /// Enemy kinds still to surface tonight.
    pub queue: Vec<u16>,
    spawn_timer: f32,
}

pub struct SiegePlugin;

impl Plugin for SiegePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Siege>()
            .add_systems(
                OnEnter(MatchPhase::Active),
                |mut siege: ResMut<Siege>, rules: Res<Rules>, mut state: ResMut<MatchState>| {
                    *siege = Siege::default();
                    if rules.day_seconds <= 0.0 {
                        state.timer = INTERMISSION;
                    }
                },
            )
            .add_systems(Update, run_director.in_set(SimSet::Logic));
    }
}

/// Builds tonight's attack: spend a threat budget on enemies unlocked so far.
pub fn compose_wave(
    catalog: &Catalog,
    day: u32,
    difficulty: f32,
    players: usize,
    rng: &mut Rng,
) -> Vec<u16> {
    let mut budget =
        (5.0 + 5.0 * day as f32) * difficulty * (1.0 + 0.4 * players.saturating_sub(1) as f32);
    let pool: Vec<(u16, &EnemyDef)> = catalog
        .enemies
        .iter()
        .enumerate()
        .filter(|(_, e)| !e.boss && e.first_wave <= day)
        .map(|(i, e)| (i as u16, e))
        .collect();
    let mut wave = Vec::new();
    while budget >= 1.0 && !pool.is_empty() && wave.len() < 600 {
        let affordable: Vec<_> = pool
            .iter()
            .filter(|(_, e)| e.threat as f32 <= budget)
            .collect();
        let Some((kind, def)) = rng.pick(&affordable) else {
            break;
        };
        wave.push(*kind);
        budget -= def.threat as f32;
    }
    // Every fifth night a boss leads the assault, arriving once the fight is underway.
    if day.is_multiple_of(5)
        && let Some(boss) = catalog.enemies.iter().position(|e| e.boss)
    {
        for _ in 0..(day / 5).min(4) {
            wave.insert(wave.len() / 2, boss as u16);
        }
    }
    wave
}

#[allow(clippy::too_many_arguments)]
fn run_director(
    time: Res<Time>,
    rules: Res<Rules>,
    content: Res<Content>,
    map: Res<Map>,
    mut state: ResMut<MatchState>,
    mut siege: ResMut<Siege>,
    mut rng: ResMut<SimRng>,
    mut ids: ResMut<NetIds>,
    mut commands: Commands,
    mut orders: EventReader<CommandEvent>,
    enemies: Query<(), With<Enemy>>,
    players: Query<&Health, With<Player>>,
    mut ores: Query<&mut OreNode>,
    mut fx: EventWriter<FxEvent>,
) {
    let dt = sim_dt(&time);
    let mut skip_day = false;
    for order in orders.read() {
        match order.command {
            PlayerCommand::Mode(action::CALL_NIGHTFALL) => skip_day = true,
            PlayerCommand::Mode(action::REPAIR_BEACON) => {
                let Some(cost) = content
                    .mode_actions
                    .get(action::REPAIR_BEACON as usize)
                    .map(|a| a.cost)
                else {
                    continue;
                };
                if state.objective_hp >= state.objective_max || state.over() {
                    fx.write(FxEvent::tell(order.player, "BEACON IS AT FULL INTEGRITY"));
                } else if state.stock.pay(&cost) {
                    state.objective_hp =
                        (state.objective_hp + action::REPAIR_AMOUNT).min(state.objective_max);
                    fx.write(FxEvent::Cue {
                        to: order.player,
                        sfx: Sfx::Buy,
                    });
                    fx.write(FxEvent::notice("BEACON REPAIRED"));
                } else {
                    fx.write(FxEvent::tell(order.player, "NOT ENOUGH RESOURCES"));
                    fx.write(FxEvent::Cue {
                        to: order.player,
                        sfx: Sfx::UiDeny,
                    });
                }
            }
            _ => {}
        }
    }

    let alive = enemies.iter().len() as u32;
    match state.phase {
        Phase::Day => {
            state.daylight = (state.daylight + dt * 0.35).min(1.0);
            state.timer -= dt;
            state.enemies_left = 0;
            if state.timer <= 0.0 || skip_day {
                let player_count = players.iter().len().max(1);
                siege.queue = compose_wave(
                    &content,
                    state.day,
                    rules.difficulty,
                    player_count,
                    &mut rng.0,
                );
                siege.spawn_timer = 3.0;
                state.phase = Phase::Night;
                state.timer = 0.0;
                state.enemies_left = siege.queue.len() as u32;
                let boss = state.day.is_multiple_of(5);
                fx.write(FxEvent::notice(if boss {
                    format!("NIGHT {} - SOMETHING HUGE IS COMING", state.day)
                } else {
                    format!("NIGHT {} - THEY RISE FROM THE SEA", state.day)
                }));
                fx.write(FxEvent::Sound {
                    sfx: Sfx::Horn,
                    at: None,
                });
            }
        }
        Phase::Night => {
            state.daylight = (state.daylight - dt * 0.35).max(0.0);
            state.timer += dt;
            siege.spawn_timer -= dt;
            // Surface a few at a time so the wave builds instead of arriving as one blob.
            while siege.spawn_timer <= 0.0 && !siege.queue.is_empty() {
                siege.spawn_timer += (1.3 - 0.07 * state.day as f32).max(0.12);
                let Some(kind) = siege.queue.pop() else { break };
                let Some(landing) = rng.0.pick(&map.landings).copied() else {
                    break;
                };
                let at = landing + Vec2::new(rng.0.range(-3.0, 3.0), rng.0.range(-3.0, 3.0));
                let hp_scale = (1.0 + 0.08 * (state.day - 1) as f32) * rules.difficulty;
                spawn_enemy(
                    &mut commands,
                    &mut ids,
                    &content,
                    kind,
                    Vec3::new(at.x, map.water - 1.2, at.y),
                    hp_scale,
                    &mut rng,
                );
            }
            state.enemies_left = siege.queue.len() as u32 + alive;
            if siege.queue.is_empty() && alive == 0 && state.timer > 4.0 {
                let bonus = 60 + 25 * state.day;
                state.stock.credits += bonus;
                state.day += 1;
                state.phase = Phase::Day;
                state.timer = if rules.day_seconds > 0.0 {
                    rules.day_seconds
                } else {
                    INTERMISSION
                };
                for mut node in &mut ores {
                    node.amount = (node.amount + node.max * 0.35).min(node.max);
                }
                fx.write(FxEvent::notice(format!("DAWN - NIGHT SURVIVED  +${bonus}")));
                fx.write(FxEvent::Sound {
                    sfx: Sfx::Dawn,
                    at: None,
                });
            }
        }
        Phase::Lobby | Phase::Live => return,
        Phase::Won | Phase::Lost => {
            state.daylight = (state.daylight - dt * 0.2).max(0.15);
            return;
        }
    }

    let beacon_lost = state.objective_max > 0.0 && state.objective_hp <= 0.0;
    let squad_wiped = state.objective_max <= 0.0
        && !players.is_empty()
        && players.iter().all(|health| !health.alive());
    if beacon_lost || squad_wiped {
        state.phase = Phase::Lost;
        fx.write(FxEvent::notice(if beacon_lost {
            "THE BEACON HAS FALLEN"
        } else {
            "THE SQUAD HAS FALLEN"
        }));
        fx.write(FxEvent::Sound {
            sfx: Sfx::Alarm,
            at: None,
        });
    }
}
