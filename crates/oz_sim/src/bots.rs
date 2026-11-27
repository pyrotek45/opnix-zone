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

/// What a versus bot is up to when it has no one in its sights.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Plan {
    /// Head for where the enemy was last seen or heard.
    #[default]
    Hunt,
    /// No idea where they are: sweep the likely places.
    Search,
    /// Hold a spot, crouched, watching the way they will come (`face` is the yaw to watch).
    Camp { face: f32 },
    /// Come at a known enemy from the side instead of head on.
    Flank,
    /// Stay with the squad leader, in a loose formation.
    Follow,
    /// Hurt: fall back toward friends before fighting again.
    Retreat,
}

/// A bot's habits in a group.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Role {
    /// Pushes straight at contacts.
    #[default]
    Rusher,
    /// Holds angles and watches the squad's back.
    Anchor,
    /// Swings wide round contacts.
    Flanker,
}

#[derive(Component, Default)]
pub struct Bot {
    pub plan: Plan,
    pub role: Role,
    /// When the current plan runs out (match seconds).
    plan_until: f32,
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
    let role = [Role::Rusher, Role::Anchor, Role::Flanker][id as usize % 3];
    let brain = Bot { strafe: 1.0, crouch_at_range: rng.0.chance(0.6), role, ..default() };
    let entity = commands.spawn((player_bundle(content, rules, map, id, name, team), ids.next(), brain)).id();
    let guns: Vec<u8> = (0..content.weapons.len() as u8).filter(|i| content.weapons[*i as usize].frame.is_some()).collect();
    if guns.len() < 2 {
        return entity;
    }
    let primary = guns[1 + rng.0.below(guns.len() - 1)];
    let loadout = Loadout { primary, secondary: guns[0], perk: rng.0.below(content.perks.len().max(1)) as u8, gear: rng.0.below(content.gear.len().max(1)) as u8, parts: None, streaks: [0; 3], vision: 0 };
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

/// Somewhere to sweep through: one of the map's waypoints on the indoor maps (their open
/// ground is mostly walled-off void), anywhere in the arena elsewhere.
fn sweep_point(map: &Map, rng: &mut SimRng) -> Vec3 {
    if let Some(p) = rng.0.pick(&map.waypoints) {
        return *p;
    }
    let reach = map.arena * 0.8;
    let (x, z) = (rng.0.range(-reach, reach), rng.0.range(-reach, reach));
    Vec3::new(x, map.height(x, z), z)
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

/// The last place each team saw (or heard) an enemy, and when.
#[derive(Default)]
pub struct TeamIntel([Option<(Vec3, f32)>; 2]);

/// Someone on the field, for planning.
#[derive(Clone, Copy)]
struct Body {
    team: u8,
    id: u8,
    at: Vec3,
    bot: bool,
}

/// Chooses (or keeps) a versus bot's plan while it has no target, and sets its goal.
#[allow(clippy::too_many_arguments)]
fn plan_versus(bot: &mut Bot, me: &Player, feet: Vec3, hurt: bool, map: &Map, rng: &mut SimRng, bodies: &[Body], intel: Option<(Vec3, f32)>, now: f32) {
    let go = |bot: &mut Bot, at: Vec3| {
        if bot.goal.distance(at) > 2.0 || bot.path.is_empty() {
            bot.goal = at;
            bot.path.clear();
            bot.stuck = 0.0;
        }
    };
    let mates: Vec<&Body> = bodies.iter().filter(|b| b.team == me.team && b.id != me.id).collect();
    let solo = mates.is_empty();
    // What the team knows: fresh if seen or heard in the last few seconds.
    let fresh = intel.filter(|(_, t)| now - t < 9.0).map(|(p, _)| p);
    // A long quiet: the bot has a fair idea where someone must be (footsteps, gunfire far off).
    let hunch = |rng: &mut SimRng| {
        let enemies: Vec<Vec3> = bodies.iter().filter(|b| b.team != me.team).map(|b| b.at).collect();
        let near = rng.0.pick(&enemies).copied();
        near.map(|p| p + Vec3::new(rng.0.range(-10.0, 10.0), 0.0, rng.0.range(-10.0, 10.0)))
    };
    let expired = now >= bot.plan_until;
    let arrived = bot.goal.xz().distance(feet.xz()) < 3.0;

    // Hurt: back toward friends (or simply away from trouble) for a breather.
    if hurt && bot.plan != Plan::Retreat {
        bot.plan = Plan::Retreat;
        bot.plan_until = now + 6.0;
        let away = match (mates.iter().min_by(|a, b| a.at.distance_squared(feet).total_cmp(&b.at.distance_squared(feet))), fresh) {
            (Some(mate), _) => mate.at,
            (None, Some(threat)) => feet + (feet - threat).normalize_or_zero() * 15.0,
            (None, None) => feet,
        };
        go(bot, away);
        return;
    }
    if bot.plan == Plan::Retreat && !expired {
        return;
    }
    // Keep a plan while it lasts, refreshing where it points.
    if !expired {
        match bot.plan {
            Plan::Hunt => {
                if let Some(p) = fresh {
                    go(bot, p);
                }
                if !arrived {
                    return;
                }
            }
            Plan::Flank if !arrived => return,
            Plan::Camp { .. } | Plan::Search if !arrived || matches!(bot.plan, Plan::Camp { .. }) => return,
            _ => {}
        }
    }

    // A new plan. Who leads the team: a human if there is one, else the lowest id.
    let leader = bodies.iter().filter(|b| b.team == me.team).min_by_key(|b| (b.bot as u8, b.id)).copied();
    let leading = leader.is_none_or(|l| l.id == me.id);
    let target = if fresh.is_some() { fresh } else if now - intel.map_or(-99.0, |(_, t)| t) > 20.0 { hunch(rng) } else { None };
    let enemy_base = map.team_spawns[(1 - me.team.min(1)) as usize].first().copied().unwrap_or(Vec2::ZERO);
    let watch = |from: Vec3, at: Vec3| yaw_to((at - from).xz());
    let flank = |bot: &mut Bot, at: Vec3| {
        // Out to one side of the line between us, then in.
        let side = if me.id % 2 == 0 { 1.0 } else { -1.0 };
        let across = (at - feet).xz().normalize_or_zero().perp() * side * 14.0;
        let wide = at + Vec3::new(across.x, 0.0, across.y);
        bot.plan = Plan::Flank;
        bot.plan_until = now + 12.0;
        go(bot, Vec3::new(wide.x, map.height(wide.x, wide.z), wide.z));
    };
    let camp = |bot: &mut Bot, at: Vec3, face: f32, seconds: f32| {
        bot.plan = Plan::Camp { face };
        bot.plan_until = now + seconds;
        go(bot, at);
    };
    if solo {
        // One on one: patient. Hold angles, ambush, flank, then go looking.
        match target {
            Some(p) if bot.plan == Plan::Flank || rng.0.chance(0.4) => {
                bot.plan = Plan::Hunt;
                bot.plan_until = now + 8.0;
                go(bot, p);
            }
            Some(p) if rng.0.chance(0.5) => flank(bot, p),
            Some(p) => camp(bot, feet, watch(feet, p), rng.0.range(5.0, 9.0)),
            None if matches!(bot.plan, Plan::Camp { .. }) => {
                bot.plan = Plan::Search;
                bot.plan_until = now + rng.0.range(12.0, 20.0);
                go(bot, sweep_point(map, rng));
            }
            None => {
                let spot = sweep_point(map, rng);
                let face = watch(spot, Vec3::new(enemy_base.x, spot.y, enemy_base.y));
                camp(bot, spot, face, rng.0.range(8.0, 14.0));
            }
        }
        return;
    }
    if leading {
        match target {
            Some(p) => {
                bot.plan = Plan::Hunt;
                bot.plan_until = now + 8.0;
                go(bot, p);
            }
            None => {
                bot.plan = Plan::Search;
                bot.plan_until = now + rng.0.range(10.0, 16.0);
                go(bot, sweep_point(map, rng));
            }
        }
        return;
    }
    match (bot.role, target) {
        (Role::Rusher, Some(p)) => {
            bot.plan = Plan::Hunt;
            bot.plan_until = now + 8.0;
            go(bot, p);
        }
        (Role::Flanker, Some(p)) if bot.plan != Plan::Flank => flank(bot, p),
        (Role::Anchor, Some(p)) => {
            // Hold the angle the trouble will come from, a little back from the leader.
            camp(bot, feet, watch(feet, p), rng.0.range(6.0, 10.0));
        }
        _ => {
            // Stay with the leader, spread out a little to one side and behind.
            let lead = leader.map_or(feet, |l| l.at);
            let angle = me.id as f32 * 2.4;
            let spot = lead + Vec3::new(angle.cos(), 0.0, angle.sin()) * 4.5;
            bot.plan = Plan::Follow;
            bot.plan_until = now + 1.5;
            if spot.distance(feet) > 6.0 {
                go(bot, Vec3::new(spot.x, lead.y, spot.z));
            }
        }
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
    mut intel: Local<TeamIntel>,
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
    // Torches lit in the dark give their carriers away.
    let dark = state.daylight < 0.5;
    let lit: Vec<Entity> = players.iter().filter(|(_, _, _, _, input, health, ..)| dark && input.light && health.alive()).map(|(entity, ..)| entity).collect();
    // Everyone alive, for planning; and what each team knows of the other.
    let bodies: Vec<Body> = players.iter().filter(|(.., health, _, _, _)| health.alive()).map(|(_, bot, p, t, ..)| Body { team: p.team, id: p.id, at: t.translation, bot: bot.is_some() }).collect();
    let now = time.elapsed_secs();
    for (_, bot, p, ..) in players.iter() {
        if let Some(seen) = bot.and_then(|b| b.target).and_then(|e| fighters.iter().find(|f| f.0 == e)) {
            intel.0[(p.team as usize).min(1)] = Some((seen.2, now));
        }
    }
    if state.phase != Phase::Live {
        intel.0 = [None, None];
    }
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
    // Gunfire gives the shooter away to the other team.
    for noise in heard_now.iter().filter(|n| !n.shout && n.radius > 40.0) {
        intel.0[(1 - noise.team.min(1)) as usize] = Some((noise.at, now));
    }

    for (_, bot, me, transform, mut input, health, arsenal, stunned, _) in &mut players {
        let Some(mut bot) = bot else { continue };
        if bot.sentry && health.alive() {
            bot.alert = (bot.alert - dt).max(0.0);
            bot.suppress -= dt;
            let feet = transform.translation;
            let base = crate::versus::hunt_base(&map);
            for noise in &heard_now {
                let ours = noise.team == me.team;
                if noise.shout != ours || noise.at.distance(feet) > noise.radius || noise.at.xz().distance(base) < crate::versus::HUNT_SAFE {
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
            let noticed = |at: Vec3, enemy_stance: u8, torch: bool| {
                if !sentry {
                    return true;
                }
                let to = at - feet;
                let distance = to.length();
                let range: f32 = if alert { 70.0 } else { 40.0 * [1.0, 0.7, 0.45][(enemy_stance as usize).min(2)] };
                // A lit torch in the dark shows from much further, and catches the eye off to the side.
                let (range, cone) = if torch { (range.max(40.0) * 1.8, -0.1) } else { (range, if alert { -0.3 } else { 0.4 }) };
                distance < 2.5 || (distance < range && to.xz().normalize_or_zero().dot(facing_dir) > cone)
            };
            // Hunt: the squad's base is off limits to hostiles.
            let safe = |at: Vec3| sentry && rules.hunt && at.xz().distance(crate::versus::hunt_base(&map)) < crate::versus::HUNT_SAFE;
            let found = fighters
                .iter()
                .filter(|(_, team, ..)| *team != me.team)
                .filter(|(_, _, at, _)| !safe(*at))
                .filter(|(entity, _, at, enemy_stance)| noticed(*at, *enemy_stance, lit.contains(entity)))
                .filter_map(|(entity, _, at, enemy_stance)| {
                    let chest = *at + Vec3::Y * (stance::eye(*enemy_stance) - 0.4);
                    // Face the target before testing a lean, so the lean goes the right way.
                    let facing = PlayerInput { yaw: yaw_to((chest - eye).xz()), ..upright };
                    [0i8].into_iter().find(|lean| sees(crate::player::eye_point(&statics, feet, &PlayerInput { lean: *lean, ..facing }), chest)).map(|lean| (*entity, chest, lean))
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
            if bot.sentry && rules.hunt && feet.xz().distance(crate::versus::hunt_base(&map)) < crate::versus::HUNT_SAFE + 2.0 {
                // Strayed into the squad's base: back to the post.
                bot.heard = None;
                bot.target = None;
                if bot.goal.distance(bot.post) > 1.0 {
                    bot.goal = bot.post;
                    bot.path.clear();
                }
            } else if bot.sentry && bot.target.is_none() {
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
                    bot.goal = sweep_point(&map, &mut rng);
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
            } else if bot.target.is_none() {
                if bot.stuck > 1.5 {
                    bot.plan_until = 0.0;
                    bot.stuck = 0.0;
                }
                let hurt = health.hp < health.max * 0.35;
                plan_versus(&mut bot, me, feet, hurt, &map, &mut rng, &bodies, intel.0[(me.team as usize).min(1)], now);
            }
        }
        bot.strafe_time -= dt;
        if bot.strafe_time <= 0.0 {
            bot.strafe_time = rng.0.range(0.6, 1.5);
            bot.strafe = -bot.strafe;
        }

        let target = bot.target.and_then(|entity| fighters.iter().find(|(e, ..)| *e == entity).copied());
        // No leaning in this game: bots shoot from where they stand.
        input.lean = 0;
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

        // Camping: settle in at the spot, low, watching the approach.
        if let Plan::Camp { face } = bot.plan
            && bot.target.is_none()
            && !bot.sentry
            && bot.goal.xz().distance(feet.xz()) < 1.5
        {
            wish = Vec2::ZERO;
            held &= !buttons::SPRINT;
            wanted_stance = stance::CROUCH;
            let scan = (now * 0.5 + me.id as f32).sin() * 0.7;
            input.yaw = turn(input.yaw, face + scan, 2.0 * dt);
            input.pitch *= 1.0 - (3.0 * dt).min(1.0);
        }
        // Retreating or hunting a known spot: no sprinting into the open when hurt.
        if bot.plan == Plan::Retreat && bot.target.is_some() {
            // Backing off while still shooting: away from the target (it is in front).
            let ahead = Vec2::new(-input.yaw.sin(), -input.yaw.cos());
            wish = -ahead * 0.8 + ahead.perp() * bot.strafe * 0.4;
            wanted_stance = stance::CROUCH;
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
        // Bots light up at night (it gives them away, as it does anyone).
        if input.light != dark {
            input.light = dark;
        }
    }
    noises.0 = shouts;
}
