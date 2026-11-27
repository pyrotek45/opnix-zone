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
        Self {
            center: (a + b) * 0.5,
            half: (a - b).abs() * 0.5,
        }
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
        (x - self.center.x).abs() < self.half.x + radius
            && (z - self.center.z).abs() < self.half.z + radius
    }

    /// Ray entry and exit distances, if the ray hits within `max`.
    pub fn ray(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<(f32, f32)> {
        let (mut lo, mut hi) = (0.0f32, max);
        for axis in 0..3 {
            let (min, max) = (
                self.center[axis] - self.half[axis],
                self.center[axis] + self.half[axis],
            );
            if dir[axis].abs() < 1e-6 {
                if origin[axis] < min || origin[axis] > max {
                    return None;
                }
            } else {
                let (a, b) = (
                    (min - origin[axis]) / dir[axis],
                    (max - origin[axis]) / dir[axis],
                );
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

pub struct StaticWorld {
    boxes: Vec<Aabb>,
    cell: f32,
    half: f32,
    side: usize,
    cells: Vec<Vec<u32>>,
}

impl StaticWorld {
    pub fn new(half_extent: f32) -> Self {
        let cell = 4.0;
        let side = (2.0 * half_extent / cell).ceil() as usize + 1;
        Self {
            boxes: Vec::new(),
            cell,
            half: half_extent,
            side,
            cells: vec![Vec::new(); side * side],
        }
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
    pub fn support_within(&self, x: f32, z: f32, feet: f32, radius: f32) -> f32 {
        let mut best = f32::NEG_INFINITY;
        self.near(x, z, radius + 0.08, |_, b| {
            if b.touches(x, z, radius) && b.top() <= feet + STEP {
                best = best.max(b.top());
            }
        });
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
            let apart = (0..3)
                .any(|axis| (b.center[axis] - center[axis]).abs() >= b.half[axis] + half[axis]);
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
                            && best.is_none_or(|(d, _)| enter < d)
                        {
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
        self.boxes
            .get(index)
            .and_then(|b| b.ray(origin, dir, f32::MAX))
            .map_or(f32::MAX, |(enter, exit)| exit - enter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall() -> StaticWorld {
        let mut w = StaticWorld::new(50.0);
        w.add(Aabb::span(
            Vec3::new(4.0, 0.0, -5.0),
            Vec3::new(4.3, 3.0, 5.0),
        ));
        w.add(Aabb::span(
            Vec3::new(-2.0, 0.0, -1.0),
            Vec3::new(-1.0, 0.3, 1.0),
        ));
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
        let (d, index) = w
            .raycast(Vec3::new(-10.0, 1.0, 0.0), Vec3::X, 40.0)
            .unwrap();
        assert!((d - 14.0).abs() < 1e-4 && index == 0);
        assert!((w.thickness(0, Vec3::new(-10.0, 1.0, 0.0), Vec3::X) - 0.3).abs() < 1e-4);
        assert!(
            w.raycast(Vec3::new(-10.0, 4.0, 0.0), Vec3::X, 40.0)
                .is_none()
        );
        assert!(w.overlaps(Vec3::new(4.0, 1.0, 0.0), Vec3::splat(0.5), None));
        assert!(!w.overlaps(
            Vec3::new(4.0, 1.0, 0.0),
            Vec3::splat(0.5),
            Some(Vec3::new(4.15, 0.0, 0.0))
        ));
    }
}

#[cfg(test)]
mod support_tests {
    use super::*;

    #[test]
    fn a_body_over_a_ledge_stands_on_it() {
        let mut world = StaticWorld::new(50.0);
        world.add(Aabb::span(
            Vec3::new(0.0, 0.0, -2.0),
            Vec3::new(1.0, 1.2, 2.0),
        ));
        // Centre just off the ledge, body still over it.
        let (x, z) = (-0.2, 0.0);
        assert_eq!(
            world.support(x, z, 1.25),
            f32::NEG_INFINITY,
            "the centre alone misses it"
        );
        assert!((world.support_within(x, z, 1.25, 0.27) - 1.2).abs() < 1e-5);
    }
}
