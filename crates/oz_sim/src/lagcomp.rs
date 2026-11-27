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
//!
//! In co-op the same goes for enemies: while guests are connected the host
//! also keeps every enemy's hit sphere per frame, and a guest's hitscan and
//! beam shots test against the swarm as that guest saw it.

use crate::{
    combat::{Grid, GridItem, ray_sphere},
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
    /// (time, player poses, enemy hit spheres). Enemies are only kept while guests are connected.
    frames: VecDeque<(f32, Vec<Pose>, Vec<GridItem>)>,
}

/// The server time of the world a remote player was looking at when they sent their latest input.
#[derive(Component, Clone, Copy, Debug)]
pub struct ViewTime(pub f32);

impl LagHistory {
    pub fn record(&mut self, time: f32, poses: Vec<Pose>) {
        self.record_with_enemies(time, poses, Vec::new());
    }

    pub fn record_with_enemies(&mut self, time: f32, poses: Vec<Pose>, enemies: Vec<GridItem>) {
        if self.frames.back().is_some_and(|(t, ..)| *t >= time) {
            return;
        }
        self.frames.push_back((time, poses, enemies));
        while self.frames.front().is_some_and(|(t, ..)| *t < time - KEEP) {
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
        let find = |frame: &(f32, Vec<Pose>, Vec<GridItem>)| {
            frame.1.iter().find(|p| p.entity == entity).copied()
        };
        let after = self.frames.iter().position(|(t, ..)| *t >= time);
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
                        Some(Pose {
                            feet: pa.feet.lerp(pb.feet, k),
                            stance: if k < 0.5 { pa.stance } else { pb.stance },
                            yaw: pb.yaw,
                            alive: pa.alive && pb.alive,
                            lean: pa.lean.lerp(pb.lean, k),
                            ..pb
                        })
                    }
                    (one, other) => one.or(other),
                }
            }
        }
    }

    /// The first enemy a ray hits, with every enemy where it was at `time`.
    /// `None` when nothing was recorded for then (fall back to the live grid).
    pub fn enemy_raycast(
        &self,
        time: f32,
        origin: Vec3,
        dir: Vec3,
        max: f32,
    ) -> Option<Option<(f32, GridItem)>> {
        let index = self
            .frames
            .iter()
            .position(|(t, ..)| *t >= time)
            .unwrap_or(self.frames.len().saturating_sub(1));
        let (_, _, enemies) = self.frames.get(index)?;
        if enemies.is_empty() {
            return None;
        }
        Some(
            enemies
                .iter()
                .filter_map(|item| {
                    ray_sphere(origin, dir, item.pos, item.radius)
                        .filter(|d| *d <= max)
                        .map(|d| (d, *item))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0)),
        )
    }

    /// Clamps a requested rewind time to what is allowed and available.
    pub fn clamp(&self, now: f32, view: f32) -> f32 {
        view.clamp(now - MAX_REWIND, now)
    }
}

/// Authority: records every player's true position this frame.
/// Remote players are recorded at their latest reported position, not the smoothed one.
pub fn record_history(
    time: Res<Time>,
    statics: Res<crate::session::Statics>,
    link: Res<crate::net::NetLink>,
    grid: Res<Grid>,
    mut history: ResMut<LagHistory>,
    players: Query<(Entity, &Transform, &PlayerInput, &Health, Option<&Remote>), With<Player>>,
) {
    let poses = players
        .iter()
        .map(|(entity, transform, input, health, remote)| {
            let (feet, yaw) = remote
                .and_then(|r| r.latest())
                .unwrap_or((transform.translation, input.yaw));
            Pose {
                entity,
                feet,
                stance: input.stance,
                yaw,
                alive: health.alive(),
                lean: crate::player::lean_offset(&statics, feet, &PlayerInput { yaw, ..*input }),
            }
        })
        .collect();
    // Enemies are only worth remembering when someone is shooting at them from across the network.
    let enemies = if link.remote_players() > 0 {
        grid.items()
    } else {
        Vec::new()
    };
    history.record_with_enemies(time.elapsed_secs(), poses, enemies);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewinds_between_frames_and_respects_teleports() {
        let mut world = World::new();
        let e = world.spawn_empty().id();
        let pose = |x: f32| Pose {
            entity: e,
            feet: Vec3::new(x, 0.0, 0.0),
            stance: 0,
            yaw: 0.0,
            alive: true,
            lean: Vec3::ZERO,
        };
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

    #[test]
    fn guests_shoot_enemies_where_they_saw_them() {
        let mut world = World::new();
        let e = world.spawn_empty().id();
        let mut history = LagHistory::default();
        // An enemy running along +x at 10 m/s.
        for i in 0..=10 {
            let t = i as f32 * 0.1;
            history.record_with_enemies(
                t,
                Vec::new(),
                vec![GridItem {
                    entity: e,
                    pos: Vec3::new(t * 10.0, 1.0, -20.0),
                    radius: 0.6,
                }],
            );
        }
        // Aimed at where it was 0.3 s ago: a hit then, a miss now.
        let (origin, dir) = (Vec3::new(7.0, 1.0, 0.0), Vec3::NEG_Z);
        assert!(
            history
                .enemy_raycast(0.7, origin, dir, 50.0)
                .unwrap()
                .is_some_and(|(_, item)| item.entity == e)
        );
        assert!(
            history
                .enemy_raycast(1.0, origin, dir, 50.0)
                .unwrap()
                .is_none()
        );
        // Nothing recorded (no guests): fall back to the live grid.
        assert!(
            LagHistory::default()
                .enemy_raycast(0.5, origin, dir, 50.0)
                .is_none()
        );
    }
}
