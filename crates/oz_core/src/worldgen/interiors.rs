//! Interiors: floors that read apart from the walls, and rooms furnished by what they are for -
//! living rooms, kitchens, bedrooms, bathrooms, offices, shops and stores.
//!
//! The rules from `shapes` hold: what you could hide behind (a sofa, a counter, a bed, a
//! wardrobe, shelving) is a solid block, marked as furniture so [`World::clear_doorways`] can take
//! it out of a doorway's way; everything small on or around it - cushions, books, a lamp, the
//! tap, the rug - is dressing laid on that furniture, and goes with it. Rugs are flat decals with
//! no collision. Furniture hugs the walls and leaves the middle of a room open, so every room
//! stays a room you can cross.

use super::{Block, World};
use crate::{collide::Aabb, model::{Paint, Shape}, rng::Rng};
use glam::{Quat, Vec2, Vec3};

/// The least [`Paint::contrast`] between a room's floor and its walls.
pub(crate) const FLOOR_CONTRAST: f32 = 85.0;

/// What a room is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RoomKind {
    Living,
    Kitchen,
    Bedroom,
    Bathroom,
    Office,
    Shop,
    Store,
}

/// A room's clear floor between its walls' inner faces, at floor level `y`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Room {
    pub x0: f32,
    pub x1: f32,
    pub z0: f32,
    pub z1: f32,
    pub y: f32,
}

impl Room {
    fn w(&self) -> f32 {
        self.x1 - self.x0
    }
    fn d(&self) -> f32 {
        self.z1 - self.z0
    }
    fn centre(&self) -> Vec2 {
        Vec2::new((self.x0 + self.x1) * 0.5, (self.z0 + self.z1) * 0.5)
    }
    /// The length of wall `side` (south, west, east, north).
    fn wall_len(&self, side: usize) -> f32 {
        if side == 0 || side == 3 { self.w() } else { self.d() }
    }
    /// A spot `t` metres along wall `side` (from its west or south end) and `depth` metres
    /// out from it, with the direction into the room.
    fn at_wall(&self, side: usize, t: f32, depth: f32) -> (Vec2, Vec2) {
        match side {
            0 => (Vec2::new(self.x0 + t, self.z0 + depth), Vec2::Y),
            1 => (Vec2::new(self.x0 + depth, self.z0 + t), Vec2::X),
            2 => (Vec2::new(self.x1 - depth, self.z0 + t), Vec2::NEG_X),
            _ => (Vec2::new(self.x0 + t, self.z1 - depth), Vec2::NEG_Y),
        }
    }
}

/// Half extents on the ground of something `width` wide along the wall and `depth` deep, facing `face`.
fn footprint(face: Vec2, width: f32, depth: f32) -> Vec2 {
    if face.x.abs() > 0.5 { Vec2::new(depth * 0.5, width * 0.5) } else { Vec2::new(width * 0.5, depth * 0.5) }
}

/// Across `face` (along the wall), as a unit vector.
fn across(face: Vec2) -> Vec2 {
    Vec2::new(-face.y, face.x)
}

impl World {
    /// The floor finish for a room walled in `walls`: `wanted`, unless that is too close to the
    /// walls to tell apart, then the first of the others that is not.
    pub(crate) fn floor_finish(wanted: Paint, walls: &[Paint]) -> Paint {
        let ok = |f: Paint| walls.iter().all(|w| f.contrast(*w) >= FLOOR_CONTRAST);
        let options = [wanted, Paint::Planks, Paint::Wood, Paint::Tile, Paint::Carpet, Paint::Lino, Paint::Slate, Paint::Cream];
        options.into_iter().find(|f| ok(*f)).unwrap_or_else(|| {
            options.into_iter().max_by(|a, b| {
                let worst = |f: Paint| walls.iter().map(|w| f.contrast(*w)).fold(f32::MAX, f32::min);
                worst(*a).total_cmp(&worst(*b))
            }).unwrap_or(wanted)
        })
    }

    /// Records a room's floor and walls (see [`World::rooms`]).
    pub(crate) fn note_room(&mut self, floor: Paint, walls: Paint) {
        self.rooms.push((floor, walls));
    }

    /// A solid piece of furniture (marked as such): a box `half` round `p` on the floor, `height` tall.
    fn piece(&mut self, p: Vec2, half: Vec2, y: f32, height: f32, paint: Paint) {
        let from = self.blocks.len();
        self.blocks.push(Block { aabb: Aabb::span(Vec3::new(p.x - half.x, y, p.y - half.y), Vec3::new(p.x + half.x, y + height, p.y + half.y)), paint, solid: true, hidden: false });
        self.mark_furniture(from);
    }

    /// A piece standing on another at `y0` (a raised back, a headboard), solid.
    fn piece_from(&mut self, p: Vec2, half: Vec2, y0: f32, y1: f32, paint: Paint) {
        let from = self.blocks.len();
        self.blocks.push(Block { aabb: Aabb::span(Vec3::new(p.x - half.x, y0, p.y - half.y), Vec3::new(p.x + half.x, y1, p.y + half.y)), paint, solid: true, hidden: false });
        self.mark_furniture(from);
    }

    /// Dressing on furniture: a box `half` round `c`.
    fn bit(&mut self, c: Vec3, half: Vec3, paint: Paint) {
        let from = self.dressing.len();
        self.trim_at(c, half, paint);
        self.mark_furniture_dressing(from);
    }

    fn bit_part(&mut self, shape: Shape, paint: Paint, pos: Vec3, size: Vec3) {
        let from = self.dressing.len();
        self.trim_part(shape, paint, pos, size, Quat::IDENTITY);
        self.mark_furniture_dressing(from);
    }

    /// A rug: a flat decal with a border, no collision.
    pub(crate) fn rug(&mut self, c: Vec2, half: Vec2, y: f32, field: Paint, border: Paint) {
        self.trim_at(Vec3::new(c.x, y + 0.017, c.y), Vec3::new(half.x, 0.005, half.y), border);
        self.trim_at(Vec3::new(c.x, y + 0.024, c.y), Vec3::new(half.x - 0.12, 0.004, half.y - 0.12), field);
        // A pattern: a band either end.
        let along_x = half.x >= half.y;
        for s in [-1.0, 1.0] {
            let o = if along_x { Vec3::new(s * (half.x - 0.35), 0.0, 0.0) } else { Vec3::new(0.0, 0.0, s * (half.y - 0.35)) };
            let h = if along_x { Vec3::new(0.06, 0.004, half.y - 0.2) } else { Vec3::new(half.x - 0.2, 0.004, 0.06) };
            self.trim_at(Vec3::new(c.x, y + 0.03, c.y) + o, h, border);
        }
    }

    /// A tiled patch of floor: a sheet with grout lines every `size` metres.
    fn tiles(&mut self, lo: Vec2, hi: Vec2, y: f32, paint: Paint, size: f32) {
        self.trim(Vec3::new(lo.x, y + 0.013, lo.y), Vec3::new(hi.x, y + 0.02, hi.y), paint);
        let grout = crate::worldgen::shapes::darker(paint);
        let mut x = lo.x + size;
        while x < hi.x - 0.05 {
            self.trim(Vec3::new(x - 0.012, y + 0.02, lo.y), Vec3::new(x + 0.012, y + 0.024, hi.y), grout);
            x += size;
        }
        let mut z = lo.y + size;
        while z < hi.y - 0.05 {
            self.trim(Vec3::new(lo.x, y + 0.02, z - 0.012), Vec3::new(hi.x, y + 0.024, z + 0.012), grout);
            z += size;
        }
    }

    // --- Pieces ---------------------------------------------------------------------------

    /// A sofa with its back to the wall, facing `face`: a solid seat and back (low cover), arms,
    /// cushions and a throw.
    fn sofa(&mut self, rng: &mut Rng, p: Vec2, face: Vec2, y: f32, width: f32) {
        let cloth = *rng.pick(&[Paint::Teal, Paint::Navy, Paint::Coral, Paint::Purple, Paint::Mustard, Paint::Slate]).unwrap_or(&Paint::Navy);
        self.piece(p + face * 0.1, footprint(face, width, 0.7), y, 0.45, cloth);
        let back = p - face * 0.32;
        self.piece_from(back, footprint(face, width, 0.22), y, y + 0.88, cloth);
        let side = across(face);
        let up = Vec3::Y;
        for s in [-1.0, 1.0] {
            let arm = p + face * 0.05 + side * s * (width * 0.5 - 0.1);
            let h = footprint(face, 0.2, 0.82);
            self.bit(Vec3::new(arm.x, y + 0.56, arm.y), Vec3::new(h.x + 0.01, 0.11, h.y + 0.01), cloth);
        }
        // Seat cushions and a scatter cushion.
        let n = if width > 1.7 { 3 } else { 2 };
        let cw = (width - 0.5) / n as f32;
        for i in 0..n {
            let c = p + face * 0.12 + side * ((i as f32 + 0.5) * cw - (width - 0.5) * 0.5);
            let h = footprint(face, cw - 0.04, 0.6);
            self.bit(Vec3::new(c.x, y + 0.5, c.y), Vec3::new(h.x, 0.05, h.y), crate::worldgen::shapes::darker(cloth));
        }
        let pillow = p - face * 0.12 + side * (width * 0.5 - 0.42);
        let h = footprint(face, 0.36, 0.12);
        self.bit(Vec3::new(pillow.x, y + 0.72, pillow.y) + up * 0.0, Vec3::new(h.x, 0.17, h.y), *rng.pick(&[Paint::Mustard, Paint::Cream, Paint::Coral]).unwrap_or(&Paint::Cream));
    }

    /// A low table with a book and a mug on it.
    fn coffee_table(&mut self, p: Vec2, face: Vec2, y: f32) {
        let h = footprint(face, 1.1, 0.55);
        self.piece(p, h, y, 0.4, Paint::Wood);
        self.bit(Vec3::new(p.x, y + 0.41, p.y), Vec3::new(h.x + 0.03, 0.025, h.y + 0.03), Paint::Planks);
        self.bit(Vec3::new(p.x - 0.2, y + 0.45, p.y + 0.05), Vec3::new(0.12, 0.02, 0.09), Paint::Red);
        self.bit_part(Shape::Cylinder, Paint::Cream, Vec3::new(p.x + 0.25, y + 0.48, p.y - 0.08), Vec3::new(0.08, 0.1, 0.08));
    }

    /// A TV on a low stand against the wall.
    fn tv(&mut self, p: Vec2, face: Vec2, y: f32) {
        self.piece(p, footprint(face, 1.4, 0.42), y, 0.5, Paint::Wood);
        let screen = p - face * 0.05;
        let h = footprint(face, 1.1, 0.05);
        self.bit(Vec3::new(screen.x, y + 0.92, screen.y), Vec3::new(h.x, 0.32, h.y), Paint::Dark);
        let glass = screen + face * 0.03;
        let g = footprint(face, 1.0, 0.01);
        self.bit(Vec3::new(glass.x, y + 0.93, glass.y), Vec3::new(g.x, 0.28, g.y), Paint::Screen);
        self.bit(Vec3::new(screen.x, y + 0.56, screen.y), Vec3::new(0.12, 0.06, 0.12), Paint::Dark);
    }

    /// A bookcase against the wall, full of books.
    fn bookcase(&mut self, rng: &mut Rng, p: Vec2, face: Vec2, y: f32, width: f32) {
        let h = footprint(face, width, 0.36);
        self.piece(p, h, y, 1.9, Paint::Wood);
        let front = p + face * 0.185;
        let side = across(face);
        for shelf in 0..4 {
            let sy = y + 0.12 + shelf as f32 * 0.45;
            let ledge = footprint(face, width - 0.06, 0.02);
            self.bit(Vec3::new(front.x, sy, front.y), Vec3::new(ledge.x, 0.015, ledge.y), Paint::Dark);
            let mut t = -width * 0.5 + 0.08;
            while t < width * 0.5 - 0.12 {
                let bw = rng.range(0.04, 0.09);
                let bh = rng.range(0.22, 0.34);
                if rng.chance(0.85) {
                    let c = front + side * (t + bw * 0.5);
                    let b = footprint(face, bw, 0.02);
                    let paint = *rng.pick(&[Paint::Red, Paint::Navy, Paint::Mustard, Paint::Lino, Paint::Cream, Paint::Carpet, Paint::Teal]).unwrap_or(&Paint::Red);
                    // Lit paints glow: keep books to the lit ones.
                    let paint = if paint.unlit() { Paint::Coral } else { paint };
                    self.bit(Vec3::new(c.x, sy + 0.015 + bh * 0.5, c.y), Vec3::new(b.x, bh * 0.5, b.y), paint);
                }
                t += bw + 0.01;
            }
        }
    }

    /// A standard lamp: a pole and a shade with a glowing bulb.
    fn floor_lamp(&mut self, p: Vec2, y: f32) {
        let from = self.dressing.len();
        self.trim_rod(Vec3::new(p.x, y, p.y), Vec3::new(p.x, y + 1.5, p.y), 0.02, Paint::Dark);
        self.trim_part(Shape::Cylinder, Paint::Dark, Vec3::new(p.x, y + 0.02, p.y), Vec3::new(0.3, 0.04, 0.3), Quat::IDENTITY);
        self.trim_part(Shape::Cone, Paint::Cream, Vec3::new(p.x, y + 1.6, p.y), Vec3::new(0.42, 0.3, 0.42), Quat::IDENTITY);
        self.trim_part(Shape::Shard, Paint::Glow, Vec3::new(p.x, y + 1.47, p.y), Vec3::splat(0.07), Quat::IDENTITY);
        self.mark_furniture_dressing(from);
    }

    /// A bed with its head to the wall, facing `face`; a nightstand and lamp either side.
    fn bed_set(&mut self, rng: &mut Rng, p: Vec2, face: Vec2, y: f32, double: bool) {
        let width = if double { 1.6 } else { 1.0 };
        let length = 2.05;
        let c = p + face * (length * 0.5);
        self.piece(c, footprint(face, width, length), y, 0.42, Paint::Wood);
        // Headboard: solid against the wall.
        self.piece_from(p + face * 0.05, footprint(face, width + 0.1, 0.1), y, y + 1.0, Paint::Wood);
        let side = across(face);
        let m = footprint(face, width - 0.06, length - 0.12);
        self.bit(Vec3::new(c.x, y + 0.5, c.y), Vec3::new(m.x, 0.08, m.y), Paint::Cream);
        // The covers over the foot two-thirds, folded back at the top.
        let blanket = *rng.pick(&[Paint::Navy, Paint::Teal, Paint::Coral, Paint::Lino, Paint::Mustard, Paint::Purple]).unwrap_or(&Paint::Navy);
        let cover = c + face * 0.25;
        let k = footprint(face, width + 0.02, length - 0.55);
        self.bit(Vec3::new(cover.x, y + 0.55, cover.y), Vec3::new(k.x, 0.06, k.y), blanket);
        let fold = c - face * (length * 0.5 - 0.85);
        let f = footprint(face, width + 0.03, 0.18);
        self.bit(Vec3::new(fold.x, y + 0.6, fold.y), Vec3::new(f.x, 0.04, f.y), Paint::White);
        let pillows = if double { vec![-0.38, 0.38] } else { vec![0.0] };
        for t in pillows {
            let q = p + face * 0.38 + side * t;
            let h = footprint(face, 0.6, 0.36);
            self.bit(Vec3::new(q.x, y + 0.64, q.y), Vec3::new(h.x, 0.07, h.y), Paint::White);
        }
        for s in [-1.0, 1.0] {
            let n = p + face * 0.25 + side * s * (width * 0.5 + 0.35);
            self.piece(n, Vec2::splat(0.22), y, 0.55, Paint::Wood);
            self.bit(Vec3::new(n.x, y + 0.42, n.y) + Vec3::new(face.x, 0.0, face.y) * 0.225, Vec3::new(0.12 * face.y.abs() + 0.005, 0.02, 0.12 * face.x.abs() + 0.005), Paint::Gold);
            if rng.chance(0.7) {
                let from = self.dressing.len();
                self.trim_part(Shape::Cylinder, Paint::Dark, Vec3::new(n.x, y + 0.6, n.y), Vec3::new(0.12, 0.1, 0.12), Quat::IDENTITY);
                self.trim_part(Shape::Cone, Paint::Cream, Vec3::new(n.x, y + 0.78, n.y), Vec3::new(0.26, 0.22, 0.26), Quat::IDENTITY);
                self.trim_part(Shape::Shard, Paint::Glow, Vec3::new(n.x, y + 0.7, n.y), Vec3::splat(0.05), Quat::IDENTITY);
                self.mark_furniture_dressing(from);
            }
        }
    }

    /// A wardrobe against the wall: two doors and handles.
    fn wardrobe(&mut self, p: Vec2, face: Vec2, y: f32, width: f32, paint: Paint) {
        self.piece(p, footprint(face, width, 0.6), y, 2.0, paint);
        let front = p + face * 0.305;
        let seam = footprint(face, 0.02, 0.01);
        self.bit(Vec3::new(front.x, y + 1.0, front.y), Vec3::new(seam.x, 0.95, seam.y), Paint::Dark);
        let side = across(face);
        for s in [-1.0, 1.0] {
            let h = front + side * s * 0.08 + face * 0.01;
            self.bit(Vec3::new(h.x, y + 1.05, h.y), Vec3::new(0.02, 0.12, 0.02), Paint::Gold);
        }
        self.bit(Vec3::new(p.x, y + 2.02, p.y), Vec3::new(footprint(face, width + 0.04, 0.64).x, 0.03, footprint(face, width + 0.04, 0.64).y), Paint::Dark);
    }

    /// A chest of drawers with a mirror or picture over it.
    fn dresser(&mut self, p: Vec2, face: Vec2, y: f32) {
        self.piece(p, footprint(face, 1.0, 0.45), y, 0.85, Paint::Planks);
        let front = p + face * 0.23;
        for k in 0..3 {
            let d = footprint(face, 0.9, 0.01);
            self.bit(Vec3::new(front.x, y + 0.2 + k as f32 * 0.24, front.y), Vec3::new(d.x, 0.012, d.y), Paint::Wood);
        }
        let mirror = p - face * 0.2;
        let m = footprint(face, 0.7, 0.02);
        self.bit(Vec3::new(mirror.x, y + 1.4, mirror.y), Vec3::new(m.x, 0.35, m.y), Paint::Wood);
        let g = footprint(face, 0.6, 0.02);
        let glass = mirror + face * 0.01;
        self.bit(Vec3::new(glass.x, y + 1.4, glass.y), Vec3::new(g.x, 0.3, g.y), Paint::Tile);
    }

    /// A run of kitchen units along a wall from `t0` to `t1`: cupboards under a worktop, a sink
    /// and a cooker set in it, wall cupboards over it, a fridge at the end.
    fn kitchen_run(&mut self, rng: &mut Rng, room: &Room, side: usize, t0: f32, t1: f32) {
        let y = room.y;
        let len = t1 - t0;
        if len < 1.8 {
            return;
        }
        let units = *rng.pick(&[Paint::Cream, Paint::Lino, Paint::White, Paint::Navy, Paint::Planks]).unwrap_or(&Paint::Cream);
        let units = if units.unlit() { Paint::Cream } else { units };
        let fridge_len = 0.75;
        let run = len - fridge_len;
        let (mid, face) = room.at_wall(side, t0 + run * 0.5, 0.3);
        self.piece(mid, footprint(face, run, 0.6), y, 0.88, units);
        let top = footprint(face, run + 0.02, 0.64);
        let worktop = mid + face * 0.02;
        self.bit(Vec3::new(worktop.x, y + 0.9, worktop.y), Vec3::new(top.x, 0.025, top.y), Paint::Slate);
        // A dark kick plate and cupboard doors.
        let kick = mid + face * 0.25;
        let kp = footprint(face, run, 0.02);
        self.bit(Vec3::new(kick.x, y + 0.05, kick.y), Vec3::new(kp.x, 0.05, kp.y), Paint::Dark);
        let side_dir = across(face);
        let doors = (run / 0.6).floor().max(1.0) as usize;
        for i in 1..doors {
            let s = mid + face * 0.305 + side_dir * ((i as f32) * run / doors as f32 - run * 0.5);
            let seam = footprint(face, 0.015, 0.01);
            self.bit(Vec3::new(s.x, y + 0.48, s.y), Vec3::new(seam.x, 0.36, seam.y), Paint::Dark);
        }
        // Sink and tap, cooker and burners.
        let sink = mid + side_dir * (-run * 0.2) + face * 0.02;
        let basin = footprint(face, 0.55, 0.4);
        self.bit(Vec3::new(sink.x, y + 0.93, sink.y), Vec3::new(basin.x, 0.012, basin.y), Paint::Metal);
        let tap = sink - face * 0.24;
        let from = self.dressing.len();
        self.trim_rod(Vec3::new(tap.x, y + 0.92, tap.y), Vec3::new(tap.x, y + 1.18, tap.y), 0.02, Paint::Iron);
        let spout = tap + face * 0.14;
        self.trim_rod(Vec3::new(tap.x, y + 1.18, tap.y), Vec3::new(spout.x, y + 1.14, spout.y), 0.018, Paint::Iron);
        self.mark_furniture_dressing(from);
        if run > 2.2 {
            let cook = mid + side_dir * (run * 0.25);
            let hob = footprint(face, 0.6, 0.55);
            self.bit(Vec3::new(cook.x, y + 0.925, cook.y), Vec3::new(hob.x, 0.01, hob.y), Paint::Dark);
            for (a, b) in [(-0.14, -0.12), (0.14, -0.12), (-0.14, 0.12), (0.14, 0.12)] {
                let q = cook + side_dir * a + face * b;
                self.bit_part(Shape::Cylinder, Paint::Steel, Vec3::new(q.x, y + 0.94, q.y), Vec3::new(0.17, 0.012, 0.17));
            }
            let oven = cook + face * 0.305;
            let o = footprint(face, 0.56, 0.01);
            self.bit(Vec3::new(oven.x, y + 0.45, oven.y), Vec3::new(o.x, 0.25, o.y), Paint::Dark);
            self.bit(Vec3::new(oven.x, y + 0.45, oven.y) + Vec3::new(face.x, 0.0, face.y) * 0.01, Vec3::new(o.x * 0.7 + 0.004, 0.13, o.y * 0.7 + 0.004), Paint::Screen);
            // A hood over it.
            let hood = cook - face * 0.12;
            let hh = footprint(face, 0.62, 0.42);
            self.bit(Vec3::new(hood.x, y + 1.85, hood.y), Vec3::new(hh.x, 0.1, hh.y), Paint::Metal);
        }
        // Wall cupboards (hung on the wall: dressing, clear of heads).
        let wall_units = mid - face * 0.12;
        let wu = footprint(face, run * 0.45, 0.34);
        for s in [-1.0, 1.0] {
            let c = wall_units + side_dir * s * run * 0.26;
            self.bit(Vec3::new(c.x, y + 1.95, c.y), Vec3::new(wu.x, 0.33, wu.y), units);
        }
        // The fridge.
        let (fp, _) = room.at_wall(side, t0 + run + fridge_len * 0.5, 0.36);
        self.piece(fp, footprint(face, fridge_len - 0.05, 0.68), y, 1.85, Paint::White);
        let fh = fp + face * 0.345 + side_dir * 0.25;
        self.bit(Vec3::new(fh.x, y + 1.15, fh.y), Vec3::new(0.025, 0.25, 0.025), Paint::Metal);
        let seam = fp + face * 0.345;
        let sm = footprint(face, 0.7, 0.01);
        self.bit(Vec3::new(seam.x, y + 1.2, seam.y), Vec3::new(sm.x, 0.01, sm.y), Paint::Metal);
    }

    /// A table with chairs round it (the chairs' seats are solid, their backs thin).
    fn dining(&mut self, rng: &mut Rng, p: Vec2, y: f32, along_x: bool) {
        let half = if along_x { Vec2::new(0.75, 0.45) } else { Vec2::new(0.45, 0.75) };
        self.piece(p, half, y, 0.74, Paint::Wood);
        self.bit(Vec3::new(p.x, y + 0.75, p.y), Vec3::new(half.x + 0.05, 0.02, half.y + 0.05), Paint::Planks);
        // A cloth runner and a bowl of fruit.
        let runner = if along_x { Vec3::new(half.x, 0.005, 0.15) } else { Vec3::new(0.15, 0.005, half.y) };
        self.bit(Vec3::new(p.x, y + 0.775, p.y), runner, Paint::Cream);
        self.bit_part(Shape::Cylinder, Paint::Bone, Vec3::new(p.x, y + 0.81, p.y), Vec3::new(0.26, 0.06, 0.26));
        for k in 0..3 {
            let o = Vec2::from_angle(k as f32 * 2.1) * 0.06;
            self.bit_part(Shape::Shard, *rng.pick(&[Paint::Red, Paint::Gold, Paint::LeafLight]).unwrap_or(&Paint::Red), Vec3::new(p.x + o.x, y + 0.86, p.y + o.y), Vec3::splat(0.06));
        }
        let chairs: [(Vec2, Vec2); 4] = if along_x {
            [(Vec2::new(-0.35, -0.75), Vec2::Y), (Vec2::new(0.35, -0.75), Vec2::Y), (Vec2::new(-0.35, 0.75), Vec2::NEG_Y), (Vec2::new(0.35, 0.75), Vec2::NEG_Y)]
        } else {
            [(Vec2::new(-0.75, -0.35), Vec2::X), (Vec2::new(-0.75, 0.35), Vec2::X), (Vec2::new(0.75, -0.35), Vec2::NEG_X), (Vec2::new(0.75, 0.35), Vec2::NEG_X)]
        };
        for (o, face) in chairs {
            let c = p + o;
            self.piece(c, Vec2::splat(0.2), y, 0.46, Paint::Wood);
            let back = c - face * 0.18;
            let b = footprint(face, 0.4, 0.04);
            self.bit(Vec3::new(back.x, y + 0.72, back.y), Vec3::new(b.x, 0.26, b.y), Paint::Wood);
        }
    }

    /// A bath, a basin and a toilet along a wall.
    fn bathroom_set(&mut self, room: &Room, side: usize, t0: f32) {
        let y = room.y;
        let (tub, face) = room.at_wall(side, t0 + 0.9, 0.38);
        self.piece(tub, footprint(face, 1.7, 0.75), y, 0.55, Paint::White);
        let water = footprint(face, 1.5, 0.55);
        self.bit(Vec3::new(tub.x, y + 0.555, tub.y), Vec3::new(water.x, 0.005, water.y), Paint::Tile);
        let (basin, _) = room.at_wall(side, t0 + 2.2, 0.25);
        self.piece(basin, footprint(face, 0.55, 0.42), y, 0.82, Paint::White);
        let mirror = basin - face * 0.2;
        let m = footprint(face, 0.5, 0.02);
        self.bit(Vec3::new(mirror.x, y + 1.45, mirror.y), Vec3::new(m.x, 0.3, m.y), Paint::Tile);
        let (wc, _) = room.at_wall(side, t0 + 3.1, 0.32);
        self.piece(wc, footprint(face, 0.4, 0.55), y, 0.42, Paint::White);
        let cistern = wc - face * 0.2;
        let ci = footprint(face, 0.42, 0.16);
        self.bit(Vec3::new(cistern.x, y + 0.65, cistern.y), Vec3::new(ci.x, 0.22, ci.y), Paint::White);
        self.tiles(Vec2::new(room.x0, room.z0).max(tub - Vec2::splat(1.2)), Vec2::new(room.x1, room.z1).min(wc + Vec2::splat(1.0)), y, Paint::Tile, 0.4);
    }

    /// Shelving with goods on it.
    fn shop_shelf(&mut self, rng: &mut Rng, p: Vec2, along_x: bool, length: f32, y: f32) {
        let half = if along_x { Vec2::new(length * 0.5, 0.3) } else { Vec2::new(0.3, length * 0.5) };
        self.piece(p, half, y, 1.55, Paint::Steel);
        let dir = if along_x { Vec2::X } else { Vec2::Y };
        let out = if along_x { Vec2::Y } else { Vec2::X };
        for shelf in 0..3 {
            let sy = y + 0.35 + shelf as f32 * 0.42;
            for s in [-1.0f32, 1.0] {
                let mut t = -length * 0.5 + 0.1;
                while t < length * 0.5 - 0.15 {
                    let w = rng.range(0.12, 0.26);
                    let c = p + dir * (t + w * 0.5) + out * s * 0.31;
                    let h = rng.range(0.12, 0.28);
                    let paint = *rng.pick(&[Paint::Red, Paint::Mustard, Paint::Cream, Paint::Teal, Paint::Coral, Paint::Navy, Paint::LeafLight]).unwrap_or(&Paint::Red);
                    let half = if along_x { Vec3::new(w * 0.5 - 0.01, h * 0.5, 0.035) } else { Vec3::new(0.035, h * 0.5, w * 0.5 - 0.01) };
                    self.bit(Vec3::new(c.x, sy + h * 0.5, c.y), half, paint);
                    t += w + 0.02;
                }
            }
        }
        // A price strip on top.
        let strip = if along_x { Vec3::new(length * 0.5, 0.06, 0.32) } else { Vec3::new(0.32, 0.06, length * 0.5) };
        self.bit(Vec3::new(p.x, y + 1.5, p.y), strip, Paint::Mustard);
    }

    /// A shop counter with a till.
    fn till(&mut self, p: Vec2, face: Vec2, y: f32) {
        self.piece(p, footprint(face, 1.8, 0.6), y, 1.0, Paint::Wood);
        self.bit(Vec3::new(p.x, y + 1.02, p.y), Vec3::new(footprint(face, 1.86, 0.66).x, 0.02, footprint(face, 1.86, 0.66).y), Paint::Slate);
        let till = p - face * 0.1 + across(face) * 0.45;
        self.bit(Vec3::new(till.x, y + 1.12, till.y), Vec3::new(0.18, 0.08, 0.15), Paint::Dark);
        self.bit(Vec3::new(till.x, y + 1.25, till.y), Vec3::new(0.14, 0.06, 0.02 + 0.1 * face.x.abs()), Paint::Green);
    }

    /// An office: desks along the main wall, a bookcase and filing cabinets along another.
    fn office_room(&mut self, rng: &mut Rng, room: &Room, main: usize, other: Option<usize>) {
        let y = room.y;
        let len = room.wall_len(main);
        let n = ((len - 0.6) / 2.2).floor().clamp(1.0, 3.0) as usize;
        for i in 0..n {
            let (p, face) = room.at_wall(main, (i as f32 + 0.5) * len / n as f32, 0.45);
            self.desk(rng, p, face, y);
        }
        if let Some(s) = other {
            let l = room.wall_len(s);
            let (q, f) = room.at_wall(s, l * 0.3, 0.19);
            self.bookcase(rng, q, f, y, 0.9);
            let (q, _) = room.at_wall(s, l * 0.7, 0.32);
            let _ = f;
            self.filing_cabinets(q, 2, y);
        }
    }

    // --- Rooms ----------------------------------------------------------------------------

    /// Furnishes a room for what it is for. `skip` marks walls (south, west, east, north) to keep
    /// clear: a stairway along them, or an open side.
    pub(crate) fn furnish_room(&mut self, kind: RoomKind, room: Room, skip: [bool; 4]) {
        if room.w() < 2.2 || room.d() < 2.2 {
            return;
        }
        let mut rng = self.detail_rng(room.centre(), 0xf0 + kind as u64);
        let y = room.y;
        let free: Vec<usize> = (0..4).filter(|s| !skip[*s]).collect();
        // The longest free wall first.
        let mut walls = free.clone();
        walls.sort_by(|a, b| room.wall_len(*b).total_cmp(&room.wall_len(*a)));
        let Some(&main) = walls.first() else { return };
        let other = walls.iter().copied().find(|s| *s != main && *s != 3 - main).or_else(|| walls.get(1).copied());
        let opposite = walls.iter().copied().find(|s| *s == 3 - main);
        let c = room.centre();
        match kind {
            RoomKind::Living => {
                let len = room.wall_len(main);
                let width = (len - 1.4).clamp(1.4, 2.2);
                let (p, face) = room.at_wall(main, len * 0.5, 0.42);
                self.sofa(&mut rng, p, face, y, width);
                let table = p + face * 1.25;
                if room.wall_len(main) > 3.0 {
                    self.coffee_table(table, face, y);
                }
                let rug = footprint(face, width + 0.6, 2.0);
                self.rug(p + face * 1.35, rug, y, *rng.pick(&[Paint::Carpet, Paint::Coral, Paint::Navy, Paint::Mustard, Paint::Lino]).unwrap_or(&Paint::Coral), Paint::Cream);
                if let Some(o) = opposite {
                    let l = room.wall_len(o);
                    let (q, f) = room.at_wall(o, l * 0.5, 0.22);
                    if q.distance(p) > 3.2 {
                        self.tv(q, f, y);
                    }
                }
                if let Some(s) = other {
                    let l = room.wall_len(s);
                    let (q, f) = room.at_wall(s, l * 0.5, 0.19);
                    self.bookcase(&mut rng, q, f, y, 1.0);
                }
                let corner = p + across(face) * (width * 0.5 + 0.4);
                if room.x0 < corner.x && corner.x < room.x1 && room.z0 < corner.y && corner.y < room.z1 {
                    self.floor_lamp(corner, y);
                }
            }
            RoomKind::Kitchen => {
                let len = room.wall_len(main);
                self.kitchen_run(&mut rng, &room, main, 0.25, (len - 0.25).min(4.8));
                // Tiles in front of the units.
                let (a, _) = room.at_wall(main, 0.0, 0.0);
                let (b, face) = room.at_wall(main, len.min(5.0), 1.6);
                let (lo, hi) = (a.min(b), a.max(b));
                let _ = face;
                self.tiles(lo, hi, y, Paint::Tile, 0.5);
                let table = c + room.at_wall(main, 0.0, 0.0).1 * 0.6;
                if room.w().min(room.d()) > 3.4 {
                    self.dining(&mut rng, table, y, room.w() > room.d());
                }
            }
            RoomKind::Bedroom => {
                let len = room.wall_len(main);
                let double = len > 3.4 && room.wall_len(3 - main).max(room.wall_len(other.unwrap_or(main))) > 3.2;
                let (p, face) = room.at_wall(main, len * 0.5, 0.0);
                let depth = if main == 0 || main == 3 { room.d() } else { room.w() };
                if depth > 2.6 {
                    self.bed_set(&mut rng, p, face, y, double);
                    let rug = footprint(face, if double { 2.4 } else { 1.8 }, 1.1);
                    let at = p + face * 2.45;
                    if depth > 3.8 {
                        self.rug(at, rug, y, *rng.pick(&[Paint::Cream, Paint::Mustard, Paint::Coral, Paint::Navy]).unwrap_or(&Paint::Cream), Paint::Dark);
                    }
                }
                if let Some(s) = other {
                    let l = room.wall_len(s);
                    let (q, f) = room.at_wall(s, l * 0.3, 0.31);
                    self.wardrobe(q, f, y, 1.2, if rng.chance(0.5) { Paint::Wood } else { Paint::Cream });
                    if l > 3.0 {
                        let (q, f) = room.at_wall(s, l * 0.75, 0.24);
                        self.dresser(q, f, y);
                    }
                }
            }
            RoomKind::Bathroom => {
                if room.wall_len(main) > 3.5 {
                    self.bathroom_set(&room, main, 0.1);
                }
            }
            RoomKind::Office => {
                let (blocks, dressing) = (self.blocks.len(), self.dressing.len());
                self.office_room(&mut rng, &room, main, other);
                self.mark_furniture(blocks);
                self.mark_furniture_dressing(dressing);
                self.rug(c, Vec2::new((room.w() * 0.25).min(1.4), (room.d() * 0.25).min(1.0)), y, Paint::Slate, Paint::Dark);
            }
            RoomKind::Shop => {
                let along_x = room.w() >= room.d();
                let (long, short) = if along_x { (room.w(), room.d()) } else { (room.d(), room.w()) };
                let rows = ((short - 2.6) / 2.3).floor().clamp(0.0, 2.0) as usize;
                for i in 0..rows {
                    let o = (i as f32 + 1.0) * short / (rows as f32 + 1.0) - short * 0.5;
                    let p = c + if along_x { Vec2::new(0.4, o) } else { Vec2::new(o, 0.4) };
                    self.shop_shelf(&mut rng, p, along_x, (long - 3.6).clamp(1.2, 4.0), y);
                }
                let (q, f) = room.at_wall(main, room.wall_len(main) * 0.2, 0.32);
                self.shop_shelf(&mut rng, q, f.y.abs() > 0.5, 1.4, y);
                if let Some(s) = other {
                    let l = room.wall_len(s);
                    let (q, f) = room.at_wall(s, l * 0.6, 1.0);
                    self.till(q, f, y);
                }
            }
            RoomKind::Store => {
                for k in 0..2 {
                    let s = walls[k % walls.len()];
                    let l = room.wall_len(s);
                    let (q, f) = room.at_wall(s, l * (0.3 + 0.4 * k as f32), 0.45);
                    self.piece(q, footprint(f, 1.6, 0.8), y, 1.0, Paint::Wood);
                    self.bit(Vec3::new(q.x, y + 1.25, q.y), Vec3::new(0.3, 0.25, 0.3), Paint::Sand);
                    self.bit(Vec3::new(q.x, y + 1.01, q.y), Vec3::new(footprint(f, 1.62, 0.82).x, 0.012, footprint(f, 1.62, 0.82).y), Paint::Dark);
                }
            }
        }
    }
}
