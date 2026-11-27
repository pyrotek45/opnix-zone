//! Static collision: every wall, floor, stair and prop as an axis-aligned box,
//! bucketed in a coarse grid so movement, line-of-sight and bullets only test
//! the handful of boxes near them.

use glam::{Vec2, Vec3, Vec3Swizzles};

/// How high a walker can step up without jumping (stairs, kerbs, low rubble).
pub const STEP: f32 = 0.45;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub center: Vec3,
    pub half: Vec3,
}

impl Aabb {
    pub fn new(center: Vec3, half: Vec3) -> Self {
        Self { center, half }
    }

    /// From two opposite corners, in any order.
    pub fn span(a: Vec3, b: Vec3) -> Self {
        Self { center: (a + b) * 0.5, half: (a - b).abs() * 0.5 }
    }

    pub fn min(&self) -> Vec3 {
        self.center - self.half
    }

    pub fn max(&self) -> Vec3 {
        self.center + self.half
    }

    pub fn top(&self) -> f32 {
        self.center.y + self.half.y
    }

    pub fn bottom(&self) -> f32 {
        self.center.y - self.half.y
    }

    /// Does the footprint (XZ) come within `radius` of a point?
    pub fn touches(&self, x: f32, z: f32, radius: f32) -> bool {
        (x - self.center.x).abs() < self.half.x + radius && (z - self.center.z).abs() < self.half.z + radius
    }

    /// Ray entry and exit distances, if the ray hits within `max`.
    pub fn ray(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<(f32, f32)> {
        let (mut lo, mut hi) = (0.0f32, max);
        for axis in 0..3 {
            let (min, max) = (self.center[axis] - self.half[axis], self.center[axis] + self.half[axis]);
            if dir[axis].abs() < 1e-6 {
                if origin[axis] < min || origin[axis] > max {
                    return None;
                }
            } else {
                let (a, b) = ((min - origin[axis]) / dir[axis], (max - origin[axis]) / dir[axis]);
                lo = lo.max(a.min(b));
                hi = hi.min(a.max(b));
                if hi < lo {
                    return None;
                }
            }
        }
        Some((lo, hi))
    }
}

/// A smooth walkable slope over a rectangular footprint: a pitched roof, walked as drawn
/// rather than as the stair steps under it that carry its other collision. Its surface is
/// the lowest of its planes, capped at `top`. Boxes it covers (tops from `bottom` up to
/// `top`, inside the footprint) are no longer stood on: their corners would poke through.
#[derive(Clone, Debug, PartialEq)]
pub struct Ramp {
    pub min: Vec2,
    pub max: Vec2,
    pub bottom: f32,
    pub top: f32,
    /// Each plane as (dh/dx, dh/dz, height at x = z = 0).
    pub planes: Vec<Vec3>,
}

impl Ramp {
    /// Surface height at (x, z), taken at the nearest point of the footprint.
    pub fn height(&self, x: f32, z: f32) -> f32 {
        let p = Vec2::new(x, z).clamp(self.min, self.max);
        self.planes.iter().fold(self.top, |h, plane| h.min(plane.x * p.x + plane.y * p.y + plane.z))
    }

    /// Does the footprint come within `radius` of (x, z)?
    pub fn touches(&self, x: f32, z: f32, radius: f32) -> bool {
        x > self.min.x - radius && x < self.max.x + radius && z > self.min.y - radius && z < self.max.y + radius
    }
}

pub struct StaticWorld {
    boxes: Vec<Aabb>,
    /// Under a ramp: not stood on (see [`Ramp`]).
    covered: Vec<bool>,
    ramps: Vec<Ramp>,
    cell: f32,
    half: f32,
    side: usize,
    cells: Vec<Vec<u32>>,
}

impl StaticWorld {
    pub fn new(half_extent: f32) -> Self {
        let cell = 4.0;
        let side = (2.0 * half_extent / cell).ceil() as usize + 1;
        Self { boxes: Vec::new(), covered: Vec::new(), ramps: Vec::new(), cell, half: half_extent, side, cells: vec![Vec::new(); side * side] }
    }

    fn coord(&self, v: f32) -> usize {
        (((v + self.half) / self.cell).floor() as isize).clamp(0, self.side as isize - 1) as usize
    }

    pub fn len(&self) -> usize {
        self.boxes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.boxes.is_empty()
    }

    pub fn boxes(&self) -> &[Aabb] {
        &self.boxes
    }

    pub fn add(&mut self, aabb: Aabb) {
        let index = self.boxes.len() as u32;
        let (min, max) = (aabb.min(), aabb.max());
        for z in self.coord(min.z)..=self.coord(max.z) {
            for x in self.coord(min.x)..=self.coord(max.x) {
                self.cells[z * self.side + x].push(index);
            }
        }
        self.boxes.push(aabb);
        self.covered.push(false);
    }

    /// Adds a smooth slope to stand on (see [`Ramp`]), after the boxes it covers. A map
    /// has a few dozen at most, so they are not bucketed.
    pub fn add_ramp(&mut self, ramp: Ramp) {
        for (b, covered) in self.boxes.iter().zip(&mut self.covered) {
            let (lo, hi) = (b.min(), b.max());
            let inside = lo.x >= ramp.min.x - 0.01 && hi.x <= ramp.max.x + 0.01 && lo.z >= ramp.min.y - 0.01 && hi.z <= ramp.max.y + 0.01;
            *covered |= inside && b.top() >= ramp.bottom - 0.01 && b.top() <= ramp.top + 0.01;
        }
        self.ramps.push(ramp);
    }

    /// Visits boxes whose cells overlap the circle (x, z, radius). A box may be visited more than once.
    pub fn near(&self, x: f32, z: f32, radius: f32, mut visit: impl FnMut(usize, &Aabb)) {
        for cz in self.coord(z - radius)..=self.coord(z + radius) {
            for cx in self.coord(x - radius)..=self.coord(x + radius) {
                for &i in &self.cells[cz * self.side + cx] {
                    visit(i as usize, &self.boxes[i as usize]);
                }
            }
        }
    }

    /// Would a body (feet height, `height` tall, `radius` wide) overlap a box it cannot step onto?
    pub fn blocked(&self, x: f32, z: f32, feet: f32, height: f32, radius: f32) -> bool {
        let mut hit = false;
        self.near(x, z, radius, |_, b| {
            hit |= b.touches(x, z, radius) && b.bottom() < feet + height && b.top() > feet + STEP;
        });
        hit
    }

    /// Highest walkable top under (x, z) that is no more than a step above `feet`.
    pub fn support(&self, x: f32, z: f32, feet: f32) -> f32 {
        self.support_within(x, z, feet, 0.12)
    }

    /// Like [`Self::support`], for a body `radius` wide: a robot whose body is
    /// over the edge of a ledge stands on it rather than sliding down into it.
    /// A ramp counts only under the body's centre, so a body on a slope stands on the
    /// slope, not up on its uphill side or on a neighbouring wing's roof edge.
    pub fn support_within(&self, x: f32, z: f32, feet: f32, radius: f32) -> f32 {
        let mut best = f32::NEG_INFINITY;
        self.near(x, z, radius + 0.08, |i, b| {
            if b.touches(x, z, radius) && b.top() <= feet + STEP && !self.covered[i] {
                best = best.max(b.top());
            }
        });
        for ramp in self.ramps.iter().filter(|r| r.touches(x, z, 0.02)) {
            let h = ramp.height(x, z);
            if h <= feet + STEP {
                best = best.max(h);
            }
        }
        best
    }

    /// Tops of every box under (x, z): the surfaces something could stand on there.
    pub fn tops_at(&self, x: f32, z: f32) -> Vec<f32> {
        let mut tops = Vec::new();
        self.near(x, z, 0.2, |_, b| {
            if b.touches(x, z, 0.12) {
                tops.push(b.top());
            }
        });
        tops
    }

    /// Does any box intersect this volume? `except` skips boxes centred within 0.6 m of a point.
    pub fn overlaps(&self, center: Vec3, half: Vec3, except: Option<Vec3>) -> bool {
        let mut hit = false;
        self.near(center.x, center.z, half.x.max(half.z), |_, b| {
            let skip = except.is_some_and(|p| b.center.xz().distance(Vec2::new(p.x, p.z)) < 0.6);
            let apart = (0..3).any(|axis| (b.center[axis] - center[axis]).abs() >= b.half[axis] + half[axis]);
            hit |= !skip && !apart;
        });
        hit
    }

    /// A box by index (as returned by [`StaticWorld::raycast`]).
    pub fn aabb(&self, index: usize) -> Option<&Aabb> {
        self.boxes.get(index)
    }

    /// First box along a ray: (distance, box index).
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<(f32, usize)> {
        let mut best: Option<(f32, usize)> = None;
        let step = self.cell * 0.5;
        let steps = (max / step).ceil() as usize;
        let mut last = (usize::MAX, usize::MAX);
        for i in 0..=steps {
            let travelled = (i as f32 * step).min(max);
            // Nothing later along the ray can beat a hit we already have.
            if best.is_some_and(|(d, _)| d + self.cell < travelled) {
                break;
            }
            let p = origin + dir * travelled;
            let (cx, cz) = (self.coord(p.x), self.coord(p.z));
            if (cx, cz) == last {
                continue;
            }
            last = (cx, cz);
            for z in cz.saturating_sub(1)..=(cz + 1).min(self.side - 1) {
                for x in cx.saturating_sub(1)..=(cx + 1).min(self.side - 1) {
                    for &index in &self.cells[z * self.side + x] {
                        if let Some((enter, _)) = self.boxes[index as usize].ray(origin, dir, max)
                            && best.is_none_or(|(d, _)| enter < d) {
                                best = Some((enter, index as usize));
                            }
                    }
                }
            }
        }
        best
    }

    /// How far the ray travels inside box `index` (for bullets punching through thin walls).
    pub fn thickness(&self, index: usize, origin: Vec3, dir: Vec3) -> f32 {
        self.boxes.get(index).and_then(|b| b.ray(origin, dir, f32::MAX)).map_or(f32::MAX, |(enter, exit)| exit - enter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall() -> StaticWorld {
        let mut w = StaticWorld::new(50.0);
        w.add(Aabb::span(Vec3::new(4.0, 0.0, -5.0), Vec3::new(4.3, 3.0, 5.0)));
        w.add(Aabb::span(Vec3::new(-2.0, 0.0, -1.0), Vec3::new(-1.0, 0.3, 1.0)));
        w
    }

    #[test]
    fn blocking_stepping_and_support() {
        let w = wall();
        assert!(w.blocked(4.1, 0.0, 0.0, 1.7, 0.3));
        assert!(!w.blocked(0.0, 0.0, 0.0, 1.7, 0.3));
        // The low box is a step, not a wall, and you stand on it.
        assert!(!w.blocked(-1.5, 0.0, 0.0, 1.7, 0.3));
        assert!((w.support(-1.5, 0.0, 0.0) - 0.3).abs() < 1e-5);
        assert_eq!(w.support(0.0, 0.0, 0.0), f32::NEG_INFINITY);
        // Standing on top of the wall itself is possible once you are up there.
        assert!((w.support(4.1, 0.0, 3.0) - 3.0).abs() < 1e-5);
    }

    #[test]
    fn rays_hit_the_nearest_box_and_measure_thickness() {
        let w = wall();
        let (d, index) = w.raycast(Vec3::new(-10.0, 1.0, 0.0), Vec3::X, 40.0).unwrap();
        assert!((d - 14.0).abs() < 1e-4 && index == 0);
        assert!((w.thickness(0, Vec3::new(-10.0, 1.0, 0.0), Vec3::X) - 0.3).abs() < 1e-4);
        assert!(w.raycast(Vec3::new(-10.0, 4.0, 0.0), Vec3::X, 40.0).is_none());
        assert!(w.overlaps(Vec3::new(4.0, 1.0, 0.0), Vec3::splat(0.5), None));
        assert!(!w.overlaps(Vec3::new(4.0, 1.0, 0.0), Vec3::splat(0.5), Some(Vec3::new(4.15, 0.0, 0.0))));
    }
}

#[cfg(test)]
mod support_tests {
    use super::*;

    #[test]
    fn a_ramp_is_walked_smoothly_over_its_steps() {
        let mut world = StaticWorld::new(50.0);
        // Two steps up to a ridge at 0.8, and a ramp from 0 at the eaves up over their corners.
        world.add(Aabb::span(Vec3::new(-1.4, 0.0, -2.0), Vec3::new(1.4, 0.4, 2.0)));
        world.add(Aabb::span(Vec3::new(-0.8, 0.4, -2.0), Vec3::new(0.8, 0.8, 2.0)));
        let slope = 0.4 / 0.6;
        world.add_ramp(Ramp { min: Vec2::splat(-2.0), max: Vec2::splat(2.0), bottom: 0.0, top: 0.8, planes: vec![Vec3::new(slope, 0.0, 2.0 * slope), Vec3::new(-slope, 0.0, 2.0 * slope)] });
        let mut feet = 0.0;
        let mut x = -1.95f32;
        while x < 1.95 {
            let next = world.support(x, 0.0, feet);
            assert!((next - feet).abs() < 0.05, "a jump from {feet} to {next} at x = {x}");
            assert!(!world.blocked(x, 0.0, next, 1.7, 0.3), "the steps under the ramp never block it at x = {x}");
            feet = next;
            x += 0.05;
        }
        assert!((world.support(0.0, 0.0, 1.0) - 0.8).abs() < 1e-5, "the ridge is flat");
    }

    #[test]
    fn a_body_over_a_ledge_stands_on_it() {
        let mut world = StaticWorld::new(50.0);
        world.add(Aabb::span(Vec3::new(0.0, 0.0, -2.0), Vec3::new(1.0, 1.2, 2.0)));
        // Centre just off the ledge, body still over it.
        let (x, z) = (-0.2, 0.0);
        assert_eq!(world.support(x, z, 1.25), f32::NEG_INFINITY, "the centre alone misses it");
        assert!((world.support_within(x, z, 1.25, 0.27) - 1.2).abs() < 1e-5);
    }
}
