//! Lag compensation.
//!
//! Remote players see everyone else slightly in the past: snapshots take
//! time to arrive and are then played back with a small interpolation delay.
//! Without correction, a shot that was dead-on for the shooter would miss on
//! the host, because the target has moved on since.
//!
//! The host therefore records where every player was each frame
//! ([`LagHistory`]). Each client input carries the server time of the world
//! it was looking at ([`ViewTime`]). When that client fires, the weapon code
//! asks the history where everyone was *at that moment* and tests the shot
//! against those poses - so if it was on your screen, it hits. Rewinds are
//! capped at [`MAX_REWIND`] so very laggy players cannot shoot far into the past.

use crate::{
    net::Remote,
    player::{Health, Player, PlayerInput},
};
use bevy::prelude::*;
use std::collections::VecDeque;

/// The furthest back a shot can be rewound, in seconds.
pub const MAX_REWIND: f32 = 0.35;
const KEEP: f32 = 1.0;

/// Where one player was at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub entity: Entity,
    pub feet: Vec3,
    pub stance: u8,
    pub yaw: f32,
    pub alive: bool,
    /// Where leaning moved the head (see [`crate::player::lean_offset`]).
    pub lean: Vec3,
}

#[derive(Resource, Default)]
pub struct LagHistory {
    frames: VecDeque<(f32, Vec<Pose>)>,
}

/// The server time of the world a remote player was looking at when they sent their latest input.
#[derive(Component, Clone, Copy, Debug)]
pub struct ViewTime(pub f32);

impl LagHistory {
    pub fn record(&mut self, time: f32, poses: Vec<Pose>) {
        if self.frames.back().is_some_and(|(t, _)| *t >= time) {
            return;
        }
        self.frames.push_back((time, poses));
        while self.frames.front().is_some_and(|(t, _)| *t < time - KEEP) {
            self.frames.pop_front();
        }
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Where `entity` was at `time`, interpolated between recorded frames.
    pub fn pose_at(&self, time: f32, entity: Entity) -> Option<Pose> {
        let find = |frame: &(f32, Vec<Pose>)| frame.1.iter().find(|p| p.entity == entity).copied();
        let after = self.frames.iter().position(|(t, _)| *t >= time);
        match after {
            None => self.frames.back().and_then(find),
            Some(0) => self.frames.front().and_then(find),
            Some(i) => {
                let (a, b) = (&self.frames[i - 1], &self.frames[i]);
                let (pa, pb) = (find(a), find(b));
                match (pa, pb) {
                    (Some(pa), Some(pb)) => {
                        let k = ((time - a.0) / (b.0 - a.0).max(1e-4)).clamp(0.0, 1.0);
                        // Respawns teleport: never interpolate across them.
                        if pa.feet.distance(pb.feet) > 4.0 {
                            return Some(if k < 0.5 { pa } else { pb });
                        }
                        Some(Pose { feet: pa.feet.lerp(pb.feet, k), stance: if k < 0.5 { pa.stance } else { pb.stance }, yaw: pb.yaw, alive: pa.alive && pb.alive, lean: pa.lean.lerp(pb.lean, k), ..pb })
                    }
                    (one, other) => one.or(other),
                }
            }
        }
    }

    /// Clamps a requested rewind time to what is allowed and available.
    pub fn clamp(&self, now: f32, view: f32) -> f32 {
        view.clamp(now - MAX_REWIND, now)
    }
}

/// Authority: records every player's true position this frame.
/// Remote players are recorded at their latest reported position, not the smoothed one.
pub fn record_history(time: Res<Time>, statics: Res<crate::session::Statics>, mut history: ResMut<LagHistory>, players: Query<(Entity, &Transform, &PlayerInput, &Health, Option<&Remote>), With<Player>>) {
    let poses = players
        .iter()
        .map(|(entity, transform, input, health, remote)| {
            let (feet, yaw) = remote.and_then(|r| r.latest()).unwrap_or((transform.translation, input.yaw));
            Pose { entity, feet, stance: input.stance, yaw, alive: health.alive(), lean: crate::player::lean_offset(&statics, feet, &PlayerInput { yaw, ..*input }) }
        })
        .collect();
    history.record(time.elapsed_secs(), poses);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewinds_between_frames_and_respects_teleports() {
        let mut world = World::new();
        let e = world.spawn_empty().id();
        let pose = |x: f32| Pose { entity: e, feet: Vec3::new(x, 0.0, 0.0), stance: 0, yaw: 0.0, alive: true, lean: Vec3::ZERO };
        let mut history = LagHistory::default();
        for i in 0..=10 {
            history.record(i as f32 * 0.1, vec![pose(i as f32)]);
        }
        assert!((history.pose_at(0.25, e).unwrap().feet.x - 2.5).abs() < 1e-4);
        assert_eq!(history.pose_at(-5.0, e).unwrap().feet.x, 0.0);
        assert_eq!(history.pose_at(9.0, e).unwrap().feet.x, 10.0);
        // A respawn jump is not smeared across the map.
        history.record(1.1, vec![pose(60.0)]);
        let mid = history.pose_at(1.06, e).unwrap().feet.x;
        assert!(mid == 10.0 || mid == 60.0);
        assert_eq!(history.clamp(2.0, 0.5), 2.0 - MAX_REWIND);
    }
}
