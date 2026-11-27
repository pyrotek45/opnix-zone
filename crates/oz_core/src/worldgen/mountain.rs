//! Mountainside: a fixed map on a slope. Five levels climb from a village in the valley
//! (south) to a lodge and radio station on the summit (north), each a flat terrace held up by a
//! rock cliff. A switchback road zig-zags up between ramps, stairs are cut into the cliffs, and
//! the cliff tops have rocky parapets to shoot down from. The team bases sit at either end of
//! the middle terrace, so both teams can fight uphill or down.

use super::{
    COVER_HEIGHT, Clutter, ClutterKind, Prop, PropKind, World,
    shapes::{Furnish, Plan, Roof},
};
use crate::{collide::Aabb, model::Paint, noise, rng::Rng};
use glam::{Vec2, Vec3};
use std::f32::consts::TAU;

/// Level heights, valley to summit.
const LEVELS: [f32; 5] = [1.2, 6.2, 11.2, 16.2, 21.2];
/// Where each cliff stands (z), between level i and i + 1.
const CLIFFS: [f32; 4] = [-62.0, -24.0, 14.0, 52.0];
/// Ramps (x) up each cliff: the switchback road climbs from the valley's ends toward the middle.
const RAMPS: [&[f32]; 4] = [&[-70.0, 70.0], &[-84.0, 84.0], &[-48.0, 48.0], &[-22.0, 22.0]];
/// Stairways (x) cut into each cliff.
const STAIRS: [&[f32]; 4] = [&[0.0], &[-30.0, 30.0], &[0.0, -92.0, 92.0], &[-70.0, 70.0]];
/// The bases: either end of the middle terrace.
const BASE: (f32, f32) = (104.0, -5.0);
const RAMP_HALF: f32 = 5.0;
const STEP_RUN: f32 = 0.42;

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
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
                let n = ((hi - lo) / 0.29).ceil() as usize;
                let rise = (hi - lo) / n as f32;
                let front = b - 1.6;
                for k in 0..n {
                    let z1 = front - (n - 1 - k) as f32 * STEP_RUN;
                    let top = lo + (k + 1) as f32 * rise;
                    self.solid(Aabb::span(Vec3::new(s - 1.3, lo - 0.4, z1 - STEP_RUN), Vec3::new(s + 1.3, top, z1)), if k % 2 == 0 { Paint::Concrete } else { Paint::Rock });
                }
                // Thin hand rails either side.
                for side in [-1.0, 1.0] {
                    let x = s + side * 1.35;
                    self.decor(Aabb::span(Vec3::new(x - 0.04, lo + 1.0, front - n as f32 * STEP_RUN), Vec3::new(x + 0.04, hi + 1.0, front)), Paint::Iron);
                }
            }
        }
    }

    /// One run of cliff from `x0` to `x1` at cliff `b`: the face, and sometimes a parapet on top.
    #[allow(clippy::too_many_arguments)]
    fn cliff_block(&mut self, rng: &mut Rng, x0: f32, x1: f32, b: f32, lo: f32, hi: f32, i: usize) {
        let front = b - 1.6 - rng.range(0.0, 0.7);
        let paint = if rng.chance(0.5) { Paint::Rock } else { Paint::Concrete };
        self.solid(Aabb::span(Vec3::new(x0, lo - 0.6, front), Vec3::new(x1, hi, b + 1.6)), paint);
        // Strata: paint-thin lines across the face.
        for k in 1..3 {
            let y = lo + (hi - lo) * k as f32 / 3.0 + rng.range(-0.3, 0.3);
            self.decor(Aabb::span(Vec3::new(x0, y, front - 0.03), Vec3::new(x1, y + 0.08, front)), Paint::Dark);
        }
        let stairs = STAIRS[i].iter().any(|s| x1 > s - 2.2 && x0 < s + 2.2);
        if !stairs && rng.chance(0.45) && x1 - x0 > 2.0 {
            let (a, c) = (x0 + rng.range(0.0, 0.6), x1 - rng.range(0.0, 0.6));
            self.solid(Aabb::span(Vec3::new(a, hi - 0.1, front), Vec3::new(c, hi + COVER_HEIGHT, front + 0.9)), Paint::Rock);
        }
    }

    /// The switchback road: along each level between the ramps, and up them.
    fn switchback(&mut self) {
        let asphalt = Paint::Dark;
        // The valley street, end to end.
        self.road(Vec2::new(-120.0, level_mid(0)), Vec2::new(120.0, level_mid(0)), 7.0, asphalt, true);
        for s in [-1.0f32, 1.0] {
            let mut x = 70.0 * s;
            let mut z = level_mid(0);
            for (i, &b) in CLIFFS.iter().enumerate() {
                let rx = RAMPS[i].iter().copied().find(|r| r.signum() == s).unwrap_or(x);
                // Along the level to the foot of the ramp, then up it.
                self.road(Vec2::new(x, z), Vec2::new(rx, z), 6.0, asphalt, true);
                self.road(Vec2::new(rx, z), Vec2::new(rx, b - 9.0), 6.0, asphalt, false);
                self.ramp_road(rx, b - 9.0, b + 9.0);
                let next = level_mid(i + 1);
                self.road(Vec2::new(rx, b + 9.0), Vec2::new(rx, next), 6.0, asphalt, false);
                x = rx;
                z = next;
            }
            // Over the summit to the lodge.
            self.road(Vec2::new(x, z), Vec2::new(0.0, z), 6.0, asphalt, true);
        }
        // The middle terrace's road runs out to the bases.
        for s in [-1.0f32, 1.0] {
            self.road(Vec2::new(s * 84.0, level_mid(2)), Vec2::new(s * 120.0, level_mid(2)), 6.0, asphalt, false);
        }
    }

    /// Road up a ramp, laid smooth on the slope.
    fn ramp_road(&mut self, x: f32, z0: f32, z1: f32) {
        self.drape(Vec2::new(x - 3.0, z0), Vec2::new(x + 3.0, z1), 0.024, Paint::Dark);
        // Crash barriers down both sides: cover.
        for s in [-1.0, 1.0] {
            let bx = x + s * 3.6;
            let mut z = z0 + 1.0;
            while z < z1 - 1.0 {
                let g = self.height(bx, z + 1.0).max(self.height(bx, z)).max(self.height(bx, z + 2.0));
                let low = self.height(bx, z).min(self.height(bx, z + 2.0));
                self.solid(Aabb::span(Vec3::new(bx - 0.15, low - 0.3, z), Vec3::new(bx + 0.15, g + COVER_HEIGHT, z + 2.0)), Paint::Metal);
                z += 2.0;
            }
        }
    }

    /// The valley village: houses along the street, a chapel with a bell tower, a square.
    fn valley_village(&mut self, rng: &mut Rng) {
        let z = level_mid(0);
        self.plaza_at(rng, Vec2::new(0.0, z - 16.0));
        // The chapel: a tall nave with a bell tower at its front.
        let chapel = Vec2::new(0.0, z + 15.0);
        self.building_plan(rng, chapel, Vec2::new(5.0, 8.0), 2, &Plan { walls: Some(Paint::Bone), trim: Some(Paint::Steel), roof: Roof::Gable(Paint::Dark), furnish: Furnish::Plain, random_doors: false, doors: [Some((chapel.x, 1.7)), None, None, None], ladder: false, ..Plan::default() });
        let tower = Vec2::new(-9.2, z + 11.0);
        let (_, top) = self.building_plan(rng, tower, Vec2::new(3.2, 3.0), 4, &Plan { walls: Some(Paint::Bone), trim: Some(Paint::Steel), roof: Roof::Flat, furnish: Furnish::Bare, random_doors: false, doors: [Some((tower.x, 1.7)), None, None, None], ladder: false, ..Plan::default() });
        // The spire stands on the roof slab.
        self.decor(Aabb::span(Vec3::new(tower.x - 0.06, top + 0.3, tower.y - 0.06), Vec3::new(tower.x + 0.06, top + 5.0, tower.y + 0.06)), Paint::Gold);
        // Houses either side of the street.
        for side in [-1.0f32, 1.0] {
            let mut x = 18.0;
            while x < 104.0 {
                for s in [-1.0f32, 1.0] {
                    let at = Vec2::new(s * (x + rng.range(-1.0, 1.0)), z + side * 12.0);
                    let floors = if rng.chance(0.4) { 2 } else { 1 };
                    let half = Vec2::new(rng.range(4.2, 5.4), rng.range(3.8, 4.4));
                    self.house(rng, at, half, floors, if side > 0.0 { 0 } else { 3 });
                }
                x += rng.range(15.0, 19.0);
            }
        }
        // Cars down the street.
        for _ in 0..8 {
            let p = Vec2::new(rng.range(-100.0, 100.0), z + if rng.chance(0.5) { 2.4 } else { -2.4 });
            if !self.occupied(p, 2.4) && self.clutter.iter().all(|c| Vec2::new(c.pos.x, c.pos.z).distance(p) > 6.0) {
                self.clutter.push(Clutter { kind: ClutterKind::Car, pos: Vec3::new(p.x, self.height(p.x, p.y), p.y), turned: true });
            }
        }
    }

    /// The lower terrace: vineyard rows behind stone walls, a farmhouse and a barn.
    fn lower_terrace(&mut self, rng: &mut Rng) {
        let z = level_mid(1);
        self.barn(rng, Vec2::new(-40.0, z + 6.0), Vec2::new(7.5, 5.5));
        self.house(rng, Vec2::new(40.0, z + 7.0), Vec2::new(5.0, 4.4), 2, 0);
        // Vine rows: low walls you can crouch behind, the length of the field.
        for side in [-1.0f32, 1.0] {
            for k in 0..4 {
                let rz = z - 12.0 + k as f32 * 6.0;
                let (a, c) = (side * 48.0, side * 60.0);
                self.low_wall(Vec2::new(a.min(c), rz), Vec2::new(a.max(c), rz), COVER_HEIGHT, 0.5, Paint::Rock);
                let (a, c) = (side * 10.0, side * 24.0);
                self.low_wall(Vec2::new(a.min(c), rz), Vec2::new(a.max(c), rz), COVER_HEIGHT, 0.5, Paint::Rock);
            }
        }
        for _ in 0..10 {
            let p = Vec2::new(rng.range(-100.0, 100.0), z + rng.range(-14.0, 14.0));
            if !self.occupied(p, 2.0) && !self.near_road(p, 1.0) {
                self.hay_stack(p, rng.chance(0.5), None);
            }
        }
    }

    /// The middle terrace: the bases at either end, an inn and a garage in the middle.
    fn middle_terrace(&mut self, rng: &mut Rng) {
        let z = level_mid(2);
        let inn = Vec2::new(-18.0, z + 9.0);
        self.building_plan(rng, inn, Vec2::new(7.0, 5.5), 3, &Plan { walls: Some(Paint::Wood), roof: Roof::Gable(Paint::Rust), furnish: Furnish::Home, chimney: true, ..Plan::default() });
        self.garage(rng, Vec2::new(16.0, z + 8.5), 0);
        self.garage(rng, Vec2::new(22.0, z + 8.5), 0);
        // A fuel stop: pumps (cover) under a canopy on posts.
        let pumps = Vec2::new(18.0, z - 10.0);
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
        for (kind, n) in [(ClutterKind::Barrel, 6), (ClutterKind::Car, 4), (ClutterKind::Crates, 6), (ClutterKind::Bin, 4)] {
            let mut placed = 0;
            for _ in 0..n * 12 {
                if placed >= n {
                    break;
                }
                let p = Vec2::new(rng.range(-80.0, 80.0), z + rng.range(-14.0, 14.0));
                let reach = kind.half().x.max(kind.half().z) + 0.8;
                if self.occupied(p, reach) || self.clutter.iter().any(|c| Vec2::new(c.pos.x, c.pos.z).distance(p) < reach + 2.0) {
                    continue;
                }
                self.clutter.push(Clutter { kind, pos: Vec3::new(p.x, self.height(p.x, p.y), p.y), turned: rng.chance(0.5) });
                placed += 1;
            }
        }
        for _ in 0..8 {
            let p = Vec2::new(rng.range(-80.0, 80.0), z + rng.range(-14.0, 12.0));
            if !self.occupied(p, 6.0) && !self.near_road(p, 3.0) {
                let size = rng.range(1.2, 2.2);
                self.outcrop(rng, p, size);
            }
        }
    }

    /// The upper terrace: chalets among rocks and pines, a cable-car station.
    fn upper_terrace(&mut self, rng: &mut Rng) {
        let z = level_mid(3);
        for (x, front) in [(-76.0, 0), (-36.0, 0), (36.0, 0), (76.0, 0), (-100.0, 2), (100.0, 1)] {
            let at = Vec2::new(x + rng.range(-3.0, 3.0), z + rng.range(2.0, 8.0));
            if !self.occupied(at, 7.0) {
                self.cabin(rng, at, front);
            }
        }
        // The cable-car station: a hall with a tall pylon beside it.
        let station = Vec2::new(0.0, z + 6.0);
        self.building_plan(rng, station, Vec2::new(6.0, 5.0), 2, &Plan { walls: Some(Paint::Concrete), roof: Roof::Flat, furnish: Furnish::Plain, ..Plan::default() });
        let g = self.height(10.0, z + 6.0);
        self.solid(Aabb::span(Vec3::new(9.0, g - 0.3, z + 4.8), Vec3::new(10.2, g + 12.0, z + 6.0)), Paint::Iron);
        self.solid(Aabb::span(Vec3::new(8.0, g + 12.0, z + 4.4), Vec3::new(11.2, g + 12.3, z + 6.4)), Paint::Iron);
        for _ in 0..10 {
            let p = Vec2::new(rng.range(-110.0, 110.0), z + rng.range(-14.0, 14.0));
            if !self.occupied(p, 6.0) && !self.near_road(p, 3.0) && !self.near_base(p, BASE.0, BASE.1) {
                let size = rng.range(1.4, 2.6);
                self.outcrop(rng, p, size);
            }
        }
    }

    /// The summit: the lodge, the radio station and its mast, a helipad, a lookout.
    fn summit(&mut self, rng: &mut Rng) {
        let z = level_mid(4) + 6.0;
        let lodge = Vec2::new(0.0, z + 8.0);
        let mut plan = Plan { walls: Some(Paint::Wood), trim: Some(Paint::Dark), roof: Roof::Gable(Paint::Shell), furnish: Furnish::Home, chimney: true, random_doors: false, ..Plan::default() };
        plan.doors[0] = Some((lodge.x - 2.5, 1.7));
        plan.doors[1] = Some((lodge.y, 1.7));
        plan.doors[2] = Some((lodge.y, 1.7));
        let (g, _) = self.building_plan(rng, lodge, Vec2::new(11.0, 7.0), 2, &plan);
        self.porch(rng, lodge, Vec2::new(11.0, 7.0), 0, g);
        // The radio station: a concrete hut, a lattice mast (thin) and a dish (solid).
        let radio = Vec2::new(-46.0, z + 2.0);
        self.building_plan(rng, radio, Vec2::new(4.5, 3.6), 1, &Plan { walls: Some(Paint::Concrete), roof: Roof::Flat, furnish: Furnish::Plain, ..Plan::default() });
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
        // A cap across the legs' tops, and the beacon on it.
        self.decor(Aabb::span(Vec3::new(mast.x - 0.96, mg + 24.0, mast.y - 0.96), Vec3::new(mast.x + 0.96, mg + 24.06, mast.y + 0.96)), Paint::Coral);
        self.decor(Aabb::new(Vec3::new(mast.x, mg + 24.18, mast.y), Vec3::new(0.12, 0.12, 0.12)), Paint::Red);
        let dish = radio + Vec2::new(2.0, -8.0);
        let dg = self.height(dish.x, dish.y);
        self.solid(Aabb::span(Vec3::new(dish.x - 0.4, dg - 0.2, dish.y - 0.4), Vec3::new(dish.x + 0.4, dg + 1.6, dish.y + 0.4)), Paint::Steel);
        self.solid(Aabb::span(Vec3::new(dish.x - 1.8, dg + 1.6, dish.y - 0.5), Vec3::new(dish.x + 1.8, dg + 4.2, dish.y - 0.2)), Paint::Bone);
        // The helipad: an H in a ring of lights, with sandbags round it.
        let pad = Vec2::new(46.0, z + 2.0);
        let pg = self.height(pad.x, pad.y) + 0.01;
        self.flat(pad.x - 8.0, pad.x + 8.0, pad.y - 8.0, pad.y + 8.0, pg, Paint::Concrete);
        self.flat(pad.x - 2.6, pad.x - 1.8, pad.y - 3.0, pad.y + 3.0, pg + 0.01, Paint::Bone);
        self.flat(pad.x + 1.8, pad.x + 2.6, pad.y - 3.0, pad.y + 3.0, pg + 0.01, Paint::Bone);
        self.flat(pad.x - 1.8, pad.x + 1.8, pad.y - 0.4, pad.y + 0.4, pg + 0.01, Paint::Bone);
        for i in 0..12 {
            let p = pad + Vec2::from_angle(i as f32 * TAU / 12.0) * 7.2;
            self.decor(Aabb::new(Vec3::new(p.x, pg + 0.05, p.y), Vec3::new(0.15, 0.04, 0.15)), Paint::Glow);
        }
        for (dx, dz, open) in [(-10.5, 0.0, 2), (10.5, 0.0, 1)] {
            self.sandbags(pad + Vec2::new(dx, dz), Vec2::new(1.4, 2.0), open);
        }
        // A lookout tower at the back for the long shots.
        self.building_plan(rng, Vec2::new(82.0, z + 10.0), Vec2::new(3.3, 3.0), 4, &Plan { walls: Some(Paint::Wood), roof: Roof::Flat, furnish: Furnish::Plain, ..Plan::default() });
        self.building_plan(rng, Vec2::new(-82.0, z + 10.0), Vec2::new(3.3, 3.0), 4, &Plan { walls: Some(Paint::Wood), roof: Roof::Flat, furnish: Furnish::Plain, ..Plan::default() });
        for _ in 0..8 {
            let p = Vec2::new(rng.range(-100.0, 100.0), z + rng.range(-18.0, 20.0));
            if !self.occupied(p, 7.0) && !self.near_road(p, 3.0) {
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
            if h < 0.8 || near_cliff || self.near_road(p, 2.0) || self.near_base(p, BASE.0, BASE.1) || self.occupied(p, 3.0) {
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
            if CLIFFS.iter().any(|b| (p.y - b).abs() < 3.0) || self.occupied(p, 0.9) || self.near_road(p, 0.3) {
                continue;
            }
            self.props.push(Prop { kind: PropKind::Bush, pos: Vec3::new(p.x, self.height(p.x, p.y), p.y), yaw: rng.range(0.0, TAU), scale: rng.range(0.7, 1.2) });
            bushes += 1;
        }
        for _ in 0..5200 {
            let p = Vec2::new(rng.range(-120.0, 120.0), rng.range(-110.0, 110.0));
            let h = self.height(p.x, p.y);
            if h < self.water + 0.5 || self.near_road(p, 0.0) || self.occupied(p, 0.3) {
                continue;
            }
            self.grass.push(Vec3::new(p.x, h, p.y));
        }
    }
}
