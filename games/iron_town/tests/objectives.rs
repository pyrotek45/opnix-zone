//! Headless objective-mode tests: domination and search and destroy, played out by bots,
//! by hand, and over a real host + guest connection.

use bevy::{prelude::*, state::app::StatesPlugin, time::TimeUpdateStrategy};
use iron_town::{IronTownPlugin, domination_rules, search_rules};
use oz_sim::{
    net,
    objective::{NEUTRAL, bomb, kind, stage},
    prelude::*,
};
use std::time::{Duration, Instant};

const FRAME: f32 = 1.0 / 30.0;

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, SimPlugin, IronTownPlugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(FRAME)));
    app
}

fn start(app: &mut App, setup: MatchSetup) {
    app.world_mut().resource_mut::<PendingMatch>().0 = Some(setup);
    for _ in 0..4 {
        app.update();
    }
    assert_eq!(*app.world().resource::<State<MatchPhase>>().get(), MatchPhase::Active);
}

fn lobby(app: &mut App, action: LobbyAction) {
    app.world_mut().send_event(CommandEvent { player: 0, command: PlayerCommand::Lobby(action) });
    app.update();
}

fn state(app: &App) -> MatchState {
    app.world().resource::<MatchState>().clone()
}

/// A match of bots only, as on a dedicated server.
fn bots_only(rules: Rules) -> MatchSetup {
    MatchSetup { local_player: None, ..MatchSetup::offline(rules, "SERVER") }
}

/// Starts the match and waits out the countdown.
fn kick_off(app: &mut App) {
    for _ in 0..20 {
        app.update();
    }
    lobby(app, LobbyAction::Start);
    for _ in 0..(6.0 / FRAME) as usize {
        app.update();
    }
    assert_eq!(state(app).phase, Phase::Live, "the countdown should have launched the match");
}

#[test]
fn bots_take_points_and_win_domination() {
    let mut app = app();
    let mut rules = domination_rules(5);
    rules.team_size = 3;
    rules.score_limit = 60;
    start(&mut app, bots_only(rules));
    for _ in 0..20 {
        app.update();
    }
    // The points are there in the lobby already: three of them, nobody's.
    let before = state(&app);
    assert_eq!(before.obj.kind, kind::DOMINATION);
    assert_eq!(before.obj.points.len(), 3);
    assert!(before.obj.points.iter().all(|p| p.owner == NEUTRAL));
    kick_off(&mut app);

    let mut seconds = 0.0;
    let mut captures = 0;
    let mut owners: Vec<u8> = vec![NEUTRAL; 3];
    let mut kills_seen = 0;
    while state(&app).phase != Phase::Won {
        app.update();
        seconds += FRAME;
        let now = state(&app);
        for (i, point) in now.obj.points.iter().enumerate() {
            if point.owner != owners[i] {
                owners[i] = point.owner;
                captures += 1;
            }
        }
        kills_seen = now.roster.iter().map(|r| r.kills as u32).sum::<u32>();
        assert!(seconds < 900.0, "bots never finished domination; scores {:?}, owners {owners:?}", now.scores);
    }
    let end = state(&app);
    println!("domination decided {:?} after {seconds:.0} s: {captures} captures, {kills_seen} kills", end.scores);
    assert_eq!(end.scores[end.winner as usize], 60);
    assert!(captures >= 2, "the points barely changed hands ({captures} captures)");
    // Points come from the flags: a team's score is not its kills.
    assert!(end.scores[0] + end.scores[1] >= 60);
}

#[test]
fn bots_play_search_and_destroy_rounds_to_a_winner() {
    let mut app = app();
    let mut rules = search_rules(9);
    rules.team_size = 3;
    rules.score_limit = 3;
    start(&mut app, bots_only(rules));
    kick_off(&mut app);
    let first = state(&app);
    assert_eq!((first.obj.kind, first.obj.round, first.obj.stage), (kind::BOMB, 1, stage::PLAYING));
    // The bomb waits in the attackers' base (someone may already have walked over it).
    assert!(matches!(first.obj.bomb, bomb::HOME | bomb::CARRIED));
    assert_eq!(first.obj.points.len(), 2, "two bomb sites");

    let mut seconds = 0.0;
    let mut round = 1;
    let mut alive = first.obj.alive;
    let mut picked_up = false;
    let mut planted = 0;
    let mut attackers_by_round = vec![first.obj.attackers];
    let mut reasons = Vec::new();
    let mut last_stage = stage::PLAYING;
    while state(&app).phase != Phase::Won {
        app.update();
        seconds += FRAME;
        let now = state(&app);
        let obj = &now.obj;
        if obj.round != round {
            round = obj.round;
            attackers_by_round.push(obj.attackers);
            alive = obj.alive;
            assert_eq!(obj.alive, [3, 3], "everyone is back for round {round}");
        } else if obj.stage == stage::PLAYING {
            // One life a round: nobody comes back until the next one.
            assert!(obj.alive[0] <= alive[0] && obj.alive[1] <= alive[1], "someone respawned mid-round: {alive:?} -> {:?}", obj.alive);
            alive = obj.alive;
        }
        picked_up |= obj.bomb == bomb::CARRIED;
        if obj.stage == stage::OVER && last_stage == stage::PLAYING {
            reasons.push(obj.last_reason.clone());
            if obj.last_reason != "ATTACKERS ELIMINATED" && obj.last_reason != "DEFENDERS ELIMINATED" && obj.last_reason != "TIME EXPIRED" {
                planted += 1;
            }
        }
        last_stage = obj.stage;
        assert!(seconds < 1200.0, "bots never finished search and destroy; scores {:?}, round {round}", now.scores);
    }
    let end = state(&app);
    println!("search and destroy decided {:?} after {seconds:.0} s over {round} rounds: {reasons:?}, attackers {attackers_by_round:?}", end.scores);
    assert_eq!(end.scores[end.winner as usize], 3);
    assert!(round >= 3);
    assert!(picked_up, "the attackers never picked the bomb up");
    // Sides swap at half time (after two rounds of a first-to-three).
    assert_eq!(attackers_by_round[0], attackers_by_round[1]);
    assert_ne!(attackers_by_round[1], attackers_by_round[2], "sides should have swapped");
    let _ = planted;
}

/// Makes the human unkillable and puts them somewhere.
fn hold(app: &mut App, at: Option<Vec3>, using: bool) {
    let world = app.world_mut();
    let (mut transform, mut health, mut input) = world.query_filtered::<(&mut Transform, &mut Health, &mut PlayerInput), With<LocalPlayer>>().single_mut(world).unwrap();
    health.hp = 5000.0;
    if let Some(at) = at {
        transform.translation = at;
    }
    input.interact = using;
    input.movement = Vec2::ZERO;
}

#[test]
fn a_player_plants_the_bomb_and_a_bot_defuses_it() {
    let mut app = app();
    let mut rules = search_rules(12);
    rules.team_size = 1;
    start(&mut app, MatchSetup::offline(rules, "TEST"));
    kick_off(&mut app);
    let obj = state(&app).obj;
    assert_eq!(obj.attackers, 0, "the human's team attacks first");
    // Walk over the bomb to take it.
    hold(&mut app, Some(obj.bomb_at), false);
    app.update();
    hold(&mut app, None, false);
    app.update();
    let obj = state(&app).obj;
    assert_eq!((obj.bomb, obj.carrier), (bomb::CARRIED, 0));
    // Plant at site B: hold Use for four seconds.
    let site = obj.points[1].at;
    let mut seconds = 0.0;
    while state(&app).obj.bomb != bomb::PLANTED {
        hold(&mut app, Some(site), true);
        app.update();
        seconds += FRAME;
        assert!(seconds < 6.0, "never planted: {:?}", state(&app).obj);
    }
    assert!(seconds > 3.5, "planting is not instant ({seconds:.1} s)");
    hold(&mut app, None, false);
    let obj = state(&app).obj;
    assert_eq!((obj.site, obj.carrier), (1, 0));
    // The planter goes down: with the bomb planted the round goes on, and the defending
    // bot comes to defuse it before it blows.
    let world = app.world_mut();
    world.query_filtered::<&mut Health, With<LocalPlayer>>().single_mut(world).unwrap().hp = 0.0;
    let mut seconds = 0.0;
    while state(&app).obj.stage == stage::PLAYING {
        app.update();
        seconds += FRAME;
        assert!(seconds < 60.0);
    }
    let end = state(&app);
    println!("round ended after {seconds:.0} s: {}", end.obj.last_reason);
    assert_eq!(end.obj.last_reason, "BOMB DEFUSED", "the bot should have defused it");
    assert_eq!(end.scores, [0, 1]);
}

#[test]
fn guests_see_the_objective_and_hold_use() {
    let (mut host, mut client) = (app(), app());
    let mut rules = domination_rules(31);
    rules.team_size = 2;
    start(&mut host, net::host_setup(rules, "HOST", "127.0.0.1:0", false).unwrap());
    let NetLink::Host(link) = host.world().resource::<NetLink>() else { panic!("not hosting") };
    let addr = link.host.local_addr().unwrap().to_string();
    let world = client.world_mut();
    net::join(&mut world.commands(), &addr, "GUEST");
    world.flush();
    let started = Instant::now();
    lobby(&mut host, LobbyAction::Start);
    loop {
        host.update();
        client.update();
        std::thread::sleep(Duration::from_millis(1));
        let joined = *client.world().resource::<State<MatchPhase>>().get() == MatchPhase::Active;
        if joined {
            // The guest holds Use; the host should see it in their input.
            let world = client.world_mut();
            if let Ok(mut input) = world.query_filtered::<&mut PlayerInput, With<LocalPlayer>>().single_mut(world) {
                input.interact = true;
            }
        }
        let using = host.world_mut().query::<(&Player, &PlayerInput)>().iter(host.world()).any(|(p, input)| p.id != 0 && p.id < 200 && input.interact);
        let (a, b) = (state(&host).obj, if joined { state(&client).obj } else { Default::default() });
        if joined && using && a.points.len() == 3 && a.points.iter().zip(&b.points).all(|(x, y)| x.at.distance(y.at) < 0.01 && x.owner == y.owner) && b.kind == kind::DOMINATION {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(20), "the guest never saw the points ({:?} vs {:?}) or the host never saw Use held ({using})", a.points, b.points);
    }
}
