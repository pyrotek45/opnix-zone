//! Kill-cam: after you go down, watch how it happened.
//!
//! Every machine keeps a short rolling recording of what it saw: every
//! robot's position, facing, aim, stance and gun, and every shot fired. When
//! the local player dies to another player, the last few seconds play back
//! through the killer's eyes: stand-in robots re-enact the moves, the
//! tracers fly again, and the killing moment runs in slow motion. Jump /
//! (A) skips it. Nothing is sent over the network: it replays what this
//! machine already saw, so it works the same offline, hosting and joined.
//!
//! It also keeps score of the match for the end screen: every kill is rated
//! (headshots, multi-kills, long shots, streaks) and the best one is kept as a
//! clip that plays behind the awards as the play of the match.

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
const BEFORE: f32 = 2.6;
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

#[derive(Clone)]
struct Frame {
    t: f32,
    bodies: Vec<Sample>,
}

#[derive(Clone, Copy)]
struct ShotRecord {
    t: f32,
    by: u8,
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
    /// The killer's gun, held in front of the camera.
    gun: Option<Entity>,
    /// The play of the match, looping behind the end screen.
    finale: bool,
}

/// One player's match, as this machine saw it.
#[derive(Clone, Debug, Default)]
pub struct Tally {
    pub id: u8,
    pub kills: u32,
    pub heads: u32,
    pub deaths: u32,
    pub longest: f32,
    pub streak: u32,
    pub best_streak: u32,
    pub best_chain: u32,
    chain: u32,
    last_kill: f32,
    pub blasts: u32,
}

/// The best kill of the match, kept for the end screen.
struct Clip {
    frames: Vec<Frame>,
    shots: Vec<ShotRecord>,
    death: f32,
    killer: u8,
    title: String,
}

#[derive(Resource, Default)]
pub struct KillCam {
    frames: VecDeque<Frame>,
    shots: VecDeque<ShotRecord>,
    replay: Option<Replay>,
    /// When the local player went down, if a replay may follow.
    pending: Option<f32>,
    was_alive: bool,
    /// Everyone's match so far.
    pub tallies: Vec<Tally>,
    pub first_blood: Option<u8>,
    /// The best kill so far: its score, and the clip once the moment has passed.
    best_score: f32,
    capture: Option<(f32, u8, String)>,
    best: Option<Clip>,
    /// The replay is looking down a scope of this kind (0 = none): the HUD draws its glass.
    pub scope: u8,
    /// While a replay runs: the killer's first-person state, which the view draws instead
    /// of the local player's.
    pub pov: Option<Pov>,
}

/// One frame of someone's first-person view, from a recording.
#[derive(Clone, Copy, Debug)]
pub struct Pov {
    pub at: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub stance: u8,
    pub aiming: bool,
    pub reloading: bool,
    pub weapon: u8,
    pub parts: [u8; SLOT_COUNT],
    pub shots: u32,
    pub speed: f32,
}

impl Pov {
    /// Stand-in components shaped like a live player, for the view code to read.
    pub fn stand_in(&self, base_arsenal: &Arsenal, base_player: &Player) -> (Transform, PlayerInput, Arsenal, oz_sim::player::Mover, Health, Player) {
        let mut arsenal = base_arsenal.clone();
        arsenal.current = self.weapon;
        if let Some(weapon) = arsenal.weapons.get_mut(self.weapon as usize) {
            weapon.owned = true;
            weapon.equipped = self.parts;
            weapon.ammo = weapon.ammo.max(1);
        }
        arsenal.reload = if self.reloading { 1.0 } else { 0.0 };
        arsenal.reload_total = if self.reloading { 2.0 } else { 0.0 };
        arsenal.shots = self.shots;
        let mut buttons = 0;
        if self.aiming {
            buttons |= oz_sim::player::buttons::AIM;
        }
        let input = PlayerInput { yaw: self.yaw, pitch: self.pitch, stance: self.stance, weapon: self.weapon, buttons, ..default() };
        let mover = oz_sim::player::Mover { speed: self.speed, grounded: true, ..default() };
        (Transform::from_translation(self.at), input, arsenal, mover, Health { hp: 100.0, max: 100.0 }, base_player.clone())
    }
}

impl KillCam {
    /// A replay is on screen.
    pub fn playing(&self) -> bool {
        self.replay.is_some()
    }

    fn tally(&mut self, id: u8) -> &mut Tally {
        if let Some(at) = self.tallies.iter().position(|t| t.id == id) {
            return &mut self.tallies[at];
        }
        self.tallies.push(Tally { id, ..default() });
        self.tallies.last_mut().expect("just pushed")
    }

    /// The end-of-match awards: (award, player id, why).
    pub fn awards(&self) -> Vec<(&'static str, u8, String)> {
        let mut out = Vec::new();
        let top = |key: &dyn Fn(&Tally) -> f32| self.tallies.iter().filter(|t| key(t) > 0.0).max_by(|a, b| key(a).total_cmp(&key(b)));
        if let Some(t) = top(&|t| t.kills as f32 - t.deaths as f32 * 0.01) {
            out.push(("MVP", t.id, format!("{} KILLS  {} DEATHS", t.kills, t.deaths)));
        }
        if let Some(t) = top(&|t| t.heads as f32) {
            out.push(("SHARPSHOOTER", t.id, format!("{} HEADSHOTS", t.heads)));
        }
        if let Some(t) = top(&|t| t.longest) {
            out.push(("LONGSHOT", t.id, format!("{:.0} M KILL", t.longest)));
        }
        if let Some(t) = top(&|t| if t.best_streak >= 3 { t.best_streak as f32 } else { 0.0 }) {
            out.push(("UNSTOPPABLE", t.id, format!("{} KILL STREAK", t.best_streak)));
        }
        if let Some(t) = top(&|t| if t.best_chain >= 2 { t.best_chain as f32 } else { 0.0 }) {
            out.push(("CHAIN REACTION", t.id, chain_name(t.best_chain).to_string()));
        }
        if let Some(t) = top(&|t| t.blasts as f32) {
            out.push(("DEMOLITIONS", t.id, format!("{} BLAST KILLS", t.blasts)));
        }
        if let Some(t) = top(&|t| if t.kills >= 3 { t.kills as f32 / (t.deaths as f32 + 1.0) } else { 0.0 }) {
            out.push(("SURVIVOR", t.id, format!("{:.1} K/D", t.kills as f32 / t.deaths.max(1) as f32)));
        }
        if let Some(id) = self.first_blood {
            out.push(("FIRST BLOOD", id, "OPENED THE SCORING".to_string()));
        }
        out
    }

    /// The play of the match: who made it and what it was.
    pub fn best_play(&self) -> Option<(u8, &str)> {
        self.best.as_ref().map(|c| (c.killer, c.title.as_str()))
    }

    /// One robot's pose at recorded time `t`, blended between the frames around it.
    fn sample(&self, id: u8, t: f32) -> Option<Sample> {
        let after = self.frames.partition_point(|f| f.t < t);
        let find = |index: usize| self.frames.get(index).and_then(|f| f.bodies.iter().find(|b| b.id == id).map(|b| (f.t, *b)));
        let (a, b) = (after.checked_sub(1).and_then(find), find(after));
        match (a, b) {
            (Some((ta, a)), Some((tb, b))) => {
                let k = ((t - ta) / (tb - ta).max(1e-4)).clamp(0.0, 1.0);
                // No sliding across the map when someone respawned between frames.
                if a.pos.distance(b.pos) > 6.0 {
                    return Some(if k < 0.5 { a } else { b });
                }
                let turn = (b.yaw - a.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
                Some(Sample { pos: a.pos.lerp(b.pos, k), yaw: a.yaw + turn * k, pitch: a.pitch + (b.pitch - a.pitch) * k, ..if k < 0.5 { a } else { b } })
            }
            (Some((_, a)), None) => Some(a),
            (None, Some((_, b))) => Some(b),
            (None, None) => None,
        }
    }
}

fn chain_name(chain: u32) -> &'static str {
    match chain {
        0 | 1 => "KILL",
        2 => "DOUBLE KILL",
        3 => "TRIPLE KILL",
        4 => "QUAD KILL",
        _ => "RAMPAGE",
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
            .add_systems(OnEnter(MatchPhase::Active), |mut cam: ResMut<KillCam>| *cam = KillCam::default())
            .add_systems(Update, bait.run_if(in_state(MatchPhase::Active)))
            .add_systems(Update, (record, run).chain().in_set(ClientSet::Present).after(crate::spectate::Spectating).before(crate::avatar::AnimateRigs).run_if(in_state(MatchPhase::Active)));
    }
}

fn record(
    time: Res<Time>,
    state: Res<MatchState>,
    content: Res<Content>,
    mut cam: ResMut<KillCam>,
    mut events: EventReader<FxEvent>,
    players: Query<(&Player, &Transform, &PlayerInput, &Health, Option<&Arsenal>)>,
    mut was_live: Local<bool>,
) {
    let now = time.elapsed_secs();
    let live = state.phase == Phase::Live;
    if !live {
        events.clear();
        *was_live = false;
        return;
    }
    if !*was_live {
        // A new match: a clean sheet.
        *was_live = true;
        cam.tallies.clear();
        cam.first_blood = None;
        cam.best_score = 0.0;
        cam.capture = None;
        cam.best = None;
        cam.frames.clear();
        cam.shots.clear();
    }
    for event in events.read() {
        match event {
            FxEvent::Shot { by, from, to, tracer, beam, .. } => cam.shots.push_back(ShotRecord { t: now, by: *by, from: *from, to: *to, tracer: *tracer, beam: *beam }),
            FxEvent::Killed { victim, by, head, blast } if victim != by => {
                let at = |id: u8| players.iter().find(|(p, ..)| p.id == id).map(|(_, t, ..)| t.translation);
                let distance = at(*victim).zip(at(*by)).map_or(0.0, |(a, b)| a.distance(b));
                if cam.first_blood.is_none() {
                    cam.first_blood = Some(*by);
                }
                cam.tally(*victim).deaths += 1;
                cam.tally(*victim).streak = 0;
                let t = cam.tally(*by);
                t.kills += 1;
                t.heads += *head as u32;
                t.blasts += *blast as u32;
                t.longest = t.longest.max(distance);
                t.streak += 1;
                t.best_streak = t.best_streak.max(t.streak);
                t.chain = if now - t.last_kill < 4.5 { t.chain + 1 } else { 1 };
                t.last_kill = now;
                t.best_chain = t.best_chain.max(t.chain);
                let (chain, streak) = (t.chain, t.streak);
                // How good a play was it?
                let score = 1.0 + if *head { 0.6 } else { 0.0 } + (chain - 1) as f32 * 1.6 + distance / 40.0 + if streak >= 5 { 0.5 } else { 0.0 };
                if score > cam.best_score {
                    cam.best_score = score;
                    let title = if chain >= 2 {
                        chain_name(chain).to_string()
                    } else if distance > 45.0 {
                        format!("{distance:.0} M {}", if *head { "HEADSHOT" } else { "LONGSHOT" })
                    } else if *head {
                        "HEADSHOT".to_string()
                    } else if *blast {
                        "BLAST KILL".to_string()
                    } else {
                        "TAKEDOWN".to_string()
                    };
                    cam.capture = Some((now, *by, title));
                }
            }
            FxEvent::Killed { victim, .. } => {
                cam.tally(*victim).deaths += 1;
                cam.tally(*victim).streak = 0;
            }
            _ => {}
        }
    }
    let bodies = players
        .iter()
        .map(|(player, transform, input, health, arsenal)| {
            let parts = arsenal
                .and_then(|a| a.weapons.get(input.weapon as usize))
                .map(|w| w.equipped)
                .or_else(|| state.roster.iter().find(|r| r.id == player.id && r.weapon == input.weapon).map(|r| r.parts))
                .or_else(|| content.weapons.get(input.weapon as usize).map(|d| d.defaults))
                .unwrap_or([0; SLOT_COUNT]);
            let lean = match input.lean {
                l if l < 0 => flags::LEAN_LEFT,
                l if l > 0 => flags::LEAN_RIGHT,
                _ => 0,
            };
            let flags = (player.flags & !(flags::STANCE | flags::LEAN_LEFT | flags::LEAN_RIGHT))
                | input.stance.max(player.flags & flags::STANCE)
                | if input.lean != 0 { lean } else { player.flags & (flags::LEAN_LEFT | flags::LEAN_RIGHT) };
            Sample { id: player.id, team: player.team, pos: transform.translation, yaw: input.yaw, pitch: input.pitch, flags, alive: health.alive(), weapon: input.weapon, parts }
        })
        .collect();
    cam.frames.push_back(Frame { t: now, bodies });
    // A great play just happened: once its moment has passed, keep it.
    if let Some((death, killer, title)) = cam.capture.clone()
        && now - death > AFTER + 0.1
    {
        cam.capture = None;
        let frames = cam.frames.iter().filter(|f| f.t >= death - BEFORE && f.t <= death + AFTER).cloned().collect();
        let shots = cam.shots.iter().filter(|s| s.t >= death - BEFORE && s.t <= death + AFTER).copied().collect();
        cam.best = Some(Clip { frames, shots, death, killer, title });
    }
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
    (art, parts, content, state): (Res<Art>, Res<RobotParts>, Res<Content>, Res<MatchState>),
    mut guns: ResMut<GunCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut cam: ResMut<KillCam>,
    mut commands: Commands,
    local: Query<(&Player, &Health), With<LocalPlayer>>,
    mut robots: Query<&mut Visibility, (With<crate::avatar::PlayerRobot>, Without<Ghost>, Without<ReplayGun>)>,
    mut ghosts: Query<(&mut Transform, &mut Pose, &mut Visibility), (With<Ghost>, Without<WorldCamera>, Without<crate::avatar::PlayerRobot>, Without<ReplayGun>)>,
    mut held: Query<&mut Transform, (With<ReplayGun>, Without<Ghost>, Without<WorldCamera>)>,
    mut gun_shown: Query<&mut Visibility, (With<ReplayGun>, Without<Ghost>, Without<crate::avatar::PlayerRobot>)>,
    mut cameras: Query<(&mut Transform, &mut Projection), (With<WorldCamera>, Without<Ghost>, Without<ReplayTracer>)>,
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
    let finish = |cam: &mut KillCam, commands: &mut Commands, robots: &mut Query<&mut Visibility, (With<crate::avatar::PlayerRobot>, Without<Ghost>, Without<ReplayGun>)>| {
        if let Some(replay) = cam.replay.take() {
            for (_, entity) in replay.ghosts {
                commands.entity(entity).despawn();
            }
            if let Some(gun) = replay.gun {
                commands.entity(gun).despawn();
            }
            commands.entity(replay.overlay).despawn();
            cam.scope = 0;
            cam.pov = None;
            for mut visibility in robots.iter_mut() {
                if *visibility == Visibility::Hidden {
                    *visibility = Visibility::Inherited;
                }
            }
        }
    };
    let finale = cam.replay.as_ref().is_some_and(|r| r.finale);
    if (health.alive() && !finale) || (state.phase != Phase::Live && !finale) {
        finish(&mut cam, &mut commands, &mut robots);
        cam.pending = None;
    }
    if finale && state.phase != Phase::Won {
        finish(&mut cam, &mut commands, &mut robots);
    }
    // The match is won: the play of the match loops behind the end screen.
    if state.phase == Phase::Won
        && cam.replay.is_none()
        && let Some(clip) = cam.best.take()
    {
        let ghosts = spawn_ghosts(&mut commands, &art, &parts, &content, &mut guns, &mut meshes, &clip.frames, clip.death - BEFORE, clip.death + AFTER);
        let name = state.roster.iter().find(|r| r.id == clip.killer).map_or("?".to_string(), |r| r.name.clone());
        let weapon = sample_in(&clip.frames, clip.killer, clip.death).and_then(|s| content.weapons.get(s.weapon as usize)).map_or("", |w| w.name);
        let overlay = overlay(&mut commands, "PLAY OF THE MATCH", &format!("{name}  -  {}", clip.title), weapon, "");
        let gun = None;
        let start = clip.frames.first().map_or(clip.death, |f| f.t);
        cam.frames = clip.frames.iter().cloned().collect();
        cam.shots = clip.shots.iter().copied().collect();
        cam.replay = Some(Replay { clock: start, end: clip.death + AFTER, death: clip.death, killer: clip.killer, ghosts, camera: None, overlay, gun, finale: true });
        cam.best = Some(clip);
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
            for frame in cam.frames.iter().filter(|f| f.t >= start && f.t <= died + AFTER) {
                for body in &frame.bodies {
                    match ids.iter_mut().find(|(id, ..)| *id == body.id) {
                        Some(entry) => *entry = (body.id, body.team, body.weapon, body.parts),
                        None => ids.push((body.id, body.team, body.weapon, body.parts)),
                    }
                }
            }
            if ids.iter().any(|(id, ..)| *id == killer) {
                let frames: Vec<Frame> = cam.frames.iter().cloned().collect();
                let ghosts = spawn_ghosts(&mut commands, &art, &parts, &content, &mut guns, &mut meshes, &frames, start, died + AFTER);
                // How far off you the killer's recorded aim was when you went down (degrees).
                if let (Some(k), Some(v)) = (cam.sample(killer, died - 0.05), cam.sample(me.id, died - 0.05)) {
                    let eye = k.pos + Vec3::Y * stance::eye(k.flags & flags::STANCE);
                    let look = Quat::from_euler(EulerRot::YXZ, k.yaw, k.pitch, 0.0) * Vec3::NEG_Z;
                    let off = look.angle_between((v.pos + Vec3::Y - eye).normalize_or_zero()).to_degrees();
                    info!("kill cam: killer {killer} aim {off:.1} deg off the victim at {:.1} m", eye.distance(v.pos));
                }
                // A bright marker over your own stand-in, so you can see yourself in their sights.
                if let Some((_, you)) = ghosts.iter().find(|(id, _)| *id == me.id) {
                    commands.spawn((
                        Mesh3d(art.primitive(Shape::Shard)),
                        MeshMaterial3d(art.paint(Paint::Red)),
                        Transform::from_xyz(0.0, 2.35, 0.0).with_scale(Vec3::new(0.18, 0.26, 0.18)),
                        NotShadowCaster,
                        ChildOf(*you),
                    ));
                }
                let name = state.roster.iter().find(|r| r.id == killer).map_or("?".to_string(), |r| r.name.clone());
                let weapon = cam.sample(killer, died).and_then(|s| content.weapons.get(s.weapon as usize)).map_or("", |w| w.name);
                let skip = if controls.gamepad { "(A)".to_string() } else { settings.bindings.label(crate::bindings::Action::Jump) };
                let overlay = overlay(&mut commands, "KILLCAM", &format!("KILLED BY  {name}"), weapon, &format!("[{skip}]  SKIP"));
                let gun = None;
                cam.replay = Some(Replay { clock: start, end: died + AFTER, death: died, killer, ghosts, camera: None, overlay, gun, finale: false });
            }
        } else if now - died > 1.6 {
            // Nobody to blame (a fall, our own grenade): no replay.
            cam.pending = None;
        }
    }

    let Some(mut replay) = cam.replay.take() else {
        return;
    };
    if replay.finale && replay.clock >= replay.end {
        // Round again, with a beat of black-free pause at the start.
        replay.clock = cam.frames.front().map_or(replay.death - BEFORE, |f| f.t);
        replay.camera = None;
    } else if !replay.finale && (controls.jump_pressed || controls.ui.accept || replay.clock >= replay.end) {
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
        let Ok((mut transform, mut pose, mut shown)) = ghosts.get_mut(*entity) else {
            continue;
        };
        // Seen through the killer's eyes: their own body stays out of the shot.
        let wanted = if *id == replay.killer { Visibility::Hidden } else { Visibility::Inherited };
        if *shown != wanted {
            *shown = wanted;
        }
        let Some(s) = cam.sample(*id, replay.clock) else {
            continue;
        };
        let earlier = cam.sample(*id, replay.clock - 0.08).unwrap_or(s);
        transform.translation = s.pos;
        transform.rotation = Quat::from_rotation_y(s.yaw);
        let moved = (s.pos - earlier.pos).xz();
        let forward = Vec2::new(-s.yaw.sin(), -s.yaw.cos());
        let speed = (moved.length() / 0.08).min(9.0) * if moved.dot(forward) < -0.01 { -1.0 } else { 1.0 };
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
    for shot in cam.shots.iter().filter(|s| s.t > before && s.t <= replay.clock) {
        let span = shot.to - shot.from;
        let length = span.length();
        if length < 0.05 {
            continue;
        }
        // Thick enough to read at range: this is what killed you.
        let width = if shot.beam { 0.06 } else { 0.05 };
        commands.spawn((
            MatchEntity,
            Ghost,
            ReplayTracer(if slow { 0.3 } else { 0.14 }),
            Mesh3d(art.primitive(Shape::Cube)),
            MeshMaterial3d(art.paint(if shot.beam { Paint::Aether } else { shot.tracer })),
            Transform::from_translation(shot.from + span * 0.5).with_rotation(Quat::from_rotation_arc(Vec3::Z, span / length)).with_scale(Vec3::new(width, width, length)),
            NotShadowCaster,
        ));
    }

    // Through the killer's eyes: the normal first-person view (camera, hands, gun, aim,
    // scope, recoil) is drawn from the killer's recorded state, as they saw it (see `view`).
    cam.scope = 0;
    cam.pov = cam.sample(replay.killer, replay.clock).map(|killer| {
        let earlier = cam.sample(replay.killer, replay.clock - 0.1).unwrap_or(killer);
        let shots = cam.shots.iter().filter(|s| s.by == replay.killer && s.t <= replay.clock).count() as u32;
        Pov {
            at: killer.pos,
            yaw: killer.yaw,
            pitch: killer.pitch,
            stance: killer.flags & flags::STANCE,
            aiming: killer.flags & flags::AIMING != 0,
            reloading: killer.flags & flags::RELOADING != 0,
            weapon: killer.weapon,
            parts: killer.parts,
            shots,
            speed: (killer.pos - earlier.pos).xz().length() / 0.1,
        }
    });
    let _ = (&mut cameras, &mut held, &mut gun_shown, slow);
    cam.replay = Some(replay);
}

/// The gun the killer was holding, as a first-person view model.
#[derive(Component)]
struct ReplayGun;

/// Pose of one robot in a recording, at time `t` (nearest frame).
fn sample_in(frames: &[Frame], id: u8, t: f32) -> Option<Sample> {
    frames.iter().min_by(|a, b| (a.t - t).abs().total_cmp(&(b.t - t).abs())).and_then(|f| f.bodies.iter().find(|b| b.id == id).copied())
}

#[allow(dead_code)]
fn held_gun(commands: &mut Commands, art: &Art, content: &Content, guns: &mut GunCache, meshes: &mut Assets<Mesh>, frames: &[Frame], killer: u8, at: f32) -> Option<Entity> {
    let s = sample_in(frames, killer, at)?;
    let baked = guns.get(art, meshes, content, s.weapon, s.parts)?;
    let root = commands.spawn((MatchEntity, ReplayGun, Transform::default(), Visibility::Inherited)).id();
    art.spawn(commands, root, &baked);
    Some(root)
}

/// Stand-ins for everyone seen in a stretch of recording.
#[allow(clippy::too_many_arguments)]
fn spawn_ghosts(commands: &mut Commands, art: &Art, parts: &RobotParts, content: &Content, guns: &mut GunCache, meshes: &mut Assets<Mesh>, frames: &[Frame], from: f32, to: f32) -> Vec<(u8, Entity)> {
    let mut ids: Vec<(u8, u8, u8, [u8; SLOT_COUNT])> = Vec::new();
    for frame in frames.iter().filter(|f| f.t >= from && f.t <= to) {
        for body in &frame.bodies {
            match ids.iter_mut().find(|(id, ..)| *id == body.id) {
                Some(entry) => *entry = (body.id, body.team, body.weapon, body.parts),
                None => ids.push((body.id, body.team, body.weapon, body.parts)),
            }
        }
    }
    ids.into_iter()
        .map(|(id, team, weapon, fitted)| {
            let root = commands.spawn((MatchEntity, Ghost, Transform::default(), Visibility::Inherited)).id();
            let gun = guns.get(art, meshes, content, weapon, fitted);
            let rig = spawn_robot(commands, art, parts, root, team, gun.as_ref());
            commands.entity(root).insert((rig, Pose::default()));
            (id, root)
        })
        .collect()
}

/// Letterbox bars: what this is, whose play, with what, and how to skip.
fn overlay(commands: &mut Commands, tag: &str, headline: &str, weapon: &str, skip: &str) -> Entity {
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
                top.spawn((Node { padding: UiRect::axes(Val::Px(8.0), Val::Px(3.0)), ..default() }, BackgroundColor(Color::srgb(0.75, 0.12, 0.1))))
                    .with_children(|t| ui::label(t, tag, 20.0, ui::TEXT));
                ui::label(top, headline, 20.0, ui::TEXT);
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
            .with_children(|bottom| ui::label(bottom, skip, 14.0, ui::DIM));
        })
        .id()
}

/// Developer aid: `OPNIX_BAIT=1` drops the local player halfway to the enemy base once the
/// match is live, facing it, so a death (and its kill cam) comes quickly for screenshots.
fn bait(map: Res<Map>, state: Res<MatchState>, mut players: Query<(&Player, &mut Transform, &mut PlayerInput), With<LocalPlayer>>, mut done: Local<bool>) {
    if *done || state.phase != Phase::Live || std::env::var("OPNIX_BAIT").is_err() {
        return;
    }
    let Ok((me, mut transform, mut input)) = players.single_mut() else { return };
    let (Some(ours), Some(theirs)) = (oz_sim::player::base_centre(&map, me.team), oz_sim::player::base_centre(&map, 1 - me.team.min(1))) else { return };
    *done = true;
    let mid = ours.lerp(theirs, 0.6);
    transform.translation = map.ground(mid.x, mid.y);
    let to = theirs - mid;
    input.yaw = f32::atan2(-to.x, -to.y);
}
