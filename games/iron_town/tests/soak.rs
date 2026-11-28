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
/// A bot that wants to move but stays put this long is stuck.
const STUCK_SECONDS: f32 = 4.0;

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
    // Bots that want to go somewhere: where they were when they last made headway, and how long ago.
    let mut headway: HashMap<u8, (Vec3, f32)> = HashMap::new();
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
        // Stuck bots: wanting to move (and not holding still on purpose) yet getting nowhere.
        let bots: Vec<(u8, Vec3, bool, u8)> = app
            .world_mut()
            .query::<(&Player, &Transform, &Health, &PlayerInput, &Bot)>()
            .iter(app.world())
            .map(|(p, t, h, i, b)| (p.id, t.translation, h.alive() && i.movement.length() > 0.3 && !b.holding(), i.stance))
            .collect();
        for (id, feet, wants, stance_now) in bots {
            let entry = headway.entry(id).or_insert((feet, 0.0));
            if !wants || entry.0.distance(feet) > 0.5 {
                *entry = (feet, 0.0);
                continue;
            }
            entry.1 += FRAME;
            if entry.1 > STUCK_SECONDS {
                let door = map.doors.iter().enumerate().map(|(i, d)| (d.center.distance(feet), i)).min_by(|a, b| a.0.total_cmp(&b.0)).map(|(d, i)| (d, state.doors.get(i).copied()));
                let mut boxes = Vec::new();
                statics.near(feet.x, feet.z, 0.5, |_, b| {
                    if b.touches(feet.x, feet.z, 0.5) && b.top() > feet.y + 0.3 && b.bottom() < feet.y + 1.8 {
                        boxes.push(format!("{:.2}..{:.2}", b.min(), b.max()));
                    }
                });
                report.flag("STUCK", format!("bot {id} has wanted to move for {:.1}s but stayed within 0.5 m of {feet:.2} (stance {stance_now}), t={elapsed:.1}; nearest door {door:?}; boxes {boxes:?}", entry.1));
                *entry = (feet, 0.0);
            }
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
fn soak_small_and_medium_towns() {
    for seed in [2, 9] {
        for layout in [Layout::SmallTown, Layout::MediumTown] {
            check(&format!("{} {seed}", layout.title()), rules_for(WorldRecipe::grid_town(layout, seed, Biome::IronShoals)));
        }
    }
}

#[test]
fn soak_random_towns() {
    // SOAK_SEEDS=3,11,... plays those seeds' towns, whatever theme each rolls.
    let seeds: Vec<u64> = std::env::var("SOAK_SEEDS").ok().map_or(vec![3, 12], |s| s.split(',').filter_map(|n| n.trim().parse().ok()).collect());
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

/// What the bots did over a match, for judging how well they play each mode.
#[derive(Default, Debug)]
struct Behaviour {
    /// Bot-seconds alive, and of those: standing about (no headway, no shooting, not planting
    /// or defusing) for more than a few seconds; holding only the starting sidearm; holding a
    /// gun with nothing left in it or behind it.
    alive: f32,
    idle: f32,
    sidearm_only: f32,
    dry: f32,
    shots: u32,
    kills: u32,
    deaths: u32,
    /// Objective work: points taken, bombs picked up / planted / defused, loot taken, things
    /// bought, teammates revived.
    captures: u32,
    bomb_pickups: u32,
    plants: u32,
    defuses: u32,
    loot: u32,
    buys: u32,
    revives: u32,
}

impl Behaviour {
    fn idle_pct(&self) -> f32 {
        100.0 * self.idle / self.alive.max(1.0)
    }
    fn per_minute(&self, n: u32) -> f32 {
        n as f32 / (self.alive.max(1.0) / 60.0)
    }
    fn line(&self, name: &str) -> String {
        format!(
            "{name:<11} idle {:>5.1}%  shots/bot-min {:>6.1}  kills/bot-min {:>4.2}  deaths {:>3}  sidearm-only {:>5.1}%  dry {:>4.1}%  captures {}  bomb pickups {} plants {} defuses {}  loot {}  buys/stims {}  revives {}",
            self.idle_pct(),
            self.per_minute(self.shots),
            self.per_minute(self.kills),
            self.deaths,
            100.0 * self.sidearm_only / self.alive.max(1.0),
            100.0 * self.dry / self.alive.max(1.0),
            self.captures,
            self.bomb_pickups,
            self.plants,
            self.defuses,
            self.loot,
            self.buys,
            self.revives
        )
    }
}

/// Seconds without headway or a shot before a bot counts as standing about.
const IDLE_AFTER: f32 = 4.0;

/// Plays a mode for `seconds` (bots only unless `human`, who then just stands about,
/// unhurt) and measures what the bots do. Hunt hostiles (sentries) are left out: they hold
/// posts on purpose; so is planting and defusing.
fn behaviour(rules: Rules, human: bool, seconds: f32) -> Behaviour {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin, SimPlugin, IronTownPlugin));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(FRAME)));
    let horde = rules.horde;
    let setup = MatchSetup::offline(rules, "SOAK");
    let setup = if human { setup } else { MatchSetup { local_player: None, ..setup } };
    app.world_mut().resource_mut::<PendingMatch>().0 = Some(setup);
    for _ in 0..20 {
        app.update();
    }
    if !horde {
        app.world_mut().send_event(CommandEvent { player: 0, command: PlayerCommand::Lobby(LobbyAction::Start) });
        for _ in 0..(6.5 / FRAME) as usize {
            app.update();
        }
        assert_eq!(app.world().resource::<MatchState>().phase, Phase::Live);
    }
    let content = app.world().resource::<Content>().clone();
    let pistol = content.weapons.iter().position(|w| w.frame.is_some()).unwrap_or(0);
    let mut cursor = app.world().resource::<Events<FxEvent>>().get_cursor();
    let mut out = Behaviour::default();
    // Per bot: where it last made headway and when, and when it last fired.
    let mut track: HashMap<u8, (Vec3, f32, f32)> = HashMap::new();
    let mut last_obj = app.world().resource::<MatchState>().obj.clone();
    let first_bot = oz_sim::bots::FIRST_BOT_ID;
    let mut debug_store: HashMap<u8, String> = HashMap::new();
    let mut elapsed = 0.0;
    while elapsed < seconds {
        if human {
            let world = app.world_mut();
            if let Ok((mut health, mut input)) = world.query_filtered::<(&mut Health, &mut PlayerInput), With<LocalPlayer>>().single_mut(world) {
                health.hp = health.max.max(100.0);
                input.movement = Vec2::ZERO;
                input.buttons = 0;
            }
        }
        app.update();
        elapsed += FRAME;
        let state = app.world().resource::<MatchState>().clone();
        let prev_arms = std::mem::take(&mut debug_store);
        let debug_arms: HashMap<u8, String> = if std::env::var_os("BEHAVIOUR_DEBUG").is_some() {
            app.world_mut()
                .query::<(&Player, &Arsenal, &Transform)>()
                .iter(app.world())
                .map(|(p, a, t)| (p.id, format!("{:?} cur {} at {:.1}", (0..a.weapons.len()).filter(|i| a.owns(*i)).map(|i| (content.weapons[i].name, a.weapons[i].ammo, a.weapons[i].reserve)).collect::<Vec<_>>(), a.current, t.translation)))
                .collect()
        } else {
            HashMap::new()
        };
        debug_store = debug_arms.clone();
        let events = app.world().resource::<Events<FxEvent>>();
        let mut fired: Vec<u8> = Vec::new();
        for event in cursor.read(events) {
            match event {
                FxEvent::Shot { by, .. } if *by >= first_bot && *by != 255 => {
                    out.shots += 1;
                    fired.push(*by);
                }
                FxEvent::Killed { victim, by, .. } => {
                    if *by >= first_bot && *by != 255 && by != victim {
                        out.kills += 1;
                    }
                    if *victim >= first_bot {
                        out.deaths += 1;
                    }
                }
                FxEvent::Notice { to, text } if *to >= first_bot && *to != 255 && text.starts_with("+ ") => {
                    out.loot += 1;
                    if std::env::var_os("BEHAVIOUR_DEBUG").is_some() {
                        println!("{elapsed:.1} bot {to} {text} {:?} <- {:?}", debug_arms.get(to), prev_arms.get(to));
                    }
                }
                FxEvent::Cue { to, sfx } if *to >= first_bot && *sfx == oz_core::synth::Sfx::Buy => out.buys += 1,
                _ => {}
            }
        }
        // Objectives.
        let obj = &state.obj;
        for (a, b) in last_obj.points.iter().zip(&obj.points) {
            if a.owner != b.owner && b.owner != oz_sim::objective::NEUTRAL {
                out.captures += 1;
            }
        }
        use oz_sim::objective::bomb;
        if obj.bomb == bomb::CARRIED && last_obj.bomb != bomb::CARRIED {
            out.bomb_pickups += 1;
        }
        if obj.bomb == bomb::PLANTED && last_obj.bomb != bomb::PLANTED {
            out.plants += 1;
        }
        if obj.last_reason == "BOMB DEFUSED" && last_obj.last_reason != "BOMB DEFUSED" {
            out.defuses += 1;
        }
        if std::env::var_os("BEHAVIOUR_DEBUG").is_some() && (obj.stage != last_obj.stage || obj.bomb != last_obj.bomb) {
            println!("{elapsed:.1} round {} stage {} bomb {} carrier {} at {:.1} site {} {}", obj.round, obj.stage, obj.bomb, obj.carrier, obj.bomb_at, obj.site, obj.last_reason);
        }
        if std::env::var_os("BEHAVIOUR_DEBUG").is_some() && obj.bomb == bomb::CARRIED && (elapsed * 30.0) as u32 % 90 == 0 {
            let world = app.world_mut();
            for (p, t, b) in world.query::<(&Player, &Transform, &Bot)>().iter(world) {
                if p.id == obj.carrier {
                    println!("{elapsed:.1} carrier at {:.1}: {}", t.translation, b.describe());
                }
            }
        }
        last_obj = obj.clone();
        let revives: u32 = state.horde.survivors.iter().filter(|s| s.id >= first_bot).map(|s| s.revives as u32).sum();
        out.revives = out.revives.max(revives);
        if horde {
            // Husks scrapped (they are not players, so no kill feed).
            out.kills = state.horde.survivors.iter().filter(|s| s.id >= first_bot).map(|s| s.kills as u32).sum();
        }
        if (state.phase != Phase::Live && !horde) || (obj.kind == oz_sim::objective::kind::BOMB && obj.stage != oz_sim::objective::stage::PLAYING) {
            continue;
        }
        let bots: Vec<(u8, Vec3, bool, bool, bool, bool)> = app
            .world_mut()
            .query::<(&Player, &Transform, &Health, &Bot, &Arsenal)>()
            .iter(app.world())
            .map(|(p, t, h, b, a)| {
                let owned = (0..a.weapons.len()).filter(|i| a.owns(*i)).count();
                let dry = a.state().is_some_and(|w| w.ammo == 0 && w.reserve == 0);
                (p.id, t.translation, h.alive() && !state.horde.is_down(p.id), b.sentry || b.hold_use, owned == 1 && a.owns(pistol), dry)
            })
            .collect();
        for (id, feet, alive, exempt, sidearm, dry) in bots {
            let entry = track.entry(id).or_insert((feet, elapsed, elapsed));
            if !alive || exempt {
                *entry = (feet, elapsed, elapsed);
                continue;
            }
            out.alive += FRAME;
            if fired.contains(&id) {
                entry.2 = elapsed;
            }
            if feet.distance(entry.0) > 1.5 {
                entry.0 = feet;
                entry.1 = elapsed;
            }
            if elapsed - entry.1 > IDLE_AFTER && elapsed - entry.2 > IDLE_AFTER {
                out.idle += FRAME;
                if std::env::var_os("BEHAVIOUR_DEBUG").is_some() && (elapsed * 30.0) as u32 % 30 == 0 {
                    let world = app.world_mut();
                    if let Some(b) = world.query::<(&Player, &Bot)>().iter(world).find(|(p, _)| p.id == id).map(|(_, b)| b.describe()) {
                        println!("{elapsed:.1} idle bot {id} at {feet:.1}: {b}");
                    }
                }
            }
            if sidearm {
                out.sidearm_only += FRAME;
                if std::env::var_os("BEHAVIOUR_DEBUG").is_some() && (elapsed * 30.0) as u32 % 150 == 0 {
                    let world = app.world_mut();
                    let gun = world.query::<(&Transform, &oz_sim::loot::Pickup)>().iter(world).filter(|(_, p)| matches!(p.loot, oz_sim::loot::Loot::Weapon { .. })).map(|(t, _)| t.translation.distance(feet)).fold(f32::MAX, f32::min);
                    if let Some(b) = world.query::<(&Player, &Bot)>().iter(world).find(|(p, _)| p.id == id).map(|(_, b)| b.describe()) {
                        println!("{elapsed:.1} pistol bot {id} at {feet:.1}, nearest gun {gun:.1} m: {b}");
                    }
                }
            }
            if dry {
                out.dry += FRAME;
            }
        }
        if state.phase == Phase::Won && !horde {
            // Decided early: play on.
            app.world_mut().resource_mut::<MatchState>().phase = Phase::Live;
        }
    }
    out
}

fn mode_seconds() -> f32 {
    std::env::var("MODE_SECONDS").ok().and_then(|s| s.parse().ok()).unwrap_or(150.0)
}

/// Bots play every mode; how much they stand about, fight and play the objective is
/// measured and held to sane limits (run with --nocapture to see the numbers).
#[test]
fn soak_bot_behaviour_by_mode() {
    let seconds = mode_seconds();
    let mut tdm = deathmatch_rules(4);
    tdm.team_size = 4;
    tdm.score_limit = 0;
    let mut duel = deathmatch_rules(6);
    duel.team_size = 1;
    duel.score_limit = 0;
    let mut scavenge = iron_town::scavenge_rules(5);
    scavenge.score_limit = 0;
    let mut domination = iron_town::domination_rules(5);
    domination.team_size = 3;
    domination.score_limit = 0;
    let mut search = iron_town::search_rules(9);
    search.team_size = 3;
    let mut hunt = iron_town::hunt_rules(9);
    hunt.team_size = 3;
    let mut horde = iron_town::horde_rules(1);
    horde.team_size = 4;
    let only = std::env::var("MODES").ok();
    let mut lines = Vec::new();
    let mut results = Vec::new();
    for (name, rules, human) in [("tdm", tdm, false), ("duel", duel, false), ("scavenge", scavenge, false), ("domination", domination, false), ("search", search, false), ("hunt", hunt, false), ("horde", horde, true)] {
        if only.as_ref().is_some_and(|o| !o.split(',').any(|m| m == name)) {
            continue;
        }
        let b = behaviour(rules, human, seconds);
        let line = b.line(name);
        println!("{line}");
        lines.push(line);
        results.push((name, b));
    }
    let report = lines.join("\n");
    let get = |n: &str| results.iter().find(|(name, _)| *name == n).map(|(_, b)| b);
    for (name, b) in &results {
        // Horde squads wait on the (here motionless) human between waves.
        let limit = if *name == "horde" { 55.0 } else { 25.0 };
        assert!(b.idle_pct() < limit, "{name}: bots stood about {:.1}% of the time\n{report}", b.idle_pct());
        assert!(b.shots > 20, "{name}: the bots hardly fought\n{report}");
        assert!(100.0 * b.dry / b.alive.max(1.0) < 10.0, "{name}: bots stood about with empty guns\n{report}");
    }
    if let Some(b) = get("scavenge") {
        assert!(100.0 * b.sidearm_only / b.alive < 35.0, "scavenge bots stayed on the pistol\n{report}");
        assert!(b.loot >= 6, "scavenge bots hardly looted\n{report}");
    }
    if let Some(b) = get("domination") {
        assert!(b.captures >= 3, "domination bots hardly took points\n{report}");
    }
    if let Some(b) = get("search") {
        assert!(b.bomb_pickups >= 1 && b.plants + b.defuses >= 1, "search bots never played the bomb\n{report}");
    }
    if let Some(b) = get("hunt") {
        assert!(b.kills >= 2, "the hunt squad hardly fought\n{report}");
    }
    if let Some(b) = get("horde") {
        assert!(b.buys >= 1, "horde bots never bought anything\n{report}");
    }
}
