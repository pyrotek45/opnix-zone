//! Objective modes on top of the versus plumbing in [`crate::versus`].
//!
//! * Domination (`Rules::domination`): three points, A by team 0's base, B in the middle
//!   and C by team 1's. Stand in one to take it - faster with friends, frozen while an
//!   enemy stands in it too. Every point a team holds scores for it every few seconds;
//!   kills score nothing.
//! * Search and destroy (`Rules::bomb`): rounds with one life each. The attackers pick
//!   the bomb up at their base, carry it to site A or B by the defenders' base and plant
//!   it (hold Use); the defenders defuse it (hold Use) before the fuse runs out. A round
//!   is won by wiping out the other side, by the blast, by a defuse, or (defenders) by
//!   running out the clock. Sides swap at half time; `score_limit` is rounds to win.
//!
//! Everything a client needs to draw is in [`Objective`], replicated inside
//! [`MatchState`]. Points and sites are placed on the authority from the map's bases,
//! its waypoints (indoor maps) and the navigation grid, so every one can be walked to.

use crate::{
    nav::{Nav, NavScratch},
    player::{Arsenal, BASE_RADIUS, Health, Player, PlayerInput, base_centre, damage_player, ground_under, part, respawn, safest_spawn},
    session::{Content, Map, Statics},
    sim_dt,
    state::{FxEvent, MatchState, Phase, Rules, SimRng},
};
use bevy::prelude::*;
use oz_core::{collide::StaticWorld, synth::Sfx, worldgen::World as GameMap};
use serde::{Deserialize, Serialize};

/// Which objective mode is running ([`Objective::kind`]).
pub mod kind {
    pub const NONE: u8 = 0;
    pub const DOMINATION: u8 = 1;
    pub const BOMB: u8 = 2;
}

/// Where the bomb is ([`Objective::bomb`]).
pub mod bomb {
    /// Lying at the attackers' base at the start of a round.
    pub const HOME: u8 = 0;
    pub const CARRIED: u8 = 1;
    /// On the floor where its carrier went down.
    pub const DROPPED: u8 = 2;
    pub const PLANTED: u8 = 3;
    pub const DEFUSED: u8 = 4;
    pub const EXPLODED: u8 = 5;
}

/// Search and destroy round stages ([`Objective::stage`]).
pub mod stage {
    pub const WAITING: u8 = 0;
    pub const PLAYING: u8 = 1;
    /// The round is decided: a short pause before the next one.
    pub const OVER: u8 = 2;
}

/// No team: an untaken point.
pub const NEUTRAL: u8 = 2;
/// No player.
pub const NOBODY: u8 = 255;
pub const LETTERS: [&str; 3] = ["A", "B", "C"];
pub const TEAM_NAMES: [&str; 2] = ["TEAL", "CORAL"];

/// Domination: how far from a point's centre you can stand and still be taking it.
pub const POINT_RADIUS: f32 = 5.5;
/// Domination: seconds for one player to take a point (more players are faster).
pub const CAPTURE_SECONDS: f32 = 7.0;
/// Domination: every held point scores one point this often.
pub const SCORE_EVERY: f32 = 2.0;
/// Search and destroy: how close to a site's centre the bomb can be planted.
pub const SITE_RADIUS: f32 = 3.5;
pub const PLANT_SECONDS: f32 = 4.0;
pub const DEFUSE_SECONDS: f32 = 6.0;
pub const FUSE_SECONDS: f32 = 40.0;
pub const ROUND_SECONDS: f32 = 120.0;
/// Pause between a decided round and the next.
pub const ROUND_BREAK: f32 = 6.0;
/// Walk within this of the bomb to pick it up; stand within this of a planted one to defuse.
pub const PICKUP_REACH: f32 = 1.6;
pub const DEFUSE_REACH: f32 = 2.0;
/// The blast: everyone this close is caught, the nearest worst.
pub const BLAST_RADIUS: f32 = 16.0;

/// A domination point or a bomb site.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub at: Vec3,
    /// The team holding it, or [`NEUTRAL`].
    pub owner: u8,
    /// The team taking it ([`NEUTRAL`] for nobody) and how far along they are (0-1).
    pub taking: u8,
    pub progress: f32,
    /// Both teams are standing in it.
    pub contested: bool,
}

impl Point {
    pub fn new(at: Vec3) -> Self {
        Self { at, owner: NEUTRAL, taking: NEUTRAL, progress: 0.0, contested: false }
    }

    /// Is someone standing at `feet` inside the point?
    pub fn holds(&self, feet: Vec3) -> bool {
        feet.xz().distance(self.at.xz()) < POINT_RADIUS && (feet.y - self.at.y).abs() < 2.6
    }

    /// One frame of capturing, given how many of each team stand in the point.
    /// Returns the team that just took it.
    pub fn tick(&mut self, present: [usize; 2], dt: f32) -> Option<u8> {
        self.contested = present[0] > 0 && present[1] > 0;
        if self.contested {
            return None;
        }
        let Some(team) = (0..2u8).find(|t| present[*t as usize] > 0) else {
            // Nobody here: a half-taken point slowly settles back.
            self.progress -= dt / (CAPTURE_SECONDS * 2.0);
            if self.progress <= 1e-4 {
                self.progress = 0.0;
                self.taking = NEUTRAL;
            }
            return None;
        };
        // Two take it half again as fast as one, three twice as fast.
        let rate = (1.0 + 0.5 * (present[team as usize] as f32 - 1.0)).min(2.5) / CAPTURE_SECONDS;
        if self.owner == team || (self.taking != team && self.taking != NEUTRAL && self.progress > 0.0) {
            // Ours already (wind back any enemy progress), or undo theirs before starting ours.
            self.progress -= dt * rate;
            if self.progress <= 1e-4 {
                self.progress = 0.0;
                self.taking = if self.owner == team { NEUTRAL } else { team };
            }
            return None;
        }
        self.taking = team;
        self.progress += dt * rate;
        if self.progress >= 1.0 {
            self.owner = team;
            self.taking = NEUTRAL;
            self.progress = 0.0;
            return Some(team);
        }
        None
    }
}

/// Search and destroy: is `feet` inside a bomb site?
pub fn in_site(site: Vec3, feet: Vec3) -> bool {
    feet.xz().distance(site.xz()) < SITE_RADIUS && (feet.y - site.y).abs() < 2.6
}

/// The objective state of the match, sent to every client with the rest of [`MatchState`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Objective {
    /// See [`kind`].
    pub kind: u8,
    /// Domination: A, B, C. Search and destroy: sites A and B.
    pub points: Vec<Point>,
    /// Domination: seconds towards the next payout.
    pub tally: f32,
    /// Search and destroy: 1-based round number (0 before the first).
    pub round: u8,
    /// The attacking team this round.
    pub attackers: u8,
    /// See [`stage`].
    pub stage: u8,
    /// Seconds left in the round while playing; until the next round once it is over.
    pub clock: f32,
    /// See [`bomb`].
    pub bomb: u8,
    /// Where the bomb is (its carrier's feet while carried).
    pub bomb_at: Vec3,
    /// Who carries it - or, once planted, who planted it.
    pub carrier: u8,
    /// Seconds until a planted bomb goes off.
    pub fuse: f32,
    /// The site it is planted at; before that, the site the attacking bots are heading for.
    pub site: u8,
    /// Who is planting or defusing, and how far along (0-1).
    pub actor: u8,
    pub progress: f32,
    /// Players still standing on each team.
    pub alive: [u8; 2],
    /// Last round's winner and why, for the HUD.
    pub last_winner: u8,
    pub last_reason: String,
}

/// Someone on the field, as the objective sees them.
#[derive(Clone, Copy, Debug)]
pub struct Agent {
    pub id: u8,
    pub team: u8,
    pub at: Vec3,
    pub alive: bool,
    /// Holding Use.
    pub using: bool,
}

/// What happened to the bomb this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BombEvent {
    PickedUp(u8),
    Dropped,
    Planted { site: u8, by: u8 },
    Defused { by: u8 },
    Exploded,
}

impl Objective {
    pub fn defenders(&self) -> u8 {
        1 - self.attackers.min(1)
    }

    /// One frame of the bomb: pick-ups, drops, planting, defusing and the fuse.
    pub fn tick_bomb(&mut self, agents: &[Agent], dt: f32) -> Vec<BombEvent> {
        let mut out = Vec::new();
        let attackers = self.attackers;
        match self.bomb {
            bomb::HOME | bomb::DROPPED => {
                if let Some(a) = agents.iter().find(|a| a.alive && a.team == attackers && a.at.distance(self.bomb_at) < PICKUP_REACH) {
                    self.bomb = bomb::CARRIED;
                    self.carrier = a.id;
                    self.bomb_at = a.at;
                    out.push(BombEvent::PickedUp(a.id));
                }
            }
            bomb::CARRIED => match agents.iter().find(|a| a.id == self.carrier) {
                Some(carrier) if carrier.alive && carrier.team == attackers => {
                    self.bomb_at = carrier.at;
                    let site = self.points.iter().position(|s| in_site(s.at, carrier.at));
                    match site.filter(|_| carrier.using) {
                        Some(site) => {
                            if self.actor != carrier.id {
                                self.actor = carrier.id;
                                self.progress = 0.0;
                            }
                            self.progress += dt / PLANT_SECONDS;
                            if self.progress >= 1.0 {
                                self.bomb = bomb::PLANTED;
                                self.site = site as u8;
                                self.fuse = FUSE_SECONDS;
                                self.progress = 0.0;
                                self.actor = NOBODY;
                                out.push(BombEvent::Planted { site: site as u8, by: carrier.id });
                            }
                        }
                        None => {
                            self.progress = 0.0;
                            self.actor = NOBODY;
                        }
                    }
                }
                gone => {
                    if let Some(body) = gone {
                        self.bomb_at = body.at;
                    }
                    self.bomb = bomb::DROPPED;
                    self.carrier = NOBODY;
                    self.progress = 0.0;
                    self.actor = NOBODY;
                    out.push(BombEvent::Dropped);
                }
            },
            bomb::PLANTED => {
                self.fuse -= dt;
                // Whoever started defusing keeps at it; anyone else waits their turn.
                let actor = self.actor;
                let defuser = agents.iter().filter(|a| a.alive && a.team != attackers && a.using && a.at.distance(self.bomb_at) < DEFUSE_REACH).min_by_key(|a| (a.id != actor) as u8);
                match defuser {
                    Some(d) => {
                        if d.id != self.actor {
                            self.actor = d.id;
                            self.progress = 0.0;
                        }
                        self.progress += dt / DEFUSE_SECONDS;
                        if self.progress >= 1.0 {
                            self.bomb = bomb::DEFUSED;
                            self.progress = 0.0;
                            self.actor = NOBODY;
                            out.push(BombEvent::Defused { by: d.id });
                        }
                    }
                    None => {
                        self.progress = 0.0;
                        self.actor = NOBODY;
                    }
                }
                if self.bomb == bomb::PLANTED && self.fuse <= 0.0 {
                    self.fuse = 0.0;
                    self.bomb = bomb::EXPLODED;
                    out.push(BombEvent::Exploded);
                }
            }
            _ => {}
        }
        out
    }

    /// Who has won the round, and how, given how many of each team are standing.
    pub fn round_winner(&self, alive: [usize; 2]) -> Option<(u8, &'static str)> {
        let (att, def) = (self.attackers, self.defenders());
        if self.bomb == bomb::EXPLODED {
            Some((att, "TARGET DESTROYED"))
        } else if self.bomb == bomb::DEFUSED {
            Some((def, "BOMB DEFUSED"))
        } else if alive[def as usize] == 0 {
            Some((att, "DEFENDERS ELIMINATED"))
        } else if alive[att as usize] == 0 && self.bomb != bomb::PLANTED {
            Some((def, "ATTACKERS ELIMINATED"))
        } else if self.bomb != bomb::PLANTED && self.clock <= 0.0 {
            Some((def, "TIME EXPIRED"))
        } else {
            None
        }
    }
}

/// Run condition: a versus match with an objective.
pub fn objective_mode(rules: Option<Res<Rules>>) -> bool {
    rules.is_some_and(|r| r.pvp && (r.domination || r.bomb))
}

/// The two ends of the map: each team's base (or the map's spawn, pushed apart).
fn ends(map: &GameMap) -> [Vec2; 2] {
    let fallback = [map.spawn + Vec2::new(-map.arena * 0.6, 0.0), map.spawn + Vec2::new(map.arena * 0.6, 0.0)];
    [base_centre(map, 0).unwrap_or(fallback[0]), base_centre(map, 1).unwrap_or(fallback[1])]
}

/// Somewhere to stand a team's first spawn, for reachability checks.
fn base_foot(map: &GameMap, team: u8) -> Vec3 {
    let p = map.team_spawns[team as usize].first().copied().unwrap_or(ends(map)[team as usize]);
    map.ground(p.x, p.y)
}

/// The best open, reachable spot near `want`: clear of both bases (unless `in_base`),
/// at least 12 m from anything in `taken`, walkable, and (with a nav grid) reachable
/// from both bases. Indoor maps choose among their waypoints.
fn spot(map: &GameMap, statics: &StaticWorld, nav: Option<&Nav>, scratch: &mut NavScratch, want: Vec2, taken: &[Vec3], in_base: bool) -> Vec3 {
    let bases = ends(map);
    let clear = |p: Vec2| in_base || bases.iter().all(|c| p.distance(*c) > BASE_RADIUS * 1.5 + POINT_RADIUS + 2.0);
    let apart = |p: Vec3| taken.iter().all(|t| t.xz().distance(p.xz()) > 12.0);
    let mut candidates: Vec<(f32, Vec3)> = Vec::new();
    if map.waypoints.is_empty() {
        for ring in 0..26 {
            let radius = ring as f32 * 2.0;
            let count = (ring * 7).max(1);
            for k in 0..count {
                let angle = k as f32 / count as f32 * std::f32::consts::TAU + ring as f32 * 0.37;
                let (x, z) = (want.x + angle.cos() * radius, want.y + angle.sin() * radius);
                if !map.in_bounds(x, z) {
                    continue;
                }
                let y = map.height(x, z);
                if y < map.water + 0.05 || statics.blocked(x, z, y + 0.05, 1.9, 0.6) {
                    continue;
                }
                // Prefer open ground under the sky over a cramped room.
                let open = !statics.blocked(x, z, y + 0.05, 2.4, 2.8);
                let roofed = statics.raycast(Vec3::new(x, y + 0.6, z), Vec3::Y, 30.0).is_some();
                candidates.push((radius + if open { 0.0 } else { 10.0 } + if roofed { 8.0 } else { 0.0 }, Vec3::new(x, y, z)));
            }
        }
    } else {
        for w in &map.waypoints {
            let climb = (w.y - map.height(w.x, w.z)).max(0.0);
            candidates.push((w.xz().distance(want) + climb * 1.5, *w));
        }
    }
    candidates.retain(|(_, p)| clear(p.xz()) && apart(*p) && nav.is_none_or(|nav| nav.walkable(*p)));
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
    let froms = [base_foot(map, 0), base_foot(map, 1)];
    for (_, p) in candidates.iter().take(10) {
        let Some(nav) = nav else { return *p };
        if froms.iter().all(|from| nav.path(scratch, *from, *p).is_some()) {
            return *p;
        }
    }
    candidates.first().map_or_else(|| map.ground(want.x, want.y), |(_, p)| *p)
}

/// Domination's three points: A on team 0's side, B in the middle, C on team 1's side.
pub fn domination_points(map: &GameMap, statics: &StaticWorld, nav: Option<&Nav>, scratch: &mut NavScratch) -> Vec<Vec3> {
    let [a, b] = ends(map);
    let mut out: Vec<Vec3> = Vec::new();
    // The middle first, so the outer two keep their distance from it.
    for t in [0.5, 0.24, 0.76] {
        let at = spot(map, statics, nav, scratch, a.lerp(b, t), &out, false);
        out.push(at);
    }
    vec![out[1], out[0], out[2]]
}

/// Search and destroy's two sites, A and B: side by side in front of the defenders' base.
pub fn bomb_sites(map: &GameMap, statics: &StaticWorld, nav: Option<&Nav>, scratch: &mut NavScratch, defenders: u8) -> Vec<Vec3> {
    let ends = ends(map);
    let (from, to) = (ends[1 - defenders.min(1) as usize], ends[defenders.min(1) as usize]);
    let axis = to - from;
    let middle = from + axis * 0.68;
    let side = axis.perp().normalize_or(Vec2::Y) * (axis.length() * 0.24).clamp(12.0, 32.0);
    let mut out: Vec<Vec3> = Vec::new();
    for want in [middle + side, middle - side] {
        let at = spot(map, statics, nav, scratch, want, &out, false);
        out.push(at);
    }
    out
}

/// Where the bomb waits at the start of a round: the middle of the attackers' base.
pub fn bomb_home(map: &GameMap, statics: &StaticWorld, nav: Option<&Nav>, scratch: &mut NavScratch, attackers: u8) -> Vec3 {
    let centre = ends(map)[attackers.min(1) as usize];
    let at = spot(map, statics, nav, scratch, centre, &[], true);
    if at.xz().distance(centre) < BASE_RADIUS * 1.5 { at } else { base_foot(map, attackers) }
}

/// Domination respawns: by a point your team holds when no enemy is near it, else the
/// safest spot in your base.
pub fn domination_spawn(map: &GameMap, statics: &StaticWorld, team: u8, id: u8, points: &[Point], enemies: &[Vec3]) -> Vec3 {
    let nearest = |at: Vec3| enemies.iter().map(|e| e.distance(at)).fold(f32::MAX, f32::min);
    let home = safest_spawn(map, team, enemies);
    let mut best = (nearest(home).min(60.0), home);
    for point in points.iter().filter(|p| p.owner == team) {
        for k in 0..6 {
            let angle = id as f32 * 2.4 + k as f32 * 1.05;
            let (x, z) = (point.at.x + angle.cos() * 3.5, point.at.z + angle.sin() * 3.5);
            if !map.in_bounds(x, z) || statics.blocked(x, z, point.at.y + 0.05, 1.8, 0.45) {
                continue;
            }
            let y = ground_under(map, statics, x, z, point.at.y + 0.6);
            if (y - point.at.y).abs() > 0.7 {
                continue;
            }
            let at = Vec3::new(x, y, z);
            let threat = nearest(at);
            if threat < 26.0 {
                continue;
            }
            // A held point beats the base, as long as it is safe.
            let score = threat.min(60.0) + 30.0;
            if score > best.0 {
                best = (score, at);
            }
            break;
        }
    }
    best.1
}

/// Where the points and sites go on the current map, worked out once per match.
#[derive(Default)]
pub struct Placed {
    key: u64,
    domination: Vec<Vec3>,
    /// Bomb sites by defending team, and the bomb's home by attacking team.
    sites: [Vec<Vec3>; 2],
    home: [Vec3; 2],
}

/// Authority: runs domination and search and destroy.
#[allow(clippy::too_many_arguments)]
pub fn run_objectives(
    time: Res<Time>,
    rules: Res<Rules>,
    content: Res<Content>,
    (map, statics, nav): (Res<Map>, Res<Statics>, Option<Res<Nav>>),
    mut scratch: ResMut<NavScratch>,
    mut state: ResMut<MatchState>,
    mut rng: ResMut<SimRng>,
    mut players: Query<(&mut Player, &mut Health, &mut Transform, &mut Arsenal, &mut PlayerInput)>,
    mut fx: EventWriter<FxEvent>,
    mut placed: Local<Placed>,
) {
    let dt = sim_dt(&time);
    let state = &mut *state;
    let key = std::sync::Arc::as_ptr(&map.0) as u64 ^ map.recipe.seed;
    if placed.key != key {
        let nav = nav.as_deref();
        placed.key = key;
        placed.domination = if rules.domination { domination_points(&map, &statics, nav, &mut scratch) } else { Vec::new() };
        if rules.bomb {
            placed.sites = [bomb_sites(&map, &statics, nav, &mut scratch, 0), bomb_sites(&map, &statics, nav, &mut scratch, 1)];
            placed.home = [bomb_home(&map, &statics, nav, &mut scratch, 0), bomb_home(&map, &statics, nav, &mut scratch, 1)];
        }
    }
    let agents: Vec<Agent> = players
        .iter()
        .map(|(p, h, t, _, input)| Agent { id: p.id, team: p.team.min(1), at: t.translation, alive: h.alive(), using: input.interact })
        .collect();
    for team in 0..2 {
        state.obj.alive[team] = agents.iter().filter(|a| a.alive && a.team as usize == team).count() as u8;
    }

    if rules.domination {
        state.obj.kind = kind::DOMINATION;
        match state.phase {
            Phase::Lobby => {
                state.obj.points = placed.domination.iter().map(|at| Point::new(*at)).collect();
                state.obj.tally = 0.0;
            }
            Phase::Live => {
                let mut taken = Vec::new();
                for (index, point) in state.obj.points.iter_mut().enumerate() {
                    let mut present = [0; 2];
                    for a in agents.iter().filter(|a| a.alive && point.holds(a.at)) {
                        present[a.team as usize] += 1;
                    }
                    if let Some(team) = point.tick(present, dt) {
                        taken.push((index, team));
                    }
                }
                for (index, team) in taken {
                    fx.write(FxEvent::notice(format!("TEAM {} TOOK {}", TEAM_NAMES[team as usize], LETTERS[index.min(2)])));
                    fx.write(FxEvent::Sound { sfx: Sfx::Streak, at: None });
                }
                state.obj.tally += dt;
                while state.obj.tally >= SCORE_EVERY {
                    state.obj.tally -= SCORE_EVERY;
                    for point in &state.obj.points {
                        if point.owner < 2 {
                            let score = &mut state.scores[point.owner as usize];
                            *score = (*score + 1).min(rules.score_limit.max(1));
                        }
                    }
                }
            }
            _ => {}
        }
        return;
    }

    // Search and destroy.
    state.obj.kind = kind::BOMB;
    match state.phase {
        Phase::Lobby => {
            let defenders = 1;
            state.obj = Objective { kind: kind::BOMB, attackers: 0, points: placed.sites[defenders].iter().map(|at| Point::new(*at)).collect(), bomb_at: placed.home[0], carrier: NOBODY, actor: NOBODY, alive: state.obj.alive, ..default() };
        }
        Phase::Live => {
            let begin = state.obj.round == 0 || (state.obj.stage == stage::OVER && state.obj.clock <= 0.0);
            if begin {
                // Half time: swap sides once half the rounds a match can take have been played.
                let played = state.scores[0] + state.scores[1];
                let mut attackers = state.obj.attackers;
                if state.obj.round > 0 && played == rules.score_limit.saturating_sub(1).max(1) {
                    attackers = 1 - attackers.min(1);
                    fx.write(FxEvent::notice("HALF TIME  -  SWITCHING SIDES"));
                }
                let defenders = 1 - attackers as usize;
                let round = state.obj.round + 1;
                state.obj = Objective {
                    kind: kind::BOMB,
                    round,
                    attackers,
                    stage: stage::PLAYING,
                    clock: ROUND_SECONDS,
                    points: placed.sites[defenders].iter().map(|at| Point::new(*at)).collect(),
                    bomb: bomb::HOME,
                    bomb_at: placed.home[attackers as usize],
                    carrier: NOBODY,
                    actor: NOBODY,
                    site: rng.0.below(2) as u8,
                    alive: state.obj.alive,
                    last_winner: state.obj.last_winner,
                    ..default()
                };
                let snapshot = state.clone();
                for (mut player, mut health, mut transform, mut arsenal, mut input) in &mut players {
                    respawn(&content, &map, &rules, &snapshot, &mut player, &mut health, &mut transform, &mut arsenal, &mut input);
                    player.protect = 1.5;
                    player.streak = 0;
                    let line = if player.team == attackers { "ATTACK  -  PLANT THE BOMB AT A OR B" } else { "DEFEND  -  PROTECT SITES A AND B" };
                    fx.write(FxEvent::tell(player.id, format!("ROUND {round}  -  {line}")));
                }
                state.obj.alive = [0, 1].map(|team| agents.iter().filter(|a| a.team == team).count() as u8);
                fx.write(FxEvent::Sound { sfx: Sfx::Horn, at: None });
                return;
            }
            if state.obj.stage == stage::OVER {
                state.obj.clock -= dt;
                return;
            }
            let planted_before = state.obj.bomb == bomb::PLANTED;
            if !planted_before {
                state.obj.clock = (state.obj.clock - dt).max(0.0);
            }
            let beep_before = state.obj.fuse;
            let events = state.obj.tick_bomb(&agents, dt);
            let name = |id: u8| agents.iter().find(|a| a.id == id).and_then(|a| state.roster.iter().find(|r| r.id == a.id)).map_or("SOMEONE".to_string(), |r| r.name.clone());
            let mut blast = false;
            for event in &events {
                match *event {
                    BombEvent::PickedUp(id) => fx.write(FxEvent::tell(id, "YOU HAVE THE BOMB  -  HOLD USE AT A SITE TO PLANT")),
                    BombEvent::Dropped => fx.write(FxEvent::notice("THE BOMB HAS BEEN DROPPED")),
                    BombEvent::Planted { site, by } => {
                        fx.write(FxEvent::Sound { sfx: Sfx::Alarm, at: None });
                        fx.write(FxEvent::notice(format!("{} PLANTED THE BOMB AT {}", name(by), LETTERS[site.min(2) as usize])))
                    }
                    BombEvent::Defused { by } => {
                        fx.write(FxEvent::Sound { sfx: Sfx::UiAccept, at: None });
                        fx.write(FxEvent::notice(format!("{} DEFUSED THE BOMB", name(by))))
                    }
                    BombEvent::Exploded => {
                        blast = true;
                        fx.write(FxEvent::Explosion { at: state.obj.bomb_at + Vec3::Y * 0.5, radius: 10.0 })
                    }
                };
            }
            // A planted bomb ticks, faster as it runs down.
            if state.obj.bomb == bomb::PLANTED {
                let every = if state.obj.fuse < 5.0 { 0.25 } else if state.obj.fuse < 15.0 { 0.5 } else { 1.0 };
                if (beep_before / every).ceil() != (state.obj.fuse / every).ceil() {
                    fx.write(FxEvent::Sound { sfx: Sfx::UiMove, at: Some(state.obj.bomb_at) });
                }
            }
            if blast {
                let at = state.obj.bomb_at;
                let planter = state.obj.carrier;
                let by = state.roster.iter().find(|r| r.id == planter).map(|r| (r.name.clone(), r.team, r.id));
                fx.write(FxEvent::Sound { sfx: Sfx::Explosion, at: Some(at) });
                for (mut player, mut health, transform, ..) in &mut players {
                    let distance = transform.translation.distance(at);
                    if distance < BLAST_RADIUS && health.alive() {
                        player.protect = 0.0;
                        player.last_hit = part::BLAST;
                        let damage = 600.0 * (1.0 - distance / BLAST_RADIUS) + 40.0;
                        damage_player(&mut health, &mut player, damage, by.as_ref().map(|(n, t, i)| (n.as_str(), *t, *i)), state, &mut fx);
                    }
                }
            }
            let alive = [0, 1].map(|team| players.iter().filter(|(p, h, ..)| h.alive() && p.team.min(1) == team).count());
            if let Some((winner, reason)) = state.obj.round_winner(alive) {
                state.obj.stage = stage::OVER;
                state.obj.clock = ROUND_BREAK;
                state.obj.progress = 0.0;
                state.obj.actor = NOBODY;
                state.obj.last_winner = winner;
                state.obj.last_reason = reason.to_string();
                state.scores[winner as usize] += 1;
                fx.write(FxEvent::notice(format!("{reason}  -  TEAM {} WINS THE ROUND", TEAM_NAMES[winner as usize])));
                fx.write(FxEvent::Sound { sfx: Sfx::Dawn, at: None });
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(id: u8, team: u8, at: Vec3, using: bool) -> Agent {
        Agent { id, team, at, alive: true, using }
    }

    #[test]
    fn one_player_takes_a_point_in_capture_time_and_friends_are_faster() {
        let mut point = Point::new(Vec3::ZERO);
        let mut seconds = 0.0;
        while point.tick([1, 0], 0.1).is_none() {
            seconds += 0.1;
            assert!(seconds < 60.0);
        }
        assert_eq!(point.owner, 0);
        assert!((seconds - CAPTURE_SECONDS).abs() < 0.3, "took {seconds}");
        // Three of team 1 take it back twice as fast as one would.
        let mut seconds = 0.0;
        while point.tick([0, 3], 0.1).is_none() {
            seconds += 0.1;
            assert!(seconds < 60.0);
        }
        assert_eq!(point.owner, 1);
        assert!(seconds < CAPTURE_SECONDS * 0.6, "took {seconds}");
    }

    #[test]
    fn a_contested_point_freezes_and_an_owner_winds_back_enemy_progress() {
        let mut point = Point::new(Vec3::ZERO);
        for _ in 0..30 {
            point.tick([1, 0], 0.1);
        }
        let halfway = point.progress;
        assert!(halfway > 0.3 && point.taking == 0);
        for _ in 0..50 {
            assert_eq!(point.tick([2, 1], 0.1), None);
        }
        assert!(point.contested);
        assert_eq!(point.progress, halfway, "nothing moves while both teams stand in it");
        // Team 1 alone: first undo team 0's progress, then start its own.
        for _ in 0..30 {
            point.tick([0, 1], 0.1);
        }
        assert_eq!(point.owner, NEUTRAL);
        assert!(point.taking == 1 && point.progress < 0.1, "{point:?}");
        assert!(point.holds(Vec3::new(3.0, 0.5, 3.0)) && !point.holds(Vec3::new(5.0, 0.0, 5.0)));
    }

    fn round() -> Objective {
        Objective {
            kind: kind::BOMB,
            attackers: 0,
            stage: stage::PLAYING,
            clock: ROUND_SECONDS,
            points: vec![Point::new(Vec3::new(40.0, 0.0, 0.0)), Point::new(Vec3::new(-40.0, 0.0, 0.0))],
            bomb: bomb::HOME,
            bomb_at: Vec3::new(0.0, 0.0, 50.0),
            carrier: NOBODY,
            actor: NOBODY,
            ..default()
        }
    }

    #[test]
    fn the_bomb_is_carried_dropped_planted_and_goes_off() {
        let mut obj = round();
        // Defenders cannot pick it up; attackers can.
        assert!(obj.tick_bomb(&[agent(7, 1, obj.bomb_at, false)], 0.1).is_empty());
        assert_eq!(obj.tick_bomb(&[agent(3, 0, obj.bomb_at + Vec3::X, false)], 0.1), vec![BombEvent::PickedUp(3)]);
        // Carried to site A; holding Use outside a site does nothing.
        let site_a = obj.points[0].at;
        obj.tick_bomb(&[agent(3, 0, Vec3::new(10.0, 0.0, 10.0), true)], 1.0);
        assert_eq!((obj.progress, obj.bomb), (0.0, bomb::CARRIED));
        // Letting go part way restarts the plant.
        obj.tick_bomb(&[agent(3, 0, site_a, true)], 2.0);
        assert!((obj.progress - 0.5).abs() < 1e-4);
        obj.tick_bomb(&[agent(3, 0, site_a, false)], 0.1);
        assert_eq!(obj.progress, 0.0);
        // The carrier dies: the bomb stays where they fell, and a teammate picks it up.
        let mut dead = agent(3, 0, site_a + Vec3::X, false);
        dead.alive = false;
        assert_eq!(obj.tick_bomb(&[dead], 0.1), vec![BombEvent::Dropped]);
        assert_eq!(obj.bomb_at, site_a + Vec3::X);
        assert_eq!(obj.tick_bomb(&[dead, agent(4, 0, site_a, false)], 0.1), vec![BombEvent::PickedUp(4)]);
        let mut events = Vec::new();
        for _ in 0..41 {
            events.extend(obj.tick_bomb(&[agent(4, 0, site_a, true)], 0.1));
        }
        assert_eq!(events, vec![BombEvent::Planted { site: 0, by: 4 }]);
        assert_eq!((obj.bomb, obj.carrier, obj.fuse), (bomb::PLANTED, 4, FUSE_SECONDS));
        // The planter can die now: a planted bomb still wins unless it is defused.
        assert_eq!(obj.round_winner([0, 2]), None);
        let mut events = Vec::new();
        for _ in 0..41 {
            events.extend(obj.tick_bomb(&[], 1.0));
        }
        assert_eq!(events, vec![BombEvent::Exploded]);
        assert_eq!(obj.round_winner([0, 2]), Some((0, "TARGET DESTROYED")));
    }

    #[test]
    fn defenders_defuse_and_win_on_time_or_elimination() {
        let mut obj = round();
        obj.bomb = bomb::PLANTED;
        obj.fuse = FUSE_SECONDS;
        obj.bomb_at = obj.points[1].at;
        let near = obj.bomb_at + Vec3::new(1.0, 0.0, 0.5);
        // Attackers cannot defuse; a defender must hold Use for the whole defuse time.
        obj.tick_bomb(&[agent(2, 0, near, true)], 1.0);
        assert_eq!(obj.progress, 0.0);
        let mut events = Vec::new();
        let mut seconds = 0.0;
        while events.is_empty() {
            events = obj.tick_bomb(&[agent(9, 1, near, true), agent(10, 1, near, true)], 0.1);
            seconds += 0.1;
            assert!(seconds < 10.0);
        }
        assert_eq!(events, vec![BombEvent::Defused { by: 9 }]);
        assert!((seconds - DEFUSE_SECONDS).abs() < 0.3);
        assert_eq!(obj.round_winner([3, 1]), Some((1, "BOMB DEFUSED")));

        let mut obj = round();
        assert_eq!(obj.round_winner([2, 2]), None);
        assert_eq!(obj.round_winner([0, 2]), Some((1, "ATTACKERS ELIMINATED")));
        assert_eq!(obj.round_winner([2, 0]), Some((0, "DEFENDERS ELIMINATED")));
        obj.clock = 0.0;
        assert_eq!(obj.round_winner([2, 2]), Some((1, "TIME EXPIRED")));
    }

    #[test]
    fn points_and_sites_sit_on_reachable_ground_outside_the_bases() {
        use oz_core::worldgen::{Biome, Layout, Weather, WorldRecipe};
        let mut recipes = vec![WorldRecipe::town(7, Biome::IronShoals), WorldRecipe::town(21, Biome::IronShoals)];
        for layout in Layout::VERSUS.iter().filter(|l| **l != Layout::Town) {
            recipes.push(WorldRecipe::preset(*layout, Biome::IronShoals, Weather::Clear));
        }
        for recipe in recipes {
            let map = GameMap::generate(recipe);
            let mut statics = StaticWorld::new(recipe.half_extent);
            for aabb in map.solids() {
                statics.add(aabb);
            }
            let nav = Nav::build(&map, &statics);
            let mut scratch = NavScratch::default();
            let points = domination_points(&map, &statics, Some(&nav), &mut scratch);
            let mut all = points.clone();
            for defenders in 0..2 {
                all.extend(bomb_sites(&map, &statics, Some(&nav), &mut scratch, defenders));
            }
            let [a, b] = ends(&map);
            for (i, p) in all.iter().enumerate() {
                assert!(nav.walkable(*p), "{:?}: point {i} at {p} is not walkable", recipe.layout);
                for team in 0..2 {
                    assert!(nav.path(&mut scratch, base_foot(&map, team), *p).is_some(), "{:?}: point {i} at {p} unreachable from base {team}", recipe.layout);
                    assert!(!crate::player::in_enemy_base(&map, team, *p), "{:?}: point {i} at {p} is in a base", recipe.layout);
                }
            }
            // A by team 0, C by team 1, B between.
            assert!(points[0].xz().distance(a) < points[2].xz().distance(a), "{:?}: {points:?}", recipe.layout);
            assert!(points[2].xz().distance(b) < points[0].xz().distance(b), "{:?}: {points:?}", recipe.layout);
            println!("{:?}: points {points:?}", recipe.layout);
        }
    }
}
