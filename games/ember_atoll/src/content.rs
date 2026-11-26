//! Ember Atoll's content: every weapon, enemy, structure, upgrade and station.
//!
//! This file is the whole "art and balance" surface of the game. Models are
//! block models built in code; there are no asset files.

use oz_sim::prelude::*;
use bevy::math::Vec3;

/// Catalog weapon slots, in order.
pub mod weapon {
    pub const MINING_LASER: usize = 0;
    pub const PULSE_PISTOL: usize = 1;
    pub const REEF_SMG: usize = 2;
}

/// `PlayerCommand::Mode` actions understood by the siege director.
pub mod action {
    pub const CALL_NIGHTFALL: u8 = 0;
    pub const REPAIR_BEACON: u8 = 1;
    pub const REPAIR_AMOUNT: f32 = 150.0;
}

pub fn catalog(rules: &Rules) -> Catalog {
    let mut catalog = Catalog {
        weapons: weapons(),
        attachments: standard_attachments(),
        mods: ModKind::ALL.to_vec(),
        enemies: enemies(),
        structures: structures(),
        upgrades: upgrades(),
        stations: stations(rules),
        mode_actions: vec![
            ModeAction { name: "CALL NIGHTFALL", blurb: "End the day now and face the next wave", cost: Cost::FREE },
            ModeAction { name: "REPAIR BEACON", blurb: "Restore 150 beacon integrity", cost: Cost::new(80, 10, 0, 0) },
        ],
        objective_model: beacon(),
    };
    if !rules.mining {
        // No ore in this mode: fold every ore price into credits.
        let flatten = |cost: &mut Cost| {
            cost.credits += cost.ore.iter().sum::<u32>() * 4;
            cost.ore = [0; 3];
        };
        catalog.weapons.iter_mut().for_each(|w| flatten(&mut w.price));
        catalog.structures.iter_mut().for_each(|s| flatten(&mut s.cost));
        catalog.upgrades.iter_mut().for_each(|u| flatten(&mut u.cost));
        catalog.mode_actions.iter_mut().for_each(|a| flatten(&mut a.cost));
        catalog.structures.retain(|s| !matches!(s.kind, StructureKind::Drill { .. }));
        catalog.upgrades.retain(|u| !matches!(u.effect, UpgradeEffect::DrillRate | UpgradeEffect::MiningPower));
        if rules.objective_hp <= 0.0 {
            catalog.upgrades.retain(|u| u.effect != UpgradeEffect::ObjectiveHull);
            catalog.mode_actions.truncate(1);
        }
    }
    catalog
}

/// Shared gun silhouette: receiver, barrel, grip, stock and iron sights.
fn gun(body: f32, length: f32, accent: Paint) -> BlockModel {
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

fn weapons() -> Vec<WeaponDef> {
    let base = WeaponDef {
        name: "", blurb: "", price: Cost::FREE, mag: 12, reserve: 0, infinite_reserve: false, ammo_price: 0, damage: 20.0, interval: 0.2,
        reload: 1.0, pellets: 1, spread: 0.01, range: 60.0, splash: 0.0, recoil: 0.6, auto: false, mode: FireMode::Hitscan, ads_fov: 55.0,
        mining: 0.0, length: 0.5, sound: 0, model: BlockModel::new(),
    };
    vec![
        WeaponDef {
            name: "MINING LASER", blurb: "Cuts ore from deposits. A poor weapon, but it never runs dry.",
            mag: 0, infinite_reserve: true, damage: 5.0, interval: 0.1, spread: 0.0, range: 7.5, recoil: 0.05, auto: true, mode: FireMode::Beam,
            ads_fov: 62.0, mining: 14.0, length: 0.52, sound: 6,
            model: BlockModel::new()
                .cube(Paint::Dark, [0.0, 0.0, -0.10], [0.20, 0.20, 0.40])
                .cube(Paint::Gold, [0.0, 0.06, -0.10], [0.225, 0.10, 0.30])
                .cylinder(Paint::Metal, [0.0, 0.02, -0.36], [0.11, 0.30, 0.11]).rotated([90.0, 0.0, 0.0])
                .cone(Paint::Aether, [0.0, 0.02, -0.56], [0.09, 0.12, 0.09]).rotated([-90.0, 0.0, 0.0])
                .cube(Paint::Aether, [0.11, 0.03, -0.12], [0.012, 0.05, 0.22])
                .cube(Paint::Dark, [0.0, -0.14, 0.04], [0.10, 0.22, 0.12])
                .cube(Paint::Wood, [0.0, -0.10, 0.16], [0.16, 0.16, 0.20]),
            ..base.clone()
        },
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
            ..base
        },
    ]
}

fn enemies() -> Vec<EnemyDef> {
    vec![
        EnemyDef {
            name: "SKITTER", hp: 55.0, speed: 4.6, radius: 0.55, damage: 8.0, attack_interval: 0.9, reward: 12, threat: 1, first_wave: 1,
            brain: Brain::Melee, prefers: Prefers::Objective, boss: false, gibs: Paint::Shell,
            model: BlockModel::new()
                .shard(Paint::Shell, [0.0, 0.5, 0.0], [0.5, 0.26, 0.46])
                .cube(Paint::Steel, [0.0, 0.34, 0.0], [0.7, 0.14, 0.6])
                .cube(Paint::Red, [0.0, 0.55, -0.42], [0.3, 0.1, 0.08])
                .pair(Shape::Cube, Paint::Coral, [0.42, 0.4, -0.58], [0.16, 0.16, 0.42], Some(0.25))
                .pair(Shape::Cube, Paint::Steel, [0.58, 0.2, -0.22], [0.5, 0.09, 0.09], Some(0.0))
                .pair(Shape::Cube, Paint::Steel, [0.6, 0.2, 0.05], [0.55, 0.09, 0.09], Some(0.5))
                .pair(Shape::Cube, Paint::Steel, [0.56, 0.2, 0.3], [0.5, 0.09, 0.09], Some(0.0)),
        },
        EnemyDef {
            name: "REEF CRAWLER", hp: 150.0, speed: 2.9, radius: 0.8, damage: 16.0, attack_interval: 1.1, reward: 22, threat: 2, first_wave: 2,
            brain: Brain::Melee, prefers: Prefers::Players, boss: false, gibs: Paint::Steel,
            model: BlockModel::new()
                .cube(Paint::Steel, [0.0, 0.62, 0.0], [1.0, 0.6, 1.7])
                .shard(Paint::Shell, [0.0, 0.98, -0.45], [0.52, 0.3, 0.42])
                .shard(Paint::Shell, [0.0, 1.02, 0.1], [0.56, 0.34, 0.42])
                .shard(Paint::Shell, [0.0, 0.94, 0.62], [0.46, 0.28, 0.38])
                .cube(Paint::Dark, [0.0, 0.6, -0.98], [0.7, 0.42, 0.3])
                .cube(Paint::Blue, [0.0, 0.7, -1.14], [0.5, 0.1, 0.05])
                .cone(Paint::Teal, [0.0, 0.75, 1.1], [0.3, 0.8, 0.12]).rotated([-70.0, 0.0, 0.0])
                .pair(Shape::Cube, Paint::Dark, [0.62, 0.3, -0.5], [0.24, 0.6, 0.24], Some(0.0))
                .pair(Shape::Cube, Paint::Dark, [0.62, 0.3, 0.5], [0.24, 0.6, 0.24], Some(0.5)),
        },
        EnemyDef {
            name: "ACID SPITTER", hp: 90.0, speed: 3.0, radius: 0.65, damage: 12.0, attack_interval: 2.2, reward: 20, threat: 2, first_wave: 2,
            brain: Brain::Ranged { range: 24.0, shot_speed: 17.0 }, prefers: Prefers::Players, boss: false, gibs: Paint::Purple,
            model: BlockModel::new()
                .shard(Paint::Purple, [0.0, 0.95, 0.1], [0.48, 0.52, 0.55])
                .shard(Paint::Green, [0.0, 1.3, 0.38], [0.3, 0.3, 0.3]).anim(Anim::Pulse)
                .cylinder(Paint::Dark, [0.0, 1.0, -0.55], [0.2, 0.6, 0.2]).rotated([90.0, 0.0, 0.0]).anim(Anim::Recoil)
                .cube(Paint::Green, [0.0, 1.0, -0.87], [0.12, 0.12, 0.04])
                .pair(Shape::Cube, Paint::Steel, [0.5, 0.42, -0.2], [0.1, 0.84, 0.1], Some(0.0))
                .pair(Shape::Cube, Paint::Steel, [0.5, 0.42, 0.4], [0.1, 0.84, 0.1], Some(0.5)),
        },
        EnemyDef {
            name: "MANTA DRONE", hp: 70.0, speed: 6.2, radius: 0.7, damage: 9.0, attack_interval: 1.6, reward: 20, threat: 2, first_wave: 3,
            brain: Brain::Flyer { altitude: 5.0, range: 20.0, shot_speed: 28.0 }, prefers: Prefers::Players, boss: false, gibs: Paint::Metal,
            model: BlockModel::new()
                .cube(Paint::Dark, [0.0, 0.7, 0.05], [2.3, 0.07, 0.9])
                .shard(Paint::Metal, [0.0, 0.7, 0.0], [0.36, 0.22, 0.56])
                .cube(Paint::Red, [0.0, 0.7, -0.52], [0.26, 0.08, 0.06])
                .cube(Paint::Steel, [0.0, 0.7, 0.9], [0.07, 0.07, 1.0])
                .pair(Shape::Cylinder, Paint::Blue, [0.75, 0.6, 0.05], [0.4, 0.04, 0.4], None)
                .cylinder(Paint::Blue, [0.0, 0.5, 0.0], [0.5, 0.03, 0.5]).anim(Anim::Spin { speed: 14.0 }),
        },
        EnemyDef {
            name: "SHELL BRUTE", hp: 520.0, speed: 2.0, radius: 1.25, damage: 45.0, attack_interval: 1.6, reward: 70, threat: 6, first_wave: 4,
            brain: Brain::Melee, prefers: Prefers::Structures, boss: false, gibs: Paint::Shell,
            model: BlockModel::new()
                .shard(Paint::Shell, [0.0, 1.35, 0.1], [1.15, 0.9, 1.25])
                .cube(Paint::Steel, [0.0, 0.8, 0.0], [1.7, 0.5, 2.0])
                .cube(Paint::Dark, [0.0, 0.95, -1.1], [0.9, 0.6, 0.5])
                .pair(Shape::Cube, Paint::Red, [0.22, 1.05, -1.37], [0.14, 0.1, 0.05], None)
                .pair(Shape::Cone, Paint::Bone, [0.55, 2.2, -0.2], [0.26, 0.7, 0.26], None)
                .pair(Shape::Cone, Paint::Bone, [0.5, 2.1, 0.6], [0.22, 0.55, 0.22], None)
                .pair(Shape::Cube, Paint::Coral, [1.0, 0.75, -0.9], [0.4, 0.4, 0.9], Some(0.25))
                .pair(Shape::Cube, Paint::Dark, [0.8, 0.4, -0.4], [0.45, 0.8, 0.45], Some(0.0))
                .pair(Shape::Cube, Paint::Dark, [0.8, 0.4, 0.7], [0.45, 0.8, 0.45], Some(0.5)),
        },
        EnemyDef {
            name: "LEVIATHAN WAR ENGINE", hp: 4200.0, speed: 1.7, radius: 2.5, damage: 90.0, attack_interval: 1.8, reward: 600, threat: 40, first_wave: 5,
            brain: Brain::Melee, prefers: Prefers::Objective, boss: true, gibs: Paint::Steel,
            model: BlockModel::new()
                .cube(Paint::Steel, [0.0, 2.4, 0.0], [3.4, 2.0, 5.2])
                .shard(Paint::Shell, [0.0, 3.5, -1.0], [1.6, 0.9, 1.5])
                .shard(Paint::Shell, [0.0, 3.6, 1.1], [1.7, 1.0, 1.6])
                .cube(Paint::Dark, [0.0, 2.3, -3.0], [2.4, 1.5, 1.0])
                .cube(Paint::Red, [0.0, 2.55, -3.52], [1.7, 0.22, 0.06])
                .shard(Paint::Aether, [0.0, 4.3, 0.1], [0.7, 0.9, 0.7]).anim(Anim::Pulse)
                .pair(Shape::Cone, Paint::Teal, [1.1, 4.3, -1.6], [0.5, 1.6, 0.2], None)
                .pair(Shape::Cone, Paint::Teal, [1.2, 4.4, 1.8], [0.5, 1.8, 0.2], None)
                .pair(Shape::Cube, Paint::Coral, [2.2, 2.0, -2.6], [0.8, 0.8, 2.2], Some(0.25))
                .pair(Shape::Cube, Paint::Dark, [1.9, 0.75, -1.8], [0.8, 1.5, 0.8], Some(0.0))
                .pair(Shape::Cube, Paint::Dark, [2.0, 0.75, 0.0], [0.8, 1.5, 0.8], Some(0.5))
                .pair(Shape::Cube, Paint::Dark, [1.9, 0.75, 1.8], [0.8, 1.5, 0.8], Some(0.0)),
        },
    ]
}

fn structures() -> Vec<StructureDef> {
    vec![
        StructureDef {
            name: "GUN TURRET", blurb: "Automatic defense. Engages anything within 25 m.",
            cost: Cost::new(120, 15, 0, 0), hp: 160.0, half: Vec3::new(0.6, 0.8, 0.6),
            kind: StructureKind::Turret { range: 25.0, damage: 14.0, interval: 0.22 },
            model: BlockModel::new()
                .cube(Paint::Dark, [0.0, 0.2, 0.0], [1.1, 0.4, 1.1])
                .cube(Paint::Metal, [0.0, 0.75, 0.0], [0.4, 0.7, 0.4])
                .cube(Paint::Teal, [0.0, 1.3, 0.0], [0.7, 0.42, 0.8])
                .cube(Paint::Glow, [0.0, 1.3, -0.41], [0.3, 0.1, 0.03])
                .pair(Shape::Cube, Paint::Metal, [0.17, 1.3, -0.8], [0.12, 0.12, 0.9], None),
        },
        StructureDef {
            name: "AUTO-DRILL", blurb: "Place on an ore deposit. Mines it for you, day and night.",
            cost: Cost::new(90, 25, 0, 0), hp: 130.0, half: Vec3::new(0.9, 1.3, 0.9),
            kind: StructureKind::Drill { rate: 2.2 },
            model: BlockModel::new()
                .pair(Shape::Cube, Paint::Steel, [0.8, 1.1, -0.8], [0.16, 2.2, 0.16], None)
                .pair(Shape::Cube, Paint::Steel, [0.8, 1.1, 0.8], [0.16, 2.2, 0.16], None)
                .cube(Paint::Gold, [0.0, 2.3, 0.0], [1.9, 0.4, 1.9])
                .cube(Paint::Dark, [0.0, 2.65, 0.0], [0.9, 0.35, 0.9])
                .cube(Paint::Glow, [0.0, 2.3, -0.96], [0.6, 0.12, 0.03])
                .cylinder(Paint::Metal, [0.0, 1.7, 0.0], [0.34, 0.9, 0.34]).anim(Anim::Spin { speed: 9.0 })
                .cone(Paint::Metal, [0.0, 0.95, 0.0], [0.5, 0.7, 0.5]).rotated([180.0, 0.0, 0.0]).anim(Anim::Spin { speed: 9.0 }),
        },
        StructureDef {
            name: "BARRICADE", blurb: "Cheap cover. Attackers must chew through it.",
            cost: Cost::new(30, 8, 0, 0), hp: 320.0, half: Vec3::new(1.4, 0.75, 0.38),
            kind: StructureKind::Wall,
            model: BlockModel::new().cube(Paint::Concrete, [0.0, 0.7, 0.0], [2.8, 1.4, 0.7]).cube(Paint::Rust, [0.0, 1.45, 0.0], [2.9, 0.12, 0.8]),
        },
        StructureDef {
            name: "ARC TOWER", blurb: "Slow, heavy lightning strikes. Needs aether.",
            cost: Cost::new(200, 0, 20, 4), hp: 140.0, half: Vec3::new(0.6, 1.6, 0.6),
            kind: StructureKind::Turret { range: 17.0, damage: 70.0, interval: 0.95 },
            model: BlockModel::new()
                .cube(Paint::Dark, [0.0, 0.2, 0.0], [1.1, 0.4, 1.1])
                .cylinder(Paint::Steel, [0.0, 1.4, 0.0], [0.3, 2.2, 0.3])
                .cylinder(Paint::Blue, [0.0, 1.5, 0.0], [0.7, 0.07, 0.7])
                .cylinder(Paint::Blue, [0.0, 2.0, 0.0], [0.56, 0.07, 0.56])
                .shard(Paint::Blue, [0.0, 2.85, 0.0], [0.34, 0.44, 0.34]).anim(Anim::Pulse),
        },
    ]
}

fn upgrades() -> Vec<UpgradeDef> {
    vec![
        UpgradeDef { name: "LASER FOCUS", blurb: "+50% hand mining speed", cost: Cost::new(100, 15, 0, 0), max_level: 4, effect: UpgradeEffect::MiningPower, per_level: 0.5 },
        UpgradeDef { name: "DRILL SERVOS", blurb: "+50% auto-drill output", cost: Cost::new(120, 0, 20, 0), max_level: 4, effect: UpgradeEffect::DrillRate, per_level: 0.5 },
        UpgradeDef { name: "BEACON PLATING", blurb: "+30% beacon integrity", cost: Cost::new(150, 30, 0, 0), max_level: 5, effect: UpgradeEffect::ObjectiveHull, per_level: 0.3 },
        UpgradeDef { name: "TURRET CALIBRATION", blurb: "+25% turret damage", cost: Cost::new(140, 0, 15, 2), max_level: 4, effect: UpgradeEffect::TurretDamage, per_level: 0.25 },
        UpgradeDef { name: "FIELD ARMOR", blurb: "+25% health for every operator", cost: Cost::new(130, 20, 0, 0), max_level: 4, effect: UpgradeEffect::PlayerArmor, per_level: 0.25 },
    ]
}

fn station(accent: Paint) -> BlockModel {
    BlockModel::new()
        .cube(Paint::Dark, [0.0, 0.2, 0.0], [1.5, 0.4, 1.5])
        .cube(Paint::Steel, [0.0, 1.0, 0.1], [1.3, 1.3, 0.9])
        .cube(Paint::Teal, [0.0, 1.72, 0.1], [1.4, 0.14, 1.0])
        .cube(accent, [0.0, 1.2, -0.37], [0.95, 0.6, 0.04])
        .cube(Paint::Metal, [0.0, 0.72, -0.46], [1.0, 0.08, 0.3]).rotated([-25.0, 0.0, 0.0])
        .cube(Paint::Metal, [0.55, 2.3, 0.3], [0.05, 1.1, 0.05])
        .shard(accent, [0.55, 2.9, 0.3], [0.09, 0.09, 0.09]).anim(Anim::Pulse)
}

fn stations(rules: &Rules) -> Vec<StationDef> {
    let mut stations = vec![
        StationDef {
            name: "ARMORY", blurb: "Weapons, ammunition, attachments and mods", accent: Paint::Gold,
            pages: vec![StationPage::Weapons, StationPage::Attachments, StationPage::Mods], model: station(Paint::Glow),
        },
        StationDef {
            name: "ENGINEERING", blurb: "Structures and team upgrades", accent: Paint::Green,
            pages: vec![StationPage::Build, StationPage::Upgrades], model: station(Paint::Green),
        },
    ];
    if rules.day_seconds > 0.0 || rules.objective_hp > 0.0 {
        stations.push(StationDef { name: "BEACON CONTROL", blurb: "Mission status and beacon systems", accent: Paint::Blue, pages: vec![StationPage::Mission], model: station(Paint::Blue) });
    }
    stations
}

fn beacon() -> BlockModel {
    BlockModel::new()
        .cube(Paint::Dark, [0.0, 0.2, 0.0], [3.6, 0.4, 3.6])
        .cube(Paint::Metal, [0.0, 0.65, 0.0], [2.0, 0.7, 2.0])
        .pair(Shape::Cube, Paint::Gold, [1.25, 0.7, 1.25], [0.32, 1.1, 0.32], None)
        .pair(Shape::Cube, Paint::Gold, [1.25, 0.7, -1.25], [0.32, 1.1, 0.32], None)
        .shard(Paint::Blue, [0.0, 2.0, 0.0], [0.6, 0.9, 0.6]).anim(Anim::Spin { speed: 0.8 })
}
