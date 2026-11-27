//! One-shot effects: tracers, beams, sparks, gibs and explosions.
//!
//! All particles are unit primitives sharing the palette materials, so even a
//! full burst batches into a few draw calls. A hard cap keeps worst-case
//! fights cheap.

use crate::{
    ClientSet,
    art::Art,
    hud::Notices,
    view::{ViewState, WorldCamera},
};
use bevy::{pbr::NotShadowCaster, prelude::*};
use oz_sim::prelude::*;

const MAX_PARTICLES: usize = 500;

/// A bullet hole left in a wall (the oldest are cleared past a cap).
#[derive(Component)]
pub struct BulletHole;

/// A loose fragment for other modules (glass, splinters, rubble): flies, falls and shrinks away.
pub fn fragment(commands: &mut Commands, art: &Art, shape: Shape, paint: Paint, at: Vec3, size: Vec3, velocity: Vec3, life: f32) {
    commands.spawn((
        MatchEntity,
        Mesh3d(art.primitive(shape)),
        MeshMaterial3d(art.paint(paint)),
        Transform::from_translation(at).with_scale(size),
        Particle { velocity, gravity: 9.8, life, initial: life, size, grow: 0.0 },
        NotShadowCaster,
    ));
}

#[derive(Component)]
pub struct Particle {
    velocity: Vec3,
    gravity: f32,
    life: f32,
    initial: f32,
    size: Vec3,
    /// Grows instead of shrinking (explosion flashes).
    grow: f32,
}

/// Who last took the local player down, for the death screen and kill-cam.
#[derive(Resource, Default)]
pub struct DeathInfo {
    pub killer: Option<u8>,
}

/// Request to play a sound. Consumed by the audio module when it is enabled.
#[derive(Event, Clone, Copy)]
pub struct PlaySfx {
    pub sfx: Sfx,
    pub at: Option<Vec3>,
    pub volume: f32,
    /// Seconds to wait before playing (casings landing, a reload's second click).
    pub delay: f32,
}

impl PlaySfx {
    pub fn new(sfx: Sfx, at: Option<Vec3>) -> Self {
        Self { sfx, at, volume: 1.0, delay: 0.0 }
    }

    /// A non-positional interface sound.
    pub fn ui(sfx: Sfx) -> Self {
        Self::new(sfx, None)
    }

    pub fn later(self, delay: f32) -> Self {
        Self { delay, ..self }
    }

    pub fn quiet(self, volume: f32) -> Self {
        Self { volume, ..self }
    }
}

pub struct FxPlugin;

impl Plugin for FxPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<PlaySfx>()
            .init_resource::<DeathInfo>()
            .add_systems(Update, (spawn_effects, smoulder, tick_particles, night_calls, dress_smoke, swell_smoke, churn_puffs).chain().in_set(ClientSet::Present).run_if(in_state(MatchPhase::Active)));
    }
}

struct Spawner<'a, 'w, 's> {
    commands: &'a mut Commands<'w, 's>,
    art: &'a Art,
    rng: &'a mut Rng,
    budget: usize,
}

impl Spawner<'_, '_, '_> {
    fn particle(&mut self, shape: Shape, paint: Paint, transform: Transform, particle: Particle) {
        if self.budget == 0 {
            return;
        }
        self.budget -= 1;
        self.commands.spawn((MatchEntity, Mesh3d(self.art.primitive(shape)), MeshMaterial3d(self.art.paint(paint)), transform, particle, NotShadowCaster));
    }

    fn burst(&mut self, at: Vec3, count: usize, paints: &[Paint], speed: f32, size: f32, life: f32) {
        for i in 0..count {
            let dir = Vec3::new(self.rng.range(-1.0, 1.0), self.rng.range(-0.3, 1.0), self.rng.range(-1.0, 1.0)).normalize_or_zero();
            let size = Vec3::splat(self.rng.range(0.4, 1.0) * size);
            let life = self.rng.range(0.4, 1.0) * life;
            let velocity = dir * self.rng.range(0.3, 1.0) * speed;
            self.particle(Shape::Cube, paints[i % paints.len()], Transform::from_translation(at).with_scale(size), Particle { velocity, gravity: 9.0, life, initial: life, size, grow: 0.0 });
        }
    }

    fn streak(&mut self, from: Vec3, to: Vec3, width: f32, paint: Paint, life: f32) {
        let span = to - from;
        let length = span.length();
        if length < 0.01 {
            return;
        }
        let size = Vec3::new(width, width, length);
        let transform = Transform::from_translation(from + span * 0.5).with_rotation(Quat::from_rotation_arc(Vec3::Z, span / length)).with_scale(size);
        self.particle(Shape::Cube, paint, transform, Particle { velocity: Vec3::ZERO, gravity: 0.0, life, initial: life, size, grow: 0.0 });
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_effects(
    mut events: EventReader<FxEvent>,
    mut commands: Commands,
    art: Res<Art>,
    content: Res<Content>,
    mut view: ResMut<ViewState>,
    mut notices: ResMut<Notices>,
    mut sounds: EventWriter<PlaySfx>,
    local: Query<&Player, With<LocalPlayer>>,
    particles: Query<(), With<Particle>>,
    cameras: Query<&Transform, With<WorldCamera>>,
    (role, mut death, statics, bodies, map): (Res<NetRole>, ResMut<DeathInfo>, Res<Statics>, Query<(&Player, &Transform)>, Res<Map>),
    (rigs, joints, holes): (Query<(&crate::avatar::Rig, &Player)>, Query<&GlobalTransform>, Query<Entity, With<BulletHole>>),
    mut rng: Local<Option<Rng>>,
) {
    let me = local.single().map_or(255, |player| player.id);
    let camera = cameras.single().ok().copied().unwrap_or_default();
    // Shake falls off with distance from the blast.
    let rumble = |at: Vec3, power: f32| (power / (1.0 + camera.translation.distance(at) * 0.12)).min(1.6);
    let rng = rng.get_or_insert_with(|| Rng::new(0x5eed));
    let mut fx = Spawner { commands: &mut commands, art: &art, rng, budget: MAX_PARTICLES.saturating_sub(particles.iter().len()) };
    for event in events.read() {
        match event {
            FxEvent::Shot { by, local, .. } if *role == NetRole::Client && *by == me && !*local => {
                // Our own shot, already shown when we predicted it.
            }
            FxEvent::Killed { victim, by, .. } => {
                if *victim == me {
                    death.killer = Some(*by);
                }
                // The robot comes apart: sparks, plates and a puff of smoke.
                if let Some((_, body)) = bodies.iter().find(|(p, _)| p.id == *victim) {
                    let chest = body.translation + Vec3::Y * 1.1;
                    sounds.write(PlaySfx::new(Sfx::LimbOff, Some(chest)).quiet(0.6));
                    sounds.write(PlaySfx::new(Sfx::Debris, Some(body.translation)).quiet(0.7).later(0.35));
                    fx.burst(chest, 14, &[Paint::Spark, Paint::Glow, Paint::Steel, Paint::Metal, Paint::Dark], 6.5, 0.12, 0.9);
                    let size = Vec3::splat(0.25);
                    fx.particle(
                        Shape::Shard,
                        Paint::Spark,
                        Transform::from_translation(chest).with_scale(size),
                        Particle { velocity: Vec3::ZERO, gravity: 0.0, life: 0.12, initial: 0.12, size, grow: 3.0 },
                    );
                    for i in 0..3 {
                        let puff = Vec3::splat(0.35);
                        let drift = Vec3::new(fx.rng.range(-0.4, 0.4), 1.2, fx.rng.range(-0.4, 0.4));
                        fx.particle(
                            Shape::Shard,
                            Paint::Dark,
                            Transform::from_translation(chest + Vec3::Y * 0.2 * i as f32).with_scale(puff),
                            Particle { velocity: drift, gravity: 0.0, life: 1.1, initial: 1.1, size: puff, grow: 1.4 },
                        );
                    }
                }
            }
            FxEvent::Shot { by, sound, from, to, beam, tracer, quiet, .. } => {
                // Tracers leave the real muzzle: our viewmodel's, or the shooter robot's gun.
                let muzzle = if *by == me && view.muzzle != Vec3::ZERO {
                    Some(camera.translation + camera.rotation * view.muzzle)
                } else {
                    rigs.iter().find(|(_, p)| p.id == *by).and_then(|(rig, _)| joints.get(rig.gun).ok()).map(|g| g.translation() + g.forward().as_vec3() * 0.55)
                };
                let from = &muzzle.unwrap_or(*from);
                if *by != me && !*quiet && !*beam {
                    // Their muzzle flash, big enough to spot a shooter by.
                    let size = Vec3::new(0.18, 0.18, 0.35);
                    let flare = Transform::from_translation(*from).looking_to(*to - *from, Vec3::Y).with_scale(size);
                    fx.particle(Shape::Shard, Paint::Spark, flare, Particle { velocity: Vec3::ZERO, gravity: 0.0, life: 0.05, initial: 0.05, size, grow: 1.5 });
                }
                if *by == me && !*quiet && !*beam {
                    // A wisp of smoke off the barrel.
                    let puff = Vec3::splat(0.012);
                    let drift = Vec3::new(fx.rng.range(-0.05, 0.05), 0.18, fx.rng.range(-0.05, 0.05));
                    fx.particle(
                        Shape::Shard,
                        Paint::Iron,
                        Transform::from_translation(*from).with_scale(puff),
                        Particle { velocity: drift, gravity: 0.0, life: 0.5, initial: 0.5, size: puff, grow: 2.0 },
                    );
                }
                if *beam {
                    fx.streak(*from, *to, 0.035, Paint::Aether, 0.11);
                    fx.burst(*to, 1, &[Paint::Aether, Paint::Spark], 2.0, 0.06, 0.25);
                } else {
                    // Our own tracers start a little ahead so they do not fill the screen.
                    let start = if *by == me { *from + (*to - *from).normalize_or_zero() * 0.6 } else { *from };
                    // Suppressed shots leave faint tracers; special ammo shows its colour.
                    let width = if *quiet { 0.012 } else { 0.022 };
                    fx.streak(start, *to, width, if *by == 255 { Paint::Blue } else { *tracer }, 0.055);
                    // The impact: dust off the ground, chips and sparks off walls, sparks off robots,
                    // and a hole that stays in the wall.
                    let robot = bodies.iter().any(|(_, t)| (t.translation + Vec3::Y).distance(*to) < 0.9);
                    let ground = to.y - map.height(to.x, to.z) < 0.2;
                    let dir = (*to - *from).normalize_or(Vec3::NEG_Z);
                    // What the round actually stopped on: the ground, or a wall / floor / prop face.
                    // Rounds that ran out of range (or hit something that has since gone) stop in the
                    // air, and must not leave a mark there.
                    let surface = if ground {
                        Some(Vec3::Y)
                    } else {
                        statics.raycast(*to - dir * 0.3, dir, 0.6).and_then(|(d, index)| statics.aabb(index).filter(|_| (d - 0.3).abs() < 0.12)).map(|b| {
                            let local = (*to - b.center) / b.half.max(Vec3::splat(1e-3));
                            let a = local.abs();
                            if a.x >= a.y && a.x >= a.z {
                                Vec3::X * local.x.signum()
                            } else if a.y >= a.z {
                                Vec3::Y * local.y.signum()
                            } else {
                                Vec3::Z * local.z.signum()
                            }
                        })
                    };
                    if robot {
                        fx.burst(*to, 8, &[Paint::Spark, Paint::Glow, Paint::Steel], 6.0, 0.06, 0.35);
                    } else if let Some(normal) = surface {
                        let dust = if ground { Paint::Sand } else { Paint::Bone };
                        fx.burst(*to, 5, &[Paint::Spark, dust, Paint::Dark], 4.0, 0.07, 0.4);
                        for _ in 0..2 {
                            let puff = Vec3::splat(fx.rng.range(0.1, 0.18));
                            let drift = -dir * fx.rng.range(0.4, 1.0) + Vec3::Y * fx.rng.range(0.2, 0.6);
                            fx.particle(
                                Shape::Shard,
                                dust,
                                Transform::from_translation(*to - dir * 0.05).with_scale(puff),
                                Particle { velocity: drift, gravity: 0.0, life: 0.6, initial: 0.6, size: puff, grow: 1.8 },
                            );
                        }
                        // Bullet hole: a dark chip in the surface, facing out of it.
                        let count = holes.iter().len();
                        if count > 500
                            && let Some(oldest) = holes.iter().next()
                        {
                            fx.commands.entity(oldest).despawn();
                        }
                        let size = Vec3::new(0.07, 0.07, 0.01) * fx.rng.range(0.8, 1.3);
                        let at = *to + normal * 0.012;
                        fx.commands.spawn((
                            MatchEntity,
                            BulletHole,
                            Mesh3d(fx.art.primitive(Shape::Cube)),
                            MeshMaterial3d(fx.art.paint(Paint::Dark)),
                            Transform::from_translation(at).looking_to(-normal, if normal.y.abs() > 0.9 { Vec3::X } else { Vec3::Y }).with_scale(size),
                            NotShadowCaster,
                        ));
                    }
                    // A hot flash where the round lands (not where it ran out of range).
                    let size = Vec3::splat(0.07);
                    if robot || surface.is_some() {
                        fx.particle(
                            Shape::Shard,
                            Paint::Spark,
                            Transform::from_translation(*to).with_scale(size),
                            Particle { velocity: Vec3::ZERO, gravity: 0.0, life: 0.07, initial: 0.07, size, grow: 2.5 },
                        );
                    }
                    if *by == me {
                        // Spent casing tumbling out of the ejection port.
                        let size = Vec3::new(0.02, 0.02, 0.05);
                        let port = camera.translation + camera.rotation * Vec3::new(0.24, -0.14, -0.45);
                        let velocity = camera.rotation * Vec3::new(fx.rng.range(1.2, 2.4), fx.rng.range(1.0, 2.0), fx.rng.range(-0.4, 0.4));
                        fx.particle(Shape::Cube, Paint::Gold, Transform::from_translation(port).with_scale(size), Particle { velocity, gravity: 9.8, life: 0.6, initial: 0.6, size, grow: 0.0 });
                    }
                }
                sounds.write(PlaySfx { volume: if *quiet { 0.3 } else { 1.0 }, ..PlaySfx::new(Sfx::Shot(*sound), (*by != me).then_some(*from)) });
                if !*beam {
                    // Where it lands: dirt, a wall, or a robot.
                    let ground = to.y - map.height(to.x, to.z) < 0.2;
                    let robot = bodies.iter().any(|(_, t)| (t.translation + Vec3::Y).distance(*to) < 0.9);
                    let dir = (*to - *from).normalize_or(Vec3::NEG_Z);
                    let solid = ground || statics.raycast(*to - dir * 0.3, dir, 0.6).is_some_and(|(d, _)| (d - 0.3).abs() < 0.12);
                    let (impact, volume) = if robot {
                        (Sfx::ImpactMetal, 0.7)
                    } else if ground {
                        (Sfx::ImpactDirt, 0.5)
                    } else {
                        (Sfx::ImpactConcrete, 0.55)
                    };
                    if robot || solid {
                        sounds.write(PlaySfx::new(impact, Some(*to)).quiet(volume));
                    }
                    if !robot && solid && !ground && fx.rng.chance(0.14) {
                        sounds.write(PlaySfx::new(Sfx::Ricochet, Some(*to)).quiet(0.45));
                    }
                    // Rounds that pass close by snap past your head.
                    if *by != me {
                        let span = *to - *from;
                        let length = span.length().max(1e-3);
                        let along = ((camera.translation - *from).dot(span) / length).clamp(0.0, length);
                        let nearest = *from + span / length * along;
                        if nearest.distance(camera.translation) < 2.5 && along > 2.0 && along < length - 0.5 {
                            sounds.write(PlaySfx::new(Sfx::Whizz, Some(nearest)));
                        }
                    }
                }
            }
            FxEvent::Explosion { at, radius } => {
                // A blinding core, a big rolling fireball, a column of black smoke and a ring of debris.
                let core = Vec3::splat(radius * 0.14);
                fx.particle(
                    Shape::Shard,
                    Paint::Spark,
                    Transform::from_translation(*at).with_scale(core),
                    Particle { velocity: Vec3::ZERO, gravity: 0.0, life: 0.14, initial: 0.14, size: core, grow: 3.0 },
                );
                let ball = Vec3::splat(radius * 0.2);
                fx.particle(
                    Shape::Shard,
                    Paint::Glow,
                    Transform::from_translation(*at + Vec3::Y * 0.4).with_scale(ball),
                    Particle { velocity: Vec3::Y * 2.0, gravity: 0.0, life: 0.45, initial: 0.45, size: ball, grow: 2.4 },
                );
                for _ in 0..6 {
                    // Lobes of flame rolling out from the centre.
                    let out = Vec3::new(fx.rng.range(-1.0, 1.0), fx.rng.range(0.2, 1.0), fx.rng.range(-1.0, 1.0)).normalize_or_zero();
                    let lobe = Vec3::splat(radius * fx.rng.range(0.08, 0.13));
                    let paint = if fx.rng.chance(0.5) { Paint::Glow } else { Paint::Spark };
                    fx.particle(
                        Shape::Shard,
                        paint,
                        Transform::from_translation(*at + out * radius * 0.15).with_scale(lobe),
                        Particle { velocity: out * radius * 1.1, gravity: -1.0, life: 0.4, initial: 0.4, size: lobe, grow: 2.0 },
                    );
                }
                for i in 0..12 {
                    let drift = Vec3::new(fx.rng.range(-1.2, 1.2), fx.rng.range(1.8, 3.6), fx.rng.range(-1.2, 1.2));
                    let puff = Vec3::splat(radius * fx.rng.range(0.07, 0.13));
                    let at = *at + drift * 0.35 + Vec3::Y * 0.25 * i as f32;
                    fx.particle(
                        Shape::Shard,
                        Paint::Dark,
                        Transform::from_translation(at).with_scale(puff),
                        Particle { velocity: drift, gravity: 0.0, life: 2.4, initial: 2.4, size: puff, grow: 1.8 },
                    );
                }
                fx.burst(*at, 30, &[Paint::Glow, Paint::Spark, Paint::Dark, Paint::Rust], radius * 1.6, 0.26, 1.3);
                // A scorch mark that stays: a dark blast circle on the ground, singed walls nearby.
                let ground = map.height(at.x, at.z).max(statics.support(at.x, at.z, at.y + 0.5));
                if at.y - ground < radius * 0.6 {
                    // Built from small patches, each kept only where the surface under it is
                    // the floor the blast hit: the mark stops at roof edges and ledges instead
                    // of hanging out over them.
                    let surface = |x: f32, z: f32| map.height(x, z).max(statics.support(x, z, ground + 0.3));
                    let reach = radius * 0.4;
                    let step = 0.45;
                    let cells = (reach / step).ceil() as i32;
                    for gz in -cells..=cells {
                        for gx in -cells..=cells {
                            let (dx, dz) = (gx as f32 * step + fx.rng.range(-0.08, 0.08), gz as f32 * step + fx.rng.range(-0.08, 0.08));
                            let from_centre = (dx * dx + dz * dz).sqrt();
                            if from_centre > reach * fx.rng.range(0.75, 1.0) {
                                continue;
                            }
                            let (x, z) = (at.x + dx, at.z + dz);
                            let half = step * 0.62;
                            let flat = [(0.0, 0.0), (half, 0.0), (-half, 0.0), (0.0, half), (0.0, -half)].iter().all(|(ox, oz)| (surface(x + ox, z + oz) - ground).abs() < 0.06);
                            if !flat {
                                continue;
                            }
                            let dark = from_centre < reach * 0.55;
                            let size = Vec3::new(step * 0.75, 0.01, step * 0.75) * fx.rng.range(0.85, 1.15);
                            fx.commands.spawn((
                                MatchEntity,
                                BulletHole,
                                Mesh3d(fx.art.primitive(Shape::Shard)),
                                MeshMaterial3d(fx.art.paint(if dark { Paint::Dark } else { Paint::Steel })),
                                Transform::from_translation(Vec3::new(x, ground + 0.015 + if dark { 0.002 } else { 0.0 }, z)).with_rotation(Quat::from_rotation_y(fx.rng.range(0.0, 6.28))).with_scale(size),
                                NotShadowCaster,
                            ));
                        }
                    }
                }
                view.shake += rumble(*at, radius * 0.6);
                sounds.write(PlaySfx::new(Sfx::Explosion, Some(*at)));
                // Bits of the street rain back down.
                sounds.write(PlaySfx::new(Sfx::Debris, Some(*at)).quiet(0.5).later(0.45));
            }
            FxEvent::Smoke { at, radius, seconds } => {
                // The host already made the cloud in its own simulation.
                if *role == NetRole::Client {
                    fx.commands.spawn((MatchEntity, SmokeCloud { radius: *radius, time: *seconds }, Transform::from_translation(*at)));
                }
                fx.burst(*at, 8, &[Paint::Iron, Paint::Bone], 3.0, 0.2, 0.6);
                sounds.write(PlaySfx::new(Sfx::SmokeHiss, Some(*at)).quiet(0.8));
            }
            FxEvent::Flash { at, radius, seconds } => {
                let size = Vec3::splat(0.4);
                fx.particle(Shape::Shard, Paint::White, Transform::from_translation(*at).with_scale(size), Particle { velocity: Vec3::ZERO, gravity: 0.0, life: 0.15, initial: 0.15, size, grow: 6.0 });
                sounds.write(PlaySfx::new(Sfx::Explosion, Some(*at)));
                // Blinded if we had a clear line to it; worse looking straight at it.
                let eye = camera.translation;
                let to = *at - eye;
                let distance = to.length().max(1e-3);
                let alive = local.single().is_ok();
                if alive && distance < *radius && statics.raycast(eye, to / distance, distance).is_none() {
                    let facing = (camera.forward().dot(to / distance) + 1.0) * 0.5;
                    let strength = (1.0 - distance / radius) * (0.3 + 0.7 * facing * facing);
                    view.blind = view.blind.max(seconds * strength + 0.3);
                    view.shake += 0.6;
                    // Ears ring; the deafness itself is in the mixer (see `audio`).
                    sounds.write(PlaySfx::new(Sfx::FlashRing, None).quiet((strength * 1.4).min(1.0)));
                }
            }
            FxEvent::Flyby { from, to } => {
                // The jet itself is a real, shootable entity now; this is its roar.
                let _ = (from, to);
                sounds.write(PlaySfx { volume: 0.9, ..PlaySfx::new(Sfx::Jet, None) });
                view.shake += 0.4;
            }
            FxEvent::EnemyDied { at, kind } => {
                let (gibs, big) = content.enemies.get(*kind as usize).map_or((Paint::Steel, 1.0), |def| (def.gibs, def.radius / 0.6));
                fx.burst(*at, (8.0 * big) as usize, &[gibs, Paint::Dark, Paint::Glow], 4.0 + big, 0.14 * big, 0.7);
                sounds.write(PlaySfx::new(Sfx::EnemyDeath, Some(*at)));
            }
            FxEvent::StructureLost { at } => {
                fx.burst(*at, 18, &[Paint::Metal, Paint::Dark, Paint::Glow], 6.0, 0.2, 0.9);
                sounds.write(PlaySfx::new(Sfx::Explosion, Some(*at)));
            }
            FxEvent::Arc { from, to } => fx.streak(*from, *to, 0.05, Paint::Blue, 0.09),
            FxEvent::Mined { at, ore } => {
                let paint = [Paint::Iron, Paint::Copper, Paint::Aether][(*ore as usize).min(2)];
                fx.burst(*at, 3, &[paint, Paint::Spark], 2.5, 0.09, 0.5);
                sounds.write(PlaySfx::new(Sfx::OreGet, Some(*at)));
            }
            FxEvent::HitMarker { player, kill, head } if *player == me => {
                view.hit = if *head { 0.22 } else { 0.14 };
                view.kill = *kill;
                view.head_hit = *head;
                sounds.write(PlaySfx::new(if *kill { Sfx::Kill } else { Sfx::Hit }, None));
                if *head && !*kill {
                    sounds.write(PlaySfx { volume: 0.5, ..PlaySfx::new(Sfx::Headshot, None) });
                }
            }
            FxEvent::LimbLost { player, by, .. } if *by == me || *player == me || bodies.iter().any(|(p, _)| p.id == *player) => {
                if let Some((_, body)) = bodies.iter().find(|(p, _)| p.id == *player) {
                    sounds.write(PlaySfx::new(Sfx::LimbOff, Some(body.translation + Vec3::Y)).quiet(0.9));
                }
                if *by != me {
                    continue;
                }
                // A limb came off: the marker flares and a crunchy tick plays.
                view.hit = 0.25;
                view.limb_hit = 0.35;
                sounds.write(PlaySfx::new(Sfx::Medal, None));
            }
            FxEvent::Hurt { player } if *player == me => {
                view.hurt = 1.0;
                view.shake += 0.7;
                sounds.write(PlaySfx::new(Sfx::Hurt, None));
            }
            FxEvent::Sound { sfx, at } => {
                sounds.write(PlaySfx::new(*sfx, *at));
            }
            FxEvent::Cue { to, sfx } if *to == me => {
                sounds.write(PlaySfx::new(*sfx, None));
            }
            FxEvent::Notice { to, text } if *to == 255 || *to == me => notices.push(text.clone()),
            _ => {}
        }
    }
}

/// Now and then something out in the dark calls to the others.
fn night_calls(time: Res<Time>, state: Res<MatchState>, enemies: Query<&Transform, With<Enemy>>, mut sounds: EventWriter<PlaySfx>, mut wait: Local<f32>, mut pick: Local<u32>) {
    *wait -= time.delta_secs();
    if *wait > 0.0 || state.phase != Phase::Night {
        return;
    }
    *pick = pick.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    *wait = 1.8 + (*pick >> 24) as f32 / 255.0 * 3.5;
    let count = enemies.iter().len();
    if let Some(caller) = enemies.iter().nth(*pick as usize % count.max(1)) {
        sounds.write(PlaySfx::new(Sfx::Growl, Some(caller.translation)));
    }
}

/// Gives a smoke cloud its puffs: a loose cluster of big grey shards.
fn dress_smoke(mut commands: Commands, art: Res<Art>, clouds: Query<(Entity, &SmokeCloud), Added<SmokeCloud>>) {
    for (entity, cloud) in &clouds {
        let mut rng = Rng::new(entity.to_bits());
        commands.entity(entity).insert((Visibility::Inherited, CloudAge::default())).with_children(|parent| {
            for i in 0..24 {
                let angle = i as f32 * 2.39996;
                let reach = cloud.radius * 0.7 * ((i as f32 + 0.5) / 24.0).sqrt();
                // A low dome: wide at the bottom, puffs piled up in the middle.
                let height = (1.0 - reach / cloud.radius).max(0.1) * cloud.radius * rng.range(0.3, 0.8) - cloud.radius * 0.2;
                let at = Vec3::new(angle.cos() * reach, height, angle.sin() * reach);
                let size = Vec3::new(rng.range(0.4, 0.6), rng.range(0.32, 0.5), rng.range(0.4, 0.6)) * cloud.radius;
                let paint = [Paint::Iron, Paint::Metal, Paint::Rock, Paint::Iron, Paint::Bone][i % 5];
                parent.spawn((
                    Mesh3d(art.primitive(Shape::Shard)),
                    MeshMaterial3d(art.paint(paint)),
                    Transform::from_translation(at).with_scale(size).with_rotation(Quat::from_euler(EulerRot::YXZ, angle, rng.range(-0.4, 0.4), rng.range(-0.4, 0.4))),
                    NotShadowCaster,
                    Puff(rng.range(-0.5, 0.5)),
                ));
            }
        });
    }
}

/// Clouds roll out quickly, churn slowly, and shrink away at the end.
fn swell_smoke(time: Res<Time>, mut clouds: Query<(&mut Transform, &SmokeCloud, &mut CloudAge)>) {
    let dt = time.delta_secs();
    for (mut transform, cloud, mut age) in &mut clouds {
        age.0 += dt;
        let grow = (age.0 / 0.8).min(1.0);
        let scale = (grow * (2.0 - grow)).max(0.05) * smoke_scale(cloud.time).max(0.05);
        transform.scale = Vec3::splat(scale);
        transform.rotate_y(dt * 0.08);
    }
}

#[derive(Component, Default)]
struct CloudAge(f32);

/// One lump of a smoke cloud, turning slowly at its own rate.
#[derive(Component)]
struct Puff(f32);

fn churn_puffs(time: Res<Time>, mut puffs: Query<(&mut Transform, &Puff)>) {
    let dt = time.delta_secs();
    for (mut transform, puff) in &mut puffs {
        transform.rotate_y(puff.0 * dt);
    }
}

fn tick_particles(time: Res<Time>, mut commands: Commands, mut particles: Query<(Entity, &mut Transform, &mut Particle)>) {
    let dt = time.delta_secs().min(0.05);
    for (entity, mut transform, mut particle) in &mut particles {
        particle.life -= dt;
        if particle.life <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        let age = 1.0 - particle.life / particle.initial;
        if particle.grow > 0.0 {
            transform.scale = particle.size * (1.0 + particle.grow * age) * (1.0 - age * age * 0.6);
            transform.translation += particle.velocity * dt;
        } else if particle.gravity > 0.0 {
            particle.velocity.y -= particle.gravity * dt;
            transform.translation += particle.velocity * dt;
            transform.rotate_local_x(dt * 7.0);
            transform.scale = particle.size * (1.0 - age).sqrt();
        } else if particle.velocity != Vec3::ZERO {
            // Gliding pieces (jets): keep their size and heading.
            transform.translation += particle.velocity * dt;
        }
    }
}

/// Shot-up cars and barrels pour smoke; burning ones (about to blow) flame too.
fn smoulder(time: Res<Time>, map: Res<Map>, state: Res<MatchState>, art: Res<Art>, mut commands: Commands, particles: Query<(), With<Particle>>, mut wait: Local<f32>, mut seed: Local<u32>) {
    *wait -= time.delta_secs();
    if *wait > 0.0 {
        return;
    }
    *wait = 0.09;
    let mut budget = 700usize.saturating_sub(particles.iter().len()).min(40);
    let mut random = || {
        *seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (*seed >> 8) as f32 / 16_777_216.0
    };
    for (index, spec) in map.clutter.iter().enumerate() {
        let Some(&hp) = state.clutter.get(index) else {
            continue;
        };
        if hp == 0 || hp > 90 || spec.kind.blast().is_none() || budget == 0 {
            continue;
        }
        let burning = hp == oz_sim::breakables::BURNING;
        // Light damage smokes now and then; a burning wreck smokes thick.
        if !burning && random() > 0.35 {
            continue;
        }
        let top = spec.center() + Vec3::Y * spec.half().y;
        let jitter = Vec3::new(random() - 0.5, 0.0, random() - 0.5) * spec.half().x.max(0.4);
        let size = Vec3::splat(if burning { 0.45 } else { 0.25 } + random() * 0.2);
        let paint = if burning || random() < 0.5 { Paint::Dark } else { Paint::Iron };
        let drift = Vec3::new((random() - 0.5) * 0.6, 1.3 + random(), (random() - 0.5) * 0.6);
        let life = if burning { 2.2 } else { 1.6 };
        commands.spawn((
            MatchEntity,
            Mesh3d(art.primitive(Shape::Shard)),
            MeshMaterial3d(art.paint(paint)),
            Transform::from_translation(top + jitter).with_scale(size),
            Particle { velocity: drift, gravity: 0.0, life, initial: life, size, grow: 1.9 },
            NotShadowCaster,
        ));
        budget -= 1;
        if burning && budget > 0 {
            let flame = Vec3::splat(0.22 + random() * 0.25);
            let paint = if random() < 0.5 { Paint::Glow } else { Paint::Spark };
            commands.spawn((
                MatchEntity,
                Mesh3d(art.primitive(Shape::Shard)),
                MeshMaterial3d(art.paint(paint)),
                Transform::from_translation(top + jitter * 0.8).with_scale(flame),
                Particle { velocity: Vec3::Y * 2.2, gravity: 0.0, life: 0.35, initial: 0.35, size: flame, grow: 0.0 },
                NotShadowCaster,
            ));
            budget -= 1;
        }
    }
}
