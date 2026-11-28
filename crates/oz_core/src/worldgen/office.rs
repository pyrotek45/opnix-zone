//! Office Tower: a fixed, mostly indoor map. One big four-storey office block (76 x 48 m) round
//! a central atrium, with four stairwells up to a roof of parapets and plant rooms.
//!
//! * Ground floor: the lobby (revolving-door entrances, reception desk, security gates, sofas,
//!   planters), a cafe, a print and mail room, a glass-walled security office, washrooms and
//!   the lift core.
//! * Upper floors: a cubicle farm on one side and glass-walled offices on the other (they swap
//!   between floors), a meeting room with a long table (a server room upstairs), copier
//!   stations, filing cabinets, plants, water coolers.
//! * Outside: a car park to the south, a fountain plaza to the north, the team bases at either
//!   end of the plaza (x = ±62).
//!
//! Desks, sofas, counters, copiers and cabinets are low cover; partitions and glass walls are
//! crouch height or have sills at [`WINDOW_SILL`]; screens, keyboards and papers are thin.

use super::{
    COVER_HEIGHT, Clutter, ClutterKind, DOOR_HEIGHT, DOOR_WIDTH, Prop, PropKind, STOREY, WINDOW_SILL, World,
};
use crate::{collide::Aabb, model::Paint, rng::Rng};
use glam::{Vec2, Vec3};
use std::f32::consts::TAU;

const X0: f32 = -38.0;
const X1: f32 = 38.0;
const Z0: f32 = -24.0;
const Z1: f32 = 24.0;
const T: f32 = 0.3;
const FLOORS: u32 = 4;
/// The atrium's hole through the upper floors.
const ATRIUM: (f32, f32, f32, f32) = (-9.0, 9.0, -6.0, 6.0);
/// Stairwells: west end x, and whether against the south wall (else the north).
const WELLS: [(f32, bool); 4] = [(-30.0, true), (-30.0, false), (23.0, true), (23.0, false)];
/// The lift shafts.
const LIFTS: [f32; 2] = [-13.0, 13.0];
const BASE_X: f32 = 62.0;

/// Openings along a wall of `length`: doorways at `doors` (centre, width) and evenly spaced
/// windows (sill at crouch-cover height) everywhere else.
fn ribbon(length: f32, doors: &[(f32, f32)], every: f32, width: f32, top: f32) -> Vec<(f32, f32, f32, f32)> {
    let mut out: Vec<(f32, f32, f32, f32)> = doors.iter().map(|(c, w)| (c - w * 0.5, c + w * 0.5, 0.0, DOOR_HEIGHT)).collect();
    let mut u = 1.0;
    while u + width < length - 0.8 {
        if doors.iter().all(|(c, w)| u + width < c - w * 0.5 - 0.45 || u > c + w * 0.5 + 0.45) {
            out.push((u, u + width, WINDOW_SILL, top));
        }
        u += every;
    }
    out
}

impl World {
    pub(super) fn office_tower(&mut self, rng: &mut Rng) {
        let g = self.highest(Vec2::ZERO, Vec2::new(X1, Z1)) + 0.12;
        self.solid(Aabb::span(Vec3::new(X0, g - 2.5, Z0), Vec3::new(X1, g, Z1)), Paint::Concrete);
        self.decor(Aabb::span(Vec3::new(X0 + T, g, Z0 + T), Vec3::new(X1 - T, g + 0.012, Z1 - T)), Paint::Tile);
        self.note_room(Paint::Tile, Paint::Steel);

        // The shell: walls of windows, doors on the ground floor, trims at every floor line.
        let long = X1 - X0;
        let short = Z1 - Z0 - 2.0 * T;
        for floor in 0..FLOORS {
            let base = g + floor as f32 * STOREY;
            let ground = floor == 0;
            let paint = if floor == 0 { Paint::Steel } else { Paint::Concrete };
            let south: Vec<(f32, f32)> = if ground { vec![(36.0, DOOR_WIDTH), (40.0, DOOR_WIDTH), (22.0, DOOR_WIDTH), (54.0, DOOR_WIDTH)] } else { vec![] };
            let north: Vec<(f32, f32)> = if ground { vec![(28.0, DOOR_WIDTH), (48.0, DOOR_WIDTH)] } else { vec![] };
            let side: Vec<(f32, f32)> = if ground { vec![(short * 0.5, DOOR_WIDTH), (short * 0.5 - 13.0, DOOR_WIDTH), (short * 0.5 + 13.0, DOOR_WIDTH)] } else { vec![] };
            self.wall(true, (Z0, Z0 + T), X0, long, base, STOREY, &ribbon(long, &south, 3.2, 2.0, 2.65), paint);
            self.wall(true, (Z1 - T, Z1), X0, long, base, STOREY, &ribbon(long, &north, 3.2, 2.0, 2.65), paint);
            self.wall(false, (X0, X0 + T), Z0 + T, short, base, STOREY, &ribbon(short, &side, 3.2, 2.0, 2.65), paint);
            self.wall(false, (X1 - T, X1), Z0 + T, short, base, STOREY, &ribbon(short, &side, 3.2, 2.0, 2.65), paint);
            self.ring(X0, X1, Z0, Z1, base + STOREY - 0.2, base + STOREY, 0.06, Paint::Dark, [false; 4]);
        }
        // Corner columns, flush with the walls, the full height.
        let roof = g + FLOORS as f32 * STOREY;
        for (x, z) in [(X0 + 0.2, Z0 + 0.2), (X1 - 0.2, Z0 + 0.2), (X0 + 0.2, Z1 - 0.2), (X1 - 0.2, Z1 - 0.2)] {
            self.solid(Aabb::new(Vec3::new(x, (g + roof) * 0.5, z), Vec3::new(0.26, (roof - g) * 0.5, 0.26)), Paint::Dark);
        }
        // An entrance canopy over the south doors.
        self.solid(Aabb::span(Vec3::new(-6.0, g + 2.9, Z0 - 3.0), Vec3::new(6.0, g + 3.15, Z0)), Paint::Dark);
        for x in [-5.6, 5.6] {
            self.solid(Aabb::span(Vec3::new(x - 0.15, g, Z0 - 2.85), Vec3::new(x + 0.15, g + 2.9, Z0 - 2.55)), Paint::Steel);
        }
        self.decor(Aabb::span(Vec3::new(-4.0, g + 3.15, Z0 - 0.4), Vec3::new(4.0, g + 3.9, Z0 - 0.3)), Paint::Blue);

        // Stairs, and the floors with their holes.
        let mut holes: Vec<Vec<(f32, f32, f32, f32)>> = vec![Vec::new(); FLOORS as usize + 1];
        for (x0, south) in WELLS {
            let (wall, inward) = if south { (Z0 + T, 1.0) } else { (Z1 - T, -1.0) };
            for (k, hole) in self.stairwell(x0, wall, inward, g, FLOORS).into_iter().enumerate() {
                holes[k + 1].push(hole);
            }
        }
        for k in 1..=FLOORS {
            let top = g + k as f32 * STOREY;
            let mut cut = holes[k as usize].clone();
            if k < FLOORS {
                cut.push(ATRIUM);
            }
            let finish = if k < FLOORS { Some((if k % 2 == 1 { Paint::Carpet } else { Paint::Lino }, T)) } else { None };
            if let Some((floor, _)) = finish {
                self.note_room(floor, Paint::Concrete);
                self.note_room(floor, Paint::Bone);
            }
            self.slab((X0, X1, Z0, Z1), top, &cut, finish);
            // Ceiling panels flush under the slab (0.2 thick).
            let under = top - 0.22;
            let mut x = X0 + 4.0;
            while x < X1 - 2.0 {
                let mut z = Z0 + 4.0;
                while z < Z1 - 2.0 {
                    let inside_atrium = x > ATRIUM.0 && x < ATRIUM.1 && z > ATRIUM.2 && z < ATRIUM.3;
                    if !inside_atrium || k == FLOORS {
                        self.decor(Aabb::new(Vec3::new(x, under, z), Vec3::new(0.6, 0.02, 0.3)), Paint::Glow);
                    }
                    z += 8.0;
                }
                x += 8.0;
            }
            if k < FLOORS {
                // A balustrade round the atrium: crouch-high cover with a rail on top.
                let (a0, a1, b0, b1) = ATRIUM;
                for (p, q) in [((a0 - 0.12, b0 - 0.12), (a1 + 0.12, b0)), ((a0 - 0.12, b1), (a1 + 0.12, b1 + 0.12)), ((a0 - 0.12, b0), (a0, b1)), ((a1, b0), (a1 + 0.12, b1))] {
                    self.solid(Aabb::span(Vec3::new(p.0, top, p.1), Vec3::new(q.0, top + COVER_HEIGHT, q.1)), Paint::Steel);
                }
                self.ring(a0 - 0.12, a1 + 0.12, b0 - 0.12, b1 + 0.12, top + COVER_HEIGHT - 0.04, top + COVER_HEIGHT + 0.04, 0.04, Paint::Gold, [false; 4]);
            }
        }
        // The lift core: two shafts floor to roof, doors on every floor.
        for x in LIFTS {
            self.solid(Aabb::span(Vec3::new(x - 1.5, g, -2.0), Vec3::new(x + 1.5, roof + 1.4, 2.0)), Paint::Bone);
            for floor in 0..FLOORS {
                let y = g + floor as f32 * STOREY;
                for s in [-1.0, 1.0] {
                    let face = x - s * 1.5 - s * 0.01;
                    self.decor(Aabb::span(Vec3::new(face - 0.02, y, -0.8), Vec3::new(face + 0.02, y + 2.3, 0.8)), Paint::Steel);
                    self.decor(Aabb::span(Vec3::new(face - 0.025, y, -0.02), Vec3::new(face + 0.025, y + 2.3, 0.02)), Paint::Dark);
                    self.decor(Aabb::span(Vec3::new(face - 0.03, y + 2.45, -0.25), Vec3::new(face + 0.03, y + 2.6, 0.25)), Paint::Glow);
                }
            }
        }

        self.lobby(rng, g);
        self.office_floor(rng, g + STOREY, 1.0, false);
        self.office_floor(rng, g + 2.0 * STOREY, -1.0, true);
        self.office_floor(rng, g + 3.0 * STOREY, 1.0, true);
        self.office_roof(rng, roof);
        self.office_grounds(rng, g);
        self.team_bases(BASE_X, 0.0, Paint::Concrete);
        // Where to fight: every floor, the roof, the plaza and the car park.
        for k in 0..=FLOORS {
            self.waypoints_over(g + k as f32 * STOREY, (X0 + 1.0, X1 - 1.0, Z0 + 1.0, Z1 - 1.0), 7.0);
        }
        self.waypoints_over(self.height(0.0, 40.0), (-30.0, 30.0, 28.0, 52.0), 9.0);
        self.waypoints_over(self.height(0.0, -40.0), (-34.0, 34.0, -50.0, -28.0), 9.0);
    }

    /// The ground floor.
    fn lobby(&mut self, rng: &mut Rng, y: f32) {
        // Reception: an L-shaped counter (cover), a sign wall behind it.
        self.solid(Aabb::span(Vec3::new(-4.0, y, -13.0), Vec3::new(4.0, y + 1.1, -12.4)), Paint::Wood);
        self.solid(Aabb::span(Vec3::new(3.4, y, -12.4), Vec3::new(4.0, y + 1.1, -10.6)), Paint::Wood);
        self.decor(Aabb::span(Vec3::new(-4.05, y + 1.1, -13.05), Vec3::new(4.05, y + 1.15, -12.35)), Paint::Bone);
        self.desk(rng, Vec2::new(-1.5, -11.6), Vec2::Y, y);
        self.desk(rng, Vec2::new(1.5, -11.6), Vec2::Y, y);
        self.solid(Aabb::span(Vec3::new(-3.5, y, -8.6), Vec3::new(3.5, y + 2.6, -8.3)), Paint::Teal);
        self.decor(Aabb::span(Vec3::new(-2.4, y + 1.6, -8.64), Vec3::new(2.4, y + 2.2, -8.6)), Paint::Glow);
        // Security gates: low posts across the hall, gaps to walk through.
        for x in [-8.0, -6.5, -5.0, 5.0, 6.5, 8.0] {
            self.solid(Aabb::span(Vec3::new(x - 0.12, y, -17.6), Vec3::new(x + 0.12, y + 1.0, -16.4)), Paint::Steel);
            self.decor(Aabb::span(Vec3::new(x - 0.13, y + 0.9, -17.2), Vec3::new(x + 0.13, y + 0.95, -16.8)), Paint::Green);
        }
        // Sofas and plants either side of the entrance, planters in the atrium, a sculpture.
        for s in [-1.0f32, 1.0] {
            self.solid(Aabb::span(Vec3::new(s * 13.0 - 1.4, y, -20.5), Vec3::new(s * 13.0 + 1.4, y + 0.8, -19.7)), Paint::Coral);
            self.solid(Aabb::span(Vec3::new(s * 13.0 - 0.6, y, -18.8), Vec3::new(s * 13.0 + 0.6, y + 0.42, -18.0)), Paint::Wood);
            self.potted_plant(rng, Vec2::new(s * 9.5, -22.6), y);
            for dz in [-3.5, 3.5] {
                let p = Vec2::new(s * 5.0, dz);
                self.solid(Aabb::span(Vec3::new(p.x - 1.6, y, p.y - 0.8), Vec3::new(p.x + 1.6, y + 0.7, p.y + 0.8)), Paint::Concrete);
                for k in [-0.8, 0.8] {
                    self.props.push(Prop { kind: PropKind::Bush, pos: Vec3::new(p.x + k, y + 0.65, p.y), yaw: rng.range(0.0, TAU), scale: 0.8 });
                }
            }
        }
        self.column(Vec2::ZERO, 0.8, y, y + 2.4, Paint::Gold);
        self.column(Vec2::ZERO, 0.5, y + 2.4, y + 3.6, Paint::Copper);

        // The cafe (west): a counter, tables and chairs, vending machines.
        self.solid(Aabb::span(Vec3::new(-34.0, y, 10.0), Vec3::new(-22.0, y + 1.05, 10.9)), Paint::Bone);
        self.decor(Aabb::span(Vec3::new(-34.05, y + 1.05, 9.95), Vec3::new(-21.95, y + 1.1, 10.95)), Paint::Wood);
        for (x, z) in [(-33.0, 10.45), (-29.0, 10.45), (-25.0, 10.45)] {
            self.solid(Aabb::span(Vec3::new(x - 0.25, y + 1.1, z - 0.2), Vec3::new(x + 0.25, y + 1.5, z + 0.2)), Paint::Steel);
        }
        for i in 0..3 {
            for j in 0..3 {
                let p = Vec2::new(-31.0 + i as f32 * 6.0, -14.0 + j as f32 * 6.5);
                self.solid(Aabb::span(Vec3::new(p.x - 0.6, y, p.y - 0.6), Vec3::new(p.x + 0.6, y + 0.76, p.y + 0.6)), Paint::Wood);
                self.office_chair(p + Vec2::new(0.0, -1.05), Vec2::NEG_Y, y);
                self.office_chair(p + Vec2::new(0.0, 1.05), Vec2::Y, y);
            }
        }
        for z in [-4.5, -3.4, -2.3] {
            self.vending(Vec2::new(X0 + T + 0.45, z * 2.0), y, if z < -4.0 { Paint::Blue } else { Paint::Glow });
        }
        // Print and mail room (east): shelving, copiers, cabinets.
        for z in [-16.0, -12.5, -9.0] {
            self.solid(Aabb::span(Vec3::new(18.0, y, z - 0.3), Vec3::new(28.0, y + 1.9, z + 0.3)), Paint::Steel);
            for k in 0..3 {
                self.decor(Aabb::span(Vec3::new(18.0, y + 0.5 + k as f32 * 0.55, z - 0.32), Vec3::new(28.0, y + 0.53 + k as f32 * 0.55, z + 0.32)), Paint::Dark);
            }
        }
        self.printer(Vec2::new(15.5, 8.0), y, true);
        self.printer(Vec2::new(15.5, 10.5), y, true);
        self.printer(Vec2::new(-15.5, 8.0), y, false);
        self.filing_cabinets(Vec2::new(22.0, 12.5), 4, y);
        // The security office: glass on two sides.
        self.glass_room((29.0, X1 - T, 3.0, 10.8), y, 1, rng);
        self.desk(rng, Vec2::new(33.5, 7.0), Vec2::NEG_X, y);
        // Washrooms by the north wall.
        self.wall(true, (14.0, 14.2), -6.0, 12.0, y, STOREY - 0.2, &[(2.0, 3.7, 0.0, DOOR_HEIGHT), (8.3, 10.0, 0.0, DOOR_HEIGHT)], Paint::Bone);
        self.wall(false, (-0.1, 0.1), 14.2, Z1 - T - 14.2, y, STOREY - 0.2, &[], Paint::Bone);
        for x in [-6.0, 6.0] {
            self.wall(false, (x - 0.1, x + 0.1), 14.2, Z1 - T - 14.2, y, STOREY - 0.2, &[], Paint::Bone);
        }
        for x in [-3.0, 3.0] {
            self.solid(Aabb::span(Vec3::new(x - 2.0, y, Z1 - T - 0.6), Vec3::new(x + 2.0, y + 0.9, Z1 - T)), Paint::Bone);
        }
    }

    /// An upper floor. `s` mirrors it (cubicles west on +1, east on -1); `servers` swaps the
    /// meeting room for a server room.
    fn office_floor(&mut self, rng: &mut Rng, y: f32, s: f32, servers: bool) {
        // Cubicle pods: two desks back to back either side of a crouch-high spine, ends closed.
        for px in [-33.0, -27.6, -22.2, -16.8] {
            for pz in [-15.0f32, -9.0, 9.0, 15.0] {
                let c = Vec2::new(px * s, pz);
                self.solid(Aabb::span(Vec3::new(c.x - 2.0, y, c.y - 0.04), Vec3::new(c.x + 2.0, y + 1.2, c.y + 0.04)), Paint::Teal);
                for e in [-2.0, 2.0] {
                    self.solid(Aabb::span(Vec3::new(c.x + e - 0.04, y, c.y - 1.6), Vec3::new(c.x + e + 0.04, y + 1.2, c.y + 1.6)), Paint::Teal);
                }
                self.decor(Aabb::span(Vec3::new(c.x - 2.05, y + 1.2, c.y - 0.06), Vec3::new(c.x + 2.05, y + 1.24, c.y + 0.06)), Paint::Steel);
                for side in [-1.0f32, 1.0] {
                    for dx in [-1.0, 1.0] {
                        if rng.chance(0.92) {
                            self.desk(rng, Vec2::new(c.x + dx, c.y + side * 0.45), Vec2::new(0.0, side), y);
                        }
                    }
                }
            }
        }
        // A copier station and a break corner on the cubicle side, near the lifts.
        self.printer(Vec2::new(-16.5 * s, 4.0), y, true);
        self.printer(Vec2::new(-16.5 * s, -4.0), y, false);
        self.solid(Aabb::span(Vec3::new((-21.0 * s).min(-19.0 * s), y, -1.0), Vec3::new((-21.0 * s).max(-19.0 * s), y + 1.0, 1.0)), Paint::Steel);
        self.vending(Vec2::new(-24.5 * s, 0.0), y, Paint::Blue);
        self.potted_plant(rng, Vec2::new(-28.0 * s, 3.0), y);
        self.potted_plant(rng, Vec2::new(-28.0 * s, -3.0), y);
        self.filing_cabinets(Vec2::new(-34.0 * s, 4.5), 3, y);
        self.filing_cabinets(Vec2::new(-34.0 * s, -4.5), 3, y);

        // Glass offices along both long walls on the other side, either side of the stairwell:
        // glass fronts with doors, solid dividers between them.
        for south in [true, false] {
            let (front, back) = if south { (-18.5f32, Z0 + T) } else { (18.5f32, Z1 - T) };
            let (z0, z1) = (front.min(back), front.max(back));
            for x in [10.0, 16.4, 22.6, 30.4] {
                let x = x * s;
                self.wall(false, (x - 0.08, x + 0.08), z0, z1 - z0, y, STOREY - 0.2, &[], Paint::Bone);
            }
            for (a, b) in [(10.0, 16.4), (16.4, 22.6), (30.4, X1 - T)] {
                let (x0, x1) = ((a * s).min(b * s), (a * s).max(b * s));
                let (lo, hi) = (x0 + 0.08, x1 - 0.08);
                let door = rng.range(1.2, (hi - lo - 1.2).max(1.2));
                let mut o = vec![(door - DOOR_WIDTH * 0.5, door + DOOR_WIDTH * 0.5, 0.0, DOOR_HEIGHT)];
                let mut u = 0.3;
                while u + 1.5 < hi - lo - 0.2 {
                    if u + 1.5 < door - DOOR_WIDTH * 0.5 - 0.3 || u > door + DOOR_WIDTH * 0.5 + 0.3 {
                        o.push((u, u + 1.5, WINDOW_SILL, 2.6));
                    }
                    u += 1.8;
                }
                self.wall(true, (front - 0.15, front + 0.15), lo, hi - lo, y, STOREY - 0.2, &o, Paint::Steel);
                let mid = Vec2::new((x0 + x1) * 0.5, (z0 + z1) * 0.5);
                let face = Vec2::new(0.0, if south { 1.0 } else { -1.0 });
                self.desk(rng, mid - face * 0.6, face, y);
                self.filing_cabinets(Vec2::new(x1 - 1.0, back + if south { 0.6 } else { -0.6 }), 2, y);
                if rng.chance(0.6) {
                    self.potted_plant(rng, Vec2::new(x0 + 0.8, back + if south { 0.8 } else { -0.8 }), y);
                }
            }
        }
        // The meeting room in the middle of that side, or the server room.
        let (x0, x1) = ((17.0 * s).min(33.0 * s), (17.0 * s).max(33.0 * s));
        self.glass_room((x0, x1, -6.0, 6.0), y, if s > 0.0 { 1 } else { 2 }, rng);
        if servers {
            for z in [-3.5f32, 0.0, 3.5] {
                for x in [x0 + 2.0, x0 + 6.5, x0 + 11.0] {
                    self.solid(Aabb::span(Vec3::new(x, y, z - 0.45), Vec3::new(x + 3.4, y + 2.0, z + 0.45)), Paint::Dark);
                    for k in 0..6 {
                        let lx = x + 0.3 + k as f32 * 0.55;
                        let paint = if (k + (z as i32).unsigned_abs() as usize).is_multiple_of(3) { Paint::Green } else { Paint::Blue };
                        self.decor(Aabb::span(Vec3::new(lx, y + 1.2, z - 0.47), Vec3::new(lx + 0.08, y + 1.25, z - 0.45)), paint);
                    }
                }
            }
        } else {
            let c = Vec2::new((x0 + x1) * 0.5, 0.0);
            self.meeting_table(c, 9.0, y);
            // A screen on the far wall.
            let wall = if s > 0.0 { x1 - 0.3 } else { x0 + 0.3 };
            self.decor(Aabb::span(Vec3::new(wall - 0.03, y + 1.0, -2.0), Vec3::new(wall + 0.03, y + 2.4, 2.0)), Paint::Screen);
            self.decor(Aabb::span(Vec3::new(wall - 0.035, y + 1.1, -1.8), Vec3::new(wall + 0.035, y + 2.3, 1.8)), Paint::Blue);
        }
        // A water cooler and a copier by the atrium on the office side.
        self.printer(Vec2::new(11.0 * s, -9.5), y, true);
        self.solid(Aabb::span(Vec3::new(11.0 * s - 0.25, y, 9.2), Vec3::new(11.0 * s + 0.25, y + 1.3, 9.7)), Paint::Bone);
        self.solid(Aabb::span(Vec3::new(11.0 * s - 0.18, y + 1.3, 9.3), Vec3::new(11.0 * s + 0.18, y + 1.75, 9.6)), Paint::Aether);
    }

    /// A glass-walled room (x0, x1, z0, z1) on floor `y`: the side `door` (south, west, east,
    /// north) is glass with a door in it, the other sides are glass above crouch height too,
    /// except where they meet the outer walls.
    fn glass_room(&mut self, (x0, x1, z0, z1): (f32, f32, f32, f32), y: f32, door: usize, rng: &mut Rng) {
        let h = STOREY - 0.2;
        let t = 0.15;
        let glass = |length: f32, with_door: bool, rng: &mut Rng| {
            let door = with_door.then(|| (rng.range(1.3, (length - 1.3).max(1.3)), DOOR_WIDTH));
            let mut out: Vec<(f32, f32, f32, f32)> = door.iter().map(|(c, w)| (c - w * 0.5, c + w * 0.5, 0.0, DOOR_HEIGHT)).collect();
            let mut u = 0.4;
            while u + 1.6 < length - 0.3 {
                if door.iter().all(|(c, w)| u + 1.6 < c - w * 0.5 - 0.3 || u > c + w * 0.5 + 0.3) {
                    out.push((u, u + 1.6, WINDOW_SILL, 2.6));
                }
                u += 1.9;
            }
            out
        };
        let outer = |v: f32, lo: f32, hi: f32| (v - lo).abs() < 0.5 || (v - hi).abs() < 0.5;
        // South and north sides.
        for (side, z) in [(0usize, z0), (3, z1)] {
            if outer(z, Z0 + T, Z1 - T) {
                continue;
            }
            let o = glass(x1 - x0, side == door, rng);
            self.wall(true, (z - t, z + t), x0, x1 - x0, y, h, &o, Paint::Steel);
        }
        for (side, x) in [(1usize, x0), (2, x1)] {
            if outer(x, X0 + T, X1 - T) {
                continue;
            }
            let o = glass(z1 - z0 - 2.0 * t, side == door, rng);
            self.wall(false, (x - t, x + t), z0 + t, z1 - z0 - 2.0 * t, y, h, &o, Paint::Steel);
        }
    }

    /// The roof: parapets, stair houses over the stairwells, plant, a skylight, a helipad.
    fn office_roof(&mut self, rng: &mut Rng, roof: f32) {
        for (p, q) in [((X0, Z0), (X1, Z0 + 0.3)), ((X0, Z1 - 0.3), (X1, Z1)), ((X0, Z0 + 0.3), (X0 + 0.3, Z1 - 0.3)), ((X1 - 0.3, Z0 + 0.3), (X1, Z1 - 0.3))] {
            self.solid(Aabb::span(Vec3::new(p.0, roof, p.1), Vec3::new(q.0, roof + COVER_HEIGHT, q.1)), Paint::Concrete);
        }
        self.decor(Aabb::span(Vec3::new(X0 - 0.05, roof + COVER_HEIGHT, Z0 - 0.05), Vec3::new(X1 + 0.05, roof + COVER_HEIGHT + 0.04, Z1 + 0.05)), Paint::Dark);
        // Stair houses: walls round the top of each stairwell, a door facing in, a roof.
        for (x0, south) in WELLS {
            let (wall, inward) = if south { (Z0 + T, 1.0) } else { (Z1 - T, -1.0) };
            let (za, zb) = (wall, wall + inward * 3.0);
            let (z0, z1) = (za.min(zb), za.max(zb));
            let (hx0, hx1) = (x0 - 0.2, x0 + 7.2);
            let front = if south { z1 } else { z0 };
            self.wall(true, (front - 0.1, front + 0.1), hx0, hx1 - hx0, roof, 2.8, &[(5.6, 7.1, 0.0, DOOR_HEIGHT)], Paint::Bone);
            for x in [hx0, hx1] {
                self.solid(Aabb::span(Vec3::new(x - 0.1, roof, z0), Vec3::new(x + 0.1, roof + 2.8, z1)), Paint::Bone);
            }
            self.solid(Aabb::span(Vec3::new(hx0 - 0.1, roof + 2.8, z0 - 0.1), Vec3::new(hx1 + 0.1, roof + 3.05, z1 + 0.1)), Paint::Dark);
        }
        // Air handling units: cover.
        for (x, z) in [(-20.0, -10.0), (-20.0, 10.0), (20.0, -10.0), (20.0, 10.0), (-32.0, 0.0), (32.0, 0.0)] {
            self.solid(Aabb::span(Vec3::new(x - 1.6, roof, z - 1.0), Vec3::new(x + 1.6, roof + 1.5, z + 1.0)), Paint::Metal);
            self.decor(Aabb::span(Vec3::new(x - 0.7, roof + 1.5, z - 0.7), Vec3::new(x + 0.7, roof + 1.53, z + 0.7)), Paint::Dark);
        }
        // The lift motor rooms (the shafts come up 1.4 m) and the skylight over the atrium.
        let (a0, a1, b0, b1) = ATRIUM;
        self.solid(Aabb::span(Vec3::new(a0 + 1.0, roof, b0 + 1.0), Vec3::new(a1 - 1.0, roof + 0.5, b1 - 1.0)), Paint::Steel);
        self.decor(Aabb::span(Vec3::new(a0 + 1.2, roof + 0.5, b0 + 1.2), Vec3::new(a1 - 1.2, roof + 0.53, b1 - 1.2)), Paint::Aether);
        // A helipad, and a mast (thin).
        let pad = Vec2::new(0.0, -16.0);
        self.flat(pad.x - 5.0, pad.x + 5.0, pad.y - 5.0, pad.y + 5.0, roof + 0.005, Paint::Dark);
        self.flat(pad.x - 1.8, pad.x - 1.2, pad.y - 2.0, pad.y + 2.0, roof + 0.01, Paint::Gold);
        self.flat(pad.x + 1.2, pad.x + 1.8, pad.y - 2.0, pad.y + 2.0, roof + 0.01, Paint::Gold);
        self.flat(pad.x - 1.2, pad.x + 1.2, pad.y - 0.3, pad.y + 0.3, roof + 0.01, Paint::Gold);
        self.pole(Vec3::new(30.0, roof, 18.0), 6.0, Paint::Metal);
        self.decor(Aabb::new(Vec3::new(30.0, roof + 6.1, 18.0), Vec3::new(0.1, 0.1, 0.1)), Paint::Red);
        let _ = rng;
    }

    /// Outside: the car park, the fountain plaza, lawns, trees and hedges.
    fn office_grounds(&mut self, rng: &mut Rng, g: f32) {
        // Paths round the building.
        let y = g - 0.11;
        self.flat(X0 - 4.0, X1 + 4.0, Z0 - 4.0, Z1 + 4.0, y, Paint::Concrete);
        self.flat(-56.0, 56.0, -2.5, 2.5, y + 0.002, Paint::Concrete);
        // The car park.
        self.flat(-34.0, 34.0, -48.0, -30.0, y + 0.004, Paint::Dark);
        for i in 0..14 {
            let x = -31.0 + i as f32 * 4.8;
            for (z0, z1) in [(-47.5, -42.5), (-35.5, -30.5)] {
                self.flat(x - 0.06, x + 0.06, z0, z1, y + 0.008, Paint::Bone);
            }
            for z in [-45.0, -33.0] {
                if rng.chance(0.45) {
                    let p = Vec2::new(x + 2.4, z);
                    self.clutter.push(Clutter { kind: ClutterKind::Car, pos: Vec3::new(p.x, self.height(p.x, p.y), p.y), turned: false });
                }
            }
        }
        for x in [-24.0, 0.0, 24.0] {
            self.boxed(Vec2::new(x, -39.0), Vec2::new(3.0, 0.8), 0.55, Paint::Concrete);
            self.props.push(Prop { kind: PropKind::Tree, pos: Vec3::new(x, self.height(x, -39.0) + 0.55, -39.0), yaw: rng.range(0.0, TAU), scale: 3.8 });
            self.props.push(Prop { kind: PropKind::Lamp, pos: Vec3::new(x + 12.0, self.height(x + 12.0, -39.0), -39.0), yaw: 0.0, scale: 1.0 });
        }
        // The plaza to the north.
        self.plaza_at(rng, Vec2::new(0.0, 40.0));
        for s in [-1.0f32, 1.0] {
            self.low_wall(Vec2::new(s * 14.0, 30.0), Vec2::new(s * 14.0, 50.0), COVER_HEIGHT, 0.6, Paint::Bone);
            self.clutter.push(Clutter { kind: ClutterKind::Bench, pos: Vec3::new(s * 9.0, self.height(s * 9.0, 32.0), 32.0), turned: false });
            self.clutter.push(Clutter { kind: ClutterKind::Bench, pos: Vec3::new(s * 9.0, self.height(s * 9.0, 48.0), 48.0), turned: false });
        }
        // Planters and hedges by the side doors and in front of the bases.
        for s in [-1.0f32, 1.0] {
            for z in [-14.0, 14.0] {
                self.boxed(Vec2::new(s * 46.0, z), Vec2::new(1.0, 3.0), COVER_HEIGHT, Paint::Concrete);
            }
            self.hedge(Vec2::new(s * 44.0, -30.0), Vec2::new(s * 44.0, -20.0), rng);
            self.hedge(Vec2::new(s * 44.0, 20.0), Vec2::new(s * 44.0, 30.0), rng);
            for (x, z) in [(s * 50.0, -26.0), (s * 50.0, 26.0), (s * 40.0, -10.0), (s * 40.0, 10.0)] {
                self.props.push(Prop { kind: PropKind::Crate, pos: Vec3::new(x, self.height(x, z), z), yaw: 0.0, scale: 1.0 });
            }
        }
        for _ in 0..60 {
            let p = Vec2::new(rng.range(-80.0, 80.0), rng.range(-78.0, 78.0));
            if self.occupied(p, 2.5) || (p.x.abs() < 44.0 && p.y.abs() < 52.0) || (p.x.abs() > BASE_X - 10.0 && p.y.abs() < 20.0) {
                continue;
            }
            self.props.push(Prop { kind: PropKind::Tree, pos: Vec3::new(p.x, self.height(p.x, p.y), p.y), yaw: rng.range(0.0, TAU), scale: rng.range(3.6, 5.4) });
        }
        for _ in 0..3200 {
            let p = Vec2::new(rng.range(-84.0, 84.0), rng.range(-84.0, 84.0));
            let h = self.height(p.x, p.y);
            if h < self.water + 0.5 || (p.x.abs() < X1 + 5.0 && p.y.abs() < Z1 + 5.0) || (p.x.abs() < 35.0 && p.y < -29.0 && p.y > -49.0) || self.occupied(p, 0.3) {
                continue;
            }
            self.grass.push(Vec3::new(p.x, h, p.y));
        }
    }
}
