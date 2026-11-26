//! Headless dedicated server: the same simulation, no window, no GPU.
//!
//! ```text
//! ember_server [--mode N] [--port PORT] [--seed SEED]
//! ```

use bevy::{app::ScheduleRunnerPlugin, log::LogPlugin, prelude::*, state::app::StatesPlugin};
use ember_atoll::EmberAtollPlugin;
use oz_net::DEFAULT_PORT;
use oz_sim::{net, prelude::*};
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let mode = value("--mode").and_then(|m| m.parse::<usize>().ok()).unwrap_or(0);
    let port = value("--port").and_then(|p| p.parse::<u16>().ok()).unwrap_or(DEFAULT_PORT);
    let seed = value("--seed").and_then(|s| s.parse::<u64>().ok()).unwrap_or(0x45eab1);

    let mut app = App::new();
    // 60 simulation ticks per second is plenty for a server and keeps CPU use low.
    app.add_plugins((MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(1.0 / 60.0))), LogPlugin::default(), StatesPlugin, SimPlugin, EmberAtollPlugin));

    let modes = app.world().resource::<Modes>();
    let Some(def) = modes.0.get(mode).filter(|def| def.playable) else {
        eprintln!("mode {mode} is not playable");
        return;
    };
    let mut rules = (def.rules)(seed);
    rules.mode = mode as u8;
    let name = def.name;
    match net::host_setup(rules, "SERVER", &format!("0.0.0.0:{port}"), true) {
        Ok(setup) => app.world_mut().resource_mut::<PendingMatch>().0 = Some(setup),
        Err(error) => {
            eprintln!("could not listen on port {port}: {error}");
            return;
        }
    }
    println!("Ember Atoll dedicated server - mode {name}, port {port}. Ctrl+C to stop.");
    app.add_systems(Update, (restart_when_lost, net::print_join_code.run_if(in_state(MatchPhase::Active)))).run();
}

/// A lost match restarts after a short pause so the server never sits dead.
fn restart_when_lost(time: Res<Time>, state: Option<Res<MatchState>>, phase: Res<State<MatchPhase>>, mut waited: Local<f32>) {
    if *phase.get() != MatchPhase::Active {
        return;
    }
    if state.is_some_and(|state| state.phase == Phase::Lost) {
        *waited += time.delta_secs();
        if *waited > 15.0 {
            println!("match lost; restart the server to begin a new one");
            std::process::exit(0);
        }
    }
}
