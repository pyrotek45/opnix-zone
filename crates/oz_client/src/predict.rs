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
    burst: u8,
    synced: bool,
}

pub struct PredictPlugin;

impl Plugin for PredictPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Prediction>()
            .add_systems(OnEnter(MatchPhase::Active), |mut prediction: ResMut<Prediction>| *prediction = Prediction::default())
            .add_systems(Update, predict.in_set(ClientSet::Input).after(crate::input::ReadDevices).run_if(in_state(MatchPhase::Active)));
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
) {
    if *role != NetRole::Client {
        return;
    }
    let Ok((player, transform, input, arsenal, health)) = players.single() else { return };
    let p = &mut *prediction;
    // The host's counter is the truth; catch up if it ever runs ahead of us.
    if !p.synced || arsenal.shots > p.total {
        p.total = arsenal.shots;
        p.synced = true;
    }
    p.pending = p.total.saturating_sub(arsenal.shots).min(u16::MAX as u32) as u16;
    p.cooldown = (p.cooldown - time.delta_secs()).max(0.0);

    let index = input.weapon as usize;
    let (Some(def), Some(weapon)) = (content.weapons.get(index), arsenal.weapons.get(index)) else { return };
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
    let eye = transform.translation + Vec3::Y * input.eye();
    let look = input.look();
    let dir = look * Vec3::NEG_Z;
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
    fx.write(FxEvent::Shot {
        by: player.id,
        sound: def.sound,
        from,
        to: eye + dir * end,
        beam: def.mode == FireMode::Beam,
        tracer: stats.tracer,
        quiet: stats.quiet,
        local: true,
    });
}
