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
    for _ in 0..(10.0 / FRAME) as usize {
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
    order(&mut app, PlayerCommand::Loadout(Loadout { primary: 2, secondary: 0, perk: 1, gear: 1 }));
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
    let loadout = Loadout { primary: oz_core::kit::gun::SHOTGUN as u8, secondary: oz_core::kit::gun::PISTOL as u8, perk: 1, gear };
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
    assert_eq!(me(&mut app).rewards, reward::UAV | reward::AIRSTRIKE);

    let call = |app: &mut App| {
        app.world_mut().send_event(CommandEvent { player: 0, command: PlayerCommand::UseStreak });
        app.update();
    };
    call(&mut app);
    assert!(app.world().resource::<MatchState>().uav[0] > 25.0, "the UAV did not go up");
    call(&mut app);
    assert_eq!(app.world_mut().query::<&Airstrike>().iter(app.world()).count(), 1);
    assert_eq!(me(&mut app).rewards, 0);
    // The bombs fall and the strike is over.
    for _ in 0..(4.0 / FRAME) as usize {
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
        player.rewards = reward::UAV;
        health.hp = 0.0;
        player.streak = 0;
    }
    assert_eq!(me(&mut app).rewards, reward::UAV);
}
