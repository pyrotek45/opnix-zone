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
    /// Loops.
    Surf,
    NightDrone,
}

impl Sfx {
    pub fn looping(self) -> bool {
        matches!(self, Sfx::Surf | Sfx::NightDrone)
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
    const B: Voice = Voice::BASE;
    let mut out = Vec::new();
    let mut add = |voice: Voice, start: f32| voice.mix_into(&mut out, start, sfx_seed(sfx) + (start * 1000.0) as u64);
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
        Sfx::Surf | Sfx::NightDrone => return render_loop(sfx),
    }
    for sample in &mut out {
        *sample = sample.clamp(-1.0, 1.0);
    }
    out
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
        low += (rng.range(-1.0, 1.0) - low) * if sfx == Sfx::Surf { 0.08 } else { 0.02 };
        let sample = if sfx == Sfx::Surf {
            let swell = 0.55 + 0.3 * (k * TAU * 2.0).sin() + 0.15 * (k * TAU * 5.0).sin();
            low * swell * 1.6
        } else {
            let t = k * seconds;
            let hum = (t * TAU * 55.0).sin() * 0.22 + (t * TAU * 82.5).sin() * 0.12 + (t * TAU * 110.25).sin() * 0.06;
            hum * (0.7 + 0.3 * (k * TAU * 3.0).sin()) + low * 0.8
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
        let mut all: Vec<Sfx> = (0..8).map(Sfx::Shot).collect();
        all.extend([Sfx::Reload, Sfx::DryFire, Sfx::Hit, Sfx::Kill, Sfx::Explosion, Sfx::EnemyShot, Sfx::EnemyDeath, Sfx::Hurt, Sfx::UiMove,
            Sfx::UiAccept, Sfx::UiDeny, Sfx::Buy, Sfx::Build, Sfx::Mine, Sfx::OreGet, Sfx::Horn, Sfx::Dawn, Sfx::Alarm, Sfx::Step, Sfx::Surf, Sfx::NightDrone]);
        for sfx in all {
            let samples = render(sfx);
            let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
            assert!(!samples.is_empty() && peak > 0.05 && peak <= 1.0, "{sfx:?} peak {peak}");
            assert!(samples.iter().all(|s| s.is_finite()));
        }
    }

    #[test]
    fn wav_header_is_valid() {
        let bytes = wav(&[0.0, 0.5, -0.5]);
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(bytes.len(), 44 + 6);
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 6);
    }
}
