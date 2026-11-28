//! `OPNIX_FRAMETIME=secs`: a developer's frame-cost probe. Once the match is live it counts
//! frames for that many seconds (after a 3 s settle) and prints the mean and worst frame time
//! and the number of meshes drawn in the world, for comparing art passes.

use bevy::prelude::*;
use oz_sim::prelude::*;

pub struct FrameTimePlugin;

impl Plugin for FrameTimePlugin {
    fn build(&self, app: &mut App) {
        if std::env::var("OPNIX_FRAMETIME").is_ok() {
            app.add_systems(Last, probe);
        }
    }
}

#[derive(Default)]
struct Probe {
    clock: f32,
    frames: u32,
    worst: f32,
    done: bool,
}

fn probe(time: Res<Time<Real>>, state: Option<Res<MatchState>>, meshes: Query<(), With<Mesh3d>>, mut probe: Local<Probe>) {
    let Some(state) = state else { return };
    if probe.done || !matches!(state.phase, Phase::Live | Phase::Night) {
        return;
    }
    let span = std::env::var("OPNIX_FRAMETIME").ok().and_then(|s| s.parse().ok()).unwrap_or(10.0f32);
    let dt = time.delta_secs();
    probe.clock += dt;
    if probe.clock < 3.0 {
        return;
    }
    probe.frames += 1;
    probe.worst = probe.worst.max(dt);
    if probe.clock >= 3.0 + span {
        let mean = (probe.clock - 3.0) / probe.frames.max(1) as f32;
        eprintln!("FRAMETIME frames {} mean {:.1} ms worst {:.1} ms meshes {}", probe.frames, mean * 1000.0, probe.worst * 1000.0, meshes.iter().count());
        probe.done = true;
    }
}
