//! Snap-to-cover, in the style of the third-person cover shooters.
//!
//! Press **Take cover** facing a wall within a couple of metres and you snap
//! onto it, back to the wall, with the camera pulled out over your shoulder.
//! Left / right moves you along the wall and stops you at its edge. Behind
//! low cover you crouch. Press it again, or pull away from the wall, to snap off.
//!
//! Aiming from cover goes back to first person: behind low cover you pop up
//! over it; at a corner you lean out around it. Let go and you are back behind
//! the wall. Firing *without* aiming is blind fire: only the gun goes over
//! the top or round the corner - your head stays safe, your aim does not.
//! Jump behind low cover (or a window sill) to vault over it.
//!
//! All of this only shapes the local player's [`PlayerInput`] - movement,
//! stance and the peek offset (`lean`) - which the simulation already
//! replicates and uses for hitboxes, so whatever you show of yourself can be
//! shot, online included.

use crate::{ClientSet, input::Controls, ui::Screen};
use bevy::prelude::*;
use oz_core::collide::StaticWorld;
use oz_sim::prelude::*;

/// Furthest wall you can take cover against.
const REACH: f32 = 2.2;
/// Gap between your centre and the wall while in cover.
const OFFSET: f32 = 0.38;
/// Walls lower than this (above your feet) are low cover: crouch, pop up to shoot.
const LOW: f32 = 1.35;
/// How far past your centre the wall must continue for you to keep moving that way.
const EDGE: f32 = 0.25;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wall {
    /// Out of the wall, toward the player (horizontal, unit length).
    pub normal: Vec3,
    pub low: bool,
}

#[derive(Resource, Default)]
pub struct Cover {
    /// Behind this wall right now.
    pub wall: Option<Wall>,
    /// Sliding into cover: where to, and how long we have been trying.
    entering: Option<(Vec3, Wall, f32)>,
    /// A wall is close enough to take cover against (for the prompt).
    pub available: bool,
    /// Aiming out of cover (first-person view while it lasts).
    pub aiming: bool,
    /// Which edges of the wall are within reach: -1 left, 1 right (along the wall, facing out).
    pub edges: [bool; 2],
}

impl Cover {
    /// The camera should be over the shoulder.
    pub fn third_person(&self) -> bool {
        (self.wall.is_some() || self.entering.is_some()) && !self.aiming
    }
}

pub struct CoverPlugin;

impl Plugin for CoverPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Cover>()
            .add_systems(OnEnter(MatchPhase::Active), |mut cover: ResMut<Cover>| *cover = Cover::default())
            .add_systems(Update, (take_cover, mark_cover).chain().in_set(ClientSet::Input).after(crate::input::DrivePlayer).run_if(in_state(MatchPhase::Active)));
    }
}

/// Tells everyone which way the wall at your back faces, so your robot sits with its back
/// to it (see `avatar`); aiming out of cover turns it to face the shot.
fn mark_cover(cover: Res<Cover>, mut players: Query<&mut PlayerInput, With<LocalPlayer>>) {
    let Ok(mut input) = players.single_mut() else { return };
    let wall = cover.wall.or(cover.entering.map(|(_, wall, _)| wall));
    let code = wall.map_or(0, |w| oz_sim::player::cover_code(w.normal));
    if input.cover != code {
        input.cover = code;
    }
}

/// Horizontal ray from `origin`: distance and the face it hit, if that face is a wall.
fn wall_hit(statics: &StaticWorld, origin: Vec3, dir: Vec3, reach: f32) -> Option<(f32, Vec3, f32)> {
    let (d, index) = statics.raycast(origin, dir, reach)?;
    let aabb = statics.aabb(index)?;
    let at = origin + dir * d;
    let local = at - aabb.center;
    // Which side face was hit: the one the point lies on.
    let normal = if (local.x.abs() - aabb.half.x).abs() < 0.06 {
        Vec3::X * local.x.signum()
    } else if (local.z.abs() - aabb.half.z).abs() < 0.06 {
        Vec3::Z * local.z.signum()
    } else {
        return None;
    };
    Some((d, normal, aabb.top()))
}

/// The nearest wall in front of the player worth taking cover against.
fn find_wall(statics: &StaticWorld, feet: Vec3, yaw: f32) -> Option<(Vec3, Wall)> {
    let chest = feet + Vec3::Y * 0.9;
    let mut best: Option<(f32, Vec3, Wall)> = None;
    for offset in [0.0f32, 0.3, -0.3, 0.6, -0.6] {
        let dir = Quat::from_rotation_y(yaw + offset) * Vec3::NEG_Z;
        let Some((d, normal, top)) = wall_hit(statics, chest, dir, REACH) else {
            continue;
        };
        // Must be at least waist high, and face us.
        if top < feet.y + 0.75 || normal.dot(dir) > -0.3 {
            continue;
        }
        if best.is_none_or(|(bd, ..)| d < bd) {
            let at = chest + dir * d;
            // Stand off the face along its normal.
            let spot = Vec3::new(at.x, feet.y, at.z) + normal * OFFSET;
            best = Some((d, spot, Wall { normal, low: top < feet.y + LOW }));
        }
    }
    best.map(|(_, spot, wall)| (spot, wall))
}

/// Is there still wall behind the player at `point` (chest height)?
fn wall_behind(statics: &StaticWorld, point: Vec3, wall: &Wall) -> Option<f32> {
    wall_hit(statics, point, -wall.normal, OFFSET + 0.5).filter(|(_, n, _)| n.dot(wall.normal) > 0.9).map(|(_, _, top)| top)
}

/// World direction -> the `movement` vector `PlayerInput` wants (x right, y forward).
fn to_input(yaw: f32, world: Vec3) -> Vec2 {
    let forward = Vec3::new(-yaw.sin(), 0.0, -yaw.cos());
    let right = Vec3::new(yaw.cos(), 0.0, -yaw.sin());
    Vec2::new(world.dot(right), world.dot(forward)).clamp_length_max(1.0)
}

#[allow(clippy::too_many_arguments)]
fn take_cover(
    time: Res<Time>,
    controls: Res<Controls>,
    screen: Res<Screen>,
    statics: Res<Statics>,
    mut cover: ResMut<Cover>,
    mut players: Query<(&Transform, &mut PlayerInput, &Health), With<LocalPlayer>>,
) {
    let Ok((transform, mut input, health)) = players.single_mut() else {
        return;
    };
    let feet = transform.translation;
    let dt = time.delta_secs();
    if !health.alive() || *screen != Screen::None {
        if !health.alive() {
            *cover = Cover::default();
        }
        return;
    }
    let in_cover = cover.wall.is_some() || cover.entering.is_some();
    let found = if in_cover { None } else { find_wall(&statics, feet, input.yaw) };
    cover.available = found.is_some();

    // A toggle: press to snap onto the wall, press again to come off it.
    if controls.cover && in_cover {
        *cover = Cover { available: cover.available, ..default() };
        return;
    }
    if controls.cover
        && !in_cover
        && let Some((spot, wall)) = found
    {
        cover.entering = Some((spot, wall, 0.0));
    }
    // Jump: over low cover, or simply out from behind a tall wall.
    if controls.jump_pressed && in_cover {
        if let Some(wall) = cover.wall.filter(|w| w.low) {
            input.movement = to_input(input.yaw, -wall.normal);
            input.stance = stance::CROUCH;
            input.buttons |= buttons::JUMP;
            input.lean = 0;
        }
        *cover = Cover { available: cover.available, ..default() };
        return;
    }

    // Sliding in: walk ourselves to the spot.
    if let Some((spot, wall, waited)) = cover.entering {
        let to = Vec3::new(spot.x - feet.x, 0.0, spot.z - feet.z);
        if to.length() < 0.08 || waited > 0.8 {
            cover.entering = None;
            cover.wall = Some(wall);
        } else {
            cover.entering = Some((spot, wall, waited + dt));
            input.movement = to_input(input.yaw, to.normalize() * (to.length() * 4.0).min(1.0));
            input.buttons &= !buttons::SPRINT;
            input.stance = if wall.low { stance::CROUCH } else { stance::STAND };
            input.lean = 0;
            return;
        }
    }

    let Some(mut wall) = cover.wall else {
        cover.aiming = false;
        return;
    };
    let chest = feet + Vec3::Y * 0.9;
    // The wall can end or change height as we slide along it.
    let Some(top) = wall_behind(&statics, chest, &wall) else {
        *cover = Cover::default();
        return;
    };
    wall.low = top < feet.y + LOW;
    let along = Vec3::new(-wall.normal.z, 0.0, wall.normal.x);
    cover.edges = [-1.0f32, 1.0].map(|side| wall_behind(&statics, chest + along * side * EDGE, &wall).is_none());

    // Movement: only along the wall, never off its end. Pulling away leaves cover.
    let yaw = input.yaw;
    let wish = Quat::from_rotation_y(yaw) * Vec3::new(controls.movement.x, 0.0, -controls.movement.y);
    let away = wish.dot(wall.normal);
    let mut slide = wish.dot(along);
    if away > 0.75 && slide.abs() < 0.4 {
        *cover = Cover::default();
        return;
    }
    if (slide < 0.0 && cover.edges[0]) || (slide > 0.0 && cover.edges[1]) {
        slide = 0.0;
    }
    cover.aiming = controls.aim;
    input.buttons &= !buttons::SPRINT;
    // A light lean into the wall keeps us pressed against it.
    let press = -wall.normal * 0.15;

    if cover.aiming {
        input.movement = Vec2::ZERO;
        if wall.low {
            // Pop up over it.
            input.stance = stance::STAND;
            input.lean = 0;
        } else {
            // Peek out round whichever edge we are at, toward where we are looking.
            let right = Vec3::new(yaw.cos(), 0.0, -yaw.sin());
            let side = match cover.edges {
                [true, false] => -1.0,
                [false, true] => 1.0,
                [true, true] => {
                    if along.dot(Quat::from_rotation_y(yaw) * Vec3::NEG_Z) >= 0.0 {
                        1.0
                    } else {
                        -1.0
                    }
                }
                [false, false] => 0.0,
            };
            input.stance = stance::STAND;
            input.lean = if side == 0.0 { 0 } else { (along * side).dot(right).signum() as i8 };
        }
    } else {
        input.movement = to_input(yaw, along * slide * 0.8 + press);
        input.stance = if wall.low { stance::CROUCH } else { stance::STAND };
        input.lean = 0;
        if input.held(buttons::FIRE) {
            // Blind fire: the gun goes over the top or round the nearest edge, the head stays put.
            if wall.low {
                input.buttons |= buttons::BLIND;
            } else {
                let right = Vec3::new(yaw.cos(), 0.0, -yaw.sin());
                let side = match cover.edges {
                    [true, false] => -1.0,
                    [false, true] => 1.0,
                    [true, true] => {
                        if along.dot(Quat::from_rotation_y(yaw) * Vec3::NEG_Z) >= 0.0 {
                            1.0
                        } else {
                            -1.0
                        }
                    }
                    [false, false] => 0.0,
                };
                if side == 0.0 {
                    // Middle of a tall wall: nothing to shoot round.
                    input.buttons &= !buttons::FIRE;
                } else {
                    input.lean = (along * side).dot(right).signum() as i8;
                    input.buttons |= buttons::BLIND;
                    input.movement = Vec2::ZERO;
                }
            }
        }
    }
    cover.wall = Some(wall);
}

#[cfg(test)]
mod tests {
    use super::*;
    use oz_core::collide::Aabb;

    #[test]
    fn finds_walls_and_their_edges() {
        let mut statics = StaticWorld::new(50.0);
        // A long tall wall along x at z = -2, and a low crate further on.
        statics.add(Aabb::span(Vec3::new(-3.0, 0.0, -2.2), Vec3::new(3.0, 3.0, -2.0)));
        statics.add(Aabb::span(Vec3::new(10.0, 0.0, -2.5), Vec3::new(11.0, 1.0, -1.5)));
        let (spot, wall) = find_wall(&statics, Vec3::ZERO, 0.0).expect("the wall ahead");
        assert!(!wall.low && wall.normal.z > 0.9 && (spot.z - (-2.0 + OFFSET)).abs() < 0.01);
        // Behind the wall in its middle; past its end there is nothing.
        let chest = Vec3::new(0.0, 0.9, spot.z);
        assert!(wall_behind(&statics, chest, &wall).is_some());
        assert!(wall_behind(&statics, chest + Vec3::X * 3.3, &wall).is_none());
        // The crate is low cover.
        let (_, crate_wall) = find_wall(&statics, Vec3::new(10.5, 0.0, 0.0), 0.0).unwrap();
        assert!(crate_wall.low);
        // Facing away from everything: no cover.
        assert!(find_wall(&statics, Vec3::ZERO, std::f32::consts::PI).is_none());
    }

    #[test]
    fn input_vectors_follow_the_view() {
        // Facing -Z: moving +X in the world is strafing right.
        assert!(to_input(0.0, Vec3::X).abs_diff_eq(Vec2::X, 1e-5));
        assert!(to_input(0.0, Vec3::NEG_Z).abs_diff_eq(Vec2::Y, 1e-5));
    }
}
