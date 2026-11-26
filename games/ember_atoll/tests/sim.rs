//! Headless end-to-end tests: the real simulation, the real director and real
//! sockets, with no window or GPU. These are the safety net for gameplay and
//! netcode changes.

use bevy::{ecs::system::RunSystemOnce, prelude::*, state::app::StatesPlugin, time::TimeUpdateStrategy};
use ember_atoll::{EmberAtollPlugin, content::{action, weapon}, frontier_rules, horde_rules};
use oz_sim::{net::{self, NetIds}, prelude::*};
use std::time::{Duration, Instant};

const FRAME: f32 = 1.0 / 30.0;

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, SimPlugin, EmberAtollPlugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(FRAME)));
    app
}

fn start(app: &mut App, setup: MatchSetup) {
    app.world_mut().resource_mut::<PendingMatch>().0 = Some(setup);
    for _ in 0..3 {
        app.update();
    }
    assert_eq!(*app.world().resource::<State<MatchPhase>>().get(), MatchPhase::Active);
}

fn run(app: &mut App, seconds: f32) {
    for _ in 0..(seconds / FRAME) as usize {
        app.update();
    }
}

fn order(app: &mut App, player: u8, command: PlayerCommand) {
    app.world_mut().send_event(CommandEvent { player, command });
    app.update();
}

fn state(app: &App) -> MatchState {
    app.world().resource::<MatchState>().clone()
}

fn count<C: Component>(app: &mut App) -> usize {
    app.world_mut().query::<&C>().iter(app.world()).count()
}

fn rich(app: &mut App) {
    app.world_mut().resource_mut::<MatchState>().stock = Stock { credits: 50_000, ore: [5_000; 3] };
}

#[test]
fn undefended_beacon_takes_damage_at_night() {
    let mut app = app();
    start(&mut app, MatchSetup::offline(frontier_rules(7), "TEST"));
    assert_eq!(state(&app).phase, Phase::Day);
    assert!(count::<OreNode>(&mut app) >= 20);
    assert_eq!(count::<Station>(&mut app), 3);

    order(&mut app, 0, PlayerCommand::Mode(action::CALL_NIGHTFALL));
    assert_eq!(state(&app).phase, Phase::Night);
    run(&mut app, 20.0);
    assert!(count::<Enemy>(&mut app) > 0, "enemies should have surfaced");
    assert!(state(&app).daylight < 0.1);

    run(&mut app, 60.0);
    let now = state(&app);
    assert!(now.objective_hp < now.objective_max, "skitters should have reached the beacon");
}

#[test]
fn turrets_hold_the_night_and_dawn_pays_out() {
    let mut app = app();
    start(&mut app, MatchSetup::offline(frontier_rules(11), "TEST"));
    rich(&mut app);
    for (x, z) in [(3.5, 4.0), (-3.5, 4.0), (5.0, 7.5), (-5.0, 7.5)] {
        order(&mut app, 0, PlayerCommand::Build { kind: 0, x, z, yaw: 0.0 });
    }
    assert_eq!(count::<Structure>(&mut app), 4, "all four turrets should be placed");
    // Too far from the player: refused.
    order(&mut app, 0, PlayerCommand::Build { kind: 0, x: 30.0, z: 30.0, yaw: 0.0 });
    assert_eq!(count::<Structure>(&mut app), 4);

    order(&mut app, 0, PlayerCommand::Mode(action::CALL_NIGHTFALL));
    let credits = state(&app).stock.credits;
    for _ in 0..200 {
        run(&mut app, 2.0);
        if state(&app).phase == Phase::Day {
            break;
        }
    }
    let now = state(&app);
    assert_eq!((now.phase, now.day), (Phase::Day, 2), "night should end once every attacker is destroyed");
    assert!(now.kills >= 5 && now.stock.credits > credits);
    assert!(now.objective_hp > 0.0);
    assert_eq!(count::<Enemy>(&mut app), 0);
}

#[test]
fn mining_by_hand_then_by_drill() {
    let mut app = app();
    start(&mut app, MatchSetup::offline(frontier_rules(3), "TEST"));
    let world = app.world_mut();
    let (node, ore) = world.query::<(&Transform, &OreNode)>().iter(world).map(|(t, n)| (t.translation, n.ore)).find(|(_, ore)| *ore == Ore::Iron).unwrap();

    // Stand next to the deposit, look at it, hold the trigger with the mining laser.
    let feet = node + Vec3::new(2.6, 0.0, 0.0);
    let to = (node + Vec3::Y * 0.6) - (feet + Vec3::Y * EYE);
    let (mut transform, mut input) = world.query_filtered::<(&mut Transform, &mut PlayerInput), With<LocalPlayer>>().single_mut(world).unwrap();
    transform.translation = feet;
    *input = PlayerInput { yaw: f32::atan2(-to.x, -to.z), pitch: (to.y / to.length()).asin(), buttons: buttons::FIRE, weapon: weapon::MINING_LASER as u8, ..default() };
    let before = state(&app).stock.ore[ore as usize];
    run(&mut app, 5.0);
    let mined = state(&app).stock.ore[ore as usize] - before;
    assert!((50..=80).contains(&mined), "expected about 70 ore from 5 s of mining, got {mined}");

    // Release the trigger and let a drill take over.
    app.world_mut().query_filtered::<&mut PlayerInput, With<LocalPlayer>>().single_mut(app.world_mut()).unwrap().buttons = 0;
    rich(&mut app);
    order(&mut app, 0, PlayerCommand::Build { kind: 1, x: node.x + 1.0, z: node.z, yaw: 0.0 });
    assert_eq!(count::<Structure>(&mut app), 1, "drill should snap onto the deposit");
    order(&mut app, 0, PlayerCommand::Build { kind: 1, x: node.x, z: node.z, yaw: 0.0 });
    assert_eq!(count::<Structure>(&mut app), 1, "one drill per deposit");
    let before = state(&app).stock.ore[ore as usize];
    run(&mut app, 10.0);
    let drilled = state(&app).stock.ore[ore as usize] - before;
    assert!((18..=26).contains(&drilled), "expected about 22 ore from 10 s of drilling, got {drilled}");
}

#[test]
fn purchases_are_validated_by_the_authority() {
    let mut app = app();
    start(&mut app, MatchSetup::offline(frontier_rules(5), "TEST"));
    let arsenal = |app: &mut App| app.world_mut().query::<&Arsenal>().single(app.world()).unwrap().clone();
    assert!(arsenal(&mut app).owns(weapon::PULSE_PISTOL) && !arsenal(&mut app).owns(weapon::REEF_SMG));

    // 220 starting credits cannot buy the 560-credit lance.
    order(&mut app, 0, PlayerCommand::BuyWeapon(5));
    assert!(!arsenal(&mut app).owns(5));
    assert_eq!(state(&app).stock.credits, 220);

    order(&mut app, 0, PlayerCommand::BuyWeapon(weapon::REEF_SMG as u8));
    assert!(arsenal(&mut app).owns(weapon::REEF_SMG));
    assert_eq!(state(&app).stock.credits, 40);

    rich(&mut app);
    order(&mut app, 0, PlayerCommand::EquipPart { weapon: 2, slot: 1, variant: 2 });
    order(&mut app, 0, PlayerCommand::BuyMod { weapon: 2, kind: ModKind::Arc });
    order(&mut app, 0, PlayerCommand::Upgrade(2));
    let smg = arsenal(&mut app).weapons[2];
    assert_eq!((smg.equipped[1], smg.mod_level(ModKind::Arc)), (2, 1));
    let now = state(&app);
    assert_eq!(now.upgrades[2], 1);
    assert!((now.objective_max - 780.0).abs() < 0.1 && (now.objective_hp - 780.0).abs() < 0.1);
}

#[test]
fn horde_mode_reuses_the_same_parts() {
    let mut app = app();
    start(&mut app, MatchSetup::offline(horde_rules(9), "TEST"));
    assert_eq!(count::<OreNode>(&mut app), 0);
    let content = app.world().resource::<Content>().clone();
    assert!(content.structures.iter().all(|s| s.cost.ore == [0; 3]), "horde prices are credits only");
    assert_eq!(state(&app).objective_max, 0.0);
    run(&mut app, 5.0);
    assert_eq!(state(&app).phase, Phase::Day, "horde starts with a short breather");
    run(&mut app, 14.0);
    assert_eq!(state(&app).phase, Phase::Night, "the short intermission should have ended");
    assert!(count::<Enemy>(&mut app) > 0);
}

/// Pumps both apps until `done` or a timeout; sockets are real, so allow wall-clock time.
fn pump(host: &mut App, client: &mut App, what: &str, mut done: impl FnMut(&mut App, &mut App) -> bool) {
    let start = Instant::now();
    while !done(host, client) {
        assert!(start.elapsed() < Duration::from_secs(20), "timed out waiting for: {what}");
        host.update();
        client.update();
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn host_and_client_share_one_match() {
    let (mut host, mut client) = (app(), app());
    let rules = frontier_rules(21);
    start(&mut host, net::host_setup(rules.clone(), "HOST", "127.0.0.1:0", false).unwrap());
    let NetLink::Host(link) = &*host.world().resource::<NetLink>() else { panic!("not hosting") };
    let addr = link.host.local_addr().unwrap().to_string();

    let world = client.world_mut();
    net::join(&mut world.commands(), &addr, "GUEST");
    world.flush();
    pump(&mut host, &mut client, "client to enter the match", |_, c| *c.world().resource::<State<MatchPhase>>().get() == MatchPhase::Active);
    assert_eq!(*client.world().resource::<Rules>(), rules);
    assert_eq!(*client.world().resource::<NetRole>(), NetRole::Client);
    assert_eq!(count::<Player>(&mut host), 2);

    // Static world is rebuilt locally; dynamic entities arrive by snapshot.
    pump(&mut host, &mut client, "ore and the host player to replicate", |h, c| count::<OreNode>(c) == count::<OreNode>(h) && count::<Player>(c) == 2);

    // The guest buys a weapon: the host charges the shared stock and the guest receives the gun.
    let guest = client.world_mut().query_filtered::<&Player, With<LocalPlayer>>().single(client.world()).unwrap().id;
    client.world_mut().send_event(CommandEvent { player: guest, command: PlayerCommand::BuyWeapon(weapon::REEF_SMG as u8) });
    pump(&mut host, &mut client, "purchase to round-trip", |h, c| {
        let owns = c.world_mut().query_filtered::<&Arsenal, With<LocalPlayer>>().single(c.world()).unwrap().owns(weapon::REEF_SMG);
        owns && h.world().resource::<MatchState>().stock.credits == 40 && c.world().resource::<MatchState>().stock.credits == 40
    });

    // Nightfall on the host shows up as enemies on the client.
    host.world_mut().send_event(CommandEvent { player: 0, command: PlayerCommand::Mode(action::CALL_NIGHTFALL) });
    pump(&mut host, &mut client, "enemies to replicate", |h, c| {
        let (on_host, on_client) = (count::<Enemy>(h), count::<Enemy>(c));
        on_host >= 3 && on_client >= 3 && c.world().resource::<MatchState>().phase == Phase::Night
    });

    // Dropping the client removes its player from the host.
    drop(client);
    let mut idle = app();
    pump(&mut host, &mut idle, "host to notice the disconnect", |h, _| count::<Player>(h) == 1);
}

#[test]
fn a_horde_of_1500_stays_cheap() {
    let mut app = app();
    start(&mut app, MatchSetup::offline(frontier_rules(13), "TEST"));
    rich(&mut app);
    for (x, z) in [(3.5, 4.0), (-3.5, 4.0), (5.0, 7.5), (-5.0, 7.5)] {
        order(&mut app, 0, PlayerCommand::Build { kind: 0, x, z, yaw: 0.0 });
    }
    app.world_mut()
        .run_system_once(|mut commands: Commands, mut ids: ResMut<NetIds>, content: Res<Content>, mut rng: ResMut<SimRng>, map: Res<Map>, mut state: ResMut<MatchState>| {
            state.phase = Phase::Night;
            state.objective_hp = 1e9;
            state.objective_max = 1e9;
            for i in 0..1500 {
                let at = Vec2::from_angle(i as f32 * 0.7) * (25.0 + (i % 60) as f32);
                spawn_enemy(&mut commands, &mut ids, &content, (i % 5) as u16, map.ground(at.x, at.y), 50.0, &mut rng);
            }
        })
        .unwrap();
    run(&mut app, 2.0);
    assert!(count::<Enemy>(&mut app) >= 1400);

    let frames = 120;
    let start = Instant::now();
    for _ in 0..frames {
        app.update();
    }
    let per_frame = start.elapsed().as_secs_f64() * 1000.0 / frames as f64;
    println!("1500 enemies: {per_frame:.2} ms per simulation frame");
    assert!(per_frame < 16.0, "simulation too slow: {per_frame:.2} ms per frame with 1500 enemies");
}
