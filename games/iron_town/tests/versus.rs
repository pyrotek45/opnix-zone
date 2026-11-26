//! Headless versus tests: bots, team scoring, loadouts and a real host + client.

use bevy::{prelude::*, state::app::StatesPlugin, time::TimeUpdateStrategy};
use iron_town::{IronTownPlugin, deathmatch_rules};
use oz_sim::{net, prelude::*};
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

fn teams(app: &mut App) -> [(usize, usize); 2] {
    let mut out = [(0, 0); 2];
    for (player, bot) in app.world_mut().query::<(&Player, Has<Bot>)>().iter(app.world()) {
        if bot { out[player.team as usize].1 += 1 } else { out[player.team as usize].0 += 1 }
    }
    out
}

#[test]
fn bots_fill_the_teams_and_fight_to_the_score_limit() {
    let mut app = app();
    let mut rules = deathmatch_rules(4);
    rules.team_size = 3;
    rules.score_limit = 6;
    start(&mut app, MatchSetup::offline(rules, "TEST"));
    for _ in 0..20 {
        app.update();
    }
    // One human on team 0, bots everywhere else.
    assert_eq!(teams(&mut app), [(1, 2), (0, 3)]);

    let mut seconds = 0.0;
    while app.world().resource::<MatchState>().phase != Phase::Won {
        app.update();
        seconds += FRAME;
        assert!(seconds < 900.0, "bots never finished the match; scores {:?}", app.world().resource::<MatchState>().scores);
    }
    let state = app.world().resource::<MatchState>().clone();
    assert_eq!(state.scores[state.winner as usize], 6);
    println!("match decided {:?} after {seconds:.0} simulated seconds", state.scores);
}

#[test]
fn loadouts_apply_with_perks_and_free_attachments() {
    let mut app = app();
    start(&mut app, MatchSetup::offline(deathmatch_rules(8), "TEST"));
    let order = |app: &mut App, command| {
        app.world_mut().send_event(CommandEvent { player: 0, command });
        app.update();
    };
    // The client switches to its new primary as it sends the class.
    app.world_mut().query_filtered::<&mut PlayerInput, With<LocalPlayer>>().single_mut(app.world_mut()).unwrap().weapon = 2;
    order(&mut app, PlayerCommand::Loadout(Loadout { primary: 2, secondary: 0, perk: 1, gear: 1 }));
    order(&mut app, PlayerCommand::EquipPart { weapon: 2, slot: 1, variant: 2 });
    let world = app.world_mut();
    let (player, arsenal) = world.query_filtered::<(&Player, &Arsenal), With<LocalPlayer>>().single(world).unwrap();
    assert_eq!((arsenal.current, player.perk, player.gear, player.gear_left), (2, 1, 1, 2));
    let owned: Vec<usize> = (0..arsenal.weapons.len()).filter(|i| arsenal.owns(*i)).collect();
    assert_eq!(owned, vec![0, 2], "a class is exactly two weapons");
    assert_eq!(arsenal.weapons[2].equipped[1], 2, "attachments are free in versus");
    // Same weapon twice is not a class.
    order(&mut app, PlayerCommand::Loadout(Loadout { primary: 3, secondary: 3, perk: 0, gear: 0 }));
    let world = app.world_mut();
    assert_eq!(world.query_filtered::<&Arsenal, With<LocalPlayer>>().single(world).unwrap().current, 2);
}

#[test]
fn a_guest_joins_the_other_team_and_replaces_a_bot() {
    let (mut host, mut client) = (app(), app());
    let mut rules = deathmatch_rules(15);
    rules.team_size = 2;
    start(&mut host, net::host_setup(rules, "HOST", "127.0.0.1:0", false).unwrap());
    for _ in 0..20 {
        host.update();
    }
    assert_eq!(teams(&mut host), [(1, 1), (0, 2)]);
    let NetLink::Host(link) = &*host.world().resource::<NetLink>() else { panic!("not hosting") };
    let addr = link.host.local_addr().unwrap().to_string();

    let world = client.world_mut();
    net::join(&mut world.commands(), &addr, "GUEST");
    world.flush();
    let started = Instant::now();
    loop {
        host.update();
        client.update();
        std::thread::sleep(Duration::from_millis(1));
        let joined = *client.world().resource::<State<MatchPhase>>().get() == MatchPhase::Active;
        let local_team = client.world_mut().query_filtered::<&Player, With<LocalPlayer>>().single(client.world()).map(|p| p.team).ok();
        let seen = client.world_mut().query::<&Player>().iter(client.world()).count();
        if joined && teams(&mut host) == [(1, 1), (1, 1)] && local_team == Some(1) && seen == 4 {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(20), "guest never settled: host teams {:?}, guest team {local_team:?}, guest sees {seen}", teams(&mut host));
    }
}
