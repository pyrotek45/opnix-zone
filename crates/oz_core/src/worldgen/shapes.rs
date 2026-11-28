//! Building shapes and set pieces every generator shares: plans for [`World::building_plan`]
//! (pitched and hipped roofs, wings, balconies, wide doors), houses, L-shaped houses,
//! mansions, garages, barns, shacks, huts, cabins, lodges, warehouses and towers; round-ish
//! silos, water towers and a windmill; fences, hedges, low walls, sandbags, hay bales, rock
//! outcrops; roads; office furniture; and the team bases.
//!
//! The rules every piece keeps: what you could take cover behind is solid; poles, pipes, wires,
//! papers and screens - anything too thin to hide behind - are decoration you walk through;
//! low cover is [`COVER_HEIGHT`]; doorways are [`DOOR_WIDTH`] by [`DOOR_HEIGHT`]; stairs are
//! eleven 0.29 m steps of 0.42 m.

use super::{COVER_HEIGHT, ClutterKind, Clutter, DOOR_WIDTH, Ladder, Prop, PropKind, World};
use std::f32::consts::TAU;
use crate::{collide::{Aabb, Ramp}, model::{Anim, Hull, Paint}, rng::Rng};
use glam::{Vec2, Vec3};

/// How a map's buildings look: what their walls and roofs are painted, and how often roofs are pitched.
#[derive(Clone, Debug)]
pub(crate) struct Style {
    pub walls: Vec<Paint>,
    pub roofs: Vec<Paint>,
    /// Chance of a pitched (gable) roof.
    pub gable: f32,
    /// Chance of a hipped roof.
    pub hip: f32,
}

impl Default for Style {
    fn default() -> Self {
        Self { walls: vec![Paint::Concrete, Paint::Rust, Paint::Bone, Paint::Teal, Paint::Steel, Paint::Sand], roofs: vec![Paint::Rust, Paint::Dark], gable: 0.0, hip: 0.0 }
    }
}

impl Style {
    pub fn roof(&self, rng: &mut Rng) -> Roof {
        let tiles = *rng.pick(&self.roofs).unwrap_or(&Paint::Rust);
        let r = rng.unit();
        if r < self.gable {
            Roof::Gable(tiles)
        } else if r < self.gable + self.hip {
            Roof::Hip(tiles)
        } else {
            Roof::Flat
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Roof {
    /// Whatever the map's style rolls.
    Auto,
    /// Flat, with parapets on two sides and rooftop clutter.
    Flat,
    /// A stepped gable: each step one stair rise, so you can walk up it and crouch behind the ridge.
    Gable(Paint),
    /// Stepped on all four sides.
    Hip(Paint),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Furnish {
    /// Nothing inside, not even the partition.
    Bare,
    /// A counter or two.
    Plain,
    /// Sofa, table, kitchen counter; beds upstairs.
    Home,
    /// Hay bales and stalls.
    Barn,
}

/// What a building must have; everything else is left to chance.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Plan {
    pub walls: Option<Paint>,
    pub trim: Option<Paint>,
    pub floor_paint: Paint,
    pub roof: Roof,
    /// Doorways per side (south, west, east, north): centre along the wall in world
    /// coordinates, and width (wider than a door is an open gateway).
    pub doors: [Option<(f32, f32)>; 4],
    /// Add one or two doors at random as well.
    pub random_doors: bool,
    /// Leave these sides' walls out: a wing joins on there.
    pub open: [bool; 4],
    /// Floor level (else just above the highest corner of the ground).
    pub floor: Option<f32>,
    pub ladder: bool,
    pub windows: bool,
    pub furnish: Furnish,
    /// A door out of the first floor's south wall onto a balcony over the front door.
    pub balcony: bool,
    pub chimney: bool,
}

impl Default for Plan {
    fn default() -> Self {
        Self {
            walls: None,
            trim: None,
            floor_paint: Paint::Wood,
            roof: Roof::Auto,
            doors: [None; 4],
            random_doors: true,
            open: [false; 4],
            floor: None,
            ladder: true,
            windows: true,
            furnish: Furnish::Plain,
            balcony: false,
            chimney: false,
        }
    }
}

/// Outward direction of a side (south, west, east, north).
pub(crate) fn side_out(side: usize) -> Vec2 {
    [Vec2::NEG_Y, Vec2::NEG_X, Vec2::X, Vec2::Y][side]
}

impl World {
    pub(crate) fn ground_at(&self, p: Vec2) -> f32 {
        self.height(p.x, p.y)
    }

    /// Flat on the ground: paths, paint, rugs, puddles. Paint-thin, so never in the way.
    pub(crate) fn flat(&mut self, x0: f32, x1: f32, z0: f32, z1: f32, y: f32, paint: Paint) {
        self.decor(Aabb::span(Vec3::new(x0, y, z0), Vec3::new(x1, y + 0.03, z1)), paint);
    }

    /// A pole: too thin to hide behind, so you walk through it.
    pub(crate) fn pole(&mut self, foot: Vec3, height: f32, paint: Paint) {
        self.decor(Aabb::span(foot + Vec3::new(-0.05, 0.0, -0.05), foot + Vec3::new(0.05, height, 0.05)), paint);
    }

    /// A solid box standing on the ground at `p` (its footprint half-size), `height` tall.
    pub(crate) fn boxed(&mut self, p: Vec2, half: Vec2, height: f32, paint: Paint) -> f32 {
        let g = self.lowest(p, half);
        let top = self.ground_at(p).max(g) + height;
        // Loose boxes (cover, woodpiles, crates) are cleared from doorways like furniture.
        let from = self.blocks.len();
        self.solid(Aabb::span(Vec3::new(p.x - half.x, g - 0.15, p.y - half.y), Vec3::new(p.x + half.x, top, p.y + half.y)), paint);
        self.mark_furniture(from);
        top
    }

    /// The lowest ground under a footprint.
    pub(crate) fn lowest(&self, p: Vec2, half: Vec2) -> f32 {
        [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0), (0.0, 0.0)].iter().map(|(a, b)| self.height(p.x + a * half.x, p.y + b * half.y)).fold(f32::MAX, f32::min)
    }

    /// The highest ground under a footprint.
    pub(crate) fn highest(&self, p: Vec2, half: Vec2) -> f32 {
        [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0), (0.0, 0.0)].iter().map(|(a, b)| self.height(p.x + a * half.x, p.y + b * half.y)).fold(f32::MIN, f32::max)
    }

    /// A pitched roof over a footprint, starting at `y`: a gable (hipped too if `hip`), its
    /// ridge running along X if `along_x`, tiled in `paint` with gable ends in `ends`.
    /// Underneath it is a stack of stair steps, each one rise, so the roof can be walked up
    /// from the eaves; the steps carry the collision and are hidden, and the roof is drawn
    /// as smooth slopes just over their outer corners (see [`World::roofs`]). Returns the
    /// height of the ridge.
    pub(crate) fn stepped_roof(&mut self, (x0, x1, z0, z1): (f32, f32, f32, f32), y: f32, hip: bool, along_x: bool, (paint, ends): (Paint, Paint)) -> f32 {
        let (run, rise) = (0.6, 0.4);
        let (cx, cz) = ((x0 + x1) * 0.5, (z0 + z1) * 0.5);
        let across = if along_x { (z1 - z0) * 0.5 } else { (x1 - x0) * 0.5 };
        let along = if along_x { (x1 - x0) * 0.5 } else { (z1 - z0) * 0.5 };
        let mut top = y;
        // Half sizes (across, along) of the top step: the flat strip along the ridge.
        let mut ridge = (across, along);
        for i in 0..8 {
            let a = across - (i + 1) as f32 * run;
            if a < 0.3 {
                break;
            }
            let l = if hip { (along - (i + 1) as f32 * run).max(0.4) } else { along };
            let half = if along_x { Vec3::new(l, rise * 0.5, a) } else { Vec3::new(a, rise * 0.5, l) };
            self.blocks.push(super::Block { aabb: Aabb::new(Vec3::new(cx, y + (i as f32 + 0.5) * rise, cz), half), paint, solid: true, hidden: true });
            top = y + (i + 1) as f32 * rise;
            ridge = (a, l);
        }
        if top > y {
            self.sloped_roof(Vec3::new(cx, y, cz), (across, along), ridge, top - y, hip, along_x, (paint, ends));
        }
        top
    }

    /// The drawn part of [`Self::stepped_roof`]: slopes from the eaves (half sizes `base`,
    /// at `foot`) up to a flat ridge strip (half sizes `ridge`) `height` higher, plus a
    /// little: eaves that overhang the walls, a ridge cap, and on a gable the end triangles
    /// filled in. The slopes are thin skins, so the end triangles show their own paint.
    #[allow(clippy::too_many_arguments)]
    fn sloped_roof(&mut self, foot: Vec3, base: (f32, f32), ridge: (f32, f32), height: f32, hip: bool, along_x: bool, (paint, ends): (Paint, Paint)) {
        // Just over the steps' corners, so nothing pokes through and feet stay on it.
        let lift = 0.04;
        // Skin thickness, eave overhang (out over the walls and past the gable ends).
        let (thick, eave, end) = (0.14, 0.3, 0.25);
        let slope = height / (base.0 - ridge.0).max(0.01);
        // (across, up, along) -> world.
        let at = |u: f32, h: f32, v: f32| if along_x { foot + Vec3::new(v, h, u) } else { foot + Vec3::new(u, h, v) };
        let (a0, a1, h1) = (base.0 + eave, ridge.0, height + lift);
        let h0 = -eave * slope + lift;
        // Walked as drawn: the slopes over the steps' footprint, up to the ridge.
        let (u_axis, v_axis) = if along_x { (Vec2::Y, Vec2::X) } else { (Vec2::X, Vec2::Y) };
        let centre = Vec2::new(foot.x, foot.z);
        let extent = u_axis * base.0 + v_axis * base.1;
        // A plane rising `rise` per metre inward from the edge `edge` metres out along `axis`.
        let plane = |axis: Vec2, edge: f32, rise: f32| {
            let k = -axis * rise;
            Vec3::new(k.x, k.y, foot.y + lift + rise * (edge + axis.dot(centre)))
        };
        let mut planes = vec![plane(u_axis, base.0, slope), plane(-u_axis, base.0, slope)];
        if hip {
            let slope = height / (base.1 - ridge.1).max(0.01);
            planes.extend([plane(v_axis, base.1, slope), plane(-v_axis, base.1, slope)]);
        }
        self.ramps.push(Ramp { min: centre - extent, max: centre + extent, bottom: foot.y, top: foot.y + h1, planes });
        if hip {
            // One solid: the eave rectangle and the ridge strip, the overhang all round.
            let l0 = base.1 + eave;
            let l1 = ridge.1;
            let mut points = Vec::with_capacity(12);
            for (su, sv) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                points.push(at(su * a0, h0, sv * l0));
                // A thinner fascia than a gable's: the underside stays above the ceiling slab's.
                points.push(at(su * a0, h0 - 0.1, sv * l0));
                points.push(at(su * a1, h1, sv * l1));
            }
            self.roofs.push(Hull { paint, points, anim: Anim::None });
            return;
        }
        let l = base.1 + end;
        // The two slopes: thin skins from the eave up to the ridge strip.
        for s in [-1.0, 1.0] {
            let mut points = Vec::with_capacity(8);
            for v in [-l, l] {
                points.extend([at(s * a0, h0, v), at(s * a0, h0 - thick, v), at(s * a1, h1, v), at(s * a1, h1 - thick, v)]);
            }
            self.roofs.push(Hull { paint, points, anim: Anim::None });
        }
        // The ridge cap.
        let cap = a1 + 0.08;
        let mut points = Vec::with_capacity(8);
        for (u, v) in [(-cap, -l), (cap, -l), (-cap, l), (cap, l)] {
            points.extend([at(u, h1 + 0.05, v), at(u, h1 - thick, v)]);
        }
        self.roofs.push(Hull { paint: darker(paint), points, anim: Anim::None });
        // The gable ends: filled up to just under the skins, flush with the eaves below.
        let mut points = Vec::with_capacity(8);
        for v in [-base.1, base.1] {
            points.extend([at(-base.0, 0.0, v), at(base.0, 0.0, v), at(-a1, height, v), at(a1, height, v)]);
        }
        self.roofs.push(Hull { paint: ends, points, anim: Anim::None });
    }

    /// Steps up to a doorway whose floor `floor` stands above the ground outside it.
    /// `p` is the middle of the doorway on the outer face of the wall, `out` points outward.
    pub(crate) fn stoop(&mut self, p: Vec2, out: Vec2, width: f32, floor: f32) {
        let outside = p + out * 0.7;
        let drop = floor - self.ground_at(outside);
        if drop < 0.3 {
            return;
        }
        let n = (drop / 0.38).ceil() as usize;
        let rise = drop / n as f32;
        let side = Vec2::new(out.y.abs(), out.x.abs());
        for j in 1..n {
            // Step j (counted down from the door) is j runs out and one rise lower.
            let top = floor - j as f32 * rise;
            let a = p + out * ((j - 1) as f32 * 0.42);
            let b = p + out * (j as f32 * 0.42);
            let lo = a.min(b) - side * width * 0.5;
            let hi = a.max(b) + side * width * 0.5;
            self.solid(Aabb::span(Vec3::new(lo.x, top - 1.2, lo.y), Vec3::new(hi.x, top, hi.y)), Paint::Concrete);
        }
    }

    /// A balcony on a south face (`front` is the middle of the wall's outer face), floor level `y`:
    /// a slab on two posts with a cover-height balustrade round it.
    pub(crate) fn balcony(&mut self, front: Vec2, y: f32, trim: Paint) {
        let (x0, x1, z0, z1) = (front.x - 2.6, front.x + 2.6, front.y - 2.0, front.y);
        self.solid(Aabb::span(Vec3::new(x0, y - 0.25, z0), Vec3::new(x1, y, z1)), Paint::Concrete);
        self.decor(Aabb::span(Vec3::new(x0 + 0.1, y, z0 + 0.1), Vec3::new(x1 - 0.1, y + 0.012, z1)), Paint::Bone);
        for (a, b, c, d) in [(x0, x1, z0, z0 + 0.14), (x0, x0 + 0.14, z0 + 0.14, z1), (x1 - 0.14, x1, z0 + 0.14, z1)] {
            self.solid(Aabb::span(Vec3::new(a, y, c), Vec3::new(b, y + COVER_HEIGHT, d)), trim);
        }
        let g = self.ground_at(Vec2::new(front.x, z0));
        for x in [x0 + 0.25, x1 - 0.25] {
            self.solid(Aabb::span(Vec3::new(x - 0.18, g - 0.1, z0 + 0.07), Vec3::new(x + 0.18, y - 0.25, z0 + 0.43)), trim);
        }
    }

    /// A barn's floor: hay stacks, and stalls along the north wall.
    pub(crate) fn furnish_barn(&mut self, rng: &mut Rng, room: (f32, f32, f32, f32), y: f32) {
        let (x0, x1, z0, z1) = room;
        for _ in 0..3 {
            let p = Vec2::new(rng.range(x0 + 1.0, x1 - 1.0), rng.range(z0, (z0 + z1) * 0.5));
            self.hay_stack(p, rng.chance(0.5), Some(y));
        }
        let mut x = x1 - 0.4;
        let stall = 2.8;
        while x - stall > x0 + 5.0 {
            self.solid(Aabb::span(Vec3::new(x - 0.07, y, z1 - 1.6), Vec3::new(x + 0.07, y + COVER_HEIGHT, z1 + 1.3)), Paint::Wood);
            x -= stall;
        }
    }

    /// A stack of square hay bales, cover height. `y` overrides the ground level.
    pub(crate) fn hay_stack(&mut self, p: Vec2, along_x: bool, y: Option<f32>) {
        let half = if along_x { Vec2::new(1.2, 0.55) } else { Vec2::new(0.55, 1.2) };
        let g = y.unwrap_or_else(|| self.lowest(p, half));
        self.solid(Aabb::span(Vec3::new(p.x - half.x, g - 0.1, p.y - half.y), Vec3::new(p.x + half.x, g + 0.6, p.y + half.y)), Paint::Gold);
        let top = Vec2::new(p.x + if along_x { 0.3 } else { 0.0 }, p.y + if along_x { 0.0 } else { 0.3 });
        let th = half - Vec2::splat(0.12) - if along_x { Vec2::new(0.3, 0.0) } else { Vec2::new(0.0, 0.3) };
        self.solid(Aabb::span(Vec3::new(top.x - th.x, g + 0.6, top.y - th.y), Vec3::new(top.x + th.x, g + COVER_HEIGHT, top.y + th.y)), Paint::Sand);
        // Twine: paint-thin.
        for k in [-0.5, 0.5] {
            let o = if along_x { Vec3::new(k * half.x, 0.0, 0.0) } else { Vec3::new(0.0, 0.0, k * half.y) };
            let h = if along_x { Vec3::new(0.03, 0.31, half.y + 0.02) } else { Vec3::new(half.x + 0.02, 0.31, 0.03) };
            self.decor(Aabb::new(Vec3::new(p.x, g + 0.3, p.y) + o, h), Paint::Rust);
        }
    }

    /// A low wall from `a` to `b` (one axis), following the ground in short runs.
    pub(crate) fn low_wall(&mut self, a: Vec2, b: Vec2, height: f32, thick: f32, paint: Paint) {
        let along_x = (b.x - a.x).abs() > (b.y - a.y).abs();
        let length = (b - a).length();
        let n = (length / 4.0).ceil().max(1.0) as usize;
        for i in 0..n {
            let p = a.lerp(b, i as f32 / n as f32);
            let q = a.lerp(b, (i + 1) as f32 / n as f32);
            let lo = p.min(q) - if along_x { Vec2::new(0.0, thick * 0.5) } else { Vec2::new(thick * 0.5, 0.0) };
            let hi = p.max(q) + if along_x { Vec2::new(0.0, thick * 0.5) } else { Vec2::new(thick * 0.5, 0.0) };
            let mid = (lo + hi) * 0.5;
            let g = self.highest(mid, (hi - lo) * 0.5);
            let bottom = self.lowest(mid, (hi - lo) * 0.5) - 0.15;
            // A run of low wall or fence that ends up across a doorway is cleared like furniture.
            let from = self.blocks.len();
            self.solid(Aabb::span(Vec3::new(lo.x, bottom, lo.y), Vec3::new(hi.x, g + height, hi.y)), paint);
            self.mark_furniture(from);
        }
    }

    /// A cover-height fence from `a` to `b` with gates (centres along it, 1.8 m wide) left open.
    pub(crate) fn fence(&mut self, a: Vec2, b: Vec2, gates: &[f32], paint: Paint) {
        let length = (b - a).length();
        if length < 0.5 {
            return;
        }
        let dir = (b - a) / length;
        let mut cuts: Vec<(f32, f32)> = gates.iter().map(|g| (g - 0.9, g + 0.9)).collect();
        cuts.sort_by(|p, q| p.0.total_cmp(&q.0));
        let mut cursor = 0.0;
        let mut runs = Vec::new();
        for (u0, u1) in cuts {
            if u0 > cursor + 0.3 {
                runs.push((cursor, u0));
            }
            cursor = cursor.max(u1);
        }
        if length > cursor + 0.3 {
            runs.push((cursor, length));
        }
        for (u0, u1) in runs {
            let (p, q) = (a + dir * u0, a + dir * u1);
            self.low_wall(p, q, COVER_HEIGHT, 0.1, paint);
            // Posts and a top rail: thin, walk-through detail.
            let mut u = u0;
            while u <= u1 + 0.01 {
                let at = a + dir * u;
                let g = self.ground_at(at);
                let from = self.blocks.len();
                self.decor(Aabb::span(Vec3::new(at.x - 0.08, g, at.y - 0.08), Vec3::new(at.x + 0.08, g + COVER_HEIGHT + 0.1, at.y + 0.08)), darker(paint));
                self.mark_furniture(from);
                u += 2.4;
            }
        }
    }

    /// A hedge: bushes in a row (hide in it; it stops nothing).
    pub(crate) fn hedge(&mut self, a: Vec2, b: Vec2, rng: &mut Rng) {
        let length = (b - a).length();
        let n = (length / 1.5).floor() as usize;
        for i in 0..=n {
            let p = a.lerp(b, i as f32 / n.max(1) as f32);
            if self.occupied(p, 0.5) {
                continue;
            }
            self.props.push(Prop { kind: PropKind::Bush, pos: Vec3::new(p.x, self.ground_at(p), p.y), yaw: rng.range(0.0, std::f32::consts::TAU), scale: rng.range(0.9, 1.15) });
        }
    }

    /// A U of sandbags round `c`, open on one side (south, west, east, north).
    pub(crate) fn sandbags(&mut self, c: Vec2, half: Vec2, open: usize) {
        let t = 0.35;
        let walls = [
            (Vec2::new(c.x - half.x, c.y - half.y), Vec2::new(c.x + half.x, c.y - half.y)),
            (Vec2::new(c.x - half.x, c.y - half.y + t), Vec2::new(c.x - half.x, c.y + half.y - t)),
            (Vec2::new(c.x + half.x, c.y - half.y + t), Vec2::new(c.x + half.x, c.y + half.y - t)),
            (Vec2::new(c.x - half.x, c.y + half.y), Vec2::new(c.x + half.x, c.y + half.y)),
        ];
        for (side, (a, b)) in walls.into_iter().enumerate() {
            if side == open {
                continue;
            }
            self.low_wall(a, b, COVER_HEIGHT, 2.0 * t, Paint::Sand);
            // Seams between the rows of bags: paint-thin.
            let along_x = (b.x - a.x).abs() > 0.1;
            let g = self.ground_at((a + b) * 0.5);
            for row in [0.4, 0.8] {
                let h = if along_x { Vec3::new((b.x - a.x).abs() * 0.5, 0.025, t + 0.02) } else { Vec3::new(t + 0.02, 0.025, (b.y - a.y).abs() * 0.5) };
                let m = (a + b) * 0.5;
                self.decor(Aabb::new(Vec3::new(m.x, g + row, m.y), h), Paint::Concrete);
            }
        }
    }

    /// A rock outcrop: a few boulders, solid, irregular but clean-faced for cover.
    pub(crate) fn outcrop(&mut self, rng: &mut Rng, c: Vec2, size: f32) {
        let skin = self.blocks.len();
        let n = 2 + rng.below(3);
        for i in 0..n {
            let p = c + Vec2::new(rng.range(-size, size), rng.range(-size, size)) * 0.7;
            let half = Vec2::new(rng.range(0.8, 1.6), rng.range(0.8, 1.6)) * size.min(2.5) * 0.6;
            let height = if i == 0 { rng.range(1.6, 2.8) * size.min(2.0) * 0.7 + 0.8 } else { rng.range(COVER_HEIGHT, 1.9) };
            let top = self.boxed(p, half, height, Paint::Rock);
            // A cap that overhangs a little.
            if rng.chance(0.5) {
                self.solid(Aabb::span(Vec3::new(p.x - half.x * 0.7, top, p.y - half.y * 0.7), Vec3::new(p.x + half.x * 0.6, top + 0.35, p.y + half.y * 0.8)), Paint::Rock);
            }
        }
        // Drawn as rock, not boxes, with stones round the foot.
        self.rock_skin(skin, Paint::Rock);
        self.scree(c, size + 1.5, 5, Paint::Rock);
        for _ in 0..3 {
            let p = c + Vec2::from_angle(rng.range(0.0, std::f32::consts::TAU)) * rng.range(size, size + 2.5);
            let (yaw, scale) = (rng.below(4) as f32 * std::f32::consts::FRAC_PI_2, rng.range(0.6, 1.2));
            if !self.occupied(p, 0.75 * scale + 0.3) {
                self.props.push(Prop { kind: PropKind::Rock, pos: Vec3::new(p.x, self.ground_at(p), p.y), yaw, scale });
            }
        }
    }

    /// A road or path from `a` to `b` (one axis), `width` wide, with kerbs and lane markings
    /// if `lines`. Paved when the map is finished (see `paving`).
    pub(crate) fn road(&mut self, a: Vec2, b: Vec2, width: f32, paint: Paint, lines: bool) {
        self.roads.push((a, b, width));
        self.lanes.push(super::paving::Lane { a, b, width, paint, lines });
    }

    /// A shipping container (`levels` high), with corrugation ribs.
    pub(crate) fn container(&mut self, rng: &mut Rng, at: Vec2, along_x: bool, levels: u32) {
        let colours = [Paint::Rust, Paint::Teal, Paint::Coral, Paint::Gold, Paint::Steel, Paint::Blue];
        let half = if along_x { Vec3::new(3.05, 1.3, 1.22) } else { Vec3::new(1.22, 1.3, 3.05) };
        let g = self.height(at.x, at.y);
        // On a slope, a concrete pad under it down to the lowest ground.
        let low = self.lowest(at, Vec2::new(half.x, half.z));
        if low < g - 0.05 {
            self.solid(Aabb::span(Vec3::new(at.x - half.x, low - 0.2, at.y - half.z), Vec3::new(at.x + half.x, g, at.y + half.z)), Paint::Concrete);
        }
        for level in 0..levels {
            let paint = *rng.pick(&colours).unwrap_or(&Paint::Rust);
            let c = Vec3::new(at.x, g + half.y + level as f32 * 2.6, at.y);
            self.solid(Aabb::new(c, half), paint);
            let rib = if along_x { Vec3::new(0.04, half.y * 0.95, half.z + 0.03) } else { Vec3::new(half.x + 0.03, half.y * 0.95, 0.04) };
            for k in -3..=3 {
                let offset = if along_x { Vec3::X * k as f32 * 0.85 } else { Vec3::Z * k as f32 * 0.85 };
                self.decor(Aabb::new(c + offset, rib), Paint::Dark);
            }
        }
    }

    /// A round column from `y0` to `y1`: an octagon of boxes to collide with, drawn smooth.
    pub(crate) fn column(&mut self, c: Vec2, r: f32, y0: f32, y1: f32, paint: Paint) {
        self.round_core(c, r, y0, y1, paint);
        self.round(c, r, y0, y1, paint, if r > 1.2 { 24 } else { 16 });
    }

    /// A grain silo: a smooth round tower with hoops and seams, a domed cap with a vent, a
    /// walkway rail round the rim and a ladder up to it.
    pub(crate) fn silo(&mut self, c: Vec2, r: f32, height: f32, paint: Paint, ladder: Option<Vec2>) {
        let g = self.lowest(c, Vec2::splat(r)) - 0.1;
        let top = self.highest(c, Vec2::splat(r)) + height;
        self.column(c, r, g, top, paint);
        // A concrete footing ring, hoops, and the vertical seams of the sheets.
        self.round(c, r + 0.25, g, self.highest(c, Vec2::splat(r)) + 0.35, Paint::Concrete, 24);
        self.hoops(c, r + 0.03, g + 1.6, top - 0.4, 1.8, darker(paint));
        for i in 0..8 {
            let a = Vec2::from_angle(i as f32 * TAU / 8.0 + 0.2) * (r + 0.02);
            self.trim_at(Vec3::new(c.x + a.x, (g + top) * 0.5, c.y + a.y), Vec3::new(0.035, (top - g) * 0.5, 0.035), darker(paint));
        }
        // The cap: hidden steps to stand on, drawn as a smooth dome over them.
        let cap = self.blocks.len();
        self.round_core(c, r * 0.72, top, top + 0.4, Paint::Metal);
        self.round_core(c, r * 0.45, top + 0.4, top + 0.8, Paint::Metal);
        let _ = cap;
        self.lathe(c, &[(r + 0.08, top - 0.02), (r + 0.08, top + 0.06), (r * 0.74, top + 0.43), (r * 0.47, top + 0.83), (r * 0.2, top + 0.95)], Paint::Metal, 24);
        // A vent on top (cover for whoever climbs up), drawn round with a little hat.
        self.round_core(c, 0.35, top + 0.8, top + 1.3, Paint::Dark);
        self.round(c, 0.35, top + 0.8, top + 1.3, Paint::Dark, 12);
        self.lathe(c, &[(0.5, top + 1.3), (0.5, top + 1.36), (0.0, top + 1.6)], Paint::Metal, 12);
        // A rail round the rim: thin posts and a ring.
        for i in 0..12 {
            let d = Vec2::from_angle(i as f32 * TAU / 12.0) * (r - 0.12);
            let foot = Vec3::new(c.x + d.x, top + 0.05, c.y + d.y);
            self.trim_rod(foot, foot + Vec3::Y * 0.95, 0.025, Paint::Iron);
        }
        for k in 0..12 {
            let (a, b) = (Vec2::from_angle(k as f32 * TAU / 12.0) * (r - 0.12), Vec2::from_angle((k + 1) as f32 * TAU / 12.0) * (r - 0.12));
            self.trim_rod(Vec3::new(c.x + a.x, top + 1.0, c.y + a.y), Vec3::new(c.x + b.x, top + 1.0, c.y + b.y), 0.03, Paint::Iron);
        }
        // A chute and a hatch near the foot.
        let side = ladder.map_or(Vec2::NEG_Y, |o| Vec2::new(o.y, -o.x));
        let hatch = c + side * (r + 0.02);
        self.trim_at(Vec3::new(hatch.x, g + 1.2, hatch.y), Vec3::new(0.45 * side.y.abs() + 0.03, 0.5, 0.45 * side.x.abs() + 0.03), darker(paint));
        if let Some(out) = ladder {
            self.ladder_up(c + out * r, out, top);
            // A safety cage round the upper ladder.
            let p = c + out * (r + 0.22);
            let mut y = top - 4.0;
            while y < top + 0.8 {
                if y > g + 2.4 {
                    let q = p + out * 0.45;
                    self.trim_rod(Vec3::new(p.x, y, p.y) + Vec3::new(out.y, 0.0, -out.x) * 0.38, Vec3::new(q.x, y, q.y) + Vec3::new(out.y, 0.0, -out.x) * 0.38, 0.02, Paint::Iron);
                    self.trim_rod(Vec3::new(p.x, y, p.y) - Vec3::new(out.y, 0.0, -out.x) * 0.38, Vec3::new(q.x, y, q.y) - Vec3::new(out.y, 0.0, -out.x) * 0.38, 0.02, Paint::Iron);
                    self.trim_rod(Vec3::new(q.x, y, q.y) + Vec3::new(out.y, 0.0, -out.x) * 0.38, Vec3::new(q.x, y, q.y) - Vec3::new(out.y, 0.0, -out.x) * 0.38, 0.02, Paint::Iron);
                }
                y += 0.9;
            }
        }
    }

    /// A ladder on a wall whose face is at `face` (facing `out`), from the ground up to `top`.
    pub(crate) fn ladder_up(&mut self, face: Vec2, out: Vec2, top: f32) {
        let p = face + out * 0.22;
        let foot = Vec3::new(p.x, self.ground_at(p), p.y);
        let side = Vec2::new(-out.y, out.x) * 0.3;
        for s in [-1.0, 1.0] {
            let rail = p + side * s;
            self.decor(Aabb::span(Vec3::new(rail.x - 0.04, foot.y, rail.y - 0.04), Vec3::new(rail.x + 0.04, top + 1.0, rail.y + 0.04)), Paint::Iron);
        }
        let mut y = foot.y + 0.3;
        let rung = if out.x.abs() > 0.5 { Vec3::new(0.03, 0.03, 0.3) } else { Vec3::new(0.3, 0.03, 0.03) };
        while y < top + 0.6 {
            self.decor(Aabb::new(Vec3::new(p.x, y, p.y), rung), Paint::Iron);
            y += 0.35;
        }
        self.ladders.push(Ladder { foot, top, out });
    }

    /// A water tower: four legs, a deck with a rail round it (and a gap for the ladder), a tank.
    pub(crate) fn water_tower(&mut self, c: Vec2, deck: f32, paint: Paint) {
        let g = self.ground_at(c);
        let top = g + deck;
        for (dx, dz) in [(-2.1, -2.1), (2.1, -2.1), (-2.1, 2.1), (2.1, 2.1)] {
            self.solid(Aabb::span(Vec3::new(c.x + dx - 0.16, g - 0.2, c.y + dz - 0.16), Vec3::new(c.x + dx + 0.16, top - 0.2, c.y + dz + 0.16)), Paint::Iron);
        }
        // Cross braces: thin.
        for (dx, dz, along_x) in [(0.0, -2.1, true), (0.0, 2.1, true), (-2.1, 0.0, false), (2.1, 0.0, false)] {
            let mut y = g + 2.4;
            while y < top - 1.0 {
                let h = if along_x { Vec3::new(2.1, 0.04, 0.04) } else { Vec3::new(0.04, 0.04, 2.1) };
                self.decor(Aabb::new(Vec3::new(c.x + dx, y, c.y + dz), h), Paint::Iron);
                y += 2.6;
            }
        }
        self.solid(Aabb::span(Vec3::new(c.x - 3.0, top - 0.2, c.y - 3.0), Vec3::new(c.x + 3.0, top, c.y + 3.0)), Paint::Iron);
        // Rail: thin, on three sides and both ends of the fourth, leaving the ladder's way on.
        for (a, b) in [((-3.0, -3.0), (3.0, -2.94)), ((-3.0, 2.94), (3.0, 3.0)), ((-3.0, -2.94), (-2.94, 2.94)), ((2.94, -2.94), (3.0, -0.8)), ((2.94, 0.8), (3.0, 2.94))] {
            self.solid(Aabb::span(Vec3::new(c.x + a.0, top + 0.95, c.y + a.1), Vec3::new(c.x + b.0, top + 1.03, c.y + b.1)), Paint::Iron);
        }
        for (dx, dz) in [(-3.0, -3.0), (3.0, -3.0), (-3.0, 3.0), (3.0, 3.0), (2.97, -0.8), (2.97, 0.8)] {
            self.pole(Vec3::new(c.x + dx, top, c.y + dz), 1.0, Paint::Iron);
        }
        // The tank: a smooth drum with hoops and a conical roof over hidden steps.
        self.column(c, 2.1, top, top + 3.2, paint);
        self.hoops(c, 2.13, top + 0.6, top + 3.0, 1.1, darker(paint));
        self.round_core(c, 1.5, top + 3.2, top + 3.6, darker(paint));
        self.round_core(c, 0.8, top + 3.6, top + 4.0, darker(paint));
        self.lathe(c, &[(2.3, top + 3.12), (2.3, top + 3.22), (1.52, top + 3.63), (0.82, top + 4.03), (0.0, top + 4.35)], darker(paint), 24);
        self.lathe(c, &[(0.16, top + 4.3), (0.0, top + 4.9)], Paint::Iron, 8);
        // A painted band and an overflow pipe down one leg.
        self.round(c, 2.12, top + 1.7, top + 2.2, Paint::Cream, 24);
        self.trim_rod(Vec3::new(c.x - 1.9, top + 0.2, c.y - 0.6), Vec3::new(c.x - 2.1, g + 0.2, c.y - 2.0), 0.08, Paint::Iron);
        self.ladder_up(Vec2::new(c.x + 3.0, c.y), Vec2::X, top);
    }

    /// A windmill: a tapering tower, a hipped cap and four sails.
    pub(crate) fn windmill(&mut self, c: Vec2, paint: Paint) {
        let g = self.lowest(c, Vec2::splat(2.6)) - 0.1;
        let mut y = g;
        let mut half = 2.6;
        for tier in 0..4 {
            let h = if tier == 0 { 3.0 } else { 2.5 };
            self.solid(Aabb::span(Vec3::new(c.x - half, y, c.y - half), Vec3::new(c.x + half, y + h, c.y + half)), if tier % 2 == 0 { paint } else { darker(paint) });
            y += h;
            half -= 0.35;
        }
        self.stepped_roof((c.x - half - 0.4, c.x + half + 0.4, c.y - half - 0.4, c.y + half + 0.4), y, true, true, (Paint::Dark, Paint::Dark));
        // A door and windows painted on: the mill is solid (cover all round).
        self.decor(Aabb::span(Vec3::new(c.x - 0.6, g, c.y - 2.62), Vec3::new(c.x + 0.6, g + 2.3, c.y - 2.6)), Paint::Wood);
        // The sails, on a hub out of the south face: paint-thin vanes.
        let hub = Vec3::new(c.x, y - 0.8, c.y - half - 0.7);
        self.decor(Aabb::new(hub, Vec3::new(0.25, 0.25, 0.04)), Paint::Dark);
        // The axle the hub turns on, out of the cap.
        self.decor(Aabb::span(Vec3::new(hub.x - 0.08, hub.y - 0.08, hub.z), Vec3::new(hub.x + 0.08, hub.y + 0.08, c.y - half)), Paint::Dark);
        let vane = |len: f32, vertical: bool| if vertical { Vec3::new(0.45, len, 0.04) } else { Vec3::new(len, 0.45, 0.04) };
        for (offset, vertical) in [(Vec3::Y * 3.4, true), (Vec3::Y * -3.4, true), (Vec3::X * 3.4, false), (Vec3::X * -3.4, false)] {
            self.decor(Aabb::new(hub + offset - Vec3::Z * 0.3, vane(3.0, vertical)), Paint::Bone);
        }
    }

    /// A tall pine made of blocks: a solid trunk to hide behind and tiers of needles high
    /// overhead, out of anyone's way (keep it clear of anything you could stand on). Only on
    /// level ground, where nobody can walk up beside the needles; returns whether it went in.
    pub(crate) fn pine(&mut self, p: Vec2, height: f32) -> bool {
        let reach = Vec2::splat(1.9 * height / 8.0 + 0.6);
        if self.highest(p, reach) - self.lowest(p, reach) > 0.5 {
            return false;
        }
        let g = self.ground_at(p);
        // A solid stump to hide behind (drawn round), a tapering trunk and cones of needles.
        self.round_core(p, 0.27, g - 0.2, g + 1.0, Paint::Wood);
        self.round(p, 0.27, g - 0.2, g + 1.0, Paint::Wood, 10);
        self.lathe(p, &[(0.22, g + 1.0), (0.06, g + height * 0.85)], Paint::Wood, 8);
        let mut rng = self.detail_rng(p, 0x91e);
        let tiers = 4;
        let span = height - 4.2;
        for i in 0..tiers {
            let t = i as f32 / tiers as f32;
            let r = (1.9 - 1.4 * t) * height / 8.0 * rng.range(0.9, 1.1);
            let y0 = g + 4.2 + span * t;
            let y1 = y0 + span / tiers as f32 + 1.3;
            let at = p + Vec2::new(rng.range(-0.08, 0.08), rng.range(-0.08, 0.08));
            self.lathe(at, &[(r * 0.55, y0 - 0.25), (r, y0 + 0.15), (0.0, y1)], if i % 2 == 0 { Paint::Leaf } else { Paint::Shell }, 9);
        }
        true
    }

    /// A cactus: a solid trunk and arms.
    pub(crate) fn cactus(&mut self, rng: &mut Rng, p: Vec2) {
        let g = self.ground_at(p);
        let h = rng.range(2.0, 3.4);
        self.solid(Aabb::span(Vec3::new(p.x - 0.22, g - 0.1, p.y - 0.22), Vec3::new(p.x + 0.22, g + h, p.y + 0.22)), Paint::Leaf);
        if rng.chance(0.7) {
            let s = if rng.chance(0.5) { 1.0 } else { -1.0 };
            let y = g + rng.range(1.0, h - 0.6);
            self.solid(Aabb::span(Vec3::new(p.x + s * 0.22, y, p.y - 0.15), Vec3::new(p.x + s * 0.75, y + 0.3, p.y + 0.15)), Paint::Leaf);
            self.solid(Aabb::span(Vec3::new(p.x + s * 0.5, y + 0.3, p.y - 0.15), Vec3::new(p.x + s * 0.78, y + 1.0, p.y + 0.15)), Paint::Leaf);
        }
    }

    /// A picnic table with benches (low cover).
    pub(crate) fn picnic_table(&mut self, p: Vec2, along_x: bool, y: Option<f32>) {
        let g = y.unwrap_or_else(|| self.ground_at(p));
        // Legs reach down to the ground where it falls away.
        let foot = y.unwrap_or_else(|| self.lowest(p, Vec2::splat(1.0)) - 0.05);
        let (t, bench) = if along_x { (Vec2::new(1.0, 0.4), Vec2::new(0.0, 0.75)) } else { (Vec2::new(0.4, 1.0), Vec2::new(0.75, 0.0)) };
        self.solid(Aabb::span(Vec3::new(p.x - t.x, foot, p.y - t.y), Vec3::new(p.x + t.x, g + 0.78, p.y + t.y)), Paint::Wood);
        for s in [-1.0, 1.0] {
            let b = p + bench * s;
            let h = if along_x { Vec2::new(1.0, 0.17) } else { Vec2::new(0.17, 1.0) };
            self.solid(Aabb::span(Vec3::new(b.x - h.x, foot, b.y - h.y), Vec3::new(b.x + h.x, g + 0.45, b.y + h.y)), Paint::Wood);
        }
    }

    /// A picnic shelter: a roof on four posts over two tables.
    pub(crate) fn shelter(&mut self, p: Vec2, half: Vec2, roof: Paint) {
        let g = self.highest(p, half);
        // A concrete base, level, down to the lowest ground under it.
        let low = self.lowest(p, half);
        if low < g - 0.05 {
            self.solid(Aabb::span(Vec3::new(p.x - half.x, low - 0.2, p.y - half.y), Vec3::new(p.x + half.x, g, p.y + half.y)), Paint::Concrete);
        }
        for (sx, sz) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            let q = p + Vec2::new(sx * (half.x - 0.2), sz * (half.y - 0.2));
            self.solid(Aabb::span(Vec3::new(q.x - 0.14, g - 0.3, q.y - 0.14), Vec3::new(q.x + 0.14, g + 2.7, q.y + 0.14)), Paint::Wood);
        }
        self.solid(Aabb::span(Vec3::new(p.x - half.x - 0.3, g + 2.7, p.y - half.y - 0.3), Vec3::new(p.x + half.x + 0.3, g + 2.95, p.y + half.y + 0.3)), Paint::Wood);
        self.stepped_roof((p.x - half.x - 0.3, p.x + half.x + 0.3, p.y - half.y - 0.3, p.y + half.y + 0.3), g + 2.95, false, half.x >= half.y, (roof, Paint::Wood));
        self.flat(p.x - half.x, p.x + half.x, p.y - half.y, p.y + half.y, g + 0.01, Paint::Concrete);
        let along_x = half.x >= half.y;
        for s in [-0.5, 0.5] {
            let q = if along_x { p + Vec2::new(s * half.x, 0.0) } else { p + Vec2::new(0.0, s * half.y) };
            self.picnic_table(q, !along_x, Some(g));
        }
    }

    /// A footbridge across a creek between two banks (one axis), deck at `y`.
    pub(crate) fn footbridge(&mut self, a: Vec2, b: Vec2, width: f32, y: f32) {
        let along_x = (b.x - a.x).abs() > (b.y - a.y).abs();
        let w = if along_x { Vec2::new(0.0, width * 0.5) } else { Vec2::new(width * 0.5, 0.0) };
        let (lo, hi) = (a.min(b) - w, a.max(b) + w);
        self.solid(Aabb::span(Vec3::new(lo.x, y - 0.3, lo.y), Vec3::new(hi.x, y, hi.y)), Paint::Wood);
        // Stone under the deck down to the bed: nobody wades in under the planks.
        let bed = self.lowest((lo + hi) * 0.5, (hi - lo) * 0.5).min(self.water - 1.0);
        self.solid(Aabb::span(Vec3::new(lo.x + 0.1, bed - 0.3, lo.y + 0.1), Vec3::new(hi.x - 0.1, y - 0.3, hi.y - 0.1)), Paint::Rock);
        // Planks and hand rails: thin.
        let n = ((b - a).length() / 0.5) as usize;
        for i in 0..n {
            let p = a.lerp(b, (i as f32 + 0.5) / n as f32);
            let h = if along_x { Vec3::new(0.02, 0.01, width * 0.5) } else { Vec3::new(width * 0.5, 0.01, 0.02) };
            self.decor(Aabb::new(Vec3::new(p.x, y + 0.01, p.y), h), Paint::Dark);
        }
        for s in [-1.0, 1.0] {
            let o = w * s * 0.95;
            let (p, q) = (a + o, b + o);
            let (l, h) = (p.min(q), p.max(q));
            self.decor(Aabb::span(Vec3::new(l.x - 0.04, y + 0.95, l.y - 0.04), Vec3::new(h.x + 0.04, y + 1.02, h.y + 0.04)), Paint::Wood);
            let m = ((b - a).length() / 2.0).ceil() as usize;
            for i in 0..=m {
                let r = p.lerp(q, i as f32 / m as f32);
                self.pole(Vec3::new(r.x, y, r.y), 1.0, Paint::Wood);
            }
        }
    }

    // --- Buildings ---------------------------------------------------------------------------

    /// A house facing a street on side `front`: a door there, a pitched roof, a chimney, a home
    /// inside, and sometimes a porch. Returns the footprint's (x0, x1, z0, z1).
    pub(crate) fn house(&mut self, rng: &mut Rng, c: Vec2, half: Vec2, floors: u32, front: usize) -> (f32, f32, f32, f32) {
        let mut plan = Plan { furnish: Furnish::Home, chimney: true, random_doors: false, ..Plan::default() };
        let (x0, x1, z0, z1) = (c.x - half.x, c.x + half.x, c.y - half.y, c.y + half.y);
        // The front door, and a back door.
        let door_center = |side: usize, rng: &mut Rng| if side == 0 || side == 3 { rng.range(x0 + 1.8, x1 - 1.8) } else { rng.range(z0 + 2.1, z1 - 2.1) };
        let front = if floors > 1 && front == 3 { 3 } else { front };
        let d = door_center(front, rng);
        // Upstairs houses keep the stair lane clear: the north door goes east of it.
        let d = if floors > 1 && front == 3 { d.max(x0 + 6.6).min(x1 - 1.2) } else { d };
        plan.doors[front] = Some((d, DOOR_WIDTH));
        let back = 3 - front;
        if rng.chance(0.7) && !(floors > 1 && back == 3) {
            plan.doors[back] = Some((door_center(back, rng), DOOR_WIDTH));
        }
        if floors > 1 && front == 0 && rng.chance(0.4) && half.x > 4.0 {
            plan.balcony = true;
            plan.doors[0] = Some((c.x + if rng.chance(0.5) { 2.8 } else { -2.8 }, DOOR_WIDTH));
        }
        let (g, _) = self.building_plan(rng, c, half, floors, &plan);
        if rng.chance(0.45) && !plan.balcony {
            self.porch(rng, c, half, front, g);
        }
        (x0, x1, z0, z1)
    }

    /// A porch on side `front`: a deck, a rail of cover height with a gap at the steps, a roof on posts.
    pub(crate) fn porch(&mut self, rng: &mut Rng, c: Vec2, half: Vec2, front: usize, g: f32) {
        let out = side_out(front);
        let depth = 2.2;
        let along = Vec2::new(out.y.abs(), out.x.abs());
        let span = if front == 0 || front == 3 { half.x } else { half.y } - 0.6;
        let face = c + out * if front == 0 || front == 3 { half.y } else { half.x };
        let lo = (face - along * span).min(face + along * span + out * depth);
        let hi = (face - along * span).max(face + along * span + out * depth);
        let ground = self.lowest((lo + hi) * 0.5, (hi - lo) * 0.5);
        self.solid(Aabb::span(Vec3::new(lo.x, ground - 0.2, lo.y), Vec3::new(hi.x, g, hi.y)), Paint::Wood);
        // Posts and a roof.
        for s in [-1.0, 1.0] {
            let p = face + along * s * (span - 0.15) + out * (depth - 0.15);
            self.solid(Aabb::span(Vec3::new(p.x - 0.12, g, p.y - 0.12), Vec3::new(p.x + 0.12, g + 3.0, p.y + 0.12)), Paint::Bone);
        }
        // High enough that someone standing on the rail still clears it.
        self.solid(Aabb::span(Vec3::new(lo.x - 0.1, g + 3.0, lo.y - 0.1), Vec3::new(hi.x + 0.1, g + 3.2, hi.y + 0.1)), Paint::Dark);
        // A rail along the front with a gap in the middle for the steps, if the deck stands high.
        let front_mid = face + out * (depth - 0.06);
        let _ = rng;
        for s in [-1.0, 1.0] {
            let a = front_mid + along * s * 1.1;
            let b = front_mid + along * s * span;
            let (l, h) = (a.min(b) - Vec2::splat(0.06), a.max(b) + Vec2::splat(0.06));
            self.solid(Aabb::span(Vec3::new(l.x, g, l.y), Vec3::new(h.x, g + 0.95, h.y)), Paint::Bone);
        }
        self.stoop(front_mid + out * 0.06, out, 2.0, g);
    }

    /// An L-shaped house: a main block and a one-storey wing off its east or west end, joined
    /// through a doorway, with flush outside corners.
    pub(crate) fn l_house(&mut self, rng: &mut Rng, c: Vec2, half: Vec2, floors: u32, front: usize, wing_east: bool) {
        let (x0, x1, z0, z1) = (c.x - half.x, c.x + half.x, c.y - half.y, c.y + half.y);
        let wing = Vec2::new(rng.range(2.8, 3.6), (half.y * rng.range(0.55, 0.75)).max(2.9));
        // The wing lines up with the front face.
        let wz = if front == 3 { z1 - wing.y } else { z0 + wing.y };
        let wx = if wing_east { x1 + wing.x } else { x0 - wing.x };
        let join = if wing_east { 2 } else { 1 };
        let mut main = Plan { furnish: Furnish::Home, chimney: true, random_doors: false, ..Plan::default() };
        let door = if front == 0 || front == 3 { rng.range(x0 + 1.8, x1 - 1.8).max(if floors > 1 && front == 3 { x0 + 6.6 } else { f32::MIN }) } else { rng.range(z0 + 2.1, z1 - 2.1) };
        main.doors[front] = Some((door, DOOR_WIDTH));
        main.doors[join] = Some((wz, DOOR_WIDTH));
        let tiles = *rng.pick(&self.style.roofs).unwrap_or(&Paint::Rust);
        main.roof = Roof::Gable(tiles);
        main.walls = Some(*rng.pick(&self.style.walls).unwrap_or(&Paint::Bone));
        let (g, _) = self.building_plan(rng, c, half, floors, &main);
        let mut open = [false; 4];
        open[3 - join] = true;
        let mut side = Plan { furnish: Furnish::Plain, random_doors: false, floor: Some(g), walls: main.walls, roof: Roof::Gable(tiles), ladder: false, ..Plan::default() };
        side.open = open;
        // A door out of the wing on its far side.
        side.doors[join] = Some((wz, DOOR_WIDTH));
        self.building_plan(rng, Vec2::new(wx, wz), wing, 1, &side);
    }

    /// A garage with a wide open mouth on side `front`, sometimes a car inside.
    pub(crate) fn garage(&mut self, rng: &mut Rng, c: Vec2, front: usize) {
        let along_z = front == 0 || front == 3;
        let half = if along_z { Vec2::new(2.2, 3.3) } else { Vec2::new(3.3, 2.2) };
        let mut plan = Plan { furnish: Furnish::Bare, random_doors: false, windows: false, ladder: rng.chance(0.5), ..Plan::default() };
        plan.doors[front] = Some((if along_z { c.x } else { c.y }, 3.0));
        plan.roof = if rng.chance(0.5) { Roof::Flat } else { Roof::Gable(*rng.pick(&self.style.roofs).unwrap_or(&Paint::Dark)) };
        let (g, _) = self.building_plan(rng, c, half, 1, &plan);
        if rng.chance(0.5) {
            // Boxes stored inside (a parked car would be too tall to stand on under this roof).
            self.clutter.push(Clutter { kind: ClutterKind::Crates, pos: Vec3::new(c.x, g, c.y), turned: false });
        } else {
            // A workbench and shelves at the back.
            let back = c - side_out(front) * (if along_z { half.y } else { half.x } - 0.75);
            let h = if along_z { Vec2::new(1.4, 0.35) } else { Vec2::new(0.35, 1.4) };
            self.solid(Aabb::span(Vec3::new(back.x - h.x, g, back.y - h.y), Vec3::new(back.x + h.x, g + 0.95, back.y + h.y)), Paint::Wood);
        }
    }

    /// A mansion: a two-storey hall with a hipped roof and a balcony over the door, a one-storey
    /// wing at each end joined through doorways, a fountain in front.
    pub(crate) fn mansion(&mut self, rng: &mut Rng, c: Vec2, front_fountain: bool) {
        let half = Vec2::new(rng.range(7.8, 8.8), rng.range(6.0, 6.8));
        let (x0, x1, z0) = (c.x - half.x, c.x + half.x, c.y - half.y);
        let walls = *rng.pick(&[Paint::Bone, Paint::Sand, Paint::Concrete]).unwrap_or(&Paint::Bone);
        let tiles = *rng.pick(&[Paint::Dark, Paint::Steel, Paint::Rust]).unwrap_or(&Paint::Dark);
        let wing = Vec2::new(rng.range(3.4, 3.9), rng.range(4.0, 4.8));
        let wz = z0 + wing.y;
        let mut main = Plan { walls: Some(walls), trim: Some(Paint::Steel), roof: Roof::Hip(tiles), furnish: Furnish::Home, random_doors: false, balcony: true, chimney: true, ..Plan::default() };
        main.doors[1] = Some((wz, DOOR_WIDTH));
        main.doors[2] = Some((wz, DOOR_WIDTH));
        main.doors[3] = Some((c.x + 3.0, DOOR_WIDTH));
        // The front door has the balcony over it, so it moves off-centre of the balcony posts.
        main.doors[0] = Some((c.x + 1.4, DOOR_WIDTH));
        let (g, _) = self.building_plan(rng, c, half, 2, &main);
        for (east, x) in [(false, x0 - wing.x), (true, x1 + wing.x)] {
            let mut plan = Plan { walls: Some(walls), trim: Some(Paint::Steel), roof: Roof::Hip(tiles), furnish: Furnish::Home, random_doors: false, floor: Some(g), ladder: false, ..Plan::default() };
            plan.open[if east { 1 } else { 2 }] = true;
            plan.doors[0] = Some((x, DOOR_WIDTH));
            self.building_plan(rng, Vec2::new(x, wz), wing, 1, &plan);
        }
        // A terrace along the front, low walls, and the fountain.
        let front = z0 - 6.0;
        if front_fountain {
            self.plaza_at(rng, Vec2::new(c.x, front - 4.0));
        }
        for s in [-1.0, 1.0] {
            let a = Vec2::new(c.x + s * 3.0, z0 - 1.2);
            let b = Vec2::new(c.x + s * (half.x - 0.5), z0 - 1.2);
            self.low_wall(a, b, COVER_HEIGHT * 0.75, 0.5, Paint::Bone);
        }
    }

    /// A barn: a big gabled hall with a hayloft, wide doors at both gable ends.
    pub(crate) fn barn(&mut self, rng: &mut Rng, c: Vec2, half: Vec2) {
        let mut plan = Plan { walls: Some(Paint::Rust), trim: Some(Paint::Bone), floor_paint: Paint::Sand, roof: Roof::Gable(Paint::Dark), furnish: Furnish::Barn, random_doors: false, ..Plan::default() };
        plan.doors[1] = Some((c.y - 0.5, 3.0));
        plan.doors[2] = Some((c.y - 0.5, 3.0));
        plan.doors[0] = Some((c.x + half.x * 0.4, DOOR_WIDTH));
        let (g, _) = self.building_plan(rng, c, half, 2, &plan);
        // White cross-bracing on the big doors' faces: paint-thin.
        for x in [c.x - half.x - 0.02, c.x + half.x + 0.02] {
            self.decor(Aabb::span(Vec3::new(x - 0.02, g + 2.95, c.y - 2.2), Vec3::new(x + 0.02, g + 3.1, c.y + 1.2)), Paint::Bone);
        }
    }

    /// A shack: one small room of corrugated iron or planks.
    pub(crate) fn shack(&mut self, rng: &mut Rng, c: Vec2) {
        let half = Vec2::new(rng.range(2.4, 3.2), rng.range(2.2, 2.8));
        let walls = *rng.pick(&[Paint::Rust, Paint::Wood, Paint::Steel, Paint::Iron]).unwrap_or(&Paint::Rust);
        let roof = if rng.chance(0.5) { Roof::Flat } else { Roof::Gable(*rng.pick(&[Paint::Rust, Paint::Iron, Paint::Dark]).unwrap_or(&Paint::Rust)) };
        let ladder = rng.chance(0.3);
        let (g, roof_y) = self.building_plan(rng, c, half, 1, &Plan { walls: Some(walls), roof, ladder, ..Plan::default() });
        // Corrugation on the long faces: paint-thin ribs.
        let _ = g;
        let mut x = c.x - half.x + 0.4;
        while x < c.x + half.x - 0.3 {
            for z in [c.y - half.y - 0.03, c.y + half.y + 0.03] {
                self.decor(Aabb::new(Vec3::new(x, (g + roof_y) * 0.5 + 0.6, z), Vec3::new(0.03, (roof_y - g) * 0.25, 0.02)), Paint::Dark);
            }
            x += 0.9;
        }
    }

    /// An adobe hut: flat roof with a parapet, beams poking out under it, sometimes a second storey.
    /// `door` fixes the side its door is on (else it gets one or two at random).
    pub(crate) fn hut(&mut self, rng: &mut Rng, c: Vec2, half: Vec2, floors: u32, door: Option<usize>) {
        let walls = *rng.pick(&[Paint::Sand, Paint::Bone, Paint::Concrete]).unwrap_or(&Paint::Sand);
        let mut plan = Plan { walls: Some(walls), trim: Some(walls), roof: Roof::Flat, floor_paint: Paint::Sand, ..Plan::default() };
        if let Some(side) = door {
            plan.random_doors = false;
            let along = if side == 0 || side == 3 { c.x } else { c.y };
            plan.doors[side] = Some((along + rng.range(-0.6, 0.6), DOOR_WIDTH));
        }
        let (_, roof_y) = self.building_plan(rng, c, half, floors, &plan);
        // Vigas: roof beams sticking out of the walls, too thin to hide behind.
        let mut x = c.x - half.x + 0.8;
        while x < c.x + half.x - 0.5 {
            for (z, s) in [(c.y - half.y, -1.0), (c.y + half.y, 1.0)] {
                self.decor(Aabb::span(Vec3::new(x - 0.08, roof_y - 0.35, z), Vec3::new(x + 0.08, roof_y - 0.2, z + s * 0.5)), Paint::Wood);
            }
            x += 1.3;
        }
    }

    /// A log cabin with a porch.
    pub(crate) fn cabin(&mut self, rng: &mut Rng, c: Vec2, front: usize) {
        let half = Vec2::new(rng.range(3.4, 4.4), rng.range(3.0, 3.6));
        let mut plan = Plan { walls: Some(Paint::Wood), trim: Some(Paint::Dark), roof: Roof::Gable(*rng.pick(&[Paint::Rust, Paint::Dark, Paint::Shell]).unwrap_or(&Paint::Rust)), furnish: Furnish::Home, random_doors: false, chimney: true, ..Plan::default() };
        let along = if front == 0 || front == 3 { c.x } else { c.y };
        plan.doors[front] = Some((along + rng.range(-1.0, 1.0), DOOR_WIDTH));
        if rng.chance(0.5) {
            plan.doors[3 - front] = Some((along, DOOR_WIDTH));
        }
        let (g, roof_y) = self.building_plan(rng, c, half, 1, &plan);
        self.porch(rng, c, half, front, g);
        // Log courses: paint-thin lines.
        let mut y = g + 0.45;
        while y < roof_y - 0.2 {
            self.decor(Aabb::new(Vec3::new(c.x, y, c.y - half.y - 0.02), Vec3::new(half.x, 0.025, 0.02)), Paint::Dark);
            self.decor(Aabb::new(Vec3::new(c.x, y, c.y + half.y + 0.02), Vec3::new(half.x, 0.025, 0.02)), Paint::Dark);
            y += 0.55;
        }
    }

    /// A warehouse: one big hall, a wide loading door, crates inside, skylights.
    pub(crate) fn warehouse(&mut self, rng: &mut Rng, c: Vec2, half: Vec2, front: usize) {
        let mut plan = Plan { walls: Some(*rng.pick(&[Paint::Steel, Paint::Rust, Paint::Iron, Paint::Teal]).unwrap_or(&Paint::Steel)), roof: Roof::Flat, furnish: Furnish::Bare, ..Plan::default() };
        plan.doors[front] = Some((if front == 0 || front == 3 { c.x } else { c.y }, 3.2));
        let (g, roof_y) = self.building_plan(rng, c, half, 1, &plan);
        for _ in 0..4 {
            let p = c + Vec2::new(rng.range(-half.x + 2.0, half.x - 2.0), rng.range(-half.y + 2.0, half.y - 2.0));
            if self.occupied(p, 1.6) {
                continue;
            }
            self.props.push(Prop { kind: PropKind::Crate, pos: Vec3::new(p.x, g, p.y), yaw: 0.0, scale: 1.0 });
            if rng.chance(0.5) {
                self.props.push(Prop { kind: PropKind::Crate, pos: Vec3::new(p.x, g + 1.1, p.y), yaw: 0.0, scale: 1.0 });
            }
        }
        for k in [-0.5, 0.0, 0.5] {
            self.decor(Aabb::new(Vec3::new(c.x + k * half.x, roof_y + 0.32, c.y), Vec3::new(0.9, 0.02, half.y * 0.6)), Paint::Blue);
        }
    }

    // --- Office furniture --------------------------------------------------------------------

    /// A desk (half cover) facing `-face` with a monitor, keyboard, papers; a chair behind it.
    pub(crate) fn desk(&mut self, rng: &mut Rng, p: Vec2, face: Vec2, y: f32) {
        let along_x = face.y.abs() > 0.5;
        let half = if along_x { Vec2::new(0.8, 0.4) } else { Vec2::new(0.4, 0.8) };
        self.solid(Aabb::span(Vec3::new(p.x - half.x, y, p.y - half.y), Vec3::new(p.x + half.x, y + 0.76, p.y + half.y)), *rng.pick(&[Paint::Wood, Paint::Bone]).unwrap_or(&Paint::Wood));
        // Screen and stand, keyboard, papers: thin.
        let back = p - face * 0.22;
        let screen = if along_x { Vec3::new(0.3, 0.2, 0.03) } else { Vec3::new(0.03, 0.2, 0.3) };
        self.decor(Aabb::new(Vec3::new(back.x, y + 1.06, back.y), screen), Paint::Screen);
        self.decor(Aabb::new(Vec3::new(back.x, y + 1.06, back.y) + Vec3::new(face.x, 0.0, face.y) * 0.035, screen * Vec3::new(0.85, 0.8, 0.85) * if along_x { Vec3::new(1.0, 1.0, 0.3) } else { Vec3::new(0.3, 1.0, 1.0) }), Paint::Blue);
        self.decor(Aabb::new(Vec3::new(back.x, y + 0.83, back.y), Vec3::new(0.04, 0.07, 0.04)), Paint::Dark);
        let kb = p + face * 0.1;
        let key = if along_x { Vec3::new(0.24, 0.012, 0.08) } else { Vec3::new(0.08, 0.012, 0.24) };
        self.decor(Aabb::new(Vec3::new(kb.x, y + 0.775, kb.y), key), Paint::Dark);
        if rng.chance(0.6) {
            let o = if along_x { Vec2::new(rng.range(0.4, 0.6), 0.0) } else { Vec2::new(0.0, rng.range(0.4, 0.6)) };
            let paper = p + o;
            self.decor(Aabb::new(Vec3::new(paper.x, y + 0.775, paper.y), Vec3::new(0.11, 0.008 + 0.01 * rng.unit(), 0.15)), Paint::White);
        }
        self.office_chair(p + face * 0.85, face, y);
    }

    /// An office chair: a solid seat; the back and base are thin.
    pub(crate) fn office_chair(&mut self, p: Vec2, face: Vec2, y: f32) {
        self.solid(Aabb::span(Vec3::new(p.x - 0.24, y, p.y - 0.24), Vec3::new(p.x + 0.24, y + 0.48, p.y + 0.24)), Paint::Dark);
        let back = p + face * 0.24;
        let h = if face.y.abs() > 0.5 { Vec3::new(0.22, 0.26, 0.03) } else { Vec3::new(0.03, 0.26, 0.22) };
        // The back rises from the seat.
        self.decor(Aabb::new(Vec3::new(back.x, y + 0.74, back.y), h), Paint::Dark);
    }

    /// A photocopier / printer: a solid box with a lid, a paper tray and a lit panel.
    pub(crate) fn printer(&mut self, p: Vec2, y: f32, big: bool) {
        let (half, height) = if big { (Vec2::new(0.5, 0.38), 1.05) } else { (Vec2::new(0.32, 0.28), 0.95) };
        self.solid(Aabb::span(Vec3::new(p.x - half.x, y, p.y - half.y), Vec3::new(p.x + half.x, y + height, p.y + half.y)), Paint::Bone);
        self.decor(Aabb::span(Vec3::new(p.x - half.x + 0.04, y + height, p.y - half.y + 0.04), Vec3::new(p.x + half.x - 0.04, y + height + 0.04, p.y + half.y - 0.04)), Paint::Dark);
        self.decor(Aabb::span(Vec3::new(p.x - half.x * 0.6, y + height * 0.55, p.y - half.y - 0.02), Vec3::new(p.x + half.x * 0.6, y + height * 0.6, p.y - half.y)), Paint::Steel);
        self.decor(Aabb::span(Vec3::new(p.x + half.x * 0.3, y + height + 0.04, p.y - half.y + 0.05), Vec3::new(p.x + half.x * 0.8, y + height + 0.06, p.y - half.y + 0.15)), Paint::Green);
        self.decor(Aabb::span(Vec3::new(p.x - half.x * 0.5, y + height + 0.04, p.y - 0.1), Vec3::new(p.x + half.x * 0.2, y + height + 0.07, p.y + 0.12)), Paint::White);
        // Drawers: thin lines on the front.
        for k in 1..3 {
            let yy = y + height * 0.18 * k as f32;
            self.decor(Aabb::span(Vec3::new(p.x - half.x + 0.05, yy, p.y - half.y - 0.015), Vec3::new(p.x + half.x - 0.05, yy + 0.02, p.y - half.y)), Paint::Steel);
        }
    }

    /// A run of filing cabinets along X (`n` wide), with drawer lines.
    pub(crate) fn filing_cabinets(&mut self, p: Vec2, n: usize, y: f32) {
        let w = 0.25 * n as f32;
        self.solid(Aabb::span(Vec3::new(p.x - w, y, p.y - 0.3), Vec3::new(p.x + w, y + 1.2, p.y + 0.3)), Paint::Steel);
        for k in 0..4 {
            let yy = y + 0.27 + k as f32 * 0.27;
            self.decor(Aabb::span(Vec3::new(p.x - w + 0.03, yy, p.y - 0.31), Vec3::new(p.x + w - 0.03, yy + 0.02, p.y + 0.31)), Paint::Dark);
        }
    }

    /// A meeting table along X with chairs down both sides.
    pub(crate) fn meeting_table(&mut self, c: Vec2, length: f32, y: f32) {
        self.solid(Aabb::span(Vec3::new(c.x - length * 0.5, y, c.y - 0.6), Vec3::new(c.x + length * 0.5, y + 0.76, c.y + 0.6)), Paint::Wood);
        self.decor(Aabb::span(Vec3::new(c.x - length * 0.5 + 0.1, y + 0.76, c.y - 0.5), Vec3::new(c.x + length * 0.5 - 0.1, y + 0.78, c.y + 0.5)), Paint::Dark);
        let n = ((length - 0.6) / 1.1).floor() as usize;
        for i in 0..n {
            let x = c.x - length * 0.5 + 0.85 + i as f32 * 1.1;
            self.office_chair(Vec2::new(x, c.y - 1.05), Vec2::NEG_Y, y);
            self.office_chair(Vec2::new(x, c.y + 1.05), Vec2::Y, y);
        }
    }

    /// A potted plant: a solid pot with a bush in it.
    pub(crate) fn potted_plant(&mut self, rng: &mut Rng, p: Vec2, y: f32) {
        self.solid(Aabb::span(Vec3::new(p.x - 0.3, y, p.y - 0.3), Vec3::new(p.x + 0.3, y + 0.55, p.y + 0.3)), Paint::Rust);
        self.props.push(Prop { kind: PropKind::Bush, pos: Vec3::new(p.x, y + 0.5, p.y), yaw: rng.range(0.0, std::f32::consts::TAU), scale: rng.range(0.6, 0.8) });
    }

    /// A vending machine or water cooler against a wall.
    pub(crate) fn vending(&mut self, p: Vec2, y: f32, glow: Paint) {
        self.solid(Aabb::span(Vec3::new(p.x - 0.45, y, p.y - 0.4), Vec3::new(p.x + 0.45, y + 1.9, p.y + 0.4)), Paint::Coral);
        self.decor(Aabb::span(Vec3::new(p.x - 0.35, y + 0.7, p.y - 0.42), Vec3::new(p.x + 0.35, y + 1.75, p.y - 0.4)), glow);
    }

    /// A floor slab over (x0, x1, z0, z1) with its top at `top`, holes cut out, and a floor
    /// finish on it inside `inset` of the edges. It is 0.2 m thick: three metres of headroom,
    /// so even a robot vaulted onto a crouching one keeps its head below the ceiling.
    pub(crate) fn slab(&mut self, (x0, x1, z0, z1): (f32, f32, f32, f32), top: f32, holes: &[(f32, f32, f32, f32)], finish: Option<(Paint, f32)>) {
        for (a0, a1, b0, b1) in rects_minus(x0, x1, z0, z1, holes) {
            if a1 - a0 < 0.02 || b1 - b0 < 0.02 {
                continue;
            }
            self.solid(Aabb::span(Vec3::new(a0, top - 0.2, b0), Vec3::new(a1, top, b1)), Paint::Concrete);
            if let Some((paint, inset)) = finish {
                let (c0, c1, d0, d1) = (a0.max(x0 + inset), a1.min(x1 - inset), b0.max(z0 + inset), b1.min(z1 - inset));
                if c1 > c0 && d1 > d0 {
                    self.decor(Aabb::span(Vec3::new(c0, top, d0), Vec3::new(c1, top + 0.012, d1)), paint);
                }
            }
        }
    }

    /// A stairwell of switchback flights (eleven steps a storey) between floors `g + k * STOREY`,
    /// against a wall that runs along X. `x0` is its west end (7 m long: a 1.2 m landing, the
    /// 4.62 m flight, a 1.18 m landing); `wall_z` is the wall's inner face and `inward` points
    /// away from it. Even flights climb east in the inner lane, odd flights climb west in the
    /// lane against the wall, so every floor has a landing beside the hole the flight below
    /// comes up through. Returns the hole each floor above needs (floor k + 1 gets entry k).
    pub(crate) fn stairwell(&mut self, x0: f32, wall_z: f32, inward: f32, g: f32, flights: u32) -> Vec<(f32, f32, f32, f32)> {
        const STEPS: usize = 11;
        const RUN: f32 = 0.42;
        let rise = super::STOREY / STEPS as f32;
        let (sx0, sx1) = (x0 + 1.2, x0 + 1.2 + STEPS as f32 * RUN);
        let lane = |k: u32| -> (f32, f32) {
            let (a, b) = if k.is_multiple_of(2) { (1.4, 2.6) } else { (0.0, 1.2) };
            let (p, q) = (wall_z + inward * a, wall_z + inward * b);
            (p.min(q), p.max(q))
        };
        let mut holes = Vec::new();
        for k in 0..flights {
            let base = g + k as f32 * super::STOREY;
            let (lz0, lz1) = lane(k);
            for i in 0..STEPS {
                let top = base + (i + 1) as f32 * rise;
                let (a, b) = if k % 2 == 0 { (sx0 + i as f32 * RUN, sx0 + (i + 1) as f32 * RUN) } else { (sx1 - (i + 1) as f32 * RUN, sx1 - i as f32 * RUN) };
                self.solid(Aabb::span(Vec3::new(a, base, lz0), Vec3::new(b, top, lz1)), if i % 2 == 0 { Paint::Concrete } else { Paint::Steel });
            }
            // A wall between the two lanes, floor to ceiling the length of the flights, so nobody
            // steps off one flight into the hole beside it; the landings at the ends stay open.
            let mid = wall_z + inward * 1.3;
            self.solid(Aabb::span(Vec3::new(sx0, base, mid - 0.08), Vec3::new(sx1, base + super::STOREY - 0.2, mid + 0.08)), Paint::Bone);
            // The hole in the floor above: the flight's length, one step short at the top.
            let (h0, h1) = if k % 2 == 0 { (sx0, sx1 - RUN * 0.5) } else { (sx0 + RUN * 0.5, sx1) };
            holes.push((h0, h1, lz0 - 0.05, lz1 + 0.05));
            // Rails round the hole on the floor above: across the low end, along the inner side,
            // and between the lanes short of the top so you can step off sideways.
            let top = base + super::STOREY;
            let (low, high) = if k % 2 == 0 { (h0, h1) } else { (h1, h0) };
            let toward_high = (high - low).signum();
            let between = if k % 2 == 0 { if inward > 0.0 { lz0 - 0.1 } else { lz1 + 0.1 } } else if inward > 0.0 { lz1 + 0.1 } else { lz0 - 0.1 };
            let (r0, r1) = (low.min(high - toward_high * 1.0), low.max(high - toward_high * 1.0));
            self.solid(Aabb::span(Vec3::new(r0, top, between - 0.04), Vec3::new(r1, top + 0.95, between + 0.04)), Paint::Dark);
            self.solid(Aabb::span(Vec3::new(low - toward_high * 0.1 - 0.04, top, lz0 - 0.05), Vec3::new(low - toward_high * 0.1 + 0.04, top + 0.95, lz1 + 0.05)), Paint::Dark);
            if k % 2 == 0 {
                let inner = if inward > 0.0 { lz1 + 0.1 } else { lz0 - 0.1 };
                self.solid(Aabb::span(Vec3::new(h0, top, inner - 0.04), Vec3::new(h1, top + 0.95, inner + 0.04)), Paint::Dark);
            }
        }
        holes
    }

    /// Waypoints on a floor at height `y` over an area, every `step` metres, wherever a robot
    /// could stand: on something (or the ground) and clear of anything solid.
    pub(crate) fn waypoints_over(&mut self, y: f32, (x0, x1, z0, z1): (f32, f32, f32, f32), step: f32) {
        let mut x = x0 + step * 0.5;
        while x < x1 {
            let mut z = z0 + step * 0.5;
            while z < z1 {
                let body = Aabb::span(Vec3::new(x - 0.5, y + 0.1, z - 0.5), Vec3::new(x + 0.5, y + 1.8, z + 0.5));
                let overlaps = |b: &Aabb| (b.center - body.center).abs().cmple(b.half + body.half).all();
                let floor = (self.height(x, z) - y).abs() < 0.3 || self.blocks.iter().any(|b| b.solid && b.aabb.touches(x, z, 0.0) && (b.aabb.top() - y).abs() < 0.15);
                if floor && !self.solids().any(|b| overlaps(&b)) {
                    self.waypoints.push(Vec3::new(x, y, z));
                }
                z += step;
            }
            x += step;
        }
    }

    // --- Bases -------------------------------------------------------------------------------

    /// Team bases at `x = ±x` round `z`: six spawn points each, and cover in front of them.
    pub(crate) fn team_bases(&mut self, x: f32, z: f32, cover: Paint) {
        for team in 0..2 {
            let s = if team == 0 { -1.0 } else { 1.0 };
            let bx = s * x;
            self.team_spawns[team] = (0..6).map(|i| Vec2::new(bx - s * (i % 2) as f32 * 2.5, z - 12.5 + i as f32 * 5.0)).collect();
            let front = bx - s * 8.0;
            for dz in [-9.0, 0.0, 9.0] {
                let p = Vec2::new(front, z + dz);
                if self.occupied(p, 1.8) {
                    continue;
                }
                let y = self.lowest(p, Vec2::new(0.4, 1.6));
                let top = self.highest(p, Vec2::new(0.4, 1.6)) + COVER_HEIGHT;
                self.solid(Aabb::span(Vec3::new(front - 0.4, y - 0.2, p.y - 1.6), Vec3::new(front + 0.4, top, p.y + 1.6)), cover);
            }
        }
        self.spawn = self.team_spawns[0][2];
    }

    /// Is this within a team base's clear zone?
    pub(crate) fn near_base(&self, p: Vec2, base_x: f32, base_z: f32) -> bool {
        (p.x.abs() - base_x).abs() < 11.0 && (p.y - base_z).abs() < 19.0
    }
}

/// A rectangle (x0, x1, z0, z1) minus some holes, as a few rectangles that cover the rest.
pub(crate) fn rects_minus(x0: f32, x1: f32, z0: f32, z1: f32, holes: &[(f32, f32, f32, f32)]) -> Vec<(f32, f32, f32, f32)> {
    let mut xs = vec![x0, x1];
    for h in holes {
        xs.push(h.0.clamp(x0, x1));
        xs.push(h.1.clamp(x0, x1));
    }
    xs.sort_by(f32::total_cmp);
    xs.dedup_by(|a, b| (*a - *b).abs() < 0.01);
    let mut out = Vec::new();
    // Rects still growing along x: (x start, z0, z1).
    let mut open: Vec<(f32, f32, f32)> = Vec::new();
    for pair in xs.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let mid = (a + b) * 0.5;
        let mut free = vec![(z0, z1)];
        for h in holes.iter().filter(|h| h.0 < mid && mid < h.1) {
            free = free
                .into_iter()
                .flat_map(|(c, d)| {
                    let mut parts = Vec::new();
                    if h.2 > c + 0.01 {
                        parts.push((c, h.2.min(d)));
                    }
                    if h.3 < d - 0.01 {
                        parts.push((h.3.max(c), d));
                    }
                    if h.3 <= c || h.2 >= d {
                        return vec![(c, d)];
                    }
                    parts
                })
                .collect();
        }
        let mut next = Vec::new();
        for (c, d) in free {
            if let Some(i) = open.iter().position(|o| (o.1 - c).abs() < 0.01 && (o.2 - d).abs() < 0.01) {
                next.push(open.remove(i));
            } else {
                next.push((a, c, d));
            }
        }
        for (start, c, d) in open.drain(..) {
            out.push((start, a, c, d));
        }
        open = next;
    }
    for (start, c, d) in open {
        out.push((start, x1, c, d));
    }
    out
}

/// A darker shade of a paint, for alternating courses.
pub(crate) fn darker(paint: Paint) -> Paint {
    match paint {
        Paint::Bone => Paint::Sand,
        Paint::Sand => Paint::Concrete,
        Paint::Concrete => Paint::Rock,
        Paint::Rust => Paint::Wood,
        Paint::Coral => Paint::Rust,
        Paint::Teal => Paint::Shell,
        Paint::Steel => Paint::Dark,
        Paint::Metal => Paint::Steel,
        Paint::Iron => Paint::Steel,
        Paint::Wood => Paint::Dark,
        Paint::LeafLight => Paint::Leaf,
        Paint::Leaf => Paint::Shell,
        Paint::Gold => Paint::Copper,
        Paint::Brick => Paint::Wood,
        Paint::Cream => Paint::Bone,
        Paint::Planks => Paint::Wood,
        Paint::Tile => Paint::Metal,
        Paint::Lino => Paint::Shell,
        Paint::Slate => Paint::Dark,
        Paint::Navy => Paint::Dark,
        Paint::Mustard => Paint::Gold,
        other => other,
    }
}
