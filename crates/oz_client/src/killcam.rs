//! Kill-cam: after you go down, watch how it happened.
//!
//! Every machine keeps a short rolling recording of what it saw: every
//! robot's position, facing, aim, stance and gun, and every shot fired. When
//! the local player dies to another player, the last few seconds play back
//! from over the killer's shoulder: stand-in robots re-enact the moves, the
//! tracers fly again, and the killing moment runs in slow motion. Jump /
//! (A) skips it. Nothing is sent over the network: it replays what this
//! machine already saw, so it works the same offline, hosting and joined.

use crate::{
    ClientSet,
    art::Art,
    avatar::{GunCache, Pose, RobotParts, spawn_robot},
    input::Controls,
    ui,
    view::WorldCamera,
};
use bevy::{pbr::NotShadowCaster, prelude::*};
use oz_core::arsenal::SLOT_COUNT;
use oz_sim::prelude::*;
use std::collections::VecDeque;

/// Seconds of history kept.
const KEEP: f32 = 9.0;
/// How much of the run-up to the death is replayed, and how much after it.
const BEFORE: f32 = 3.4;
const AFTER: f32 = 0.5;
/// Seconds after the death before the replay starts.
const DELAY: f32 = 0.75;
/// Time runs at this rate around the killing shot.
const SLOW: f32 = 0.35;
const SLOW_SPAN: f32 = 0.45;

#[derive(Clone, Copy, Debug)]
struct Sample {
    id: u8,
    team: u8,
    pos: Vec3,
    yaw: f32,
    pitch: f32,
    flags: u8,
    alive: bool,
    weapon: u8,
    parts: [u8; SLOT_COUNT],
}

struct Frame {
    t: f32,
    bodies: Vec<Sample>,
}

#[derive(Clone, Copy)]
struct ShotRecord {
    t: f32,
    from: Vec3,
    to: Vec3,
    tracer: Paint,
    beam: bool,
}

struct Replay {
    /// Recorded time now playing, and where it ends.
    clock: f32,
    end: f32,
    death: f32,
    killer: u8,
    ghosts: Vec<(u8, Entity)>,
    camera: Option<(Vec3, Quat)>,
    overlay: Entity,
}

#[derive(Resource, Default)]
pub struct KillCam {
    frames: VecDeque<Frame>,
    shots: VecDeque<ShotRecord>,
    replay: Option<Replay>,
    /// When the local player went down, if a replay may follow.
    pending: Option<f32>,
    was_alive: bool,
}

impl KillCam {
    /// A replay is on screen.
    pub fn playing(&self) -> bool {
        self.replay.is_some()
    }

    /// One robot's pose at recorded time `t`, blended between the frames around it.
    fn sample(&self, id: u8, t: f32) -> Option<Sample> {
        let after = self.frames.partition_point(|f| f.t < t);
        let find = |index: usize| {
            self.frames
                .get(index)
                .and_then(|f| f.bodies.iter().find(|b| b.id == id).map(|b| (f.t, *b)))
        };
        let (a, b) = (after.checked_sub(1).and_then(find), find(after));
        match (a, b) {
            (Some((ta, a)), Some((tb, b))) => {
                let k = ((t - ta) / (tb - ta).max(1e-4)).clamp(0.0, 1.0);
                // No sliding across the map when someone respawned between frames.
                if a.pos.distance(b.pos) > 6.0 {
                    return Some(if k < 0.5 { a } else { b });
                }
                let turn = (b.yaw - a.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
                    - std::f32::consts::PI;
                Some(Sample {
                    pos: a.pos.lerp(b.pos, k),
                    yaw: a.yaw + turn * k,
                    pitch: a.pitch + (b.pitch - a.pitch) * k,
                    ..if k < 0.5 { a } else { b }
                })
            }
            (Some((_, a)), None) => Some(a),
            (None, Some((_, b))) => Some(b),
            (None, None) => None,
        }
    }
}

/// Marks the stand-in robots and tracers of a replay.
#[derive(Component)]
struct Ghost;

#[derive(Component)]
struct ReplayTracer(f32);

pub struct KillCamPlugin;

impl Plugin for KillCamPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<KillCam>()
            .add_systems(OnEnter(MatchPhase::Active), |mut cam: ResMut<KillCam>| {
                *cam = KillCam::default()
            })
            .add_systems(
                Update,
                (record, run)
                    .chain()
                    .in_set(ClientSet::Present)
                    .after(crate::spectate::Spectating)
                    .before(crate::avatar::AnimateRigs)
                    .run_if(in_state(MatchPhase::Active)),
            );
    }
}

fn record(
    time: Res<Time>,
    state: Res<MatchState>,
    content: Res<Content>,
    mut cam: ResMut<KillCam>,
    mut events: EventReader<FxEvent>,
    players: Query<(&Player, &Transform, &PlayerInput, &Health, Option<&Arsenal>)>,
) {
    let now = time.elapsed_secs();
    for event in events.read() {
        if let FxEvent::Shot {
            from,
            to,
            tracer,
            beam,
            ..
        } = event
        {
            cam.shots.push_back(ShotRecord {
                t: now,
                from: *from,
                to: *to,
                tracer: *tracer,
                beam: *beam,
            });
        }
    }
    let bodies = players
        .iter()
        .map(|(player, transform, input, health, arsenal)| {
            let parts = arsenal
                .and_then(|a| a.weapons.get(input.weapon as usize))
                .map(|w| w.equipped)
                .or_else(|| {
                    state
                        .roster
                        .iter()
                        .find(|r| r.id == player.id && r.weapon == input.weapon)
                        .map(|r| r.parts)
                })
                .or_else(|| {
                    content
                        .weapons
                        .get(input.weapon as usize)
                        .map(|d| d.defaults)
                })
                .unwrap_or([0; SLOT_COUNT]);
            let lean = match input.lean {
                l if l < 0 => flags::LEAN_LEFT,
                l if l > 0 => flags::LEAN_RIGHT,
                _ => 0,
            };
            let flags = (player.flags & !(flags::STANCE | flags::LEAN_LEFT | flags::LEAN_RIGHT))
                | input.stance.max(player.flags & flags::STANCE)
                | if input.lean != 0 {
                    lean
                } else {
                    player.flags & (flags::LEAN_LEFT | flags::LEAN_RIGHT)
                };
            Sample {
                id: player.id,
                team: player.team,
                pos: transform.translation,
                yaw: input.yaw,
                pitch: input.pitch,
                flags,
                alive: health.alive(),
                weapon: input.weapon,
                parts,
            }
        })
        .collect();
    cam.frames.push_back(Frame { t: now, bodies });
    while cam.frames.front().is_some_and(|f| f.t < now - KEEP) {
        cam.frames.pop_front();
    }
    while cam.shots.front().is_some_and(|s| s.t < now - KEEP) {
        cam.shots.pop_front();
    }
}

#[allow(clippy::too_many_arguments)]
fn run(
    time: Res<Time>,
    controls: Res<Controls>,
    settings: Res<crate::settings::Settings>,
    death: Res<crate::fx::DeathInfo>,
    (art, parts, content, statics, state): (
        Res<Art>,
        Res<RobotParts>,
        Res<Content>,
        Res<Statics>,
        Res<MatchState>,
    ),
    mut guns: ResMut<GunCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut cam: ResMut<KillCam>,
    mut commands: Commands,
    local: Query<(&Player, &Health), With<LocalPlayer>>,
    mut robots: Query<&mut Visibility, (With<crate::avatar::PlayerRobot>, Without<Ghost>)>,
    mut ghosts: Query<(&mut Transform, &mut Pose), (With<Ghost>, Without<WorldCamera>)>,
    mut cameras: Query<
        (&mut Transform, &mut Projection),
        (With<WorldCamera>, Without<Ghost>, Without<ReplayTracer>),
    >,
    mut tracers: Query<(Entity, &mut ReplayTracer)>,
) {
    let now = time.elapsed_secs();
    let dt = time.delta_secs().min(0.05);
    for (entity, mut tracer) in &mut tracers {
        tracer.0 -= dt;
        if tracer.0 <= 0.0 {
            commands.entity(entity).despawn();
        }
    }
    let Ok((me, health)) = local.single() else {
        return;
    };

    // Watch for our own death.
    if health.alive() {
        cam.was_alive = true;
        cam.pending = None;
    } else if cam.was_alive {
        cam.was_alive = false;
        cam.pending = Some(now);
    }
    let finish = |cam: &mut KillCam,
                  commands: &mut Commands,
                  robots: &mut Query<
        &mut Visibility,
        (With<crate::avatar::PlayerRobot>, Without<Ghost>),
    >| {
        if let Some(replay) = cam.replay.take() {
            for (_, entity) in replay.ghosts {
                commands.entity(entity).despawn();
            }
            commands.entity(replay.overlay).despawn();
            for mut visibility in robots.iter_mut() {
                if *visibility == Visibility::Hidden {
                    *visibility = Visibility::Inherited;
                }
            }
        }
    };
    if health.alive() || state.phase != Phase::Live {
        finish(&mut cam, &mut commands, &mut robots);
        cam.pending = None;
    }

    // Start once we know who did it.
    if let Some(died) = cam.pending
        && cam.replay.is_none()
        && now - died >= DELAY
    {
        let killer = death.killer.filter(|k| *k != me.id);
        if let Some(killer) = killer {
            cam.pending = None;
            let start = (died - BEFORE).max(cam.frames.front().map_or(died, |f| f.t));
            // Everyone seen in the window gets a stand-in.
            let mut ids: Vec<(u8, u8, u8, [u8; SLOT_COUNT])> = Vec::new();
            for frame in cam
                .frames
                .iter()
                .filter(|f| f.t >= start && f.t <= died + AFTER)
            {
                for body in &frame.bodies {
                    match ids.iter_mut().find(|(id, ..)| *id == body.id) {
                        Some(entry) => *entry = (body.id, body.team, body.weapon, body.parts),
                        None => ids.push((body.id, body.team, body.weapon, body.parts)),
                    }
                }
            }
            if ids.iter().any(|(id, ..)| *id == killer) {
                let ghosts = ids
                    .into_iter()
                    .map(|(id, team, weapon, fitted)| {
                        let root = commands
                            .spawn((
                                MatchEntity,
                                Ghost,
                                Transform::default(),
                                Visibility::Inherited,
                            ))
                            .id();
                        let gun = guns.get(&art, &mut meshes, &content, weapon, fitted);
                        let rig =
                            spawn_robot(&mut commands, &art, &parts, root, team, gun.as_ref());
                        commands.entity(root).insert((rig, Pose::default()));
                        (id, root)
                    })
                    .collect();
                let name = state
                    .roster
                    .iter()
                    .find(|r| r.id == killer)
                    .map_or("?".to_string(), |r| r.name.clone());
                let weapon = cam
                    .sample(killer, died)
                    .and_then(|s| content.weapons.get(s.weapon as usize))
                    .map_or("", |w| w.name);
                let skip = if controls.gamepad {
                    "(A)".to_string()
                } else {
                    settings.bindings.label(crate::bindings::Action::Jump)
                };
                let overlay = overlay(&mut commands, &name, weapon, &skip);
                cam.replay = Some(Replay {
                    clock: start,
                    end: died + AFTER,
                    death: died,
                    killer,
                    ghosts,
                    camera: None,
                    overlay,
                });
            }
        } else if now - died > 1.6 {
            // Nobody to blame (a fall, our own grenade): no replay.
            cam.pending = None;
        }
    }

    let Some(mut replay) = cam.replay.take() else {
        return;
    };
    if controls.jump_pressed || controls.ui.accept || replay.clock >= replay.end {
        cam.replay = Some(replay);
        finish(&mut cam, &mut commands, &mut robots);
        return;
    }
    // Slow motion around the killing shot.
    let slow = (replay.clock - replay.death).abs() < SLOW_SPAN;
    let before = replay.clock;
    replay.clock += dt * if slow { SLOW } else { 1.0 };

    for mut visibility in robots.iter_mut() {
        if *visibility != Visibility::Hidden {
            *visibility = Visibility::Hidden;
        }
    }
    for (id, entity) in &replay.ghosts {
        let Ok((mut transform, mut pose)) = ghosts.get_mut(*entity) else {
            continue;
        };
        let Some(s) = cam.sample(*id, replay.clock) else {
            continue;
        };
        let earlier = cam.sample(*id, replay.clock - 0.08).unwrap_or(s);
        transform.translation = s.pos;
        transform.rotation = Quat::from_rotation_y(s.yaw);
        let moved = (s.pos - earlier.pos).xz();
        let forward = Vec2::new(-s.yaw.sin(), -s.yaw.cos());
        let speed = (moved.length() / 0.08).min(9.0)
            * if moved.dot(forward) < -0.01 {
                -1.0
            } else {
                1.0
            };
        pose.speed += (speed - pose.speed) * 0.3;
        pose.stance = s.flags & flags::STANCE;
        pose.pitch = s.pitch;
        pose.aiming = s.flags & flags::AIMING != 0;
        pose.reloading = s.flags & flags::RELOADING != 0;
        pose.sprinting = s.flags & flags::SPRINTING != 0;
        pose.dead = !s.alive;
        let lean = flags::lean(s.flags) as f32;
        pose.lean += (lean - pose.lean) * 0.25;
    }

    // The shots fired in this slice of the recording fly again.
    for shot in cam
        .shots
        .iter()
        .filter(|s| s.t > before && s.t <= replay.clock)
    {
        let span = shot.to - shot.from;
        let length = span.length();
        if length < 0.05 {
            continue;
        }
        let width = if shot.beam { 0.04 } else { 0.025 };
        commands.spawn((
            MatchEntity,
            Ghost,
            ReplayTracer(if slow { 0.18 } else { 0.07 }),
            Mesh3d(art.primitive(Shape::Cube)),
            MeshMaterial3d(art.paint(if shot.beam {
                Paint::Aether
            } else {
                shot.tracer
            })),
            Transform::from_translation(shot.from + span * 0.5)
                .with_rotation(Quat::from_rotation_arc(Vec3::Z, span / length))
                .with_scale(Vec3::new(width, width, length)),
            NotShadowCaster,
        ));
    }

    // Over the killer's right shoulder, looking where they looked.
    if let (Some(killer), Ok((mut camera, mut projection))) = (
        cam.sample(replay.killer, replay.clock),
        cameras.single_mut(),
    ) {
        let stance_now = killer.flags & flags::STANCE;
        let head = killer.pos + Vec3::Y * (stance::eye(stance_now) + 0.2);
        let look = Quat::from_euler(EulerRot::YXZ, killer.yaw, killer.pitch * 0.8 - 0.08, 0.0);
        let offset = Quat::from_rotation_y(killer.yaw) * Vec3::new(0.75, 0.3, 2.3);
        let distance = offset.length();
        let reach = statics
            .raycast(head, offset / distance, distance + 0.3)
            .map_or(distance, |(d, _)| (d - 0.3).max(0.35))
            .min(distance);
        let wanted = head + offset / distance * reach;
        let target = Transform::from_translation(wanted)
            .looking_at(head + look * Vec3::new(0.6, -0.1, -14.0), Vec3::Y)
            .rotation;
        let blend = 1.0 - (-dt * 9.0).exp();
        let (position, rotation) = replay.camera.map_or((wanted, target), |(p, r)| {
            (p.lerp(wanted, blend), r.slerp(target, blend))
        });
        replay.camera = Some((position, rotation));
        camera.translation = position;
        camera.rotation = rotation;
        if let Projection::Perspective(perspective) = &mut *projection {
            perspective.fov = if slow { 58f32 } else { 66f32 }.to_radians();
        }
    }
    cam.replay = Some(replay);
}

/// Letterbox bars and who got you.
fn overlay(commands: &mut Commands, killer: &str, weapon: &str, skip: &str) -> Entity {
    commands
        .spawn((
            MatchEntity,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::SpaceBetween,
                ..default()
            },
            GlobalZIndex(35),
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Percent(11.0),
                    padding: UiRect::axes(Val::Px(28.0), Val::Px(10.0)),
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(18.0),
                    ..default()
                },
                BackgroundColor(Color::BLACK),
            ))
            .with_children(|top| {
                top.spawn((
                    Node {
                        padding: UiRect::axes(Val::Px(8.0), Val::Px(3.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.75, 0.12, 0.1)),
                ))
                .with_children(|tag| ui::label(tag, "KILLCAM", 20.0, ui::TEXT));
                ui::label(top, format!("KILLED BY  {killer}"), 20.0, ui::TEXT);
                if !weapon.is_empty() {
                    ui::label(top, weapon, 15.0, ui::ACCENT);
                }
            });
            root.spawn((
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Percent(11.0),
                    padding: UiRect::axes(Val::Px(28.0), Val::Px(10.0)),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::End,
                    ..default()
                },
                BackgroundColor(Color::BLACK),
            ))
            .with_children(|bottom| ui::label(bottom, format!("[{skip}]  SKIP"), 14.0, ui::DIM));
        })
        .id()
}
