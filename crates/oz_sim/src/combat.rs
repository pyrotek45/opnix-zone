//! Hits, splash damage, projectiles, collision solids and the spatial grid.

use crate::{
    enemy::Enemy,
    player::{Health, Player, damage_player},
    session::{Content, Map, MatchEntity},
    sim_dt,
    state::{FxEvent, MatchState, Rules},
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

    /// Every item, in no particular order.
    pub fn items(&self) -> Vec<GridItem> {
        self.cells.iter().flatten().copied().collect()
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
            if let Some(d) = ray_sphere(origin, dir, item.pos, item.radius)
                && d <= max && best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, *item));
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
    /// The owner's team in versus modes; shots never hurt their own team.
    pub team: u8,
    pub life: f32,
    /// What happens where it lands.
    pub effect: Impact,
}

/// What a projectile does when it lands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Impact {
    /// Damage on a direct hit, plus `splash` if set.
    Blast,
    /// A cloud that blocks sight.
    Smoke { radius: f32, seconds: f32 },
    /// Blinds and stuns anyone facing it within `radius`.
    Flash { radius: f32, seconds: f32 },
    /// Becomes a proximity mine where it lands.
    Mine { damage: f32, radius: f32 },
}

/// Thrown grenades and canisters bounce and roll until their fuse runs out instead of
/// going off where they first land.
pub fn bounces(shot: &Projectile) -> bool {
    matches!(shot.look, ShotLook::Grenade | ShotLook::Canister) && !matches!(shot.effect, Impact::Mine { .. })
}

/// One step of a bouncing throw: falls, glances off walls, skips along the ground and
/// rolls to a stop. Returns the new position. Shared by the host and predicted copies.
pub fn bounce_step(map: &oz_core::worldgen::World, statics: &oz_core::collide::StaticWorld, at: Vec3, shot: &mut Projectile, dt: f32) -> Vec3 {
    const RADIUS: f32 = 0.08;
    shot.velocity.y -= shot.gravity * dt;
    let step = shot.velocity * dt;
    let length = step.length();
    let mut to = at + step;
    if length > 1e-5 {
        let dir = step / length;
        if let Some((d, index)) = statics.raycast(at, dir, length + RADIUS) {
            // Off a wall (or the underside of something): reflect about the face it hit.
            let hit = at + dir * d;
            let normal = statics.aabb(index).map_or(-dir, |b| {
                let rel = (hit - b.center) / b.half.max(Vec3::splat(1e-3));
                let a = rel.abs();
                if a.x >= a.y && a.x >= a.z { Vec3::X * rel.x.signum() } else if a.y >= a.z { Vec3::Y * rel.y.signum() } else { Vec3::Z * rel.z.signum() }
            });
            let into = shot.velocity.dot(normal);
            if into < 0.0 {
                shot.velocity -= normal * into * 1.45;
                shot.velocity *= 0.6;
            }
            to = hit + normal * RADIUS;
        }
    }
    // The ground (terrain, or a floor or roof under it).
    let ground = map.height(to.x, to.z).max(statics.support(to.x, to.z, to.y + 0.3)) + RADIUS;
    if to.y <= ground {
        to.y = ground;
        if shot.velocity.y < -1.2 {
            // A skip: up again, slower, and scrubbing speed along the ground.
            shot.velocity.y = -shot.velocity.y * 0.35;
            shot.velocity.x *= 0.65;
            shot.velocity.z *= 0.65;
        } else {
            // Rolling: friction brings it to rest.
            shot.velocity.y = 0.0;
            let keep = (1.0 - 3.5 * dt).max(0.0);
            shot.velocity.x *= keep;
            shot.velocity.z *= keep;
        }
    }
    to
}

/// How a piece of thrown gear flies: its launch velocity along `dir` and a
/// template projectile (owner and team still to fill in). Shared by the host
/// and by clients predicting their own throws.
pub fn throw(kind: &oz_core::content::GearKind, dir: Vec3) -> Option<(Vec3, Projectile)> {
    use oz_core::content::GearKind;
    let (effect, damage, splash, speed, gravity, life, look) = match *kind {
        // A long fuse: throw it far, bank it off walls, let it roll in (cooking shortens it).
        GearKind::Frag { damage, radius } => (Impact::Blast, damage, radius, 19.0, 13.0, 4.5, ShotLook::Grenade),
        GearKind::Smoke { radius, seconds } => (Impact::Smoke { radius, seconds }, 0.0, 0.0, 17.0, 13.0, 2.5, ShotLook::Canister),
        GearKind::Flash { radius, seconds } => (Impact::Flash { radius, seconds }, 0.0, 0.0, 18.0, 13.0, 2.2, ShotLook::Canister),
        GearKind::Mine { damage, radius } => (Impact::Mine { damage, radius }, 0.0, 0.0, 7.0, 13.0, 3.0, ShotLook::Grenade),
        GearKind::Knife { damage } => (Impact::Blast, damage, 0.0, 38.0, 2.0, 1.2, ShotLook::Knife),
        GearKind::Stim { .. } | GearKind::Ammo { .. } => return None,
    };
    let velocity = dir * speed + if gravity > 5.0 { Vec3::Y * 4.5 } else { Vec3::ZERO };
    Some((velocity, Projectile { velocity, gravity, damage, splash, look, owner: None, team: 0, life, effect }))
}

/// A sight-blocking cloud. Exists on every machine (clients make theirs from the effect event).
#[derive(Component, Clone, Copy, Debug)]
pub struct SmokeCloud {
    pub radius: f32,
    pub time: f32,
}

/// Does anything in `smokes` sit across the line from `a` to `b`?
pub fn smoke_between<'a>(smokes: impl IntoIterator<Item = (&'a Transform, &'a SmokeCloud)>, a: Vec3, b: Vec3) -> bool {
    let span = b - a;
    let length = span.length();
    if length < 1e-3 {
        return false;
    }
    let dir = span / length;
    smokes.into_iter().any(|(t, smoke)| ray_sphere(a, dir, t.translation, smoke.radius * smoke_scale(smoke.time)).is_some_and(|d| d < length))
}

/// Clouds swell in over a second and thin out over their last two.
pub fn smoke_scale(time_left: f32) -> f32 {
    (time_left / 2.0).clamp(0.0, 1.0)
}

/// A placed proximity mine.
#[derive(Component, Clone, Copy, Debug)]
pub struct Mine {
    pub owner: u8,
    pub team: u8,
    pub damage: f32,
    pub radius: f32,
    /// Seconds until it arms.
    pub arm: f32,
    pub life: f32,
}

/// A bot blinded by a flashbang (seconds left).
#[derive(Component, Clone, Copy, Debug)]
pub struct Stunned(pub f32);

/// Explosive damage multiplier from a victim's perk.
pub fn blast_scale(content: &Content, player: &Player) -> f32 {
    match crate::player::perk(content, player) {
        Some(oz_core::content::PerkEffect::FlakJacket(scale)) => scale,
        _ => 1.0,
    }
}

/// Damages players caught in a blast (versus), with falloff and Flak Jacket.
#[allow(clippy::too_many_arguments)]
/// Is a robot standing at `feet` completely behind solid walls as seen from a
/// blast at `at`? Blasts do not go through buildings.
pub fn sheltered(statics: &oz_core::collide::StaticWorld, at: Vec3, feet: Vec3) -> bool {
    [0.4, 1.1, 1.7].into_iter().all(|h| {
        let to = feet + Vec3::Y * h - at;
        let length = to.length();
        length > 0.3 && statics.raycast(at, to / length, length - 0.2).is_some()
    })
}

pub fn blast_players<F: bevy::ecs::query::QueryFilter>(
    statics: &oz_core::collide::StaticWorld,
    content: &Content,
    state: &mut MatchState,
    fx: &mut EventWriter<FxEvent>,
    players: &mut Query<(&Transform, &mut Health, &mut Player), F>,
    at: Vec3,
    radius: f32,
    damage: f32,
    team: u8,
    owner: u8,
    skip: Option<u8>,
    from_above: bool,
) {
    let name = state.roster.iter().find(|r| r.id == owner).map_or(String::new(), |r| r.name.clone());
    for (target, mut health, mut player) in players.iter_mut() {
        let distance = (target.translation + Vec3::Y).distance(at);
        // Indoors is safe from what falls out of the sky (the roof takes it).
        let roofed = from_above && at.y > target.translation.y + 2.0 && statics.raycast(target.translation + Vec3::Y * 1.7, Vec3::Y, 60.0).is_some();
        if player.team != team && Some(player.id) != skip && distance < radius && !roofed && !sheltered(statics, at, target.translation) {
            let amount = damage * (1.0 - 0.7 * distance / radius) * blast_scale(content, &player);
            player.last_hit = crate::player::part::BLAST;
            // A big blast tears a leg off anyone it does not kill outright.
            if amount > 60.0 {
                let leg = if player.id % 2 == 0 { crate::player::part::LEFT_LEG } else { crate::player::part::RIGHT_LEG };
                crate::player::wound_limb(&mut player, leg, 999.0, owner, fx);
            }
            damage_player(&mut health, &mut player, amount, Some((&name, team, owner)), state, fx);
            player.last_hit = crate::player::part::TORSO;
        }
    }
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
    statics: Res<crate::session::Statics>,
    (rules, content): (Res<Rules>, Res<Content>),
    grid: Res<Grid>,
    mut state: ResMut<MatchState>,
    mut ids: ResMut<crate::net::NetIds>,
    mut commands: Commands,
    mut shots: Query<(Entity, &mut Transform, &mut Projectile)>,
    mut players: Query<(&Transform, &mut Health, &mut Player), (Without<Projectile>, Without<Structure>)>,
    mut structures: Query<(&Transform, &Solid, &mut Health), (With<Structure>, Without<Projectile>, Without<Player>)>,
    bots: Query<(Entity, &Transform, &crate::player::PlayerInput), (With<crate::bots::Bot>, Without<Projectile>)>,
    props: Query<(&Transform, &Solid), (Or<(With<crate::breakables::Door>, With<crate::breakables::ClutterPiece>)>, Without<Projectile>, Without<Structure>, Without<Player>)>,
    mut hits: EventWriter<Hit>,
    mut fx: EventWriter<FxEvent>,
) {
    let dt = sim_dt(&time);
    let core = map.ground(map.core.x, map.core.y) + Vec3::Y * 1.5;
    for (entity, mut transform, mut shot) in &mut shots {
        // Grenades and canisters bounce about until the fuse is spent, then go off where they lie.
        if bounces(&shot) && shot.life - dt > 0.0 {
            shot.life -= dt;
            let at = bounce_step(&map, &statics, transform.translation, &mut shot, dt);
            transform.translation = at;
            continue;
        }
        shot.velocity.y -= shot.gravity * dt;
        let from = transform.translation;
        let step = shot.velocity * dt;
        let to = from + step;
        let length = step.length().max(1e-4);
        let dir = step / length;
        shot.life -= dt;
        let harmful = shot.effect == Impact::Blast;

        let mut impact = None;
        if to.y <= map.height(to.x, to.z) || shot.life <= 0.0 {
            impact = Some(to);
        }
        // The first wall, door or prop in the way this step: nothing beyond it can be hit.
        let mut wall = length;
        if let Some((d, _)) = statics.raycast(from, dir, length) {
            wall = d;
        }
        for (transform, solid) in &props {
            if let Some(d) = ray_box(from, dir, solid.center(transform), solid.half, wall) {
                wall = wall.min(d);
            }
        }
        if wall < length {
            // Grenades and rockets burst just in front of walls rather than inside them.
            impact = Some(from + dir * (wall - 0.05).max(0.0));
        }
        if let Some(owner) = shot.owner {
            if harmful {
                if let Some((d, item)) = grid.raycast(from, dir, wall) {
                    impact = Some(from + dir * d);
                    hits.write(Hit::plain(item.entity, shot.damage, shot.owner));
                    if shot.splash > 0.0 {
                        splash(&grid, &mut hits, from + dir * d, shot.splash, shot.damage * 0.7, shot.owner, Some(item.entity));
                    }
                } else if impact.is_some() && shot.splash > 0.0 {
                    splash(&grid, &mut hits, to, shot.splash, shot.damage * 0.7, shot.owner, None);
                }
            }
            if rules.pvp {
                // Direct hit, or caught in the blast when it lands.
                let name = state.roster.iter().find(|r| r.id == owner).map_or(String::new(), |r| r.name.clone());
                let mut direct = None;
                for (target, mut health, mut player) in &mut players {
                    if let Some(d) = ray_sphere(from, dir, target.translation + Vec3::Y * 1.0, 0.6).filter(|d| *d <= wall)
                        && player.team != shot.team && health.alive() {
                        if harmful {
                            damage_player(&mut health, &mut player, shot.damage, Some((&name, shot.team, owner)), &mut state, &mut fx);
                        }
                        direct = Some(player.id);
                        impact = Some(from + dir * d);
                    }
                }
                if let (Some(at), true) = (impact, harmful && shot.splash > 0.0) {
                    // Bombs and shells from the sky cannot reach anyone with a roof over them.
                    let from_above = matches!(shot.look, ShotLook::Bomb | ShotLook::Shell);
                    blast_players(&statics, &content, &mut state, &mut fx, &mut players, at, shot.splash, shot.damage, shot.team, owner, direct, from_above);
                }
            }
        } else {
            for (target, mut health, mut player) in &mut players {
                if health.alive() && ray_sphere(from, dir, target.translation + Vec3::Y * 1.0, 0.75).is_some_and(|d| d <= wall) {
                    damage_player(&mut health, &mut player, shot.damage, None, &mut state, &mut fx);
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

        let Some(at) = impact else {
            transform.translation = to;
            transform.look_to(dir, Vec3::Y);
            continue;
        };
        commands.entity(entity).despawn();
        match shot.effect {
            Impact::Blast if shot.splash > 0.0 => {
                fx.write(FxEvent::Explosion { at, radius: shot.splash });
            }
            Impact::Blast => {
                fx.write(FxEvent::Sound { sfx: Sfx::Hit, at: Some(at) });
            }
            Impact::Smoke { radius, seconds } => {
                let at = Vec3::new(at.x, map.height(at.x, at.z).max(at.y - 1.0) + radius * 0.4, at.z);
                commands.spawn((MatchEntity, SmokeCloud { radius, time: seconds }, Transform::from_translation(at)));
                fx.write(FxEvent::Smoke { at, radius, seconds });
            }
            Impact::Flash { radius, seconds } => {
                fx.write(FxEvent::Flash { at, radius, seconds });
                // Bots facing the blast with a clear line are stunned; people get blinded on their own screens.
                for (bot, transform, input) in &bots {
                    let eye = transform.translation + Vec3::Y * input.eye();
                    let to_blast = at - eye;
                    let distance = to_blast.length();
                    if distance < radius && statics.raycast(eye, to_blast / distance.max(1e-3), distance).is_none() {
                        let facing = (input.aim_dir().dot(to_blast / distance.max(1e-3)) + 1.0) * 0.5;
                        let strength = (1.0 - distance / radius) * (0.35 + 0.65 * facing);
                        commands.entity(bot).insert(Stunned(seconds * strength));
                    }
                }
            }
            Impact::Mine { damage, radius } => {
                let ground = map.height(at.x, at.z).max(statics.support(at.x, at.z, at.y + 0.3));
                let mine = Mine { owner: shot.owner.unwrap_or(255), team: shot.team, damage, radius, arm: 1.2, life: 120.0 };
                commands.spawn((MatchEntity, ids.next(), mine, Transform::from_xyz(at.x, ground.max(at.y - 2.0), at.z)));
                fx.write(FxEvent::Sound { sfx: Sfx::Build, at: Some(at) });
            }
        }
    }
}

/// Mines arm, wait, and go off when an enemy comes within reach.
#[allow(clippy::too_many_arguments)]
pub fn run_mines(
    time: Res<Time>,
    content: Res<Content>,
    grid: Res<Grid>,
    statics: Res<crate::session::Statics>,
    mut state: ResMut<MatchState>,
    mut commands: Commands,
    mut mines: Query<(Entity, &Transform, &mut Mine)>,
    mut players: Query<(&Transform, &mut Health, &mut Player), (Without<Projectile>, Without<Structure>)>,
    mut hits: EventWriter<Hit>,
    mut fx: EventWriter<FxEvent>,
) {
    let dt = sim_dt(&time);
    for (entity, transform, mut mine) in &mut mines {
        mine.arm -= dt;
        mine.life -= dt;
        if mine.life <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        if mine.arm > 0.0 {
            continue;
        }
        let at = transform.translation + Vec3::Y * 0.3;
        let player_near = players.iter().any(|(t, h, p)| p.team != mine.team && h.alive() && t.translation.distance(transform.translation) < 2.2);
        let enemy_near = grid.nearest(at, 1.6, None).is_some();
        if !player_near && !enemy_near {
            continue;
        }
        commands.entity(entity).despawn();
        fx.write(FxEvent::Explosion { at, radius: mine.radius });
        splash(&grid, &mut hits, at, mine.radius, mine.damage, Some(mine.owner), None);
        blast_players(&statics, &content, &mut state, &mut fx, &mut players, at, mine.radius, mine.damage, mine.team, mine.owner, None, false);
    }
}

/// Clouds drift apart; on every machine.
pub fn tick_smoke(time: Res<Time>, mut commands: Commands, mut smokes: Query<(Entity, &mut SmokeCloud)>) {
    let dt = sim_dt(&time);
    for (entity, mut smoke) in &mut smokes {
        smoke.time -= dt;
        if smoke.time <= 0.0 {
            commands.entity(entity).despawn();
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
        if hit.arc > 0.0
            && let Some(other) = grid.nearest(at, 7.0, Some(hit.target)) {
                chained.push(Hit::plain(other.entity, hit.amount * hit.arc, hit.by));
                fx.write(FxEvent::Arc { from: at, to: other.pos });
            }
        let killed = !health.alive();
        if killed {
            state.kills += 1;
            state.stock.credits += def.reward;
            fx.write(FxEvent::EnemyDied { at, kind: enemy.kind });
            commands.entity(hit.target).despawn();
        }
        if let Some(player) = hit.by {
            fx.write(FxEvent::HitMarker { player, kill: killed, head: false });
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

#[cfg(test)]
mod wall_tests {
    use super::sheltered;
    use bevy::prelude::*;
    use oz_core::collide::{Aabb, StaticWorld};

    #[test]
    fn blasts_do_not_go_through_walls() {
        let mut world = StaticWorld::new(50.0);
        // A wall 3 m tall between x = 0 and x = 0.3.
        world.add(Aabb::span(Vec3::new(0.0, 0.0, -5.0), Vec3::new(0.3, 3.0, 5.0)));
        let blast = Vec3::new(-2.0, 0.5, 0.0);
        assert!(sheltered(&world, blast, Vec3::new(2.0, 0.0, 0.0)), "behind the wall is safe");
        assert!(!sheltered(&world, blast, Vec3::new(-3.0, 0.0, 1.0)), "the same side is not");
        // Peeking over a waist-high wall still gets you hurt.
        let mut low = StaticWorld::new(50.0);
        low.add(Aabb::span(Vec3::new(0.0, 0.0, -5.0), Vec3::new(0.3, 1.0, 5.0)));
        assert!(!sheltered(&low, blast, Vec3::new(2.0, 0.0, 0.0)));
    }
}
