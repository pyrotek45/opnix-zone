//! Data-driven weapon firing for every player the authority knows about:
//! humans, remote clients and bots alike.

use crate::{
    combat::{Grid, Hit, Impact, Projectile, Solid, ray_box, ray_sphere, spawn_projectile, splash, throw},
    lagcomp::{LagHistory, ViewTime},
    net::NetIds,
    player::{Arsenal, Health, Player, PlayerInput, buttons, damage_player, perk, stance},
    session::{Content, Map, Statics},
    sim_dt,
    state::{FxEvent, MatchState, Phase, Rules, SimRng},
    structures::{OreNode, Structure},
};
use bevy::prelude::*;
use oz_core::{
    arsenal::{Action, FireMode, ReloadStyle, Resolved, WeaponDef, WeaponState, resolve},
    collide::StaticWorld,
    content::{GearKind, PerkEffect, ShotLook, UpgradeEffect},
    synth::Sfx,
};

const SWAP_TIME: f32 = 0.3;
const BEAM_RANGE: f32 = 7.5;
/// Thickest wall a piercing round can punch through, and what it keeps.
const PIERCE_DEPTH: f32 = 0.65;
const PIERCE_KEEP: f32 = 0.6;
const BURN_SECONDS: f32 = 3.0;
/// A gun-butt strike: one puts a standard robot down (armour perks survive it).
pub const MELEE_DAMAGE: f32 = 100.0;

/// Where shots leave the gun, relative to the eye, so tracers line up with the viewmodel.
fn muzzle(eye: Vec3, look: Quat, length: f32) -> Vec3 {
    eye + look * Vec3::new(0.16, -0.13, -(0.4 + length * 0.62))
}

/// A player as a target for other players' shots (versus modes).
#[derive(Clone)]
struct Body {
    entity: Entity,
    id: u8,
    team: u8,
    feet: Vec3,
    stance: u8,
    yaw: f32,
    /// Head offset from leaning.
    lean: Vec3,
    /// Limbs already shot off (cannot be hit again).
    lost: u8,
    name: String,
}

impl Body {
    /// Hit spheres (centre, radius, part) for the current stance: head, torso, two arms, two legs.
    fn spheres(&self) -> [(Vec3, f32, u8); 6] {
        use crate::player::part;
        let up = |y: f32| self.feet + Vec3::Y * y;
        let forward = Vec3::new(-self.yaw.sin(), 0.0, -self.yaw.cos());
        let right = Vec3::new(self.yaw.cos(), 0.0, -self.yaw.sin());
        let (head, torso, arms, legs, torso_r) = match self.stance {
            stance::PRONE => (up(0.32) + forward * 0.62, up(0.26) + forward * 0.05, up(0.24) + forward * 0.45, up(0.18) - forward * 0.6, 0.34),
            // Crouched, the head sits just under low cover (`worldgen::COVER_HEIGHT`).
            stance::CROUCH => (up(0.95) + self.lean, up(0.62) + self.lean * 0.5, up(0.72) + self.lean * 0.5 - forward * 0.05, up(0.28), 0.36),
            _ => (up(1.62) + self.lean, up(1.05) + self.lean * 0.55, up(1.18) + self.lean * 0.55, up(0.45), 0.36),
        };
        let arm_x = if self.stance == stance::PRONE { 0.28 } else { 0.36 };
        [
            (head, 0.24, part::HEAD),
            (torso, torso_r, part::TORSO),
            (arms - right * arm_x, 0.15, part::LEFT_ARM),
            (arms + right * arm_x, 0.15, part::RIGHT_ARM),
            (legs - right * 0.13, 0.19, part::LEFT_LEG),
            (legs + right * 0.13, 0.19, part::RIGHT_LEG),
        ]
    }

    /// Distance along the ray to this body, and which part it hit.
    fn hit(&self, origin: Vec3, dir: Vec3) -> Option<(f32, u8)> {
        self.spheres()
            .into_iter()
            .filter(|(_, _, p)| crate::player::limb::of_part(*p).is_none_or(|bit| self.lost & bit == 0))
            .filter_map(|(center, radius, part)| ray_sphere(origin, dir, center, radius).map(|d| (d, part)))
            .min_by(|a, b| a.0.total_cmp(&b.0))
    }

    fn chest(&self) -> Vec3 {
        self.spheres()[1].0
    }
}

/// How far a ray gets before something solid stops it.
pub fn obstruction<F: bevy::ecs::query::QueryFilter>(map: &Map, statics: &StaticWorld, solids: &Query<(&Transform, &Solid), F>, origin: Vec3, dir: Vec3, range: f32) -> f32 {
    let mut best = map.raycast(origin, dir, range);
    if let Some((d, _)) = statics.raycast(origin, dir, best) {
        best = best.min(d);
    }
    for (transform, solid) in solids {
        if let Some(d) = ray_box(origin, dir, solid.center(transform), solid.half, best) {
            best = best.min(d);
        }
    }
    best
}

/// Like [`obstruction`], but a piercing round may pass one thin wall.
/// Returns (reach, damage multiplier at the target).
fn reach_with_pierce<F: bevy::ecs::query::QueryFilter>(map: &Map, statics: &StaticWorld, solids: &Query<(&Transform, &Solid), F>, origin: Vec3, dir: Vec3, range: f32, pierce: bool) -> (f32, f32) {
    let first = obstruction(map, statics, solids, origin, dir, range);
    if !pierce || first >= range {
        return (first, 1.0);
    }
    match statics.raycast(origin, dir, first + 0.01) {
        Some((d, index)) if (d - first).abs() < 0.02 && statics.thickness(index, origin, dir) < PIERCE_DEPTH => {
            let exit = d + statics.thickness(index, origin, dir) + 0.02;
            let beyond = obstruction(map, statics, solids, origin + dir * exit, dir, range - exit);
            (exit + beyond, PIERCE_KEEP)
        }
        _ => (first, 1.0),
    }
}

fn spread_dir(look: Quat, spread: f32, rng: &mut SimRng) -> Vec3 {
    let angle = rng.0.range(0.0, std::f32::consts::TAU);
    let radius = spread * rng.0.unit().sqrt();
    (look * Vec3::new(angle.cos() * radius, angle.sin() * radius, -1.0)).normalize()
}

fn hit_from(stats: &Resolved, target: Entity, amount: f32, player: u8) -> Hit {
    let venom = stats.venom + stats.burn;
    Hit { target, amount, by: Some(player), frost: stats.frost, venom, arc: stats.arc, scavenge: stats.scavenge, repair: stats.repair }
}

/// Damage dealt to a player this frame: (victim, amount before the part multiplier, attacker index into bodies, burn, part hit).
type Strike = (Entity, f32, usize, f32, u8);

#[allow(clippy::too_many_arguments)]
pub fn fire_weapons(
    time: Res<Time>,
    map: Res<Map>,
    statics: Res<Statics>,
    (rules, history): (Res<Rules>, Res<LagHistory>),
    content: Res<Content>,
    grid: Res<Grid>,
    mut state: ResMut<MatchState>,
    mut rng: ResMut<SimRng>,
    mut ids: ResMut<NetIds>,
    mut commands: Commands,
    mut players: Query<(Entity, &mut Player, &Transform, &PlayerInput, &mut Arsenal, &mut Health, Option<&ViewTime>)>,
    solids: Query<(&Transform, &Solid), (Or<(With<Structure>, With<crate::breakables::Door>, With<crate::breakables::ClutterPiece>)>, Without<OreNode>)>,
    mut ores: Query<(&Transform, &mut OreNode)>,
    (mut hits, mut traces, mut noises): (EventWriter<Hit>, EventWriter<crate::breakables::Trace>, ResMut<crate::bots::Noises>),
    mut fx: EventWriter<FxEvent>,
    mut ore_carry: Local<[f32; 3]>,
) {
    let dt = sim_dt(&time);
    let bodies: Vec<Body> = if rules.pvp {
        players
            .iter()
            .filter(|(.., health, _)| health.alive())
            .map(|(entity, player, transform, input, ..)| Body {
                entity,
                id: player.id,
                team: player.team,
                feet: transform.translation,
                stance: input.stance,
                yaw: input.yaw,
                lean: crate::player::lean_offset(&statics, transform.translation, input),
                lost: player.lost,
                name: player.name.clone(),
            })
            .collect()
    } else {
        Vec::new()
    };
    let mut struck: Vec<Strike> = Vec::new();
    // Ammo packs dropped this frame: (where, reach, team).
    let mut resupply: Vec<(Vec3, f32, u8)> = Vec::new();
    let lobby = state.phase == Phase::Lobby;

    let now = time.elapsed_secs();
    for (entity, mut player, transform, input, mut arsenal, mut health, view) in &mut players {
        // Plain reborrow so separate fields can be borrowed at once.
        let arsenal = &mut *arsenal;
        arsenal.cooldown = (arsenal.cooldown - dt).max(0.0);
        if !health.alive() || state.over() || lobby {
            // Killed with the pin out: the grenade drops at their feet and still goes off.
            if let Some(cooked) = player.cooking.take()
                && !health.alive()
                && let Some(gear) = content.gear.get(player.gear as usize)
                && let Some((_, shot)) = throw(&gear.kind, Vec3::NEG_Y)
            {
                let life = if matches!(gear.kind, GearKind::Knife { .. }) { shot.life } else { (shot.life - cooked).max(0.05) };
                let shot = Projectile { velocity: Vec3::new(0.0, 1.0, 0.0), owner: Some(player.id), team: player.team, life, ..shot };
                spawn_projectile(&mut commands, &mut ids, transform.translation + Vec3::Y * 0.6, shot);
            }
            player.cooking = None;
            arsenal.reload = 0.0;
            arsenal.trigger = false;
            arsenal.burst = 0;
            continue;
        }
        let eye = crate::player::shot_origin(&statics, transform.translation, input);
        let look = input.look();
        let me = bodies.iter().position(|body| body.entity == entity);
        // Lag compensation: remote shooters hit the world as they saw it.
        let rewind_at = view.filter(|_| !history.is_empty()).map(|view| history.clamp(now, view.0));
        // Enemies as this shooter saw them (co-op guests), else as they are now.
        let enemy_ray = |origin: Vec3, dir: Vec3, max: f32| match rewind_at.and_then(|at| history.enemy_raycast(at, origin, dir, max)) {
            Some(hit) => hit,
            None => grid.raycast(origin, dir, max),
        };
        let rewound: Option<Vec<Body>> = rewind_at.filter(|_| rules.pvp).map(|at| {
            bodies
                .iter()
                .filter_map(|b| match history.pose_at(at, b.entity) {
                    Some(pose) if pose.alive => Some(Body { feet: pose.feet, stance: pose.stance, yaw: pose.yaw, lean: pose.lean, name: b.name.clone(), ..*b }),
                    Some(_) => None,
                    None => Some(Body { name: b.name.clone(), ..*b }),
                })
                .collect()
        });
        let targets: &[Body] = rewound.as_deref().unwrap_or(&bodies);

        // Gear. Stims and ammo packs are used on the press. Throwables pull the pin on the
        // press and cook while held (the fuse is burning), fly on release, and go off in the
        // hand if held too long. Reload while cooking puts the pin back.
        let gear_held = input.held(buttons::GEAR);
        let gear_pressed = gear_held && !player.gear_latch;
        player.gear_latch = gear_held;
        if !input.held(buttons::RELOAD) {
            player.reload_block = false;
        }
        let dir = look * Vec3::NEG_Z;
        if gear_pressed
            && player.gear_left > 0
            && player.cooking.is_none()
            && let Some(gear) = content.gear.get(player.gear as usize)
        {
            match gear.kind {
                GearKind::Stim { heal } => {
                    player.gear_left -= 1;
                    health.hp = (health.hp + heal).min(health.max);
                    player.burn_time = 0.0;
                    fx.write(FxEvent::Cue { to: player.id, sfx: Sfx::Buy });
                }
                GearKind::Ammo { radius } => {
                    player.gear_left -= 1;
                    resupply.push((transform.translation, radius, player.team));
                    fx.write(FxEvent::Sound { sfx: Sfx::Buy, at: Some(transform.translation) });
                }
                _ => {
                    player.cooking = Some(0.0);
                    arsenal.reload = 0.0;
                    arsenal.burst = 0;
                    fx.write(FxEvent::Gear { by: player.id, stage: 0 });
                    fx.write(FxEvent::Sound { sfx: Sfx::PinPull, at: Some(eye) });
                }
            }
        }
        if let Some(cooked) = player.cooking {
            let cooked = cooked + dt;
            let gear = content.gear.get(player.gear as usize);
            let template = gear.and_then(|gear| throw(&gear.kind, dir));
            let knife = gear.is_some_and(|gear| matches!(gear.kind, GearKind::Knife { .. }));
            let fuse = template.as_ref().map_or(0.0, |(_, shot)| shot.life);
            if input.held(buttons::RELOAD) && !player.reload_block {
                // Pin back in: nothing used.
                player.cooking = None;
                player.reload_block = true;
                fx.write(FxEvent::Gear { by: player.id, stage: 2 });
            } else if let Some((velocity, shot)) = template {
                let cooked_off = !knife && matches!(shot.effect, Impact::Blast) && cooked >= fuse;
                if !gear_held || cooked_off {
                    player.cooking = None;
                    player.gear_left = player.gear_left.saturating_sub(1);
                    let life = if knife { shot.life } else { (shot.life - cooked).max(0.03) };
                    let (velocity, life) = if cooked_off { (Vec3::ZERO, 0.01) } else { (velocity, life) };
                    let shot = Projectile { velocity, owner: Some(player.id), team: player.team, life, ..shot };
                    let out = obstruction(&map, &statics, &solids, eye, dir, 0.7);
                    spawn_projectile(&mut commands, &mut ids, eye + dir * (out - 0.15).max(0.0), shot);
                    fx.write(FxEvent::Gear { by: player.id, stage: 1 });
                    fx.write(FxEvent::Sound { sfx: Sfx::Throw, at: Some(eye) });
                    noises.0.push(crate::bots::Noise { at: eye, radius: 6.0, team: player.team, shout: false });
                } else {
                    player.cooking = Some(cooked);
                }
            } else {
                player.cooking = None;
            }
        }
        // Hands full with a grenade: no shooting, reloading or melee.
        if player.cooking.is_some() {
            arsenal.trigger = input.held(buttons::FIRE);
            continue;
        }

        // Melee: a gun-butt strike at whoever is in arm's reach ahead.
        player.melee = (player.melee - dt).max(0.0);
        let melee_pressed = input.held(buttons::MELEE) && !player.melee_latch;
        player.melee_latch = input.held(buttons::MELEE);
        if melee_pressed && player.melee <= 0.0 {
            player.melee = 0.75;
            arsenal.reload = 0.0;
            arsenal.burst = 0;
            arsenal.cooldown = arsenal.cooldown.max(0.45);
            player.protect = 0.0;
            let flat = Vec3::new(dir.x, 0.0, dir.z).normalize_or(Vec3::NEG_Z);
            let mut landed = false;
            if let Some(me) = me {
                let victim = targets
                    .iter()
                    .filter(|body| body.team != player.team)
                    .filter_map(|body| {
                        let to = body.chest() - eye;
                        let distance = to.length();
                        let facing = Vec3::new(to.x, 0.0, to.z).normalize_or_zero().dot(flat);
                        (distance < 2.3 && facing > 0.5 && obstruction(&map, &statics, &solids, eye, to / distance.max(1e-3), distance) >= distance - 0.35).then_some((distance, body.entity))
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0));
                if let Some((_, victim)) = victim {
                    struck.push((victim, MELEE_DAMAGE, me, 0.0, crate::player::part::TORSO));
                    landed = true;
                }
            }
            if !landed && let Some((_, item)) = enemy_ray(eye, dir, 2.3) {
                hits.write(Hit::plain(item.entity, MELEE_DAMAGE, Some(player.id)));
                landed = true;
            }
            fx.write(FxEvent::Melee { by: player.id, hit: landed });
            fx.write(FxEvent::Sound { sfx: if landed { Sfx::ImpactMetal } else { Sfx::Swap }, at: Some(eye + dir) });
            noises.0.push(crate::bots::Noise { at: eye, radius: 7.0, team: player.team, shout: false });
            arsenal.trigger = input.held(buttons::FIRE);
            continue;
        }

        // Weapon swap (handling makes it quicker).
        if input.weapon != arsenal.current && arsenal.owns(input.weapon as usize) {
            arsenal.current = input.weapon;
            arsenal.reload = 0.0;
            arsenal.burst = 0;
            let handling = content.weapons.get(input.weapon as usize).zip(arsenal.weapons.get(input.weapon as usize)).map_or(1.0, |(def, state)| resolve(def, state, &content.attachments).handling);
            let quick = match perk(&content, &player) {
                Some(PerkEffect::Handling(scale)) => scale,
                _ => 1.0,
            };
            arsenal.cooldown = SWAP_TIME / (handling * quick).max(0.3);
        }
        let index = arsenal.current as usize;
        let Some(def) = content.weapons.get(index) else {
            continue;
        };
        let mut stats = resolve(def, &arsenal.weapons[index], &content.attachments);
        match perk(&content, &player) {
            Some(PerkEffect::Reload(multiplier)) => stats.reload *= multiplier,
            Some(PerkEffect::Spread(multiplier)) => stats.spread *= multiplier,
            _ => {}
        }
        // Missing an arm: wild aim and slow, one-handed reloads.
        stats.spread *= crate::player::limb::aim_penalty(player.lost);
        if player.lost & crate::player::limb::ARMS != 0 {
            stats.reload *= 1.5;
        }
        let uses_ammo = def.mag > 0;
        let fire_held = input.held(buttons::FIRE);
        let pulled = fire_held && !arsenal.trigger;
        arsenal.trigger = fire_held;

        // Reloading.
        if arsenal.reload > 0.0 {
            let weapon = &mut arsenal.weapons[index];
            // Shell-by-shell reloads can be cut short to fire.
            if def.reload_style == ReloadStyle::Single && pulled && weapon.ammo > 0 {
                arsenal.reload = 0.0;
            } else {
                arsenal.reload -= dt;
                if arsenal.reload <= 0.0 {
                    arsenal.reload = 0.0;
                    finish_reload(def, &stats, weapon, &mut arsenal.reload, &mut arsenal.reload_total);
                    if arsenal.reload > 0.0 {
                        fx.write(FxEvent::Sound { sfx: Sfx::Reload, at: Some(transform.translation) });
                    }
                }
                continue;
            }
        }
        let weapon = arsenal.weapons[index];
        let can_reload = uses_ammo && weapon.ammo < stats.mag && (def.infinite_reserve || weapon.reserve > 0);
        let wants_fire = arsenal.burst > 0 || (fire_held && (def.auto() || pulled));
        if arsenal.reload <= 0.0 && arsenal.burst == 0 && can_reload && ((input.held(buttons::RELOAD) && !player.reload_block) || (wants_fire && weapon.ammo == 0)) {
            arsenal.reload_total = if def.reload_style == ReloadStyle::Single { stats.reload } else { stats.reload + if weapon.ammo == 0 { 0.25 } else { 0.0 } };
            arsenal.reload = arsenal.reload_total;
            arsenal.reload_empty = weapon.ammo == 0;
            fx.write(FxEvent::Sound { sfx: Sfx::Reload, at: Some(transform.translation) });
            continue;
        }
        if !wants_fire || arsenal.cooldown > 0.0 {
            continue;
        }
        if uses_ammo && weapon.ammo == 0 {
            arsenal.burst = 0;
            arsenal.cooldown = 0.3;
            fx.write(FxEvent::Sound { sfx: Sfx::DryFire, at: Some(transform.translation) });
            continue;
        }

        // Fire.
        arsenal.cooldown = stats.interval;
        if let Action::Burst(rounds) = def.action {
            if arsenal.burst == 0 {
                arsenal.burst = rounds;
            }
            arsenal.burst -= 1;
            if arsenal.burst == 0 {
                arsenal.cooldown = stats.interval * 4.5;
            }
        }
        arsenal.shots = arsenal.shots.wrapping_add(1);
        player.protect = 0.0;
        if !stats.quiet && !matches!(perk(&content, &player), Some(PerkEffect::Ghost)) {
            arsenal.loud = 1.5;
        }
        if uses_ammo {
            arsenal.weapons[index].ammo -= 1;
        }
        let from = muzzle(eye, look, stats.length);
        let aiming = input.held(buttons::AIM);
        let hipfire = match perk(&content, &player) {
            Some(PerkEffect::Hipfire(scale)) if !aiming => scale,
            _ => 1.0,
        };
        // Blind fire from cover: the gun is out, the eyes are not.
        let blind = if input.held(buttons::BLIND) { 3.2 } else { 1.0 };
        let spread = (stats.spread * if aiming { stats.ads_spread } else { hipfire } * stance::steadiness(input.stance) * blind).max(if blind > 1.0 { 0.045 } else { 0.0 });
        noises.0.push(crate::bots::Noise { at: eye, radius: if stats.quiet { 9.0 } else { 80.0 }, team: player.team, shout: false });
        let shot = |to: Vec3, beam: bool| FxEvent::Shot { by: player.id, sound: def.sound, from, to, beam, tracer: stats.tracer, quiet: stats.quiet, local: false };
        // Nearest enemy-team player along a ray, within `reach`.
        let player_hit = |dir: Vec3, reach: f32| {
            targets
                .iter()
                .filter(|body| body.team != player.team)
                .filter_map(|body| body.hit(eye, dir).filter(|(d, _)| *d <= reach).map(|(d, part)| (d, part, body.entity)))
                .min_by(|a, b| a.0.total_cmp(&b.0))
        };

        match def.mode {
            FireMode::Projectile { speed, gravity } => {
                let dir = spread_dir(look, spread, &mut rng);
                let projectile = Projectile {
                    velocity: dir * speed,
                    gravity,
                    damage: stats.damage,
                    splash: stats.splash,
                    look: ShotLook::Rocket,
                    owner: Some(player.id),
                    team: player.team,
                    life: 6.0,
                    effect: Impact::Blast,
                };
                // Never start past a wall the shooter is pressed against.
                let out = obstruction(&map, &statics, &solids, eye, dir, 0.8);
                spawn_projectile(&mut commands, &mut ids, eye + dir * (out - 0.15).max(0.0), projectile);
                // The launch flash stops at a wall too.
                let flash = obstruction(&map, &statics, &solids, eye, dir, from.distance(eye) + 1.5);
                fx.write(shot(eye + dir * flash, false));
            }
            FireMode::Beam => {
                let dir = look * Vec3::NEG_Z;
                let reach = obstruction(&map, &statics, &solids, eye, dir, BEAM_RANGE);
                let enemy = enemy_ray(eye, dir, reach);
                let ore = ores
                    .iter_mut()
                    .filter(|(_, node)| node.amount > 0.0)
                    .filter_map(|(t, node)| ray_sphere(eye, dir, t.translation + Vec3::Y * 0.6, 1.3).filter(|d| *d <= BEAM_RANGE).map(|d| (d, t.translation, node)))
                    .min_by(|a, b| a.0.total_cmp(&b.0));
                let mut end = eye + dir * reach;
                match (enemy, ore) {
                    (Some((d, item)), ore) if ore.as_ref().is_none_or(|(od, ..)| d < *od) => {
                        end = eye + dir * d;
                        hits.write(hit_from(&stats, item.entity, stats.damage, player.id));
                    }
                    (_, Some((d, at, mut node))) if def.mining > 0.0 => {
                        end = eye + dir * d;
                        let power = def.mining * content.bonus(&state.upgrades, UpgradeEffect::MiningPower);
                        let mined = (power * stats.interval).min(node.amount);
                        node.amount -= mined;
                        let slot = node.ore as usize;
                        ore_carry[slot] += mined;
                        if ore_carry[slot] >= 1.0 {
                            state.stock.add_ore(node.ore, ore_carry[slot] as u32);
                            ore_carry[slot] = ore_carry[slot].fract();
                            fx.write(FxEvent::Mined { at: at + Vec3::Y * 0.8, ore: node.ore as u8 });
                        }
                    }
                    _ => {
                        if let (Some((d, _, victim)), Some(me)) = (player_hit(dir, reach), me) {
                            end = eye + dir * d;
                            struck.push((victim, stats.damage, me, 0.0, crate::player::part::TORSO));
                        }
                    }
                }
                fx.write(shot(end, true));
            }
            FireMode::Hitscan => {
                for _ in 0..stats.pellets {
                    let dir = spread_dir(look, spread, &mut rng);
                    let (mut reach, kept) = reach_with_pierce(&map, &statics, &solids, eye, dir, stats.range, stats.pierce);
                    // Damage falls off over the last 40% of the range.
                    let falloff = |d: f32| kept * (1.0 - 0.5 * ((d / stats.range - 0.6) / 0.4).clamp(0.0, 1.0));
                    let body = player_hit(dir, reach);
                    if let Some((d, ..)) = body {
                        reach = d;
                    }
                    let end = eye + dir * reach;
                    // Doors, glass and clutter along the way find out about it.
                    traces.write(crate::breakables::Trace { origin: eye, dir, reach, damage: stats.damage * falloff(reach), by: player.id });
                    let mut end_at = end;
                    if let Some((d, item)) = enemy_ray(eye, dir, reach) {
                        end_at = eye + dir * d;
                        hits.write(hit_from(&stats, item.entity, stats.damage * falloff(d), player.id));
                        if stats.splash > 0.0 {
                            splash(&grid, &mut hits, end_at, stats.splash, stats.damage * 0.5, Some(player.id), Some(item.entity));
                        }
                    } else if let (Some((d, head, victim)), Some(me)) = (body, me) {
                        struck.push((victim, stats.damage * falloff(d), me, stats.burn, head));
                    } else if stats.splash > 0.0 && reach < stats.range {
                        splash(&grid, &mut hits, end, stats.splash, stats.damage * 0.5, Some(player.id), None);
                    }
                    // Explosive rounds also catch nearby players.
                    if stats.splash > 0.0
                        && rules.pvp
                        && let Some(me) = me
                    {
                        for body in targets.iter().filter(|b| b.team != player.team && Some(b.entity) != body.map(|h| h.2)) {
                            let distance = body.chest().distance(end_at);
                            if distance < stats.splash {
                                struck.push((body.entity, stats.damage * 0.4 * (1.0 - distance / stats.splash), me, 0.0, crate::player::part::BLAST));
                            }
                        }
                    }
                    fx.write(shot(end_at, false));
                }
            }
        }
    }

    for (at, radius, team) in resupply {
        for (_, player, transform, _, mut arsenal, health, _) in &mut players {
            if player.team == team && health.alive() && transform.translation.distance(at) <= radius {
                top_up(&mut arsenal, &content.weapons);
            }
        }
    }

    for (victim, damage, attacker, burn, part) in struck {
        let attacker = &bodies[attacker];
        if let Ok((_, mut player, _, _, _, mut health, _)) = players.get_mut(victim) {
            if burn > 0.0 {
                player.burn = burn;
                player.burn_time = BURN_SECONDS;
                player.burn_by = Some(attacker.id);
            }
            // Limbs soak their share and can come off; the robot takes the rest scaled by where it was hit.
            crate::player::wound_limb(&mut player, part, damage, attacker.id, &mut fx);
            player.last_hit = part;
            let amount = damage * crate::player::part::multiplier(part);
            damage_player(&mut health, &mut player, amount, Some((&attacker.name, attacker.team, attacker.id)), &mut state, &mut fx);
            player.last_hit = crate::player::part::TORSO;
        }
    }
}

/// End of one reload step. Shell-by-shell reloads queue the next round.
fn finish_reload(def: &WeaponDef, stats: &Resolved, weapon: &mut WeaponState, reload: &mut f32, total: &mut f32) {
    let wanted = stats.mag.saturating_sub(weapon.ammo);
    let per_step = if def.reload_style == ReloadStyle::Single { wanted.min(1) } else { wanted };
    let loaded = if def.infinite_reserve { per_step } else { per_step.min(weapon.reserve) };
    weapon.ammo += loaded;
    if !def.infinite_reserve {
        weapon.reserve -= loaded;
    }
    if def.reload_style == ReloadStyle::Single && weapon.ammo < stats.mag && (def.infinite_reserve || weapon.reserve > 0) {
        *reload = stats.reload;
        *total = stats.reload;
    }
}

/// Fills every owned weapon's reserve back to at least its starting amount.
pub fn top_up(arsenal: &mut Arsenal, defs: &[WeaponDef]) {
    for (state, def) in arsenal.weapons.iter_mut().zip(defs) {
        if state.owned && !def.infinite_reserve {
            state.reserve = state.reserve.max(def.reserve);
        }
    }
}

/// Gives a player a weapon with its default parts, a full magazine and its starting reserve.
pub fn grant_weapon(arsenal: &mut Arsenal, defs: &[WeaponDef], index: usize) {
    if let (Some(state), Some(def)) = (arsenal.weapons.get_mut(index), defs.get(index)) {
        state.owned = true;
        state.ammo = def.mag;
        state.reserve = def.reserve;
    }
}
