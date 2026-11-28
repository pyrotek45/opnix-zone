//! Roads, pavements and painted ground, drawn as surfaces laid on the terrain facet by
//! facet (render only: the ground under them is what you walk on). Roads are laid when the
//! map is finished, so each one knows where the others cross it: kerbs, edge lines and
//! centre dashes stop at junctions instead of running across them.

use super::World;
use crate::{model::Paint, rng::Rng};
use glam::{Vec2, Vec3};

/// A road as [`World::road`] asked for it, kept until the map is paved.
#[derive(Clone, Copy, Debug)]
pub struct Lane {
    pub a: Vec2,
    pub b: Vec2,
    pub width: f32,
    pub paint: Paint,
    pub lines: bool,
}

impl Lane {
    pub fn along_x(&self) -> bool {
        (self.b.x - self.a.x).abs() > (self.b.y - self.a.y).abs()
    }

    /// The paved rectangle, (low corner, high corner).
    pub fn rect(&self) -> (Vec2, Vec2) {
        let w = if self.along_x() { Vec2::new(0.0, self.width * 0.5) } else { Vec2::new(self.width * 0.5, 0.0) };
        (self.a.min(self.b) - w, self.a.max(self.b) + w)
    }

    /// A pavement: a narrow concrete strip, drawn raised a kerb's height.
    pub fn pavement(&self) -> bool {
        self.paint == Paint::Concrete && self.width <= 2.5 && !self.lines
    }

    /// Wide enough to drive and park on.
    pub fn carriageway(&self) -> bool {
        !self.pavement() && self.width >= 4.0
    }

    /// Does a disc at `p` of radius `r` reach more than `slack` onto it?
    pub fn covers(&self, p: Vec2, r: f32, slack: f32) -> bool {
        let (lo, hi) = self.rect();
        let m = r - slack;
        p.x > lo.x - m && p.x < hi.x + m && p.y > lo.y - m && p.y < hi.y + m
    }
}

/// How high above the ground each layer lies, so no two ever share a plane.
const GROUND_PAINT: f32 = 0.012;
const ROAD: f32 = 0.024;
const PATCH: f32 = 0.036;
const MARKING: f32 = 0.044;
const KERB: f32 = 0.13;

/// Keeps the part of a convex polygon where `f` is not positive.
fn clip(poly: &[Vec2], f: impl Fn(Vec2) -> f32) -> Vec<Vec2> {
    let mut out = Vec::with_capacity(poly.len() + 1);
    for i in 0..poly.len() {
        let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
        let (fp, fq) = (f(p), f(q));
        if fp <= 0.0 {
            out.push(p);
        }
        if (fp < 0.0 && fq > 0.0) || (fp > 0.0 && fq < 0.0) {
            out.push(p + (q - p) * (fp / (fp - fq)));
        }
    }
    out
}

/// `span` with the `cuts` taken out (all intervals along one axis).
fn subtract(span: (f32, f32), cuts: &[(f32, f32)]) -> Vec<(f32, f32)> {
    let mut open = vec![span];
    for &(c0, c1) in cuts {
        open = open
            .into_iter()
            .flat_map(|(a, b)| {
                if c1 <= a || c0 >= b {
                    vec![(a, b)]
                } else {
                    [(a, c0), (c1, b)].into_iter().filter(|(x, y)| y - x > 0.05).collect()
                }
            })
            .collect();
    }
    open
}

impl World {
    /// One triangle of paving, wound to face `out`.
    fn paving_triangle(&mut self, paint: Paint, mut t: [Vec3; 3], out: Vec3) {
        if (t[1] - t[0]).cross(t[2] - t[0]).dot(out) < 0.0 {
            t.swap(1, 2);
        }
        self.paving.push((paint, t));
    }

    /// Lays `paint` over the ground in a rectangle, `lift` above it, following every facet of
    /// the terrain exactly (each terrain cell is two flat triangles).
    pub(crate) fn drape(&mut self, lo: Vec2, hi: Vec2, lift: f32, paint: Paint) {
        let (cell, half) = (self.recipe.cell, self.recipe.half_extent);
        let grid = |v: f32| (v + half) / cell;
        let (i0, i1) = (grid(lo.x).floor() as i32, grid(hi.x).ceil() as i32);
        let (j0, j1) = (grid(lo.y).floor() as i32, grid(hi.y).ceil() as i32);
        for j in j0..j1 {
            for i in i0..i1 {
                let (x0, x1) = ((i as f32 * cell - half).max(lo.x), ((i + 1) as f32 * cell - half).min(hi.x));
                let (z0, z1) = ((j as f32 * cell - half).max(lo.y), ((j + 1) as f32 * cell - half).min(hi.y));
                if x1 - x0 < 1e-4 || z1 - z0 < 1e-4 {
                    continue;
                }
                let rect = [Vec2::new(x0, z0), Vec2::new(x1, z0), Vec2::new(x1, z1), Vec2::new(x0, z1)];
                // The cell's two triangles meet on the diagonal gx + gz = i + j + 1.
                let diagonal = (i + j + 1) as f32;
                for keep in [1.0, -1.0] {
                    let poly = clip(&rect, |p| keep * (grid(p.x) + grid(p.y) - diagonal));
                    if poly.len() < 3 {
                        continue;
                    }
                    let at = |p: Vec2| Vec3::new(p.x, self.height(p.x, p.y) + lift, p.y);
                    let points: Vec<Vec3> = poly.iter().map(|p| at(*p)).collect();
                    for k in 1..points.len() - 1 {
                        self.paving_triangle(paint, [points[0], points[k], points[k + 1]], Vec3::Y);
                    }
                }
            }
        }
    }

    /// The upright face of a kerb along `p`..`q`, from `low` to `high` over the ground, facing `out`.
    fn skirt(&mut self, p: Vec2, q: Vec2, (low, high): (f32, f32), out: Vec3, paint: Paint) {
        let n = ((q - p).length() / (self.recipe.cell * 0.5)).ceil().max(1.0) as usize;
        for k in 0..n {
            let (a, b) = (p.lerp(q, k as f32 / n as f32), p.lerp(q, (k + 1) as f32 / n as f32));
            let (ga, gb) = (self.height(a.x, a.y), self.height(b.x, b.y));
            let (a0, a1) = (Vec3::new(a.x, ga + low, a.y), Vec3::new(a.x, ga + high, a.y));
            let (b0, b1) = (Vec3::new(b.x, gb + low, b.y), Vec3::new(b.x, gb + high, b.y));
            self.paving_triangle(paint, [a0, b0, b1], out);
            self.paving_triangle(paint, [a0, b1, a1], out);
        }
    }

    /// A raised strip (a kerb or pavement) over `lo`..`hi`: its top `KERB` over the ground,
    /// upright faces down to `base` on every side.
    fn raised(&mut self, lo: Vec2, hi: Vec2, base: f32, paint: Paint) {
        self.drape(lo, hi, KERB, paint);
        let (a, b, c, d) = (lo, Vec2::new(hi.x, lo.y), hi, Vec2::new(lo.x, hi.y));
        self.skirt(a, b, (base, KERB), Vec3::NEG_Z, paint);
        self.skirt(b, c, (base, KERB), Vec3::X, paint);
        self.skirt(c, d, (base, KERB), Vec3::Z, paint);
        self.skirt(d, a, (base, KERB), Vec3::NEG_X, paint);
    }

    /// Paves every road laid: asphalt and dirt on the ground, raised pavements, and on
    /// marked roads kerbs, edge lines, centre dashes, patches and manhole covers.
    pub(crate) fn pave(&mut self) {
        let lanes = self.lanes.clone();
        let mut rng = Rng::new(self.recipe.seed ^ 0x9a7e);
        for (k, lane) in lanes.iter().enumerate() {
            let along_x = lane.along_x();
            let (lo, hi) = lane.rect();
            // (along, across) <-> world.
            let to = |t: f32, s: f32| if along_x { Vec2::new(t, s) } else { Vec2::new(s, t) };
            let (t0, t1) = if along_x { (lo.x, hi.x) } else { (lo.y, hi.y) };
            let mid = if along_x { (lo.y + hi.y) * 0.5 } else { (lo.x + hi.x) * 0.5 };
            // Where other roads cross this one, along it.
            let cuts: Vec<(f32, f32)> = lanes
                .iter()
                .enumerate()
                .filter(|(m, other)| *m != k && !other.pavement())
                .filter_map(|(_, other)| {
                    let (olo, ohi) = other.rect();
                    if olo.x >= hi.x || ohi.x <= lo.x || olo.y >= hi.y || ohi.y <= lo.y {
                        return None;
                    }
                    Some(if along_x { (olo.x - 0.2, ohi.x + 0.2) } else { (olo.y - 0.2, ohi.y + 0.2) })
                })
                .collect();
            let open = subtract((t0, t1), &cuts);
            let strip = |t: (f32, f32), s: (f32, f32)| {
                let (a, b) = (to(t.0, s.0), to(t.1, s.1));
                (a.min(b), a.max(b))
            };
            if lane.pavement() {
                for &span in &open {
                    let (a, b) = strip(span, (mid - lane.width * 0.5, mid + lane.width * 0.5));
                    self.raised(a, b, 0.0, lane.paint);
                }
                continue;
            }
            // Each road a hair above the last, so where two overlap one is cleanly on top.
            self.drape(lo, hi, ROAD + 0.0015 * (k % 6) as f32, lane.paint);
            if !lane.lines {
                continue;
            }
            let half = lane.width * 0.5;
            for &span in &open {
                // Kerbs along both edges, edge lines just inside them.
                for side in [-1.0, 1.0] {
                    let (a, b) = strip(span, (mid + side * half, mid + side * (half - 0.25)));
                    self.raised(a, b, 0.0, Paint::Concrete);
                    let (a, b) = strip((span.0 + 0.3, span.1 - 0.3), (mid + side * (half - 0.45), mid + side * (half - 0.57)));
                    if b.x > a.x && b.y > a.y {
                        self.drape(a, b, MARKING, Paint::Bone);
                    }
                }
                // A dashed centre line: 2.4 m dashes every 6 m, whole dashes only.
                let mut t = span.0 + 1.5;
                while t + 2.4 < span.1 - 1.5 {
                    let (a, b) = strip((t, t + 2.4), (mid - 0.07, mid + 0.07));
                    self.drape(a, b, MARKING, Paint::Bone);
                    t += 6.0;
                }
                // Patched asphalt and the odd manhole cover.
                let mut t = span.0 + rng.range(3.0, 12.0);
                while t + 3.0 < span.1 {
                    let s = mid + rng.range(-half + 1.0, half - 1.0);
                    if rng.chance(0.5) {
                        let (l, w) = (rng.range(1.0, 2.6), rng.range(0.6, 1.4));
                        let (a, b) = strip((t, t + l), (s - w * 0.5, s + w * 0.5));
                        self.drape(a, b, PATCH, Paint::Steel);
                    } else {
                        let s = mid + if rng.chance(0.5) { -1.6 } else { 1.6 };
                        let (a, b) = strip((t, t + 0.7), (s - 0.35, s + 0.35));
                        self.drape(a, b, MARKING, Paint::Iron);
                    }
                    t += rng.range(9.0, 20.0);
                }
            }
        }
    }

    /// Paint over an area of ground (fields, yards, car parks), lying on it facet by facet.
    pub(crate) fn ground_paint(&mut self, x0: f32, x1: f32, z0: f32, z1: f32, paint: Paint) {
        // Each area a hair above the last, so where two overlap one is cleanly on top.
        let lift = GROUND_PAINT + 0.0015 * (self.painted % 6) as f32;
        self.painted += 1;
        self.drape(Vec2::new(x0, z0), Vec2::new(x1, z1), lift, paint);
    }
}
