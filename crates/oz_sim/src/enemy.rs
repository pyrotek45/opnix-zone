//! Catalog-driven enemy AI.
//!
//! Thinking (target selection) is staggered so a horde never thinks on the
//! same frame; acting (steering, attacking) is a tight per-frame loop that
//! only touches the spatial grid.

use crate::{
    combat::{Grid, Hit, Projectile, spawn_projectile},
    net::NetIds,
    player::{Health, Player},
    session::{Content, Map, MatchEntity},
    sim_dt,
    state::{FxEvent, MatchState, SimRng},
    structures::Structure,
};
use bevy::prelude::*;
use oz_core::{
    content::{Brain, Prefers, ShotLook},
    synth::Sfx,
};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Target {
    #[default]
    None,
    Objective,
    Player(Entity),
    Structure(Entity),
}

#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Enemy {
    pub kind: u16,
    pub target: Target,
    pub target_pos: Vec3,
    /// How close the surface of the target is considered "in reach".
    pub target_radius: f32,
    pub think: f32,
    pub cooldown: f32,
    /// Seconds of slow remaining.
    pub slow: f32,
    pub venom: f32,
    pub venom_time: f32,
    pub venom_by: Option<u8>,
    /// Incremented on every attack; replicated so clients can animate it.
    pub attacks: u8,
}

pub fn spawn_enemy(commands: &mut Commands, ids: &mut NetIds, content: &Content, kind: u16, at: Vec3, hp_scale: f32, rng: &mut SimRng) -> Entity {
    let def = &content.enemies[kind as usize];
    commands
        .spawn((
            MatchEntity,
            ids.next(),
            Enemy { kind, think: rng.0.range(0.0, 0.4), cooldown: rng.0.range(0.3, 1.0), ..default() },
            Health::full(def.hp * hp_scale),
            Transform::from_translation(at),
        ))
        .id()
}

/// Picks targets. Each enemy re-thinks a few times per second.
pub fn think(
    time: Res<Time>,
    map: Res<Map>,
    content: Res<Content>,
    state: Res<MatchState>,
    mut enemies: Query<(&Transform, &mut Enemy)>,
    players: Query<(Entity, &Transform, &Health), With<Player>>,
    structures: Query<(Entity, &Transform, &crate::combat::Solid), With<Structure>>,
) {
    let dt = sim_dt(&time);
    let has_objective = state.objective_max > 0.0;
    let core = map.ground(map.core.x, map.core.y);
    for (transform, mut enemy) in &mut enemies {
        enemy.think -= dt;
        if enemy.think > 0.0 {
            continue;
        }
        enemy.think = 0.45;
        let def = &content.enemies[enemy.kind as usize];
        let here = transform.translation;
        let nearest_player = players
            .iter()
            .filter(|(_, _, health)| health.alive())
            .map(|(entity, t, _)| (t.translation.distance_squared(here), entity, t.translation))
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let nearest_structure = |limit: f32| {
            structures
                .iter()
                .map(|(entity, t, solid)| (t.translation.distance(here) - solid.half.x.max(solid.half.z), entity, t.translation, solid.half.x.max(solid.half.z)))
                .filter(|(d, ..)| *d < limit)
                .min_by(|a, b| a.0.total_cmp(&b.0))
        };

        let player_target = |limit: f32| nearest_player.filter(|(d, ..)| *d < limit * limit).map(|(_, e, p)| (Target::Player(e), p, 0.4));
        let structure_target = |limit: f32| nearest_structure(limit).map(|(_, e, p, r)| (Target::Structure(e), p, r));
        let objective = has_objective.then_some((Target::Objective, core, 1.8));
        // Anything standing right in the way gets attacked regardless of preference.
        let blocking = structure_target(def.radius + 1.2);
        let choice = match def.prefers {
            Prefers::Players => player_target(70.0).or(blocking).or(objective).or(player_target(1e4)),
            Prefers::Structures => structure_target(45.0).or(objective).or(player_target(1e4)),
            Prefers::Objective => blocking.or(player_target(5.0)).or(objective).or(player_target(1e4)),
        };
        match choice {
            Some((target, pos, radius)) => {
                enemy.target = target;
                enemy.target_pos = pos;
                enemy.target_radius = radius;
            }
            None => enemy.target = Target::None,
        }
    }
}

/// Steering, separation, status effects and attacks.
#[allow(clippy::too_many_arguments)]
pub fn act(
    time: Res<Time>,
    map: Res<Map>,
    content: Res<Content>,
    grid: Res<Grid>,
    mut state: ResMut<MatchState>,
    mut rng: ResMut<SimRng>,
    mut ids: ResMut<NetIds>,
    mut commands: Commands,
    mut enemies: Query<(Entity, &mut Transform, &mut Enemy), (Without<Player>, Without<Structure>)>,
    mut players: Query<(&Transform, &mut Health, &mut Player), Without<Structure>>,
    mut structures: Query<&mut Health, (With<Structure>, Without<Player>)>,
    mut hits: EventWriter<Hit>,
    mut fx: EventWriter<FxEvent>,
) {
    let dt = sim_dt(&time);
    let elapsed = time.elapsed_secs();
    let difficulty = state.day.max(1) as f32;
    for (entity, mut transform, mut enemy) in &mut enemies {
        let def = &content.enemies[enemy.kind as usize];
        let here = transform.translation;

        if enemy.venom_time > 0.0 {
            enemy.venom_time -= dt;
            hits.write(Hit::plain(entity, enemy.venom * dt, None));
        }
        let slow = if enemy.slow > 0.0 { 0.45 } else { 1.0 };
        enemy.slow = (enemy.slow - dt).max(0.0);
        enemy.cooldown -= dt;

        // Players move, so track them every frame; other targets stay put.
        if let Target::Player(player) = enemy.target {
            match players.get(player) {
                Ok((t, health, _)) if health.alive() => enemy.target_pos = t.translation,
                _ => enemy.think = 0.0,
            }
        }
        if enemy.target == Target::None {
            continue;
        }

        let to_target = (enemy.target_pos - here).xz();
        let distance = to_target.length();
        let dir = to_target / distance.max(1e-3);
        let (stop_at, altitude) = match def.brain {
            Brain::Melee => (def.radius + enemy.target_radius + 0.5, 0.0),
            Brain::Ranged { range, .. } => (range * 0.8, 0.0),
            Brain::Flyer { altitude, range, .. } => (range * 0.7, altitude),
        };

        let mut step = Vec2::ZERO;
        if distance > stop_at {
            step += dir * def.speed * slow;
        } else if altitude > 0.0 {
            // Flyers orbit instead of hovering in place.
            step += dir.perp() * def.speed * 0.5 * slow;
        }
        // Push away from neighbours so hordes spread instead of stacking.
        let mut push = Vec2::ZERO;
        grid.for_each_near(here, def.radius + 1.5, |other| {
            if other.entity != entity {
                let away = (here - other.pos).xz();
                let gap = away.length();
                let overlap = def.radius + other.radius - gap;
                if overlap > 0.0 && gap > 1e-3 {
                    push += away / gap * overlap;
                }
            }
        });
        step += push.clamp_length_max(1.0) * 4.0;

        let mut pos = here + Vec3::new(step.x, 0.0, step.y) * dt;
        let ground = map.height(pos.x, pos.z).max(map.water - def.radius * 0.9);
        pos.y = if altitude > 0.0 {
            let hover = ground.max(map.water) + altitude + (elapsed * 1.7 + entity.index() as f32).sin() * 0.35;
            here.y + (hover - here.y) * (dt * 3.0).min(1.0)
        } else {
            ground
        };
        transform.translation = pos;
        if distance > 0.05 {
            let facing = Quat::from_rotation_y(f32::atan2(-dir.x, -dir.y));
            transform.rotation = transform.rotation.slerp(facing, (dt * 8.0).min(1.0));
        }

        // Attack.
        if distance > stop_at + 0.6 || enemy.cooldown > 0.0 {
            continue;
        }
        enemy.cooldown = def.attack_interval * rng.0.range(0.9, 1.15);
        enemy.attacks = enemy.attacks.wrapping_add(1);
        let damage = def.damage * (1.0 + 0.06 * (difficulty - 1.0));
        match def.brain {
            Brain::Melee => match enemy.target {
                Target::Objective => state.damage_objective(damage),
                Target::Player(player) => {
                    if let Ok((_, mut health, mut info)) = players.get_mut(player) {
                        crate::player::damage_player(&mut health, &mut info, damage, None, &mut state, &mut fx);
                    }
                }
                Target::Structure(structure) => match structures.get_mut(structure) {
                    Ok(mut health) => health.hp -= damage,
                    Err(_) => enemy.think = 0.0,
                },
                Target::None => {}
            },
            Brain::Ranged { shot_speed, .. } | Brain::Flyer { shot_speed, .. } => {
                let from = pos + Vec3::Y * def.radius * 1.2;
                let aim = enemy.target_pos + Vec3::Y * if enemy.target == Target::Objective { 1.5 } else { 1.1 };
                let flight = from.distance(aim) / shot_speed;
                let gravity = if altitude > 0.0 { 0.0 } else { 6.0 };
                // Solve the arc so lobbed shots land on the target.
                let velocity = (aim - from) / flight.max(0.05) + Vec3::Y * 0.5 * gravity * flight;
                let look = if altitude > 0.0 { ShotLook::Bolt } else { ShotLook::Acid };
                spawn_projectile(&mut commands, &mut ids, from, Projectile { velocity, gravity, damage, splash: 0.0, look, owner: None, team: 255, life: 5.0, effect: crate::combat::Impact::Blast });
                fx.write(FxEvent::Sound { sfx: Sfx::EnemyShot, at: Some(from) });
            }
        }
    }
}
