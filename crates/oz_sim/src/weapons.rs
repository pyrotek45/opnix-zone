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
    arsenal::{Action, FireMode, GunClass, ReloadStyle, Resolved, WeaponDef, WeaponState, falloff as range_falloff, pellet_offset, resolve},
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
/// Rounds fly until they hit something or leave the map: past a gun's effective range they
/// only lose punch (see `falloff` in `fire_weapons`).
pub const MAX_RANGE: f32 = 400.0;
/// Hip fire is a spray: this much wider than a long gun's listed spread before parts and
/// perks (run-and-gun classes are tighter: see [`GunClass::hip_width`]).
pub const HIP_WIDTH: f32 = 1.6;
/// A single pellet to the head counts this much (a slug or bullet gets the full
/// [`crate::player::part::multiplier`]): buckshot to the face kills up close because most of
/// the pattern lands, not because one stray pellet from a far spray got lucky.
pub const PELLET_HEAD: f32 = 1.5;
/// Firing blind from cover throws shots this much wider, and never tighter than the minimum
/// (the tangent of the widest stray).
pub const BLIND_SPREAD: f32 = 3.2;
pub const BLIND_MIN_SPREAD: f32 = 0.045;

/// How much moving opens the cone: walking nearly doubles it, sprinting about triples it,
/// in the air worse still. Aiming down the sights barely blooms.
pub fn bloom(speed: f32, grounded: bool, aiming: bool) -> f32 {
    let walk = (speed / crate::player::WALK_SPEED).clamp(0.0, 1.0);
    let run = ((speed - crate::player::WALK_SPEED) / (crate::player::SPRINT_SPEED - crate::player::WALK_SPEED)).clamp(0.0, 1.0);
    let moving = 1.0 + 0.9 * walk + 1.1 * run + if grounded { 0.0 } else { 1.5 };
    if aiming { 1.0 + (moving - 1.0) * 0.15 } else { moving }
}

/// The cone a shot can land in (the tangent of the widest stray), as the simulation fires
/// it and the crosshair draws it: the gun's spread after parts, the hip-fire width for its
/// class (or the sights' squeeze), stance, and movement bloom weighted by class.
/// `hip_perk` is the hip-fire perk's multiplier (1 without it).
pub fn shot_cone(class: GunClass, stats: &Resolved, aiming: bool, hip_perk: f32, stance_now: u8, speed: f32, grounded: bool) -> f32 {
    let moving = 1.0 + (bloom(speed, grounded, aiming) - 1.0) * class.bloom_weight();
    stats.spread * if aiming { stats.ads_spread } else { hip_perk * class.hip_width() } * stance::steadiness(stance_now) * moving
}

/// Damage one round or pellet deals to a player `distance` away: the falloff for its class
/// and range, the body part's multiplier (pellets capped at [`PELLET_HEAD`] on the head)
/// and, for classes with [`GunClass::lethal_headshot`], a head hit always takes all of `hp`.
pub fn player_damage(class: GunClass, damage: f32, range: f32, distance: f32, pellets: u8, part: u8, hp: f32) -> f32 {
    let head = part == crate::player::part::HEAD;
    let multiplier = if head && pellets > 1 { PELLET_HEAD } else { crate::player::part::multiplier(part) };
    let amount = damage * oz_core::arsenal::falloff(class, range, distance) * multiplier;
    if head && class.lethal_headshot() { amount.max(hp) } else { amount }
}
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
    /// Crouched in cover: hunkered lower.
    hunker: bool,
    /// Firing blind: where the gun (and the arm holding it) is exposed.
    blind: Option<Vec3>,
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
            // Hunkered in cover it folds lower still: the body stays under a waist-high wall.
            stance::CROUCH if self.hunker => (up(0.82) + forward * 0.12, up(0.55), up(0.62) - forward * 0.02, up(0.26), 0.34),
            stance::CROUCH => (up(0.95) + self.lean, up(0.62) + self.lean * 0.5, up(0.72) + self.lean * 0.5 - forward * 0.05, up(0.28), 0.36),
            _ => (up(1.62) + self.lean, up(1.05) + self.lean * 0.55, up(1.18) + self.lean * 0.55, up(0.45), 0.36),
        };
        let arm_x = if self.stance == stance::PRONE { 0.28 } else { 0.36 };
        [
            (head, 0.24, part::HEAD),
            (torso, torso_r, part::TORSO),
            (arms - right * arm_x, 0.15, part::LEFT_ARM),
            // Blind fire: the gun arm is up over the top (or round the edge), and can be hit there.
            (self.blind.unwrap_or(arms + right * arm_x), 0.15, part::RIGHT_ARM),
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

/// Direction of pellet `i` of `n` in one shotgun blast turned by `spin`: the fixed pattern
/// of [`pellet_offset`] scaled to the cone, with a little jitter on each pellet. A single
/// round (or a slug) falls anywhere in the cone, as any bullet does.
fn pellet_dir(look: Quat, spread: f32, i: u8, n: u8, spin: f32, rng: &mut SimRng) -> Vec3 {
    if n <= 1 {
        return spread_dir(look, spread, rng);
    }
    let jitter = (rng.0.range(-1.0, 1.0), rng.0.range(-1.0, 1.0));
    let (x, y) = pellet_offset(i, n, spin, jitter);
    (look * Vec3::new(x * spread, y * spread, -1.0)).normalize()
}

fn hit_from(stats: &Resolved, target: Entity, amount: f32, player: u8) -> Hit {
    let venom = stats.venom + stats.burn;
    Hit { target, amount, by: Some(player), frost: stats.frost, venom, arc: stats.arc, scavenge: stats.scavenge, repair: stats.repair }
}

/// Damage dealt to a player this frame.
struct Strike {
    victim: Entity,
    /// Falloff applied, the part multiplier not yet.
    damage: f32,
    /// Index into the frame's bodies.
    attacker: usize,
    burn: f32,
    part: u8,
    /// Pellets in the shot it came from (pellets are capped on the head).
    pellets: u8,
    /// The weapon's class, for [`GunClass::lethal_headshot`].
    class: GunClass,
}

impl Strike {
    fn plain(victim: Entity, damage: f32, attacker: usize, part: u8) -> Self {
        Self { victim, damage, attacker, burn: 0.0, part, pellets: 1, class: GunClass::Tool }
    }
}

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
    mut players: Query<(Entity, &mut Player, &Transform, &PlayerInput, &mut Arsenal, &mut Health, Option<&ViewTime>, Option<&crate::player::Mover>, Option<&crate::net::Remote>)>,
    solids: Query<(&Transform, &Solid), (Or<(With<Structure>, With<crate::breakables::Door>, With<crate::breakables::ClutterPiece>)>, Without<OreNode>)>,
    mut ores: Query<(&Transform, &mut OreNode)>,
    (mut hits, mut traces, mut noises, mut aircraft): (EventWriter<Hit>, EventWriter<crate::breakables::Trace>, ResMut<crate::bots::Noises>, Query<(Entity, &Transform, &mut crate::killstreak::Aircraft), Without<Player>>),
    mut fx: EventWriter<FxEvent>,
    mut ore_carry: Local<[f32; 3]>,
) {
    let dt = sim_dt(&time);
    let bodies: Vec<Body> = if rules.pvp {
        players
            .iter()
            .filter(|(.., health, _, _, _)| health.alive())
            .map(|(entity, player, transform, input, ..)| Body {
                entity,
                id: player.id,
                team: player.team,
                feet: transform.translation,
                stance: input.stance,
                yaw: input.yaw,
                lean: crate::player::lean_offset(&statics, transform.translation, input),
                hunker: crate::lagcomp::hunkered(input),
                blind: input.held(crate::player::buttons::BLIND).then(|| crate::lagcomp::blind_arm(&statics, transform.translation, input)),
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
    for (entity, mut player, transform, input, mut arsenal, mut health, view, mover, remote) in &mut players {
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
        // A guest shoots from where it really is: its latest report, not the host's
        // slightly delayed playback of it (that would put the muzzle a step behind).
        let feet = remote.and_then(|r| r.latest()).map_or(transform.translation, |(at, _)| at);
        let eye = crate::player::shot_origin(&statics, feet, input);
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
                    Some(pose) if pose.alive => Some(Body {
                        feet: pose.feet,
                        stance: pose.stance,
                        yaw: pose.yaw,
                        lean: pose.lean,
                        hunker: pose.hunker,
                        blind: pose.blind.map(|arm| pose.feet + arm),
                        name: b.name.clone(),
                        ..*b
                    }),
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
                    struck.push(Strike::plain(victim, MELEE_DAMAGE, me, crate::player::part::TORSO));
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
        let blind = if input.held(buttons::BLIND) { BLIND_SPREAD } else { 1.0 };
        let (speed, grounded) = mover.map_or((0.0, true), |m| (m.speed, m.grounded));
        let spread = (shot_cone(def.class, &stats, aiming, hipfire, input.stance, speed, grounded) * blind).max(if blind > 1.0 { BLIND_MIN_SPREAD } else { 0.0 });
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
                            struck.push(Strike::plain(victim, stats.damage, me, crate::player::part::TORSO));
                        }
                    }
                }
                fx.write(shot(end, true));
            }
            FireMode::Hitscan => {
                // One blast, one pattern: only its turn and a little jitter change from shot to shot.
                let spin = rng.0.range(0.0, std::f32::consts::TAU);
                for pellet in 0..stats.pellets {
                    let dir = pellet_dir(look, spread, pellet, stats.pellets, spin, &mut rng);
                    let (mut reach, kept) = reach_with_pierce(&map, &statics, &solids, eye, dir, MAX_RANGE, stats.pierce);
                    // Full damage out to the effective range, then it eases off to the class's floor by three times it.
                    let falloff = |d: f32| kept * range_falloff(def.class, stats.range, d);
                    // Enemy killstreak aircraft in the way take the round.
                    let plane = aircraft
                        .iter()
                        .filter(|(_, _, craft)| craft.team != player.team && craft.hp > 0.0)
                        .filter_map(|(e, t, craft)| ray_sphere(eye, dir, t.translation, craft.radius).filter(|d| *d < reach).map(|d| (d, e)))
                        .min_by(|a, b| a.0.total_cmp(&b.0));
                    if let Some((d, e)) = plane {
                        reach = d;
                        if let Ok((_, _, mut craft)) = aircraft.get_mut(e) {
                            craft.hp -= stats.damage * falloff(d);
                            craft.hit_by = Some(player.id);
                        }
                    }
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
                        struck.push(Strike { victim, damage: stats.damage * falloff(d), attacker: me, burn: stats.burn, part: head, pellets: stats.pellets, class: def.class });
                    } else if stats.splash > 0.0 && reach < MAX_RANGE {
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
                                struck.push(Strike::plain(body.entity, stats.damage * 0.4 * (1.0 - distance / stats.splash), me, crate::player::part::BLAST));
                            }
                        }
                    }
                    fx.write(shot(end_at, false));
                }
            }
        }
    }

    for (at, radius, team) in resupply {
        for (_, player, transform, _, mut arsenal, health, ..) in &mut players {
            if player.team == team && health.alive() && transform.translation.distance(at) <= radius {
                top_up(&mut arsenal, &content.weapons);
            }
        }
    }

    for Strike { victim, damage, attacker, burn, part, pellets, class } in struck {
        let attacker = &bodies[attacker];
        if let Ok((_, mut player, _, _, _, mut health, ..)) = players.get_mut(victim) {
            if burn > 0.0 {
                player.burn = burn;
                player.burn_time = BURN_SECONDS;
                player.burn_by = Some(attacker.id);
            }
            // Limbs soak their share and can come off; the robot takes the rest scaled by where it was hit.
            crate::player::wound_limb(&mut player, part, damage, attacker.id, &mut fx);
            player.last_hit = part;
            // Falloff is already in `damage` (range 0 leaves it alone): this adds the part,
            // the pellet cap on the head, and the sniper's lethal headshot.
            let amount = player_damage(class, damage, f32::MAX, 0.0, pellets, part, health.hp);
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

/// Role guarantees for the standard arsenal, from the real numbers, hit spheres, pellet
/// pattern, falloff and part multipliers the simulation fires with.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::{EYE, part};
    use oz_core::{
        arsenal::{GunClass, standard_attachments},
        kit::{gun, standard_guns},
        rng::Rng,
    };

    /// Health of a robot in heavy armour (the most anyone can have in versus).
    const ARMOURED: f32 = crate::loot::ARMORED;

    fn target(distance: f32) -> Body {
        Body { entity: Entity::PLACEHOLDER, id: 1, team: 1, feet: Vec3::new(0.0, 0.0, -distance), stance: stance::STAND, yaw: 0.0, lean: Vec3::ZERO, hunker: false, blind: None, lost: 0, name: String::new() }
    }

    fn stats(def: &WeaponDef) -> Resolved {
        resolve(def, &WeaponState::fresh(def), &standard_attachments())
    }

    /// Damage one trigger pull deals to `body`, from a standing, still shooter aimed at `aim`.
    fn pull_at(def: &WeaponDef, stats: &Resolved, body: &Body, aim: Vec3, aiming: bool, hp: f32, seed: u64) -> f32 {
        let eye = Vec3::Y * EYE;
        let look = Transform::from_translation(eye).looking_at(aim, Vec3::Y).rotation;
        let spread = shot_cone(def.class, stats, aiming, 1.0, stance::STAND, 0.0, true);
        let mut rng = SimRng(Rng::new(seed));
        let spin = rng.0.range(0.0, std::f32::consts::TAU);
        (0..stats.pellets)
            .filter_map(|i| {
                let dir = pellet_dir(look, spread, i, stats.pellets, spin, &mut rng);
                body.hit(eye, dir).map(|(d, part)| player_damage(def.class, stats.damage, stats.range, d, stats.pellets, part, hp))
            })
            .sum()
    }

    /// One pull at a standing robot `distance` away, aimed at its chest (or its head).
    fn pull(def: &WeaponDef, distance: f32, head: bool, aiming: bool, seed: u64) -> f32 {
        let body = target(distance);
        let aim = if head { body.spheres()[0].0 } else { body.chest() };
        pull_at(def, &stats(def), &body, aim, aiming, 100.0, seed)
    }

    /// Seconds from the first round to the `shots`th, at the gun's fastest cadence.
    fn time_for(def: &WeaponDef, stats: &Resolved, shots: u32) -> f32 {
        (1..shots).map(|k| match def.action { Action::Burst(n) if k % n as u32 == 0 => stats.interval * 4.5, _ => stats.interval }).sum::<f32>().max(0.0)
    }

    /// Shots and seconds to put a 100-health robot down at `distance`, every round landing on
    /// the chest (or head). Shotguns count the pellets their pattern really puts on the body
    /// (`worst` = the weakest of many blasts, else the average): hip fire inside 6 m, aimed beyond.
    fn to_kill(def: &WeaponDef, distance: f32, head: bool, worst: bool) -> Option<(u32, f32)> {
        let stats = stats(def);
        let per = if stats.pellets > 1 {
            let aiming = distance > 6.0;
            let pulls: Vec<f32> = (0..64).map(|seed| pull(def, distance, head, aiming, seed)).collect();
            if worst { pulls.iter().copied().fold(f32::MAX, f32::min) } else { pulls.iter().sum::<f32>() / pulls.len() as f32 }
        } else {
            player_damage(def.class, stats.damage, stats.range, distance, 1, if head { part::HEAD } else { part::TORSO }, 100.0)
        };
        if per <= 0.01 {
            return None;
        }
        let shots = (100.0 / per - 1e-4).ceil().max(1.0) as u32;
        Some((shots, time_for(def, &stats, shots)))
    }

    fn hitscan_guns() -> Vec<WeaponDef> {
        standard_guns().into_iter().filter(|g| g.mode == FireMode::Hitscan).collect()
    }

    #[test]
    fn shotguns_one_shot_up_close_with_their_pattern_and_two_shot_at_mid_range() {
        let guns = standard_guns();
        // Pump and double barrel: one blast at the chest drops anyone out to 8 m from the hip, every time.
        for index in [gun::SHOTGUN, gun::DOUBLE] {
            let def = &guns[index];
            for distance in [2.0, 5.0, 7.0, 8.0] {
                for seed in 0..200 {
                    let dealt = pull(def, distance, false, false, seed);
                    assert!(dealt >= 100.0, "{} hip at {distance} m dealt only {dealt:.0} (seed {seed})", def.name);
                }
            }
            // At 15 m, aimed, two blasts always do it.
            for seed in 0..200 {
                let dealt = pull(def, 15.0, false, true, seed);
                assert!(dealt * 2.0 >= 100.0, "{} aimed at 15 m dealt only {dealt:.0} (seed {seed})", def.name);
            }
            // Past 30 m buckshot is spent: no shotgun is a rifle.
            let far = (0..50).map(|seed| pull(def, 35.0, false, true, seed)).fold(0.0, f32::max);
            assert!(far < 50.0, "{} still deals {far:.0} at 35 m", def.name);
        }
        // The automatic shotguns trade the one-shot for speed: point blank they still drop
        // anyone, and two quick blasts clear a doorway.
        for index in [gun::BREACHER, gun::AUTO_SHOTGUN] {
            let def = &guns[index];
            for seed in 0..200 {
                assert!(pull(def, 3.0, false, false, seed) >= 100.0, "{} point blank", def.name);
                assert!(pull(def, 6.0, false, false, seed) * 2.0 >= 100.0, "{} two blasts at 6 m", def.name);
            }
        }
    }

    #[test]
    fn every_pellet_is_its_own_ray_and_head_pellets_are_capped() {
        let guns = standard_guns();
        let pump = &guns[gun::SHOTGUN];
        let stats = stats(pump);
        // Aimed at the chest point blank, every pellet lands, each for its own damage.
        let dealt = pull(pump, 3.0, false, false, 1);
        assert!((dealt - stats.damage * stats.pellets as f32).abs() < 1e-3, "{dealt} from {} pellets", stats.pellets);
        // A pellet to the head counts 1.5x, a bullet 2.5x.
        let pellet = player_damage(GunClass::Shotgun, stats.damage, stats.range, 1.0, stats.pellets, part::HEAD, 100.0);
        assert!((pellet - stats.damage * PELLET_HEAD).abs() < 1e-4);
        assert!((player_damage(GunClass::Rifle, 30.0, 40.0, 1.0, 1, part::HEAD, 100.0) - 75.0).abs() < 1e-4);
        // A whole blast to the face is lethal, but not 2.5 times over.
        let face = pull(pump, 3.0, true, false, 1);
        assert!(face >= 100.0 && face <= stats.damage * stats.pellets as f32 * PELLET_HEAD + 1e-3, "{face}");
        // Slugs are a single round, so they get the full headshot.
        let mut slug = WeaponState::fresh(pump);
        slug.equipped[oz_core::arsenal::slot::AMMO] = 1;
        assert_eq!(resolve(pump, &slug, &standard_attachments()).pellets, 1);
    }

    #[test]
    fn every_sniper_kills_with_one_headshot_near_and_far() {
        let table = standard_attachments();
        for def in standard_guns().iter().filter(|g| g.class == GunClass::Sniper) {
            // Stock, and with the weakest barrel and ammunition it can fit.
            let mut weak = WeaponState::fresh(def);
            weak.equipped[oz_core::arsenal::slot::BARREL] = 2;
            weak.equipped[oz_core::arsenal::slot::AMMO] = oz_core::arsenal::ammo_for(def.caliber).iter().enumerate().min_by(|a, b| a.1.damage.total_cmp(&b.1.damage)).unwrap().0 as u8;
            for state in [WeaponState::fresh(def), weak] {
                let stats = resolve(def, &state, &table);
                for distance in [10.0, 80.0] {
                    let body = target(distance);
                    let head = body.spheres()[0].0;
                    for seed in 0..20 {
                        // Aimed at the head, it lands there and kills, even through heavy armour.
                        let dealt = pull_at(def, &stats, &body, head, true, ARMOURED, seed);
                        assert!(dealt >= ARMOURED, "{} head at {distance} m dealt {dealt:.0}", def.name);
                    }
                }
                // Any range at all, once it connects.
                for distance in [250.0, 1000.0] {
                    let dealt = player_damage(def.class, stats.damage, stats.range, distance, stats.pellets, part::HEAD, ARMOURED);
                    assert!(dealt >= ARMOURED, "{} head at {distance} m dealt {dealt:.0}", def.name);
                }
            }
            for distance in [10.0, 80.0, 250.0] {
                // The stock round alone already takes a standard robot's head off.
                let stats = stats(def);
                let raw = stats.damage * oz_core::arsenal::falloff(def.class, stats.range, distance) * part::multiplier(part::HEAD);
                assert!(raw >= 100.0, "{} raw head damage {raw:.0} at {distance} m", def.name);
                // A body shot is a two-shot at most.
                let (shots, _) = to_kill(def, distance, false, true).unwrap();
                assert!(shots <= 2, "{} needs {shots} body shots at {distance} m", def.name);
            }
        }
        // The heaviest sniper rifle drops anyone with one round to the chest.
        let guns = standard_guns();
        let heaviest = guns.iter().filter(|g| g.class == GunClass::Sniper).max_by(|a, b| a.weight.total_cmp(&b.weight)).unwrap();
        assert_eq!(to_kill(heaviest, 120.0, false, true).unwrap().0, 1, "{}", heaviest.name);
    }

    #[test]
    fn a_lag_compensated_sniper_headshot_still_kills() {
        use crate::lagcomp::{LagHistory, Pose};
        // A robot running sideways 60 m away; the shooter saw it a moment ago.
        let mut world = World::new();
        let e = world.spawn_empty().id();
        let mut history = LagHistory::default();
        for i in 0..=10 {
            let t = i as f32 * 0.05;
            history.record(t, vec![Pose { entity: e, feet: Vec3::new(t * 6.0, 0.0, -60.0), stance: stance::STAND, yaw: 0.0, alive: true, lean: Vec3::ZERO, hunker: false, blind: None }]);
        }
        let seen = history.pose_at(0.3, e).unwrap();
        // The rewound body, built just as `fire_weapons` builds it from the history.
        let body = Body { feet: seen.feet, stance: seen.stance, yaw: seen.yaw, lean: seen.lean, hunker: seen.hunker, ..target(60.0) };
        let head = body.spheres()[0].0;
        for def in standard_guns().iter().filter(|g| g.class == GunClass::Sniper) {
            let dealt = pull_at(def, &stats(def), &body, head, true, ARMOURED, 3);
            assert!(dealt >= ARMOURED, "{} rewound headshot dealt {dealt:.0}", def.name);
        }
    }

    #[test]
    fn head_spheres_are_fair() {
        let body = target(10.0);
        let spheres = body.spheres();
        let (head, radius, _) = spheres[0];
        // About a robot's head wide: neither a pinhead nor a beach ball, and sitting on the shoulders.
        assert!((0.2..=0.26).contains(&radius), "head radius {radius}");
        assert!(head.y - spheres[1].0.y > radius + spheres[1].1 - 0.05);
        // A shot at the throat, between head and chest, is not a headshot.
        let eye = Vec3::Y * EYE;
        let throat = body.feet + Vec3::Y * 1.36;
        assert_ne!(body.hit(eye, (throat - eye).normalize()).map(|(_, part)| part), Some(part::HEAD));
    }

    #[test]
    fn the_grenade_launcher_is_lethal_inside_its_inner_radius_and_hurts_the_ring_past_it() {
        use crate::combat::blast_damage;
        let guns = standard_guns();
        let launcher = stats(&guns[gun::LAUNCHER]);
        for d in [0.0, 1.0, 2.0, 3.0] {
            let dealt = blast_damage(launcher.damage, launcher.splash, d);
            assert!(dealt >= 100.0, "{d} m from the blast: {dealt:.0}");
        }
        // Even plate carriers go down when it lands at their feet.
        assert!(blast_damage(launcher.damage, launcher.splash, 1.0) >= 130.0);
        for d in [4.5, 5.5, 6.2] {
            let dealt = blast_damage(launcher.damage, launcher.splash, d);
            assert!((30.0..100.0).contains(&dealt), "{d} m: {dealt:.0}");
        }
        assert_eq!(blast_damage(launcher.damage, launcher.splash, launcher.splash + 0.1), 0.0);
    }

    #[test]
    fn smgs_win_close_and_lose_far() {
        let guns = hitscan_guns();
        let ttk = |g: &WeaponDef, d: f32| to_kill(g, d, false, false).map_or(f32::MAX, |(_, t)| t);
        let smgs: Vec<&WeaponDef> = guns.iter().filter(|g| g.class == GunClass::Smg).collect();
        let rifles: Vec<&WeaponDef> = guns.iter().filter(|g| g.class == GunClass::Rifle && g.action == Action::Auto).collect();
        assert!(smgs.len() >= 4 && rifles.len() >= 4);
        for smg in &smgs {
            for rifle in &rifles {
                for d in [3.0, 5.0, 10.0] {
                    assert!(ttk(smg, d) < ttk(rifle, d), "{} ({:.3}s) should out-kill {} ({:.3}s) at {d} m", smg.name, ttk(smg, d), rifle.name, ttk(rifle, d));
                }
            }
            // Past 30 m the best assault rifle wins.
            let best_rifle = rifles.iter().map(|r| ttk(r, 30.0)).fold(f32::MAX, f32::min);
            assert!(best_rifle < ttk(smg, 30.0), "{} still wins at 30 m", smg.name);
        }
        // Up close no other automatic out-kills the slowest SMG (bar the 14 kg rotary cannon).
        let slowest_smg = smgs.iter().map(|s| ttk(s, 5.0)).fold(0.0, f32::max);
        for g in guns.iter().filter(|g| g.action == Action::Auto && !matches!(g.class, GunClass::Smg | GunClass::Shotgun)) {
            if g.weight > 12.0 {
                continue;
            }
            assert!(ttk(g, 5.0) >= slowest_smg, "{} ({:.3}s) kills faster than SMGs ({slowest_smg:.3}s) at 5 m", g.name, ttk(g, 5.0));
        }
        // Hip fire up close is tight: an SMG's hip cone fits on a chest at 10 m, and walking barely opens it.
        for smg in &smgs {
            let stats = stats(smg);
            let cone = shot_cone(smg.class, &stats, false, 1.0, stance::STAND, 0.0, true);
            assert!(cone * 10.0 <= 0.36, "{} hip cone {cone:.3}", smg.name);
            let walking = shot_cone(smg.class, &stats, false, 1.0, stance::STAND, crate::player::WALK_SPEED, true);
            assert!(walking < cone * 1.5, "{} walking cone {walking:.3}", smg.name);
        }
    }

    #[test]
    fn every_gun_kills_in_a_sane_time_at_its_intended_range() {
        for def in hitscan_guns() {
            let stats = stats(&def);
            // At its effective range (aimed), every gun is a real threat...
            let (shots, time) = to_kill(&def, stats.range, false, true).unwrap_or_else(|| panic!("{} cannot kill at its range", def.name));
            let limit = match def.class {
                GunClass::Sniper => 1.0,
                GunClass::Shotgun => 0.8,
                GunClass::Pistol => 0.6,
                _ => 0.45,
            };
            assert!(time <= limit, "{}: {shots} shots, {time:.2}s at {} m", def.name, stats.range);
            // ...but only shotguns and sniper rifles drop a body with a single round.
            if shots == 1 {
                assert!(matches!(def.class, GunClass::Shotgun | GunClass::Sniper), "{} one-shots bodies", def.name);
            }
            // And no automatic is a laser: past shotgun range everything else takes a moment.
            if def.class != GunClass::Shotgun && shots > 1 {
                let (_, t15) = to_kill(&def, 15.0, false, true).unwrap();
                assert!(t15 >= 0.15, "{} kills in {t15:.2}s at 15 m", def.name);
            }
        }
    }

    /// The TTK table in the balance notes: `cargo test -p oz_sim ttk_table -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn ttk_table() {
        println!("| Gun | Class | 5 m body | 5 m head | 15 m body | 15 m head | 30 m body | 30 m head | 60 m body | 60 m head |");
        println!("|---|---|---|---|---|---|---|---|---|---|");
        for def in standard_guns() {
            let cells: Vec<String> = [5.0, 15.0, 30.0, 60.0]
                .into_iter()
                .flat_map(|d| [false, true].map(move |head| (d, head)))
                .map(|(d, head)| match (def.mode, to_kill(&def, d, head, false)) {
                    (FireMode::Projectile { .. }, _) => "blast".into(),
                    (_, Some((shots, t))) if shots <= 40 => format!("{shots} / {t:.2}s"),
                    _ => ">40".into(),
                })
                .collect();
            println!("| {} | {:?} | {} |", def.name, def.class, cells.join(" | "));
        }
    }
}
