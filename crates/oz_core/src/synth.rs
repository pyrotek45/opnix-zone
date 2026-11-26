//! Procedural sound effects. No audio files ship with the game.
//!
//! [`render`] returns mono samples in `-1..1` at [`SAMPLE_RATE`]; [`wav`] wraps
//! them in a WAV container so any audio backend can decode them.

use crate::rng::Rng;
use std::f32::consts::TAU;

pub const SAMPLE_RATE: u32 = 22_050;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Sfx {
    /// Weapon report; the index comes from `WeaponDef::sound`.
    Shot(u8),
    Reload,
    DryFire,
    Hit,
    Kill,
    Explosion,
    EnemyShot,
    EnemyDeath,
    Hurt,
    UiMove,
    UiAccept,
    UiDeny,
    Buy,
    Build,
    Mine,
    OreGet,
    Horn,
    Dawn,
    Alarm,
    Step,
    /// Heavy landing after a jump or fall.
    Land,
    /// Weapon swap.
    Swap,
    /// A distant machine call in the dark.
    Growl,
    /// Headshot kill: a bright metallic ding.
    Headshot,
    /// Killstreak earned: a rising fanfare.
    Streak,
    /// Medal popup (double kill, revenge...): a short chime.
    Medal,
    /// A jet screaming overhead (airstrike).
    Jet,
    /// Low health.
    Heartbeat,
    /// Recon drone / UAV online.
    Radar,
    // Movement.
    /// Crouched or careful steps.
    StepSoft,
    /// Sprinting steps.
    StepHeavy,
    /// Steps on wooden floors indoors.
    StepWood,
    /// Crawling while prone.
    Crawl,
    Jump,
    /// Gear and cloth rustle (aiming, changing stance).
    Cloth,
    // Weapon handling.
    MagOut,
    MagIn,
    /// Bolt / charging handle racked.
    Bolt,
    /// A spent casing tinkling on the ground.
    Shell,
    // Bullets arriving.
    /// A round passing close by.
    Whizz,
    ImpactDirt,
    ImpactConcrete,
    /// A round hitting a robot.
    ImpactMetal,
    Ricochet,
    // Gear.
    PinPull,
    Throw,
    SmokeHiss,
    /// Ears ringing after a flashbang.
    FlashRing,
    Shatter,
    /// Metal tearing as a limb comes off.
    LimbOff,
    /// Parts clattering down.
    Debris,
    Thunder,
    DoorOpen,
    DoorBreak,
    /// Loops.
    Surf,
    NightDrone,
    Rain,
    Wind,
    Birds,
    /// A rocket's motor (loop).
    RocketFly,
    /// A killstreak drone's rotors (loop).
    DroneBuzz,
}

/// Where a sound is heard, which changes how it sounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Space {
    /// Outdoors, nearby: the plain sound with a slap-back echo off the town.
    Open,
    /// Indoors: a short, boxy reverb.
    Room,
    /// Far away: dull, softer, with a long rolling tail.
    Far,
}

impl Sfx {
    pub fn looping(self) -> bool {
        matches!(self, Sfx::Surf | Sfx::NightDrone | Sfx::Rain | Sfx::Wind | Sfx::Birds | Sfx::RocketFly | Sfx::DroneBuzz)
    }

    /// Every sound effect.
    pub fn all() -> Vec<Sfx> {
        let mut all: Vec<Sfx> = (0..9).map(Sfx::Shot).collect();
        all.extend([
            Sfx::Reload, Sfx::DryFire, Sfx::Hit, Sfx::Kill, Sfx::Explosion, Sfx::EnemyShot, Sfx::EnemyDeath, Sfx::Hurt, Sfx::UiMove, Sfx::UiAccept,
            Sfx::UiDeny, Sfx::Buy, Sfx::Build, Sfx::Mine, Sfx::OreGet, Sfx::Horn, Sfx::Dawn, Sfx::Alarm, Sfx::Step, Sfx::Land, Sfx::Swap, Sfx::Growl,
            Sfx::Headshot, Sfx::Streak, Sfx::Medal, Sfx::Jet, Sfx::Heartbeat, Sfx::Radar, Sfx::StepSoft, Sfx::StepHeavy, Sfx::StepWood, Sfx::Crawl,
            Sfx::Jump, Sfx::Cloth, Sfx::MagOut, Sfx::MagIn, Sfx::Bolt, Sfx::Shell, Sfx::Whizz, Sfx::ImpactDirt, Sfx::ImpactConcrete, Sfx::ImpactMetal,
            Sfx::Ricochet, Sfx::PinPull, Sfx::Throw, Sfx::SmokeHiss, Sfx::FlashRing, Sfx::Shatter, Sfx::LimbOff, Sfx::Debris, Sfx::Thunder,
            Sfx::DoorOpen, Sfx::DoorBreak, Sfx::Surf, Sfx::NightDrone, Sfx::Rain, Sfx::Wind, Sfx::Birds, Sfx::RocketFly, Sfx::DroneBuzz,
        ]);
        all
    }

    /// How many differently seeded takes to render, so repeats never sound identical.
    pub fn variants(self) -> u32 {
        match self {
            Sfx::Shot(_) | Sfx::Step | Sfx::StepSoft | Sfx::StepHeavy | Sfx::StepWood | Sfx::Crawl | Sfx::ImpactDirt | Sfx::ImpactConcrete | Sfx::ImpactMetal | Sfx::Whizz | Sfx::Shell => 3,
            Sfx::Explosion | Sfx::Ricochet | Sfx::Debris | Sfx::Thunder => 2,
            _ => 1,
        }
    }

    /// Does where it is heard change the sound (rooms, distance)?
    pub fn spatial_variants(self) -> bool {
        matches!(self, Sfx::Shot(_) | Sfx::Explosion | Sfx::Step | Sfx::StepHeavy | Sfx::StepWood | Sfx::EnemyShot | Sfx::Thunder | Sfx::DoorBreak | Sfx::Shatter)
    }
}

/// A tiny additive voice: pitch sweep + noise through a low-pass, with an exponential decay.
struct Voice {
    seconds: f32,
    freq: (f32, f32),
    tone: f32,
    square: bool,
    noise: f32,
    /// Low-pass coefficient for the noise, 0..1 (1 = bright).
    bright: f32,
    decay: f32,
    attack: f32,
    gain: f32,
}

impl Voice {
    const BASE: Voice = Voice { seconds: 0.2, freq: (440.0, 440.0), tone: 1.0, square: false, noise: 0.0, bright: 0.5, decay: 6.0, attack: 0.002, gain: 0.8 };

    fn mix_into(&self, out: &mut Vec<f32>, start: f32, seed: u64) {
        let mut rng = Rng::new(seed);
        let count = (self.seconds * SAMPLE_RATE as f32) as usize;
        let offset = (start * SAMPLE_RATE as f32) as usize;
        if out.len() < offset + count {
            out.resize(offset + count, 0.0);
        }
        let (mut phase, mut low) = (0.0f32, 0.0f32);
        for i in 0..count {
            let t = i as f32 / SAMPLE_RATE as f32;
            let k = t / self.seconds;
            let freq = self.freq.0 * (self.freq.1 / self.freq.0).powf(k);
            phase = (phase + freq / SAMPLE_RATE as f32).fract();
            let osc = if self.square { if phase < 0.5 { 0.6 } else { -0.6 } } else { (phase * TAU).sin() };
            low += (rng.range(-1.0, 1.0) - low) * self.bright;
            let envelope = (t / self.attack).min(1.0) * (-self.decay * k).exp() * (1.0 - k).min(1.0);
            out[offset + i] += (osc * self.tone + low * self.noise) * envelope * self.gain;
        }
    }
}

pub fn render(sfx: Sfx) -> Vec<f32> {
    render_take(sfx, 0)
}

/// One take of a sound in a given space.
pub fn render_in(sfx: Sfx, take: u32, space: Space) -> Vec<f32> {
    let mut out = render_take(sfx, take);
    if sfx.looping() {
        return out;
    }
    match space {
        Space::Open => {}
        Space::Room => {
            reverb(&mut out, 0.32, 0.55);
        }
        Space::Far => {
            // Distance eats the highs and stretches the tail.
            let mut low = 0.0f32;
            for sample in out.iter_mut() {
                low += (*sample - low) * 0.12;
                *sample = low * 1.6;
            }
            echo(&mut out, &[(0.3, 0.45), (0.75, 0.3), (1.3, 0.16)]);
        }
    }
    for sample in &mut out {
        *sample = sample.clamp(-1.0, 1.0);
    }
    out
}

/// A tiny Schroeder reverb: parallel combs into series all-passes.
fn reverb(out: &mut Vec<f32>, size: f32, wet: f32) {
    let extra = (size * SAMPLE_RATE as f32) as usize;
    out.resize(out.len() + extra, 0.0);
    let dry = out.clone();
    let mut wet_mix = vec![0.0f32; dry.len()];
    for (delay_ms, feedback) in [(29.7, 0.77), (37.1, 0.75), (41.1, 0.73), (43.7, 0.71)] {
        let delay = (delay_ms * 0.001 * SAMPLE_RATE as f32) as usize;
        let mut buffer = vec![0.0f32; dry.len()];
        let mut low = 0.0f32;
        for i in 0..dry.len() {
            let back = if i >= delay { buffer[i - delay] } else { 0.0 };
            low += (back - low) * 0.45;
            buffer[i] = dry[i] + low * feedback;
            wet_mix[i] += buffer[i] * 0.25;
        }
    }
    for delay_ms in [5.0, 1.7] {
        let delay = (delay_ms * 0.001 * SAMPLE_RATE as f32) as usize;
        let input = wet_mix.clone();
        for i in 0..input.len() {
            let back = if i >= delay { wet_mix[i - delay] } else { 0.0 };
            wet_mix[i] = -0.7 * input[i] + if i >= delay { input[i - delay] } else { 0.0 } + 0.7 * back;
        }
    }
    for (sample, w) in out.iter_mut().zip(wet_mix) {
        *sample = *sample * (1.0 - wet * 0.4) + w * wet;
    }
}

fn render_take(sfx: Sfx, take: u32) -> Vec<f32> {
    const B: Voice = Voice::BASE;
    let mut out = Vec::new();
    // Each take nudges the pitch and reseeds the noise.
    let pitch = 1.0 + (take as f32 - 1.0) * 0.045;
    let seed = sfx_seed(sfx) ^ (take as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    let mut add = |voice: Voice, start: f32| Voice { freq: (voice.freq.0 * pitch, voice.freq.1 * pitch), ..voice }.mix_into(&mut out, start, seed + (start * 1000.0) as u64);
    match sfx {
        Sfx::Shot(kind) => match kind {
            // Pistol, SMG, shotgun, carbine, lance, launcher, mining laser, turret.
            0 => { add(Voice { seconds: 0.16, freq: (620.0, 110.0), noise: 0.7, bright: 0.6, decay: 7.0, ..B }, 0.0); }
            1 => { add(Voice { seconds: 0.10, freq: (480.0, 140.0), noise: 0.8, bright: 0.75, decay: 8.0, gain: 0.6, ..B }, 0.0); }
            2 => {
                add(Voice { seconds: 0.34, freq: (210.0, 45.0), noise: 1.2, bright: 0.35, decay: 6.0, gain: 0.9, ..B }, 0.0);
                add(Voice { seconds: 0.12, freq: (900.0, 300.0), tone: 0.2, noise: 0.6, bright: 0.9, decay: 9.0, ..B }, 0.0);
            }
            3 => { add(Voice { seconds: 0.20, freq: (390.0, 70.0), noise: 0.9, bright: 0.5, decay: 7.0, ..B }, 0.0); }
            4 => {
                add(Voice { seconds: 0.45, freq: (1900.0, 160.0), tone: 0.7, square: true, noise: 0.25, decay: 5.0, gain: 0.55, ..B }, 0.0);
                add(Voice { seconds: 0.30, freq: (90.0, 40.0), decay: 5.0, ..B }, 0.0);
            }
            5 => { add(Voice { seconds: 0.50, freq: (150.0, 50.0), noise: 0.9, bright: 0.2, decay: 4.0, attack: 0.02, ..B }, 0.0); }
            8 => {
                // Sniper: a hard supersonic crack with a long rolling tail.
                add(Voice { seconds: 0.5, freq: (900.0, 60.0), tone: 0.5, noise: 1.3, bright: 0.7, decay: 9.0, gain: 1.0, ..B }, 0.0);
                add(Voice { seconds: 0.9, freq: (70.0, 30.0), noise: 0.5, bright: 0.08, decay: 3.0, gain: 0.7, ..B }, 0.0);
            }
            6 => { add(Voice { seconds: 0.09, freq: (1250.0, 1180.0), tone: 0.35, square: true, noise: 0.12, bright: 0.9, decay: 2.0, gain: 0.3, ..B }, 0.0); }
            _ => { add(Voice { seconds: 0.13, freq: (300.0, 90.0), tone: 0.8, square: true, noise: 0.5, decay: 8.0, gain: 0.5, ..B }, 0.0); }
        },
        Sfx::Reload => {
            add(Voice { seconds: 0.05, freq: (900.0, 500.0), tone: 0.3, noise: 0.6, bright: 0.8, decay: 9.0, gain: 0.5, ..B }, 0.0);
            add(Voice { seconds: 0.07, freq: (500.0, 800.0), tone: 0.3, noise: 0.6, bright: 0.7, decay: 9.0, gain: 0.5, ..B }, 0.22);
        }
        Sfx::DryFire => add(Voice { seconds: 0.04, freq: (1200.0, 800.0), tone: 0.2, noise: 0.5, bright: 0.9, decay: 10.0, gain: 0.4, ..B }, 0.0),
        Sfx::Hit => add(Voice { seconds: 0.06, freq: (1500.0, 900.0), tone: 0.6, square: true, decay: 8.0, gain: 0.35, ..B }, 0.0),
        Sfx::Kill => {
            add(Voice { seconds: 0.08, freq: (880.0, 880.0), square: true, decay: 5.0, gain: 0.3, ..B }, 0.0);
            add(Voice { seconds: 0.14, freq: (1320.0, 1320.0), square: true, decay: 5.0, gain: 0.3, ..B }, 0.07);
        }
        Sfx::Headshot => {
            add(Voice { seconds: 0.35, freq: (2350.0, 2300.0), tone: 0.6, decay: 7.0, gain: 0.35, ..B }, 0.0);
            add(Voice { seconds: 0.35, freq: (3525.0, 3500.0), tone: 0.3, decay: 8.0, gain: 0.25, ..B }, 0.0);
            add(Voice { seconds: 0.05, freq: (900.0, 600.0), tone: 0.2, noise: 0.6, bright: 0.9, decay: 10.0, gain: 0.3, ..B }, 0.0);
        }
        Sfx::Streak => {
            for (i, f) in [392.0, 523.0, 659.0, 784.0, 1046.0].into_iter().enumerate() {
                add(Voice { seconds: 0.22, freq: (f, f), square: true, decay: 3.0, gain: 0.22, ..B }, i as f32 * 0.07);
            }
            add(Voice { seconds: 0.7, freq: (1046.0, 1046.0), square: true, decay: 2.0, attack: 0.02, gain: 0.2, ..B }, 0.35);
            add(Voice { seconds: 0.7, freq: (523.0, 523.0), decay: 2.0, attack: 0.02, gain: 0.25, ..B }, 0.35);
        }
        Sfx::Medal => {
            add(Voice { seconds: 0.12, freq: (988.0, 988.0), square: true, decay: 4.0, gain: 0.22, ..B }, 0.0);
            add(Voice { seconds: 0.25, freq: (1480.0, 1480.0), square: true, decay: 4.0, gain: 0.22, ..B }, 0.08);
        }
        Sfx::Jet => {
            // Doppler: a roar that climbs, peaks overhead and falls away.
            add(Voice { seconds: 2.4, freq: (900.0, 220.0), tone: 0.15, noise: 1.0, bright: 0.35, decay: 0.6, attack: 0.9, gain: 0.7, ..B }, 0.0);
            add(Voice { seconds: 2.0, freq: (2400.0, 500.0), tone: 0.12, noise: 0.4, bright: 0.8, decay: 0.8, attack: 0.8, gain: 0.3, ..B }, 0.15);
        }
        Sfx::Heartbeat => {
            add(Voice { seconds: 0.14, freq: (70.0, 45.0), decay: 6.0, gain: 0.7, ..B }, 0.0);
            add(Voice { seconds: 0.16, freq: (62.0, 40.0), decay: 6.0, gain: 0.55, ..B }, 0.2);
        }
        Sfx::Radar => {
            for i in 0..3 {
                add(Voice { seconds: 0.18, freq: (1200.0, 1600.0), tone: 0.6, decay: 4.0, gain: 0.22, ..B }, i as f32 * 0.22);
            }
        }
        Sfx::Explosion => {
            add(Voice { seconds: 0.9, freq: (110.0, 28.0), noise: 1.3, bright: 0.12, decay: 4.5, gain: 1.0, ..B }, 0.0);
            add(Voice { seconds: 0.25, freq: (400.0, 100.0), tone: 0.1, noise: 0.9, bright: 0.8, decay: 8.0, ..B }, 0.0);
        }
        Sfx::EnemyShot => add(Voice { seconds: 0.22, freq: (260.0, 620.0), tone: 0.7, square: true, noise: 0.2, decay: 5.0, gain: 0.4, ..B }, 0.0),
        Sfx::EnemyDeath => {
            add(Voice { seconds: 0.35, freq: (520.0, 60.0), tone: 0.7, square: true, noise: 0.5, bright: 0.4, decay: 5.0, gain: 0.5, ..B }, 0.0);
        }
        Sfx::Hurt => add(Voice { seconds: 0.22, freq: (190.0, 80.0), noise: 0.6, bright: 0.3, decay: 5.0, ..B }, 0.0),
        Sfx::UiMove => add(Voice { seconds: 0.035, freq: (700.0, 700.0), square: true, decay: 4.0, gain: 0.18, ..B }, 0.0),
        Sfx::UiAccept => {
            add(Voice { seconds: 0.06, freq: (660.0, 660.0), square: true, decay: 4.0, gain: 0.22, ..B }, 0.0);
            add(Voice { seconds: 0.09, freq: (990.0, 990.0), square: true, decay: 4.0, gain: 0.22, ..B }, 0.05);
        }
        Sfx::UiDeny => add(Voice { seconds: 0.16, freq: (160.0, 120.0), square: true, decay: 3.0, gain: 0.25, ..B }, 0.0),
        Sfx::Buy => {
            for (i, f) in [523.0, 659.0, 784.0, 1047.0].into_iter().enumerate() {
                add(Voice { seconds: 0.12, freq: (f, f), square: true, decay: 4.0, gain: 0.2, ..B }, i as f32 * 0.055);
            }
        }
        Sfx::Build => {
            add(Voice { seconds: 0.12, freq: (240.0, 90.0), noise: 0.8, bright: 0.5, decay: 7.0, ..B }, 0.0);
            add(Voice { seconds: 0.25, freq: (330.0, 660.0), square: true, decay: 4.0, gain: 0.2, ..B }, 0.1);
        }
        Sfx::Mine => add(Voice { seconds: 0.08, freq: (320.0, 180.0), tone: 0.3, noise: 0.9, bright: 0.85, decay: 9.0, gain: 0.4, ..B }, 0.0),
        Sfx::OreGet => add(Voice { seconds: 0.09, freq: (1250.0, 1900.0), decay: 5.0, gain: 0.25, ..B }, 0.0),
        Sfx::Horn => {
            for f in [98.0, 147.0, 196.5] {
                add(Voice { seconds: 2.2, freq: (f, f * 0.97), square: true, tone: 0.5, decay: 1.6, attack: 0.25, gain: 0.3, ..B }, 0.0);
            }
        }
        Sfx::Dawn => {
            for (i, f) in [392.0, 494.0, 587.0, 784.0].into_iter().enumerate() {
                add(Voice { seconds: 0.9, freq: (f, f), decay: 3.0, attack: 0.03, gain: 0.25, ..B }, i as f32 * 0.16);
            }
        }
        Sfx::Alarm => {
            for i in 0..3 {
                add(Voice { seconds: 0.2, freq: (740.0, 520.0), square: true, decay: 2.0, gain: 0.3, ..B }, i as f32 * 0.26);
            }
        }
        Sfx::Step => add(Voice { seconds: 0.07, freq: (120.0, 70.0), tone: 0.3, noise: 0.8, bright: 0.25, decay: 8.0, gain: 0.3, ..B }, 0.0),
        Sfx::Land => add(Voice { seconds: 0.16, freq: (95.0, 42.0), tone: 0.9, noise: 0.7, bright: 0.18, decay: 6.0, gain: 0.55, ..B }, 0.0),
        Sfx::Swap => {
            add(Voice { seconds: 0.04, freq: (700.0, 420.0), tone: 0.25, noise: 0.7, bright: 0.85, decay: 9.0, gain: 0.4, ..B }, 0.0);
            add(Voice { seconds: 0.05, freq: (380.0, 560.0), tone: 0.25, noise: 0.6, bright: 0.7, decay: 9.0, gain: 0.4, ..B }, 0.09);
        }
        Sfx::Growl => {
            add(Voice { seconds: 0.9, freq: (150.0, 62.0), tone: 0.6, square: true, noise: 0.35, bright: 0.12, decay: 2.2, attack: 0.12, gain: 0.5 }, 0.0);
            add(Voice { seconds: 0.6, freq: (305.0, 118.0), tone: 0.3, square: true, decay: 3.0, attack: 0.2, gain: 0.3, ..B }, 0.1);
        }
        Sfx::StepSoft => add(Voice { seconds: 0.06, freq: (150.0, 90.0), tone: 0.3, noise: 0.7, bright: 0.25, decay: 9.0, gain: 0.32, ..B }, 0.0),
        Sfx::StepHeavy => {
            add(Voice { seconds: 0.09, freq: (110.0, 55.0), tone: 0.5, noise: 0.9, bright: 0.3, decay: 7.0, gain: 0.45, ..B }, 0.0);
            // A little gear rattle with every stride.
            add(Voice { seconds: 0.05, freq: (2400.0, 1800.0), tone: 0.1, noise: 0.35, bright: 0.9, decay: 9.0, gain: 0.12, ..B }, 0.02);
        }
        Sfx::StepWood => {
            add(Voice { seconds: 0.08, freq: (210.0, 120.0), tone: 0.55, noise: 0.5, bright: 0.35, decay: 8.0, gain: 0.35, ..B }, 0.0);
            add(Voice { seconds: 0.12, freq: (420.0, 380.0), tone: 0.2, decay: 6.0, gain: 0.08, ..B }, 0.01);
        }
        Sfx::Crawl => add(Voice { seconds: 0.25, freq: (300.0, 200.0), tone: 0.0, noise: 0.9, bright: 0.45, decay: 3.0, attack: 0.08, gain: 0.5, ..B }, 0.0),
        Sfx::Jump => {
            add(Voice { seconds: 0.08, freq: (140.0, 90.0), tone: 0.3, noise: 0.6, bright: 0.25, decay: 7.0, gain: 0.3, ..B }, 0.0);
            add(Voice { seconds: 0.12, freq: (900.0, 700.0), tone: 0.0, noise: 0.4, bright: 0.6, decay: 5.0, gain: 0.12, ..B }, 0.02);
        }
        Sfx::Cloth => add(Voice { seconds: 0.18, freq: (500.0, 500.0), tone: 0.0, noise: 0.9, bright: 0.55, decay: 4.0, attack: 0.04, gain: 0.4, ..B }, 0.0),
        Sfx::MagOut => {
            add(Voice { seconds: 0.03, freq: (1800.0, 1200.0), tone: 0.3, noise: 0.7, bright: 0.9, decay: 10.0, gain: 0.35, ..B }, 0.0);
            add(Voice { seconds: 0.12, freq: (600.0, 300.0), tone: 0.2, noise: 0.5, bright: 0.5, decay: 6.0, gain: 0.25, ..B }, 0.03);
        }
        Sfx::MagIn => {
            add(Voice { seconds: 0.05, freq: (700.0, 400.0), tone: 0.3, noise: 0.8, bright: 0.7, decay: 9.0, gain: 0.45, ..B }, 0.0);
            add(Voice { seconds: 0.03, freq: (2600.0, 2000.0), tone: 0.4, noise: 0.6, bright: 0.95, decay: 10.0, gain: 0.35, ..B }, 0.06);
        }
        Sfx::Bolt => {
            add(Voice { seconds: 0.06, freq: (1200.0, 700.0), tone: 0.4, noise: 0.7, bright: 0.85, decay: 8.0, gain: 0.4, ..B }, 0.0);
            add(Voice { seconds: 0.06, freq: (800.0, 1500.0), tone: 0.4, noise: 0.7, bright: 0.85, decay: 8.0, gain: 0.4, ..B }, 0.12);
        }
        Sfx::Shell => {
            for (i, f) in [3100.0, 2700.0, 3300.0].into_iter().enumerate() {
                add(Voice { seconds: 0.07, freq: (f, f * 0.98), tone: 0.6, noise: 0.1, bright: 0.9, decay: 7.0, gain: 0.12 / (i as f32 + 1.0), ..B }, i as f32 * 0.09);
            }
        }
        Sfx::Whizz => add(Voice { seconds: 0.16, freq: (2200.0, 900.0), tone: 0.25, noise: 0.8, bright: 0.75, decay: 2.5, attack: 0.05, gain: 0.5, ..B }, 0.0),
        Sfx::ImpactDirt => add(Voice { seconds: 0.08, freq: (300.0, 120.0), tone: 0.1, noise: 0.9, bright: 0.3, decay: 8.0, gain: 0.35, ..B }, 0.0),
        Sfx::ImpactConcrete => {
            add(Voice { seconds: 0.05, freq: (1800.0, 900.0), tone: 0.2, noise: 1.0, bright: 0.85, decay: 10.0, gain: 0.4, ..B }, 0.0);
            add(Voice { seconds: 0.1, freq: (500.0, 300.0), tone: 0.1, noise: 0.5, bright: 0.35, decay: 6.0, gain: 0.2, ..B }, 0.01);
        }
        Sfx::ImpactMetal => {
            for f in [1430.0, 2210.0, 3170.0] {
                add(Voice { seconds: 0.3, freq: (f, f * 0.99), tone: 0.6, noise: 0.05, decay: 6.0, gain: 0.14, ..B }, 0.0);
            }
            add(Voice { seconds: 0.03, freq: (900.0, 600.0), tone: 0.2, noise: 0.8, bright: 0.9, decay: 10.0, gain: 0.3, ..B }, 0.0);
        }
        Sfx::Ricochet => add(Voice { seconds: 0.45, freq: (3200.0, 1300.0), tone: 0.55, noise: 0.15, bright: 0.8, decay: 3.5, gain: 0.3, ..B }, 0.02),
        Sfx::PinPull => {
            add(Voice { seconds: 0.03, freq: (3000.0, 2500.0), tone: 0.5, noise: 0.4, bright: 0.95, decay: 10.0, gain: 0.3, ..B }, 0.0);
            add(Voice { seconds: 0.08, freq: (1500.0, 2600.0), tone: 0.5, noise: 0.2, decay: 6.0, gain: 0.18, ..B }, 0.05);
        }
        Sfx::Throw => add(Voice { seconds: 0.22, freq: (400.0, 900.0), tone: 0.0, noise: 0.8, bright: 0.4, decay: 3.0, attack: 0.08, gain: 0.3, ..B }, 0.0),
        Sfx::SmokeHiss => {
            add(Voice { seconds: 0.08, freq: (300.0, 200.0), tone: 0.2, noise: 0.6, bright: 0.4, decay: 7.0, gain: 0.4, ..B }, 0.0);
            add(Voice { seconds: 3.2, freq: (5000.0, 4000.0), tone: 0.0, noise: 0.55, bright: 0.85, decay: 1.2, attack: 0.15, gain: 0.35, ..B }, 0.05);
        }
        Sfx::FlashRing => {
            add(Voice { seconds: 3.5, freq: (3520.0, 3480.0), tone: 0.35, decay: 1.6, attack: 0.05, gain: 0.35, ..B }, 0.0);
            add(Voice { seconds: 3.0, freq: (2640.0, 2620.0), tone: 0.12, decay: 1.8, attack: 0.05, gain: 0.25, ..B }, 0.0);
        }
        Sfx::Shatter => {
            for i in 0..9 {
                let f = 2600.0 + (i * 977 % 2400) as f32;
                add(Voice { seconds: 0.18, freq: (f, f * 0.9), tone: 0.45, noise: 0.5, bright: 0.95, decay: 6.0, gain: 0.16, ..B }, i as f32 * 0.035);
            }
            add(Voice { seconds: 0.08, freq: (900.0, 500.0), tone: 0.1, noise: 0.9, bright: 0.8, decay: 8.0, gain: 0.35, ..B }, 0.0);
        }
        Sfx::LimbOff => {
            add(Voice { seconds: 0.25, freq: (600.0, 180.0), tone: 0.45, square: true, noise: 0.7, bright: 0.6, decay: 4.0, gain: 0.45, ..B }, 0.0);
            add(Voice { seconds: 0.35, freq: (1900.0, 1700.0), tone: 0.4, decay: 5.0, gain: 0.15, ..B }, 0.05);
            add(Voice { seconds: 0.06, freq: (4000.0, 3000.0), tone: 0.1, noise: 0.6, bright: 0.95, decay: 9.0, gain: 0.25, ..B }, 0.0);
        }
        Sfx::Debris => {
            for i in 0..6 {
                let f = 700.0 + (i * 431 % 900) as f32;
                add(Voice { seconds: 0.12, freq: (f, f * 0.8), tone: 0.4, noise: 0.6, bright: 0.6, decay: 7.0, gain: 0.18, ..B }, i as f32 * 0.11 + (i % 2) as f32 * 0.04);
            }
        }
        Sfx::Thunder => {
            add(Voice { seconds: 0.4, freq: (300.0, 80.0), tone: 0.0, noise: 1.2, bright: 0.5, decay: 4.0, gain: 0.5, ..B }, 0.0);
            add(Voice { seconds: 3.5, freq: (60.0, 30.0), tone: 0.2, noise: 1.2, bright: 0.04, decay: 1.4, attack: 0.2, gain: 0.9, ..B }, 0.1);
        }
        Sfx::DoorOpen => {
            add(Voice { seconds: 0.45, freq: (320.0, 520.0), tone: 0.35, square: true, noise: 0.25, bright: 0.3, decay: 2.5, attack: 0.05, gain: 0.18 }, 0.0);
            add(Voice { seconds: 0.1, freq: (160.0, 90.0), tone: 0.6, noise: 0.5, bright: 0.25, decay: 7.0, gain: 0.35, ..B }, 0.4);
        }
        Sfx::DoorBreak => {
            add(Voice { seconds: 0.35, freq: (180.0, 60.0), tone: 0.4, noise: 1.0, bright: 0.35, decay: 5.0, gain: 0.7, ..B }, 0.0);
            for i in 0..5 {
                add(Voice { seconds: 0.1, freq: (500.0 + i as f32 * 130.0, 300.0), tone: 0.4, noise: 0.6, bright: 0.5, decay: 8.0, gain: 0.2, ..B }, 0.15 + i as f32 * 0.09);
            }
        }
        Sfx::Surf | Sfx::NightDrone | Sfx::Rain | Sfx::Wind | Sfx::Birds | Sfx::RocketFly | Sfx::DroneBuzz => return render_loop(sfx),
    }
    // Gunshots get a chest thump and everything loud gets a slap-back echo, so
    // the island sounds like a place rather than a sample player.
    match sfx {
        Sfx::Shot(8) => echo(&mut out, &[(0.16, 0.35), (0.38, 0.2), (0.7, 0.1)]),
        Sfx::Shot(kind) if kind != 6 => {
            Voice { seconds: 0.14, freq: (150.0, 48.0), decay: 6.0, gain: 0.5, ..B }.mix_into(&mut out, 0.0, 1);
            echo(&mut out, &[(0.085, 0.24), (0.19, 0.11)]);
        }
        Sfx::Explosion | Sfx::Horn | Sfx::Growl | Sfx::EnemyDeath => echo(&mut out, &[(0.11, 0.3), (0.26, 0.16), (0.43, 0.07)]),
        _ => {}
    }
    for sample in &mut out {
        // Soft clip: loud layers saturate instead of crackling.
        *sample = (*sample * 1.2).tanh();
    }
    out
}

/// Adds delayed, quieter copies of the sound (`(seconds, gain)` taps).
fn echo(out: &mut Vec<f32>, taps: &[(f32, f32)]) {
    let dry = out.clone();
    for (delay, gain) in taps {
        let offset = (delay * SAMPLE_RATE as f32) as usize;
        if out.len() < dry.len() + offset {
            out.resize(dry.len() + offset, 0.0);
        }
        let mut low = 0.0f32;
        for (i, sample) in dry.iter().enumerate() {
            // Echoes come back duller than they left.
            low += (sample - low) * 0.35;
            out[offset + i] += low * gain;
        }
    }
}

fn sfx_seed(sfx: Sfx) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    sfx.hash(&mut hasher);
    hasher.finish()
}

/// Seamless ambience: everything is built from whole-number cycles over the loop length.
fn render_loop(sfx: Sfx) -> Vec<f32> {
    let seconds = 8.0;
    let count = (seconds * SAMPLE_RATE as f32) as usize;
    let mut rng = Rng::new(sfx_seed(sfx));
    let mut low = 0.0f32;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let k = i as f32 / count as f32;
        let raw = rng.range(-1.0, 1.0);
        let t = k * seconds;
        low += (raw - low) * match sfx {
            Sfx::Surf => 0.08,
            Sfx::Rain => 0.7,
            Sfx::Wind => 0.012,
            Sfx::RocketFly => 0.35,
            Sfx::DroneBuzz => 0.3,
            _ => 0.02,
        };
        let sample = match sfx {
            Sfx::Surf => {
                let swell = 0.55 + 0.3 * (k * TAU * 2.0).sin() + 0.15 * (k * TAU * 5.0).sin();
                low * swell * 1.6
            }
            // Hiss of rain, with fat drops popping out of it.
            Sfx::Rain => {
                let drop = if rng.chance(0.0009) { rng.range(0.3, 0.8) } else { 0.0 };
                (raw - low * 0.6) * 0.18 + low * 0.25 + drop
            }
            // Gusts: slow swells of low rumble and a whistle that rides them.
            Sfx::Wind => {
                let gust = 0.45 + 0.35 * (k * TAU * 2.0).sin() + 0.2 * (k * TAU * 5.0 + 1.3).sin();
                low * 6.0 * gust + (t * TAU * 420.0 * (1.0 + 0.04 * (k * TAU * 2.0).sin())).sin() * 0.03 * gust
            }
            // Birdsong: short warbling chirps here and there over a quiet bed.
            Sfx::Birds => {
                let mut chirp = 0.0;
                for (start, f) in [(0.4, 3400.0), (0.55, 3900.0), (2.1, 2800.0), (2.3, 3100.0), (2.45, 3500.0), (4.6, 4200.0), (5.9, 3000.0), (6.05, 3600.0)] {
                    let local = t - start;
                    if (0.0..0.09).contains(&local) {
                        let wobble = 1.0 + 0.12 * (local * TAU * 40.0).sin();
                        chirp += (local * TAU * f * wobble).sin() * (local / 0.09 * std::f32::consts::PI).sin() * 0.35;
                    }
                }
                chirp + low * 0.15
            }
            // A buzzing motor whose pitch wavers a little.
            Sfx::RocketFly => low * 1.4 + (t * TAU * 90.0).sin() * 0.25,
            Sfx::DroneBuzz => {
                let rotor = (t * TAU * 120.0).sin().signum() * 0.18 + (t * TAU * 241.0).sin() * 0.12;
                rotor * (0.8 + 0.2 * (t * TAU * 6.0).sin()) + low * 0.4
            }
            _ => {
                let hum = (t * TAU * 55.0).sin() * 0.22 + (t * TAU * 82.5).sin() * 0.12 + (t * TAU * 110.25).sin() * 0.06;
                hum * (0.7 + 0.3 * (k * TAU * 3.0).sin()) + low * 0.8
            }
        };
        out.push(sample.clamp(-1.0, 1.0));
    }
    // Cross-fade the tail into the head so the noise bed loops without a click.
    let fade = SAMPLE_RATE as usize / 4;
    for i in 0..fade {
        let k = i as f32 / fade as f32;
        out[i] = out[i] * k + out[count - fade + i] * (1.0 - k);
    }
    out.truncate(count - fade);
    out
}

/// 16-bit mono WAV file bytes.
pub fn wav(samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + data_len as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    bytes.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&((sample.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sound_renders_audibly() {
        let all = Sfx::all();
        let mut quiet = Vec::new();
        for sfx in all {
            let samples = render_in(sfx, sfx.variants() - 1, if sfx.spatial_variants() { Space::Far } else { Space::Open });
            let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            assert!(samples.iter().all(|s| s.is_finite()), "{sfx:?} is not finite");
            if samples.is_empty() || peak <= 0.05 || peak > 1.0 {
                quiet.push(format!("{sfx:?} peak {peak}"));
            }
        }
        assert!(quiet.is_empty(), "too quiet or clipped: {quiet:?}");
    }

    #[test]
    fn wav_header_is_valid() {
        let bytes = wav(&[0.0, 0.5, -0.5]);
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(bytes.len(), 44 + 6);
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 6);
    }
}
