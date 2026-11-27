//! Robots coming apart.
//!
//! * A limb shot off ([`FxEvent::LimbLost`]) drops from the robot as a loose
//!   part, with sparks; the robot carries on without it.
//! * A normal kill knocks the robot over the way the shot pushed it and
//!   throws one more limb off.
//! * A headshot kill pops the TV head clean off.
//! * An explosion kill blows the whole robot into tumbling pieces.
//!
//! Loose parts are cheap debris: they fly, spin, bounce on the ground and
//! walls with friction, settle, and are cleared after a while. Everything
//! grows back on respawn.

use crate::{
    ClientSet,
    art::{Art, Baked},
    avatar::{PlayerRobot, RobotParts, Rig},
};
use bevy::prelude::*;
use oz_sim::{player::limb, prelude::*};

pub const HIDE_HEAD: u8 = 16;
pub const HIDE_ALL: u8 = 32;
const DEBRIS_SECONDS: f32 = 90.0;
const MAX_DEBRIS: usize = 160;

#[derive(Component)]
pub struct Debris {
    velocity: Vec3,
    spin: Vec3,
    life: f32,
    /// Roughly how far the part's centre sits above whatever it lies on.
    radius: f32,
}

pub struct WreckPlugin;

impl Plugin for WreckPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (break_apart, show_parts, tumble).chain().in_set(ClientSet::Present).after(crate::avatar::AnimateRigs).run_if(in_state(MatchPhase::Active)));
    }
}

/// One part to throw: which mesh, from which joint, and how hard.
fn throw(commands: &mut Commands, art: &Art, baked: &Baked, from: &GlobalTransform, velocity: Vec3, seed: u32) {
    let mut transform = from.compute_transform();
    transform.scale = Vec3::ONE;
    let r = |k: u32| (((seed.wrapping_mul(2654435761).wrapping_add(k * 40503)) >> 8) % 1000) as f32 / 500.0 - 1.0;
    let spin = Vec3::new(r(1), r(2), r(3)) * 9.0;
    let entity = commands.spawn((MatchEntity, Debris { velocity, spin, life: DEBRIS_SECONDS, radius: 0.12 }, transform)).id();
    art.spawn(commands, entity, baked);
}

#[allow(clippy::too_many_arguments)]
fn break_apart(
    mut commands: Commands,
    mut events: EventReader<FxEvent>,
    art: Res<Art>,
    parts: Option<Res<RobotParts>>,
    mut robots: Query<(&mut Rig, &Player, &Transform), With<PlayerRobot>>,
    joints: Query<&GlobalTransform>,
    debris: Query<Entity, With<Debris>>,
) {
    let Some(parts) = parts else { return };
    // Keep the pile bounded: the oldest pieces go first.
    let count = debris.iter().len();
    if count > MAX_DEBRIS {
        for entity in debris.iter().take(count - MAX_DEBRIS) {
            commands.entity(entity).try_despawn();
        }
    }
    let positions: Vec<(u8, Vec3)> = robots.iter().map(|(_, p, t)| (p.id, t.translation)).collect();
    for event in events.read() {
        let (victim, killer, limbs, head, blast) = match event {
            FxEvent::LimbLost { player, limb, by } => (*player, *by, *limb, false, false),
            FxEvent::Killed { victim, by, head, blast } => (*victim, *by, 0, *head, *blast),
            _ => continue,
        };
        let killed = matches!(event, FxEvent::Killed { .. });
        let Some((mut rig, player, transform)) = robots.iter_mut().find(|(_, p, _)| p.id == victim) else { continue };
        let m = &parts.teams[(rig.team as usize).min(parts.teams.len() - 1)];
        // Push away from whoever did it.
        let from = positions.iter().find(|(id, _)| *id == killer).map_or(transform.translation - transform.forward().as_vec3(), |(_, at)| *at);
        let push = (transform.translation - from).with_y(0.0).normalize_or(Vec3::X);
        let seed = victim as u32 * 7919 + killer as u32 * 31 + player.deaths as u32;
        let up = Vec3::Y;
        // Limbs as (joint, mesh) pairs.
        let (arms, thighs, shins, head_joint, torso, hips) = (rig.arms, rig.thighs, rig.shins, rig.head, rig.torso, rig.hips);
        let limb_parts = |bit: u8| -> Vec<(Entity, &Baked)> {
            match bit {
                limb::LEFT_ARM => vec![(arms[0], &m.arms[0])],
                limb::RIGHT_ARM => vec![(arms[1], &m.arms[1])],
                limb::LEFT_LEG => vec![(thighs[0], &m.thigh), (shins[0], &m.shin)],
                _ => vec![(thighs[1], &m.thigh), (shins[1], &m.shin)],
            }
        };
        let mut thrown: Vec<(Entity, &Baked, Vec3)> = Vec::new();
        if limbs != 0 {
            for (joint, baked) in limb_parts(limbs) {
                thrown.push((joint, baked, push * 2.5 + up * 2.0));
            }
        }
        let mut hide = 0;
        if killed {
            rig.fall = Vec3::new(push.dot(transform.right().as_vec3()), 0.0, push.dot(transform.back().as_vec3()));
            // How it goes down: thrown back from close range, slumping from a headshot, crumpling otherwise.
            let range = transform.translation.distance(from);
            rig.death = if head {
                crate::avatar::death::SLUMP
            } else if range < 6.0 {
                crate::avatar::death::KNOCKBACK
            } else {
                crate::avatar::death::CRUMPLE
            };
            if blast {
                // Everything goes, outward and up.
                hide = HIDE_ALL;
                let mut all: Vec<(Entity, &Baked)> = vec![(head_joint, &m.head), (torso, &m.torso), (hips, &m.hips)];
                for bit in [limb::LEFT_ARM, limb::RIGHT_ARM, limb::LEFT_LEG, limb::RIGHT_LEG] {
                    if player.lost & bit == 0 {
                        all.extend(limb_parts(bit));
                    }
                }
                for (i, (joint, baked)) in all.into_iter().enumerate() {
                    let angle = i as f32 * 2.4 + seed as f32;
                    let out = Vec3::new(angle.cos(), 0.0, angle.sin()) * 4.0 + push * 3.0 + up * (5.0 + (i % 3) as f32 * 1.5);
                    thrown.push((joint, baked, out));
                }
            } else if head {
                // The set pops off its neck.
                hide = HIDE_HEAD;
                thrown.push((head_joint, &m.head, push * 3.5 + up * 4.5));
            } else {
                // One more limb comes loose as it goes down.
                let options: Vec<u8> = [limb::LEFT_ARM, limb::RIGHT_ARM, limb::LEFT_LEG, limb::RIGHT_LEG].into_iter().filter(|b| player.lost & b == 0).collect();
                if let Some(bit) = options.get(seed as usize % options.len().max(1)) {
                    hide = *bit;
                    for (joint, baked) in limb_parts(*bit) {
                        thrown.push((joint, baked, push * 2.0 + up * 2.5));
                    }
                }
            }
            rig.hidden |= hide;
        }
        for (i, (joint, baked, velocity)) in thrown.into_iter().enumerate() {
            if let Ok(global) = joints.get(joint) {
                throw(&mut commands, &art, baked, global, velocity, seed.wrapping_add(i as u32 * 97));
            }
        }
    }
}

/// Hides what has come off: limbs lost in this life, and what death took.
fn show_parts(mut robots: Query<(&mut Rig, &Player, &Health), With<PlayerRobot>>, mut visibility: Query<&mut Visibility, Without<Rig>>) {
    for (mut rig, player, health) in &mut robots {
        if health.alive() && rig.hidden != 0 && player.lost == 0 {
            // Respawned: a fresh chassis.
            rig.hidden = 0;
        }
        let gone = player.lost | rig.hidden;
        let all = gone & HIDE_ALL != 0;
        let mut set = |entity: Entity, hidden: bool| {
            if let Ok(mut v) = visibility.get_mut(entity) {
                let wanted = if hidden { Visibility::Hidden } else { Visibility::Inherited };
                if *v != wanted {
                    *v = wanted;
                }
            }
        };
        set(rig.body, all);
        set(rig.head, gone & HIDE_HEAD != 0);
        set(rig.arms[0], gone & limb::LEFT_ARM != 0);
        set(rig.arms[1], gone & limb::RIGHT_ARM != 0);
        // With no arms left, the gun drops too.
        set(rig.gun, gone & limb::ARMS == limb::ARMS);
        for side in 0..2 {
            let off = gone & [limb::LEFT_LEG, limb::RIGHT_LEG][side] != 0;
            set(rig.thighs[side], off);
            set(rig.shins[side], off);
        }
    }
}

/// Loose parts fly, spin, bounce and settle.
fn tumble(time: Res<Time>, map: Res<Map>, statics: Res<Statics>, mut commands: Commands, mut parts: Query<(Entity, &mut Transform, &mut Debris)>) {
    let dt = time.delta_secs().min(0.05);
    for (entity, mut transform, mut debris) in &mut parts {
        debris.life -= dt;
        if debris.life <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        // Sink away for the last second.
        if debris.life < 1.0 {
            transform.translation.y -= dt * 0.3;
            continue;
        }
        debris.velocity.y -= 16.0 * dt;
        let from = transform.translation;
        let step = debris.velocity * dt;
        // Walls: bounce off.
        let length = step.length();
        if length > 1e-4
            && statics.raycast(from, step / length, length + debris.radius).is_some() {
                debris.velocity = Vec3::new(-debris.velocity.x * 0.4, debris.velocity.y, -debris.velocity.z * 0.4);
                debris.spin *= 0.6;
                continue;
            }
        let mut to = from + step;
        let ground = map.height(to.x, to.z).max(statics.support(to.x, to.z, from.y + 0.2)) + debris.radius;
        if to.y <= ground {
            to.y = ground;
            if debris.velocity.y < -1.5 {
                // Bounce, losing most of the energy.
                debris.velocity.y *= -0.32;
                debris.velocity.x *= 0.6;
                debris.velocity.z *= 0.6;
                debris.spin *= 0.55;
            } else {
                // Resting: slide to a stop.
                debris.velocity = Vec3::new(debris.velocity.x * 0.85, 0.0, debris.velocity.z * 0.85);
                debris.spin *= 0.8;
            }
        }
        transform.translation = to;
        let spin = debris.spin * dt;
        transform.rotate(Quat::from_euler(EulerRot::XYZ, spin.x, spin.y, spin.z));
    }
}
