//! Hits, splash damage, projectiles, collision solids and the spatial grid.

use crate::{
    enemy::Enemy,
    player::{Health, Player},
    session::{Content, Map, MatchEntity},
    sim_dt,
    state::{FxEvent, MatchState},
    structures::Structure,
};
use bevy::prelude::*;
use oz_core::{content::ShotLook, synth::Sfx};

/// Axis-aligned collision box standing on the entity's origin (its base sits at `translation.y`).
#[derive(Component, Clone, Copy, Debug)]
pub struct Solid {
    pub half: Vec3,
}

impl Solid {
    pub fn center(&self, transform: &Transform) -> Vec3 {
        transform.translation + Vec3::Y * self.half.y
    }
}

/// Damage request. Everything that hurts an enemy goes through here so that
/// rewards, mods and death effects live in exactly one place.
#[derive(Event, Clone, Copy, Debug)]
pub struct Hit {
    pub target: Entity,
    pub amount: f32,
    /// Player to credit, if any.
    pub by: Option<u8>,
    pub frost: f32,
    pub venom: f32,
    pub arc: f32,
    pub scavenge: f32,
    pub repair: f32,
}

impl Hit {
    pub fn plain(target: Entity, amount: f32, by: Option<u8>) -> Self {
        Self { target, amount, by, frost: 0.0, venom: 0.0, arc: 0.0, scavenge: 0.0, repair: 0.0 }
    }
}

#[derive(Clone, Copy)]
pub struct GridItem {
    pub entity: Entity,
    pub pos: Vec3,
    pub radius: f32,
}

/// Uniform grid over the map holding every enemy. Rebuilt once per frame, it
/// keeps neighbour queries (separation, splash, turret targeting, hitscan)
/// proportional to local density instead of total unit count.
#[derive(Resource)]
pub struct Grid {
    cell: f32,
    half: f32,
    side: usize,
    cells: Vec<Vec<GridItem>>,
    pub count: usize,
}

impl Grid {
    pub fn new(half_extent: f32) -> Self {
        let cell = 6.0;
        let side = (2.0 * half_extent / cell).ceil() as usize + 1;
        Self { cell, half: half_extent, side, cells: vec![Vec::new(); side * side], count: 0 }
    }

    fn coord(&self, v: f32) -> usize {
        (((v + self.half) / self.cell) as isize).clamp(0, self.side as isize - 1) as usize
    }

    pub fn clear(&mut self) {
        for cell in &mut self.cells {
            cell.clear();
        }
        self.count = 0;
    }

    pub fn insert(&mut self, item: GridItem) {
        let index = self.coord(item.pos.z) * self.side + self.coord(item.pos.x);
        self.cells[index].push(item);
        self.count += 1;
    }

    /// Visits every item whose cell overlaps the circle (x, z, radius).
    pub fn for_each_near(&self, center: Vec3, radius: f32, mut visit: impl FnMut(&GridItem)) {
        let (x0, x1) = (self.coord(center.x - radius), self.coord(center.x + radius));
        let (z0, z1) = (self.coord(center.z - radius), self.coord(center.z + radius));
        for z in z0..=z1 {
            for x in x0..=x1 {
                self.cells[z * self.side + x].iter().for_each(&mut visit);
            }
        }
    }

    /// Closest item (surface distance) within `radius`, optionally skipping one entity.
    pub fn nearest(&self, center: Vec3, radius: f32, skip: Option<Entity>) -> Option<GridItem> {
        let mut best: Option<(f32, GridItem)> = None;
        self.for_each_near(center, radius, |item| {
            let d = item.pos.distance(center) - item.radius;
            if d <= radius && Some(item.entity) != skip && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, *item));
            }
        });
        best.map(|(_, item)| item)
    }

    /// First item hit by a ray, as (distance, item). Walks only the cells under the ray.
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<(f32, GridItem)> {
        let mut best: Option<(f32, GridItem)> = None;
        let mut test = |item: &GridItem| {
            if let Some(d) = ray_sphere(origin, dir, item.pos, item.radius) {
                if d <= max && best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, *item));
                }
            }
        };
        // Sample the ray every half cell and test a one-cell margin; cells repeat, which is fine.
        let steps = (max / (self.cell * 0.5)).ceil() as usize;
        let mut last = (usize::MAX, usize::MAX);
        for i in 0..=steps {
            let p = origin + dir * (i as f32 * self.cell * 0.5).min(max);
            let (cx, cz) = (self.coord(p.x), self.coord(p.z));
            if (cx, cz) == last {
                continue;
            }
            last = (cx, cz);
            for z in cz.saturating_sub(1)..=(cz + 1).min(self.side - 1) {
                for x in cx.saturating_sub(1)..=(cx + 1).min(self.side - 1) {
                    self.cells[z * self.side + x].iter().for_each(&mut test);
                }
            }
        }
        best
    }
}

pub fn rebuild_grid(mut grid: ResMut<Grid>, content: Res<Content>, enemies: Query<(Entity, &Transform, &Enemy)>) {
    grid.clear();
    for (entity, transform, enemy) in &enemies {
        let radius = content.enemies[enemy.kind as usize].radius;
        grid.insert(GridItem { entity, pos: transform.translation + Vec3::Y * radius, radius });
    }
}

pub fn ray_sphere(origin: Vec3, dir: Vec3, center: Vec3, radius: f32) -> Option<f32> {
    let oc = origin - center;
    let b = oc.dot(dir);
    let h = b * b - (oc.length_squared() - radius * radius);
    if h < 0.0 {
        return None;
    }
    let (near, far) = (-b - h.sqrt(), -b + h.sqrt());
    if near >= 0.0 { Some(near) } else if far >= 0.0 { Some(far) } else { None }
}

pub fn ray_box(origin: Vec3, dir: Vec3, center: Vec3, half: Vec3, max: f32) -> Option<f32> {
    let (mut lo, mut hi) = (0.0f32, max);
    for axis in 0..3 {
        let (min, max) = (center[axis] - half[axis], center[axis] + half[axis]);
        if dir[axis].abs() < 1e-6 {
            if origin[axis] < min || origin[axis] > max {
                return None;
            }
        } else {
            let (a, b) = ((min - origin[axis]) / dir[axis], (max - origin[axis]) / dir[axis]);
            lo = lo.max(a.min(b));
            hi = hi.min(a.max(b));
            if hi < lo {
                return None;
            }
        }
    }
    Some(lo)
}

/// A simulated shot: rockets from players, acid and bolts from enemies.
#[derive(Component, Clone, Copy, Debug)]
pub struct Projectile {
    pub velocity: Vec3,
    pub gravity: f32,
    pub damage: f32,
    pub splash: f32,
    pub look: ShotLook,
    /// `Some(player)` = friendly, hurts enemies. `None` = hostile, hurts players, structures, objective.
    pub owner: Option<u8>,
    pub life: f32,
}

/// Damages every enemy within `radius`, with linear falloff.
pub fn splash(grid: &Grid, hits: &mut EventWriter<Hit>, at: Vec3, radius: f32, damage: f32, by: Option<u8>, skip: Option<Entity>) {
    grid.for_each_near(at, radius, |item| {
        let d = (item.pos.distance(at) - item.radius).max(0.0);
        if d < radius && Some(item.entity) != skip {
            hits.write(Hit::plain(item.entity, damage * (1.0 - 0.6 * d / radius), by));
        }
    });
}

#[allow(clippy::too_many_arguments)]
pub fn fly_projectiles(
    time: Res<Time>,
    map: Res<Map>,
    grid: Res<Grid>,
    mut state: ResMut<MatchState>,
    mut commands: Commands,
    mut shots: Query<(Entity, &mut Transform, &mut Projectile)>,
    mut players: Query<(&Transform, &mut Health, &mut Player), (Without<Projectile>, Without<Structure>)>,
    mut structures: Query<(&Transform, &Solid, &mut Health), (With<Structure>, Without<Projectile>, Without<Player>)>,
    mut hits: EventWriter<Hit>,
    mut fx: EventWriter<FxEvent>,
) {
    let dt = sim_dt(&time);
    let core = map.ground(map.core.x, map.core.y) + Vec3::Y * 1.5;
    for (entity, mut transform, mut shot) in &mut shots {
        shot.velocity.y -= shot.gravity * dt;
        let from = transform.translation;
        let step = shot.velocity * dt;
        let to = from + step;
        let length = step.length().max(1e-4);
        let dir = step / length;
        shot.life -= dt;

        let mut impact = None;
        if to.y <= map.height(to.x, to.z) || shot.life <= 0.0 {
            impact = Some(to);
        }
        if shot.owner.is_some() {
            if let Some((d, item)) = grid.raycast(from, dir, length) {
                impact = Some(from + dir * d);
                hits.write(Hit::plain(item.entity, shot.damage, shot.owner));
                if shot.splash > 0.0 {
                    splash(&grid, &mut hits, from + dir * d, shot.splash, shot.damage * 0.7, shot.owner, Some(item.entity));
                }
            } else if impact.is_some() && shot.splash > 0.0 {
                splash(&grid, &mut hits, to, shot.splash, shot.damage * 0.7, shot.owner, None);
            }
        } else {
            for (target, mut health, mut player) in &mut players {
                if health.alive() && ray_sphere(from, dir, target.translation + Vec3::Y * 1.0, 0.75).is_some_and(|d| d <= length) {
                    health.hp -= shot.damage;
                    player.calm = 0.0;
                    fx.write(FxEvent::Hurt { player: player.id });
                    impact = Some(to);
                }
            }
            if impact.is_none() {
                for (target, solid, mut health) in &mut structures {
                    if ray_box(from, dir, solid.center(target), solid.half, length).is_some() {
                        health.hp -= shot.damage;
                        impact = Some(to);
                        break;
                    }
                }
            }
            if impact.is_none() && state.objective_max > 0.0 && ray_sphere(from, dir, core, 1.9).is_some_and(|d| d <= length) {
                state.damage_objective(shot.damage);
                impact = Some(to);
            }
        }

        match impact {
            Some(at) => {
                if shot.splash > 0.0 {
                    fx.write(FxEvent::Explosion { at, radius: shot.splash });
                } else {
                    fx.write(FxEvent::Sound { sfx: Sfx::Hit, at: Some(at) });
                }
                commands.entity(entity).despawn();
            }
            None => {
                transform.translation = to;
                transform.look_to(dir, Vec3::Y);
            }
        }
    }
}

/// Spawns a projectile entity. Used by weapons and enemies alike.
pub fn spawn_projectile(commands: &mut Commands, ids: &mut crate::net::NetIds, at: Vec3, projectile: Projectile) {
    commands.spawn((
        MatchEntity,
        ids.next(),
        projectile,
        Transform::from_translation(at).looking_to(projectile.velocity.normalize_or(Vec3::NEG_Z), Vec3::Y),
    ));
}

/// Applies queued hits: damage, mod effects, kills, rewards.
pub fn resolve_hits(
    mut events: EventReader<Hit>,
    grid: Res<Grid>,
    content: Res<Content>,
    mut state: ResMut<MatchState>,
    mut enemies: Query<(&Transform, &mut Enemy, &mut Health)>,
    mut commands: Commands,
    mut fx: EventWriter<FxEvent>,
    mut bonus_credits: Local<f32>,
) {
    let mut chained = Vec::new();
    let mut apply = |hit: &Hit, state: &mut MatchState, chained: &mut Vec<Hit>, commands: &mut Commands, fx: &mut EventWriter<FxEvent>| {
        let Ok((transform, mut enemy, mut health)) = enemies.get_mut(hit.target) else { return };
        if !health.alive() {
            return;
        }
        health.hp -= hit.amount;
        enemy.slow = enemy.slow.max(hit.frost);
        if hit.venom > 0.0 {
            enemy.venom = enemy.venom.max(hit.venom);
            enemy.venom_time = 4.0;
            enemy.venom_by = hit.by;
        }
        *bonus_credits += hit.scavenge;
        if hit.repair > 0.0 && state.objective_max > 0.0 {
            state.objective_hp = (state.objective_hp + hit.repair).min(state.objective_max);
        }
        let def = &content.enemies[enemy.kind as usize];
        let at = transform.translation + Vec3::Y * def.radius;
        if hit.arc > 0.0 {
            if let Some(other) = grid.nearest(at, 7.0, Some(hit.target)) {
                chained.push(Hit::plain(other.entity, hit.amount * hit.arc, hit.by));
                fx.write(FxEvent::Arc { from: at, to: other.pos });
            }
        }
        let killed = !health.alive();
        if killed {
            state.kills += 1;
            state.stock.credits += def.reward;
            fx.write(FxEvent::EnemyDied { at, kind: enemy.kind });
            commands.entity(hit.target).despawn();
        }
        if let Some(player) = hit.by {
            fx.write(FxEvent::HitMarker { player, kill: killed });
        }
    };
    for hit in events.read() {
        apply(hit, &mut state, &mut chained, &mut commands, &mut fx);
    }
    // Arc chains resolve once; they never chain again.
    for hit in std::mem::take(&mut chained) {
        apply(&hit, &mut state, &mut chained, &mut commands, &mut fx);
    }
    if *bonus_credits >= 1.0 {
        state.stock.credits += bonus_credits.floor() as u32;
        *bonus_credits = bonus_credits.fract();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rays_hit_and_miss() {
        assert_eq!(ray_sphere(Vec3::ZERO, Vec3::Z, Vec3::Z * 5.0, 1.0), Some(4.0));
        assert_eq!(ray_sphere(Vec3::ZERO, Vec3::Z, Vec3::X * 3.0, 1.0), None);
        assert_eq!(ray_box(Vec3::ZERO, Vec3::Z, Vec3::Z * 5.0, Vec3::ONE, 100.0), Some(4.0));
        assert_eq!(ray_box(Vec3::ZERO, Vec3::Z, Vec3::X * 3.0, Vec3::ONE, 100.0), None);
    }

    #[test]
    fn grid_finds_neighbours_and_ray_targets() {
        let mut grid = Grid::new(160.0);
        let mut world = World::new();
        let (near, far) = (world.spawn_empty().id(), world.spawn_empty().id());
        grid.insert(GridItem { entity: near, pos: Vec3::new(10.0, 1.0, 0.0), radius: 1.0 });
        grid.insert(GridItem { entity: far, pos: Vec3::new(60.0, 1.0, 0.0), radius: 1.0 });
        assert_eq!(grid.nearest(Vec3::new(8.0, 1.0, 0.0), 5.0, None).map(|i| i.entity), Some(near));
        assert_eq!(grid.nearest(Vec3::new(8.0, 1.0, 0.0), 5.0, Some(near)).map(|i| i.entity), None);
        let (d, item) = grid.raycast(Vec3::new(-100.0, 1.0, 0.0), Vec3::X, 300.0).unwrap();
        assert_eq!(item.entity, near);
        assert!((d - 109.0).abs() < 1e-3);
        assert!(grid.raycast(Vec3::new(-100.0, 1.0, 9.0), Vec3::X, 300.0).is_none());
    }
}
