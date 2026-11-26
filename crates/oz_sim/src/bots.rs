//! AI-controlled players.
//!
//! A bot is an ordinary player entity plus a [`Bot`] brain. The brain only
//! ever writes [`PlayerInput`] - the same struct a keyboard or a remote client
//! fills in - so bots move, crouch, shoot, reload and die through exactly the
//! same code as humans. They travel with the [`Nav`] pathfinder and fight
//! with ranges that suit the weapon in their hands.

use crate::{
    combat::Solid,
    nav::{Nav, NavBudget, NavScratch},
    net::NetIds,
    player::{Arsenal, Health, Loadout, Player, PlayerInput, apply_loadout, buttons, stance},
    session::{Content, Map, Statics, player_bundle},
    sim_dt,
    state::{MatchState, Phase, Rules, SimRng},
    structures::Structure,
    weapons::obstruction,
};
use bevy::prelude::*;
use oz_core::arsenal::{GunClass, SLOT_COUNT, options, slot};
use std::f32::consts::{PI, TAU};

#[derive(Component, Default)]
pub struct Bot {
    think: f32,
    target: Option<Entity>,
    /// Where the bot wants to go, and the path it is following there.
    goal: Vec2,
    path: Vec<Vec2>,
    repath: f32,
    stuck: f32,
    last: Vec3,
    strafe: f32,
    strafe_time: f32,
    aim_error: Vec2,
    /// Seconds the current target has been in sight (reaction time).
    seen: f32,
    crouch_at_range: bool,
    /// Which way to lean to see the current target around cover.
    lean: i8,
}

/// First player id used by bots; humans stay below it.
pub const FIRST_BOT_ID: u8 = 200;
const NAMES: [&str; 12] = ["VEGA", "ONYX", "RIGGS", "HALO", "DUSK", "MOTH", "KITE", "BRASS", "ECHO", "FLINT", "NOVA", "WREN"];

/// Spawns a bot on `team` with a random class and random attachments.
pub fn spawn_bot(commands: &mut Commands, ids: &mut NetIds, content: &Content, rules: &Rules, map: &Map, rng: &mut SimRng, id: u8, team: u8) -> Entity {
    let name = format!("{} [BOT]", NAMES[id as usize % NAMES.len()]);
    let brain = Bot { strafe: 1.0, crouch_at_range: rng.0.chance(0.6), ..default() };
    let entity = commands.spawn((player_bundle(content, rules, map, id, name, team), ids.next(), brain)).id();
    let guns: Vec<u8> = (0..content.weapons.len() as u8).filter(|i| content.weapons[*i as usize].frame.is_some()).collect();
    if guns.len() < 2 {
        return entity;
    }
    let primary = guns[1 + rng.0.below(guns.len() - 1)];
    let loadout = Loadout { primary, secondary: guns[0], perk: rng.0.below(content.perks.len().max(1)) as u8, gear: rng.0.below(content.gear.len().max(1)) as u8 };
    // Kit the primary out: each slot has an even chance of a random part that fits.
    let mut parts = content.weapons[primary as usize].defaults;
    for (slot_index, part) in parts.iter_mut().enumerate().take(SLOT_COUNT) {
        let choices = options(&content.weapons[primary as usize], &content.attachments, slot_index);
        if rng.0.chance(0.5) && !choices.is_empty() && !(slot_index == slot::OPTIC && *part != 0) {
            *part = choices[rng.0.below(choices.len())].0 as u8;
        }
    }
    let content = content.clone();
    commands.entity(entity).queue(move |mut entity: EntityWorldMut| {
        let Some(mut player) = entity.take::<Player>() else { return };
        if let Some(mut arsenal) = entity.get_mut::<Arsenal>() {
            if let Some(weapon) = arsenal.weapons.get_mut(primary as usize) {
                weapon.equipped = parts;
            }
            apply_loadout(&content, &mut player, &mut arsenal, loadout);
        }
        let weapon = entity.get::<Arsenal>().map_or(0, |arsenal| arsenal.current);
        if let Some(mut input) = entity.get_mut::<PlayerInput>() {
            input.weapon = weapon;
        }
        entity.insert(player);
    });
    entity
}

fn yaw_to(dir: Vec2) -> f32 {
    f32::atan2(-dir.x, -dir.y)
}

fn turn(current: f32, wanted: f32, max_step: f32) -> f32 {
    let diff = (wanted - current + PI).rem_euclid(TAU) - PI;
    current + diff.clamp(-max_step, max_step)
}

/// Preferred fighting distance (min, max) for the weapon class.
fn engagement(class: GunClass) -> (f32, f32) {
    match class {
        GunClass::Shotgun => (2.0, 8.0),
        GunClass::Smg | GunClass::Pistol => (5.0, 16.0),
        GunClass::Sniper => (25.0, 70.0),
        GunClass::Launcher => (14.0, 35.0),
        _ => (9.0, 28.0),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn drive_bots(
    time: Res<Time>,
    map: Res<Map>,
    statics: Res<Statics>,
    nav: Option<Res<Nav>>,
    content: Res<Content>,
    rules: Res<Rules>,
    state: Res<MatchState>,
    mut scratch: ResMut<NavScratch>,
    mut budget: ResMut<NavBudget>,
    mut rng: ResMut<SimRng>,
    mut players: Query<(Entity, Option<&mut Bot>, &Player, &Transform, &mut PlayerInput, &Health, &Arsenal, Option<&mut crate::combat::Stunned>)>,
    solids: Query<(&Transform, &Solid), (With<Structure>, Without<Player>)>,
    smokes: Query<(&Transform, &crate::combat::SmokeCloud), Without<Player>>,
) {
    let dt = sim_dt(&time);
    // Everyone a bot might fight, humans and bots alike.
    let fighters: Vec<(Entity, u8, Vec3, u8)> = players
        .iter()
        .filter(|(.., health, _, _)| health.alive())
        .map(|(entity, _, player, transform, input, ..)| (entity, player.team, transform.translation, input.stance))
        .collect();

    for (_, bot, me, transform, mut input, health, arsenal, stunned) in &mut players {
        let Some(mut bot) = bot else { continue };
        // Flashed: stumble about blind until it wears off.
        if let Some(mut stunned) = stunned
            && stunned.0 > 0.0 {
                stunned.0 -= dt;
                input.buttons = 0;
                input.movement = Vec2::new(bot.strafe * 0.4, -0.3);
                input.yaw += bot.strafe * dt * 1.5;
                bot.target = None;
                bot.seen = 0.0;
                continue;
            }
        if !health.alive() || state.phase == Phase::Lobby || state.over() {
            input.buttons = 0;
            input.movement = Vec2::ZERO;
            bot.target = None;
            bot.path.clear();
            continue;
        }
        let feet = transform.translation;
        let eye = feet + Vec3::Y * input.eye();
        let moved = feet.distance(bot.last) / dt.max(1e-4);
        bot.last = feet;
        let class = content.weapons.get(arsenal.current as usize).map_or(GunClass::Rifle, |w| w.class);
        let (near, far) = engagement(class);

        bot.think -= dt;
        if bot.think <= 0.0 {
            bot.think = rng.0.range(0.2, 0.35);
            // The nearest enemy we can actually see, upright or leaning out of cover.
            let upright = PlayerInput { lean: 0, ..*input };
            let sees = |from: Vec3, chest: Vec3| {
                let distance = chest.distance(from);
                distance < 85.0
                    && obstruction(&map, &statics, &solids, from, (chest - from) / distance.max(1e-3), distance) >= distance - 0.6
                    && !crate::combat::smoke_between(smokes, from, chest)
            };
            let found = fighters
                .iter()
                .filter(|(_, team, ..)| *team != me.team)
                .filter_map(|(entity, _, at, enemy_stance)| {
                    let chest = *at + Vec3::Y * (stance::eye(*enemy_stance) - 0.4);
                    // Face the target before testing a lean, so the lean goes the right way.
                    let facing = PlayerInput { yaw: yaw_to((chest - eye).xz()), ..upright };
                    [0i8, -1, 1]
                        .into_iter()
                        .find(|lean| sees(crate::player::eye_point(&statics, feet, &PlayerInput { lean: *lean, ..facing }), chest))
                        .map(|lean| (*entity, chest, lean))
                })
                .min_by(|a, b| a.1.distance_squared(eye).total_cmp(&b.1.distance_squared(eye)));
            let target = found.map(|(entity, ..)| entity);
            bot.lean = found.map_or(0, |(.., lean)| lean);
            if target != bot.target {
                bot.seen = 0.0;
            }
            bot.target = target;
            let miss = 0.07 / rules.difficulty.max(0.25);
            bot.aim_error = Vec2::new(rng.0.range(-miss, miss), rng.0.range(-miss, miss) * 0.6);
            if bot.target.is_none() && (bot.path.is_empty() || bot.goal.distance(feet.xz()) < 4.0 || bot.stuck > 1.5) {
                // Hunt: head for an enemy's last position, or sweep through the town.
                let enemies: Vec<Vec2> = fighters.iter().filter(|(_, team, ..)| *team != me.team).map(|(_, _, at, _)| at.xz()).collect();
                bot.goal = match rng.0.pick(&enemies) {
                    Some(enemy) if rng.0.chance(0.65) => *enemy,
                    _ => Vec2::new(rng.0.range(-40.0, 40.0), rng.0.range(-40.0, 40.0)),
                };
                bot.path.clear();
                bot.stuck = 0.0;
            }
        }
        bot.strafe_time -= dt;
        if bot.strafe_time <= 0.0 {
            bot.strafe_time = rng.0.range(0.6, 1.5);
            bot.strafe = -bot.strafe;
        }

        let target = bot.target.and_then(|entity| fighters.iter().find(|(e, ..)| *e == entity).copied());
        input.lean = if target.is_some() { bot.lean } else { 0 };
        let mut held = 0;
        let mut wanted_stance = stance::STAND;
        let mut wish = Vec2::ZERO;
        if let Some((_, _, enemy, enemy_stance)) = target {
            // Fight: track the target, strafe, hold a comfortable range.
            bot.seen += dt;
            let to = enemy + Vec3::Y * (stance::eye(enemy_stance) - 0.45) - crate::player::eye_point(&statics, feet, &input);
            let distance = to.length().max(1e-3);
            let wanted_yaw = yaw_to(to.xz()) + bot.aim_error.x;
            let wanted_pitch = (to.y / distance).asin() + bot.aim_error.y;
            input.yaw = turn(input.yaw, wanted_yaw, 6.5 * dt);
            input.pitch += (wanted_pitch - input.pitch).clamp(-5.0 * dt, 5.0 * dt);
            let off_target = ((wanted_yaw - input.yaw + PI).rem_euclid(TAU) - PI).abs();
            let reacted = bot.seen > 0.35 / rules.difficulty.max(0.25);
            // Tap the trigger every other frame so semi-automatic weapons fire too.
            if reacted && off_target < 0.08 && !input.held(buttons::FIRE) {
                held |= buttons::FIRE;
            }
            if distance > 12.0 && class != GunClass::Shotgun {
                held |= buttons::AIM;
            }
            let facing = to.xz() / to.xz().length().max(1e-3);
            let forward = if distance > far { 1.0 } else if distance < near { -1.0 } else { 0.0 };
            wish = facing * forward + facing.perp() * bot.strafe * if class == GunClass::Sniper { 0.3 } else { 1.0 };
            if bot.lean != 0 {
                // Peeking: hold the corner instead of strafing out of it.
                wish *= 0.15;
            }
            // Hunker down for long shots.
            if bot.crouch_at_range && distance > 18.0 {
                wanted_stance = if class == GunClass::Sniper && distance > 35.0 { stance::PRONE } else { stance::CROUCH };
                wish *= 0.4;
            }
            bot.goal = enemy.xz();
            bot.path.clear();
        } else if let Some(nav) = nav.as_deref() {
            // Travel along a path; search again when it runs out or goes stale.
            bot.repath -= dt;
            if (bot.path.is_empty() || bot.repath <= 0.0) && budget.0 > 0 {
                budget.0 -= 1;
                bot.repath = 3.0;
                bot.path = nav.path(&mut scratch, feet.xz(), bot.goal).unwrap_or_default();
            }
            while bot.path.first().is_some_and(|p| p.distance(feet.xz()) < 0.9) {
                bot.path.remove(0);
            }
            if let Some(next) = bot.path.first() {
                wish = (*next - feet.xz()).normalize_or_zero();
                input.yaw = turn(input.yaw, yaw_to(wish), 6.0 * dt);
                input.pitch *= 1.0 - (4.0 * dt).min(1.0);
                held |= buttons::SPRINT;
            }
        }

        // Stuck against something: jump, and look for a fresh route.
        if wish != Vec2::ZERO && moved < 0.5 {
            bot.stuck += dt;
            if bot.stuck > 0.6 {
                held |= buttons::JUMP;
                bot.repath = 0.0;
                bot.strafe = -bot.strafe;
            }
        } else {
            bot.stuck = (bot.stuck - dt).max(0.0);
        }
        // Stand up to jump or sprint.
        if held & (buttons::JUMP | buttons::SPRINT) != 0 {
            wanted_stance = stance::STAND;
        }
        input.stance = wanted_stance;

        // World-space wish -> stick input relative to where the bot is looking.
        let forward = Vec2::new(-input.yaw.sin(), -input.yaw.cos());
        let right = Vec2::new(input.yaw.cos(), -input.yaw.sin());
        input.movement = Vec2::new(wish.dot(right), wish.dot(forward)).clamp_length_max(1.0);
        input.buttons = held;
    }
}
