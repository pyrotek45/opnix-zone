//! The two fixed maps of the horde mode (round-based survival against scrap husks).
//!
//! * **Cinder Works** - an abandoned foundry compound at night. You start in a walled
//!   courtyard; doors and rubble open onto a racked warehouse, a block of offices with the
//!   generator room, a furnace hall still glowing with slag, and an open loading dock.
//! * **Blackfen Station** - a swamp research station in thick fog. You start on the landing
//!   pad; round it sit a reed-choked boardwalk, a greenhouse lab, a specimen lab of glowing
//!   tanks, a pump house and a cold store.
//!
//! Besides the geometry, each map fills in a [`HordeLayout`]: the zones and the doors
//! between them, barricaded windows (the only way husks get in) and where husks gather outside them, chalk
//! outlines of weapons for sale, perk machines, the salvage crate's spots, the reforge and the
//! power switch. The simulation and the client both read it; nothing here is replicated.

use super::{Layout, Prop, PropKind, World};
use crate::{collide::Aabb, kit::gun, model::Paint, noise, rng::Rng};
use glam::{Vec2, Vec3};
use std::f32::consts::TAU;

/// Wall thickness.
const T: f32 = 0.4;
/// Window openings: sill and top above the floor, and width.
pub const SILL: f32 = 1.0;
pub const LINTEL: f32 = 2.4;
pub const WINDOW: f32 = 1.5;
/// How tall a buyable doorway is.
pub const DOORWAY: f32 = 2.9;
/// Ground height of each map's play area.
const FOUNDRY_GROUND: f32 = 0.5;
const FEN_GROUND: f32 = 1.0;

/// A room (or yard) that opens as a whole. Zone 0 is where the squad starts.
#[derive(Clone, Debug)]
pub struct HordeZone {
    pub name: &'static str,
    /// (x0, x1, z0, z1).
    pub rect: (f32, f32, f32, f32),
}

impl HordeZone {
    pub fn contains(&self, p: Vec2) -> bool {
        let (x0, x1, z0, z1) = self.rect;
        p.x >= x0 && p.x <= x1 && p.y >= z0 && p.y <= z1
    }
}

/// A buyable door or pile of rubble between two zones.
#[derive(Clone, Copy, Debug)]
pub struct HordeDoor {
    /// What blocks the doorway while it is shut.
    pub aabb: Aabb,
    pub zones: (u8, u8),
    pub price: u32,
    /// Rubble rather than a steel shutter.
    pub debris: bool,
    /// The wall it sits in runs along X (else Z).
    pub along_x: bool,
}

/// A window husks tear their way in through. Players board it back up.
#[derive(Clone, Copy, Debug)]
pub struct HordeWindow {
    /// Middle of the opening at floor level, in the middle of the wall.
    pub center: Vec3,
    pub along_x: bool,
    /// Unit vector out of the building (horizontal, x/z).
    pub out: Vec2,
    /// Where a husk stands inside once through, and outside while tearing at the boards.
    pub inside: Vec3,
    pub outside: Vec3,
    pub zone: u8,
    /// Where husks for this window turn up: out of sight, well outside every zone, with a
    /// clear straight walk in to `outside`.
    pub spawn: Vec3,
    /// How far either side of `spawn` (along the wall) is clear too.
    pub spread: f32,
}

impl HordeWindow {
    /// What stops players climbing out (and nothing else).
    pub fn blocker(&self) -> Aabb {
        let half = if self.along_x { Vec3::new(WINDOW * 0.5, 0.0, T * 0.5 + 0.05) } else { Vec3::new(T * 0.5 + 0.05, 0.0, WINDOW * 0.5) };
        Aabb::span(self.center + Vec3::new(-half.x, SILL - 0.05, -half.z), self.center + Vec3::new(half.x, LINTEL, half.z))
    }
}

/// A chalk outline on a wall: buy the gun, or ammo for it once you own it.
#[derive(Clone, Copy, Debug)]
pub struct WallBuy {
    /// On the wall face, at chest height.
    pub pos: Vec3,
    /// Out of the wall.
    pub normal: Vec2,
    /// Catalog weapon index (see [`crate::kit::gun`]).
    pub weapon: u8,
    pub price: u32,
    pub zone: u8,
}

/// The chalk frame drawn round every wall gun: width along the wall, and height (centred on
/// [`WallBuy::pos`]).
pub const CHALK_FRAME: Vec2 = Vec2::new(1.6, 0.8);
/// Clear space kept between a wall gun's outline and its frame, on every side.
pub const CHALK_MARGIN: f32 = 0.09;

/// How to draw a gun model inside its chalk frame: the uniform scale, and the model-space
/// point that goes at the frame's centre. `lo`/`hi` are the model's bounds (+Y up, -Z the
/// muzzle; length along Z goes along the wall). Long guns shrink to fit; small ones grow a
/// little, but not past 1.6 times life size.
pub fn chalk_fit(lo: Vec3, hi: Vec3) -> (f32, Vec3) {
    let size = (hi - lo).max(Vec3::splat(1e-3));
    let room = CHALK_FRAME - Vec2::splat(CHALK_MARGIN * 2.0);
    let scale = (room.x / size.z).min(room.y / size.y).min(1.6);
    (scale, (lo + hi) * 0.5)
}

/// A machine standing against a wall:a perk dispenser, a crate spot, the reforge, the switch.
#[derive(Clone, Copy, Debug)]
pub struct HordeMachine {
    /// Floor point in front of the machine, where you stand to use it.
    pub pos: Vec3,
    /// The way the machine faces (towards whoever uses it).
    pub facing: Vec2,
    pub zone: u8,
    /// Which perk, for perk machines.
    pub kind: u8,
}

/// Everything the horde mode needs to know about a map besides its blocks.
#[derive(Clone, Debug, Default)]
pub struct HordeLayout {
    pub name: &'static str,
    pub zones: Vec<HordeZone>,
    pub doors: Vec<HordeDoor>,
    pub windows: Vec<HordeWindow>,
    pub buys: Vec<WallBuy>,
    pub perks: Vec<HordeMachine>,
    /// Where the salvage crate can be; it starts at the first.
    pub crates: Vec<HordeMachine>,
    pub forge: Option<HordeMachine>,
    pub power: Option<HordeMachine>,
    /// Ground height of the play area.
    pub ground: f32,
}

impl HordeLayout {
    /// The zone a point lies in, if any.
    pub fn zone_at(&self, p: Vec2) -> Option<u8> {
        self.zones.iter().position(|z| z.contains(p)).map(|i| i as u8)
    }

    /// Zones that are open with doors `open` (a bitmask of door indices) open.
    pub fn open_zones(&self, open: u64) -> u64 {
        let mut zones = 1u64;
        for (i, door) in self.doors.iter().enumerate() {
            if open & (1 << i) != 0 {
                zones |= (1 << door.zones.0) | (1 << door.zones.1);
            }
        }
        zones
    }
}

/// Perk ids (shared with the simulation's `zombies::perk`).
pub mod perk_id {
    pub const IRON_HIDE: u8 = 0;
    pub const QUICKFIX: u8 = 1;
    pub const HAIR_TRIGGER: u8 = 2;
    pub const PATCH_UP: u8 = 3;
    pub const FLEETFOOT: u8 = 4;
    pub const SPARE_SOCKET: u8 = 5;
    pub const SCRAP_MAGNET: u8 = 6;
}

/// An opening in a wall line, centred `at` (world coordinate along the line).
#[derive(Clone, Copy)]
enum Cut {
    /// A buyable doorway (or any open passage), `width` wide.
    Door(f32, f32),
    /// A barricaded window into zone `zone`; `out` is +1 when outside is the positive side.
    Window(f32, u8, f32),
    /// A plain interior doorway.
    Pass(f32, f32),
}

impl World {
    pub(super) fn horde_height(&self, x: f32, z: f32) -> f32 {
        let seed = self.recipe.seed;
        match self.recipe.layout {
            Layout::Blackfen => {
                // The station stands on a raised pad; outside it the fen sinks into black pools.
                let d = (x - (-1.0)).abs().max(0.0) - 44.0;
                let e = (z - (-1.0)).abs() - 42.0;
                let out = d.max(e).max(0.0);
                let pools = noise::fbm(seed ^ 0xfe7, x * 0.06, z * 0.06, 3);
                let sink = (out / 8.0).clamp(0.0, 1.0);
                FEN_GROUND - sink * (1.0 + 1.6 * (pools + 0.3).max(0.0)) + 0.25 * sink * noise::value(seed ^ 0x51, x * 0.2, z * 0.2)
            }
            _ => {
                let out = ((x - 11.0).abs() - 42.0).max((z.abs()) - 42.0).max(0.0);
                FOUNDRY_GROUND + (out / 12.0).clamp(0.0, 1.0) * 0.6 * noise::fbm(seed ^ 0xc1d, x * 0.05, z * 0.05, 2)
            }
        }
    }

    /// A straight wall from `a` to `b` (along one axis), `h` tall, with its cuts. Windows and
    /// doors are recorded in `lay`.
    fn zline(&mut self, lay: &mut HordeLayout, a: Vec2, b: Vec2, h: f32, cuts: &[Cut], paint: Paint) {
        let g = lay.ground;
        let along_x = (b.x - a.x).abs() > (b.y - a.y).abs();
        let (start, end) = if along_x { (a.x.min(b.x), a.x.max(b.x)) } else { (a.y.min(b.y), a.y.max(b.y)) };
        let across = if along_x { a.y } else { a.x };
        let mut openings = Vec::new();
        for cut in cuts {
            match *cut {
                Cut::Door(at, width) | Cut::Pass(at, width) => openings.push((at - width * 0.5 - start, at + width * 0.5 - start, 0.0, DOORWAY)),
                Cut::Window(at, zone, out) => {
                    openings.push((at - WINDOW * 0.5 - start, at + WINDOW * 0.5 - start, SILL, LINTEL));
                    let center = if along_x { Vec3::new(at, g, across) } else { Vec3::new(across, g, at) };
                    let n = if along_x { Vec2::new(0.0, out) } else { Vec2::new(out, 0.0) };
                    let n3 = Vec3::new(n.x, 0.0, n.y);
                    lay.windows.push(HordeWindow { center, along_x, out: n, inside: center - n3 * 1.0, outside: center + n3 * 1.1, zone, spawn: center + n3 * 1.1, spread: 0.0 });
                    // A caged lamp over the inside of every window, so the way they come in
                    // reads at a glance in the dark.
                    let lamp = center - n3 * (T * 0.5 + 0.06) + Vec3::Y * (LINTEL + 0.22);
                    let half = if along_x { Vec3::new(0.32, 0.07, 0.06) } else { Vec3::new(0.06, 0.07, 0.32) };
                    self.decor(Aabb::new(lamp, half), Paint::Glow);
                    self.decor(Aabb::new(lamp + Vec3::Y * 0.09, half + Vec3::new(0.03, -0.05, 0.03)), Paint::Dark);
                }
            }
        }
        self.wall(along_x, (across - T * 0.5, across + T * 0.5), start, end - start, g, h, &openings, paint);
        // A dark plinth and a cap along the top read better than a bare slab.
        let span = |y0: f32, y1: f32, grow: f32| {
            if along_x {
                Aabb::span(Vec3::new(start, g + y0, across - T * 0.5 - grow), Vec3::new(end, g + y1, across + T * 0.5 + grow))
            } else {
                Aabb::span(Vec3::new(across - T * 0.5 - grow, g + y0, start), Vec3::new(across + T * 0.5 + grow, g + y1, end))
            }
        };
        self.decor(span(h - 0.12, h + 0.04, 0.06), Paint::Dark);
    }

    /// A buyable doorway's blocker, recorded in `lay`.
    fn buy_door(&mut self, lay: &mut HordeLayout, at: Vec2, along_x: bool, width: f32, zones: (u8, u8), price: u32, debris: bool) {
        let g = lay.ground;
        let half = if along_x { Vec3::new(width * 0.5, DOORWAY * 0.5, 0.32) } else { Vec3::new(0.32, DOORWAY * 0.5, width * 0.5) };
        lay.doors.push(HordeDoor { aabb: Aabb::new(Vec3::new(at.x, g + DOORWAY * 0.5, at.y), half), zones, price, debris, along_x });
    }

    /// A wall buy on the face of a wall: `at` is the point on the face, `normal` points out of it.
    fn wall_buy(lay: &mut HordeLayout, at: Vec2, normal: Vec2, weapon: usize, price: u32, zone: u8) {
        lay.buys.push(WallBuy { pos: Vec3::new(at.x, lay.ground + 1.45, at.y), normal, weapon: weapon as u8, price, zone });
    }

    /// A machine body against a wall (`back` is the point on the wall face behind it).
    fn machine(&mut self, lay: &mut HordeLayout, back: Vec2, facing: Vec2, zone: u8, kind: u8, height: f32) -> HordeMachine {
        let g = lay.ground;
        let c = back + facing * 0.45;
        let half = if facing.x.abs() > 0.5 { Vec2::new(0.42, 0.6) } else { Vec2::new(0.6, 0.42) };
        if height > 0.0 {
            self.solid(Aabb::span(Vec3::new(c.x - half.x, g, c.y - half.y), Vec3::new(c.x + half.x, g + height, c.y + half.y)), Paint::Dark);
        }
        let front = back + facing * 1.4;
        HordeMachine { pos: Vec3::new(front.x, g, front.y), facing, zone, kind }
    }

    fn lamp(&mut self, p: Vec2) {
        self.props.push(Prop { kind: PropKind::Lamp, pos: Vec3::new(p.x, self.height(p.x, p.y), p.y), yaw: 0.0, scale: 1.0 });
    }

    /// A hanging work lamp under a ceiling at `y`.
    fn hanging_light(&mut self, p: Vec2, y: f32, paint: Paint) {
        self.decor(Aabb::span(Vec3::new(p.x - 0.02, y - 0.9, p.y - 0.02), Vec3::new(p.x + 0.02, y, p.y + 0.02)), Paint::Dark);
        self.decor(Aabb::span(Vec3::new(p.x - 0.35, y - 1.0, p.y - 0.2), Vec3::new(p.x + 0.35, y - 0.9, p.y + 0.2)), Paint::Steel);
        self.decor(Aabb::span(Vec3::new(p.x - 0.3, y - 1.03, p.y - 0.15), Vec3::new(p.x + 0.3, y - 1.0, p.y + 0.15)), paint);
    }

    /// A crate stack: low cover.
    fn crates(&mut self, p: Vec2, high: bool) {
        let g = self.height(p.x, p.y);
        self.solid(Aabb::span(Vec3::new(p.x - 0.6, g, p.y - 0.6), Vec3::new(p.x + 0.6, g + 1.1, p.y + 0.6)), Paint::Wood);
        self.decor(Aabb::span(Vec3::new(p.x - 0.62, g + 0.5, p.y - 0.62), Vec3::new(p.x + 0.62, g + 0.6, p.y + 0.62)), Paint::Dark);
        if high {
            self.solid(Aabb::span(Vec3::new(p.x - 0.5, g + 1.1, p.y - 0.45), Vec3::new(p.x + 0.4, g + 1.95, p.y + 0.45)), Paint::Wood);
        }
    }

    /// Cinder Works: the foundry compound.
    pub(super) fn cinder_works(&mut self, rng: &mut Rng) {
        let g = FOUNDRY_GROUND;
        let mut lay = HordeLayout { name: "CINDER WORKS", ground: g, ..Default::default() };
        use Cut::{Door, Window};
        lay.zones = vec![
            HordeZone { name: "COURTYARD", rect: (-12.0, 12.0, -12.0, 10.0) },
            HordeZone { name: "WAREHOUSE", rect: (-22.0, 14.0, 10.0, 32.0) },
            HordeZone { name: "OFFICES", rect: (12.0, 32.0, -12.0, 8.0) },
            HordeZone { name: "FURNACE HALL", rect: (14.0, 44.0, 8.0, 32.0) },
            HordeZone { name: "LOADING DOCK", rect: (-22.0, 12.0, -32.0, -12.0) },
        ];
        let brick = Paint::Rust;
        let sheet = Paint::Steel;
        // Floors: cracked concrete outside, boards and plate indoors.
        self.flat(-12.0, 12.0, -12.0, 10.0, g + 0.002, Paint::Rock);
        self.flat(-22.0, 14.0, 10.0, 32.0, g + 0.002, Paint::Concrete);
        self.flat(12.0, 32.0, -12.0, 8.0, g + 0.002, Paint::Wood);
        self.flat(14.0, 44.0, 8.0, 32.0, g + 0.002, Paint::Iron);
        self.flat(-22.0, 12.0, -32.0, -12.0, g + 0.002, Paint::Rock);

        // The wall lines.
        self.zline(&mut lay, Vec2::new(-22.0, 10.0), Vec2::new(14.0, 10.0), 6.5, &[Door(0.0, 2.4)], brick);
        self.zline(&mut lay, Vec2::new(12.0, -12.0), Vec2::new(12.0, 10.0), 4.5, &[Door(-2.0, 2.4)], brick);
        self.zline(&mut lay, Vec2::new(14.0, 8.0), Vec2::new(14.0, 32.0), 9.0, &[Door(21.0, 2.4)], sheet);
        self.zline(&mut lay, Vec2::new(12.0, 8.0), Vec2::new(44.0, 8.0), 4.0, &[Door(19.0, 2.4), Window(38.0, 3, -1.0)], brick);
        self.zline(&mut lay, Vec2::new(-22.0, -12.0), Vec2::new(32.0, -12.0), 4.5, &[Door(0.0, 2.4), Window(22.0, 2, -1.0)], brick);
        // Outer walls.
        self.zline(&mut lay, Vec2::new(-12.0, -12.0), Vec2::new(-12.0, 10.0), 4.5, &[Window(-7.0, 0, -1.0), Window(-1.0, 0, -1.0), Window(5.0, 0, -1.0)], brick);
        self.zline(&mut lay, Vec2::new(-22.0, 10.0), Vec2::new(-22.0, 32.0), 6.5, &[Window(16.0, 1, -1.0), Window(26.0, 1, -1.0)], sheet);
        self.zline(&mut lay, Vec2::new(-22.0, 32.0), Vec2::new(14.0, 32.0), 6.5, &[Window(-14.0, 1, 1.0), Window(-2.0, 1, 1.0), Window(8.0, 1, 1.0)], sheet);
        self.zline(&mut lay, Vec2::new(14.0, 32.0), Vec2::new(44.0, 32.0), 9.0, &[Window(22.0, 3, 1.0), Window(36.0, 3, 1.0)], brick);
        self.zline(&mut lay, Vec2::new(44.0, 8.0), Vec2::new(44.0, 32.0), 9.0, &[Window(14.0, 3, 1.0), Window(25.0, 3, 1.0)], brick);
        self.zline(&mut lay, Vec2::new(32.0, -12.0), Vec2::new(32.0, 8.0), 4.0, &[Window(-2.0, 2, 1.0)], brick);
        self.zline(&mut lay, Vec2::new(-22.0, -32.0), Vec2::new(12.0, -32.0), 4.5, &[Window(-14.0, 4, -1.0), Window(2.0, 4, -1.0)], brick);
        self.zline(&mut lay, Vec2::new(-22.0, -32.0), Vec2::new(-22.0, -12.0), 4.5, &[Window(-22.0, 4, -1.0)], brick);
        self.zline(&mut lay, Vec2::new(12.0, -32.0), Vec2::new(12.0, -12.0), 4.5, &[Window(-26.0, 4, 1.0)], brick);
        // The sealed nook where four buildings meet.
        self.solid(Aabb::span(Vec3::new(12.0, g, 8.0), Vec3::new(14.0, g + 6.5, 10.0)), brick);

        // Buyable ways through.
        self.buy_door(&mut lay, Vec2::new(0.0, 10.0), true, 2.4, (0, 1), 750, false);
        self.buy_door(&mut lay, Vec2::new(12.0, -2.0), false, 2.4, (0, 2), 750, true);
        self.buy_door(&mut lay, Vec2::new(14.0, 21.0), false, 2.4, (1, 3), 1000, false);
        self.buy_door(&mut lay, Vec2::new(19.0, 8.0), true, 2.4, (2, 3), 1000, true);
        self.buy_door(&mut lay, Vec2::new(0.0, -12.0), true, 2.4, (0, 4), 1000, false);

        // Roofs: decor only (nothing climbs up there), with skylights over the furnace.
        self.decor(Aabb::span(Vec3::new(-22.2, g + 6.5, 9.8), Vec3::new(14.2, g + 6.75, 32.2)), Paint::Iron);
        self.decor(Aabb::span(Vec3::new(11.8, g + 4.0, -12.2), Vec3::new(32.2, g + 4.25, 8.2)), Paint::Dark);
        self.decor(Aabb::span(Vec3::new(13.8, g + 9.0, 7.8), Vec3::new(44.2, g + 9.3, 32.2)), Paint::Iron);
        // Roof trusses in the warehouse and the hall.
        for x in [-16.0, -8.0, 0.0, 8.0] {
            self.decor(Aabb::span(Vec3::new(x - 0.12, g + 5.9, 10.0), Vec3::new(x + 0.12, g + 6.5, 32.0)), Paint::Dark);
        }
        for x in [20.0, 26.0, 32.0, 38.0] {
            self.decor(Aabb::span(Vec3::new(x - 0.15, g + 8.2, 8.0), Vec3::new(x + 0.15, g + 9.0, 32.0)), Paint::Dark);
        }

        self.foundry_courtyard(&mut lay, rng);
        self.foundry_warehouse(&mut lay, rng);
        self.foundry_offices(&mut lay, rng);
        self.foundry_furnace(&mut lay, rng);
        self.foundry_dock(&mut lay, rng);
        self.foundry_outside(rng);

        self.spawn = Vec2::new(0.0, -2.0);
        self.waypoints_over(g, (-11.0, 11.0, -11.0, 9.0), 5.0);
        self.finish_horde(lay);
    }

    fn foundry_courtyard(&mut self, lay: &mut HordeLayout, rng: &mut Rng) {
        let g = lay.ground;
        // A dead flatbed and some pallets for cover; puddles.
        self.solid(Aabb::span(Vec3::new(3.0, g, -8.0), Vec3::new(5.6, g + 1.3, -2.5)), Paint::Steel);
        self.solid(Aabb::span(Vec3::new(3.1, g, -9.6), Vec3::new(5.5, g + 2.2, -8.0)), Paint::Coral);
        self.decor(Aabb::span(Vec3::new(3.3, g + 1.5, -9.62), Vec3::new(5.3, g + 2.0, -9.6)), Paint::Screen);
        for (x, z) in [(2.6, -7.2), (6.0, -7.2), (2.6, -3.3), (6.0, -3.3)] {
            self.decor(Aabb::span(Vec3::new(x - 0.2, g, z - 0.35), Vec3::new(x + 0.2, g + 0.7, z + 0.35)), Paint::Dark);
        }
        self.crates(Vec2::new(-7.5, 7.0), true);
        self.crates(Vec2::new(8.5, 6.0), false);
        self.crates(Vec2::new(-8.0, -9.5), false);
        for (x, z, w, d) in [(-3.0, 3.0, 2.4, 1.4), (6.5, 1.0, 1.6, 2.2), (-6.0, -2.0, 1.2, 1.0)] {
            self.flat(x - w, x + w, z - d, z + d, g + 0.006, Paint::Steel);
        }
        self.lamp(Vec2::new(-10.8, 8.8));
        self.lamp(Vec2::new(10.8, -10.8));
        Self::wall_buy(lay, Vec2::new(-6.0, 9.8), Vec2::NEG_Y, gun::DMR, 500, 0);
        Self::wall_buy(lay, Vec2::new(11.8, 5.0), Vec2::NEG_X, gun::MACHINE_PISTOL, 600, 0);
        let m = self.machine(lay, Vec2::new(-11.8, -10.0), Vec2::X, 0, perk_id::PATCH_UP, 2.2);
        lay.perks.push(m);
        let _ = rng;
    }

    fn foundry_warehouse(&mut self, lay: &mut HordeLayout, rng: &mut Rng) {
        let g = lay.ground;
        // Pallet racks: tall rows with gaps to run through.
        for x in [-16.0, -9.0] {
            for (z0, z1) in [(13.5, 20.0), (23.0, 29.0)] {
                self.solid(Aabb::span(Vec3::new(x - 0.6, g, z0), Vec3::new(x + 0.6, g + 3.2, z1)), Paint::Steel);
                for k in 0..3 {
                    let y = g + 0.9 + k as f32 * 1.0;
                    self.decor(Aabb::span(Vec3::new(x - 0.65, y, z0 - 0.02), Vec3::new(x + 0.65, y + 0.08, z1 + 0.02)), Paint::Gold);
                }
            }
        }
        // A forklift and crates in the open bay.
        self.solid(Aabb::span(Vec3::new(1.0, g, 22.0), Vec3::new(2.4, g + 1.5, 24.4)), Paint::Gold);
        self.decor(Aabb::span(Vec3::new(1.1, g + 0.1, 21.2), Vec3::new(1.3, g + 2.6, 22.0)), Paint::Dark);
        self.decor(Aabb::span(Vec3::new(2.1, g + 0.1, 21.2), Vec3::new(2.3, g + 2.6, 22.0)), Paint::Dark);
        self.crates(Vec2::new(-3.0, 16.0), true);
        self.crates(Vec2::new(7.0, 26.0), true);
        self.crates(Vec2::new(8.4, 26.2), false);
        self.crates(Vec2::new(-4.5, 27.0), false);
        for x in [-14.0, -4.0, 6.0] {
            for z in [15.0, 27.0] {
                self.hanging_light(Vec2::new(x, z), g + 6.5, Paint::Glow);
            }
        }
        Self::wall_buy(lay, Vec2::new(8.0, 10.2), Vec2::Y, gun::SMG, 1000, 1);
        Self::wall_buy(lay, Vec2::new(-21.8, 21.0), Vec2::X, gun::PDW, 1100, 1);
        let m = self.machine(lay, Vec2::new(-12.5, 10.2), Vec2::Y, 1, perk_id::IRON_HIDE, 2.2);
        lay.perks.push(m);
        let m = self.machine(lay, Vec2::new(13.8, 14.5), Vec2::NEG_X, 1, perk_id::SCRAP_MAGNET, 2.2);
        lay.perks.push(m);
        let c = self.machine(lay, Vec2::new(-4.0, 31.8), Vec2::NEG_Y, 1, 0, 0.0);
        lay.crates.push(c);
        let _ = rng;
    }

    fn foundry_offices(&mut self, lay: &mut HordeLayout, rng: &mut Rng) {
        let g = lay.ground;
        // A partition with two doorways: the clerks' office west, the generator room east.
        self.zline(lay, Vec2::new(23.0, -12.0), Vec2::new(23.0, 8.0), 4.0, &[Cut::Pass(-7.0, 1.8), Cut::Pass(3.0, 1.8)], Paint::Bone);
        // Desks with their chairs to the north, so the way in from the window stays clear.
        for (x, z) in [(15.5, -7.6), (19.5, -7.6), (15.5, -2.6), (19.5, -2.6)] {
            self.desk(rng, Vec2::new(x, z), Vec2::Y, g);
        }
        self.filing_cabinets(Vec2::new(13.0, 1.0), 3, g);
        self.filing_cabinets(Vec2::new(14.0, -11.2), 2, g);
        // The generator: a big humming block with cables.
        self.solid(Aabb::span(Vec3::new(25.5, g, -4.0), Vec3::new(29.5, g + 2.3, 1.0)), Paint::Iron);
        self.decor(Aabb::span(Vec3::new(25.4, g + 1.6, -3.6), Vec3::new(25.45, g + 2.1, 0.6)), Paint::Screen);
        for k in 0..5 {
            let z = -3.4 + k as f32 * 0.9;
            self.decor(Aabb::span(Vec3::new(25.35, g + 0.5, z), Vec3::new(25.5, g + 1.2, z + 0.5)), Paint::Dark);
        }
        self.decor(Aabb::span(Vec3::new(27.3, g, 1.0), Vec3::new(27.6, g + 0.1, 7.6)), Paint::Dark);
        for z in [-6.0, 4.0] {
            self.hanging_light(Vec2::new(17.5, z), g + 4.0, Paint::Glow);
            self.hanging_light(Vec2::new(27.5, z), g + 4.0, Paint::Spark);
        }
        Self::wall_buy(lay, Vec2::new(12.2, -8.5), Vec2::X, gun::SHOTGUN, 1200, 2);
        Self::wall_buy(lay, Vec2::new(28.0, 7.8), Vec2::NEG_Y, gun::CARBINE, 1400, 2);
        let m = self.machine(lay, Vec2::new(16.0, 7.8), Vec2::NEG_Y, 2, perk_id::QUICKFIX, 2.2);
        lay.perks.push(m);
        let p = self.machine(lay, Vec2::new(31.8, -8.0), Vec2::NEG_X, 2, 0, 0.0);
        lay.power = Some(p);
    }

    fn foundry_furnace(&mut self, lay: &mut HordeLayout, rng: &mut Rng) {
        let g = lay.ground;
        // The furnace: a brick block with a glowing mouth, a hood and a stack through the roof.
        let (x0, x1, z0, z1) = (29.0, 36.0, 17.0, 24.0);
        self.solid(Aabb::span(Vec3::new(x0, g, z0), Vec3::new(x1, g + 6.0, z1)), Paint::Rust);
        self.solid(Aabb::span(Vec3::new(x0 + 1.0, g + 6.0, z0 + 1.0), Vec3::new(x1 - 1.0, g + 14.0, z1 - 1.0)), Paint::Dark);
        self.decor(Aabb::span(Vec3::new(x0 + 1.0, g + 14.0, z0 + 1.0), Vec3::new(x1 - 1.0, g + 14.2, z1 - 1.0)), Paint::Spark);
        self.decor(Aabb::span(Vec3::new(x0 + 1.8, g + 0.6, z0 - 0.04), Vec3::new(x1 - 1.8, g + 2.6, z0)), Paint::Glow);
        self.decor(Aabb::span(Vec3::new(x0 + 2.3, g + 0.9, z0 - 0.05), Vec3::new(x1 - 2.3, g + 2.2, z0 - 0.04)), Paint::Spark);
        // Slag channels: glowing troughs behind kerbs (cover, and they light the hall).
        for (a, b) in [((17.0, 13.0), (27.0, 14.2)), ((37.5, 12.0), (42.5, 13.2)), ((17.0, 27.0), (26.0, 28.2))] {
            self.solid(Aabb::span(Vec3::new(a.0, g, a.1), Vec3::new(b.0, g + 0.7, b.1)), Paint::Iron);
            self.decor(Aabb::span(Vec3::new(a.0 + 0.2, g + 0.7, a.1 + 0.2), Vec3::new(b.0 - 0.2, g + 0.72, b.1 - 0.2)), Paint::Glow);
        }
        // Ladles and a crane rail overhead.
        self.decor(Aabb::span(Vec3::new(14.0, g + 7.4, 19.8), Vec3::new(44.0, g + 7.7, 20.4)), Paint::Gold);
        self.decor(Aabb::span(Vec3::new(23.6, g + 4.0, 19.4), Vec3::new(24.4, g + 7.4, 20.8)), Paint::Dark);
        self.solid(Aabb::span(Vec3::new(22.8, g + 2.6, 18.9), Vec3::new(25.2, g + 4.0, 21.3)), Paint::Iron);
        self.crates(Vec2::new(39.5, 28.0), true);
        self.crates(Vec2::new(19.0, 23.0), false);
        for (x, z) in [(20.0, 12.0), (38.0, 28.0), (20.0, 28.0), (40.0, 12.0)] {
            self.hanging_light(Vec2::new(x, z), g + 9.0, Paint::Glow);
        }
        Self::wall_buy(lay, Vec2::new(14.2, 27.0), Vec2::X, gun::BATTLE_RIFLE, 1600, 3);
        Self::wall_buy(lay, Vec2::new(43.8, 20.0), Vec2::NEG_X, gun::BULLPUP, 1400, 3);
        let m = self.machine(lay, Vec2::new(43.8, 10.5), Vec2::NEG_X, 3, perk_id::HAIR_TRIGGER, 2.2);
        lay.perks.push(m);
        let m = self.machine(lay, Vec2::new(30.0, 31.8), Vec2::NEG_Y, 3, perk_id::SPARE_SOCKET, 2.2);
        lay.perks.push(m);
        let f = self.machine(lay, Vec2::new(18.0, 31.8), Vec2::NEG_Y, 3, 0, 1.6);
        lay.forge = Some(f);
        let c = self.machine(lay, Vec2::new(34.0, 8.2), Vec2::Y, 3, 0, 0.0);
        lay.crates.push(c);
        let _ = rng;
    }

    fn foundry_dock(&mut self, lay: &mut HordeLayout, rng: &mut Rng) {
        let g = lay.ground;
        // Two trailers and a loading ramp.
        for (x, z) in [(-14.0, -21.0), (1.0, -24.0)] {
            self.solid(Aabb::span(Vec3::new(x - 1.3, g, z - 4.0), Vec3::new(x + 1.3, g + 2.8, z + 4.0)), Paint::Coral);
            for k in 0..6 {
                let zz = z - 3.6 + k as f32 * 1.45;
                self.decor(Aabb::span(Vec3::new(x - 1.33, g + 0.3, zz), Vec3::new(x + 1.33, g + 2.6, zz + 0.12)), Paint::Rust);
            }
        }
        self.solid(Aabb::span(Vec3::new(-6.0, g, -31.6), Vec3::new(-2.0, g + 1.1, -28.0)), Paint::Concrete);
        self.decor(Aabb::span(Vec3::new(-6.0, g + 1.1, -28.1), Vec3::new(-2.0, g + 1.14, -27.95)), Paint::Gold);
        self.crates(Vec2::new(8.0, -16.0), true);
        self.crates(Vec2::new(-18.0, -29.5), false);
        self.lamp(Vec2::new(-21.0, -31.0));
        self.lamp(Vec2::new(11.0, -13.0));
        Self::wall_buy(lay, Vec2::new(-8.0, -12.2), Vec2::NEG_Y, gun::BURST, 1200, 4);
        Self::wall_buy(lay, Vec2::new(11.8, -20.0), Vec2::NEG_X, gun::BREACHER, 1500, 4);
        let m = self.machine(lay, Vec2::new(-21.8, -14.5), Vec2::X, 4, perk_id::FLEETFOOT, 2.2);
        lay.perks.push(m);
        let c = self.machine(lay, Vec2::new(8.0, -31.8), Vec2::Y, 4, 0, 0.0);
        lay.crates.push(c);
        let _ = rng;
    }

    /// The dead works outside the walls: chimneys, silos, a fence line, scrap heaps.
    fn foundry_outside(&mut self, rng: &mut Rng) {
        for (x, z, h) in [(-34.0, 40.0, 22.0), (52.0, 38.0, 26.0), (58.0, -6.0, 18.0), (-36.0, -40.0, 20.0)] {
            self.column(Vec2::new(x, z), 1.4, self.height(x, z), self.height(x, z) + h, Paint::Rust);
            self.decor(Aabb::new(Vec3::new(x, self.height(x, z) + h + 0.2, z), Vec3::new(1.1, 0.2, 1.1)), Paint::Spark);
        }
        for (x, z) in [(-40.0, 10.0), (56.0, 18.0)] {
            self.silo(Vec2::new(x, z), 3.5, 12.0, Paint::Iron, None);
        }
        // Scrap heaps.
        for _ in 0..24 {
            let p = Vec2::new(rng.range(-60.0, 66.0), rng.range(-60.0, 60.0));
            if p.x > -32.0 && p.x < 54.0 && p.y > -42.0 && p.y < 42.0 {
                continue;
            }
            self.props.push(Prop { kind: PropKind::Ruin, pos: Vec3::new(p.x, self.height(p.x, p.y), p.y), yaw: rng.range(0.0, TAU), scale: rng.range(0.8, 1.4) });
        }
        for _ in 0..30 {
            let p = Vec2::new(rng.range(-64.0, 66.0), rng.range(-64.0, 64.0));
            if p.x > -30.0 && p.x < 52.0 && p.y > -40.0 && p.y < 40.0 {
                continue;
            }
            self.props.push(Prop { kind: PropKind::Tree, pos: Vec3::new(p.x, self.height(p.x, p.y), p.y), yaw: rng.range(0.0, TAU), scale: rng.range(3.0, 4.6) });
        }
    }

    /// Blackfen Station: the swamp research station.
    pub(super) fn blackfen(&mut self, rng: &mut Rng) {
        let g = FEN_GROUND;
        let mut lay = HordeLayout { name: "BLACKFEN STATION", ground: g, ..Default::default() };
        use Cut::{Door, Window};
        lay.zones = vec![
            HordeZone { name: "LANDING PAD", rect: (-10.0, 10.0, -10.0, 10.0) },
            HordeZone { name: "BOARDWALK", rect: (-36.0, -10.0, -7.0, 10.0) },
            HordeZone { name: "GREENHOUSE", rect: (-14.0, 14.0, 10.0, 30.0) },
            HordeZone { name: "SPECIMEN LAB", rect: (10.0, 34.0, -12.0, 10.0) },
            HordeZone { name: "PUMP HOUSE", rect: (-26.0, -2.0, -32.0, -10.0) },
            HordeZone { name: "COLD STORE", rect: (-36.0, -14.0, 10.0, 30.0) },
        ];
        let panel = Paint::Shell;
        let metal = Paint::Steel;
        let glass = Paint::Teal;
        self.flat(-10.0, 10.0, -10.0, 10.0, g + 0.002, Paint::Dark);
        self.flat(-36.0, -10.0, -7.0, 10.0, g + 0.002, Paint::Leaf);
        self.flat(-14.0, 14.0, 10.0, 30.0, g + 0.002, Paint::Bone);
        self.flat(10.0, 34.0, -12.0, 10.0, g + 0.002, Paint::Metal);
        self.flat(-26.0, -2.0, -32.0, -10.0, g + 0.002, Paint::Concrete);
        self.flat(-36.0, -14.0, 10.0, 30.0, g + 0.002, Paint::Steel);

        // Shared walls.
        self.zline(&mut lay, Vec2::new(-10.0, -10.0), Vec2::new(-10.0, 10.0), 3.4, &[Door(1.5, 2.4)], metal);
        self.zline(&mut lay, Vec2::new(-36.0, 10.0), Vec2::new(14.0, 10.0), 4.2, &[Door(0.0, 2.4), Door(-25.0, 2.4)], panel);
        self.zline(&mut lay, Vec2::new(10.0, -12.0), Vec2::new(10.0, 10.0), 3.8, &[Door(0.0, 2.4)], metal);
        // The pump house sits off to the south-west, so the pad's south wall looks out on the fen.
        self.zline(&mut lay, Vec2::new(-26.0, -10.0), Vec2::new(-2.0, -10.0), 5.0, &[Door(-6.0, 2.4)], Paint::Concrete);
        self.zline(&mut lay, Vec2::new(-2.0, -10.0), Vec2::new(10.0, -10.0), 3.4, &[Window(0.2, 0, -1.0), Window(4.5, 0, -1.0), Window(8.5, 0, -1.0)], metal);
        self.zline(&mut lay, Vec2::new(-14.0, 10.0), Vec2::new(-14.0, 30.0), 4.2, &[Door(20.0, 2.4)], panel);
        // Outer walls.
        self.zline(&mut lay, Vec2::new(-36.0, -7.0), Vec2::new(-10.0, -7.0), 3.4, &[Window(-33.0, 1, -1.0), Window(-28.5, 1, -1.0)], metal);
        self.zline(&mut lay, Vec2::new(-36.0, -7.0), Vec2::new(-36.0, 10.0), 3.4, &[Window(2.0, 1, -1.0)], metal);
        self.zline(&mut lay, Vec2::new(-14.0, 30.0), Vec2::new(14.0, 30.0), 4.2, &[Window(-7.0, 2, 1.0), Window(7.0, 2, 1.0)], glass);
        self.zline(&mut lay, Vec2::new(14.0, 10.0), Vec2::new(14.0, 30.0), 4.2, &[Window(22.0, 2, 1.0)], glass);
        self.zline(&mut lay, Vec2::new(-36.0, 30.0), Vec2::new(-14.0, 30.0), 4.2, &[Window(-26.0, 5, 1.0)], panel);
        self.zline(&mut lay, Vec2::new(-36.0, 10.0), Vec2::new(-36.0, 30.0), 4.2, &[Window(20.0, 5, -1.0)], panel);
        self.zline(&mut lay, Vec2::new(14.0, 10.0), Vec2::new(34.0, 10.0), 3.8, &[Window(26.0, 3, 1.0)], metal);
        self.zline(&mut lay, Vec2::new(34.0, -12.0), Vec2::new(34.0, 10.0), 3.8, &[Window(-5.0, 3, 1.0), Window(4.0, 3, 1.0)], metal);
        self.zline(&mut lay, Vec2::new(10.0, -12.0), Vec2::new(34.0, -12.0), 3.8, &[Window(22.0, 3, -1.0)], metal);
        self.zline(&mut lay, Vec2::new(-26.0, -32.0), Vec2::new(-2.0, -32.0), 5.0, &[Window(-18.0, 4, -1.0), Window(-8.0, 4, -1.0)], Paint::Concrete);
        self.zline(&mut lay, Vec2::new(-26.0, -32.0), Vec2::new(-26.0, -10.0), 5.0, &[Window(-20.0, 4, -1.0)], Paint::Concrete);
        self.zline(&mut lay, Vec2::new(-2.0, -32.0), Vec2::new(-2.0, -10.0), 5.0, &[Window(-24.0, 4, 1.0)], Paint::Concrete);

        self.buy_door(&mut lay, Vec2::new(-10.0, 1.5), false, 2.4, (0, 1), 750, true);
        self.buy_door(&mut lay, Vec2::new(0.0, 10.0), true, 2.4, (0, 2), 1000, false);
        self.buy_door(&mut lay, Vec2::new(10.0, 0.0), false, 2.4, (0, 3), 750, false);
        self.buy_door(&mut lay, Vec2::new(-6.0, -10.0), true, 2.4, (0, 4), 1250, false);
        self.buy_door(&mut lay, Vec2::new(-25.0, 10.0), true, 2.4, (1, 5), 1000, true);
        self.buy_door(&mut lay, Vec2::new(-14.0, 20.0), false, 2.4, (5, 2), 1000, false);

        // Roofs: the greenhouse is a lattice of glass, the rest flat.
        self.decor(Aabb::span(Vec3::new(-36.2, g + 4.2, 9.8), Vec3::new(-13.8, g + 4.45, 30.2)), Paint::Dark);
        self.decor(Aabb::span(Vec3::new(9.8, g + 3.8, -12.2), Vec3::new(34.2, g + 4.05, 10.2)), Paint::Dark);
        self.decor(Aabb::span(Vec3::new(-26.2, g + 5.0, -32.2), Vec3::new(-1.8, g + 5.25, -9.8)), Paint::Iron);
        let mut x = -14.0;
        while x <= 14.0 {
            self.decor(Aabb::span(Vec3::new(x - 0.08, g + 4.1, 10.0), Vec3::new(x + 0.08, g + 4.3, 30.0)), Paint::Dark);
            x += 3.5;
        }
        let mut z = 10.0;
        while z <= 30.0 {
            self.decor(Aabb::span(Vec3::new(-14.0, g + 4.1, z - 0.08), Vec3::new(14.0, g + 4.3, z + 0.08)), Paint::Dark);
            z += 4.0;
        }

        self.fen_pad(&mut lay, rng);
        self.fen_boardwalk(&mut lay, rng);
        self.fen_greenhouse(&mut lay, rng);
        self.fen_lab(&mut lay, rng);
        self.fen_pump(&mut lay, rng);
        self.fen_cold(&mut lay, rng);
        self.fen_outside(rng);

        self.spawn = Vec2::new(0.0, 0.0);
        self.waypoints_over(g, (-9.0, 9.0, -9.0, 9.0), 5.0);
        self.finish_horde(lay);
    }

    fn fen_pad(&mut self, lay: &mut HordeLayout, rng: &mut Rng) {
        let g = lay.ground;
        // Helipad markings and edge lights.
        self.flat(-6.0, 6.0, -6.0, 6.0, g + 0.006, Paint::Steel);
        self.flat(-2.6, -1.8, -3.0, 3.0, g + 0.01, Paint::Gold);
        self.flat(1.8, 2.6, -3.0, 3.0, g + 0.01, Paint::Gold);
        self.flat(-1.8, 1.8, -0.4, 0.4, g + 0.01, Paint::Gold);
        for k in 0..8 {
            let a = k as f32 * TAU / 8.0;
            let p = Vec2::from_angle(a) * 6.4;
            self.decor(Aabb::new(Vec3::new(p.x, g + 0.08, p.y), Vec3::new(0.12, 0.08, 0.12)), if k % 2 == 0 { Paint::Green } else { Paint::Blue });
        }
        self.crates(Vec2::new(-7.5, 7.5), true);
        self.crates(Vec2::new(2.4, -6.2), false);
        self.crates(Vec2::new(-7.0, -2.6), false);
        self.lamp(Vec2::new(9.0, 9.0));
        self.lamp(Vec2::new(-9.0, -9.0));
        Self::wall_buy(lay, Vec2::new(-5.0, 9.8), Vec2::NEG_Y, gun::DMR, 500, 0);
        Self::wall_buy(lay, Vec2::new(-9.8, -5.5), Vec2::X, gun::MACHINE_PISTOL, 600, 0);
        let m = self.machine(lay, Vec2::new(9.8, 7.0), Vec2::NEG_X, 0, perk_id::PATCH_UP, 2.2);
        lay.perks.push(m);
        let _ = rng;
    }

    fn fen_boardwalk(&mut self, lay: &mut HordeLayout, rng: &mut Rng) {
        let g = lay.ground;
        // Planks down the middle, black water and reeds either side.
        self.flat(-36.0, -10.0, -1.4, 4.4, g + 0.03, Paint::Wood);
        let mut x = -35.5;
        while x < -10.0 {
            self.decor(Aabb::span(Vec3::new(x, g + 0.03, -1.4), Vec3::new(x + 0.06, g + 0.05, 4.4)), Paint::Dark);
            x += 0.9;
        }
        for (x0, x1, z0, z1) in [(-34.0, -24.0, -6.6, -2.0), (-21.0, -12.0, -6.6, -2.2), (-34.0, -27.0, 5.4, 9.6), (-22.0, -13.0, 5.8, 9.6)] {
            self.flat(x0, x1, z0, z1, g + 0.008, Paint::Shell);
            for _ in 0..4 {
                let p = Vec2::new(rng.range(x0, x1), rng.range(z0, z1));
                self.props.push(Prop { kind: PropKind::Bush, pos: Vec3::new(p.x, g, p.y), yaw: rng.range(0.0, TAU), scale: rng.range(0.8, 1.3) });
            }
        }
        // Rails along the walk (thin, walk-through), and the mud hole husks crawl out of.
        for z in [-1.4, 4.4] {
            self.decor(Aabb::span(Vec3::new(-36.0, g + 0.95, z - 0.04), Vec3::new(-10.0, g + 1.0, z + 0.04)), Paint::Wood);
        }
        self.crates(Vec2::new(-16.0, -5.5), true);
        for p in [Vec2::new(-30.0, 1.5), Vec2::new(-18.0, 1.5)] {
            self.lamp(p + Vec2::new(0.0, 3.2));
        }
        Self::wall_buy(lay, Vec2::new(-22.0, 9.8), Vec2::NEG_Y, gun::SMG, 1000, 1);
        Self::wall_buy(lay, Vec2::new(-35.8, -4.0), Vec2::X, gun::BREACHER, 1500, 1);
        let m = self.machine(lay, Vec2::new(-12.0, -6.8), Vec2::Y, 1, perk_id::QUICKFIX, 2.2);
        lay.perks.push(m);
    }

    fn fen_greenhouse(&mut self, lay: &mut HordeLayout, rng: &mut Rng) {
        let g = lay.ground;
        // Raised beds of glowing growth, with aisles between.
        for x in [-10.0, -4.0, 4.0, 10.0] {
            for (z0, z1) in [(13.0, 19.0), (22.0, 27.5)] {
                self.solid(Aabb::span(Vec3::new(x - 1.0, g, z0), Vec3::new(x + 1.0, g + 0.8, z1)), Paint::Wood);
                self.decor(Aabb::span(Vec3::new(x - 0.9, g + 0.8, z0 + 0.1), Vec3::new(x + 0.9, g + 0.83, z1 - 0.1)), Paint::Leaf);
                for k in 0..3 {
                    let z = z0 + 1.0 + k as f32 * ((z1 - z0 - 2.0) / 2.0);
                    self.props.push(Prop { kind: PropKind::Bush, pos: Vec3::new(x, g + 0.8, z), yaw: rng.range(0.0, TAU), scale: 0.7 });
                    self.decor(Aabb::new(Vec3::new(x + 0.6, g + 1.0, z), Vec3::new(0.05, 0.2, 0.05)), Paint::Green);
                }
            }
        }
        for x in [-7.0, 0.0, 7.0] {
            self.hanging_light(Vec2::new(x, 20.5), g + 4.2, Paint::Violet);
        }
        Self::wall_buy(lay, Vec2::new(13.8, 15.0), Vec2::NEG_X, gun::CARBINE, 1400, 2);
        let m = self.machine(lay, Vec2::new(-13.8, 27.5), Vec2::X, 2, perk_id::HAIR_TRIGGER, 2.2);
        lay.perks.push(m);
        let c = self.machine(lay, Vec2::new(0.0, 29.8), Vec2::NEG_Y, 2, 0, 0.0);
        lay.crates.push(c);
    }

    fn fen_lab(&mut self, lay: &mut HordeLayout, rng: &mut Rng) {
        let g = lay.ground;
        // Specimen tanks: tall glowing cylinders in a row, and benches.
        for (x, z) in [(17.0, 5.5), (21.5, 5.5), (26.0, 5.5), (30.5, 5.5)] {
            self.solid(Aabb::span(Vec3::new(x - 0.9, g, z - 0.9), Vec3::new(x + 0.9, g + 0.4, z + 0.9)), Paint::Dark);
            self.column(Vec2::new(x, z), 0.75, g + 0.4, g + 2.8, Paint::Aether);
            self.decor(Aabb::new(Vec3::new(x, g + 1.5, z - 0.02), Vec3::new(0.25, 0.55, 0.25)), Paint::Dark);
            self.decor(Aabb::span(Vec3::new(x - 0.85, g + 2.8, z - 0.85), Vec3::new(x + 0.85, g + 3.1, z + 0.85)), Paint::Steel);
        }
        for (x, z) in [(17.0, -4.0), (24.0, -4.0)] {
            self.solid(Aabb::span(Vec3::new(x - 2.2, g, z - 0.5), Vec3::new(x + 2.2, g + 0.95, z + 0.5)), Paint::Bone);
            self.decor(Aabb::span(Vec3::new(x - 1.5, g + 0.95, z - 0.3), Vec3::new(x - 0.9, g + 1.3, z + 0.1)), Paint::Screen);
            self.decor(Aabb::span(Vec3::new(x + 0.5, g + 0.95, z - 0.2), Vec3::new(x + 0.7, g + 1.25, z)), Paint::Green);
        }
        self.solid(Aabb::span(Vec3::new(28.5, g, -8.0), Vec3::new(31.0, g + 1.0, -5.5)), Paint::Steel);
        self.decor(Aabb::span(Vec3::new(28.7, g + 1.0, -7.8), Vec3::new(30.8, g + 1.04, -5.7)), Paint::White);
        for x in [16.0, 24.0, 31.0] {
            self.hanging_light(Vec2::new(x, 0.5), g + 3.8, Paint::Aether);
        }
        Self::wall_buy(lay, Vec2::new(10.2, -7.0), Vec2::X, gun::PDW, 1100, 3);
        Self::wall_buy(lay, Vec2::new(10.2, 6.0), Vec2::X, gun::BULLPUP, 1400, 3);
        let m = self.machine(lay, Vec2::new(33.8, 0.0), Vec2::NEG_X, 3, perk_id::IRON_HIDE, 2.2);
        lay.perks.push(m);
        let f = self.machine(lay, Vec2::new(15.5, -11.8), Vec2::Y, 3, 0, 1.6);
        lay.forge = Some(f);
        let c = self.machine(lay, Vec2::new(20.0, 9.8), Vec2::NEG_Y, 3, 0, 0.0);
        lay.crates.insert(0, c);
        let _ = rng;
    }

    fn fen_pump(&mut self, lay: &mut HordeLayout, rng: &mut Rng) {
        let g = lay.ground;
        // Pumps on plinths, pipes along the walls.
        for (x, z) in [(-20.0, -26.0), (-8.0, -18.0)] {
            self.solid(Aabb::span(Vec3::new(x - 1.6, g, z - 1.2), Vec3::new(x + 1.6, g + 1.6, z + 1.2)), Paint::Teal);
            self.column(Vec2::new(x, z), 0.5, g + 1.6, g + 2.6, Paint::Iron);
            self.decor(Aabb::span(Vec3::new(x - 1.62, g + 1.0, z - 0.4), Vec3::new(x - 1.6, g + 1.3, z + 0.4)), Paint::Green);
        }
        for z in [-31.2, -10.8] {
            self.decor(Aabb::span(Vec3::new(-25.6, g + 3.6, z - 0.18), Vec3::new(-2.4, g + 3.95, z + 0.18)), Paint::Copper);
        }
        self.crates(Vec2::new(-22.5, -12.2), true);
        self.crates(Vec2::new(-4.5, -29.5), false);
        for (x, z) in [(-18.0, -15.0), (-18.0, -27.0), (-7.0, -27.0)] {
            self.hanging_light(Vec2::new(x, z), g + 5.0, Paint::Glow);
        }
        Self::wall_buy(lay, Vec2::new(-25.8, -15.5), Vec2::X, gun::BATTLE_RIFLE, 1600, 4);
        let m = self.machine(lay, Vec2::new(-2.2, -14.5), Vec2::NEG_X, 4, perk_id::SPARE_SOCKET, 2.2);
        lay.perks.push(m);
        let p = self.machine(lay, Vec2::new(-14.0, -31.8), Vec2::Y, 4, 0, 0.0);
        lay.power = Some(p);
        let c = self.machine(lay, Vec2::new(-25.8, -28.0), Vec2::X, 4, 0, 0.0);
        lay.crates.push(c);
        let _ = rng;
    }

    fn fen_cold(&mut self, lay: &mut HordeLayout, rng: &mut Rng) {
        let g = lay.ground;
        // Freezer racks and frosted panels.
        for z in [15.0, 21.0, 26.5] {
            self.solid(Aabb::span(Vec3::new(-31.0, g, z - 0.5), Vec3::new(-22.0, g + 2.2, z + 0.5)), Paint::Bone);
            self.decor(Aabb::span(Vec3::new(-31.05, g + 1.6, z - 0.52), Vec3::new(-21.95, g + 1.66, z + 0.52)), Paint::Blue);
        }
        for x in [-35.7, -14.3] {
            self.decor(Aabb::span(Vec3::new(x - 0.05, g + 0.4, 12.0), Vec3::new(x + 0.05, g + 3.6, 28.0)), Paint::White);
        }
        for z in [13.0, 24.0] {
            self.hanging_light(Vec2::new(-25.0, z), g + 4.2, Paint::Blue);
        }
        Self::wall_buy(lay, Vec2::new(-30.0, 10.2), Vec2::Y, gun::SHOTGUN, 1200, 5);
        let m = self.machine(lay, Vec2::new(-35.8, 13.0), Vec2::X, 5, perk_id::FLEETFOOT, 2.2);
        lay.perks.push(m);
        let m = self.machine(lay, Vec2::new(-14.2, 25.5), Vec2::NEG_X, 5, perk_id::SCRAP_MAGNET, 2.2);
        lay.perks.push(m);
        let c = self.machine(lay, Vec2::new(-18.0, 29.8), Vec2::NEG_Y, 5, 0, 0.0);
        lay.crates.push(c);
        let _ = rng;
    }

    /// The fen outside: dead trees, reeds, a radio mast, a sunk boat.
    fn fen_outside(&mut self, rng: &mut Rng) {
        for _ in 0..90 {
            let p = Vec2::new(rng.range(-66.0, 64.0), rng.range(-66.0, 64.0));
            if p.x > -44.0 && p.x < 42.0 && p.y > -40.0 && p.y < 38.0 {
                continue;
            }
            let h = self.height(p.x, p.y);
            let kind = if h > self.water + 0.2 && rng.chance(0.45) { PropKind::Tree } else { PropKind::Bush };
            let scale = if kind == PropKind::Tree { rng.range(3.4, 5.6) } else { rng.range(0.9, 1.6) };
            self.props.push(Prop { kind, pos: Vec3::new(p.x, h.max(self.water - 0.2), p.y), yaw: rng.range(0.0, TAU), scale });
        }
        let mast = Vec2::new(46.0, 40.0);
        let h = self.height(mast.x, mast.y).max(self.water);
        self.column(mast, 0.3, h, h + 24.0, Paint::Steel);
        self.decor(Aabb::new(Vec3::new(mast.x, h + 24.2, mast.y), Vec3::new(0.25, 0.25, 0.25)), Paint::Red);
        for (x, z) in [(-52.0, -30.0), (40.0, -46.0)] {
            let h = self.height(x, z).max(self.water - 0.3);
            self.decor(Aabb::span(Vec3::new(x - 1.4, h - 0.6, z - 4.0), Vec3::new(x + 1.4, h + 0.7, z + 4.0)), Paint::Rust);
        }
    }

    /// Done: no swinging doors or glass on these maps (the horde mode has its own), and the
    /// layout recorded.
    fn finish_horde(&mut self, mut lay: HordeLayout) {
        self.doors.clear();
        self.windows.clear();
        self.team_spawns = [Vec::new(), Vec::new()];
        self.place_window_spawns(&mut lay);
        self.horde = Some(Box::new(lay));
    }

    /// Where the husks for each window turn up: straight out from it, as far as the ground
    /// stays clear and outside every zone (about `SPAWN_OUT`, further down a narrow alley), so they always walk in to the
    /// boards in the open and never appear inside the walls.
    fn place_window_spawns(&self, lay: &mut HordeLayout) {
        const SPAWN_OUT: f32 = 7.5;
        const SPAWN_FAR: f32 = 18.0;
        let mut statics = crate::collide::StaticWorld::new(self.recipe.half_extent);
        for b in self.solids() {
            statics.add(b);
        }
        let zones = lay.zones.clone();
        let inside_any = |p: Vec2| zones.iter().any(|z| {
            let (x0, x1, z0, z1) = z.rect;
            p.x > x0 - 0.7 && p.x < x1 + 0.7 && p.y > z0 - 0.7 && p.y < z1 + 0.7
        });
        let clear = |p: Vec2| {
            let y = self.height(p.x, p.y).max(lay.ground);
            !statics.blocked(p.x, p.y, y + 0.25, 1.4, 0.5) && self.in_bounds(p.x, p.y) && self.in_bounds(p.x + 3.0, p.y + 3.0) && self.in_bounds(p.x - 3.0, p.y - 3.0)
        };
        for w in &mut lay.windows {
            let start = Vec2::new(w.outside.x, w.outside.z);
            let side = Vec2::new(-w.out.y, w.out.x);
            // The lane from just outside the boards: the first step stays next to the wall (the
            // boards' own zone check would see the building itself), the rest must be open ground.
            let lane_ok = |d: f32, lateral: f32| {
                let p = start + w.out * d + side * lateral;
                clear(p) && (d < 1.0 || !inside_any(p))
            };
            let mut most = 0.0;
            let mut d = 0.5;
            while d <= SPAWN_FAR && lane_ok(d, 0.0) {
                most = d;
                d += 0.5;
            }
            // Out in the open if a narrow alley leads up to the window: from the far end of it.
            let wide = |d: f32| lane_ok(d, 1.6) && lane_ok(d, -1.6);
            let mut reach = most.min(SPAWN_OUT);
            if !wide(reach) {
                let mut d = reach;
                while d <= most {
                    if wide(d) {
                        reach = d + 1.0f32.min(most - d);
                        break;
                    }
                    d += 0.5;
                }
            }
            // How wide the lane is all along (for spreading husks out a little).
            let mut spread = 0.0;
            for lateral in [0.4, 0.8, 1.2] {
                let ok = (1..=((reach / 0.5) as usize)).all(|k| lane_ok(k as f32 * 0.5, lateral) && lane_ok(k as f32 * 0.5, -lateral));
                if !ok {
                    break;
                }
                spread = lateral;
            }
            let p = start + w.out * reach;
            w.spawn = Vec3::new(p.x, self.height(p.x, p.y).max(lay.ground), p.y);
            w.spread = spread;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Biome, Weather, WorldRecipe};
    use super::*;
    use crate::collide::StaticWorld;
    use glam::Vec3Swizzles;

    fn statics(w: &World) -> StaticWorld {
        let mut s = StaticWorld::new(w.recipe.half_extent);
        for b in w.solids() {
            s.add(b);
        }
        s
    }

    #[test]
    fn horde_maps_are_complete_and_clear() {
        for layout in Layout::HORDE {
            let w = World::generate(WorldRecipe::preset(layout, Biome::IronShoals, Weather::Clear));
            let what = format!("{layout:?}");
            let lay = w.horde.as_ref().expect("a horde map has a layout");
            // Budgets: a few thousand blocks at most.
            assert!(w.blocks.len() > 300 && w.blocks.len() < 4000, "{what}: {} blocks", w.blocks.len());
            assert!(lay.zones.len() >= 5 && lay.doors.len() >= 5, "{what}");
            assert!(lay.windows.len() >= 12, "{what}: {} windows", lay.windows.len());
            assert!(lay.buys.len() >= 8 && lay.perks.len() == 7 && lay.crates.len() >= 3, "{what}");
            assert!(lay.forge.is_some() && lay.power.is_some(), "{what}");
            assert!(w.doors.is_empty() && w.windows.is_empty(), "{what}: no swinging doors or glass");
            let s = statics(&w);
            let clear = |p: Vec3, why: &str| {
                assert!(w.in_bounds(p.x, p.z), "{what}: {why} {p:?} out of bounds");
                assert!(!s.blocked(p.x, p.z, p.y, 1.7, 0.3), "{what}: {why} {p:?} is inside a wall");
            };
            clear(Vec3::new(w.spawn.x, lay.ground, w.spawn.y), "spawn");
            for (i, win) in lay.windows.iter().enumerate() {
                clear(win.inside, &format!("window {i} inside"));
                clear(win.outside, &format!("window {i} outside"));
                let out = Vec3::new(win.out.x, 0.0, win.out.y);
                clear(win.outside + out * 4.0, &format!("window {i} approach"));
                // Outside really is outside: no zone holds it.
                assert!(lay.zone_at(win.outside.xz() + win.out * 2.0).is_none(), "{what}: window {i} opens into a zone");
                assert_eq!(lay.zone_at(win.inside.xz()), Some(win.zone), "{what}: window {i} is in the wrong zone");
                // Husks turn up well out in the open and walk straight in.
                let reach = win.spawn.xz().distance(win.outside.xz());
                assert!(reach >= 3.0, "{what}: window {i} has only {reach:.1} m of open ground outside");
                for k in 0..=20 {
                    let p = win.outside.lerp(win.spawn, k as f32 / 20.0);
                    clear(Vec3::new(p.x, w.height(p.x, p.z).max(lay.ground), p.z), &format!("window {i} lane"));
                    if k > 2 {
                        assert!(lay.zone_at(p.xz()).is_none(), "{what}: window {i}'s lane runs through a zone at {p:?}");
                    }
                }
            }
            for (i, m) in lay.perks.iter().chain(&lay.crates).chain(lay.forge.iter()).chain(lay.power.iter()).enumerate() {
                clear(m.pos, &format!("machine {i}"));
                assert_eq!(lay.zone_at(m.pos.xz()), Some(m.zone), "{what}: machine {i} at {:?}", m.pos);
            }
            for (i, buy) in lay.buys.iter().enumerate() {
                let stand = buy.pos + Vec3::new(buy.normal.x, 0.0, buy.normal.y) * 0.9;
                clear(Vec3::new(stand.x, lay.ground, stand.z), &format!("wall buy {i}"));
                // The outline sits on a wall face.
                let behind = buy.pos - Vec3::new(buy.normal.x, 0.0, buy.normal.y) * 0.3;
                assert!(s.blocked(behind.x, behind.z, lay.ground, 1.7, 0.05), "{what}: wall buy {i} is not on a wall");
                assert_eq!(lay.zone_at(stand.xz()), Some(buy.zone), "{what}: wall buy {i}");
                // The whole chalk frame is on solid wall (no window, door or corner under it).
                let along = Vec3::new(-buy.normal.y, 0.0, buy.normal.x);
                for (a, b) in [(-0.5, -0.5), (-0.5, 0.5), (0.5, -0.5), (0.5, 0.5), (0.0, 0.0)] {
                    let p = buy.pos + along * (a * CHALK_FRAME.x) + Vec3::Y * (b * CHALK_FRAME.y) - Vec3::new(buy.normal.x, 0.0, buy.normal.y) * 0.2;
                    assert!(s.blocked(p.x, p.z, p.y - 0.05, 0.1, 0.02), "{what}: wall buy {i}'s frame hangs off the wall at {p:?}");
                }
            }
            for (i, door) in lay.doors.iter().enumerate() {
                let c = door.aabb.center;
                // A doorway: nothing solid in it.
                assert!(!s.blocked(c.x, c.z, lay.ground, 1.7, 0.3), "{what}: door {i} is walled up");
                assert!(door.price >= 500);
            }
        }
    }
}
