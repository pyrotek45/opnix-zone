//! Soak test: full bot matches on every kind of map, checking the rules of
//! the world every single tick. Nobody may stand inside a wall, sink below
//! the ground, leave the map, hang in the air, or teleport; no bullet may
//! reach a robot through a wall.
//!
//! The default run is short. `SOAK_SECONDS=600 cargo test --release -p
//! iron_town --test soak -- --nocapture` plays much longer matches.

use bevy::{ecs::event::Events, prelude::*, state::app::StatesPlugin, time::TimeUpdateStrategy};
use iron_town::{IronTownPlugin, deathmatch_rules};
use oz_core::worldgen::{Biome, Layout, Theme, Weather, WorldRecipe};
use oz_sim::prelude::*;
use std::{collections::HashMap, time::Duration};

const FRAME: f32 = 1.0 / 30.0;

#[derive(Default)]
struct Report {
    problems: Vec<String>,
    shots: usize,
    ticks: usize,
}

impl Report {
    fn flag(&mut self, kind: &str, text: String) {
        // Keep the report readable: a handful of examples per kind.
        if self.problems.iter().filter(|p| p.starts_with(kind)).count() < 6 {
            self.problems.push(format!("{kind}: {text}"));
        }
    }
}

fn soak(rules: Rules, seconds: f32) -> Report {
    let rules_seed = rules.recipe.seed;
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, SimPlugin, IronTownPlugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(FRAME)));
    app.world_mut().resource_mut::<PendingMatch>().0 = Some(MatchSetup::offline(rules, "SOAK"));
    for _ in 0..20 {
        app.update();
    }
    app.world_mut().send_event(CommandEvent { player: 0, command: PlayerCommand::Lobby(LobbyAction::Start) });
    for _ in 0..(6.5 / FRAME) as usize {
        app.update();
    }
    assert_eq!(app.world().resource::<MatchState>().phase, Phase::Live);

    let map = app.world().resource::<Map>().0.clone();
    let statics = app.world().resource::<Statics>().0.clone();
    let half = map.recipe.half_extent;
    let mut report = Report::default();
    let mut cursor = app.world().resource::<Events<FxEvent>>().get_cursor();
    let mut last: HashMap<u8, (Vec3, f32)> = HashMap::new();
    let mut airborne: HashMap<u8, f32> = HashMap::new();
    let mut trails: HashMap<u8, Vec<(f32, Vec3, bool, u8)>> = HashMap::new();
    let mut elapsed = 0.0;
    // The human seat is a chaos monkey: random runs, jumps, stance changes,
    // leaning and shooting, changing every fraction of a second.
    let mut chaos = oz_core::rng::Rng::new(rules_seed ^ 0xC4A05);
    let mut wish = PlayerInput::default();
    while elapsed < seconds {
        if chaos.chance(0.12) {
            wish.movement = Vec2::new(chaos.range(-1.0, 1.0), chaos.range(-0.3, 1.0)).clamp_length_max(1.0);
            wish.yaw += chaos.range(-1.2, 1.2);
            wish.pitch = chaos.range(-0.6, 0.6);
            wish.buttons = 0;
            for (button, odds) in [(buttons::JUMP, 0.3), (buttons::SPRINT, 0.4), (buttons::FIRE, 0.4), (buttons::AIM, 0.3), (buttons::GEAR, 0.03), (buttons::RELOAD, 0.05)] {
                if chaos.chance(odds) {
                    wish.buttons |= button;
                }
            }
            if chaos.chance(0.2) {
                wish.stance = (chaos.range(0.0, 2.99)) as u8;
            }
            wish.lean = if chaos.chance(0.15) { if chaos.chance(0.5) { -1 } else { 1 } } else { 0 };
        }
        let mut monkey = app.world_mut().query_filtered::<&mut PlayerInput, With<LocalPlayer>>();
        if let Ok(mut input) = monkey.single_mut(app.world_mut()) {
            *input = PlayerInput { weapon: input.weapon, ..wish };
        }
        app.update();
        elapsed += FRAME;
        report.ticks += 1;
        // The robots' bodies as the sim left them this tick.
        let bodies: Vec<(u8, Vec3, bool, f32, bool, PlayerInput)> = app
            .world_mut()
            .query::<(&Player, &Transform, &Health, &Mover, &PlayerInput)>()
            .iter(app.world())
            .map(|(p, t, h, m, i)| (p.id, t.translation, h.alive(), p.life, m.grounded, *i))
            .collect();
        let state = app.world().resource::<MatchState>().clone();
        if state.phase != Phase::Live {
            // The match was won: play the next one.
            app.world_mut().resource_mut::<MatchState>().phase = Phase::Live;
            app.world_mut().resource_mut::<MatchState>().scores = [0, 0];
        }
        for (id, feet, alive, life, grounded, input) in &bodies {
            let (id, feet) = (*id, *feet);
            if !feet.is_finite() {
                report.flag("NOT FINITE", format!("player {id} at {feet}"));
                continue;
            }
            if !alive {
                last.remove(&id);
                airborne.remove(&id);
                continue;
            }
            let ground = map.height(feet.x, feet.z);
            if feet.y < ground - 0.25 {
                report.flag("UNDER GROUND", format!("player {id} at {feet:.2}, ground {ground:.2}, t={elapsed:.1}"));
            }
            if feet.x.abs() > half + 0.01 || feet.z.abs() > half + 0.01 {
                report.flag("OUT OF BOUNDS", format!("player {id} at {feet:.2}, edge {half}, t={elapsed:.1}"));
            }
            // A slimmer body than the real one, so standing against a wall is fine.
            if statics.blocked(feet.x, feet.z, feet.y + 0.05, stance::height(input.stance) - 0.1, 0.18) {
                if std::env::var_os("SOAK_DEBUG").is_some() && !report.problems.iter().any(|p| p.starts_with("INSIDE")) {
                    let mut boxes = Vec::new();
                    statics.near(feet.x, feet.z, 0.18, |_, b| if b.touches(feet.x, feet.z, 0.18) { boxes.push(format!("{:.2}..{:.2}", b.min(), b.max())) });
                    let trail: Vec<String> = trails.get(&id).map(|t| t.iter().map(|(t, p, g, st)| format!("{t:.2} {p:.2} {g} stance={st}")).collect()).unwrap_or_default();
                    println!("INSIDE {id} at {feet:.2}\nboxes {boxes:?}\n{}", trail.join("\n"));
                }
                report.flag("INSIDE WALL", format!("player {id} at {feet:.2}, t={elapsed:.1}"));
            }
            let trail = trails.entry(id).or_default();
            trail.push((elapsed, feet, *grounded, input.stance));
            if trail.len() > 40 {
                trail.remove(0);
            }
            let air = airborne.entry(id).or_default();
            *air = if *grounded { 0.0 } else { *air + FRAME };
            if *air > 4.0 {
                report.flag("FLYING", format!("player {id} off the ground for {air:.1}s at {feet:.2}, support {:.2}", statics.support(feet.x, feet.z, feet.y + 0.3).max(ground)));
                *air = -999.0;
            }
            if let Some((before, before_life)) = last.get(&id)
                && *life > *before_life
                && before.distance(feet) > 1.2
            {
                report.flag("TELEPORT", format!("player {id} jumped {:.2} m in one tick, {before:.2} -> {feet:.2}", before.distance(feet)));
            }
            last.insert(id, (feet, *life));
        }
        // Bullets: the line from the shooter's eye to where the shot ended must be clear.
        let events = app.world().resource::<Events<FxEvent>>();
        let shots: Vec<(u8, Vec3, bool)> = cursor
            .read(events)
            .filter_map(|e| match e {
                FxEvent::Shot { by, to, beam, .. } => Some((*by, *to, *beam)),
                _ => None,
            })
            .collect();
        for (by, to, _beam) in shots {
            let Some((_, feet, _, _, _, input)) = bodies.iter().find(|b| b.0 == by) else { continue };
            report.shots += 1;
            let eye = oz_sim::player::eye_point(&statics, *feet, input);
            let path = to - eye;
            let length = path.length();
            if length < 0.3 {
                continue;
            }
            let dir = path / length;
            if let Some((d, index)) = statics.raycast(eye, dir, length - 0.15) {
                let thickness = statics.thickness(index, eye + dir * d, dir);
                // Piercing rounds may cross one thin wall; anything thicker is a bug.
                if thickness > 0.7 {
                    report.flag("SHOT THROUGH WALL", format!("player {by} from {eye:.2} to {to:.2}: wall at {d:.2} m, {thickness:.2} m thick"));
                }
            }
        }
    }
    report
}

fn seconds() -> f32 {
    std::env::var("SOAK_SECONDS").ok().and_then(|s| s.parse().ok()).unwrap_or(90.0)
}

fn check(name: &str, rules: Rules) {
    let report = soak(rules, seconds());
    println!("{name}: {} ticks, {} shots checked", report.ticks, report.shots);
    assert!(report.shots > 50, "{name}: the bots hardly fought ({} shots)", report.shots);
    assert!(report.problems.is_empty(), "{name}:\n{}", report.problems.join("\n"));
}

fn rules_for(recipe: WorldRecipe) -> Rules {
    let mut rules = deathmatch_rules(recipe.seed);
    rules.recipe = recipe;
    rules.team_size = 4;
    rules.score_limit = 0;
    rules
}

#[test]
fn soak_random_towns() {
    // SOAK_SEEDS=3,11,... plays those seeds' towns, whatever theme each rolls.
    let seeds: Vec<u64> = std::env::var("SOAK_SEEDS").ok().map_or(vec![3], |s| s.split(',').filter_map(|n| n.trim().parse().ok()).collect());
    for seed in seeds {
        check(&format!("town {seed}"), rules_for(deathmatch_rules(seed).recipe));
    }
}

fn soak_theme(theme: Theme) {
    check(&format!("{theme:?}"), rules_for(WorldRecipe::themed(11, Biome::IronShoals, theme)));
}

#[test]
fn soak_suburbs() {
    soak_theme(Theme::Suburbs);
}

#[test]
fn soak_desert_outpost() {
    soak_theme(Theme::DesertOutpost);
}

#[test]
fn soak_farmland() {
    soak_theme(Theme::Farmland);
}

#[test]
fn soak_industrial() {
    soak_theme(Theme::Industrial);
}

#[test]
fn soak_woods_park() {
    soak_theme(Theme::WoodsPark);
}

fn soak_fixed(layout: Layout) {
    check(layout.title(), rules_for(WorldRecipe::preset(layout, Biome::IronShoals, Weather::Clear)));
}

#[test]
fn soak_crossroads() {
    soak_fixed(Layout::Crossroads);
}

#[test]
fn soak_dockyard() {
    soak_fixed(Layout::Dockyard);
}

#[test]
fn soak_office_tower() {
    soak_fixed(Layout::OfficeTower);
}

#[test]
fn soak_space_station() {
    soak_fixed(Layout::SpaceStation);
}

#[test]
fn soak_mountainside() {
    soak_fixed(Layout::Mountainside);
}
