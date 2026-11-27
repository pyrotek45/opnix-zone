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
    goal: Vec3,
    path: Vec<Vec3>,
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
    /// Terrorist hunt: a sentry holds a post, sees only what is in front of it, and
    /// reacts to what it hears instead of always knowing where everyone is.
    pub sentry: bool,
    pub post: Vec3,
    /// Seconds of high alert left (wider view, further sight, running).
    alert: f32,
    /// Where the last noise worth checking came from.
    heard: Option<Vec3>,
    /// Firing back blind at a gunshot it heard: how long, and at what.
    suppress: f32,
    suppress_at: Vec3,
    patrol: f32,
    last_hp: f32,
}

impl Bot {
    /// A terrorist-hunt hostile holding `post`.
    pub fn sentry(post: Vec3) -> Self {
        Self { strafe: 1.0, sentry: true, post, crouch_at_range: true, ..default() }
    }
}

/// Something a bot might hear this frame: gunfire, footsteps, a body falling, a shout.
#[derive(Clone, Copy, Debug)]
pub struct Noise {
    pub at: Vec3,
    /// Heard by anyone within this distance.
    pub radius: f32,
    /// Who made it. Enemies of this team react to it...
    pub team: u8,
    /// ...unless it is a shout: then this team's own bots hear it (a callout of where an enemy is).
    pub shout: bool,
}

/// Noises made since the bots last listened (authority only).
#[derive(Resource, Default)]
pub struct Noises(pub Vec<Noise>);

/// Does a line of sight pass through leaves? Leaves around the looker do not count:
/// you can see out of the bush you are hiding in.
pub fn leaves_between(foliage: &[(Vec3, f32)], from: Vec3, to: Vec3) -> bool {
    let span = to - from;
    let length = span.length();
    if length < 1e-3 {
        return false;
    }
    let dir = span / length;
    foliage.iter().any(|(center, radius)| center.distance(from) > *radius && crate::combat::ray_sphere(from, dir, *center, *radius).is_some_and(|d| d < length))
}

/// How far footsteps carry for a given stance and speed.
pub fn footstep_radius(stance_now: u8, speed: f32) -> f32 {
    if speed < 1.0 {
        return 0.0;
    }
    match stance_now {
        stance::PRONE => 1.5,
        stance::CROUCH => 2.5,
        _ if speed > 6.5 => 15.0,
        _ => 7.0,
    }
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
    let loadout = Loadout { primary, secondary: guns[0], perk: rng.0.below(content.perks.len().max(1)) as u8, gear: rng.0.below(content.gear.len().max(1)) as u8, parts: None, streaks: [0; 3] };
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
        let Some(mut player) = entity.take::<Player>() else {
            return;
        };
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
    mut players: Query<(Entity, Option<&mut Bot>, &Player, &Transform, &mut PlayerInput, &Health, &Arsenal, Option<&mut crate::combat::Stunned>, &crate::player::Mover)>,
    mut noises: ResMut<Noises>,
    solids: Query<(&Transform, &Solid), (Or<(With<Structure>, With<crate::breakables::Door>, With<crate::breakables::ClutterPiece>)>, Without<Player>)>,
    (smokes, pickups): (Query<(&Transform, &crate::combat::SmokeCloud), Without<Player>>, Query<(&Transform, &crate::loot::Pickup), Without<Player>>),
    mut leaves: Local<(u64, Vec<(Vec3, f32)>)>,
) {
    let dt = sim_dt(&time);
    // Tree crowns and bushes hide whoever is behind (or inside) them.
    let key = map.recipe.seed ^ map.props.len() as u64;
    if leaves.0 != key || leaves.1.is_empty() {
        *leaves = (key, map.foliage());
    }
    let foliage = &leaves.1;
    // Everyone a bot might fight, humans and bots alike.
    let fighters: Vec<(Entity, u8, Vec3, u8)> =
        players.iter().filter(|(.., health, _, _, _)| health.alive()).map(|(entity, _, player, transform, input, ..)| (entity, player.team, transform.translation, input.stance)).collect();
    // Living humans, for squadmates to stay with.
    let humans: Vec<(u8, Vec3)> = players.iter().filter(|(_, bot, .., health, _, _, _)| bot.is_none() && health.alive()).map(|(_, _, p, t, ..)| (p.team, t.translation)).collect();
    // What can be heard this frame: everything made since last frame, plus everyone's footsteps.
    let mut heard_now = std::mem::take(&mut noises.0);
    for (_, _, player, transform, input, health, _, _, mover) in &players {
        let mut radius = footstep_radius(input.stance, mover.speed);
        // Pushing through a bush rustles: it carries much further than a footstep.
        let at = transform.translation;
        if radius > 0.0 && map.props.iter().any(|p| matches!(p.kind, oz_core::worldgen::PropKind::Bush) && (p.pos.xz() - at.xz()).length() < 0.75 * p.scale + 0.25 && (p.pos.y - at.y).abs() < 1.2) {
            radius = radius.max(18.0);
        }
        if health.alive() && radius > 0.0 {
            heard_now.push(Noise { at: transform.translation, radius, team: player.team, shout: false });
        }
    }
    let mut shouts: Vec<Noise> = Vec::new();

    for (_, bot, me, transform, mut input, health, arsenal, stunned, _) in &mut players {
        let Some(mut bot) = bot else { continue };
        if bot.sentry && health.alive() {
            bot.alert = (bot.alert - dt).max(0.0);
            bot.suppress -= dt;
            let feet = transform.translation;
            for noise in &heard_now {
                let ours = noise.team == me.team;
                if noise.shout != ours || noise.at.distance(feet) > noise.radius {
                    continue;
                }
                bot.heard = Some(noise.at);
                bot.alert = 15.0;
                // Gunfire close by: fire back toward it if there is a line to it.
                let distance = noise.at.distance(feet);
                if !noise.shout && noise.radius > 40.0 && distance < 45.0 && bot.target.is_none() {
                    let eye = feet + Vec3::Y * input.eye();
                    let dir = (noise.at - eye) / distance.max(1e-3);
                    if obstruction(&map, &statics, &solids, eye, dir, distance) > distance * 0.85 {
                        bot.suppress = rng.0.range(0.6, 1.2);
                        bot.suppress_at = noise.at + Vec3::Y * 0.2;
                    }
                }
            }
            // Hit by someone it never saw: on edge, looking everywhere.
            if health.hp < bot.last_hp - 0.5 {
                bot.alert = 15.0;
            }
            bot.last_hp = health.hp;
        }
        // Flashed: stumble about blind until it wears off.
        if let Some(mut stunned) = stunned
            && stunned.0 > 0.0
        {
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
                    && (distance < 4.0 || !leaves_between(foliage, from, chest))
            };
            let (sentry, alert) = (bot.sentry, bot.alert > 0.0);
            let facing_dir = Vec2::new(-input.yaw.sin(), -input.yaw.cos());
            // Sentries only notice what is in front of them, and low, still shapes are hard to spot.
            let noticed = |at: Vec3, enemy_stance: u8| {
                if !sentry {
                    return true;
                }
                let to = at - feet;
                let distance = to.length();
                let range = if alert { 70.0 } else { 40.0 * [1.0, 0.7, 0.45][(enemy_stance as usize).min(2)] };
                let cone = if alert { -0.3 } else { 0.4 };
                distance < 2.5 || (distance < range && to.xz().normalize_or_zero().dot(facing_dir) > cone)
            };
            let found = fighters
                .iter()
                .filter(|(_, team, ..)| *team != me.team)
                .filter(|(_, _, at, enemy_stance)| noticed(*at, *enemy_stance))
                .filter_map(|(entity, _, at, enemy_stance)| {
                    let chest = *at + Vec3::Y * (stance::eye(*enemy_stance) - 0.4);
                    // Face the target before testing a lean, so the lean goes the right way.
                    let facing = PlayerInput { yaw: yaw_to((chest - eye).xz()), ..upright };
                    [0i8, -1, 1].into_iter().find(|lean| sees(crate::player::eye_point(&statics, feet, &PlayerInput { lean: *lean, ..facing }), chest)).map(|lean| (*entity, chest, lean))
                })
                .min_by(|a, b| a.1.distance_squared(eye).total_cmp(&b.1.distance_squared(eye)));
            let target = found.map(|(entity, ..)| entity);
            bot.lean = found.map_or(0, |(.., lean)| lean);
            if target != bot.target {
                // Caught off guard, a sentry takes a moment to react.
                bot.seen = if bot.sentry && bot.alert <= 0.0 { -0.45 } else { 0.0 };
            }
            if bot.sentry
                && let Some((_, chest, _)) = found
            {
                bot.alert = 15.0;
                bot.heard = Some(chest);
                shouts.push(Noise { at: chest, radius: 22.0, team: me.team, shout: true });
            }
            bot.target = target;
            let miss = 0.07 / rules.difficulty.max(0.25);
            bot.aim_error = Vec2::new(rng.0.range(-miss, miss), rng.0.range(-miss, miss) * 0.6);
            if bot.sentry && bot.target.is_none() {
                bot.patrol -= 0.3;
                if let Some(at) = bot.heard {
                    // Go and see what that was.
                    if at.distance(feet) < 3.0 {
                        bot.heard = None;
                    } else if bot.goal.distance(at) > 2.0 {
                        bot.goal = at;
                        bot.path.clear();
                    }
                } else if bot.patrol <= 0.0 || bot.stuck > 1.5 {
                    // Mill about near the post.
                    bot.patrol = rng.0.range(8.0, 18.0);
                    let wander = Vec2::from_angle(rng.0.range(0.0, TAU)) * rng.0.range(0.0, 6.0);
                    bot.goal = bot.post + Vec3::new(wander.x, 0.0, wander.y);
                    bot.path.clear();
                    bot.stuck = 0.0;
                }
            } else if rules.hunt && me.team == 0 && bot.target.is_none() {
                // Squadmates stay with the humans.
                if let Some((_, lead)) = humans.iter().filter(|(team, _)| *team == me.team).min_by(|a, b| a.1.distance_squared(feet).total_cmp(&b.1.distance_squared(feet))) {
                    if lead.distance(feet) > 5.0 && (bot.goal.distance(*lead) > 3.0 || bot.path.is_empty()) {
                        bot.goal = *lead;
                        bot.path.clear();
                    }
                } else if bot.path.is_empty() || bot.goal.distance(feet) < 4.0 {
                    let (x, z) = (rng.0.range(-40.0, 40.0), rng.0.range(-40.0, 40.0));
                    bot.goal = Vec3::new(x, map.height(x, z), z);
                    bot.path.clear();
                }
            } else if rules.loot
                && bot.target.is_none()
                && (0..arsenal.weapons.len()).filter(|i| arsenal.owns(*i)).count() < 2
                && let Some(gun) = crate::loot::nearest_gun(feet, pickups.iter())
            {
                // Scavenge: a pistol is not enough - go and get a real gun.
                if bot.goal.distance(gun) > 1.0 || bot.path.is_empty() {
                    bot.goal = gun;
                    bot.path.clear();
                    bot.stuck = 0.0;
                }
            } else if bot.target.is_none() && (bot.path.is_empty() || bot.goal.distance(feet) < 4.0 || bot.stuck > 1.5) {
                // Hunt: head for an enemy's last position, or sweep through the town.
                let enemies: Vec<Vec3> = fighters.iter().filter(|(_, team, ..)| *team != me.team).map(|(_, _, at, _)| *at).collect();
                bot.goal = match rng.0.pick(&enemies) {
                    Some(enemy) if rng.0.chance(0.65) => *enemy,
                    _ => {
                        let (x, z) = (rng.0.range(-40.0, 40.0), rng.0.range(-40.0, 40.0));
                        Vec3::new(x, map.height(x, z), z)
                    }
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
            let forward = if distance > far {
                1.0
            } else if distance < near {
                -1.0
            } else {
                0.0
            };
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
            bot.goal = enemy;
            bot.path.clear();
        } else if let Some(nav) = nav.as_deref() {
            // Travel along a path; search again when it runs out or goes stale.
            bot.repath -= dt;
            if (bot.path.is_empty() || bot.repath <= 0.0) && budget.0 > 0 {
                budget.0 -= 1;
                bot.repath = 3.0;
                bot.path = nav.path(&mut scratch, feet, bot.goal).unwrap_or_default();
            }
            while bot.path.first().is_some_and(|p| p.xz().distance(feet.xz()) < 0.6 && (p.y - feet.y).abs() < 1.6) {
                bot.path.remove(0);
            }
            if let Some(next) = bot.path.first() {
                wish = (next.xz() - feet.xz()).normalize_or_zero();
                input.yaw = turn(input.yaw, yaw_to(wish), 6.0 * dt);
                input.pitch *= 1.0 - (4.0 * dt).min(1.0);
                if bot.sentry && bot.alert <= 0.0 {
                    // An unhurried patrol.
                    wish *= 0.45;
                } else {
                    held |= buttons::SPRINT;
                }
            } else if bot.sentry {
                // Standing guard: look about.
                input.yaw += (bot.strafe * 0.5 + if bot.alert > 0.0 { bot.strafe * 1.2 } else { 0.0 }) * dt;
            }
            // Return fire at a gunshot it heard but cannot see the shooter of.
            if bot.sentry && bot.suppress > 0.0 {
                let to = bot.suppress_at - (feet + Vec3::Y * input.eye());
                let distance = to.length().max(1e-3);
                input.yaw = turn(input.yaw, yaw_to(to.xz()) + bot.aim_error.x * 2.0, 7.0 * dt);
                input.pitch = (to.y / distance).asin() + bot.aim_error.y;
                wish = Vec2::ZERO;
                held &= !buttons::SPRINT;
                if !input.held(buttons::FIRE) {
                    held |= buttons::FIRE;
                }
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
    noises.0 = shouts;
}
