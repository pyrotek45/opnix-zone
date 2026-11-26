//! Spectator camera: watch other players over the shoulder.
//!
//! Toggle with V (Select on a pad is taken, so V or `--spectate` at launch).
//! The camera cuts to whoever is in a firefight, holds each shot for a few
//! seconds, and never clips through walls. Useful for watching bots play,
//! recording matches, and as the base for kill-cams later.

use crate::{ClientSet, view::WorldCamera};
use bevy::{platform::collections::HashMap, prelude::*};
use oz_sim::prelude::*;

#[derive(Resource, Default)]
pub struct Spectate {
    pub enabled: bool,
    pub target: Option<Entity>,
    pub name: String,
    hold: f32,
    /// The spectator camera's own pose. The first-person camera rewrites the
    /// camera every frame, so smoothing must start from here, not from it.
    position: Option<Vec3>,
    rotation: Option<Quat>,
}

/// Label for the spectator camera; name tags run after it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct Spectating;

pub struct SpectatePlugin;

impl Plugin for SpectatePlugin {
    fn build(&self, app: &mut App) {
        let enabled = std::env::args().any(|a| a == "--spectate");
        app.insert_resource(Spectate { enabled, ..default() })
            .add_systems(Update, follow.in_set(Spectating).in_set(ClientSet::Present).after(crate::view::FollowPlayer).run_if(in_state(MatchPhase::Active)));
    }
}

#[allow(clippy::too_many_arguments)]
fn follow(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    statics: Res<Statics>,
    mut spectate: ResMut<Spectate>,
    mut events: EventReader<FxEvent>,
    players: Query<(Entity, &Player, &Transform, &PlayerInput, &Health), Without<LocalPlayer>>,
    mut cameras: Query<(&mut Transform, &mut Projection), (With<WorldCamera>, Without<Player>)>,
    mut last_shot: Local<HashMap<u8, f32>>,
    (local, mut death): (Query<&Health, With<LocalPlayer>>, ResMut<crate::fx::DeathInfo>),
    mut was_dead: Local<bool>,
) {
    let now = time.elapsed_secs();
    for event in events.read() {
        if let FxEvent::Shot { by, .. } = event {
            last_shot.insert(*by, now);
        }
    }
    if keys.just_pressed(KeyCode::KeyV) {
        spectate.enabled = !spectate.enabled;
        spectate.position = None;
        spectate.rotation = None;
    }
    // While dead, watch whoever got you until you are back in.
    let dead = local.single().is_ok_and(|h| !h.alive());
    if !dead {
        death.killer = None;
    }
    let killer = death.killer.filter(|_| dead).and_then(|id| players.iter().find(|(_, p, .., h)| p.id == id && h.alive()).map(|(e, ..)| e));
    if dead != *was_dead {
        *was_dead = dead;
        spectate.position = None;
        spectate.rotation = None;
    }
    if !spectate.enabled && killer.is_none() {
        return;
    }
    let dt = time.delta_secs().min(0.05);
    spectate.hold -= dt;
    if let Some(killer) = killer {
        if spectate.target != Some(killer) {
            spectate.position = None;
            spectate.rotation = None;
        }
        spectate.target = Some(killer);
        spectate.hold = 1.0;
    }

    // Cut when the shot has run long or the subject went down; prefer whoever is shooting.
    let current = spectate.target.and_then(|e| players.get(e).ok()).filter(|(.., health)| health.alive());
    if current.is_none() || spectate.hold <= 0.0 {
        let busiest = players
            .iter()
            .filter(|(.., health)| health.alive())
            .filter(|(entity, ..)| Some(*entity) != spectate.target || current.is_none())
            .max_by(|a, b| last_shot.get(&a.1.id).unwrap_or(&-99.0).total_cmp(last_shot.get(&b.1.id).unwrap_or(&-99.0)));
        if let Some((entity, player, ..)) = busiest {
            if spectate.target != Some(entity) {
                spectate.position = None;
                spectate.rotation = None;
            }
            spectate.target = Some(entity);
            spectate.name = player.name.clone();

            spectate.hold = 7.0;
        }
    }
    let Some((_, _, transform, input, _)) = spectate.target.and_then(|e| players.get(e).ok()) else { return };
    let Ok((mut camera, mut projection)) = cameras.single_mut() else { return };

    // Over the right shoulder; if a wall is in the way, rise up and look down instead.
    let yaw = transform.rotation.to_euler(EulerRot::YXZ).0;
    let forward = Vec3::new(-yaw.sin(), 0.0, -yaw.cos());
    // A prone body lies ahead of its feet.
    let body = transform.translation + if input.stance == stance::PRONE { forward * 0.6 } else { Vec3::ZERO };
    let head = body + Vec3::Y * (stance::eye(input.stance) + 0.25);
    let look = Quat::from_euler(EulerRot::YXZ, yaw, input.pitch * 0.7 - 0.12, 0.0);
    let candidates = [Vec3::new(1.1, 0.45, 3.3), Vec3::new(-1.1, 0.45, 3.3), Vec3::new(0.6, 1.8, 2.8), Vec3::new(0.0, 3.4, 1.8)];
    // Take the first clear view; failing that (indoors), the clearest one pulled in to the wall.
    let mut best: Option<(f32, Vec3)> = None;
    for local in candidates {
        let offset = Quat::from_rotation_y(yaw) * local;
        let distance = offset.length();
        let reach = statics.raycast(head, offset / distance, distance + 0.3).map_or(distance, |(d, _)| (d - 0.3).max(0.0)).min(distance);
        let spot = head + offset / distance * reach.max(0.5);
        if reach >= distance * 0.8 {
            best = Some((f32::MAX, spot));
            break;
        }
        if best.is_none_or(|(r, _)| reach / distance > r) {
            best = Some((reach / distance, spot));
        }
    }
    let wanted = best.map_or(head, |(_, spot)| spot);
    let blend = 1.0 - (-dt * 7.0).exp();
    let position = spectate.position.map_or(wanted, |p| p.lerp(wanted, blend));
    spectate.position = Some(position);
    camera.translation = position;

    // Aim past the subject so it sits left of centre with the fight ahead of it.
    let aim = head + look * Vec3::new(1.1, -0.2, -14.0);
    let target = Transform::from_translation(position).looking_at(aim, Vec3::Y).rotation;
    let rotation = spectate.rotation.map_or(target, |r| r.slerp(target, (blend * 1.5).min(1.0)));
    spectate.rotation = Some(rotation);
    camera.rotation = rotation;
    if let Projection::Perspective(perspective) = &mut *projection {
        perspective.fov = 72f32.to_radians();
    }
}
