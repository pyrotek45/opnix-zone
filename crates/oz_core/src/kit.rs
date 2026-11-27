//! A ready-made arsenal any game can start from.
//!
//! Every gun here is a [`GunFrame`] plus numbers, so every attachment in
//! [`crate::arsenal::standard_attachments`] fits it automatically. Games are
//! free to tweak prices and reserves, or to add their own frames.

use crate::{
    arsenal::{
        Action, BarrelShape, Caliber, FireMode, Foregrip, GripShape, GunClass, GunFrame, GunLayout, MagShape, ReceiverShape, ReloadStyle, SLOT_COUNT, StockShape, WeaponDef, slot,
    },
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
        weight: 3.0,
    }
}

const FRAME: GunFrame = GunFrame::RIFLE;

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
    pub const BULLPUP: usize = 14;
    pub const MACHINE_PISTOL: usize = 15;
    pub const DRUM_SMG: usize = 16;
    pub const CQB: usize = 17;
    pub const LEVER: usize = 18;
    pub const ANTI_MATERIEL: usize = 19;
    pub const ROTARY: usize = 20;
    pub const SCOUT: usize = 21;
    pub const MARKSMAN: usize = 22;
    pub const DOUBLE: usize = 23;
    pub const AUTO_SHOTGUN: usize = 24;
    pub const HAND_CANNON: usize = 25;
    pub const LIGHT_LMG: usize = 26;
    pub const STINGER: usize = 27;
}

/// Every kit gun, in [`gun`] index order.
pub fn standard_guns() -> Vec<WeaponDef> {
    let base = blank_weapon();
    let mut scoped = [0; SLOT_COUNT];
    scoped[slot::OPTIC] = 4;
    let mut six = [0; SLOT_COUNT];
    six[slot::OPTIC] = 5;
    vec![
        WeaponDef {
            name: "PULSE PISTOL",
            blurb: "Standard sidearm with a self-charging reserve. Fast to draw.",
            class: GunClass::Pistol,
            caliber: Caliber::Pistol,
            action: Action::Semi,
            mag: 12,
            infinite_reserve: true,
            damage: 26.0,
            interval: 0.18,
            reload: 0.85,
            spread: 0.012,
            ads_spread: 0.4,
            range: 25.0,
            recoil: 0.6,
            ads_fov: 58.0,
            sound: 0,
            weight: 0.9,
            frame: Some(GunFrame {
                accent: Paint::Gold,
                width: 0.065,
                height: 0.11,
                receiver: 0.24,
                barrel: 0.05,
                bore: 0.03,
                stock: StockShape::None,
                handguard: false,
                shape: ReceiverShape::Angular,
                ..FRAME
            }),
            ..base.clone()
        },
        WeaponDef {
            name: "REEF SMG",
            blurb: "Stick magazine, folding stock. Shreds anything up close.",
            price: Cost::credits(180),
            class: GunClass::Smg,
            caliber: Caliber::Pistol,
            action: Action::Auto,
            mag: 30,
            reserve: 90,
            ammo_price: 30,
            damage: 20.0,
            interval: 0.075,
            reload: 1.3,
            spread: 0.022,
            ads_spread: 0.45,
            range: 20.0,
            recoil: 0.34,
            ads_fov: 58.0,
            sound: 1,
            weight: 2.6,
            frame: Some(GunFrame {
                width: 0.075,
                height: 0.12,
                receiver: 0.3,
                barrel: 0.14,
                bore: 0.035,
                mag_len: 1.4,
                stock: StockShape::Folding,
                shape: ReceiverShape::Angular,
                fore: Foregrip::Angled,
                ..FRAME
            }),
            ..base.clone()
        },
        WeaponDef {
            name: "TIDE SHOTGUN",
            blurb: "Pump-action. Owns a room, useless past ten metres.",
            price: Cost::credits(260),
            class: GunClass::Shotgun,
            caliber: Caliber::Shell,
            action: Action::Pump,
            reload_style: ReloadStyle::Single,
            mag: 7,
            reserve: 28,
            ammo_price: 35,
            damage: 20.0,
            interval: 0.75,
            reload: 0.42,
            pellets: 8,
            spread: 0.1,
            ads_spread: 0.7,
            range: 9.0,
            recoil: 1.5,
            ads_fov: 60.0,
            sound: 2,
            weight: 3.4,
            frame: Some(GunFrame {
                accent: Paint::Gold,
                furniture: Paint::Wood,
                height: 0.12,
                receiver: 0.32,
                barrel: 0.44,
                bore: 0.05,
                mag: MagShape::Tube,
                stock: StockShape::Sloped,
                handguard: false,
                barrel_shape: BarrelShape::Rib,
                grip: GripShape::Wrist,
                ..FRAME
            }),
            ..base.clone()
        },
        WeaponDef {
            name: "RANGER CARBINE",
            blurb: "Carry handle, banana mag. Good at every range, best at none.",
            price: Cost::new(390, 20, 0, 0),
            class: GunClass::Rifle,
            caliber: Caliber::Rifle,
            action: Action::Auto,
            mag: 30,
            reserve: 90,
            ammo_price: 40,
            damage: 30.0,
            interval: 0.11,
            reload: 1.4,
            spread: 0.016,
            ads_spread: 0.3,
            range: 45.0,
            recoil: 0.5,
            ads_fov: 50.0,
            sound: 3,
            weight: 3.4,
            frame: Some(GunFrame { mag: MagShape::Curved, stock: StockShape::Tube, carry: true, ..FRAME }),
            ..base.clone()
        },
        WeaponDef {
            name: "ARC LANCE",
            blurb: "Charged rail shot. One trigger pull, one body down.",
            price: Cost::new(560, 0, 30, 0),
            class: GunClass::Sniper,
            caliber: Caliber::Energy,
            action: Action::Semi,
            mag: 6,
            reserve: 18,
            ammo_price: 60,
            damage: 110.0,
            interval: 0.75,
            reload: 1.5,
            spread: 0.05,
            ads_spread: 0.04,
            range: 100.0,
            recoil: 1.2,
            ads_fov: 40.0,
            sound: 4,
            weight: 5.4,
            frame: Some(GunFrame {
                accent: Paint::Glow,
                furniture: Paint::Dark,
                width: 0.09,
                height: 0.13,
                receiver: 0.4,
                barrel: 0.42,
                bore: 0.045,
                mag: MagShape::Cell,
                stock: StockShape::Thumbhole,
                handguard: false,
                shape: ReceiverShape::Sleek,
                barrel_shape: BarrelShape::Rails,
                grip: GripShape::Thumbhole,
                ..FRAME
            }),
            ..base.clone()
        },
        WeaponDef {
            name: "SUNBURST LAUNCHER",
            blurb: "Four-shot revolving grenade launcher. A direct hit kills; the reload takes forever.",
            price: Cost::new(700, 0, 25, 10),
            class: GunClass::Launcher,
            caliber: Caliber::Rocket,
            action: Action::Semi,
            mag: 4,
            reserve: 4,
            ammo_price: 90,
            damage: 125.0,
            interval: 1.1,
            reload: 4.5,
            spread: 0.008,
            range: 60.0,
            splash: 4.5,
            recoil: 1.9,
            mode: FireMode::Projectile { speed: 40.0, gravity: 8.5 },
            ads_fov: 55.0,
            sound: 5,
            weight: 8.2,
            frame: Some(GunFrame {
                body: Paint::Teal,
                accent: Paint::Gold,
                furniture: Paint::Wood,
                width: 0.14,
                height: 0.16,
                receiver: 0.38,
                barrel: 0.26,
                bore: 0.11,
                mag: MagShape::Cylinder,
                stock: StockShape::Folding,
                handguard: false,
                fore: Foregrip::Vertical,
                ..FRAME
            }),
            ..base.clone()
        },
        WeaponDef {
            name: "TEMPEST LMG",
            blurb: "Box-fed, bipod, a hundred rounds. Holds a lane shut.",
            price: Cost::new(950, 40, 40, 12),
            class: GunClass::Lmg,
            caliber: Caliber::Rifle,
            action: Action::Auto,
            reload_style: ReloadStyle::Belt,
            mag: 100,
            reserve: 200,
            ammo_price: 120,
            damage: 26.0,
            interval: 0.08,
            reload: 3.4,
            spread: 0.04,
            ads_spread: 0.4,
            range: 55.0,
            recoil: 0.6,
            ads_fov: 55.0,
            sound: 1,
            weight: 8.6,
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
                handguard: false,
                shape: ReceiverShape::Slab,
                barrel_shape: BarrelShape::Shroud,
                carry: true,
                bipod: true,
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
            damage: 95.0,
            interval: 1.2,
            reload: 2.2,
            spread: 0.1,
            ads_spread: 0.01,
            range: 150.0,
            recoil: 1.8,
            ads_fov: 40.0,
            sound: 8,
            weight: 5.9,
            defaults: scoped,
            frame: Some(GunFrame {
                body: Paint::Dark,
                accent: Paint::Gold,
                furniture: Paint::Wood,
                height: 0.12,
                receiver: 0.36,
                barrel: 0.54,
                bore: 0.038,
                mag_len: 0.5,
                stock: StockShape::Sloped,
                shape: ReceiverShape::Round,
                barrel_shape: BarrelShape::Brake,
                grip: GripShape::Wrist,
                ..FRAME
            }),
            ..base.clone()
        },
        WeaponDef {
            name: "TRIDENT BURST",
            blurb: "Three-round bursts. One clean burst drops a target.",
            price: Cost::new(450, 25, 5, 0),
            class: GunClass::Rifle,
            caliber: Caliber::Rifle,
            action: Action::Burst(3),
            mag: 30,
            reserve: 90,
            ammo_price: 45,
            damage: 34.0,
            interval: 0.06,
            reload: 1.45,
            spread: 0.018,
            ads_spread: 0.25,
            range: 50.0,
            recoil: 0.5,
            ads_fov: 48.0,
            sound: 3,
            weight: 3.5,
            frame: Some(GunFrame {
                body: Paint::Concrete,
                accent: Paint::Red,
                receiver: 0.32,
                barrel: 0.28,
                stock: StockShape::Skeleton,
                shape: ReceiverShape::Angular,
                fore: Foregrip::Stop,
                ..FRAME
            }),
            ..base.clone()
        },
        WeaponDef {
            name: "HARBOR REVOLVER",
            blurb: "Six heavy rounds. Two to the body, one to the head.",
            price: Cost::credits(240),
            class: GunClass::Pistol,
            caliber: Caliber::Magnum,
            action: Action::Semi,
            mag: 6,
            reserve: 24,
            ammo_price: 30,
            damage: 55.0,
            interval: 0.4,
            reload: 1.9,
            spread: 0.016,
            ads_spread: 0.3,
            range: 35.0,
            recoil: 1.3,
            ads_fov: 56.0,
            sound: 8,
            weight: 1.4,
            frame: Some(GunFrame {
                body: Paint::Metal,
                accent: Paint::Wood,
                furniture: Paint::Wood,
                width: 0.07,
                height: 0.12,
                receiver: 0.2,
                barrel: 0.12,
                bore: 0.035,
                mag: MagShape::Cylinder,
                stock: StockShape::None,
                handguard: false,
                barrel_shape: BarrelShape::Lug,
                grip: GripShape::Raked,
                ..FRAME
            }),
            ..base.clone()
        },
        WeaponDef {
            name: "SENTRY DMR",
            blurb: "Semi-automatic marksman rifle. Two to the chest at range.",
            price: Cost::new(480, 25, 0, 0),
            class: GunClass::Rifle,
            caliber: Caliber::Rifle,
            action: Action::Semi,
            mag: 15,
            reserve: 45,
            ammo_price: 45,
            damage: 55.0,
            interval: 0.24,
            reload: 1.6,
            spread: 0.03,
            ads_spread: 0.1,
            range: 80.0,
            recoil: 0.95,
            ads_fov: 42.0,
            sound: 3,
            weight: 4.6,
            frame: Some(GunFrame {
                body: Paint::Dark,
                accent: Paint::Teal,
                furniture: Paint::Wood,
                receiver: 0.36,
                barrel: 0.42,
                bore: 0.038,
                mag_len: 0.8,
                stock: StockShape::Thumbhole,
                barrel_shape: BarrelShape::Fluted,
                rails: true,
                ..FRAME
            }),
            ..base.clone()
        },
        WeaponDef {
            name: "BREACHER",
            blurb: "Drum-fed automatic shotgun for clearing rooms.",
            price: Cost::new(520, 20, 10, 0),
            class: GunClass::Shotgun,
            caliber: Caliber::Shell,
            action: Action::Auto,
            mag: 12,
            reserve: 36,
            ammo_price: 45,
            damage: 13.0,
            interval: 0.24,
            reload: 2.2,
            pellets: 8,
            spread: 0.12,
            ads_spread: 0.75,
            range: 7.0,
            recoil: 1.25,
            ads_fov: 62.0,
            sound: 2,
            weight: 5.2,
            frame: Some(GunFrame {
                body: Paint::Steel,
                accent: Paint::Red,
                width: 0.09,
                height: 0.14,
                receiver: 0.34,
                barrel: 0.3,
                bore: 0.05,
                mag: MagShape::Drum,
                handguard: false,
                shape: ReceiverShape::Slab,
                barrel_shape: BarrelShape::Shroud,
                fore: Foregrip::Vertical,
                ..FRAME
            }),
            ..base.clone()
        },
        WeaponDef {
            name: "WASP PDW",
            blurb: "Bullpup, fifty rounds on top, absurd fire rate. Run and gun.",
            price: Cost::credits(260),
            class: GunClass::Smg,
            caliber: Caliber::Pistol,
            action: Action::Auto,
            mag: 50,
            reserve: 150,
            ammo_price: 35,
            damage: 14.0,
            interval: 0.05,
            reload: 1.6,
            spread: 0.02,
            ads_spread: 0.45,
            range: 14.0,
            recoil: 0.24,
            ads_fov: 60.0,
            sound: 1,
            weight: 2.2,
            frame: Some(GunFrame {
                body: Paint::Gold,
                accent: Paint::Dark,
                width: 0.075,
                height: 0.12,
                receiver: 0.2,
                barrel: 0.1,
                bore: 0.03,
                mag: MagShape::Top,
                stock: StockShape::None,
                handguard: false,
                layout: GunLayout::Bullpup,
                shape: ReceiverShape::Sleek,
                grip: GripShape::Thumbhole,
                ..FRAME
            }),
            ..base.clone()
        },
        WeaponDef {
            name: "BULWARK RIFLE",
            blurb: "Full-power battle rifle. Three hits at any range, kicks hard.",
            price: Cost::new(560, 35, 10, 0),
            class: GunClass::Rifle,
            caliber: Caliber::Rifle,
            action: Action::Auto,
            mag: 20,
            reserve: 60,
            ammo_price: 50,
            damage: 42.0,
            interval: 0.12,
            reload: 1.75,
            spread: 0.02,
            ads_spread: 0.3,
            range: 60.0,
            recoil: 1.0,
            ads_fov: 48.0,
            sound: 3,
            weight: 4.8,
            frame: Some(GunFrame {
                body: Paint::Wood,
                accent: Paint::Steel,
                furniture: Paint::Wood,
                width: 0.085,
                height: 0.14,
                receiver: 0.38,
                barrel: 0.36,
                bore: 0.042,
                mag_len: 1.15,
                shape: ReceiverShape::Slab,
                ..FRAME
            }),
            ..base.clone()
        },
        WeaponDef {
            name: "WARDEN BULLPUP",
            blurb: "Bullpup assault rifle: a full barrel in a short body.",
            price: Cost::new(470, 25, 5, 0),
            class: GunClass::Rifle,
            caliber: Caliber::Rifle,
            action: Action::Auto,
            mag: 30,
            reserve: 90,
            ammo_price: 45,
            damage: 28.0,
            interval: 0.09,
            reload: 1.6,
            spread: 0.017,
            ads_spread: 0.3,
            range: 40.0,
            recoil: 0.44,
            ads_fov: 52.0,
            sound: 3,
            weight: 3.7,
            frame: Some(GunFrame {
                body: Paint::Leaf,
                accent: Paint::Dark,
                receiver: 0.26,
                barrel: 0.3,
                mag: MagShape::Curved,
                stock: StockShape::None,
                layout: GunLayout::Bullpup,
                shape: ReceiverShape::Angular,
                fore: Foregrip::Stop,
                carry: true,
                ..FRAME
            }),
            ..base.clone()
        },
        WeaponDef {
            name: "HORNET AUTOPISTOL",
            blurb: "Full-auto sidearm with a long magazine. Panic button.",
            price: Cost::credits(220),
            class: GunClass::Pistol,
            caliber: Caliber::Pistol,
            action: Action::Auto,
            mag: 24,
            reserve: 96,
            ammo_price: 30,
            damage: 15.0,
            interval: 0.055,
            reload: 1.15,
            spread: 0.03,
            ads_spread: 0.5,
            range: 12.0,
            recoil: 0.38,
            ads_fov: 60.0,
            sound: 0,
            weight: 1.3,
            frame: Some(GunFrame {
                body: Paint::Dark,
                accent: Paint::Coral,
                width: 0.07,
                height: 0.12,
                receiver: 0.26,
                barrel: 0.07,
                bore: 0.032,
                mag_len: 1.9,
                stock: StockShape::Folding,
                handguard: false,
                shape: ReceiverShape::Angular,
                barrel_shape: BarrelShape::Brake,
                fore: Foregrip::Vertical,
                ..FRAME
            }),
            ..base.clone()
        },
        WeaponDef {
            name: "FOXHOLE SMG",
            blurb: "Wartime drum gun. Fifty rounds, wood and steel, hits hard.",
            price: Cost::credits(240),
            class: GunClass::Smg,
            caliber: Caliber::Pistol,
            action: Action::Auto,
            mag: 50,
            reserve: 100,
            ammo_price: 40,
            damage: 22.0,
            interval: 0.095,
            reload: 2.3,
            spread: 0.024,
            ads_spread: 0.4,
            range: 24.0,
            recoil: 0.36,
            ads_fov: 58.0,
            sound: 1,
            weight: 4.6,
            frame: Some(GunFrame {
                body: Paint::Iron,
                accent: Paint::Dark,
                furniture: Paint::Wood,
                width: 0.07,
                height: 0.1,
                receiver: 0.34,
                barrel: 0.2,
                bore: 0.035,
                mag: MagShape::Drum,
                stock: StockShape::Sloped,
                handguard: false,
                shape: ReceiverShape::Round,
                barrel_shape: BarrelShape::Shroud,
                grip: GripShape::Raked,
                fore: Foregrip::Vertical,
                ..FRAME
            }),
            ..base.clone()
        },
        WeaponDef {
            name: "SPECTRE CQB",
            blurb: "Short-barrel carbine covered in rails. Rifle punch indoors.",
            price: Cost::new(420, 20, 0, 0),
            class: GunClass::Rifle,
            caliber: Caliber::Rifle,
            action: Action::Auto,
            mag: 30,
            reserve: 90,
            ammo_price: 40,
            damage: 26.0,
            interval: 0.08,
            reload: 1.2,
            spread: 0.018,
            ads_spread: 0.32,
            range: 28.0,
            recoil: 0.4,
            ads_fov: 54.0,
            sound: 3,
            weight: 3.0,
            frame: Some(GunFrame {
                body: Paint::Sand,
                accent: Paint::Dark,
                width: 0.08,
                height: 0.13,
                receiver: 0.3,
                barrel: 0.2,
                stock: StockShape::Tube,
                fore: Foregrip::Stop,
                rails: true,
                ..FRAME
            }),
            ..base.clone()
        },
        WeaponDef {
            name: "PRAIRIE LEVER",
            blurb: "Lever-action magnum. Fast follow-ups, loads round by round.",
            price: Cost::new(380, 15, 0, 0),
            class: GunClass::Rifle,
            caliber: Caliber::Magnum,
            action: Action::Bolt,
            reload_style: ReloadStyle::Single,
            mag: 8,
            reserve: 32,
            ammo_price: 40,
            damage: 70.0,
            interval: 0.6,
            reload: 0.45,
            spread: 0.035,
            ads_spread: 0.12,
            range: 60.0,
            recoil: 1.1,
            ads_fov: 46.0,
            sound: 8,
            weight: 3.6,
            frame: Some(GunFrame {
                body: Paint::Copper,
                accent: Paint::Gold,
                furniture: Paint::Wood,
                width: 0.07,
                height: 0.11,
                receiver: 0.3,
                barrel: 0.46,
                bore: 0.036,
                mag: MagShape::Tube,
                stock: StockShape::Sloped,
                shape: ReceiverShape::Round,
                grip: GripShape::Lever,
                ..FRAME
            }),
            ..base.clone()
        },
        WeaponDef {
            name: "GOLIATH RIFLE",
            blurb: "Anti-materiel rifle. One shot to the chest, if you can carry it.",
            price: Cost::new(900, 40, 20, 5),
            class: GunClass::Sniper,
            caliber: Caliber::Magnum,
            action: Action::Semi,
            mag: 5,
            reserve: 15,
            ammo_price: 70,
            damage: 140.0,
            interval: 1.0,
            reload: 2.8,
            spread: 0.12,
            ads_spread: 0.01,
            range: 200.0,
            recoil: 2.8,
            ads_fov: 40.0,
            sound: 8,
            weight: 13.0,
            defaults: scoped,
            frame: Some(GunFrame {
                body: Paint::Steel,
                accent: Paint::Rust,
                width: 0.1,
                height: 0.15,
                receiver: 0.44,
                barrel: 0.62,
                bore: 0.05,
                mag_len: 1.1,
                stock: StockShape::Heavy,
                handguard: false,
                shape: ReceiverShape::Slab,
                barrel_shape: BarrelShape::Brake,
                bipod: true,
                ..FRAME
            }),
            ..base.clone()
        },
        WeaponDef {
            name: "CYCLONE ROTARY",
            blurb: "Six spinning barrels. Erases anything close, then reloads forever.",
            price: Cost::new(1100, 45, 45, 15),
            class: GunClass::Lmg,
            caliber: Caliber::Rifle,
            action: Action::Auto,
            reload_style: ReloadStyle::Belt,
            mag: 200,
            reserve: 400,
            ammo_price: 150,
            damage: 16.0,
            interval: 0.04,
            reload: 4.2,
            spread: 0.05,
            ads_spread: 0.6,
            range: 30.0,
            recoil: 0.42,
            ads_fov: 58.0,
            sound: 1,
            weight: 14.5,
            frame: Some(GunFrame {
                body: Paint::Iron,
                accent: Paint::Gold,
                width: 0.13,
                height: 0.16,
                receiver: 0.36,
                barrel: 0.4,
                bore: 0.035,
                mag: MagShape::Belt,
                mag_len: 1.3,
                stock: StockShape::None,
                handguard: false,
                shape: ReceiverShape::Slab,
                barrel_shape: BarrelShape::Rotary,
                fore: Foregrip::Vertical,
                carry: true,
                ..FRAME
            }),
            ..base.clone()
        },
        // New guns go on the end, so saved classes keep their indices.
        WeaponDef {
            name: "KESTREL SCOUT",
            blurb: "Light bolt-action with a 6x. Quick to shoulder, two to the body.",
            price: Cost::new(520, 20, 5, 0),
            class: GunClass::Sniper,
            caliber: Caliber::Magnum,
            action: Action::Bolt,
            mag: 6,
            reserve: 24,
            ammo_price: 50,
            damage: 80.0,
            interval: 0.95,
            reload: 1.9,
            spread: 0.08,
            ads_spread: 0.012,
            range: 130.0,
            recoil: 1.3,
            ads_fov: 40.0,
            sound: 8,
            weight: 3.6,
            defaults: six,
            frame: Some(GunFrame {
                body: Paint::Sand,
                accent: Paint::Teal,
                furniture: Paint::Dark,
                width: 0.075,
                height: 0.11,
                receiver: 0.32,
                barrel: 0.48,
                bore: 0.034,
                mag_len: 0.7,
                stock: StockShape::Skeleton,
                shape: ReceiverShape::Sleek,
                barrel_shape: BarrelShape::Fluted,
                ..FRAME
            }),
            ..base.clone()
        },
        WeaponDef {
            name: "HERON MARKSMAN",
            blurb: "Semi-automatic sniper on a chassis stock. Ten rounds, fast follow-ups.",
            price: Cost::new(640, 30, 10, 0),
            class: GunClass::Sniper,
            caliber: Caliber::Rifle,
            action: Action::Semi,
            mag: 10,
            reserve: 30,
            ammo_price: 55,
            damage: 68.0,
            interval: 0.36,
            reload: 2.0,
            spread: 0.06,
            ads_spread: 0.02,
            range: 110.0,
            recoil: 1.15,
            ads_fov: 40.0,
            sound: 3,
            weight: 5.3,
            defaults: scoped,
            frame: Some(GunFrame {
                body: Paint::Leaf,
                accent: Paint::Dark,
                furniture: Paint::Dark,
                width: 0.085,
                height: 0.13,
                receiver: 0.38,
                barrel: 0.46,
                bore: 0.04,
                mag_len: 0.9,
                stock: StockShape::Precision,
                shape: ReceiverShape::Angular,
                fore: Foregrip::Stop,
                rails: true,
                ..FRAME
            }),
            ..base.clone()
        },
        WeaponDef {
            name: "MAULER DOUBLE",
            blurb: "Side-by-side double barrel. Two enormous blasts, then two shells in.",
            price: Cost::credits(300),
            class: GunClass::Shotgun,
            caliber: Caliber::Shell,
            action: Action::Semi,
            reload_style: ReloadStyle::Single,
            mag: 2,
            reserve: 16,
            ammo_price: 30,
            damage: 19.0,
            interval: 0.16,
            reload: 0.8,
            pellets: 10,
            spread: 0.11,
            ads_spread: 0.75,
            range: 10.0,
            recoil: 1.9,
            ads_fov: 62.0,
            sound: 2,
            weight: 3.2,
            frame: Some(GunFrame {
                body: Paint::Iron,
                accent: Paint::Gold,
                furniture: Paint::Wood,
                width: 0.07,
                height: 0.11,
                receiver: 0.24,
                barrel: 0.5,
                bore: 0.042,
                mag: MagShape::Internal,
                stock: StockShape::Sloped,
                handguard: false,
                barrel_shape: BarrelShape::Twin,
                grip: GripShape::Wrist,
                ..FRAME
            }),
            ..base.clone()
        },
        WeaponDef {
            name: "RIPTIDE AUTO-12",
            blurb: "Magazine-fed semi-auto shotgun. Fast follow-ups, quick reloads.",
            price: Cost::new(480, 20, 5, 0),
            class: GunClass::Shotgun,
            caliber: Caliber::Shell,
            action: Action::Semi,
            mag: 8,
            reserve: 32,
            ammo_price: 40,
            damage: 15.0,
            interval: 0.28,
            reload: 2.0,
            pellets: 8,
            spread: 0.1,
            ads_spread: 0.7,
            range: 11.0,
            recoil: 1.3,
            ads_fov: 60.0,
            sound: 2,
            weight: 4.4,
            frame: Some(GunFrame {
                body: Paint::Dark,
                accent: Paint::Coral,
                furniture: Paint::Dark,
                width: 0.085,
                height: 0.14,
                receiver: 0.36,
                barrel: 0.34,
                bore: 0.048,
                mag_len: 0.9,
                stock: StockShape::Tube,
                shape: ReceiverShape::Angular,
                fore: Foregrip::Vertical,
                rails: true,
                ..FRAME
            }),
            ..base.clone()
        },
        WeaponDef {
            name: "WARHORSE HAND CANNON",
            blurb: "Gas-operated magnum pistol. Seven heavy rounds, a brutal kick.",
            price: Cost::credits(280),
            class: GunClass::Pistol,
            caliber: Caliber::Magnum,
            action: Action::Semi,
            mag: 7,
            reserve: 28,
            ammo_price: 30,
            damage: 52.0,
            interval: 0.32,
            reload: 1.6,
            spread: 0.018,
            ads_spread: 0.28,
            range: 32.0,
            recoil: 1.7,
            ads_fov: 55.0,
            sound: 8,
            weight: 2.0,
            frame: Some(GunFrame {
                body: Paint::Steel,
                accent: Paint::Gold,
                furniture: Paint::Dark,
                width: 0.075,
                height: 0.14,
                receiver: 0.3,
                barrel: 0.08,
                bore: 0.04,
                stock: StockShape::None,
                handguard: false,
                barrel_shape: BarrelShape::Brake,
                ..FRAME
            }),
            ..base.clone()
        },
        WeaponDef {
            name: "SKIRMISHER LMG",
            blurb: "Light machine gun with a drum. Moves like a rifle, fires like an LMG.",
            price: Cost::new(780, 35, 30, 8),
            class: GunClass::Lmg,
            caliber: Caliber::Rifle,
            action: Action::Auto,
            mag: 75,
            reserve: 225,
            ammo_price: 100,
            damage: 24.0,
            interval: 0.075,
            reload: 2.6,
            spread: 0.032,
            ads_spread: 0.38,
            range: 50.0,
            recoil: 0.5,
            ads_fov: 52.0,
            sound: 1,
            weight: 6.2,
            frame: Some(GunFrame {
                body: Paint::Concrete,
                accent: Paint::Dark,
                width: 0.1,
                height: 0.14,
                receiver: 0.38,
                barrel: 0.34,
                bore: 0.045,
                mag: MagShape::Drum,
                stock: StockShape::Tube,
                shape: ReceiverShape::Angular,
                barrel_shape: BarrelShape::Fluted,
                fore: Foregrip::Angled,
                bipod: true,
                rails: true,
                ..FRAME
            }),
            ..base.clone()
        },
        WeaponDef {
            name: "STINGER PDW",
            blurb: "Featherweight personal defence weapon. Forty rounds and quick hands.",
            price: Cost::credits(230),
            class: GunClass::Smg,
            caliber: Caliber::Pistol,
            action: Action::Auto,
            mag: 40,
            reserve: 160,
            ammo_price: 30,
            damage: 16.0,
            interval: 0.058,
            reload: 1.4,
            spread: 0.019,
            ads_spread: 0.42,
            range: 17.0,
            recoil: 0.26,
            ads_fov: 58.0,
            sound: 1,
            weight: 1.9,
            frame: Some(GunFrame {
                body: Paint::Teal,
                accent: Paint::Dark,
                width: 0.07,
                height: 0.115,
                receiver: 0.24,
                barrel: 0.08,
                bore: 0.03,
                mag_len: 1.5,
                stock: StockShape::Tube,
                handguard: false,
                shape: ReceiverShape::Sleek,
                fore: Foregrip::Vertical,
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
        assert_eq!(guns.len(), 28);
        for gun in &guns {
            let state = WeaponState::fresh(gun);
            let stats = resolve(gun, &state, &table);
            assert!(gun.mag > 0 && stats.damage > 0.0 && stats.interval > 0.0 && stats.length > 0.1, "{}", gun.name);
            let plain = assemble(gun, &state.equipped);
            assert!(plain.parts.len() + plain.hulls.len() * 2 >= 12, "{}", gun.name);
            // Every part in one model stays sane and within a modest budget, even fully kitted.
            let kitted = assemble(gun, &[3, 2, 1, 1, 2, 2, 0]);
            assert!(kitted.parts.len() <= 80, "{} has {} parts", gun.name, kitted.parts.len());
            assert!(kitted.parts.iter().all(|p| p.pos.is_finite() && p.rot.is_finite() && p.size.min_element() > 0.0), "{}", gun.name);
        }
        assert_eq!(resolve(&guns[gun::SNIPER], &WeaponState::fresh(&guns[gun::SNIPER]), &table).scope, 2, "snipers ship with a scope");
    }

    #[test]
    fn every_part_fits_and_builds_on_every_gun() {
        let guns = standard_guns();
        let table = standard_attachments();
        for gun in &guns {
            let plain = assemble(gun, &gun.defaults);
            for slot in 0..crate::arsenal::slot::AMMO {
                for (variant, name, ..) in crate::arsenal::options(gun, &table, slot) {
                    let mut parts = gun.defaults;
                    parts[slot] = variant as u8;
                    let model = assemble(gun, &parts);
                    assert!(model.parts.len() <= 90, "{} with {name}: {} parts", gun.name, model.parts.len());
                    assert!(model.parts.iter().all(|p| p.pos.is_finite() && p.size.min_element() > 0.0), "{} with {name}", gun.name);
                    // Every part shows on the gun (magazines only where the feed can grow: not cylinders or drums).
                    if variant != gun.defaults[slot] as usize && variant != 0 && slot != crate::arsenal::slot::MAGAZINE && !(slot == crate::arsenal::slot::STOCK && variant == 1) {
                        assert!(format!("{:?}{:?}", model.parts, model.hulls) != format!("{:?}{:?}", plain.parts, plain.hulls), "{name} is invisible on {}", gun.name);
                    }
                    let stats = resolve(gun, &WeaponState { equipped: parts, ..WeaponState::fresh(gun) }, &table);
                    assert!(stats.weight > 0.5 && stats.weight < 18.0, "{} with {name} weighs {}", gun.name, stats.weight);
                }
            }
        }
    }

    #[test]
    fn stronger_optics_zoom_further_and_long_glass_stays_on_long_guns() {
        let table = standard_attachments();
        let optic = |name: &str| table[slot::OPTIC].iter().find(|a| a.name == name).unwrap();
        let ladder = ["ACOG 3X", "HYBRID 4X", "THERMAL 4X", "MARKSMAN 6X", "SNIPER SCOPE 8X", "VARIABLE 12X"];
        for pair in ladder.windows(2) {
            assert!(optic(pair[1]).mods.fov < optic(pair[0]).mods.fov, "{} must zoom further than {}", pair[1], pair[0]);
        }
        for long in ["SNIPER SCOPE 8X", "VARIABLE 12X"] {
            let classes = optic(long).classes;
            assert!(classes & GunClass::Sniper.bit() != 0 && classes & GunClass::Rifle.bit() != 0);
            assert!(classes & (GunClass::Pistol.bit() | GunClass::Smg.bit() | GunClass::Shotgun.bit() | GunClass::Launcher.bit()) == 0, "{long}");
        }
        assert_eq!(optic("THERMAL 4X").mods.scope, 3);
        // Every slot fits a u16 purchase mask.
        assert!(table.iter().all(|slot| slot.len() <= 16));
    }

    #[test]
    fn weight_sets_the_pace() {
        use crate::arsenal::{LOAD_FASTEST, LOAD_SLOWEST, carried_weight, load_speed};
        let guns = standard_guns();
        let table = standard_attachments();
        let kg = |i: usize| carried_weight(&guns[i], &guns[i].defaults, &table);
        let light = load_speed(kg(gun::PISTOL) + kg(gun::STINGER) + 0.75);
        let rifle = load_speed(kg(gun::CARBINE) + kg(gun::PISTOL) + 0.8);
        let lmg = load_speed(kg(gun::MINIGUN) + kg(gun::PISTOL) + 0.8);
        let launcher = load_speed(kg(gun::LAUNCHER) + kg(gun::PISTOL) + 0.8);
        let rotary = load_speed(kg(gun::ROTARY) + kg(gun::REVOLVER) + 0.8);
        assert!(light > 1.0 && light <= LOAD_FASTEST, "light kit {light}");
        assert!((0.97..=1.03).contains(&rifle), "rifle kit {rifle}");
        assert!(lmg < 0.92 && launcher < 0.93, "heavy kits {lmg} {launcher}");
        assert!(rotary >= LOAD_SLOWEST && rotary < lmg);
    }

    #[test]
    fn the_launcher_is_slow_to_reload_and_small() {
        let guns = standard_guns();
        let launcher = &guns[gun::LAUNCHER];
        assert!(launcher.reload >= 4.0 && launcher.mag <= 4 && launcher.reserve <= 4);
        assert!(launcher.splash <= 5.0 && launcher.damage <= 130.0);
    }
}
