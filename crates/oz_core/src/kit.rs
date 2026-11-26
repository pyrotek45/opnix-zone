//! A ready-made arsenal any game can start from.
//!
//! Games are free to define their own weapons, but most shooters want a
//! pistol, an SMG, a shotgun and friends on day one. These are the Ember
//! Atoll guns, balanced against each other and reusable as-is.

use crate::{
    arsenal::{FireMode, WeaponDef},
    economy::Cost,
    model::{BlockModel, Paint, Shape},
};

/// Shared gun silhouette: receiver, barrel, grip, stock and iron sights.
pub fn gun(body: f32, length: f32, accent: Paint) -> BlockModel {
    BlockModel::new()
        .cube(Paint::Dark, [0.0, 0.0, -0.10], [body, 0.17, 0.38])
        .cube(accent, [0.0, 0.055, -0.10], [body + 0.025, 0.12, 0.34])
        .cube(Paint::Metal, [0.0, 0.02, -(length + 0.25) * 0.5], [body * 0.65, 0.10, length - 0.25])
        .cube(Paint::Dark, [0.0, 0.02, -length + 0.035], [body * 0.9, 0.13, 0.07])
        .cube(Paint::Dark, [0.0, -0.14, 0.04], [0.10, 0.22, 0.12])
        .cube(Paint::Wood, [0.0, -0.17, 0.14], [0.18, 0.22, 0.28])
        .pair(Shape::Cube, Paint::Dark, [0.052, 0.157, -0.02], [0.029, 0.066, 0.045], None)
        .cube(Paint::Dark, [0.0, 0.165, -length + 0.09], [0.026, 0.050, 0.035])
        .cube(Paint::Glow, [0.0, 0.188, -length + 0.09], [0.022, 0.008, 0.022])
        .cube(Paint::Glow, [body * 0.53, 0.03, -0.11], [0.012, 0.04, 0.16])
}

/// A neutral template to build weapon definitions from with struct-update syntax.
pub fn blank_weapon() -> WeaponDef {
    WeaponDef {
        name: "", blurb: "", price: Cost::FREE, mag: 12, reserve: 0, infinite_reserve: false, ammo_price: 0, damage: 20.0, interval: 0.2,
        reload: 1.0, pellets: 1, spread: 0.01, range: 60.0, splash: 0.0, recoil: 0.6, auto: false, mode: FireMode::Hitscan, ads_fov: 55.0,
        mining: 0.0, length: 0.5, sound: 0, model: BlockModel::new(),
    }
}

/// Pistol, SMG, shotgun, carbine, lance, launcher and minigun, in that order.
pub fn standard_guns() -> Vec<WeaponDef> {
    let base = blank_weapon();
    vec![
        WeaponDef {
            name: "PULSE PISTOL", blurb: "Standard sidearm with a self-charging reserve.",
            mag: 12, infinite_reserve: true, damage: 27.0, interval: 0.24, reload: 0.85, spread: 0.008, range: 65.0, recoil: 0.65, ads_fov: 55.0,
            length: 0.46, sound: 0, model: gun(0.16, 0.46, Paint::Gold),
            ..base.clone()
        },
        WeaponDef {
            name: "REEF SMG", blurb: "Fast and loose. Shreds skitters up close.",
            price: Cost::credits(180), mag: 32, reserve: 96, ammo_price: 30, damage: 17.0, interval: 0.075, reload: 1.25, spread: 0.027, range: 55.0,
            recoil: 0.32, auto: true, ads_fov: 57.0, length: 0.58, sound: 1,
            model: gun(0.19, 0.58, Paint::Teal).cube(Paint::Metal, [0.0, -0.13, -0.12], [0.12, 0.26, 0.17]).cube(Paint::Dark, [0.0, 0.0, 0.19], [0.14, 0.14, 0.30]),
            ..base.clone()
        },
        WeaponDef {
            name: "TIDE SHOTGUN", blurb: "Seven pellets of bad news.",
            price: Cost::credits(260), mag: 7, reserve: 28, ammo_price: 35, damage: 24.0, interval: 0.68, reload: 1.65, pellets: 7, spread: 0.10,
            range: 36.0, recoil: 1.35, ads_fov: 59.0, length: 0.77, sound: 2,
            model: gun(0.23, 0.77, Paint::Gold)
                .pair(Shape::Cube, Paint::Metal, [0.075, 0.025, -0.48], [0.09, 0.10, 0.55], None)
                .cube(Paint::Wood, [0.0, -0.04, -0.48], [0.24, 0.12, 0.25]),
            ..base.clone()
        },
        WeaponDef {
            name: "RANGER CARBINE", blurb: "Accurate, hard-hitting and reliable at range.",
            price: Cost::new(390, 20, 0, 0), mag: 24, reserve: 72, ammo_price: 40, damage: 42.0, interval: 0.15, reload: 1.30, spread: 0.009, range: 95.0,
            recoil: 0.58, auto: true, ads_fov: 48.0, length: 0.73, sound: 3,
            model: gun(0.18, 0.73, Paint::Teal).cube(Paint::Metal, [0.0, -0.13, -0.12], [0.12, 0.26, 0.17]).cube(Paint::Dark, [0.0, 0.0, 0.19], [0.14, 0.14, 0.30]),
            ..base.clone()
        },
        WeaponDef {
            name: "ARC LANCE", blurb: "Charged rail shot. One trigger pull, one dead brute.",
            price: Cost::new(560, 0, 30, 0), mag: 6, reserve: 18, ammo_price: 60, damage: 170.0, interval: 0.72, reload: 1.45, spread: 0.001, range: 110.0,
            recoil: 0.85, ads_fov: 36.0, length: 0.88, sound: 4,
            model: gun(0.19, 0.88, Paint::Glow)
                .pair(Shape::Cube, Paint::Blue, [0.10, 0.015, -0.48], [0.035, 0.055, 0.55], None)
                .cube(Paint::Metal, [0.0, 0.112, -0.1], [0.1, 0.04, 0.21]),
            ..base.clone()
        },
        WeaponDef {
            name: "SUNBURST LAUNCHER", blurb: "Lobs an explosive charge. Mind the blast.",
            price: Cost::new(700, 0, 25, 10), mag: 4, reserve: 8, ammo_price: 90, damage: 205.0, interval: 0.88, reload: 1.75, spread: 0.006, range: 75.0,
            splash: 5.5, recoil: 1.45, mode: FireMode::Projectile { speed: 46.0, gravity: 7.0 }, ads_fov: 53.0, length: 0.69, sound: 5,
            model: gun(0.29, 0.69, Paint::Wood)
                .cube(Paint::Teal, [0.0, 0.015, -0.35], [0.30, 0.27, 0.55])
                .cube(Paint::Gold, [0.0, 0.015, -0.62], [0.33, 0.30, 0.09])
                .cube(Paint::Dark, [0.0, 0.015, -0.68], [0.21, 0.19, 0.03]),
            ..base.clone()
        },
        WeaponDef {
            name: "TEMPEST MINIGUN", blurb: "A storm in a barrel. Eats ammunition alive.",
            price: Cost::new(950, 40, 40, 12), mag: 150, reserve: 300, ammo_price: 120, damage: 15.0, interval: 0.045, reload: 2.6, spread: 0.035,
            range: 60.0, recoil: 0.22, auto: true, ads_fov: 60.0, length: 0.82, sound: 1,
            model: gun(0.27, 0.82, Paint::Rust)
                .cylinder(Paint::Metal, [0.0, 0.02, -0.58], [0.22, 0.52, 0.22]).rotated([90.0, 0.0, 0.0])
                .cylinder(Paint::Dark, [0.0, 0.02, -0.84], [0.25, 0.05, 0.25]).rotated([90.0, 0.0, 0.0])
                .cube(Paint::Gold, [0.0, -0.2, -0.14], [0.22, 0.24, 0.26]),
            ..base.clone()
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guns_are_sane() {
        let guns = standard_guns();
        assert_eq!(guns.len(), 7);
        for gun in &guns {
            assert!(gun.mag > 0 && gun.damage > 0.0 && gun.interval > 0.0 && !gun.model.parts.is_empty(), "{}", gun.name);
        }
    }
}
