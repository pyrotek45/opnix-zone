//! Mountainside: a fixed map on a slope. Five levels climb from a village in the valley
//! (south) to a lodge and radio station on the summit (north), each a flat terrace held up by a
//! rock cliff. The team bases sit at either end of the middle terrace, so both teams can fight
//! uphill or down.
//!
//! Everything joins up: a switchback road climbs each flank - Main Street through the village,
//! up a ramp, along a terrace, up the next - and ends where roads end, in the market square on
//! the middle terrace, at the bases and in the lodge's forecourt on the summit. Stairs cut into
//! the cliffs join footpaths that run along the terraces to the road. Every house faces its
//! street or path, with a path to its door and, in the village, sometimes a driveway.

use super::{
    Area, AreaKind, COVER_HEIGHT, Clutter, ClutterKind, Prop, PropKind, World,
    shapes::{Furnish, Plan, Roof},
};
use crate::{collide::Aabb, model::Paint, noise, rng::Rng};
use glam::{Vec2, Vec3};
use std::f32::consts::TAU;

/// Level heights, valley to summit.
pub(super) const LEVELS: [f32; 5] = [1.2, 6.2, 11.2, 16.2, 21.2];
/// Where each cliff stands (z), between level i and i + 1.
pub(super) const CLIFFS: [f32; 4] = [-62.0, -24.0, 14.0, 52.0];
/// Ramps (x) up each cliff: the switchback road climbs from the valley's ends toward the middle.
const RAMPS: [&[f32]; 4] = [&[-70.0, 70.0], &[-84.0, 84.0], &[-48.0, 48.0], &[-22.0, 22.0]];
/// Stairways (x) cut into each cliff - clear of the buildings at their feet and tops (the
/// valley's flight once ran up through the chapel).
pub(super) const STAIRS: [&[f32]; 4] = [&[9.0], &[-30.0, 30.0], &[0.0, -92.0, 92.0], &[-70.0, 70.0]];
/// The bases: either end of the middle terrace.
const BASE: (f32, f32) = (104.0, -5.0);
const RAMP_HALF: f32 = 5.0;
/// The lodge's forecourt on the summit: half its width, and its north edge (the lodge's front).
const FORECOURT: (f32, f32) = (12.0, 71.0);
/// The market square in the middle of the middle terrace: half its width and depth.
const SQUARE: (f32, f32) = (12.0, 9.0);
/// The footpaths along the terraces, (z, x from, x to): the lower one between the ramp roads,
/// the upper one from chalet to chalet past every flight of stairs, stopping at the kerbs of
/// the roads it crosses.
const PROMENADES: [(f32, f32, f32); 4] = [(-47.0, -67.0, 67.0), (26.0, -104.0, -51.0), (26.0, -45.0, 45.0), (26.0, 51.0, 104.0)];
pub(super) const STEP_RUN: f32 = 0.42;

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Where the paths from the foot and the top of the stairs at `s` up cliff `i` run to (z):
/// the road or footpath below, and the one above.
fn stair_paths(i: usize, s: f32) -> (f32, f32) {
    match i {
        0 => (level_mid(0) + 3.5, PROMENADES[0].0),
        1 => (PROMENADES[0].0, level_mid(2) - 3.0),
        // The middle flight comes down into the market square; the outer ones to the road.
        2 if s == 0.0 => (level_mid(2) + SQUARE.1 - 0.5, PROMENADES[1].0),
        2 => (level_mid(2) + 3.0, PROMENADES[1].0),
        _ => (PROMENADES[1].0, level_mid(4) + 2.0),
    }
}

/// Middle of each level's flat (z).
fn level_mid(i: usize) -> f32 {
    match i {
        0 => -86.0,
        4 => 64.0,
        _ => (CLIFFS[i - 1] + CLIFFS[i]) * 0.5,
    }
}

impl World {
    pub(super) fn mountain_height(&self, x: f32, z: f32) -> f32 {
        let seed = self.recipe.seed;
        let mut h = LEVELS[0];
        for (i, &b) in CLIFFS.iter().enumerate() {
            let rise = LEVELS[i + 1] - LEVELS[i];
            let mut t = smoothstep((z - (b - 1.5)) / 3.0);
            for &r in RAMPS[i] {
                let w = smoothstep((RAMP_HALF + 1.5 - (x - r).abs()) / 1.5);
                if w > 0.0 {
                    let ramp = ((z - (b - 9.0)) / 18.0).clamp(0.0, 1.0);
                    t = t + (ramp - t) * w;
                }
            }
            h += rise * t;
        }
        // Beyond the summit the mountain climbs on; ridges close the flanks.
        h += (z - 98.0).max(0.0) * 0.9 * (1.0 + 0.4 * noise::value(seed ^ 0x9ea, x * 0.05, z * 0.05));
        h += (x.abs() - 117.0).max(0.0) * 0.7 * (1.0 + 0.5 * noise::value(seed ^ 0x9eb, x * 0.06, z * 0.06));
        // A little texture on the terraces, nothing to trip on.
        h += 0.08 * noise::value(seed ^ 0x9ec, x * 0.2, z * 0.2);
        // The valley runs down to the sea in the south.
        if z < -96.0 {
            let shore = ((self.recipe.main_radius + 4.0 * noise::value(seed ^ 0x5a, x * 0.04, z * 0.04) + z) / 9.0).clamp(-1.0, 1.0);
            h = h.min(shore * 1.35 - 0.15);
        }
        h
    }

    pub(super) fn mountainside(&mut self, rng: &mut Rng) {
        self.cliffs(rng);
        self.switchback();
        self.valley_village(rng);
        self.lower_terrace(rng);
        self.middle_terrace(rng);
        self.upper_terrace(rng);
        self.summit(rng);
        self.mountain_greenery(rng);
        self.team_bases(BASE.0, BASE.1, Paint::Rock);
    }

    /// Rock faces along every cliff line, with gaps for the ramps and flush landings at the stairs,
    /// and parapets of rock along the tops.
    fn cliffs(&mut self, rng: &mut Rng) {
        for (i, &b) in CLIFFS.iter().enumerate() {
            let (lo, hi) = (LEVELS[i], LEVELS[i + 1]);
            let mut x = -126.0;
            while x < 126.0 {
                let mut end = (x + rng.range(5.0, 9.0)).min(126.0);
                // Stop short of a ramp, and jump past it.
                if let Some(&r) = RAMPS[i].iter().find(|r| end > **r - RAMP_HALF - 1.5 && x < **r + RAMP_HALF + 1.5) {
                    end = r - RAMP_HALF - 1.5;
                    if end - x > 0.5 {
                        self.cliff_block(rng, x, end, b, lo, hi, i);
                    }
                    x = r + RAMP_HALF + 1.5;
                    continue;
                }
                self.cliff_block(rng, x, end, b, lo, hi, i);
                x = end;
            }
            // Stairs cut into the face: eighteen steps up to the cliff top.
            for &s in STAIRS[i] {
                self.cliff_stairs(i, s, b, lo, hi);
            }
        }
    }

    /// A flight of steps up a cliff face at `x`, from the terrace at `lo` to the top at `hi`:
    /// solid steps from the ground up, kerbed cheeks either side that run down into the ground
    /// (low cover, drawn as one smooth sloping wall), hand rails on posts, a landing pad at the
    /// foot and a gravel path out from it.
    fn cliff_stairs(&mut self, i: usize, s: f32, b: f32, lo: f32, hi: f32) {
        let n = ((hi - lo) / 0.29).ceil() as usize;
        let rise = (hi - lo) / n as f32;
        let front = b - 1.6;
        let foot = front - n as f32 * STEP_RUN;
        for k in 0..n {
            let z1 = front - (n - 1 - k) as f32 * STEP_RUN;
            let top = lo + (k + 1) as f32 * rise;
            self.solid(Aabb::span(Vec3::new(s - 1.3, lo - 0.6, z1 - STEP_RUN), Vec3::new(s + 1.3, top, z1)), if k % 2 == 0 { Paint::Concrete } else { Paint::Rock });
            // Nosing on each tread.
            self.trim(Vec3::new(s - 1.3, top - 0.04, z1 - STEP_RUN - 0.01), Vec3::new(s + 1.3, top + 0.005, z1 - STEP_RUN + 0.05), Paint::Slate);
        }
        // The cheeks: hidden kerbs a step's height over the treads, drawn as one sloping wall
        // from below the ground at the foot to the cliff top.
        for side in [-1.0f32, 1.0] {
            let (a, c) = (s + side * 1.3, s + side * 1.62);
            let (x0, x1) = (a.min(c), a.max(c));
            for k in 0..n {
                let z1 = front - (n - 1 - k) as f32 * STEP_RUN;
                let top = lo + (k + 1) as f32 * rise + 0.35;
                self.blocks.push(super::Block { aabb: Aabb::span(Vec3::new(x0, lo - 0.6, z1 - STEP_RUN), Vec3::new(x1, top, z1)), paint: Paint::Rock, solid: true, hidden: true });
            }
            let mut points = Vec::new();
            for x in [x0 - 0.02, x1 + 0.02] {
                points.extend([
                    Vec3::new(x, lo - 0.6, foot - 0.02),
                    Vec3::new(x, lo + rise + 0.37, foot - 0.02),
                    Vec3::new(x, lo - 0.6, front + 0.3),
                    Vec3::new(x, hi + 0.37, front + 0.3),
                    Vec3::new(x, hi + 0.37, front - STEP_RUN),
                ]);
            }
            self.roofs.push(crate::model::Hull { paint: Paint::Concrete, points, anim: crate::model::Anim::None });
            // A hand rail on posts along the cheek.
            let rail_x = s + side * 1.46;
            let at = |z: f32| {
                let k = ((z - foot) / STEP_RUN).clamp(0.0, n as f32 - 0.01);
                lo + (k.floor() + 1.0) * rise + 0.35
            };
            let posts = 5;
            for j in 0..=posts {
                let z = foot + 0.2 + (front - foot - 0.4) * j as f32 / posts as f32;
                let y = at(z);
                self.trim_rod(Vec3::new(rail_x, y, z), Vec3::new(rail_x, y + 0.9, z), 0.03, Paint::Iron);
            }
            let (za, zb) = (foot + 0.2, front - 0.2);
            self.trim_rod(Vec3::new(rail_x, at(za) + 0.9, za), Vec3::new(rail_x, at(zb) + 0.9, zb), 0.035, Paint::Iron);
        }
        // A paved landing at the foot and a gravel path on out across the terrace.
        let g = self.height(s, foot - 1.0);
        self.flat(s - 1.7, s + 1.7, foot - 1.6, foot, g + 0.01, Paint::Concrete);
        self.areas.push(Area { lo: Vec2::new(s - 1.7, foot - 1.6), hi: Vec2::new(s + 1.7, b + 1.8), kind: AreaKind::Steps });
        // And at the top, the path goes on north. Both run to the road or footpath they join.
        let (down, up) = stair_paths(i, s);
        if foot - 1.6 - down > 0.3 {
            self.road(Vec2::new(s, foot - 1.6), Vec2::new(s, down), 1.8, Paint::Sand, false);
        }
        self.road(Vec2::new(s, b + 1.8), Vec2::new(s, up), 1.8, Paint::Sand, false);
        self.scree(Vec2::new(s - 2.6, foot + 0.8), 1.2, 6, Paint::Rock);
        self.scree(Vec2::new(s + 2.6, foot + 0.8), 1.2, 6, Paint::Rock);
    }

    /// One run of cliff from `x0` to `x1` at cliff `b`: the face, and sometimes a parapet on top.
    #[allow(clippy::too_many_arguments)]
    fn cliff_block(&mut self, rng: &mut Rng, x0: f32, x1: f32, b: f32, lo: f32, hi: f32, i: usize) {
        let front = b - 1.6 - rng.range(0.0, 0.7);
        let paint = if rng.chance(0.5) { Paint::Rock } else { Paint::Concrete };
        let from = self.blocks.len();
        self.solid(Aabb::span(Vec3::new(x0, lo - 0.6, front), Vec3::new(x1, hi, b + 1.6)), paint);
        // Strata: the old paint-thin lines are drawn as the rock's two tones now.
        for _ in 1..3 {
            let _ = rng.range(-0.3, 0.3);
        }
        let stairs = STAIRS[i].iter().any(|s| x1 > s - 2.2 && x0 < s + 2.2);
        if !stairs && rng.chance(0.45) && x1 - x0 > 2.0 {
            let (a, c) = (x0 + rng.range(0.0, 0.6), x1 - rng.range(0.0, 0.6));
            self.solid(Aabb::span(Vec3::new(a, hi - 0.1, front), Vec3::new(c, hi + COVER_HEIGHT, front + 0.9)), Paint::Rock);
        }
        // Drawn as weathered rock, its strata showing as a darker band, with scree at the foot.
        let face_paint = if paint == Paint::Concrete { Paint::Slate } else { Paint::Rock };
        self.rock_skin(from, face_paint);
        let mut detail = self.detail_rng(Vec2::new(x0, b), 0xc1f);
        let band = lo + (hi - lo) * detail.range(0.35, 0.6);
        self.trim(Vec3::new(x0 + 0.05, band, front - 0.06), Vec3::new(x1 - 0.05, band + 0.22, front + 0.2), Paint::Slate);
        if detail.chance(0.6) && !stairs {
            self.scree(Vec2::new((x0 + x1) * 0.5, front - 0.9), (x1 - x0) * 0.4, 5, Paint::Rock);
        }
        // Moss and a shrub or two along the top.
        if detail.chance(0.5) && !stairs {
            let p = Vec2::new(detail.range(x0 + 0.5, x1 - 0.5), b + 2.6);
            self.flowers(p, &mut detail);
        }
    }

    /// The switchback road, one smooth climb on each flank: Main Street through the village,
    /// up the ramps to the terraces, out to the bases on the middle one, and over the summit
    /// into the lodge's forecourt. Each terrace's footpath joins the stairs to it.
    fn switchback(&mut self) {
        let asphalt = Paint::Dark;
        // Main Street, end to end of the valley.
        self.road(Vec2::new(-112.0, level_mid(0)), Vec2::new(112.0, level_mid(0)), 7.0, asphalt, true);
        for s in [-1.0f32, 1.0] {
            let mut x = RAMPS[0][1] * s;
            let mut z = level_mid(0);
            for (i, &b) in CLIFFS.iter().enumerate() {
                let rx = RAMPS[i].iter().copied().find(|r| r.signum() == s).unwrap_or(x);
                // Along the level to the foot of the ramp, then up it.
                if (rx - x).abs() > 0.1 {
                    self.road(Vec2::new(x, z), Vec2::new(rx, z), 6.0, asphalt, true);
                }
                self.road(Vec2::new(rx, z), Vec2::new(rx, b - 9.0), 6.0, asphalt, true);
                self.ramp_road(rx, b - 9.0, b + 9.0);
                self.areas.push(Area { lo: Vec2::new(rx - RAMP_HALF, b - 9.5), hi: Vec2::new(rx + RAMP_HALF, b + 9.5), kind: AreaKind::Steps });
                let next = level_mid(i + 1);
                self.road(Vec2::new(rx, b + 9.0), Vec2::new(rx, next), 6.0, asphalt, true);
                x = rx;
                z = next;
            }
            // Over the summit into the lodge's forecourt.
            self.road(Vec2::new(x, z), Vec2::new(s * FORECOURT.0, z), 6.0, asphalt, true);
            // The middle terrace: out to the base, and in to the market square.
            let mid = level_mid(2);
            self.road(Vec2::new(s * RAMPS[1][1], mid), Vec2::new(s * (BASE.0 - 12.0), mid), 6.0, asphalt, true);
            self.road(Vec2::new(s * RAMPS[2][1], mid), Vec2::new(s * SQUARE.0, mid), 6.0, asphalt, true);
        }
        // The lodge's forecourt and the market square, where the roads end.
        let (fx, fz) = FORECOURT;
        self.ground_paint(-fx, fx, level_mid(4) - 3.5, fz, Paint::Concrete);
        self.areas.push(Area { lo: Vec2::new(-fx, level_mid(4) - 3.5), hi: Vec2::new(fx, fz), kind: AreaKind::Parking });
        let (sx, sz) = SQUARE;
        let mid = level_mid(2);
        self.ground_paint(-sx, sx, mid - sz, mid + sz, Paint::Slate);
        self.areas.push(Area { lo: Vec2::new(-sx, mid - sz), hi: Vec2::new(sx, mid + sz), kind: AreaKind::Plaza });
        // The bases' pads.
        for s in [-1.0f32, 1.0] {
            let (a, c) = (s * (BASE.0 - 11.0), s * (BASE.0 + 8.0));
            self.areas.push(Area { lo: Vec2::new(a.min(c), BASE.1 - 18.0), hi: Vec2::new(a.max(c), BASE.1 + 14.0), kind: AreaKind::Base });
        }
        // Footpaths along the terraces, from stairs to stairs and on to the road.
        for (z, x0, x1) in PROMENADES {
            self.road(Vec2::new(x0, z), Vec2::new(x1, z), 1.8, Paint::Sand, false);
        }
    }

    /// A gravel path (one axis) from `a` to `b`.
    fn path(&mut self, a: Vec2, b: Vec2) {
        // Narrow enough to pass between a balcony's posts.
        self.road(a, b, 1.1, Paint::Sand, false);
    }

    /// The doorway built since door `from` on a face at `z` (a south or north wall), if any.
    fn door_at(&self, from: usize, z: f32) -> Option<Vec2> {
        self.doors[from..].iter().filter(|d| d.along_x && (d.center.z - z).abs() < 0.6).map(|d| Vec2::new(d.center.x, d.center.z)).next()
    }

    /// A house facing the street on the side `front` (0 south, 3 north) with a path from its
    /// door to the kerb at `kerb` (z), and sometimes a driveway beside it with the car in it.
    #[allow(clippy::too_many_arguments)]
    fn street_house(&mut self, rng: &mut Rng, at: Vec2, half: Vec2, floors: u32, front: usize, kerb: f32, driveway: Option<f32>) {
        let from = self.doors.len();
        let dir = if front == 0 { -1.0 } else { 1.0 };
        let face = at.y + dir * half.y;
        self.house(rng, at, half, floors, front);
        if let Some(door) = self.door_at(from, face) {
            // Past the porch, if it has one, then down to the street.
            self.path(Vec2::new(door.x, face + dir * 0.2), Vec2::new(door.x, kerb));
        }
        if let Some(side) = driveway {
            let x = at.x + side * (half.x + 2.0);
            let (z0, z1) = (kerb.min(face - dir * 3.0), kerb.max(face - dir * 3.0));
            self.ground_paint(x - 1.6, x + 1.6, z0, z1, Paint::Concrete);
            self.areas.push(Area { lo: Vec2::new(x - 1.6, z0), hi: Vec2::new(x + 1.6, z1), kind: AreaKind::Parking });
            if rng.chance(0.6) {
                let p = Vec2::new(x, (z0 + z1) * 0.5);
                self.clutter.push(Clutter { kind: ClutterKind::Car, pos: Vec3::new(p.x, self.height(p.x, p.y), p.y), turned: false });
            }
        }
    }

    /// A car at the kerb of an east-west road at `z` (`side` the kerb's side), if there is room.
    fn kerb_car(&mut self, x: f32, z: f32, width: f32, side: f32) {
        let p = Vec2::new(x, z + side * (width * 0.5 - 1.5));
        if !self.occupied(p, 2.4) && self.clutter.iter().all(|c| Vec2::new(c.pos.x, c.pos.z).distance(p) > 6.0) {
            self.clutter.push(Clutter { kind: ClutterKind::Car, pos: Vec3::new(p.x, self.height(p.x, p.y), p.y), turned: true });
        }
    }

    /// Road up a ramp, laid smooth on the slope.
    fn ramp_road(&mut self, x: f32, z0: f32, z1: f32) {
        self.drape(Vec2::new(x - 3.0, z0), Vec2::new(x + 3.0, z1), 0.024, Paint::Dark);
        // Crash barriers down both sides: cover, in metre-long pieces that follow the slope
        // (hidden), drawn as one smooth concrete barrier on top of them.
        for s in [-1.0, 1.0] {
            let bx = x + s * 3.6;
            let mut z = z0 + 1.0;
            while z < z1 - 1.0 {
                let (ha, hb) = (self.height(bx, z), self.height(bx, z + 1.0));
                let low = ha.min(hb);
                self.blocks.push(super::Block { aabb: Aabb::span(Vec3::new(bx - 0.15, low - 0.3, z), Vec3::new(bx + 0.15, ha.max(hb) + COVER_HEIGHT, z + 1.0)), paint: Paint::Metal, solid: true, hidden: true });
                let mut points = Vec::new();
                for (zz, h) in [(z, ha), (z + 1.0, hb)] {
                    points.extend([
                        Vec3::new(bx - 0.32, low - 0.3, zz),
                        Vec3::new(bx + 0.32, low - 0.3, zz),
                        Vec3::new(bx - 0.3, h + 0.25, zz),
                        Vec3::new(bx + 0.3, h + 0.25, zz),
                        Vec3::new(bx - 0.13, h + COVER_HEIGHT + 0.02, zz),
                        Vec3::new(bx + 0.13, h + COVER_HEIGHT + 0.02, zz),
                    ]);
                }
                self.roofs.push(crate::model::Hull { paint: Paint::Concrete, points, anim: crate::model::Anim::None });
                // A reflector stripe every few metres.
                if (z - z0) as i32 % 4 == 1 {
                    self.trim(Vec3::new(bx - 0.18, ha + 0.9, z + 0.45), Vec3::new(bx + 0.18, ha + 1.0, z + 0.55), Paint::Gold);
                }
                z += 1.0;
            }
        }
        // Retaining walls where the ramp's sides fall away to the terrace below: down from the
        // ramp's edge to the ground, between the ramp and the cliff on either side.
        for s in [-1.0, 1.0] {
            let wx = x + s * (RAMP_HALF + 0.4);
            let mut z = z0;
            while z < z1 {
                let top = self.height(x + s * 3.0, z + 0.5).max(self.height(x + s * 3.0, z));
                let low = self.lowest(Vec2::new(wx, z + 0.5), Vec2::new(1.0, 0.5));
                if top - low > 0.6 {
                    let from = self.blocks.len();
                    self.solid(Aabb::span(Vec3::new(wx - 0.5, low - 0.4, z), Vec3::new(wx + 0.5, top + 0.1, z + 1.0)), Paint::Rock);
                    self.rock_skin(from, Paint::Rock);
                }
                z += 1.0;
            }
        }
    }

    /// The valley village: Main Street with a row of houses either side, each facing it with a
    /// path to its door and some with a driveway; the chapel and its bell tower on the north
    /// side looking over the village square across the street; cars at the kerbs.
    fn valley_village(&mut self, rng: &mut Rng) {
        let z = level_mid(0);
        let (north_kerb, south_kerb) = (z + 3.5, z - 3.5);
        // The square: paved, a fountain in the middle, opening onto Main Street.
        let (sq_lo, sq_hi) = (Vec2::new(-14.0, z - 18.0), Vec2::new(14.0, south_kerb));
        self.ground_paint(sq_lo.x, sq_hi.x, sq_lo.y, sq_hi.y, Paint::Slate);
        self.areas.push(Area { lo: sq_lo, hi: sq_hi, kind: AreaKind::Plaza });
        self.plaza_at(rng, Vec2::new(0.0, z - 11.0));
        for s in [-1.0f32, 1.0] {
            let p = Vec2::new(s * 10.5, z - 15.5);
            self.clutter.push(Clutter { kind: ClutterKind::Bench, pos: Vec3::new(p.x, self.height(p.x, p.y), p.y), turned: true });
        }
        // The chapel faces the square across the street, its tower beside it.
        let chapel = Vec2::new(0.0, z + 12.0);
        let from = self.doors.len();
        self.building_plan(rng, chapel, Vec2::new(5.0, 6.5), 2, &Plan { walls: Some(Paint::Bone), trim: Some(Paint::Steel), roof: Roof::Gable(Paint::Dark), furnish: Furnish::Plain, random_doors: false, doors: [Some((chapel.x, 1.7)), None, None, None], ladder: false, ..Plan::default() });
        if let Some(door) = self.door_at(from, chapel.y - 6.5) {
            self.road(Vec2::new(door.x, chapel.y - 6.7), Vec2::new(door.x, north_kerb), 2.4, Paint::Sand, false);
        }
        let tower = Vec2::new(-9.2, z + 9.0);
        let from = self.doors.len();
        let (_, top) = self.building_plan(rng, tower, Vec2::new(3.2, 3.0), 4, &Plan { walls: Some(Paint::Bone), trim: Some(Paint::Steel), roof: Roof::Flat, furnish: Furnish::Bare, random_doors: false, doors: [Some((tower.x, 1.7)), None, None, None], ladder: false, ..Plan::default() });
        if let Some(door) = self.door_at(from, tower.y - 3.0) {
            self.path(Vec2::new(door.x, tower.y - 3.2), Vec2::new(door.x, north_kerb));
        }
        self.decor(Aabb::span(Vec3::new(tower.x - 0.06, top + 0.3, tower.y - 0.06), Vec3::new(tower.x + 0.06, top + 5.0, tower.y + 0.06)), Paint::Gold);
        // The rows of houses, clear of the ramps and the stairs (north) and the square (south).
        for s in [-1.0f32, 1.0] {
            for (k, &x) in [20.0f32, 36.0, 52.0, 92.0, 108.0].iter().enumerate() {
                let half = Vec2::new(rng.range(4.2, 5.0), rng.range(3.8, 4.4));
                let at = Vec2::new(s * x, north_kerb + 4.5 + half.y);
                let floors = if rng.chance(0.4) { 2 } else { 1 };
                // Driveways where the gap to the next house allows.
                let driveway = matches!(k, 1 | 3).then_some(s);
                self.street_house(rng, at, half, floors, 0, north_kerb, driveway);
            }
            for (k, &x) in [24.0f32, 40.0, 56.0, 72.0, 88.0, 104.0].iter().enumerate() {
                let half = Vec2::new(rng.range(4.2, 5.0), rng.range(3.8, 4.4));
                let at = Vec2::new(s * x, south_kerb - 4.5 - half.y);
                let floors = if rng.chance(0.4) { 2 } else { 1 };
                let driveway = matches!(k, 0 | 2 | 4).then_some(s);
                self.street_house(rng, at, half, floors, 3, south_kerb, driveway);
            }
        }
        // Cars at the kerbs, clear of the junctions.
        for (x, side) in [(28.0f32, 1.0f32), (84.0, 1.0), (46.0, -1.0), (80.0, -1.0)] {
            for s in [-1.0, 1.0] {
                self.kerb_car(s * (x + rng.range(-1.0, 1.0)), z, 7.0, side);
            }
        }
    }

    /// The lower terrace: vineyard rows behind stone walls, a farmhouse and a barn either side
    /// of the stairs, the footpath along it.
    fn lower_terrace(&mut self, rng: &mut Rng) {
        let z = level_mid(1);
        let walk = PROMENADES[0].0;
        let barn = Vec2::new(-42.0, z + 6.0);
        self.barn(rng, barn, Vec2::new(7.5, 5.5));
        // From the barn's east doors along to the stairs' path.
        self.path(Vec2::new(barn.x + 7.7, barn.y - 0.5), Vec2::new(-30.0, barn.y - 0.5));
        // The yard in front of the barn doors.
        self.ground_paint(barn.x + 7.5, barn.x + 11.0, barn.y - 3.0, barn.y + 2.0, Paint::Sand);
        self.areas.push(Area { lo: Vec2::new(barn.x + 7.5, barn.y - 3.0), hi: Vec2::new(barn.x + 11.0, barn.y + 2.0), kind: AreaKind::Parking });
        self.street_house(rng, Vec2::new(42.0, z + 7.0), Vec2::new(5.0, 4.4), 2, 0, walk + 0.9, None);
        // Vine rows: low walls you can crouch behind, the length of the field.
        for side in [-1.0f32, 1.0] {
            for k in 0..4 {
                let rz = z - 12.0 + k as f32 * 6.0;
                if (rz - walk).abs() < 2.0 {
                    continue;
                }
                let (a, c) = (side * 48.0, side * 60.0);
                self.low_wall(Vec2::new(a.min(c), rz), Vec2::new(a.max(c), rz), COVER_HEIGHT, 0.5, Paint::Rock);
                let (a, c) = (side * 12.0, side * 24.0);
                self.low_wall(Vec2::new(a.min(c), rz), Vec2::new(a.max(c), rz), COVER_HEIGHT, 0.5, Paint::Rock);
            }
        }
        for _ in 0..10 {
            let p = Vec2::new(rng.range(-100.0, 100.0), z + rng.range(-14.0, 14.0));
            if !self.occupied(p, 2.0) && !self.near_road(p, 1.0) && !self.in_area(p, 2.0) {
                self.hay_stack(p, rng.chance(0.5), None);
            }
        }
    }

    /// The middle terrace: the bases at either end, the market square in the middle with the
    /// inn on it, garages with their driveways and a fuel stop on the road.
    fn middle_terrace(&mut self, rng: &mut Rng) {
        let z = level_mid(2);
        let kerb = z + 3.0;
        // The inn, looking onto the square.
        // A fountain in the middle of the square: cover, and it breaks the line between the bases.
        self.plaza_at(rng, Vec2::new(0.0, z));
        let inn = Vec2::new(-21.0, kerb + 6.5);
        let from = self.doors.len();
        let mut plan = Plan { walls: Some(Paint::Wood), roof: Roof::Gable(Paint::Rust), furnish: Furnish::Home, chimney: true, random_doors: false, ..Plan::default() };
        plan.doors[0] = Some((inn.x + 2.0, super::DOOR_WIDTH));
        plan.doors[3] = Some((inn.x - 2.0, super::DOOR_WIDTH));
        self.building_plan(rng, inn, Vec2::new(7.0, 5.5), 3, &plan);
        if let Some(door) = self.door_at(from, inn.y - 5.5) {
            self.path(Vec2::new(door.x, inn.y - 5.7), Vec2::new(door.x, kerb));
        }
        // Two garages, each with its driveway down to the road.
        for gx in [17.0, 24.0] {
            let g = Vec2::new(gx, kerb + 1.5 + 3.3);
            self.garage(rng, g, 0);
            self.ground_paint(gx - 1.6, gx + 1.6, kerb, g.y - 3.3, Paint::Concrete);
            self.areas.push(Area { lo: Vec2::new(gx - 1.6, kerb), hi: Vec2::new(gx + 1.6, g.y - 3.3), kind: AreaKind::Parking });
        }
        // The fuel stop: pumps (cover) under a canopy on posts, on a forecourt off the road.
        let pumps = Vec2::new(22.0, z - 12.0);
        let (lo, hi) = (Vec2::new(SQUARE.0, pumps.y - 7.0), Vec2::new(28.8, z - 3.0));
        self.ground_paint(lo.x, hi.x, lo.y, hi.y, Paint::Concrete);
        self.areas.push(Area { lo, hi, kind: AreaKind::Parking });
        let g = self.height(pumps.x, pumps.y);
        for dx in [-4.0, 0.0, 4.0] {
            self.solid(Aabb::span(Vec3::new(pumps.x + dx - 0.4, g, pumps.y - 0.3), Vec3::new(pumps.x + dx + 0.4, g + 1.5, pumps.y + 0.3)), Paint::Coral);
            self.decor(Aabb::span(Vec3::new(pumps.x + dx - 0.25, g + 1.0, pumps.y - 0.32), Vec3::new(pumps.x + dx + 0.25, g + 1.35, pumps.y - 0.3)), Paint::Glow);
        }
        for (dx, dz) in [(-6.0, -2.0), (6.0, -2.0), (-6.0, 2.0), (6.0, 2.0)] {
            self.solid(Aabb::span(Vec3::new(pumps.x + dx - 0.15, g, pumps.y + dz - 0.15), Vec3::new(pumps.x + dx + 0.15, g + 3.6, pumps.y + dz + 0.15)), Paint::Bone);
        }
        self.solid(Aabb::span(Vec3::new(pumps.x - 7.0, g + 3.6, pumps.y - 3.0), Vec3::new(pumps.x + 7.0, g + 4.0, pumps.y + 3.0)), Paint::Bone);
        self.decor(Aabb::span(Vec3::new(pumps.x - 7.02, g + 3.65, pumps.y - 3.02), Vec3::new(pumps.x + 7.02, g + 3.8, pumps.y + 3.02)), Paint::Red);
        // Cars filling up and waiting, lined up with the pumps.
        for dx in [-2.0, 2.0] {
            let p = Vec2::new(pumps.x + dx * 1.6, pumps.y + 3.4);
            self.clutter.push(Clutter { kind: ClutterKind::Car, pos: Vec3::new(p.x, self.height(p.x, p.y), p.y), turned: false });
        }
        // Cars at the kerbs out toward the bases.
        for x in [-74.0f32, -60.0, 62.0, 76.0] {
            self.kerb_car(x, z, 6.0, if x < 0.0 { 1.0 } else { -1.0 });
        }
        // Barrels, crates and bins for cover round the terrace.
        for (kind, n) in [(ClutterKind::Barrel, 6), (ClutterKind::Crates, 8), (ClutterKind::Bin, 3)] {
            let mut placed = 0;
            for _ in 0..n * 12 {
                if placed >= n {
                    break;
                }
                let p = Vec2::new(rng.range(-80.0, 80.0), z + rng.range(-14.0, 14.0));
                let reach = kind.half().x.max(kind.half().z) + 0.8;
                if self.occupied(p, reach) || self.on_lane(p, reach) || self.in_area(p, reach) || self.clutter.iter().any(|c| Vec2::new(c.pos.x, c.pos.z).distance(p) < reach + 2.0) {
                    continue;
                }
                self.clutter.push(Clutter { kind, pos: Vec3::new(p.x, self.height(p.x, p.y), p.y), turned: rng.chance(0.5) });
                placed += 1;
            }
        }
        for _ in 0..8 {
            let p = Vec2::new(rng.range(-80.0, 80.0), z + rng.range(-14.0, 12.0));
            if !self.occupied(p, 6.0) && !self.near_road(p, 3.0) && !self.in_area(p, 3.0) {
                let size = rng.range(1.2, 2.2);
                self.outcrop(rng, p, size);
            }
        }
    }

    /// The upper terrace: chalets facing the road and the footpath, the cable-car station on
    /// the path above the middle stairs, rocks and pines.
    fn upper_terrace(&mut self, rng: &mut Rng) {
        let z = level_mid(3);
        let walk = PROMENADES[1].0;
        for s in [-1.0f32, 1.0] {
            // Two by the road, facing it; one out past the stairs, facing the path.
            for (x, kerb) in [(35.0f32, z + 3.0), (82.0, walk + 0.9), (104.0, walk + 0.9)] {
                let at = Vec2::new(s * x, kerb + 7.5);
                let from = self.doors.len();
                self.cabin(rng, at, 0);
                // Its front door, on the south wall, and down off the porch to the road or path.
                let door = self.doors[from..].iter().find(|d| d.along_x && d.center.z < at.y).map(|d| Vec2::new(d.center.x, d.center.z));
                if let Some(door) = door {
                    self.path(Vec2::new(door.x, door.y - 2.4), Vec2::new(door.x, kerb));
                }
            }
        }
        // The cable-car station: a hall with a tall pylon beside it, its door on the path.
        let station = Vec2::new(0.0, walk + 0.9 + 6.0 + 5.0);
        let from = self.doors.len();
        let mut plan = Plan { walls: Some(Paint::Concrete), roof: Roof::Flat, furnish: Furnish::Plain, random_doors: false, ..Plan::default() };
        plan.doors[0] = Some((station.x - 2.0, super::DOOR_WIDTH));
        plan.doors[1] = Some((station.y, super::DOOR_WIDTH));
        self.building_plan(rng, station, Vec2::new(6.0, 5.0), 2, &plan);
        if let Some(door) = self.door_at(from, station.y - 5.0) {
            self.path(Vec2::new(door.x, station.y - 5.2), Vec2::new(door.x, walk + 0.9));
        }
        let g = self.height(10.0, station.y);
        self.solid(Aabb::span(Vec3::new(9.0, g - 0.3, station.y - 1.2), Vec3::new(10.2, g + 12.0, station.y)), Paint::Iron);
        self.solid(Aabb::span(Vec3::new(8.0, g + 12.0, station.y - 1.6), Vec3::new(11.2, g + 12.3, station.y + 0.4)), Paint::Iron);
        for _ in 0..10 {
            let p = Vec2::new(rng.range(-110.0, 110.0), z + rng.range(-14.0, 14.0));
            if !self.occupied(p, 6.0) && !self.near_road(p, 3.0) && !self.in_area(p, 3.0) && !self.near_base(p, BASE.0, BASE.1) {
                let size = rng.range(1.4, 2.6);
                self.outcrop(rng, p, size);
            }
        }
    }

    /// The summit: the lodge over its forecourt, the radio station and its mast on the west
    /// shoulder and the helipad on the east, paths to both from the stairs, lookout towers.
    fn summit(&mut self, rng: &mut Rng) {
        let z = level_mid(4) + 6.0;
        let lodge = Vec2::new(0.0, FORECOURT.1 + 7.0);
        let mut plan = Plan { walls: Some(Paint::Wood), trim: Some(Paint::Dark), roof: Roof::Gable(Paint::Shell), furnish: Furnish::Home, chimney: true, random_doors: false, ..Plan::default() };
        plan.doors[0] = Some((lodge.x - 2.5, 1.7));
        plan.doors[1] = Some((lodge.y, 1.7));
        plan.doors[2] = Some((lodge.y, 1.7));
        let (g, _) = self.building_plan(rng, lodge, Vec2::new(11.0, 7.0), 2, &plan);
        self.porch(rng, lodge, Vec2::new(11.0, 7.0), 0, g);
        // Cars in the forecourt, nose in to the lodge.
        for x in [-8.5f32, 8.5] {
            let p = Vec2::new(x, FORECOURT.1 - 5.0);
            self.clutter.push(Clutter { kind: ClutterKind::Car, pos: Vec3::new(p.x, self.height(p.x, p.y), p.y), turned: false });
        }
        // The paths along the shoulders from the stairs.
        let walk = level_mid(4) + 2.0;
        // The radio station: a concrete hut, a lattice mast (thin) and a dish (solid).
        let radio = Vec2::new(-46.0, walk + 1.0 + 3.6);
        let from = self.doors.len();
        let mut hut = Plan { walls: Some(Paint::Concrete), roof: Roof::Flat, furnish: Furnish::Plain, random_doors: false, ..Plan::default() };
        hut.doors[0] = Some((radio.x, super::DOOR_WIDTH));
        self.building_plan(rng, radio, Vec2::new(4.5, 3.6), 1, &hut);
        let door = self.door_at(from, radio.y - 3.6).unwrap_or(Vec2::new(radio.x, radio.y - 3.6));
        self.path(Vec2::new(-70.0, walk), Vec2::new(door.x, walk));
        self.path(Vec2::new(door.x, walk), Vec2::new(door.x, radio.y - 3.8));
        let mast = radio + Vec2::new(-9.0, 4.0);
        let mg = self.height(mast.x, mast.y);
        for (dx, dz) in [(-0.9, -0.9), (0.9, -0.9), (-0.9, 0.9), (0.9, 0.9)] {
            self.decor(Aabb::span(Vec3::new(mast.x + dx - 0.06, mg, mast.y + dz - 0.06), Vec3::new(mast.x + dx + 0.06, mg + 24.0, mast.y + dz + 0.06)), Paint::Coral);
        }
        let mut y = mg + 2.0;
        while y < mg + 24.0 {
            self.decor(Aabb::span(Vec3::new(mast.x - 0.95, y, mast.y - 0.95), Vec3::new(mast.x + 0.95, y + 0.06, mast.y - 0.88)), Paint::Bone);
            self.decor(Aabb::span(Vec3::new(mast.x - 0.95, y, mast.y + 0.88), Vec3::new(mast.x + 0.95, y + 0.06, mast.y + 0.95)), Paint::Bone);
            y += 2.0;
        }
        self.decor(Aabb::span(Vec3::new(mast.x - 0.96, mg + 24.0, mast.y - 0.96), Vec3::new(mast.x + 0.96, mg + 24.06, mast.y + 0.96)), Paint::Coral);
        self.decor(Aabb::new(Vec3::new(mast.x, mg + 24.18, mast.y), Vec3::new(0.12, 0.12, 0.12)), Paint::Red);
        let dish = radio + Vec2::new(10.0, 2.0);
        let dg = self.height(dish.x, dish.y);
        self.solid(Aabb::span(Vec3::new(dish.x - 0.4, dg - 0.2, dish.y - 0.4), Vec3::new(dish.x + 0.4, dg + 1.6, dish.y + 0.4)), Paint::Steel);
        self.solid(Aabb::span(Vec3::new(dish.x - 1.8, dg + 1.6, dish.y - 0.5), Vec3::new(dish.x + 1.8, dg + 4.2, dish.y - 0.2)), Paint::Bone);
        // The helipad: an H in a ring of lights, with sandbags round it, the path to its edge.
        let pad = Vec2::new(46.0, z + 4.0);
        let pg = self.height(pad.x, pad.y) + 0.01;
        self.flat(pad.x - 8.0, pad.x + 8.0, pad.y - 8.0, pad.y + 8.0, pg, Paint::Concrete);
        self.areas.push(Area { lo: pad - Vec2::splat(8.0), hi: pad + Vec2::splat(8.0), kind: AreaKind::Plaza });
        self.path(Vec2::new(70.0, walk), Vec2::new(pad.x + 8.0, walk));
        self.flat(pad.x - 2.6, pad.x - 1.8, pad.y - 3.0, pad.y + 3.0, pg + 0.01, Paint::Bone);
        self.flat(pad.x + 1.8, pad.x + 2.6, pad.y - 3.0, pad.y + 3.0, pg + 0.01, Paint::Bone);
        self.flat(pad.x - 1.8, pad.x + 1.8, pad.y - 0.4, pad.y + 0.4, pg + 0.01, Paint::Bone);
        for i in 0..12 {
            let p = pad + Vec2::from_angle(i as f32 * TAU / 12.0) * 7.2;
            self.decor(Aabb::new(Vec3::new(p.x, pg + 0.05, p.y), Vec3::new(0.15, 0.04, 0.15)), Paint::Glow);
        }
        for (dx, dz, open) in [(-10.5, 4.0, 2), (10.5, 0.0, 1)] {
            self.sandbags(pad + Vec2::new(dx, dz), Vec2::new(1.4, 2.0), open);
        }
        // A lookout tower on each flank for the long shots.
        self.building_plan(rng, Vec2::new(82.0, z + 12.0), Vec2::new(3.3, 3.0), 4, &Plan { walls: Some(Paint::Wood), roof: Roof::Flat, furnish: Furnish::Plain, ..Plan::default() });
        self.building_plan(rng, Vec2::new(-82.0, z + 12.0), Vec2::new(3.3, 3.0), 4, &Plan { walls: Some(Paint::Wood), roof: Roof::Flat, furnish: Furnish::Plain, ..Plan::default() });
        for _ in 0..8 {
            let p = Vec2::new(rng.range(-100.0, 100.0), z + rng.range(-12.0, 20.0));
            if !self.occupied(p, 7.0) && !self.near_road(p, 3.0) && !self.in_area(p, 3.0) {
                let size = rng.range(1.6, 2.8);
                self.outcrop(rng, p, size);
            }
        }
    }

    /// Pines and trees on every level, bushes, grass; snowy rock above.
    fn mountain_greenery(&mut self, rng: &mut Rng) {
        let mut trees = 0;
        for _ in 0..2400 {
            if trees >= 230 {
                break;
            }
            let p = Vec2::new(rng.range(-124.0, 124.0), rng.range(-110.0, 124.0));
            let h = self.height(p.x, p.y);
            // Not on the cliffs, the roads or the bases.
            let near_cliff = CLIFFS.iter().any(|b| (p.y - b).abs() < 6.5);
            if h < 0.8 || near_cliff || self.near_road(p, 2.0) || self.in_area(p, 2.0) || self.near_base(p, BASE.0, BASE.1) || self.occupied(p, 3.0) {
                continue;
            }
            let height = rng.range(7.0, 10.5);
            if (p.y > 96.0 || p.x.abs() > 117.0 || trees % 4 == 3) && !self.occupied(p, 3.2) && self.pine(p, height) {
                // A block pine.
            } else {
                self.props.push(Prop { kind: PropKind::Tree, pos: Vec3::new(p.x, h, p.y), yaw: rng.range(0.0, TAU), scale: rng.range(3.6, 5.6) });
            }
            trees += 1;
        }
        let mut bushes = 0;
        for _ in 0..1000 {
            if bushes >= 140 {
                break;
            }
            let p = Vec2::new(rng.range(-112.0, 112.0), rng.range(-100.0, 96.0));
            if CLIFFS.iter().any(|b| (p.y - b).abs() < 3.0) || self.occupied(p, 0.9) || self.near_road(p, 0.8) || self.in_area(p, 0.8) {
                continue;
            }
            self.props.push(Prop { kind: PropKind::Bush, pos: Vec3::new(p.x, self.height(p.x, p.y), p.y), yaw: rng.range(0.0, TAU), scale: rng.range(0.7, 1.2) });
            bushes += 1;
        }
        // Wild flowers and ferns in the grass, and rocky scree where the ground is high.
        let mut wild = self.detail_rng(Vec2::new(3.0, 7.0), 0xf10);
        for _ in 0..420 {
            let p = Vec2::new(wild.range(-118.0, 118.0), wild.range(-104.0, 110.0));
            if CLIFFS.iter().any(|b| (p.y - b).abs() < 3.0) || self.near_road(p, 0.6) || self.occupied(p, 0.6) || self.height(p.x, p.y) < self.water + 0.6 {
                continue;
            }
            if p.y > 60.0 && wild.chance(0.5) {
                self.scree(p, 1.4, 4, Paint::Rock);
            } else {
                self.flowers(p, &mut wild);
            }
        }
        for _ in 0..5200 {
            let p = Vec2::new(rng.range(-120.0, 120.0), rng.range(-110.0, 110.0));
            let h = self.height(p.x, p.y);
            if h < self.water + 0.5 || self.near_road(p, 0.0) || self.in_area(p, 0.0) || self.occupied(p, 0.3) {
                continue;
            }
            self.grass.push(Vec3::new(p.x, h, p.y));
        }
    }
}
