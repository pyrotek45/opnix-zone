//! Headless Iron Town server: the full versus match with no window or GPU.
//! Anyone can join with the normal client (`iron_town --join ADDRESS`).
//!
//! ```text
//! iron_server [--teams N] [--score N] [--port PORT] [--seed SEED] [--lobby SECONDS]
//! ```
//! With no host player in the room, the server starts each match itself after
//! `--lobby` seconds (default 20), giving people time to join and pick a class.

use bevy::{app::ScheduleRunnerPlugin, log::LogPlugin, prelude::*, state::app::StatesPlugin};
use iron_town::{IronTownPlugin, deathmatch_rules};
use oz_net::DEFAULT_PORT;
use oz_sim::{net, prelude::*};
use std::time::Duration;

#[derive(Resource)]
struct LobbyWait(f32);

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let number = |flag: &str| value(flag).and_then(|v| v.parse::<u64>().ok());
    let port = number("--port").map_or(DEFAULT_PORT, |p| p as u16);
    let mut rules = deathmatch_rules(number("--seed").unwrap_or(0x1205));
    rules.team_size = number("--teams").map_or(4, |t| t.clamp(1, 6) as u8);
    rules.score_limit = number("--score").map_or(rules.score_limit, |s| s as u32);
    let wait = number("--lobby").map_or(20.0, |s| s as f32);

    let mut app = App::new();
    app.add_plugins((MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(1.0 / 60.0))), LogPlugin::default(), StatesPlugin, SimPlugin, IronTownPlugin));
    match net::host_setup(rules.clone(), "SERVER", &format!("0.0.0.0:{port}"), true) {
        Ok(setup) => app.world_mut().resource_mut::<PendingMatch>().0 = Some(setup),
        Err(error) => {
            eprintln!("could not listen on port {port}: {error}");
            return;
        }
    }
    println!("Iron Town server - {n}v{n}, first to {}, port {port}. Ctrl+C to stop.", rules.score_limit, n = rules.team_size);
    app.insert_resource(LobbyWait(wait)).add_systems(Update, (auto_start, report, net::print_join_code).run_if(in_state(MatchPhase::Active))).run();
}

/// Stands in for the host: launches the match once the lobby has waited long enough.
fn auto_start(time: Res<Time>, wait: Res<LobbyWait>, state: Res<MatchState>, mut waited: Local<f32>, mut orders: EventWriter<CommandEvent>) {
    if state.phase != Phase::Lobby || state.countdown > 0.0 {
        *waited = 0.0;
        return;
    }
    *waited += time.delta_secs();
    if *waited >= wait.0 {
        orders.write(CommandEvent { player: 0, command: PlayerCommand::Lobby(LobbyAction::Start) });
    }
}

/// Prints the scoreline and who is connected, so a server log tells the story of a match.
fn report(time: Res<Time>, state: Res<MatchState>, link: Res<NetLink>, mut last: Local<f32>, mut phase: Local<Option<Phase>>) {
    if *phase != Some(state.phase) {
        *phase = Some(state.phase);
        println!("[{:>5.0}s] phase {:?}", time.elapsed_secs(), state.phase);
    }
    if time.elapsed_secs() - *last < 10.0 {
        return;
    }
    *last = time.elapsed_secs();
    let humans: Vec<&str> = state.roster.iter().filter(|r| !r.bot).map(|r| r.name.as_str()).collect();
    let best = state.roster.iter().max_by_key(|r| r.kills).map_or(String::new(), |r| format!("  top: {} {}/{}", r.name, r.kills, r.deaths));
    println!("[{:>5.0}s] TEAL {} - {} CORAL   guests {} {:?}{best}", time.elapsed_secs(), state.scores[0], state.scores[1], link.remote_players(), humans);
}
