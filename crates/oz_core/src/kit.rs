//! A ready-made arsenal any game can start from.
//!
//! Every gun here is a [`GunFrame`] plus numbers, so every attachment in
//! [`crate::arsenal::standard_attachments`] fits it automatically. Games are
//! free to tweak prices and reserves, or to add their own frames.

use crate::{
    arsenal::{Action, Caliber, FireMode, GunClass, GunFrame, MagShape, ReloadStyle, SLOT_COUNT, StockShape, WeaponDef, slot},
    economy::Cost,
    model::{BlockModel, Paint},
};

/// A neutral template to build weapon definitions from with struct-update syntax.
pub fn blank_weapon() -> WeaponDef {
    WeaponDef {
        name: "",
        blurb: "",
        price: Cost::FREE,
        class: GunClass::Rifle,
        caliber: Caliber::Rifle,
        action: Action::Semi,
        reload_style: ReloadStyle::Magazine,
        mag: 12,
        reserve: 0,
        infinite_reserve: false,
        ammo_price: 0,
        damage: 20.0,
        interval: 0.2,
        reload: 1.0,
        pellets: 1,
        spread: 0.01,
        ads_spread: 0.35,
        range: 60.0,
        splash: 0.0,
        recoil: 0.6,
        mode: FireMode::Hitscan,
        ads_fov: 55.0,
        mining: 0.0,
        sound: 0,
        frame: None,
        defaults: [0; SLOT_COUNT],
        model: BlockModel::new(),
        length: 0.5,
    }
}

const FRAME: GunFrame = GunFrame {
    body: Paint::Steel,
    accent: Paint::Teal,
    furniture: Paint::Dark,
    width: 0.08,
    height: 0.13,
    receiver: 0.34,
    barrel: 0.3,
    bore: 0.04,
    mag: MagShape::Box,
    stock: StockShape::Fixed,
    handguard: true,
};

/// Index of each kit gun in [`standard_guns`].
pub mod gun {
    pub const PISTOL: usize = 0;
    pub const SMG: usize = 1;
    pub const SHOTGUN: usize = 2;
    pub const CARBINE: usize = 3;
    pub const LANCE: usize = 4;
    pub const LAUNCHER: usize = 5;
    pub const MINIGUN: usize = 6;
    pub const SNIPER: usize = 7;
    pub const BURST: usize = 8;
    pub const REVOLVER: usize = 9;
    pub const DMR: usize = 10;
    pub const BREACHER: usize = 11;
    pub const PDW: usize = 12;
    pub const BATTLE_RIFLE: usize = 13;
}

/// Every kit gun, in [`gun`] index order.
pub fn standard_guns() -> Vec<WeaponDef> {
    let base = blank_weapon();
    let mut sniper_defaults = [0; SLOT_COUNT];
    sniper_defaults[slot::OPTIC] = 4;
    vec![
        WeaponDef {
            name: "PULSE PISTOL",
            blurb: "Standard sidearm with a self-charging reserve.",
            class: GunClass::Pistol,
            caliber: Caliber::Pistol,
            action: Action::Semi,
            mag: 12,
            infinite_reserve: true,
            damage: 27.0,
            interval: 0.2,
            reload: 0.85,
            spread: 0.012,
            ads_spread: 0.4,
            range: 65.0,
            recoil: 0.65,
            ads_fov: 58.0,
            sound: 0,
            frame: Some(GunFrame { body: Paint::Steel, accent: Paint::Gold, width: 0.065, height: 0.11, receiver: 0.24, barrel: 0.05, bore: 0.03, stock: StockShape::None, handguard: false, ..FRAME }),
            ..base.clone()
        },
        WeaponDef {
            name: "REEF SMG",
            blurb: "Fast and loose. Shreds anything up close.",
            price: Cost::credits(180),
            class: GunClass::Smg,
            caliber: Caliber::Pistol,
            action: Action::Auto,
            mag: 32,
            reserve: 96,
            ammo_price: 30,
            damage: 17.0,
            interval: 0.072,
            reload: 1.25,
            spread: 0.03,
            range: 55.0,
            recoil: 0.32,
            ads_fov: 58.0,
            sound: 1,
            frame: Some(GunFrame { width: 0.075, height: 0.12, receiver: 0.3, barrel: 0.14, bore: 0.035, mag: MagShape::Curved, stock: StockShape::Skeleton, ..FRAME }),
            ..base.clone()
        },
        WeaponDef {
            name: "TIDE SHOTGUN",
            blurb: "Pump-action. Loads one shell at a time.",
            price: Cost::credits(260),
            class: GunClass::Shotgun,
            caliber: Caliber::Shell,
            action: Action::Pump,
            reload_style: ReloadStyle::Single,
            mag: 7,
            reserve: 28,
            ammo_price: 35,
            damage: 24.0,
            interval: 0.75,
            reload: 0.42,
            pellets: 7,
            spread: 0.1,
            ads_spread: 0.7,
            range: 36.0,
            recoil: 1.4,
            ads_fov: 60.0,
            sound: 2,
            frame: Some(GunFrame { accent: Paint::Gold, furniture: Paint::Wood, height: 0.12, receiver: 0.32, barrel: 0.44, bore: 0.05, mag: MagShape::Tube, handguard: false, ..FRAME }),
            ..base.clone()
        },
        WeaponDef {
            name: "RANGER CARBINE",
            blurb: "Accurate, hard-hitting and reliable at range.",
            price: Cost::new(390, 20, 0, 0),
            class: GunClass::Rifle,
            caliber: Caliber::Rifle,
            action: Action::Auto,
            mag: 24,
            reserve: 72,
            ammo_price: 40,
            damage: 38.0,
            interval: 0.13,
            reload: 1.3,
            spread: 0.012,
            range: 95.0,
            recoil: 0.58,
            ads_fov: 50.0,
            sound: 3,
            frame: Some(GunFrame { mag: MagShape::Curved, ..FRAME }),
            ..base.clone()
        },
        WeaponDef {
            name: "ARC LANCE",
            blurb: "Charged rail shot. One trigger pull, one dead brute.",
            price: Cost::new(560, 0, 30, 0),
            class: GunClass::Sniper,
            caliber: Caliber::Energy,
            action: Action::Semi,
            mag: 6,
            reserve: 18,
            ammo_price: 60,
            damage: 170.0,
            interval: 0.72,
            reload: 1.45,
            spread: 0.02,
            ads_spread: 0.05,
            range: 110.0,
            recoil: 0.85,
            ads_fov: 40.0,
            sound: 4,
            frame: Some(GunFrame {
                accent: Paint::Glow,
                furniture: Paint::Dark,
                width: 0.09,
                height: 0.13,
                receiver: 0.4,
                barrel: 0.42,
                bore: 0.045,
                mag: MagShape::Cell,
                stock: StockShape::Heavy,
                ..FRAME
            }),
            ..base.clone()
        },
        WeaponDef {
            name: "SUNBURST LAUNCHER",
            blurb: "Lobs an explosive charge. Mind the blast.",
            price: Cost::new(700, 0, 25, 10),
            class: GunClass::Launcher,
            caliber: Caliber::Rocket,
            action: Action::Semi,
            mag: 4,
            reserve: 8,
            ammo_price: 90,
            damage: 205.0,
            interval: 0.88,
            reload: 1.75,
            spread: 0.006,
            range: 75.0,
            splash: 9.0,
            recoil: 1.45,
            mode: FireMode::Projectile { speed: 46.0, gravity: 7.0 },
            ads_fov: 55.0,
            sound: 5,
            frame: Some(GunFrame {
                body: Paint::Teal,
                accent: Paint::Gold,
                furniture: Paint::Wood,
                width: 0.14,
                height: 0.16,
                receiver: 0.38,
                barrel: 0.26,
                bore: 0.11,
                mag: MagShape::Canister,
                handguard: false,
                ..FRAME
            }),
            ..base.clone()
        },
        WeaponDef {
            name: "TEMPEST LMG",
            blurb: "A storm on a belt. Eats ammunition alive.",
            price: Cost::new(950, 40, 40, 12),
            class: GunClass::Lmg,
            caliber: Caliber::Rifle,
            action: Action::Auto,
            reload_style: ReloadStyle::Belt,
            mag: 100,
            reserve: 200,
            ammo_price: 120,
            damage: 24.0,
            interval: 0.075,
            reload: 3.2,
            spread: 0.035,
            ads_spread: 0.4,
            range: 80.0,
            recoil: 0.42,
            ads_fov: 55.0,
            sound: 1,
            frame: Some(GunFrame {
                body: Paint::Rust,
                accent: Paint::Gold,
                width: 0.12,
                height: 0.15,
                receiver: 0.4,
                barrel: 0.38,
                bore: 0.055,
                mag: MagShape::Belt,
                stock: StockShape::Heavy,
                ..FRAME
            }),
            ..base.clone()
        },
        WeaponDef {
            name: "LONGSHOT RIFLE",
            blurb: "Bolt-action. One clean hit to the head ends it.",
            price: Cost::new(620, 25, 10, 0),
            class: GunClass::Sniper,
            caliber: Caliber::Magnum,
            action: Action::Bolt,
            mag: 5,
            reserve: 20,
            ammo_price: 55,
            damage: 115.0,
            interval: 1.15,
            reload: 2.1,
            spread: 0.09,
            ads_spread: 0.01,
            range: 170.0,
            recoil: 1.8,
            ads_fov: 40.0,
            sound: 8,
            defaults: sniper_defaults,
            frame: Some(GunFrame { body: Paint::Dark, accent: Paint::Gold, furniture: Paint::Wood, width: 0.08, height: 0.12, receiver: 0.36, barrel: 0.54, bore: 0.038, handguard: true, ..FRAME }),
            ..base.clone()
        },
        WeaponDef {
            name: "TRIDENT BURST",
            blurb: "Three-round bursts. Rewards a steady aim.",
            price: Cost::new(450, 25, 5, 0),
            class: GunClass::Rifle,
            caliber: Caliber::Rifle,
            action: Action::Burst(3),
            mag: 30,
            reserve: 90,
            ammo_price: 45,
            damage: 34.0,
            interval: 0.06,
            reload: 1.4,
            spread: 0.014,
            ads_spread: 0.3,
            range: 90.0,
            recoil: 0.5,
            ads_fov: 50.0,
            sound: 3,
            frame: Some(GunFrame { body: Paint::Concrete, accent: Paint::Red, height: 0.13, receiver: 0.32, barrel: 0.28, stock: StockShape::Skeleton, ..FRAME }),
            ..base.clone()
        },
        WeaponDef {
            name: "HARBOR REVOLVER",
            blurb: "Six heavy rounds. Slow, loud and decisive.",
            price: Cost::credits(240),
            class: GunClass::Pistol,
            caliber: Caliber::Magnum,
            action: Action::Semi,
            mag: 6,
            reserve: 24,
            ammo_price: 30,
            damage: 62.0,
            interval: 0.42,
            reload: 1.9,
            spread: 0.014,
            ads_spread: 0.3,
            range: 70.0,
            recoil: 1.3,
            ads_fov: 56.0,
            sound: 8,
            frame: Some(GunFrame {
                body: Paint::Metal,
                accent: Paint::Wood,
                furniture: Paint::Wood,
                width: 0.07,
                height: 0.12,
                receiver: 0.2,
                barrel: 0.12,
                bore: 0.035,
                mag: MagShape::Drum,
                stock: StockShape::None,
                handguard: false,
            }),
            ..base.clone()
        },
        WeaponDef {
            name: "SENTRY DMR",
            blurb: "Semi-automatic marksman rifle. Two to the chest.",
            price: Cost::new(480, 25, 0, 0),
            class: GunClass::Rifle,
            caliber: Caliber::Rifle,
            action: Action::Semi,
            mag: 15,
            reserve: 45,
            ammo_price: 45,
            damage: 58.0,
            interval: 0.24,
            reload: 1.6,
            spread: 0.02,
            ads_spread: 0.12,
            range: 130.0,
            recoil: 0.95,
            ads_fov: 42.0,
            sound: 3,
            frame: Some(GunFrame { body: Paint::Dark, accent: Paint::Teal, furniture: Paint::Wood, receiver: 0.36, barrel: 0.42, bore: 0.038, ..FRAME }),
            ..base.clone()
        },
        WeaponDef {
            name: "BREACHER",
            blurb: "Magazine-fed automatic shotgun for clearing rooms.",
            price: Cost::new(520, 20, 10, 0),
            class: GunClass::Shotgun,
            caliber: Caliber::Shell,
            action: Action::Auto,
            mag: 10,
            reserve: 30,
            ammo_price: 45,
            damage: 15.0,
            interval: 0.24,
            reload: 2.0,
            pellets: 8,
            spread: 0.12,
            ads_spread: 0.75,
            range: 28.0,
            recoil: 1.1,
            ads_fov: 62.0,
            sound: 2,
            frame: Some(GunFrame { body: Paint::Steel, accent: Paint::Red, width: 0.09, height: 0.14, receiver: 0.34, barrel: 0.3, bore: 0.05, ..FRAME }),
            ..base.clone()
        },
        WeaponDef {
            name: "WASP PDW",
            blurb: "Tiny, light, absurd fire rate. Melts at close range.",
            price: Cost::credits(260),
            class: GunClass::Smg,
            caliber: Caliber::Pistol,
            action: Action::Auto,
            mag: 40,
            reserve: 120,
            ammo_price: 35,
            damage: 13.0,
            interval: 0.052,
            reload: 1.5,
            spread: 0.035,
            ads_spread: 0.4,
            range: 42.0,
            recoil: 0.26,
            ads_fov: 60.0,
            sound: 1,
            frame: Some(GunFrame { body: Paint::Gold, accent: Paint::Dark, width: 0.07, height: 0.11, receiver: 0.26, barrel: 0.08, bore: 0.03, stock: StockShape::Skeleton, ..FRAME }),
            ..base.clone()
        },
        WeaponDef {
            name: "BULWARK RIFLE",
            blurb: "Full-power battle rifle. Hits hard, kicks harder.",
            price: Cost::new(560, 35, 10, 0),
            class: GunClass::Rifle,
            caliber: Caliber::Rifle,
            action: Action::Auto,
            mag: 20,
            reserve: 60,
            ammo_price: 50,
            damage: 48.0,
            interval: 0.11,
            reload: 1.7,
            spread: 0.016,
            range: 110.0,
            recoil: 0.9,
            ads_fov: 48.0,
            sound: 3,
            frame: Some(GunFrame {
                body: Paint::Wood,
                accent: Paint::Steel,
                furniture: Paint::Wood,
                width: 0.085,
                height: 0.14,
                receiver: 0.38,
                barrel: 0.36,
                bore: 0.042,
                stock: StockShape::Heavy,
                ..FRAME
            }),
            ..base
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arsenal::{WeaponState, assemble, resolve, standard_attachments};

    #[test]
    fn guns_are_sane_and_buildable() {
        let guns = standard_guns();
        let table = standard_attachments();
        assert_eq!(guns.len(), 14);
        for gun in &guns {
            let state = WeaponState::fresh(gun);
            let stats = resolve(gun, &state, &table);
            assert!(gun.mag > 0 && stats.damage > 0.0 && stats.interval > 0.0 && stats.length > 0.1, "{}", gun.name);
            assert!(assemble(gun, &state.equipped).parts.len() >= 12, "{}", gun.name);
        }
        assert_eq!(resolve(&guns[gun::SNIPER], &WeaponState::fresh(&guns[gun::SNIPER]), &table).scope, 2, "snipers ship with a scope");
    }
}
