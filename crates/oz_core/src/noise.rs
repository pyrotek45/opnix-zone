//! Seeded 2D value noise and fractal Brownian motion.

fn hash(seed: u64, x: i32, z: i32) -> f32 {
    let mut h = seed ^ (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (z as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h = (h ^ (h >> 31)).wrapping_mul(0xD6E8_FEB8_6659_FD93);
    h ^= h >> 32;
    (h >> 40) as f32 / (1u64 << 24) as f32
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Value noise in `[-1, 1]`, one feature per unit.
pub fn value(seed: u64, x: f32, z: f32) -> f32 {
    let (x0, z0) = (x.floor(), z.floor());
    let (u, v) = (smooth(x - x0), smooth(z - z0));
    let (xi, zi) = (x0 as i32, z0 as i32);
    let a = hash(seed, xi, zi);
    let b = hash(seed, xi + 1, zi);
    let c = hash(seed, xi, zi + 1);
    let d = hash(seed, xi + 1, zi + 1);
    let top = a + (b - a) * u;
    let bottom = c + (d - c) * u;
    (top + (bottom - top) * v) * 2.0 - 1.0
}

/// Fractal noise in roughly `[-1, 1]`.
pub fn fbm(seed: u64, x: f32, z: f32, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for octave in 0..octaves {
        sum += amp * value(seed.wrapping_add(octave as u64 * 1013), x * freq, z * freq);
        norm += amp;
        amp *= 0.5;
        freq *= 2.03;
    }
    sum / norm
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_and_continuous() {
        for i in 0..500 {
            let (x, z) = (i as f32 * 0.37, i as f32 * -0.21);
            let n = fbm(42, x, z, 4);
            assert!((-1.0..=1.0).contains(&n));
            assert!((n - fbm(42, x + 0.001, z, 4)).abs() < 0.02);
        }
    }
}
