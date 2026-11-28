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
    /// Stepped out round a corner to shoot: where we stepped from, and which way.
    peek: Option<(Vec3, f32)>,
    /// How much further out (or back in) than the usual step the player has shuffled.
    peek_reach: f32,
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
            .add_systems(Update, (demo, take_cover, mark_cover, demo_photos).chain().in_set(ClientSet::Input).after(crate::input::DrivePlayer).run_if(in_state(MatchPhase::Active)));
    }
}

/// Developer photo spots: `OPNIX_COVER_DEMO=corner` or `=window` drops the local player into
/// cover at a building corner (to peek round) or under a window sill (to pop up and look out)
/// once the match is live. Handy for screenshots and for testing the cover system.
fn demo(
    statics: Res<Statics>,
    map: Res<Map>,
    state: Res<MatchState>,
    mut cover: ResMut<Cover>,
    mut players: Query<(&mut Transform, &mut PlayerInput, &Health), With<LocalPlayer>>,
    mut done: Local<bool>,
    (time, mut controls, mut clock, mut placed): (Res<Time>, ResMut<Controls>, Local<f32>, Local<Option<(Vec3, f32, Wall)>>),
) {
    let Ok(which) = std::env::var("OPNIX_COVER_DEMO") else { return };
    if state.phase != Phase::Live {
        return;
    }
    let Ok((mut transform, mut input, health)) = players.single_mut() else { return };
    if !health.alive() {
        return;
    }
    if *done {
        // Hold the spot a moment: the match's own spawn can land after the first frame.
        if let Some((feet, yaw, wall)) = *placed
            && *clock < 1.5
        {
            transform.translation = feet;
            input.yaw = yaw;
            cover.wall = Some(wall);
            if std::env::var("OPNIX_DEMO_FIRE").is_ok_and(|v| v != "seq") {
                *clock += time.delta_secs();
            }
        }
        // `OPNIX_DEMO_FIRE=1` keeps the trigger held (blind fire from cover).
        if std::env::var("OPNIX_DEMO_FIRE").is_ok_and(|v| v == "1") {
            input.buttons |= buttons::FIRE;
        }
        // `OPNIX_DEMO_FIRE=seq`: blind fire for 4 s, rest in cover 4 s, up on the sights 4 s,
        // then fire on the sights.
        if std::env::var("OPNIX_DEMO_FIRE").is_ok_and(|v| v == "seq") {
            *clock += time.delta_secs();
            let t = *clock;
            if !(4.0..8.0).contains(&t) {
                input.buttons |= buttons::FIRE;
            }
            if t >= 8.0 {
                controls.aim = true;
                input.buttons |= buttons::AIM;
            }
            if (8.0..12.0).contains(&t) {
                input.buttons &= !buttons::FIRE;
            }
        }
        return;
    }
    let dirs = [Vec3::X, Vec3::NEG_X, Vec3::Z, Vec3::NEG_Z];
    let mut spot: Option<(Vec3, Vec3, Vec3)> = None; // feet, wall normal, where to look
    if which == "low" {
        // Crouched behind a waist-high crate or wall, looking out over it.
        'o: for block in map.blocks.iter().filter(|b| b.solid && b.aabb.half.y > 0.4 && b.aabb.half.y < 0.65 && b.aabb.half.x.max(b.aabb.half.z) > 0.4) {
            let b = block.aabb;
            // Standing on its own: nothing else solid within a couple of metres.
            let near = |o: &oz_core::collide::Aabb| (o.center.x - b.center.x).abs() < o.half.x + b.half.x + 1.5 && (o.center.z - b.center.z).abs() < o.half.z + b.half.z + 1.5 && o.top() > b.bottom() + 0.1 && o.center != b.center;
            let crowded = map.blocks.iter().filter(|o| o.solid && near(&o.aabb)).count();
            if crowded > 1 && std::env::var("OPNIX_DEMO_ALONE").is_ok() {
                continue;
            }
            for normal in dirs {
                let face = b.center + normal * (b.half.dot(normal.abs()) + OFFSET + 0.02);
                let feet = Vec3::new(face.x, map.height(face.x, face.z), face.z);
                // Waist high from where we crouch (low cover, not a wall), with open ground beyond.
                let rise = b.top() - feet.y;
                if !(0.85..1.2).contains(&rise) || statics.raycast(feet + Vec3::Y, Vec3::Y, 40.0).is_some() || statics.blocked(feet.x, feet.z, feet.y, 1.2, 0.3) || statics.raycast(feet + Vec3::Y * 1.5, -normal, 25.0).is_some() {
                    continue;
                }
                let turn = std::env::var("OPNIX_DEMO_LOOK").ok().and_then(|d| d.parse::<f32>().ok()).unwrap_or(0.0).to_radians();
                spot = Some((feet, normal, feet + Quat::from_rotation_y(turn) * (-normal * 10.0) + Vec3::Y * 1.0));
                break 'o;
            }
        }
    } else if which == "ledge" {
        // Facing a low wall a step away, out of cover: the vault chevron should show.
        'l: for block in map.blocks.iter().filter(|b| b.solid && b.aabb.half.y < 0.6 && b.aabb.half.y > 0.25 && b.aabb.half.x.max(b.aabb.half.z) > 0.8) {
            let b = block.aabb;
            for normal in dirs {
                let face = b.center + normal * (b.half.dot(normal.abs()) + 0.75);
                let feet = Vec3::new(face.x, map.height(face.x, face.z), face.z);
                if (b.bottom() - feet.y).abs() > 0.2 || statics.blocked(feet.x, feet.z, feet.y, 1.75, 0.3) {
                    continue;
                }
                if oz_sim::player::vault_target(&map, &statics, feet, -normal).is_some() {
                    spot = Some((feet, normal, feet - normal * 10.0 + Vec3::Y * 0.6));
                    break 'l;
                }
            }
        }
    } else if which == "beside" {
        // Inside, back to the wall just beside a window (the wall carries on under its sill).
        'b: for window in &map.windows {
            let along = if window.along_x { Vec3::X } else { Vec3::Z };
            for normal in dirs.into_iter().filter(|n| if window.along_x { n.x == 0.0 } else { n.z == 0.0 }) {
                let floor = window.center.y - oz_core::worldgen::WINDOW_SILL;
                let feet = Vec3::new(window.center.x, floor, window.center.z) + normal * (OFFSET + 0.15) + along * (window.width * 0.5 + 0.55);
                let wall = Wall { normal, low: false };
                if statics.blocked(feet.x, feet.z, feet.y, 1.7, 0.3)
                    || statics.raycast(feet + Vec3::Y, Vec3::Y, 3.0).is_none()
                    || wall_behind(&statics, feet + Vec3::Y * 1.6, &wall).is_none()
                {
                    continue;
                }
                spot = Some((feet, normal, feet - normal * 10.0 + Vec3::Y * 1.4));
                break 'b;
            }
        }
    } else if which == "window" {
        'w: for window in &map.windows {
            for normal in dirs.into_iter().filter(|n| if window.along_x { n.x == 0.0 } else { n.z == 0.0 }) {
                let floor = window.center.y - oz_core::worldgen::WINDOW_SILL;
                let feet = Vec3::new(window.center.x, floor, window.center.z) + normal * (OFFSET + 0.15);
                // Inside: a ceiling overhead.
                if statics.blocked(feet.x, feet.z, feet.y, 1.25, 0.3) || statics.raycast(feet + Vec3::Y, Vec3::Y, 3.0).is_none() {
                    continue;
                }
                spot = Some((feet, normal, feet - normal * 10.0 + Vec3::Y * 1.2));
                break 'w;
            }
        }
    } else {
        // A tall wall face with its end (the building's corner) a metre away.
        'c: for block in map.blocks.iter().filter(|b| b.solid && b.aabb.half.y > 1.3 && b.aabb.half.x.max(b.aabb.half.z) > 1.5) {
            let b = block.aabb;
            for normal in dirs {
                let along = Vec3::new(normal.z, 0.0, -normal.x);
                let face = b.center + normal * (b.half.dot(normal.abs()) + OFFSET + 0.02);
                let end = b.half.dot(along.abs());
                let feet = face + along * (end - 0.9);
                let feet = Vec3::new(feet.x, map.height(feet.x, feet.z), feet.z);
                if feet.y + 0.2 < b.bottom() || statics.blocked(feet.x, feet.z, feet.y, 1.75, 0.3) {
                    continue;
                }
                let chest = feet + Vec3::Y * 0.9;
                let wall = Wall { normal, low: false };
                if wall_behind(&statics, chest, &wall).is_some() && wall_behind(&statics, chest + along * 1.2, &wall).is_none() {
                    spot = Some((feet, normal, feet + along * 10.0 + normal * 3.0 + Vec3::Y * 1.5));
                    break 'c;
                }
            }
        }
    }
    *done = true;
    info!("cover demo {which}: spot {spot:?}");
    let Some((feet, normal, look)) = spot else { return };
    transform.translation = feet;
    let to = look - feet;
    input.yaw = f32::atan2(-to.x, -to.z);
    if which == "ledge" {
        return;
    }
    let low = which == "window" || which == "low";
    cover.wall = Some(Wall { normal, low });
    *placed = Some((feet, input.yaw, Wall { normal, low }));
}

/// `OPNIX_SHOTS=dir`: once the match is live, saves `OPNIX_SHOT_COUNT` screenshots (default 6)
/// `OPNIX_SHOT_EVERY` seconds apart (default 0.4) after a 3 s settle, then quits.
fn demo_photos(mut commands: Commands, time: Res<Time>, state: Res<MatchState>, mut clock: Local<f32>, mut taken: Local<u32>, mut exit: EventWriter<AppExit>) {
    let Ok(dir) = std::env::var("OPNIX_SHOTS") else { return };
    if state.phase != Phase::Live {
        return;
    }
    let count = std::env::var("OPNIX_SHOT_COUNT").ok().and_then(|n| n.parse().ok()).unwrap_or(6u32);
    let every = std::env::var("OPNIX_SHOT_EVERY").ok().and_then(|n| n.parse().ok()).unwrap_or(0.4f32);
    *clock += time.delta_secs();
    let due = 3.0 + every * *taken as f32;
    if *taken >= count {
        if *clock > due + 1.0 {
            exit.write(AppExit::Success);
        }
        return;
    }
    if *clock >= due {
        let path = format!("{dir}/shot{:02}.png", *taken);
        commands.spawn(bevy::render::view::screenshot::Screenshot::primary_window()).observe(crate::devshots::save_ppm(path));
        *taken += 1;
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
    (map, camera): (Res<Map>, Query<&GlobalTransform, With<crate::view::WorldCamera>>),
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
    let x_vault = controls.vault && cover.wall.is_some_and(|w| w.low);
    if (controls.jump_pressed || x_vault) && in_cover {
        if let Some(wall) = cover.wall.filter(|w| w.low) {
            input.buttons &= !buttons::RELOAD;
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
    // Stepped out round a corner, the wall is still behind where we stepped from.
    let chest = cover.peek.map_or(feet, |(home, _)| home) + Vec3::Y * 0.9;
    // The wall can end or change height as we slide along it.
    let Some(top) = wall_behind(&statics, chest, &wall) else {
        *cover = Cover::default();
        return;
    };
    // A window right behind (open at head height over a sill) is low cover too: pop up and
    // look out through it.
    let head = chest + Vec3::Y * 0.7;
    wall.low = top < feet.y + LOW || wall_behind(&statics, head, &wall).is_none();
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
    // Going to the sights from the shoulder view: aim at what the crosshair was on, so the
    // point you were looking at stays in the middle of the screen.
    if controls.aim && !cover.aiming
        && let Ok(eye) = camera.single()
    {
        let head = feet + Vec3::Y * stance::eye(stance::STAND);
        let dir = eye.forward().as_vec3();
        // Trace from level with the head, not from the camera behind it: the stretch behind
        // us only ever finds our own wall or window frame.
        let from = eye.translation() + dir * (head - eye.translation()).dot(dir).max(0.0);
        let mut reach = map.raycast(from, dir, 150.0);
        if let Some((d, _)) = statics.raycast(from, dir, reach) {
            reach = reach.min(d);
        }
        let to = from + dir * reach - head;
        // Only a parallax nudge for something out in the world: a hit right in front of us
        // (a window frame, the edge of the wall) would swing the view round into it.
        if to.length() > 3.0 {
            let wrap = |a: f32| (a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            let yaw = f32::atan2(-to.x, -to.z);
            input.yaw += wrap(yaw - input.yaw).clamp(-0.35, 0.35);
            let pitch = (to.y / to.length()).clamp(-1.0, 1.0).asin().clamp(-1.45, 1.45);
            input.pitch += (pitch - input.pitch).clamp(-0.25, 0.25);
        }
    }
    cover.aiming = controls.aim;
    input.buttons &= !buttons::SPRINT;
    // A light lean into the wall keeps us pressed against it.
    let press = -wall.normal * 0.15;

    // Where we can look (or shoot) out: the wall's ends, or a window beside us (the wall
    // carries on under its sill, so look for openings at head height too).
    let open = |side: f32, d: f32| {
        let p = along * side * d;
        wall_behind(&statics, chest + p, &wall).is_none() || wall_behind(&statics, head + p, &wall).is_none()
    };
    let edges = [open(-1.0, EDGE), open(1.0, EDGE)];
    // The nearest opening within reach along the wall, the way we look first.
    let looking = if along.dot(Quat::from_rotation_y(yaw) * Vec3::NEG_Z) >= 0.0 { 1.0 } else { -1.0 };
    let nearest = (1..=12).find_map(|step| {
        let d = step as f32 * 0.1;
        [looking, -looking].into_iter().find(|dir| open(*dir, d)).map(|dir| (dir, d))
    });

    if cover.aiming {
        input.movement = Vec2::ZERO;
        if wall.low {
            // Pop up over it, and still shuffle along it while aiming (never off its end).
            input.stance = stance::STAND;
            input.lean = 0;
            input.movement = to_input(yaw, along * slide * 0.45 + press);
        } else {
            // Step out round whichever edge we are at, toward where we are looking (no
            // leaning in this game: the whole robot comes out past the corner).
            let side = match edges {
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
            input.lean = 0;
            if side != 0.0 && cover.peek.is_none() {
                cover.peek = Some((feet, side));
            } else if side == 0.0 && cover.peek.is_none() {
                // Not right at an end: find the nearest corner within reach along the wall
                // (either way, the one toward where we look first) and go round that.
                if let Some((dir, d)) = nearest {
                    let home = feet + along * dir * (d - EDGE).max(0.0);
                    cover.peek = Some((Vec3::new(home.x, feet.y, home.z), dir));
                }
            }
            if let Some((home, side)) = cover.peek {
                // Stepped out: the stick shuffles further out or back in a little.
                cover.peek_reach = (cover.peek_reach + slide * side * dt * 1.2).clamp(-0.3, 0.6);
                let out = home + along * side * (0.65 + cover.peek_reach) + wall.normal * 0.1;
                let to = Vec3::new(out.x - feet.x, 0.0, out.z - feet.z);
                input.movement = if to.length() > 0.05 { to_input(yaw, to.normalize() * (to.length() * 6.0).min(1.0)) } else { Vec2::ZERO };
            }
        }
    } else if let Some((home, _)) = cover.peek {
        // Back in behind the corner before moving along the wall again.
        let to = Vec3::new(home.x - feet.x, 0.0, home.z - feet.z);
        if to.length() < 0.06 {
            cover.peek = None;
            cover.peek_reach = 0.0;
        } else {
            input.movement = to_input(yaw, to.normalize() * (to.length() * 6.0).min(1.0));
        }
        input.stance = stance::STAND;
        input.lean = 0;
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
                // Round the end we are at, or else the nearest end or window within reach.
                let side = match edges {
                    [true, false] => -1.0,
                    [false, true] => 1.0,
                    [true, true] => looking,
                    [false, false] => nearest.map_or(0.0, |(dir, _)| dir),
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
