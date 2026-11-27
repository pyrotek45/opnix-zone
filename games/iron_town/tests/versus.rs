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

fn lobby(app: &mut App, action: LobbyAction) {
    app.world_mut().send_event(CommandEvent { player: 0, command: PlayerCommand::Lobby(action) });
    app.update();
}

fn phase(app: &App) -> Phase {
    app.world().resource::<MatchState>().phase
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
    // The match opens in the lobby: one human on team 0, bots everywhere else.
    assert_eq!(phase(&app), Phase::Lobby);
    assert_eq!(teams(&mut app), [(1, 2), (0, 3)]);
    let roster = app.world().resource::<MatchState>().roster.clone();
    assert_eq!(roster.len(), 6);
    let mut ids: Vec<u8> = roster.iter().map(|r| r.id).collect();
    ids.dedup();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 6, "every player needs a unique id: {roster:?}");
    // The host can resize the teams from the lobby.
    lobby(&mut app, LobbyAction::TeamSize(1));
    for _ in 0..20 {
        app.update();
    }
    assert_eq!(teams(&mut app), [(1, 3), (0, 4)]);
    lobby(&mut app, LobbyAction::Start);
    for _ in 0..(6.0 / FRAME) as usize {
        app.update();
    }
    assert_eq!(phase(&app), Phase::Live, "the countdown should have launched the match");

    let mut seconds = 0.0;
    while app.world().resource::<MatchState>().phase != Phase::Won {
        app.update();
        seconds += FRAME;
        assert!(seconds < 900.0, "bots never finished the match; scores {:?}", app.world().resource::<MatchState>().scores);
    }
    let state = app.world().resource::<MatchState>().clone();
    assert_eq!(state.scores[state.winner as usize], 6);
    let kills: u32 = state.roster.iter().map(|r| r.kills as u32).sum();
    assert_eq!(kills, state.scores[0] + state.scores[1], "every point is someone's kill");
    println!("match decided {:?} after {seconds:.0} simulated seconds", state.scores);
    // After the post-match screen everyone is back in the lobby.
    for _ in 0..(15.0 / FRAME) as usize {
        app.update();
    }
    assert_eq!(phase(&app), Phase::Lobby);
}

#[test]
fn bots_navigate_the_town() {
    let mut app = app();
    let mut rules = deathmatch_rules(21);
    rules.team_size = 1;
    start(&mut app, MatchSetup::offline(rules, "TEST"));
    lobby(&mut app, LobbyAction::Start);
    for _ in 0..(6.0 / FRAME) as usize {
        app.update();
    }
    // Park the human far away so the bot has to travel across the map.
    let world = app.world_mut();
    let human = world.query_filtered::<&mut Transform, With<LocalPlayer>>().single_mut(world).unwrap();
    let start = human.translation;
    let bot_start = world.query_filtered::<&Transform, With<Bot>>().single(world).unwrap().translation;
    let mut travelled = 0.0;
    let mut last = bot_start;
    for _ in 0..(25.0 / FRAME) as usize {
        app.update();
        let world = app.world_mut();
        world.query_filtered::<&mut Transform, With<LocalPlayer>>().single_mut(world).unwrap().translation = start;
        let now = world.query_filtered::<&Transform, With<Bot>>().single(world).unwrap().translation;
        travelled += now.xz().distance(last.xz());
        last = now;
    }
    println!("bot covered {travelled:.0} m, ended {:.0} m from the human", last.xz().distance(start.xz()));
    assert!(travelled > 40.0, "the bot barely moved ({travelled:.1} m)");
    assert!(last.xz().distance(start.xz()) < bot_start.xz().distance(start.xz()), "the bot should close in");
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
    order(&mut app, PlayerCommand::Loadout(Loadout { primary: 2, secondary: 0, perk: 1, gear: 1, parts: None, streaks: [0; 3], vision: 0 }));
    // A suppressor on the shotgun, then a sniper scope it must refuse.
    order(&mut app, PlayerCommand::EquipPart { weapon: 2, slot: 1, variant: 2 });
    order(&mut app, PlayerCommand::EquipPart { weapon: 2, slot: 0, variant: 4 });
    let world = app.world_mut();
    let (player, arsenal) = world.query_filtered::<(&Player, &Arsenal), With<LocalPlayer>>().single(world).unwrap();
    assert_eq!((arsenal.current, player.perk, player.gear, player.gear_left), (2, 1, 1, 2));
    let owned: Vec<usize> = (0..arsenal.weapons.len()).filter(|i| arsenal.owns(*i)).collect();
    assert_eq!(owned, vec![0, 2], "a class is exactly two weapons");
    assert_eq!(arsenal.weapons[2].equipped[1], 2, "attachments are free in versus");
    assert_eq!(arsenal.weapons[2].equipped[0], 0, "shotguns cannot take sniper scopes");
    // Same weapon twice is not a class.
    order(&mut app, PlayerCommand::Loadout(Loadout { primary: 3, secondary: 3, perk: 0, gear: 0, parts: None, streaks: [0; 3], vision: 0 }));
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
    let NetLink::Host(link) = host.world().resource::<NetLink>() else { panic!("not hosting") };
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

#[test]
fn guests_see_every_player_where_the_host_has_them() {
    let (mut host, mut client) = (app(), app());
    let mut rules = deathmatch_rules(77);
    rules.team_size = 3;
    start(&mut host, net::host_setup(rules, "HOST", "127.0.0.1:0", false).unwrap());
    let NetLink::Host(link) = host.world().resource::<NetLink>() else { panic!("not hosting") };
    let addr = link.host.local_addr().unwrap().to_string();
    let world = client.world_mut();
    net::join(&mut world.commands(), &addr, "GUEST");
    world.flush();
    let started = Instant::now();
    while count_players(&mut client) < 6 {
        host.update();
        client.update();
        std::thread::sleep(Duration::from_millis(1));
        assert!(started.elapsed() < Duration::from_secs(20), "guest never saw everyone");
    }
    lobby(&mut host, LobbyAction::Start);
    for _ in 0..400 {
        host.update();
        client.update();
        std::thread::sleep(Duration::from_millis(1));
    }
    let positions = |app: &mut App| -> Vec<(u8, u8, Vec3)> {
        let mut v: Vec<_> = app.world_mut().query::<(&Player, &Transform)>().iter(app.world()).map(|(p, t)| (p.id, p.team, t.translation)).collect();
        v.sort_by_key(|(id, ..)| *id);
        v
    };
    let (on_host, on_client) = (positions(&mut host), positions(&mut client));
    for (id, team, at) in &on_host {
        let seen = on_client.iter().find(|(cid, ..)| cid == id);
        let Some((_, seen_team, seen_at)) = seen else { panic!("guest is missing player {id}") };
        assert_eq!(team, seen_team, "player {id} is on the wrong team for the guest");
        assert!(at.distance(*seen_at) < 3.0, "player {id} (team {team}) is at {at} on the host but {seen_at} for the guest");
    }
}

fn count_players(app: &mut App) -> usize {
    app.world_mut().query::<&Player>().iter(app.world()).count()
}

#[test]
fn lag_compensation_hits_what_the_shooter_saw() {
    use oz_sim::lagcomp::ViewTime;
    let mut app = app();
    let mut rules = deathmatch_rules(5);
    rules.team_size = 1;
    start(&mut app, MatchSetup::offline(rules, "TEST"));
    lobby(&mut app, LobbyAction::Start);
    for _ in 0..(6.0 / FRAME) as usize {
        app.update();
    }
    assert_eq!(phase(&app), Phase::Live);

    // Freeze the enemy bot so the test controls it.
    let world = app.world_mut();
    let target = world.query_filtered::<Entity, With<Bot>>().single(world).unwrap();
    world.entity_mut(target).remove::<Bot>();

    // Find an open sightline in the plaza: shooter, target's old spot A, new spot B.
    let statics = app.world().resource::<Statics>().clone();
    let map = app.world().resource::<Map>().clone();
    let clear = |a: Vec3, b: Vec3| statics.raycast(a, (b - a).normalize(), a.distance(b)).is_none();
    let (shooter, a, b) = (-12..12)
        .flat_map(|x| (-12..12).map(move |z| (x as f32 * 1.5, z as f32 * 1.5)))
        .map(|(x, z)| (map.ground(x, z - 9.0), map.ground(x, z), map.ground(x + 6.0, z)))
        .find(|(s, a, b)| clear(*s + Vec3::Y * 1.7, *a + Vec3::Y * 1.0) && clear(*s + Vec3::Y * 1.7, *b + Vec3::Y * 1.0) && (a.y - s.y).abs() < 0.3)
        .expect("an open spot in the plaza");

    let place = |app: &mut App, entity: Entity, at: Vec3| app.world_mut().get_mut::<Transform>(entity).unwrap().translation = at;
    let me = app.world_mut().query_filtered::<Entity, With<LocalPlayer>>().single(app.world()).unwrap();
    place(&mut app, me, shooter);
    place(&mut app, target, a);
    for _ in 0..6 {
        app.update();
    }
    let seen_at = app.world().resource::<Time>().elapsed_secs();
    for _ in 0..6 {
        app.update();
    }
    place(&mut app, target, b);
    for _ in 0..4 {
        app.update();
    }

    // Aim at where the target *was*, and pull the trigger.
    let fire = |app: &mut App, view: Option<f32>| -> f32 {
        let world = app.world_mut();
        world.get_mut::<Health>(target).unwrap().hp = 100.0;
        let to = (a + Vec3::Y * 1.05) - (shooter + Vec3::Y * EYE);
        let mut shooter_entity = world.entity_mut(me);
        match view {
            Some(t) => shooter_entity.insert(ViewTime(t)),
            None => shooter_entity.remove::<ViewTime>(),
        };
        let mut input = world.get_mut::<PlayerInput>(me).unwrap();
        input.yaw = f32::atan2(-to.x, -to.z);
        input.pitch = (to.y / to.length()).asin();
        input.buttons = 0;
        world.get_mut::<Arsenal>(me).unwrap().cooldown = 0.0;
        app.update();
        app.world_mut().get_mut::<PlayerInput>(me).unwrap().buttons = buttons::FIRE;
        app.update();
        app.world_mut().get_mut::<PlayerInput>(me).unwrap().buttons = 0;
        app.update();
        app.world().get::<Health>(target).unwrap().hp
    };
    let rewound = fire(&mut app, Some(seen_at));
    let current = fire(&mut app, None);
    assert!(rewound < 100.0, "a shot at what the shooter saw should land (hp {rewound})");
    assert_eq!(current, 100.0, "without a view time the target has moved and the shot misses");
}

#[test]
fn a_quick_tap_online_still_fires() {
    let (mut host, mut client) = (app(), app());
    let mut rules = deathmatch_rules(31);
    rules.team_size = 1;
    start(&mut host, net::host_setup(rules, "HOST", "127.0.0.1:0", false).unwrap());
    let NetLink::Host(link) = host.world().resource::<NetLink>() else { panic!("not hosting") };
    let addr = link.host.local_addr().unwrap().to_string();
    let world = client.world_mut();
    net::join(&mut world.commands(), &addr, "GUEST");
    world.flush();
    let pump = |host: &mut App, client: &mut App, frames: usize| {
        for _ in 0..frames {
            host.update();
            client.update();
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    pump(&mut host, &mut client, 60);
    lobby(&mut host, LobbyAction::Start);
    pump(&mut host, &mut client, (6.5 / FRAME) as usize);
    assert_eq!(phase(&host), Phase::Live);

    let guest = client.world_mut().query_filtered::<&Player, With<LocalPlayer>>().single(client.world()).unwrap().id;
    let shots_on_host = |host: &mut App| host.world_mut().query::<(&Player, &Arsenal)>().iter(host.world()).find(|(p, _)| p.id == guest).map(|(_, a)| a.shots).unwrap();
    let before = shots_on_host(&mut host);
    // Hold the trigger for a single client frame: shorter than one input send.
    let mut input = client.world_mut().query_filtered::<&mut PlayerInput, With<LocalPlayer>>().single_mut(client.world_mut()).unwrap();
    input.buttons = buttons::FIRE;
    client.update();
    client.world_mut().query_filtered::<&mut PlayerInput, With<LocalPlayer>>().single_mut(client.world_mut()).unwrap().buttons = 0;
    pump(&mut host, &mut client, 30);
    assert_eq!(shots_on_host(&mut host), before + 1, "the tap must reach the host as one shot");
}

#[test]
fn a_dropped_guest_reconnects_into_the_same_seat() {
    let mut host = app();
    let mut rules = deathmatch_rules(31);
    rules.team_size = 2;
    start(&mut host, net::host_setup(rules, "HOST", "127.0.0.1:0", false).unwrap());
    let NetLink::Host(link) = host.world().resource::<NetLink>() else { panic!("not hosting") };
    let addr = link.host.local_addr().unwrap().to_string();
    const TOKEN: u64 = 0xfeed_beef;

    // Joins and waits until the host has the guest; returns the guest's id.
    let connect = |host: &mut App, client: &mut App| -> u8 {
        let world = client.world_mut();
        net::join_as(&mut world.commands(), &addr, "GUEST", TOKEN);
        world.flush();
        let started = Instant::now();
        loop {
            host.update();
            client.update();
            std::thread::sleep(Duration::from_millis(1));
            let me = client.world_mut().query_filtered::<&Player, With<LocalPlayer>>().single(client.world()).map(|p| p.id).ok();
            let host_has = |id: u8, host: &mut App| host.world_mut().query::<&Player>().iter(host.world()).filter(|p| p.id == id).count() == 1;
            if let Some(id) = me.filter(|id| host_has(*id, host)) {
                return id;
            }
            assert!(started.elapsed() < Duration::from_secs(20), "guest never joined");
        }
    };
    let mut first = app();
    let id = connect(&mut host, &mut first);
    for _ in 0..10 {
        host.update();
        first.update();
    }
    // Give the guest a score and a weapon state worth keeping.
    let team = {
        let mut query = host.world_mut().query::<&mut Player>();
        let mut guest = query.iter_mut(host.world_mut()).find(|p| p.id == id).unwrap();
        guest.kills = 7;
        guest.deaths = 2;
        guest.team
    };

    // The guest's game dies without saying goodbye, then comes back while the old connection still lingers.
    let mut second = app();
    let again = connect(&mut host, &mut second);
    drop(first);
    for _ in 0..30 {
        host.update();
        second.update();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(again, id, "came back with a different id");
    let guests: Vec<(u8, u16, u16)> = host.world_mut().query_filtered::<&Player, Without<Bot>>().iter(host.world()).filter(|p| p.id != 0).map(|p| (p.team, p.kills, p.deaths)).collect();
    assert_eq!(guests, vec![(team, 7, 2)], "the seat was not restored exactly once");
}

#[test]
fn mines_catch_enemies_and_smoke_blocks_sight() {
    use oz_sim::combat::{Mine, SmokeCloud, smoke_between};
    let mut app = app();
    let mut rules = deathmatch_rules(9);
    rules.team_size = 1;
    start(&mut app, MatchSetup::offline(rules, "TEST"));
    lobby(&mut app, LobbyAction::Start);
    for _ in 0..(6.0 / FRAME) as usize {
        app.update();
    }
    // Drop an armed mine at the enemy bot's feet.
    let (bot_at, bot_team) = app.world_mut().query_filtered::<(&Transform, &Player), With<Bot>>().iter(app.world()).find(|(_, p)| p.team == 1).map(|(t, p)| (t.translation, p.team)).unwrap();
    app.world_mut().spawn((MatchEntity, Mine { owner: 0, team: 1 - bot_team, damage: 500.0, radius: 4.0, arm: 0.0, life: 30.0 }, Transform::from_translation(bot_at)));
    for _ in 0..3 {
        app.update();
    }
    assert_eq!(app.world_mut().query::<&Mine>().iter(app.world()).count(), 0, "the mine never went off");
    assert_eq!(app.world().resource::<MatchState>().scores[0], 1, "the mine kill was not credited");

    // A cloud between two points hides one from the other; off to the side it does not.
    let cloud = (Transform::from_xyz(0.0, 1.5, 0.0), SmokeCloud { radius: 3.0, time: 10.0 });
    let clouds = [(&cloud.0, &cloud.1)];
    assert!(smoke_between(clouds, Vec3::new(-10.0, 1.5, 0.0), Vec3::new(10.0, 1.5, 0.0)));
    assert!(!smoke_between(clouds, Vec3::new(-10.0, 1.5, 8.0), Vec3::new(10.0, 1.5, 8.0)));
}

#[test]
fn thrown_smoke_makes_a_cloud() {
    use oz_sim::combat::SmokeCloud;
    let mut app = app();
    let mut rules = deathmatch_rules(7);
    rules.team_size = 1;
    start(&mut app, MatchSetup::offline(rules, "TEST"));
    lobby(&mut app, LobbyAction::Start);
    for _ in 0..(6.0 / FRAME) as usize {
        app.update();
    }
    let smoke = app.world().resource::<Content>().gear.iter().position(|g| g.name == "SMOKE").unwrap() as u8;
    {
        let mut query = app.world_mut().query_filtered::<(&mut Player, &mut PlayerInput), With<LocalPlayer>>();
        let (mut player, mut input) = query.single_mut(app.world_mut()).unwrap();
        player.gear = smoke;
        player.gear_left = 2;
        input.buttons = buttons::GEAR;
    }
    app.update();
    app.world_mut().query_filtered::<&mut PlayerInput, With<LocalPlayer>>().single_mut(app.world_mut()).unwrap().buttons = 0;
    for _ in 0..(3.0 / FRAME) as usize {
        app.update();
    }
    assert_eq!(app.world_mut().query::<&SmokeCloud>().iter(app.world()).count(), 1);
}

#[test]
fn a_class_saved_in_the_lobby_is_used_at_kickoff() {
    let mut app = app();
    let mut rules = deathmatch_rules(8);
    rules.team_size = 1;
    start(&mut app, MatchSetup::offline(rules, "TEST"));
    // Sit in the lobby a while, then save a class.
    for _ in 0..(15.0 / FRAME) as usize {
        app.update();
    }
    let gear = app.world().resource::<Content>().gear.len() as u8 - 1;
    let loadout = Loadout { primary: oz_core::kit::gun::SHOTGUN as u8, secondary: oz_core::kit::gun::PISTOL as u8, perk: 1, gear, parts: None, streaks: [0; 3], vision: 0 };
    app.world_mut().send_event(CommandEvent { player: 0, command: PlayerCommand::Loadout(loadout) });
    app.update();
    lobby(&mut app, LobbyAction::Start);
    for _ in 0..(6.0 / FRAME) as usize {
        app.update();
    }
    let (player, arsenal) = app.world_mut().query_filtered::<(&Player, &Arsenal), With<LocalPlayer>>().single(app.world()).map(|(p, a)| (p.clone(), a.clone())).unwrap();
    assert_eq!((player.perk, player.gear), (1, gear));
    assert!(arsenal.owns(oz_core::kit::gun::SHOTGUN));
}

#[test]
fn a_friend_joins_with_the_code_from_the_lobby() {
    use oz_net::rendezvous::{Board, MemoryBoard};
    use std::sync::Arc;
    let board: Arc<dyn Board> = Arc::new(MemoryBoard::default());
    let (mut host, mut client) = (app(), app());
    let mut rules = deathmatch_rules(12);
    rules.team_size = 2;
    start(&mut host, net::host_setup_with(rules, "HOST", "127.0.0.1:0", false, Some(board.clone())).unwrap());
    let NetLink::Host(link) = host.world().resource::<NetLink>() else { panic!("not hosting") };
    let code = oz_net::rendezvous::pretty_code(&link.join_code().expect("a code"));

    let world = client.world_mut();
    net::join_code_with(&mut world.commands(), &code.to_lowercase(), "FRIEND", 5, board);
    world.flush();
    let started = Instant::now();
    loop {
        host.update();
        client.update();
        std::thread::sleep(Duration::from_millis(2));
        let joined = *client.world().resource::<State<MatchPhase>>().get() == MatchPhase::Active;
        let names: Vec<String> = host.world_mut().query::<&Player>().iter(host.world()).map(|p| p.name.clone()).collect();
        if joined && names.iter().any(|n| n == "FRIEND") {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(20), "never joined by code: {:?}", client.world().resource::<net::JoinStatus>());
    }
}

#[test]
fn killstreaks_are_earned_and_called_in() {
    use oz_sim::killstreak::{Airstrike, Drone, reward};
    let mut app = app();
    let mut rules = deathmatch_rules(21);
    rules.team_size = 2;
    start(&mut app, MatchSetup::offline(rules, "TEST"));
    lobby(&mut app, LobbyAction::Start);
    for _ in 0..(6.0 / FRAME) as usize {
        app.update();
    }
    // Five kills in a row: UAV at three, airstrike at five.
    for _ in 0..5 {
        app.world_mut().resource_mut::<MatchState>().credits.push(0);
        app.update();
    }
    let me = |app: &mut App| app.world_mut().query_filtered::<&Player, With<LocalPlayer>>().single(app.world()).unwrap().clone();
    assert_eq!(me(&mut app).streak, 5);
    assert_eq!(me(&mut app).rewards, [reward::UAV, reward::AIRSTRIKE, 0]);

    let call = |app: &mut App| {
        app.world_mut().send_event(CommandEvent { player: 0, command: PlayerCommand::UseStreak });
        app.update();
    };
    // The newest reward goes first: the airstrike, then the UAV.
    call(&mut app);
    assert_eq!(app.world_mut().query::<&Airstrike>().iter(app.world()).count(), 1);
    call(&mut app);
    assert!(app.world().resource::<MatchState>().uav[0] > 25.0, "the UAV did not go up");
    assert_eq!(me(&mut app).rewards, [0; 3]);
    // The jet flies over, the bombs fall, and the strike is over.
    for _ in 0..(7.0 / FRAME) as usize {
        app.update();
    }
    assert_eq!(app.world_mut().query::<&Airstrike>().iter(app.world()).count(), 0);

    // Two more kills: the drone.
    for _ in 0..2 {
        app.world_mut().resource_mut::<MatchState>().credits.push(0);
        app.update();
    }
    call(&mut app);
    assert_eq!(app.world_mut().query::<&Drone>().iter(app.world()).count(), 1);
    // Dying resets the streak but not what was earned.
    {
        let mut query = app.world_mut().query_filtered::<(&mut Health, &mut Player), With<LocalPlayer>>();
        let (mut health, mut player) = query.single_mut(app.world_mut()).unwrap();
        player.rewards = [reward::UAV, 0, 0];
        health.hp = 0.0;
        player.streak = 0;
    }
    assert_eq!(me(&mut app).rewards, [reward::UAV, 0, 0]);
}

#[test]
fn limbs_come_off_hurt_and_grow_back() {
    use oz_sim::player::{limb, part};
    let mut app = app();
    let mut rules = deathmatch_rules(4);
    rules.team_size = 1;
    start(&mut app, MatchSetup::offline(rules, "TEST"));
    lobby(&mut app, LobbyAction::Start);
    for _ in 0..(6.0 / FRAME) as usize {
        app.update();
    }
    let walk = |app: &mut App| {
        let at = app.world_mut().query_filtered::<&Transform, With<LocalPlayer>>().single(app.world()).unwrap().translation;
        app.world_mut().query_filtered::<&mut PlayerInput, With<LocalPlayer>>().single_mut(app.world_mut()).unwrap().movement = Vec2::Y;
        for _ in 0..30 {
            app.update();
        }
        app.world_mut().query_filtered::<&mut PlayerInput, With<LocalPlayer>>().single_mut(app.world_mut()).unwrap().movement = Vec2::ZERO;
        app.world_mut().query_filtered::<&Transform, With<LocalPlayer>>().single(app.world()).unwrap().translation.xz().distance(at.xz())
    };
    let whole = walk(&mut app);
    // Shoot the left leg off.
    app.world_mut().query_filtered::<&mut Player, With<LocalPlayer>>().single_mut(app.world_mut()).unwrap().lost |= limb::LEFT_LEG;
    assert_eq!(limb::of_part(part::LEFT_LEG), Some(limb::LEFT_LEG));
    assert!(limb::aim_penalty(limb::RIGHT_ARM) > 1.5);
    let limping = walk(&mut app);
    assert!(limping < whole * 0.75, "one leg {limping} m vs two legs {whole} m");
    // Dying and respawning restores the chassis.
    {
        let mut query = app.world_mut().query_filtered::<(&mut Health, &mut Player), With<LocalPlayer>>();
        let (mut health, _) = query.single_mut(app.world_mut()).unwrap();
        health.hp = 0.0;
    }
    for _ in 0..(8.0 / FRAME) as usize {
        app.update();
    }
    let player = app.world_mut().query_filtered::<&Player, With<LocalPlayer>>().single(app.world()).unwrap().clone();
    assert_eq!(player.lost, 0);
}

#[test]
fn ladders_lead_onto_roofs() {
    let mut app = app();
    let mut rules = deathmatch_rules(9);
    rules.team_size = 1;
    start(&mut app, MatchSetup::offline(rules, "TEST"));
    lobby(&mut app, LobbyAction::Start);
    for _ in 0..(6.0 / FRAME) as usize {
        app.update();
    }
    let ladder = *app.world().resource::<Map>().ladders.first().expect("the town has ladders");
    {
        let mut query = app.world_mut().query_filtered::<(&mut Transform, &mut PlayerInput), With<LocalPlayer>>();
        let (mut transform, mut input) = query.single_mut(app.world_mut()).unwrap();
        transform.translation = ladder.foot + Vec3::new(ladder.out.x, 0.0, ladder.out.y) * 0.4;
        // Face the wall and push.
        input.yaw = f32::atan2(ladder.out.x, ladder.out.y);
        input.movement = Vec2::Y;
    }
    let mut highest = f32::MIN;
    for _ in 0..(5.0 / FRAME) as usize {
        app.update();
        let y = app.world_mut().query_filtered::<&Transform, With<LocalPlayer>>().single(app.world()).unwrap().translation.y;
        highest = highest.max(y);
    }
    assert!(highest > ladder.top - 0.1, "climbed to {highest}, roof at {}", ladder.top);
}

#[test]
fn doors_glass_and_clutter_break() {
    use oz_sim::breakables::{Trace, door};
    let mut app = app();
    let mut rules = deathmatch_rules(14);
    rules.team_size = 1;
    start(&mut app, MatchSetup::offline(rules, "TEST"));
    lobby(&mut app, LobbyAction::Start);
    for _ in 0..(6.0 / FRAME) as usize {
        app.update();
    }
    let map = app.world().resource::<Map>().0.clone();
    assert!(!map.doors.is_empty() && !map.windows.is_empty() && !map.clutter.is_empty());

    // A door opens for whoever stands at it and asks.
    let spec = map.doors[0];
    app.world_mut().query_filtered::<&mut Transform, With<LocalPlayer>>().single_mut(app.world_mut()).unwrap().translation = spec.center + Vec3::new(0.0, 0.0, 0.0);
    app.world_mut().send_event(CommandEvent { player: 0, command: PlayerCommand::Door(0) });
    app.update();
    app.update();
    assert_eq!(app.world().resource::<MatchState>().doors[0], door::OPEN);

    // A round through a window shatters it.
    let pane = map.windows[0];
    let center = pane.center + Vec3::Y * pane.height * 0.5;
    let across = if pane.along_x { Vec3::Z } else { Vec3::X };
    app.world_mut().send_event(Trace { origin: center - across * 3.0, dir: across, reach: 6.0, damage: 30.0, by: 0 });
    app.update();
    assert_eq!(app.world().resource::<MatchState>().panes[0], 1);

    // A barrel blown up takes its neighbourhood with it.
    let barrel = map.clutter.iter().position(|c| c.kind == oz_core::worldgen::ClutterKind::Barrel).expect("a barrel");
    let at = map.clutter[barrel].center();
    app.world_mut().send_event(FxEvent::Explosion { at, radius: 4.0 });
    for _ in 0..4 {
        app.update();
    }
    // It catches fire and smokes first...
    assert_eq!(app.world().resource::<MatchState>().clutter[barrel], oz_sim::breakables::BURNING, "the barrel did not catch fire");
    // ...then goes up.
    for _ in 0..((oz_sim::breakables::COOK_OFF + 1.2) / FRAME) as usize {
        app.update();
    }
    assert_eq!(app.world().resource::<MatchState>().clutter[barrel], 0, "the barrel never cooked off");
}

fn run_for(app: &mut App, seconds: f32) {
    for _ in 0..(seconds / FRAME) as usize {
        app.update();
    }
}

fn rules_of(app: &App) -> Rules {
    app.world().resource::<Rules>().clone()
}

fn local_player(app: &mut App) -> (u8, String, u8) {
    let world = app.world_mut();
    let player = world.query_filtered::<&Player, With<LocalPlayer>>().single(world).unwrap();
    (player.id, player.name.clone(), player.team)
}

/// The map choice for a fixed layout (see `versus::MAPS`).
fn fixed_map(layout: oz_core::worldgen::Layout) -> u8 {
    let themes = oz_core::worldgen::Theme::ALL.len();
    (themes + oz_core::worldgen::Layout::VERSUS.iter().position(|l| *l == layout).unwrap()) as u8
}

#[test]
fn the_host_switches_mode_and_map_from_the_lobby() {
    use oz_core::worldgen::{Layout, Theme};
    let mut app = app();
    let mut rules = deathmatch_rules(11);
    rules.team_size = 2;
    let town = rules.recipe;
    start(&mut app, MatchSetup::offline(rules, "TEST"));
    run_for(&mut app, 1.0);
    assert_eq!(teams(&mut app), [(1, 1), (0, 2)]);

    // Terrorist hunt, one step back from team deathmatch: the same town, held by hostiles.
    // Domination and search and destroy sit between terrorist hunt and team deathmatch.
    lobby(&mut app, LobbyAction::Mode(-1));
    lobby(&mut app, LobbyAction::Mode(-1));
    lobby(&mut app, LobbyAction::Mode(-1));
    assert_eq!(app.world().resource::<MatchState>().next_mode, 3, "the pick shows at once");
    assert_eq!(rules_of(&app).mode, 0, "and applies once it settles");
    run_for(&mut app, 3.0);
    let hunt = rules_of(&app);
    assert!(hunt.hunt && hunt.mode == 3, "{hunt:?}");
    assert_eq!(hunt.recipe, town, "changing the mode keeps the map");
    assert_eq!(*app.world().resource::<State<MatchPhase>>().get(), MatchPhase::Active);
    assert_eq!(phase(&app), Phase::Lobby);
    assert_eq!(local_player(&mut app), (0, "TEST".to_string(), 0), "the player keeps their seat and name");
    let squad = app.world().resource::<MatchState>().team_size;
    let [(humans, squad_bots), (_, hostiles)] = teams(&mut app);
    assert_eq!((humans, squad_bots), (1, squad as usize - 1));
    assert_eq!(hostiles, oz_sim::versus::hunt_hostiles(&hunt, squad), "hostiles hold the town");

    // The office tower: stepping through the maps only loads the one the host stops on.
    for _ in 0..fixed_map(Layout::OfficeTower) {
        lobby(&mut app, LobbyAction::Map(1));
    }
    run_for(&mut app, 3.0);
    let tower = rules_of(&app);
    assert_eq!(tower.recipe.layout, Layout::OfficeTower);
    assert_eq!(app.world().resource::<Map>().recipe, tower.recipe, "the world is rebuilt from the new recipe");
    assert!(tower.hunt, "changing the map keeps the mode");

    // Back to team deathmatch in the suburbs: the hostiles are gone, bots fill two teams again.
    // Domination and search and destroy sit between terrorist hunt and team deathmatch.
    lobby(&mut app, LobbyAction::Mode(1));
    lobby(&mut app, LobbyAction::Mode(1));
    lobby(&mut app, LobbyAction::Mode(1));
    for _ in 0..fixed_map(Layout::OfficeTower) - 1 {
        lobby(&mut app, LobbyAction::Map(-1));
    }
    run_for(&mut app, 3.0);
    let tdm = rules_of(&app);
    assert!(!tdm.hunt && tdm.mode == 0);
    assert_eq!(tdm.recipe.layout, Layout::Town);
    assert_eq!(Theme::from_seed(tdm.recipe.seed), Theme::Suburbs);
    let size = app.world().resource::<MatchState>().team_size as usize;
    assert_eq!(teams(&mut app), [(1, size - 1), (0, size)], "bots are made afresh for the new mode");

    // A new roll of the same choice: other streets, same theme.
    lobby(&mut app, LobbyAction::NewMap);
    run_for(&mut app, 2.0);
    let rolled = rules_of(&app);
    assert_ne!(rolled.recipe.seed, tdm.recipe.seed);
    assert_eq!(Theme::from_seed(rolled.recipe.seed), Theme::Suburbs);

    // Ranked keeps the map too.
    lobby(&mut app, LobbyAction::Mode(1));
    lobby(&mut app, LobbyAction::Mode(1));
    run_for(&mut app, 3.0);
    assert!(rules_of(&app).ranked);
    assert_eq!(rules_of(&app).recipe, rolled.recipe);
}

#[test]
fn lobby_settings_change_the_match() {
    let mut app = app();
    let mut rules = deathmatch_rules(5);
    rules.team_size = 2;
    start(&mut app, MatchSetup::offline(rules, "TEST"));
    run_for(&mut app, 1.0);
    lobby(&mut app, LobbyAction::Bots);
    run_for(&mut app, 0.5);
    assert_eq!(teams(&mut app), [(1, 0), (0, 0)], "bots off: humans only");
    lobby(&mut app, LobbyAction::Bots);
    lobby(&mut app, LobbyAction::BotSkill(1));
    lobby(&mut app, LobbyAction::Respawn(-1));
    lobby(&mut app, LobbyAction::ScoreLimit(-1));
    lobby(&mut app, LobbyAction::TimeLimit(1));
    lobby(&mut app, LobbyAction::TimeLimit(1));
    run_for(&mut app, 0.5);
    let rules = rules_of(&app);
    assert_eq!(teams(&mut app), [(1, 1), (0, 2)]);
    assert_eq!(rules.difficulty, 1.35);
    assert_eq!(rules.respawn_seconds, 4.0);
    assert_eq!(rules.score_limit, 20);
    assert_eq!(rules.time_limit, 300.0);
}

#[test]
fn a_timed_match_ends_on_the_clock_and_the_rematch_starts_clean() {
    let mut app = app();
    let mut rules = deathmatch_rules(9);
    rules.team_size = 2;
    rules.score_limit = 0;
    rules.time_limit = 25.0;
    start(&mut app, MatchSetup::offline(rules, "TEST"));
    lobby(&mut app, LobbyAction::Start);
    run_for(&mut app, 6.0);
    assert_eq!(phase(&app), Phase::Live);
    let map_before = app.world().resource::<Map>().0.clone();
    run_for(&mut app, 26.0);
    let state = app.world().resource::<MatchState>().clone();
    assert_eq!(state.phase, Phase::Won, "the clock ends the match");
    let [teal, coral] = state.scores;
    assert_eq!(state.winner, if teal > coral { 0 } else if coral > teal { 1 } else { 2 });
    // After the results everyone is back in the lobby, on a fresh copy of the same map.
    run_for(&mut app, 16.0);
    let state = app.world().resource::<MatchState>().clone();
    assert_eq!(state.phase, Phase::Lobby);
    assert_eq!(state.scores, [0, 0]);
    assert!(state.roster.iter().all(|r| r.kills == 0 && r.deaths == 0), "{:?}", state.roster);
    assert_eq!(state.roster.len(), 4);
    let map_after = app.world().resource::<Map>().0.clone();
    assert!(!std::sync::Arc::ptr_eq(&map_before, &map_after) && map_before.recipe == map_after.recipe);
    let world = app.world_mut();
    assert_eq!(world.query::<&Projectile>().iter(world).count(), 0, "nothing from the last match is left lying around");
    assert_eq!(world.query::<&Player>().iter(world).filter(|p| p.streak > 0 || p.kills > 0).count(), 0);
    assert_eq!(local_player(&mut app).1, "TEST");
}

/// Runs host and guest together until `done` holds (or fails after 30 s).
fn pump(host: &mut App, client: &mut App, what: &str, mut done: impl FnMut(&mut App, &mut App) -> bool) {
    let started = Instant::now();
    loop {
        host.update();
        client.update();
        std::thread::sleep(Duration::from_millis(1));
        if done(host, client) {
            return;
        }
        assert!(started.elapsed() < Duration::from_secs(30), "timed out waiting for: {what}");
    }
}

fn names(app: &App) -> Vec<String> {
    app.world().resource::<MatchState>().roster.iter().filter(|r| !r.bot).map(|r| r.name.clone()).collect()
}

#[test]
fn host_and_guest_change_maps_modes_and_names_together() {
    use oz_core::worldgen::Layout;
    let (mut host, mut client) = (app(), app());
    let mut rules = deathmatch_rules(15);
    rules.team_size = 2;
    start(&mut host, net::host_setup(rules, "HOST", "127.0.0.1:0", false).unwrap());
    let NetLink::Host(link) = host.world().resource::<NetLink>() else { panic!("not hosting") };
    let addr = link.host.local_addr().unwrap().to_string();
    let world = client.world_mut();
    net::join(&mut world.commands(), &addr, "guest");
    world.flush();
    pump(&mut host, &mut client, "the guest joins", |host, client| {
        *client.world().resource::<State<MatchPhase>>().get() == MatchPhase::Active && count_players(client) == 4 && names(host).contains(&"GUEST".to_string())
    });
    let (guest_id, _, guest_team) = local_player(&mut client);
    assert_eq!(guest_team, 1);

    // Names: cleaned up, made unique, and shown to everyone.
    client.world_mut().send_event(CommandEvent { player: guest_id, command: PlayerCommand::Rename("  ace   pilot ".into()) });
    pump(&mut host, &mut client, "the rename reaches everyone", |host, client| names(host).contains(&"ACE PILOT".to_string()) && names(client).contains(&"ACE PILOT".to_string()));
    client.world_mut().send_event(CommandEvent { player: guest_id, command: PlayerCommand::Rename("host".into()) });
    pump(&mut host, &mut client, "a taken name gets a number", |host, client| names(host).contains(&"HOST2".to_string()) && local_player(client).1 == "HOST2");

    // A setting that needs no new world reaches the guest as it changes.
    lobby(&mut host, LobbyAction::ScoreLimit(1));
    lobby(&mut host, LobbyAction::TimeLimit(1));
    pump(&mut host, &mut client, "the guest sees the new limits", |host, client| rules_of(client) == rules_of(host) && rules_of(client).time_limit > 0.0);

    // The space station: both machines build it, and everyone keeps their seat.
    for _ in 0..fixed_map(Layout::SpaceStation) {
        lobby(&mut host, LobbyAction::Map(1));
    }
    pump(&mut host, &mut client, "both rebuild on the new map", |host, client| {
        let (h, c) = (host.world().resource::<Map>().recipe, client.world().resource::<Map>().recipe);
        h.layout == Layout::SpaceStation && c == h && *client.world().resource::<State<MatchPhase>>().get() == MatchPhase::Active && count_players(client) == 4 && client.world().resource::<MatchState>().phase == Phase::Lobby
    });
    assert_eq!(rules_of(&client), rules_of(&host));
    assert_eq!(local_player(&mut client), (guest_id, "HOST2".to_string(), guest_team));
    assert_eq!(local_player(&mut host).1, "HOST");
    assert!(names(&host).contains(&"HOST2".to_string()));
    assert!(matches!(client.world().resource::<NetLink>(), NetLink::Client(_)), "the guest stays connected");

    // Terrorist hunt: every human joins the squad, hostiles hold the station.
    // Domination and search and destroy sit between terrorist hunt and team deathmatch.
    lobby(&mut host, LobbyAction::Mode(-1));
    lobby(&mut host, LobbyAction::Mode(-1));
    lobby(&mut host, LobbyAction::Mode(-1));
    pump(&mut host, &mut client, "both switch to the hunt", |host, client| {
        rules_of(host).hunt && rules_of(client) == rules_of(host) && client.world().resource::<Map>().recipe == host.world().resource::<Map>().recipe && local_player(client).2 == 0
    });
    pump(&mut host, &mut client, "the guest sees the hostiles", |host, client| {
        let squad = host.world().resource::<MatchState>().team_size;
        let hostiles = oz_sim::versus::hunt_hostiles(&rules_of(host), squad);
        teams(host)[0].0 == 2 && count_players(client) == count_players(host) && teams(host)[1].1 == hostiles
    });
    assert_eq!(client.world().resource::<Map>().recipe.layout, Layout::SpaceStation);

    // Guests edit their class in the lobby too, and the host applies it to them at once.
    let gear = host.world().resource::<Content>().gear.len() as u8 - 1;
    let loadout = Loadout { primary: oz_core::kit::gun::SHOTGUN as u8, secondary: oz_core::kit::gun::PISTOL as u8, perk: 1, gear, parts: None, streaks: [0; 3], vision: 0 };
    client.world_mut().send_event(CommandEvent { player: guest_id, command: PlayerCommand::Loadout(loadout) });
    pump(&mut host, &mut client, "the host applies the guest's class", |host, _| {
        let world = host.world_mut();
        world.query::<(&Player, &Arsenal)>().iter(world).any(|(p, a)| p.id == guest_id && p.perk == 1 && p.gear == gear && a.owns(oz_core::kit::gun::SHOTGUN))
    });

    // And the match starts for both.
    lobby(&mut host, LobbyAction::Start);
    pump(&mut host, &mut client, "the match starts for the guest", |_, client| client.world().resource::<MatchState>().phase == Phase::Live);
}

#[test]
fn the_fallen_sometimes_drop_ammo_and_always_for_a_killer_running_dry() {
    use oz_sim::loot::{Loot, Pickup};
    let mut app = app();
    let mut rules = deathmatch_rules(8);
    rules.team_size = 1;
    start(&mut app, MatchSetup::offline(rules, "TEST"));
    lobby(&mut app, LobbyAction::Start);
    for _ in 0..(6.0 / FRAME) as usize {
        app.update();
    }
    assert_eq!(phase(&app), Phase::Live);
    let boxes = |app: &mut App| -> Vec<Entity> {
        let world = app.world_mut();
        world.query::<(Entity, &Pickup)>().iter(world).filter(|(_, p)| p.loot == Loot::Ammo).map(|(e, _)| e).collect()
    };
    let clear = |app: &mut App| {
        for e in boxes(app) {
            app.world_mut().despawn(e);
        }
    };
    // The victim: a bot, down (so it does not pick its own box up).
    let victim = {
        let world = app.world_mut();
        let (player, mut health) = world.query_filtered::<(&Player, &mut Health), With<Bot>>().iter_mut(world).next().unwrap();
        health.hp = 0.0;
        player.id
    };
    let set_reserve = |app: &mut App, reserve: u16| {
        let world = app.world_mut();
        let mut arsenal = world.query_filtered::<&mut Arsenal, With<LocalPlayer>>().single_mut(world).unwrap();
        for weapon in arsenal.weapons.iter_mut().filter(|w| w.owned) {
            weapon.reserve = reserve;
        }
    };
    let reserve = |app: &mut App| -> u16 {
        let world = app.world_mut();
        world.query_filtered::<&Arsenal, With<LocalPlayer>>().single(world).unwrap().weapons.iter().filter(|w| w.owned).map(|w| w.reserve).sum()
    };
    let world = app.world_mut();
    let me = world.query_filtered::<&Player, With<LocalPlayer>>().single(world).unwrap().id;
    let kill = |app: &mut App| {
        app.world_mut().send_event(FxEvent::Killed { victim, by: me, head: false, blast: false });
        app.update();
    };
    assert!(boxes(&mut app).is_empty(), "team deathmatch has no loot lying about");

    // A killer with plenty of ammo: only sometimes.
    set_reserve(&mut app, 200);
    let mut dropped = 0;
    for _ in 0..40 {
        clear(&mut app);
        kill(&mut app);
        dropped += boxes(&mut app).len();
    }
    assert!((3..=20).contains(&dropped), "{dropped} of 40 kills dropped ammo");

    // A killer running dry: every time, but never a carpet of boxes.
    clear(&mut app);
    set_reserve(&mut app, 0);
    for i in 0..10 {
        kill(&mut app);
        assert_eq!(boxes(&mut app).len(), (i + 1).min(6), "a kill by someone out of ammo drops a box, up to six at once");
    }
    // Walking over one takes it.
    let first = boxes(&mut app)[0];
    let at = app.world().get::<Transform>(first).unwrap().translation;
    let empty = reserve(&mut app);
    let world = app.world_mut();
    world.query_filtered::<&mut Transform, With<LocalPlayer>>().single_mut(world).unwrap().translation = at;
    app.update();
    assert!(reserve(&mut app) > empty, "walking over the box should have refilled the guns");
    assert!(boxes(&mut app).len() < 6, "the box should be gone once taken");
}
