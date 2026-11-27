//! Space Station: a fixed indoor map, fully enclosed. A hangar (west) and a cargo bay (east)
//! hold the team bases; from each, three corridors - an airlock in the middle, two service
//! passages - lead to a ring corridor round the core. Inside the ring: labs, crew quarters,
//! hydroponics, a comms room, a med bay, the reactor and a store, and in the middle a two-level
//! hub with a balcony along either side and a holo-table.
//!
//! Every room is walled and roofed, so there is no way out to the "outside". The sky is never
//! seen; viewports on the hull walls are painted starfields.

use super::{COVER_HEIGHT, Clutter, ClutterKind, DOOR_HEIGHT, DOOR_WIDTH, Prop, PropKind, STOREY, WINDOW_SILL, World, shapes::rects_minus};
use crate::{collide::Aabb, model::Paint, rng::Rng};
use glam::{Vec2, Vec3};
use std::f32::consts::TAU;

/// Height of corridors and rooms, the hub and the bays.
const ROOM: f32 = 3.6;
const HUB: f32 = 8.0;
const BAY: f32 = 9.0;
const WALL: f32 = 0.4;
/// Ring corridor: outer and inner half-sizes.
const RING_OUT: (f32, f32) = (36.0, 30.0);
const RING_IN: (f32, f32) = (31.0, 25.0);
const HUB_HALF: (f32, f32) = (13.0, 11.0);
/// Bays: from |x| = 58 to 88, |z| < 22.
const BAY_X: (f32, f32) = (58.0, 88.0);
const BAY_Z: f32 = 22.0;
/// The service passages' centre lines.
const SIDE_Z: f32 = 16.0;

/// An opening in a wall line: centre (world coordinate along the line), width, and for windows
/// the sill and top.
#[derive(Clone, Copy)]
struct Gap {
    at: f32,
    width: f32,
    window: Option<(f32, f32)>,
}

fn door(at: f32) -> Gap {
    Gap { at, width: DOOR_WIDTH, window: None }
}

fn arch(at: f32, width: f32) -> Gap {
    Gap { at, width, window: None }
}

fn pane(at: f32, width: f32) -> Gap {
    Gap { at, width, window: Some((WINDOW_SILL, 2.5)) }
}

impl World {
    /// A straight wall from `a` to `b` (one axis), `h` tall from the deck, with gaps.
    fn line(&mut self, a: Vec2, b: Vec2, y: f32, h: f32, gaps: &[Gap], paint: Paint) {
        let along_x = (b.x - a.x).abs() > (b.y - a.y).abs();
        let (start, end) = if along_x { (a.x.min(b.x), a.x.max(b.x)) } else { (a.y.min(b.y), a.y.max(b.y)) };
        let across = if along_x { a.y } else { a.x };
        let openings: Vec<(f32, f32, f32, f32)> = gaps
            .iter()
            .map(|g| {
                let (y0, y1) = g.window.unwrap_or((0.0, if g.width > DOOR_WIDTH + 0.05 { 2.9 } else { DOOR_HEIGHT }));
                (g.at - g.width * 0.5 - start, g.at + g.width * 0.5 - start, y0, y1)
            })
            .collect();
        self.wall(along_x, (across - WALL * 0.5, across + WALL * 0.5), start, end - start, y, h, &openings, paint);
        // A glowing strip along the top of each face.
        let lo = if along_x { Vec3::new(start, y + h - 0.35, across - WALL * 0.5 - 0.03) } else { Vec3::new(across - WALL * 0.5 - 0.03, y + h - 0.35, start) };
        let hi = if along_x { Vec3::new(end, y + h - 0.3, across + WALL * 0.5 + 0.03) } else { Vec3::new(across + WALL * 0.5 + 0.03, y + h - 0.3, end) };
        self.decor(Aabb::span(lo, hi), Paint::Aether);
    }

    pub(super) fn space_station(&mut self, rng: &mut Rng) {
        let g = self.highest(Vec2::ZERO, Vec2::new(BAY_X.1, RING_OUT.1)) + 0.12;
        // The deck: one slab under everything.
        self.solid(Aabb::span(Vec3::new(-BAY_X.1, g - 1.5, -RING_OUT.1), Vec3::new(BAY_X.1, g, RING_OUT.1)), Paint::Steel);
        self.deck_plating(g);
        self.station_walls(g);
        self.station_ceilings(g);
        self.hub(rng, g);
        self.station_rooms(rng, g);
        self.ring_details(rng, g);
        for s in [-1.0f32, 1.0] {
            self.bay(rng, g, s);
            self.connectors(rng, g, s);
        }
        self.team_bases(78.0, 0.0, Paint::Steel);
        // Everywhere inside: the ring, the rooms, the hub and its balconies, the corridors, the bays.
        let (ox, oz) = RING_OUT;
        self.waypoints_over(g, (-ox + 0.5, ox - 0.5, -oz + 0.5, oz - 0.5), 5.0);
        self.waypoints_over(g + STOREY, (-HUB_HALF.0, HUB_HALF.0, -HUB_HALF.1, HUB_HALF.1), 4.0);
        for s in [-1.0f32, 1.0] {
            let (a, b) = (s * ox, s * BAY_X.0);
            for z in [0.0, SIDE_Z, -SIDE_Z] {
                self.waypoints_over(g, (a.min(b), a.max(b), z - 1.0, z + 1.0), 4.0);
            }
            let (a, b) = (s * BAY_X.0, s * BAY_X.1);
            self.waypoints_over(g, (a.min(b), a.max(b), -BAY_Z, BAY_Z), 7.0);
        }
    }

    /// Floor plating: panels with dark seams, warning stripes at the airlocks.
    fn deck_plating(&mut self, g: f32) {
        let y = g + 0.004;
        self.flat(-RING_OUT.0 + WALL, RING_OUT.0 - WALL, -RING_OUT.1 + WALL, RING_OUT.1 - WALL, y, Paint::Metal);
        for s in [-1.0f32, 1.0] {
            let (a, b) = (s * BAY_X.0, s * BAY_X.1);
            self.flat(a.min(b), a.max(b), -BAY_Z, BAY_Z, y, Paint::Steel);
            let (a, b) = (s * RING_OUT.0, s * BAY_X.0);
            for (z0, z1) in [(-2.0, 2.0), (SIDE_Z - 1.5, SIDE_Z + 1.5), (-SIDE_Z - 1.5, -SIDE_Z + 1.5)] {
                self.flat(a.min(b), a.max(b), z0, z1, y, Paint::Metal);
            }
        }
        // Seams every four metres.
        let mut x = -BAY_X.1 + 4.0;
        while x < BAY_X.1 {
            self.flat(x - 0.03, x + 0.03, -RING_OUT.1 + WALL, RING_OUT.1 - WALL, y + 0.003, Paint::Dark);
            x += 4.0;
        }
        let mut z = -RING_OUT.1 + 4.0;
        while z < RING_OUT.1 {
            self.flat(-BAY_X.1, BAY_X.1, z - 0.03, z + 0.03, y + 0.003, Paint::Dark);
            z += 4.0;
        }
    }

    fn station_walls(&mut self, g: f32) {
        let (ox, oz) = RING_OUT;
        let (ix, iz) = RING_IN;
        let (hx, hz) = HUB_HALF;
        let hull = Paint::Steel;
        let inner = Paint::Metal;
        for s in [-1.0f32, 1.0] {
            // The ring's outer wall: gaps for the three corridors on the ends.
            self.line(Vec2::new(-ox, s * oz), Vec2::new(ox, s * oz), g, ROOM, &[], hull);
            self.line(Vec2::new(s * ox, -oz), Vec2::new(s * ox, oz), g, ROOM, &[arch(0.0, 3.6), arch(SIDE_Z, 3.0), arch(-SIDE_Z, 3.0)], hull);
            // The ring's inner wall: doors into the rooms, arches into the spokes.
            self.line(Vec2::new(-ix, s * iz), Vec2::new(ix, s * iz), g, ROOM, &[door(-18.0), arch(0.0, 3.4), door(18.0), pane(-9.0, 2.4), pane(9.0, 2.4)], inner);
            self.line(Vec2::new(s * ix, -iz), Vec2::new(s * ix, iz), g, ROOM, &[arch(0.0, 3.4), door(-6.5), door(6.5), door(-18.0), door(18.0)], inner);
            // The hub, two storeys tall.
            self.line(Vec2::new(-hx, s * hz), Vec2::new(hx, s * hz), g, HUB, &[arch(0.0, 3.4), pane(-7.0, 2.0), pane(7.0, 2.0)], hull);
            self.line(Vec2::new(s * hx, -hz), Vec2::new(s * hx, hz), g, HUB, &[arch(0.0, 3.4)], hull);
            // Room dividers between the hub's sides and the ring.
            for side in [-1.0f32, 1.0] {
                let (a, b) = (side * hx, side * ix);
                self.line(Vec2::new(a.min(b), s * hz), Vec2::new(a.max(b), s * hz), g, ROOM, &[door(side * 22.0)], inner);
                // Spoke walls: north and south spokes run along Z, east and west along X.
                let (c, d) = (s * hz, s * iz);
                self.line(Vec2::new(side * 2.0, c.min(d)), Vec2::new(side * 2.0, c.max(d)), g, ROOM, &[door(s * 18.0)], inner);
                let (e, f) = (s * hx, s * ix);
                self.line(Vec2::new(e.min(f), side * 2.0), Vec2::new(e.max(f), side * 2.0), g, ROOM, &[door(s * 22.0)], inner);
            }
            // The bays: hull all round, the corridors coming in on the inner side.
            let (bx0, bx1) = (s * BAY_X.0, s * BAY_X.1);
            self.line(Vec2::new(bx0, -BAY_Z), Vec2::new(bx0, BAY_Z), g, BAY, &[arch(0.0, 3.6), arch(SIDE_Z, 3.0), arch(-SIDE_Z, 3.0)], hull);
            self.line(Vec2::new(bx1, -BAY_Z), Vec2::new(bx1, BAY_Z), g, BAY, &[], hull);
            for side in [-1.0f32, 1.0] {
                self.line(Vec2::new(bx0.min(bx1), side * BAY_Z), Vec2::new(bx0.max(bx1), side * BAY_Z), g, BAY, &[], hull);
            }
            // The corridors between ring and bay: the airlock (two doors) and the service passages.
            let (cx0, cx1) = ((s * ox).min(s * BAY_X.0), (s * ox).max(s * BAY_X.0));
            for side in [-1.0f32, 1.0] {
                self.line(Vec2::new(cx0, side * 2.0), Vec2::new(cx1, side * 2.0), g, ROOM, &[], hull);
                for edge in [SIDE_Z - 1.5, SIDE_Z + 1.5] {
                    self.line(Vec2::new(cx0, side * edge), Vec2::new(cx1, side * edge), g, ROOM, &[], hull);
                }
            }
            for x in [s * 44.0, s * 50.0] {
                self.line(Vec2::new(x, -1.8), Vec2::new(x, 1.8), g, ROOM, &[door(0.0)], Paint::Gold);
            }
        }
    }

    /// Roofs over everything: the rooms and ring, the hub (higher), corridors, bays.
    fn station_ceilings(&mut self, g: f32) {
        let (ox, oz) = RING_OUT;
        let (hx, hz) = HUB_HALF;
        let slab = |world: &mut World, (x0, x1, z0, z1): (f32, f32, f32, f32), top: f32| {
            world.solid(Aabb::span(Vec3::new(x0, top, z0), Vec3::new(x1, top + 0.3, z1)), Paint::Dark);
            // Light panels underneath.
            let mut x = x0 + 2.5;
            while x < x1 - 1.0 {
                let mut z = z0 + 2.5;
                while z < z1 - 1.0 {
                    world.decor(Aabb::new(Vec3::new(x, top - 0.02, z), Vec3::new(0.7, 0.02, 0.25)), Paint::White);
                    z += 6.0;
                }
                x += 6.0;
            }
        };
        for r in rects_minus(-ox - 0.2, ox + 0.2, -oz - 0.2, oz + 0.2, &[(-hx + 0.2, hx - 0.2, -hz + 0.2, hz - 0.2)]) {
            slab(self, r, g + ROOM);
        }
        slab(self, (-hx - 0.2, hx + 0.2, -hz - 0.2, hz + 0.2), g + HUB);
        for s in [-1.0f32, 1.0] {
            let (a, b) = (s * ox, s * BAY_X.0);
            for (z0, z1) in [(-2.2, 2.2), (SIDE_Z - 1.7, SIDE_Z + 1.7), (-SIDE_Z - 1.7, -SIDE_Z + 1.7)] {
                slab(self, (a.min(b), a.max(b), z0, z1), g + ROOM);
            }
            let (a, b) = (s * (BAY_X.0 - 0.2), s * (BAY_X.1 + 0.2));
            slab(self, (a.min(b), a.max(b), -BAY_Z - 0.2, BAY_Z + 0.2), g + BAY);
        }
    }

    /// The hub: balconies along the north and south walls with stairs up, a holo-table, consoles.
    fn hub(&mut self, rng: &mut Rng, g: f32) {
        let (hx, hz) = HUB_HALF;
        let deck = g + STOREY;
        let rise = STOREY / 11.0;
        for s in [-1.0f32, 1.0] {
            // The balcony: 3.3 m deep along the wall, a crouch-high balustrade, a gap at the stairs.
            let (z0, z1) = ((s * (hz - WALL * 0.5)).min(s * (hz - 3.5)), (s * (hz - WALL * 0.5)).max(s * (hz - 3.5)));
            self.solid(Aabb::span(Vec3::new(-hx + WALL * 0.5, deck - 0.25, z0), Vec3::new(hx - WALL * 0.5, deck, z1)), Paint::Steel);
            self.flat(-hx + WALL * 0.5, hx - WALL * 0.5, z0, z1, deck, Paint::Metal);
            let edge = s * (hz - 3.5);
            // Stairs climb along the balcony edge, west to east on the north, east to west on the south.
            let start = -s * 9.5;
            let lane = (edge - s * 1.25, edge - s * 0.05);
            for i in 0..11 {
                let top = g + (i + 1) as f32 * rise;
                let (a, b) = (start + s * i as f32 * 0.42, start + s * (i + 1) as f32 * 0.42);
                self.solid(Aabb::span(Vec3::new(a.min(b), g, lane.0.min(lane.1)), Vec3::new(a.max(b), top, lane.0.max(lane.1))), if i % 2 == 0 { Paint::Steel } else { Paint::Dark });
            }
            let top_x = start + s * 11.0 * 0.42;
            // Balustrade: from the far wall to the stairs' top, and on past it.
            let rail = |world: &mut World, a: f32, b: f32| {
                world.solid(Aabb::span(Vec3::new(a.min(b), deck, edge - 0.06), Vec3::new(a.max(b), deck + COVER_HEIGHT, edge + 0.06)), Paint::Steel);
                world.decor(Aabb::span(Vec3::new(a.min(b), deck + COVER_HEIGHT, edge - 0.08), Vec3::new(a.max(b), deck + COVER_HEIGHT + 0.04, edge + 0.08)), Paint::Aether);
            };
            rail(self, -s * (hx - WALL * 0.5), top_x - s * 1.2);
            rail(self, top_x + s * 0.4, s * (hx - WALL * 0.5));
            // Under the balcony: lockers against the wall; on it: a console row.
            for x in [-9.0, -5.0, 5.0, 9.0] {
                let z = s * (hz - 0.6);
                if (x * s) < 0.0 && (x - start).abs() < 2.0 {
                    continue;
                }
                self.solid(Aabb::span(Vec3::new(x - 1.2, g, z - 0.35), Vec3::new(x + 1.2, g + 2.0, z + 0.35)), Paint::Teal);
                self.console(Vec2::new(x, s * (hz - 1.2)), Vec2::new(0.0, -s), deck);
            }
        }
        // The holo-table: a round-ish low pedestal (cover), a glowing map floating above.
        self.column(Vec2::ZERO, 2.0, g, g + 1.0, Paint::Dark);
        self.decor(Aabb::new(Vec3::new(0.0, g + 1.02, 0.0), Vec3::new(1.7, 0.02, 0.7)), Paint::Aether);
        self.decor(Aabb::new(Vec3::new(0.0, g + 1.02, 0.0), Vec3::new(0.7, 0.02, 1.7)), Paint::Aether);
        for (h, r) in [(2.0, 1.2), (2.6, 0.8), (3.1, 0.4)] {
            self.decor(Aabb::new(Vec3::new(0.0, g + h, 0.0), Vec3::new(r, 0.02, r)), Paint::Blue);
        }
        for (x, z) in [(-6.0, -3.0), (6.0, 3.0), (-6.0, 3.0), (6.0, -3.0)] {
            self.console(Vec2::new(x, z), Vec2::new(-x.signum(), 0.0), g);
        }
        for _ in 0..4 {
            let p = Vec2::new(rng.range(-10.0, 10.0), rng.range(-5.5, 5.5));
            if !self.occupied(p, 1.4) && p.length() > 3.5 {
                self.props.push(Prop { kind: PropKind::Crate, pos: Vec3::new(p.x, g, p.y), yaw: 0.0, scale: 1.0 });
            }
        }
    }

    /// A console: a waist-high desk with a lit screen on top, facing `face`.
    pub(crate) fn console(&mut self, p: Vec2, face: Vec2, y: f32) {
        let along_x = face.y.abs() > 0.5;
        let half = if along_x { Vec2::new(0.9, 0.4) } else { Vec2::new(0.4, 0.9) };
        self.solid(Aabb::span(Vec3::new(p.x - half.x, y, p.y - half.y), Vec3::new(p.x + half.x, y + 1.0, p.y + half.y)), Paint::Steel);
        let back = p - face * 0.25;
        let screen = if along_x { Vec3::new(0.7, 0.3, 0.03) } else { Vec3::new(0.03, 0.3, 0.7) };
        self.decor(Aabb::new(Vec3::new(back.x, y + 1.35, back.y), screen), Paint::Screen);
        let glow = if along_x { Vec3::new(0.6, 0.22, 0.01) } else { Vec3::new(0.01, 0.22, 0.6) };
        let front = back + face * 0.035;
        self.decor(Aabb::new(Vec3::new(front.x, y + 1.35, front.y), glow), Paint::Blue);
        let keys = if along_x { Vec3::new(0.6, 0.015, 0.15) } else { Vec3::new(0.15, 0.015, 0.6) };
        let k = p + face * 0.15;
        self.decor(Aabb::new(Vec3::new(k.x, y + 1.015, k.y), keys), Paint::Green);
    }

    /// The rooms between the hub and the ring.
    fn station_rooms(&mut self, rng: &mut Rng, g: f32) {
        let (ix, _) = RING_IN;
        let (hx, hz) = HUB_HALF;
        // Labs (north): benches, specimen tanks, consoles.
        for s in [-1.0f32, 1.0] {
            let cx = s * (ix + 2.0) * 0.5;
            for k in 0..3 {
                let x = cx + (k as f32 - 1.0) * 8.0;
                if (x - s * 22.0).abs() < 1.6 || (x - s * 2.0).abs() < 2.0 {
                    continue;
                }
                self.solid(Aabb::span(Vec3::new(x - 2.2, g, 16.0), Vec3::new(x + 2.2, g + 0.95, 17.2)), Paint::Bone);
                for j in 0..3 {
                    let p = Vec2::new(x - 1.4 + j as f32 * 1.4, 16.6);
                    self.solid(Aabb::span(Vec3::new(p.x - 0.18, g + 0.95, p.y - 0.18), Vec3::new(p.x + 0.18, g + 1.35, p.y + 0.18)), if j == 1 { Paint::Aether } else { Paint::Steel });
                }
            }
            for x in [s * 8.0, s * 28.0] {
                self.column(Vec2::new(x, 22.5), 0.8, g, g + 2.4, Paint::Teal);
                self.decor(Aabb::new(Vec3::new(x, g + 1.3, 22.5), Vec3::new(0.83, 0.03, 0.37)), Paint::Aether);
            }
            self.console(Vec2::new(s * 16.0, 23.6), Vec2::NEG_Y, g);
        }
        // Crew quarters (south-west): bunks and lockers.
        for x in [-28.5f32, -24.0, -12.5, -7.5] {
            self.bunk(Vec2::new(x, -23.4), g);
            self.solid(Aabb::span(Vec3::new(x - 0.5, g, -13.0), Vec3::new(x + 0.5, g + 2.1, -11.6)), Paint::Steel);
        }
        self.solid(Aabb::span(Vec3::new(-16.0, g, -19.0), Vec3::new(-10.0, g + 0.76, -17.6)), Paint::Wood);
        // Hydroponics (south-east): planter rows with leaves on, grow lights.
        for k in 0..3 {
            let z = -22.5 + k as f32 * 3.6;
            for x0 in [5.0, 23.0] {
                let x1 = x0 + if x0 < 10.0 { 11.0 } else { 6.0 };
                self.solid(Aabb::span(Vec3::new(x0, g, z - 0.6), Vec3::new(x1, g + 0.8, z + 0.6)), Paint::Wood);
                let mut x = x0 + 0.8;
                while x < x1 - 0.5 {
                    self.props.push(Prop { kind: PropKind::Bush, pos: Vec3::new(x, g + 0.75, z), yaw: rng.range(0.0, TAU), scale: rng.range(0.6, 0.8) });
                    x += 1.6;
                }
                self.decor(Aabb::span(Vec3::new(x0, g + ROOM - 0.45, z - 0.3), Vec3::new(x1, g + ROOM - 0.42, z + 0.3)), Paint::Violet);
                // Hung from the ceiling on two rods.
                for x in [x0 + 0.4, x1 - 0.4] {
                    self.decor(Aabb::span(Vec3::new(x - 0.02, g + ROOM - 0.42, z - 0.02), Vec3::new(x + 0.02, g + ROOM, z + 0.02)), Paint::Metal);
                }
            }
        }
        // West: comms (north) and the med bay (south).
        self.console(Vec2::new(-ix + 1.0, 6.5 + 2.5), Vec2::X, g);
        self.console(Vec2::new(-hx - 1.0, 7.0), Vec2::NEG_X, g);
        self.decor(Aabb::span(Vec3::new(-hx - 0.24, g + 1.2, 4.0), Vec3::new(-hx - 0.2, g + 2.8, 9.5)), Paint::Screen);
        self.decor(Aabb::span(Vec3::new(-hx - 0.25, g + 1.4, 4.4), Vec3::new(-hx - 0.24, g + 2.6, 9.1)), Paint::Blue);
        for z in [-9.0, -4.5] {
            self.solid(Aabb::span(Vec3::new(-28.0, g, z - 0.6), Vec3::new(-25.6, g + 0.7, z + 0.6)), Paint::Bone);
            self.decor(Aabb::span(Vec3::new(-27.9, g + 0.7, z - 0.5), Vec3::new(-25.7, g + 0.74, z + 0.5)), Paint::White);
        }
        self.solid(Aabb::span(Vec3::new(-18.0, g, -10.6), Vec3::new(-15.0, g + 1.8, -10.0)), Paint::Bone);
        // East: the reactor (south) and the store (north).
        let core = Vec2::new(22.0, -6.5);
        self.column(core, 2.4, g, g + ROOM, Paint::Dark);
        for h in [0.6, 1.4, 2.2, 3.0] {
            self.decor(Aabb::new(Vec3::new(core.x, g + h, core.y), Vec3::new(2.43, 0.04, 1.03)), Paint::Spark);
            self.decor(Aabb::new(Vec3::new(core.x, g + h, core.y), Vec3::new(1.03, 0.04, 2.43)), Paint::Spark);
        }
        for p in [Vec2::new(16.0, -9.6), Vec2::new(28.0, -3.4)] {
            self.clutter.push(Clutter { kind: ClutterKind::Barrel, pos: Vec3::new(p.x, g, p.y), turned: false });
        }
        self.console(Vec2::new(15.0, -4.0), Vec2::X, g);
        for (x, z) in [(16.5, 5.0), (16.5, 9.0), (27.5, 5.5), (27.5, 9.5), (19.5, 6.8)] {
            self.props.push(Prop { kind: PropKind::Crate, pos: Vec3::new(x, g, z), yaw: 0.0, scale: 1.0 });
            if rng.chance(0.4) {
                self.props.push(Prop { kind: PropKind::Crate, pos: Vec3::new(x, g + 1.1, z), yaw: 0.0, scale: 1.0 });
            }
        }
        let _ = hz;
    }

    /// A two-tier bunk along X: one solid block (nobody fits between the berths), the lower
    /// berth a dark recess painted on its face.
    fn bunk(&mut self, p: Vec2, g: f32) {
        self.solid(Aabb::span(Vec3::new(p.x - 1.0, g, p.y - 0.45), Vec3::new(p.x + 1.0, g + 1.75, p.y + 0.45)), Paint::Steel);
        self.decor(Aabb::span(Vec3::new(p.x - 0.9, g + 0.55, p.y + 0.45), Vec3::new(p.x + 0.9, g + 1.4, p.y + 0.47)), Paint::Dark);
        self.decor(Aabb::span(Vec3::new(p.x - 0.95, g + 1.75, p.y - 0.4), Vec3::new(p.x + 0.95, g + 1.8, p.y + 0.4)), Paint::Teal);
        for x in [p.x - 0.98, p.x + 0.98] {
            self.decor(Aabb::span(Vec3::new(x - 0.04, g, p.y - 0.04), Vec3::new(x + 0.04, g + 1.75, p.y + 0.04)), Paint::Dark);
        }
    }

    /// The ring: pipes along the ceiling, viewports onto painted stars, cover in the corridor.
    fn ring_details(&mut self, rng: &mut Rng, g: f32) {
        let (ox, oz) = RING_OUT;
        let (ix, iz) = RING_IN;
        let mid = ((ox + ix) * 0.5, (oz + iz) * 0.5);
        for s in [-1.0f32, 1.0] {
            // Pipes: thin.
            self.decor(Aabb::span(Vec3::new(-ox, g + ROOM - 0.5, s * (oz - 0.5) - 0.08), Vec3::new(ox, g + ROOM - 0.34, s * (oz - 0.5) + 0.08)), Paint::Copper);
            self.decor(Aabb::span(Vec3::new(s * (ox - 0.5) - 0.08, g + ROOM - 0.5, -oz), Vec3::new(s * (ox - 0.5) + 0.08, g + ROOM - 0.34, oz)), Paint::Copper);
            // Viewports on the outer wall: dark glass and stars.
            let mut x = -ox + 6.0;
            while x < ox - 4.0 {
                let face = s * (oz - WALL * 0.5) - s * 0.02;
                self.decor(Aabb::span(Vec3::new(x - 1.6, g + 1.1, face - 0.01), Vec3::new(x + 1.6, g + 2.6, face + 0.01)), Paint::Screen);
                for _ in 0..7 {
                    let (sx, sy) = (x + rng.range(-1.5, 1.5), g + rng.range(1.2, 2.5));
                    self.decor(Aabb::span(Vec3::new(sx - 0.03, sy - 0.03, face - s * 0.015 - 0.005), Vec3::new(sx + 0.03, sy + 0.03, face - s * 0.015 + 0.005)), Paint::White);
                }
                x += 12.0;
            }
            // Barricades and crates along the long sides of the ring, leaving a way past.
            let mut x = -ox + 8.0;
            let mut left = rng.chance(0.5);
            while x < ox - 6.0 {
                if (x.abs() - 18.0).abs() > 2.5 && x.abs() > 3.0 {
                    let z = if left { s * (iz + 0.9) } else { s * (oz - 0.9) };
                    self.solid(Aabb::span(Vec3::new(x - 1.3, g, z - 0.45), Vec3::new(x + 1.3, g + COVER_HEIGHT, z + 0.45)), Paint::Gold);
                    self.decor(Aabb::span(Vec3::new(x - 1.32, g + 0.5, z - 0.47), Vec3::new(x + 1.32, g + 0.7, z + 0.47)), Paint::Dark);
                    left = !left;
                }
                x += rng.range(9.0, 13.0);
            }
            // Down the short sides: crates by the walls.
            for z in [-21.0f32, -9.0, 9.0, 21.0] {
                let x = s * if rng.chance(0.5) { ix + 0.9 } else { ox - 0.9 };
                if (z.abs() - SIDE_Z).abs() > 2.2 && (z.abs() - 18.0).abs() > 1.6 {
                    self.props.push(Prop { kind: PropKind::Crate, pos: Vec3::new(x, g, z), yaw: 0.0, scale: 1.0 });
                }
            }
        }
        let _ = mid;
    }

    /// A bay at the `s` end: the team base. A shuttle on the deck, cargo, fuel tanks, a catwalk
    /// along the back wall with stairs up and windows looking down from the control room.
    fn bay(&mut self, rng: &mut Rng, g: f32, s: f32) {
        let (inner, outer) = (s * BAY_X.0, s * BAY_X.1);
        // The shuttle: a long body, a stepped nose, stub wings, all solid.
        let ship = Vec2::new(s * 76.0, if s > 0.0 { 17.5 } else { -17.5 });
        self.solid(Aabb::span(Vec3::new(ship.x - 5.0, g + 0.6, ship.y - 1.6), Vec3::new(ship.x + 5.0, g + 3.2, ship.y + 1.6)), Paint::Bone);
        self.solid(Aabb::span(Vec3::new(ship.x - 3.6, g, ship.y - 1.0), Vec3::new(ship.x + 3.6, g + 0.6, ship.y + 1.0)), Paint::Dark);
        let nose = ship.x - s * 5.0;
        self.solid(Aabb::span(Vec3::new(nose.min(nose - s * 1.4), g + 1.0, ship.y - 1.1), Vec3::new(nose.max(nose - s * 1.4), g + 2.8, ship.y + 1.1)), Paint::Bone);
        self.decor(Aabb::span(Vec3::new(nose.min(nose - s * 1.42), g + 2.0, ship.y - 0.8), Vec3::new(nose.max(nose - s * 1.42), g + 2.6, ship.y + 0.8)), Paint::Screen);
        for side in [-1.0, 1.0] {
            let z0 = ship.y + side * 1.6;
            let z1 = ship.y + side * 4.0;
            self.solid(Aabb::span(Vec3::new(ship.x - 1.5, g + 1.0, z0.min(z1)), Vec3::new(ship.x + 2.5, g + 1.4, z0.max(z1))), Paint::Steel);
        }
        self.decor(Aabb::span(Vec3::new(ship.x + s * 5.0 - 0.02, g + 1.0, ship.y - 1.0), Vec3::new(ship.x + s * 5.0 + 0.02, g + 2.8, ship.y + 1.0)), Paint::Spark);
        // Cargo: containers and crates, fuel tanks.
        let cargo_z = -ship.y;
        let levels = if rng.chance(0.5) { 2 } else { 1 };
        self.container(rng, Vec2::new(s * 79.0, cargo_z), true, levels);
        self.container(rng, Vec2::new(s * 67.0, cargo_z * 1.1), true, 1);
        for z in [-6.0f32, 6.0] {
            self.column(Vec2::new(s * 64.0, z), 1.1, g, g + 2.6, Paint::Coral);
        }
        for (dx, dz) in [(-2.0, -1.0), (2.5, 1.5)] {
            self.props.push(Prop { kind: PropKind::Crate, pos: Vec3::new(s * 70.0 + dx, g, cargo_z * 0.6 + dz), yaw: 0.0, scale: 1.0 });
        }
        self.clutter.push(Clutter { kind: ClutterKind::Barrel, pos: Vec3::new(s * 61.0, g, -cargo_z * 0.2), turned: false });
        // The catwalk along the back wall, a stair flight up to it, a rail along its edge.
        let deck = g + STOREY;
        let back = outer - s * WALL * 0.5;
        let front = outer - s * 3.0;
        self.solid(Aabb::span(Vec3::new(back.min(front), deck - 0.25, -BAY_Z + 0.2), Vec3::new(back.max(front), deck, BAY_Z - 0.2)), Paint::Iron);
        let rise = STOREY / 11.0;
        let lane = (front - s * 1.2, front - s * 0.05);
        for i in 0..11 {
            let top = g + (i + 1) as f32 * rise;
            let (a, b) = (-20.0 + i as f32 * 0.42, -20.0 + (i + 1) as f32 * 0.42);
            self.solid(Aabb::span(Vec3::new(lane.0.min(lane.1), g, a), Vec3::new(lane.0.max(lane.1), top, b)), Paint::Iron);
        }
        // Rail along the catwalk's edge, open where the stairs come up.
        let top_z = -20.0 + 11.0 * 0.42;
        self.solid(Aabb::span(Vec3::new(front - 0.05, deck, top_z + 1.0), Vec3::new(front + 0.05, deck + COVER_HEIGHT, BAY_Z - 0.2)), Paint::Iron);
        // The bay door on the hull: stripes, paint-thin.
        let face = outer - s * (WALL * 0.5 + 0.02);
        for k in 0..6 {
            let z = -9.0 + k as f32 * 3.6;
            self.decor(Aabb::span(Vec3::new(face - 0.01, g + 0.3, z - 0.8), Vec3::new(face + 0.01, g + 3.0, z + 0.8)), if k % 2 == 0 { Paint::Gold } else { Paint::Dark });
        }
        // Big lights high on the walls.
        for z in [-BAY_Z + 0.25, BAY_Z - 0.25] {
            for x in [s * 64.0, s * 74.0, s * 84.0] {
                self.decor(Aabb::span(Vec3::new(x - 1.0, g + BAY - 1.0, z - 0.03), Vec3::new(x + 1.0, g + BAY - 0.7, z + 0.03)), Paint::White);
            }
        }
        let _ = inner;
    }

    /// The corridors from the ring to a bay: lights, warning stripes at the airlock.
    fn connectors(&mut self, rng: &mut Rng, g: f32, s: f32) {
        for x in [s * 44.0, s * 50.0] {
            for side in [-1.0f32, 1.0] {
                self.decor(Aabb::span(Vec3::new(x - 0.6, g + 0.006, side * 1.0 - 0.6), Vec3::new(x + 0.6, g + 0.02, side * 1.0 + 0.6)), if side > 0.0 { Paint::Gold } else { Paint::Dark });
            }
            self.decor(Aabb::span(Vec3::new(x - 0.25, g + 2.6, -1.78), Vec3::new(x + 0.25, g + 2.8, -1.76)), Paint::Red);
        }
        // A barricade in each service passage, against alternate walls.
        for (k, z) in [SIDE_Z, -SIDE_Z].into_iter().enumerate() {
            let x = s * rng.range(42.0, 52.0);
            let edge = if k == 0 { z - 0.9 } else { z + 0.9 };
            self.solid(Aabb::span(Vec3::new(x - 1.0, g, edge - 0.4), Vec3::new(x + 1.0, g + COVER_HEIGHT, edge + 0.4)), Paint::Gold);
        }
    }
}
