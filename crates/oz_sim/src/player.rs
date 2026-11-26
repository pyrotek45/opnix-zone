//! Players: input, stances, first-person movement over terrain and
//! buildings, health, burning and respawn.

use crate::{
    bots::Bot,
    combat::Solid,
    session::{Content, Map, Statics},
    sim_dt,
    state::{FxEvent, MatchState, Phase, Rules},
};
use bevy::prelude::*;
use oz_core::{
    arsenal::{WeaponState, resolve},
    collide::StaticWorld,
    content::{Catalog, PerkEffect, UpgradeEffect},
    synth::Sfx,
    worldgen::{WADE_DEPTH, World as GameMap},
};
use serde::{Deserialize, Serialize};

/// Standing eye height above the feet.
pub const EYE: f32 = 1.68;
pub const WALK_SPEED: f32 = 5.4;
pub const SPRINT_SPEED: f32 = 8.2;
const JUMP_SPEED: f32 = 6.2;
const GRAVITY: f32 = 15.0;
const BODY_RADIUS: f32 = 0.3;

/// Bit flags for [`PlayerInput::buttons`].
pub mod buttons {
    pub const FIRE: u8 = 1;
    pub const AIM: u8 = 2;
    pub const RELOAD: u8 = 4;
    pub const JUMP: u8 = 8;
    pub const SPRINT: u8 = 16;
    /// Throw / use equipment.
    pub const GEAR: u8 = 32;
}

/// How a body is holding itself. Stored in [`PlayerInput::stance`].
pub mod stance {
    pub const STAND: u8 = 0;
    pub const CROUCH: u8 = 1;
    pub const PRONE: u8 = 2;

    /// Eye height for a stance.
    pub fn eye(stance: u8) -> f32 {
        [super::EYE, 1.12, 0.42][(stance as usize).min(2)]
    }

    /// Headroom a stance needs.
    pub fn height(stance: u8) -> f32 {
        [1.75, 1.25, 0.6][(stance as usize).min(2)]
    }

    /// Movement speed multiplier: crouching is slow, crawling slower.
    pub fn speed(stance: u8) -> f32 {
        [1.0, 0.5, 0.24][(stance as usize).min(2)]
    }

    /// Spread multiplier: steadier when low, steadiest prone.
    pub fn steadiness(stance: u8) -> f32 {
        [1.0, 0.62, 0.38][(stance as usize).min(2)]
    }

    /// Recoil multiplier: braced positions kick less.
    pub fn recoil(stance: u8) -> f32 {
        [1.0, 0.72, 0.45][(stance as usize).min(2)]
    }
}

/// Presentation bits in [`Player::flags`], replicated so others see what you are doing.
pub mod flags {
    /// Two bits of stance.
    pub const STANCE: u8 = 3;
    pub const AIMING: u8 = 4;
    pub const RELOADING: u8 = 8;
    pub const SPRINTING: u8 = 16;
    /// Fired an unsuppressed weapon recently: shows on enemy radar.
    pub const LOUD: u8 = 32;
    pub const LEAN_LEFT: u8 = 64;
    pub const LEAN_RIGHT: u8 = 128;

    /// The lean direction (-1, 0, 1) encoded in a flags byte.
    pub fn lean(bits: u8) -> i8 {
        (bits & LEAN_RIGHT != 0) as i8 - (bits & LEAN_LEFT != 0) as i8
    }
}

/// How far the head moves out when leaning around cover.
pub const LEAN_REACH: f32 = 0.6;
/// Body roll at full lean, in radians (for presentation).
pub const LEAN_ROLL: f32 = 0.3;

/// Where leaning puts the eye relative to standing straight: sideways,
/// cut short by any wall in the way. Nothing while prone.
pub fn lean_offset(statics: &StaticWorld, feet: Vec3, input: &PlayerInput) -> Vec3 {
    if input.lean == 0 || input.stance == stance::PRONE {
        return Vec3::ZERO;
    }
    let side = Quat::from_rotation_y(input.yaw) * Vec3::X * input.lean.signum() as f32;
    let eye = feet + Vec3::Y * input.eye();
    let reach = statics.raycast(eye, side, LEAN_REACH + 0.2).map_or(LEAN_REACH, |(d, _)| (d - 0.2).max(0.0)).min(LEAN_REACH);
    side * reach - Vec3::Y * 0.08 * (reach / LEAN_REACH)
}

/// The point a player sees and shoots from.
pub fn eye_point(statics: &StaticWorld, feet: Vec3, input: &PlayerInput) -> Vec3 {
    feet + Vec3::Y * input.eye() + lean_offset(statics, feet, input)
}

#[derive(Component, Clone, Debug)]
pub struct Player {
    pub id: u8,
    pub name: String,
    /// 0 or 1 in versus modes; everyone is team 0 in co-op.
    pub team: u8,
    /// Seconds until respawn; 0 while alive.
    pub respawn: f32,
    /// Seconds since last damage (for regeneration).
    pub calm: f32,
    /// Seconds since the last (re)spawn.
    pub life: f32,
    pub kills: u16,
    pub deaths: u16,
    /// Lobby: ready to start.
    pub ready: bool,
    /// Index into `Catalog::perks`.
    pub perk: u8,
    /// Index into `Catalog::gear`, and uses left this life.
    pub gear: u8,
    pub gear_left: u8,
    pub gear_latch: bool,
    /// Loadout to apply at the next spawn.
    pub loadout: Option<Loadout>,
    /// Burn damage per second, seconds left, and who lit it.
    pub burn: f32,
    pub burn_time: f32,
    pub burn_by: Option<u8>,
    /// See [`flags`].
    pub flags: u8,
    /// Round trip to the host in ms, as the client reports it.
    pub ping: u16,
    /// Seconds of spawn protection left (versus). Firing ends it early.
    pub protect: f32,
    /// Kills since the last death (versus killstreaks).
    pub streak: u16,
    /// Earned, unused killstreak rewards (bits of `killstreak::reward`).
    pub rewards: u8,
    /// The hit being applied right now was to the head (for kill credits).
    pub last_hit_head: bool,
}

impl Player {
    pub fn new(id: u8, name: impl Into<String>, team: u8) -> Self {
        Self {
            id, name: name.into(), team, respawn: 0.0, calm: 0.0, life: 0.0, kills: 0, deaths: 0, ready: false, perk: 0, gear: 0, gear_left: 0,
            gear_latch: false, loadout: None, burn: 0.0, burn_time: 0.0, burn_by: None, flags: 0, ping: 0, protect: 0.0,
            streak: 0, rewards: 0, last_hit_head: false,
        }
    }

    pub fn stance(&self) -> u8 {
        self.flags & flags::STANCE
    }
}

/// A versus-mode class: two weapons, a perk and a piece of gear.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Loadout {
    pub primary: u8,
    pub secondary: u8,
    pub perk: u8,
    pub gear: u8,
}

/// Looks up the active perk's effect, if any.
pub fn perk(catalog: &Catalog, player: &Player) -> Option<PerkEffect> {
    catalog.perks.get(player.perk as usize).map(|perk| perk.effect)
}

/// Equips a loadout: exactly the two chosen weapons, full ammo, fresh gear.
pub fn apply_loadout(catalog: &Catalog, player: &mut Player, arsenal: &mut Arsenal, loadout: Loadout) {
    let valid = |index: u8| (index as usize) < catalog.weapons.len();
    if !valid(loadout.primary) || !valid(loadout.secondary) || loadout.primary == loadout.secondary {
        return;
    }
    for (index, weapon) in arsenal.weapons.iter_mut().enumerate() {
        weapon.owned = index as u8 == loadout.primary || index as u8 == loadout.secondary;
    }
    player.perk = loadout.perk.min(catalog.perks.len().saturating_sub(1) as u8);
    player.gear = loadout.gear.min(catalog.gear.len().saturating_sub(1) as u8);
    arsenal.current = loadout.primary;
    refill(catalog, player, arsenal);
}

/// Full magazines, full reserves and fresh gear, as on every spawn.
pub fn refill(catalog: &Catalog, player: &mut Player, arsenal: &mut Arsenal) {
    for (weapon, def) in arsenal.weapons.iter_mut().zip(&catalog.weapons) {
        if weapon.owned {
            weapon.ammo = resolve(def, weapon, &catalog.attachments).mag;
            weapon.reserve = weapon.reserve.max(def.reserve);
        }
    }
    arsenal.reload = 0.0;
    arsenal.burst = 0;
    player.gear_left = catalog.gear.get(player.gear as usize).map_or(0, |gear| gear.count);
}

/// Hurts a player and handles the kill: score, feed, hit marker. Returns true on a kill.
/// `attacker` is (name, team, id).
pub fn damage_player(health: &mut Health, victim: &mut Player, amount: f32, attacker: Option<(&str, u8, u8)>, state: &mut MatchState, fx: &mut EventWriter<FxEvent>) -> bool {
    if !health.alive() || state.phase == Phase::Lobby || (victim.protect > 0.0 && attacker.is_some()) {
        return false;
    }
    health.hp -= amount;
    victim.calm = 0.0;
    fx.write(FxEvent::Hurt { player: victim.id });
    let killed = !health.alive();
    if killed {
        victim.deaths += 1;
        victim.burn_time = 0.0;
        victim.streak = 0;
    }
    if let Some((name, team, id)) = attacker {
        fx.write(FxEvent::HitMarker { player: id, kill: killed });
        if killed && id != victim.id {
            state.scores[(team as usize).min(1)] += 1;
            state.credits.push(id);
            fx.write(FxEvent::notice(format!("{name}  >  {}", victim.name)));
            fx.write(FxEvent::Killed { victim: victim.id, by: id, head: victim.last_hit_head });
        }
    }
    killed
}

/// The player entity controlled by this machine's keyboard / gamepad.
#[derive(Component)]
pub struct LocalPlayer;

/// What a player wants to do this frame. Produced by the client's input layer
/// (or a bot), replicated to the host, consumed by movement and weapons.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PlayerInput {
    /// x = strafe right, y = forward; length <= 1.
    pub movement: Vec2,
    pub yaw: f32,
    pub pitch: f32,
    pub buttons: u8,
    /// Desired weapon slot.
    pub weapon: u8,
    /// See [`stance`].
    pub stance: u8,
    /// Leaning around cover: -1 left, 0 upright, 1 right.
    pub lean: i8,
}

impl PlayerInput {
    pub fn held(&self, button: u8) -> bool {
        self.buttons & button != 0
    }

    pub fn look(&self) -> Quat {
        Quat::from_euler(EulerRot::YXZ, self.yaw, self.pitch, 0.0)
    }

    pub fn aim_dir(&self) -> Vec3 {
        self.look() * Vec3::NEG_Z
    }

    pub fn eye(&self) -> f32 {
        stance::eye(self.stance)
    }
}

/// Kinematic body state for a walking character.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Mover {
    pub vertical: f32,
    pub grounded: bool,
    /// Horizontal speed last frame (for footsteps and view bob).
    pub speed: f32,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Health {
    pub hp: f32,
    pub max: f32,
}

impl Health {
    pub fn full(max: f32) -> Self {
        Self { hp: max, max }
    }

    pub fn alive(&self) -> bool {
        self.hp > 0.0
    }

    pub fn fraction(&self) -> f32 {
        if self.max > 0.0 { (self.hp / self.max).clamp(0.0, 1.0) } else { 0.0 }
    }
}

/// A player's weapons and firing state.
#[derive(Component, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Arsenal {
    /// One entry per catalog weapon.
    pub weapons: Vec<WeaponState>,
    pub current: u8,
    pub cooldown: f32,
    /// Seconds of reload remaining; 0 = not reloading.
    pub reload: f32,
    /// Total length of the reload in progress (for animation).
    pub reload_total: f32,
    /// The reload started on an empty gun (the action must be worked).
    pub reload_empty: bool,
    /// Rounds left in the current burst.
    pub burst: u8,
    /// Trigger was held last frame (semi-auto gating).
    pub trigger: bool,
    /// Total shots fired; the client watches this to play recoil and action animations.
    pub shots: u32,
    /// Seconds an unsuppressed shot keeps you on enemy radar.
    pub loud: f32,
}

impl Arsenal {
    pub fn state(&self) -> Option<&WeaponState> {
        self.weapons.get(self.current as usize)
    }

    pub fn owns(&self, weapon: usize) -> bool {
        self.weapons.get(weapon).is_some_and(|w| w.owned)
    }
}

/// Can a body of `height` with its feet at `feet` stand at (x, z)?
pub fn can_stand(map: &GameMap, statics: &StaticWorld, solids: &Query<(&Transform, &Solid), Without<Player>>, x: f32, z: f32, feet: f32, height: f32) -> bool {
    let ground = map.height(x, z);
    // Walking further out to sea is blocked, walking back up the beach never is.
    if !map.in_bounds(x, z) || (ground < map.water - WADE_DEPTH && ground < feet - 0.02) {
        return false;
    }
    if statics.blocked(x, z, feet, height, BODY_RADIUS) {
        return false;
    }
    solids.iter().all(|(transform, solid)| {
        let p = solid.center(transform);
        !(feet + height > p.y - solid.half.y
            && feet + oz_core::collide::STEP < p.y + solid.half.y
            && (x - p.x).abs() < solid.half.x + BODY_RADIUS
            && (z - p.z).abs() < solid.half.z + BODY_RADIUS)
    })
}

/// What a walker stands on at (x, z): terrain, shallow sea floor, or a floor / step / crate.
pub fn ground_under(map: &GameMap, statics: &StaticWorld, x: f32, z: f32, feet: f32) -> f32 {
    map.height(x, z).max(map.water - WADE_DEPTH).max(statics.support(x, z, feet))
}

/// Moves every player this machine controls: its own (movement is
/// owner-authoritative so it always feels instant) and, on the authority, bots.
pub fn move_players(
    time: Res<Time>,
    map: Res<Map>,
    statics: Res<Statics>,
    content: Res<Content>,
    mut players: Query<(&mut Transform, &PlayerInput, &mut Mover, &Health, &Player, &Arsenal), Or<(With<LocalPlayer>, With<Bot>)>>,
    solids: Query<(&Transform, &Solid), Without<Player>>,
) {
    let dt = sim_dt(&time);
    for (mut transform, input, mut mover, health, player, arsenal) in &mut players {
        if !health.alive() {
            mover.speed = 0.0;
            continue;
        }
        let wish = input.movement.clamp_length_max(1.0);
        let standing = input.stance == stance::STAND;
        let leaning = input.lean != 0;
        let sprinting = standing && !leaning && input.held(buttons::SPRINT) && wish.y > 0.3 && !input.held(buttons::AIM);
        let perk_speed = match perk(&content, player) {
            Some(PerkEffect::Speed(multiplier)) => multiplier,
            _ => 1.0,
        };
        let weapon_speed = content
            .weapons
            .get(arsenal.current as usize)
            .zip(arsenal.state())
            .map_or(1.0, |(def, state)| resolve(def, state, &content.attachments).mobility);
        let gait = if sprinting { SPRINT_SPEED } else if input.held(buttons::AIM) { WALK_SPEED * 0.6 } else { WALK_SPEED };
        let speed = gait * perk_speed * weapon_speed * stance::speed(input.stance) * if leaning { 0.5 } else { 1.0 };
        let step = Quat::from_rotation_y(input.yaw) * Vec3::new(wish.x, 0.0, -wish.y) * speed * dt;
        let height = stance::height(input.stance);

        let mut pos = transform.translation;
        let before = pos;
        // Axis-separated so the player slides along walls.
        if can_stand(&map, &statics, &solids, pos.x + step.x, pos.z, pos.y, height) {
            pos.x += step.x;
        }
        if can_stand(&map, &statics, &solids, pos.x, pos.z + step.z, pos.y, height) {
            pos.z += step.z;
        }

        let ground = ground_under(&map, &statics, pos.x, pos.z, pos.y);
        if mover.grounded && standing && input.held(buttons::JUMP) {
            mover.vertical = JUMP_SPEED;
            mover.grounded = false;
        }
        if mover.grounded && pos.y - ground < oz_core::collide::STEP {
            pos.y = ground; // Stick to slopes and stairs.
        } else {
            mover.vertical -= GRAVITY * dt;
            pos.y += mover.vertical * dt;
            mover.grounded = false;
            // Bump heads on ceilings.
            if mover.vertical > 0.0 && statics.blocked(pos.x, pos.z, pos.y + 0.5, height - 0.5, BODY_RADIUS * 0.7) {
                mover.vertical = 0.0;
            }
            let landing = ground_under(&map, &statics, pos.x, pos.z, pos.y.max(before.y));
            if pos.y <= landing {
                pos.y = landing;
                mover.vertical = 0.0;
                mover.grounded = true;
            }
        }
        mover.speed = (pos - before).xz().length() / dt.max(1e-4);
        transform.translation = pos;
        transform.rotation = Quat::from_rotation_y(input.yaw);
    }
}

/// Authority: what everyone else should see this player doing.
pub fn presentation_flags(mut players: Query<(&mut Player, &PlayerInput, &Arsenal, &Mover)>) {
    for (mut player, input, arsenal, mover) in &mut players {
        let mut bits = input.stance.min(2);
        if input.held(buttons::AIM) {
            bits |= flags::AIMING;
        }
        if arsenal.reload > 0.0 {
            bits |= flags::RELOADING;
        }
        if input.held(buttons::SPRINT) && mover.speed > 6.0 {
            bits |= flags::SPRINTING;
        }
        if arsenal.loud > 0.0 {
            bits |= flags::LOUD;
        }
        bits |= match input.lean {
            l if l < 0 => flags::LEAN_LEFT,
            l if l > 0 => flags::LEAN_RIGHT,
            _ => 0,
        };
        if player.flags != bits {
            player.flags = bits;
        }
    }
}

/// Authority: regeneration, burning, death and respawn.
#[allow(clippy::too_many_arguments)]
pub fn vitals(
    time: Res<Time>,
    map: Res<Map>,
    rules: Res<Rules>,
    content: Res<Content>,
    mut state: ResMut<MatchState>,
    mut players: Query<(&mut Player, &mut Health, &mut Transform, &mut Arsenal, &mut PlayerInput)>,
    mut fx: EventWriter<FxEvent>,
) {
    let dt = sim_dt(&time);
    // Where every living fighter is, so respawns can pick the safest spot.
    let fighters: Vec<(u8, Vec3)> = players.iter().filter(|(_, h, ..)| h.alive()).map(|(p, _, t, ..)| (p.team, t.translation)).collect();
    for (mut player, mut health, mut transform, mut arsenal, mut input) in &mut players {
        if health.alive() {
            player.calm += dt;
            player.life += dt;
            player.protect = (player.protect - dt).max(0.0);
            arsenal.loud = (arsenal.loud - dt).max(0.0);
            if player.burn_time > 0.0 {
                player.burn_time -= dt;
                let by = player.burn_by;
                let attacker = by.and_then(|id| state.roster.iter().find(|r| r.id == id).map(|r| (r.name.clone(), r.team, id)));
                let damage = player.burn * dt;
                damage_player(&mut health, &mut player, damage, attacker.as_ref().map(|(n, t, i)| (n.as_str(), *t, *i)), &mut state, &mut fx);
                continue;
            }
            if player.calm > 5.0 && health.hp < health.max {
                health.hp = (health.hp + 9.0 * dt).min(health.max);
            }
            continue;
        }
        if player.respawn <= 0.0 {
            player.respawn = rules.respawn_seconds.max(1.0);
            if !rules.pvp {
                fx.write(FxEvent::notice(format!("{} IS DOWN", player.name)));
                fx.write(FxEvent::Sound { sfx: Sfx::Alarm, at: None });
            }
            continue;
        }
        player.respawn -= dt;
        if player.respawn <= 0.0 && !state.over() {
            respawn(&content, &map, &rules, &state, &mut player, &mut health, &mut transform, &mut arsenal, &mut input);
            if rules.pvp {
                // Pick the base spot furthest from any enemy, and shield the player briefly.
                let enemies: Vec<Vec3> = fighters.iter().filter(|(team, _)| *team != player.team).map(|(_, at)| *at).collect();
                transform.translation = safest_spawn(&map, player.team, &enemies);
                player.protect = 2.5;
            }
        }
    }
}

/// Brings a player back at their spawn with a fresh life.
#[allow(clippy::too_many_arguments)]
pub fn respawn(content: &Catalog, map: &GameMap, rules: &Rules, state: &MatchState, player: &mut Player, health: &mut Health, transform: &mut Transform, arsenal: &mut Arsenal, input: &mut PlayerInput) {
    player.respawn = 0.0;
    player.life = 0.0;
    player.burn_time = 0.0;
    if let Some(loadout) = player.loadout.take() {
        apply_loadout(content, player, arsenal, loadout);
    } else if rules.pvp {
        refill(content, player, arsenal);
    }
    health.max = max_health(content, state, player);
    health.hp = health.max;
    transform.translation = spawn_point(map, player.id, rules.pvp.then_some(player.team));
    input.stance = stance::STAND;
}

/// The team spawn point furthest from the nearest enemy.
pub fn safest_spawn(map: &GameMap, team: u8, enemies: &[Vec3]) -> Vec3 {
    let spawns = &map.team_spawns[(team as usize).min(1)];
    spawns
        .iter()
        .map(|p| map.ground(p.x, p.y))
        .max_by(|a, b| {
            let nearest = |at: &Vec3| enemies.iter().map(|e| e.distance(*at)).fold(f32::MAX, f32::min);
            nearest(a).total_cmp(&nearest(b))
        })
        .unwrap_or_else(|| spawn_point(map, 0, Some(team)))
}

pub fn max_health(catalog: &Catalog, state: &MatchState, player: &Player) -> f32 {
    let bonus = match perk(catalog, player) {
        Some(PerkEffect::Health(extra)) => extra,
        _ => 0.0,
    };
    100.0 * catalog.bonus(&state.upgrades, UpgradeEffect::PlayerArmor) + bonus
}

/// Spawn positions: a team base in versus modes, otherwise a fan around the map's spawn.
pub fn spawn_point(map: &GameMap, id: u8, team: Option<u8>) -> Vec3 {
    if let Some(spawns) = team.map(|team| &map.team_spawns[(team as usize).min(1)]).filter(|spawns| !spawns.is_empty()) {
        let p = spawns[(id as usize / 2) % spawns.len()];
        return map.ground(p.x, p.y);
    }
    let offset = Vec2::from_angle(id as f32 * 2.4) * (1.2 * (id as f32).sqrt());
    let p = map.spawn + offset;
    map.ground(p.x, p.y)
}
