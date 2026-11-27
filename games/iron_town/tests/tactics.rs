//! Headless tests for the tactical layer: terrorist hunt, vaulting, cooking
//! grenades and melee.

use bevy::{prelude::*, state::app::StatesPlugin, time::TimeUpdateStrategy};
use iron_town::{IronTownPlugin, deathmatch_rules, hunt_rules};
use oz_sim::{
    combat::Projectile,
    prelude::*,
    versus::{HUNT_LIVES, hunt_hostiles},
};
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

fn start(app: &mut App, rules: Rules) {
    app.world_mut().resource_mut::<PendingMatch>().0 = Some(MatchSetup::offline(rules, "TEST"));
    run(app, 2.0);
    app.world_mut().send_event(CommandEvent { player: 0, command: PlayerCommand::Lobby(LobbyAction::Start) });
    run(app, 6.0);
    assert_eq!(app.world().resource::<MatchState>().phase, Phase::Live);
}

fn me<'a>(app: &'a mut App) -> Mut<'a, PlayerInput> {
    app.world_mut().query_filtered::<&mut PlayerInput, With<LocalPlayer>>().single_mut(app.world_mut()).unwrap()
}

fn my_feet(app: &mut App) -> Vec3 {
    app.world_mut().query_filtered::<&Transform, With<LocalPlayer>>().single(app.world()).unwrap().translation
}

fn place_me(app: &mut App, at: Vec3, yaw: f32) {
    app.world_mut().query_filtered::<&mut Transform, With<LocalPlayer>>().single_mut(app.world_mut()).unwrap().translation = at;
    let mut input = me(app);
    input.yaw = yaw;
    input.pitch = 0.0;
    input.movement = Vec2::ZERO;
    input.buttons = 0;
}

#[test]
fn terrorist_hunt_hostiles_hold_posts_stay_down_and_the_squad_wins() {
    let mut app = app();
    let rules = hunt_rules(9);
    let expected = hunt_hostiles(&rules, rules.team_size);
    start(&mut app, rules);
    let state = app.world().resource::<MatchState>().clone();
    assert_eq!(state.lives, HUNT_LIVES);
    let mut hostiles = app.world_mut().query::<(Entity, &Player, &Bot, &Transform)>();
    let list: Vec<(Entity, Vec3, Vec3, u8)> = hostiles.iter(app.world()).filter(|(_, p, ..)| p.team == 1).map(|(e, p, bot, t)| (e, bot.post, t.translation, p.id)).collect();
    assert_eq!(list.len(), expected, "every hostile spawned");
    let map = app.world().resource::<Map>().0.clone();
    let base = map.team_spawns[0][0];
    for (_, post, at, _) in &list {
        assert!(Vec2::new(post.x, post.z).distance(base) > 20.0, "a hostile was posted next to the squad's base");
        assert!(at.distance(*post) < 12.0, "a hostile wandered far from its post before the shooting started");
    }
    // Hostiles that go down stay down.
    for (entity, ..) in &list {
        app.world_mut().get_mut::<Health>(*entity).unwrap().hp = 0.0;
    }
    run(&mut app, 1.0);
    let state = app.world().resource::<MatchState>().clone();
    assert_eq!(state.phase, Phase::Won);
    assert_eq!(state.winner, 0, "clearing the map wins it for the squad");
}

#[test]
fn hostiles_hear_gunfire_and_come_to_look() {
    let mut app = app();
    start(&mut app, hunt_rules(5));
    let (bot_entity, post) = {
        let mut q = app.world_mut().query::<(Entity, &Player, &Bot)>();
        q.iter(app.world()).find(|(_, p, _)| p.team == 1).map(|(e, _, b)| (e, b.post)).unwrap()
    };
    // Make a lot of noise 25 m away from one of them, out of sight behind it.
    let noise = post + Vec3::new(25.0, 0.0, 0.0);
    app.world_mut().resource_mut::<oz_sim::bots::Noises>().0.push(oz_sim::bots::Noise { at: noise, radius: 80.0, team: 0, shout: false });
    run(&mut app, 6.0);
    let at = app.world().get::<Transform>(bot_entity).unwrap().translation;
    assert!(at.distance(noise) < post.distance(noise) - 2.0 || at.distance(post) > 3.0, "the hostile ignored a gunshot ({} -> {})", post, at);
}

#[test]
fn jumping_at_low_cover_vaults_it() {
    let mut app = app();
    let mut rules = deathmatch_rules(3);
    rules.team_size = 1;
    start(&mut app, rules);
    let map = app.world().resource::<Map>().0.clone();
    let statics = app.world().resource::<Statics>().0.clone();
    // A street barrier: a solid block exactly cover height, standing on the ground.
    let barrier = map
        .blocks
        .iter()
        .filter(|b| b.solid)
        .map(|b| b.aabb)
        .find(|b| {
            let ground = map.height(b.center.x, b.center.z);
            (b.top() - ground - oz_core::worldgen::COVER_HEIGHT).abs() < 0.05 && b.half.x.min(b.half.z) < 0.35 && b.half.x.max(b.half.z) > 1.0
        })
        .expect("a street barrier");
    let across = if barrier.half.x < barrier.half.z { Vec3::X } else { Vec3::Z };
    let start_at = barrier.center - across * (barrier.half.dot(across) + 0.6);
    let start_at = Vec3::new(start_at.x, oz_sim::player::ground_under(&map, &statics, start_at.x, start_at.z, start_at.y + 0.5), start_at.z);
    // Face the barrier.
    let yaw = f32::atan2(-across.x, -across.z);
    place_me(&mut app, start_at, yaw);
    run(&mut app, 0.2);
    {
        let mut input = me(&mut app);
        input.movement = Vec2::Y;
        input.buttons = buttons::JUMP;
    }
    run(&mut app, 1.2);
    let feet = my_feet(&mut app);
    let side = (feet - barrier.center).dot(across);
    assert!(side > barrier.half.dot(across), "did not get over the barrier: {feet} vs {}", barrier.center);
}

fn grenades(app: &mut App) -> Vec<Projectile> {
    app.world_mut().query::<&Projectile>().iter(app.world()).copied().collect()
}

#[test]
fn grenades_cook_in_the_hand_and_can_be_put_away() {
    let mut app = app();
    let mut rules = deathmatch_rules(8);
    rules.team_size = 1;
    app.world_mut().resource_mut::<PendingMatch>().0 = Some(MatchSetup::offline(rules, "TEST"));
    run(&mut app, 2.0);
    // A frag in the class.
    let loadout = Loadout { primary: oz_core::kit::gun::CARBINE as u8, secondary: oz_core::kit::gun::PISTOL as u8, perk: 0, gear: 0, parts: None, streaks: [0; 3] };
    app.world_mut().send_event(CommandEvent { player: 0, command: PlayerCommand::Loadout(loadout) });
    app.world_mut().send_event(CommandEvent { player: 0, command: PlayerCommand::Lobby(LobbyAction::Start) });
    run(&mut app, 6.0);
    let gear_left = |app: &mut App| app.world_mut().query_filtered::<&Player, With<LocalPlayer>>().single(app.world()).unwrap().gear_left;
    let before = gear_left(&mut app);
    assert!(before > 0);

    // Pull the pin, then put it back: nothing thrown, nothing used.
    me(&mut app).buttons = buttons::GEAR;
    run(&mut app, 0.4);
    me(&mut app).buttons = buttons::GEAR | buttons::RELOAD;
    run(&mut app, 0.1);
    me(&mut app).buttons = 0;
    run(&mut app, 0.5);
    assert!(grenades(&mut app).is_empty(), "the grenade left the hand after the pin went back");
    assert_eq!(gear_left(&mut app), before);

    // Cook for 0.8 s and throw: the fuse in flight is that much shorter.
    me(&mut app).buttons = buttons::GEAR;
    run(&mut app, 0.8);
    assert!(grenades(&mut app).is_empty(), "it flew before it was let go");
    me(&mut app).buttons = 0;
    app.update();
    let thrown = grenades(&mut app);
    assert_eq!(thrown.len(), 1);
    assert!(thrown[0].life < 1.2 && thrown[0].life > 0.6, "fuse left {}", thrown[0].life);
    assert_eq!(gear_left(&mut app), before - 1);
}

#[test]
fn melee_drops_an_enemy_in_arms_reach() {
    let mut app = app();
    let mut rules = deathmatch_rules(12);
    rules.team_size = 1;
    start(&mut app, rules);
    let me_at = my_feet(&mut app);
    let enemy = {
        let mut q = app.world_mut().query::<(Entity, &Player)>();
        q.iter(app.world()).find(|(_, p)| p.team == 1).map(|(e, _)| e).unwrap()
    };
    // Put the enemy right in front of us, and keep it busy elsewhere.
    app.world_mut().entity_mut(enemy).remove::<Bot>();
    app.world_mut().get_mut::<Transform>(enemy).unwrap().translation = me_at + Vec3::new(0.0, 0.0, -1.2);
    app.world_mut().get_mut::<Player>(enemy).unwrap().protect = 0.0;
    place_me(&mut app, me_at, 0.0);
    app.world_mut().query_filtered::<&mut Player, With<LocalPlayer>>().single_mut(app.world_mut()).unwrap().protect = 0.0;
    app.update();
    me(&mut app).buttons = buttons::MELEE;
    app.update();
    app.update();
    me(&mut app).buttons = 0;
    app.update();
    let health = *app.world().get::<Health>(enemy).unwrap();
    assert!(health.hp < health.max - 50.0, "the strike did not land: {health:?}");
}

/// Every place a robot can stand on the maps, checked against the decoration that is drawn
/// but has no collision: nothing you could take cover behind may be walked into. Thin things
/// (poles, pipes, wires) and bushes are meant to be walked through.
#[test]
fn nothing_drawn_can_be_walked_into() {
    use oz_core::worldgen::{Layout, Weather, WorldRecipe};
    let recipes = [
        deathmatch_rules(3).recipe,
        deathmatch_rules(17).recipe,
        WorldRecipe::preset(Layout::Crossroads, oz_core::worldgen::Biome::IronShoals, Weather::Clear),
        WorldRecipe::preset(Layout::Dockyard, oz_core::worldgen::Biome::IronShoals, Weather::Clear),
    ];
    let mut problems = Vec::new();
    for recipe in recipes {
        let mut rules = deathmatch_rules(recipe.seed);
        rules.recipe = recipe;
        rules.team_size = 1;
        let mut app = app();
        app.world_mut().resource_mut::<PendingMatch>().0 = Some(MatchSetup::offline(rules, "TEST"));
        run(&mut app, 0.5);
        let map = app.world().resource::<Map>().0.clone();
        let statics = app.world().resource::<Statics>().0.clone();
        for block in map.blocks.iter().filter(|b| !b.solid) {
            let b = block.aabb;
            // The rule: what you could take cover behind is solid; paint-thin trims and floor
            // finishes, and anything too thin to hide behind (poles, masts, pipes, wires), you walk through.
            if b.half.min_element() < 0.05 || b.half.x.min(b.half.z) < 0.1 {
                continue;
            }
            let (radius, height) = (0.3f32, 1.75f32);
            let mut x = b.center.x - b.half.x - radius;
            'scan: while x <= b.center.x + b.half.x + radius {
                let mut z = b.center.z - b.half.z - radius;
                while z <= b.center.z + b.half.z + radius {
                    let mut tops = statics.tops_at(x, z);
                    tops.push(map.height(x, z));
                    for top in tops {
                        let feet = oz_sim::player::ground_under(&map, &statics, x, z, top + 0.05);
                        if (feet - top).abs() > 0.05 || statics.blocked(x, z, feet, height, radius) || !map.in_bounds(x, z) {
                            continue;
                        }
                        // How far the body would sink into the block on each axis.
                        let dx = radius + b.half.x - (x - b.center.x).abs();
                        let dz = radius + b.half.z - (z - b.center.z).abs();
                        let dy = (feet + height).min(b.top()) - feet.max(b.bottom());
                        if dx > 0.1 && dz > 0.1 && dy > 0.1 {
                            problems.push(format!("{:?} seed {}: decor {:?} at {:?} (half {:?}), standing at ({x:.2}, {feet:.2}, {z:.2})", recipe.layout, recipe.seed, block.paint, b.center, b.half));
                            break 'scan;
                        }
                    }
                    z += 0.15;
                }
                x += 0.15;
            }
        }
    }
    assert!(problems.is_empty(), "{} spots to walk into scenery:\n{}", problems.len(), problems.iter().take(40).cloned().collect::<Vec<_>>().join("\n"));
}

#[test]
fn killstreak_aircraft_can_be_shot_down() {
    use oz_sim::killstreak::{Aircraft, Recon};
    let mut app = app();
    let mut rules = deathmatch_rules(6);
    rules.team_size = 1;
    start(&mut app, rules);
    // Earn and call in a UAV: a radar plane appears over the town.
    for _ in 0..3 {
        app.world_mut().resource_mut::<MatchState>().credits.push(0);
        app.update();
    }
    app.world_mut().send_event(CommandEvent { player: 0, command: PlayerCommand::UseStreak });
    run(&mut app, 0.2);
    assert!(app.world().resource::<MatchState>().uav[0] > 0.0);
    let (plane, at) = {
        let mut q = app.world_mut().query_filtered::<(Entity, &Transform), With<Recon>>();
        q.single(app.world()).map(|(e, t)| (e, t.translation)).expect("a radar plane in the sky")
    };
    // The enemy shoots straight at it until it falls.
    let enemy = {
        let mut q = app.world_mut().query::<(Entity, &Player)>();
        q.iter(app.world()).find(|(_, p)| p.team == 1).map(|(e, _)| e).unwrap()
    };
    app.world_mut().entity_mut(enemy).remove::<Bot>();
    let mut down = false;
    for _ in 0..(10.0 / FRAME) as usize {
        let target = app.world().get::<Transform>(plane).map(|t| t.translation).unwrap_or(at);
        let feet = app.world().get::<Transform>(enemy).unwrap().translation;
        let to = target - (feet + Vec3::Y * EYE);
        let mut input = app.world_mut().get_mut::<PlayerInput>(enemy).unwrap();
        input.yaw = f32::atan2(-to.x, -to.z);
        input.pitch = (to.y / to.length()).asin();
        // Its sidearm: plain bullets.
        input.weapon = oz_core::kit::gun::PISTOL as u8;
        input.buttons = if input.buttons & buttons::FIRE == 0 { buttons::FIRE | buttons::AIM } else { buttons::AIM };
        app.update();
        if app.world().get_entity(plane).is_err() {
            down = true;
            break;
        }
    }
    assert!(down, "the radar plane survived ten seconds of fire: {:?}", app.world().get::<Aircraft>(plane).map(|a| a.hp));
    assert_eq!(app.world().resource::<MatchState>().uav[0], 0.0, "the UAV went down with its plane");
}

#[test]
fn scavenge_starts_with_a_pistol_and_loot_arms_you() {
    use oz_sim::loot::{Loot, Pickup};
    let mut app = app();
    let mut rules = iron_town::scavenge_rules(15);
    rules.team_size = 1;
    start(&mut app, rules);
    run(&mut app, 3.0);
    let owned = |app: &mut App| {
        let arsenal = app.world_mut().query_filtered::<&Arsenal, With<LocalPlayer>>().single(app.world()).unwrap().clone();
        (0..arsenal.weapons.len()).filter(|i| arsenal.owns(*i)).collect::<Vec<_>>()
    };
    assert_eq!(owned(&mut app), vec![oz_core::kit::gun::PISTOL], "scavengers spawn with only a pistol");
    let loot: Vec<(Vec3, Loot)> = app.world_mut().query::<(&Transform, &Pickup)>().iter(app.world()).map(|(t, p)| (t.translation, p.loot)).collect();
    assert!(loot.len() > 20, "the town should be stocked: {} pickups", loot.len());
    // Walk onto a gun: it is ours.
    let (at, gun) = loot.iter().find_map(|(at, l)| match l {
        Loot::Weapon { index, .. } => Some((*at, *index)),
        _ => None,
    }).expect("a gun somewhere");
    place_me(&mut app, at, 0.0);
    run(&mut app, 0.3);
    assert!(owned(&mut app).contains(&(gun as usize)), "walking over the gun did not pick it up");
    // Go down: the gun drops where we fell.
    app.world_mut().query_filtered::<&mut Health, With<LocalPlayer>>().single_mut(app.world_mut()).unwrap().hp = 0.0;
    run(&mut app, 0.3);
    let dropped = app.world_mut().query::<(&Transform, &Pickup)>().iter(app.world()).any(|(t, p)| p.life.is_some() && t.translation.distance(at) < 2.0 && matches!(p.loot, Loot::Weapon { .. }));
    assert!(dropped, "the fallen should drop their gun");
}

#[test]
fn cover_facing_round_trips() {
    use oz_sim::player::{cover_code, cover_yaw};
    // A wall facing +Z: the body faces +Z (back to the wall), i.e. yaw of half a turn.
    let yaw = cover_yaw(cover_code(Vec3::Z)).unwrap();
    let facing = Vec3::new(-yaw.sin(), 0.0, -yaw.cos());
    assert!(facing.distance(Vec3::Z) < 1e-4, "{facing}");
    assert_eq!(cover_yaw(0), None);
    // Every eighth survives the four bits it travels in.
    for i in 0..8 {
        let angle = i as f32 * std::f32::consts::FRAC_PI_4;
        let code = cover_code(Vec3::new(-angle.sin(), 0.0, -angle.cos()));
        assert!(code < 16);
        assert!((cover_yaw(code).unwrap() - angle).abs() < 1e-4);
    }
}

#[test]
fn leaves_hide_what_is_behind_them() {
    use oz_sim::bots::leaves_between;
    let crown = [(Vec3::new(0.0, 2.0, 0.0), 1.5)];
    // Straight through a crown: hidden. Past it: seen. From inside your own bush: you see out.
    assert!(leaves_between(&crown, Vec3::new(-5.0, 2.0, 0.0), Vec3::new(5.0, 2.0, 0.0)));
    assert!(!leaves_between(&crown, Vec3::new(-5.0, 2.0, 4.0), Vec3::new(5.0, 2.0, 4.0)));
    assert!(!leaves_between(&crown, Vec3::new(0.0, 2.0, 0.0), Vec3::new(8.0, 2.0, 0.0)));
    // Every town has some.
    let map = oz_core::worldgen::World::generate(deathmatch_rules(3).recipe);
    assert!(!map.foliage().is_empty());
}

#[test]
fn vaulting_from_a_crouch_in_cover() {
    let mut app = app();
    let mut rules = deathmatch_rules(3);
    rules.team_size = 1;
    start(&mut app, rules);
    let map = app.world().resource::<Map>().0.clone();
    let statics = app.world().resource::<Statics>().0.clone();
    let mut tried = 0;
    for barrier in map.blocks.iter().filter(|b| b.solid).map(|b| b.aabb).filter(|b| {
        let ground = map.height(b.center.x, b.center.z);
        (b.top() - ground - oz_core::worldgen::COVER_HEIGHT).abs() < 0.05 && b.half.x.min(b.half.z) < 0.35 && b.half.x.max(b.half.z) > 1.0
    }) {
        let across = if barrier.half.x < barrier.half.z { Vec3::X } else { Vec3::Z };
        // Pressed against it the way cover leaves you: 0.38 m off the face, crouched, looking any way.
        let at = barrier.center - across * (barrier.half.dot(across) + 0.38);
        let at = Vec3::new(at.x, oz_sim::player::ground_under(&map, &statics, at.x, at.z, at.y + 0.5), at.z);
        if statics.blocked(at.x, at.z, at.y, 1.25, 0.3) {
            continue;
        }
        tried += 1;
        place_me(&mut app, at, 1.0);
        run(&mut app, 0.2);
        {
            let mut input = me(&mut app);
            input.stance = stance::CROUCH;
            // Movement into the wall, relative to a view turned away from it.
            let yaw = input.yaw;
            let forward = Vec3::new(-yaw.sin(), 0.0, -yaw.cos());
            let right = Vec3::new(yaw.cos(), 0.0, -yaw.sin());
            input.movement = Vec2::new(across.dot(right), across.dot(forward));
            input.buttons = buttons::JUMP;
        }
        run(&mut app, 1.2);
        let feet = my_feet(&mut app);
        assert!((feet - barrier.center).dot(across) > barrier.half.dot(across), "did not vault from cover: {feet} vs {}", barrier.center);
        break;
    }
    assert!(tried > 0, "no barrier to try");
}
