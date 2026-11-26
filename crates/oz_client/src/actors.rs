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
use oz_sim::prelude::*;

/// Per-match baked models, indexed like the catalog.
#[derive(Resource, Default)]
pub struct Models {
    enemies: Vec<Baked>,
    pub(crate) structures: Vec<Baked>,
    stations: Vec<Baked>,
    ore: Vec<Baked>,
    /// One operator model per team colour.
    avatars: Vec<Baked>,
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

fn avatar_model(team: Paint) -> BlockModel {
    BlockModel::new()
        .pair(Shape::Cube, Paint::Dark, [0.16, 0.42, 0.0], [0.2, 0.84, 0.24], Some(0.0))
        .cube(team, [0.0, 1.15, 0.0], [0.56, 0.66, 0.32])
        .cube(Paint::Gold, [0.0, 1.2, -0.17], [0.3, 0.16, 0.03])
        .cube(Paint::Bone, [0.0, 1.66, 0.0], [0.3, 0.3, 0.3])
        .cube(Paint::Blue, [0.0, 1.68, -0.155], [0.24, 0.1, 0.02])
        .pair(Shape::Cube, Paint::Dark, [0.36, 1.2, -0.12], [0.14, 0.5, 0.4], None)
        .cube(Paint::Metal, [0.2, 1.22, -0.5], [0.1, 0.12, 0.7])
}

fn bake_models(content: Res<Content>, art: Res<Art>, mut meshes: ResMut<Assets<Mesh>>, mut models: ResMut<Models>) {
    *models = Models {
        enemies: content.enemies.iter().map(|e| art.bake(&e.model, &mut meshes)).collect(),
        structures: content.structures.iter().map(|s| art.bake(&s.model, &mut meshes)).collect(),
        stations: content.stations.iter().map(|s| art.bake(&s.model, &mut meshes)).collect(),
        ore: Ore::ALL.into_iter().map(|ore| art.bake(&ore_model(ore), &mut meshes)).collect(),
        avatars: [Paint::Teal, Paint::Coral].into_iter().map(|team| art.bake(&avatar_model(team), &mut meshes)).collect(),
    };
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
    avatars: Query<(Entity, &Transform, &Player), (Added<Player>, Without<LocalPlayer>)>,
) {
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
        commands.entity(entity).insert(Visibility::Inherited).with_children(|parent| {
            parent.spawn((
                Mesh3d(art.primitive(Shape::Shard)),
                MeshMaterial3d(art.paint(shot.look.paint())),
                Transform::from_scale(Vec3::new(0.16, 0.16, 0.34)),
                NotShadowCaster,
            ));
        });
    }
    for (entity, transform, player) in &avatars {
        art.spawn(&mut commands, entity, &models.avatars[(player.team as usize).min(1)]);
        commands.entity(entity).insert(Motion { last: transform.translation, stride: 0.22, ..default() });
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
            Anim::None | Anim::Leg { .. } => {}
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
