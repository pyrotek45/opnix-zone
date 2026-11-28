//! Street dressing and set pieces: the kerbside life of a town - lamp posts, power poles and
//! their wires, hydrants, signs, bollards, planters, parked bikes, phone booths, vending
//! machines and billboards (with original art: no brands, no words) - and doorways dressed
//! with awnings, canopies, lights and shop signs; a market, a playground's rides and a
//! construction site where there is room.
//!
//! All of it is rolled from its own position (never the map's main stream), placed where
//! nothing stands - clear of doors, ladders, spawns, cars and props - and, apart from the few
//! pieces big enough to hide behind (booths, machines, planters, stalls, pipes), is dressing.

use super::{ClutterKind, World, interiors, shapes::darker};
use crate::{
    collide::Aabb,
    model::{Anim, Hull, Paint, Shape},
    rng::Rng,
};
use glam::{Quat, Vec2, Vec3};
use std::f32::consts::{FRAC_PI_2, TAU};

impl World {
    /// Is there room at `p` (radius `r`) for street furniture?
    fn street_clear(&self, p: Vec2, r: f32) -> bool {
        let reach = self.arena + 8.0;
        self.in_bounds(p.x, p.y)
            && p.x.abs() < reach
            && p.y.abs() < reach
            && self.height(p.x, p.y) > self.water + 0.4
            && !self.occupied(p, r)
            && !self.props.iter().any(|q| Vec2::new(q.pos.x, q.pos.z).distance(p) < r + 1.0)
            && !self.clutter.iter().any(|c| Vec2::new(c.pos.x, c.pos.z).distance(p) < r + if c.kind == ClutterKind::Car { 2.6 } else { 1.2 })
            && !self.doors.iter().any(|d| Vec2::new(d.center.x, d.center.z).distance(p) < r + 2.6)
            && !self.ladders.iter().any(|l| Vec2::new(l.foot.x, l.foot.z).distance(p) < r + 1.5)
            && !self.team_spawns.iter().flatten().any(|s| s.distance(p) < r + 5.0)
            && !self.pads.iter().any(|s| s.distance(p) < r + 4.0)
            && self.core.distance(p) > r + 4.0
            && !self.waypoints.iter().any(|w| Vec2::new(w.x, w.z).distance(p) < r + 1.5)
            && self.dressing_near(p, r)
    }

    /// Nothing tall from the dressing already stands within `r` (lamps, poles, signs).
    fn dressing_near(&self, p: Vec2, r: f32) -> bool {
        !self.street_spots.iter().any(|q| q.distance(p) < r + 1.2)
    }

    /// The kerbside of every street, dressed.
    pub(crate) fn dress_streets(&mut self) {
        if self.horde.is_some() || self.recipe.layout.is_indoor() && self.recipe.layout == super::Layout::SpaceStation {
            return;
        }
        let theme = self.theme;
        let rural = matches!(theme, Some(super::Theme::Farmland | super::Theme::DesertOutpost | super::Theme::WoodsPark)) || self.recipe.layout == super::Layout::Mountainside;
        let roads = self.roads.clone();
        for (k, &(a, b, w)) in roads.iter().enumerate() {
            let length = a.distance(b);
            if w < 4.5 || length < 6.0 {
                continue;
            }
            let dir = (b - a) / length;
            let normal = Vec2::new(-dir.y, dir.x);
            let mut rng = self.detail_rng(a + b, 0x57e + k as u64);
            // Power poles down one side, wired up.
            let pole_side = if rng.chance(0.5) { 1.0 } else { -1.0 };
            let mut last: Option<Vec3> = None;
            let mut t = rng.range(3.0, 9.0);
            while t < length - 3.0 {
                let p = a + dir * t + normal * pole_side * (w * 0.5 + 1.3);
                if self.street_clear(p, 0.6) {
                    let top = self.power_pole(p, dir);
                    if let Some(prev) = last
                        && prev.distance(top) < 26.0
                    {
                        for o in [-0.8, 0.0, 0.8] {
                            let off = Vec3::new(dir.y, 0.0, -dir.x) * o;
                            self.wire(prev + off, top + off, 0.5);
                        }
                    }
                    last = Some(top);
                } else {
                    last = None;
                }
                t += rng.range(17.0, 21.0);
            }
            // Everything else, both sides, every few metres.
            let mut t = rng.range(2.0, 6.0);
            while t < length - 2.0 {
                let side = if rng.chance(0.5) { 1.0 } else { -1.0 };
                let p = a + dir * t + normal * side * (w * 0.5 + 0.8);
                let face = -normal * side;
                let roll = rng.unit();
                let placed = if roll < 0.24 && !rural {
                    self.street_clear(p, 0.5) && {
                        self.lamp_post(p, face);
                        true
                    }
                } else if roll < 0.32 {
                    self.street_clear(p, 0.4) && {
                        self.hydrant(p);
                        true
                    }
                } else if roll < 0.40 {
                    self.street_clear(p, 0.5) && {
                        self.street_sign(p, dir, &mut rng);
                        true
                    }
                } else if roll < 0.47 && !rural {
                    let q = p + normal * side * 0.3;
                    self.street_clear(q, 1.0) && {
                        self.planter(q, dir, &mut rng);
                        true
                    }
                } else if roll < 0.53 {
                    self.street_clear(p, 1.0) && {
                        self.bike(p, dir, &mut rng);
                        true
                    }
                } else if roll < 0.58 && !rural {
                    self.street_clear(p, 0.9) && {
                        for s in [-0.7, 0.0, 0.7] {
                            self.bollard(p + dir * s);
                        }
                        true
                    }
                } else if roll < 0.61 && !rural {
                    let q = p + normal * side * 0.6;
                    self.street_clear(q, 1.0) && {
                        self.phone_booth(q, face);
                        true
                    }
                } else if roll < 0.64 {
                    let q = p + normal * side * 0.6;
                    self.street_clear(q, 0.9) && {
                        let glow = *rng.pick(&[Paint::Glow, Paint::Blue, Paint::Green]).unwrap_or(&Paint::Glow);
                        let g = self.lowest(q, Vec2::splat(0.5));
                        let from = self.blocks.len();
                        self.vending(q, g, glow);
                        self.mark_furniture(from);
                        self.vending_detail(q, face, g);
                        true
                    }
                } else if roll < 0.67 && w >= 6.0 {
                    let q = p + normal * side * 3.5;
                    self.street_clear(q, 3.6) && {
                        self.billboard(q, face, &mut rng);
                        true
                    }
                } else {
                    false
                };
                if placed {
                    self.street_spots.push(p);
                }
                t += rng.range(6.0, 11.0);
            }
        }
    }

    /// A timber power pole with a cross-arm and insulators; returns the middle of its wires.
    fn power_pole(&mut self, p: Vec2, along: Vec2) -> Vec3 {
        let g = self.height(p.x, p.y);
        let top = g + 7.6;
        self.trim_rod(Vec3::new(p.x, g - 0.2, p.y), Vec3::new(p.x, top, p.y), 0.13, Paint::Wood);
        let arm = Vec3::new(along.y, 0.0, -along.x);
        let c = Vec3::new(p.x, top - 0.45, p.y);
        self.trim_bar(c - arm * 1.05, c + arm * 1.05, 0.12, Paint::Wood);
        for o in [-0.8, 0.0, 0.8] {
            let q = c + arm * o + Vec3::Y * 0.12;
            self.trim_part(Shape::Cylinder, Paint::Cream, q, Vec3::new(0.09, 0.16, 0.09), Quat::IDENTITY);
        }
        // A transformer drum on some.
        if (p.x * 7.0 + p.y * 3.0).sin() > 0.6 {
            self.lathe(p + Vec2::new(arm.x, arm.z) * 0.32, &[(0.26, top - 2.2), (0.26, top - 1.3), (0.2, top - 1.22)], Paint::Metal, 10);
        }
        Vec3::new(p.x, top - 0.33, p.y)
    }

    /// A street lamp: a pole with an arm reaching over the road and a glowing head.
    fn lamp_post(&mut self, p: Vec2, face: Vec2) {
        let g = self.height(p.x, p.y);
        self.trim_part(Shape::Cylinder, Paint::Dark, Vec3::new(p.x, g + 0.25, p.y), Vec3::new(0.26, 0.5, 0.26), Quat::IDENTITY);
        self.trim_rod(Vec3::new(p.x, g, p.y), Vec3::new(p.x, g + 4.6, p.y), 0.06, Paint::Dark);
        let reach = Vec3::new(face.x, 0.0, face.y) * 1.1;
        let top = Vec3::new(p.x, g + 4.6, p.y);
        self.trim_rod(top, top + reach + Vec3::Y * 0.25, 0.045, Paint::Dark);
        let head = top + reach + Vec3::Y * 0.2;
        self.trim_part(Shape::Cube, Paint::Dark, head, Vec3::new(0.5, 0.12, 0.3) * if face.x.abs() > 0.5 { Vec3::ONE } else { Vec3::new(0.6, 1.0, 1.66) }, Quat::IDENTITY);
        self.trim_part(Shape::Cube, Paint::Glow, head - Vec3::Y * 0.08, Vec3::new(0.36, 0.04, 0.22) * if face.x.abs() > 0.5 { Vec3::ONE } else { Vec3::new(0.6, 1.0, 1.66) }, Quat::IDENTITY);
    }

    /// A fire hydrant (too small to hide behind).
    fn hydrant(&mut self, p: Vec2) {
        let g = self.height(p.x, p.y);
        self.lathe(p, &[(0.17, g - 0.05), (0.17, g + 0.08), (0.13, g + 0.1), (0.13, g + 0.55), (0.15, g + 0.58), (0.1, g + 0.72), (0.0, g + 0.78)], Paint::Mustard, 10);
        for d in [Vec3::X, Vec3::NEG_X] {
            let c = Vec3::new(p.x, g + 0.4, p.y);
            self.trim_rod(c, c + d * 0.22, 0.06, Paint::Cream);
        }
    }

    /// A street sign: a post and a plain coloured plate (an arrow or a bar, no words).
    fn street_sign(&mut self, p: Vec2, along: Vec2, rng: &mut Rng) {
        let g = self.height(p.x, p.y);
        self.trim_rod(Vec3::new(p.x, g, p.y), Vec3::new(p.x, g + 2.6, p.y), 0.035, Paint::Metal);
        let flat = Vec3::new(along.x.abs(), 0.0, along.y.abs());
        let plate = |w: f32| Vec3::new(0.02, 0.0, 0.02) + flat * w;
        let colour = *rng.pick(&[Paint::Navy, Paint::Lino, Paint::Mustard, Paint::Coral]).unwrap_or(&Paint::Navy);
        self.trim_part(Shape::Cube, colour, Vec3::new(p.x, g + 2.35, p.y), plate(0.7) + Vec3::Y * 0.36, Quat::IDENTITY);
        self.trim_part(Shape::Cube, Paint::Cream, Vec3::new(p.x, g + 2.35, p.y), plate(0.5) * Vec3::new(1.2, 1.0, 1.2) + Vec3::Y * 0.06, Quat::IDENTITY);
        if rng.chance(0.5) {
            // A round sign under it.
            let disc = Quat::from_rotation_arc(Vec3::Y, Vec3::new(along.y, 0.0, -along.x));
            self.trim_part(Shape::Cylinder, Paint::Cream, Vec3::new(p.x, g + 1.75, p.y), Vec3::new(0.5, 0.025, 0.5), disc);
            self.trim_part(Shape::Cylinder, Paint::Carpet, Vec3::new(p.x, g + 1.75, p.y), Vec3::new(0.42, 0.03, 0.42), disc);
        }
    }

    /// A planter box with flowers or a shrub: solid, waist-low cover.
    fn planter(&mut self, p: Vec2, along: Vec2, rng: &mut Rng) {
        let half = if along.x.abs() > 0.5 { Vec2::new(0.9, 0.4) } else { Vec2::new(0.4, 0.9) };
        let g = self.lowest(p, half);
        let from = self.blocks.len();
        self.solid(Aabb::span(Vec3::new(p.x - half.x, g - 0.1, p.y - half.y), Vec3::new(p.x + half.x, g + 0.55, p.y + half.y)), Paint::Concrete);
        self.mark_furniture(from);
        self.trim(Vec3::new(p.x - half.x + 0.08, g + 0.55, p.y - half.y + 0.08), Vec3::new(p.x + half.x - 0.08, g + 0.6, p.y + half.y - 0.08), Paint::Wood);
        for s in [-0.5, 0.0, 0.5] {
            let q = p + along * s * (half.x.max(half.y) * 1.2);
            let colour = *rng.pick(&[Paint::Coral, Paint::Mustard, Paint::Purple, Paint::Cream]).unwrap_or(&Paint::Coral);
            self.trim_part(Shape::Shard, Paint::Leaf, Vec3::new(q.x, g + 0.75, q.y), Vec3::new(0.5, 0.35, 0.45), Quat::from_rotation_y(rng.range(0.0, TAU)));
            for _ in 0..3 {
                let f = q + Vec2::new(rng.range(-0.2, 0.2), rng.range(-0.2, 0.2));
                self.trim_part(Shape::Shard, colour, Vec3::new(f.x, g + 0.95, f.y), Vec3::splat(0.1), Quat::IDENTITY);
            }
        }
    }

    /// A bicycle leaning on its stand.
    fn bike(&mut self, p: Vec2, along: Vec2, rng: &mut Rng) {
        let g = self.height(p.x, p.y);
        let d = Vec3::new(along.x, 0.0, along.y);
        let side = Vec3::new(along.y, 0.0, -along.x);
        let frame = *rng.pick(&[Paint::Coral, Paint::Teal, Paint::Mustard, Paint::Navy, Paint::Lino]).unwrap_or(&Paint::Teal);
        let c = Vec3::new(p.x, g, p.y);
        let wheel = Quat::from_rotation_arc(Vec3::Y, side);
        for s in [-0.52, 0.52] {
            self.trim_part(Shape::Cylinder, Paint::Dark, c + d * s + Vec3::Y * 0.34, Vec3::new(0.66, 0.04, 0.66), wheel);
            self.trim_part(Shape::Cylinder, Paint::Metal, c + d * s + Vec3::Y * 0.34, Vec3::new(0.1, 0.05, 0.1), wheel);
        }
        let (rear, front, seat, bars, crank) = (c + d * -0.52 + Vec3::Y * 0.34, c + d * 0.52 + Vec3::Y * 0.34, c + d * -0.2 + Vec3::Y * 0.85, c + d * 0.42 + Vec3::Y * 0.95, c + Vec3::Y * 0.36);
        for (x, y) in [(rear, seat), (seat, crank), (crank, rear), (seat, bars), (bars, crank), (bars, front)] {
            self.trim_rod(x, y, 0.022, frame);
        }
        self.trim_part(Shape::Cube, Paint::Dark, seat + Vec3::Y * 0.04, Vec3::new(0.12, 0.04, 0.12) + Vec3::new(d.x.abs(), 0.0, d.z.abs()) * 0.12, Quat::IDENTITY);
        self.trim_rod(bars - side * 0.25, bars + side * 0.25, 0.02, Paint::Dark);
    }

    /// A short bollard.
    fn bollard(&mut self, p: Vec2) {
        let g = self.height(p.x, p.y);
        self.lathe(p, &[(0.11, g - 0.05), (0.11, g + 0.8), (0.07, g + 0.88), (0.0, g + 0.9)], Paint::Dark, 8);
        self.round(p, 0.115, g + 0.62, g + 0.7, Paint::Cream, 8);
    }

    /// A phone booth: a solid box of a booth with glazed sides, a lit sign band and a handset.
    fn phone_booth(&mut self, p: Vec2, face: Vec2) {
        let g = self.lowest(p, Vec2::splat(0.5));
        let from = self.blocks.len();
        // Solid as a whole (cover), drawn as a frame round glass.
        self.blocks.push(super::Block { aabb: Aabb::span(Vec3::new(p.x - 0.5, g - 0.05, p.y - 0.5), Vec3::new(p.x + 0.5, g + 2.3, p.y + 0.5)), paint: Paint::Carpet, solid: true, hidden: true });
        self.mark_furniture(from);
        let base = Vec3::new(p.x, g, p.y);
        self.trim_at(base + Vec3::Y * 0.06, Vec3::new(0.52, 0.06, 0.52), Paint::Dark);
        self.trim_at(base + Vec3::Y * 2.28, Vec3::new(0.54, 0.06, 0.54), Paint::Carpet);
        self.trim_at(base + Vec3::Y * 2.12, Vec3::new(0.52, 0.1, 0.52), Paint::Cream);
        for (x, z) in [(-0.46, -0.46), (0.46, -0.46), (-0.46, 0.46), (0.46, 0.46)] {
            self.trim_at(base + Vec3::new(x, 1.1, z), Vec3::new(0.05, 1.05, 0.05), Paint::Carpet);
        }
        for (n, h) in [(Vec3::X, Vec3::new(0.02, 0.95, 0.42)), (Vec3::NEG_X, Vec3::new(0.02, 0.95, 0.42)), (Vec3::Z, Vec3::new(0.42, 0.95, 0.02)), (Vec3::NEG_Z, Vec3::new(0.42, 0.95, 0.02))] {
            self.trim_at(base + n * 0.47 + Vec3::Y * 1.12, h, Paint::Tile);
            self.trim_at(base + n * 0.475 + Vec3::Y * 0.6, h * Vec3::new(1.0, 0.03, 1.0) + Vec3::new(0.005, 0.0, 0.005), Paint::Carpet);
        }
        let back = base - Vec3::new(face.x, 0.0, face.y) * 0.35 + Vec3::Y * 1.3;
        self.trim_at(back, Vec3::new(0.15, 0.22, 0.15), Paint::Dark);
        self.trim_at(back + Vec3::Y * 0.1 + Vec3::new(face.x, 0.0, face.y) * 0.12, Vec3::new(0.06, 0.03, 0.06), Paint::Glow);
    }

    /// Detail on a vending machine (see `shapes::vending`): coin slot, tray, a stripe of art.
    fn vending_detail(&mut self, p: Vec2, face: Vec2, g: f32) {
        let front = Vec3::new(p.x, g, p.y) + Vec3::new(face.x, 0.0, face.y) * 0.41;
        let across = Vec3::new(-face.y, 0.0, face.x);
        let flat = |w: f32, h: f32| Vec3::new(across.x.abs() * w + 0.012, h, across.z.abs() * w + 0.012);
        self.trim_at(front + Vec3::Y * 0.35, flat(0.3, 0.1), Paint::Dark);
        self.trim_at(front + across * 0.36 + Vec3::Y * 1.2, flat(0.05, 0.1), Paint::Metal);
        self.trim_at(front + Vec3::Y * 0.55, flat(0.4, 0.05), Paint::Mustard);
        for k in 0..3 {
            self.trim_part(Shape::Cylinder, Paint::Cream, front + across * (-0.2 + 0.2 * k as f32) + Vec3::Y * 1.0, Vec3::new(0.08, 0.2, 0.08), Quat::IDENTITY);
        }
    }

    /// A billboard on two legs, facing the street, with a painted picture: hills under a
    /// big sun, a smiling robot - original art, no words, no brands.
    fn billboard(&mut self, p: Vec2, face: Vec2, rng: &mut Rng) {
        let g = self.height(p.x, p.y);
        let across = Vec3::new(-face.y, 0.0, face.x);
        let f = Vec3::new(face.x, 0.0, face.y);
        let base = Vec3::new(p.x, g, p.y);
        for s in [-1.8, 1.8] {
            self.trim_rod(base + across * s, base + across * s + Vec3::Y * 6.6, 0.12, Paint::Iron);
        }
        let flat = |w: f32, h: f32, d: f32| Vec3::new(across.x.abs() * w + f.x.abs() * d, h, across.z.abs() * w + f.z.abs() * d);
        let centre = base + Vec3::Y * 5.0;
        self.trim_at(centre, flat(3.2, 1.6, 0.1), Paint::Dark);
        let sky = *rng.pick(&[Paint::Navy, Paint::Teal, Paint::Mustard, Paint::Coral]).unwrap_or(&Paint::Teal);
        self.trim_at(centre + f * 0.06, flat(3.0, 1.42, 0.02), sky);
        // Rolling hills.
        for (x, w, h, paint) in [(-1.6f32, 1.6f32, 0.55, Paint::Lino), (0.4, 2.0, 0.42, Paint::Leaf), (2.0, 1.0, 0.6, Paint::Lino)] {
            let c = centre + f * 0.08 + across * x + Vec3::Y * (-1.42 + h);
            self.trim_at(c, flat(w.min(3.0 - x.abs()).max(0.4), h, 0.02), paint);
        }
        // The sun, and a robot waving.
        let sun = Quat::from_rotation_arc(Vec3::Y, f);
        self.trim_part(Shape::Cylinder, Paint::Cream, centre + f * 0.09 + across * 1.6 + Vec3::Y * 0.6, Vec3::new(0.8, 0.02, 0.8), sun);
        let bot = centre + f * 0.1 + across * -0.6;
        self.trim_at(bot + Vec3::Y * 0.1, flat(0.32, 0.38, 0.02), Paint::Steel);
        self.trim_at(bot + Vec3::Y * 0.65, flat(0.26, 0.2, 0.02), Paint::Steel);
        self.trim_at(bot + Vec3::Y * 0.68 + f * 0.01 + across * 0.1, flat(0.05, 0.05, 0.02), Paint::Mustard);
        self.trim_at(bot + Vec3::Y * 0.68 + f * 0.01 - across * 0.1, flat(0.05, 0.05, 0.02), Paint::Mustard);
        self.trim_part(Shape::Cube, Paint::Steel, bot + across * 0.48 + Vec3::Y * 0.45, flat(0.06, 0.32, 0.02), Quat::from_axis_angle(f, -0.6));
        // A lamp over it.
        self.trim_rod(centre + Vec3::Y * 1.6 + f * 0.05, centre + Vec3::Y * 1.9 + f * 0.6, 0.03, Paint::Dark);
        self.trim_at(centre + Vec3::Y * 1.9 + f * 0.6, flat(0.5, 0.05, 0.12), Paint::Glow);
    }

    /// A doorway dressed by what is behind it: a canopy and a lamp over a home's door, an awning
    /// and a sign with a picture on it over a shop's; a mat on the step either way.
    pub(crate) fn dress_door(&mut self, p: Vec2, out: Vec2, width: f32, floor: f32, furnish: super::shapes::Furnish, walls: Paint) {
        let mut rng = self.detail_rng(p, 0xd00);
        let o = Vec3::new(out.x, 0.0, out.y);
        let along = Vec3::new(-out.y, 0.0, out.x);
        let flat = |w: f32, h: f32, d: f32| Vec3::new(along.x.abs() * w + o.x.abs() * d, h, along.z.abs() * w + o.z.abs() * d);
        let face = Vec3::new(p.x, floor, p.y);
        // Door frame.
        for s in [-1.0, 1.0] {
            self.trim_at(face + o * 0.03 + along * s * (width * 0.5 + 0.06) + Vec3::Y * 1.2, flat(0.06, 1.2, 0.04), darker(walls));
        }
        // A mat on the step.
        let step = self.height(p.x + out.x * 0.7, p.y + out.y * 0.7);
        if (step - floor).abs() < 0.3 {
            self.trim_at(Vec3::new(p.x, floor.max(step) + 0.02, p.y) + o * 0.55, flat(0.5, 0.01, 0.3), Paint::Wood);
        }
        match furnish {
            super::shapes::Furnish::Home => {
                // A little canopy on brackets and a lamp beside the door.
                let top = face + Vec3::Y * 2.62 + o * 0.45;
                self.trim_at(top, flat(width * 0.5 + 0.35, 0.05, 0.48), darker(walls));
                for s in [-1.0, 1.0] {
                    self.trim_rod(face + along * s * (width * 0.5 + 0.25) + Vec3::Y * 2.2 + o * 0.02, top + along * s * (width * 0.5 + 0.25) - Vec3::Y * 0.04 + o * 0.3, 0.025, Paint::Dark);
                }
                let lamp = face + along * (width * 0.5 + 0.4) + Vec3::Y * 2.0 + o * 0.1;
                self.trim_at(lamp, Vec3::splat(0.08), Paint::Dark);
                self.trim_at(lamp - Vec3::Y * 0.02 + o * 0.03, Vec3::splat(0.05), Paint::Glow);
                // A house number plate.
                self.trim_at(face + o * 0.02 - along * (width * 0.5 + 0.35) + Vec3::Y * 1.7, flat(0.12, 0.08, 0.01), Paint::Cream);
            }
            super::shapes::Furnish::Plain if rng.chance(0.7) => {
                // A striped shop awning sloping out over the door.
                let stripes = *rng.pick(&[(Paint::Coral, Paint::Cream), (Paint::Lino, Paint::Cream), (Paint::Navy, Paint::Mustard), (Paint::Carpet, Paint::Cream)]).unwrap_or(&(Paint::Coral, Paint::Cream));
                let span = width * 0.5 + 0.9;
                let n = ((span * 2.0) / 0.4).round().max(2.0) as usize;
                let tilt = Quat::from_axis_angle(along, -0.38 * if (out.x + out.y) > 0.0 { 1.0 } else { -1.0 } * if out.x.abs() > 0.5 { -1.0 } else { 1.0 });
                for i in 0..n {
                    let u = -span + (i as f32 + 0.5) * span * 2.0 / n as f32;
                    let c = face + along * u + o * 0.6 + Vec3::Y * 2.78;
                    self.trim_part(Shape::Cube, if i % 2 == 0 { stripes.0 } else { stripes.1 }, c, flat(span / n as f32 + 0.005, 0.03, 0.62) * 2.0, tilt);
                }
                self.trim_at(face + o * 1.15 + Vec3::Y * 2.56, flat(span, 0.08, 0.02), stripes.0);
                // The sign: a board over the awning with a picture (a cup, a wrench, a loaf...).
                let board = face + o * 0.05 + Vec3::Y * 3.2;
                self.trim_at(board, flat(1.1, 0.28, 0.04), stripes.1);
                self.trim_at(board + o * 0.03, flat(1.0, 0.22, 0.02), stripes.0);
                let icon = board + o * 0.05;
                match rng.below(4) {
                    0 => {
                        self.trim_part(Shape::Cylinder, Paint::Cream, icon, Vec3::new(0.22, 0.04, 0.22), Quat::from_rotation_arc(Vec3::Y, o));
                        self.trim_at(icon + along * 0.15, flat(0.04, 0.06, 0.02), Paint::Cream);
                    }
                    1 => {
                        self.trim_part(Shape::Cube, Paint::Cream, icon, flat(0.3, 0.04, 0.02), Quat::from_axis_angle(o, 0.5));
                        self.trim_part(Shape::Cylinder, Paint::Cream, icon + along * 0.28 + Vec3::Y * 0.14, Vec3::new(0.12, 0.04, 0.12), Quat::from_rotation_arc(Vec3::Y, o));
                    }
                    2 => self.trim_part(Shape::Shard, Paint::Mustard, icon, flat(0.3, 0.13, 0.04), Quat::IDENTITY),
                    _ => {
                        self.trim_part(Shape::Cylinder, Paint::Cream, icon, Vec3::new(0.24, 0.04, 0.24), Quat::from_rotation_arc(Vec3::Y, o));
                        self.trim_part(Shape::Cylinder, stripes.0, icon + o * 0.01, Vec3::new(0.1, 0.04, 0.1), Quat::from_rotation_arc(Vec3::Y, o));
                    }
                }
            }
            _ => {
                // A plain lamp over the door.
                self.trim_at(face + o * 0.12 + Vec3::Y * 2.55, flat(0.18, 0.05, 0.12), Paint::Dark);
                self.trim_at(face + o * 0.12 + Vec3::Y * 2.5, flat(0.12, 0.02, 0.08), Paint::Glow);
            }
        }
    }

    /// Rooftop clutter on a flat roof at `y`: vent stacks, a pipe run, an aerial or a dish.
    pub(crate) fn dress_roof(&mut self, c: Vec2, half: Vec2, y: f32) {
        let mut rng = self.detail_rng(c, 0x200f);
        for _ in 0..2 {
            let p = c + Vec2::new(rng.range(-half.x + 0.8, half.x - 0.8), rng.range(-half.y + 0.8, half.y - 0.8));
            self.lathe(p, &[(0.14, y), (0.14, y + 0.7), (0.24, y + 0.74), (0.24, y + 0.8), (0.0, y + 0.92)], Paint::Metal, 8);
        }
        let a = c + Vec2::new(-half.x + 0.7, rng.range(-half.y + 0.7, half.y - 0.7));
        let b = Vec2::new(c.x + half.x - 0.7, a.y);
        self.trim_rod(Vec3::new(a.x, y + 0.22, a.y), Vec3::new(b.x, y + 0.22, b.y), 0.07, Paint::Iron);
        for t in [0.2, 0.5, 0.8] {
            let q = a.lerp(b, t);
            self.trim_at(Vec3::new(q.x, y + 0.08, q.y), Vec3::new(0.06, 0.08, 0.12), Paint::Dark);
        }
        if rng.chance(0.4) {
            let d = c + Vec2::new(rng.range(-half.x + 1.0, half.x - 1.0), rng.range(-half.y + 1.0, half.y - 1.0));
            self.trim_rod(Vec3::new(d.x, y, d.y), Vec3::new(d.x, y + 0.6, d.y), 0.04, Paint::Dark);
            self.trim_part(Shape::Cylinder, Paint::Cream, Vec3::new(d.x, y + 0.75, d.y), Vec3::new(0.7, 0.06, 0.7), Quat::from_rotation_x(0.9) * Quat::from_rotation_y(rng.range(0.0, TAU)));
        }
    }

    /// A market stall: a solid counter (cover) under a striped canopy on four poles, with crates
    /// of produce on it.
    pub(crate) fn market_stall(&mut self, p: Vec2, face: Vec2, rng: &mut Rng) {
        let along = Vec2::new(-face.y, face.x);
        let half = if face.x.abs() > 0.5 { Vec2::new(0.45, 1.3) } else { Vec2::new(1.3, 0.45) };
        let g = self.lowest(p, half + Vec2::splat(0.4));
        let from = self.blocks.len();
        self.solid(Aabb::span(Vec3::new(p.x - half.x, g - 0.1, p.y - half.y), Vec3::new(p.x + half.x, g + 0.95, p.y + half.y)), Paint::Wood);
        self.mark_furniture(from);
        let top = g + 0.96;
        let colours = *rng.pick(&[(Paint::Coral, Paint::Cream), (Paint::Lino, Paint::Cream), (Paint::Mustard, Paint::Navy), (Paint::Teal, Paint::Cream)]).unwrap_or(&(Paint::Coral, Paint::Cream));
        for s in [-0.9, 0.0, 0.9] {
            let q = p + along * s;
            self.trim_at(Vec3::new(q.x, top + 0.1, q.y), Vec3::new(0.25 + 0.1 * face.x.abs(), 0.1, 0.25 + 0.1 * face.y.abs()), Paint::Planks);
            let fruit = *rng.pick(&[Paint::Red, Paint::Mustard, Paint::LeafLight, Paint::Coral, Paint::Purple]).unwrap_or(&Paint::Mustard);
            let fruit = if fruit.unlit() { Paint::Coral } else { fruit };
            for k in 0..4 {
                let f = q + Vec2::from_angle(k as f32 * FRAC_PI_2 + 0.4) * 0.12;
                self.trim_part(Shape::Shard, fruit, Vec3::new(f.x, top + 0.24, f.y), Vec3::splat(0.14), Quat::IDENTITY);
            }
        }
        let back = p - face * 1.1;
        for (s, h) in [(-1.35, 2.5), (1.35, 2.5)] {
            for (q, hh) in [(p + face * 0.6 + along * s, h - 0.3), (back + along * s, h)] {
                self.trim_rod(Vec3::new(q.x, g, q.y), Vec3::new(q.x, g + hh, q.y), 0.04, Paint::Wood);
            }
        }
        let n = 7;
        let tilt = Quat::from_axis_angle(Vec3::new(along.x, 0.0, along.y), 0.25);
        let tilt = if (face.x + face.y) > 0.0 { tilt } else { tilt.inverse() };
        for i in 0..n {
            let u = -1.45 + (i as f32 + 0.5) * 2.9 / n as f32;
            let q = p - face * 0.25 + along * u;
            let size = Vec3::new(along.x.abs() * 2.9 / n as f32 + face.x.abs() * 1.9 + 0.01, 0.03, along.y.abs() * 2.9 / n as f32 + face.y.abs() * 1.9 + 0.01);
            let tilt = if face.x.abs() > 0.5 { Quat::from_axis_angle(Vec3::Z, 0.25 * face.x) } else { tilt };
            self.trim_part(Shape::Cube, if i % 2 == 0 { colours.0 } else { colours.1 }, Vec3::new(q.x, g + 2.35, q.y), size, tilt);
        }
    }

    /// The rides of a playground at `c`: swings on chains, a slide's chute, a see-saw, a spring rider.
    pub(crate) fn playground_rides(&mut self, c: Vec2, y: f32) {
        // Swing seats on chains from the swing frame's beam (see the park).
        for k in 0..2 {
            let x = c.x + 1.8 + k as f32 * 1.2;
            for s in [-0.18, 0.18] {
                self.trim_rod(Vec3::new(x + s, y + 2.22, c.y + 2.43), Vec3::new(x + s, y + 0.55, c.y + 2.43), 0.012, Paint::Iron);
            }
            self.trim_at(Vec3::new(x, y + 0.53, c.y + 2.43), Vec3::new(0.24, 0.025, 0.1), Paint::Dark);
        }
        // A see-saw and a spring rider.
        let s = c + Vec2::new(2.5, -2.0);
        self.trim_part(Shape::Cube, Paint::Mustard, Vec3::new(s.x, y + 0.42, s.y), Vec3::new(2.6, 0.06, 0.24), Quat::from_rotation_z(0.18));
        self.trim_at(Vec3::new(s.x, y + 0.2, s.y), Vec3::new(0.1, 0.2, 0.12), Paint::Dark);
        let r = c + Vec2::new(-1.5, 2.6);
        self.trim_rod(Vec3::new(r.x, y, r.y), Vec3::new(r.x, y + 0.4, r.y), 0.06, Paint::Iron);
        self.trim_part(Shape::Shard, Paint::Lino, Vec3::new(r.x, y + 0.6, r.y), Vec3::new(0.35, 0.3, 0.6), Quat::IDENTITY);
        self.trim_part(Shape::Shard, Paint::Lino, Vec3::new(r.x, y + 0.85, r.y - 0.25), Vec3::new(0.2, 0.25, 0.2), Quat::IDENTITY);
    }

    /// A construction site in an empty lot at `c`: a site fence of panels, a scaffold up a
    /// half-built wall, a heap of sand, concrete pipes (cover), cones, a skip.
    pub(crate) fn construction_site(&mut self, c: Vec2) {
        let mut rng = self.detail_rng(c, 0xc0de);
        let g = self.height(c.x, c.y);
        let half = Vec2::new(7.0, 5.5);
        // A fence of mesh panels round three sides (thin), with a gap to walk in.
        for (a, b) in [(Vec2::new(-1.0, -1.0), Vec2::new(1.0, -1.0)), (Vec2::new(-1.0, -1.0), Vec2::new(-1.0, 1.0)), (Vec2::new(1.0, -1.0), Vec2::new(1.0, 1.0)), (Vec2::new(-1.0, 1.0), Vec2::new(-0.2, 1.0))] {
            let (p, q) = (c + a * half, c + b * half);
            let n = (p.distance(q) / 2.2).ceil() as usize;
            for i in 0..n {
                let (u, v) = (p.lerp(q, i as f32 / n as f32), p.lerp(q, (i + 1) as f32 / n as f32));
                let (gu, gv) = (self.height(u.x, u.y), self.height(v.x, v.y));
                self.trim_rod(Vec3::new(u.x, gu, u.y), Vec3::new(u.x, gu + 2.0, u.y), 0.03, Paint::Metal);
                let mid = (u + v) * 0.5;
                let along_x = (q.x - p.x).abs() > (q.y - p.y).abs();
                let len = u.distance(v) * 0.5;
                let h = if along_x { Vec3::new(len, 0.9, 0.01) } else { Vec3::new(0.01, 0.9, len) };
                self.trim_at(Vec3::new(mid.x, (gu + gv) * 0.5 + 1.05, mid.y), h, Paint::Metal);
                self.trim_at(Vec3::new(mid.x, (gu + gv) * 0.5 + 0.12, mid.y), h * Vec3::new(1.0, 0.1, 1.0) + Vec3::new(0.02, 0.0, 0.02), Paint::Mustard);
            }
        }
        // A half-built wall of block, solid, with scaffolding along it.
        let wall = c + Vec2::new(0.0, 3.0);
        let from = self.blocks.len();
        self.solid(Aabb::span(Vec3::new(wall.x - 4.0, g - 0.2, wall.y - 0.15), Vec3::new(wall.x + 4.0, g + 2.4, wall.y + 0.15)), Paint::Concrete);
        self.solid(Aabb::span(Vec3::new(wall.x - 4.0, g + 2.4, wall.y - 0.15), Vec3::new(wall.x + 0.5, g + 3.4, wall.y + 0.15)), Paint::Concrete);
        self.mark_furniture(from);
        for k in 0..6 {
            let x = wall.x - 4.0 + k as f32 * 1.6;
            for z in [wall.y - 0.5, wall.y - 1.6] {
                self.trim_rod(Vec3::new(x, g, z), Vec3::new(x, g + 4.0, z), 0.04, Paint::Mustard);
            }
        }
        for lvl in [1.4, 2.8] {
            for z in [wall.y - 0.5, wall.y - 1.6] {
                self.trim_rod(Vec3::new(wall.x - 4.0, g + lvl, z), Vec3::new(wall.x + 4.0, g + lvl, z), 0.035, Paint::Mustard);
            }
            self.trim(Vec3::new(wall.x - 4.0, g + lvl + 0.04, wall.y - 1.6), Vec3::new(wall.x + 4.0, g + lvl + 0.08, wall.y - 0.5), Paint::Planks);
        }
        // A heap of sand.
        let heap = c + Vec2::new(-4.0, -2.5);
        self.lathe(heap, &[(1.6, g - 0.1), (0.9, g + 0.6), (0.0, g + 1.0)], Paint::Sand, 10);
        // Concrete pipes on their sides: hidden cover boxes, drawn round.
        for k in 0..2 {
            let p = c + Vec2::new(2.5 + k as f32 * 1.4, -2.5);
            let gp = self.lowest(p, Vec2::splat(0.65));
            let r = 0.62;
            let from = self.blocks.len();
            self.blocks.push(super::Block { aabb: Aabb::span(Vec3::new(p.x - r, gp - 0.05, p.y - 1.1), Vec3::new(p.x + r, gp + 2.0 * r, p.y + 1.1)), paint: Paint::Concrete, solid: true, hidden: true });
            self.mark_furniture(from);
            let mut points = Vec::new();
            for z in [p.y - 1.1, p.y + 1.1] {
                for i in 0..14 {
                    let a = (i as f32 + 0.5) * TAU / 14.0;
                    points.push(Vec3::new(p.x + a.cos() * r, gp + r + a.sin() * r, z));
                }
            }
            self.roofs.push(Hull { paint: Paint::Concrete, points, anim: Anim::None });
            self.trim_part(Shape::Cylinder, Paint::Dark, Vec3::new(p.x, gp + r, p.y - 1.1), Vec3::new(r * 1.4, 0.02, r * 1.4), Quat::from_rotation_x(FRAC_PI_2));
        }
        // Traffic cones by the gap.
        for k in 0..4 {
            let p = c + Vec2::new(-0.6 + k as f32 * 1.1, half.y + 0.8);
            let gp = self.height(p.x, p.y);
            self.trim_at(Vec3::new(p.x, gp + 0.02, p.y), Vec3::new(0.18, 0.02, 0.18), Paint::Dark);
            self.trim_part(Shape::Cone, Paint::Coral, Vec3::new(p.x, gp + 0.3, p.y), Vec3::new(0.3, 0.56, 0.3), Quat::IDENTITY);
            self.trim_part(Shape::Cylinder, Paint::Cream, Vec3::new(p.x, gp + 0.3, p.y), Vec3::new(0.2, 0.06, 0.2), Quat::IDENTITY);
        }
        // A skip.
        let skip = c + Vec2::new(-4.0, 1.0);
        let gs = self.lowest(skip, Vec2::new(1.4, 0.8));
        let from = self.blocks.len();
        self.solid(Aabb::span(Vec3::new(skip.x - 1.4, gs - 0.05, skip.y - 0.8), Vec3::new(skip.x + 1.4, gs + 1.1, skip.y + 0.8)), Paint::Mustard);
        self.mark_furniture(from);
        self.trim(Vec3::new(skip.x - 1.3, gs + 1.1, skip.y - 0.7), Vec3::new(skip.x + 1.3, gs + 1.13, skip.y + 0.7), Paint::Dark);
        for _ in 0..5 {
            let q = skip + Vec2::new(rng.range(-1.0, 1.0), rng.range(-0.5, 0.5));
            self.trim_part(Shape::Shard, Paint::Rock, Vec3::new(q.x, gs + 1.2, q.y), Vec3::splat(rng.range(0.2, 0.4)), Quat::IDENTITY);
        }
    }

    /// The set pieces a town has room for: a market round the fountain square, a construction
    /// site in an empty lot, rides in the playground.
    pub(crate) fn set_pieces(&mut self) {
        if self.horde.is_some() || self.recipe.layout == super::Layout::SpaceStation {
            return;
        }
        let mut rng = self.detail_rng(Vec2::new(11.0, -7.0), 0x5e7);
        if let Some(f) = self.fountain {
            let c = Vec2::new(f.x, f.z);
            let mut placed = 0;
            for k in 0..12 {
                if placed >= 3 {
                    break;
                }
                let a = k as f32 * TAU / 12.0 + 0.3;
                let p = c + Vec2::from_angle(a) * 11.5;
                let face = (c - p).normalize_or_zero();
                let face = if face.x.abs() > face.y.abs() { Vec2::new(face.x.signum(), 0.0) } else { Vec2::new(0.0, face.y.signum()) };
                if self.street_clear(p, 2.0) && !self.near_road(p, 0.5) {
                    self.market_stall(p, face, &mut rng);
                    self.street_spots.push(p);
                    placed += 1;
                }
            }
        }
        // One construction site, in the first empty lot found.
        if self.theme.is_some() || self.recipe.layout.is_grid_town() || self.recipe.layout == super::Layout::Crossroads {
            for _ in 0..200 {
                let c = Vec2::new(rng.range(-self.arena * 0.8, self.arena * 0.8), rng.range(-self.arena * 0.8, self.arena * 0.8));
                let flat = self.highest(c, Vec2::new(8.0, 6.5)) - self.lowest(c, Vec2::new(8.0, 6.5)) < 0.6;
                if flat && self.street_clear(c, 9.0) && !self.near_road(c, 7.0) {
                    self.construction_site(c);
                    self.street_spots.push(c);
                    break;
                }
            }
        }
        let _ = interiors::FLOOR_CONTRAST;
    }
}
