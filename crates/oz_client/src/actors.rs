//! Visuals for simulated entities.
//!
//! The simulation spawns bare gameplay components; this module notices them
//! (`Added<T>`) and attaches baked models. Because clients reconstruct the
//! same components from snapshots, the code is identical for host and client.

use crate::{
    ClientSet,
    art::{AnimPart, Art, Baked, LegGroup},
};
use bevy::{pbr::NotShadowCaster, prelude::*};
use oz_core::content::ShotLook;
use oz_sim::prelude::*;

/// Per-match baked models, indexed like the catalog.
#[derive(Resource, Default)]
pub struct Models {
    enemies: Vec<Baked>,
    pub(crate) structures: Vec<Baked>,
    stations: Vec<Baked>,
    ore: Vec<Baked>,
}

/// Animation state on a model root.
#[derive(Component, Default)]
pub struct Motion {
    last: Vec3,
    /// Walk cycle position.
    phase: f32,
    stride: f32,
    /// 1 right after an attack or shot, decaying to 0.
    kick: f32,
    counter: u8,
}

pub struct ActorsPlugin;

impl Plugin for ActorsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Models>()
            .add_systems(OnEnter(MatchPhase::Active), bake_models)
            .add_systems(Update, (attach, track_motion, animate_legs, animate_parts, size_ore).chain().in_set(ClientSet::Present).run_if(in_state(MatchPhase::Active)));
    }
}

fn ore_model(ore: Ore) -> BlockModel {
    let (paint, glow) = match ore {
        Ore::Iron => (Paint::Iron, Paint::Spark),
        Ore::Copper => (Paint::Copper, Paint::Glow),
        Ore::Aether => (Paint::Teal, Paint::Aether),
    };
    BlockModel::new()
        .shard(Paint::Rock, [0.0, 0.3, 0.0], [0.95, 0.5, 0.85])
        .shard(paint, [0.25, 0.75, 0.1], [0.4, 0.55, 0.38])
        .shard(paint, [-0.35, 0.6, -0.2], [0.32, 0.42, 0.3])
        .shard(paint, [0.0, 0.55, 0.42], [0.28, 0.36, 0.26])
        .shard(glow, [0.28, 1.2, 0.1], [0.12, 0.2, 0.12])
        .shard(glow, [-0.36, 0.95, -0.2], [0.09, 0.15, 0.09])
}

fn bake_models(content: Res<Content>, art: Res<Art>, mut meshes: ResMut<Assets<Mesh>>, mut models: ResMut<Models>) {
    *models = Models {
        enemies: content.enemies.iter().map(|e| art.bake(&e.model, &mut meshes)).collect(),
        structures: content.structures.iter().map(|s| art.bake(&s.model, &mut meshes)).collect(),
        stations: content.stations.iter().map(|s| art.bake(&s.model, &mut meshes)).collect(),
        ore: Ore::ALL.into_iter().map(|ore| art.bake(&ore_model(ore), &mut meshes)).collect(),
    };
}

/// Killstreak aircraft and their payloads, nose toward -Z, in the owner's team colours.
fn air_model(look: ShotLook, team: u8) -> BlockModel {
    let paint = crate::avatar::team_paint(team.min(2));
    let light = crate::avatar::team_light(team.min(2));
    match look {
        // A swept-wing attack jet: long nose, delta wings, twin tails, hot exhaust.
        ShotLook::Jet => BlockModel::new()
            .cube(Paint::Dark, [0.0, 0.0, 0.0], [1.3, 1.1, 8.5])
            .shard(Paint::Dark, [0.0, 0.0, -5.0], [0.9, 0.8, 2.2])
            .cube(Paint::Glow, [0.0, 0.45, -2.6], [0.6, 0.35, 1.6])
            .cube(paint, [-2.6, -0.1, 1.2], [4.2, 0.18, 2.4])
            .rotated([0.0, -28.0, 0.0])
            .cube(paint, [2.6, -0.1, 1.2], [4.2, 0.18, 2.4])
            .rotated([0.0, 28.0, 0.0])
            .cube(paint, [-0.7, 1.1, 3.6], [0.15, 1.6, 1.5])
            .rotated([0.0, 0.0, -14.0])
            .cube(paint, [0.7, 1.1, 3.6], [0.15, 1.6, 1.5])
            .rotated([0.0, 0.0, 14.0])
            .cube(Paint::Steel, [0.0, -0.1, 3.8], [3.4, 0.14, 1.0])
            .cylinder(Paint::Spark, [0.0, 0.0, 4.4], [0.55, 0.3, 0.55])
            .rotated([90.0, 0.0, 0.0])
            .cube(light, [-4.4, -0.05, 2.2], [0.3, 0.2, 0.3])
            .cube(light, [4.4, -0.05, 2.2], [0.3, 0.2, 0.3]),
        // A slow, straight-winged radar plane with a big dish on its back.
        ShotLook::RadarPlane => BlockModel::new()
            .cube(Paint::Steel, [0.0, 0.0, 0.0], [1.5, 1.4, 9.0])
            .shard(Paint::Steel, [0.0, 0.0, -5.0], [1.1, 1.0, 1.6])
            .cube(paint, [0.0, 0.0, -0.5], [15.0, 0.22, 1.8])
            .cube(paint, [0.0, 1.0, 4.0], [0.2, 2.0, 1.6])
            .cube(Paint::Steel, [0.0, 0.2, 4.2], [4.6, 0.16, 1.1])
            .cylinder(Paint::Dark, [0.0, 1.7, 0.6], [4.0, 0.45, 4.0])
            .cylinder(light, [0.0, 1.95, 0.6], [3.2, 0.06, 3.2])
            .cube(Paint::Dark, [0.0, 1.05, 0.6], [0.3, 0.8, 0.3])
            .cylinder(Paint::Metal, [-3.8, 0.0, -1.7], [1.6, 0.1, 1.6])
            .rotated([90.0, 0.0, 0.0])
            .cylinder(Paint::Metal, [3.8, 0.0, -1.7], [1.6, 0.1, 1.6])
            .rotated([90.0, 0.0, 0.0])
            .cube(Paint::Dark, [-3.8, 0.0, -0.9], [0.6, 0.6, 1.6])
            .cube(Paint::Dark, [3.8, 0.0, -0.9], [0.6, 0.6, 1.6]),
        // A quadcopter: a body, four arms and rotor discs, a red eye.
        ShotLook::Drone => {
            let mut m = BlockModel::new().cube(paint, [0.0, 0.0, 0.0], [0.6, 0.22, 0.7]).shard(Paint::Red, [0.0, -0.05, -0.38], [0.14, 0.14, 0.1]).cube(Paint::Dark, [0.0, -0.2, 0.0], [0.18, 0.18, 0.4]);
            for (x, z) in [(-0.55, -0.55), (0.55, -0.55), (-0.55, 0.55), (0.55, 0.55)] {
                m = m.segment(Paint::Dark, [0.0, 0.05, 0.0], [x, 0.1, z], 0.06).cylinder(Paint::Steel, [x, 0.16, z], [0.5, 0.03, 0.5]).cylinder(light, [x, 0.12, z], [0.1, 0.06, 0.1]);
            }
            m
        }
        // A finned bomb, nose first.
        ShotLook::Bomb => BlockModel::new()
            .cylinder(Paint::Iron, [0.0, 0.0, 0.0], [0.42, 1.4, 0.42])
            .rotated([90.0, 0.0, 0.0])
            .shard(Paint::Iron, [0.0, 0.0, -0.85], [0.36, 0.36, 0.4])
            .cube(Paint::Dark, [0.0, 0.0, 0.85], [0.8, 0.06, 0.4])
            .cube(Paint::Dark, [0.0, 0.0, 0.85], [0.06, 0.8, 0.4])
            .cube(Paint::Gold, [0.0, 0.0, -0.3], [0.44, 0.44, 0.08]),
        // An artillery shell with a glowing tail.
        _ => BlockModel::new()
            .cylinder(Paint::Gold, [0.0, 0.0, 0.0], [0.24, 0.9, 0.24])
            .rotated([90.0, 0.0, 0.0])
            .shard(Paint::Iron, [0.0, 0.0, -0.55], [0.22, 0.22, 0.3])
            .shard(Paint::Spark, [0.0, 0.0, 0.6], [0.2, 0.2, 0.4]),
    }
}

#[allow(clippy::too_many_arguments)]
fn attach(
    mut commands: Commands,
    art: Res<Art>,
    models: Res<Models>,
    content: Res<Content>,
    enemies: Query<(Entity, &Enemy, &Transform), Added<Enemy>>,
    structures: Query<(Entity, &Structure), Added<Structure>>,
    stations: Query<(Entity, &Station), Added<Station>>,
    ores: Query<(Entity, &OreNode), Added<OreNode>>,
    shots: Query<(Entity, &Projectile), Added<Projectile>>,
    mines: Query<Entity, Added<oz_sim::combat::Mine>>,
    (aircraft, mut meshes, mut air): (Query<(Entity, &oz_sim::killstreak::Aircraft), (Added<oz_sim::killstreak::Aircraft>, Without<Projectile>)>, ResMut<Assets<Mesh>>, Local<bevy::platform::collections::HashMap<(u8, u8), crate::art::Baked>>),
) {
    // Aircraft and bombs get real models (the host's aircraft are not projectiles; clients' are).
    let mut air_model_for = |commands: &mut Commands, entity: Entity, look: ShotLook, team: u8| {
        let baked = air.entry((look as u8, team)).or_insert_with(|| art.bake(&air_model(look, team), &mut meshes)).clone();
        commands.entity(entity).insert(Visibility::Inherited);
        art.spawn(commands, entity, &baked);
    };
    for (entity, craft) in &aircraft {
        air_model_for(&mut commands, entity, craft.look, craft.team);
    }
    for entity in &mines {
        let look = oz_core::content::ShotLook::Mine;
        commands.entity(entity).insert(Visibility::Inherited).with_children(|parent| {
            parent.spawn((Mesh3d(art.primitive(Shape::Shard)), MeshMaterial3d(art.paint(look.paint())), Transform::from_scale(Vec3::from_array(look.scale())), NotShadowCaster));
        });
    }
    for (entity, enemy, transform) in &enemies {
        let Some(baked) = models.enemies.get(enemy.kind as usize) else { continue };
        art.spawn(&mut commands, entity, baked);
        let stride = content.enemies[enemy.kind as usize].radius * 0.3;
        commands.entity(entity).insert(Motion { last: transform.translation, stride, counter: enemy.attacks, ..default() });
    }
    for (entity, structure) in &structures {
        if let Some(baked) = models.structures.get(structure.kind as usize) {
            art.spawn(&mut commands, entity, baked);
            commands.entity(entity).insert(Motion { counter: structure.shots, ..default() });
        }
    }
    for (entity, station) in &stations {
        if let Some(baked) = models.stations.get(station.kind as usize) {
            art.spawn(&mut commands, entity, baked);
        }
    }
    for (entity, node) in &ores {
        art.spawn(&mut commands, entity, &models.ore[node.ore as usize]);
    }
    for (entity, shot) in &shots {
        if shot.look.is_air() {
            air_model_for(&mut commands, entity, shot.look, shot.team);
            continue;
        }
        commands.entity(entity).insert(Visibility::Inherited).with_children(|parent| {
            parent.spawn((
                Mesh3d(art.primitive(Shape::Shard)),
                MeshMaterial3d(art.paint(shot.look.paint())),
                Transform::from_scale(Vec3::from_array(shot.look.scale())),
                NotShadowCaster,
            ));
        });
    }
}

fn track_motion(time: Res<Time>, mut roots: Query<(&Transform, &mut Motion, Option<&Enemy>, Option<&Structure>)>) {
    let dt = time.delta_secs().max(1e-4);
    for (transform, mut motion, enemy, structure) in &mut roots {
        let moved = (transform.translation - motion.last).xz().length();
        motion.last = transform.translation;
        motion.phase += moved / (motion.stride.max(0.05) * 9.0);
        motion.kick = (motion.kick - dt * 5.0).max(0.0);
        let counter = enemy.map(|e| e.attacks).or(structure.map(|s| s.shots)).unwrap_or(0);
        if counter != motion.counter {
            motion.counter = counter;
            motion.kick = 1.0;
        }
    }
}

/// Each leg group swings as a unit: forward and up, then back along the ground.
fn animate_legs(roots: Query<&Motion>, mut legs: Query<(&ChildOf, &LegGroup, &mut Transform)>) {
    for (child_of, group, mut transform) in &mut legs {
        let Ok(motion) = roots.get(child_of.parent()) else { continue };
        let cycle = motion.phase * std::f32::consts::TAU + group.0 as f32 * std::f32::consts::PI;
        let target = Vec3::new(0.0, cycle.cos().max(0.0) * motion.stride * 0.6, cycle.sin() * motion.stride);
        if transform.translation.distance_squared(target) > 1e-6 {
            transform.translation = target;
        }
    }
}

fn animate_parts(time: Res<Time>, roots: Query<&Motion>, mut parts: Query<(&ChildOf, &AnimPart, &mut Transform)>) {
    let t = time.elapsed_secs();
    for (child_of, part, mut transform) in &mut parts {
        match part.anim {
            Anim::Spin { speed } => transform.rotation = part.base.rotation * Quat::from_rotation_y(t * speed),
            Anim::Pulse => transform.scale = part.base.scale * (1.0 + 0.14 * (t * 4.0).sin()),
            Anim::Bob => transform.translation.y = part.base.translation.y + (t * 2.0).sin() * 0.12,
            Anim::Recoil => {
                let kick = roots.get(child_of.parent()).map_or(0.0, |motion| motion.kick);
                transform.translation = part.base.translation + Vec3::Z * 0.22 * kick;
            }
            Anim::None | Anim::Leg { .. } | Anim::Mag | Anim::Slide => {}
        }
    }
}

/// Deposits visibly shrink as they are mined out.
fn size_ore(mut ores: Query<(&OreNode, &mut Transform), Changed<OreNode>>) {
    for (node, mut transform) in &mut ores {
        let left = (node.amount / node.max.max(1e-3)).clamp(0.0, 1.0);
        transform.scale = Vec3::splat(0.45 + 0.55 * left);
    }
}
