//! A real host and a real guest in one process, talking over loopback through a
//! proxy that delays every packet (so about 120 ms of round trip), on real time.
//!
//! * A guest who dies comes back at their base, on the host and on their own screen,
//!   not where they fell.
//! * A guest shooting at a moving player where its own screen shows them hits on the
//!   host (lag compensation rewinds to what the guest saw).

use bevy::{prelude::*, state::app::StatesPlugin};
use iron_town::{IronTownPlugin, deathmatch_rules, horde_rules, hunt_rules};
use oz_sim::{net, prelude::*};
use std::{
    collections::VecDeque,
    net::{SocketAddr, UdpSocket},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

/// The guest's reconnect token (an install's identity, see `oz_client::settings`).
const TOKEN: u64 = 0x5eed_cafe;

/// Each way, so the round trip is twice this.
const ONE_WAY: Duration = Duration::from_millis(60);

/// A UDP relay between one guest and the host that holds every datagram for a while.
struct Lag {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Lag {
    fn new(host: SocketAddr, delay: Duration) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let addr = socket.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let quit = stop.clone();
        let thread = std::thread::spawn(move || {
            let mut guest: Option<SocketAddr> = None;
            let mut queue: VecDeque<(Instant, SocketAddr, Vec<u8>)> = VecDeque::new();
            let mut buffer = [0u8; 2048];
            while !quit.load(Ordering::Relaxed) {
                while let Ok((n, from)) = socket.recv_from(&mut buffer) {
                    let to = if from == host {
                        match guest {
                            Some(guest) => guest,
                            None => continue,
                        }
                    } else {
                        guest = Some(from);
                        host
                    };
                    queue.push_back((Instant::now() + delay, to, buffer[..n].to_vec()));
                }
                let now = Instant::now();
                while queue.front().is_some_and(|(due, ..)| *due <= now) {
                    let (_, to, data) = queue.pop_front().unwrap();
                    let _ = socket.send_to(&data, to);
                }
                std::thread::sleep(Duration::from_micros(500));
            }
        });
        Self { addr, stop, thread: Some(thread) }
    }
}

impl Drop for Lag {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Real time: what the network delays has to line up with what the game clocks say.
fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, SimPlugin, IronTownPlugin));
    app
}

struct Game {
    host: App,
    guest: App,
    _lag: Lag,
}

impl Game {
    /// A host and a guest on opposite teams, no bots, the match live.
    fn new(seed: u64, respawn: f32) -> Self {
        let mut rules = deathmatch_rules(seed);
        rules.team_size = 1;
        rules.no_bots = true;
        rules.respawn_seconds = respawn;
        rules.score_limit = 200;
        Self::with(rules)
    }

    fn with(rules: Rules) -> Self {
        let mut host = app();
        host.world_mut().resource_mut::<PendingMatch>().0 = Some(net::host_setup(rules, "HOST", "127.0.0.1:0", false).unwrap());
        for _ in 0..4 {
            host.update();
        }
        let NetLink::Host(link) = host.world().resource::<NetLink>() else { panic!("not hosting") };
        let lag = Lag::new(link.host.local_addr().unwrap(), ONE_WAY);
        let mut game = Self { host, guest: app(), _lag: lag };
        game.connect(TOKEN);
        // Co-op runs start straight away; versus goes through the lobby.
        if !game.host.world().resource::<Rules>().pvp {
            game.run(0.5);
            return game;
        }
        game.host.world_mut().send_event(CommandEvent { player: 0, command: PlayerCommand::Lobby(LobbyAction::Start) });
        let started = Instant::now();
        while game.host.world().resource::<MatchState>().phase != Phase::Live {
            game.frame();
            assert!(started.elapsed() < Duration::from_secs(30), "the match never started");
        }
        game.run(1.0);
        game
    }

    /// A fresh guest game joins (through the lag) with `token` and waits until it is in.
    fn connect(&mut self, token: u64) {
        self.guest = app();
        let world = self.guest.world_mut();
        net::join_as(&mut world.commands(), &self._lag.addr.to_string(), "GUEST", token);
        world.flush();
        let started = Instant::now();
        loop {
            self.frame();
            if let Some((_, id, _)) = self.guest_me()
                && self.players(true).iter().any(|p| p.1 == id)
                && self.players(false).len() == self.players(true).len()
            {
                break;
            }
            assert!(started.elapsed() < Duration::from_secs(20), "the guest never joined");
        }
    }

    /// The guest's game dies without a word (a crash, a cable pulled).
    fn crash_guest(&mut self) {
        self.guest = app();
    }

    fn frame(&mut self) {
        self.host.update();
        self.guest.update();
        std::thread::sleep(Duration::from_millis(4));
    }

    fn run(&mut self, seconds: f32) {
        let until = Instant::now() + Duration::from_secs_f32(seconds);
        while Instant::now() < until {
            self.frame();
        }
    }

    /// (entity, id, team, feet, alive) for every player on the host or the guest.
    fn players(&mut self, on_host: bool) -> Vec<(Entity, u8, u8, Vec3, bool)> {
        let app = if on_host { &mut self.host } else { &mut self.guest };
        app.world_mut().query::<(Entity, &Player, &Transform, &Health)>().iter(app.world()).map(|(e, p, t, h)| (e, p.id, p.team, t.translation, h.alive())).collect()
    }

    /// The guest's own player, on the guest.
    fn guest_me(&mut self) -> Option<(Entity, u8, u8)> {
        let world = self.guest.world_mut();
        world.query_filtered::<(Entity, &Player), With<LocalPlayer>>().single(world).ok().map(|(e, p)| (e, p.id, p.team))
    }

    fn host_entity(&mut self, id: u8) -> Entity {
        self.players(true).iter().find(|p| p.1 == id).map(|p| p.0).unwrap()
    }
}

/// An open spot near `around` with nothing overhead and level ground.
fn open_spot(map: &GameMap, statics: &oz_core::collide::StaticWorld, around: Vec2, clear: impl Fn(Vec3) -> bool) -> Option<Vec3> {
    (0..30).flat_map(|r| (0..16).map(move |k| (r, k))).find_map(|(r, k)| {
        let p = around + Vec2::from_angle(k as f32 * std::f32::consts::TAU / 16.0) * r as f32 * 1.5;
        let at = map.ground(p.x, p.y);
        (map.in_bounds(p.x, p.y) && !statics.blocked(at.x, at.z, at.y + 0.05, 1.8, 0.5) && statics.raycast(at + Vec3::Y * 30.0, Vec3::NEG_Y, 28.0).is_none() && clear(at)).then_some(at)
    })
}

/// Puts player `id` out at a spot `k` far from its base, kills it there, and checks that
/// it comes back at a team spawn - on the host and on the guest's screen (the guest's own
/// predicted body for the guest, its played-back copy for anyone else) - soon, and stays.
fn die_and_respawn(game: &mut Game, id: u8, k: usize) {
    let guest_id = game.guest_me().unwrap().1;
    let map = game.host.world().resource::<Map>().clone();
    let statics = game.host.world().resource::<Statics>().clone();
    let on_host = game.host_entity(id);
    let team = game.host.world().get::<Player>(on_host).unwrap().team;
    let spawns: Vec<Vec3> = map.team_spawns[team as usize].iter().map(|p| map.ground(p.x, p.y)).collect();
    let at_base = |at: Vec3| spawns.iter().any(|s| s.xz().distance(at.xz()) < 2.0);
    let on_guest = |game: &mut Game| game.players(false).iter().find(|p| p.1 == id).map(|p| p.0).unwrap();

    // Out into the open (a different spot each time), far from the base. Whoever owns the
    // body moves it: the guest its own, the host everyone else.
    let middle = (player_base(&map, 0) + player_base(&map, 1)) * 0.5 + Vec2::new(k as f32 * 7.0, 0.0);
    let fell = open_spot(&map, &statics, middle, |at| spawns.iter().all(|s| s.distance(at) > 25.0)).expect("an open spot mid-map");
    let started = Instant::now();
    loop {
        if id == guest_id {
            let me = on_guest(game);
            game.guest.world_mut().get_mut::<Transform>(me).unwrap().translation = fell;
        } else {
            game.host.world_mut().get_mut::<Transform>(on_host).unwrap().translation = fell;
        }
        game.frame();
        let seen = on_guest(game);
        let seen = game.guest.world().get::<Transform>(seen).unwrap().translation;
        if game.host.world().get::<Transform>(on_host).unwrap().translation.distance(fell) < 1.0 && seen.distance(fell) < 1.0 {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(5), "player {id} never got out to {fell}");
    }

    // Shot dead there.
    game.host.world_mut().get_mut::<Health>(on_host).unwrap().hp = 0.0;
    let started = Instant::now();
    while !game.host.world().get::<Health>(on_host).unwrap().alive() {
        game.frame();
        assert!(started.elapsed() < Duration::from_secs(8), "player {id} never respawned");
    }
    // Within a couple of round trips the guest has them at the base too...
    let back = Instant::now();
    loop {
        game.frame();
        let seen = on_guest(game);
        let seen = game.guest.world().get::<Transform>(seen).unwrap().translation;
        if at_base(seen) {
            break;
        }
        assert!(back.elapsed() < Duration::from_millis(800), "player {id} is at {seen} on the guest's screen, not at a team {team} spawn (fell at {fell})");
    }
    // ...and they stay there: no stale report drags them back.
    let until = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < until {
        game.frame();
        let host_view = game.host.world().get::<Transform>(on_host).unwrap().translation;
        let guest = on_guest(game);
        let guest_view = game.guest.world().get::<Transform>(guest).unwrap().translation;
        assert!(game.guest.world().get::<Health>(guest).unwrap().alive(), "alive again on the guest");
        assert!(at_base(host_view), "host has player {id} at {host_view}, not at a team {team} spawn (fell at {fell})");
        assert!(at_base(guest_view), "the guest has player {id} at {guest_view}, not at a team {team} spawn (fell at {fell})");
        assert!(host_view.distance(guest_view) < 0.6, "host {host_view} and guest {guest_view} disagree on player {id}");
    }
}

#[test]
fn a_guest_who_dies_respawns_at_their_base_not_where_they_fell() {
    let mut game = Game::new(11, 1.0);
    let guest = game.guest_me().unwrap().1;
    // Again and again, and the host too.
    for k in 0..3 {
        die_and_respawn(&mut game, guest, k);
    }
    for k in 0..2 {
        die_and_respawn(&mut game, 0, k);
    }
}

#[test]
fn terrorist_hunt_squad_respawns_at_base_online() {
    let mut rules = hunt_rules(13);
    rules.respawn_seconds = 1.0;
    let mut game = Game::with(rules);
    // Hostiles hold still so nobody else is shooting.
    let world = game.host.world_mut();
    let bots: Vec<Entity> = world.query_filtered::<Entity, With<Bot>>().iter(world).collect();
    for bot in bots {
        world.entity_mut(bot).remove::<Bot>();
    }
    let guest = game.guest_me().unwrap().1;
    for k in 0..2 {
        die_and_respawn(&mut game, guest, k);
    }
    die_and_respawn(&mut game, 0, 0);
}

#[test]
fn horde_a_guest_who_bled_out_comes_back_with_the_squad_next_round() {
    let mut rules = horde_rules(1);
    rules.team_size = 2;
    let mut game = Game::with(rules);
    let (me, id, _) = game.guest_me().unwrap();
    let map = game.host.world().resource::<Map>().clone();
    let statics = game.host.world().resource::<Statics>().clone();
    let home = map.spawn;
    // A few steps off into the first zone, somewhere the body fits.
    let on_host = game.host_entity(id);
    let start_y = game.host.world().get::<Transform>(on_host).unwrap().translation.y;
    let fell = (8..40)
        .flat_map(|r| (0..16).map(move |k| home + Vec2::from_angle(k as f32 * 0.39) * r as f32 * 0.25))
        .map(|p| Vec3::new(p.x, oz_sim::player::ground_under(&map, &statics, p.x, p.y, start_y + 0.5), p.y))
        .find(|at| (at.y - start_y).abs() < 0.3 && !statics.blocked(at.x, at.z, at.y + 0.05, 1.8, 0.5) && at.xz().distance(home) > 6.0)
        .expect("room to step away from the start");
    let husks = |game: &mut Game| {
        let world = game.host.world_mut();
        let husks: Vec<Entity> = world.query_filtered::<Entity, With<oz_sim::zombies::Husk>>().iter(world).collect();
        for husk in husks {
            world.send_event(oz_sim::combat::Hit::plain(husk, 1.0e7, Some(0)));
        }
    };
    let started = Instant::now();
    while game.host.world().get::<Transform>(on_host).unwrap().translation.distance(fell) > 0.5 {
        game.guest.world_mut().get_mut::<Transform>(me).unwrap().translation = fell;
        husks(&mut game);
        game.frame();
        assert!(started.elapsed() < Duration::from_secs(5), "the host never saw the guest step away");
    }
    // Bled out there: sits out the rest of the round.
    let round = game.host.world().resource::<MatchState>().horde.round;
    game.host.world_mut().get_mut::<Health>(on_host).unwrap().hp = 0.0;
    if let Some(s) = game.host.world_mut().resource_mut::<MatchState>().horde.survivor_mut(id) {
        s.out = true;
    }
    // The squad clears the round (twice if it had not begun); the next one brings them back.
    let started = Instant::now();
    while game.host.world().resource::<MatchState>().horde.round < round.max(1) + 1 || !game.host.world().get::<Health>(on_host).unwrap().alive() {
        husks(&mut game);
        game.frame();
        assert!(started.elapsed() < Duration::from_secs(60), "the next round never came (round {})", game.host.world().resource::<MatchState>().horde.round);
    }
    let back = Instant::now();
    let near_start = |at: Vec3| at.xz().distance(home) < 4.0;
    loop {
        husks(&mut game);
        game.frame();
        if near_start(game.guest.world().get::<Transform>(me).unwrap().translation) {
            break;
        }
        assert!(back.elapsed() < Duration::from_millis(800), "the guest is still at {} after coming back (fell at {fell})", game.guest.world().get::<Transform>(me).unwrap().translation);
    }
    let until = Instant::now() + Duration::from_millis(1500);
    while Instant::now() < until {
        husks(&mut game);
        game.frame();
        let host_view = game.host.world().get::<Transform>(on_host).unwrap().translation;
        let own_view = game.guest.world().get::<Transform>(me).unwrap().translation;
        assert!(near_start(host_view) && near_start(own_view), "back at {host_view} (host) / {own_view} (guest), fell at {fell}, start {home}");
        assert!(host_view.distance(own_view) < 0.6);
    }
}

fn player_base(map: &GameMap, team: u8) -> Vec2 {
    oz_sim::player::base_centre(map, team).unwrap()
}

#[test]
fn a_guest_hits_a_moving_player_where_its_screen_shows_them() {
    let mut game = Game::new(23, 2.0);
    let (me, id, _) = game.guest_me().unwrap();
    let map = game.host.world().resource::<Map>().clone();
    let statics = game.host.world().resource::<Statics>().clone();
    let target = game.host_entity(0);
    let shooter_on_host = game.host_entity(id);

    // A lane in the open: the target runs 8 m back and forth across the guest's view, 14 m off.
    let middle = (player_base(&map, 0) + player_base(&map, 1)) * 0.5;
    let sees = |a: Vec3, b: Vec3| statics.raycast(a, (b - a).normalize(), a.distance(b)).is_none() && map.raycast(a, (b - a).normalize(), a.distance(b)) >= a.distance(b) - 0.1;
    let lane = |shooter: Vec3, k: f32| map.ground(shooter.x - 4.0 + 8.0 * k, shooter.z - 14.0);
    let shooter = open_spot(&map, &statics, middle, |s| {
        (0..=8).all(|i| {
            let p = lane(s, i as f32 / 8.0);
            (p.y - s.y).abs() < 0.4 && !statics.blocked(p.x, p.z, p.y + 0.05, 1.8, 0.5) && sees(s + Vec3::Y * EYE, p + Vec3::Y * 1.1)
        })
    })
    .expect("an open lane mid-map");
    game.guest.world_mut().get_mut::<Transform>(me).unwrap().translation = shooter;
    let place_target = |game: &mut Game, t: f32| {
        // 8 m every 1.6 s: 5 m/s, a jog.
        let k = (t / 1.6).rem_euclid(2.0);
        let k = if k > 1.0 { 2.0 - k } else { k };
        let world = game.host.world_mut();
        world.get_mut::<Transform>(target).unwrap().translation = lane(shooter, k);
        let mut player = world.get_mut::<Player>(target).unwrap();
        player.protect = 0.0;
        world.get_mut::<Health>(target).unwrap().hp = 100.0;
    };
    let started = Instant::now();
    while game.host.world().get::<Transform>(shooter_on_host).unwrap().translation.distance(shooter) > 1.0 {
        place_target(&mut game, started.elapsed().as_secs_f32());
        game.frame();
        assert!(started.elapsed() < Duration::from_secs(5), "the host never saw the guest move");
    }
    game.run(0.5);

    let shots_before = game.host.world().get::<Arsenal>(shooter_on_host).unwrap().shots;
    let (mut hits, mut lead, mut aimed) = (0, 0.0, 0);
    let mut last_tap = Instant::now();
    let clock = Instant::now();
    while clock.elapsed() < Duration::from_secs(6) {
        let t = started.elapsed().as_secs_f32();
        place_target(&mut game, t);
        // The guest aims at the target's chest where its own screen has it.
        let seen = game.players(false).iter().find(|p| p.1 == 0).map(|p| p.3).unwrap();
        let truth = game.host.world().get::<Transform>(target).unwrap().translation;
        let world = game.guest.world_mut();
        let feet = world.get::<Transform>(me).unwrap().translation;
        let mut input = world.get_mut::<PlayerInput>(me).unwrap();
        let to = seen + Vec3::Y * 1.05 - oz_sim::player::shot_origin(&statics, feet, &input);
        input.yaw = f32::atan2(-to.x, -to.z);
        input.pitch = (to.y / to.length()).asin();
        input.buttons = buttons::AIM;
        if last_tap.elapsed() > Duration::from_millis(350) {
            last_tap = Instant::now();
            input.buttons |= buttons::FIRE;
            lead += truth.distance(seen);
            aimed += 1;
        }
        game.frame();
        if game.host.world().get::<Health>(target).unwrap().hp < 100.0 {
            hits += 1;
        }
    }
    let shots = game.host.world().get::<Arsenal>(shooter_on_host).unwrap().shots.wrapping_sub(shots_before);
    let lead = lead / aimed.max(1) as f32;
    assert!(shots >= 10, "only {shots} shots reached the host");
    // The guest's screen really is behind the host (else this proves nothing)...
    assert!(lead > 0.6, "the guest saw the target only {lead:.2} m behind the host");
    // ...and its shots land anyway.
    assert!(hits as f32 >= shots as f32 * 0.7, "{hits} of {shots} shots hit (target drawn {lead:.2} m behind where the host has it)");
}

#[test]
fn a_dropped_guest_gets_their_robot_back_or_rejoins_at_their_base_with_their_score() {
    let mut game = Game::new(37, 2.0);
    let (_, id, team) = game.guest_me().unwrap();
    let map = game.host.world().resource::<Map>().clone();
    let spawns: Vec<Vec3> = map.team_spawns[team as usize].iter().map(|p| map.ground(p.x, p.y)).collect();
    let at_base = |at: Vec3| spawns.iter().any(|s| s.xz().distance(at.xz()) < 2.0);
    let robot = game.host_entity(id);
    game.host.world_mut().get_mut::<Player>(robot).unwrap().kills = 5;

    // A blip: gone for a moment, back while the robot still waits. Same robot, same everything.
    game.crash_guest();
    game.run(1.5);
    assert_eq!(game.host_entity(id), robot, "the robot waits for its guest");
    game.connect(TOKEN);
    let (me, again, again_team) = game.guest_me().unwrap();
    assert_eq!((again, again_team), (id, team));
    assert_eq!(game.host_entity(id), robot, "came back to the same robot");
    game.run(1.0);
    let host_view = game.host.world().get::<Transform>(robot).unwrap().translation;
    let own_view = game.guest.world().get::<Transform>(me).unwrap().translation;
    assert!(host_view.distance(own_view) < 0.5, "host {host_view} and guest {own_view} disagree after the blip");

    // Shot dead out in the open, then the game crashes and stays away too long.
    let statics = game.host.world().resource::<Statics>().clone();
    let middle = (player_base(&map, 0) + player_base(&map, 1)) * 0.5;
    let fell = open_spot(&map, &statics, middle, |at| spawns.iter().all(|s| s.distance(at) > 25.0)).unwrap();
    game.guest.world_mut().get_mut::<Transform>(me).unwrap().translation = fell;
    game.run(1.0);
    game.host.world_mut().get_mut::<Health>(robot).unwrap().hp = 0.0;
    game.run(0.3);
    game.crash_guest();
    if let NetLink::Host(link) = &mut *game.host.world_mut().resource_mut::<NetLink>() {
        link.grace = 1.0;
    }
    // The host has to notice the silence first (a crash says no goodbye), then wait out the grace.
    let started = Instant::now();
    while game.players(true).iter().any(|p| p.1 == id) {
        game.frame();
        assert!(started.elapsed() < Duration::from_secs(20), "the dropped guest's robot was never cleaned up");
    }
    game.run(0.5);
    assert_eq!(game.host.world().resource::<MatchState>().phase, Phase::Live, "the match carries on without them");

    game.connect(TOKEN);
    let (me, again, again_team) = game.guest_me().unwrap();
    assert_eq!((again, again_team), (id, team), "back in the same seat on the same team");
    game.run(1.5);
    let robot = game.host_entity(id);
    let world = game.host.world();
    assert_eq!(world.get::<Player>(robot).unwrap().kills, 5, "the score was kept");
    let host_view = world.get::<Transform>(robot).unwrap().translation;
    let own_view = game.guest.world().get::<Transform>(me).unwrap().translation;
    assert!(at_base(host_view) && at_base(own_view), "back at {host_view} (host) / {own_view} (guest), not at a team {team} spawn; fell at {fell}");
    assert!(host_view.distance(own_view) < 0.5);
}
