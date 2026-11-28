//! Placement audit: is everything where it should be? Street furniture on pavements, squares
//! and yards (never on a carriageway or a path, never in a doorway, at a ladder's foot, on a
//! stair landing or across a driveway); cars on roads and in car parks, lined up with them;
//! trees and rocks off the roads and out of the buildings; nothing floating or buried, nothing
//! grown into anything else; and every road going somewhere - ending at another road, an
//! open area, a doorway or the edge of the map, never running into a wall.
//!
//! [`World::audit`] lists what breaks the rules; the generators keep it empty (the tests run
//! it over every map and many seeds), and [`World::settle_street_furniture`] moves or takes
//! away the few pieces random placement still gets wrong.

use super::{AreaKind, Clutter, ClutterKind, Lane, PropKind, World};
use crate::collide::Aabb;
use glam::{Vec2, Vec3};

/// One broken placement rule.
#[derive(Clone, Debug, PartialEq)]
pub struct Finding {
    pub rule: &'static str,
    pub at: Vec2,
    pub what: String,
}

/// How far a road end may stop short of what it ends at.
const END_SLACK: f32 = 1.0;

/// Street furniture, as opposed to cover clutter (barrels, crate stacks) and cars.
pub(crate) fn furniture(kind: ClutterKind) -> bool {
    matches!(kind, ClutterKind::Bench | ClutterKind::Bin | ClutterKind::Mailbox)
}

/// Footprint of a clutter item, (lo, hi).
fn footprint(c: &Clutter) -> (Vec2, Vec2) {
    let h = c.half();
    (Vec2::new(c.pos.x - h.x, c.pos.z - h.z), Vec2::new(c.pos.x + h.x, c.pos.z + h.z))
}

fn rects_overlap(a: (Vec2, Vec2), b: (Vec2, Vec2), by: f32) -> bool {
    a.0.x < b.1.x - by && a.1.x > b.0.x + by && a.0.y < b.1.y - by && a.1.y > b.0.y + by
}

/// Does the rectangle reach more than `by` onto the lane?
fn on(lane: &Lane, r: (Vec2, Vec2), by: f32) -> bool {
    rects_overlap(lane.rect(), r, by)
}

impl World {
    /// What stands on at `p` at about height `y`: the ground, or the top of a solid under it.
    pub(crate) fn support(&self, p: Vec2, y: f32, solids: &[Aabb]) -> f32 {
        solids.iter().filter(|b| b.touches(p.x, p.y, 0.0) && b.top() <= y + 0.3).map(|b| b.top()).fold(self.height(p.x, p.y), f32::max)
    }

    /// Does a disc at `p` (radius `r`, standing at `y`) get in a doorway, at a ladder's
    /// foot, or on a stair landing or a driveway?
    pub(crate) fn in_the_way(&self, p: Vec2, y: f32, r: f32) -> Option<&'static str> {
        if self.doors.iter().any(|d| (d.center.y - y).abs() < 1.6 && Vec2::new(d.center.x, d.center.z).distance(p) < r + 1.5) {
            return Some("blocks a doorway");
        }
        if self.ladders.iter().any(|l| (l.foot.y - y).abs() < 1.6 && Vec2::new(l.foot.x, l.foot.z).distance(p) < r + 1.0) {
            return Some("blocks a ladder");
        }
        if self.areas.iter().any(|a| a.kind == AreaKind::Steps && a.contains(p, r - 0.05)) {
            return Some("blocks a flight of steps");
        }
        if self.areas.iter().any(|a| a.kind == AreaKind::Parking && a.contains(p, r - 0.05)) {
            return Some("stands across a driveway or car park");
        }
        None
    }

    /// Is a car at this spot parked properly: on a carriageway lined up with it, or in a car
    /// park or driveway? `None` if so, else what is wrong.
    pub(crate) fn parking_fault(&self, car: &Clutter) -> Option<&'static str> {
        let p = Vec2::new(car.pos.x, car.pos.z);
        let (lo, hi) = footprint(car);
        let parked = self.areas.iter().any(|a| a.kind == AreaKind::Parking && a.lo.x <= lo.x + 0.6 && a.lo.y <= lo.y + 0.6 && a.hi.x >= hi.x - 0.6 && a.hi.y >= hi.y - 0.6);
        if parked {
            return None;
        }
        let road = self.lanes.iter().filter(|l| l.carriageway()).find(|l| {
            let (a, b) = l.rect();
            p.x > a.x && p.x < b.x && p.y > a.y && p.y < b.y
        });
        match road {
            None => Some("is parked off the road"),
            // Turned, a car's long side runs along X.
            Some(l) if l.along_x() != car.turned => Some("is parked across the road"),
            Some(_) => None,
        }
    }

    /// Every broken placement rule on the map.
    pub fn audit(&self) -> Vec<Finding> {
        let mut found = Vec::new();
        let mut flag = |rule: &'static str, at: Vec2, what: String| found.push(Finding { rule, at, what });
        let solids: Vec<Aabb> = self.blocks.iter().filter(|b| b.solid).map(|b| b.aabb).collect();
        let paths: Vec<&Lane> = self.lanes.iter().filter(|l| !l.pavement()).collect();

        // Street furniture: never on a carriageway or a path, never in the way.
        for c in self.clutter.iter().filter(|c| furniture(c.kind)) {
            let p = Vec2::new(c.pos.x, c.pos.z);
            if paths.iter().any(|l| on(l, footprint(c), 0.1)) {
                flag("furniture-on-road", p, format!("a {:?} stands on a road", c.kind));
            }
        }
        for &(kind, p, r) in &self.street_items {
            if paths.iter().any(|l| l.covers(p, r, 0.1)) {
                flag("furniture-on-road", p, format!("a {kind} stands on a road"));
            }
            if let Some(why) = self.in_the_way(p, self.height(p.x, p.y), r) {
                flag("furniture-in-the-way", p, format!("a {kind} {why}"));
            }
        }
        for c in &self.clutter {
            let p = Vec2::new(c.pos.x, c.pos.z);
            let h = c.half();
            let r = h.x.min(h.z);
            if c.kind != ClutterKind::Car
                && let Some(why) = self.in_the_way(p, c.pos.y, r)
            {
                flag("furniture-in-the-way", p, format!("a {:?} {why}", c.kind));
            }
            if c.kind == ClutterKind::Car {
                if !self.lanes.is_empty()
                    && let Some(why) = self.parking_fault(c)
                {
                    flag("car-parking", p, format!("a car {why}"));
                }
                if self.doors.iter().any(|d| (d.center.y - c.pos.y).abs() < 1.6 && Vec2::new(d.center.x, d.center.z).distance(p) < r + 1.5) {
                    flag("furniture-in-the-way", p, "a car blocks a doorway".into());
                }
            }
            // Standing on something: not floating, not sunk into the ground.
            let under = self.support(p, c.pos.y, &solids);
            if (c.pos.y - under).abs() > 0.3 {
                flag("floating-or-buried", p, format!("a {:?} stands at {:.2}, its ground at {:.2}", c.kind, c.pos.y, under));
            }
        }
        // Nothing overlapping anything else.
        for (i, a) in self.clutter.iter().enumerate() {
            for b in &self.clutter[i + 1..] {
                if (a.pos.y - b.pos.y).abs() < 1.0 && rects_overlap(footprint(a), footprint(b), 0.05) {
                    flag("overlap", Vec2::new(a.pos.x, a.pos.z), format!("a {:?} and a {:?} stand in each other", a.kind, b.kind));
                }
            }
        }
        // Trees and rocks off the roads; every prop on the ground.
        for prop in &self.props {
            let p = Vec2::new(prop.pos.x, prop.pos.z);
            let r = match prop.kind {
                PropKind::Tree | PropKind::Palm => 0.35,
                PropKind::Rock => prop.scale * 0.75,
                PropKind::Bush => 0.5 * prop.scale,
                _ => continue,
            };
            if paths.iter().any(|l| l.covers(p, r, 0.1)) {
                flag("prop-on-road", p, format!("a {:?} grows on a road", prop.kind));
            }
            let under = self.support(p, prop.pos.y, &solids);
            if prop.pos.y - under > 0.3 || under - prop.pos.y > 0.6 {
                flag("floating-or-buried", p, format!("a {:?} stands at {:.2}, its ground at {:.2}", prop.kind, prop.pos.y, under));
            }
            if let Some(c) = self.clutter.iter().find(|c| rects_overlap(footprint(c), (p - Vec2::splat(r * 0.7), p + Vec2::splat(r * 0.7)), 0.05)) {
                flag("overlap", p, format!("a {:?} grows into a {:?}", prop.kind, c.kind));
            }
        }
        // Roads go somewhere, and never through a building.
        for (k, lane) in self.lanes.iter().enumerate() {
            for end in [lane.a, lane.b] {
                if !self.road_end_ok(k, end) {
                    flag("road-dead-end", end, format!("a {:.1} m road from {:?} to {:?} ends in nothing", lane.width, lane.a, lane.b));
                }
            }
            let (lo, hi) = lane.rect();
            let (lo, hi) = (lo + Vec2::splat(0.3), hi - Vec2::splat(0.3));
            for block in self.blocks.iter().filter(|b| b.solid) {
                let b = &block.aabb;
                let (bl, bh) = (b.min(), b.max());
                if bl.x >= hi.x || bh.x <= lo.x || bl.z >= hi.y || bh.z <= lo.y {
                    continue;
                }
                let c = Vec2::new(b.center.x.clamp(lo.x, hi.x), b.center.z.clamp(lo.y, hi.y));
                let g = self.height(c.x, c.y);
                // A wall: from about the road's surface to over a head.
                if bl.y < g + 1.0 && bh.y > g + 2.2 {
                    flag("road-into-building", c, format!("a {:.1} m road from {:?} to {:?} runs into a {:?} wall at {:?} half {:?}", lane.width, lane.a, lane.b, block.paint, b.center, b.half));
                    break;
                }
            }
        }
        found
    }

    /// Does the end of lane `k` at `end` lead somewhere?
    fn road_end_ok(&self, k: usize, end: Vec2) -> bool {
        // Off the edge of the fighting square, where the map runs on out of reach.
        let edge = (self.recipe.half_extent - 34.0).min(self.arena - 3.0);
        if end.x.abs() > edge || end.y.abs() > edge {
            return true;
        }
        let lane = self.lanes[k];
        let reach = lane.width * 0.5 + END_SLACK;
        self.lanes.iter().enumerate().any(|(m, o)| m != k && o.covers(end, reach, 0.0))
            || self.areas.iter().any(|a| a.contains(end, reach))
            || self.doors.iter().any(|d| Vec2::new(d.center.x, d.center.z).distance(end) < reach + 2.5)
    }
}

impl World {
    /// Puts right what random placement got wrong: street furniture on a road goes to the
    /// kerbside (turned to face along it) or, with no room there, is taken away; a car is
    /// lined up with the road it is on, or moved to the nearest kerb, or taken away; nothing
    /// is left in a doorway, at a ladder or on a landing; trees and rocks on a road go;
    /// anything floating or sunk is set on what is under it; and of two things standing in
    /// each other, the later goes.
    pub(crate) fn settle_street_furniture(&mut self) {
        let solids: Vec<Aabb> = self.blocks.iter().filter(|b| b.solid).map(|b| b.aabb).collect();
        let lanes: Vec<Lane> = self.lanes.iter().copied().filter(|l| !l.pavement()).collect();
        let items = std::mem::take(&mut self.clutter);
        let mut kept: Vec<Clutter> = Vec::with_capacity(items.len());
        for mut c in items {
            let ok = |w: &World, c: &Clutter, kept: &[Clutter]| {
                let p = Vec2::new(c.pos.x, c.pos.z);
                let h = c.half();
                let fp = footprint(c);
                let clear_of_walls = !solids.iter().any(|b| b.top() > c.pos.y + 0.45 && b.bottom() < c.pos.y + 2.0 * h.y && rects_overlap((Vec2::new(b.min().x, b.min().z), Vec2::new(b.max().x, b.max().z)), fp, 0.02));
                let clear_of_others = !kept.iter().any(|o| (o.pos.y - c.pos.y).abs() < 1.0 && rects_overlap(footprint(o), fp, -0.3));
                let placed = if c.kind == ClutterKind::Car {
                    w.lanes.is_empty() || w.parking_fault(c).is_none()
                } else {
                    !(furniture(c.kind) && lanes.iter().any(|l| on(l, fp, 0.1)))
                };
                let r = if c.kind == ClutterKind::Car { h.x.min(h.z) } else { h.x.max(h.z) * 0.8 };
                let door = w.doors.iter().any(|d| (d.center.y - c.pos.y).abs() < 1.6 && Vec2::new(d.center.x, d.center.z).distance(p) < r + 1.5);
                let way = c.kind != ClutterKind::Car && w.in_the_way(p, c.pos.y, h.x.min(h.z)).is_some();
                clear_of_walls && clear_of_others && placed && !door && !way && w.in_bounds(p.x, p.y)
            };
            let settle = |w: &World, c: &mut Clutter| {
                let p = Vec2::new(c.pos.x, c.pos.z);
                c.pos.y = w.support(p, c.pos.y.max(w.height(p.x, p.y)), &solids);
            };
            settle(self, &mut c);
            if ok(self, &c, &kept) {
                kept.push(c);
                continue;
            }
            let p = Vec2::new(c.pos.x, c.pos.z);
            // Try it again somewhere sensible near by: furniture at the kerbside, cars lined up
            // with the road or at its nearest kerb.
            let mut tries: Vec<Clutter> = Vec::new();
            let mut near: Vec<&Lane> = lanes.iter().filter(|l| if c.kind == ClutterKind::Car { l.carriageway() } else { true }).filter(|l| l.covers(p, 7.0, 0.0)).collect();
            near.sort_by(|a, b| {
                let d = |l: &Lane| super::themes::distance_to_segment(p, l.a, l.b);
                d(a).total_cmp(&d(b))
            });
            for l in near.iter().take(3) {
                let (lo, hi) = l.rect();
                let along_x = l.along_x();
                let h = half_at(c.kind, if c.kind == ClutterKind::Car { along_x } else { !along_x });
                for side in [-1.0f32, 1.0] {
                    let (edge, across) = if along_x { (if side < 0.0 { lo.y } else { hi.y }, p.y) } else { (if side < 0.0 { lo.x } else { hi.x }, p.x) };
                    let depth = if along_x { h.z } else { h.x };
                    // A car parks just inside the kerb; furniture stands just outside it.
                    let to = if c.kind == ClutterKind::Car { edge - side * (depth + 0.5) } else { edge + side * (depth + 0.45) };
                    let _ = across;
                    let at = if along_x { Vec2::new(p.x.clamp(lo.x + 2.5, hi.x - 2.5), to) } else { Vec2::new(to, p.y.clamp(lo.y + 2.5, hi.y - 2.5)) };
                    let turned = if c.kind == ClutterKind::Car { along_x } else { !along_x };
                    tries.push(Clutter { kind: c.kind, pos: Vec3::new(at.x, self.height(at.x, at.y), at.y), turned });
                }
            }
            tries.sort_by(|a, b| a.pos.distance_squared(c.pos).total_cmp(&b.pos.distance_squared(c.pos)));
            for mut t in tries {
                settle(self, &mut t);
                if ok(self, &t, &kept) && (t.pos.y - c.pos.y).abs() < 1.5 {
                    kept.push(t);
                    break;
                }
            }
        }
        self.clutter = kept;

        // Trees, rocks and bushes: off the roads, on the ground, clear of the clutter.
        let clutter = self.clutter.clone();
        let water = self.water;
        let props = std::mem::take(&mut self.props);
        self.props = props
            .into_iter()
            .filter_map(|mut prop| {
                let p = Vec2::new(prop.pos.x, prop.pos.z);
                let r = match prop.kind {
                    PropKind::Tree | PropKind::Palm => 0.35,
                    PropKind::Rock => prop.scale * 0.75,
                    PropKind::Bush => 0.5 * prop.scale,
                    _ => return Some(prop),
                };
                if lanes.iter().any(|l| l.covers(p, r, 0.1)) || clutter.iter().any(|c| rects_overlap(footprint(c), (p - Vec2::splat(r * 0.7), p + Vec2::splat(r * 0.7)), 0.05)) {
                    return None;
                }
                let under = self.support(p, prop.pos.y, &solids);
                if prop.pos.y - under > 0.3 || under - prop.pos.y > 0.6 {
                    if under < water - 0.3 {
                        return None;
                    }
                    prop.pos.y = under;
                }
                Some(prop)
            })
            .collect();
    }
}

/// The footprint of a clutter item at `pos`, for placing it.
pub(crate) fn half_at(kind: ClutterKind, turned: bool) -> Vec3 {
    Clutter { kind, pos: Vec3::ZERO, turned }.half()
}
