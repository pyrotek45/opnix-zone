//! Bushes: the one piece of cover you can walk into. They hide you, but they
//! give you away: anyone moving through one makes it shake and rustle loudly
//! enough to be heard across the street. (They stop no bullets.)

use crate::{ClientSet, fx::PlaySfx};
use bevy::prelude::*;
use oz_sim::prelude::*;

#[derive(Component, Clone, Copy, Debug)]
pub struct Bush {
    /// The bush's resting turn.
    pub yaw: f32,
    pub radius: f32,
    /// How hard it is shaking (decays to 0).
    pub shake: f32,
    /// Seconds until it can rustle again.
    pub heard: f32,
}

pub struct BushesPlugin;

impl Plugin for BushesPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            shake_bushes
                .in_set(ClientSet::Present)
                .run_if(in_state(MatchPhase::Active)),
        );
    }
}

fn shake_bushes(
    time: Res<Time>,
    mut bushes: Query<(&mut Bush, &mut Transform), Without<Player>>,
    players: Query<(&Transform, &Health, Option<&Mover>, Has<LocalPlayer>), With<Player>>,
    mut sounds: EventWriter<PlaySfx>,
    mut last: Local<Vec<Vec3>>,
) {
    let dt = time.delta_secs().max(1e-4);
    let t = time.elapsed_secs();
    // How fast everyone is moving (remote players have no mover here: measure them).
    let now: Vec<(Vec3, f32, bool)> = players
        .iter()
        .enumerate()
        .filter(|(_, (_, health, ..))| health.alive())
        .map(|(i, (transform, _, mover, local))| {
            let speed = mover.map(|m| m.speed).unwrap_or_else(|| {
                last.get(i)
                    .map_or(0.0, |p| (transform.translation - *p).xz().length() / dt)
            });
            (transform.translation, speed, local)
        })
        .collect();
    *last = players.iter().map(|(t, ..)| t.translation).collect();
    for (mut bush, mut transform) in &mut bushes {
        bush.heard -= dt;
        for (at, speed, local) in &now {
            let inside = (at.xz() - transform.translation.xz()).length() < bush.radius + 0.25
                && (at.y - transform.translation.y).abs() < 1.2;
            if inside && *speed > 0.6 {
                bush.shake = 1.0;
                if bush.heard <= 0.0 {
                    bush.heard = 0.45;
                    let at = (!local).then_some(transform.translation + Vec3::Y * 0.5);
                    sounds.write(PlaySfx::new(Sfx::Rustle, at).quiet(if *local {
                        0.8
                    } else {
                        1.5
                    }));
                }
            }
        }
        bush.shake = (bush.shake - dt * 1.8).max(0.0);
        let wobble = (t * 23.0 + transform.translation.x).sin() * 0.14 * bush.shake;
        let lean = (t * 17.0 + transform.translation.z).cos() * 0.1 * bush.shake;
        transform.rotation = Quat::from_rotation_y(bush.yaw + wobble)
            * Quat::from_rotation_x(lean)
            * Quat::from_rotation_z(wobble * 0.7);
    }
}
