//! Combat punch: the loud, chunky layer of feedback on top of [`crate::fx`]
//! and [`crate::juice`]. Everything here is client-side and cosmetic: it only
//! reads `FxEvent`s and replicated components, so it works the same for the
//! host, offline and online clients, and never touches the simulation.
//!
//! * **Hits you land:** a pitched tick that climbs through a burst, sparks and
//!   scrap in the target's team colours, a soft edge flash, and damage numbers
//!   that float off the target (rapid hits on one robot add up in one number).
//! * **Robots flinch:** the torso and head recoil away from each hit, and a
//!   limb that is hit is knocked back on a spring.
//! * **Kills:** a short hit-stop (the world view holds for a few frames while
//!   the game runs on), a big kill-confirm X with a burst and a skull, an XP
//!   ticker that counts up, sparks, oil smoke and a shockwave ring off the body.
//! * **Explosions:** an additive fireball that cools from white to red, a black
//!   smoke column that lingers and drifts, debris trailing smoke and bouncing,
//!   sparks, a ground shockwave and dust skirt, a flickering light, colour
//!   fringing up close, and a rumble and crack that arrive at the speed of sound.
//! * **Guns:** a flash star at every muzzle, short muzzle lights, brass from
//!   other robots' guns, soft dust puffs where rounds land.
//!
//! The `EFFECTS` setting picks FULL or REDUCED: reduced spawns about a third
//! of the particles, never holds the frame, and shakes and flashes less.
//! Particles are capped (and lights too), so a street full of explosions
//! stays smooth.
//!
//! Dev aid: `OZ_JUICE_DEMO=1` sets off explosions in front of the camera and
//! fakes hits and kills on the nearest robot in view, and logs the frame
//! rate, so the effects can be checked without aiming.

use crate::{
    ClientSet,
    art::Art,
    avatar::{Rig, team_light, team_paint},
    fx::PlaySfx,
    retro::Retro,
    settings::Settings,
    view::{ViewState, WorldCamera},
};
use bevy::{pbr::NotShadowCaster, platform::collections::HashMap, prelude::*};
use oz_sim::prelude::*;
use std::f32::consts::{FRAC_PI_2, TAU};

/// Particle caps for FULL and REDUCED effects (on top of `fx`'s own).
const MAX_BITS: [usize; 2] = [900, 260];
/// Short-lived point lights (blasts, muzzles) alive at once.
const MAX_FLARES: usize = 6;
/// Floating damage numbers on screen at once.
const POPUPS: usize = 10;
/// Seconds between hits that still count as one burst (the tick climbs, the number adds up).
const COMBO: f32 = 0.55;
/// Seconds between kills that keep the XP ticker counting.
const XP_CHAIN: f32 = 3.0;

pub struct PunchPlugin;

impl Plugin for PunchPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Punch>()
            .init_resource::<Popups>()
            .init_resource::<Flinches>()
            .add_systems(Startup, make_materials)
            .add_systems(OnEnter(MatchPhase::Active), (spawn_overlay, |mut punch: ResMut<Punch>, mut popups: ResMut<Popups>, mut flinches: ResMut<Flinches>| {
                *punch = Punch::default();
                *popups = Popups::default();
                flinches.0.clear();
            }))
            .add_systems(Update, unflinch.in_set(ClientSet::Present).before(crate::avatar::AnimateRigs).run_if(in_state(MatchPhase::Active)))
            .add_systems(
                Update,
                (demo, react, (aftermath, clinks), tick_bits, tick_flares, watch_bodies, flinch, screen_pulse, hit_stop, show_popups, show_confirm)
                    .chain()
                    .in_set(ClientSet::Present)
                    .after(crate::avatar::AnimateRigs)
                    .run_if(in_state(MatchPhase::Active)),
            )
            .add_systems(OnExit(MatchPhase::Active), thaw);
    }
}

/// Is the FULL effects level on?
fn full(settings: &Settings) -> bool {
    settings.full_effects
}

/// Transient feedback state for the local player.
#[derive(Resource, Default)]
struct Punch {
    /// Hits in the current burst, and seconds since the last one.
    combo: u32,
    since_hit: f32,
    /// Seconds left of the world-view hold, frames held so far, and the cooldown before another.
    stop: f32,
    stop_frames: u32,
    stop_cool: f32,
    frozen: bool,
    /// Edge glow colour and strength, and colour fringing.
    edge: Vec3,
    edge_a: f32,
    aberration: f32,
    /// Kill-confirm X: seconds since it fired (large = hidden), and whether it was a headshot.
    confirm: f32,
    confirm_head: bool,
    /// XP: what the ticker is counting toward, what it shows now, and seconds since the last kill.
    xp_target: f32,
    xp_shown: f32,
    xp_age: f32,
    /// Where our last shot landed, and when (seconds of game time).
    last_shot: Option<(Vec3, Vec3, f32)>,
    /// Kills this life (for the streak counter).
    streak: u32,
    /// Kills in the current multi-kill chain, and seconds since the last one.
    chain: u32,
    since_kill: f32,
}

impl Default for Popups {
    fn default() -> Self {
        Self { slots: (0..POPUPS).map(|_| None).collect(), history: HashMap::default() }
    }
}

/// One damage number.
#[derive(Clone)]
struct Popup {
    victim: u8,
    /// The victim's health just before the burst began; the number is what has gone since.
    base: f32,
    shown: f32,
    at: Vec3,
    age: f32,
    /// 1 right after a hit, decaying: the number swells as it lands.
    pop: f32,
    head: bool,
    kill: bool,
    drift: f32,
}

#[derive(Resource)]
struct Popups {
    slots: Vec<Option<Popup>>,
    /// Recent health of every robot: (game time, hp), to know what a hit took off.
    history: HashMap<u8, Vec<(f32, f32)>>,
}

impl Popups {
    fn hp_before(&self, victim: u8, now: f32) -> Option<f32> {
        self.history.get(&victim)?.iter().filter(|(t, _)| now - t < 0.4).map(|(_, hp)| *hp).reduce(f32::max)
    }
}

/// The ramped materials: additive fire that cools, smoke and dust that thin out.
#[derive(Resource)]
struct Mats {
    fire: Vec<Handle<StandardMaterial>>,
    smoke: Vec<Handle<StandardMaterial>>,
    dust: Vec<Handle<StandardMaterial>>,
    ring: Handle<Mesh>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ramp {
    /// A plain palette material, shrinking away.
    Solid,
    Fire,
    Smoke,
    Dust,
}

/// A particle of this module. Unlike `fx`'s, these can cool through a colour
/// ramp, drag through the air, bounce, spin, stretch along their flight and
/// leave smoke trails.
#[derive(Component)]
struct Bit {
    velocity: Vec3,
    gravity: f32,
    drag: f32,
    life: f32,
    initial: f32,
    size: Vec3,
    /// Scale at birth and at death, as multiples of `size`.
    grow: (f32, f32),
    ramp: Ramp,
    stage: u8,
    spin: Vec3,
    bounce: bool,
    /// Seconds between smoke-trail puffs (0 = none), and the countdown.
    trail: f32,
    trail_t: f32,
    /// Stretched along its flight like a spark.
    streak: bool,
}

impl Bit {
    fn new(velocity: Vec3, life: f32, size: Vec3) -> Self {
        Self {
            velocity,
            gravity: 0.0,
            drag: 0.0,
            life,
            initial: life,
            size,
            grow: (1.0, 0.0),
            ramp: Ramp::Solid,
            stage: 0,
            spin: Vec3::ZERO,
            bounce: false,
            trail: 0.0,
            trail_t: 0.0,
            streak: false,
        }
    }
    fn gravity(self, gravity: f32) -> Self {
        Self { gravity, ..self }
    }
    fn drag(self, drag: f32) -> Self {
        Self { drag, ..self }
    }
    fn grow(self, from: f32, to: f32) -> Self {
        Self { grow: (from, to), ..self }
    }
    fn ramp(self, ramp: Ramp) -> Self {
        Self { ramp, ..self }
    }
    fn spin(self, spin: Vec3) -> Self {
        Self { spin, ..self }
    }
    fn bouncy(self) -> Self {
        Self { bounce: true, ..self }
    }
    fn trail(self, every: f32) -> Self {
        Self { trail: every, trail_t: every, ..self }
    }
    fn streak(self) -> Self {
        Self { streak: true, ..self }
    }
}

/// A short flash of real light.
#[derive(Component)]
struct Flare {
    life: f32,
    initial: f32,
    intensity: f32,
    flicker: f32,
}

fn ramp_material(color: Color, mode: AlphaMode, unlit: bool) -> StandardMaterial {
    StandardMaterial { base_color: color, alpha_mode: mode, unlit, cull_mode: None, perceptual_roughness: 1.0, ..default() }
}

fn make_materials(mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>, mut meshes: ResMut<Assets<Mesh>>) {
    // Fire cools from white-hot through orange to a dull ember; additive, so the last steps fade out.
    let fire = [(1.0, 0.97, 0.86), (1.0, 0.84, 0.42), (1.0, 0.58, 0.18), (0.95, 0.36, 0.08), (0.6, 0.17, 0.04), (0.28, 0.07, 0.02)]
        .into_iter()
        .map(|(r, g, b)| materials.add(ramp_material(Color::srgb(r, g, b), AlphaMode::Add, true)))
        .collect();
    let smoke = [0.88, 0.72, 0.55, 0.38, 0.22, 0.1].into_iter().map(|a| materials.add(ramp_material(Color::srgba(0.1, 0.095, 0.09, a), AlphaMode::Blend, false))).collect();
    let dust = [0.75, 0.6, 0.45, 0.3, 0.16, 0.07].into_iter().map(|a| materials.add(ramp_material(Color::srgba(0.78, 0.7, 0.58, a), AlphaMode::Blend, false))).collect();
    let ring = meshes.add(Annulus::new(0.82, 1.0));
    commands.insert_resource(Mats { fire, smoke, dust, ring });
}

/// Spawns this module's particles within a budget.
struct Kit<'a, 'w, 's> {
    commands: &'a mut Commands<'w, 's>,
    art: &'a Art,
    mats: &'a Mats,
    rng: &'a mut Rng,
    budget: usize,
    /// Particle count multiplier (reduced effects spawn fewer).
    scale: f32,
}

impl Kit<'_, '_, '_> {
    fn count(&self, n: usize) -> usize {
        ((n as f32 * self.scale).round() as usize).max(1)
    }

    fn material(&self, ramp: Ramp, paint: Paint) -> Handle<StandardMaterial> {
        match ramp {
            Ramp::Solid => self.art.paint(paint),
            Ramp::Fire => self.mats.fire[0].clone(),
            Ramp::Smoke => self.mats.smoke[0].clone(),
            Ramp::Dust => self.mats.dust[0].clone(),
        }
    }

    fn spawn(&mut self, shape: Shape, paint: Paint, at: Vec3, bit: Bit) {
        if self.budget == 0 {
            return;
        }
        self.budget -= 1;
        let rotation = Quat::from_euler(EulerRot::YXZ, self.rng.range(0.0, TAU), self.rng.range(-0.5, 0.5), self.rng.range(-0.5, 0.5));
        let material = self.material(bit.ramp, paint);
        let transform = Transform::from_translation(at).with_rotation(rotation).with_scale(bit.size * bit.grow.0);
        self.commands.spawn((MatchEntity, Mesh3d(self.art.primitive(shape)), MeshMaterial3d(material), transform, bit, NotShadowCaster));
    }

    /// A flat ring on the ground that races outward.
    fn ring(&mut self, at: Vec3, radius: f32, ramp: Ramp, life: f32) {
        if self.budget == 0 {
            return;
        }
        self.budget -= 1;
        let size = Vec3::splat(radius);
        let bit = Bit::new(Vec3::ZERO, life, size).grow(0.12, 1.0).ramp(ramp);
        let transform = Transform::from_translation(at).with_rotation(Quat::from_rotation_x(-FRAC_PI_2)).with_scale(size * 0.12);
        self.commands.spawn((MatchEntity, Mesh3d(self.mats.ring.clone()), MeshMaterial3d(self.material(ramp, Paint::Dark)), transform, bit, NotShadowCaster));
    }

    fn dir(&mut self, up: (f32, f32)) -> Vec3 {
        Vec3::new(self.rng.range(-1.0, 1.0), self.rng.range(up.0, up.1), self.rng.range(-1.0, 1.0)).normalize_or(Vec3::Y)
    }

    /// Fast sparks that streak along their flight and fall.
    fn sparks(&mut self, at: Vec3, n: usize, paints: &[Paint], speed: f32, up: (f32, f32), around: Vec3) {
        for i in 0..self.count(n) {
            let dir = (self.dir(up) + around).normalize_or(Vec3::Y);
            let velocity = dir * speed * self.rng.range(0.45, 1.0);
            let width = self.rng.range(0.018, 0.032);
            let life = self.rng.range(0.25, 0.6);
            self.spawn(Shape::Cube, paints[i % paints.len()], at, Bit::new(velocity, life, Vec3::new(width, width, 1.0)).gravity(12.0).drag(1.2).streak());
        }
    }

    /// Chunky bits that tumble, bounce and settle.
    fn chips(&mut self, at: Vec3, n: usize, paints: &[Paint], speed: f32, size: f32, shape: Shape, around: Vec3) {
        for i in 0..self.count(n) {
            let dir = (self.dir((0.2, 1.0)) + around).normalize_or(Vec3::Y);
            let velocity = dir * speed * self.rng.range(0.4, 1.0);
            let s = size * self.rng.range(0.6, 1.2);
            let spin = Vec3::new(self.rng.range(-14.0, 14.0), self.rng.range(-14.0, 14.0), self.rng.range(-14.0, 14.0));
            let life = self.rng.range(1.2, 2.2);
            let shape_size = if shape == Shape::Cylinder { Vec3::new(s * 0.5, s * 1.3, s * 0.5) } else { Vec3::new(s, s * 0.6, s * 0.9) };
            self.spawn(shape, paints[i % paints.len()], at, Bit::new(velocity, life, shape_size).gravity(14.0).drag(0.3).spin(spin).bouncy().grow(1.0, 0.6));
        }
    }

    /// Soft puffs of smoke or dust drifting off.
    fn puffs(&mut self, at: Vec3, n: usize, ramp: Ramp, size: f32, rise: f32, spread: f32, life: f32) {
        for _ in 0..self.count(n) {
            let drift = Vec3::new(self.rng.range(-spread, spread), rise * self.rng.range(0.6, 1.2), self.rng.range(-spread, spread));
            let s = Vec3::splat(size * self.rng.range(0.7, 1.3));
            let life = life * self.rng.range(0.75, 1.25);
            let jitter = Vec3::new(self.rng.range(-0.3, 0.3), self.rng.range(0.0, 0.3), self.rng.range(-0.3, 0.3)) * size;
            self.spawn(Shape::Shard, Paint::Dark, at + jitter, Bit::new(drift, life, s).drag(0.8).grow(0.5, 2.0).ramp(ramp));
        }
    }

    /// One additive flash that swells and cools.
    fn flash(&mut self, at: Vec3, size: f32, life: f32) {
        self.spawn(Shape::Shard, Paint::Spark, at, Bit::new(Vec3::ZERO, life, Vec3::splat(size)).grow(0.6, 1.6).ramp(Ramp::Fire));
    }

    /// A short real light; skipped when too many are already burning.
    fn light(&mut self, at: Vec3, color: Color, intensity: f32, range: f32, life: f32, flares: usize) {
        if flares >= MAX_FLARES {
            return;
        }
        self.commands.spawn((
            MatchEntity,
            PointLight { color, intensity, range, shadows_enabled: false, ..default() },
            Transform::from_translation(at),
            Flare { life, initial: life, intensity, flicker: self.rng.range(0.0, TAU) },
        ));
    }
}

/// The robot nearest a point (id, chest position).
fn body_near(bodies: &[(u8, u8, Vec3, Quat)], at: Vec3, within: f32) -> Option<(u8, u8, Vec3, Quat)> {
    bodies.iter().map(|b| (b, (b.2 + Vec3::Y * 1.0).distance(at))).filter(|(_, d)| *d < within).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(b, _)| *b)
}

#[allow(clippy::too_many_arguments)]
fn react(
    time: Res<Time>,
    mut events: EventReader<FxEvent>,
    mut commands: Commands,
    art: Res<Art>,
    mats: Option<Res<Mats>>,
    (settings, map): (Res<Settings>, Res<Map>),
    (mut punch, mut popups): (ResMut<Punch>, ResMut<Popups>),
    mut view: ResMut<ViewState>,
    mut sounds: EventWriter<PlaySfx>,
    mut flinches: ResMut<Flinches>,
    local: Query<&Player, With<LocalPlayer>>,
    bodies: Query<(&Player, &Transform, &Health)>,
    cameras: Query<&Transform, With<WorldCamera>>,
    (rigs, joints, bits, flares): (Query<(&Rig, &Player)>, Query<&GlobalTransform>, Query<(), With<Bit>>, Query<(), With<Flare>>),
    mut rng: Local<Option<Rng>>,
) {
    let Some(mats) = mats else { return };
    let full = full(&settings);
    let now = time.elapsed_secs();
    let dt = time.delta_secs();
    let me = local.single().map_or(255, |p| p.id);
    let camera = cameras.single().ok().copied().unwrap_or_default();
    let people: Vec<(u8, u8, Vec3, Quat)> = bodies.iter().map(|(p, t, _)| (p.id, p.team, t.translation, t.rotation)).collect();
    let rng = rng.get_or_insert_with(|| Rng::new(0x9a11_b00b));
    let cap = MAX_BITS[(!full) as usize];
    let mut kit = Kit { commands: &mut commands, art: &art, mats: &mats, rng, budget: cap.saturating_sub(bits.iter().len()), scale: if full { 1.0 } else { 0.35 } };
    let mut flare_count = flares.iter().len();
    let calm = if full { 1.0 } else { 0.45 };
    punch.since_hit += dt;
    punch.since_kill += dt;
    punch.xp_age += dt;
    // The Hurt that comes right before a HitMarker names who was hit.
    let mut last_hurt: Option<u8> = None;
    for event in events.read() {
        match event {
            FxEvent::Shot { by, from, to, beam, quiet, .. } => {
                let dir = (*to - *from).normalize_or(Vec3::NEG_Z);
                if *by == me {
                    punch.last_shot = Some((*from, *to, now));
                }
                // Robots in the line of fire recoil from rounds landing on them.
                if let Some((id, ..)) = body_near(&people, *to, 1.1) {
                    flinches.0.entry(id).or_default().push = Some((dir, now));
                }
                if *beam || camera.translation.distance(*to) > 70.0 {
                    continue;
                }
                // A flash star at the muzzle: ours from the viewmodel, theirs from the robot's gun.
                let muzzle = if *by == me && view.muzzle != Vec3::ZERO {
                    Some(camera.translation + camera.rotation * view.muzzle)
                } else {
                    rigs.iter().find(|(_, p)| p.id == *by).and_then(|(rig, _)| joints.get(rig.gun).ok()).map(|g| g.translation() + g.forward().as_vec3() * 0.55)
                };
                if let Some(muzzle) = muzzle.filter(|_| !*quiet) {
                    let mine = *by == me;
                    let size = if mine { 0.05 } else { 0.16 };
                    for k in 0..2 {
                        let roll = kit.rng.range(0.0, TAU) + k as f32 * FRAC_PI_2;
                        let arm = Quat::from_rotation_arc(Vec3::Z, dir) * Quat::from_rotation_z(roll);
                        let length = size * kit.rng.range(2.6, 3.6);
                        let at = muzzle + dir * size * 0.6;
                        if kit.budget > 0 {
                            kit.budget -= 1;
                            let s = Vec3::new(size * 0.18, length, size * 0.18);
                            let bit = Bit::new(Vec3::ZERO, 0.045, s).grow(1.0, 0.4).ramp(Ramp::Fire);
                            kit.commands.spawn((MatchEntity, Mesh3d(art.primitive(Shape::Cube)), MeshMaterial3d(mats.fire[0].clone()), Transform::from_translation(at).with_rotation(arm).with_scale(s), bit, NotShadowCaster));
                        }
                    }
                    kit.flash(muzzle + dir * size, size * 1.3, 0.05);
                    if !mine && full && camera.translation.distance(muzzle) < 35.0 {
                        kit.light(muzzle, Color::srgb(1.0, 0.75, 0.4), 120_000.0, 7.0, 0.05, flare_count);
                        flare_count += 1;
                    }
                    if !mine && let Some((rig, _)) = rigs.iter().find(|(_, p)| p.id == *by)
                        && let Ok(gun) = joints.get(rig.gun)
                    {
                        // Brass kicked out of the right side of their gun.
                        let right = gun.right().as_vec3();
                        let velocity = right * kit.rng.range(2.0, 3.2) + Vec3::Y * kit.rng.range(1.5, 2.5);
                        let spin = Vec3::new(kit.rng.range(-20.0, 20.0), 0.0, kit.rng.range(-20.0, 20.0));
                        kit.spawn(Shape::Cube, Paint::Gold, gun.translation() + right * 0.06, Bit::new(velocity, 0.9, Vec3::new(0.025, 0.025, 0.06)).gravity(11.0).spin(spin).bouncy().grow(1.0, 1.0));
                    }
                }
                // Soft dust off whatever the round hit (robots spark in `fx` instead).
                let robot = body_near(&people, *to, 0.9).is_some();
                let ground = to.y - map.height(to.x, to.z) < 0.2;
                if !robot && full && camera.translation.distance(*to) < 45.0 {
                    let out = -dir * 0.9 + Vec3::Y * 0.3;
                    for _ in 0..2 {
                        let drift = out * kit.rng.range(0.5, 1.2) + Vec3::new(kit.rng.range(-0.3, 0.3), kit.rng.range(0.1, 0.4), kit.rng.range(-0.3, 0.3));
                        let size = Vec3::splat(kit.rng.range(0.12, 0.2));
                        let life = kit.rng.range(0.5, 0.9);
                        kit.spawn(Shape::Shard, Paint::Dark, *to - dir * 0.08, Bit::new(drift, life, size).drag(2.5).grow(0.6, 2.2).ramp(if ground { Ramp::Dust } else { Ramp::Smoke }));
                    }
                }
            }
            FxEvent::Hurt { player } => {
                last_hurt = Some(*player);
                if *player == me && full {
                    punch.aberration = punch.aberration.max(0.35);
                }
            }
            FxEvent::HitMarker { player, kill, head } if *player == me => {
                // Which robot, and where on it.
                let victim = last_hurt.or_else(|| punch.last_shot.and_then(|(_, to, _)| body_near(&people, to, 1.6)).map(|b| b.0));
                let body = victim.and_then(|v| people.iter().find(|b| b.0 == v).copied());
                let shot = punch.last_shot.filter(|(_, _, t)| now - t < 0.6);
                let at = match (body, shot) {
                    (Some(b), Some((_, to, _))) if (b.2 + Vec3::Y).distance(to) < 1.6 => to,
                    (Some(b), _) => b.2 + Vec3::Y * if *head { 1.75 } else { 1.15 },
                    (None, Some((_, to, _))) => to,
                    _ => continue,
                };
                let toward = shot.map_or(Vec3::ZERO, |(from, to, _)| (from - to).normalize_or_zero());
                // The tick climbs through a burst; a headshot rings.
                punch.combo = if punch.since_hit < COMBO { punch.combo + 1 } else { 0 };
                punch.since_hit = 0.0;
                let climb = 1.0 + (punch.combo.min(10) as f32) * 0.035;
                if *kill {
                    // Each kill in a multi-kill chain rings a step higher.
                    punch.chain = if punch.since_kill < 4.0 { punch.chain + 1 } else { 1 };
                    punch.since_kill = 0.0;
                    let step = 1.0 + 0.0595 * (punch.chain.min(6) - 1) as f32;
                    sounds.write(PlaySfx::ui(if *head { Sfx::KillHeadshot } else { Sfx::KillConfirm }).pitched(step));
                } else {
                    sounds.write(PlaySfx::ui(Sfx::HitTick).pitched(climb * if *head { 1.25 } else { 1.0 }).quiet(0.85));
                    if *head {
                        sounds.write(PlaySfx::ui(Sfx::Headshot).quiet(0.45));
                    }
                }
                // Sparks and scrap in their colours, a hot flash, a bolt or two.
                let team = body.map_or(1, |b| b.1);
                kit.flash(at, if *head { 0.14 } else { 0.09 }, 0.07);
                kit.sparks(at, if *head { 9 } else { 5 }, &[team_light(team), Paint::Spark, Paint::Glow], 7.0, (-0.2, 1.0), toward * 0.8);
                kit.chips(at, 2, &[team_paint(team), Paint::Steel], 3.5, 0.06, Shape::Cube, toward * 0.6);
                if kit.rng.chance(0.4) {
                    kit.chips(at, 1, &[Paint::Metal], 3.0, 0.045, Shape::Cylinder, toward * 0.6);
                }
                // A soft white wash at the screen edges.
                if !*kill {
                    punch.edge = if *head { Vec3::new(1.0, 0.9, 0.55) } else { Vec3::ONE };
                    punch.edge_a = punch.edge_a.max(if *head { 0.16 } else { 0.08 } * calm);
                }
                // The damage number.
                if let Some(victim) = victim {
                    let base = popups.hp_before(victim, now);
                    let drift = kit.rng.range(-1.0, 1.0);
                    let slot = popups.slots.iter().position(|p| p.as_ref().is_some_and(|p| p.victim == victim && p.age < 0.75 && !p.kill));
                    match slot {
                        Some(i) => {
                            if let Some(p) = popups.slots[i].as_mut() {
                                p.age = 0.0;
                                p.pop = 1.0;
                                p.head |= *head;
                                p.kill |= *kill;
                                p.at = at;
                            }
                        }
                        None => {
                            let i = popups.slots.iter().position(Option::is_none).unwrap_or_else(|| {
                                popups.slots.iter().enumerate().max_by(|a, b| a.1.as_ref().map_or(0.0, |p| p.age).total_cmp(&b.1.as_ref().map_or(0.0, |p| p.age))).map_or(0, |(i, _)| i)
                            });
                            let hp_now = bodies.iter().find(|(p, ..)| p.id == victim).map_or(0.0, |(_, _, h)| h.hp);
                            popups.slots[i] = Some(Popup { victim, base: base.unwrap_or(hp_now), shown: 0.0, at, age: 0.0, pop: 1.0, head: *head, kill: *kill, drift });
                        }
                    }
                }
            }
            FxEvent::Killed { victim, by, head, blast } => {
                let Some(body) = people.iter().find(|b| b.0 == *victim).copied() else { continue };
                let chest = body.2 + Vec3::Y * 1.1;
                let team = body.1;
                let near = camera.translation.distance(chest) < 80.0;
                if camera.translation.distance(chest) < 50.0 {
                    // It winds down, then hits the ground.
                    sounds.write(PlaySfx::new(Sfx::PowerDown, Some(chest)).quiet(0.55));
                    sounds.write(PlaySfx::new(Sfx::BodyFall, Some(body.2)).later(if *head { 0.95 } else { 0.8 }).quiet(0.9));
                }
                if near {
                    // The robot gives up: a pop of light, sparks, oil smoke, scrap and bolts, a ring of dust.
                    kit.flash(chest, 0.35, 0.14);
                    kit.sparks(chest, 12, &[team_light(team), Paint::Spark, Paint::Glow], 9.0, (-0.1, 1.0), Vec3::ZERO);
                    kit.chips(chest, 5, &[team_paint(team), Paint::Steel, Paint::Dark], 5.0, 0.08, Shape::Cube, Vec3::Y * 0.3);
                    kit.chips(chest, 3, &[Paint::Metal, Paint::Iron], 4.0, 0.05, Shape::Cylinder, Vec3::Y * 0.3);
                    kit.puffs(chest, 4, Ramp::Smoke, 0.35, 1.4, 0.4, 1.8);
                    let floor = map.height(body.2.x, body.2.z).max(body.2.y - 0.1);
                    kit.ring(Vec3::new(body.2.x, floor + 0.06, body.2.z), 2.4, Ramp::Dust, 0.45);
                    if *head {
                        // The head pops off (see `wreck`): a fountain of sparks from the neck.
                        let neck = body.2 + Vec3::Y * 1.6;
                        kit.sparks(neck, 14, &[Paint::Spark, team_light(team), Paint::White], 8.0, (1.2, 3.0), Vec3::Y);
                        kit.chips(neck, 4, &[Paint::Screen, team_light(team)], 4.0, 0.06, Shape::Cube, Vec3::Y);
                        kit.flash(neck, 0.3, 0.12);
                    }
                }
                if *by != me || *victim == me {
                    continue;
                }
                // Ours: hold the frame, slam the X, count the XP.
                punch.streak += 1;
                if full && punch.stop_cool <= 0.0 {
                    punch.stop = if *head { 0.09 } else { 0.06 };
                    punch.stop_frames = 0;
                    punch.stop_cool = 0.4;
                }
                punch.confirm = 0.0;
                punch.confirm_head = *head;
                punch.edge = if *head { Vec3::new(1.0, 0.8, 0.3) } else { Vec3::new(1.0, 0.45, 0.3) };
                punch.edge_a = punch.edge_a.max(0.3 * calm);
                if full {
                    punch.aberration = punch.aberration.max(if *head { 0.7 } else { 0.45 });
                }
                view.shake += 0.25 * calm;
                if punch.xp_age > XP_CHAIN {
                    punch.xp_target = 0.0;
                    punch.xp_shown = 0.0;
                }
                punch.xp_age = 0.0;
                punch.xp_target += 100.0 + if *head { 50.0 } else { 0.0 } + if *blast { 25.0 } else { 0.0 };
                if full && near {
                    kit.light(chest, Color::srgb(1.0, 0.7, 0.4), 400_000.0, 9.0, 0.18, flare_count);
                    flare_count += 1;
                }
            }
            FxEvent::LimbLost { player, limb, by } => {
                if let Some(state) = flinches.0.get_mut(player) {
                    // Whatever was left of it jerks hard as it goes.
                    for (slot, bit) in [oz_sim::player::limb::LEFT_ARM, oz_sim::player::limb::RIGHT_ARM, oz_sim::player::limb::LEFT_LEG, oz_sim::player::limb::RIGHT_LEG].iter().enumerate() {
                        if limb & bit != 0 {
                            state.limbs[slot].kick(9.0);
                        }
                    }
                }
                if *by == me {
                    punch.edge = Vec3::new(1.0, 0.85, 0.4);
                    punch.edge_a = punch.edge_a.max(0.14 * calm);
                    punch.xp_target += 10.0;
                }
            }
            FxEvent::Explosion { at, radius } => {
                let distance = camera.translation.distance(*at);
                explode(&mut kit, *at, *radius, &map, distance, flare_count);
                flare_count += 1;
                // Up close the whole screen takes it: shake, fringing, an orange wash.
                let close = (1.0 - distance / (radius * 6.0 + 10.0)).clamp(0.0, 1.0);
                view.shake += close * radius * 0.25 * calm;
                if full {
                    punch.aberration = punch.aberration.max(close * 0.9);
                }
                punch.edge = Vec3::new(1.0, 0.55, 0.2);
                punch.edge_a = punch.edge_a.max(close * 0.35 * calm);
                // Sound travels: far blasts are seen before they are heard.
                let delay = (distance / 340.0).min(0.5);
                sounds.write(PlaySfx::new(Sfx::Rumble, Some(*at)).later(delay).quiet((0.6 + radius * 0.08).min(1.0)));
                if distance < 40.0 {
                    sounds.write(PlaySfx::new(Sfx::Crack, Some(*at)).later(delay).quiet(0.8));
                }
            }
            _ => {}
        }
    }
}

/// The layered blast.
fn explode(kit: &mut Kit, at: Vec3, radius: f32, map: &Map, distance: f32, flares: usize) {
    let r = radius.clamp(1.5, 8.0);
    let floor = map.height(at.x, at.z);
    let grounded = at.y - floor < r * 0.6;
    // Light first: a blinding flicker that lights the street.
    kit.light(at + Vec3::Y * 0.8, Color::srgb(1.0, 0.62, 0.28), 6_000_000.0 * (r / 4.0), r * 7.0, 0.42, flares);
    // White-hot core.
    kit.flash(at + Vec3::Y * 0.3, r * 0.32, 0.16);
    // Fire lobes rolling out and up, cooling as they go.
    for _ in 0..kit.count(12) {
        let out = kit.dir((0.0, 1.0));
        let velocity = out * r * kit.rng.range(1.4, 2.6) + Vec3::Y * r * 0.4;
        let size = Vec3::splat(r * kit.rng.range(0.14, 0.24));
        let life = kit.rng.range(0.45, 0.75);
        kit.spawn(Shape::Shard, Paint::Glow, at + out * r * 0.1, Bit::new(velocity, life, size).drag(4.5).gravity(-2.5).grow(0.7, 1.9).ramp(Ramp::Fire));
    }
    // A rising fireball cap.
    kit.spawn(Shape::Shard, Paint::Glow, at + Vec3::Y * r * 0.3, Bit::new(Vec3::Y * r * 0.9, 0.7, Vec3::splat(r * 0.3)).drag(1.5).grow(0.6, 1.6).ramp(Ramp::Fire));
    // The smoke column: thick and dark, it climbs, spreads and drifts with the wind for seconds.
    let wind = Vec3::new(0.6, 0.0, 0.25);
    for i in 0..kit.count(11) {
        let height = i as f32 / 11.0;
        let drift = Vec3::new(kit.rng.range(-0.6, 0.6), r * (0.25 + height * 0.55), kit.rng.range(-0.6, 0.6)) + wind;
        let size = Vec3::splat(r * kit.rng.range(0.2, 0.32));
        let life = kit.rng.range(3.5, 5.5);
        let jitter = Vec3::new(kit.rng.range(-0.4, 0.4), kit.rng.range(0.0, 0.6), kit.rng.range(-0.4, 0.4)) * r * 0.4;
        kit.spawn(Shape::Shard, Paint::Dark, at + jitter, Bit::new(drift, life, size).drag(0.35).grow(0.6, 2.4).ramp(Ramp::Smoke));
    }
    // Debris thrown high, trailing smoke, bouncing where it lands.
    for i in 0..kit.count(9) {
        let out = kit.dir((0.6, 1.6));
        let velocity = out * r * kit.rng.range(2.0, 3.4);
        let s = r * kit.rng.range(0.03, 0.06);
        let spin = Vec3::new(kit.rng.range(-12.0, 12.0), kit.rng.range(-12.0, 12.0), kit.rng.range(-12.0, 12.0));
        let paint = [Paint::Dark, Paint::Rust, Paint::Steel, Paint::Concrete][i % 4];
        let bit = Bit::new(velocity, kit.rng.range(1.6, 2.6), Vec3::new(s, s * 0.7, s * 1.2)).gravity(13.0).drag(0.15).spin(spin).bouncy().grow(1.0, 0.7);
        kit.spawn(Shape::Cube, paint, at + Vec3::Y * 0.3, if i % 3 == 0 { bit } else { bit.trail(0.05) });
    }
    // Sparks everywhere.
    kit.sparks(at + Vec3::Y * 0.3, 22, &[Paint::Spark, Paint::Glow, Paint::White], r * 4.0, (-0.1, 1.2), Vec3::ZERO);
    if grounded {
        // The shockwave: a bright ring and a dust ring racing out along the ground, and a skirt of dust.
        let ground = Vec3::new(at.x, floor + 0.08, at.z);
        kit.ring(ground, r * 2.2, Ramp::Fire, 0.22);
        kit.ring(ground + Vec3::Y * 0.02, r * 3.0, Ramp::Dust, 0.6);
        for _ in 0..kit.count(10) {
            let angle = kit.rng.range(0.0, TAU);
            let out = Vec3::new(angle.cos(), 0.0, angle.sin());
            let velocity = out * r * kit.rng.range(2.0, 3.2) + Vec3::Y * 0.6;
            let size = Vec3::splat(r * kit.rng.range(0.12, 0.2));
            let life = kit.rng.range(1.0, 1.6);
            kit.spawn(Shape::Shard, Paint::Dark, ground + out * r * 0.3, Bit::new(velocity, life, size).drag(2.8).grow(0.5, 2.3).ramp(Ramp::Dust));
        }
    }
    let _ = distance;
}

fn tick_bits(
    time: Res<Time>,
    mut commands: Commands,
    mats: Option<Res<Mats>>,
    art: Res<Art>,
    map: Res<Map>,
    punch: Res<Punch>,
    mut bits: Query<(Entity, &mut Transform, &mut Bit, &mut MeshMaterial3d<StandardMaterial>)>,
) {
    let Some(mats) = mats else { return };
    // Held frames do not age the effects either, so the hold reads as a freeze.
    if punch.frozen {
        return;
    }
    let dt = time.delta_secs().min(0.05);
    let mut trails = Vec::new();
    let room = MAX_BITS[0].saturating_sub(bits.iter().len());
    for (entity, mut transform, mut bit, mut material) in &mut bits {
        bit.life -= dt;
        if bit.life <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        let age = 1.0 - bit.life / bit.initial;
        bit.velocity.y -= bit.gravity * dt;
        let drag = (-bit.drag * dt).exp();
        bit.velocity *= drag;
        transform.translation += bit.velocity * dt;
        if bit.bounce {
            let floor = map.height(transform.translation.x, transform.translation.z) + bit.size.y * 0.5;
            if transform.translation.y < floor {
                transform.translation.y = floor;
                if bit.velocity.y < -1.5 {
                    bit.velocity.y = -bit.velocity.y * 0.35;
                    bit.velocity.x *= 0.6;
                    bit.velocity.z *= 0.6;
                    bit.spin *= 0.5;
                } else {
                    // Settled: lie still, stop smoking.
                    bit.velocity = Vec3::ZERO;
                    bit.spin = Vec3::ZERO;
                    bit.trail = 0.0;
                }
            }
        }
        if bit.spin != Vec3::ZERO {
            let spin = bit.spin * dt;
            transform.rotate_local(Quat::from_euler(EulerRot::XYZ, spin.x, spin.y, spin.z));
        }
        // Scale over life: from `grow.0` to `grow.1`, eased so puffs swell fast and settle.
        let k = 1.0 - (1.0 - age) * (1.0 - age);
        let scale = bit.grow.0 + (bit.grow.1 - bit.grow.0) * k;
        if bit.streak {
            let speed = bit.velocity.length();
            if speed > 0.05 {
                transform.rotation = Quat::from_rotation_arc(Vec3::Z, bit.velocity / speed);
            }
            let length = (speed * 0.035).clamp(0.04, 0.6);
            transform.scale = Vec3::new(bit.size.x, bit.size.y, length) * (1.0 - age * 0.5);
        } else {
            transform.scale = bit.size * scale.max(0.01);
        }
        let ramp = match bit.ramp {
            Ramp::Solid => None,
            Ramp::Fire => Some(&mats.fire),
            Ramp::Smoke => Some(&mats.smoke),
            Ramp::Dust => Some(&mats.dust),
        };
        if let Some(ramp) = ramp {
            let stage = ((age * ramp.len() as f32) as usize).min(ramp.len() - 1) as u8;
            if stage != bit.stage {
                bit.stage = stage;
                material.0 = ramp[stage as usize].clone();
            }
        }
        if bit.trail > 0.0 {
            bit.trail_t -= dt;
            if bit.trail_t <= 0.0 {
                bit.trail_t = bit.trail;
                trails.push(transform.translation);
            }
        }
    }
    for (i, at) in trails.into_iter().take(room).enumerate() {
        let size = Vec3::splat(0.14 + (i % 3) as f32 * 0.03);
        let bit = Bit::new(Vec3::Y * 0.5, 0.9, size).drag(1.0).grow(0.6, 2.0).ramp(Ramp::Smoke);
        commands.spawn((MatchEntity, Mesh3d(art.primitive(Shape::Shard)), MeshMaterial3d(mats.smoke[1].clone()), Transform::from_translation(at).with_scale(size * 0.6), bit, NotShadowCaster));
    }
}

fn tick_flares(time: Res<Time>, mut commands: Commands, mut flares: Query<(Entity, &mut PointLight, &mut Flare)>) {
    let dt = time.delta_secs().min(0.05);
    let t = time.elapsed_secs();
    for (entity, mut light, mut flare) in &mut flares {
        flare.life -= dt;
        if flare.life <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        let k = flare.life / flare.initial;
        let flicker = 0.75 + 0.25 * (t * 47.0 + flare.flicker).sin() * (t * 31.0 + flare.flicker * 2.0).cos();
        light.intensity = flare.intensity * k * k * flicker;
    }
}

/// A damped spring for the flinch.
#[derive(Clone, Copy, Default)]
struct Wobble {
    value: f32,
    velocity: f32,
}

impl Wobble {
    fn kick(&mut self, impulse: f32) {
        self.velocity += impulse;
    }

    fn step(&mut self, dt: f32) -> f32 {
        let (stiffness, damping) = (220.0, 0.42);
        let c = 2.0 * f32::sqrt(stiffness) * damping;
        let h = dt / 4.0;
        for _ in 0..4 {
            let accel = -stiffness * self.value - c * self.velocity;
            self.velocity += accel * h;
            self.value += self.velocity * h;
        }
        self.value = self.value.clamp(-1.5, 1.5);
        self.value
    }
}

/// How one robot is flinching right now.
#[derive(Default)]
struct FlinchState {
    /// Torso pitch away from the hit, and a little twist.
    body: Wobble,
    twist: Wobble,
    /// Left arm, right arm, left leg, right leg knocked back.
    limbs: [Wobble; 4],
    /// The direction of the last round to land on it, and when.
    push: Option<(Vec3, f32)>,
    /// Which way the body is tipping (robot-local, horizontal).
    axis: Vec3,
    /// Last frame's health, limb health, and the rotations we added to each joint.
    hp: f32,
    limb_hp: [f32; 4],
    applied: Vec<(Entity, Quat)>,
}

#[derive(Resource, Default)]
struct Flinches(HashMap<u8, FlinchState>);

/// Robots that lose health flinch; limbs that lose health are knocked back.
fn watch_bodies(time: Res<Time>, settings: Res<Settings>, mut flinches: ResMut<Flinches>, mut popups: ResMut<Popups>, bodies: Query<(&Player, &Transform, &Health)>, mut seed: Local<u32>) {
    let now = time.elapsed_secs();
    let strength = if full(&settings) { 1.0 } else { 0.5 };
    for (player, transform, health) in &bodies {
        // Health history for the damage numbers.
        let history = popups.history.entry(player.id).or_default();
        history.retain(|(t, _)| now - t < 0.5);
        history.push((now, health.hp));
        let state = flinches.0.entry(player.id).or_default();
        let lost = state.hp - health.hp;
        if lost > 0.5 && health.alive() {
            *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let side = if *seed & 0x100 != 0 { 1.0 } else { -1.0 };
            let push = state.push.filter(|(_, t)| now - t < 0.4).map_or(transform.rotation * Vec3::Z, |(d, _)| d);
            let local = transform.rotation.inverse() * push;
            state.axis = Vec3::new(local.x, 0.0, local.z).normalize_or(Vec3::Z);
            let hard = (lost / 25.0).clamp(0.5, 1.6) * strength;
            state.body.kick(9.0 * hard);
            state.twist.kick(side * 6.0 * hard);
        }
        for slot in 0..4 {
            if state.limb_hp[slot] - player.limb_hp[slot] > 0.5 {
                state.limbs[slot].kick(12.0 * strength);
            }
        }
        state.hp = health.hp;
        state.limb_hp = player.limb_hp;
    }
    let alive: Vec<u8> = bodies.iter().map(|(p, ..)| p.id).collect();
    flinches.0.retain(|id, _| alive.contains(id));
    popups.history.retain(|id, _| alive.contains(id));
}

/// Takes last frame's flinch back out of the joints before the rig is posed.
fn unflinch(mut flinches: ResMut<Flinches>, mut transforms: Query<&mut Transform, Without<Rig>>) {
    for state in flinches.0.values_mut() {
        for (entity, added) in state.applied.drain(..) {
            if let Ok(mut transform) = transforms.get_mut(entity) {
                transform.rotation = (transform.rotation * added.inverse()).normalize();
            }
        }
    }
}

/// Adds this frame's flinch on top of the posed rig.
fn flinch(time: Res<Time>, mut flinches: ResMut<Flinches>, rigs: Query<(&Rig, &Player, &Health)>, mut transforms: Query<&mut Transform, Without<Rig>>) {
    let dt = time.delta_secs().clamp(1e-4, 0.05);
    for (rig, player, health) in &rigs {
        let Some(state) = flinches.0.get_mut(&player.id) else { continue };
        let body = state.body.step(dt);
        let twist = state.twist.step(dt);
        let limbs = state.limbs.each_mut().map(|w| w.step(dt));
        if !health.alive() {
            continue;
        }
        let axis = Vec3::Y.cross(state.axis).normalize_or(Vec3::X);
        let mut add = |entity: Entity, rotation: Quat| {
            if rotation.angle_between(Quat::IDENTITY) < 1e-3 {
                return;
            }
            if let Ok(mut transform) = transforms.get_mut(entity) {
                transform.rotation = (transform.rotation * rotation).normalize();
                state.applied.push((entity, rotation));
            }
        };
        add(rig.torso, Quat::from_axis_angle(axis, body * 0.07) * Quat::from_rotation_y(twist * 0.05));
        add(rig.head, Quat::from_axis_angle(axis, body * 0.06) * Quat::from_rotation_z(twist * 0.06));
        for side in 0..2 {
            add(rig.arms[side], Quat::from_rotation_x(limbs[side] * 0.12) * Quat::from_rotation_z((side as f32 * 2.0 - 1.0) * limbs[side] * 0.05));
            add(rig.thighs[side], Quat::from_rotation_x(-limbs[2 + side] * 0.08));
        }
    }
}

/// Edge glow and colour fringing on the world camera's post-process.
fn screen_pulse(time: Res<Time>, settings: Res<Settings>, mut punch: ResMut<Punch>, mut cameras: Query<&mut Retro, With<WorldCamera>>, mut view: ResMut<ViewState>) {
    let dt = time.delta_secs().min(0.05);
    punch.edge_a = (punch.edge_a - dt * 1.6 * punch.edge_a.max(0.15)).max(0.0);
    punch.aberration = (punch.aberration - dt * 2.4 * punch.aberration.max(0.2)).max(0.0);
    if !full(&settings) {
        punch.aberration = 0.0;
        // Calmer camera for those who asked for it.
        view.shake = view.shake.min(0.9);
    }
    for mut retro in &mut cameras {
        retro.aberration = punch.aberration;
        retro.edge_r = punch.edge.x;
        retro.edge_g = punch.edge.y;
        retro.edge_b = punch.edge.z;
        retro.edge_a = punch.edge_a;
    }
}

/// The kill hit-stop: the world camera stops drawing for a few frames, so the
/// last frame holds on screen while the game (and the HUD) runs on.
fn hit_stop(time: Res<Time>, mut punch: ResMut<Punch>, mut cameras: Query<&mut Camera, With<WorldCamera>>) {
    let dt = time.delta_secs();
    punch.stop_cool = (punch.stop_cool - dt).max(0.0);
    // Hold for the planned time, but at least two frames and never more than eight.
    let holding = punch.stop > 0.0 || (punch.stop_frames > 0 && punch.stop_frames < 2);
    if holding {
        punch.stop -= dt;
        punch.stop_frames += 1;
    } else {
        punch.stop_frames = 0;
    }
    let hold = holding && punch.stop_frames <= 8;
    if hold != punch.frozen {
        punch.frozen = hold;
        for mut camera in &mut cameras {
            camera.is_active = !hold;
        }
    }
}

/// Leaving a match never leaves the world view frozen.
fn thaw(mut punch: ResMut<Punch>, mut cameras: Query<&mut Camera, With<WorldCamera>>) {
    if punch.frozen {
        punch.frozen = false;
        for mut camera in &mut cameras {
            camera.is_active = true;
        }
    }
    punch.stop = 0.0;
}

// ---- Overlay: damage numbers, kill-confirm X, skull, XP ticker. ----

#[derive(Component)]
struct PopupText(usize);

#[derive(Component)]
struct ConfirmRoot;

/// One arm of the kill X (0..4).
#[derive(Component)]
struct ConfirmArm(u8);

/// One ray of the burst (0..8).
#[derive(Component)]
struct ConfirmRay(u8);

#[derive(Component)]
struct Skull;

#[derive(Component)]
struct XpText;

#[derive(Component)]
struct StreakText;

const SKULL: [&str; 8] = [".#####.", "#######", "#.###.#", "#..#..#", "#######", ".##.##.", ".#.#.#.", ".#####."];

fn spawn_overlay(mut commands: Commands) {
    for i in 0..POPUPS {
        commands.spawn((
            MatchEntity,
            PopupText(i),
            Text::new(""),
            TextFont { font_size: 18.0, ..default() },
            TextColor(Color::WHITE),
            TextShadow { offset: Vec2::splat(2.0), color: Color::srgba(0.0, 0.0, 0.0, 0.8) },
            Node { position_type: PositionType::Absolute, ..default() },
            Visibility::Hidden,
            GlobalZIndex(-30),
        ));
    }
    // Centred on the crosshair: a zero-size anchor everything hangs off.
    commands
        .spawn((
            MatchEntity,
            ConfirmRoot,
            Node { position_type: PositionType::Absolute, left: Val::Percent(50.0), top: Val::Percent(50.0), width: Val::Px(0.0), height: Val::Px(0.0), ..default() },
            Visibility::Hidden,
            GlobalZIndex(25),
        ))
        .with_children(|root| {
            for i in 0..4u8 {
                root.spawn((ConfirmArm(i), Node { position_type: PositionType::Absolute, width: Val::Px(5.0), height: Val::Px(16.0), ..default() }, BackgroundColor(Color::WHITE)));
            }
            for i in 0..8u8 {
                root.spawn((ConfirmRay(i), Node { position_type: PositionType::Absolute, width: Val::Px(2.0), height: Val::Px(9.0), ..default() }, BackgroundColor(Color::WHITE)));
            }
            // The skull, in chunky pixels, under the crosshair.
            let px = 3.0;
            root.spawn((Skull, Node { position_type: PositionType::Absolute, left: Val::Px(-px * 3.5 - 34.0), top: Val::Px(40.0), width: Val::Px(px * 7.0), height: Val::Px(px * 8.0), ..default() }))
                .with_children(|skull| {
                    for (y, row) in SKULL.iter().enumerate() {
                        for (x, c) in row.chars().enumerate() {
                            if c == '#' {
                                skull.spawn((
                                    Node { position_type: PositionType::Absolute, left: Val::Px(x as f32 * px), top: Val::Px(y as f32 * px), width: Val::Px(px), height: Val::Px(px), ..default() },
                                    BackgroundColor(Color::WHITE),
                                ));
                            }
                        }
                    }
                });
            root.spawn((
                XpText,
                Text::new("+100"),
                TextFont { font_size: 22.0, ..default() },
                TextColor(Color::srgb(1.0, 0.85, 0.35)),
                TextShadow { offset: Vec2::splat(2.0), color: Color::srgba(0.0, 0.0, 0.0, 0.85) },
                Node { position_type: PositionType::Absolute, left: Val::Px(-6.0), top: Val::Px(38.0), ..default() },
            ));
            root.spawn((
                StreakText,
                Text::new(""),
                TextFont { font_size: 13.0, ..default() },
                TextColor(Color::srgb(1.0, 0.55, 0.4)),
                TextShadow { offset: Vec2::splat(1.0), color: Color::srgba(0.0, 0.0, 0.0, 0.85) },
                Node { position_type: PositionType::Absolute, left: Val::Px(-6.0), top: Val::Px(64.0), ..default() },
            ));
        });
}

fn show_popups(
    time: Res<Time>,
    mut popups: ResMut<Popups>,
    punch: Res<Punch>,
    bodies: Query<(&Player, &Health)>,
    camera: Query<(&Camera, &GlobalTransform), With<WorldCamera>>,
    mut texts: Query<(&PopupText, &mut Text, &mut TextColor, &mut TextFont, &mut Node, &mut Visibility)>,
) {
    let dt = if punch.frozen { 0.0 } else { time.delta_secs().min(0.05) };
    let camera = camera.single().ok();
    for slot in popups.slots.iter_mut() {
        let Some(p) = slot else { continue };
        p.age += dt;
        p.pop = (p.pop - dt * 6.0).max(0.0);
        if let Some((_, health)) = bodies.iter().find(|(pl, _)| pl.id == p.victim) {
            p.shown = p.shown.max(p.base - health.hp.max(0.0));
        }
        let life = if p.kill { 1.3 } else { 0.95 };
        // Nothing ever came off (the host's numbers did not arrive): drop it quietly.
        if p.age > life || (p.shown < 0.5 && p.age > 0.5) {
            *slot = None;
        }
    }
    for (index, mut text, mut color, mut font, mut node, mut visibility) in &mut texts {
        let shown = popups.slots.get(index.0).and_then(|p| p.as_ref()).filter(|p| p.shown >= 0.5);
        let projected = shown.and_then(|p| {
            let (camera, view) = camera?;
            let rise = Vec3::Y * (0.25 + p.age * 0.7);
            camera.world_to_ndc(view, p.at + rise).filter(|ndc| ndc.z > 0.0 && ndc.z < 1.0).map(|ndc| (p, ndc))
        });
        let Some((p, ndc)) = projected else {
            if *visibility != Visibility::Hidden {
                *visibility = Visibility::Hidden;
            }
            continue;
        };
        *visibility = Visibility::Inherited;
        let amount = p.shown.round() as i32;
        let label = if p.head { format!("{amount}!") } else { format!("{amount}") };
        if text.0 != label {
            text.0 = label;
        }
        // White for a graze, amber for a solid hit, hot orange for a big one, gold for heads, red for the kill.
        let tint = if p.kill {
            Color::srgb(1.0, 0.32, 0.26)
        } else if p.head {
            Color::srgb(1.0, 0.86, 0.25)
        } else if p.shown >= 60.0 {
            Color::srgb(1.0, 0.55, 0.25)
        } else if p.shown >= 30.0 {
            Color::srgb(1.0, 0.82, 0.55)
        } else {
            Color::WHITE
        };
        let life = if p.kill { 1.3 } else { 0.95 };
        let fade = (1.0 - ((p.age - life * 0.6) / (life * 0.4)).clamp(0.0, 1.0)).powf(1.5);
        color.0 = tint.with_alpha(fade);
        let size = (if p.kill || p.head { 22.0 } else { 17.0 } + (p.shown / 12.0).min(8.0)) * (1.0 + 0.6 * p.pop * p.pop);
        font.font_size = size.round();
        let x = (ndc.x + 1.0) * 50.0;
        let y = (1.0 - ndc.y) * 50.0;
        node.left = Val::Percent(x);
        node.top = Val::Percent(y);
        // Each number slides a little to its own side as it rises.
        node.margin = UiRect { left: Val::Px(p.drift * 18.0 * p.age.min(1.0) - size * 0.6), top: Val::Px(-size * 0.5), ..default() };
    }
}

#[allow(clippy::type_complexity)]
fn show_confirm(
    time: Res<Time>,
    mut punch: ResMut<Punch>,
    mut root: Query<&mut Visibility, With<ConfirmRoot>>,
    mut arms: Query<(&ConfirmArm, &mut Node, &mut Transform, &mut BackgroundColor), Without<ConfirmRay>>,
    mut rays: Query<(&ConfirmRay, &mut Node, &mut Transform, &mut BackgroundColor), Without<ConfirmArm>>,
    mut skull: Query<(&mut Transform, &Children), (With<Skull>, Without<ConfirmArm>, Without<ConfirmRay>)>,
    mut pixels: Query<&mut BackgroundColor, (Without<ConfirmArm>, Without<ConfirmRay>)>,
    mut xp: Query<(&mut Text, &mut TextColor, &mut Transform), (With<XpText>, Without<StreakText>, Without<Skull>, Without<ConfirmArm>, Without<ConfirmRay>)>,
    mut streak: Query<(&mut Text, &mut TextColor), (With<StreakText>, Without<XpText>)>,
    local: Query<(&Player, &Health), With<LocalPlayer>>,
) {
    let dt = time.delta_secs().min(0.05);
    punch.confirm += dt;
    // The ticker counts up quickly toward what has been earned.
    let gap = punch.xp_target - punch.xp_shown;
    punch.xp_shown += gap * (1.0 - (-dt * 9.0).exp()) + gap.signum() * (dt * 60.0).min(gap.abs());
    let alive = local.single().is_ok_and(|(_, h)| h.alive());
    if !alive {
        punch.streak = 0;
    }
    const SHOW: f32 = 1.1;
    let showing = punch.confirm < SHOW.max(if punch.xp_age < XP_CHAIN * 0.6 { 99.0 } else { 0.0 }) && alive && punch.xp_target > 0.0;
    let Ok(mut visibility) = root.single_mut() else { return };
    let wanted = if showing { Visibility::Inherited } else { Visibility::Hidden };
    if *visibility != wanted {
        *visibility = wanted;
    }
    if !showing {
        return;
    }
    let t = punch.confirm;
    let head = punch.confirm_head;
    let color = if head { Color::srgb(1.0, 0.85, 0.3) } else { Color::srgb(1.0, 0.3, 0.25) };
    // The X slams in from big and settles, then fades.
    let x_fade = (1.0 - ((t - 0.45) / 0.3).clamp(0.0, 1.0)).max(0.0);
    let slam = 1.0 + 1.3 * (1.0 - (t / 0.12).min(1.0)).powi(2);
    for (arm, mut node, mut transform, mut background) in &mut arms {
        let angle = [0.25, 0.75, 1.25, 1.75][arm.0 as usize] * std::f32::consts::PI;
        let reach = (13.0 + 3.0 * (1.0 - (t / 0.2).min(1.0))) * slam;
        let (s, c) = angle.sin_cos();
        node.left = Val::Px((c * reach - 2.5).round());
        node.top = Val::Px((s * reach - 8.0).round());
        node.width = Val::Px(if head { 6.0 } else { 5.0 });
        node.height = Val::Px((16.0 * slam.sqrt()).round());
        transform.rotation = Quat::from_rotation_z(angle + FRAC_PI_2);
        background.0 = if t < 0.05 { Color::WHITE } else { color.with_alpha(x_fade) };
    }
    // Rays burst outward and thin away.
    let burst = (t / 0.28).min(1.0);
    for (ray, mut node, mut transform, mut background) in &mut rays {
        let angle = ray.0 as f32 / 8.0 * TAU + 0.2;
        let reach = 16.0 + 46.0 * (1.0 - (1.0 - burst).powi(3));
        let (s, c) = angle.sin_cos();
        node.left = Val::Px((c * reach - 1.0).round());
        node.top = Val::Px((s * reach - 4.5).round());
        node.height = Val::Px((12.0 * (1.0 - burst) + 2.0).round());
        transform.rotation = Quat::from_rotation_z(angle + FRAC_PI_2);
        background.0 = if burst >= 1.0 { Color::NONE } else if burst < 0.3 { Color::WHITE.with_alpha(1.0 - burst) } else { color.with_alpha(1.0 - burst) };
    }
    // The skull and the XP hang on while kills keep coming.
    let hang = (1.0 - ((punch.xp_age - XP_CHAIN * 0.45) / (XP_CHAIN * 0.15)).clamp(0.0, 1.0)).max(0.0);
    let pop = 1.0 + 0.5 * (1.0 - (t / 0.15).min(1.0)).powi(2);
    if let Ok((mut transform, children)) = skull.single_mut() {
        transform.scale = Vec3::splat(pop);
        let ink = if head { Color::srgb(1.0, 0.85, 0.3) } else { Color::srgb(1.0, 0.92, 0.85) };
        for child in children.iter() {
            if let Ok(mut background) = pixels.get_mut(child) {
                background.0 = ink.with_alpha(hang);
            }
        }
    }
    if let Ok((mut text, mut tint, mut transform)) = xp.single_mut() {
        let label = format!("+{}", punch.xp_shown.round() as i32);
        if text.0 != label {
            text.0 = label;
        }
        tint.0 = Color::srgb(1.0, 0.85, 0.35).with_alpha(hang);
        transform.scale = Vec3::splat(pop);
    }
    if let Ok((mut text, mut tint)) = streak.single_mut() {
        let label = if punch.streak >= 2 { format!("STREAK {}", punch.streak) } else { String::new() };
        if text.0 != label {
            text.0 = label;
        }
        tint.0 = Color::srgb(1.0, 0.55, 0.4).with_alpha(hang);
    }
}

/// `OZ_JUICE_DEMO=1`: blasts in front of the camera, hits and kills on the
/// nearest robot in view (through the simulation's own damage path, so offline
/// or hosting only), and a frame-rate log, for checking the effects.
#[allow(clippy::too_many_arguments)]
fn demo(
    time: Res<Time<Real>>,
    mut fx: EventWriter<FxEvent>,
    map: Res<Map>,
    role: Res<NetRole>,
    mut state: ResMut<MatchState>,
    mut local: Query<(&Player, &mut PlayerInput), With<LocalPlayer>>,
    mut bodies: Query<(&mut Player, &Transform, &mut Health), Without<LocalPlayer>>,
    cameras: Query<&Transform, With<WorldCamera>>,
    bits: Query<(), With<Bit>>,
    mut clock: Local<(f32, f32, u32, f32, u32)>,
    mut enabled: Local<Option<bool>>,
    mut watch: Local<(f32, Option<(Vec3, f32)>)>,
) {
    let enabled = *enabled.get_or_insert_with(|| std::env::var("OZ_JUICE_DEMO").is_ok_and(|v| v == "1"));
    if !enabled || *role == NetRole::Client {
        return;
    }
    let dt = time.delta_secs();
    let (blast, hit, frames, fps_t, step) = &mut *clock;
    *blast += dt;
    *hit += dt;
    *frames += 1;
    *fps_t += dt;
    if *fps_t >= 5.0 {
        let yaw = local.single().map_or(-1.0, |(_, input)| input.yaw);
        let eye = cameras.single().map_or(Vec3::ZERO, |c| c.translation);
        let enemies = bodies.iter().filter(|(_, _, h)| h.alive()).map(|(_, t, _)| t.translation.distance(eye) as i32).collect::<Vec<_>>();
        info!("juice demo: {:.0} fps, {} punch particles, yaw {yaw:.2}, others alive at {enemies:?} m", *frames as f32 / *fps_t, bits.iter().len());
        *frames = 0;
        *fps_t = 0.0;
    }
    let (Ok((me, mut input)), Ok(camera)) = (local.single_mut(), cameras.single()) else { return };
    let forward = camera.forward().as_vec3();
    // Settle in for a few seconds after the fight starts.
    let (warm, corpse) = &mut *watch;
    *warm = if state.phase == Phase::Live { *warm + dt } else { 0.0 };
    if *warm < 8.0 {
        return;
    }
    // After a kill, watch the body go down for a while.
    let now = time.elapsed_secs();
    let watching = corpse.filter(|(_, at)| now - at < 3.5).map(|(body, _)| body);
    // Otherwise look at the nearest robot (friend or foe: this is only a test), so there is something to hit.
    let eye = camera.translation;
    let look = watching.or_else(|| bodies.iter().filter(|(_, _, h)| h.alive()).map(|(_, t, _)| t.translation + Vec3::Y * 1.1).min_by(|a, b| a.distance(eye).total_cmp(&b.distance(eye))));
    if let Some(at) = look {
        let to = (at - eye).normalize_or(Vec3::NEG_Z);
        input.yaw = f32::atan2(-to.x, -to.z).rem_euclid(TAU);
        input.pitch = to.y.clamp(-0.9, 0.9).asin();
    }
    if *blast > 2.6 {
        *blast = 0.0;
        *step += 1;
        // Off to one side, so the blasts do not hide the robots.
        let flat = Vec3::new(forward.x, 0.0, forward.z).normalize_or(Vec3::NEG_Z);
        let ahead = camera.translation + flat * (16.0 + (*step % 3) as f32 * 6.0) + Vec3::new(-flat.z, 0.0, flat.x) * 11.0;
        let at = Vec3::new(ahead.x, map.height(ahead.x, ahead.z) + 0.3, ahead.z);
        fx.write(FxEvent::Explosion { at, radius: 4.0 });
    }
    if *hit > 0.16 && watching.is_none() {
        *hit = 0.0;
        let target = bodies
            .iter_mut()
            .filter(|(_, t, h)| h.alive() && (t.translation + Vec3::Y * 1.1 - eye).normalize_or_zero().dot(forward) > 0.8 && t.translation.distance(eye) < 150.0)
            .min_by(|a, b| a.1.translation.distance(eye).total_cmp(&b.1.translation.distance(eye)));
        if let Some((mut victim, transform, mut health)) = target {
            *step += 1;
            let head = *step % 5 == 0;
            let at = transform.translation + Vec3::Y * if head { 1.75 } else { 1.1 };
            fx.write(FxEvent::Shot { by: me.id, sound: 3, from: eye, to: at, beam: false, tracer: Paint::Glow, quiet: false, local: true });
            victim.last_hit = if head { oz_sim::player::part::HEAD } else { oz_sim::player::part::TORSO };
            let amount = if head { 22.0 } else { 7.0 };
            if oz_sim::player::damage_player(&mut health, &mut victim, amount, Some((&me.name, me.team, me.id)), &mut state, &mut fx) {
                *corpse = Some((transform.translation + Vec3::Y * 0.4, now));
            }
        }
    }
}

/// Fresh wrecks keep fizzing for a few seconds: sparks and smoke from the neck
/// (or the chest), thinning out as they go cold.
fn aftermath(
    time: Res<Time>,
    settings: Res<Settings>,
    mut commands: Commands,
    art: Res<Art>,
    mats: Option<Res<Mats>>,
    rigs: Query<(&Rig, &Health)>,
    joints: Query<&GlobalTransform>,
    cameras: Query<&Transform, With<WorldCamera>>,
    bits: Query<(), With<Bit>>,
    mut wait: Local<f32>,
    mut rng: Local<Option<Rng>>,
) {
    let Some(mats) = mats else { return };
    *wait -= time.delta_secs();
    if *wait > 0.0 {
        return;
    }
    let full = full(&settings);
    *wait = if full { 0.12 } else { 0.3 };
    let eye = cameras.single().map_or(Vec3::ZERO, |c| c.translation);
    let rng = rng.get_or_insert_with(|| Rng::new(0xa57e));
    let cap = MAX_BITS[(!full) as usize];
    let mut kit = Kit { commands: &mut commands, art: &art, mats: &mats, rng, budget: cap.saturating_sub(bits.iter().len()).min(24), scale: 1.0 };
    for (rig, health) in &rigs {
        if health.alive() || rig.dead_for > 4.0 || rig.dead_for < 0.2 {
            continue;
        }
        let headless = rig.hidden & crate::wreck::HIDE_HEAD != 0;
        let Ok(joint) = joints.get(if headless { rig.head } else { rig.torso }) else { continue };
        let at = joint.translation() + if headless { Vec3::ZERO } else { joint.up().as_vec3() * 0.3 };
        if at.distance(eye) > 60.0 || rig.hidden & crate::wreck::HIDE_ALL != 0 {
            continue;
        }
        // Livelier at first, fading toward the end.
        let heat = 1.0 - rig.dead_for / 4.0;
        if kit.rng.chance(0.35 + 0.6 * heat) {
            kit.sparks(at, if headless { 3 } else { 1 }, &[Paint::Spark, team_light(rig.team), Paint::Glow], 3.5, (0.4, 1.4), Vec3::ZERO);
        }
        if kit.rng.chance(0.5) {
            let drift = Vec3::new(kit.rng.range(-0.2, 0.2), kit.rng.range(0.6, 1.0), kit.rng.range(-0.2, 0.2));
            let size = Vec3::splat(kit.rng.range(0.1, 0.16));
            kit.spawn(Shape::Shard, Paint::Dark, at, Bit::new(drift, 1.4, size).drag(0.6).grow(0.5, 2.4).ramp(Ramp::Smoke));
        }
    }
}

/// Grenades and canisters clink each time they skip off the ground. Read from
/// how the thrown thing moves on this machine, so it works online too.
fn clinks(projectiles: Query<(Entity, &Transform, &Projectile)>, time: Res<Time>, mut sounds: EventWriter<PlaySfx>, mut last: Local<HashMap<Entity, (Vec3, f32)>>) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let mut seen = HashMap::default();
    for (entity, transform, shot) in &projectiles {
        if !matches!(shot.look, ShotLook::Grenade | ShotLook::Canister) {
            continue;
        }
        let at = transform.translation;
        let rise = last.get(&entity).map_or(0.0, |(before, _)| (at.y - before.y) / dt);
        if let Some((_, fall)) = last.get(&entity)
            && *fall < -1.2
            && rise > 0.2
        {
            // It was falling and now goes up: a bounce. Harder landings ring louder.
            sounds.write(PlaySfx::new(Sfx::Clink, Some(at)).quiet((-fall / 8.0).clamp(0.25, 1.0)));
        }
        seen.insert(entity, (at, rise));
    }
    *last = seen;
}
