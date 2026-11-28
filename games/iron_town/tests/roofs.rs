//! Headless tests for walking over rooftops: pitched roofs are walked as the smooth
//! slopes they are drawn as, flat roofs are flat, and the feet never snap or flicker.

use bevy::{prelude::*, state::app::StatesPlugin, time::TimeUpdateStrategy};
use iron_town::{IronTownPlugin, deathmatch_rules};
use oz_sim::prelude::*;
use std::time::Duration;

const FRAME: f32 = 1.0 / 30.0;

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, SimPlugin, IronTownPlugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(FRAME)));
    app
}

fn run(app: &mut App, seconds: f32) {
    for _ in 0..(seconds / FRAME).ceil() as usize {
        app.update();
    }
}

/// A walk's feet heights and grounded flags, frame by frame.
struct Walk {
    ys: Vec<f32>,
    airborne: usize,
    covered: f32,
}

/// Puts the human at `from` facing `dir` and walks it forward for up to `distance` metres.
fn walk(app: &mut App, from: Vec3, dir: Vec2, distance: f32) -> Walk {
    let world = app.world_mut();
    let (mut transform, mut input, mut mover, mut health) = world.query_filtered::<(&mut Transform, &mut PlayerInput, &mut Mover, &mut Health), With<LocalPlayer>>().single_mut(world).unwrap();
    transform.translation = from;
    *mover = Mover { grounded: true, ..default() };
    health.hp = health.max;
    input.yaw = f32::atan2(-dir.x, -dir.y);
    input.pitch = 0.0;
    input.buttons = 0;
    input.stance = stance::STAND;
    input.movement = Vec2::Y;
    let mut out = Walk { ys: vec![from.y], airborne: 0, covered: 0.0 };
    for _ in 0..(distance / 3.0 / FRAME) as usize {
        app.update();
        let world = app.world_mut();
        let (transform, mover) = world.query_filtered::<(&Transform, &Mover), With<LocalPlayer>>().single(world).unwrap();
        out.ys.push(transform.translation.y);
        out.airborne += !mover.grounded as usize;
        out.covered = transform.translation.xz().distance(from.xz());
        if out.covered >= distance {
            break;
        }
    }
    let world = app.world_mut();
    world.query_filtered::<&mut PlayerInput, With<LocalPlayer>>().single_mut(world).unwrap().movement = Vec2::ZERO;
    out
}

/// Checks a walk was smooth: no frame-to-frame jump bigger than walking up the slope
/// gives, no going up and down and up again, and the feet stayed down throughout.
fn assert_smooth(what: &str, walk: &Walk, turns: usize) {
    let steps: Vec<f32> = walk.ys.windows(2).map(|w| w[1] - w[0]).collect();
    let biggest = steps.iter().fold(0.0f32, |m, d| m.max(d.abs()));
    // Walking 5.4 m/s up the steepest roof (0.4 up per 0.6 across) rises 0.12 a frame.
    assert!(biggest < 0.15, "{what}: the feet jumped {biggest:.2} m in one frame: {:?}", walk.ys);
    let signs: Vec<f32> = steps.iter().filter(|d| d.abs() > 0.004).map(|d| d.signum()).collect();
    let reversals = signs.windows(2).filter(|w| w[0] != w[1]).count();
    assert!(reversals <= turns, "{what}: the feet went up and down {reversals} times: {:?}", walk.ys);
    assert_eq!(walk.airborne, 0, "{what}: the feet left the roof {} times: {:?}", walk.airborne, walk.ys);
}

#[test]
fn walking_over_rooftops_is_smooth() {
    let mut app = app();
    let mut rules = deathmatch_rules(9);
    rules.no_bots = true;
    app.world_mut().resource_mut::<PendingMatch>().0 = Some(MatchSetup::offline(rules, "TEST"));
    run(&mut app, 1.0);
    app.world_mut().send_event(CommandEvent { player: 0, command: PlayerCommand::Lobby(LobbyAction::Start) });
    run(&mut app, 6.0);
    let map = app.world().resource::<Map>().0.clone();
    assert!(!map.ramps.is_empty(), "the town has no pitched roofs");

    // Pitched roofs: straight across, up one slope over the ridge and down the other,
    // and along the slope.
    let mut pitched = 0;
    for ramp in &map.ramps {
        let (centre, half) = ((ramp.min + ramp.max) * 0.5, (ramp.max - ramp.min) * 0.5);
        // Across the ridge: the way its first (main) slope rises. Usually the short way, but
        // a roof with a ladder up to it turns its gable ends to the ladder's wall.
        let across = if ramp.planes[0].x.abs() > ramp.planes[0].y.abs() { Vec2::X } else { Vec2::Y };
        let (wide, long) = (half.dot(across), half.dot(across.perp().abs()));
        for offset in [-0.6, 0.0, 0.6] {
            // Clear of the eaves, where a joined wing's roof can overlap this one.
            let from = centre - across * (wide - 0.5) + across.perp() * long * offset;
            let start = Vec3::new(from.x, ramp.height(from.x, from.y), from.y);
            let w = walk(&mut app, start, across, 2.0 * wide - 1.0);
            if w.covered < 2.0 * wide - 1.3 {
                continue; // A chimney in the way.
            }
            assert_smooth(&format!("across the roof at {centre}"), &w, 1);
            assert!(w.ys.iter().fold(f32::MIN, |m, y| m.max(*y)) > start.y + 0.15, "never climbed the roof at {centre}: {:?}", w.ys);
            pitched += 1;
            break;
        }
        let from = centre - across * wide * 0.5 - across.perp() * (long - 0.5);
        let start = Vec3::new(from.x, ramp.height(from.x, from.y), from.y);
        let w = walk(&mut app, start, across.perp(), (long - 0.5) * 2.0);
        if w.covered > long {
            assert_smooth(&format!("along the roof at {centre}"), &w, 2);
        }
    }
    assert!(pitched >= 3, "only {pitched} pitched roofs could be walked across");

    // Ladder landings (flat roofs and gable ends, walked level), from the top of each ladder inward.
    let mut flat = 0;
    for ladder in &map.ladders {
        let inward = -ladder.out;
        let from = Vec2::new(ladder.foot.x, ladder.foot.z) + inward * 0.9;
        let w = walk(&mut app, Vec3::new(from.x, ladder.top, from.y), inward, 4.0);
        if w.covered > 3.0 {
            assert_smooth(&format!("the flat roof at {from}"), &w, 0);
            flat += 1;
        }
    }
    assert!(flat >= 1, "no flat roof could be walked");
    println!("walked {pitched} pitched and {flat} flat roofs smoothly");
}
