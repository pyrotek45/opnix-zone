//! Data-driven weapon firing for every player the authority knows about:
//! humans, remote clients and bots alike.

use crate::{
    combat::{Grid, Hit, Projectile, Solid, ray_box, ray_sphere, spawn_projectile, splash},
    net::NetIds,
    player::{Arsenal, EYE, Health, Player, PlayerInput, buttons, damage_player, perk},
    session::{Content, Map},
    sim_dt,
    state::{FxEvent, MatchState, Rules, SimRng},
    structures::OreNode,
};
use bevy::prelude::*;
use oz_core::{
    arsenal::{FireMode, Resolved, WeaponDef, resolve},
    content::{GearKind, PerkEffect, ShotLook, UpgradeEffect},
    synth::Sfx,
};

const SWAP_TIME: f32 = 0.28;
const BEAM_RANGE: f32 = 7.5;
const HEADSHOT: f32 = 1.6;

/// Where shots leave the gun, relative to the eye, so tracers line up with the viewmodel.
fn muzzle(eye: Vec3, look: Quat, length: f32) -> Vec3 {
    eye + look * Vec3::new(0.16, -0.13, -(0.4 + length * 0.62))
}

/// A player as a target for other players' shots (versus modes).
struct Body {
    entity: Entity,
    id: u8,
    team: u8,
    feet: Vec3,
    name: String,
}

impl Body {
    /// Distance along the ray to this body, and whether it was a headshot.
    fn hit(&self, origin: Vec3, dir: Vec3) -> Option<(f32, bool)> {
        let head = ray_sphere(origin, dir, self.feet + Vec3::Y * 1.62, 0.24).map(|d| (d, true));
        let torso = ray_sphere(origin, dir, self.feet + Vec3::Y * 1.0, 0.5).map(|d| (d, false));
        let legs = ray_sphere(origin, dir, self.feet + Vec3::Y * 0.4, 0.4).map(|d| (d, false));
        [head, torso, legs].into_iter().flatten().min_by(|a, b| a.0.total_cmp(&b.0))
    }
}

#[allow(clippy::too_many_arguments)]
pub fn fire_weapons(
    time: Res<Time>,
    map: Res<Map>,
    rules: Res<Rules>,
    content: Res<Content>,
    grid: Res<Grid>,
    mut state: ResMut<MatchState>,
    mut rng: ResMut<SimRng>,
    mut ids: ResMut<NetIds>,
    mut commands: Commands,
    mut players: Query<(Entity, &mut Player, &Transform, &PlayerInput, &mut Arsenal, &mut Health)>,
    solids: Query<(&Transform, &Solid), Without<OreNode>>,
    mut ores: Query<(&Transform, &mut OreNode)>,
    mut hits: EventWriter<Hit>,
    mut fx: EventWriter<FxEvent>,
    mut ore_carry: Local<[f32; 3]>,
) {
    let dt = sim_dt(&time);
    let bodies: Vec<Body> = if rules.pvp {
        players.iter().filter(|(.., health)| health.alive()).map(|(entity, player, transform, ..)| Body { entity, id: player.id, team: player.team, feet: transform.translation, name: player.name.clone() }).collect()
    } else {
        Vec::new()
    };
    // (victim, damage, attacker index into `bodies`)
    let mut struck: Vec<(Entity, f32, usize)> = Vec::new();

    for (entity, mut player, transform, input, mut arsenal, mut health) in &mut players {
        arsenal.cooldown = (arsenal.cooldown - dt).max(0.0);
        if !health.alive() || state.over() {
            arsenal.reload = 0.0;
            arsenal.trigger = false;
            continue;
        }
        let eye = transform.translation + Vec3::Y * EYE;
        let look = input.look();

        // Gear: one use per button press.
        let gear_pressed = input.held(buttons::GEAR) && !player.gear_latch;
        player.gear_latch = input.held(buttons::GEAR);
        if gear_pressed && player.gear_left > 0 {
            if let Some(gear) = content.gear.get(player.gear as usize) {
                player.gear_left -= 1;
                match gear.kind {
                    GearKind::Frag { damage, radius } => {
                        let dir = look * Vec3::NEG_Z;
                        let shot = Projectile { velocity: dir * 19.0 + Vec3::Y * 4.5, gravity: 13.0, damage, splash: radius, look: ShotLook::Rocket, owner: Some(player.id), team: player.team, life: 1.8 };
                        spawn_projectile(&mut commands, &mut ids, eye + dir * 0.7, shot);
                        fx.write(FxEvent::Sound { sfx: Sfx::Reload, at: Some(eye) });
                    }
                    GearKind::Stim { heal } => {
                        health.hp = (health.hp + heal).min(health.max);
                        fx.write(FxEvent::Cue { to: player.id, sfx: Sfx::Buy });
                    }
                }
            }
        }

        // Weapon swap.
        if input.weapon != arsenal.current && arsenal.owns(input.weapon as usize) {
            arsenal.current = input.weapon;
            arsenal.reload = 0.0;
            arsenal.cooldown = SWAP_TIME;
        }
        let index = arsenal.current as usize;
        let Some(def) = content.weapons.get(index) else { continue };
        let mut stats = resolve(def, &arsenal.weapons[index], &content.attachments);
        match perk(&content, &player) {
            Some(PerkEffect::Reload(multiplier)) => stats.reload *= multiplier,
            Some(PerkEffect::Spread(multiplier)) => stats.spread *= multiplier,
            _ => {}
        }
        let uses_ammo = def.mag > 0;

        // Reload.
        if arsenal.reload > 0.0 {
            arsenal.reload -= dt;
            if arsenal.reload <= 0.0 {
                arsenal.reload = 0.0;
                let weapon = &mut arsenal.weapons[index];
                let wanted = stats.mag.saturating_sub(weapon.ammo);
                let loaded = if def.infinite_reserve { wanted } else { wanted.min(weapon.reserve) };
                weapon.ammo += loaded;
                if !def.infinite_reserve {
                    weapon.reserve -= loaded;
                }
            }
        }
        let weapon = arsenal.weapons[index];
        let can_reload = uses_ammo && weapon.ammo < stats.mag && (def.infinite_reserve || weapon.reserve > 0);
        let fire_held = input.held(buttons::FIRE);
        let wants_fire = fire_held && (def.auto || !arsenal.trigger);
        arsenal.trigger = fire_held;
        if arsenal.reload <= 0.0 && can_reload && (input.held(buttons::RELOAD) || (wants_fire && weapon.ammo == 0)) {
            arsenal.reload = stats.reload;
            fx.write(FxEvent::Sound { sfx: Sfx::Reload, at: Some(transform.translation) });
            continue;
        }
        if !wants_fire || arsenal.cooldown > 0.0 || arsenal.reload > 0.0 {
            continue;
        }
        if uses_ammo && weapon.ammo == 0 {
            arsenal.cooldown = 0.3;
            fx.write(FxEvent::Sound { sfx: Sfx::DryFire, at: Some(transform.translation) });
            continue;
        }

        // Fire.
        arsenal.cooldown = stats.interval;
        arsenal.shots = arsenal.shots.wrapping_add(1);
        if uses_ammo {
            arsenal.weapons[index].ammo -= 1;
        }
        let from = muzzle(eye, look, stats.length);
        let aim_bonus = if input.held(buttons::AIM) { 0.35 } else { 1.0 };
        let me = bodies.iter().position(|body| body.entity == entity);
        // Nearest enemy-team player along a ray, within `reach`.
        let player_hit = |dir: Vec3, reach: f32| {
            bodies
                .iter()
                .filter(|body| body.team != player.team)
                .filter_map(|body| body.hit(eye, dir).filter(|(d, _)| *d <= reach).map(|(d, head)| (d, head, body.entity)))
                .min_by(|a, b| a.0.total_cmp(&b.0))
        };

        match def.mode {
            FireMode::Projectile { speed, gravity } => {
                let dir = spread_dir(look, stats.spread * aim_bonus, &mut rng);
                let projectile = Projectile { velocity: dir * speed, gravity, damage: stats.damage, splash: stats.splash, look: ShotLook::Rocket, owner: Some(player.id), team: player.team, life: 6.0 };
                spawn_projectile(&mut commands, &mut ids, eye + dir * 0.8, projectile);
                fx.write(FxEvent::Shot { by: player.id, sound: def.sound, from, to: from + dir * 1.5, beam: false });
            }
            FireMode::Beam => {
                let dir = look * Vec3::NEG_Z;
                let reach = obstruction(&map, &solids, eye, dir, BEAM_RANGE);
                let enemy = grid.raycast(eye, dir, reach);
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
                            struck.push((victim, stats.damage, me));
                        }
                    }
                }
                fx.write(FxEvent::Shot { by: player.id, sound: def.sound, from, to: end, beam: true });
            }
            FireMode::Hitscan => {
                for _ in 0..def.pellets.max(1) {
                    let dir = spread_dir(look, stats.spread * aim_bonus, &mut rng);
                    let mut reach = obstruction(&map, &solids, eye, dir, stats.range);
                    // Damage falls off over the last 40% of the range.
                    let falloff = |d: f32| 1.0 - 0.5 * ((d / stats.range - 0.6) / 0.4).clamp(0.0, 1.0);
                    let body = player_hit(dir, reach);
                    if let Some((d, ..)) = body {
                        reach = d;
                    }
                    let mut end = eye + dir * reach;
                    if let Some((d, item)) = grid.raycast(eye, dir, reach) {
                        end = eye + dir * d;
                        hits.write(hit_from(&stats, item.entity, stats.damage * falloff(d), player.id));
                        if stats.splash > 0.0 {
                            splash(&grid, &mut hits, end, stats.splash, stats.damage * 0.5, Some(player.id), Some(item.entity));
                        }
                    } else if let (Some((d, head, victim)), Some(me)) = (body, me) {
                        struck.push((victim, stats.damage * falloff(d) * if head { HEADSHOT } else { 1.0 }, me));
                    } else if stats.splash > 0.0 && reach < stats.range {
                        splash(&grid, &mut hits, end, stats.splash, stats.damage * 0.5, Some(player.id), None);
                    }
                    fx.write(FxEvent::Shot { by: player.id, sound: def.sound, from, to: end, beam: false });
                }
            }
        }
    }

    for (victim, damage, attacker) in struck {
        let attacker = &bodies[attacker];
        if let Ok((_, mut player, _, _, _, mut health)) = players.get_mut(victim) {
            damage_player(&mut health, &mut player, damage, Some((&attacker.name, attacker.team, attacker.id)), &mut state, &mut fx);
        }
    }
}

fn hit_from(stats: &Resolved, target: Entity, amount: f32, player: u8) -> Hit {
    Hit { target, amount, by: Some(player), frost: stats.frost, venom: stats.venom, arc: stats.arc, scavenge: stats.scavenge, repair: stats.repair }
}

fn spread_dir(look: Quat, spread: f32, rng: &mut SimRng) -> Vec3 {
    let angle = rng.0.range(0.0, std::f32::consts::TAU);
    let radius = spread * rng.0.unit().sqrt();
    (look * Vec3::new(angle.cos() * radius, angle.sin() * radius, -1.0)).normalize()
}

/// Distance until terrain or a solid blocks the ray.
pub fn obstruction<F: bevy::ecs::query::QueryFilter>(map: &Map, solids: &Query<(&Transform, &Solid), F>, origin: Vec3, dir: Vec3, range: f32) -> f32 {
    let mut best = map.raycast(origin, dir, range);
    for (transform, solid) in solids {
        if let Some(d) = ray_box(origin, dir, solid.center(transform), solid.half, best) {
            best = best.min(d);
        }
    }
    best
}

/// Gives a player a weapon with a full magazine and its starting reserve.
pub fn grant_weapon(arsenal: &mut Arsenal, defs: &[WeaponDef], index: usize) {
    if let (Some(state), Some(def)) = (arsenal.weapons.get_mut(index), defs.get(index)) {
        state.owned = true;
        state.ammo = def.mag;
        state.reserve = def.reserve;
    }
}
