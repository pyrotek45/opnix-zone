//! Client-side shot prediction.
//!
//! Online, the host decides what a shot hits, but waiting a round trip to
//! *see* your own gun fire feels awful. When this machine is a client, this
//! module runs the trigger, fire-rate and burst rules locally and plays the
//! shot at once: recoil, muzzle flash, sound and tracer, and the ammo counter
//! drops immediately. The host's echo of your own shot is then ignored (see
//! `fx`), and the host's shot counter tells us which predictions it has
//! confirmed, so the ammo display never drifts.
//!
//! Offline and on the host nothing is predicted: the real simulation runs
//! right here, so its own effects are already instant.

use crate::{ClientSet, ui::Screen};
use bevy::prelude::*;
use oz_core::arsenal::{Action, FireMode, resolve};
use oz_sim::prelude::*;

#[derive(Resource, Default)]
pub struct Prediction {
    /// Shots fired locally (predicted), as a running total in the host's numbering.
    pub total: u32,
    /// Predicted shots the host has not confirmed yet; the HUD subtracts these from the magazine.
    pub pending: u16,
    cooldown: f32,
    trigger: bool,
    gear_latch: bool,
    /// A grenade cooking in hand (seconds).
    cook: Option<f32>,
    burst: u8,
    synced: bool,
}

pub struct PredictPlugin;

impl Plugin for PredictPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Prediction>()
            .add_systems(OnEnter(MatchPhase::Active), |mut prediction: ResMut<Prediction>| *prediction = Prediction::default())
            .add_systems(Update, predict.in_set(ClientSet::Input).after(crate::input::ReadDevices).run_if(in_state(MatchPhase::Active)))
            .add_systems(Update, (fly_ghosts, hide_echoes).in_set(ClientSet::Present).run_if(in_state(MatchPhase::Active)));
    }
}

#[allow(clippy::too_many_arguments)]
fn predict(
    time: Res<Time>,
    role: Res<NetRole>,
    screen: Res<Screen>,
    content: Res<Content>,
    map: Res<Map>,
    statics: Res<Statics>,
    state: Res<MatchState>,
    mut prediction: ResMut<Prediction>,
    players: Query<(&Player, &Transform, &PlayerInput, &Arsenal, &Health), With<LocalPlayer>>,
    mut fx: EventWriter<FxEvent>,
    mut commands: Commands,
) {
    if *role != NetRole::Client {
        return;
    }
    let Ok((player, transform, input, arsenal, health)) = players.single() else {
        return;
    };
    let p = &mut *prediction;
    let eye = oz_sim::player::eye_point(&statics, transform.translation, input);
    let look = input.look();
    let dir = look * Vec3::NEG_Z;

    // Thrown gear leaves the hand the moment it is let go (after cooking); the host's copy stays hidden for us.
    let gear_held = input.held(buttons::GEAR) && *screen == Screen::None;
    let live = health.alive() && state.phase != Phase::Lobby && !state.over();
    let template = content.gear.get(player.gear as usize).and_then(|gear| oz_sim::combat::throw(&gear.kind, dir).map(|t| (t, matches!(gear.kind, GearKind::Knife { .. }))));
    if gear_held && !p.gear_latch && player.gear_left > 0 && live && template.is_some() && p.cook.is_none() {
        p.cook = Some(0.0);
    }
    p.gear_latch = gear_held;
    if let Some(cooked) = p.cook {
        let cooked = cooked + time.delta_secs();
        p.cook = Some(cooked);
        if input.held(buttons::RELOAD) || !live {
            p.cook = None;
        } else if let Some(((_, shot), knife)) = template {
            let blast = !knife && matches!(shot.effect, oz_sim::combat::Impact::Blast);
            if !gear_held || (blast && cooked >= shot.life) {
                p.cook = None;
                if gear_held {
                    // Cooked off in the hand: the host's blast says it all.
                } else {
                    let life = if knife { shot.life } else { (shot.life - cooked).max(0.03) };
                    commands.spawn((MatchEntity, Ghost, Projectile { owner: Some(player.id), life, ..shot }, Transform::from_translation(eye + dir * 0.7)));
                }
            }
        }
    }

    // The host's counter is the truth; catch up if it ever runs ahead of us.
    if !p.synced || arsenal.shots > p.total {
        p.total = arsenal.shots;
        p.synced = true;
    }
    p.pending = p.total.saturating_sub(arsenal.shots).min(u16::MAX as u32) as u16;
    p.cooldown = (p.cooldown - time.delta_secs()).max(0.0);

    let index = input.weapon as usize;
    let (Some(def), Some(weapon)) = (content.weapons.get(index), arsenal.weapons.get(index)) else {
        return;
    };
    let held = input.held(buttons::FIRE) && *screen == Screen::None;
    let pulled = held && !p.trigger;
    p.trigger = held;
    let blocked = !health.alive() || arsenal.reload > 0.0 || !weapon.owned || state.over() || state.phase == Phase::Lobby || input.weapon != arsenal.current;
    let wants = p.burst > 0 || (held && (def.auto() || pulled));
    let ammo = weapon.ammo.saturating_sub(p.pending);
    if blocked || !wants || p.cooldown > 0.0 || (def.mag > 0 && ammo == 0) {
        if blocked {
            p.burst = 0;
        }
        return;
    }
    let stats = resolve(def, weapon, &content.attachments);
    p.cooldown = stats.interval;
    if let Action::Burst(rounds) = def.action {
        if p.burst == 0 {
            p.burst = rounds;
        }
        p.burst -= 1;
        if p.burst == 0 {
            p.cooldown = stats.interval * 4.5;
        }
    }
    p.total = p.total.wrapping_add(1);
    if def.mag > 0 {
        p.pending = p.pending.saturating_add(1);
    }

    // Draw the shot where it would go; the host decides what it actually hits.
    if let FireMode::Projectile { speed, gravity } = def.mode {
        let shot = Projectile {
            velocity: dir * speed,
            gravity,
            damage: 0.0,
            splash: stats.splash,
            look: oz_core::content::ShotLook::Rocket,
            owner: Some(player.id),
            team: player.team,
            life: 6.0,
            effect: oz_sim::combat::Impact::Blast,
        };
        commands.spawn((MatchEntity, Ghost, shot, Transform::from_translation(eye + dir * 0.8)));
    }
    let reach = match def.mode {
        FireMode::Projectile { .. } => 1.5,
        FireMode::Beam => 7.5,
        FireMode::Hitscan => stats.range,
    };
    let mut end = map.raycast(eye, dir, reach);
    if let Some((d, _)) = statics.raycast(eye, dir, end) {
        end = end.min(d);
    }
    let from = eye + look * Vec3::new(0.16, -0.13, -(0.4 + stats.length * 0.62));
    fx.write(FxEvent::Shot { by: player.id, sound: def.sound, from, to: eye + dir * end, beam: def.mode == FireMode::Beam, tracer: stats.tracer, quiet: stats.quiet, local: true });
}

/// A locally predicted projectile: pure visuals, flown until it would land.
#[derive(Component)]
pub struct Ghost;

fn fly_ghosts(
    time: Res<Time>,
    map: Res<Map>,
    statics: Res<Statics>,
    mut commands: Commands,
    mut ghosts: Query<(Entity, &mut Transform, &mut Projectile), With<Ghost>>,
    players: Query<(&Transform, &Player, &Health), Without<Ghost>>,
) {
    let dt = time.delta_secs().min(0.05);
    for (entity, mut transform, mut shot) in &mut ghosts {
        shot.velocity.y -= shot.gravity * dt;
        shot.life -= dt;
        let from = transform.translation;
        let step = shot.velocity * dt;
        let length = step.length().max(1e-4);
        let dir = step / length;
        let to = from + step;
        let hit_player = players.iter().any(|(t, p, h)| Some(p.id) != shot.owner && h.alive() && oz_sim::combat::ray_sphere(from, dir, t.translation + Vec3::Y, 0.6).is_some_and(|d| d <= length));
        if shot.life <= 0.0 || to.y <= map.height(to.x, to.z) || statics.raycast(from, dir, length).is_some() || hit_player {
            // The host's impact effect arrives on its own; just vanish.
            commands.entity(entity).despawn();
            continue;
        }
        transform.translation = to;
        transform.look_to(dir, Vec3::Y);
    }
}

/// Hides the host's copies of projectiles this machine already predicted.
fn hide_echoes(role: Res<NetRole>, local: Query<&Player, With<LocalPlayer>>, mut shots: Query<(&Projectile, &mut Visibility), Without<Ghost>>) {
    if *role != NetRole::Client {
        return;
    }
    let Ok(me) = local.single() else { return };
    for (shot, mut visibility) in &mut shots {
        if shot.owner == Some(me.id) && *visibility != Visibility::Hidden {
            *visibility = Visibility::Hidden;
        }
    }
}
