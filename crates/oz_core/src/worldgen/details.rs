//! Detailing: the render-only dressing every map is finished with, and round things.
//!
//! [`World::dressing`] holds small primitives in world space - trims, frames, gutters, wires,
//! cushions, books, flowers - drawn with the blocks but never collided with: the rule from
//! `shapes` holds, anything you could hide behind is a solid block, anything thinner is
//! decoration. Round things (silos, tanks, columns, trunks) keep box collision (hidden blocks,
//! see [`crate::worldgen::Block::hidden`]) and are drawn as smooth many-sided solids in
//! [`World::roofs`].
//!
//! The helpers here never draw from the map's main random stream: detail is rolled from its
//! own position (see [`World::detail_rng`]), so dressing a map never moves its streets.

use super::{Block, World};
use crate::{
    collide::Aabb,
    model::{Anim, Hull, Paint, Part, Shape},
    rng::Rng,
};
use glam::{Quat, Vec2, Vec3};
use std::f32::consts::TAU;

/// Points round a horizontal circle.
pub(crate) fn ring_points(c: Vec2, r: f32, y: f32, sides: usize) -> Vec<Vec3> {
    (0..sides).map(|i| (i as f32 + 0.5) * TAU / sides as f32).map(|a| Vec3::new(c.x + a.cos() * r, y, c.y + a.sin() * r)).collect()
}

impl World {
    /// A random stream for detail at `p`, independent of the map's main one.
    pub(crate) fn detail_rng(&self, p: Vec2, salt: u64) -> Rng {
        let key = ((p.x * 8.0).round() as i64 as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ ((p.y * 8.0).round() as i64 as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f);
        Rng::new(self.recipe.seed ^ key ^ salt.wrapping_mul(0x1656_67b1_9e37_79f9))
    }

    /// A dressing primitive at `pos`, `size` across, turned by `rot`.
    pub(crate) fn trim_part(&mut self, shape: Shape, paint: Paint, pos: Vec3, size: Vec3, rot: Quat) {
        self.dressing.push(Part { shape, paint, pos, size, rot, anim: Anim::None });
    }

    /// A dressing box between two corners.
    pub(crate) fn trim(&mut self, lo: Vec3, hi: Vec3, paint: Paint) {
        let (a, b) = (lo.min(hi), lo.max(hi));
        self.trim_part(Shape::Cube, paint, (a + b) * 0.5, b - a, Quat::IDENTITY);
    }

    /// A dressing box centred at `c` with half extents `half`.
    pub(crate) fn trim_at(&mut self, c: Vec3, half: Vec3, paint: Paint) {
        self.trim_part(Shape::Cube, paint, c, half * 2.0, Quat::IDENTITY);
    }

    /// A square bar from `a` to `b`.
    pub(crate) fn trim_bar(&mut self, a: Vec3, b: Vec3, thick: f32, paint: Paint) {
        let span = b - a;
        let length = span.length().max(1e-4);
        self.trim_part(Shape::Cube, paint, a + span * 0.5, Vec3::new(thick, length, thick), Quat::from_rotation_arc(Vec3::Y, span / length));
    }

    /// A round rod (pipe, pole, rail, wire) from `a` to `b`.
    pub(crate) fn trim_rod(&mut self, a: Vec3, b: Vec3, radius: f32, paint: Paint) {
        let span = b - a;
        let length = span.length().max(1e-4);
        // Thin ones (wires, chains, rails) are square: at that size nobody can tell, and a box is a quarter the triangles.
        let shape = if radius < 0.05 { Shape::Cube } else { Shape::Cylinder };
        self.trim_part(shape, paint, a + span * 0.5, Vec3::new(radius * 2.0, length, radius * 2.0), Quat::from_rotation_arc(Vec3::Y, span / length));
    }

    /// A sagging wire between two points, in a few straight runs.
    pub(crate) fn wire(&mut self, a: Vec3, b: Vec3, sag: f32) {
        let n = 4;
        let at = |t: f32| a.lerp(b, t) - Vec3::Y * sag * 4.0 * t * (1.0 - t);
        for i in 0..n {
            self.trim_rod(at(i as f32 / n as f32), at((i + 1) as f32 / n as f32), 0.018, Paint::Dark);
        }
    }

    /// A smooth upright cylinder from `y0` to `y1`, drawn only (see [`World::roofs`]).
    pub(crate) fn round(&mut self, c: Vec2, r: f32, y0: f32, y1: f32, paint: Paint, sides: usize) {
        let mut points = ring_points(c, r, y0, sides);
        points.extend(ring_points(c, r, y1, sides));
        self.roofs.push(Hull { paint, points, anim: Anim::None });
    }

    /// A smooth solid of revolution through rings of (radius, height); a radius of zero is a point.
    pub(crate) fn lathe(&mut self, c: Vec2, rings: &[(f32, f32)], paint: Paint, sides: usize) {
        let mut points = Vec::new();
        for &(r, y) in rings {
            if r <= 1e-3 {
                points.push(Vec3::new(c.x, y, c.y));
            } else {
                points.extend(ring_points(c, r, y, sides));
            }
        }
        self.roofs.push(Hull { paint, points, anim: Anim::None });
    }

    /// The solid core of a round thing: boxes that fill an octagon of radius `r` (collision,
    /// cover), hidden - the round thing is drawn over them.
    pub(crate) fn round_core(&mut self, c: Vec2, r: f32, y0: f32, y1: f32, paint: Paint) {
        let (ym, hy) = ((y0 + y1) * 0.5, (y1 - y0) * 0.5);
        for (a, b) in [(1.0, 0.42), (0.42, 1.0), (0.86, 0.66), (0.66, 0.86)] {
            self.blocks.push(Block { aabb: Aabb::new(Vec3::new(c.x, ym, c.y), Vec3::new(r * a, hy, r * b)), paint, solid: true, hidden: true });
        }
    }

    /// Thin hoops round a cylinder, every `every` metres between `y0` and `y1`.
    pub(crate) fn hoops(&mut self, c: Vec2, r: f32, y0: f32, y1: f32, every: f32, paint: Paint) {
        let mut y = y0;
        while y < y1 {
            self.round(c, r, y - 0.05, y + 0.05, paint, 20);
            y += every;
        }
    }
}

impl World {
    /// Draws the solid blocks from index `from` on as weathered rock instead of boxes: each one
    /// hidden under a faceted skin that wraps it (never inside it, so what you see is never
    /// smaller than what stops you), with bulging faces and rounded shoulders.
    pub(crate) fn rock_skin(&mut self, from: usize, paint: Paint) {
        for i in from..self.blocks.len() {
            let b = self.blocks[i];
            if !b.solid || b.hidden {
                continue;
            }
            self.blocks[i].hidden = true;
            let (lo, hi) = (b.aabb.min(), b.aabb.max());
            let mut rng = self.detail_rng(Vec2::new(b.aabb.center.x, b.aabb.center.z), 0x40c + i as u64);
            let bulge = (b.aabb.half.min_element() * 0.35).clamp(0.04, 0.3);
            let mut points = Vec::with_capacity(40);
            let size = hi - lo;
            // Corners, pushed out; the top ones a little lower and further out (a weathered lip).
            for &x in &[lo.x, hi.x] {
                for &z in &[lo.z, hi.z] {
                    let out = Vec3::new((x - b.aabb.center.x).signum(), 0.0, (z - b.aabb.center.z).signum());
                    points.push(Vec3::new(x, lo.y, z) + out * rng.range(0.0, bulge));
                    points.push(Vec3::new(x, hi.y, z) + out * rng.range(0.0, bulge * 0.4) + Vec3::Y * rng.range(0.0, 0.05));
                }
            }
            // Bulges across each side, at a few heights.
            let rows = ((size.y / 1.2).ceil() as usize).clamp(1, 4);
            for r in 0..rows {
                let y = lo.y + size.y * (r as f32 + rng.range(0.3, 0.7)) / rows as f32;
                for (u, side) in [(rng.range(0.2, 0.8), 0), (rng.range(0.2, 0.8), 1), (rng.range(0.2, 0.8), 2), (rng.range(0.2, 0.8), 3)] {
                    let p = match side {
                        0 => Vec3::new(lo.x + size.x * u, y, lo.z - rng.range(0.02, bulge)),
                        1 => Vec3::new(hi.x + rng.range(0.02, bulge), y, lo.z + size.z * u),
                        2 => Vec3::new(lo.x + size.x * u, y, hi.z + rng.range(0.02, bulge)),
                        _ => Vec3::new(lo.x - rng.range(0.02, bulge), y, lo.z + size.z * u),
                    };
                    points.push(p);
                }
            }
            // A crown: a few points just over the top.
            for _ in 0..3 {
                points.push(Vec3::new(rng.range(lo.x + size.x * 0.2, hi.x - size.x * 0.2), hi.y + rng.range(0.02, 0.12), rng.range(lo.z + size.z * 0.2, hi.z - size.z * 0.2)));
            }
            let paint = if rng.chance(0.3) { crate::worldgen::shapes::darker(paint) } else { paint };
            self.roofs.push(Hull { paint, points, anim: Anim::None });
        }
    }

    /// Loose stones and pebbles strewn round `c` (dressing: too small to hide behind).
    pub(crate) fn scree(&mut self, c: Vec2, radius: f32, count: usize, paint: Paint) {
        let mut rng = self.detail_rng(c, 0x5c3);
        for _ in 0..count {
            let p = c + Vec2::from_angle(rng.range(0.0, TAU)) * rng.range(0.0, radius);
            let s = rng.range(0.12, 0.45);
            let g = self.height(p.x, p.y);
            self.trim_part(Shape::Shard, paint, Vec3::new(p.x, g + s * 0.2, p.y), Vec3::new(s, s * 0.6, s * rng.range(0.7, 1.1)), Quat::from_rotation_y(rng.range(0.0, TAU)));
        }
    }

    /// A clump of wild flowers or a fern at `p` (dressing).
    pub(crate) fn flowers(&mut self, p: Vec2, rng: &mut Rng) {
        let g = self.height(p.x, p.y);
        let colour = *rng.pick(&[Paint::Coral, Paint::Mustard, Paint::Cream, Paint::Purple, Paint::Red, Paint::LeafLight]).unwrap_or(&Paint::Coral);
        let colour = if colour.unlit() { Paint::Coral } else { colour };
        for _ in 0..rng.below(4) + 3 {
            let q = p + Vec2::new(rng.range(-0.35, 0.35), rng.range(-0.35, 0.35));
            let h = rng.range(0.18, 0.45);
            self.trim_rod(Vec3::new(q.x, g, q.y), Vec3::new(q.x, g + h, q.y), 0.012, Paint::Leaf);
            self.trim_part(Shape::Shard, colour, Vec3::new(q.x, g + h, q.y), Vec3::splat(rng.range(0.07, 0.12)), Quat::IDENTITY);
        }
        self.trim_part(Shape::Shard, Paint::Leaf, Vec3::new(p.x, g + 0.05, p.y), Vec3::new(0.5, 0.12, 0.45), Quat::from_rotation_y(rng.range(0.0, TAU)));
    }
}
