//! Killstreaks (versus modes).
//!
//! Each class picks up to three of these; kills without dying earn them:
//!
//! | Kills | Reward | What it does |
//! |------:|--------|--------------|
//! | 3 | Recon UAV | 30 s of every enemy showing on your team's radar |
//! | 4 | Supply drop | your whole team healed and rearmed, gear included |
//! | 5 | Airstrike | a jet drops a line of bombs where you are aiming |
//! | 7 | Attack drone | hovers over you for 30 s and shoots enemies it can see |
//! | 5 | Drone strike | a kamikaze drone dives on the enemy nearest where you aim |
//! | 6 | Sentry gun | a turret set down in front of you that guards the spot for 45 s |
//! | 8 | EMP | the enemy team's radar and minimap go dark for 20 s; their aircraft and sentries drop |
//! | 9 | Artillery barrage | shells rain down around where you aim for several seconds |
//! | 10 | Advanced UAV | 40 s of every enemy on your radar - Ghosts too - from too high for most guns |
//!
//! The Hardline perk takes one kill off every streak.
//!
//! Earned rewards go on a stack of at most three; calling one in (a key /
//! button that sends [`PlayerCommand::UseStreak`]) uses the newest first.
//! Streaks reset on death; earned rewards wait until used. Kills made by a
//! reward count toward the next one.

use crate::{
    combat::{SmokeCloud, smoke_between},
    net::NetIds,
    player::{Arsenal, Health, Player, PlayerInput, damage_player, eye_point},
    session::{Content, Map, MatchEntity, Statics},
    sim_dt,
    state::{CommandEvent, FxEvent, MatchState, Phase, PlayerCommand, Rules},
};
use bevy::prelude::*;
use oz_core::synth::Sfx;

/// Reward ids (0 = none).
pub mod reward {
    pub const UAV: u8 = 1;
    pub const SUPPLY: u8 = 2;
    pub const AIRSTRIKE: u8 = 3;
    pub const DRONE: u8 = 4;
    pub const ARTILLERY: u8 = 5;
    pub const DRONE_STRIKE: u8 = 6;
    pub const SENTRY: u8 = 7;
    pub const EMP: u8 = 8;
    pub const ADVANCED_UAV: u8 = 9;
}

pub struct StreakDef {
    pub kills: u16,
    pub id: u8,
    pub name: &'static str,
    pub blurb: &'static str,
}

/// Every reward, cheapest first.
pub const STREAKS: [StreakDef; 9] = [
    StreakDef { kills: 3, id: reward::UAV, name: "RECON UAV", blurb: "Every enemy shows on your team's radar for 30 seconds (Ghosts excepted)" },
    StreakDef { kills: 4, id: reward::SUPPLY, name: "SUPPLY DROP", blurb: "Heals and rearms your whole team, gear included" },
    StreakDef { kills: 5, id: reward::AIRSTRIKE, name: "AIRSTRIKE", blurb: "A jet drops a line of bombs where you aim" },
    StreakDef { kills: 5, id: reward::DRONE_STRIKE, name: "DRONE STRIKE", blurb: "A kamikaze drone dives on the enemy nearest where you aim" },
    StreakDef { kills: 6, id: reward::SENTRY, name: "SENTRY GUN", blurb: "A turret in front of you guards the spot for 45 seconds" },
    StreakDef { kills: 7, id: reward::DRONE, name: "ATTACK DRONE", blurb: "Hovers over you for 30 seconds, shooting enemies it sees" },
    StreakDef { kills: 8, id: reward::EMP, name: "EMP", blurb: "Enemy radar and minimap go dark for 20 seconds; their aircraft and sentries drop" },
    StreakDef { kills: 9, id: reward::ARTILLERY, name: "ARTILLERY BARRAGE", blurb: "Shells rain down around where you aim" },
    StreakDef { kills: 10, id: reward::ADVANCED_UAV, name: "ADVANCED UAV", blurb: "40 seconds of every enemy on radar - Ghosts too - flying too high for most guns" },
];

/// The three streaks a class carries when it does not choose.
pub const DEFAULT_STREAKS: [u8; 3] = [reward::UAV, reward::AIRSTRIKE, reward::DRONE];

/// How many rewards can wait to be used.
pub const STACK: usize = 3;

pub fn def(id: u8) -> Option<&'static StreakDef> {
    STREAKS.iter().find(|s| s.id == id)
}

pub fn name(id: u8) -> &'static str {
    def(id).map_or("REWARD", |s| s.name)
}

/// Kills in a row a streak takes; Hardline takes one off (never below two).
pub fn kills_needed(def: &StreakDef, hardline: bool) -> u16 {
    if hardline { def.kills.saturating_sub(1).max(2) } else { def.kills }
}

/// Does this player carry the Hardline perk?
pub fn hardline(content: &oz_core::content::Catalog, player: &Player) -> bool {
    matches!(crate::player::perk(content, player), Some(oz_core::content::PerkEffect::Hardline))
}

/// A class's chosen streaks, made valid: known ids, no repeats, at least one.
pub fn chosen(streaks: [u8; 3]) -> [u8; 3] {
    let mut out = [0u8; 3];
    let mut n = 0;
    for id in streaks {
        if def(id).is_some() && !out.contains(&id) {
            out[n] = id;
            n += 1;
        }
    }
    if n == 0 { DEFAULT_STREAKS } else { out }
}

/// The reward the next call-in will use: the newest one earned.
pub fn next(rewards: &[u8; STACK]) -> Option<&'static StreakDef> {
    rewards.iter().rev().find(|id| **id != 0).and_then(|id| def(*id))
}

/// Puts a freshly earned reward on top of the stack; a full stack drops its oldest.
pub fn push(rewards: &mut [u8; STACK], id: u8) {
    if let Some(slot) = rewards.iter().position(|r| *r == 0) {
        rewards[slot] = id;
    } else {
        rewards.rotate_left(1);
        rewards[STACK - 1] = id;
    }
}

/// Takes the newest reward off the stack.
pub fn pop(rewards: &mut [u8; STACK]) -> Option<u8> {
    let slot = rewards.iter().rposition(|r| *r != 0)?;
    Some(std::mem::take(&mut rewards[slot]))
}

/// Killstreak aircraft are real things in the sky: drawn, replicated, and shootable by the
/// other team. Destroying one ends the reward early (a downed jet drops nothing more, a
/// downed radar plane takes the UAV with it).
#[derive(Component, Clone, Copy, Debug)]
pub struct Aircraft {
    pub look: oz_core::content::ShotLook,
    pub hp: f32,
    pub team: u8,
    pub owner: u8,
    /// Hit sphere radius.
    pub radius: f32,
    /// Who hurt it last (for the shoot-down notice).
    pub hit_by: Option<u8>,
}

/// The recon UAV's radar plane, circling the town.
#[derive(Component, Clone, Copy, Debug)]
pub struct Recon {
    pub life: f32,
    pub angle: f32,
    /// The advanced UAV: flies higher, sees everyone (and shrugs off an EMP).
    pub advanced: bool,
}

/// A killstreak sentry gun: stands where it was set down and shoots enemies it can see.
#[derive(Component, Clone, Copy, Debug)]
pub struct Sentry {
    pub owner: u8,
    pub team: u8,
    pub life: f32,
    pub cooldown: f32,
}

/// A drone strike: steers its projectile onto `target` (a player id).
#[derive(Component, Clone, Copy, Debug)]
pub struct Homing {
    pub target: u8,
    pub speed: f32,
    /// Radians per second it can turn.
    pub turn: f32,
}

const JET_HP: f32 = 260.0;
const JET_SPEED: f32 = 48.0;
const JET_HEIGHT: f32 = 30.0;
const RECON_HP: f32 = 240.0;
const RECON_HEIGHT: f32 = 90.0;
const RECON_ORBIT: f32 = 55.0;
const DRONE_HP: f32 = 130.0;
/// Seconds a bomb takes to fall from the jet.
const BOMB_FALL: f32 = 1.1;

const SHELLS: u8 = 16;
const SHELL_SPREAD: f32 = 13.0;
const BARRAGE_DELAY: f32 = 3.0;
const UAV_SECONDS: f32 = 30.0;
const STRIKE_DELAY: f32 = 2.6;
const BOMBS: u8 = 7;
const BOMB_SPACING: f32 = 3.6;
const BOMB_DAMAGE: f32 = 170.0;
const BOMB_RADIUS: f32 = 9.0;
const DRONE_SECONDS: f32 = 30.0;
pub const SENTRY_SECONDS: f32 = 45.0;
const SENTRY_HP: f32 = 200.0;
pub const SENTRY_RANGE: f32 = 38.0;
const SENTRY_DAMAGE: f32 = 12.0;
const SENTRY_RATE: f32 = 0.3;
/// The sentry's gun sits this high above its feet (also the centre of its hit sphere).
pub const SENTRY_HEIGHT: f32 = 0.9;
pub const EMP_SECONDS: f32 = 20.0;
pub const SWEEP_SECONDS: f32 = 40.0;
const SWEEP_HP: f32 = 520.0;
const SWEEP_HEIGHT: f32 = 140.0;
const STRIKE_SPEED: f32 = 34.0;
const STRIKE_DAMAGE: f32 = 200.0;
const STRIKE_SPLASH: f32 = 6.0;
const STRIKE_HEIGHT: f32 = 45.0;
const DRONE_RANGE: f32 = 45.0;
const DRONE_DAMAGE: f32 = 13.0;
const DRONE_RATE: f32 = 0.32;
/// A sentinel shooter id for drone tracers, so the owner's client does not
/// mistake them for its own predicted shots.
pub const DRONE_SHOOTER: u8 = 254;

#[derive(Component, Clone, Copy, Debug)]
pub struct Airstrike {
    pub at: Vec3,
    pub dir: Vec3,
    pub owner: u8,
    pub team: u8,
    pub timer: f32,
    pub flown: bool,
    pub dropped: u8,
    /// Artillery: shells scattered around `at` over several seconds instead of a line of bombs.
    pub barrage: bool,
}

#[derive(Component, Clone, Copy, Debug)]
pub struct Drone {
    pub owner: u8,
    pub team: u8,
    pub life: f32,
    pub cooldown: f32,
    /// Where it circles, around the owner.
    pub orbit: f32,
}

/// A cargo plane on its way to drop supplies at `at`.
#[derive(Component, Clone, Copy, Debug)]
pub struct Cargo {
    pub at: Vec3,
    pub dir: Vec3,
    pub team: u8,
    pub owner: u8,
    pub dropped: bool,
}

/// A supply crate: falls under its parachute, then resupplies each teammate who reaches it once.
#[derive(Component, Clone, Debug)]
pub struct SupplyCrate {
    pub team: u8,
    pub ground: f32,
    pub used: Vec<u8>,
}

const CARGO_HP: f32 = 300.0;
const CARGO_SPEED: f32 = 36.0;
const CARGO_HEIGHT: f32 = 38.0;
/// Seconds from the call to the drop.
const CARGO_DELAY: f32 = 4.0;
/// How fast a crate comes down under its chute (m/s), and how long it stays once landed.
const CHUTE_SPEED: f32 = 4.5;
const CRATE_LIFE: f32 = 60.0;
/// Resupplies in one crate: press Use at it to take one.
pub const CRATE_USES: u8 = 2;

/// Authority: flies cargo planes, drops their crates, lands them and hands out supplies.
#[allow(clippy::too_many_arguments)]
pub fn run_supply(
    time: Res<Time>,
    content: Res<Content>,
    mut commands: Commands,
    mut ids: ResMut<NetIds>,
    mut planes: Query<(Entity, &mut Cargo, &mut Transform), (Without<SupplyCrate>, Without<Player>)>,
    mut crates: Query<(Entity, &mut SupplyCrate, &mut Transform, &mut crate::loot::Pickup), (Without<Cargo>, Without<Player>)>,
    mut players: Query<(&Transform, &mut Player, &mut Health, &mut Arsenal, &PlayerInput), (Without<Cargo>, Without<SupplyCrate>)>,
    mut fx: EventWriter<FxEvent>,
) {
    let dt = sim_dt(&time);
    for (entity, mut cargo, mut transform) in &mut planes {
        transform.translation += cargo.dir * CARGO_SPEED * dt;
        let past = (transform.translation - cargo.at).xz().dot(cargo.dir.xz());
        if !cargo.dropped && past >= 0.0 {
            cargo.dropped = true;
            commands.spawn((
                MatchEntity,
                ids.next(),
                SupplyCrate { team: cargo.team, ground: cargo.at.y, used: Vec::new() },
                crate::loot::Pickup { loot: crate::loot::Loot::Supply { team: cargo.team, uses: CRATE_USES }, life: Some(CRATE_LIFE) },
                Transform::from_translation(transform.translation - Vec3::Y * 2.5),
            ));
        }
        if past > 220.0 {
            commands.entity(entity).despawn();
        }
    }
    for (entity, mut supply, mut transform, mut pickup) in &mut crates {
        if transform.translation.y > supply.ground + 0.01 {
            // Under the chute: a slow, slightly swaying descent.
            transform.translation.y = (transform.translation.y - CHUTE_SPEED * dt).max(supply.ground);
            continue;
        }
        let life = pickup.life.unwrap_or(0.0) - dt;
        pickup.life = Some(life);
        if life <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        let at = transform.translation;
        let crate::loot::Loot::Supply { team, uses } = pickup.loot else { continue };
        if uses == 0 {
            // Emptied: it stays a little while as an open, empty box, then goes.
            pickup.life = Some(life.min(8.0));
            continue;
        }
        for (body, mut mate, mut health, mut arsenal, input) in &mut players {
            // Walk up and press Use to open it and take a resupply (once each).
            if mate.team != supply.team || !health.alive() || !input.interact || supply.used.contains(&mate.id) || body.translation.distance(at) > 2.2 {
                continue;
            }
            let crate::loot::Loot::Supply { uses, .. } = pickup.loot else { break };
            if uses == 0 {
                break;
            }
            pickup.loot = crate::loot::Loot::Supply { team, uses: uses - 1 };
            supply.used.push(mate.id);
            health.hp = health.max;
            mate.burn_time = 0.0;
            crate::weapons::top_up(&mut arsenal, &content.weapons);
            mate.gear_left = crate::player::gear_count(&content, &mate).max(mate.gear_left);
            fx.write(FxEvent::tell(mate.id, "RESUPPLIED"));
            fx.write(FxEvent::Sound { sfx: Sfx::Buy, at: Some(at) });
        }
    }
}

/// Authority: turns call-in commands into rewards.
#[allow(clippy::too_many_arguments)]
pub fn use_streaks(
    mut orders: EventReader<CommandEvent>,
    rules: Res<Rules>,
    map: Res<Map>,
    statics: Res<Statics>,
    mut state: ResMut<MatchState>,
    mut ids: ResMut<NetIds>,
    mut commands: Commands,
    mut players: Query<(&mut Player, &Transform, &PlayerInput, &mut Health, &mut Arsenal)>,
    mut aircraft: Query<(&mut Aircraft, Option<&Recon>)>,
    mut fx: EventWriter<FxEvent>,
) {
    for order in orders.read() {
        // Who is where, for a drone strike to pick its target: (team, alive, id, chest).
        let targets: Vec<(u8, bool, u8, Vec3)> = players.iter().map(|(p, t, _, h, _)| (p.team, h.alive() && p.protect <= 0.0, p.id, t.translation + Vec3::Y * 1.1)).collect();
        if order.command != PlayerCommand::UseStreak {
            continue;
        }
        let Some((mut player, transform, input, health, _)) = players.iter_mut().find(|(p, ..)| p.id == order.player) else {
            continue;
        };
        let Some(def) = next(&player.rewards).filter(|_| rules.pvp && health.alive() && state.phase == Phase::Live) else {
            fx.write(FxEvent::Cue { to: order.player, sfx: Sfx::UiDeny });
            continue;
        };
        pop(&mut player.rewards);
        let team = player.team;
        let (name, id) = (player.name.clone(), player.id);
        let aim = |transform: &Transform, input: &PlayerInput| {
            let eye = eye_point(&statics, transform.translation, input);
            let dir = input.aim_dir();
            let mut reach = map.raycast(eye, dir, 150.0);
            if let Some((d, _)) = statics.raycast(eye, dir, reach) {
                reach = reach.min(d);
            }
            (eye + dir * reach, Vec3::new(dir.x, 0.0, dir.z).normalize_or(Vec3::NEG_Z))
        };
        match def.id {
            reward::UAV | reward::ADVANCED_UAV => {
                let advanced = def.id == reward::ADVANCED_UAV;
                let (seconds, height, hp) = if advanced { (SWEEP_SECONDS, SWEEP_HEIGHT, SWEEP_HP) } else { (UAV_SECONDS, RECON_HEIGHT, RECON_HP) };
                if advanced {
                    state.sweep[(team as usize).min(1)] = seconds;
                } else {
                    state.uav[(team as usize).min(1)] = seconds;
                }
                let angle = transform.translation.x.atan2(transform.translation.z);
                let at = Vec3::new(angle.sin() * RECON_ORBIT, height, angle.cos() * RECON_ORBIT);
                commands.spawn((
                    MatchEntity,
                    ids.next(),
                    Recon { life: seconds, angle, advanced },
                    Aircraft { look: oz_core::content::ShotLook::RadarPlane, hp, team, owner: id, radius: 4.0, hit_by: None },
                    Transform::from_translation(at),
                ));
            }
            reward::EMP => {
                // The other team goes dark; whatever they have up in the air (or set down) falls.
                let enemy = 1 - (team as usize).min(1);
                state.emp[enemy] = EMP_SECONDS;
                state.uav[enemy] = 0.0;
                for (mut craft, recon) in &mut aircraft {
                    if craft.team != team && !recon.is_some_and(|r| r.advanced) {
                        craft.hp = 0.0;
                        craft.hit_by = Some(id);
                    }
                }
                fx.write(FxEvent::Sound { sfx: Sfx::Alarm, at: None });
            }
            reward::SENTRY => {
                // Set down a couple of steps ahead (or at your feet if there is no room there).
                let forward = Vec3::new(-input.yaw.sin(), 0.0, -input.yaw.cos());
                let feet = transform.translation;
                let blocked = statics.raycast(feet + Vec3::Y * 0.6, forward, 1.9).is_some();
                let spot = if blocked { feet } else { feet + forward * 1.6 };
                let ground = map.height(spot.x, spot.z).max(statics.support(spot.x, spot.z, feet.y + 0.5));
                let at = Vec3::new(spot.x, ground + SENTRY_HEIGHT, spot.z);
                commands.spawn((
                    MatchEntity,
                    ids.next(),
                    Sentry { owner: id, team, life: SENTRY_SECONDS, cooldown: 1.2 },
                    Aircraft { look: oz_core::content::ShotLook::Sentry, hp: SENTRY_HP, team, owner: id, radius: 0.8, hit_by: None },
                    Transform::from_translation(at).looking_to(forward, Vec3::Y),
                ));
                fx.write(FxEvent::tell(id, "SENTRY GUN DEPLOYED"));
            }
            reward::DRONE_STRIKE => {
                // Comes down out of the sky over where you aim and steers itself onto the
                // enemy nearest that spot (or the nearest one to you, if nobody is near it).
                let (at, flat) = aim(transform, input);
                let me = transform.translation;
                let score = |p: Vec3| p.distance(at).min(25.0) * 4.0 + p.distance(me) * 0.05;
                let target = targets.iter().filter(|t| t.0 != team && t.1).min_by(|a, b| score(a.3).total_cmp(&score(b.3))).map(|t| t.2);
                let from = Vec3::new(at.x, at.y.max(me.y) + STRIKE_HEIGHT, at.z) - flat * 12.0;
                let velocity = (at - from).normalize_or(Vec3::NEG_Y) * STRIKE_SPEED;
                let shot = crate::combat::Projectile { velocity, gravity: 0.0, damage: STRIKE_DAMAGE, splash: STRIKE_SPLASH, look: oz_core::content::ShotLook::Drone, owner: Some(id), team, life: 9.0, effect: crate::combat::Impact::Blast };
                let entity = commands.spawn((MatchEntity, ids.next(), shot, Transform::from_translation(from).looking_to(velocity, Vec3::Y))).id();
                if let Some(target) = target {
                    commands.entity(entity).insert(Homing { target, speed: STRIKE_SPEED, turn: 2.6 });
                }
                fx.write(FxEvent::Flyby { from: from + Vec3::Y * 20.0, to: at });
            }
            reward::SUPPLY => {
                // A cargo plane flies over and drops a crate on a parachute a few steps
                // ahead of you; teammates walk up to it to heal and rearm.
                let forward = Vec3::new(-input.yaw.sin(), 0.0, -input.yaw.cos());
                let spot = transform.translation + forward * 5.0;
                let at = Vec3::new(spot.x, map.height(spot.x, spot.z).max(statics.support(spot.x, spot.z, transform.translation.y + 1.0)), spot.z);
                let start = at + Vec3::Y * CARGO_HEIGHT - forward * CARGO_SPEED * CARGO_DELAY;
                commands.spawn((
                    MatchEntity,
                    ids.next(),
                    Cargo { at, dir: forward, team, owner: id, dropped: false },
                    Aircraft { look: oz_core::content::ShotLook::RadarPlane, hp: CARGO_HP, team, owner: id, radius: 4.0, hit_by: None },
                    Transform::from_translation(start).looking_to(forward, Vec3::Y),
                ));
                fx.write(FxEvent::Sound { sfx: Sfx::Buy, at: None });
                fx.write(FxEvent::tell(id, "SUPPLY DROP INBOUND"));
            }
            reward::ARTILLERY => {
                let (at, dir) = aim(transform, input);
                commands.spawn((MatchEntity, Airstrike { at, dir, owner: id, team, timer: BARRAGE_DELAY, flown: true, dropped: 0, barrage: true }));
            }
            reward::AIRSTRIKE => {
                // Bombs fall along the line you are looking down, centred where you aim.
                let (at, flat) = aim(transform, input);
                let start = at + Vec3::Y * JET_HEIGHT - flat * JET_SPEED * STRIKE_DELAY;
                commands.spawn((
                    MatchEntity,
                    ids.next(),
                    Airstrike { at, dir: flat, owner: id, team, timer: STRIKE_DELAY, flown: false, dropped: 0, barrage: false },
                    Aircraft { look: oz_core::content::ShotLook::Jet, hp: JET_HP, team, owner: id, radius: 3.4, hit_by: None },
                    Transform::from_translation(start).looking_to(flat, Vec3::Y),
                ));
            }
            _ => {
                let at = transform.translation + Vec3::Y * 4.5;
                commands.spawn((
                    MatchEntity,
                    ids.next(),
                    Drone { owner: id, team, life: DRONE_SECONDS, cooldown: 1.0, orbit: 0.0 },
                    Aircraft { look: oz_core::content::ShotLook::Drone, hp: DRONE_HP, team, owner: id, radius: 0.9, hit_by: None },
                    Transform::from_translation(at),
                ));
            }
        }
        fx.write(FxEvent::StreakUsed { name, team, reward: def.id });
    }
}

/// Authority: UAV timers, falling bombs and hunting drones.
#[allow(clippy::too_many_arguments)]
pub fn run_streaks(
    time: Res<Time>,
    map: Res<Map>,
    statics: Res<Statics>,
    mut state: ResMut<MatchState>,
    mut commands: Commands,
    mut strikes: Query<(Entity, &mut Airstrike, Option<&mut Transform>), (Without<Drone>, Without<Player>, Without<Recon>, Without<Sentry>)>,
    mut recons: Query<(Entity, &mut Recon, &mut Transform), (Without<Drone>, Without<Player>, Without<Airstrike>, Without<Sentry>)>,
    mut ids: ResMut<NetIds>,
    mut drones: Query<(Entity, &mut Drone, &mut Transform), (Without<Player>, Without<Airstrike>, Without<Recon>, Without<Sentry>)>,
    mut sentries: Query<(Entity, &mut Sentry, &mut Transform), (Without<Player>, Without<Airstrike>, Without<Recon>, Without<Drone>)>,
    mut players: Query<(&Transform, &mut Health, &mut Player), (Without<crate::combat::Projectile>, Without<crate::structures::Structure>, Without<Drone>, Without<Sentry>)>,
    smokes: Query<(&Transform, &SmokeCloud), (Without<Player>, Without<Drone>, Without<Airstrike>, Without<Recon>, Without<Sentry>)>,
    mut fx: EventWriter<FxEvent>,
) {
    let dt = sim_dt(&time);
    let state_ref = &mut *state;
    for uav in state_ref.uav.iter_mut().chain(state_ref.sweep.iter_mut()).chain(state_ref.emp.iter_mut()) {
        *uav = (*uav - dt).max(0.0);
    }
    let name_of = |state: &MatchState, id: u8| state.roster.iter().find(|r| r.id == id).map_or(String::new(), |r| r.name.clone());

    for (entity, mut strike, mut transform) in &mut strikes {
        strike.timer -= dt;
        // The jet flies straight over the target; the engines are heard as it comes in.
        if !strike.barrage {
            if let Some(transform) = transform.as_mut() {
                // Over the target as the first bombs land; the bombs keep the jet's speed as they fall.
                transform.translation = strike.at + Vec3::Y * JET_HEIGHT - strike.dir * JET_SPEED * strike.timer;
            }
            if !strike.flown && strike.timer <= 1.6 {
                strike.flown = true;
                let high = strike.at + Vec3::Y * JET_HEIGHT;
                fx.write(FxEvent::Flyby { from: high - strike.dir * 160.0, to: high + strike.dir * 160.0 });
            }
        }
        // Bombs leave the jet one after another, timed to land in a line across the target;
        // artillery shells come down out of the sky, slower and scattered.
        let (count, gap) = if strike.barrage { (SHELLS, 0.38) } else { (BOMBS, 0.12) };
        let lead = if strike.barrage { 0.0 } else { BOMB_FALL };
        while strike.dropped < count && strike.timer <= lead - (strike.dropped as f32) * gap {
            let p = if strike.barrage {
                // A sunflower scatter: even cover of the target area, no two shells alike.
                let k = strike.dropped as f32 + 0.5;
                let r = SHELL_SPREAD * (k / count as f32).sqrt();
                let angle = k * 2.399 + strike.at.x;
                strike.at + Vec3::new(angle.cos() * r, 0.0, angle.sin() * r)
            } else {
                strike.at + strike.dir * (strike.dropped as f32 - (BOMBS - 1) as f32 * 0.5) * BOMB_SPACING
            };
            let ground = map.height(p.x, p.z).max(statics.support(p.x, p.z, p.y + 0.5));
            let target = Vec3::new(p.x, ground.max(p.y - 3.0), p.z);
            let (from, look, gravity, fall) = if strike.barrage {
                (target + Vec3::new(4.0, 70.0, 0.0), oz_core::content::ShotLook::Shell, 20.0, 1.6)
            } else {
                (transform.as_ref().map_or(target + Vec3::Y * JET_HEIGHT, |t| t.translation - Vec3::Y * 1.2), oz_core::content::ShotLook::Bomb, 14.0, BOMB_FALL)
            };
            // Launch so it lands on `target` after `fall` seconds under gravity.
            let velocity = (target - from) / fall + Vec3::Y * 0.5 * gravity * fall;
            let shot = crate::combat::Projectile { velocity, gravity, damage: BOMB_DAMAGE, splash: BOMB_RADIUS, look, owner: Some(strike.owner), team: strike.team, life: fall + 2.0, effect: crate::combat::Impact::Blast };
            crate::combat::spawn_projectile(&mut commands, &mut ids, from, shot);
            strike.dropped += 1;
        }
        // Done once everything is away and the jet has flown off.
        if strike.dropped >= count && strike.timer < -2.5 {
            commands.entity(entity).despawn();
        }
    }

    // The radar plane circles the town until the UAV runs out.
    for (entity, mut recon, mut transform) in &mut recons {
        recon.life -= dt;
        recon.angle += dt * 0.22;
        let height = if recon.advanced { SWEEP_HEIGHT } else { RECON_HEIGHT };
        let at = Vec3::new(recon.angle.sin() * RECON_ORBIT, height, recon.angle.cos() * RECON_ORBIT);
        let heading = Vec3::new(recon.angle.cos(), 0.0, -recon.angle.sin());
        transform.translation = at;
        transform.look_to(heading, Vec3::Y);
        if recon.life <= 0.0 || state.phase != Phase::Live {
            commands.entity(entity).despawn();
        }
    }

    // Sentry guns: turn to the nearest enemy in sight and fire short bursts.
    for (entity, mut sentry, mut transform) in &mut sentries {
        sentry.life -= dt;
        if sentry.life <= 0.0 || state.phase != Phase::Live {
            fx.write(FxEvent::Explosion { at: transform.translation, radius: 1.5 });
            commands.entity(entity).despawn();
            continue;
        }
        sentry.cooldown -= dt;
        let at = transform.translation + Vec3::Y * 0.25;
        let target = players
            .iter()
            .filter(|(_, h, p)| p.team != sentry.team && h.alive() && p.protect <= 0.0)
            .map(|(t, _, p)| (p.id, t.translation + Vec3::Y * 1.1))
            .filter(|(_, chest)| {
                let span = *chest - at;
                let d = span.length();
                d < SENTRY_RANGE && statics.raycast(at, span / d.max(1e-3), d).is_none() && map.raycast(at, span / d.max(1e-3), d) >= d - 0.3 && !smoke_between(smokes, at, *chest)
            })
            .min_by(|a, b| a.1.distance_squared(at).total_cmp(&b.1.distance_squared(at)));
        let Some((victim, chest)) = target else {
            continue;
        };
        let level = transform.translation.y;
        transform.look_at(Vec3::new(chest.x, level, chest.z), Vec3::Y);
        if sentry.cooldown > 0.0 {
            continue;
        }
        sentry.cooldown = SENTRY_RATE;
        fx.write(FxEvent::Shot { by: DRONE_SHOOTER, sound: 1, from: at, to: chest, beam: false, tracer: oz_core::model::Paint::Gold, quiet: false, local: false });
        let name = name_of(&state, sentry.owner);
        if let Some((_, mut health, mut player)) = players.iter_mut().find(|(_, _, p)| p.id == victim) {
            damage_player(&mut health, &mut player, SENTRY_DAMAGE, Some((&name, sentry.team, sentry.owner)), &mut state, &mut fx);
        }
    }

    for (entity, mut drone, mut transform) in &mut drones {
        drone.life -= dt;
        let owner = players.iter().find(|(_, _, p)| p.id == drone.owner).map(|(t, h, _)| (t.translation, h.alive()));
        if drone.life <= 0.0 || owner.is_none() || state.phase != Phase::Live {
            fx.write(FxEvent::Explosion { at: transform.translation, radius: 1.5 });
            commands.entity(entity).despawn();
            continue;
        }
        // Circle above the owner (or where they fell).
        drone.orbit += dt * 0.9;
        let (owner_at, _) = owner.unwrap_or((transform.translation, false));
        let wanted = owner_at + Vec3::new(drone.orbit.cos() * 2.5, 4.5, drone.orbit.sin() * 2.5);
        transform.translation = transform.translation.lerp(wanted, 1.0 - (-dt * 2.5).exp());
        let at = transform.translation;

        drone.cooldown -= dt;
        // Nearest enemy it can actually see.
        let target = players
            .iter()
            .filter(|(_, h, p)| p.team != drone.team && h.alive() && p.protect <= 0.0)
            .map(|(t, _, p)| (p.id, t.translation + Vec3::Y * 1.1))
            .filter(|(_, chest)| {
                let span = *chest - at;
                let d = span.length();
                d < DRONE_RANGE && statics.raycast(at, span / d.max(1e-3), d).is_none() && map.raycast(at, span / d.max(1e-3), d) >= d - 0.3 && !smoke_between(smokes, at, *chest)
            })
            .min_by(|a, b| a.1.distance_squared(at).total_cmp(&b.1.distance_squared(at)));
        let Some((victim, chest)) = target else {
            continue;
        };
        transform.look_at(Vec3::new(chest.x, at.y, chest.z), Vec3::Y);
        if drone.cooldown > 0.0 {
            continue;
        }
        drone.cooldown = DRONE_RATE;
        fx.write(FxEvent::Shot { by: DRONE_SHOOTER, sound: 1, from: at - Vec3::Y * 0.3, to: chest, beam: false, tracer: oz_core::model::Paint::Red, quiet: false, local: false });
        let name = name_of(&state, drone.owner);
        if let Some((_, mut health, mut player)) = players.iter_mut().find(|(_, _, p)| p.id == victim) {
            damage_player(&mut health, &mut player, DRONE_DAMAGE, Some((&name, drone.team, drone.owner)), &mut state, &mut fx);
        }
    }
}

/// Authority: steers drone strikes onto their targets (straight on once a target is gone).
pub fn steer_strikes(time: Res<Time>, mut shots: Query<(&mut crate::combat::Projectile, &Transform, &Homing)>, players: Query<(&Player, &Transform, &Health)>) {
    let dt = sim_dt(&time);
    for (mut shot, transform, homing) in &mut shots {
        let Some(chest) = players.iter().find(|(p, _, h)| p.id == homing.target && h.alive()).map(|(_, t, _)| t.translation + Vec3::Y * 1.0) else {
            continue;
        };
        shot.velocity = steer(shot.velocity, chest - transform.translation, homing.speed, homing.turn * dt);
    }
}

/// Turns `velocity` toward `to` by at most `max_turn` radians, at `speed`.
pub fn steer(velocity: Vec3, to: Vec3, speed: f32, max_turn: f32) -> Vec3 {
    let now = velocity.normalize_or(Vec3::NEG_Y);
    let want = to.normalize_or(now);
    let angle = now.angle_between(want);
    let dir = if angle <= max_turn || angle < 1e-4 {
        want
    } else {
        let axis = now.cross(want).normalize_or(Vec3::X);
        Quat::from_axis_angle(axis, max_turn) * now
    };
    dir * speed
}

/// Authority: bots call in what they earn - strikes while they are in a firefight, the rest
/// as soon as they have it (a sentry where they stand, an EMP or a radar plane any time).
pub fn bots_call_in(state: Res<MatchState>, bots: Query<(&Player, &PlayerInput, &Health), With<crate::bots::Bot>>, mut orders: EventWriter<CommandEvent>) {
    if state.phase != Phase::Live {
        return;
    }
    for (player, input, health) in &bots {
        let Some(def) = next(&player.rewards) else {
            continue;
        };
        let fighting = input.held(crate::player::buttons::FIRE);
        if health.alive() && (!matches!(def.id, reward::AIRSTRIKE | reward::ARTILLERY | reward::DRONE_STRIKE) || fighting) {
            orders.write(CommandEvent { player: player.id, command: PlayerCommand::UseStreak });
        }
    }
}

/// A bot's killstreak picks: three different ones, cheapest first.
pub fn bot_streaks(rng: &mut oz_core::rng::Rng) -> [u8; 3] {
    let mut ids: Vec<u8> = STREAKS.iter().map(|s| s.id).collect();
    let mut out = [0u8; 3];
    for slot in &mut out {
        let pick = ids.remove(rng.below(ids.len()));
        *slot = pick;
    }
    out.sort_by_key(|id| def(*id).map_or(0, |d| d.kills));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewards_stack_newest_first_and_hold_three() {
        let mut stack = [0u8; STACK];
        assert!(next(&stack).is_none());
        push(&mut stack, reward::UAV);
        push(&mut stack, reward::AIRSTRIKE);
        assert_eq!(next(&stack).map(|s| s.id), Some(reward::AIRSTRIKE));
        push(&mut stack, reward::DRONE);
        push(&mut stack, reward::ARTILLERY);
        // Full: the oldest (the UAV) fell off.
        assert_eq!(stack, [reward::AIRSTRIKE, reward::DRONE, reward::ARTILLERY]);
        assert_eq!(pop(&mut stack), Some(reward::ARTILLERY));
        assert_eq!(pop(&mut stack), Some(reward::DRONE));
        assert_eq!(name(reward::DRONE), "ATTACK DRONE");
        assert_eq!(chosen([0, 0, 0]), DEFAULT_STREAKS);
        assert_eq!(chosen([reward::SUPPLY, reward::SUPPLY, 99]), [reward::SUPPLY, 0, 0]);
    }
}

/// Authority: rockets and grenades that reach an enemy aircraft hurt it (bullets are handled in
/// `weapons`), and anything shot down falls out of the sky.
#[allow(clippy::too_many_arguments)]
pub fn shoot_down(
    mut commands: Commands,
    mut state: ResMut<MatchState>,
    shots: Query<(Entity, &Transform, &crate::combat::Projectile), Without<Aircraft>>,
    mut aircraft: Query<(Entity, &Transform, &mut Aircraft, Option<&Recon>)>,
    mut fx: EventWriter<FxEvent>,
) {
    use oz_core::content::ShotLook;
    for (shot_entity, at, shot) in &shots {
        if shot.owner.is_none() || shot.damage <= 0.0 || matches!(shot.look, ShotLook::Bomb | ShotLook::Shell | ShotLook::Drone) {
            continue;
        }
        for (_, plane, mut craft, _) in &mut aircraft {
            if craft.team != shot.team && craft.hp > 0.0 && plane.translation.distance(at.translation) < craft.radius + 0.6 {
                craft.hp -= shot.damage.max(120.0);
                craft.hit_by = shot.owner;
                fx.write(FxEvent::Explosion { at: at.translation, radius: shot.splash.max(2.0) });
                commands.entity(shot_entity).despawn();
                break;
            }
        }
    }
    for (entity, plane, craft, recon) in &mut aircraft {
        if craft.hp > 0.0 {
            continue;
        }
        let what = match craft.look {
            ShotLook::Jet => "AIRSTRIKE JET",
            ShotLook::RadarPlane if recon.is_some_and(|r| r.advanced) => "ADVANCED UAV",
            ShotLook::RadarPlane => "RADAR PLANE",
            ShotLook::Sentry => "SENTRY GUN",
            _ => "ATTACK DRONE",
        };
        let by = craft.hit_by.and_then(|id| state.roster.iter().find(|r| r.id == id)).map_or(String::new(), |r| format!("{} SHOT DOWN ", r.name));
        fx.write(FxEvent::notice(format!("{by}{what}")));
        fx.write(FxEvent::Explosion { at: plane.translation, radius: if matches!(craft.look, ShotLook::Drone | ShotLook::Sentry) { 3.0 } else { 7.0 } });
        match recon {
            Some(r) if r.advanced => state.sweep[(craft.team as usize).min(1)] = 0.0,
            Some(_) => state.uav[(craft.team as usize).min(1)] = 0.0,
            None => {}
        }
        commands.entity(entity).despawn();
    }
}
