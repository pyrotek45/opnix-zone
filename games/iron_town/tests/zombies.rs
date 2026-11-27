//! Headless horde-mode tests: rounds, points and buying, downs and revives, game over, the
//! husks' hunt, and the two horde maps' layouts.

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
    place(&mut app, power.pos, -Vec3::new(power.facing.x, 0.0, power.facing.y));
    use_it(&mut app);
    assert!(state(&app).horde.power, "the switch should turn the power on");
    place(&mut app, machine.pos, face);
    use_it(&mut app);
    run(&mut app, 0.2, keep_clear);
    assert!(state(&app).horde.survivor(0).unwrap().has(perk::IRON_HIDE));
    let world = app.world_mut();
    let health = *world.query_filtered::<&Health, With<LocalPlayer>>().single(world).unwrap();
    assert_eq!(health.max, 250.0, "iron hide plates you up");

    // The reforge takes the gun in hand and hands back its reforged twin.
    let forge = layout.forge.unwrap();
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
        let reaches = |nav: &mut HordeNav, to: Vec3| nav.path(start, to).is_some_and(|p| p.last().unwrap().xz().distance(to.xz()) < 1.6);
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
        for (i, m) in lay.perks.iter().chain(&lay.crates).chain(lay.forge.iter()).chain(lay.power.iter()).enumerate() {
            spots.push((format!("machine {i}"), m.pos));
        }
        for (i, b) in lay.buys.iter().enumerate() {
            spots.push((format!("wall buy {i}"), ground(b.pos.xz() + b.normal * 0.7)));
        }
        for (i, v) in lay.vents.iter().enumerate() {
            spots.push((format!("vent {i}"), v.pos));
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
