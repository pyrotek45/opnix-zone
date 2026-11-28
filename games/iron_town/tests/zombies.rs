//! Headless horde-mode tests: rounds, points and buying, downs and revives, game over, the
//! husks' hunt, and the two horde maps' layouts. Husks only ever turn up outside and come in
//! through the windows; every zone opens up door by door; wall guns fit their chalk frames;
//! bot squads play rounds through on both maps; volt husks burst; power-ups are sure to come;
//! the generator, the arc grates and the relay beacons do what they say.

use bevy::{prelude::*, state::app::StatesPlugin, time::TimeUpdateStrategy};
use iron_town::{IronTownPlugin, horde_rules};
use oz_core::{
    collide::StaticWorld,
    worldgen::{Layout, World as GameMap, WorldRecipe},
};
use oz_sim::{
    combat::Hit,
    prelude::*,
    zombies::{self, HordeNav, Husk, act, perk},
};
use std::time::Duration;

const FRAME: f32 = 1.0 / 30.0;

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, SimPlugin, IronTownPlugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(FRAME)));
    app
}

fn start(layout: Layout, squad: u8) -> App {
    let mut app = app();
    let mut rules = horde_rules(1);
    rules.recipe = WorldRecipe::horde(layout);
    rules.team_size = squad;
    app.world_mut().resource_mut::<PendingMatch>().0 = Some(MatchSetup::offline(rules, "TEST"));
    for _ in 0..4 {
        app.update();
    }
    assert_eq!(*app.world().resource::<State<MatchPhase>>().get(), MatchPhase::Active);
    app
}

fn state(app: &App) -> &MatchState {
    app.world().resource::<MatchState>()
}

/// Every husk standing takes a killing hit from `by`.
fn scrap_husks(app: &mut App, by: Option<u8>) -> usize {
    let world = app.world_mut();
    let husks: Vec<Entity> = world.query_filtered::<Entity, With<Husk>>().iter(world).collect();
    for &husk in &husks {
        world.send_event(Hit::plain(husk, 1.0e7, by));
    }
    husks.len()
}

fn run(app: &mut App, seconds: f32, mut each: impl FnMut(&mut App)) {
    for _ in 0..(seconds / FRAME) as usize {
        each(app);
        app.update();
    }
}

/// Puts the local player at `at`, looking along `look`.
fn place(app: &mut App, at: Vec3, look: Vec3) {
    let world = app.world_mut();
    let (mut transform, mut input) = world.query_filtered::<(&mut Transform, &mut PlayerInput), With<LocalPlayer>>().single_mut(world).unwrap();
    transform.translation = at;
    input.yaw = f32::atan2(-look.x, -look.z);
    input.pitch = 0.0;
}

/// Holds Use for `seconds` (as the client does: a press, then "still held" every frame).
fn hold_use(app: &mut App, seconds: f32) {
    app.world_mut().send_event(CommandEvent { player: 0, command: PlayerCommand::Mode(act::USE) });
    run(app, seconds, |app| {
        app.world_mut().send_event(CommandEvent { player: 0, command: PlayerCommand::Mode(act::HOLD) });
        invulnerable(app);
    });
}

/// Stands at a machine, facing it.
fn at_machine(app: &mut App, m: &oz_core::worldgen::HordeMachine) {
    place(app, m.pos, -Vec3::new(m.facing.x, 0.0, m.facing.y));
}

fn use_it(app: &mut App) {
    app.world_mut().send_event(CommandEvent { player: 0, command: PlayerCommand::Mode(act::USE) });
    app.update();
    app.update();
}

fn points(app: &App) -> u32 {
    state(app).horde.survivor(0).map_or(0, |s| s.points)
}

fn local_arsenal(app: &mut App) -> Arsenal {
    let world = app.world_mut();
    world.query_filtered::<&Arsenal, With<LocalPlayer>>().single(world).unwrap().clone()
}

#[test]
fn rounds_advance_as_the_husks_fall_and_kills_pay() {
    let mut app = start(Layout::CinderWorks, 1);
    assert_eq!(state(&app).horde.round, 0);
    assert_eq!(points(&app), zombies::START_POINTS);
    let mut seconds = 0.0;
    let mut killed = 0;
    while state(&app).horde.round < 4 {
        killed += scrap_husks(&mut app, Some(0));
        app.update();
        seconds += FRAME;
        assert!(seconds < 400.0, "stuck in round {}", state(&app).horde.round);
    }
    let s = state(&app).horde.survivor(0).unwrap().clone();
    println!("round 4 after {seconds:.0} s, {killed} husks, {} points", s.points);
    // Rounds 1-3 send 6 + 8 + 13 husks.
    assert!(killed >= 27, "only {killed} husks in three rounds");
    assert!(s.kills as usize >= 27);
    assert!(s.points >= zombies::START_POINTS + 27 * 60, "kills should pay: {}", s.points);
    assert_eq!(state(&app).phase, Phase::Night, "the run is still on");
}

#[test]
fn wall_buys_doors_power_perks_and_the_reforge() {
    let mut app = start(Layout::CinderWorks, 1);
    let layout = app.world().resource::<Map>().horde.clone().unwrap();
    app.world_mut().resource_mut::<MatchState>().horde.survivor_mut(0).unwrap().points = 30_000;
    let keep_clear = |app: &mut App| {
        scrap_husks(app, None);
    };

    // A wall buy in the courtyard.
    let buy = layout.buys.iter().find(|b| b.zone == 0).copied().unwrap();
    let n = Vec3::new(buy.normal.x, 0.0, buy.normal.y);
    place(&mut app, Vec3::new(buy.pos.x, layout.ground, buy.pos.z) + n * 0.7, -n);
    let before = points(&app);
    use_it(&mut app);
    assert!(local_arsenal(&mut app).owns(buy.weapon as usize), "the wall gun should be ours");
    assert_eq!(points(&app), before - buy.price);
    assert_eq!(local_arsenal(&mut app).current, buy.weapon, "and in hand");

    // The first door.
    let door = layout.doors[0];
    let c = door.aabb.center;
    let side = if door.along_x { Vec3::new(0.0, 0.0, -0.9) } else { Vec3::new(-0.9, 0.0, 0.0) };
    place(&mut app, Vec3::new(c.x, layout.ground, c.z) + side, -side);
    let before = points(&app);
    use_it(&mut app);
    assert!(state(&app).horde.door_open(0), "the door should open");
    assert_eq!(points(&app), before - door.price);
    run(&mut app, 0.2, keep_clear);
    let world = app.world_mut();
    assert_eq!(world.query::<&zombies::DoorBlock>().iter(world).filter(|d| d.0 == 0).count(), 0, "nothing blocks an open door");
    assert_eq!(app.world().resource::<HordeNav>().doors & 1, 1, "the husks' map knows");

    // Perks need the power on.
    let machine = layout.perks.iter().find(|m| m.kind == perk::IRON_HIDE).copied().unwrap();
    let face = -Vec3::new(machine.facing.x, 0.0, machine.facing.y);
    place(&mut app, machine.pos, face);
    use_it(&mut app);
    assert!(!state(&app).horde.survivor(0).unwrap().has(perk::IRON_HIDE), "no power, no perk");
    let power = layout.power.unwrap();
    at_machine(&mut app, &power);
    hold_use(&mut app, zombies::CRANK_SECONDS + 0.5);
    assert!(state(&app).horde.power, "cranking the generator should turn the power on");
    place(&mut app, machine.pos, face);
    use_it(&mut app);
    run(&mut app, 0.2, keep_clear);
    assert!(state(&app).horde.survivor(0).unwrap().has(perk::IRON_HIDE));
    let world = app.world_mut();
    let health = *world.query_filtered::<&Health, With<LocalPlayer>>().single(world).unwrap();
    assert_eq!(health.max, 250.0, "iron hide plates you up");

    // The reforge (up on its deck) takes the gun in hand and hands back its reforged twin.
    let forge = layout.forge.unwrap();
    assert!(forge.pos.y > layout.ground + 2.0);
    place(&mut app, forge.pos, -Vec3::new(forge.facing.x, 0.0, forge.facing.y));
    use_it(&mut app);
    assert_eq!(state(&app).horde.forge_phase, zombies::forge_phase::WORKING);
    run(&mut app, 5.0, keep_clear);
    assert_eq!(state(&app).horde.forge_phase, zombies::forge_phase::OFFER);
    use_it(&mut app);
    let content = app.world().resource::<Content>().clone();
    let up = zombies::upgraded(&content, buy.weapon as usize).unwrap();
    let arsenal = local_arsenal(&mut app);
    assert!(arsenal.owns(up) && !arsenal.owns(buy.weapon as usize), "reforged");
    assert!(content.weapons[up].damage > content.weapons[buy.weapon as usize].damage * 2.0);
    assert_ne!(content.weapons[up].name, content.weapons[buy.weapon as usize].name);

    // The crate: pay, wait, take.
    let spot = layout.crates[0];
    place(&mut app, spot.pos, -Vec3::new(spot.facing.x, 0.0, spot.facing.y));
    let before = points(&app);
    use_it(&mut app);
    assert_eq!(points(&app), before - zombies::CRATE_PRICE);
    run(&mut app, 4.5, keep_clear);
    assert_eq!(state(&app).horde.crate_phase, zombies::crate_phase::OFFER);
    let offered = state(&app).horde.crate_weapon as usize;
    use_it(&mut app);
    assert!(local_arsenal(&mut app).owns(offered), "the crate's gun is ours");
    // Hands are full: two guns, the pistol was swapped for it.
    let owned = local_arsenal(&mut app).weapons.iter().filter(|w| w.owned).count();
    assert_eq!(owned, 2, "two guns without a spare socket");
}

#[test]
fn a_bot_gets_a_downed_player_back_up() {
    let mut app = start(Layout::CinderWorks, 2);
    run(&mut app, 1.0, |_| {});
    let world = app.world_mut();
    assert_eq!(world.query_filtered::<&Player, With<Bot>>().iter(world).count(), 1, "one bot fills the squad");
    // Down the human, as a husk would.
    {
        let mut st = app.world_mut().resource_mut::<MatchState>();
        let s = st.horde.survivor_mut(0).unwrap();
        s.down = true;
        s.bleed = zombies::BLEED;
    }
    let mut seconds = 0.0;
    while state(&app).horde.is_down(0) {
        scrap_husks(&mut app, None);
        app.update();
        seconds += FRAME;
        assert!(seconds < 40.0, "the bot never revived us");
        assert_ne!(state(&app).phase, Phase::Lost);
    }
    let bot = state(&app).horde.survivors.iter().find(|s| s.id != 0).unwrap().clone();
    println!("revived after {seconds:.1} s");
    assert_eq!(bot.revives, 1);
    // While down the player crawled with a pistol; back up, the stance comes back.
    let world = app.world_mut();
    let health = *world.query_filtered::<&Health, With<LocalPlayer>>().single(world).unwrap();
    assert!(health.alive() && health.hp >= health.max * 0.99);
}

#[test]
fn a_held_use_revives_and_everyone_down_ends_the_run() {
    let mut app = start(Layout::Blackfen, 2);
    run(&mut app, 1.0, |_| {});
    // Down the bot; the human holds Use on it.
    let bot_id = state(&app).horde.survivors.iter().find(|s| s.id != 0).unwrap().id;
    {
        let mut st = app.world_mut().resource_mut::<MatchState>();
        let s = st.horde.survivor_mut(bot_id).unwrap();
        s.down = true;
        s.bleed = zombies::BLEED;
    }
    let world = app.world_mut();
    let bot_at = world.query::<(&Player, &Transform)>().iter(world).find(|(p, _)| p.id == bot_id).unwrap().1.translation;
    let mut seconds = 0.0;
    while state(&app).horde.is_down(bot_id) {
        place(&mut app, bot_at + Vec3::X * 0.8, -Vec3::X);
        app.world_mut().send_event(CommandEvent { player: 0, command: PlayerCommand::Mode(act::HOLD) });
        scrap_husks(&mut app, None);
        app.update();
        seconds += FRAME;
        assert!(seconds < 8.0, "holding Use should revive in about four seconds");
    }
    assert!(seconds > 3.0, "a revive takes a moment ({seconds:.1} s)");
    assert_eq!(state(&app).horde.survivor(0).unwrap().revives, 1);

    // Now both go down: the run is over.
    {
        let mut st = app.world_mut().resource_mut::<MatchState>();
        for s in &mut st.horde.survivors {
            s.down = true;
            s.bleed = zombies::BLEED;
        }
    }
    run(&mut app, 0.3, |_| {});
    assert_eq!(state(&app).phase, Phase::Lost, "everyone down ends it");
}

#[test]
fn a_lone_player_bleeds_out_into_game_over_when_husks_get_through() {
    // Stand still and do nothing: the husks tear in through the boards, find the player and
    // claw it down; alone, that is the end.
    let mut app = start(Layout::CinderWorks, 1);
    let full: u32 = state(&app).horde.planks.iter().map(|p| *p as u32).sum();
    let mut seconds = 0.0;
    let mut torn = false;
    let mut was_down = false;
    while state(&app).phase != Phase::Lost {
        app.update();
        seconds += FRAME;
        torn |= state(&app).horde.planks.iter().map(|p| *p as u32).sum::<u32>() < full;
        was_down |= state(&app).horde.is_down(0);
        assert!(seconds < 240.0, "the husks never got the player (round {}, {} left)", state(&app).horde.round, state(&app).horde.left);
    }
    println!("overrun after {seconds:.0} s");
    assert!(torn, "husks tear boards off the windows");
    assert!(was_down, "the player goes down before the end");
    assert_eq!(state(&app).horde.survivor(0).unwrap().downs, 1);
}

fn statics(map: &GameMap) -> StaticWorld {
    let mut s = StaticWorld::new(map.recipe.half_extent);
    for b in map.solids() {
        s.add(b);
    }
    s
}

#[test]
fn horde_maps_connect_through_their_doors() {
    for layout in Layout::HORDE {
        let map = GameMap::generate(WorldRecipe::horde(layout));
        let lay = map.horde.clone().unwrap();
        let what = format!("{layout:?}");
        let ground = |p: Vec2| Vec3::new(p.x, lay.ground, p.y);
        let start = ground(map.spawn);
        let center = |z: usize| {
            let (x0, x1, z0, z1) = lay.zones[z].rect;
            ground(Vec2::new((x0 + x1) * 0.5, (z0 + z1) * 0.5))
        };
        let reaches = |nav: &mut HordeNav, to: Vec3| nav.path(start, to).is_some_and(|p| p.last().unwrap().xz().distance(to.xz()) < 1.6 && (p.last().unwrap().y - to.y).abs() < 1.0);
        // Doors shut: the start zone is sealed off from the rest.
        let mut shut = HordeNav::build(&map, &lay, 0);
        for z in 1..lay.zones.len() {
            assert!(!reaches(&mut shut, center(z)), "{what}: zone {} reachable with every door shut", lay.zones[z].name);
        }
        // Doors open: everything that matters can be walked to.
        let mut open = HordeNav::build(&map, &lay, u64::MAX);
        let mut spots: Vec<(String, Vec3)> = Vec::new();
        for (i, w) in lay.windows.iter().enumerate() {
            spots.push((format!("window {i}"), w.inside));
        }
        for (i, m) in lay.machines().enumerate() {
            spots.push((format!("machine {i}"), m.pos));
        }
        for (i, t) in lay.traps.iter().enumerate() {
            spots.push((format!("trap {i}"), ground(Vec2::new((t.floor.0 + t.floor.1) * 0.5, (t.floor.2 + t.floor.3) * 0.5))));
        }
        for (i, b) in lay.buys.iter().enumerate() {
            spots.push((format!("wall buy {i}"), ground(b.pos.xz() + b.normal * 0.7)));
        }
        for z in 0..lay.zones.len() {
            spots.push((lay.zones[z].name.to_string(), center(z)));
        }
        for (name, at) in spots {
            assert!(open.nav.walkable(at) || reaches(&mut open, at), "{what}: {name} at {at:?} is not on the floor");
            assert!(reaches(&mut open, at), "{what}: cannot walk to {name} at {at:?}");
        }
        // The spawn is clear and the budgets are sane.
        let s = statics(&map);
        assert!(!s.blocked(start.x, start.z, start.y, 1.7, 0.35), "{what}: spawn blocked");
        assert!(map.blocks.len() < 4000, "{what}: {} blocks", map.blocks.len());
        println!("{what}: {} blocks, {} windows, {} doors, {} wall buys", map.blocks.len(), lay.windows.len(), lay.doors.len(), lay.buys.len());
    }
}

/// Buying the doors one at a time from the start zone outward opens every zone in turn: each
/// zone is sealed until a door into it is bought, and walkable (with its windows, wall guns
/// and machines) right after. The last zones out hold the reforge and cost the most to reach.
#[test]
fn every_zone_opens_up_door_by_door() {
    for layout in Layout::HORDE {
        let map = GameMap::generate(WorldRecipe::horde(layout));
        let lay = map.horde.clone().unwrap();
        let what = format!("{layout:?}");
        let ground = |p: Vec2| Vec3::new(p.x, lay.ground, p.y);
        let start = ground(map.spawn);
        let center = |z: usize| {
            let (x0, x1, z0, z1) = lay.zones[z].rect;
            ground(Vec2::new((x0 + x1) * 0.5, (z0 + z1) * 0.5))
        };
        let reaches = |nav: &mut HordeNav, to: Vec3| nav.path(start, to).is_some_and(|p| p.last().unwrap().xz().distance(to.xz()) < 1.6 && (p.last().unwrap().y - to.y).abs() < 1.0);
        let mut doors = 0u64;
        let mut zones = 1u64;
        let mut nav = HordeNav::build(&map, &lay, 0);
        // Cheapest door out of what is open first, the way a squad would go.
        let mut order = Vec::new();
        loop {
            let next = lay
                .doors
                .iter()
                .enumerate()
                .filter(|(_, d)| (zones >> d.zones.0) & 1 != (zones >> d.zones.1) & 1)
                .min_by_key(|(i, d)| (d.price, *i))
                .map(|(i, d)| (i, *d));
            let Some((i, door)) = next else { break };
            let fresh = if zones & (1 << door.zones.0) != 0 { door.zones.1 } else { door.zones.0 } as usize;
            assert!(!reaches(&mut nav, center(fresh)), "{what}: {} is open before door {i} is bought", lay.zones[fresh].name);
            doors |= 1 << i;
            zones |= 1 << fresh;
            order.push((lay.zones[fresh].name, door.price));
            nav = HordeNav::build(&map, &lay, doors);
            assert!(reaches(&mut nav, center(fresh)), "{what}: door {i} should open {}", lay.zones[fresh].name);
            let mut spots: Vec<(String, Vec3)> = Vec::new();
            for (k, w) in lay.windows.iter().enumerate().filter(|(_, w)| w.zone as usize == fresh) {
                spots.push((format!("window {k}"), w.inside));
            }
            for (k, b) in lay.buys.iter().enumerate().filter(|(_, b)| b.zone as usize == fresh) {
                spots.push((format!("wall buy {k}"), ground(b.pos.xz() + b.normal * 0.7)));
            }
            for (k, m) in lay.machines().enumerate().filter(|(_, m)| m.zone as usize == fresh) {
                spots.push((format!("machine {k}"), m.pos));
            }
            for (name, at) in spots {
                assert!(reaches(&mut nav, at), "{what}: {name} in {} out of reach once it opens", lay.zones[fresh].name);
            }
        }
        println!("{what}: {order:?}");
        assert_eq!(zones.count_ones() as usize, lay.zones.len(), "{what}: some zone never opens");
        // The reforge is in one of the last zones to open, behind the dearest doors.
        let forge = lay.forge.unwrap().zone as usize;
        let rank = order.iter().position(|(name, _)| *name == lay.zones[forge].name).unwrap();
        assert!(rank + 3 >= order.len(), "{what}: the reforge opens too early ({rank} of {})", order.len());
        let dearest = lay.doors.iter().map(|d| d.price).max().unwrap();
        assert!(lay.doors.iter().filter(|d| d.zones.0 as usize == forge || d.zones.1 as usize == forge).all(|d| d.price == dearest), "{what}: the way to the reforge should cost the most");
        // Loops: more doors than zones, so the door graph has cycles to run round.
        assert!(lay.doors.len() > lay.zones.len(), "{what}: dead ends");
    }
}

#[test]
fn the_generator_arc_grates_and_relay_beacons() {
    for layout in Layout::HORDE {
        let what = format!("{layout:?}");
        let mut app = start(layout, 1);
        let lay = app.world().resource::<Map>().horde.clone().unwrap();
        {
            let mut st = app.world_mut().resource_mut::<MatchState>();
            st.horde.doors = u64::MAX;
            st.horde.survivor_mut(0).unwrap().points = 50_000;
        }
        // Without the power a grate stays dead.
        let trap = lay.traps[0];
        at_machine(&mut app, &trap.panel);
        let before = points(&app);
        use_it(&mut app);
        assert_eq!(state(&app).horde.traps[0], 0.0, "{what}: a grate needs power");
        assert_eq!(points(&app), before);

        // The generator: a tap does nothing; it takes a few seconds of cranking.
        let power = lay.power.unwrap();
        at_machine(&mut app, &power);
        use_it(&mut app);
        assert!(!state(&app).horde.power, "{what}: a tap does not restart the generator");
        hold_use(&mut app, zombies::CRANK_SECONDS * 0.5);
        let charge = state(&app).horde.charge;
        assert!(!state(&app).horde.power && charge > 0.3 && charge < 0.7, "{what}: halfway cranked ({charge:.2})");
        hold_use(&mut app, zombies::CRANK_SECONDS * 0.5 + 0.4);
        assert!(state(&app).horde.power, "{what}: cranked long enough, the power comes back");

        // Wait for a husk, then set the grate going and drop the husk on it.
        let mut seconds = 0.0;
        let husk = loop {
            invulnerable(&mut app);
            app.update();
            seconds += FRAME;
            assert!(seconds < 30.0, "{what}: no husk turned up");
            let world = app.world_mut();
            if let Some(e) = world.query_filtered::<Entity, With<Husk>>().iter(world).next() {
                break e;
            }
        };
        at_machine(&mut app, &trap.panel);
        let before = points(&app);
        use_it(&mut app);
        assert!(state(&app).horde.trap(0).0, "{what}: the grate is live");
        assert_eq!(points(&app), before - zombies::TRAP_PRICE);
        let (x0, x1, z0, z1) = trap.floor;
        {
            let world = app.world_mut();
            world.get_mut::<Transform>(husk).unwrap().translation = Vec3::new((x0 + x1) * 0.5, lay.ground, (z0 + z1) * 0.5);
            world.get_mut::<Husk>(husk).unwrap().phase = zombies::Phase2::Hunt;
        }
        run(&mut app, 0.5, invulnerable);
        let world = app.world_mut();
        assert!(world.get::<Health>(husk).is_none_or(|h| !h.alive()), "{what}: the grate fries a husk on it");
        // It runs out, cools down (and will not start again meanwhile), and is ready again.
        run(&mut app, zombies::TRAP_SECONDS, invulnerable);
        let (running, left) = state(&app).horde.trap(0);
        assert!(!running && left > 0.0, "{what}: the grate cools down after its run");
        let before = points(&app);
        use_it(&mut app);
        assert_eq!(points(&app), before, "{what}: no paying for a grate that is cooling");
        run(&mut app, zombies::TRAP_COOLDOWN, invulnerable);
        assert_eq!(state(&app).horde.trap(0), (false, 0.0), "{what}: ready again");

        // Relays: each one pays; the last pays everyone and leaves a power-up.
        let n = lay.relays.len();
        for (i, relay) in lay.relays.iter().enumerate() {
            at_machine(&mut app, relay);
            let (before, drops) = (points(&app), state(&app).horde.drops.len());
            use_it(&mut app);
            assert!(state(&app).horde.relay_on(i), "{what}: relay {i} switches on");
            let gained = points(&app) - before;
            if i + 1 < n {
                assert!(gained >= zombies::RELAY_POINTS, "{what}: relay {i} pays");
            } else {
                assert!(gained >= zombies::RELAY_POINTS + zombies::RELAY_BONUS, "{what}: the whole network pays a bonus");
                assert!(state(&app).horde.drops.len() > drops, "{what}: and leaves a power-up");
            }
            // A second press does nothing.
            let before = points(&app);
            use_it(&mut app);
            assert_eq!(points(&app), before);
        }
    }
}

#[test]
fn both_maps_run_rounds() {
    for layout in Layout::HORDE {
        let mut app = start(layout, 1);
        let mut seconds = 0.0;
        while state(&app).horde.round < 2 {
            scrap_husks(&mut app, Some(0));
            app.update();
            seconds += FRAME;
            assert!(seconds < 200.0, "{layout:?}: round 1 never ended");
        }
    }
}

/// Keeps every robot topped up so the husks can be watched all the way in.
fn invulnerable(app: &mut App) {
    let world = app.world_mut();
    for mut health in world.query_filtered::<&mut Health, With<Player>>().iter_mut(world) {
        health.hp = health.max;
    }
    let mut st = world.resource_mut::<MatchState>();
    for s in &mut st.horde.survivors {
        s.down = false;
        s.out = false;
    }
}

/// Watches every husk from the moment it turns up: where it appears, the way it comes in,
/// and whether it gets to the squad. Husks that reach a robot are scrapped (by player 0).
#[derive(Default)]
struct Watch {
    /// Per husk: inside yet, reached a robot yet.
    seen: std::collections::HashMap<Entity, (bool, bool)>,
    spawned: usize,
    reached: usize,
    climbed: usize,
    teleports: usize,
    /// Where each husk was last seen inside (to say where a stuck one got stuck).
    last: std::collections::HashMap<Entity, Vec3>,
}

impl Watch {
    fn step(&mut self, app: &mut App, what: &str) {
        let layout = app.world().resource::<Map>().horde.clone().unwrap();
        let world = app.world_mut();
        let robots: Vec<Vec3> = world.query_filtered::<&Transform, With<Player>>().iter(world).map(|t| t.translation).collect();
        let mut scrap = Vec::new();
        for (entity, transform, husk, enemy) in world.query::<(Entity, &Transform, &Husk, &oz_sim::enemy::Enemy)>().iter(world) {
            let at = transform.translation;
            let inside = layout.zone_at(at.xz()).is_some();
            let fresh = !self.seen.contains_key(&entity);
            // A husk that loses its legs comes back as a crawler right where it fell.
            let crawler = enemy.kind == zombies::kind::CRAWLER;
            if fresh && !crawler {
                self.spawned += 1;
                assert!(!inside, "{what}: a husk appeared inside the walls at {at:?} ({:?})", husk.phase);
                assert!(matches!(husk.phase, zombies::Phase2::Approach { .. }), "{what}: a husk started as {:?}", husk.phase);
                // Out in the open, clear of every building by a margin.
                for z in &layout.zones {
                    let (x0, x1, z0, z1) = z.rect;
                    let gap = (x0 - at.x).max(at.x - x1).max(z0 - at.z).max(at.z - z1);
                    assert!(gap > 1.0, "{what}: a husk appeared only {gap:.1} m from {} at {at:?}", z.name);
                }
            }
            let entry = self.seen.entry(entity).or_insert((crawler, false));
            match husk.phase {
                zombies::Phase2::Approach { .. } | zombies::Phase2::Tear { .. } => {
                    assert!(!inside, "{what}: a husk outside the boards is inside a zone at {at:?}");
                    if entry.0 && !fresh {
                        // In, and then out again: only a stuck husk is put back out.
                        self.teleports += 1;
                        entry.0 = false;
                        println!("{what}: husk stuck near {:?}, put back out", self.last.get(&entity));
                    }
                }
                zombies::Phase2::Climb { .. } => {
                    if !entry.0 {
                        self.climbed += 1;
                    }
                    entry.0 = true;
                }
                zombies::Phase2::Hunt => {
                    assert!(entry.0, "{what}: a husk is hunting inside without having climbed in at {at:?}");
                }
            }
            if inside {
                self.last.insert(entity, at);
            }
            if inside && robots.iter().any(|r| r.distance(at) < 2.2) {
                if !entry.1 {
                    entry.1 = true;
                    if !crawler {
                        self.reached += 1;
                    }
                }
                scrap.push(entity);
            }
        }
        for husk in scrap {
            world.send_event(Hit::plain(husk, 1.0e7, Some(0)));
        }
    }
}

#[test]
fn husks_only_come_in_through_the_windows_and_reach_the_squad() {
    for layout in Layout::HORDE {
        for all_open in [false, true] {
            let what = format!("{layout:?}{}", if all_open { " (every door open)" } else { "" });
            let mut app = start(layout, 1);
            if all_open {
                app.world_mut().resource_mut::<MatchState>().horde.doors = u64::MAX;
            }
            let mut watch = Watch::default();
            let mut seconds = 0.0;
            while state(&app).horde.round < 4 {
                invulnerable(&mut app);
                watch.step(&mut app, &what);
                app.update();
                seconds += FRAME;
                assert!(seconds < 600.0, "{what}: stalled in round {} with {} left ({} in, {} reached)", state(&app).horde.round, state(&app).horde.left, watch.climbed, watch.reached);
            }
            println!("{what}: {} husks, {} climbed in, {} reached the player, {} put back out, {seconds:.0} s", watch.spawned, watch.climbed, watch.reached, watch.teleports);
            assert!(watch.spawned >= 27, "{what}: rounds 1-3 send at least 27 husks");
            assert!(watch.climbed * 10 >= watch.spawned * 9, "{what}: most husks climb in through a window");
            assert!(watch.reached * 10 >= watch.spawned * 8, "{what}: most husks find the player");
            assert!(watch.teleports * 10 <= watch.spawned, "{what}: {} husks got stuck and were put back out", watch.teleports);
        }
    }
}

#[test]
fn every_window_leads_husks_to_the_squad() {
    for layout in Layout::HORDE {
        let map = GameMap::generate(WorldRecipe::horde(layout));
        let lay = map.horde.clone().unwrap();
        let what = format!("{layout:?}");
        let start = Vec3::new(map.spawn.x, lay.ground, map.spawn.y);
        let mut shut = HordeNav::build(&map, &lay, 0);
        let mut open = HordeNav::build(&map, &lay, u64::MAX);
        let s = statics(&map);
        for (i, w) in lay.windows.iter().enumerate() {
            // The way in from where they turn up is open ground.
            assert!(lay.zone_at(w.spawn.xz()).is_none(), "{what}: window {i} spawns husks inside");
            assert!(!s.blocked(w.spawn.x, w.spawn.z, w.spawn.y + 0.2, 1.4, 0.45), "{what}: window {i}'s spawn is in something");
            assert!(w.spawn.xz().distance(w.outside.xz()) >= 3.0, "{what}: window {i}'s husks turn up right at the boards");
            // Once through, a path to the squad's start.
            let nav = if w.zone == 0 { &mut shut } else { &mut open };
            let path = nav.path(w.inside, start);
            assert!(path.is_some_and(|p| p.last().unwrap().xz().distance(start.xz()) < 1.6), "{what}: no way from window {i} (zone {}) to the start", w.zone);
        }
        let zone0 = lay.windows.iter().filter(|w| w.zone == 0).count();
        assert!(zone0 >= 3, "{what}: the start zone needs a few windows, has {zone0}");
    }
}

#[test]
fn wall_guns_fit_their_chalk_frames() {
    use oz_core::worldgen::{CHALK_FRAME, chalk_fit};
    for layout in Layout::HORDE {
        let app = start(layout, 1);
        let lay = app.world().resource::<Map>().horde.clone().unwrap();
        let content = app.world().resource::<Content>().clone();
        for buy in &lay.buys {
            let def = &content.weapons[buy.weapon as usize];
            let model = oz_core::arsenal::assemble(def, &def.defaults);
            let (lo, hi) = model.bounds().expect("a gun has a model");
            let (scale, mid) = chalk_fit(lo, hi);
            // Drawn as (p - mid) * scale round the frame's centre: along the wall is Z, up is Y.
            let (a, b) = ((lo - mid) * scale, (hi - mid) * scale);
            let room = CHALK_FRAME * 0.5;
            assert!(a.z >= -room.x && b.z <= room.x, "{layout:?}: {} runs out of its frame ({:.2}..{:.2} of {:.2})", def.name, a.z, b.z, room.x);
            assert!(a.y >= -room.y && b.y <= room.y, "{layout:?}: {} sticks out of its frame ({:.2}..{:.2} of {:.2})", def.name, a.y, b.y, room.y);
            assert!((a.z + b.z).abs() < 1e-3 && (a.y + b.y).abs() < 1e-3, "{layout:?}: {} is off centre", def.name);
            // Big enough to make out: it fills most of the frame one way or the other.
            let fill = ((b.z - a.z) / CHALK_FRAME.x).max((b.y - a.y) / CHALK_FRAME.y);
            println!("{layout:?}: {} scale {scale:.2} fills {fill:.2}", def.name);
            assert!(fill > 0.65, "{layout:?}: {} is lost in its frame ({fill:.2})", def.name);
        }
    }
}

#[test]
fn a_full_squad_of_bots_plays_rounds_without_stalling() {
    // One human (kept standing, never shooting) and three bots: the bots do the work, on
    // both maps, to round six.
    for layout in Layout::HORDE {
        let mut app = start(layout, 4);
        let mut seconds = 0.0;
        while state(&app).horde.round < 6 {
            invulnerable(&mut app);
            app.update();
            seconds += FRAME;
            assert!(seconds < 1200.0, "{layout:?}: stalled in round {} with {} left", state(&app).horde.round, state(&app).horde.left);
        }
        let st = state(&app);
        let bots: u32 = st.horde.survivors.iter().filter(|s| s.id != 0).map(|s| s.kills as u32).sum();
        println!("{layout:?}: round 6 after {seconds:.0} s; bots scrapped {bots}, {} in all", st.horde.kills);
        assert!(bots >= 60, "{layout:?}: the bots should do the killing ({bots})");
    }
}

#[test]
fn volt_husks_burst_and_lockdown_boards_up() {
    let mut app = start(Layout::Blackfen, 1);
    // Skip ahead to the first round with volt husks.
    {
        let mut st = app.world_mut().resource_mut::<MatchState>();
        st.horde.round = zombies::VOLT_ROUND - 1;
        st.horde.rest = 0.1;
    }
    let mut volt = None;
    let mut seconds = 0.0;
    while volt.is_none() {
        invulnerable(&mut app);
        app.update();
        seconds += FRAME;
        assert!(seconds < 120.0, "no volt husk in round {}", zombies::VOLT_ROUND);
        let world = app.world_mut();
        volt = world.query::<(Entity, &oz_sim::enemy::Enemy)>().iter(world).find(|(_, e)| e.kind == zombies::kind::VOLT).map(|(e, _)| e);
    }
    // Bring it right up to the player, with a plain husk beside it, and scrap it.
    let world = app.world_mut();
    let me = world.query_filtered::<&Transform, With<LocalPlayer>>().single(world).unwrap().translation;
    let other = world.query::<(Entity, &oz_sim::enemy::Enemy)>().iter(world).find(|(_, e)| e.kind == zombies::kind::HUSK).map(|(e, _)| e);
    world.get_mut::<Transform>(volt.unwrap()).unwrap().translation = me + Vec3::X * 1.5;
    if let Some(other) = other {
        world.get_mut::<Transform>(other).unwrap().translation = me + Vec3::X * 2.5;
    }
    world.send_event(Hit::plain(volt.unwrap(), 1.0e7, Some(0)));
    let before = state(&app).horde.survivor(0).unwrap().kills;
    for _ in 0..4 {
        app.update();
    }
    let world = app.world_mut();
    let health = *world.query_filtered::<&Health, With<LocalPlayer>>().single(world).unwrap();
    assert!(health.hp <= health.max - zombies::VOLT_PLAYER_DAMAGE + 0.1, "the burst hurts a robot close by ({})", health.hp);
    if other.is_some() {
        assert!(state(&app).horde.survivor(0).unwrap().kills >= before + 2, "the burst takes the husk beside it too");
    }

    // A lockdown boards every open window back up.
    {
        let mut st = app.world_mut().resource_mut::<MatchState>();
        for p in &mut st.horde.planks {
            *p = 0;
        }
        st.horde.drops.push(zombies::Drop { kind: zombies::drop::LOCKDOWN, at: me + Vec3::Y * 0.9, ttl: 10.0 });
    }
    place(&mut app, me, Vec3::NEG_Z);
    app.update();
    let lay = app.world().resource::<Map>().horde.clone().unwrap();
    for (i, w) in lay.windows.iter().enumerate() {
        if w.zone == 0 {
            assert!(state(&app).horde.planks[i] >= zombies::PLANKS - 1, "window {i} boarded up again");
        }
    }
}

#[test]
fn power_ups_are_sure_to_come() {
    let mut app = start(Layout::CinderWorks, 1);
    let mut seen = 0;
    let mut kills = 0;
    let mut seconds = 0.0;
    while kills < zombies::drop_every(1) as usize * 3 {
        let before = state(&app).horde.drops.len();
        kills += scrap_husks(&mut app, Some(0));
        app.update();
        seen += state(&app).horde.drops.len().saturating_sub(before);
        seconds += FRAME;
        assert!(seconds < 600.0);
    }
    assert!(seen >= 2, "{kills} kills and only {seen} power-ups");
}
