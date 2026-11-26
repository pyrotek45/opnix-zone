//! One-shot effects: tracers, beams, sparks, gibs and explosions.
//!
//! All particles are unit primitives sharing the palette materials, so even a
//! full burst batches into a few draw calls. A hard cap keeps worst-case
//! fights cheap.

use crate::{ClientSet, art::Art, hud::Notices, view::{ViewState, WorldCamera}};
use bevy::{pbr::NotShadowCaster, prelude::*};
use oz_sim::prelude::*;

const MAX_PARTICLES: usize = 500;

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

/// Request to play a sound. Consumed by the audio module when it is enabled.
#[derive(Event, Clone, Copy)]
pub struct PlaySfx {
    pub sfx: Sfx,
    pub at: Option<Vec3>,
    pub volume: f32,
}

impl PlaySfx {
    pub fn new(sfx: Sfx, at: Option<Vec3>) -> Self {
        Self { sfx, at, volume: 1.0 }
    }

    /// A non-positional interface sound.
    pub fn ui(sfx: Sfx) -> Self {
        Self::new(sfx, None)
    }
}

pub struct FxPlugin;

impl Plugin for FxPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<PlaySfx>().add_systems(Update, (spawn_effects, tick_particles, night_calls).chain().in_set(ClientSet::Present).run_if(in_state(MatchPhase::Active)));
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
            FxEvent::Shot { by, sound, from, to, beam, tracer, quiet } => {
                if *beam {
                    fx.streak(*from, *to, 0.035, Paint::Aether, 0.11);
                    fx.burst(*to, 1, &[Paint::Aether, Paint::Spark], 2.0, 0.06, 0.25);
                } else {
                    // Our own tracers start a little ahead so they do not fill the screen.
                    let start = if *by == me { *from + (*to - *from).normalize_or_zero() * 0.6 } else { *from };
                    // Suppressed shots leave faint tracers; special ammo shows its colour.
                    let width = if *quiet { 0.012 } else { 0.022 };
                    fx.streak(start, *to, width, if *by == 255 { Paint::Blue } else { *tracer }, 0.055);
                    fx.burst(*to, 3, &[Paint::Spark, Paint::Dark], 3.5, 0.07, 0.3);
                    // A hot flash where the round lands.
                    let size = Vec3::splat(0.07);
                    fx.particle(Shape::Shard, Paint::Spark, Transform::from_translation(*to).with_scale(size), Particle { velocity: Vec3::ZERO, gravity: 0.0, life: 0.07, initial: 0.07, size, grow: 2.5 });
                    if *by == me {
                        // Spent casing tumbling out of the ejection port.
                        let size = Vec3::new(0.02, 0.02, 0.05);
                        let port = camera.translation + camera.rotation * Vec3::new(0.24, -0.14, -0.45);
                        let velocity = camera.rotation * Vec3::new(fx.rng.range(1.2, 2.4), fx.rng.range(1.0, 2.0), fx.rng.range(-0.4, 0.4));
                        fx.particle(Shape::Cube, Paint::Gold, Transform::from_translation(port).with_scale(size), Particle { velocity, gravity: 9.8, life: 0.6, initial: 0.6, size, grow: 0.0 });
                    }
                }
                sounds.write(PlaySfx { volume: if *quiet { 0.3 } else { 1.0 }, ..PlaySfx::new(Sfx::Shot(*sound), (*by != me).then_some(*from)) });
            }
            FxEvent::Explosion { at, radius } => {
                // A short hot core, an orange fireball, then dark smoke and debris.
                let core = Vec3::splat(radius * 0.08);
                fx.particle(Shape::Shard, Paint::Spark, Transform::from_translation(*at).with_scale(core), Particle { velocity: Vec3::ZERO, gravity: 0.0, life: 0.1, initial: 0.1, size: core, grow: 2.5 });
                let ball = Vec3::splat(radius * 0.11);
                fx.particle(Shape::Shard, Paint::Glow, Transform::from_translation(*at + Vec3::Y * 0.3).with_scale(ball), Particle { velocity: Vec3::Y * 1.5, gravity: 0.0, life: 0.3, initial: 0.3, size: ball, grow: 2.2 });
                for i in 0..5 {
                    let drift = Vec3::new(fx.rng.range(-1.0, 1.0), fx.rng.range(1.5, 3.0), fx.rng.range(-1.0, 1.0));
                    let puff = Vec3::splat(radius * fx.rng.range(0.05, 0.09));
                    let at = *at + drift * 0.3 + Vec3::Y * 0.2 * i as f32;
                    fx.particle(Shape::Shard, Paint::Dark, Transform::from_translation(at).with_scale(puff), Particle { velocity: drift, gravity: 0.0, life: 0.9, initial: 0.9, size: puff, grow: 1.6 });
                }
                fx.burst(*at, 22, &[Paint::Glow, Paint::Spark, Paint::Dark, Paint::Rust], 9.0, 0.22, 0.9);
                view.shake += rumble(*at, radius * 0.45);
                sounds.write(PlaySfx::new(Sfx::Explosion, Some(*at)));
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
            FxEvent::HitMarker { player, kill } if *player == me => {
                view.hit = 0.14;
                view.kill = *kill;
                sounds.write(PlaySfx::new(if *kill { Sfx::Kill } else { Sfx::Hit }, None));
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
        }
    }
}
