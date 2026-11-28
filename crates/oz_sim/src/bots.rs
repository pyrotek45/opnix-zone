//! AI-controlled players.
//!
//! A bot is an ordinary player entity plus a [`Bot`] brain. The brain only
//! ever writes [`PlayerInput`] - the same struct a keyboard or a remote client
//! fills in - so bots move, crouch, shoot, reload and die through exactly the
//! same code as humans. They travel with the [`Nav`] pathfinder and fight
//! with ranges that suit the weapon in their hands.

use crate::{
    combat::Solid,
    nav::{Nav, NavBudget, NavScratch},
    net::NetIds,
    player::{Arsenal, Health, Loadout, Player, PlayerInput, apply_loadout, buttons, stance},
    session::{Content, Map, Statics, player_bundle},
    sim_dt,
    objective::Objective,
    state::{CommandEvent, FxEvent, MatchState, Phase, PlayerCommand, Rules, SimRng},
    structures::Structure,
    weapons::obstruction,
};
use bevy::prelude::*;
use oz_core::{
    arsenal::{GunClass, SLOT_COUNT, options, slot},
    collide::StaticWorld,
};
use std::f32::consts::{PI, TAU};

/// What a versus bot is up to when it has no one in its sights.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Plan {
    /// Head for where the enemy was last seen or heard.
    #[default]
    Hunt,
    /// No idea where they are: sweep the likely places.
    Search,
    /// Hold a spot, crouched, watching the way they will come (`face` is the yaw to watch).
    Camp { face: f32 },
    /// Come at a known enemy from the side instead of head on.
    Flank,
    /// Stay with the squad leader, in a loose formation.
    Follow,
    /// Hurt: fall back toward friends before fighting again.
    Retreat,
}

/// A bot's habits in a group.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Role {
    /// Pushes straight at contacts.
    #[default]
    Rusher,
    /// Holds angles and watches the squad's back.
    Anchor,
    /// Swings wide round contacts.
    Flanker,
}

#[derive(Component, Default)]
pub struct Bot {
    /// Last shot at or hit by someone it could not see: where from, and when.
    threat: Option<(Vec3, f32)>,
    pub plan: Plan,
    pub role: Role,
    /// When the current plan runs out (match seconds).
    plan_until: f32,
    think: f32,
    target: Option<Entity>,
    /// Where the bot wants to go, and the path it is following there.
    goal: Vec3,
    path: Vec<Vec3>,
    repath: f32,
    stuck: f32,
    last: Vec3,
    /// Where the bot last made real headway on its way somewhere, and how long ago: shuffling
    /// on the spot (forward, aside, forward again) is no headway.
    anchor: Vec3,
    anchor_time: f32,
    strafe: f32,
    strafe_time: f32,
    aim_error: Vec2,
    /// Seconds the current target has been in sight (reaction time).
    seen: f32,
    crouch_at_range: bool,
    /// Which way to lean to see the current target around cover.
    lean: i8,
    /// Terrorist hunt: a sentry holds a post, sees only what is in front of it, and
    /// reacts to what it hears instead of always knowing where everyone is.
    pub sentry: bool,
    pub post: Vec3,
    /// Seconds of high alert left (wider view, further sight, running).
    alert: f32,
    /// Where the last noise worth checking came from.
    heard: Option<Vec3>,
    /// Firing back blind at a gunshot it heard: how long, and at what.
    suppress: f32,
    suppress_at: Vec3,
    patrol: f32,
    last_hp: f32,
    /// Objective modes: the point it is going for, and whether to hold it (defend)
    /// rather than take it.
    pub objective: Option<(u8, bool)>,
    /// Holding Use (planting or defusing).
    pub hold_use: bool,
    /// Cover for the fight in hand, re-chosen whenever `cover_time` runs out.
    cover: Option<Cover>,
    cover_time: f32,
    /// Peek cycle at cover: out shooting, or tucked in, for `peek` more seconds.
    exposed: bool,
    peek: f32,
    /// Where the target last stood (and how), and until when to keep fighting from cover
    /// after it ducks out of sight.
    foe: Vec3,
    foe_stance: u8,
    engaged_until: f32,
    /// A camp spot moved into cover: (spot the plan asked for, spot taken, crouch there).
    camp_spot: Option<(Vec3, Vec3, bool)>,
    /// In a narrow passage (doorway, alley, gap); `glance` sweeps the view across its corners.
    choke: bool,
    glance: f32,
    /// When it last came under fire (a hit, or a round cracking past), and who fired.
    shot_at: Option<f32>,
    shooter: Option<u8>,
    /// Death memory: where it went down lately, where the killer stood, and when.
    deaths: Vec<(Vec3, Vec3, f32)>,
    dead_noted: bool,
    /// A way round a spot it keeps dying to: the waypoint, the goal behind it, and until
    /// when detours are off (it has just taken one).
    detour: Option<(Vec3, Vec3)>,
    detour_off: f32,
    /// Seconds with no headway, no fight and nothing to hold on purpose: doing nothing.
    idle: f32,
    idle_from: Vec3,
    /// Times it has had to shake itself out of doing nothing (shifts where it holds).
    shifts: u8,
    /// Throwing gear: seconds left holding the button, where at, and the wait before another.
    throw: f32,
    throw_at: Vec3,
    gear_wait: f32,
    /// Loot it is going for: where, and whether it means swapping the gun in hand for it.
    loot: Option<(Vec3, bool)>,
    swap_wait: f32,
    /// Seconds spent tucked in under fire from someone it cannot see: past a few, it stops
    /// hiding and goes after them.
    pinned: f32,
    /// Loot it could not get to (a roof with no way up, say): where, and until when to leave it.
    loot_skip: Vec<(Vec3, f32)>,
    /// When it started going for the loot it is after.
    loot_since: f32,
    /// A better gun close enough to grab in the middle of a fight.
    grab: Option<Vec3>,
}

/// What the bot fears from a spot it has died to more than once: (where the killers
/// stood, how many deaths).
impl Bot {
    pub fn danger(&self, now: f32) -> Option<(Vec3, usize)> {
        let recent: Vec<Vec3> = self.deaths.iter().filter(|(.., t)| now - t < 150.0).map(|(_, killer, _)| *killer).collect();
        recent
            .iter()
            .map(|k| {
                let near: Vec<Vec3> = recent.iter().copied().filter(|o| o.distance(*k) < 15.0).collect();
                (near.iter().fold(Vec3::ZERO, |a, b| a + *b) / near.len() as f32, near.len())
            })
            .filter(|(_, n)| *n >= 2)
            .max_by_key(|(_, n)| *n)
    }

    /// A one-line account of what the bot is up to, for debugging tests.
    pub fn describe(&self) -> String {
        format!(
            "plan {:?} goal {:.1} path {} target {} cover {} engaged_until {:.1} shot_at {:?} threat {:?} idle {:.1} loot {:?} detour {:?} hold_use {} stuck {:.1}",
            self.plan,
            self.goal,
            self.path.len(),
            self.target.is_some(),
            self.cover.is_some(),
            self.engaged_until,
            self.shot_at,
            self.threat.map(|(p, _)| p),
            self.idle,
            self.loot,
            self.detour,
            self.hold_use,
            self.stuck
        )
    }

    /// Under fire within the last `seconds`.
    pub fn under_fire(&self, now: f32, seconds: f32) -> bool {
        self.shot_at.is_some_and(|t| now - t < seconds)
    }

    /// The spot it believes it was last shot from, if recent.
    pub fn threat_at(&self, now: f32, seconds: f32) -> Option<Vec3> {
        self.threat.filter(|_| self.under_fire(now, seconds)).map(|(p, _)| p)
    }
}

/// Distance from `point` to the segment `a`-`b`, on the ground plane.
fn off_line(point: Vec3, a: Vec3, b: Vec3) -> f32 {
    let (p, a, b) = (point.xz(), a.xz(), b.xz());
    let t = ((p - a).dot(b - a) / (b - a).length_squared().max(1e-4)).clamp(0.0, 1.0);
    (a + (b - a) * t).distance(p)
}

/// Where to go on the way to `at` so as not to walk into a spot the bot keeps dying to:
/// out to one side of it, then in. `at` itself when nothing is in the way.
fn route_round(bot: &mut Bot, feet: Vec3, at: Vec3, now: f32, ground: impl Fn(f32, f32) -> Option<f32>) -> Vec3 {
    let Some((danger, deaths)) = bot.danger(now) else { return at };
    if now < bot.detour_off {
        return at;
    }
    if let Some((via, goal)) = bot.detour
        && goal.distance(at) < 6.0
    {
        if via.xz().distance(feet.xz()) < 4.0 {
            // Round the side: straight in from here, no more detours for a while.
            bot.detour = None;
            bot.detour_off = now + 20.0;
            return at;
        }
        return via;
    }
    // Only when the way there runs past the danger (or ends in it).
    if off_line(danger, feet, at) > 14.0 || danger.xz().distance(feet.xz()) < 8.0 {
        return at;
    }
    let side = if (deaths + bot.shifts as usize) % 2 == 0 { 1.0 } else { -1.0 };
    let across = (danger - feet).xz().normalize_or(Vec2::X).perp() * side * 22.0;
    let halfway = feet.lerp(danger, 0.6);
    let (x, z) = (halfway.x + across.x, halfway.z + across.y);
    let (x, z) = if ground(x, z).is_some() { (x, z) } else { (halfway.x - across.x, halfway.z - across.y) };
    let Some(y) = ground(x, z) else { return at };
    let via = Vec3::new(x, y, z);
    bot.detour = Some((via, at));
    via
}

/// How much use a gun of this class is in a fight (sidearm 1).
pub fn gun_value(class: GunClass) -> f32 {
    match class {
        GunClass::Pistol => 1.0,
        GunClass::Smg => 3.0,
        GunClass::Shotgun => 2.7,
        GunClass::Rifle => 3.6,
        GunClass::Lmg => 3.4,
        GunClass::Sniper => 3.1,
        GunClass::Launcher => 2.0,
        GunClass::Tool => 0.2,
    }
}

/// Does a carried gun have anything left to shoot?
fn has_rounds(def: &oz_core::arsenal::WeaponDef, weapon: &oz_core::arsenal::WeaponState) -> bool {
    weapon.ammo > 0 || weapon.reserve > 0 || def.infinite_reserve
}

/// The best gun carried that still has rounds.
fn best_gun(content: &Content, arsenal: &Arsenal) -> Option<u8> {
    (0..arsenal.weapons.len())
        .filter(|i| arsenal.owns(*i))
        .filter(|i| content.weapons.get(*i).is_some_and(|d| has_rounds(d, &arsenal.weapons[*i])))
        .max_by(|a, b| gun_value(content.weapons[*a].class).total_cmp(&gun_value(content.weapons[*b].class)))
        .map(|i| i as u8)
}

/// What a bot would most like to pick up of what lies about, and how much it wants it:
/// ammo when running dry, a better gun when holding a weak one, armour, gear. Returns
/// (where, value less the walk, whether it means swapping the gun in hand for it).
pub fn loot_wish(content: &Content, player: &Player, health: &Health, arsenal: &Arsenal, feet: Vec3, pickups: &[(Vec3, crate::loot::Loot)], reach: f32) -> Option<(Vec3, f32, bool)> {
    use crate::loot::Loot;
    let owned: Vec<usize> = (0..arsenal.weapons.len()).filter(|i| arsenal.owns(*i) && *i < content.weapons.len()).collect();
    let value_of = |i: usize| if has_rounds(&content.weapons[i], &arsenal.weapons[i]) { gun_value(content.weapons[i].class) } else { 0.3 };
    let best = owned.iter().map(|i| value_of(*i)).fold(0.0f32, f32::max);
    let worst = owned.iter().map(|i| value_of(*i)).fold(f32::MAX, f32::min);
    let cap = |i: usize| content.weapons[i].reserve.max(content.weapons[i].mag * 4);
    let finite: Vec<usize> = owned.iter().copied().filter(|i| !content.weapons[*i].infinite_reserve && content.weapons[*i].mag > 0).collect();
    let dry = !finite.is_empty() && finite.iter().all(|i| arsenal.weapons[*i].reserve < content.weapons[*i].mag);
    let low = crate::loot::low_on_ammo(content, arsenal);
    let roomy = finite.iter().any(|i| arsenal.weapons[*i].reserve < cap(*i) / 2);
    pickups
        .iter()
        .filter_map(|(at, loot)| {
            let distance = at.distance(feet);
            if distance > reach {
                return None;
            }
            let (value, swap) = match *loot {
                Loot::Ammo if dry => (4.0, false),
                Loot::Ammo if low => (2.6, false),
                Loot::Ammo if roomy => (0.8, false),
                Loot::Weapon { index, ammo, reserve, .. } => {
                    let i = index as usize;
                    let def = content.weapons.get(i)?;
                    if owned.contains(&i) {
                        let short = arsenal.weapons[i].reserve < cap(i) / 2;
                        if short && ammo + reserve > 0 { (if low { 2.4 } else { 0.7 }, false) } else { return None }
                    } else if ammo + reserve == 0 {
                        return None;
                    } else if owned.len() < 2 {
                        ((gun_value(def.class) - best).max(0.0) + if best < 1.5 { 1.0 } else { 0.3 }, false)
                    } else {
                        let gain = gun_value(def.class) - worst;
                        if gain < 0.8 { return None } else { (gain, true) }
                    }
                }
                Loot::Armor if health.max < crate::loot::ARMORED => (if health.hp < health.max * 0.6 { 2.6 } else { 1.8 }, false),
                Loot::Gear(_) if player.gear_left == 0 => (1.0, false),
                _ => return None,
            };
            let score = value - distance / 14.0;
            (score > 0.0).then_some((*at, score, swap))
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
}

/// Pitch to lob a thrown grenade `range` metres (they leave the hand at about 19 m/s, a
/// little upward).
fn lob_pitch(range: f32) -> f32 {
    ((range - 13.0) / 50.0).clamp(-0.25, 0.55)
}

/// Where to fight or hold from. Fighting, `spot` is tucked in (crouched) and `peek` is
/// where to stand to shoot (the same spot when shooting over low cover); without a `shot`
/// it is only somewhere to hide and reload. Holding, the bot waits at `spot`, crouched if `low`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Cover {
    spot: Vec3,
    peek: Vec3,
    low: bool,
    shot: bool,
}

/// What a cover search is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Seek {
    /// Trading shots: the nearest spot that hides a crouched body and lets it shoot.
    Fight,
    /// Out of range: only spots that close the distance, so the bot bounds cover to cover.
    Push,
    /// Watching an approach: a concealed spot that sees it, best overlooking a choke point.
    Hold,
}

/// Someone (or somewhere) to take cover from: feet, eye and chest.
#[derive(Clone, Copy, Debug)]
struct Contact {
    feet: Vec3,
    eye: Vec3,
    chest: Vec3,
}

impl Contact {
    fn at(feet: Vec3, stance_now: u8) -> Self {
        let eye = stance::eye(stance_now);
        Self { feet, eye: feet + Vec3::Y * eye, chest: feet + Vec3::Y * (eye - 0.45) }
    }
}

/// Line-of-sight rays one cover search may cast (the finalists' choke checks come on top).
const COVER_RAYS: u32 = 96;
/// A crouched body's head and torso: what fighting cover has to hide.
const TUCKED: [f32; 2] = [0.95, 0.6];
/// Seconds a bot keeps fighting from cover after the target ducks out of sight.
const ENGAGED: f32 = 2.5;

/// Walls close on both sides across one axis but open along the other: a doorway, an
/// alley, a gap between buildings.
fn narrow(statics: &StaticWorld, feet: Vec3) -> bool {
    let chest = feet + Vec3::Y * 1.1;
    let room = |dir: Vec3| statics.raycast(chest, dir, 6.0).map_or(6.0, |(d, _)| d);
    let across_x = room(Vec3::X) + room(Vec3::NEG_X);
    let across_z = room(Vec3::Z) + room(Vec3::NEG_Z);
    across_x.min(across_z) < 4.5 && across_x.max(across_z) > 8.0
}

/// Picks a spot near `feet` to fight `threat` from (or, for [`Seek::Hold`], to watch it
/// from). `clear` tests a line of sight, `stand` finds the standing point near a column
/// (at about a height) if a body fits there, and `reach` says whether the bot can walk
/// straight to a spot. Near spots, real cover, keeping out of sight of `others` and the
/// weapon's `range` score higher; spots inside a narrow passage score lower; holding,
/// spots whose view crosses a choke point or a ladder score higher. `current` keeps the
/// choice steady from one search to the next.
#[allow(clippy::too_many_arguments)]
fn find_cover(
    feet: Vec3,
    threat: Contact,
    others: &[Contact],
    seek: Seek,
    range: (f32, f32),
    current: Option<Vec3>,
    ladders: &[Vec3],
    statics: &StaticWorld,
    clear: &mut impl FnMut(Vec3, Vec3) -> bool,
    stand: &impl Fn(Vec2, f32) -> Option<Vec3>,
    reach: &impl Fn(Vec3, Vec3) -> bool,
) -> Option<Cover> {
    let rays = std::cell::Cell::new(0u32);
    let mut look = |a: Vec3, b: Vec3| {
        rays.set(rays.get() + 1);
        clear(a, b)
    };
    let away = (feet.xz() - threat.feet.xz()).normalize_or(Vec2::X);
    let now = feet.xz().distance(threat.feet.xz());
    let mut finalists: Vec<(f32, Cover)> = Vec::new();
    'search: for radius in [0.0f32, 1.5, 3.0, 4.5, 6.5, 9.0] {
        let count = if radius == 0.0 { 1 } else { 8 };
        for i in 0..count {
            if rays.get() >= COVER_RAYS {
                break 'search;
            }
            let xz = feet.xz() + Vec2::from_angle(i as f32 * TAU / count as f32).rotate(away) * radius;
            let distance = xz.distance(threat.feet.xz());
            if seek == Seek::Push && distance > now - 2.5 {
                continue;
            }
            let Some(spot) = stand(xz, feet.y) else { continue };
            let mut score = -radius * if seek == Seek::Hold { 0.6 } else { 0.8 };
            if current.is_some_and(|c| c.distance(spot) < 1.2) {
                score += 1.5;
            }
            let cover = if seek == Seek::Hold {
                // Somewhere to watch from, crouched if that still sees, with something in front.
                let low = look(spot + Vec3::Y * stance::eye(stance::CROUCH), threat.chest);
                if !low && !look(spot + Vec3::Y * stance::eye(stance::STAND), threat.chest) {
                    continue;
                }
                if !look(threat.eye, spot + Vec3::Y * 0.6) {
                    score += 3.0;
                }
                score += if low { 1.0 } else { 0.0 };
                Cover { spot, peek: spot, low, shot: true }
            } else {
                // Hidden crouched; shooting either standing up over the top or a step out to the side.
                if TUCKED.iter().any(|h| look(threat.eye, spot + Vec3::Y * *h)) {
                    continue;
                }
                let side = (xz - threat.feet.xz()).normalize_or(Vec2::X).perp();
                let peek = std::iter::once(xz).chain([1.2, -1.2, 2.2, -2.2].map(|step| xz + side * step)).find_map(|p| {
                    let at = if p == xz { Some(spot) } else { stand(p, spot.y) };
                    at.filter(|at| rays.get() < COVER_RAYS && look(*at + Vec3::Y * stance::eye(stance::STAND), threat.chest))
                });
                score += if peek.is_some() { 4.0 } else { 0.0 };
                score -= (range.0 - distance).max(0.0) * 1.0 + (distance - range.1).max(0.0) * 0.3;
                for other in others.iter().take(3) {
                    if look(other.eye, spot + Vec3::Y * TUCKED[0]) {
                        score -= 4.0;
                    }
                }
                Cover { spot, peek: peek.unwrap_or(spot), low: true, shot: peek.is_some() }
            };
            finalists.push((score, cover));
        }
    }
    finalists.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut best: Option<(f32, Cover)> = None;
    for (mut score, cover) in finalists.into_iter().take(4) {
        if cover.spot.xz().distance(feet.xz()) > 0.5 && !reach(feet, cover.spot) {
            continue;
        }
        if narrow(statics, cover.spot) {
            score -= 2.5;
        }
        if seek == Seek::Hold {
            // Overlooking a doorway, an alley or a ladder the enemy has to come through.
            let watched = cover.spot.lerp(threat.feet, 0.5);
            if narrow(statics, watched) || narrow(statics, cover.spot.lerp(threat.feet, 0.8)) {
                score += 2.5;
            }
            let (a, b) = (cover.spot.xz(), threat.feet.xz());
            let near_line = |p: Vec3| {
                let t = ((p.xz() - a).dot(b - a) / (b - a).length_squared().max(1e-4)).clamp(0.0, 1.0);
                (a + (b - a) * t).distance(p.xz()) < 2.5
            };
            if ladders.iter().any(|l| near_line(*l)) {
                score += 2.0;
            }
        }
        if best.is_none_or(|(s, _)| score > s) {
            best = Some((score, cover));
        }
    }
    best.filter(|(score, _)| *score > -8.0).map(|(_, cover)| cover)
}

/// Seeks `point`, slowing on arrival; once there keeps shuffling along `across` so the bot
/// never stands stock still.
fn approach(feet: Vec3, point: Vec3, across: Vec2) -> Vec2 {
    let to = point.xz() - feet.xz();
    let distance = to.length();
    if distance > 0.3 { to / distance * (distance / 0.8).min(1.0) } else { across * 0.25 }
}

impl Bot {
    /// A terrorist-hunt hostile holding `post`.
    pub fn sentry(post: Vec3) -> Self {
        Self { strafe: 1.0, sentry: true, post, crouch_at_range: true, ..default() }
    }
}

/// Something a bot might hear this frame: gunfire, footsteps, a body falling, a shout.
#[derive(Clone, Copy, Debug)]
pub struct Noise {
    pub at: Vec3,
    /// Heard by anyone within this distance.
    pub radius: f32,
    /// Who made it. Enemies of this team react to it...
    pub team: u8,
    /// ...unless it is a shout: then this team's own bots hear it (a callout of where an enemy is).
    pub shout: bool,
}

/// Noises made since the bots last listened (authority only).
#[derive(Resource, Default)]
pub struct Noises(pub Vec<Noise>);

/// Does a line of sight pass through leaves? Leaves around the looker do not count:
/// you can see out of the bush you are hiding in.
pub fn leaves_between(foliage: &[(Vec3, f32)], from: Vec3, to: Vec3) -> bool {
    let span = to - from;
    let length = span.length();
    if length < 1e-3 {
        return false;
    }
    let dir = span / length;
    foliage.iter().any(|(center, radius)| center.distance(from) > *radius && crate::combat::ray_sphere(from, dir, *center, *radius).is_some_and(|d| d < length))
}

/// How far footsteps carry for a given stance and speed.
pub fn footstep_radius(stance_now: u8, speed: f32) -> f32 {
    if speed < 1.0 {
        return 0.0;
    }
    match stance_now {
        stance::PRONE => 1.5,
        stance::CROUCH => 2.5,
        _ if speed > 6.5 => 15.0,
        _ => 7.0,
    }
}

/// Seconds without headway before a stuck bot steps aside, backs off, and gives up on where it was going.
pub const UNSTICK_SIDESTEP: f32 = 1.3;
pub const UNSTICK_BACK_OFF: f32 = 2.2;
pub const UNSTICK_GIVE_UP: f32 = 3.0;

/// Which way a bot that has been getting nowhere for `stuck` seconds tries to move instead of
/// `wish`: straight on at first, then sideways (`side` picks which), then backwards.
pub fn unstick(wish: Vec2, stuck: f32, side: f32) -> Vec2 {
    if stuck > UNSTICK_BACK_OFF {
        -wish
    } else if stuck > UNSTICK_SIDESTEP {
        (wish.perp() * side.signum() + wish * 0.2).normalize_or_zero()
    } else {
        wish
    }
}

impl Bot {
    /// Holding still on purpose: fighting, at cover, camping, planting or defusing, holding a
    /// point, or standing guard.
    pub fn holding(&self) -> bool {
        self.target.is_some() || self.cover.is_some() || self.hold_use || self.sentry || matches!(self.plan, Plan::Camp { .. }) || self.objective.is_some_and(|(_, hold)| hold)
    }
}

/// First player id used by bots; humans stay below it.
pub const FIRST_BOT_ID: u8 = 200;
const NAMES: [&str; 12] = ["VEGA", "ONYX", "RIGGS", "HALO", "DUSK", "MOTH", "KITE", "BRASS", "ECHO", "FLINT", "NOVA", "WREN"];

/// Spawns a bot on `team` with a random class and random attachments.
pub fn spawn_bot(commands: &mut Commands, ids: &mut NetIds, content: &Content, rules: &Rules, map: &Map, rng: &mut SimRng, id: u8, team: u8) -> Entity {
    let name = format!("{} [BOT]", NAMES[id as usize % NAMES.len()]);
    let role = [Role::Rusher, Role::Anchor, Role::Flanker][id as usize % 3];
    let brain = Bot { strafe: 1.0, crouch_at_range: rng.0.chance(0.6), role, ..default() };
    let entity = commands.spawn((player_bundle(content, rules, map, id, name, team), ids.next(), brain)).id();
    let guns: Vec<u8> = (0..content.weapons.len() as u8).filter(|i| content.weapons[*i as usize].frame.is_some()).collect();
    if guns.len() < 2 {
        return entity;
    }
    let primary = guns[1 + rng.0.below(guns.len() - 1)];
    let loadout = Loadout { primary, secondary: guns[0], perk: rng.0.below(content.perks.len().max(1)) as u8, gear: rng.0.below(content.gear.len().max(1)) as u8, parts: None, streaks: crate::killstreak::bot_streaks(&mut rng.0), vision: 0 };
    // Kit the primary out: each slot has an even chance of a random part that fits.
    let mut parts = content.weapons[primary as usize].defaults;
    for (slot_index, part) in parts.iter_mut().enumerate().take(SLOT_COUNT) {
        let choices = options(&content.weapons[primary as usize], &content.attachments, slot_index);
        if rng.0.chance(0.5) && !choices.is_empty() && !(slot_index == slot::OPTIC && *part != 0) {
            *part = choices[rng.0.below(choices.len())].0 as u8;
        }
    }
    let content = content.clone();
    commands.entity(entity).queue(move |mut entity: EntityWorldMut| {
        let Some(mut player) = entity.take::<Player>() else {
            return;
        };
        if let Some(mut arsenal) = entity.get_mut::<Arsenal>() {
            if let Some(weapon) = arsenal.weapons.get_mut(primary as usize) {
                weapon.equipped = parts;
            }
            apply_loadout(&content, &mut player, &mut arsenal, loadout);
        }
        let weapon = entity.get::<Arsenal>().map_or(0, |arsenal| arsenal.current);
        if let Some(mut input) = entity.get_mut::<PlayerInput>() {
            input.weapon = weapon;
        }
        entity.insert(player);
    });
    entity
}

/// Somewhere to sweep through: one of the map's waypoints on the indoor maps (their open
/// ground is mostly walled-off void), anywhere in the arena elsewhere.
fn sweep_point(map: &Map, rng: &mut SimRng) -> Vec3 {
    if let Some(p) = rng.0.pick(&map.waypoints) {
        return *p;
    }
    let reach = map.arena * 0.8;
    let (x, z) = (rng.0.range(-reach, reach), rng.0.range(-reach, reach));
    Vec3::new(x, map.height(x, z), z)
}

fn yaw_to(dir: Vec2) -> f32 {
    f32::atan2(-dir.x, -dir.y)
}

fn turn(current: f32, wanted: f32, max_step: f32) -> f32 {
    let diff = (wanted - current + PI).rem_euclid(TAU) - PI;
    current + diff.clamp(-max_step, max_step)
}

/// Preferred fighting distance (min, max) for the weapon class.
fn engagement(class: GunClass) -> (f32, f32) {
    match class {
        GunClass::Shotgun => (2.0, 8.0),
        GunClass::Smg | GunClass::Pistol => (5.0, 16.0),
        GunClass::Sniper => (25.0, 70.0),
        GunClass::Launcher => (14.0, 35.0),
        _ => (9.0, 28.0),
    }
}

/// The last place each team saw (or heard) an enemy, and when.
#[derive(Default)]
pub struct TeamIntel([Option<(Vec3, f32)>; 2]);

/// Someone on the field, for planning.
#[derive(Clone, Copy)]
struct Body {
    team: u8,
    id: u8,
    at: Vec3,
    bot: bool,
}

/// Chooses (or keeps) a versus bot's plan while it has no target, and sets its goal.
#[allow(clippy::too_many_arguments)]
fn plan_versus(bot: &mut Bot, me: &Player, feet: Vec3, hurt: bool, long: bool, map: &Map, rng: &mut SimRng, bodies: &[Body], intel: Option<(Vec3, f32)>, now: f32) {
    let go = |bot: &mut Bot, at: Vec3| {
        // Not the way it keeps getting killed: round the side.
        let at = route_round(bot, feet, at, now, |x, z| map.in_bounds(x, z).then(|| map.height(x, z)));
        // Never into the enemy base: stop at its edge.
        let at = match crate::player::base_centre(map, 1 - me.team.min(1)) {
            Some(c) if crate::player::in_enemy_base(map, me.team, at) => {
                let out = (at.xz() - c).normalize_or(Vec2::X) * crate::player::BASE_RADIUS * 1.6 + c;
                Vec3::new(out.x, map.height(out.x, out.y), out.y)
            }
            _ => at,
        };
        if bot.goal.distance(at) > 2.0 || bot.path.is_empty() {
            bot.goal = at;
            bot.path.clear();
            bot.stuck = 0.0;
        }
    };
    let mates: Vec<&Body> = bodies.iter().filter(|b| b.team == me.team && b.id != me.id).collect();
    let solo = mates.is_empty();
    // What the team knows: fresh if seen or heard in the last few seconds.
    let fresh = intel.filter(|(_, t)| now - t < 9.0).map(|(p, _)| p);
    // A long quiet: the bot has a fair idea where someone must be (footsteps, gunfire far off).
    let hunch = |rng: &mut SimRng| {
        let enemies: Vec<Vec3> = bodies.iter().filter(|b| b.team != me.team).map(|b| b.at).collect();
        let near = rng.0.pick(&enemies).copied();
        near.map(|p| p + Vec3::new(rng.0.range(-10.0, 10.0), 0.0, rng.0.range(-10.0, 10.0)))
    };
    let expired = now >= bot.plan_until;
    let arrived = bot.goal.xz().distance(feet.xz()) < 3.0;

    // Hurt: back toward friends (or simply away from trouble) for a breather.
    if hurt && bot.plan != Plan::Retreat {
        bot.plan = Plan::Retreat;
        bot.plan_until = now + 6.0;
        let away = match (mates.iter().min_by(|a, b| a.at.distance_squared(feet).total_cmp(&b.at.distance_squared(feet))), fresh) {
            (Some(mate), _) => mate.at,
            (None, Some(threat)) => feet + (feet - threat).normalize_or_zero() * 15.0,
            (None, None) => feet,
        };
        go(bot, away);
        return;
    }
    if bot.plan == Plan::Retreat && !expired {
        return;
    }
    // Shot at by someone it cannot see: never sit there and take it. Close in on them from
    // the side, or rush them, sprinting from cover to cover.
    if let Some((from, at)) = bot.threat
        && now - at < 4.0
        && !matches!(bot.plan, Plan::Flank | Plan::Hunt)
    {
        bot.threat = Some((from, at - 10.0));
        let far = from.distance(feet) > 35.0;
        if far && long {
            // A long gun against a far shooter: hold a concealed spot on that line and trade.
            bot.plan = Plan::Camp { face: yaw_to((from - feet).xz()) };
            bot.plan_until = now + rng.0.range(4.0, 6.0);
            bot.camp_spot = None;
            go(bot, feet);
        } else if far || rng.0.chance(0.55) {
            let side = if rng.0.chance(0.5) { 1.0 } else { -1.0 };
            let across = (from - feet).xz().normalize_or_zero().perp() * side * 16.0;
            let wide = from + Vec3::new(across.x, 0.0, across.y);
            bot.plan = Plan::Flank;
            bot.plan_until = now + 10.0;
            go(bot, Vec3::new(wide.x, map.height(wide.x, wide.z), wide.z));
        } else {
            bot.plan = Plan::Hunt;
            bot.plan_until = now + 8.0;
            go(bot, from);
        }
        return;
    }
    // Strayed from a human squad leader: drop whatever it was doing and catch up.
    let strayed = bodies.iter().filter(|b| b.team == me.team && !b.bot).any(|_| true)
        && bodies.iter().filter(|b| b.team == me.team).min_by_key(|b| (b.bot as u8, b.id)).is_some_and(|l| !l.bot && l.id != me.id && l.at.distance(feet) > 10.0);
    // Keep a plan while it lasts, refreshing where it points.
    if !expired && !strayed {
        match bot.plan {
            Plan::Hunt => {
                if let Some(p) = fresh {
                    go(bot, p);
                }
                if !arrived {
                    return;
                }
            }
            Plan::Flank if !arrived => return,
            Plan::Camp { .. } | Plan::Search if !arrived || matches!(bot.plan, Plan::Camp { .. }) => return,
            _ => {}
        }
    }

    // A new plan. Who leads the team: a human if there is one, else the lowest id.
    let leader = bodies.iter().filter(|b| b.team == me.team).min_by_key(|b| (b.bot as u8, b.id)).copied();
    let leading = leader.is_none_or(|l| l.id == me.id);
    let target = if fresh.is_some() { fresh } else if now - intel.map_or(-99.0, |(_, t)| t) > 12.0 { hunch(rng) } else { None };
    let enemy_base = map.team_spawns[(1 - me.team.min(1)) as usize].first().copied().unwrap_or(Vec2::ZERO);
    let watch = |from: Vec3, at: Vec3| yaw_to((at - from).xz());
    let flank = |bot: &mut Bot, at: Vec3| {
        // Out to one side of the line between us, then in.
        let side = if me.id % 2 == 0 { 1.0 } else { -1.0 };
        let across = (at - feet).xz().normalize_or_zero().perp() * side * 14.0;
        let wide = at + Vec3::new(across.x, 0.0, across.y);
        bot.plan = Plan::Flank;
        bot.plan_until = now + 12.0;
        go(bot, Vec3::new(wide.x, map.height(wide.x, wide.z), wide.z));
    };
    let camp = |bot: &mut Bot, at: Vec3, face: f32, seconds: f32| {
        bot.plan = Plan::Camp { face };
        bot.plan_until = now + seconds;
        go(bot, at);
    };
    if solo {
        // One on one: patient. Hold angles, ambush, flank, then go looking.
        match target {
            Some(p) if bot.plan == Plan::Flank || rng.0.chance(0.4) => {
                bot.plan = Plan::Hunt;
                bot.plan_until = now + 8.0;
                go(bot, p);
            }
            Some(p) if rng.0.chance(0.5) || fresh.is_none() => flank(bot, p),
            // A real sighting: wait for them to come round, gun on the angle.
            Some(p) => camp(bot, feet, watch(feet, p), rng.0.range(3.0, 5.0)),
            None if matches!(bot.plan, Plan::Camp { .. }) => {
                bot.plan = Plan::Search;
                bot.plan_until = now + rng.0.range(12.0, 20.0);
                go(bot, sweep_point(map, rng));
            }
            None => {
                let spot = sweep_point(map, rng);
                let face = watch(spot, Vec3::new(enemy_base.x, spot.y, enemy_base.y));
                camp(bot, spot, face, rng.0.range(4.0, 7.0));
            }
        }
        return;
    }
    if leading {
        match target {
            Some(p) => {
                bot.plan = Plan::Hunt;
                bot.plan_until = now + 8.0;
                go(bot, p);
            }
            None => {
                bot.plan = Plan::Search;
                bot.plan_until = now + rng.0.range(10.0, 16.0);
                go(bot, sweep_point(map, rng));
            }
        }
        return;
    }
    // With a human leading, the squad sticks close: they only peel off for trouble near
    // the human, and never wander far.
    let human_lead = leader.is_some_and(|l| !l.bot);
    let lead_at = leader.map_or(feet, |l| l.at);
    let target = if human_lead { target.filter(|p| p.distance(lead_at) < 25.0) } else { target };
    let far_from_lead = human_lead && feet.distance(lead_at) > 10.0;
    match (bot.role, target) {
        _ if far_from_lead => {
            bot.plan = Plan::Follow;
            bot.plan_until = now + 1.0;
            let angle = me.id as f32 * 2.4;
            let spot = lead_at + Vec3::new(angle.cos(), 0.0, angle.sin()) * 3.5;
            go(bot, Vec3::new(spot.x, lead_at.y, spot.z));
        }
        (Role::Rusher, Some(p)) => {
            bot.plan = Plan::Hunt;
            bot.plan_until = now + 8.0;
            go(bot, p);
        }
        (Role::Flanker, Some(p)) if bot.plan != Plan::Flank => flank(bot, p),
        (Role::Anchor, Some(p)) => {
            // Hold the angle the trouble will come from, a little back from the leader.
            camp(bot, feet, watch(feet, p), rng.0.range(3.0, 6.0));
        }
        _ => {
            // Stay with the leader, spread out a little to one side and behind.
            let lead = leader.map_or(feet, |l| l.at);
            let angle = me.id as f32 * 2.4;
            let spot = lead + Vec3::new(angle.cos(), 0.0, angle.sin()) * 4.5;
            bot.plan = Plan::Follow;
            bot.plan_until = now + 1.5;
            if spot.distance(feet) > 6.0 {
                go(bot, Vec3::new(spot.x, lead.y, spot.z));
            }
        }
    }
}

/// Objective modes: plays the points or the bomb instead of hunting for kills (fights
/// still come first: this only runs with no one in sight). Returns false when there is
/// no objective, and the usual plans take over.
#[allow(clippy::too_many_arguments)]
fn plan_objective(bot: &mut Bot, me: &Player, feet: Vec3, obj: &Objective, map: &Map, rng: &mut SimRng, bodies: &[Body], claims: &[u8], intel: Option<(Vec3, f32)>, now: f32) -> bool {
    use crate::objective::{DEFUSE_REACH, NEUTRAL, POINT_RADIUS, bomb, kind};
    bot.hold_use = false;
    if obj.kind == kind::NONE || obj.points.is_empty() || bot.sentry {
        return false;
    }
    let go = |bot: &mut Bot, at: Vec3, slack: f32| {
        // Far from it, come round the side of a spot it keeps dying to.
        let at = if at.distance(feet) > 25.0 { route_round(bot, feet, at, now, |x, z| map.in_bounds(x, z).then(|| map.height(x, z))) } else { at };
        if bot.goal.distance(at) > slack || (bot.path.is_empty() && bot.goal.xz().distance(feet.xz()) > 1.0) {
            bot.goal = at;
            bot.path.clear();
            bot.stuck = 0.0;
        }
    };
    // Spread round a spot so a squad does not stand in one heap.
    // Each time it has had to shake itself out of standing about, somewhere else round it.
    let shifts = bot.shifts;
    let around = |at: Vec3, radius: f32| {
        let angle = me.id as f32 * 2.4 + shifts as f32 * 1.3;
        Vec3::new(at.x + angle.cos() * radius, at.y, at.z + angle.sin() * radius)
    };
    let watch = |from: Vec3, at: Vec3| yaw_to((at - from).xz());
    let team = me.team.min(1);
    let distance = |at: Vec3| at.xz().distance(feet.xz());
    let middle = obj.points.iter().map(|p| p.at).fold(Vec3::ZERO, |a, b| a + b) / obj.points.len() as f32;
    let enemies: Vec<Vec3> = bodies.iter().filter(|b| b.team != team).map(|b| b.at).collect();
    let closest_enemy = |to: Vec3| enemies.iter().copied().min_by(|a, b| a.distance(to).total_cmp(&b.distance(to)));

    if obj.kind == kind::DOMINATION {
        let points = &obj.points;
        let ours: Vec<usize> = (0..points.len()).filter(|i| points[*i].owner == team).collect();
        let mut theirs: Vec<usize> = (0..points.len()).filter(|i| points[*i].owner != team).collect();
        theirs.sort_by(|a, b| distance(points[*a].at).total_cmp(&distance(points[*b].at)));
        let threatened = ours.iter().copied().find(|i| points[*i].contested || (points[*i].taking != team && points[*i].taking != NEUTRAL));
        let still_good = |(index, defend): (u8, bool)| points.get(index as usize).is_some_and(|p| (p.owner == team) == defend);
        let keep = bot.objective.filter(|o| now < bot.plan_until && still_good(*o));
        let (index, defend) = match keep {
            Some(o) => (o.0 as usize, o.1),
            None => {
                let nearest_ours = ours.iter().copied().min_by(|a, b| distance(points[*a].at).total_cmp(&distance(points[*b].at)));
                let claimed = |i: usize| claims.get(i).copied().unwrap_or(0) as f32;
                // Split the team: a point others are already going for counts as further off.
                let attack = theirs.iter().copied().min_by(|a, b| (distance(points[*a].at) + claimed(*a) * 28.0).total_cmp(&(distance(points[*b].at) + claimed(*b) * 28.0)));
                let second = theirs.iter().copied().filter(|i| Some(*i) != attack).min_by(|a, b| (distance(points[*a].at) + claimed(*a) * 28.0).total_cmp(&(distance(points[*b].at) + claimed(*b) * 28.0)));
                let choice = match (bot.role, threatened, nearest_ours) {
                    // A point being taken: go and hold it, unless two are on it already.
                    (_, Some(i), _) if claimed(i) < 2.0 && rng.0.chance(0.75) => (i, true),
                    // Anchors hold what the team has (one each is plenty).
                    (Role::Anchor, _, Some(i)) if claimed(i) < 1.0 && rng.0.chance(0.75) => (i, true),
                    // Flankers go for another point than the rest.
                    (Role::Flanker, ..) if second.is_some() && rng.0.chance(0.6) => (second.unwrap_or(0), false),
                    _ if attack.is_some() => (attack.unwrap_or(0), false),
                    _ => {
                        // Everything held: defend the least guarded.
                        let i = ours.iter().copied().min_by(|a, b| (claimed(*a) * 30.0 + distance(points[*a].at)).total_cmp(&(claimed(*b) * 30.0 + distance(points[*b].at)))).unwrap_or(0);
                        (i, true)
                    }
                };
                bot.objective = Some((choice.0 as u8, choice.1));
                bot.plan_until = now + rng.0.range(9.0, 16.0);
                choice
            }
        };
        let point = points[index].at;
        // Look out from the point toward trouble: the closest enemy, else the middle.
        let threat = closest_enemy(point).unwrap_or(middle);
        if points[index].holds(feet) && distance(point) < POINT_RADIUS * 0.8 {
            let face = if threat.xz().distance(feet.xz()) > 1.0 { watch(feet, threat) } else { watch(point, feet) };
            bot.plan = Plan::Camp { face };
            go(bot, feet, 1.5);
        } else {
            bot.plan = Plan::Hunt;
            go(bot, around(point, if defend { POINT_RADIUS * 0.45 } else { 1.5 }), 2.0);
        }
        return true;
    }

    // Search and destroy.
    let attacking = team == obj.attackers;
    let site_at = |i: u8| obj.points[(i as usize).min(obj.points.len() - 1)].at;
    let mates: Vec<&Body> = bodies.iter().filter(|b| b.team == team).collect();
    let nearest_mate_to = |at: Vec3| mates.iter().min_by(|a, b| a.at.distance(at).total_cmp(&b.at.distance(at))).map(|b| b.id);
    match (attacking, obj.bomb) {
        (true, bomb::CARRIED) if obj.carrier == me.id => {
            let site = site_at(obj.site);
            if crate::objective::in_site(site, feet) {
                bot.plan = Plan::Camp { face: watch(site, middle) };
                bot.hold_use = true;
                go(bot, feet, 1.5);
            } else {
                bot.plan = Plan::Hunt;
                go(bot, site, 1.0);
            }
        }
        (true, bomb::HOME | bomb::DROPPED) => {
            // The closest goes for the bomb; the rest cover it.
            bot.plan = Plan::Hunt;
            if nearest_mate_to(obj.bomb_at) == Some(me.id) || distance(obj.bomb_at) < 6.0 {
                go(bot, obj.bomb_at, 0.8);
            } else {
                go(bot, around(obj.bomb_at, 5.0), 4.0);
            }
        }
        (true, bomb::CARRIED) if bot.role == Role::Flanker => {
            // Not everyone in one bunch: the flanker goes on ahead to clear the site.
            bot.plan = Plan::Hunt;
            go(bot, around(site_at(obj.site), 7.0), 3.0);
        }
        (true, bomb::CARRIED) => {
            // Escort: a few steps ahead of the carrier, toward the site.
            let carrier = bodies.iter().find(|b| b.id == obj.carrier).map_or(obj.bomb_at, |b| b.at);
            let ahead = carrier + (site_at(obj.site) - carrier).normalize_or_zero() * 5.0;
            bot.plan = Plan::Follow;
            go(bot, around(ahead, 3.0), 4.0);
        }
        (true, _) => {
            // Planted: guard it, watching the way the defenders will come.
            let spot = around(obj.bomb_at, 6.0);
            let face = closest_enemy(obj.bomb_at).map_or_else(|| watch(obj.bomb_at, spot), |e| watch(spot, e));
            bot.plan = Plan::Camp { face };
            go(bot, spot, 2.0);
        }
        (false, bomb::PLANTED) => {
            bot.plan = Plan::Hunt;
            if nearest_mate_to(obj.bomb_at) == Some(me.id) || distance(obj.bomb_at) < DEFUSE_REACH {
                if distance(obj.bomb_at) < DEFUSE_REACH * 0.6 {
                    bot.plan = Plan::Camp { face: watch(obj.bomb_at, feet) };
                    bot.hold_use = true;
                    go(bot, feet, 1.5);
                } else {
                    go(bot, obj.bomb_at, 0.6);
                }
            } else {
                go(bot, around(obj.bomb_at, 5.0), 2.0);
            }
        }
        (false, _) => {
            // Hold a site, split between them by id. Rushers go for fresh contacts close by.
            let mut ids: Vec<u8> = mates.iter().map(|b| b.id).collect();
            ids.sort();
            let slot = ids.iter().position(|id| *id == me.id).unwrap_or(me.id as usize);
            let mut site = site_at((slot % obj.points.len()) as u8);
            let fresh = intel.filter(|(_, t)| now - t < 6.0).map(|(p, _)| p);
            // Rotate: attackers seen closing on the other site pull all but the anchors over.
            if let Some(p) = fresh
                && bot.role != Role::Anchor
                && let Some(other) = obj.points.iter().map(|s| s.at).min_by(|a, b| a.distance(p).total_cmp(&b.distance(p)))
                && other.distance(p) < 30.0
            {
                site = other;
            }
            match fresh.filter(|p| bot.role == Role::Rusher && p.distance(site) < 35.0) {
                Some(contact) => {
                    bot.plan = Plan::Hunt;
                    go(bot, contact, 3.0);
                }
                None => {
                    let spot = around(site, 4.0);
                    let face = closest_enemy(site).map_or_else(|| watch(site, spot), |e| watch(spot, e));
                    bot.plan = Plan::Camp { face };
                    go(bot, spot, 2.0);
                }
            }
        }
    }
    true
}

#[allow(clippy::too_many_arguments)]
pub fn drive_bots(
    time: Res<Time>,
    map: Res<Map>,
    statics: Res<Statics>,
    nav: Option<Res<Nav>>,
    content: Res<Content>,
    rules: Res<Rules>,
    state: Res<MatchState>,
    mut scratch: ResMut<NavScratch>,
    mut budget: ResMut<NavBudget>,
    mut rng: ResMut<SimRng>,
    mut players: Query<(Entity, Option<&mut Bot>, &Player, &Transform, &mut PlayerInput, &Health, &Arsenal, Option<&mut crate::combat::Stunned>, &crate::player::Mover)>,
    mut noises: ResMut<Noises>,
    solids: Query<(&Transform, &Solid), (Or<(With<Structure>, With<crate::breakables::Door>, With<crate::breakables::ClutterPiece>)>, Without<Player>)>,
    (smokes, pickups, mut shots, mut orders): (Query<(&Transform, &crate::combat::SmokeCloud), Without<Player>>, Query<(&Transform, &crate::loot::Pickup), Without<Player>>, EventReader<FxEvent>, EventWriter<CommandEvent>),
    mut leaves: Local<(u64, Vec<(Vec3, f32)>)>,
    mut intel: Local<TeamIntel>,
) {
    let dt = sim_dt(&time);
    // Tree crowns and bushes hide whoever is behind (or inside) them.
    let key = map.recipe.seed ^ map.props.len() as u64;
    if leaves.0 != key || leaves.1.is_empty() {
        *leaves = (key, map.foliage());
    }
    let foliage = &leaves.1;
    // Everyone a bot might fight, humans and bots alike.
    let fighters: Vec<(Entity, u8, Vec3, u8)> =
        players.iter().filter(|(.., health, _, _, _)| health.alive()).map(|(entity, _, player, transform, input, ..)| (entity, player.team, transform.translation, input.stance)).collect();
    // Torches lit in the dark give their carriers away.
    let dark = state.daylight < 0.5;
    let lit: Vec<Entity> = players.iter().filter(|(_, _, _, _, input, health, ..)| dark && input.light && health.alive()).map(|(entity, ..)| entity).collect();
    // Everyone alive, for planning; and what each team knows of the other.
    let bodies: Vec<Body> = players.iter().filter(|(.., health, _, _, _)| health.alive()).map(|(_, bot, p, t, ..)| Body { team: p.team, id: p.id, at: t.translation, bot: bot.is_some() }).collect();
    let now = time.elapsed_secs();
    for (_, bot, p, ..) in players.iter() {
        if let Some(seen) = bot.and_then(|b| b.target).and_then(|e| fighters.iter().find(|f| f.0 == e)) {
            intel.0[(p.team as usize).min(1)] = Some((seen.2, now));
        }
    }
    if state.phase != Phase::Live {
        intel.0 = [None, None];
    }
    // Living humans, for squadmates to stay with.
    let humans: Vec<(u8, Vec3)> = players.iter().filter(|(_, bot, .., health, _, _, _)| bot.is_none() && health.alive()).map(|(_, _, p, t, ..)| (p.team, t.translation)).collect();
    // What can be heard this frame: everything made since last frame, plus everyone's footsteps.
    let mut heard_now = std::mem::take(&mut noises.0);
    for (_, _, player, transform, input, health, _, _, mover) in &players {
        // Dead Silence: footsteps carry a fifth as far.
        let mut radius = footstep_radius(input.stance, mover.speed) * crate::player::footstep_volume(&content, player.perk);
        // Pushing through a bush rustles: it carries much further than a footstep.
        let at = transform.translation;
        if radius > 0.0 && map.props.iter().any(|p| matches!(p.kind, oz_core::worldgen::PropKind::Bush) && (p.pos.xz() - at.xz()).length() < 0.75 * p.scale + 0.25 && (p.pos.y - at.y).abs() < 1.2) {
            radius = radius.max(18.0);
        }
        if health.alive() && radius > 0.0 {
            heard_now.push(Noise { at: transform.translation, radius, team: player.team, shout: false });
        }
    }
    let mut shouts: Vec<Noise> = Vec::new();
    // Rounds fired this frame (who by, from where, to where): sentries notice fire aimed at them.
    let teams: Vec<(u8, u8)> = players.iter().map(|(_, _, p, ..)| (p.id, p.team)).collect();
    let spots: Vec<(u8, Vec3)> = players.iter().map(|(_, _, p, t, ..)| (p.id, t.translation)).collect();
    // (who fired, their team, from, to), and who killed whom.
    let mut fired: Vec<(u8, u8, Vec3, Vec3)> = Vec::new();
    let mut killed: Vec<(u8, u8)> = Vec::new();
    for event in shots.read() {
        match event {
            FxEvent::Shot { by, from, to, .. } => {
                if let Some((_, team)) = teams.iter().find(|(id, _)| id == by) {
                    fired.push((*by, *team, *from, *to));
                }
            }
            FxEvent::Killed { victim, by, .. } => killed.push((*victim, *by)),
            _ => {}
        }
    }
    // Loot lying about, for bots to weigh up.
    let loot: Vec<(Vec3, crate::loot::Loot)> = pickups.iter().filter(|(_, p)| !matches!(p.loot, crate::loot::Loot::Supply { .. })).map(|(t, p)| (t.translation, p.loot)).collect();
    // Which point each team's bots are going for (domination), to split up.
    let mut claims = [[0u8; 8]; 2];
    for (_, bot, p, ..) in players.iter() {
        if let Some((index, _)) = bot.and_then(|b| b.objective) {
            claims[(p.team as usize).min(1)][(index as usize).min(7)] += 1;
        }
    }
    // Gunfire gives the shooter away to the other team.
    for noise in heard_now.iter().filter(|n| !n.shout && n.radius > 40.0) {
        intel.0[(1 - noise.team.min(1)) as usize] = Some((noise.at, now));
    }

    // Ladder ends, for holding bots to watch.
    let ladders: Vec<Vec3> = map.ladders.iter().flat_map(|l| [l.foot, Vec3::new(l.foot.x, l.top, l.foot.z)]).collect();
    for (_, bot, me, transform, mut input, health, arsenal, stunned, mover) in &mut players {
        let Some(mut bot) = bot else { continue };
        let team = (me.team as usize).min(1);
        if health.alive() {
            bot.dead_noted = false;
            let feet = transform.translation;
            if bot.sentry {
                bot.alert = (bot.alert - dt).max(0.0);
                let base = crate::versus::hunt_base(&map);
                for noise in &heard_now {
                    let ours = noise.team == me.team;
                    if noise.shout != ours || noise.at.distance(feet) > noise.radius || noise.at.xz().distance(base) < crate::versus::HUNT_SAFE {
                        continue;
                    }
                    bot.heard = Some(noise.at);
                    bot.alert = 15.0;
                    // Gunfire close by: fire back toward it if there is a line to it.
                    let distance = noise.at.distance(feet);
                    if !noise.shout && noise.radius > 40.0 && distance < 45.0 && bot.target.is_none() {
                        let eye = feet + Vec3::Y * input.eye();
                        let dir = (noise.at - eye) / distance.max(1e-3);
                        if obstruction(&map, &statics, &solids, eye, dir, distance) > distance * 0.85 {
                            bot.suppress = rng.0.range(0.6, 1.2);
                            bot.suppress_at = noise.at + Vec3::Y * 0.2;
                        }
                    }
                }
            }
            bot.suppress -= dt;
            // Under fire: a hit, or a round cracking past or smacking in within a few metres,
            // gives away where it came from - exactly, for a round it saw go by.
            let chest = feet + Vec3::Y * 1.1;
            let incoming = fired
                .iter()
                .filter(|(_, team, ..)| *team != me.team)
                .filter(|(_, _, from, to)| {
                    let span = *to - *from;
                    let along = ((chest - *from).dot(span) / span.length_squared().max(1e-4)).clamp(0.0, 1.0);
                    (*from + span * along).distance(chest) < 3.5
                })
                .min_by(|a, b| a.2.distance(feet).total_cmp(&b.2.distance(feet)));
            let hit = health.hp < bot.last_hp - 0.5;
            bot.last_hp = health.hp;
            // Hit by something it never saw come (a blast, a blade): roughly, the nearest enemy.
            let source = incoming.map(|(by, _, from, _)| (*from, Some(*by))).or_else(|| {
                hit.then(|| fighters.iter().filter(|(_, team, ..)| *team != me.team).map(|(_, _, at, _)| *at).min_by(|a, b| a.distance(feet).total_cmp(&b.distance(feet))))
                    .flatten()
                    .map(|at| (at + Vec3::Y * 1.2, None))
            });
            if let Some((from, by)) = source {
                let fresh = !bot.under_fire(now, 2.5);
                bot.threat = Some((from, now));
                bot.shot_at = Some(now);
                if by.is_some() {
                    bot.shooter = by;
                }
                intel.0[team] = Some((from, now));
                let class = content.weapons.get(arsenal.current as usize).map_or(GunClass::Rifle, |w| w.class);
                if bot.sentry {
                    bot.alert = 15.0;
                    bot.patrol = 0.0;
                    // Go after them - from the side when they are far off and it has no gun to answer with.
                    let far = from.distance(feet) > 40.0 && !matches!(class, GunClass::Rifle | GunClass::Sniper | GunClass::Lmg);
                    let side = if me.id % 2 == 0 { 1.0 } else { -1.0 };
                    let across = (from - feet).xz().normalize_or_zero().perp() * side * 18.0;
                    let (x, z) = (from.x + across.x, from.z + across.y);
                    bot.heard = Some(if far && map.in_bounds(x, z) { Vec3::new(x, map.height(x, z), z) } else { from });
                }
                if fresh {
                    // React now, not at the next leisurely look round.
                    bot.think = bot.think.min(rng.0.range(0.03, 0.12));
                    bot.cover_time = 0.0;
                    if bot.target.is_none() {
                        bot.plan_until = 0.0;
                    }
                    // Call it out: friends within earshot come to help.
                    shouts.push(Noise { at: from, radius: 50.0, team: me.team, shout: true });
                    // Fire straight back at where it came from, given a line to it and a gun that carries.
                    let eye = feet + Vec3::Y * input.eye();
                    let distance = from.distance(eye);
                    let carries = distance < engagement(class).1 * 1.6 || matches!(class, GunClass::Rifle | GunClass::Sniper | GunClass::Lmg);
                    if carries && bot.target.is_none() && obstruction(&map, &statics, &solids, eye, (from - eye) / distance.max(1e-3), distance) > distance * 0.9 {
                        bot.suppress = rng.0.range(0.7, 1.3);
                        bot.suppress_at = from;
                    }
                }
            }
        } else if !bot.dead_noted {
            // Down: remember where, and where the killer stood, so as not to walk into it again.
            bot.dead_noted = true;
            let killer = killed
                .iter()
                .find(|(victim, by)| *victim == me.id && by != victim)
                .and_then(|(_, by)| spots.iter().find(|(id, _)| id == by))
                .map(|(_, at)| *at)
                .or_else(|| bot.threat.filter(|(_, t)| now - t < 6.0).map(|(p, _)| p))
                .or_else(|| (now < bot.engaged_until + 2.0).then_some(bot.foe));
            if let Some(killer) = killer {
                bot.deaths.push((transform.translation, killer, now));
                if bot.deaths.len() > 6 {
                    bot.deaths.remove(0);
                }
            }
            bot.detour = None;
        }
        // Flashed: stumble about blind until it wears off.
        if let Some(mut stunned) = stunned
            && stunned.0 > 0.0
        {
            stunned.0 -= dt;
            input.buttons = 0;
            input.movement = Vec2::new(bot.strafe * 0.4, -0.3);
            input.yaw += bot.strafe * dt * 1.5;
            bot.target = None;
            bot.seen = 0.0;
            continue;
        }
        if !health.alive() || state.phase == Phase::Lobby || state.over() {
            input.buttons = 0;
            input.movement = Vec2::ZERO;
            input.interact = false;
            bot.hold_use = false;
            bot.target = None;
            bot.path.clear();
            bot.cover = None;
            bot.camp_spot = None;
            bot.engaged_until = 0.0;
            continue;
        }
        let feet = transform.translation;
        let eye = feet + Vec3::Y * input.eye();
        let moved = feet.distance(bot.last) / dt.max(1e-4);
        bot.last = feet;
        let class = content.weapons.get(arsenal.current as usize).map_or(GunClass::Rifle, |w| w.class);
        let (near, far) = engagement(class);
        // Senses for choosing cover: sight lines, places to stand, and walkable straight lines.
        let mut clear = |a: Vec3, b: Vec3| {
            let d = a.distance(b);
            d < 1e-3 || obstruction(&map, &statics, &solids, a, (b - a) / d, d) >= d - 0.3
        };
        let stand = |xz: Vec2, near: f32| {
            let y = crate::player::ground_under(&map, &statics, xz.x, xz.y, near + 0.5);
            let at = Vec3::new(xz.x, y, xz.y);
            let fits = map.in_bounds(xz.x, xz.y) && (y - near).abs() < 1.0 && !statics.blocked(xz.x, xz.y, y, stance::height(stance::STAND), 0.3);
            (fits && nav.as_deref().is_none_or(|n| n.walkable(at))).then_some(at)
        };
        let reach = |a: Vec3, b: Vec3| nav.as_deref().is_none_or(|n| n.line_clear(a, b));

        bot.think -= dt;
        if bot.think <= 0.0 {
            bot.think = rng.0.range(0.2, 0.35);
            // The nearest enemy we can actually see, upright or leaning out of cover.
            let upright = PlayerInput { lean: 0, ..*input };
            // Under fire, it scans hard and far for whoever is shooting.
            // (Rifles reach a long way: being shot from far off must never go unanswered.)
            let threat_spot = bot.threat_at(now, 6.0);
            let sight = if threat_spot.is_some() { 220.0 } else { 85.0 };
            let sees = |from: Vec3, chest: Vec3| {
                let distance = chest.distance(from);
                distance < sight
                    && obstruction(&map, &statics, &solids, from, (chest - from) / distance.max(1e-3), distance) >= distance - 0.6
                    && !crate::combat::smoke_between(smokes, from, chest)
                    && (distance < 4.0 || !leaves_between(foliage, from, chest))
            };
            let (sentry, alert) = (bot.sentry, bot.alert > 0.0);
            let facing_dir = Vec2::new(-input.yaw.sin(), -input.yaw.cos());
            // Sentries only notice what is in front of them, and low, still shapes are hard to spot.
            let noticed = |at: Vec3, enemy_stance: u8, torch: bool| {
                if !sentry {
                    return true;
                }
                // Whoever just shot at it: it is looking for them, wherever they are.
                if threat_spot.is_some_and(|t| t.distance(at + Vec3::Y * 1.2) < 14.0) {
                    return true;
                }
                let to = at - feet;
                let distance = to.length();
                let range: f32 = if alert { 90.0 } else { 40.0 * [1.0, 0.7, 0.45][(enemy_stance as usize).min(2)] };
                // A lit torch in the dark shows from much further, and catches the eye off to the side.
                let (range, cone) = if torch { (range.max(40.0) * 1.8, -0.1) } else { (range, if alert { -0.3 } else { 0.4 }) };
                distance < 2.5 || (distance < range && to.xz().normalize_or_zero().dot(facing_dir) > cone)
            };
            // Hunt: the squad's base is off limits to hostiles.
            let safe = |at: Vec3| sentry && rules.hunt && at.xz().distance(crate::versus::hunt_base(&map)) < crate::versus::HUNT_SAFE;
            // Search and destroy: the bomb carrier, and a defender going for a planted bomb,
            // play the objective - they only take on someone right on top of them.
            let obj = &state.obj;
            let on_bomb_duty = obj.kind == crate::objective::kind::BOMB
                && ((obj.bomb == crate::objective::bomb::CARRIED && obj.carrier == me.id)
                    || (obj.bomb == crate::objective::bomb::PLANTED && me.team.min(1) != obj.attackers && obj.bomb_at.distance(feet) < 9.0));
            // Scavenging with only a sidearm: no long-range duels it cannot win - a real gun first.
            let sidearm_only = best_gun(&content, arsenal).is_none_or(|b| gun_value(content.weapons[b as usize].class) < 1.5);
            let pistol_reach = if !rules.loot || !sidearm_only {
                f32::MAX
            } else if threat_spot.is_some() {
                30.0
            } else {
                20.0
            };
            let found = fighters
                .iter()
                .filter(|(_, team, ..)| *team != me.team)
                .filter(|(_, _, at, _)| at.distance(feet) < pistol_reach)
                .filter(|(_, _, at, _)| !safe(*at))
                // (Shot at on the way, it answers whoever is doing it.)
                .filter(|(_, _, at, _)| !on_bomb_duty || at.distance(feet) < 10.0 || threat_spot.is_some_and(|t| t.distance(*at + Vec3::Y * 1.2) < 6.0 && t.distance(feet) < 30.0))
                .filter(|(entity, _, at, enemy_stance)| noticed(*at, *enemy_stance, lit.contains(entity)))
                .filter_map(|(entity, _, at, enemy_stance)| {
                    let chest = *at + Vec3::Y * (stance::eye(*enemy_stance) - 0.4);
                    // Face the target before testing a lean, so the lean goes the right way.
                    let facing = PlayerInput { yaw: yaw_to((chest - eye).xz()), ..upright };
                    [0i8].into_iter().find(|lean| sees(crate::player::eye_point(&statics, feet, &PlayerInput { lean: *lean, ..facing }), chest)).map(|lean| (*entity, chest, lean))
                })
                .min_by(|a, b| a.1.distance_squared(eye).total_cmp(&b.1.distance_squared(eye)));
            let target = found.map(|(entity, ..)| entity);
            bot.lean = found.map_or(0, |(.., lean)| lean);
            if target != bot.target {
                // Caught off guard, a sentry takes a moment to react.
                bot.seen = if bot.sentry && bot.alert <= 0.0 { -0.45 } else { 0.0 };
            }
            if bot.sentry
                && let Some((_, chest, _)) = found
            {
                bot.alert = 15.0;
                bot.heard = Some(chest);
                shouts.push(Noise { at: chest, radius: 22.0, team: me.team, shout: true });
            }
            bot.target = target;
            let choke = narrow(&statics, feet);
            if choke != bot.choke {
                bot.glance = 1.0;
            }
            bot.choke = choke;
            // Settles its aim on a far target the longer it tracks it.
            let far_off = found.is_some_and(|(_, chest, _)| chest.distance(eye) > 30.0);
            let focus = if far_off && target == bot.target { (1.0 - bot.seen.max(0.0) * 0.4).clamp(0.35, 1.0) } else { 1.0 };
            let miss = 0.07 / rules.difficulty.max(0.25) * focus;
            bot.aim_error = Vec2::new(rng.0.range(-miss, miss), rng.0.range(-miss, miss) * 0.6);
            // The gun in hand: never stand there clicking an empty one, and out of a fight
            // hold the best gun carried.
            let current = arsenal.current as usize;
            let loaded = content.weapons.get(current).zip(arsenal.weapons.get(current)).is_some_and(|(d, w)| has_rounds(d, w));
            if let Some(best) = best_gun(&content, arsenal)
                && arsenal.reload <= 0.0
                && best != arsenal.current
                && (!loaded || (bot.target.is_none() && gun_value(content.weapons[best as usize].class) > gun_value(class) + 0.2))
            {
                input.weapon = best;
            }
            // Loot worth the walk: always in scavenge; elsewhere when short of ammo or close by.
            // Never while carrying or working the bomb.
            let leader = humans.iter().filter(|(t, _)| *t == me.team).map(|(_, at)| *at).min_by(|a, b| a.distance(feet).total_cmp(&b.distance(feet)));
            let squad = rules.hunt && me.team == 0;
            let carrying = obj.kind == crate::objective::kind::BOMB && obj.bomb == crate::objective::bomb::CARRIED && obj.carrier == me.id;
            bot.swap_wait -= 0.3;
            // Loot it has been trying to reach for a long while is out of reach: leave it be.
            bot.loot_skip.retain(|(_, until)| now < *until);
            if let Some((at, _)) = bot.loot
                && now - bot.loot_since > 6.0 + at.distance(feet) * 0.4
            {
                bot.loot_skip.push((at, now + 60.0));
            }
            let reachable: Vec<(Vec3, crate::loot::Loot)> = loot.iter().copied().filter(|(at, _)| !bot.loot_skip.iter().any(|(s, _)| s.distance(*at) < 0.5)).collect();
            let wish_loot = if bot.sentry || bot.target.is_some() || on_bomb_duty || carrying || bot.hold_use {
                None
            } else {
                loot_wish(&content, me, health, arsenal, feet, &reachable, if rules.loot { 60.0 } else { 35.0 })
                    .filter(|(at, score, _)| rules.loot || *score > 1.2 || at.distance(feet) < 12.0)
                    .filter(|(at, ..)| !squad || leader.is_none_or(|l| l.distance(*at) < 20.0))
            };
            // Fighting with only a sidearm and a real gun lies a few steps off: go and grab it.
            bot.grab = if bot.target.is_some() && sidearm_only && !carrying {
                loot_wish(&content, me, health, arsenal, feet, &reachable, 12.0).filter(|(_, score, swap)| !swap && *score > 1.0).map(|(at, ..)| at)
            } else {
                None
            };
            if wish_loot.is_none_or(|(at, ..)| bot.loot.is_none_or(|(was, _)| was.distance(at) > 0.5)) {
                bot.loot_since = now;
            }
            bot.loot = None;
            if let Some((at, _, true)) = wish_loot
                && at.distance(feet) < 1.3
            {
                // Swap: the weaker gun in hand, then Use over the better one.
                let worst = (0..arsenal.weapons.len())
                    .filter(|i| arsenal.owns(*i) && *i < content.weapons.len())
                    .min_by(|a, b| {
                        let v = |i: usize| if has_rounds(&content.weapons[i], &arsenal.weapons[i]) { gun_value(content.weapons[i].class) } else { 0.3 };
                        v(*a).total_cmp(&v(*b))
                    })
                    .map(|i| i as u8);
                // Use takes the nearest gun in reach: only press it standing over the one it wants
                // (not the one it just dropped).
                let nearest = loot
                    .iter()
                    .filter(|(p, l)| matches!(l, crate::loot::Loot::Weapon { index, .. } if !arsenal.owns(*index as usize)) && p.xz().distance(feet.xz()) < 1.4)
                    .min_by(|a, b| a.0.distance(feet).total_cmp(&b.0.distance(feet)))
                    .map(|(p, _)| *p);
                if let Some(worst) = worst.filter(|_| nearest.is_some_and(|n| n.distance(at) < 0.05)) {
                    input.weapon = worst;
                    if arsenal.current == worst && arsenal.cooldown <= 0.0 && bot.swap_wait <= 0.0 {
                        bot.swap_wait = 1.0;
                        orders.write(CommandEvent { player: me.id, command: PlayerCommand::Pickup });
                    }
                }
            }
            // Gear: a stim when hurt, an ammo pack when short; grenades at a foe tucked in
            // behind cover, at a spot it keeps dying to, or smoke to cross a sniper's lane.
            bot.gear_wait -= 0.3;
            let gear = content.gear.get(me.gear as usize).map(|g| g.kind);
            if me.gear_left > 0 && bot.gear_wait <= 0.0 && bot.throw <= 0.0 && me.cooking.is_none() {
                use oz_core::content::GearKind;
                let hidden_foe = bot.target.is_none() && now < bot.engaged_until && bot.cover.is_some();
                let danger = bot.danger(now).map(|(at, _)| at).filter(|at| (8.0..32.0).contains(&at.distance(feet)) && bot.target.is_none());
                let pick = match gear {
                    Some(GearKind::Stim { .. }) if health.hp < health.max * 0.45 && bot.target.is_none() => Some(feet),
                    Some(GearKind::Ammo { .. }) if crate::loot::low_on_ammo(&content, arsenal) => Some(feet),
                    Some(GearKind::Frag { .. } | GearKind::Flash { .. }) if hidden_foe && (7.0..28.0).contains(&bot.foe.distance(feet)) => Some(bot.foe),
                    Some(GearKind::Frag { .. } | GearKind::Flash { .. }) => danger,
                    Some(GearKind::Smoke { .. }) => match bot.threat_at(now, 3.0) {
                        Some(from) if bot.target.is_none() && bot.cover.is_none() && from.distance(feet) > 25.0 => Some(feet + (from - feet).normalize_or_zero() * 7.0),
                        _ => danger.map(|d| feet.lerp(d, 0.5)),
                    },
                    _ => None,
                };
                if let Some(at) = pick {
                    bot.throw = if matches!(gear, Some(GearKind::Stim { .. } | GearKind::Ammo { .. })) { 0.1 } else { 0.45 };
                    bot.throw_at = at;
                    bot.gear_wait = rng.0.range(9.0, 15.0);
                }
            }
            if bot.sentry && rules.hunt && feet.xz().distance(crate::versus::hunt_base(&map)) < crate::versus::HUNT_SAFE + 2.0 {
                // Strayed into the squad's base: back to the post.
                bot.heard = None;
                bot.target = None;
                if bot.goal.distance(bot.post) > 1.0 {
                    bot.goal = bot.post;
                    bot.path.clear();
                }
            } else if bot.sentry && bot.target.is_none() {
                bot.patrol -= 0.3;
                if let Some(at) = bot.heard {
                    // Go and see what that was.
                    if at.distance(feet) < 3.0 {
                        bot.heard = None;
                    } else if bot.goal.distance(at) > 2.0 {
                        bot.goal = at;
                        bot.path.clear();
                    }
                } else if bot.patrol <= 0.0 || bot.stuck > UNSTICK_BACK_OFF + 0.5 {
                    // Mill about near the post.
                    bot.patrol = rng.0.range(8.0, 18.0);
                    let wander = Vec2::from_angle(rng.0.range(0.0, TAU)) * rng.0.range(0.0, 6.0);
                    bot.goal = bot.post + Vec3::new(wander.x, 0.0, wander.y);
                    bot.path.clear();
                    bot.stuck = 0.0;
                }
            } else if let Some((at, _, swap)) = wish_loot {
                // Something worth having lies close: ammo when running dry, a better gun, armour.
                bot.loot = Some((at, swap));
                if bot.goal.distance(at) > 0.8 || bot.path.is_empty() {
                    bot.goal = at;
                    bot.path.clear();
                    bot.stuck = 0.0;
                }
            } else if rules.hunt && me.team == 0 && bot.target.is_none() {
                // Squadmates stay with the humans.
                if let Some((_, lead)) = humans.iter().filter(|(team, _)| *team == me.team).min_by(|a, b| a.1.distance_squared(feet).total_cmp(&b.1.distance_squared(feet))) {
                    if lead.distance(feet) > 5.0 && (bot.goal.distance(*lead) > 3.0 || bot.path.is_empty()) {
                        bot.goal = *lead;
                        bot.path.clear();
                    }
                } else if bot.goal.distance(feet) < 4.0 || (bot.path.is_empty() && bot.repath <= 0.0) {
                    bot.goal = sweep_point(&map, &mut rng);
                    bot.path.clear();
                }
            } else if rules.loot
                && bot.target.is_none()
                && (0..arsenal.weapons.len()).filter(|i| arsenal.owns(*i)).count() < 2
                && let Some(gun) = reachable
                    .iter()
                    .filter(|(_, l)| matches!(l, crate::loot::Loot::Weapon { .. }))
                    .map(|(at, _)| *at)
                    .min_by(|a, b| a.distance_squared(feet).total_cmp(&b.distance_squared(feet)))
            {
                // Scavenge: a pistol is not enough - go and get a real gun, however far.
                bot.loot = Some((gun, false));
                if bot.goal.distance(gun) > 1.0 || bot.path.is_empty() {
                    bot.goal = gun;
                    bot.path.clear();
                    bot.stuck = 0.0;
                }
            } else if bot.target.is_none() {
                if bot.stuck > UNSTICK_BACK_OFF + 0.5 {
                    bot.plan_until = 0.0;
                    bot.stuck = 0.0;
                }
                let hurt = health.hp < health.max * 0.35;
                let known = intel.0[team];
                if !plan_objective(&mut bot, me, feet, &state.obj, &map, &mut rng, &bodies, &claims[team], known, now) {
                    let long = matches!(class, GunClass::Rifle | GunClass::Sniper | GunClass::Lmg);
                    plan_versus(&mut bot, me, feet, hurt, long, &map, &mut rng, &bodies, known, now);
                }
            }
            // Camping in versus: not out in the open or in a doorway, but tucked in nearby
            // with something in front, watching the approach (best one that funnels through
            // a choke point). Objective modes keep their exact spots.
            match bot.plan {
                Plan::Camp { face } if bot.target.is_none() && !bot.sentry && state.obj.kind == crate::objective::kind::NONE => {
                    let asked = bot.goal;
                    match bot.camp_spot.filter(|(a, taken, _)| a.distance(asked) < 0.5 || taken.distance(asked) < 0.5) {
                        Some((_, taken, _)) => {
                            if bot.goal.distance(taken) > 0.1 {
                                bot.goal = taken;
                                bot.path.clear();
                            }
                        }
                        None => {
                            let ahead = Vec2::new(-face.sin(), -face.cos()) * 14.0;
                            let watch = Vec3::new(asked.x + ahead.x, asked.y, asked.z + ahead.y);
                            let pick = find_cover(asked, Contact::at(watch, stance::STAND), &[], Seek::Hold, (near, far), None, &ladders, &statics, &mut clear, &stand, &reach);
                            let (taken, low) = pick.map_or((asked, true), |c| (c.spot, c.low));
                            bot.camp_spot = Some((asked, taken, low));
                            if taken.distance(asked) > 0.1 {
                                bot.goal = taken;
                                bot.path.clear();
                                bot.plan = Plan::Camp { face: yaw_to((watch - taken).xz()) };
                            }
                        }
                    }
                }
                Plan::Camp { .. } => {}
                _ => bot.camp_spot = None,
            }
        }
        bot.strafe_time -= dt;
        if bot.strafe_time <= 0.0 {
            bot.strafe_time = rng.0.range(0.6, 1.5);
            bot.strafe = -bot.strafe;
        }

        let target = bot.target.and_then(|entity| fighters.iter().find(|(e, ..)| *e == entity).copied());
        if let Some((_, _, enemy, enemy_stance)) = target {
            bot.foe = enemy;
            bot.foe_stance = enemy_stance;
            bot.engaged_until = now + ENGAGED;
        }
        // Lost sight of the target from cover (ducked, or it did): keep at it from there for
        // a moment rather than wander off - unless falling back or on the bomb.
        // Shot at by someone it cannot see: get behind something between it and them.
        // (Not on the bomb: carrying it or going to defuse, it pushes on and leaves the fight to
        // the others.)
        let obj = &state.obj;
        let duty = obj.kind == crate::objective::kind::BOMB
            && ((obj.bomb == crate::objective::bomb::CARRIED && obj.carrier == me.id)
                || (obj.bomb == crate::objective::bomb::PLANTED && me.team.min(1) != obj.attackers && obj.bomb_at.distance(feet) < 12.0));
        if bot.threat_at(now, 3.0).is_none() {
            bot.pinned = 0.0;
        } else if target.is_none() && bot.cover.is_some() {
            bot.pinned += dt;
        }
        let under_fire = target.is_none() && !bot.hold_use && !duty && bot.pinned < 3.0 && bot.threat_at(now, 3.0).is_some();
        if under_fire && let Some(from) = bot.threat_at(now, 3.0) {
            bot.foe = from - Vec3::Y * 1.2;
            bot.foe_stance = stance::STAND;
            bot.engaged_until = bot.engaged_until.max(now + 1.0);
        }
        let engaged = target.is_some() || (now < bot.engaged_until && bot.cover.is_some() && bot.plan != Plan::Retreat && !bot.hold_use);
        bot.cover_time -= dt;
        if !engaged && !under_fire {
            bot.cover = None;
        } else if bot.cover_time <= 0.0 {
            let seek = if target.is_some() && bot.foe.distance(feet) > far { Seek::Push } else { Seek::Fight };
            // Pushing, it takes a few shots from each spot before bounding on to the next.
            bot.cover_time = if seek == Seek::Push { rng.0.range(1.8, 3.0) } else { rng.0.range(0.6, 0.9) };
            // Cover must not leave it in the open to the target's friends close by.
            let mut others: Vec<Contact> =
                fighters.iter().filter(|f| f.1 != me.team && Some(f.0) != bot.target && f.2.distance(feet) < 40.0).map(|f| Contact::at(f.2, f.3)).collect();
            others.sort_by(|a, b| a.feet.distance(feet).total_cmp(&b.feet.distance(feet)));
            others.truncate(2);
            let current = bot.cover.map(|c| c.spot);
            let foe = Contact::at(bot.foe, bot.foe_stance);
            bot.cover = find_cover(feet, foe, &others, seek, (near, far), current, &ladders, &statics, &mut clear, &stand, &reach);
            // Nothing further forward, and under fire: get behind what is here before pushing on.
            if bot.cover.is_none() && seek == Seek::Push && bot.threat.is_some_and(|(_, at)| now - at < 2.0) {
                bot.cover = find_cover(feet, foe, &others, Seek::Fight, (near, far), current, &ladders, &statics, &mut clear, &stand, &reach);
            }
        }
        // No leaning in this game: bots shoot from where they stand.
        input.lean = 0;
        let mut held = 0;
        let mut wanted_stance = stance::STAND;
        let mut wish = Vec2::ZERO;
        if let Some((_, _, enemy, enemy_stance)) = target {
            // Fight: track the target, strafe, hold a comfortable range.
            bot.seen += dt;
            let to = enemy + Vec3::Y * (stance::eye(enemy_stance) - 0.45) - crate::player::eye_point(&statics, feet, &input);
            let distance = to.length().max(1e-3);
            let wanted_yaw = yaw_to(to.xz()) + bot.aim_error.x;
            let wanted_pitch = (to.y / distance).asin() + bot.aim_error.y;
            // Being shot at, it snaps round and answers fast.
            let snap = bot.under_fire(now, 4.0);
            input.yaw = turn(input.yaw, wanted_yaw, if snap { 10.0 } else { 6.5 } * dt);
            input.pitch += (wanted_pitch - input.pitch).clamp(-5.0 * dt, 5.0 * dt);
            let off_target = ((wanted_yaw - input.yaw + PI).rem_euclid(TAU) - PI).abs();
            let reacted = bot.seen > if snap { 0.15 } else { 0.35 } / rules.difficulty.max(0.25);
            // Tap the trigger every other frame so semi-automatic weapons fire too.
            if reacted && off_target < 0.08 && !input.held(buttons::FIRE) {
                held |= buttons::FIRE;
            }
            if distance > 12.0 && class != GunClass::Shotgun {
                held |= buttons::AIM;
            }
            let facing = to.xz() / to.xz().length().max(1e-3);
            // In the open (no cover in reach): never stand still. Close in from out of range,
            // keep the weapon's distance, and don't trade shots from a doorway: back out of it.
            let forward = if distance > far {
                1.0
            } else if distance < near || bot.choke {
                -1.0
            } else {
                0.0
            };
            wish = facing * forward + facing.perp() * bot.strafe * if class == GunClass::Sniper { 0.3 } else { 1.0 };
            if bot.lean != 0 {
                // Peeking: hold the corner instead of strafing out of it.
                wish *= 0.15;
            }
            // Only a sniper settles down for a long shot in the open; everyone else keeps moving.
            if bot.crouch_at_range && distance > 18.0 && class == GunClass::Sniper {
                wanted_stance = if distance > 35.0 { stance::PRONE } else { stance::CROUCH };
                wish *= 0.4;
            }
            if let Some(at) = bot.grab {
                // Over to the gun lying close by, still shooting.
                wish = (at.xz() - feet.xz()).normalize_or_zero() + facing.perp() * bot.strafe * 0.3;
            }
            bot.goal = enemy;
            bot.path.clear();
        } else if engaged {
            // Tucked in out of sight: keep the gun on where the target was.
            let to = Contact::at(bot.foe, bot.foe_stance).chest - crate::player::eye_point(&statics, feet, &input);
            input.yaw = turn(input.yaw, yaw_to(to.xz()), if under_fire { 10.0 } else { 6.5 } * dt);
            input.pitch += ((to.y / to.length().max(1e-3)).asin() - input.pitch).clamp(-5.0 * dt, 5.0 * dt);
        } else if let Some(nav) = nav.as_deref() {
            // Travel along a path; search again when it runs out or goes stale.
            bot.repath -= dt;
            if (bot.path.is_empty() || bot.repath <= 0.0) && budget.0 > 0 {
                budget.0 -= 1;
                bot.repath = 3.0;
                bot.path = nav.path(&mut scratch, feet, bot.goal).unwrap_or_default();
            }
            while bot.path.first().is_some_and(|p| p.xz().distance(feet.xz()) < 0.6 && (p.y - feet.y).abs() < 1.6) {
                bot.path.remove(0);
            }
            if bot.path.is_empty() && bot.goal.xz().distance(feet.xz()) > 2.0 {
                // No route found from here (off the walkable grid, say): head straight for it, and
                // let the unsticking take over if that goes nowhere.
                wish = (bot.goal.xz() - feet.xz()).normalize_or_zero();
                input.yaw = turn(input.yaw, yaw_to(wish), 6.0 * dt);
            }
            if bot.path.is_empty()
                && let Some((at, _)) = bot.loot
                && at.xz().distance(feet.xz()) < 2.0
            {
                // The last step onto the thing it wants.
                let to = at.xz() - feet.xz();
                wish = if to.length() > 0.08 { to.normalize_or_zero() * (to.length() / 0.5).min(1.0) * 0.6 } else { Vec2::ZERO };
            }
            if let Some(next) = bot.path.first() {
                wish = (next.xz() - feet.xz()).normalize_or_zero();
                // Into or out of a narrow passage: sweep the view across both corners on the move.
                let glance = 0.75 * (bot.glance * 9.0).sin() * (bot.glance * 2.0).min(1.0);
                bot.glance = (bot.glance - dt).max(0.0);
                input.yaw = turn(input.yaw, yaw_to(wish) + glance, 6.0 * dt);
                input.pitch *= 1.0 - (4.0 * dt).min(1.0);
                if bot.sentry && bot.alert <= 0.0 && !bot.choke {
                    // An unhurried patrol (but never loitering in a doorway).
                    wish *= 0.45;
                } else {
                    held |= buttons::SPRINT;
                }
            } else if bot.sentry {
                // Standing guard: look about.
                input.yaw += (bot.strafe * 0.5 + if bot.alert > 0.0 { bot.strafe * 1.2 } else { 0.0 }) * dt;
            }
            if let Some(from) = bot.threat_at(now, 2.5) {
                // Shot at from somewhere it cannot see: face it at once, and never stand still for it.
                let to = from - (feet + Vec3::Y * input.eye());
                input.yaw = turn(input.yaw, yaw_to(to.xz()), 10.0 * dt);
                input.pitch += ((to.y / to.length().max(1e-3)).asin() - input.pitch).clamp(-4.0 * dt, 4.0 * dt);
                if wish == Vec2::ZERO {
                    wish = to.xz().normalize_or(Vec2::X).perp() * bot.strafe;
                }
            }
            // Return fire at a gunshot it heard (or a round that came at it) but cannot see the shooter of.
            if bot.suppress > 0.0 {
                let to = bot.suppress_at - (feet + Vec3::Y * input.eye());
                let distance = to.length().max(1e-3);
                input.yaw = turn(input.yaw, yaw_to(to.xz()) + bot.aim_error.x * 2.0, 10.0 * dt);
                input.pitch = (to.y / distance).asin() + bot.aim_error.y;
                // Sidestepping while it shoots back, not standing for it.
                wish = to.xz().normalize_or(Vec2::X).perp() * bot.strafe * 0.6;
                held &= !buttons::SPRINT;
                if !input.held(buttons::FIRE) {
                    held |= buttons::FIRE;
                }
                if distance > 12.0 {
                    held |= buttons::AIM;
                }
            }
        }

        // Fighting from cover: tucked in crouched, standing up (or stepping out) to shoot,
        // back in to reload. Moving between the two upright, never sprinting.
        if let Some(cover) = bot.cover.filter(|_| engaged && bot.grab.is_none()) {
            bot.peek -= dt;
            if bot.peek <= 0.0 {
                bot.exposed = !bot.exposed;
                bot.peek = if bot.exposed { rng.0.range(1.2, 2.4) } else { rng.0.range(0.6, 1.2) };
            }
            let out = bot.exposed && cover.shot && arsenal.reload <= 0.0;
            let point = if out { cover.peek } else { cover.spot };
            let across = (bot.foe - feet).xz().normalize_or(Vec2::X).perp() * bot.strafe;
            wish = approach(feet, point, across);
            let there = point.xz().distance(feet.xz()) < 0.6;
            wanted_stance = if out || !there { stance::STAND } else { stance::CROUCH };
            held &= !buttons::SPRINT;
            let mag = content.weapons.get(arsenal.current as usize).map_or(0, |w| w.mag);
            if !out && target.is_none() && arsenal.state().is_some_and(|w| w.ammo * 2 < mag) {
                held |= buttons::RELOAD;
            }
        }
        // Camping: settle in at the spot (low, unless it has to stand to see), watching the approach.
        if let Plan::Camp { face } = bot.plan
            && bot.target.is_none()
            && !engaged
            && !bot.sentry
            && !bot.under_fire(now, 2.5)
            && bot.goal.xz().distance(feet.xz()) < 1.5
        {
            wish = Vec2::ZERO;
            held &= !buttons::SPRINT;
            wanted_stance = if bot.camp_spot.is_some_and(|(.., low)| !low) { stance::STAND } else { stance::CROUCH };
            let scan = (now * 0.5 + me.id as f32).sin() * 0.7;
            input.yaw = turn(input.yaw, face + scan, 2.0 * dt);
            input.pitch *= 1.0 - (3.0 * dt).min(1.0);
        }
        // Planting or defusing: stay put, low.
        if bot.hold_use && bot.target.is_none() {
            wish = Vec2::ZERO;
            held &= !buttons::SPRINT;
            wanted_stance = stance::CROUCH;
        }
        // Retreating or hunting a known spot: no sprinting into the open when hurt.
        if bot.plan == Plan::Retreat && bot.target.is_some() {
            // Backing off while still shooting: away from the target (it is in front).
            let ahead = Vec2::new(-input.yaw.sin(), -input.yaw.cos());
            wish = -ahead * 0.8 + ahead.perp() * bot.strafe * 0.4;
            wanted_stance = stance::CROUCH;
        }
        // Doing nothing - no headway, no fight, nothing held on purpose - for too long:
        // shake out of it (a new plan, somewhere else to hold).
        let busy = engaged || bot.hold_use || bot.suppress > 0.0 || (bot.sentry && !bot.under_fire(now, 4.0)) || arsenal.reload > 0.0;
        if busy || feet.distance(bot.idle_from) > 2.0 {
            bot.idle = 0.0;
            bot.idle_from = feet;
        } else {
            bot.idle += dt;
            if bot.idle > 6.0 {
                bot.idle = 0.0;
                bot.shifts = bot.shifts.wrapping_add(1);
                bot.plan_until = 0.0;
                bot.think = 0.0;
                bot.camp_spot = None;
                bot.objective = None;
                bot.heard = None;
                if matches!(bot.plan, Plan::Camp { .. }) {
                    bot.plan = Plan::Search;
                }
                if bot.goal.xz().distance(feet.xz()) < 3.0 && !bot.sentry && state.obj.kind == crate::objective::kind::NONE {
                    bot.goal = sweep_point(&map, &mut rng);
                    bot.path.clear();
                }
            }
        }
        // Gear in hand: stims and ammo packs on a press; grenades cooked a moment and lobbed.
        if bot.throw > 0.0 {
            bot.throw -= dt;
            let throwable = content.gear.get(me.gear as usize).is_some_and(|g| crate::combat::throw(&g.kind, Vec3::NEG_Z).is_some());
            if throwable {
                let to = bot.throw_at - eye;
                input.yaw = turn(input.yaw, yaw_to(to.xz()), 10.0 * dt);
                input.pitch = lob_pitch(to.xz().length()) + ((to.y / to.length().max(1e-3)).asin() * 0.5).clamp(-0.3, 0.3);
            }
            held &= !(buttons::FIRE | buttons::SPRINT);
            if bot.throw > 0.0 && (throwable || bot.throw > 0.05) {
                held |= buttons::GEAR;
            }
        }
        // Stuck against something: stand up, jump (or vault) once upright, and look for a
        // fresh route. Low or lying down, it gets up first; a body that cannot stand never tries.
        // Still no headway after that, it works through the ways out in turn: step aside,
        // back off and search again from where it stands, then give the destination up for a
        // new one. (A closed door in the way is pushed open by `breakables::work_doors`.)
        let legs = me.lost & crate::player::limb::LEGS;
        let travelling = wish != Vec2::ZERO && !bot.holding();
        if !travelling || feet.distance(bot.anchor) > 0.6 {
            bot.anchor = feet;
            bot.anchor_time = 0.0;
        } else {
            bot.anchor_time += dt;
            // Going nowhere for a second however busy the legs: as good as stuck.
            if bot.anchor_time > 1.0 {
                bot.stuck = bot.stuck.max(bot.anchor_time - 0.4);
            }
        }
        if wish != Vec2::ZERO && (moved < 0.5 || bot.anchor_time > 1.0) {
            let before = bot.stuck;
            bot.stuck += dt;
            if bot.stuck > 0.6 {
                bot.repath = 0.0;
                if before <= 0.6 {
                    bot.strafe = -bot.strafe;
                }
                if legs == 0 && bot.stuck < UNSTICK_SIDESTEP {
                    wanted_stance = stance::STAND;
                    held |= buttons::JUMP;
                }
            }
            let step = unstick(wish, bot.stuck, bot.strafe);
            if step != wish {
                wish = step;
                held &= !buttons::SPRINT;
                wanted_stance = if legs == 0 { stance::STAND } else { wanted_stance };
            }
            if bot.stuck > UNSTICK_GIVE_UP {
                // Nothing worked: forget the route and the goal, and think again.
                bot.path.clear();
                bot.plan_until = 0.0;
                bot.cover = None;
                bot.stuck = 0.0;
                bot.anchor_time = 0.0;
                bot.strafe = -bot.strafe;
                if !bot.sentry {
                    bot.goal = feet + Vec3::new(rng.0.range(-12.0, 12.0), 0.0, rng.0.range(-12.0, 12.0));
                }
            }
        } else {
            bot.stuck = (bot.stuck - dt).max(0.0);
        }
        // Stand up to sprint.
        if held & buttons::SPRINT != 0 {
            wanted_stance = stance::STAND;
        }
        // Legs shot off: crawl (both) or stay low (one).
        if legs == crate::player::limb::LEGS {
            wanted_stance = stance::PRONE;
        } else if legs != 0 && wanted_stance == stance::STAND {
            wanted_stance = stance::CROUCH;
        }
        // In the air or going over something: no getting down (or up) until it lands.
        if mover.vault.is_some() || (!mover.grounded && !mover.climbing) {
            wanted_stance = input.stance;
        }
        // Jump only from a settled, upright stance on the ground: never from a crouch, prone, or mid-change.
        let upright = input.stance == stance::STAND && mover.stance_was == stance::STAND && mover.stance_age > 0.15 && mover.grounded && mover.vault.is_none();
        if wanted_stance != stance::STAND || !upright {
            held &= !buttons::JUMP;
        }
        input.stance = wanted_stance;

        // World-space wish -> stick input relative to where the bot is looking.
        let forward = Vec2::new(-input.yaw.sin(), -input.yaw.cos());
        let right = Vec2::new(input.yaw.cos(), -input.yaw.sin());
        input.movement = Vec2::new(wish.dot(right), wish.dot(forward)).clamp_length_max(1.0);
        input.buttons = held;
        // Planting or defusing: only with nobody to shoot at.
        let using = bot.hold_use && bot.target.is_none();
        if input.interact != using {
            input.interact = using;
        }
        // Bots light up at night (it gives them away, as it does anyone).
        if input.light != dark {
            input.light = dark;
        }
    }
    noises.0 = shouts;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stuck_bot_tries_ahead_then_aside_then_back() {
        let wish = Vec2::new(0.0, 1.0);
        assert_eq!(unstick(wish, 0.5, 1.0), wish);
        let aside = unstick(wish, UNSTICK_SIDESTEP + 0.1, 1.0);
        assert!(aside.dot(wish).abs() < 0.3 && aside.length() > 0.9, "{aside}");
        assert!(unstick(wish, UNSTICK_SIDESTEP + 0.1, -1.0).dot(aside) < 0.0, "the other side");
        assert_eq!(unstick(wish, UNSTICK_BACK_OFF + 0.1, 1.0), -wish);
        assert!(UNSTICK_SIDESTEP < UNSTICK_BACK_OFF && UNSTICK_BACK_OFF < UNSTICK_GIVE_UP);
    }
    use crate::player::EYE;
    use oz_core::{collide::Aabb, worldgen::COVER_HEIGHT};

    /// Flat open ground holding only `boxes` (centre on the ground, half extents).
    fn ground(boxes: &[(Vec2, Vec3)]) -> StaticWorld {
        let mut statics = StaticWorld::new(64.0);
        for (at, half) in boxes {
            statics.add(Aabb::new(Vec3::new(at.x, half.y, at.y), *half));
        }
        statics
    }

    fn search(statics: &StaticWorld, feet: Vec3, threat: Vec3, seek: Seek) -> (Option<Cover>, u32) {
        let mut rays = 0;
        let mut clear = |a: Vec3, b: Vec3| {
            rays += 1;
            let d = a.distance(b);
            statics.raycast(a, (b - a) / d, d).is_none_or(|(hit, _)| hit >= d - 0.3)
        };
        let stand = |xz: Vec2, _: f32| (!statics.blocked(xz.x, xz.y, 0.0, stance::height(stance::STAND), 0.3)).then_some(Vec3::new(xz.x, 0.0, xz.y));
        let cover = find_cover(feet, Contact::at(threat, stance::STAND), &[], seek, (9.0, 28.0), None, &[], statics, &mut clear, &stand, &|_, _| true);
        (cover, rays)
    }

    fn sees(statics: &StaticWorld, a: Vec3, b: Vec3) -> bool {
        let d = a.distance(b);
        statics.raycast(a, (b - a) / d, d).is_none_or(|(hit, _)| hit >= d - 0.3)
    }

    /// A waist-high wall along X at the origin, an enemy 18 m north of it, the bot out in the open beside it.
    fn wall() -> (StaticWorld, Vec3, Vec3) {
        (ground(&[(Vec2::ZERO, Vec3::new(1.6, COVER_HEIGHT * 0.5, 0.25))]), Vec3::new(3.5, 0.0, -2.0), Vec3::new(0.0, 0.0, 18.0))
    }

    #[test]
    fn exposed_bot_tucks_in_behind_low_cover_and_shoots_over_it() {
        let (statics, bot, enemy) = wall();
        let threat = Contact::at(enemy, stance::STAND);
        assert!(sees(&statics, threat.eye, bot + Vec3::Y * TUCKED[0]), "the test bot should start in the open");
        let (cover, rays) = search(&statics, bot, enemy, Seek::Fight);
        let cover = cover.expect("a spot behind the wall");
        assert!(cover.spot.z < 0.0 && cover.spot.x.abs() < 2.0, "{cover:?} is not behind the wall");
        assert!(TUCKED.iter().all(|h| !sees(&statics, threat.eye, cover.spot + Vec3::Y * *h)), "{cover:?}: a crouched body there is still seen");
        assert!(cover.shot && sees(&statics, cover.peek + Vec3::Y * EYE, threat.chest), "{cover:?}: no shot from it");
        assert!(rays <= COVER_RAYS + 8, "search cost {rays} rays");
    }

    #[test]
    fn far_off_bots_only_take_cover_that_closes_the_distance() {
        let (statics, _, enemy) = wall();
        // Already tucked behind the wall, but well out of range: pushing finds nothing closer here.
        let (cover, _) = search(&statics, Vec3::new(0.0, 0.0, -1.0), enemy + Vec3::Z * 20.0, Seek::Push);
        assert_eq!(cover, None);
    }

    #[test]
    fn open_ground_offers_no_cover() {
        let (cover, _) = search(&ground(&[]), Vec3::ZERO, Vec3::new(0.0, 0.0, 18.0), Seek::Fight);
        assert_eq!(cover, None);
    }

    #[test]
    fn holding_bots_watch_from_behind_something() {
        let (statics, bot, enemy) = wall();
        let cover = search(&statics, bot, enemy, Seek::Hold).0.expect("somewhere to watch from");
        let threat = Contact::at(enemy, stance::STAND);
        assert!(!sees(&statics, threat.eye, cover.spot + Vec3::Y * 0.6), "{cover:?}: the body is in plain view");
        // The wall is too tall to see over crouched, so it watches standing.
        assert!(!cover.low && sees(&statics, cover.spot + Vec3::Y * EYE, threat.chest), "{cover:?}");
    }

    #[test]
    fn a_bot_killed_twice_from_one_spot_comes_round_the_side_next_time() {
        let mut bot = Bot::default();
        let (from, goal, killer) = (Vec3::new(0.0, 0.0, -40.0), Vec3::new(0.0, 0.0, 40.0), Vec3::new(1.0, 0.0, 10.0));
        let flat = |_: f32, _: f32| Some(0.0);
        // Once is bad luck: straight there.
        bot.deaths.push((Vec3::new(0.0, 0.0, -5.0), killer, 10.0));
        assert_eq!(route_round(&mut bot, from, goal, 20.0, flat), goal);
        // Twice from the same corner: the way past it is out to one side.
        bot.deaths.push((Vec3::new(2.0, 0.0, -3.0), killer + Vec3::X * 3.0, 30.0));
        let via = route_round(&mut bot, from, goal, 40.0, flat);
        assert!(off_line(killer, from, via) > 8.0 && via.xz().distance(killer.xz()) > 15.0, "{via} still runs past {killer}");
        // It keeps to the detour until it gets there, then goes on in.
        assert_eq!(route_round(&mut bot, from + Vec3::X, goal, 41.0, flat), via);
        assert_eq!(route_round(&mut bot, via, goal, 50.0, flat), goal);
        // A way that does not pass the spot is left alone.
        let mut other = Bot { deaths: bot.deaths.clone(), ..default() };
        assert_eq!(route_round(&mut other, Vec3::new(60.0, 0.0, 0.0), Vec3::new(60.0, 0.0, 50.0), 40.0, flat), Vec3::new(60.0, 0.0, 50.0));
    }

    #[test]
    fn gaps_between_buildings_are_narrow_and_streets_are_not() {
        // Two buildings 3 m apart along X leave a north-south alley at the origin.
        let statics = ground(&[(Vec2::new(-4.5, 0.0), Vec3::new(3.0, 3.0, 4.0)), (Vec2::new(4.5, 0.0), Vec3::new(3.0, 3.0, 4.0))]);
        assert!(narrow(&statics, Vec3::ZERO));
        assert!(!narrow(&statics, Vec3::new(0.0, 0.0, 12.0)));
        assert!(!narrow(&statics, Vec3::new(20.0, 0.0, 0.0)));
    }
}
