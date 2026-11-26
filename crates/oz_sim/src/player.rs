//! Players: input, first-person movement, health and respawn.

use crate::{combat::Solid, session::Map, sim_dt, state::{FxEvent, MatchState, Rules}};
use bevy::prelude::*;
use oz_core::{arsenal::WeaponState, synth::Sfx, worldgen::World as GameMap};
use serde::{Deserialize, Serialize};

/// Eye height above the feet.
pub const EYE: f32 = 1.68;
pub const WALK_SPEED: f32 = 5.4;
pub const SPRINT_SPEED: f32 = 8.2;
const JUMP_SPEED: f32 = 5.6;
const GRAVITY: f32 = 15.0;
const BODY_RADIUS: f32 = 0.3;

/// Bit flags for [`PlayerInput::buttons`].
pub mod buttons {
    pub const FIRE: u8 = 1;
    pub const AIM: u8 = 2;
    pub const RELOAD: u8 = 4;
    pub const JUMP: u8 = 8;
    pub const SPRINT: u8 = 16;
}

#[derive(Component, Clone, Debug)]
pub struct Player {
    pub id: u8,
    pub name: String,
    /// Seconds until respawn; 0 while alive.
    pub respawn: f32,
    /// Seconds since last damage (for regeneration).
    pub calm: f32,
}

/// The player entity controlled by this machine's keyboard / gamepad.
#[derive(Component)]
pub struct LocalPlayer;

/// What a player wants to do this frame. Produced by the client's input layer,
/// replicated to the host, consumed by movement and weapons.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PlayerInput {
    /// x = strafe right, y = forward; length <= 1.
    pub movement: Vec2,
    pub yaw: f32,
    pub pitch: f32,
    pub buttons: u8,
    /// Desired weapon slot.
    pub weapon: u8,
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
    /// Trigger was held last frame (semi-auto gating).
    pub trigger: bool,
    /// Total shots fired; the client watches this to play recoil.
    pub shots: u32,
}

impl Arsenal {
    pub fn state(&self) -> Option<&WeaponState> {
        self.weapons.get(self.current as usize)
    }

    pub fn owns(&self, weapon: usize) -> bool {
        self.weapons.get(weapon).is_some_and(|w| w.owned)
    }
}

/// Can a body with its feet at `feet` stand at (x, z)?
pub fn can_stand(map: &GameMap, x: f32, z: f32, feet: f32, solids: &Query<(&Transform, &Solid), Without<LocalPlayer>>) -> bool {
    let ground = map.height(x, z);
    // Walking further out to sea is blocked, walking back up the beach never is.
    if !map.in_bounds(x, z) || (ground < map.water - oz_core::worldgen::WADE_DEPTH && ground < feet - 0.02) {
        return false;
    }
    solids.iter().all(|(transform, solid)| {
        let p = solid.center(transform);
        !(feet + 1.6 > p.y - solid.half.y
            && feet + 0.35 < p.y + solid.half.y
            && (x - p.x).abs() < solid.half.x + BODY_RADIUS
            && (z - p.z).abs() < solid.half.z + BODY_RADIUS)
    })
}

/// Moves the locally controlled player. Runs on every machine, for its own player only:
/// movement is owner-authoritative so it always feels instant.
pub fn move_local_player(
    time: Res<Time>,
    map: Res<Map>,
    mut players: Query<(&mut Transform, &PlayerInput, &mut Mover, &Health), With<LocalPlayer>>,
    solids: Query<(&Transform, &Solid), Without<LocalPlayer>>,
) {
    let dt = sim_dt(&time);
    for (mut transform, input, mut mover, health) in &mut players {
        if !health.alive() {
            mover.speed = 0.0;
            continue;
        }
        let wish = input.movement.clamp_length_max(1.0);
        let sprinting = input.held(buttons::SPRINT) && wish.y > 0.3 && !input.held(buttons::AIM);
        let speed = if sprinting { SPRINT_SPEED } else if input.held(buttons::AIM) { WALK_SPEED * 0.6 } else { WALK_SPEED };
        let step = Quat::from_rotation_y(input.yaw) * Vec3::new(wish.x, 0.0, -wish.y) * speed * dt;

        let mut pos = transform.translation;
        let before = pos;
        // Axis-separated so the player slides along walls.
        if can_stand(&map, pos.x + step.x, pos.z, pos.y, &solids) {
            pos.x += step.x;
        }
        if can_stand(&map, pos.x, pos.z + step.z, pos.y, &solids) {
            pos.z += step.z;
        }

        let ground = map.height(pos.x, pos.z).max(map.water - oz_core::worldgen::WADE_DEPTH);
        if mover.grounded && input.held(buttons::JUMP) {
            mover.vertical = JUMP_SPEED;
            mover.grounded = false;
        }
        if mover.grounded && pos.y - ground < 0.45 {
            pos.y = ground; // Stick to slopes.
        } else {
            mover.vertical -= GRAVITY * dt;
            pos.y += mover.vertical * dt;
            mover.grounded = false;
            if pos.y <= ground {
                pos.y = ground;
                mover.vertical = 0.0;
                mover.grounded = true;
            }
        }
        mover.speed = (pos - before).xz().length() / dt.max(1e-4);
        transform.translation = pos;
        transform.rotation = Quat::from_rotation_y(input.yaw);
    }
}

/// Authority: regeneration, death and respawn.
pub fn vitals(
    time: Res<Time>,
    map: Res<Map>,
    rules: Res<Rules>,
    state: Res<MatchState>,
    mut players: Query<(&mut Player, &mut Health, &mut Transform)>,
    mut fx: EventWriter<FxEvent>,
) {
    let dt = sim_dt(&time);
    for (mut player, mut health, mut transform) in &mut players {
        if health.alive() {
            player.calm += dt;
            if player.calm > 5.0 && health.hp < health.max {
                health.hp = (health.hp + 9.0 * dt).min(health.max);
            }
            continue;
        }
        if player.respawn <= 0.0 {
            player.respawn = rules.respawn_seconds.max(1.0);
            fx.write(FxEvent::notice(format!("{} IS DOWN", player.name)));
            fx.write(FxEvent::Sound { sfx: Sfx::Alarm, at: None });
            continue;
        }
        player.respawn -= dt;
        if player.respawn <= 0.0 && !state.over() {
            player.respawn = 0.0;
            health.hp = health.max;
            let spawn = spawn_point(&map, player.id);
            transform.translation = spawn;
        }
    }
}

/// Spawn positions fan out around the map's spawn so players never stack.
pub fn spawn_point(map: &GameMap, id: u8) -> Vec3 {
    let offset = Vec2::from_angle(id as f32 * 2.4) * (1.2 * (id as f32).sqrt());
    let p = map.spawn + offset;
    map.ground(p.x, p.y)
}
