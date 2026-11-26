//! Data-driven weapon firing for every player the authority knows about.

use crate::{
    combat::{Grid, Hit, Projectile, Solid, ray_box, ray_sphere, spawn_projectile, splash},
    net::NetIds,
    player::{Arsenal, EYE, Health, Player, PlayerInput, buttons},
    session::{Content, Map},
    sim_dt,
    state::{FxEvent, MatchState, SimRng},
    structures::OreNode,
};
use bevy::prelude::*;
use oz_core::{
    arsenal::{FireMode, Resolved, WeaponDef, resolve},
    content::{ShotLook, UpgradeEffect},
    synth::Sfx,
};

const SWAP_TIME: f32 = 0.28;
const BEAM_RANGE: f32 = 7.5;

/// Where shots leave the gun, relative to the eye, so tracers line up with the viewmodel.
fn muzzle(eye: Vec3, look: Quat, length: f32) -> Vec3 {
    eye + look * Vec3::new(0.22, -0.16, -(0.45 + length))
}

#[allow(clippy::too_many_arguments)]
pub fn fire_weapons(
    time: Res<Time>,
    map: Res<Map>,
    content: Res<Content>,
    grid: Res<Grid>,
    mut state: ResMut<MatchState>,
    mut rng: ResMut<SimRng>,
    mut ids: ResMut<NetIds>,
    mut commands: Commands,
    mut players: Query<(&Player, &Transform, &PlayerInput, &mut Arsenal, &Health)>,
    solids: Query<(&Transform, &Solid), Without<OreNode>>,
    mut ores: Query<(&Transform, &mut OreNode)>,
    mut hits: EventWriter<Hit>,
    mut fx: EventWriter<FxEvent>,
    mut ore_carry: Local<[f32; 3]>,
) {
    let dt = sim_dt(&time);
    for (player, transform, input, mut arsenal, health) in &mut players {
        arsenal.cooldown = (arsenal.cooldown - dt).max(0.0);
        if !health.alive() || state.over() {
            arsenal.reload = 0.0;
            arsenal.trigger = false;
            continue;
        }

        // Weapon swap.
        if input.weapon != arsenal.current && arsenal.owns(input.weapon as usize) {
            arsenal.current = input.weapon;
            arsenal.reload = 0.0;
            arsenal.cooldown = SWAP_TIME;
        }
        let index = arsenal.current as usize;
        let Some(def) = content.weapons.get(index) else { continue };
        let stats = resolve(def, &arsenal.weapons[index], &content.attachments);
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
        let eye = transform.translation + Vec3::Y * EYE;
        let look = input.look();
        let from = muzzle(eye, look, stats.length);
        let aim_bonus = if input.held(buttons::AIM) { 0.35 } else { 1.0 };

        match def.mode {
            FireMode::Projectile { speed, gravity } => {
                let dir = spread_dir(look, stats.spread * aim_bonus, &mut rng);
                let projectile = Projectile { velocity: dir * speed, gravity, damage: stats.damage, splash: stats.splash, look: ShotLook::Rocket, owner: Some(player.id), life: 6.0 };
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
                    _ => {}
                }
                fx.write(FxEvent::Shot { by: player.id, sound: def.sound, from, to: end, beam: true });
            }
            FireMode::Hitscan => {
                for _ in 0..def.pellets.max(1) {
                    let dir = spread_dir(look, stats.spread * aim_bonus, &mut rng);
                    let reach = obstruction(&map, &solids, eye, dir, stats.range);
                    let mut end = eye + dir * reach;
                    if let Some((d, item)) = grid.raycast(eye, dir, reach) {
                        end = eye + dir * d;
                        // Damage falls off over the last 40% of the range.
                        let falloff = 1.0 - 0.5 * ((d / stats.range - 0.6) / 0.4).clamp(0.0, 1.0);
                        hits.write(hit_from(&stats, item.entity, stats.damage * falloff, player.id));
                        if stats.splash > 0.0 {
                            splash(&grid, &mut hits, end, stats.splash, stats.damage * 0.5, Some(player.id), Some(item.entity));
                        }
                    } else if stats.splash > 0.0 && reach < stats.range {
                        splash(&grid, &mut hits, end, stats.splash, stats.damage * 0.5, Some(player.id), None);
                    }
                    fx.write(FxEvent::Shot { by: player.id, sound: def.sound, from, to: end, beam: false });
                }
            }
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
