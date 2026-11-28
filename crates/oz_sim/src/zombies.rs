//! Horde: round-based co-op survival against husks - scrap robots that gather out in the
//! dark and claw their way in through the barricaded windows (the only way in).
//!
//! The squad (humans, plus bots for any empty seats) starts in one zone of a horde map
//! (`worldgen::HordeLayout`) with a pistol and 500 points. Points come from hits and kills
//! (more for headshots and melee) and from boarding windows back up, and buy everything:
//! guns chalked on the walls (and ammo for them), doors and rubble into the rest of the map,
//! perks from machines once the power is on, a gamble at the salvage crate, a trip through
//! the reforge for a much better gun, and arc grates that fry husks for a while.
//!
//! The power is off at the start: someone has to crank the generator back to life (hold Use
//! on it for a few seconds) before the perk machines, the reforge and the grates work. Relay
//! beacons are dotted round each map: switch every one of them on for a bonus.
//!
//! Rounds escalate: more husks, tougher ones, faster ones, brutes every fifth round, volt husks
//! that burst when scrapped from the sixth, and husks that lose their legs keep coming on
//! their hands. The last few of a round break into a run. Kills now and then (and surely,
//! every so many) drop power-ups. A player who
//! goes down crawls with a pistol until a teammate holds Use on them, or bleeds out and sits
//! out the round. When everyone is down at once, the run is over.
//!
//! Everything the clients need to draw it travels in [`MatchState::horde`]; the husks are
//! ordinary replicated `Enemy` entities whose kind picks the model.

use crate::{
    SimSet,
    bots::{Bot, FIRST_BOT_ID},
    combat::{Grid, GridItem, Hit},
    nav::{Nav, NavScratch},
    net::NetIds,
    player::{Arsenal, Health, LocalPlayer, Player, PlayerInput, buttons, ground_under, stance},
    session::{Content, Map, MatchEntity, Statics, player_bundle},
    sim_dt,
    state::{CommandEvent, FxEvent, MatchPhase, MatchState, Phase, PlayerCommand, Rules, SimRng},
    weapons::grant_weapon,
};
use bevy::{platform::collections::HashMap, prelude::*};
use oz_core::{
    arsenal::{ModKind, WeaponState},
    collide::{Aabb, StaticWorld},
    content::Catalog,
    kit::gun,
    synth::Sfx,
    worldgen::{HordeLayout, World as GameMap},
};
use serde::{Deserialize, Serialize};

pub use oz_core::worldgen::perk_id as perk_ids;

/// Perks: ids, names and prices. The effects live in [`perk_effects`] and around the module.
pub mod perk {
    use oz_core::model::Paint;
    pub use oz_core::worldgen::perk_id::*;
    pub const COUNT: usize = 7;
    pub const NAMES: [&str; COUNT] = ["IRON HIDE", "QUICKFIX", "HAIR TRIGGER", "PATCH-UP", "FLEETFOOT", "SPARE SOCKET", "SCRAP MAGNET"];
    pub const BLURBS: [&str; COUNT] = [
        "Plating: take two and a half times the punishment",
        "Reload nearly twice as fast",
        "Fire a third faster, hit a fifth harder",
        "Revive teammates twice as fast. Alone: get yourself back up",
        "Run and walk faster",
        "Carry a third gun",
        "Kills pay half again; power-ups turn up twice as often",
    ];
    pub const PRICES: [u32; COUNT] = [2500, 3000, 2000, 1500, 2000, 4000, 2500];
    /// Two-letter badges for the HUD.
    pub const SHORT: [&str; COUNT] = ["IH", "QF", "HT", "PU", "FF", "SS", "SM"];
    pub const COLORS: [Paint; COUNT] = [Paint::Red, Paint::Green, Paint::Gold, Paint::Blue, Paint::Coral, Paint::Violet, Paint::Teal];
    /// Most perks one robot can hold.
    pub const MAX: u32 = 4;
}

/// `PlayerCommand::Mode` codes the horde director understands.
pub mod act {
    /// Use whatever you are at: buy, open, take, switch on.
    pub const USE: u8 = 40;
    /// Use is held (sent a few times a second): revive, rebuild boards.
    pub const HOLD: u8 = 41;
}

/// Catalog enemy indices the horde mode expects.
pub mod kind {
    pub const HUSK: u16 = 0;
    pub const BRUTE: u16 = 1;
    pub const CRAWLER: u16 = 2;
    /// Overcharged: sprints, and bursts when scrapped (hurting robots and husks close by).
    pub const VOLT: u16 = 3;
}

/// A volt husk's burst: reach, and what it does to robots and to other husks.
pub const VOLT_RADIUS: f32 = 3.2;
pub const VOLT_PLAYER_DAMAGE: f32 = 45.0;
pub const VOLT_HUSK_DAMAGE: f32 = 900.0;

/// Power-ups husks drop.
pub mod drop {
    pub const AMMO: u8 = 0;
    pub const DOUBLE: u8 = 1;
    pub const INSTA: u8 = 2;
    pub const NUKE: u8 = 3;
    /// Every window boarded up again, and points for the trouble.
    pub const LOCKDOWN: u8 = 4;
    /// Points for everyone.
    pub const HAUL: u8 = 5;
    /// The salvage crate for next to nothing, for a while.
    pub const CLEARANCE: u8 = 6;
    pub const COUNT: u8 = 7;
    pub const NAMES: [&str; COUNT as usize] = ["FULL LOAD", "DOUBLE SCRAP", "KILLSWITCH", "MELTDOWN", "LOCKDOWN", "SCRAP HAUL", "CLEARANCE"];
    /// What each does, for the pickup notice.
    pub const BLURBS: [&str; COUNT as usize] = [
        "every gun topped up",
        "points count double",
        "every hit is a kill",
        "every husk scrapped",
        "every window boarded",
        "500 points each",
        "the crate for 10 points",
    ];
    /// How often each turns up (relative).
    pub const WEIGHTS: [u32; COUNT as usize] = [4, 3, 2, 2, 2, 2, 1];
}

pub const START_POINTS: u32 = 500;
pub const CRATE_PRICE: u32 = 950;
/// The crate's price while a clearance runs.
pub const SALE_PRICE: u32 = 10;
pub const FORGE_PRICE: u32 = 5000;
pub const UPGRADED_AMMO: u32 = 2500;
pub const PLANKS: u8 = 6;
/// Seconds of holding Use on the generator to bring the power back.
pub const CRANK_SECONDS: f32 = 5.0;
/// Arc grates: price, how long one runs, how long it then needs to cool down, and what it
/// does to a husk standing on it (per second).
pub const TRAP_PRICE: u32 = 1000;
pub const TRAP_SECONDS: f32 = 25.0;
pub const TRAP_COOLDOWN: f32 = 45.0;
pub const TRAP_DPS: f32 = 2500.0;
/// Relay beacons: points for switching one on, and for everyone once all of them are.
pub const RELAY_POINTS: u32 = 250;
pub const RELAY_BONUS: u32 = 1500;
/// Seconds a downed robot lasts without help.
pub const BLEED: f32 = 30.0;
const REVIVE: f32 = 4.0;
const REST: f32 = 10.0;
/// How light the horde night is: dark enough for torches, light enough to see the husks coming.
const NIGHT_LIGHT: f32 = 0.3;
const FIRST_REST: f32 = 6.0;
const POWERUP_SECONDS: f32 = 30.0;
const REPAIR_EVERY: f32 = 0.9;
const REPAIR_CAP: u32 = 500;
/// Crate phases.
pub mod crate_phase {
    pub const IDLE: u8 = 0;
    pub const ROLLING: u8 = 1;
    pub const OFFER: u8 = 2;
    pub const LEAVING: u8 = 3;
}
/// Reforge phases.
pub mod forge_phase {
    pub const IDLE: u8 = 0;
    pub const WORKING: u8 = 1;
    pub const OFFER: u8 = 2;
}
/// Crate roll result meaning "it moves".
pub const MOVES: u8 = 255;
pub const NOBODY: u8 = 255;

/// One player's run: points, stats, perks and whether they are down.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Survivor {
    pub id: u8,
    pub points: u32,
    pub kills: u16,
    pub headshots: u16,
    pub downs: u16,
    pub revives: u16,
    /// Bitmask of [`perk`] ids.
    pub perks: u8,
    pub down: bool,
    /// Seconds of bleeding left while down.
    pub bleed: f32,
    /// Revive progress, 0..1, and who is doing it.
    pub revive: f32,
    pub reviver: u8,
    /// Bled out: back next round.
    pub out: bool,
    /// Patch-up alone: seconds until the robot gets itself up.
    pub self_revive: f32,
    pub self_revives: u8,
    /// Weapon in hand when it went down, and whether the pistol it was given is borrowed.
    pub held: u8,
    pub borrowed: bool,
    /// The gun the spare socket holds (lost with the perk).
    pub third: Option<u8>,
    /// Points from boarding windows this round (capped).
    pub repaired: u32,
}

impl Survivor {
    pub fn new(id: u8) -> Self {
        Self { id, points: START_POINTS, reviver: NOBODY, held: gun::PISTOL as u8, ..default() }
    }

    pub fn has(&self, perk: u8) -> bool {
        self.perks & (1 << perk) != 0
    }
}

/// A power-up lying where a husk fell.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Drop {
    pub kind: u8,
    pub at: Vec3,
    pub ttl: f32,
}

/// The replicated horde state (part of [`MatchState`]).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HordeState {
    pub round: u16,
    /// Seconds until the next round (between rounds); 0 while one is on.
    pub rest: f32,
    /// Husks still to come this round plus those standing.
    pub left: u16,
    pub survivors: Vec<Survivor>,
    /// Bitmask of opened doors.
    pub doors: u64,
    /// Boards left on each window.
    pub planks: Vec<u8>,
    pub power: bool,
    pub crate_at: u8,
    pub crate_phase: u8,
    pub crate_timer: f32,
    pub crate_weapon: u8,
    pub crate_for: u8,
    pub forge_phase: u8,
    pub forge_timer: f32,
    /// The upgraded weapon being made (catalog index).
    pub forge_weapon: u8,
    pub forge_for: u8,
    pub drops: Vec<Drop>,
    /// Seconds of double points and of one-hit kills left.
    pub double: f32,
    pub insta: f32,
    /// Seconds of the crate on clearance left.
    pub sale: f32,
    pub kills: u32,
    /// How far the generator has been cranked (0..1) while the power is still off.
    pub charge: f32,
    /// Per arc grate: seconds of running left (> 0), of cooling down (< 0), or ready (0).
    pub traps: Vec<f32>,
    /// Bitmask of relay beacons switched on.
    pub relays: u8,
}

impl HordeState {
    pub fn survivor(&self, id: u8) -> Option<&Survivor> {
        self.survivors.iter().find(|s| s.id == id)
    }

    pub fn survivor_mut(&mut self, id: u8) -> Option<&mut Survivor> {
        self.survivors.iter_mut().find(|s| s.id == id)
    }

    pub fn is_down(&self, id: u8) -> bool {
        self.survivor(id).is_some_and(|s| s.down)
    }

    pub fn door_open(&self, index: usize) -> bool {
        self.doors & (1 << index) != 0
    }

    pub fn relay_on(&self, index: usize) -> bool {
        self.relays & (1 << index) != 0
    }

    /// An arc grate's state: (running, seconds left of running or cooling).
    pub fn trap(&self, index: usize) -> (bool, f32) {
        let t = self.traps.get(index).copied().unwrap_or(0.0);
        (t > 0.0, t.abs())
    }

    /// What the salvage crate costs right now.
    pub fn crate_price(&self) -> u32 {
        if self.sale > 0.0 { SALE_PRICE } else { CRATE_PRICE }
    }

    /// Points, doubled while the power-up lasts.
    pub fn earn(&mut self, id: u8, base: u32) {
        let amount = if self.double > 0.0 { base * 2 } else { base };
        if let Some(s) = self.survivor_mut(id) {
            s.points += amount;
        }
    }

    fn pay(&mut self, id: u8, price: u32) -> bool {
        match self.survivor_mut(id) {
            Some(s) if s.points >= price => {
                s.points -= price;
                true
            }
            _ => false,
        }
    }
}

/// Run condition: a horde match is on.
pub fn horde(rules: Option<Res<Rules>>) -> bool {
    rules.is_some_and(|r| r.horde)
}

/// The upgraded twin of a base weapon (catalogs for this mode hold the base guns, then their
/// reforged versions in the same order).
pub fn upgraded(catalog: &Catalog, weapon: usize) -> Option<usize> {
    let half = catalog.weapons.len() / 2;
    (weapon < half).then_some(weapon + half)
}

pub fn is_upgraded(catalog: &Catalog, weapon: usize) -> bool {
    weapon >= catalog.weapons.len() / 2
}

/// The base gun of a weapon (itself if it is not upgraded).
pub fn base_of(catalog: &Catalog, weapon: usize) -> usize {
    let half = catalog.weapons.len() / 2;
    if weapon >= half { weapon - half } else { weapon }
}

/// How many husks a round sends, for `players` in the squad.
pub fn round_size(round: u16, players: usize) -> u32 {
    let base = match round {
        0 => 0.0,
        1 => 6.0,
        2 => 8.0,
        3 => 13.0,
        4 => 18.0,
        5 => 24.0,
        r => 24.0 + 5.0 * (r - 5) as f32,
    };
    (base * (1.0 + 0.5 * players.saturating_sub(1) as f32)).min(240.0) as u32
}

/// A husk's health in a round.
pub fn husk_health(round: u16) -> f32 {
    if round < 10 { 150.0 + 100.0 * (round.max(1) - 1) as f32 } else { 950.0 * 1.1f32.powi(round as i32 - 9) }
}

/// Most husks up at once.
fn max_alive(players: usize) -> usize {
    (16 + 4 * players).min(32)
}

/// What a husk is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase2 {
    /// Walking up to a window from outside.
    Approach { window: u8 },
    /// Tearing boards off a window.
    Tear { window: u8 },
    /// Climbing in through a window.
    Climb { window: u8, t: f32 },
    /// Inside: after the nearest robot.
    Hunt,
}

/// A husk's brain (authority only; clients see an `Enemy`).
#[derive(Component, Clone, Debug)]
pub struct Husk {
    pub phase: Phase2,
    pub speed: f32,
    path: Vec<Vec3>,
    repath: f32,
    target: Option<Entity>,
    retarget: f32,
    windup: f32,
    cooldown: f32,
    /// Stuck check: where it was, and when it was last checked.
    last: Vec3,
    check: f32,
    stuck: u8,
    tear: f32,
    /// Damage soaked by the legs: enough and it goes down to its hands.
    legs: f32,
    pub crawl: bool,
    /// Where along the window it stands to tear at the boards (metres off the middle).
    lane: f32,
    /// Stuck on a corner: seconds of stepping sideways left, and which way.
    dodge: f32,
    dodge_side: f32,
}

impl Husk {
    fn new(phase: Phase2, speed: f32) -> Self {
        Self { phase, speed, path: Vec::new(), repath: 0.0, target: None, retarget: 0.0, windup: 0.0, cooldown: 0.5, last: Vec3::ZERO, check: 4.0, stuck: 0, tear: 0.6, legs: 0.0, crawl: false, lane: 0.0, dodge: 0.0, dodge_side: 1.0 }
    }
}

/// A bot squadmate's brain (alongside the engine's `Bot`, whose inputs it overrides).
#[derive(Component, Default)]
pub struct HordeBot {
    path: Vec<Vec3>,
    repath: f32,
    goal: Vec3,
    aim: Vec2,
    pulse: bool,
}

/// Where a closed door stands (on every machine), and the boards on a window.
#[derive(Component)]
pub struct DoorBlock(pub usize);

#[derive(Component)]
pub struct WindowBlock(pub usize);

/// The husks' navigation grid, rebuilt as doors open (authority).
#[derive(Resource)]
pub struct HordeNav {
    pub nav: Nav,
    pub doors: u64,
    scratch: NavScratch,
}

impl HordeNav {
    pub fn build(map: &GameMap, layout: &HordeLayout, doors: u64) -> Self {
        let mut statics = StaticWorld::new(map.recipe.half_extent);
        for aabb in map.solids() {
            statics.add(aabb);
        }
        for (i, door) in layout.doors.iter().enumerate() {
            if doors & (1 << i) == 0 {
                statics.add(door.aabb);
            }
        }
        Self { nav: Nav::build(map, &statics), doors, scratch: NavScratch::default() }
    }

    pub fn path(&mut self, from: Vec3, to: Vec3) -> Option<Vec<Vec3>> {
        self.nav.path(&mut self.scratch, from, to)
    }
}

/// Authority bookkeeping that never leaves the host.
#[derive(Resource, Default)]
pub struct Horde {
    /// Husks (and brutes) still to send this round.
    queue: u32,
    brutes: u32,
    spawn_timer: f32,
    /// Seconds each player's Use counts as held.
    hold: HashMap<u8, f32>,
    repair_timer: HashMap<u8, f32>,
    crate_rolls: u32,
    /// What the crate's current roll was paid (refunded if it moves on).
    crate_paid: u32,
    drops: u8,
    /// Kills since the last power-up (one is sure to come after enough of them).
    since_drop: u32,
    last_drop: u8,
    /// Volt husks still to send this round.
    volts: u32,
    /// Bursts to set off (where, and whose kill set it off).
    bursts: Vec<(Vec3, Option<u8>)>,
    bots: bool,
    /// The round whose start has been handled (respawns, grenades).
    started: u16,
}

impl Horde {
    /// A bot holding Use (bots have no keyboard).
    pub fn hold(&mut self, id: u8) {
        self.hold.insert(id, 0.3);
    }

    fn holding(&self, id: u8) -> bool {
        self.hold.get(&id).is_some_and(|t| *t > 0.0)
    }
}

pub struct HordePlugin;

impl Plugin for HordePlugin {
    fn build(&self, app: &mut App) {
        use crate::{bots, combat, enemy, lagcomp, player, weapons};
        let authority = crate::authority;
        app.init_resource::<Horde>()
            .add_systems(OnEnter(MatchPhase::Active), (setup_horde, spawn_blocks).chain().run_if(horde))
            .add_systems(Update, (sync_blocks, downed_controls.after(SimSet::Receive).before(player::move_players)).run_if(in_state(MatchPhase::Active)).run_if(horde))
            .add_systems(Update, horde_commands.in_set(SimSet::Orders).after(crate::orders::process_commands).run_if(horde))
            .add_systems(Update, hit_spheres.in_set(SimSet::Logic).after(combat::rebuild_grid).before(lagcomp::record_history).run_if(horde))
            .add_systems(Update, horde_bots.in_set(SimSet::Logic).after(bots::drive_bots).before(weapons::fire_weapons).run_if(horde))
            .add_systems(Update, (rounds, survivors, husk_brains).chain().in_set(SimSet::Logic).after(enemy::act).run_if(horde))
            .add_systems(Update, perk_effects.in_set(SimSet::Logic).after(weapons::fire_weapons).run_if(horde))
            .add_systems(Update, score_hits.in_set(SimSet::Resolve).before(combat::resolve_hits).run_if(horde).run_if(authority));
    }
}

/// Authority: a fresh run.
fn setup_horde(mut commands: Commands, role: Res<crate::state::NetRole>, map: Res<Map>, mut state: ResMut<MatchState>, mut horde: ResMut<Horde>) {
    let Some(layout) = map.horde.as_deref() else { return };
    *horde = Horde { last_drop: NOBODY, ..default() };
    if *role == crate::state::NetRole::Client {
        return;
    }
    state.horde = HordeState { rest: FIRST_REST, planks: vec![PLANKS; layout.windows.len()], traps: vec![0.0; layout.traps.len()], crate_for: NOBODY, forge_for: NOBODY, ..default() };
    state.phase = Phase::Night;
    state.daylight = NIGHT_LIGHT;
    commands.insert_resource(HordeNav::build(&map, layout, 0));
}

/// Every machine: what blocks closed doors and windows (collision only; the client dresses them).
fn spawn_blocks(mut commands: Commands, map: Res<Map>) {
    let Some(layout) = map.horde.as_deref() else { return };
    for (i, door) in layout.doors.iter().enumerate() {
        let base = door.aabb.center - Vec3::Y * door.aabb.half.y;
        commands.spawn((MatchEntity, DoorBlock(i), crate::combat::Solid { half: door.aabb.half }, Transform::from_translation(base)));
    }
    for (i, window) in layout.windows.iter().enumerate() {
        let b = window.blocker();
        let base = b.center - Vec3::Y * b.half.y;
        commands.spawn((MatchEntity, WindowBlock(i), crate::combat::Solid { half: b.half }, Transform::from_translation(base)));
    }
}

/// Every machine: opened doors stop blocking.
fn sync_blocks(mut commands: Commands, state: Res<MatchState>, doors: Query<(Entity, &DoorBlock)>) {
    for (entity, door) in &doors {
        if state.horde.door_open(door.0) {
            commands.entity(entity).despawn();
        }
    }
}

/// Every machine, for the robots it moves: a downed robot crawls with its pistol; a new gun
/// comes straight to hand; after a revive, the gun it had comes back.
fn downed_controls(state: Res<MatchState>, content: Res<Content>, mut players: Query<(&Player, &mut PlayerInput, &Arsenal), Or<(With<LocalPlayer>, With<Bot>)>>, mut seen: Local<HashMap<u8, (bool, u8, u64)>>) {
    for (player, mut input, arsenal) in &mut players {
        let down = state.horde.is_down(player.id);
        let owned = arsenal.weapons.iter().enumerate().filter(|(_, w)| w.owned).fold(0u64, |m, (i, _)| m | (1 << i.min(63)));
        let (was_down, before, was_owned) = seen.get(&player.id).copied().unwrap_or((false, arsenal.current, owned));
        if down {
            input.stance = stance::PRONE;
            input.buttons &= !(buttons::SPRINT | buttons::JUMP | buttons::GEAR);
            input.weapon = gun::PISTOL as u8;
            seen.insert(player.id, (true, if was_down { before } else { arsenal.current }, owned));
            continue;
        }
        if was_down {
            // Up again: the gun it had, or whatever the host handed back.
            input.weapon = if arsenal.owns(before as usize) { before } else { arsenal.current };
            input.stance = stance::STAND;
        } else {
            let fresh = owned & !was_owned;
            if fresh != 0 {
                input.weapon = (63 - fresh.leading_zeros()) as u8;
            }
        }
        if !arsenal.owns(input.weapon as usize) && arsenal.owns(arsenal.current as usize) {
            input.weapon = arsenal.current;
        }
        let _ = &content;
        seen.insert(player.id, (false, input.weapon, owned));
    }
}

/// Husks are tall: the engine's grid holds one sphere at the feet, so add the chest and head.
fn hit_spheres(mut grid: ResMut<Grid>, husks: Query<(Entity, &Transform, &Enemy)>) {
    for (entity, transform, enemy) in &husks {
        for (center, radius) in body_spheres(enemy.kind, transform).into_iter().skip(1) {
            grid.insert(GridItem { entity, pos: center, radius });
        }
    }
}

use crate::enemy::Enemy;

/// A husk's hit spheres: legs, chest, head (centre, radius).
pub fn body_spheres(kind: u16, transform: &Transform) -> [(Vec3, f32); 3] {
    let p = transform.translation;
    let forward = transform.rotation * Vec3::NEG_Z;
    match kind {
        kind::BRUTE => [(p + Vec3::Y * 0.6, 0.6), (p + Vec3::Y * 1.55 + forward * 0.1, 0.62), (p + Vec3::Y * 2.25 + forward * 0.25, 0.3)],
        kind::CRAWLER => [(p + Vec3::Y * 0.35, 0.4), (p + Vec3::Y * 0.4 + forward * 0.45, 0.32), (p + Vec3::Y * 0.5 + forward * 0.9, 0.24)],
        _ => [(p + Vec3::Y * 0.42, 0.42), (p + Vec3::Y * 1.12 + forward * 0.08, 0.36), (p + Vec3::Y * 1.58 + forward * 0.18, 0.23)],
    }
}

/// Where a ray goes through a husk: its head, its legs, or neither.
fn struck_part(kind: u16, transform: &Transform, eye: Vec3, dir: Vec3) -> (bool, bool) {
    let [legs, _, head] = body_spheres(kind, transform);
    let hits = |(c, r): (Vec3, f32), grow: f32| crate::combat::ray_sphere(eye, dir, c, r + grow).is_some();
    (hits(head, 0.06), hits(legs, 0.0) && !hits(body_spheres(kind, transform)[1], 0.0))
}

/// Something to use, near a player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Interact {
    WallBuy(usize),
    Door(usize),
    Perk(usize),
    Crate,
    CrateTake,
    Forge,
    ForgeTake,
    /// Hold: crank the generator.
    Power,
    /// Switch an arc grate on.
    Trap(usize),
    /// Switch a relay beacon on.
    Relay(usize),
    /// Hold: board a window back up.
    Repair(usize),
    /// Hold: get a downed teammate up.
    Revive(u8),
}

impl Interact {
    pub fn held(self) -> bool {
        matches!(self, Interact::Repair(_) | Interact::Revive(_) | Interact::Power)
    }
}

/// The thing a player at `feet` looking along `look` would use, if any. Shared by the host
/// (to act on Use) and the client (to show the prompt). `downed`: teammates that are down.
pub fn interaction(layout: &HordeLayout, state: &HordeState, me: u8, feet: Vec3, look: Vec3, downed: &[(u8, Vec3)]) -> Option<Interact> {
    if state.is_down(me) {
        return None;
    }
    let flat = Vec2::new(look.x, look.z).normalize_or_zero();
    // Candidates by tier (a downed teammate first, boards last), then by how well placed.
    let mut found: Vec<(u8, f32, Interact)> = Vec::new();
    let mut consider = |tier: u8, anchor: Vec3, reach: f32, what: Interact| {
        let to = anchor.xz() - feet.xz();
        let distance = to.length();
        if distance > reach || (anchor.y - feet.y).abs() > 1.6 {
            return;
        }
        // Roughly facing it (very close always counts).
        let facing = if distance < 0.9 { 1.0 } else { flat.dot(to / distance) };
        if facing < 0.1 {
            return;
        }
        found.push((tier, distance - facing, what));
    };
    for (id, at) in downed {
        if *id != me {
            consider(0, *at, 2.0, Interact::Revive(*id));
        }
    }
    let open = layout.open_zones(state.doors);
    let zone_open = |zone: u8| open & (1 << zone) != 0;
    for (i, buy) in layout.buys.iter().enumerate() {
        let n = Vec3::new(buy.normal.x, 0.0, buy.normal.y);
        let anchor = Vec3::new(buy.pos.x, feet.y, buy.pos.z) + n * 0.6;
        if zone_open(buy.zone) {
            consider(1, anchor, 1.7, Interact::WallBuy(i));
        }
    }
    for (i, door) in layout.doors.iter().enumerate() {
        if !state.door_open(i) {
            let c = door.aabb.center;
            consider(1, Vec3::new(c.x, feet.y, c.z), 2.3, Interact::Door(i));
        }
    }
    for (i, machine) in layout.perks.iter().enumerate() {
        consider(1, machine.pos, 1.6, Interact::Perk(i));
    }
    if let Some(spot) = layout.crates.get(state.crate_at as usize)
        && state.crate_phase != crate_phase::LEAVING
        && state.crate_phase != crate_phase::ROLLING
    {
        let take = state.crate_phase == crate_phase::OFFER;
        if !take || state.crate_for == me {
            consider(1, spot.pos, 1.7, if take { Interact::CrateTake } else { Interact::Crate });
        }
    }
    if let Some(forge) = layout.forge.as_ref() {
        match state.forge_phase {
            forge_phase::IDLE => consider(1, forge.pos, 1.7, Interact::Forge),
            forge_phase::OFFER if state.forge_for == me => consider(1, forge.pos, 1.7, Interact::ForgeTake),
            _ => {}
        }
    }
    if let Some(power) = layout.power.as_ref()
        && !state.power
    {
        consider(1, power.pos, 1.6, Interact::Power);
    }
    for (i, trap) in layout.traps.iter().enumerate() {
        if zone_open(trap.zone) {
            consider(1, trap.panel.pos, 1.6, Interact::Trap(i));
        }
    }
    for (i, relay) in layout.relays.iter().enumerate() {
        if zone_open(relay.zone) && !state.relay_on(i) {
            consider(1, relay.pos, 1.6, Interact::Relay(i));
        }
    }
    for (i, window) in layout.windows.iter().enumerate() {
        if state.planks.get(i).is_some_and(|p| *p < PLANKS) && zone_open(window.zone) {
            consider(2, window.inside, 1.9, Interact::Repair(i));
        }
    }
    found.into_iter().min_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1))).map(|(_, _, what)| what)
}

/// What the prompt says for an interaction, and its price (if it costs anything).
pub fn describe(layout: &HordeLayout, state: &HordeState, catalog: &Catalog, me: u8, arsenal: &Arsenal, what: Interact, names: impl Fn(u8) -> String) -> (String, Option<u32>) {
    let survivor = state.survivor(me);
    let has = |p: u8| survivor.is_some_and(|s| s.has(p));
    let weapon_name = |i: usize| catalog.weapons.get(i).map_or("?", |w| w.name).to_string();
    match what {
        Interact::WallBuy(i) => {
            let buy = &layout.buys[i];
            let w = buy.weapon as usize;
            let up = upgraded(catalog, w);
            if up.is_some_and(|u| arsenal.owns(u)) {
                (format!("{} AMMO", weapon_name(up.unwrap_or(w))), Some(UPGRADED_AMMO))
            } else if arsenal.owns(w) {
                (format!("{} AMMO", weapon_name(w)), Some(buy.price / 2))
            } else {
                (format!("BUY {}", weapon_name(w)), Some(buy.price))
            }
        }
        Interact::Door(i) => {
            let door = &layout.doors[i];
            (if door.debris { "CLEAR RUBBLE".to_string() } else { "OPEN DOOR".to_string() }, Some(door.price))
        }
        Interact::Perk(i) => {
            let p = layout.perks[i].kind;
            let name = perk::NAMES[p as usize];
            if has(p) {
                (format!("{name} - YOURS"), None)
            } else if !state.power && p != perk::PATCH_UP {
                (format!("{name} - NEEDS POWER"), None)
            } else if survivor.is_some_and(|s| s.perks.count_ones() >= perk::MAX) {
                (format!("{name} - {} PERKS MAX", perk::MAX), None)
            } else {
                (format!("{name} - {}", perk::BLURBS[p as usize]), Some(perk::PRICES[p as usize]))
            }
        }
        Interact::Crate => (if state.sale > 0.0 { "SALVAGE CRATE - CLEARANCE".to_string() } else { "SALVAGE CRATE".to_string() }, Some(state.crate_price())),
        Interact::CrateTake => (format!("TAKE {}", weapon_name(state.crate_weapon as usize)), None),
        Interact::Forge => {
            let current = arsenal.current as usize;
            if !state.power {
                ("REFORGE - NEEDS POWER".to_string(), None)
            } else if is_upgraded(catalog, current) || !arsenal.owns(current) {
                ("REFORGE - BRING A STOCK GUN".to_string(), None)
            } else {
                (format!("REFORGE {}", weapon_name(current)), Some(FORGE_PRICE))
            }
        }
        Interact::ForgeTake => (format!("TAKE {}", weapon_name(state.forge_weapon as usize)), None),
        Interact::Power => {
            if state.charge > 0.0 {
                (format!("CRANK THE GENERATOR - {:.0}%", state.charge * 100.0), None)
            } else {
                ("CRANK THE GENERATOR - RESTORES POWER".to_string(), None)
            }
        }
        Interact::Trap(i) => {
            let (running, left) = state.trap(i);
            if !state.power {
                ("ARC GRATE - NEEDS POWER".to_string(), None)
            } else if running {
                (format!("ARC GRATE - LIVE {:.0}", left.ceil()), None)
            } else if left > 0.0 {
                (format!("ARC GRATE - COOLING {:.0}", left.ceil()), None)
            } else {
                ("ARC GRATE - FRIES HUSKS ON IT".to_string(), Some(TRAP_PRICE))
            }
        }
        Interact::Relay(_) => {
            let on = state.relays.count_ones();
            (format!("SWITCH ON RELAY - {on}/{} ONLINE", layout.relays.len()), None)
        }
        Interact::Repair(_) => ("REBUILD BARRICADE".to_string(), None),
        Interact::Revive(id) => (format!("REVIVE {}", names(id)), None),
    }
}

/// Puts a gun in a robot's hands, dropping the one held if its hands are full.
pub fn give_weapon(catalog: &Catalog, survivor: &mut Survivor, arsenal: &mut Arsenal, weapon: usize) {
    if weapon >= arsenal.weapons.len() {
        return;
    }
    let limit = if survivor.has(perk::SPARE_SOCKET) { 3 } else { 2 };
    let owned: Vec<usize> = (0..arsenal.weapons.len()).filter(|i| arsenal.owns(*i)).collect();
    let borrowed = survivor.borrowed && arsenal.current as usize == gun::PISTOL;
    if owned.len() >= limit && !borrowed {
        let drop = if arsenal.owns(arsenal.current as usize) { arsenal.current as usize } else { owned[owned.len() - 1] };
        arsenal.weapons[drop] = WeaponState::fresh(&catalog.weapons[drop]);
        if survivor.third == Some(drop as u8) {
            survivor.third = None;
        }
    } else if owned.len() + 1 == 3 {
        survivor.third = Some(weapon as u8);
    }
    arsenal.weapons[weapon] = WeaponState::fresh(&catalog.weapons[weapon]);
    grant_weapon(arsenal, &catalog.weapons, weapon);
    arsenal.current = weapon as u8;
    arsenal.reload = 0.0;
    arsenal.burst = 0;
}

/// Authority: acts on Use and keeps track of Use being held.
#[allow(clippy::too_many_arguments)]
fn horde_commands(
    mut orders: EventReader<CommandEvent>,
    map: Res<Map>,
    content: Res<Content>,
    mut state: ResMut<MatchState>,
    mut horde: ResMut<Horde>,
    mut rng: ResMut<SimRng>,
    mut players: Query<(&Player, &Transform, &PlayerInput, &mut Arsenal, &Health)>,
    mut fx: EventWriter<FxEvent>,
) {
    let Some(layout) = map.horde.as_deref() else { return };
    let downed: Vec<(u8, Vec3)> = players.iter().filter(|(p, ..)| state.horde.is_down(p.id)).map(|(p, t, ..)| (p.id, t.translation)).collect();
    for order in orders.read() {
        let id = order.player;
        match order.command {
            PlayerCommand::Mode(act::HOLD) => {
                horde.hold(id);
                continue;
            }
            PlayerCommand::Mode(act::USE) => {}
            _ => continue,
        }
        if state.over() {
            continue;
        }
        horde.hold(id);
        let Some((_, transform, input, mut arsenal, health)) = players.iter_mut().find(|(p, ..)| p.id == id) else { continue };
        if !health.alive() {
            continue;
        }
        let feet = transform.translation;
        let Some(what) = interaction(layout, &state.horde, id, feet, input.aim_dir(), &downed) else { continue };
        let deny = |fx: &mut EventWriter<FxEvent>, why: &str| {
            fx.write(FxEvent::tell(id, why));
            fx.write(FxEvent::Cue { to: id, sfx: Sfx::UiDeny });
        };
        let bought = |fx: &mut EventWriter<FxEvent>, at: Vec3| {
            fx.write(FxEvent::Cue { to: id, sfx: Sfx::Buy });
            fx.write(FxEvent::Sound { sfx: Sfx::Buy, at: Some(at) });
        };
        let h = &mut state.horde;
        match what {
            Interact::WallBuy(i) => {
                let buy = layout.buys[i];
                let w = buy.weapon as usize;
                let up = upgraded(&content, w).filter(|u| arsenal.owns(*u));
                if let Some(owned) = up.or(arsenal.owns(w).then_some(w)) {
                    let def = &content.weapons[owned];
                    let state_w = &arsenal.weapons[owned];
                    if state_w.reserve >= def.reserve {
                        deny(&mut fx, "AMMO FULL");
                        continue;
                    }
                    let price = if up.is_some() { UPGRADED_AMMO } else { buy.price / 2 };
                    if !h.pay(id, price) {
                        deny(&mut fx, "NOT ENOUGH POINTS");
                        continue;
                    }
                    arsenal.weapons[owned].reserve = def.reserve;
                    bought(&mut fx, buy.pos);
                } else {
                    if !h.pay(id, buy.price) {
                        deny(&mut fx, "NOT ENOUGH POINTS");
                        continue;
                    }
                    let Some(survivor) = h.survivor_mut(id) else { continue };
                    give_weapon(&content, survivor, &mut arsenal, w);
                    bought(&mut fx, buy.pos);
                }
            }
            Interact::Door(i) => {
                let door = layout.doors[i];
                if !h.pay(id, door.price) {
                    deny(&mut fx, "NOT ENOUGH POINTS");
                    continue;
                }
                h.doors |= 1 << i;
                fx.write(FxEvent::Sound { sfx: if door.debris { Sfx::Debris } else { Sfx::Build }, at: Some(door.aabb.center) });
                fx.write(FxEvent::Cue { to: id, sfx: Sfx::Buy });
            }
            Interact::Perk(i) => {
                let p = layout.perks[i].kind;
                let Some(s) = h.survivor(id) else { continue };
                if s.has(p) {
                    continue;
                }
                if !h.power && p != perk::PATCH_UP {
                    deny(&mut fx, "THE POWER IS OFF");
                    continue;
                }
                if s.perks.count_ones() >= perk::MAX {
                    deny(&mut fx, "NO ROOM FOR ANOTHER PERK");
                    continue;
                }
                if !h.pay(id, perk::PRICES[p as usize]) {
                    deny(&mut fx, "NOT ENOUGH POINTS");
                    continue;
                }
                if let Some(s) = h.survivor_mut(id) {
                    s.perks |= 1 << p;
                }
                fx.write(FxEvent::tell(id, perk::NAMES[p as usize]));
                fx.write(FxEvent::Cue { to: id, sfx: Sfx::Streak });
                fx.write(FxEvent::Sound { sfx: Sfx::Buy, at: Some(layout.perks[i].pos) });
            }
            Interact::Crate => {
                if h.crate_phase != crate_phase::IDLE {
                    continue;
                }
                let price = h.crate_price();
                if !h.pay(id, price) {
                    deny(&mut fx, "NOT ENOUGH POINTS");
                    continue;
                }
                horde.crate_rolls += 1;
                horde.crate_paid = price;
                // It never wanders off while on clearance.
                let moves = layout.crates.len() > 1 && horde.crate_rolls > 3 && h.sale <= 0.0 && rng.0.chance(0.18);
                let h = &mut state.horde;
                h.crate_phase = crate_phase::ROLLING;
                h.crate_timer = 4.2;
                h.crate_for = id;
                h.crate_weapon = if moves { MOVES } else { roll_crate(&content, &arsenal, &mut rng) };
                if moves {
                    horde.crate_rolls = 0;
                }
                let at = layout.crates[h.crate_at as usize].pos;
                fx.write(FxEvent::Sound { sfx: Sfx::Medal, at: Some(at) });
            }
            Interact::CrateTake => {
                let weapon = h.crate_weapon as usize;
                if h.crate_phase != crate_phase::OFFER || h.crate_for != id || weapon >= content.weapons.len() {
                    continue;
                }
                h.crate_phase = crate_phase::IDLE;
                h.crate_for = NOBODY;
                let Some(survivor) = h.survivor_mut(id) else { continue };
                give_weapon(&content, survivor, &mut arsenal, weapon);
                fx.write(FxEvent::Cue { to: id, sfx: Sfx::Swap });
            }
            Interact::Forge => {
                let current = arsenal.current as usize;
                if !h.power {
                    deny(&mut fx, "THE POWER IS OFF");
                    continue;
                }
                let Some(up) = upgraded(&content, current).filter(|_| arsenal.owns(current)) else {
                    deny(&mut fx, "THAT GUN CANNOT BE REFORGED");
                    continue;
                };
                if !h.pay(id, FORGE_PRICE) {
                    deny(&mut fx, "NOT ENOUGH POINTS");
                    continue;
                }
                // The gun goes into the press.
                arsenal.weapons[current] = WeaponState::fresh(&content.weapons[current]);
                if let Some(next) = (0..arsenal.weapons.len()).find(|i| arsenal.owns(*i)) {
                    arsenal.current = next as u8;
                }
                h.forge_phase = forge_phase::WORKING;
                h.forge_timer = 4.5;
                h.forge_weapon = up as u8;
                h.forge_for = id;
                if let Some(forge) = layout.forge {
                    fx.write(FxEvent::Sound { sfx: Sfx::Build, at: Some(forge.pos) });
                }
            }
            Interact::ForgeTake => {
                if h.forge_phase != forge_phase::OFFER || h.forge_for != id {
                    continue;
                }
                let weapon = h.forge_weapon as usize;
                h.forge_phase = forge_phase::IDLE;
                h.forge_for = NOBODY;
                let Some(survivor) = h.survivor_mut(id) else { continue };
                give_weapon(&content, survivor, &mut arsenal, weapon);
                fx.write(FxEvent::Cue { to: id, sfx: Sfx::Streak });
            }
            Interact::Trap(i) => {
                let Some(trap) = layout.traps.get(i) else { continue };
                if !h.power {
                    deny(&mut fx, "THE POWER IS OFF");
                    continue;
                }
                if h.traps.get(i).is_none_or(|t| *t != 0.0) {
                    deny(&mut fx, "NOT READY");
                    continue;
                }
                if !h.pay(id, TRAP_PRICE) {
                    deny(&mut fx, "NOT ENOUGH POINTS");
                    continue;
                }
                h.traps[i] = TRAP_SECONDS;
                fx.write(FxEvent::tell(id, "ARC GRATE LIVE"));
                fx.write(FxEvent::Sound { sfx: Sfx::Radar, at: Some(trap.panel.pos) });
                fx.write(FxEvent::Cue { to: id, sfx: Sfx::Buy });
            }
            Interact::Relay(i) => {
                let Some(relay) = layout.relays.get(i).copied() else { continue };
                if h.relay_on(i) {
                    continue;
                }
                h.relays |= 1 << i;
                h.earn(id, RELAY_POINTS);
                let on = h.relays.count_ones() as usize;
                let all = layout.relays.len();
                fx.write(FxEvent::Sound { sfx: Sfx::Medal, at: Some(relay.pos) });
                if on < all {
                    fx.write(FxEvent::notice(format!("RELAY {on} OF {all} ONLINE")));
                } else {
                    // The whole network: points for everyone, and a power-up at the last beacon.
                    let ids: Vec<u8> = h.survivors.iter().map(|s| s.id).collect();
                    for who in ids {
                        h.earn(who, RELAY_BONUS);
                    }
                    let kind = pick_drop(horde.last_drop, &mut rng);
                    horde.last_drop = kind;
                    let h = &mut state.horde;
                    h.drops.push(Drop { kind, at: relay.pos + Vec3::new(relay.facing.x, 0.0, relay.facing.y) * 2.0 + Vec3::Y * 0.9, ttl: 40.0 });
                    fx.write(FxEvent::notice(format!("RELAY NETWORK ONLINE - {RELAY_BONUS} POINTS EACH")));
                    fx.write(FxEvent::Sound { sfx: Sfx::Streak, at: None });
                }
            }
            // Held: the generator, boards and revives are worked in `survivors`.
            Interact::Power | Interact::Repair(_) | Interact::Revive(_) => {}
        }
    }
}

/// A random gun from the crate: any stock gun but the pistol, and none the robot carries.
fn roll_crate(catalog: &Catalog, arsenal: &Arsenal, rng: &mut SimRng) -> u8 {
    let half = catalog.weapons.len() / 2;
    let pool: Vec<usize> = (0..half).filter(|i| *i != gun::PISTOL && !arsenal.owns(*i) && upgraded(catalog, *i).is_none_or(|u| !arsenal.owns(u))).collect();
    rng.0.pick(&pool).copied().unwrap_or(gun::SMG) as u8
}

/// Authority: the round clock, spawning, power-ups, the crate and the reforge.
#[allow(clippy::too_many_arguments)]
fn rounds(
    time: Res<Time>,
    map: Res<Map>,
    content: Res<Content>,
    rules: Res<Rules>,
    mut state: ResMut<MatchState>,
    mut horde: ResMut<Horde>,
    nav: Option<ResMut<HordeNav>>,
    mut rng: ResMut<SimRng>,
    mut ids: ResMut<NetIds>,
    mut commands: Commands,
    husks: Query<(Entity, &Transform), With<Husk>>,
    mut players: Query<(&Player, &Transform, &Health, &mut Arsenal), Without<Husk>>,
    mut hits: EventWriter<Hit>,
    mut fx: EventWriter<FxEvent>,
) {
    let Some(layout) = map.horde.as_deref() else { return };
    let dt = sim_dt(&time);
    state.daylight += (NIGHT_LIGHT - state.daylight) * (dt * 0.5).min(1.0);
    for t in horde.hold.values_mut() {
        *t -= dt;
    }
    if let Some(mut nav) = nav
        && nav.doors != state.horde.doors
    {
        *nav = HordeNav::build(&map, layout, state.horde.doors);
    }
    if state.over() {
        return;
    }
    let h = &mut state.horde;
    h.double = (h.double - dt).max(0.0);
    h.insta = (h.insta - dt).max(0.0);
    h.sale = (h.sale - dt).max(0.0);

    // The crate and the reforge.
    match h.crate_phase {
        crate_phase::ROLLING => {
            h.crate_timer -= dt;
            if h.crate_timer <= 0.0 {
                if h.crate_weapon == MOVES {
                    h.crate_phase = crate_phase::LEAVING;
                    h.crate_timer = 7.0;
                    let who = h.crate_for;
                    if let Some(s) = h.survivor_mut(who) {
                        s.points += horde.crate_paid;
                    }
                    fx.write(FxEvent::notice("THE CRATE HAS MOVED ON"));
                    fx.write(FxEvent::Sound { sfx: Sfx::Growl, at: layout.crates.get(h.crate_at as usize).map(|c| c.pos) });
                } else {
                    h.crate_phase = crate_phase::OFFER;
                    h.crate_timer = 9.0;
                }
            }
        }
        crate_phase::OFFER => {
            h.crate_timer -= dt;
            if h.crate_timer <= 0.0 {
                h.crate_phase = crate_phase::IDLE;
                h.crate_for = NOBODY;
            }
        }
        crate_phase::LEAVING => {
            h.crate_timer -= dt;
            if h.crate_timer <= 0.0 && layout.crates.len() > 1 {
                let mut next = rng.0.below(layout.crates.len() - 1) as u8;
                if next >= h.crate_at {
                    next += 1;
                }
                h.crate_at = next;
                h.crate_phase = crate_phase::IDLE;
                h.crate_for = NOBODY;
            }
        }
        _ => {}
    }
    match h.forge_phase {
        forge_phase::WORKING => {
            h.forge_timer -= dt;
            if h.forge_timer <= 0.0 {
                h.forge_phase = forge_phase::OFFER;
                h.forge_timer = 10.0;
                if let Some(forge) = layout.forge {
                    fx.write(FxEvent::Sound { sfx: Sfx::Medal, at: Some(forge.pos) });
                }
            }
        }
        forge_phase::OFFER => {
            h.forge_timer -= dt;
            if h.forge_timer <= 0.0 {
                h.forge_phase = forge_phase::IDLE;
                h.forge_for = NOBODY;
            }
        }
        _ => {}
    }

    // Arc grates: fry whatever stands on a live one, then cool down.
    for (i, trap) in layout.traps.iter().enumerate() {
        let Some(t) = h.traps.get_mut(i) else { continue };
        if *t > 0.0 {
            *t -= dt;
            for (entity, transform) in &husks {
                let at = transform.translation;
                if trap.covers(at.xz()) && at.y < layout.ground + 1.2 {
                    hits.write(Hit::plain(entity, TRAP_DPS * dt, None));
                }
            }
            if *t <= 0.0 {
                *t = -TRAP_COOLDOWN;
                fx.write(FxEvent::Sound { sfx: Sfx::DryFire, at: Some(trap.panel.pos) });
            }
        } else if *t < 0.0 {
            *t = (*t + dt).min(0.0);
        }
    }

    // Power-ups: blink out after a while, or get picked up.
    let pickers: Vec<Vec3> = players.iter().filter(|(p, _, health, _)| health.alive() && !h.is_down(p.id)).map(|(_, t, ..)| t.translation).collect();
    for d in &mut h.drops {
        d.ttl -= dt;
    }
    let mut taken = Vec::new();
    h.drops.retain(|d| {
        if d.ttl <= 0.0 {
            return false;
        }
        if pickers.iter().any(|p| p.distance(d.at - Vec3::Y * 0.9) < 1.6) {
            taken.push(d.kind);
            return false;
        }
        true
    });
    for kind in taken {
        fx.write(FxEvent::notice(format!("{} - {}", drop::NAMES[kind as usize], drop::BLURBS[kind as usize].to_uppercase())));
        fx.write(FxEvent::Sound { sfx: Sfx::Streak, at: None });
        match kind {
            drop::AMMO => {
                for (_, _, _, mut arsenal) in &mut players {
                    for (weapon, def) in arsenal.weapons.iter_mut().zip(&content.weapons) {
                        if weapon.owned {
                            weapon.reserve = weapon.reserve.max(def.reserve);
                        }
                    }
                }
            }
            drop::DOUBLE => state.horde.double = POWERUP_SECONDS,
            drop::INSTA => state.horde.insta = POWERUP_SECONDS,
            drop::CLEARANCE => state.horde.sale = POWERUP_SECONDS,
            drop::LOCKDOWN => {
                let open = layout.open_zones(state.horde.doors);
                for (i, w) in layout.windows.iter().enumerate() {
                    if open & (1 << w.zone) != 0
                        && let Some(p) = state.horde.planks.get_mut(i)
                    {
                        *p = PLANKS;
                    }
                }
                let ids: Vec<u8> = state.horde.survivors.iter().map(|s| s.id).collect();
                for id in ids {
                    state.horde.earn(id, 200);
                }
            }
            drop::HAUL => {
                let ids: Vec<u8> = state.horde.survivors.iter().map(|s| s.id).collect();
                for id in ids {
                    state.horde.earn(id, 500);
                }
            }
            _ => {
                for (entity, transform) in &husks {
                    hits.write(Hit::plain(entity, 1e9, None));
                    fx.write(FxEvent::Sound { sfx: Sfx::Explosion, at: Some(transform.translation) });
                }
                let ids: Vec<u8> = state.horde.survivors.iter().map(|s| s.id).collect();
                for id in ids {
                    state.horde.earn(id, 400);
                }
            }
        }
    }

    // Bots for the empty seats, once.
    let humans = players.iter().filter(|(p, ..)| p.id < FIRST_BOT_ID).count();
    if !horde.bots {
        horde.bots = true;
        for i in 0..(rules.team_size as usize).saturating_sub(humans.max(1)).min(3) {
            let id = FIRST_BOT_ID + i as u8;
            let name = format!("{} [BOT]", ["VEGA", "ONYX", "RIGGS"][i % 3]);
            commands.spawn((player_bundle(&content, &rules, &map, id, name, 0), ids.next(), Bot::default(), HordeBot::default()));
        }
    }

    // The round clock.
    let alive = husks.iter().len() as u32;
    let squad = players.iter().count().max(1);
    let h = &mut state.horde;
    if h.rest > 0.0 {
        h.rest -= dt;
        h.left = 0;
        if h.rest <= 0.0 {
            h.rest = 0.0;
            h.round += 1;
            let round = h.round;
            let size = round_size(round, squad);
            horde.brutes = if round % 5 == 0 { (1 + round as u32 / 10) * (1 + squad as u32 / 3) } else { 0 };
            horde.volts = if content.enemies.len() > kind::VOLT as usize { volt_count(round, size) } else { 0 };
            horde.queue = size.saturating_sub(horde.brutes + horde.volts);
            horde.spawn_timer = 1.5;
            horde.drops = 0;
            h.left = size as u16;
            for s in &mut h.survivors {
                s.repaired = 0;
            }
            state.day = round as u32;
            fx.write(FxEvent::notice(match round {
                r if r % 5 == 0 => format!("ROUND {r} - BRUTES ON THE PROWL"),
                VOLT_ROUND => format!("ROUND {VOLT_ROUND} - VOLT HUSKS: SCRAP THEM AT A DISTANCE"),
                r => format!("ROUND {r}"),
            }));
            fx.write(FxEvent::Sound { sfx: Sfx::Horn, at: None });
        }
        return;
    }
    if h.round == 0 {
        return;
    }
    // Send husks while there is room.
    horde.spawn_timer -= dt;
    let room = max_alive(squad) as u32;
    while horde.spawn_timer <= 0.0 && horde.queue + horde.brutes + horde.volts > 0 && alive < room {
        horde.spawn_timer += (2.0 * 0.92f32.powi(h.round as i32 - 1)).max(0.35);
        let brute = horde.brutes > 0 && (horde.queue + horde.volts == 0 || rng.0.chance(0.15));
        let volt = !brute && horde.volts > 0 && (horde.queue == 0 || rng.0.chance(horde.volts as f32 / (horde.queue + horde.volts) as f32));
        let kind = if brute {
            kind::BRUTE
        } else if volt {
            kind::VOLT
        } else {
            kind::HUSK
        };
        let targets: Vec<Vec3> = players.iter().filter(|(_, _, health, _)| health.alive()).map(|(_, t, ..)| t.translation).collect();
        let Some((phase, at)) = pick_spawn(layout, &state.horde, &targets, &mut rng) else { break };
        if brute {
            horde.brutes -= 1;
        } else if volt {
            horde.volts -= 1;
        } else {
            horde.queue -= 1;
        }
        let round = state.horde.round;
        let (hp, speed) = match kind {
            kind::BRUTE => (husk_health(round) * 5.0, gait(round, true, &mut rng)),
            // Fragile, and in a hurry.
            kind::VOLT => (husk_health(round) * 0.6, rng.0.range(4.4, 5.2)),
            _ => (husk_health(round), gait(round, false, &mut rng)),
        };
        spawn_husk(&mut commands, &mut ids, &content, kind, at, phase, hp, speed, &mut rng);
        break;
    }
    let h = &mut state.horde;
    h.left = (horde.queue + horde.brutes + horde.volts + alive) as u16;
    if horde.queue + horde.brutes + horde.volts == 0 && alive == 0 {
        h.rest = REST;
        fx.write(FxEvent::notice(format!("ROUND {} SURVIVED", h.round)));
        fx.write(FxEvent::Sound { sfx: Sfx::Dawn, at: None });
    }
}

/// How fast a husk moves this round: shamblers first, then joggers, then runners.
fn gait(round: u16, brute: bool, rng: &mut SimRng) -> f32 {
    if brute {
        return 2.6 + 0.04 * round as f32;
    }
    let r = round as f32;
    let roll = rng.0.unit();
    let sprint = ((r - 4.0) * 0.12).clamp(0.0, 0.7);
    let jog = ((r - 1.5) * 0.3).clamp(0.0, 1.0);
    let base = if roll < sprint {
        5.0
    } else if roll < sprint + jog * (1.0 - sprint) {
        2.7
    } else {
        1.3
    };
    base * rng.0.range(0.9, 1.1)
}

/// The first round volt husks come in.
pub const VOLT_ROUND: u16 = 6;

/// How many of a round's husks are volt husks: none before [`VOLT_ROUND`], then a growing
/// share (a fifth at most).
pub fn volt_count(round: u16, size: u32) -> u32 {
    if round < VOLT_ROUND {
        return 0;
    }
    let share = (0.06 + 0.02 * (round - VOLT_ROUND) as f32).min(0.2);
    ((size as f32 * share).round() as u32).max(1)
}

/// Where the next husk comes from: out in the open beyond a window of an open zone (never
/// inside; every husk has to tear its way in), favouring windows near the squad.
pub fn pick_spawn(layout: &HordeLayout, state: &HordeState, targets: &[Vec3], rng: &mut SimRng) -> Option<(Phase2, Vec3)> {
    let open = layout.open_zones(state.doors);
    let near = |p: Vec3| targets.iter().map(|t| t.distance(p)).fold(f32::MAX, f32::min);
    let mut spots: Vec<(f32, Phase2, Vec3)> = Vec::new();
    for (i, w) in layout.windows.iter().enumerate() {
        if open & (1 << w.zone) != 0 {
            let side = Vec3::new(-w.out.y, 0.0, w.out.x) * rng.0.range(-w.spread, w.spread.max(1e-3));
            spots.push((near(w.inside), Phase2::Approach { window: i as u8 }, w.spawn + side));
        }
    }
    if spots.is_empty() {
        return None;
    }
    spots.sort_by(|a, b| a.0.total_cmp(&b.0));
    let keep = spots.iter().filter(|s| s.0 < 30.0).count().max(4).min(spots.len());
    let (_, phase, at) = spots[rng.0.below(keep)];
    Some((phase, at))
}

#[allow(clippy::too_many_arguments)]
fn spawn_husk(commands: &mut Commands, ids: &mut NetIds, content: &Content, kind: u16, at: Vec3, phase: Phase2, hp: f32, speed: f32, rng: &mut SimRng) -> Entity {
    let def = &content.enemies[kind as usize];
    let entity = crate::enemy::spawn_enemy(commands, ids, content, kind, at, hp / def.hp.max(1.0), rng);
    // The engine's own enemy brain never thinks for a husk: it has its own.
    let lane = rng.0.range(-0.45, 0.45);
    commands.entity(entity).insert((Enemy { kind, think: f32::MAX, ..default() }, Husk { lane, ..Husk::new(phase, speed) }));
    entity
}

/// Authority: survivors join, go down, bleed, get revived, board windows; the run ends.
#[allow(clippy::too_many_arguments)]
fn survivors(
    time: Res<Time>,
    map: Res<Map>,
    rules: Res<Rules>,
    content: Res<Content>,
    mut state: ResMut<MatchState>,
    mut horde: ResMut<Horde>,
    mut players: Query<(&mut Player, &mut Health, &mut Transform, &mut Arsenal, &mut PlayerInput), Without<Husk>>,
    mut fx: EventWriter<FxEvent>,
) {
    let Some(layout) = map.horde.as_deref() else { return };
    let dt = sim_dt(&time);
    for (player, ..) in &players {
        if state.horde.survivor(player.id).is_none() {
            state.horde.survivors.push(Survivor::new(player.id));
        }
    }
    if state.over() {
        return;
    }
    let solo = players.iter().count() <= 1;

    // A new round: the fallen come back with a pistol, everyone gets grenades.
    if horde.started != state.horde.round && state.horde.rest <= 0.0 {
        horde.started = state.horde.round;
        for (mut player, mut health, mut transform, mut arsenal, mut input) in &mut players {
            player.gear_left = (player.gear_left + 2).min(4);
            if health.alive() {
                continue;
            }
            crate::player::respawn(&content, &map, &rules, &state, &mut player, &mut health, &mut transform, &mut arsenal, &mut input);
            for (i, weapon) in arsenal.weapons.iter_mut().enumerate() {
                *weapon = WeaponState::fresh(&content.weapons[i]);
            }
            grant_weapon(&mut arsenal, &content.weapons, gun::PISTOL);
            arsenal.current = gun::PISTOL as u8;
            input.weapon = gun::PISTOL as u8;
            if let Some(s) = state.horde.survivor_mut(player.id) {
                s.out = false;
                s.perks = 0;
                s.third = None;
            }
            fx.write(FxEvent::notice(format!("{} IS BACK", player.name)));
        }
    }

    let where_: Vec<(u8, Vec3, bool, bool)> = players.iter().map(|(p, h, t, ..)| (p.id, t.translation, h.alive(), state.horde.is_down(p.id))).collect();
    // Held Use: revive a teammate, or board up a window.
    let mut revivers: HashMap<u8, u8> = HashMap::new();
    for &(id, at, alive, down) in &where_ {
        if !alive || down || !horde.holding(id) {
            continue;
        }
        let close = where_.iter().filter(|(o, p, a, d)| *o != id && *a && *d && p.distance(at) < 2.2).min_by(|a, b| a.1.distance(at).total_cmp(&b.1.distance(at)));
        if let Some((patient, ..)) = close {
            revivers.insert(*patient, id);
            continue;
        }
        // The generator: cranked a little more every moment Use is held on it.
        if let Some(power) = layout.power.as_ref()
            && !state.horde.power
            && power.pos.xz().distance(at.xz()) < 1.8
            && (power.pos.y - at.y).abs() < 1.6
        {
            let before = state.horde.charge;
            state.horde.charge = (before + dt / CRANK_SECONDS).min(1.0);
            if (before * 4.0).floor() != (state.horde.charge * 4.0).floor() {
                fx.write(FxEvent::Sound { sfx: Sfx::Build, at: Some(power.pos + Vec3::Y) });
            }
            if state.horde.charge >= 1.0 {
                state.horde.power = true;
                fx.write(FxEvent::notice("POWER RESTORED - PERKS, THE REFORGE AND THE GRATES ARE LIVE"));
                fx.write(FxEvent::Sound { sfx: Sfx::Radar, at: None });
            }
            continue;
        }
        // Boards: one every so often, points for each (capped per round).
        let window = layout.windows.iter().enumerate().filter(|(i, w)| state.horde.planks.get(*i).is_some_and(|p| *p < PLANKS) && w.inside.distance(at) < 2.0).map(|(i, _)| i).next();
        let Some(window) = window else { continue };
        let timer = horde.repair_timer.entry(id).or_insert(REPAIR_EVERY);
        *timer -= dt;
        if *timer > 0.0 {
            continue;
        }
        *timer = REPAIR_EVERY;
        state.horde.planks[window] += 1;
        let give = state.horde.survivor(id).is_some_and(|s| s.repaired < REPAIR_CAP);
        if give {
            state.horde.earn(id, 10);
            if let Some(s) = state.horde.survivor_mut(id) {
                s.repaired += 10;
            }
        }
        fx.write(FxEvent::Sound { sfx: Sfx::Build, at: Some(layout.windows[window].center + Vec3::Y * 1.6) });
    }

    let mut everyone_down = true;
    let mut anyone = false;
    for (mut player, mut health, _, mut arsenal, mut input) in &mut players {
        anyone = true;
        let id = player.id;
        let Some(s) = state.horde.survivor(id).cloned() else { continue };
        if !health.alive() {
            continue;
        }
        if !s.down {
            everyone_down = false;
            continue;
        }
        health.hp = health.max * 0.3;
        if arsenal.current as usize != gun::PISTOL {
            arsenal.current = gun::PISTOL as u8;
        }
        input.weapon = gun::PISTOL as u8;
        let mut s = s;
        let reviver = revivers.get(&id).copied();
        let mut up = false;
        if let Some(by) = reviver {
            let quick = state.horde.survivor(by).is_some_and(|r| r.has(perk::PATCH_UP));
            s.revive += dt / if quick { REVIVE * 0.5 } else { REVIVE };
            s.reviver = by;
            up = s.revive >= 1.0;
        } else {
            s.revive = 0.0;
            s.reviver = NOBODY;
            s.bleed -= dt;
        }
        if s.self_revive > 0.0 {
            everyone_down = false;
            s.self_revive -= dt;
            if s.self_revive <= 0.0 {
                up = true;
            }
        }
        if up {
            s.down = false;
            s.revive = 0.0;
            s.reviver = NOBODY;
            s.self_revive = 0.0;
            health.hp = health.max;
            if s.borrowed {
                arsenal.weapons[gun::PISTOL] = WeaponState::fresh(&content.weapons[gun::PISTOL]);
                s.borrowed = false;
            }
            arsenal.current = if arsenal.owns(s.held as usize) { s.held } else { (0..arsenal.weapons.len()).find(|i| arsenal.owns(*i)).unwrap_or(0) as u8 };
            input.weapon = arsenal.current;
            input.stance = stance::STAND;
            if let Some(by) = reviver
                && let Some(r) = state.horde.survivor_mut(by)
            {
                r.revives += 1;
            }
            fx.write(FxEvent::notice(format!("{} IS BACK UP", player.name)));
            everyone_down = false;
        } else if s.bleed <= 0.0 {
            // Bled out: the robot falls apart and sits out the rest of the round.
            s.down = false;
            s.out = true;
            s.perks = 0;
            s.borrowed = false;
            health.hp = 0.0;
            player.deaths += 1;
            fx.write(FxEvent::notice(format!("{} BLED OUT", player.name)));
        } else if reviver.is_some() {
            everyone_down = false;
        }
        if let Some(slot) = state.horde.survivor_mut(id) {
            *slot = s;
        }
    }
    let _ = solo;
    if anyone && everyone_down {
        state.phase = Phase::Lost;
        state.kills = state.horde.kills;
        state.day = state.horde.round as u32;
        fx.write(FxEvent::notice(format!("OVERRUN - YOU SURVIVED {} ROUNDS", state.horde.round.saturating_sub(1))));
        fx.write(FxEvent::Sound { sfx: Sfx::Alarm, at: None });
    }
}

/// Takes a robot down (instead of killing it).
fn go_down(state: &mut HordeState, content: &Catalog, player: &Player, health: &mut Health, arsenal: &mut Arsenal, solo: bool, fx: &mut EventWriter<FxEvent>) {
    let Some(s) = state.survivor_mut(player.id) else { return };
    if s.down {
        return;
    }
    s.down = true;
    s.downs += 1;
    s.bleed = BLEED;
    s.revive = 0.0;
    s.reviver = NOBODY;
    if solo && s.has(perk::PATCH_UP) && s.self_revives < 3 {
        s.self_revive = 4.0;
        s.self_revives += 1;
    }
    // Perks are lost, and the spare socket's gun with them.
    if s.has(perk::SPARE_SOCKET)
        && let Some(third) = s.third.take()
        && arsenal.owns(third as usize)
        && arsenal.weapons.iter().filter(|w| w.owned).count() > 2
    {
        arsenal.weapons[third as usize] = WeaponState::fresh(&content.weapons[third as usize]);
        if arsenal.current == third {
            arsenal.current = (0..arsenal.weapons.len()).find(|i| arsenal.owns(*i)).unwrap_or(0) as u8;
        }
    }
    s.perks = 0;
    s.held = arsenal.current;
    s.borrowed = !arsenal.owns(gun::PISTOL);
    if s.borrowed {
        grant_weapon(arsenal, &content.weapons, gun::PISTOL);
    }
    arsenal.current = gun::PISTOL as u8;
    arsenal.reload = 0.0;
    health.max = 100.0;
    health.hp = 30.0;
    fx.write(FxEvent::notice(format!("{} IS DOWN", player.name)));
    fx.write(FxEvent::Sound { sfx: Sfx::Alarm, at: None });
}

/// Authority: husks climb in, hunt, and claw.
#[allow(clippy::too_many_arguments)]
fn husk_brains(
    time: Res<Time>,
    map: Res<Map>,
    statics: Res<Statics>,
    content: Res<Content>,
    grid: Res<Grid>,
    nav: Option<ResMut<HordeNav>>,
    mut state: ResMut<MatchState>,
    mut rng: ResMut<SimRng>,
    mut ids: ResMut<NetIds>,
    mut commands: Commands,
    mut husks: Query<(Entity, &mut Transform, &mut Enemy, &mut Husk, &Health)>,
    mut players: Query<(Entity, &Player, &Transform, &mut Health, &mut Arsenal), Without<Husk>>,
    mut horde: ResMut<Horde>,
    mut hits: EventWriter<Hit>,
    mut fx: EventWriter<FxEvent>,
) {
    let Some(layout) = map.horde.as_deref() else { return };
    let Some(mut nav) = nav else { return };
    let dt = sim_dt(&time);
    let solo = players.iter().count() <= 1;

    // Volt husks scrapped last frame go off: robots close by get hurt, husks close by get
    // torn apart (on the account of whoever set it off).
    for (at, by) in std::mem::take(&mut horde.bursts) {
        fx.write(FxEvent::Explosion { at: at + Vec3::Y * 0.9, radius: VOLT_RADIUS });
        fx.write(FxEvent::Sound { sfx: Sfx::Explosion, at: Some(at) });
        for (_, player, transform, mut health, mut arsenal) in &mut players {
            if !health.alive() || state.horde.is_down(player.id) || transform.translation.distance(at) > VOLT_RADIUS {
                continue;
            }
            let player = player.clone();
            health.hp -= VOLT_PLAYER_DAMAGE;
            fx.write(FxEvent::Hurt { player: player.id });
            if health.hp <= 0.0 {
                go_down(&mut state.horde, &content, &player, &mut health, &mut arsenal, solo, &mut fx);
            }
        }
        for (entity, transform, _, _, health) in &husks {
            if health.alive() && transform.translation.distance(at) < VOLT_RADIUS {
                hits.write(Hit::plain(entity, VOLT_HUSK_DAMAGE, by));
            }
        }
    }
    // Last few of a round: they stop shambling.
    let hurry = state.horde.round >= 2 && state.horde.left <= 3;
    // One husk at a time climbs through a window.
    let mut climbing: Vec<u8> = husks.iter().filter_map(|(_, _, _, h, _)| if let Phase2::Climb { window, .. } = h.phase { Some(window) } else { None }).collect();
    let targets: Vec<(Entity, Vec3)> = players.iter().filter(|(_, p, _, h, _)| h.alive() && !state.horde.is_down(p.id)).map(|(e, _, t, ..)| (e, t.translation)).collect();
    let closed: Vec<Aabb> = layout.doors.iter().enumerate().filter(|(i, _)| !state.horde.door_open(*i)).map(|(_, d)| d.aabb).collect();
    let free = |p: Vec3| !statics.blocked(p.x, p.z, p.y, 1.5, 0.28) && !closed.iter().any(|b| b.touches(p.x, p.z, 0.28)) && map.in_bounds(p.x, p.z);
    let mut searches = 4;
    let mut crawlers = Vec::new();
    let mut downs: Vec<Entity> = Vec::new();
    for (entity, mut transform, mut enemy, mut husk, health) in &mut husks {
        if !health.alive() {
            continue;
        }
        let def = &content.enemies[enemy.kind as usize];
        let here = transform.translation;
        let slow = if enemy.slow > 0.0 { 0.5 } else { 1.0 };
        enemy.think = f32::MAX;
        if husk.crawl && enemy.kind == kind::HUSK {
            crawlers.push((entity, here, transform.rotation, health.fraction()));
            continue;
        }
        let face = |transform: &mut Transform, dir: Vec2, dt: f32| {
            if dir.length_squared() > 1e-4 {
                let want = Quat::from_rotation_y(f32::atan2(-dir.x, -dir.y));
                transform.rotation = transform.rotation.slerp(want, (dt * 7.0).min(1.0));
            }
        };
        let lane = |w: &oz_core::worldgen::HordeWindow, lane: f32| w.outside + Vec3::new(-w.out.y, 0.0, w.out.x) * lane;
        match husk.phase {
            Phase2::Approach { window } => {
                let w = layout.windows[window as usize];
                let to = (lane(&w, husk.lane) - here).xz();
                if to.length() < 0.35 {
                    husk.phase = Phase2::Tear { window };
                    continue;
                }
                let step = to.normalize() * (husk.speed.min(4.5) * slow * dt).min(to.length());
                transform.translation += Vec3::new(step.x, 0.0, step.y);
                transform.translation.y = map.height(transform.translation.x, transform.translation.z).max(layout.ground);
                face(&mut transform, to, dt);
                continue;
            }
            Phase2::Tear { window } => {
                let w = layout.windows[window as usize];
                face(&mut transform, -w.out, dt);
                let planks = state.horde.planks.get(window as usize).copied().unwrap_or(0);
                if planks == 0 {
                    // Through the gap one at a time.
                    if !climbing.contains(&window) {
                        climbing.push(window);
                        husk.phase = Phase2::Climb { window, t: 0.0 };
                    }
                    continue;
                }
                husk.tear -= dt;
                if husk.tear <= 0.0 {
                    husk.tear = if enemy.kind == kind::BRUTE { 0.6 } else { 1.0 } * rng.0.range(0.85, 1.2);
                    state.horde.planks[window as usize] = planks - 1;
                    enemy.attacks = enemy.attacks.wrapping_add(1);
                    fx.write(FxEvent::Sound { sfx: Sfx::Debris, at: Some(w.center + Vec3::Y * 1.6) });
                }
                continue;
            }
            Phase2::Climb { window, t } => {
                let w = layout.windows[window as usize];
                let t = t + dt / if enemy.kind == kind::VOLT { 0.7 } else { 1.1 };
                let k = t.min(1.0);
                let mut p = lane(&w, husk.lane).lerp(w.inside, k);
                p.y += (k * std::f32::consts::PI).sin() * 1.05;
                transform.translation = p;
                face(&mut transform, -w.out, dt);
                husk.phase = if t >= 1.0 { Phase2::Hunt } else { Phase2::Climb { window, t } };
                continue;
            }
            Phase2::Hunt => {}
        }

        // Hunting: pick the nearest robot now and then.
        husk.retarget -= dt;
        if husk.retarget <= 0.0 || husk.target.is_none_or(|e| !targets.iter().any(|(t, _)| *t == e)) {
            husk.retarget = 0.6;
            husk.target = targets.iter().min_by(|a, b| a.1.distance_squared(here).total_cmp(&b.1.distance_squared(here))).map(|(e, _)| *e);
        }
        husk.cooldown -= dt;
        let Some(goal) = husk.target.and_then(|e| targets.iter().find(|(t, _)| *t == e)).map(|(_, p)| *p) else {
            continue;
        };
        let to_goal = goal - here;
        let distance = to_goal.xz().length();
        let reach = def.radius + 0.75;

        // Claws: a wind-up, then the swipe lands if the robot is still there.
        if husk.windup > 0.0 {
            husk.windup -= dt;
            face(&mut transform, to_goal.xz(), dt);
            if husk.windup <= 0.0 {
                husk.cooldown = def.attack_interval;
                if distance < reach + 0.5 && to_goal.y.abs() < 1.6 {
                    if let Some(target) = husk.target {
                        downs.push(target);
                        let damage = def.damage;
                        if let Ok((_, player, _, mut health, mut arsenal)) = players.get_mut(target) {
                            let player = player.clone();
                            health.hp -= damage;
                            fx.write(FxEvent::Hurt { player: player.id });
                            fx.write(FxEvent::Sound { sfx: Sfx::ImpactMetal, at: Some(goal + Vec3::Y) });
                            if health.hp <= 0.0 {
                                go_down(&mut state.horde, &content, &player, &mut health, &mut arsenal, solo, &mut fx);
                            }
                        }
                    }
                }
            }
            continue;
        }
        if distance < reach && to_goal.y.abs() < 1.4 && husk.cooldown <= 0.0 {
            husk.windup = if enemy.kind == kind::BRUTE { 0.6 } else { 0.4 };
            enemy.attacks = enemy.attacks.wrapping_add(1);
            continue;
        }

        // Where to step: straight at the robot when the way is clear, else along a path.
        husk.repath -= dt;
        let direct = distance < 16.0 && nav.nav.line_clear(here, goal);
        let mut aim = goal;
        if !direct {
            if (husk.path.is_empty() || husk.repath <= 0.0) && searches > 0 {
                searches -= 1;
                husk.repath = rng.0.range(1.2, 2.0);
                husk.path = nav.path(here, goal).unwrap_or_default();
            }
            while husk.path.first().is_some_and(|p| p.xz().distance(here.xz()) < 0.6) {
                husk.path.remove(0);
            }
            if let Some(next) = husk.path.first() {
                aim = *next;
            }
        } else {
            husk.path.clear();
        }
        let mut dir = (aim - here).xz().normalize_or_zero();
        if husk.dodge > 0.0 {
            husk.dodge -= dt;
            dir = (Vec2::new(-dir.y, dir.x) * husk.dodge_side + dir * 0.4).normalize_or_zero();
        }
        let pace = if hurry { husk.speed.max(if enemy.kind == kind::CRAWLER { 1.7 } else { 3.0 }) } else { husk.speed };
        let speed = if distance < reach + 0.3 { 0.0 } else { pace * slow };
        let mut step = dir * speed;
        // Shoulder past other husks instead of stacking up.
        let mut push = Vec2::ZERO;
        grid.for_each_near(here, def.radius + 1.2, |other| {
            if other.entity != entity {
                let away = (here - other.pos).xz();
                let gap = away.length();
                let overlap = def.radius + other.radius.min(0.45) - gap;
                if overlap > 0.0 && gap > 1e-3 {
                    push += away / gap * overlap;
                }
            }
        });
        step += push.clamp_length_max(1.0) * 2.5;
        let delta = step * dt;
        let mut p = here;
        let tryx = Vec3::new(p.x + delta.x, p.y, p.z);
        if free(tryx) {
            p = tryx;
        }
        let tryz = Vec3::new(p.x, p.y, p.z + delta.y);
        if free(tryz) {
            p = tryz;
        }
        p.y = ground_under(&map, &statics, p.x, p.z, p.y + 0.3).max(layout.ground - 0.5);
        if p.y < here.y - 0.6 {
            p.y = here.y - (here.y - p.y).min(8.0 * dt);
        }
        transform.translation = p;
        face(&mut transform, if distance < 3.0 { to_goal.xz() } else { dir }, dt);

        // Stuck: try a new path; still stuck with nobody close, climb back out somewhere else.
        husk.check -= dt;
        if husk.check <= 0.0 {
            husk.check = 4.0;
            if husk.last.distance(p) < 0.6 && distance > 3.0 {
                husk.stuck += 1;
                husk.path.clear();
                husk.repath = 0.0;
                husk.dodge = 0.9;
                husk.dodge_side = if rng.0.chance(0.5) { 1.0 } else { -1.0 };
            } else {
                husk.stuck = 0;
            }
            husk.last = p;
            if husk.stuck >= 3 && distance > 12.0 {
                let targets_at: Vec<Vec3> = targets.iter().map(|(_, p)| *p).collect();
                if let Some((phase, at)) = pick_spawn(layout, &state.horde, &targets_at, &mut rng) {
                    transform.translation = at;
                    husk.phase = phase;
                    husk.stuck = 0;
                    husk.path.clear();
                }
            }
        }
    }
    let _ = downs;
    // Husks that lost their legs come back on their hands.
    for (entity, at, rotation, fraction) in crawlers {
        commands.entity(entity).despawn();
        let hp = husk_health(state.horde.round) * fraction.max(0.05);
        let crawler = spawn_husk(&mut commands, &mut ids, &content, kind::CRAWLER, at, Phase2::Hunt, hp, 1.0, &mut rng);
        commands.entity(crawler).insert(Transform::from_translation(at).with_rotation(rotation));
        fx.write(FxEvent::Sound { sfx: Sfx::LimbOff, at: Some(at + Vec3::Y * 0.5) });
    }
}

/// Authority (before hits are applied): points, headshots, melee, crawlers, power-up drops.
#[allow(clippy::too_many_arguments)]
fn score_hits(
    mut hits: EventReader<Hit>,
    content: Res<Content>,
    mut state: ResMut<MatchState>,
    mut rng: ResMut<SimRng>,
    mut players: Query<(&mut Player, &Transform, &PlayerInput)>,
    mut husks: Query<(&Transform, &Enemy, &mut Health, &mut Husk), Without<Player>>,
    mut pending: Local<HashMap<Entity, f32>>,
    mut paid: Local<Vec<(Entity, u8)>>,
    mut horde: ResMut<Horde>,
) {
    pending.clear();
    paid.clear();
    for hit in hits.read() {
        let Ok((transform, enemy, mut health, mut husk)) = husks.get_mut(hit.target) else { continue };
        let hp = *pending.entry(hit.target).or_insert(health.hp);
        if hp <= 0.0 {
            continue;
        }
        let shooter = hit.by.and_then(|id| players.iter().find(|(p, ..)| p.id == id).map(|(p, t, input)| (p.id, t.translation + Vec3::Y * input.eye(), input.aim_dir(), p.melee > 0.7)));
        let (head, legs, melee) = match shooter {
            Some((_, eye, dir, melee)) => {
                let (head, legs) = struck_part(enemy.kind, transform, eye, dir);
                (head && !melee, legs && !melee, melee)
            }
            None => (false, false, false),
        };
        // Headshots hurt three times as much; the killswitch drops anything in one.
        let bonus = if state.horde.insta > 0.0 && hit.by.is_some() {
            hp * 10.0
        } else if head {
            hit.amount * 2.0
        } else {
            0.0
        };
        if bonus > 0.0 {
            health.hp = (health.hp - bonus).max(0.01);
        }
        let left = hp - hit.amount - bonus;
        pending.insert(hit.target, left);
        if legs && left > 0.0 && enemy.kind == kind::HUSK {
            husk.legs += hit.amount;
            if husk.legs > health.max * 0.45 && rng.0.chance(0.5) {
                husk.crawl = true;
            }
        }
        // A volt husk scrapped by someone goes off (a meltdown just switches it off).
        if left <= 0.0 && enemy.kind == kind::VOLT && hit.by.is_some() {
            horde.bursts.push((transform.translation, hit.by));
        }
        let Some(id) = hit.by else { continue };
        let magnet = state.horde.survivor(id).is_some_and(|s| s.has(perk::SCRAP_MAGNET));
        if left <= 0.0 {
            let points = if melee {
                130
            } else if head {
                100
            } else {
                60
            };
            state.horde.earn(id, if magnet { points * 3 / 2 } else { points });
            state.horde.kills += 1;
            if let Some(s) = state.horde.survivor_mut(id) {
                s.kills += 1;
                s.headshots += head as u16;
            }
            if let Some((mut player, ..)) = players.iter_mut().find(|(p, ..)| p.id == id) {
                player.kills = player.kills.saturating_add(1);
            }
            // Now and then a husk leaves something behind, and surely after enough kills.
            horde.since_drop += 1;
            let due = horde.since_drop >= drop_every(state.horde.round);
            if horde.drops < 4 && (due || rng.0.chance(if magnet { 0.07 } else { 0.035 })) {
                horde.drops += 1;
                horde.since_drop = 0;
                let kind = pick_drop(horde.last_drop, &mut rng);
                horde.last_drop = kind;
                let at = transform.translation;
                state.horde.drops.push(Drop { kind, at: Vec3::new(at.x, at.y.max(0.0) + 0.9, at.z), ttl: 26.0 });
            }
        } else if !paid.contains(&(hit.target, id)) {
            paid.push((hit.target, id));
            state.horde.earn(id, 10);
        }
    }
    let _ = &content;
}

/// Kills between sure power-ups: often early on, less so as the rounds swell.
pub fn drop_every(round: u16) -> u32 {
    (20 + 3 * round as u32).min(70)
}

/// A power-up by weight, never the same one twice running.
fn pick_drop(last: u8, rng: &mut SimRng) -> u8 {
    let total: u32 = (0..drop::COUNT).filter(|k| *k != last).map(|k| drop::WEIGHTS[k as usize]).sum();
    let mut roll = rng.0.below(total.max(1) as usize) as u32;
    for k in (0..drop::COUNT).filter(|k| *k != last) {
        let w = drop::WEIGHTS[k as usize];
        if roll < w {
            return k;
        }
        roll -= w;
    }
    drop::AMMO
}

/// Authority: what the perks do.
fn perk_effects(time: Res<Time>, state: Res<MatchState>, mut players: Query<(&mut Player, &mut Health, &mut Arsenal)>) {
    let dt = sim_dt(&time);
    for (mut player, mut health, mut arsenal) in &mut players {
        let Some(s) = state.horde.survivor(player.id) else { continue };
        if s.down || !health.alive() {
            continue;
        }
        let max = if s.has(perk::IRON_HIDE) { 250.0 } else { 100.0 };
        if health.max != max {
            let gained = max > health.max;
            health.max = max;
            health.hp = if gained { max } else { health.hp.min(max) };
        }
        let fleet = s.has(perk::FLEETFOOT) as u8;
        if player.perk != fleet {
            player.perk = fleet;
        }
        if s.has(perk::QUICKFIX) && arsenal.reload > 0.0 {
            arsenal.reload = (arsenal.reload - dt * 0.85).max(0.001);
        }
        let mods = if s.has(perk::HAIR_TRIGGER) { [Some((ModKind::Capacitor, 2)), Some((ModKind::Heavy, 1)), None] } else { [None; 3] };
        for weapon in arsenal.weapons.iter_mut() {
            if weapon.mods != mods {
                weapon.mods = mods;
            }
        }
    }
}

/// Authority: bot squadmates follow the nearest human, shoot husks they can see, pick up
/// the fallen, and board up windows near the squad when nothing needs shooting.
#[allow(clippy::too_many_arguments)]
fn horde_bots(
    time: Res<Time>,
    statics: Res<Statics>,
    content: Res<Content>,
    map: Res<Map>,
    state: Res<MatchState>,
    nav: Option<ResMut<HordeNav>>,
    mut horde: ResMut<Horde>,
    mut rng: ResMut<SimRng>,
    mut bots: Query<(&Player, &Transform, &mut PlayerInput, &mut Arsenal, &Health, &mut HordeBot), With<Bot>>,
    people: Query<(&Player, &Transform, &Health), Without<HordeBot>>,
    husks: Query<(&Transform, &Enemy), With<Husk>>,
) {
    let Some(mut nav) = nav else { return };
    let dt = sim_dt(&time);
    let downed: Vec<(u8, Vec3)> = people.iter().chain(bots.iter().map(|(p, t, _, _, h, _)| (p, t, h))).filter(|(p, _, h)| h.alive() && state.horde.is_down(p.id)).map(|(p, t, _)| (p.id, t.translation)).collect();
    let leader = people.iter().filter(|(p, _, h)| h.alive() && !state.horde.is_down(p.id)).map(|(_, t, _)| t.translation).next();
    let enemies: Vec<(Vec3, u16)> = husks.iter().map(|(t, e)| (t.translation, e.kind)).collect();
    let round = state.horde.round;
    for (player, transform, mut input, mut arsenal, health, mut brain) in &mut bots {
        if !health.alive() {
            continue;
        }
        let me = transform.translation;
        let down = state.horde.is_down(player.id);
        input.lean = 0;
        input.cover = 0;
        input.light = state.daylight < 0.5;
        input.stance = if down { stance::PRONE } else { stance::STAND };
        let mut held = 0u8;
        // Better guns as the rounds go by (bots never shop).
        let wanted = match round {
            0..=2 => gun::PISTOL,
            3..=5 => gun::SMG,
            6..=9 => gun::CARBINE,
            10..=14 => gun::MINIGUN,
            _ => upgraded(&content, gun::CARBINE).unwrap_or(gun::CARBINE),
        };
        if !down && !arsenal.owns(wanted) && wanted < arsenal.weapons.len() {
            for (i, weapon) in arsenal.weapons.iter_mut().enumerate() {
                if i != gun::PISTOL {
                    *weapon = WeaponState::fresh(&content.weapons[i]);
                }
            }
            grant_weapon(&mut arsenal, &content.weapons, wanted);
            arsenal.current = wanted as u8;
        }
        if !down {
            input.weapon = if arsenal.owns(wanted) { wanted as u8 } else { arsenal.current };
            let current = input.weapon as usize;
            if let (Some(w), Some(def)) = (arsenal.weapons.get_mut(current), content.weapons.get(current))
                && w.reserve < def.mag
            {
                w.reserve = def.reserve;
            }
        }

        // Shoot the closest husk in sight.
        let eye = me + Vec3::Y * input.eye();
        let mut target: Option<Vec3> = None;
        let mut nearest = f32::MAX;
        for (at, kind) in &enemies {
            let aim = *at + Vec3::Y * if *kind == kind::CRAWLER { 0.4 } else if *kind == kind::BRUTE { 1.6 } else { 1.2 };
            let d = aim.distance(eye);
            if d < 28.0 && d < nearest {
                let dir = (aim - eye) / d.max(1e-3);
                if statics.raycast(eye, dir, d).is_none() {
                    nearest = d;
                    target = Some(aim);
                }
            }
        }
        if let Some(aim) = target {
            let wobble = Vec2::new(rng.0.range(-1.0, 1.0), rng.0.range(-1.0, 1.0)) * 0.03;
            let drift = (wobble - brain.aim) * (dt * 2.0).min(1.0);
            brain.aim += drift;
            let dir = (aim - eye).normalize_or(Vec3::NEG_Z);
            let yaw = f32::atan2(-dir.x, -dir.z) + brain.aim.x;
            let pitch = dir.y.clamp(-1.0, 1.0).asin() + brain.aim.y;
            let turn = |from: f32, to: f32, max: f32| {
                let diff = (to - from + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
                from + diff.clamp(-max, max)
            };
            input.yaw = turn(input.yaw, yaw, dt * 7.0);
            input.pitch = turn(input.pitch, pitch, dt * 7.0);
            let on = (input.yaw - yaw).abs() < 0.12;
            let auto = content.weapons.get(input.weapon as usize).is_some_and(|d| d.auto());
            brain.pulse = !brain.pulse;
            if on && (auto || brain.pulse) {
                held |= buttons::FIRE;
            }
        }
        if let Some(w) = arsenal.state()
            && w.ammo == 0
        {
            held |= buttons::RELOAD;
        }

        // Where to go: a downed teammate, else the leader.
        let mut wish = Vec2::ZERO;
        if !down {
            let patient = downed.iter().filter(|(id, _)| *id != player.id).min_by(|a, b| a.1.distance(me).total_cmp(&b.1.distance(me))).copied();
            let threat = enemies.iter().any(|(p, _)| p.distance(me) < 2.5);
            // Boards down on a window close to the squad, and nothing to shoot: fix them.
            let repair = map.horde.as_deref().filter(|_| target.is_none() && !threat).and_then(|layout| {
                let open = layout.open_zones(state.horde.doors);
                layout
                    .windows
                    .iter()
                    .enumerate()
                    .filter(|(i, w)| open & (1 << w.zone) != 0 && state.horde.planks.get(*i).is_some_and(|p| *p < PLANKS))
                    .map(|(_, w)| w.inside)
                    .filter(|at| at.distance(me) < 9.0 && leader.is_none_or(|l| l.distance(*at) < 12.0))
                    .min_by(|a, b| a.distance(me).total_cmp(&b.distance(me)))
            });
            let goal = match (patient, leader) {
                (Some((id, at)), _) if at.distance(me) < 40.0 => {
                    if at.distance(me) < 1.6 && !threat {
                        horde.hold(player.id);
                        let _ = id;
                        None
                    } else {
                        Some((at, 0.8))
                    }
                }
                _ if repair.is_some() => {
                    let at = repair.unwrap_or(me);
                    if at.distance(me) < 1.3 {
                        horde.hold(player.id);
                        None
                    } else {
                        Some((at, 0.5))
                    }
                }
                (_, Some(at)) => Some((at, 4.0)),
                _ => None,
            };
            if let Some((goal, stop)) = goal
                && goal.distance(me) > stop
            {
                brain.repath -= dt;
                if brain.path.is_empty() || brain.repath <= 0.0 || brain.goal.distance(goal) > 3.0 {
                    brain.repath = 1.0;
                    brain.goal = goal;
                    brain.path = nav.path(me, goal).unwrap_or_default();
                }
                while brain.path.first().is_some_and(|p| p.xz().distance(me.xz()) < 0.6) {
                    brain.path.remove(0);
                }
                let next = brain.path.first().copied().unwrap_or(goal);
                wish = (next - me).xz().normalize_or_zero();
            }
            // Back off a husk in its face.
            if let Some((p, _)) = enemies.iter().filter(|(p, _)| p.distance(me) < 2.2).min_by(|a, b| a.0.distance(me).total_cmp(&b.0.distance(me))) {
                wish = (wish + (me - *p).xz().normalize_or_zero() * 1.5).normalize_or_zero();
            }
            if target.is_none() && wish.length() > 0.1 {
                input.yaw = f32::atan2(-wish.x, -wish.y);
                input.pitch *= 0.9;
            }
        }
        let local = Quat::from_rotation_y(-input.yaw) * Vec3::new(wish.x, 0.0, wish.y);
        input.movement = Vec2::new(local.x, -local.z).clamp_length_max(1.0);
        input.buttons = held;
    }
}
