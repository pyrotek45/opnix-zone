//! The parts of a map that change as you play: doors, glass and street
//! clutter.
//!
//! * **Doors** open and close (walk up and press Use; bots shoulder through
//!   them) and can be shot or blown off their hinges.
//! * **Windows** are glass: any bullet or blast through one shatters it.
//! * **Clutter** - bins, benches, mailboxes, crate stacks, parked cars and
//!   barrels - is cover until it is shot apart; barrels and cars explode,
//!   and one blast can set off the next.
//!
//! The state of every piece lives in [`MatchState`] (`doors`, `panes`,
//! `clutter`), so every machine - and anyone joining late - agrees on it.
//! Each machine spawns the pieces from the map and keeps their collision in
//! step with that state; only the authority changes it.

use crate::{
    combat::{Grid, Solid, blast_players, ray_box, splash},
    player::{Health, Player},
    session::{Content, Map, MatchEntity},
    state::{CommandEvent, FxEvent, MatchState, PlayerCommand},
};
use bevy::prelude::*;
use oz_core::{synth::Sfx, worldgen::DoorSpec};

pub mod door {
    pub const CLOSED: u8 = 0;
    pub const OPEN: u8 = 1;
    pub const BROKEN: u8 = 2;
}

const DOOR_HEALTH: f32 = 110.0;
const DOOR_THICK: f32 = 0.06;
/// How close you must be to work a door.
pub const DOOR_REACH: f32 = 2.2;

#[derive(Component, Clone, Copy)]
pub struct Door {
    pub index: usize,
    pub hp: f32,
}

#[derive(Component, Clone, Copy)]
pub struct Pane {
    pub index: usize,
}

#[derive(Component, Clone, Copy)]
pub struct ClutterPiece {
    pub index: usize,
    pub hp: f32,
    /// Shot to pieces but still standing: burning down to the bang (seconds left, who did it).
    pub fuse: Option<(f32, u8)>,
}

/// The clutter state value that means "burning, about to blow" (see [`MatchState::clutter`]).
pub const BURNING: u8 = 1;
/// How long a wrecked car or barrel smokes and burns before it goes up.
pub const COOK_OFF: f32 = 3.5;

/// A bullet's path through the world, for breaking what it passes.
#[derive(Event, Clone, Copy, Debug)]
pub struct Trace {
    pub origin: Vec3,
    pub dir: Vec3,
    pub reach: f32,
    pub damage: f32,
    pub by: u8,
}

/// Where a door panel sits (bottom centre) and its half extents, closed or swung open.
pub fn door_pose(spec: &DoorSpec, open: bool) -> (Vec3, Vec3) {
    let along = if spec.along_x { Vec3::X } else { Vec3::Z };
    let across = if spec.along_x { Vec3::Z } else { Vec3::X };
    let w = spec.width * 0.5;
    let h = spec.height * 0.5;
    if open {
        // Swung a quarter turn about the hinge at one end of the opening.
        let hinge = spec.center - along * w;
        (
            hinge + across * (w + DOOR_THICK) + along * DOOR_THICK,
            along * DOOR_THICK + across * w + Vec3::Y * h,
        )
    } else {
        (spec.center, along * w + across * DOOR_THICK + Vec3::Y * h)
    }
}

/// Every machine: make the pieces from the map, and size the shared state for them.
pub fn spawn_breakables(mut commands: Commands, map: Res<Map>, mut state: ResMut<MatchState>) {
    if state.doors.len() != map.doors.len() {
        state.doors = vec![door::CLOSED; map.doors.len()];
    }
    if state.panes.len() != map.windows.len() {
        state.panes = vec![0; map.windows.len()];
    }
    if state.clutter.len() != map.clutter.len() {
        state.clutter = vec![255; map.clutter.len()];
    }
    for (index, spec) in map.doors.iter().enumerate() {
        let (at, half) = door_pose(spec, false);
        commands.spawn((
            MatchEntity,
            Door {
                index,
                hp: DOOR_HEALTH,
            },
            Solid { half },
            Transform::from_translation(at),
        ));
    }
    for (index, spec) in map.windows.iter().enumerate() {
        commands.spawn((
            MatchEntity,
            Pane { index },
            Transform::from_translation(spec.center),
        ));
    }
    for (index, piece) in map.clutter.iter().enumerate() {
        commands.spawn((
            MatchEntity,
            ClutterPiece {
                index,
                hp: piece.kind.health(),
                fuse: None,
            },
            Solid { half: piece.half() },
            Transform::from_translation(piece.pos),
        ));
    }
}

/// Every machine: keep collision in step with the shared state.
pub fn sync_breakables(
    mut commands: Commands,
    map: Res<Map>,
    state: Res<MatchState>,
    mut doors: Query<(Entity, &Door, &mut Transform, Option<&mut Solid>), Without<ClutterPiece>>,
    clutter: Query<(Entity, &ClutterPiece), With<Solid>>,
) {
    if !state.is_changed() {
        return;
    }
    for (entity, door, mut transform, solid) in &mut doors {
        let Some(spec) = map.doors.get(door.index) else {
            continue;
        };
        match state.doors.get(door.index).copied().unwrap_or(door::CLOSED) {
            door::BROKEN => {
                if solid.is_some() {
                    commands.entity(entity).remove::<Solid>();
                }
            }
            status => {
                let (at, half) = door_pose(spec, status == door::OPEN);
                if transform.translation != at {
                    transform.translation = at;
                }
                match solid {
                    Some(mut solid) if solid.half != half => solid.half = half,
                    Some(_) => {}
                    None => {
                        commands.entity(entity).insert(Solid { half });
                    }
                }
            }
        }
    }
    for (entity, piece) in &clutter {
        if state.clutter.get(piece.index).is_some_and(|hp| *hp == 0) {
            commands.entity(entity).remove::<Solid>();
        }
    }
}

/// Authority: Use on a door near you opens or closes it; bots push through closed doors.
pub fn work_doors(
    map: Res<Map>,
    mut state: ResMut<MatchState>,
    mut orders: EventReader<CommandEvent>,
    players: Query<(&Player, &Transform, &Health, Has<crate::bots::Bot>)>,
    mut fx: EventWriter<FxEvent>,
) {
    let toggle = |index: usize, state: &mut MatchState, fx: &mut EventWriter<FxEvent>| {
        let Some(status) = state.doors.get_mut(index) else {
            return;
        };
        if *status == door::BROKEN {
            return;
        }
        *status = if *status == door::OPEN {
            door::CLOSED
        } else {
            door::OPEN
        };
        if let Some(spec) = map.doors.get(index) {
            fx.write(FxEvent::Sound {
                sfx: Sfx::DoorOpen,
                at: Some(spec.center + Vec3::Y),
            });
        }
    };
    for order in orders.read() {
        let PlayerCommand::Door(index) = order.command else {
            continue;
        };
        let index = index as usize;
        let near = players.iter().any(|(p, t, h, _)| {
            p.id == order.player
                && h.alive()
                && map
                    .doors
                    .get(index)
                    .is_some_and(|d| d.center.distance(t.translation) < DOOR_REACH + 0.6)
        });
        if near {
            toggle(index, &mut state, &mut fx);
        }
    }
    // Bots open any closed door they walk into.
    for (_, transform, health, bot) in &players {
        if !bot || !health.alive() {
            continue;
        }
        let close = map.doors.iter().position(|d| {
            d.center.xz().distance(transform.translation.xz()) < 0.9
                && (d.center.y - transform.translation.y).abs() < 1.0
        });
        if let Some(index) = close.filter(|i| state.doors.get(*i) == Some(&door::CLOSED)) {
            toggle(index, &mut state, &mut fx);
        }
    }
}

/// Explosions this frame, gathered for [`break_things`] (which also causes some).
#[derive(Resource, Default)]
pub struct Blasts(pub Vec<(Vec3, f32)>);

pub fn collect_blasts(mut events: EventReader<FxEvent>, mut blasts: ResMut<Blasts>) {
    blasts.0.extend(events.read().filter_map(|e| {
        if let FxEvent::Explosion { at, radius } = e {
            Some((*at, *radius))
        } else {
            None
        }
    }));
}

/// Authority: bullets break what they pass through or stop on; blasts break what is near.
#[allow(clippy::too_many_arguments)]
pub fn break_things(
    time: Res<Time>,
    map: Res<Map>,
    statics: Res<crate::session::Statics>,
    content: Res<Content>,
    grid: Res<Grid>,
    mut state: ResMut<MatchState>,
    mut traces: EventReader<Trace>,
    mut pending: ResMut<Blasts>,
    mut doors: Query<(&mut Door, &Transform, &Solid), Without<ClutterPiece>>,
    mut clutter: Query<(&mut ClutterPiece, &Transform, &Solid), Without<Door>>,
    mut players: Query<
        (&Transform, &mut Health, &mut Player),
        (
            Without<crate::combat::Projectile>,
            Without<crate::structures::Structure>,
            Without<Door>,
            Without<ClutterPiece>,
        ),
    >,
    mut hits: EventWriter<crate::combat::Hit>,
    mut fx: EventWriter<FxEvent>,
) {
    let blasts = std::mem::take(&mut pending.0);
    let mut chain: Vec<(Vec3, f32, f32, u8)> = Vec::new();
    let hurt_clutter = |piece: &mut ClutterPiece,
                        amount: f32,
                        by: u8,
                        state: &mut MatchState,
                        fx: &mut EventWriter<FxEvent>,
                        _chain: &mut Vec<(Vec3, f32, f32, u8)>| {
        let Some(spec) = map.clutter.get(piece.index) else {
            return;
        };
        if state.clutter.get(piece.index).is_none_or(|hp| *hp == 0) || piece.fuse.is_some() {
            return;
        }
        piece.hp -= amount;
        let max = spec.kind.health();
        if piece.hp <= 0.0 && spec.kind.blast().is_some() {
            // Cars and barrels do not go at once: they pour smoke, catch fire, then blow.
            piece.fuse = Some((COOK_OFF + (piece.index % 5) as f32 * 0.15, by));
            state.clutter[piece.index] = BURNING;
            fx.write(FxEvent::Sound {
                sfx: Sfx::Debris,
                at: Some(spec.center()),
            });
            return;
        }
        // Above zero health never reads as "burning" or "wrecked".
        let left = if piece.hp <= 0.0 {
            0
        } else {
            (((piece.hp / max).clamp(0.0, 1.0) * 255.0).ceil() as u8).max(BURNING + 1)
        };
        state.clutter[piece.index] = left;
        if left == 0 {
            fx.write(FxEvent::Sound {
                sfx: Sfx::Debris,
                at: Some(spec.center()),
            });
        }
    };

    // Burning wrecks counting down: when the fire reaches the tank, it goes.
    for (mut piece, ..) in &mut clutter {
        let Some((left, by)) = piece.fuse else {
            continue;
        };
        let left = left - crate::sim_dt(&time);
        if left > 0.0 {
            piece.fuse = Some((left, by));
            continue;
        }
        piece.fuse = None;
        if let Some(spec) = map.clutter.get(piece.index) {
            state.clutter[piece.index] = 0;
            fx.write(FxEvent::Sound {
                sfx: Sfx::Debris,
                at: Some(spec.center()),
            });
            if let Some((damage, radius)) = spec.kind.blast() {
                chain.push((spec.center(), damage, radius, by));
            }
        }
    }

    // Bullets: shatter glass along the way, and damage the door or clutter they stop on.
    for trace in traces.read() {
        for (index, spec) in map.windows.iter().enumerate() {
            if state.panes.get(index) != Some(&0) {
                continue;
            }
            let center = spec.center + Vec3::Y * spec.height * 0.5;
            let half = if spec.along_x {
                Vec3::new(spec.width * 0.5, spec.height * 0.5, 0.2)
            } else {
                Vec3::new(0.2, spec.height * 0.5, spec.width * 0.5)
            };
            if ray_box(trace.origin, trace.dir, center, half, trace.reach + 0.05).is_some() {
                state.panes[index] = 1;
                fx.write(FxEvent::Sound {
                    sfx: Sfx::Shatter,
                    at: Some(center),
                });
            }
        }
        let end = trace.origin + trace.dir * trace.reach;
        let near_end = |center: Vec3, half: Vec3| {
            ray_box(trace.origin, trace.dir, center, half, trace.reach + 0.1)
                .is_some_and(|d| d >= trace.reach - 0.15)
                || (end - center).abs().cmple(half + 0.1).all()
        };
        for (mut door, transform, solid) in &mut doors {
            if state.doors.get(door.index) == Some(&door::BROKEN)
                || !near_end(solid.center(transform), solid.half)
            {
                continue;
            }
            door.hp -= trace.damage;
            if door.hp <= 0.0 {
                state.doors[door.index] = door::BROKEN;
                fx.write(FxEvent::Sound {
                    sfx: Sfx::DoorBreak,
                    at: Some(solid.center(transform)),
                });
            }
        }
        for (mut piece, transform, solid) in &mut clutter {
            if near_end(solid.center(transform), solid.half) {
                hurt_clutter(
                    &mut piece,
                    trace.damage,
                    trace.by,
                    &mut state,
                    &mut fx,
                    &mut chain,
                );
            }
        }
    }

    // Bodies: vaulting through a window takes the glass with you.
    for (transform, health, _) in &players {
        if !health.alive() {
            continue;
        }
        let body = transform.translation + Vec3::Y * 0.5;
        for (index, spec) in map.windows.iter().enumerate() {
            if state.panes.get(index) != Some(&0) {
                continue;
            }
            let center = spec.center + Vec3::Y * spec.height * 0.5;
            let half = if spec.along_x {
                Vec3::new(spec.width * 0.5, spec.height * 0.5 + 0.4, 0.35)
            } else {
                Vec3::new(0.35, spec.height * 0.5 + 0.4, spec.width * 0.5)
            };
            if (body - center).abs().cmple(half).all() {
                state.panes[index] = 1;
                fx.write(FxEvent::Sound {
                    sfx: Sfx::Shatter,
                    at: Some(center),
                });
            }
        }
    }

    // Blasts: everything in reach takes a beating; glass goes for metres around.
    for (at, radius) in blasts {
        for (index, spec) in map.windows.iter().enumerate() {
            if state.panes.get(index) == Some(&0)
                && (spec.center + Vec3::Y).distance(at) < radius * 1.4
            {
                state.panes[index] = 1;
            }
        }
        for (mut door, transform, solid) in &mut doors {
            let distance = solid.center(transform).distance(at);
            if distance < radius && state.doors.get(door.index) != Some(&door::BROKEN) {
                door.hp -= 160.0 * (1.0 - distance / radius);
                if door.hp <= 0.0 {
                    state.doors[door.index] = door::BROKEN;
                    fx.write(FxEvent::Sound {
                        sfx: Sfx::DoorBreak,
                        at: Some(solid.center(transform)),
                    });
                }
            }
        }
        for (mut piece, transform, solid) in &mut clutter {
            let distance = solid.center(transform).distance(at);
            if distance < radius + solid.half.max_element() {
                hurt_clutter(
                    &mut piece,
                    150.0 * (1.0 - (distance / (radius + 1.0)).min(1.0)),
                    255,
                    &mut state,
                    &mut fx,
                    &mut chain,
                );
            }
        }
    }

    // Barrels and cars that just went up: a real explosion, which may set off the next one.
    for (at, damage, radius, by) in chain {
        fx.write(FxEvent::Explosion { at, radius });
        splash(
            &grid,
            &mut hits,
            at,
            radius,
            damage,
            (by != 255).then_some(by),
            None,
        );
        if by != 255 {
            blast_players(
                &statics,
                &content,
                &mut state,
                &mut fx,
                &mut players,
                at,
                radius,
                damage,
                255,
                by,
                None,
            );
        } else {
            // Nobody to credit: it just hurts whoever is close.
            for (transform, mut health, mut player) in &mut players {
                let distance = (transform.translation + Vec3::Y).distance(at);
                if distance < radius
                    && !crate::combat::sheltered(&statics, at, transform.translation)
                {
                    let amount = damage
                        * (1.0 - 0.7 * distance / radius)
                        * crate::combat::blast_scale(&content, &player);
                    player.last_hit = crate::player::part::BLAST;
                    crate::player::damage_player(
                        &mut health,
                        &mut player,
                        amount,
                        None,
                        &mut state,
                        &mut fx,
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doors_swing_about_their_hinge() {
        let spec = DoorSpec {
            center: Vec3::new(10.0, 0.0, 5.0),
            along_x: true,
            width: 1.7,
            height: 2.35,
        };
        let (closed, closed_half) = door_pose(&spec, false);
        let (open, open_half) = door_pose(&spec, true);
        assert_eq!(closed, spec.center);
        assert!(closed_half.x > closed_half.z && open_half.z > open_half.x);
        // The open panel sits beside the opening, at its hinge end.
        assert!(open.x < spec.center.x - 0.5 && (open.z - spec.center.z).abs() > 0.5);
    }
}
