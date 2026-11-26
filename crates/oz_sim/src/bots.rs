//! AI-controlled players.
//!
//! A bot is an ordinary player entity plus a [`Bot`] brain. The brain only
//! ever writes [`PlayerInput`] - the same struct a keyboard or a remote client
//! fills in - so bots move, shoot, reload and die through exactly the same
//! code as humans. Any mode can use them to fill empty slots.

use crate::{
    combat::Solid,
    net::NetIds,
    player::{EYE, Health, Loadout, Player, PlayerInput, apply_loadout, buttons, can_stand},
    session::{Content, Map, player_bundle},
    sim_dt,
    state::{Rules, SimRng},
    weapons::obstruction,
};
use bevy::prelude::*;
use std::f32::consts::{PI, TAU};

#[derive(Component, Default)]
pub struct Bot {
    think: f32,
    target: Option<Entity>,
    goal: Vec2,
    /// Seconds spent pushing against something.
    stuck: f32,
    last: Vec3,
    /// Preferred side to dodge around obstacles (+1 / -1).
    side: f32,
    strafe: f32,
    strafe_time: f32,
    aim_error: Vec2,
}

/// First player id used by bots; humans stay below it.
pub const FIRST_BOT_ID: u8 = 200;
const NAMES: [&str; 12] = ["VEGA", "ONYX", "RIGGS", "HALO", "DUSK", "MOTH", "KITE", "BRASS", "ECHO", "FLINT", "NOVA", "WREN"];

/// Spawns a bot on `team` with a random class. `id` must have the team's parity.
pub fn spawn_bot(commands: &mut Commands, ids: &mut NetIds, content: &Content, rules: &Rules, map: &Map, rng: &mut SimRng, id: u8) -> Entity {
    let name = format!("{} [BOT]", NAMES[id as usize % NAMES.len()]);
    let entity = commands.spawn((player_bundle(content, rules, map, id, name), ids.next(), Bot { side: 1.0, strafe: 1.0, ..default() })).id();
    // Give it a real class: any proper gun as primary, the first sidearm as backup.
    let guns: Vec<u8> = (0..content.weapons.len() as u8).filter(|i| content.weapons[*i as usize].mag > 0).collect();
    if guns.len() >= 2 {
        let primary = guns[1 + rng.0.below(guns.len() - 1)];
        let loadout = Loadout { primary, secondary: guns[0], perk: rng.0.below(content.perks.len().max(1)) as u8, gear: rng.0.below(content.gear.len().max(1)) as u8 };
        let content = content.clone();
        commands.entity(entity).queue(move |mut entity: EntityWorldMut| {
            let Some(mut player) = entity.take::<Player>() else { return };
            if let Some(mut arsenal) = entity.get_mut::<crate::player::Arsenal>() {
                apply_loadout(&content, &mut player, &mut arsenal, loadout);
            }
            let weapon = entity.get::<crate::player::Arsenal>().map_or(0, |arsenal| arsenal.current);
            if let Some(mut input) = entity.get_mut::<PlayerInput>() {
                input.weapon = weapon;
            }
            entity.insert(player);
        });
    }
    entity
}

fn yaw_to(dir: Vec2) -> f32 {
    f32::atan2(-dir.x, -dir.y)
}

fn turn(current: f32, wanted: f32, max_step: f32) -> f32 {
    let diff = (wanted - current + PI).rem_euclid(TAU) - PI;
    current + diff.clamp(-max_step, max_step)
}

pub fn drive_bots(
    time: Res<Time>,
    map: Res<Map>,
    rules: Res<Rules>,
    mut rng: ResMut<SimRng>,
    mut bots: Query<(&mut Bot, &Player, &Transform, &mut PlayerInput, &Health)>,
    everyone: Query<(Entity, &Player, &Transform, &Health)>,
    solids: Query<(&Transform, &Solid), Without<Player>>,
) {
    let dt = sim_dt(&time);
    for (mut bot, me, transform, mut input, health) in &mut bots {
        if !health.alive() {
            input.buttons = 0;
            input.movement = Vec2::ZERO;
            bot.target = None;
            continue;
        }
        let feet = transform.translation;
        let eye = feet + Vec3::Y * EYE;
        let moved = feet.distance(bot.last) / dt.max(1e-4);
        bot.last = feet;

        bot.think -= dt;
        if bot.think <= 0.0 {
            bot.think = rng.0.range(0.2, 0.4);
            // Look for the nearest enemy we can actually see.
            bot.target = everyone
                .iter()
                .filter(|(_, other, _, other_health)| other.team != me.team && other_health.alive())
                .map(|(entity, _, t, _)| (entity, t.translation + Vec3::Y * 1.1))
                .filter(|(_, chest)| {
                    let distance = chest.distance(eye);
                    distance < 75.0 && obstruction(&map, &solids, eye, (*chest - eye) / distance.max(1e-3), distance) >= distance - 0.6
                })
                .min_by(|a, b| a.1.distance_squared(eye).total_cmp(&b.1.distance_squared(eye)))
                .map(|(entity, _)| entity);
            let miss = 0.075 / rules.difficulty.max(0.25);
            bot.aim_error = Vec2::new(rng.0.range(-miss, miss), rng.0.range(-miss, miss) * 0.6);
            if bot.target.is_none() && (bot.goal == Vec2::ZERO || bot.goal.distance(feet.xz()) < 5.0 || bot.stuck > 1.5) {
                // Hunt: head for where an enemy is, or sweep through the middle of the map.
                let enemies: Vec<Vec2> = everyone.iter().filter(|(_, other, _, h)| other.team != me.team && h.alive()).map(|(_, _, t, _)| t.translation.xz()).collect();
                bot.goal = match rng.0.pick(&enemies) {
                    Some(enemy) if rng.0.chance(0.7) => *enemy,
                    _ => Vec2::new(rng.0.range(-30.0, 30.0), rng.0.range(-40.0, 40.0)),
                };
                bot.stuck = 0.0;
            }
        }
        bot.strafe_time -= dt;
        if bot.strafe_time <= 0.0 {
            bot.strafe_time = rng.0.range(0.6, 1.5);
            bot.strafe = -bot.strafe;
        }

        let target = bot.target.and_then(|entity| everyone.get(entity).ok()).filter(|(_, _, _, h)| h.alive());
        let mut held = 0;
        let wish: Vec2;
        if let Some((_, _, enemy, _)) = target {
            // Fight: track the target, strafe, keep a comfortable range.
            let to = enemy.translation + Vec3::Y * 1.1 - eye;
            let distance = to.length().max(1e-3);
            let wanted_yaw = yaw_to(to.xz()) + bot.aim_error.x;
            let wanted_pitch = (to.y / distance).asin() + bot.aim_error.y;
            input.yaw = turn(input.yaw, wanted_yaw, 7.0 * dt);
            input.pitch += (wanted_pitch - input.pitch).clamp(-5.0 * dt, 5.0 * dt);
            let off_target = ((wanted_yaw - input.yaw + PI).rem_euclid(TAU) - PI).abs();
            // Tap the trigger every other frame so semi-automatic weapons fire too.
            if off_target < 0.09 && !input.held(buttons::FIRE) {
                held |= buttons::FIRE;
            }
            let forward = if distance > 20.0 { 1.0 } else if distance < 7.0 { -1.0 } else { 0.0 };
            let facing = to.xz() / to.xz().length().max(1e-3);
            wish = facing * forward + facing.perp() * bot.strafe;
            bot.goal = enemy.translation.xz();
        } else {
            // Travel: steer for the goal, sliding around whatever is in the way.
            let to_goal = (bot.goal - feet.xz()).normalize_or_zero();
            let mut heading = to_goal;
            for angle in [0.0f32, 0.8, 1.57, 2.4, -0.8, -1.57, -2.4] {
                let probe = Vec2::from_angle(angle * bot.side).rotate(to_goal);
                if can_stand(&map, feet.x + probe.x * 1.5, feet.z + probe.y * 1.5, feet.y, &solids) {
                    heading = probe;
                    break;
                }
            }
            wish = heading;
            input.yaw = turn(input.yaw, yaw_to(heading), 5.0 * dt);
            input.pitch *= 1.0 - (4.0 * dt).min(1.0);
            held |= buttons::SPRINT;
        }

        // Stuck against geometry: try the other way round and jump.
        if wish != Vec2::ZERO && moved < 0.6 {
            bot.stuck += dt;
            if bot.stuck > 0.5 {
                bot.side = -bot.side;
                held |= buttons::JUMP;
            }
        } else {
            bot.stuck = (bot.stuck - dt).max(0.0);
        }

        // World-space wish -> stick input relative to where the bot is looking.
        let forward = Vec2::new(-input.yaw.sin(), -input.yaw.cos());
        let right = Vec2::new(input.yaw.cos(), -input.yaw.sin());
        input.movement = Vec2::new(wish.dot(right), wish.dot(forward)).clamp_length_max(1.0);
        input.buttons = held;
    }
}
