//! The random town. Every seed rolls a [`Theme`] that picks the street pattern, the kinds of
//! building, the paint, the cover and the greenery, so the whole map belongs together:
//!
//! * **Suburbs** - a main avenue, side streets ending in cul-de-sacs, houses (some L-shaped,
//!   some two storeys with balconies) with garages, driveways and fenced yards, a park, mansions.
//! * **Desert outpost** - a walled fort with a wall-walk, walled compounds of adobe huts,
//!   a water tower, sandbag nests, dunes and an oasis: sparse cover, long sightlines.
//! * **Farmland** - a farmstead (barn with hayloft, grain silos, farmhouse, windmill, machine
//!   shed), smaller farms, and fields between stone walls and fences: corn to hide in, hay to
//!   hide behind.
//! * **Industrial town** - a street grid of warehouses, shacks, tank farms, container yards,
//!   tall blocks and a rail yard round a fountain square.
//! * **Woods park** - dense trees on rolling ground, a lake and a creek with footbridges, a
//!   lodge, cabins, picnic shelters, rock outcrops and a ranger's lookout tower.
//!
//! The playable square is about 224 m across with the team bases at the far ends (x = ±104).

use super::{
    COVER_HEIGHT, Clutter, ClutterKind, DOOR_WIDTH, Prop, PropKind, World,
    shapes::{Furnish, Plan, Roof, Style},
};
use crate::{collide::Aabb, model::Paint, noise, rng::Rng};
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::f32::consts::TAU;

/// The look of a random town.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Theme {
    Suburbs,
    DesertOutpost,
    Farmland,
    Industrial,
    WoodsPark,
}

/// Where the team bases sit.
pub(crate) const BASE_X: f32 = 104.0;

impl Theme {
    pub const ALL: [Theme; 5] = [Theme::Suburbs, Theme::DesertOutpost, Theme::Farmland, Theme::Industrial, Theme::WoodsPark];

    /// The theme a random town's seed rolls.
    pub fn from_seed(seed: u64) -> Theme {
        Theme::ALL[(Rng::new(seed ^ 0x7E3E_5EED).next_u64() % Theme::ALL.len() as u64) as usize]
    }

    pub fn title(self) -> &'static str {
        match self {
            Theme::Suburbs => "SUBURBS",
            Theme::DesertOutpost => "DESERT OUTPOST",
            Theme::Farmland => "FARMLAND",
            Theme::Industrial => "INDUSTRIAL TOWN",
            Theme::WoodsPark => "WOODS PARK",
        }
    }

    pub fn blurb(self) -> &'static str {
        match self {
            Theme::Suburbs => "Cul-de-sacs, fenced yards, garages and a couple of mansions.",
            Theme::DesertOutpost => "A walled fort, adobe compounds and open dunes.",
            Theme::Farmland => "Barns, silos and fields between stone walls.",
            Theme::Industrial => "Warehouses, tank farms, container yards and a rail yard.",
            Theme::WoodsPark => "Dense woods, a lake and creek, cabins and a lodge.",
        }
    }

    pub(crate) fn style(self) -> Style {
        match self {
            Theme::Suburbs => Style { walls: vec![Paint::Bone, Paint::Sand, Paint::Teal, Paint::Coral, Paint::Concrete, Paint::Shell], roofs: vec![Paint::Dark, Paint::Rust, Paint::Steel, Paint::Shell], gable: 0.6, hip: 0.3 },
            Theme::DesertOutpost => Style { walls: vec![Paint::Sand, Paint::Bone, Paint::Concrete], roofs: vec![Paint::Wood, Paint::Sand], gable: 0.0, hip: 0.0 },
            Theme::Farmland => Style { walls: vec![Paint::Bone, Paint::Rust, Paint::Wood, Paint::Sand], roofs: vec![Paint::Dark, Paint::Rust, Paint::Steel], gable: 0.7, hip: 0.15 },
            Theme::Industrial => Style { walls: vec![Paint::Concrete, Paint::Rust, Paint::Bone, Paint::Teal, Paint::Steel, Paint::Sand, Paint::Iron], roofs: vec![Paint::Rust, Paint::Dark, Paint::Steel], gable: 0.12, hip: 0.0 },
            Theme::WoodsPark => Style { walls: vec![Paint::Wood, Paint::Shell, Paint::Bone], roofs: vec![Paint::Rust, Paint::Dark, Paint::Shell], gable: 0.75, hip: 0.1 },
        }
    }

    /// (shore, grass, rock) ground colours, sRGB.
    fn ground(self) -> [[f32; 3]; 3] {
        match self {
            Theme::Suburbs => [[0.66, 0.62, 0.50], [0.38, 0.60, 0.33], [0.42, 0.45, 0.42]],
            Theme::DesertOutpost => [[0.86, 0.70, 0.46], [0.84, 0.67, 0.43], [0.68, 0.52, 0.37]],
            Theme::Farmland => [[0.60, 0.52, 0.36], [0.47, 0.60, 0.31], [0.48, 0.44, 0.36]],
            Theme::Industrial => [[0.56, 0.53, 0.48], [0.46, 0.49, 0.42], [0.42, 0.43, 0.45]],
            Theme::WoodsPark => [[0.50, 0.44, 0.32], [0.26, 0.46, 0.27], [0.40, 0.42, 0.38]],
        }
    }
}

/// The lake in the woods: centre and radius.
const LAKE: (Vec2, f32) = (Vec2::new(-6.0, 58.0), 19.0);
/// The oasis in the desert.
const OASIS: (Vec2, f32) = (Vec2::new(-36.0, -76.0), 9.0);

/// Where the woods' creek runs at `z` (it flows south out of the lake).
fn creek_x(z: f32) -> f32 {
    LAKE.0.x + 7.0 * (z * 0.045).sin() + 3.0 * (z * 0.11 + 1.0).sin()
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl World {
    /// Ground colours: the theme's (or layout's) own, tinted by the biome.
    pub(super) fn ground_colors(&self) -> ([f32; 3], [f32; 3], [f32; 3]) {
        let look = self.recipe.biome.look();
        let own = match (self.recipe.layout, self.theme) {
            (_, Some(theme)) => theme.ground(),
            (super::Layout::Mountainside, _) => [[0.58, 0.54, 0.44], [0.36, 0.52, 0.32], [0.47, 0.46, 0.46]],
            (super::Layout::OfficeTower, _) => [[0.58, 0.56, 0.52], [0.40, 0.55, 0.36], [0.50, 0.50, 0.50]],
            (super::Layout::SpaceStation, _) => [[0.20, 0.23, 0.28], [0.22, 0.25, 0.30], [0.18, 0.20, 0.24]],
            (super::Layout::CinderWorks, _) => [[0.30, 0.26, 0.24], [0.32, 0.30, 0.26], [0.28, 0.27, 0.27]],
            (super::Layout::Blackfen, _) => [[0.25, 0.27, 0.20], [0.22, 0.30, 0.22], [0.25, 0.28, 0.24]],
            _ => return (look.sand, look.grass, look.high),
        };
        let mix = |a: [f32; 3], b: [f32; 3]| [a[0] * 0.72 + b[0] * 0.28, a[1] * 0.72 + b[1] * 0.28, a[2] * 0.72 + b[2] * 0.28];
        (mix(own[0], look.sand), mix(own[1], look.grass), mix(own[2], look.high))
    }

    /// A square of land, `main_radius` from the middle to the beach on every side.
    pub(super) fn square_land(&self, x: f32, z: f32) -> f32 {
        let seed = self.recipe.seed;
        let d = x.abs().max(z.abs());
        let edge = self.recipe.main_radius + 4.0 * noise::value(seed ^ 0x5a, x * 0.04, z * 0.04);
        let shore = ((edge - d) / 9.0).clamp(-1.0, 1.0);
        shore * (1.35 + 0.08 * noise::value(seed ^ 0x70, x * 0.09, z * 0.09)) - 0.15
    }

    /// Building sites decided before the ground is shaped, so the ground is flat under them.
    pub(super) fn plan_flats(&self) -> Vec<(Vec2, f32)> {
        let mut rng = Rng::new(self.recipe.seed ^ 0x0005_17E5);
        let mut flats = vec![(Vec2::new(-BASE_X, 0.0), 20.0), (Vec2::new(BASE_X, 0.0), 20.0)];
        match self.theme {
            Some(Theme::DesertOutpost) => {
                flats.push((Vec2::ZERO, 30.0));
                // Walled compounds round the fort, mirrored loosely so both sides get cover.
                let spots = [(-52.0, 30.0), (-48.0, -32.0), (52.0, -30.0), (50.0, 32.0), (-78.0, 50.0), (78.0, -50.0), (-22.0, 66.0), (24.0, -64.0), (-80.0, -48.0), (80.0, 48.0)];
                for (x, z) in spots {
                    if rng.chance(0.85) {
                        flats.push((Vec2::new(x + rng.range(-5.0, 5.0), z + rng.range(-5.0, 5.0)), 14.0));
                    }
                }
                flats.push((OASIS.0, OASIS.1 + 10.0));
            }
            Some(Theme::WoodsPark) => {
                // The lodge, the lookout and the cabins' clearings.
                flats.push((Vec2::new(-40.0 + rng.range(-4.0, 4.0), 26.0 + rng.range(-3.0, 3.0)), 18.0));
                flats.push((Vec2::new(42.0 + rng.range(-5.0, 5.0), -40.0 + rng.range(-5.0, 5.0)), 9.0));
                let spots = [(-70.0, -30.0), (-28.0, -58.0), (30.0, 30.0), (66.0, 34.0), (-66.0, 62.0), (64.0, -78.0), (-58.0, -84.0), (22.0, 84.0), (78.0, 4.0), (-82.0, 14.0), (14.0, -20.0)];
                for (x, z) in spots {
                    if rng.chance(0.8) {
                        flats.push((Vec2::new(x + rng.range(-6.0, 6.0), z + rng.range(-6.0, 6.0)), 10.0));
                    }
                }
            }
            _ => {}
        }
        flats
    }

    /// The random town's ground: the square of land, shaped for the theme.
    pub(super) fn themed_height(&self, x: f32, z: f32) -> f32 {
        let seed = self.recipe.seed;
        let base = self.square_land(x, z);
        let p = Vec2::new(x, z);
        let inland = ((self.arena + 6.0 - x.abs().max(z.abs())) / 10.0).clamp(0.0, 1.0);
        // How far from any building site (0 on one, 1 well clear).
        let clear = self.flats.iter().map(|(c, r)| ((p.distance(*c) - r) / 10.0).clamp(0.0, 1.0)).fold(1.0f32, f32::min);
        match self.theme {
            Some(Theme::DesertOutpost) => {
                // Dunes: long ridges across the map, stronger towards the north and south flanks.
                let flank = smoothstep((z.abs() - 20.0) / 40.0) * 0.7 + 0.3;
                let ridge = (x * 0.07 + z * 0.025 + 2.0 * noise::value(seed ^ 0xd0e, x * 0.02, z * 0.02)).sin() * 0.5 + 0.5;
                let roads = smoothstep((z.abs() - 5.0) / 9.0) * smoothstep((x.abs() - 4.0) / 9.0);
                let dunes = (2.4 * ridge * ridge + 0.6 * noise::fbm(seed ^ 0xd1, x * 0.03, z * 0.03, 2)) * flank * roads;
                let mut h = base + dunes.max(0.0) * clear * inland;
                h = self.pond(h, p, OASIS.0, OASIS.1);
                h
            }
            Some(Theme::WoodsPark) => {
                let hills = 1.7 * (noise::fbm(seed ^ 0x411, x * 0.014, z * 0.014, 3) + 0.35).max(0.0) + 0.25 * noise::value(seed ^ 0x412, x * 0.08, z * 0.08);
                // Level by the water, so the banks meet the bridges.
                let creek = if z < LAKE.0.y { (x - creek_x(z)).abs() } else { f32::MAX };
                let wet = smoothstep((creek - 6.0) / 14.0) * smoothstep((p.distance(LAKE.0) - LAKE.1 - 4.0) / 14.0);
                let mut h = base + hills * clear * inland * wet;
                h = self.pond(h, p, LAKE.0, LAKE.1);
                // The creek: a wadeable channel south out of the lake.
                if z < LAKE.0.y - LAKE.1 * 0.5 {
                    let d = (x - creek_x(z)).abs();
                    let bed = -0.85;
                    if d < 7.0 {
                        h = h.min(bed + (h - bed) * smoothstep((d - 2.0) / 4.0));
                    }
                }
                h
            }
            Some(Theme::Suburbs) => base + 0.3 * noise::fbm(seed ^ 0x5b, x * 0.02, z * 0.02, 2) * inland * clear,
            Some(Theme::Farmland) => base + 0.2 * noise::fbm(seed ^ 0xfa, x * 0.015, z * 0.015, 2) * inland,
            _ => base,
        }
    }

    /// A pond dug into the ground: deep in the middle (you cannot walk there), shelving banks.
    fn pond(&self, h: f32, p: Vec2, c: Vec2, r: f32) -> f32 {
        let d = p.distance(c) * (1.0 + 0.08 * noise::value(self.recipe.seed ^ 0x90d, p.x * 0.1, p.y * 0.1));
        if d > r + 7.0 {
            return h;
        }
        let t = (d / r).clamp(0.0, 1.0);
        let bed = -1.6 + 2.5 * t * t;
        if d <= r { bed.min(h) } else { bed + (h - bed) * smoothstep((d - r) / 7.0) }
    }

    /// Build the random town for its theme.
    pub(super) fn themed_town(&mut self, rng: &mut Rng) {
        let theme = self.theme.unwrap_or(Theme::Industrial);
        match theme {
            Theme::Suburbs => self.suburbs(rng),
            Theme::DesertOutpost => self.desert(rng),
            Theme::Farmland => self.farmland(rng),
            Theme::Industrial => self.industrial(rng),
            Theme::WoodsPark => self.woods(rng),
        }
        self.finish_themed(rng, theme);
    }

    fn base_clear(&self, p: Vec2, margin: f32) -> bool {
        !(p.x.abs() > BASE_X - 12.0 - margin && p.y.abs() < 20.0 + margin)
    }

    // --- Suburbs ----------------------------------------------------------------------------

    fn suburbs(&mut self, rng: &mut Rng) {
        // The avenue from base to base: a long, straight sightline.
        self.road(Vec2::new(-120.0, 0.0), Vec2::new(120.0, 0.0), 9.0, Paint::Dark, true);
        for s in [-1.0, 1.0] {
            self.road(Vec2::new(-118.0, s * 5.6), Vec2::new(118.0, s * 5.6), 2.0, Paint::Concrete, false);
        }
        for sx in [-76.0, -38.0, 38.0, 76.0] {
            for dir in [-1.0f32, 1.0] {
                let end = dir * rng.range(50.0, 57.0);
                self.road(Vec2::new(sx, dir * 4.5), Vec2::new(sx, end), 7.0, Paint::Dark, false);
                // The cul-de-sac's turning circle, and a tree in a planter in the middle.
                let bulb = Vec2::new(sx, end + dir * 5.0);
                let y = self.ground_at(bulb) + 0.01;
                self.ground_paint(bulb.x - 8.5, bulb.x + 8.5, bulb.y - 6.0, bulb.y + 6.0, Paint::Dark);
                self.ground_paint(bulb.x - 6.0, bulb.x + 6.0, bulb.y - 8.5, bulb.y + 8.5, Paint::Dark);
                self.boxed(bulb, Vec2::splat(1.3), 0.35, Paint::Concrete);
                self.props.push(Prop { kind: PropKind::Tree, pos: Vec3::new(bulb.x, y + 0.35, bulb.y), yaw: rng.range(0.0, TAU), scale: rng.range(3.8, 4.8) });
                for row in [15.0, 37.0] {
                    for side in [-1.0f32, 1.0] {
                        let lot = Vec2::new(sx + side * 12.0, dir * row);
                        if !self.base_clear(lot, 8.0) {
                            // Too close to a base: a yard with a tree instead.
                            continue;
                        }
                        self.suburban_lot(rng, lot, if side > 0.0 { 1 } else { 2 });
                    }
                }
                // The house at the end of the street, facing down it.
                let at = Vec2::new(sx, end + dir * 19.5);
                let floors = if rng.chance(0.5) { 2 } else { 1 };
                let half = Vec2::new(rng.range(4.4, 5.4), 4.2);
                self.house(rng, at, half, floors, if dir > 0.0 { 0 } else { 3 });
                self.ground_paint(at.x - 1.6, at.x + 1.6, (at.y - dir * 4.2).min(end + dir * 13.0), (at.y - dir * 4.2).max(end + dir * 13.0), Paint::Concrete);
            }
        }
        // The middle: a mansion facing the avenue across its fountain, and a park and a second
        // estate on the other side.
        self.mansion(rng, Vec2::new(0.0, 46.0), true);
        self.road(Vec2::new(0.0, 4.5), Vec2::new(0.0, 16.0), 5.0, Paint::Concrete, false);
        for s in [-1.0, 1.0] {
            self.hedge(Vec2::new(s * 17.4, 14.0), Vec2::new(s * 17.4, 62.0), rng);
            self.low_wall(Vec2::new(s * 3.5, 12.0), Vec2::new(s * 16.6, 12.0), COVER_HEIGHT, 0.5, Paint::Bone);
        }
        self.park(rng, Vec2::new(0.0, -28.0));
        self.mansion(rng, Vec2::new(0.0, -72.0), false);
        for s in [-1.0, 1.0] {
            self.hedge(Vec2::new(s * 19.0, -52.0), Vec2::new(s * 19.0, -90.0), rng);
        }
    }

    /// One house lot beside a street: the house faces the street (`front` west or east), with a
    /// garage and driveway or an L-shaped wing, fences round the back yard, a mailbox, a car.
    fn suburban_lot(&mut self, rng: &mut Rng, lot: Vec2, front: usize) {
        // Toward the street is -x for a west-facing house.
        let toward = if front == 1 { -1.0 } else { 1.0 };
        let street_edge = lot.x + toward * 8.5;
        let floors = if rng.chance(0.4) { 2 } else { 1 };
        let garage = rng.chance(0.55);
        let half = Vec2::new(rng.range(3.6, 4.2), if garage { rng.range(4.0, 4.6) } else { rng.range(4.2, 5.4) });
        let c = Vec2::new(street_edge - toward * (4.0 + half.x), lot.y + if garage { -2.2 } else { 0.0 });
        if !garage && rng.chance(0.5) {
            let east = rng.chance(0.5);
            self.l_house(rng, c, half, floors, front, east);
        } else {
            self.house(rng, c, half, floors, front);
        }
        if garage {
            let g = Vec2::new(street_edge - toward * 5.6, c.y + half.y + 2.8);
            self.garage(rng, g, front);
            let (a, b) = (street_edge, g.x + toward * 3.3);
            self.ground_paint(a.min(b), a.max(b), g.y - 1.7, g.y + 1.7, Paint::Concrete);
            if rng.chance(0.5) {
                // Parked at the kerb outside.
                let p = Vec2::new(street_edge + toward * 1.25, g.y);
                if !self.occupied(p, 1.2) && self.clutter.iter().all(|c| Vec2::new(c.pos.x, c.pos.z).distance(p) > 5.0) {
                    self.clutter.push(Clutter { kind: ClutterKind::Car, pos: Vec3::new(p.x, self.ground_at(p), p.y), turned: false });
                }
            }
        }
        // A path to the front door is the street side; a mailbox at the kerb.
        let m = Vec2::new(street_edge + toward * 0.6, lot.y - 5.0);
        if !self.occupied(m, 0.6) {
            self.clutter.push(Clutter { kind: ClutterKind::Mailbox, pos: Vec3::new(m.x, self.ground_at(m), m.y), turned: false });
        }
        // Fences: along the back and between the yards, with a gate in each.
        let back = lot.x - toward * 7.4;
        let paint = *rng.pick(&[Paint::Bone, Paint::Wood, Paint::Bone]).unwrap_or(&Paint::Bone);
        self.fence(Vec2::new(back, lot.y - 9.3), Vec2::new(back, lot.y + 9.3), &[rng.range(3.0, 15.6)], paint);
        let inner = c.x - toward * half.x;
        for s in [-1.0, 1.0] {
            let z = lot.y + s * 9.6;
            let (a, b) = (back.min(inner), back.max(inner));
            if b - a > 2.5 {
                self.fence(Vec2::new(a, z), Vec2::new(b, z), &[(b - a) * 0.5], paint);
            }
        }
        // Greenery: a tree out front, bushes along the house.
        if rng.chance(0.6) {
            let p = Vec2::new(street_edge - toward * 1.8, lot.y + rng.range(3.0, 7.0));
            if !self.occupied(p, 1.2) {
                self.props.push(Prop { kind: PropKind::Tree, pos: Vec3::new(p.x, self.ground_at(p), p.y), yaw: rng.range(0.0, TAU), scale: rng.range(3.4, 4.8) });
            }
        }
        for _ in 0..2 {
            let p = Vec2::new(c.x + toward * (half.x + 0.9), c.y + rng.range(-half.y, half.y));
            if !self.occupied(p, 0.7) {
                self.props.push(Prop { kind: PropKind::Bush, pos: Vec3::new(p.x, self.ground_at(p), p.y), yaw: rng.range(0.0, TAU), scale: rng.range(0.7, 1.0) });
            }
        }
    }

    /// A neighbourhood park: paths, a gazebo, a playground, benches and trees.
    fn park(&mut self, rng: &mut Rng, c: Vec2) {
        let y = self.ground_at(c) + 0.01;
        self.ground_paint(c.x - 1.2, c.x + 1.2, c.y - 20.0, c.y + 20.0, Paint::Sand);
        self.ground_paint(c.x - 16.0, c.x + 16.0, c.y - 1.2, c.y + 1.2, Paint::Sand);
        self.shelter(c + Vec2::new(-8.0, -9.0), Vec2::new(3.0, 3.0), Paint::Dark);
        // Playground: a sandpit with a low rim, a slide tower (cover), swings (thin frames).
        let pg = c + Vec2::new(8.0, 9.0);
        self.ground_paint(pg.x - 5.0, pg.x + 5.0, pg.y - 4.0, pg.y + 4.0, Paint::Sand);
        // The slide sits on the tower's top, wherever the slope left it.
        let tower = self.boxed(pg + Vec2::new(-2.0, 0.0), Vec2::new(1.0, 1.0), 1.5, Paint::Coral);
        self.solid(Aabb::span(Vec3::new(pg.x - 1.0, tower, pg.y - 1.0), Vec3::new(pg.x + 0.6, tower + 0.15, pg.y + 1.0)), Paint::Gold);
        for k in 0..3 {
            let x = pg.x + 1.2 + k as f32 * 1.2;
            self.decor(Aabb::span(Vec3::new(x - 0.03, y, pg.y + 2.4), Vec3::new(x + 0.03, y + 2.2, pg.y + 2.46)), Paint::Teal);
        }
        self.decor(Aabb::span(Vec3::new(pg.x + 1.1, y + 2.2, pg.y + 2.38), Vec3::new(pg.x + 3.7, y + 2.27, pg.y + 2.48)), Paint::Teal);
        for (dx, dz) in [(-6.0, 3.0), (6.0, -3.0), (-3.0, -14.0), (3.0, 14.0)] {
            let p = c + Vec2::new(dx, dz);
            self.clutter.push(Clutter { kind: ClutterKind::Bench, pos: Vec3::new(p.x, self.ground_at(p), p.y), turned: dx.abs() < 4.0 });
        }
        for _ in 0..10 {
            let p = c + Vec2::new(rng.range(-16.0, 16.0), rng.range(-19.0, 19.0));
            if !self.occupied(p, 1.6) && p.x.abs() > 2.0 && (p.y - c.y).abs() > 2.0 {
                self.props.push(Prop { kind: PropKind::Tree, pos: Vec3::new(p.x, self.ground_at(p), p.y), yaw: rng.range(0.0, TAU), scale: rng.range(3.8, 5.4) });
            }
        }
    }

    // --- Desert outpost ---------------------------------------------------------------------

    fn desert(&mut self, rng: &mut Rng) {
        self.road(Vec2::new(-120.0, 0.0), Vec2::new(120.0, 0.0), 6.0, Paint::Concrete, false);
        self.road(Vec2::new(0.0, -96.0), Vec2::new(0.0, 96.0), 5.0, Paint::Concrete, false);
        self.fort(rng, Vec2::ZERO);
        let sites: Vec<(Vec2, f32)> = self.flats.iter().copied().filter(|(c, r)| *r < 15.0 && *c != OASIS.0).collect();
        for (c, _) in sites {
            self.compound(rng, c);
        }
        // The oasis: palms round the water, rocks.
        for i in 0..9 {
            let a = i as f32 * TAU / 9.0 + rng.range(-0.2, 0.2);
            let p = OASIS.0 + Vec2::from_angle(a) * rng.range(OASIS.1 + 1.5, OASIS.1 + 5.0);
            self.props.push(Prop { kind: PropKind::Palm, pos: Vec3::new(p.x, self.ground_at(p), p.y), yaw: rng.range(0.0, TAU), scale: rng.range(3.4, 5.2) });
        }
        // Sandbag nests out on the sand, and outcrops: the only cover between the compounds.
        let mut nests = 0;
        for _ in 0..80 {
            if nests >= 14 {
                break;
            }
            let p = Vec2::new(rng.range(-95.0, 95.0), rng.range(-100.0, 100.0));
            if self.occupied(p, 5.0) || !self.base_clear(p, 6.0) || p.distance(OASIS.0) < OASIS.1 + 4.0 || p.x.abs() < 3.5 || p.y.abs() < 3.5 {
                continue;
            }
            self.sandbags(p, Vec2::new(rng.range(1.5, 2.4), rng.range(1.5, 2.4)), rng.below(4));
            nests += 1;
        }
        for _ in 0..10 {
            let p = Vec2::new(rng.range(-100.0, 100.0), rng.range(-104.0, 104.0));
            if !self.occupied(p, 6.0) && self.base_clear(p, 6.0) && p.distance(OASIS.0) > OASIS.1 + 6.0 {
                let size = rng.range(1.4, 2.6);
                self.outcrop(rng, p, size);
            }
        }
        // Ruined walls out in the open, and lone huts by the tracks.
        let mut ruins = 0;
        for _ in 0..120 {
            if ruins >= 16 {
                break;
            }
            let p = Vec2::new(rng.range(-95.0, 95.0), rng.range(-100.0, 100.0));
            if self.occupied(p, 6.0) || !self.base_clear(p, 6.0) || self.near_road(p, 3.0) || p.distance(OASIS.0) < OASIS.1 + 5.0 {
                continue;
            }
            let (a, b) = (rng.range(2.5, 5.0), rng.range(1.5, 4.0));
            let high = if rng.chance(0.5) { COVER_HEIGHT } else { rng.range(1.8, 2.6) };
            let paint = *rng.pick(&[Paint::Sand, Paint::Bone, Paint::Concrete]).unwrap_or(&Paint::Sand);
            let (sx, sz) = (if rng.chance(0.5) { 1.0 } else { -1.0 }, if rng.chance(0.5) { 1.0 } else { -1.0 });
            self.low_wall(p, p + Vec2::new(sx * a, 0.0), high, 0.5, paint);
            self.low_wall(p + Vec2::new(0.0, sz * 0.25), p + Vec2::new(0.0, sz * b), high * rng.range(0.6, 1.0), 0.5, paint);
            ruins += 1;
        }
        let mut huts = 0;
        for _ in 0..80 {
            if huts >= 6 {
                break;
            }
            let along = rng.range(-90.0, 90.0);
            let off = if rng.chance(0.5) { 9.0 } else { -9.0 };
            let p = if rng.chance(0.5) { Vec2::new(along, off) } else { Vec2::new(off, along) };
            let half = Vec2::new(rng.range(2.8, 3.6), rng.range(2.6, 3.2));
            if self.occupied(p, 6.0) || !self.base_clear(p, 8.0) || p.length() < 30.0 {
                continue;
            }
            let door = if p.y.abs() < 10.0 { if p.y > 0.0 { 0 } else { 3 } } else if p.x > 0.0 { 1 } else { 2 };
            let floors = if rng.chance(0.3) && door != 3 { 2 } else { 1 };
            self.hut(rng, p, half, floors, Some(door));
            huts += 1;
        }
        for _ in 0..6 {
            let p = Vec2::new(rng.range(-90.0, 90.0), rng.range(-95.0, 95.0));
            if !self.occupied(p, 3.0) && self.base_clear(p, 3.0) && !self.near_road(p, 2.0) && self.ground_at(p) > 0.8 {
                self.clutter.push(Clutter { kind: ClutterKind::Car, pos: Vec3::new(p.x, self.ground_at(p), p.y), turned: rng.chance(0.5) });
            }
        }
        for _ in 0..50 {
            let p = Vec2::new(rng.range(-108.0, 108.0), rng.range(-108.0, 108.0));
            if !self.occupied(p, 2.0) && self.base_clear(p, 2.0) && self.ground_at(p) > 0.8 && self.roads.iter().all(|(a, b, w)| distance_to_segment(p, *a, *b) > w * 0.5 + 1.0) {
                self.cactus(rng, p);
            }
        }
    }

    /// The fort: walls above head height with gates on every side, a wall-walk behind the north
    /// and south walls, corner towers, an HQ, a water tower.
    fn fort(&mut self, rng: &mut Rng, c: Vec2) {
        let (hx, hz, t, high) = (19.0, 15.0, 0.6, 2.8);
        let paint = Paint::Sand;
        // North and south walls run along X with a gate in the middle; west and east the same.
        for s in [-1.0f32, 1.0] {
            let z = c.y + s * (hz - t * 0.5);
            self.low_wall(Vec2::new(c.x - hx, z), Vec2::new(c.x - 2.2, z), high, t, paint);
            self.low_wall(Vec2::new(c.x + 2.2, z), Vec2::new(c.x + hx, z), high, t, paint);
            let x = c.x + s * (hx - t * 0.5);
            self.low_wall(Vec2::new(x, c.y - hz + t), Vec2::new(x, c.y - 2.2), high, t, paint);
            self.low_wall(Vec2::new(x, c.y + 2.2), Vec2::new(x, c.y + hz - t), high, t, paint);
            // Gate posts, taller than the wall.
            for g in [-1.0, 1.0] {
                let post = Vec2::new(c.x + g * 2.6, z);
                self.boxed(post, Vec2::new(0.4, 0.3), high + 0.6, Paint::Bone);
                let post = Vec2::new(x, c.y + g * 2.6);
                self.boxed(post, Vec2::new(0.3, 0.4), high + 0.6, Paint::Bone);
            }
            // The wall-walk: a raised step behind the wall, the wall cover height above it, and
            // stairs up at its inner end.
            let g = self.ground_at(c);
            let wz0 = z - s * (t * 0.5 + 1.2);
            let (lo, hi) = (wz0.min(z - s * t * 0.5), wz0.max(z - s * t * 0.5));
            for side in [-1.0f32, 1.0] {
                let (a, b) = (c.x + side * 4.2, c.x + side * 12.0);
                let walk = high - COVER_HEIGHT;
                self.solid(Aabb::span(Vec3::new(a.min(b), g - 0.3, lo), Vec3::new(a.max(b), g + walk, hi)), Paint::Concrete);
                let steps = (walk / 0.32).ceil() as usize;
                for k in 0..steps {
                    let top = g + walk * (k + 1) as f32 / (steps + 1) as f32;
                    let x0 = a - side * (steps - k) as f32 * 0.42;
                    let x1 = x0 + side * 0.42;
                    self.solid(Aabb::span(Vec3::new(x0.min(x1), g - 0.3, lo), Vec3::new(x0.max(x1), top, hi)), Paint::Concrete);
                }
            }
        }
        // Corner towers, the HQ, the water tower, nests by the gates.
        self.hut(rng, c + Vec2::new(-hx + 3.6, hz - 3.6), Vec2::new(2.9, 2.9), 2, Some(2));
        self.hut(rng, c + Vec2::new(hx - 3.6, -hz + 3.6), Vec2::new(2.9, 2.9), 2, Some(1));
        self.hut(rng, c + Vec2::new(-6.5, -5.5), Vec2::new(4.6, 3.6), 2, None);
        self.hut(rng, c + Vec2::new(8.0, 6.0), Vec2::new(3.4, 3.0), 1, Some(0));
        self.water_tower(c + Vec2::new(9.0, -6.5), 6.4, Paint::Rust);
        for (dx, dz) in [(-10.5, 6.0), (2.0, 9.5)] {
            self.sandbags(c + Vec2::new(dx, dz), Vec2::new(1.6, 1.4), rng.below(4));
        }
        // A radio mast: thin, walk-through.
        let m = c + Vec2::new(-11.5, 1.0);
        let g = self.ground_at(m);
        self.pole(Vec3::new(m.x, g, m.y), 11.0, Paint::Iron);
        self.decor(Aabb::new(Vec3::new(m.x, g + 11.0, m.y), Vec3::new(0.08, 0.08, 0.08)), Paint::Red);
        self.flags.push(Vec3::new(m.x, g + 10.6, m.y));
    }

    /// A walled compound: walls over head height with a gate or two, one or two huts, a well.
    fn compound(&mut self, rng: &mut Rng, c: Vec2) {
        let half = Vec2::new(rng.range(7.5, 9.5), rng.range(6.5, 8.0));
        let (high, t) = (rng.range(2.2, 2.6), 0.5);
        let paint = *rng.pick(&[Paint::Sand, Paint::Bone, Paint::Concrete]).unwrap_or(&Paint::Sand);
        // Gates face the fort, and sometimes the far side too.
        let toward = if c.x.abs() > c.y.abs() { if c.x > 0.0 { 1 } else { 2 } } else if c.y > 0.0 { 0 } else { 3 };
        let mut gates = [false; 4];
        gates[toward] = true;
        if rng.chance(0.5) {
            gates[3 - toward] = true;
        }
        let gap = 1.3;
        let gate_at = rng.range(-1.5, 1.5);
        let wall = |world: &mut World, side: usize, a: Vec2, b: Vec2| {
            if gates[side] {
                let m = (a + b) * 0.5 + (b - a).normalize() * gate_at;
                let dir = (b - a).normalize();
                world.low_wall(a, m - dir * gap, high, t, paint);
                world.low_wall(m + dir * gap, b, high, t, paint);
            } else {
                world.low_wall(a, b, high, t, paint);
            }
        };
        wall(self, 0, Vec2::new(c.x - half.x, c.y - half.y), Vec2::new(c.x + half.x, c.y - half.y));
        wall(self, 3, Vec2::new(c.x - half.x, c.y + half.y), Vec2::new(c.x + half.x, c.y + half.y));
        wall(self, 1, Vec2::new(c.x - half.x, c.y - half.y + t * 0.5), Vec2::new(c.x - half.x, c.y + half.y - t * 0.5));
        wall(self, 2, Vec2::new(c.x + half.x, c.y - half.y + t * 0.5), Vec2::new(c.x + half.x, c.y + half.y - t * 0.5));
        // Huts against the back wall, away from the gate.
        let back = 3 - toward;
        let out = super::shapes::side_out(back);
        let hut_half = Vec2::new(rng.range(2.8, 3.6), rng.range(2.6, 3.0));
        let hut_half = if out.x.abs() > 0.5 { Vec2::new(hut_half.y, hut_half.x) } else { hut_half };
        let depth = if out.x.abs() > 0.5 { half.x - hut_half.x } else { half.y - hut_half.y } - t - 0.6;
        let across = Vec2::new(out.y.abs(), out.x.abs());
        let span = if out.x.abs() > 0.5 { half.y - hut_half.y } else { half.x - hut_half.x } - 0.9;
        let first = c + out * depth + across * span * if rng.chance(0.5) { 1.0 } else { -1.0 };
        let floors = if rng.chance(0.3) && toward != 3 { 2 } else { 1 };
        self.hut(rng, first, hut_half, floors, Some(toward));
        if rng.chance(0.5) && span > hut_half.x.max(hut_half.y) + 0.5 {
            let second = c + out * depth - (first - c - out * depth);
            self.hut(rng, second, hut_half, 1, Some(toward));
        }
        // A well in the yard: round-ish, low cover, with a little roof on thin posts.
        let w = c - out * (depth * 0.3) + across * rng.range(-2.0, 2.0);
        if !self.occupied(w, 1.6) {
            let g = self.ground_at(w);
            self.column(w, 0.9, g - 0.1, g + 0.85, Paint::Rock);
            self.decor(Aabb::new(Vec3::new(w.x, g + 0.86, w.y), Vec3::new(0.6, 0.01, 0.6)), Paint::Blue);
            for s in [-0.75, 0.75] {
                self.pole(Vec3::new(w.x + s, g + 0.85, w.y), 1.6, Paint::Wood);
            }
            self.decor(Aabb::new(Vec3::new(w.x, g + 2.5, w.y), Vec3::new(1.0, 0.04, 0.6)), Paint::Wood);
        }
        for _ in 0..2 {
            let p = c + Vec2::new(rng.range(-half.x + 1.5, half.x - 1.5), rng.range(-half.y + 1.5, half.y - 1.5));
            if !self.occupied(p, 1.3) {
                self.props.push(Prop { kind: PropKind::Crate, pos: Vec3::new(p.x, self.ground_at(p), p.y), yaw: 0.0, scale: 1.0 });
            }
        }
        if rng.chance(0.5) {
            let p = c - out * (half.y * 0.5).min(3.0) + across * rng.range(-4.0, 4.0);
            if !self.occupied(p, 1.0) {
                self.props.push(Prop { kind: PropKind::Palm, pos: Vec3::new(p.x, self.ground_at(p), p.y), yaw: rng.range(0.0, TAU), scale: rng.range(3.0, 4.6) });
            }
        }
    }

    // --- Farmland ---------------------------------------------------------------------------

    fn farmland(&mut self, rng: &mut Rng) {
        let dirt = Paint::Sand;
        self.road(Vec2::new(-120.0, 0.0), Vec2::new(120.0, 0.0), 6.0, dirt, false);
        self.road(Vec2::new(0.0, 3.0), Vec2::new(0.0, 30.0), 5.0, dirt, false);
        // The main farmstead north of the road.
        let flip = if rng.chance(0.5) { 1.0 } else { -1.0 };
        self.barn(rng, Vec2::new(0.0, 46.0), Vec2::new(9.0, 6.5));
        self.silo(Vec2::new(flip * 16.5, 52.0), 2.7, 12.5, Paint::Metal, Some(Vec2::new(flip, 0.0)));
        self.silo(Vec2::new(flip * 16.5, 42.5), 2.3, 10.0, Paint::Iron, None);
        self.silo(Vec2::new(flip * 22.5, 47.0), 2.0, 8.5, Paint::Bone, Some(Vec2::new(flip, 0.0)));
        self.house(rng, Vec2::new(-flip * 21.0, 24.0), Vec2::new(5.2, 4.6), 2, 0);
        self.machine_shed(rng, Vec2::new(-flip * 22.0, 47.0), Vec2::new(6.0, 4.5));
        self.windmill(Vec2::new(flip * 36.0, 22.0), Paint::Bone);
        for _ in 0..8 {
            let p = Vec2::new(rng.range(-28.0, 28.0), rng.range(30.0, 62.0));
            if !self.occupied(p, 2.0) {
                self.hay_stack(p, rng.chance(0.5), None);
            }
        }
        // The paddock fence round the yard, gates to the road and the fields.
        let (x0, x1, z0, z1) = (-31.0, 31.0, 14.0, 64.0);
        self.fence(Vec2::new(x0, z0), Vec2::new(x1, z0), &[31.0, 31.0 - flip * 21.0, 10.0], Paint::Wood);
        self.fence(Vec2::new(x0, z1), Vec2::new(x1, z1), &[12.0, 50.0], Paint::Wood);
        self.fence(Vec2::new(x0, z0), Vec2::new(x0, z1), &[25.0], Paint::Wood);
        self.fence(Vec2::new(x1, z0), Vec2::new(x1, z1), &[25.0], Paint::Wood);
        // Two smaller farms south of the road.
        for (s, at) in [(-1.0f32, Vec2::new(-46.0, -44.0)), (1.0, Vec2::new(48.0, -46.0))] {
            let at = at + Vec2::new(rng.range(-4.0, 4.0), rng.range(-4.0, 4.0));
            self.road(Vec2::new(at.x, -3.0), Vec2::new(at.x, at.y + 12.0), 4.0, dirt, false);
            if rng.chance(0.5) {
                self.barn(rng, at + Vec2::new(-s * 2.0, -6.0), Vec2::new(7.0, 5.0));
            } else {
                self.machine_shed(rng, at + Vec2::new(-s * 2.0, -6.0), Vec2::new(5.5, 4.0));
            }
            let floors = if rng.chance(0.5) { 2 } else { 1 };
            self.house(rng, at + Vec2::new(s * 13.0, 9.0), Vec2::new(4.4, 4.0), floors, 3);
            if rng.chance(0.5) {
                self.silo(at + Vec2::new(-s * 14.0, 4.0), 2.2, 9.0, Paint::Metal, Some(Vec2::new(-s, 0.0)));
            } else {
                self.water_tower(at + Vec2::new(-s * 14.0, 4.0), 7.5, Paint::Bone);
            }
            for _ in 0..4 {
                let p = at + Vec2::new(rng.range(-12.0, 12.0), rng.range(-14.0, 14.0));
                if !self.occupied(p, 2.0) {
                    self.hay_stack(p, rng.chance(0.5), None);
                }
            }
        }
        // Fields between low stone walls and fences.
        let mut corn = 0;
        for (fx0, fx1) in [(-92.0, -62.0), (-58.0, -32.0), (32.0, 58.0), (62.0, 92.0)] {
            for (fz0, fz1) in [(-96.0, -64.0), (-60.0, -32.0), (-28.0, -6.0), (6.0, 30.0), (34.0, 64.0), (68.0, 96.0)] {
                let (c, half) = (Vec2::new((fx0 + fx1) * 0.5, (fz0 + fz1) * 0.5), Vec2::new((fx1 - fx0) * 0.5, (fz1 - fz0) * 0.5));
                if (fx0.abs().min(fx1.abs()) > 60.0 && fz0 < 30.0 && fz1 > -30.0) || self.occupied(c, half.x.min(half.y) - 2.0) {
                    continue;
                }
                corn += self.field(rng, c, half, corn < 2) as usize;
            }
        }
        // Orchards fill some of the inner ground.
        for c in [Vec2::new(-14.0, -24.0), Vec2::new(14.0, -78.0), Vec2::new(-12.0, 84.0)] {
            for i in 0..4 {
                for j in 0..3 {
                    let p = c + Vec2::new(i as f32 * 5.0 - 7.5, j as f32 * 6.0 - 6.0);
                    if !self.occupied(p, 1.5) {
                        self.props.push(Prop { kind: PropKind::Tree, pos: Vec3::new(p.x, self.ground_at(p), p.y), yaw: rng.range(0.0, TAU), scale: rng.range(3.0, 3.8) });
                    }
                }
            }
        }
    }

    /// One field: a stone wall or fence round it with gaps, and a crop. Returns whether it is corn.
    fn field(&mut self, rng: &mut Rng, c: Vec2, half: Vec2, corn_ok: bool) -> bool {
        let stone = rng.chance(0.6);
        let edges = [
            (Vec2::new(c.x - half.x, c.y - half.y), Vec2::new(c.x + half.x, c.y - half.y)),
            (Vec2::new(c.x - half.x, c.y + half.y), Vec2::new(c.x + half.x, c.y + half.y)),
            (Vec2::new(c.x - half.x, c.y - half.y + 0.3), Vec2::new(c.x - half.x, c.y + half.y - 0.3)),
            (Vec2::new(c.x + half.x, c.y - half.y + 0.3), Vec2::new(c.x + half.x, c.y + half.y - 0.3)),
        ];
        for (a, b) in edges {
            if rng.chance(0.2) {
                continue; // A side left open.
            }
            let length = (b - a).length();
            let gates = [rng.range(3.0, length * 0.45), rng.range(length * 0.55, length - 3.0)];
            if stone {
                let dir = (b - a) / length;
                let mut cursor = 0.0;
                for gate in gates {
                    self.low_wall(a + dir * cursor, a + dir * (gate - 1.1), COVER_HEIGHT, 0.6, Paint::Rock);
                    cursor = gate + 1.1;
                }
                self.low_wall(a + dir * cursor, b, COVER_HEIGHT, 0.6, Paint::Rock);
            } else {
                self.fence(a, b, &gates, Paint::Wood);
            }
        }
        let inner = (c.x - half.x + 1.5, c.x + half.x - 1.5, c.y - half.y + 1.5, c.y + half.y - 1.5);
        let roll = rng.unit();
        if corn_ok && roll < 0.3 {
            // Corn: rows of tall leaves you can hide in.
            let mut x = inner.0 + 1.0;
            while x < inner.1 {
                let mut z = inner.2 + 0.5;
                while z < inner.3 {
                    let p = Vec2::new(x + rng.range(-0.3, 0.3), z);
                    self.props.push(Prop { kind: PropKind::Bush, pos: Vec3::new(p.x, self.ground_at(p), p.y), yaw: rng.range(0.0, TAU), scale: rng.range(1.1, 1.4) });
                    z += 2.3;
                }
                x += 4.6;
            }
            return true;
        }
        if roll < 0.55 {
            // Wheat stubble and bales.
            self.ground_paint(inner.0, inner.1, inner.2, inner.3, Paint::Gold);
            for _ in 0..5 {
                let p = Vec2::new(rng.range(inner.0 + 1.5, inner.1 - 1.5), rng.range(inner.2 + 1.5, inner.3 - 1.5));
                if !self.occupied(p, 2.0) {
                    self.hay_stack(p, rng.chance(0.5), None);
                }
            }
        } else if roll < 0.8 {
            // Ploughed furrows.
            let mut x = inner.0;
            let mut k = 0;
            while x < inner.1 {
                self.ground_paint(x, (x + 1.1).min(inner.1), inner.2, inner.3, if k % 2 == 0 { Paint::Wood } else { Paint::Rust });
                x += 1.1;
                k += 1;
            }
        } else {
            // Pasture with a trough.
            let p = Vec2::new(rng.range(inner.0 + 2.0, inner.1 - 2.0), rng.range(inner.2 + 2.0, inner.3 - 2.0));
            self.boxed(p, Vec2::new(1.4, 0.45), 0.7, Paint::Wood);
            self.decor(Aabb::span(Vec3::new(p.x - 1.25, self.ground_at(p) + 0.6, p.y - 0.3), Vec3::new(p.x + 1.25, self.ground_at(p) + 0.65, p.y + 0.3)), Paint::Blue);
        }
        false
    }

    /// An open-sided machine shed: a roof on posts, a tractor and a trailer under it.
    fn machine_shed(&mut self, rng: &mut Rng, c: Vec2, half: Vec2) {
        let g = self.highest(c, half);
        for i in 0..3 {
            for s in [-1.0, 1.0] {
                let p = Vec2::new(c.x - half.x + 0.2 + i as f32 * (half.x - 0.2), c.y + s * (half.y - 0.2));
                self.solid(Aabb::span(Vec3::new(p.x - 0.15, g - 0.3, p.y - 0.15), Vec3::new(p.x + 0.15, g + 3.4, p.y + 0.15)), Paint::Wood);
            }
        }
        self.solid(Aabb::span(Vec3::new(c.x - half.x - 0.3, g + 3.4, c.y - half.y - 0.4), Vec3::new(c.x + half.x + 0.3, g + 3.6, c.y + half.y + 0.4)), Paint::Iron);
        self.stepped_roof((c.x - half.x - 0.3, c.x + half.x + 0.3, c.y - half.y - 0.4, c.y + half.y + 0.4), g + 3.6, false, true, (Paint::Rust, Paint::Iron));
        self.flat(c.x - half.x, c.x + half.x, c.y - half.y, c.y + half.y, g + 0.01, Paint::Concrete);
        // The tractor: body, cab and big wheels, all solid.
        let t = c + Vec2::new(-half.x * 0.4, 0.0);
        self.solid(Aabb::span(Vec3::new(t.x - 1.4, g + 0.5, t.y - 0.6), Vec3::new(t.x + 1.0, g + 1.3, t.y + 0.6)), *rng.pick(&[Paint::LeafLight, Paint::Coral, Paint::Gold]).unwrap_or(&Paint::Coral));
        self.solid(Aabb::span(Vec3::new(t.x + 0.0, g + 1.3, t.y - 0.6), Vec3::new(t.x + 1.0, g + 2.4, t.y + 0.6)), Paint::Dark);
        for (dx, r) in [(-1.0, 0.5), (0.6, 0.75)] {
            for s in [-1.0, 1.0] {
                self.solid(Aabb::span(Vec3::new(t.x + dx - r, g, t.y + s * 0.6 - 0.12 * (1.0 - s) - 0.0), Vec3::new(t.x + dx + r, g + 2.0 * r, t.y + s * 0.6 + 0.12 * (1.0 + s))), Paint::Dark);
            }
        }
        let trailer = c + Vec2::new(half.x * 0.45, 0.0);
        self.boxed(trailer, Vec2::new(1.6, 1.0), 1.1, Paint::Wood);
    }

    // --- Industrial town --------------------------------------------------------------------

    fn industrial(&mut self, rng: &mut Rng) {
        const PITCH: f32 = 28.0;
        // Streets between the lots: a grid with an avenue down the middle each way.
        for k in -3..=4 {
            let at = (k as f32 - 0.5) * PITCH;
            self.road(Vec2::new(-112.0, at), Vec2::new(112.0, at), 6.0, Paint::Dark, true);
            self.road(Vec2::new(at, -112.0), Vec2::new(at, 112.0), 6.0, Paint::Dark, true);
        }
        let rail = if rng.chance(0.5) { 2 } else { -2 };
        for gx in -3i32..=3 {
            for gz in -3i32..=3 {
                let c = Vec2::new(gx as f32 * PITCH, gz as f32 * PITCH);
                if (gx, gz) == (0, 0) {
                    self.plaza_at(rng, c);
                    continue;
                }
                if gz == rail {
                    continue; // The rail yard runs through this row.
                }
                if !self.base_clear(c, 10.0) {
                    continue;
                }
                if rng.chance(0.6) {
                    self.ground_paint(c.x - 11.0, c.x + 11.0, c.y - 11.0, c.y + 11.0, if rng.chance(0.5) { Paint::Concrete } else { Paint::Rock });
                }
                let roll = rng.unit();
                if roll < 0.2 {
                    let at = c + Vec2::new(rng.range(-1.0, 1.0), rng.range(-1.0, 1.0));
                    let half = Vec2::new(rng.range(8.5, 10.5), rng.range(6.5, 8.5));
                    let front = rng.below(4);
                    self.warehouse(rng, at, half, front);
                } else if roll < 0.32 {
                    // A huddle of shacks.
                    for (dx, dz) in [(-5.5, -5.0), (5.5, -5.5), (-5.0, 5.5), (5.5, 5.0)] {
                        if rng.chance(0.8) {
                            let at = c + Vec2::new(dx + rng.range(-1.0, 1.0), dz + rng.range(-1.0, 1.0));
                            self.shack(rng, at);
                        }
                    }
                } else if roll < 0.42 {
                    // A tank farm: silos with pipes between them, a ladder up one.
                    let r = rng.range(2.4, 3.2);
                    let positions = [(-5.0, -5.0), (5.0, -5.0), (-5.0, 5.0), (5.0, 5.0)];
                    for (i, (dx, dz)) in positions.iter().enumerate() {
                        if i == 3 && rng.chance(0.5) {
                            continue;
                        }
                        let p = c + Vec2::new(*dx, *dz);
                        self.silo(p, r, rng.range(7.0, 12.0), *rng.pick(&[Paint::Metal, Paint::Bone, Paint::Iron]).unwrap_or(&Paint::Metal), (i == 0).then_some(Vec2::new(-1.0, 0.0)));
                    }
                    // The pipes run silo to silo (the first to the second and to the third, which
                    // are always there), not through the empty middle.
                    let g = self.ground_at(c);
                    self.decor(Aabb::span(Vec3::new(c.x - 5.0, g + 4.0, c.y - 5.08), Vec3::new(c.x + 5.0, g + 4.16, c.y - 4.92)), Paint::Steel);
                    self.decor(Aabb::span(Vec3::new(c.x - 5.08, g + 3.0, c.y - 5.0), Vec3::new(c.x - 4.92, g + 3.16, c.y + 5.0)), Paint::Rust);
                } else if roll < 0.52 {
                    // A tall block for the snipers.
                    let half = Vec2::new(rng.range(6.6, 7.6), rng.range(5.8, 6.8));
                    let floors = 4 + rng.below(5) as u32;
                    self.building(rng, c, half, floors);
                } else if roll < 0.6 {
                    // Container stacks.
                    for (dx, dz, along) in [(-4.0, -5.0, true), (4.0, 0.0, true), (-4.0, 5.5, true), (7.5, -7.0, false)] {
                        if rng.chance(0.75) {
                            let levels = if rng.chance(0.4) { 2 } else { 1 };
                            self.container(rng, c + Vec2::new(dx, dz), along, levels);
                        }
                    }
                } else if roll < 0.7 {
                    for side in [-1.0, 1.0] {
                        let half = Vec2::new(rng.range(3.8, 6.0), rng.range(2.9, 3.4));
                        let at = c + Vec2::new(rng.range(-0.8, 0.8), side * 4.6);
                        let floors = 1 + rng.below(2) as u32;
                        self.building(rng, at, half, floors);
                    }
                } else if roll < 0.76 {
                    self.courtyard(rng, c);
                } else if roll < 0.8 {
                    self.water_tower(c + Vec2::new(rng.range(-3.0, 3.0), rng.range(-3.0, 3.0)), rng.range(7.0, 9.0), Paint::Rust);
                } else if roll < 0.84 {
                    // A car park.
                    self.ground_paint(c.x - 10.0, c.x + 10.0, c.y - 9.0, c.y + 9.0, Paint::Steel);
                    for i in 0..6 {
                        for row in [-4.5, 4.5] {
                            if rng.chance(0.5) {
                                let p = c + Vec2::new(-8.0 + i as f32 * 3.2, row);
                                self.clutter.push(Clutter { kind: ClutterKind::Car, pos: Vec3::new(p.x, self.ground_at(p), p.y), turned: false });
                            }
                        }
                    }
                } else {
                    let half = Vec2::new(rng.range(4.6, 7.0), rng.range(4.4, 7.0));
                    let floors = if half.x > 5.6 && half.y > 5.0 && rng.chance(0.4) { 3 } else if rng.chance(0.55) { 2 } else { 1 };
                    let at = c + Vec2::new(rng.range(-1.0, 1.0), rng.range(-1.0, 1.0));
                    self.building(rng, at, half, floors);
                }
            }
        }
        // The rail yard: tracks the length of the map and broken lines of box cars - a long lane.
        let z = rail as f32 * PITCH;
        for track in [-4.5, 0.0, 4.5] {
            let tz = z + track;
            let y = self.ground_at(Vec2::new(0.0, tz)) + 0.01;
            self.ground_paint(-112.0, 112.0, tz - 1.0, tz + 1.0, Paint::Rock);
            for s in [-0.72, 0.72] {
                self.decor(Aabb::span(Vec3::new(-112.0, y, tz + s - 0.04), Vec3::new(112.0, y + 0.12, tz + s + 0.04)), Paint::Iron);
            }
            let mut x = -86.0 + rng.range(0.0, 10.0);
            while x < 86.0 {
                let length = rng.range(5.0, 7.5);
                if rng.chance(0.55) && self.base_clear(Vec2::new(x, tz), 4.0) {
                    let c = Vec2::new(x + length * 0.5, tz);
                    let paint = *rng.pick(&[Paint::Rust, Paint::Teal, Paint::Wood, Paint::Iron]).unwrap_or(&Paint::Rust);
                    let g = self.ground_at(c);
                    self.solid(Aabb::span(Vec3::new(c.x - length * 0.5, g + 0.4, c.y - 1.3), Vec3::new(c.x + length * 0.5, g + 3.1, c.y + 1.3)), paint);
                    self.solid(Aabb::span(Vec3::new(c.x - length * 0.5 + 0.6, g, c.y - 0.9), Vec3::new(c.x + length * 0.5 - 0.6, g + 0.4, c.y + 0.9)), Paint::Dark);
                    self.decor(Aabb::span(Vec3::new(c.x - 0.8, g + 0.6, c.y - 1.33), Vec3::new(c.x + 0.8, g + 2.8, c.y + 1.33)), Paint::Dark);
                }
                x += length + rng.range(2.0, 9.0);
            }
        }
    }

    // --- Woods park -------------------------------------------------------------------------

    fn woods(&mut self, rng: &mut Rng) {
        let dirt = Paint::Wood;
        let sites = self.flats.clone();
        // Trails: one across the map (over the creek on a bridge), loops to the lodge and lake.
        let bank = 1.25;
        let cross = creek_x(0.0);
        self.road(Vec2::new(-120.0, 0.0), Vec2::new(cross - 6.0, 0.0), 3.0, dirt, false);
        self.road(Vec2::new(cross + 6.0, 0.0), Vec2::new(120.0, 0.0), 3.0, dirt, false);
        let _ = bank;
        for z in [0.0, -46.0, -88.0] {
            let x = creek_x(z);
            let (a, b) = (Vec2::new(x - 6.0, z), Vec2::new(x + 6.0, z));
            let deck = self.ground_at(a).max(self.ground_at(b)) + 0.2;
            self.footbridge(a, b, 2.4, deck);
        }
        self.road(Vec2::new(-40.0, 0.0), Vec2::new(-40.0, 18.0), 3.0, dirt, false);
        // The lodge.
        let lodge = sites[2].0;
        self.lodge(rng, lodge);
        // The lookout tower.
        let tower = sites[3].0;
        self.building_plan(rng, tower, Vec2::new(3.3, 3.0), 4, &Plan { walls: Some(Paint::Wood), trim: Some(Paint::Dark), roof: Roof::Flat, furnish: Furnish::Plain, ..Plan::default() });
        // Cabins in the clearings, shelters and a dock by the lake.
        for (c, _) in sites.iter().skip(4) {
            let front = if c.y.abs() > c.x.abs() { if c.y > 0.0 { 0 } else { 3 } } else if c.x > 0.0 { 1 } else { 2 };
            if rng.chance(0.25) {
                self.shelter(*c, Vec2::new(3.2, 2.6), Paint::Rust);
            } else {
                self.cabin(rng, *c, front);
            }
        }
        for i in 0..4 {
            let a = 3.4 + i as f32 * 0.8 + rng.range(-0.15, 0.15);
            let p = LAKE.0 + Vec2::from_angle(a) * (LAKE.1 + 8.0);
            if !self.occupied(p, 5.0) {
                self.shelter(p, Vec2::new(3.0, 2.4), Paint::Dark);
            }
        }
        let dock = LAKE.0 + Vec2::new(0.0, -LAKE.1 - 1.0);
        let g = self.ground_at(dock + Vec2::new(0.0, -5.0));
        self.footbridge(dock + Vec2::new(0.0, -4.0), dock + Vec2::new(0.0, 7.0), 2.6, g + 0.2);
        // Rock outcrops through the trees.
        for _ in 0..40 {
            let p = Vec2::new(rng.range(-100.0, 100.0), rng.range(-104.0, 104.0));
            if self.occupied(p, 7.0) || !self.base_clear(p, 6.0) || self.ground_at(p) < 0.9 || self.near_road(p, 4.0) {
                continue;
            }
            let size = rng.range(1.5, 2.8);
            self.outcrop(rng, p, size);
            if self.blocks.iter().filter(|b| b.paint == Paint::Rock).count() > 60 {
                break;
            }
        }
        // Woods: tall trees everywhere there is room, block pines among them, undergrowth.
        let mut trees = 0;
        for _ in 0..2600 {
            if trees >= 520 {
                break;
            }
            let p = Vec2::new(rng.range(-122.0, 122.0), rng.range(-122.0, 122.0));
            let h = self.ground_at(p);
            if h < 0.6 || self.near_road(p, 2.0) || !self.base_clear(p, 2.0) || self.occupied(p, 3.0) || self.props.iter().any(|q| q.kind == PropKind::Tree && Vec2::new(q.pos.x, q.pos.z).distance(p) < 3.2) {
                continue;
            }
            let height = rng.range(7.0, 10.0);
            if trees % 9 == 8 && !self.occupied(p, 3.0) && self.pine(p, height) {
                // A block pine.
            } else {
                self.props.push(Prop { kind: PropKind::Tree, pos: Vec3::new(p.x, h, p.y), yaw: rng.range(0.0, TAU), scale: rng.range(3.8, 6.2) });
            }
            trees += 1;
        }
    }

    /// The lodge: a big two-storey timber hall with a porch along the front and a chimney.
    fn lodge(&mut self, rng: &mut Rng, c: Vec2) {
        let half = Vec2::new(10.0, 6.5);
        let mut plan = Plan { walls: Some(Paint::Wood), trim: Some(Paint::Dark), roof: Roof::Gable(Paint::Shell), furnish: Furnish::Home, random_doors: false, chimney: true, ..Plan::default() };
        plan.doors[0] = Some((c.x + 2.5, DOOR_WIDTH));
        plan.doors[1] = Some((c.y - 1.5, DOOR_WIDTH));
        plan.doors[2] = Some((c.y - 1.5, DOOR_WIDTH));
        let (g, _) = self.building_plan(rng, c, half, 2, &plan);
        self.porch(rng, c, half, 0, g);
        // A woodpile and a flagpole.
        let w = c + Vec2::new(-half.x - 1.4, -2.0);
        self.boxed(w, Vec2::new(0.6, 1.6), COVER_HEIGHT, Paint::Wood);
        let f = c + Vec2::new(-4.0, -half.y - 6.0);
        let y = self.ground_at(f);
        self.pole(Vec3::new(f.x, y, f.y), 7.0, Paint::Metal);
        self.flags.push(Vec3::new(f.x, y + 6.6, f.y));
    }

    pub(crate) fn near_road(&self, p: Vec2, margin: f32) -> bool {
        self.roads.iter().any(|(a, b, w)| distance_to_segment(p, *a, *b) < w * 0.5 + margin)
    }

    // --- Finishing --------------------------------------------------------------------------

    /// Bases, street furniture, scattered cover, trees, bushes and grass, by theme.
    fn finish_themed(&mut self, rng: &mut Rng, theme: Theme) {
        let cover = match theme {
            Theme::DesertOutpost | Theme::Farmland => Paint::Sand,
            Theme::WoodsPark => Paint::Wood,
            _ => Paint::Concrete,
        };
        self.team_bases(BASE_X, 0.0, cover);
        // Tents at the bases in the outpost; a hut or a shed elsewhere would block the spawns.
        if theme == Theme::DesertOutpost {
            for s in [-1.0f32, 1.0] {
                for dz in [-15.0, 15.0] {
                    let p = Vec2::new(s * (BASE_X + 4.0), dz);
                    self.building_plan(rng, p, Vec2::new(2.4, 1.9), 1, &Plan { walls: Some(Paint::Leaf), roof: Roof::Hip(Paint::Leaf), windows: false, ladder: false, furnish: Furnish::Bare, floor_paint: Paint::Sand, ..Plan::default() });
                }
            }
        }
        // Street barriers in the town themes.
        let barriers = match theme {
            Theme::Industrial => 30,
            Theme::Suburbs => 8,
            Theme::DesertOutpost => 10,
            _ => 0,
        };
        let mut placed = 0;
        for _ in 0..barriers * 10 {
            if placed >= barriers {
                break;
            }
            let Some(&(a, b, w)) = rng.pick(&self.roads) else { break };
            if w < 5.0 {
                continue;
            }
            let p = a.lerp(b, rng.unit());
            let along_x = rng.chance(0.5);
            let half = if along_x { Vec2::new(1.3, 0.28) } else { Vec2::new(0.28, 1.3) };
            if self.occupied(p, 2.4) || !self.base_clear(p, 4.0) || p.length() < 6.0 {
                continue;
            }
            let g = self.ground_at(p);
            self.solid(Aabb::span(Vec3::new(p.x - half.x, g - 0.1, p.y - half.y), Vec3::new(p.x + half.x, g + COVER_HEIGHT, p.y + half.y)), Paint::Concrete);
            self.decor(Aabb::span(Vec3::new(p.x - half.x - 0.02, g + COVER_HEIGHT - 0.12, p.y - half.y - 0.02), Vec3::new(p.x + half.x + 0.02, g + COVER_HEIGHT - 0.04, p.y + half.y + 0.02)), Paint::Gold);
            placed += 1;
        }
        // Street furniture along the roads.
        let kinds: &[(ClutterKind, usize)] = match theme {
            Theme::Suburbs => &[(ClutterKind::Car, 14), (ClutterKind::Bin, 18), (ClutterKind::Bench, 6), (ClutterKind::Barrel, 4)],
            Theme::DesertOutpost => &[(ClutterKind::Car, 8), (ClutterKind::Barrel, 22), (ClutterKind::Crates, 16)],
            Theme::Farmland => &[(ClutterKind::Car, 5), (ClutterKind::Barrel, 12), (ClutterKind::Crates, 12), (ClutterKind::Mailbox, 4)],
            Theme::Industrial => &[(ClutterKind::Car, 18), (ClutterKind::Bin, 22), (ClutterKind::Barrel, 22), (ClutterKind::Crates, 18), (ClutterKind::Bench, 8), (ClutterKind::Mailbox, 8)],
            Theme::WoodsPark => &[(ClutterKind::Bench, 10), (ClutterKind::Bin, 10), (ClutterKind::Barrel, 3)],
        };
        for &(kind, count) in kinds {
            let mut placed = 0;
            for _ in 0..count * 14 {
                if placed >= count {
                    break;
                }
                let Some(&(a, b, w)) = rng.pick(&self.roads) else { break };
                let along_x = (b.x - a.x).abs() > (b.y - a.y).abs();
                let t = rng.unit();
                let side = if rng.chance(0.5) { 1.0 } else { -1.0 };
                let offset = if kind == ClutterKind::Car { (w * 0.5 - 1.2).max(0.0) } else { w * 0.5 + rng.range(0.4, 1.2) } * side;
                let p = a.lerp(b, t) + if along_x { Vec2::new(0.0, offset) } else { Vec2::new(offset, 0.0) };
                let half = kind.half();
                let reach = half.x.max(half.z) + 0.8;
                if !self.in_bounds(p.x, p.y) || p.x.abs() > self.arena || p.y.abs() > self.arena || !self.base_clear(p, 3.0) || p.length() < 6.0 || self.occupied(p, reach) || self.clutter.iter().any(|c| Vec2::new(c.pos.x, c.pos.z).distance(p) < reach + 1.5) {
                    continue;
                }
                if self.ground_at(p) < 0.5 {
                    continue;
                }
                self.clutter.push(Clutter { kind, pos: Vec3::new(p.x, self.ground_at(p), p.y), turned: along_x });
                placed += 1;
            }
        }
        // Loose cover in the open.
        let loose: &[(PropKind, usize)] = match theme {
            Theme::Suburbs => &[(PropKind::Crate, 6), (PropKind::Rock, 6)],
            Theme::DesertOutpost => &[(PropKind::Crate, 18), (PropKind::Ruin, 14), (PropKind::Rock, 26), (PropKind::Pylon, 4)],
            Theme::Farmland => &[(PropKind::Crate, 10), (PropKind::Rock, 22), (PropKind::Ruin, 4)],
            Theme::Industrial => &[(PropKind::Crate, 40), (PropKind::Ruin, 16), (PropKind::Pylon, 14), (PropKind::Rock, 10)],
            Theme::WoodsPark => &[(PropKind::Rock, 40), (PropKind::Ruin, 6)],
        };
        for &(kind, count) in loose {
            let mut placed = 0;
            for _ in 0..count * 10 {
                if placed >= count {
                    break;
                }
                let p = Vec2::new(rng.range(-self.arena, self.arena), rng.range(-self.arena, self.arena));
                if self.occupied(p, 1.8) || !self.base_clear(p, 2.0) || self.ground_at(p) < 0.6 || self.clutter.iter().any(|c| Vec2::new(c.pos.x, c.pos.z).distance(p) < 2.5) {
                    continue;
                }
                let scale = match kind {
                    PropKind::Ruin => rng.range(0.6, 0.8),
                    PropKind::Rock => rng.range(0.8, 1.5),
                    PropKind::Pylon => rng.range(0.7, 1.0),
                    _ => 1.0,
                };
                let yaw = if kind == PropKind::Rock { rng.below(4) as f32 * std::f32::consts::FRAC_PI_2 } else { 0.0 };
                self.props.push(Prop { kind, pos: Vec3::new(p.x, self.ground_at(p), p.y), yaw, scale });
                placed += 1;
            }
        }
        // Trees and bushes (the woods have theirs already).
        let (trees, palms, bushes, tufts) = match theme {
            Theme::Suburbs => (60, 0, 90, 5200),
            Theme::DesertOutpost => (0, 18, 14, 700),
            Theme::Farmland => (40, 0, 50, 4800),
            Theme::Industrial => (36, 4, 40, 1800),
            Theme::WoodsPark => (0, 0, 230, 5600),
        };
        let lining = theme == Theme::Suburbs || theme == Theme::Industrial;
        let mut placed = 0;
        for _ in 0..trees * 12 {
            if placed >= trees {
                break;
            }
            let p = if lining && !self.roads.is_empty() {
                // Street trees, along the kerbs.
                let (a, b, w) = self.roads[rng.below(self.roads.len())];
                let along_x = (b.x - a.x).abs() > (b.y - a.y).abs();
                let off = (w * 0.5 + 1.4) * if rng.chance(0.5) { 1.0 } else { -1.0 };
                a.lerp(b, rng.unit()) + if along_x { Vec2::new(0.0, off) } else { Vec2::new(off, 0.0) }
            } else {
                Vec2::new(rng.range(-118.0, 118.0), rng.range(-118.0, 118.0))
            };
            if self.ground_at(p) < 0.6 || self.occupied(p, 1.4) || !self.base_clear(p, 1.0) || self.near_road(p, 0.8) {
                continue;
            }
            self.props.push(Prop { kind: PropKind::Tree, pos: Vec3::new(p.x, self.ground_at(p), p.y), yaw: rng.range(0.0, TAU), scale: rng.range(3.6, 5.4) });
            placed += 1;
        }
        // A belt of trees round the outside of the town.
        let edge = match theme {
            Theme::Suburbs => 110,
            Theme::Farmland => 60,
            Theme::Industrial => 30,
            _ => 0,
        };
        let mut placed = 0;
        for _ in 0..edge * 8 {
            if placed >= edge {
                break;
            }
            let p = Vec2::new(rng.range(-124.0, 124.0), rng.range(-124.0, 124.0));
            if p.x.abs().max(p.y.abs()) < 86.0 || self.ground_at(p) < 0.6 || self.occupied(p, 1.6) || !self.base_clear(p, 1.0) || self.near_road(p, 1.0) {
                continue;
            }
            self.props.push(Prop { kind: PropKind::Tree, pos: Vec3::new(p.x, self.ground_at(p), p.y), yaw: rng.range(0.0, TAU), scale: rng.range(3.8, 5.8) });
            placed += 1;
        }
        for _ in 0..palms {
            let p = Vec2::new(rng.range(-118.0, 118.0), rng.range(-118.0, 118.0));
            if self.ground_at(p) > 0.6 && !self.occupied(p, 1.2) && self.base_clear(p, 1.0) && !self.near_road(p, 1.0) {
                self.props.push(Prop { kind: PropKind::Palm, pos: Vec3::new(p.x, self.ground_at(p), p.y), yaw: rng.range(0.0, TAU), scale: rng.range(2.8, 5.0) });
            }
        }
        let mut placed = 0;
        for _ in 0..bushes * 6 {
            if placed >= bushes {
                break;
            }
            let p = Vec2::new(rng.range(-self.arena, self.arena), rng.range(-self.arena, self.arena));
            if self.ground_at(p) < 0.6 || self.occupied(p, 0.9) || p.length() < 5.0 || self.near_road(p, 0.3) {
                continue;
            }
            self.props.push(Prop { kind: PropKind::Bush, pos: Vec3::new(p.x, self.ground_at(p), p.y), yaw: rng.range(0.0, TAU), scale: rng.range(0.7, 1.2) });
            placed += 1;
        }
        for _ in 0..tufts {
            let p = Vec2::new(rng.range(-120.0, 120.0), rng.range(-120.0, 120.0));
            let h = self.ground_at(p);
            if h < self.water + 0.5 || self.near_road(p, 0.0) || self.occupied(p, 0.3) {
                continue;
            }
            self.grass.push(Vec3::new(p.x, h, p.y));
        }
    }
}

pub(crate) fn distance_to_segment(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}
