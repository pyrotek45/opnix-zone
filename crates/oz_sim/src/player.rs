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
    arsenal::{WeaponState, carried_weight, load_speed, resolve},
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
/// How far out from its rungs a climber's body hangs: enough to clear the
/// wall and the floor slabs that stick out of it.
pub const LADDER_STANDOFF: f32 = 0.45;

/// Bit flags for [`PlayerInput::buttons`].
pub mod buttons {
    pub const FIRE: u8 = 1;
    pub const AIM: u8 = 2;
    pub const RELOAD: u8 = 4;
    pub const JUMP: u8 = 8;
    pub const SPRINT: u8 = 16;
    /// Throw / use equipment. Held to cook a grenade, released to throw it.
    pub const GEAR: u8 = 32;
    /// Melee strike (gun butt).
    pub const MELEE: u8 = 64;
    /// Firing blind from cover: only the gun shows past the edge; wild aim.
    pub const BLIND: u8 = 128;
}

/// How a body is holding itself. Stored in [`PlayerInput::stance`].
pub mod stance {
    pub const STAND: u8 = 0;
    pub const CROUCH: u8 = 1;
    pub const PRONE: u8 = 2;

    /// Eye height for a stance.
    pub fn eye(stance: u8) -> f32 {
        [super::EYE, 1.02, 0.42][(stance as usize).min(2)]
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
    // Blind fire keeps the body behind cover; only the gun goes out (see [`shot_origin`]).
    if input.lean == 0 || input.stance == stance::PRONE || input.held(buttons::BLIND) {
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

/// Where shots start. Normally the eye; when firing blind from cover, the gun
/// held out round the corner (`lean` set) or up over the top of low cover.
pub fn shot_origin(statics: &StaticWorld, feet: Vec3, input: &PlayerInput) -> Vec3 {
    if !input.held(buttons::BLIND) {
        return eye_point(statics, feet, input);
    }
    let eye = feet + Vec3::Y * input.eye();
    if input.lean == 0 {
        // Arms up over the top of low cover.
        let up = statics.raycast(eye, Vec3::Y, 0.75).map_or(0.6, |(d, _)| (d - 0.1).max(0.0));
        return eye + Vec3::Y * up;
    }
    let side = Quat::from_rotation_y(input.yaw) * Vec3::X * input.lean.signum() as f32;
    let reach = statics.raycast(eye, side, 0.95).map_or(0.8, |(d, _)| (d - 0.1).max(0.0)).min(0.8);
    eye + side * reach - Vec3::Y * 0.12
}

/// Highest obstacle a body can vault (or climb onto), above the feet.
pub const VAULT_HIGH: f32 = 1.4;
const VAULT_LOW: f32 = 0.45;

/// A vault in progress: hands on the obstacle, body over it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vault {
    pub from: Vec3,
    pub to: Vec3,
    /// Height of the obstacle's top (the arc clears it).
    pub top: f32,
    pub t: f32,
    pub duration: f32,
}

/// Is there something to vault over (or climb onto) straight ahead along `dir`?
/// Returns where the body lands and the top of what it goes over: a low wall,
/// a window sill, a crate, the lip of a ledge.
pub fn vault_target(map: &GameMap, statics: &StaticWorld, feet: Vec3, dir: Vec3) -> Option<(Vec3, f32)> {
    let dir = Vec3::new(dir.x, 0.0, dir.z).normalize_or_zero();
    if dir == Vec3::ZERO {
        return None;
    }
    // Something in front at knee height, close enough to put hands on.
    let knee = feet + Vec3::Y * 0.35;
    let (d, _) = statics.raycast(knee, dir, 1.3)?;
    let probe = feet + dir * (d + 0.12);
    let top = statics.support(probe.x, probe.z, feet.y + VAULT_HIGH + 0.05);
    let rise = top - feet.y;
    if !(VAULT_LOW..=VAULT_HIGH).contains(&rise) {
        return None;
    }
    // Room for the body to go over the top (a window opening is tall enough).
    if statics.blocked(probe.x, probe.z, top + 0.05, 0.85, 0.22) {
        return None;
    }
    // Land on the far side as soon as there is somewhere to stand; failing that, on top.
    let mut onto: Option<Vec3> = None;
    let mut k = d + 0.25;
    while k <= d + 2.2 {
        let q = feet + dir * k;
        if statics.blocked(q.x, q.z, top + 0.05, 0.85, 0.22) {
            break;
        }
        let ground = ground_under(map, statics, q.x, q.z, top + 0.1);
        let clear = map.in_bounds(q.x, q.z) && !statics.blocked(q.x, q.z, ground, stance::height(stance::CROUCH), BODY_RADIUS);
        if clear && ground < top - 0.3 && ground > feet.y - 3.0 {
            return Some((Vec3::new(q.x, ground, q.z), top));
        }
        if clear && (ground - top).abs() < 0.08 && k > d + 0.4 && onto.is_none() {
            onto = Some(Vec3::new(q.x, ground, q.z));
        }
        k += 0.15;
    }
    onto.map(|at| (at, top))
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
    /// Seconds spent in the enemy base (versus): after a short grace it hurts.
    pub trespass: f32,
    /// Kills since the last death (versus killstreaks).
    pub streak: u16,
    /// Earned, unused killstreak rewards, oldest first (ids of `killstreak::reward`, 0 = empty).
    pub rewards: [u8; crate::killstreak::STACK],
    /// The killstreaks this player's class carries.
    pub streaks: [u8; 3],
    /// Vision equipment carried (see [`vision`]).
    pub vision: u8,
    /// Which part the hit being applied right now struck (see [`part`]), for kill credits.
    pub last_hit: u8,
    /// Health left in each limb: left arm, right arm, left leg, right leg.
    pub limb_hp: [f32; 4],
    /// Limbs shot off this life (see [`limb`]).
    pub lost: u8,
    /// Seconds a grenade has been cooking in hand (pin pulled), if one is.
    pub cooking: Option<f32>,
    /// Seconds until the next melee strike, and the button state last frame.
    pub melee: f32,
    pub melee_latch: bool,
    /// Reload was used to put the pin back: no reloading until it is let go.
    pub reload_block: bool,
}

/// Where a hit landed.
pub mod part {
    pub const HEAD: u8 = 0;
    pub const TORSO: u8 = 1;
    pub const LEFT_ARM: u8 = 2;
    pub const RIGHT_ARM: u8 = 3;
    pub const LEFT_LEG: u8 = 4;
    pub const RIGHT_LEG: u8 = 5;
    /// Not a body part: an explosion.
    pub const BLAST: u8 = 6;

    /// Damage multiplier for a hit on this part.
    pub fn multiplier(part: u8) -> f32 {
        match part {
            HEAD => 2.0,
            LEFT_ARM | RIGHT_ARM => 0.7,
            LEFT_LEG | RIGHT_LEG => 0.75,
            _ => 1.0,
        }
    }
}

/// Bits of [`Player::lost`].
pub mod limb {
    pub const LEFT_ARM: u8 = 1;
    pub const RIGHT_ARM: u8 = 2;
    pub const LEFT_LEG: u8 = 4;
    pub const RIGHT_LEG: u8 = 8;
    pub const ARMS: u8 = LEFT_ARM | RIGHT_ARM;
    pub const LEGS: u8 = LEFT_LEG | RIGHT_LEG;

    /// The limb bit for a [`super::part`], if it is a limb.
    pub fn of_part(part: u8) -> Option<u8> {
        (super::part::LEFT_ARM..=super::part::RIGHT_LEG).contains(&part).then(|| 1 << (part - super::part::LEFT_ARM))
    }

    /// Spread multiplier from missing arms.
    pub fn aim_penalty(lost: u8) -> f32 {
        [1.0, 1.7, 2.6][(lost & ARMS).count_ones() as usize]
    }

    /// Movement multiplier from missing legs.
    pub fn speed_penalty(lost: u8) -> f32 {
        [1.0, 0.62, 0.38][(lost & LEGS).count_ones() as usize]
    }
}

/// How much a limb can take before it comes off.
pub const LIMB_HP: f32 = 45.0;

/// Applies damage to a limb; returns true if it just came off.
pub fn wound_limb(player: &mut Player, part: u8, amount: f32, by: u8, fx: &mut EventWriter<FxEvent>) -> bool {
    let Some(bit) = limb::of_part(part) else {
        return false;
    };
    let slot = (part - part::LEFT_ARM) as usize;
    if player.lost & bit != 0 {
        return false;
    }
    player.limb_hp[slot] -= amount;
    if player.limb_hp[slot] > 0.0 {
        return false;
    }
    player.lost |= bit;
    fx.write(FxEvent::LimbLost { player: player.id, limb: bit, by });
    true
}

impl Player {
    pub fn new(id: u8, name: impl Into<String>, team: u8) -> Self {
        Self {
            id,
            name: name.into(),
            team,
            respawn: 0.0,
            calm: 0.0,
            life: 0.0,
            kills: 0,
            deaths: 0,
            ready: false,
            perk: 0,
            gear: 0,
            gear_left: 0,
            gear_latch: false,
            loadout: None,
            burn: 0.0,
            burn_time: 0.0,
            burn_by: None,
            flags: 0,
            ping: 0,
            protect: 0.0,
            trespass: 0.0,
            streak: 0,
            rewards: [0; crate::killstreak::STACK],
            streaks: crate::killstreak::DEFAULT_STREAKS,
            vision: 0,
            last_hit: part::TORSO,
            limb_hp: [LIMB_HP; 4],
            lost: 0,
            cooking: None,
            melee: 0.0,
            melee_latch: false,
            reload_block: false,
        }
    }

    pub fn stance(&self) -> u8 {
        self.flags & flags::STANCE
    }
}

/// A versus-mode class: two weapons with their attachments, a perk and a piece of gear.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Loadout {
    pub primary: u8,
    pub secondary: u8,
    pub perk: u8,
    pub gear: u8,
    /// Attachments for the primary and secondary. `None` keeps whatever the weapons have fitted.
    pub parts: Option<[[u8; oz_core::arsenal::SLOT_COUNT]; 2]>,
    /// Up to three killstreaks (ids of `killstreak::reward`; zeros = the defaults).
    pub streaks: [u8; 3],
    /// Vision equipment: see [`vision`].
    pub vision: u8,
}

/// Vision equipment a class can carry, switched on and off in the field.
pub mod vision {
    pub const NONE: u8 = 0;
    /// A green visor that brightens dark maps.
    pub const NIGHT: u8 = 1;
    /// Enemies glow hot: through smoke, in shadow, at night.
    pub const THERMAL: u8 = 2;
    pub const NAMES: [&str; 3] = ["NONE", "NIGHT VISION", "THERMAL"];
    pub const BLURBS: [&str; 3] = [
        "Nothing on the helmet: nothing to give you away.",
        "A green visor that turns night into day. Washes out in daylight.",
        "Enemy robots glow hot - through smoke, in shadow, at night.",
    ];
}

/// Looks up the active perk's effect, if any.
pub fn perk(catalog: &Catalog, player: &Player) -> Option<PerkEffect> {
    catalog.perks.get(player.perk as usize).map(|perk| perk.effect)
}

/// Kilograms a player carries right now: the gun in hand, one gun slung (the heaviest
/// other one owned, so a shop game's whole armoury never counts), the gear still left and
/// the perk's armour. Runs wherever movement does - on the host for bots, in prediction
/// for the local player - from replicated state, so both agree.
pub fn carried(catalog: &Catalog, player: &Player, arsenal: &Arsenal) -> f32 {
    let weight = |index: usize| {
        catalog.weapons.get(index).zip(arsenal.weapons.get(index)).filter(|(_, state)| state.owned).map_or(0.0, |(def, state)| carried_weight(def, &state.equipped, &catalog.attachments))
    };
    let held = arsenal.current as usize;
    let slung = (0..arsenal.weapons.len()).filter(|i| *i != held).map(weight).fold(0.0, f32::max);
    weight(held) + slung + catalog.load(&[], player.perk, player.gear, player.gear_left)
}

/// Movement speed multiplier for what a player carries (see [`oz_core::arsenal::load_speed`]).
pub fn load_factor(catalog: &Catalog, player: &Player, arsenal: &Arsenal) -> f32 {
    load_speed(carried(catalog, player, arsenal))
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
    // Fit the class's parts; anything that does not fit that gun falls back to its default.
    if let Some(parts) = loadout.parts {
        for (which, index) in [loadout.primary, loadout.secondary].into_iter().enumerate() {
            let (Some(def), Some(weapon)) = (catalog.weapons.get(index as usize), arsenal.weapons.get_mut(index as usize)) else {
                continue;
            };
            for slot in 0..oz_core::arsenal::SLOT_COUNT {
                let wanted = parts[which][slot];
                let fits = oz_core::arsenal::options(def, &catalog.attachments, slot).iter().any(|(i, ..)| *i == wanted as usize);
                weapon.equipped[slot] = if fits { wanted } else { def.defaults[slot] };
            }
        }
    }
    player.perk = loadout.perk.min(catalog.perks.len().saturating_sub(1) as u8);
    player.streaks = crate::killstreak::chosen(loadout.streaks);
    player.vision = loadout.vision.min(vision::THERMAL);
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
        fx.write(FxEvent::HitMarker { player: id, kill: killed, head: victim.last_hit == part::HEAD });
        if killed && id != victim.id {
            // Objective modes score points and rounds, not kills.
            if state.obj.kind == crate::objective::kind::NONE {
                state.scores[(team as usize).min(1)] += 1;
            }
            state.credits.push(id);
            fx.write(FxEvent::notice(format!("{name}  >  {}", victim.name)));
            fx.write(FxEvent::Killed { victim: victim.id, by: id, head: victim.last_hit == part::HEAD, blast: victim.last_hit == part::BLAST });
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
    /// In cover (presentation): 0 = no; otherwise 1 + 2 x the direction out of the wall in
    /// eighths of a turn (see [`cover_yaw`]). Others see your back against the wall.
    pub cover: u8,
    /// Flashlight on: everyone sees the beam (and hostiles see you further off in the dark).
    pub light: bool,
    /// Use held down (planting or defusing a bomb).
    #[serde(default)]
    pub interact: bool,
}

/// The facing (yaw) of a body with its back to the wall, from [`PlayerInput::cover`].
pub fn cover_yaw(code: u8) -> Option<f32> {
    (code & 1 != 0).then(|| ((code >> 1) & 7) as f32 * std::f32::consts::FRAC_PI_4)
}

/// [`PlayerInput::cover`] for a wall whose face points along `normal` (horizontal).
pub fn cover_code(normal: Vec3) -> u8 {
    let yaw = f32::atan2(-normal.x, -normal.z);
    let eighth = (yaw / std::f32::consts::FRAC_PI_4).round().rem_euclid(8.0) as u8;
    1 | (eighth << 1)
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
    /// Going over an obstacle right now.
    pub vault: Option<Vault>,
    /// Where the body was last frame (to measure remote players' speed on the host).
    pub last: Vec3,
    /// On a ladder right now (for the climbing animation).
    pub climbing: bool,
}

/// Authority: remote players move on their own machines, so measure how fast they are going
/// here (hip-fire bloom and footsteps depend on it).
pub fn measure_remote_speed(time: Res<Time>, mut players: Query<(&Transform, &mut Mover), (Without<LocalPlayer>, Without<Bot>)>) {
    let dt = sim_dt(&time).max(1e-4);
    for (transform, mut mover) in &mut players {
        let moved = (transform.translation - mover.last).xz().length() / dt;
        let moved = if moved > 30.0 { 0.0 } else { moved };
        mover.speed += (moved - mover.speed) * (1.0 - (-dt * 8.0).exp());
        mover.grounded = true;
        mover.last = transform.translation;
    }
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
        !(feet + height > p.y - solid.half.y && feet + oz_core::collide::STEP < p.y + solid.half.y && (x - p.x).abs() < solid.half.x + BODY_RADIUS && (z - p.z).abs() < solid.half.z + BODY_RADIUS)
    })
}

/// What a walker stands on at (x, z): terrain, shallow sea floor, or a floor / step / crate.
pub fn ground_under(map: &GameMap, statics: &StaticWorld, x: f32, z: f32, feet: f32) -> f32 {
    map.height(x, z).max(map.water - WADE_DEPTH).max(statics.support(x, z, feet))
}

/// Moves every player this machine controls: its own (movement is
/// owner-authoritative so it always feels instant) and, on the authority, bots.
///
/// Standing up needs headroom: under a low ceiling, a stairway or a beam
/// overhead in mid-jump you stay down until there is room.
fn headroom(statics: &StaticWorld, feet: Vec3, input: &mut PlayerInput) {
    let fits = |stance: u8| !statics.blocked(feet.x, feet.z, feet.y + 0.5, stance::height(stance) - 0.5, BODY_RADIUS * 0.7);
    if let Some(room) = (input.stance..=stance::PRONE).find(|s| fits(*s))
        && room != input.stance
    {
        input.stance = room;
    }
}

/// Applies [`headroom`] to stances chosen after movement (bots decide later
/// in the tick), so nobody shoots or is drawn standing with their head in a ceiling.
pub fn enforce_headroom(statics: Res<Statics>, mut players: Query<(&Transform, &mut PlayerInput, &Health)>) {
    for (transform, mut input, health) in &mut players {
        if health.alive() {
            headroom(&statics, transform.translation, &mut input);
        }
    }
}

pub fn move_players(
    time: Res<Time>,
    map: Res<Map>,
    statics: Res<Statics>,
    content: Res<Content>,
    mut players: Query<(&mut Transform, &mut PlayerInput, &mut Mover, &Health, &Player, &Arsenal), Or<(With<LocalPlayer>, With<Bot>)>>,
    solids: Query<(&Transform, &Solid), Without<Player>>,
) {
    let dt = sim_dt(&time);
    for (mut transform, mut input, mut mover, health, player, arsenal) in &mut players {
        if !health.alive() {
            mover.speed = 0.0;
            mover.vault = None;
            continue;
        }
        // Mid-vault: follow the arc over the obstacle, nothing else.
        if let Some(mut vault) = mover.vault {
            vault.t += dt;
            let k = (vault.t / vault.duration).min(1.0);
            let ease = k * k * (3.0 - 2.0 * k);
            let mut pos = vault.from.lerp(vault.to, ease);
            // An arc that clears the top in the middle of the move.
            let lift = (vault.top + 0.12 - vault.from.y.max(vault.to.y)).max(0.0);
            pos.y += (k * std::f32::consts::PI).sin() * lift;
            // Never dip into the obstacle on the way over.
            if (0.15..0.85).contains(&k) {
                pos.y = pos.y.max(vault.top + 0.05);
            }
            mover.speed = (pos - transform.translation).xz().length() / dt.max(1e-4);
            transform.translation = pos;
            input.stance = stance::CROUCH;
            mover.vertical = 0.0;
            mover.grounded = true;
            mover.vault = if k >= 1.0 { None } else { Some(vault) };
            if mover.vault.is_none() {
                input.stance = stance::STAND;
            }
            continue;
        }
        headroom(&statics, transform.translation, &mut input);
        // Jump at something low ahead (a wall, a sill, a crate): vault it instead.
        if mover.grounded && input.held(buttons::JUMP) && input.stance != stance::PRONE {
            let facing = Quat::from_rotation_y(input.yaw) * Vec3::NEG_Z;
            let wish = Quat::from_rotation_y(input.yaw) * Vec3::new(input.movement.x, 0.0, -input.movement.y);
            let dir = if wish.length() > 0.2 { wish } else { facing };
            if let Some((to, top)) = vault_target(&map, &statics, transform.translation, dir) {
                let distance = to.xz().distance(transform.translation.xz());
                mover.vault = Some(Vault { from: transform.translation, to, top, t: 0.0, duration: 0.38 + distance * 0.12 });
                continue;
            }
        }
        let input = &*input;
        let wish = input.movement.clamp_length_max(1.0);
        let standing = input.stance == stance::STAND;
        let leaning = input.lean != 0;
        let limping = player.lost & limb::LEGS != 0;
        let sprinting = standing && !leaning && !limping && input.held(buttons::SPRINT) && wish.y > 0.3 && !input.held(buttons::AIM);
        let perk_speed = match perk(&content, player) {
            Some(PerkEffect::Speed(multiplier)) => multiplier,
            _ => 1.0,
        };
        // Weight slows every gait; aiming a heavy gun slows you a little more on top.
        let load = load_factor(&content, player, arsenal);
        let gait = if sprinting {
            SPRINT_SPEED
        } else if input.held(buttons::AIM) {
            let held = content.weapons.get(arsenal.current as usize).zip(arsenal.state()).map_or(0.0, |(def, state)| resolve(def, state, &content.attachments).weight);
            WALK_SPEED * 0.6 * (1.0 - (held - 4.0).max(0.0) * 0.02).max(0.82)
        } else {
            WALK_SPEED
        };
        let speed = gait * perk_speed * load * stance::speed(input.stance) * if leaning { 0.5 } else { 1.0 } * limb::speed_penalty(player.lost);
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

        // Ladders: push into one to climb it; at the top, walk off onto the roof.
        let facing = Vec2::new(-input.yaw.sin(), -input.yaw.cos());
        let ladder = map.ladders.iter().find(|l| {
            let near = Vec2::new(before.x - l.foot.x, before.z - l.foot.z).length() < 0.8;
            near && before.y < l.top - 0.01 && before.y > l.foot.y - 0.6 && facing.dot(-l.out) > 0.35 && wish.y > 0.3
        });
        if let Some(ladder) = ladder {
            // On the rungs: straight up a line clear of the wall and any ledge on it,
            // no stepping sideways (or through the windows).
            let line = Vec2::new(ladder.foot.x, ladder.foot.z) + ladder.out * LADDER_STANDOFF;
            let pull = (line - before.xz()).clamp_length_max(2.0 * dt);
            pos = before + Vec3::new(pull.x, 0.0, pull.y);
            pos.y = (pos.y + 3.2 * dt).min(ladder.top + 0.02);
            mover.vertical = 0.0;
            mover.grounded = true;
            mover.speed = (pos - before).xz().length() / dt.max(1e-4);
            mover.climbing = true;
            transform.translation = pos;
            transform.rotation = Quat::from_rotation_y(input.yaw);
            continue;
        }
        mover.climbing = false;
        let ground = ground_under(&map, &statics, pos.x, pos.z, pos.y);
        if mover.grounded && standing && input.held(buttons::JUMP) {
            mover.vertical = JUMP_SPEED;
            mover.grounded = false;
        }
        // Standing on the lip of a ledge counts as standing on it.
        let ground = if mover.grounded { ground.max(statics.support_within(pos.x, pos.z, pos.y, BODY_RADIUS * 0.9)) } else { ground };
        if mover.grounded && pos.y - ground < oz_core::collide::STEP {
            pos.y = ground; // Stick to slopes and stairs.
        } else {
            mover.vertical -= GRAVITY * dt;
            pos.y += mover.vertical * dt;
            mover.grounded = false;
            // Bump heads on ceilings.
            // Bump heads on ceilings, without poking into them.
            if mover.vertical > 0.0 && statics.blocked(pos.x, pos.z, pos.y + 0.5, height - 0.5, BODY_RADIUS * 0.7) {
                mover.vertical = 0.0;
                pos.y = before.y;
            }
            // Land on whatever is under any part of the body, not just its centre:
            // coming down beside a ledge must not sink the body into it.
            let from = pos.y.max(before.y);
            let landing = ground_under(&map, &statics, pos.x, pos.z, from).max(statics.support_within(pos.x, pos.z, from, BODY_RADIUS * 0.9));
            if pos.y <= landing {
                pos.y = landing;
                mover.vertical = 0.0;
                mover.grounded = true;
            }
            // Still overlapping a wall (drifted sideways into it in mid-air): undo the drift.
            if statics.blocked(pos.x, pos.z, pos.y, height, BODY_RADIUS * 0.9) && !statics.blocked(before.x, before.z, pos.y, height, BODY_RADIUS * 0.9) {
                pos.x = before.x;
                pos.z = before.z;
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
    mut noises: ResMut<crate::bots::Noises>,
    statics: Res<crate::session::Statics>,
) {
    let dt = sim_dt(&time);
    // Where every living fighter is, so respawns can pick the safest spot.
    let fighters: Vec<(u8, Vec3)> = players.iter().filter(|(_, h, ..)| h.alive()).map(|(p, _, t, ..)| (p.team, t.translation)).collect();
    for (mut player, mut health, mut transform, mut arsenal, mut input) in &mut players {
        if health.alive() {
            // The other team's base is off limits: no camping where people spawn.
            if rules.pvp && !rules.hunt && !rules.bomb && state.phase == Phase::Live && in_enemy_base(&map, player.team, transform.translation) {
                if player.trespass == 0.0 {
                    fx.write(FxEvent::tell(player.id, "RESTRICTED AREA - LEAVE THE ENEMY BASE"));
                }
                player.trespass += dt;
                if player.trespass > 2.0 {
                    damage_player(&mut health, &mut player, 40.0 * dt, None, &mut state, &mut fx);
                }
            } else {
                player.trespass = 0.0;
            }
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
            // A robot hitting the floor carries: anyone nearby comes to look.
            noises.0.push(crate::bots::Noise { at: transform.translation, radius: 12.0, team: player.team, shout: false });
            if rules.hunt && player.team == 0 && state.phase == Phase::Live {
                state.lives = state.lives.saturating_sub(1);
            }
            if !rules.pvp {
                fx.write(FxEvent::notice(format!("{} IS DOWN", player.name)));
                fx.write(FxEvent::Sound { sfx: Sfx::Alarm, at: None });
            }
            continue;
        }
        // Terrorist hunt: hostiles stay down, and the squad only has so many lives.
        if rules.hunt && state.phase == Phase::Live && (player.team == 1 || state.lives == 0) {
            player.respawn = player.respawn.max(0.5);
            continue;
        }
        // Search and destroy: one life per round; the next round brings everyone back.
        if rules.bomb && state.phase == Phase::Live {
            player.respawn = player.respawn.max(0.5);
            continue;
        }
        player.respawn -= dt;
        if player.respawn <= 0.0 && !state.over() {
            respawn(&content, &map, &rules, &state, &mut player, &mut health, &mut transform, &mut arsenal, &mut input);
            if rules.pvp {
                // Pick the base spot furthest from any enemy, and shield the player briefly.
                let enemies: Vec<Vec3> = fighters.iter().filter(|(team, _)| *team != player.team).map(|(_, at)| *at).collect();
                transform.translation = if rules.domination {
                    crate::objective::domination_spawn(&map, &statics, player.team, player.id, &state.obj.points, &enemies)
                } else {
                    safest_spawn(&map, player.team, &enemies)
                };
                player.protect = if rules.hunt { 5.0 } else { 2.5 };
            }
        }
    }
}

/// Brings a player back at their spawn with a fresh life.
#[allow(clippy::too_many_arguments)]
pub fn respawn(
    content: &Catalog,
    map: &GameMap,
    rules: &Rules,
    state: &MatchState,
    player: &mut Player,
    health: &mut Health,
    transform: &mut Transform,
    arsenal: &mut Arsenal,
    input: &mut PlayerInput,
) {
    player.respawn = 0.0;
    player.life = 0.0;
    player.burn_time = 0.0;
    player.cooking = None;
    // A fresh chassis.
    player.limb_hp = [LIMB_HP; 4];
    player.lost = 0;
    if let Some(loadout) = player.loadout.take() {
        apply_loadout(content, player, arsenal, loadout);
    } else if rules.pvp {
        refill(content, player, arsenal);
    }
    if rules.loot {
        crate::loot::strip(content, player, arsenal);
    }
    health.max = max_health(content, state, player);
    health.hp = health.max;
    transform.translation = spawn_point(map, player.id, rules.pvp.then_some(player.team));
    input.stance = stance::STAND;
}

/// How far round a team's spawn points its base reaches (versus).
pub const BASE_RADIUS: f32 = 14.0;

/// Middle of a team's base.
pub fn base_centre(map: &GameMap, team: u8) -> Option<Vec2> {
    let spawns = &map.team_spawns[(team as usize).min(1)];
    (!spawns.is_empty()).then(|| spawns.iter().copied().sum::<Vec2>() / spawns.len() as f32)
}

/// Is `at` inside the base of `team`'s enemies?
pub fn in_enemy_base(map: &GameMap, team: u8, at: Vec3) -> bool {
    let other = 1 - team.min(1);
    base_centre(map, other).is_some_and(|c| {
        let reach = map.team_spawns[other as usize].iter().map(|s| s.distance(c)).fold(0.0, f32::max) + BASE_RADIUS * 0.6;
        at.xz().distance(c) < reach.min(BASE_RADIUS * 1.5)
    })
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
