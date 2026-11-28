//! The two fixed maps of the horde mode (round-based survival against scrap husks).
//!
//! * **Cinder Works** - an abandoned foundry compound at night. You start in a walled
//!   courtyard; doors and rubble open onto a racked warehouse, a block of offices with the
//!   generator room, a furnace hall still glowing with slag and an open loading dock; further
//!   out a narrow cable tunnel runs the length of the west side, a long rolling mill crosses
//!   the north, a fenced scrap yard and a rail siding of boxcars ring the east and south, and
//!   last of all the casting floor, with its catwalk and the reforge up on it.
//! * **Blackfen Station** - a swamp research station in thick fog. You start on the landing
//!   pad; round it sit a reed-choked boardwalk, a greenhouse lab, a specimen lab of glowing
//!   tanks, a pump house and a cold store; beyond them a pipe tunnel, a long glass gallery,
//!   a hangar, the reed yard and the observatory, whose upper deck holds the reforge.
//!
//! Both are rings of zones, so there is always a way round: a squad can lead husks in a loop
//! instead of backing into a dead end.
//!
//! Besides the geometry, each map fills in a [`HordeLayout`]: the zones and the doors
//! between them, barricaded windows (the only way husks get in) and where husks gather outside them, chalk
//! outlines of weapons for sale, perk machines, the salvage crate's spots, the reforge, the
//! generator, arc-grate traps and relay beacons. The simulation and the client both read it; nothing here is replicated.

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

/// An arc grate: a floor of live bars husks fry on while it runs, switched on (for points,
/// once the power is on) at a panel on the wall beside it, then cooling down for a while.
#[derive(Clone, Copy, Debug)]
pub struct HordeTrap {
    /// The live floor, (x0, x1, z0, z1).
    pub floor: (f32, f32, f32, f32),
    /// The switch panel.
    pub panel: HordeMachine,
    pub zone: u8,
}

impl HordeTrap {
    pub fn covers(&self, p: Vec2) -> bool {
        let (x0, x1, z0, z1) = self.floor;
        p.x >= x0 && p.x <= x1 && p.y >= z0 && p.y <= z1
    }
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
    /// The generator: crank it (hold Use) to bring the power back.
    pub power: Option<HordeMachine>,
    pub traps: Vec<HordeTrap>,
    /// Relay beacons: switch every one on (a side job) for points and a power-up.
    pub relays: Vec<HordeMachine>,
    /// Ground height of the play area.
    pub ground: f32,
}

impl HordeLayout {
    /// The zone a point lies in, if any.
    pub fn zone_at(&self, p: Vec2) -> Option<u8> {
        self.zones.iter().position(|z| z.contains(p)).map(|i| i as u8)
    }

    /// Every machine on the map: perks, crate spots, the reforge, the generator, trap panels
    /// and relays.
    pub fn machines(&self) -> impl Iterator<Item = &HordeMachine> {
        self.perks.iter().chain(&self.crates).chain(self.forge.iter()).chain(self.power.iter()).chain(self.traps.iter().map(|t| &t.panel)).chain(&self.relays)
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
                let d = (x - 7.0).abs() - 51.0;
                let e = (z + 4.0).abs() - 48.0;
                let out = d.max(e).max(0.0);
                let pools = noise::fbm(seed ^ 0xfe7, x * 0.06, z * 0.06, 3);
                let sink = (out / 8.0).clamp(0.0, 1.0);
                FEN_GROUND - sink * (1.0 + 1.6 * (pools + 0.3).max(0.0)) + 0.25 * sink * noise::value(seed ^ 0x51, x * 0.2, z * 0.2)
            }
            _ => {
                let out = ((x - 17.0).abs() - 46.0).max((z - 12.0).abs() - 63.0).max(0.0);
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

    /// A machine standing on a floor at height `floor` (an upper deck), else as [`Self::machine`].
    #[allow(clippy::too_many_arguments)]
    fn machine_on(&mut self, lay: &mut HordeLayout, back: Vec2, facing: Vec2, zone: u8, kind: u8, height: f32, floor: f32) -> HordeMachine {
        let ground = lay.ground;
        lay.ground = floor;
        let m = self.machine(lay, back, facing, zone, kind, height);
        lay.ground = ground;
        m
    }

    /// An arc grate on the floor (`floor` is x0, x1, z0, z1) and its switch panel on the wall
    /// face at `back`.
    fn horde_trap(&mut self, lay: &mut HordeLayout, floor: (f32, f32, f32, f32), back: Vec2, facing: Vec2, zone: u8) {
        let g = lay.ground;
        let (x0, x1, z0, z1) = floor;
        self.flat(x0, x1, z0, z1, g + 0.012, Paint::Dark);
        // Hazard kerbs round the edge (flush with the floor: nothing to trip on).
        for (a, b) in [((x0, z0 - 0.12), (x1, z0)), ((x0, z1), (x1, z1 + 0.12)), ((x0 - 0.12, z0), (x0, z1)), ((x1, z0), (x1 + 0.12, z1))] {
            self.flat(a.0, b.0, a.1, b.1, g + 0.014, Paint::Gold);
        }
        let panel = self.machine(lay, back, facing, zone, 0, 0.0);
        lay.traps.push(HordeTrap { floor, panel, zone });
    }

    /// A relay beacon: a squat transmitter against the wall at `back`, on a floor at `floor`.
    fn relay(&mut self, lay: &mut HordeLayout, back: Vec2, facing: Vec2, zone: u8, floor: f32) {
        let m = self.machine_on(lay, back, facing, zone, 0, 1.2, floor);
        lay.relays.push(m);
    }

    /// A straight flight of stairs up to a deck at `top`, starting at the deck's edge `edge`
    /// (a Z coordinate) and running down along `dir` (+1 or -1 in Z), `x0..x1` wide.
    fn stairs_z(&mut self, x0: f32, x1: f32, edge: f32, dir: f32, base: f32, top: f32) {
        const RUN: f32 = 0.45;
        let steps = ((top - base) / 0.32).round().max(2.0) as usize;
        let rise = (top - base) / steps as f32;
        for i in 1..steps {
            let (a, b) = (edge + dir * (i - 1) as f32 * RUN, edge + dir * i as f32 * RUN);
            let h = top - i as f32 * rise;
            self.solid(Aabb::span(Vec3::new(x0, base, a.min(b)), Vec3::new(x1, h, a.max(b))), if i % 2 == 0 { Paint::Steel } else { Paint::Iron });
        }
        // Stringers either side.
        let far = edge + dir * (steps - 1) as f32 * RUN;
        for x in [x0 - 0.06, x1] {
            self.decor(Aabb::span(Vec3::new(x, base, edge.min(far)), Vec3::new(x + 0.06, top - rise * 0.5, edge.max(far))), Paint::Dark);
        }
    }

    /// An upper deck (x0..x1, z0..z1) with its top at `top`, a rail along the edge facing
    /// `rail_z` between `rail` x-limits, and posts under that edge.
    #[allow(clippy::too_many_arguments)]
    fn deck(&mut self, x0: f32, x1: f32, z0: f32, z1: f32, base: f32, top: f32, rail_z: f32, rail: (f32, f32), posts: &[f32]) {
        self.solid(Aabb::span(Vec3::new(x0, top - 0.25, z0), Vec3::new(x1, top, z1)), Paint::Iron);
        self.decor(Aabb::span(Vec3::new(x0, top - 0.3, z0 - 0.02), Vec3::new(x1, top - 0.25, z1 + 0.02)), Paint::Dark);
        let inward = if rail_z <= z0 + 0.01 { 1.0 } else { -1.0 };
        for x in posts {
            let z = rail_z + inward * 0.25;
            self.solid(Aabb::span(Vec3::new(x - 0.18, base, z - 0.18), Vec3::new(x + 0.18, top - 0.25, z + 0.18)), Paint::Dark);
        }
        // The rail: a thin solid bar at waist height (and its posts as decor).
        self.solid(Aabb::span(Vec3::new(rail.0, top + 0.9, rail_z - 0.04), Vec3::new(rail.1, top + 1.0, rail_z + 0.04)), Paint::Gold);
        self.solid(Aabb::span(Vec3::new(rail.0, top, rail_z - 0.03), Vec3::new(rail.1, top + 0.12, rail_z + 0.03)), Paint::Dark);
        let mut x = rail.0;
        while x <= rail.1 + 0.01 {
            self.decor(Aabb::span(Vec3::new(x - 0.03, top, rail_z - 0.03), Vec3::new(x + 0.03, top + 0.9, rail_z + 0.03)), Paint::Dark);
            x += 2.0;
        }
        // Lamps under the deck.
        let mut x = x0 + 3.0;
        while x < x1 - 1.0 {
            self.decor(Aabb::span(Vec3::new(x - 0.3, top - 0.36, (z0 + z1) * 0.5 - 0.1), Vec3::new(x + 0.3, top - 0.3, (z0 + z1) * 0.5 + 0.1)), Paint::Glow);
            x += 8.0;
        }
    }

    /// Cinder Works: the foundry compound.
    pub(super) fn cinder_works(&mut self, rng: &mut Rng) {
        let g = FOUNDRY_GROUND;
        let mut lay = HordeLayout { name: "CINDER WORKS", ground: g, ..Default::default() };
        use Cut::{Door, Window};
        let v = Vec2::new;
        lay.zones = vec![
            HordeZone { name: "COURTYARD", rect: (-12.0, 12.0, -12.0, 10.0) },
            HordeZone { name: "WAREHOUSE", rect: (-22.0, 14.0, 10.0, 32.0) },
            HordeZone { name: "OFFICES", rect: (12.0, 32.0, -12.0, 8.0) },
            HordeZone { name: "FURNACE HALL", rect: (14.0, 44.0, 8.0, 32.0) },
            HordeZone { name: "LOADING DOCK", rect: (-22.0, 12.0, -32.0, -12.0) },
            HordeZone { name: "ROLLING MILL", rect: (-26.0, 60.0, 32.0, 44.0) },
            HordeZone { name: "SCRAP YARD", rect: (44.0, 60.0, -32.0, 32.0) },
            HordeZone { name: "CABLE TUNNEL", rect: (-26.0, -22.0, -32.0, 32.0) },
            HordeZone { name: "RAIL SIDING", rect: (-26.0, 60.0, -48.0, -32.0) },
            HordeZone { name: "CASTING FLOOR", rect: (14.0, 60.0, 44.0, 72.0) },
        ];
        let brick = Paint::Rust;
        let sheet = Paint::Steel;
        let fence = Paint::Iron;
        // Floors: cracked concrete outside, boards and plate indoors.
        let floors = [Paint::Rock, Paint::Concrete, Paint::Wood, Paint::Iron, Paint::Rock, Paint::Concrete, Paint::Sand, Paint::Steel, Paint::Rock, Paint::Iron];
        for (zone, paint) in lay.zones.clone().iter().zip(floors) {
            let (x0, x1, z0, z1) = zone.rect;
            self.flat(x0, x1, z0, z1, g + 0.002, paint);
        }

        // The old core: courtyard, warehouse, offices, furnace hall, loading dock.
        self.zline(&mut lay, v(-22.0, 10.0), v(14.0, 10.0), 6.5, &[Door(0.0, 2.4), Window(-19.0, 1, -1.0), Window(-15.0, 1, -1.0)], brick);
        self.zline(&mut lay, v(12.0, -12.0), v(12.0, 10.0), 4.5, &[Door(-2.0, 2.4)], brick);
        self.zline(&mut lay, v(14.0, 8.0), v(14.0, 32.0), 9.0, &[Door(21.0, 2.4)], sheet);
        self.zline(&mut lay, v(12.0, 8.0), v(44.0, 8.0), 9.0, &[Door(19.0, 2.4), Window(36.0, 3, -1.0), Window(41.0, 3, -1.0)], brick);
        self.zline(&mut lay, v(-22.0, -12.0), v(32.0, -12.0), 4.5, &[Door(0.0, 2.4), Window(-19.0, 4, 1.0), Window(-15.0, 4, 1.0), Window(22.0, 2, -1.0)], brick);
        self.zline(&mut lay, v(-12.0, -12.0), v(-12.0, 10.0), 4.5, &[Window(-7.0, 0, -1.0), Window(-1.0, 0, -1.0), Window(5.0, 0, -1.0)], brick);
        self.zline(&mut lay, v(32.0, -12.0), v(32.0, 8.0), 4.0, &[Window(-2.0, 2, 1.0)], brick);
        self.zline(&mut lay, v(12.0, -32.0), v(12.0, -12.0), 4.5, &[Window(-26.0, 4, 1.0)], brick);
        // The sealed nook where four buildings meet.
        self.solid(Aabb::span(Vec3::new(12.0, g, 8.0), Vec3::new(14.0, g + 6.5, 10.0)), brick);
        // The cable tunnel down the west side.
        self.zline(&mut lay, v(-22.0, -32.0), v(-22.0, -12.0), 4.5, &[Door(-22.0, 2.4)], brick);
        self.zline(&mut lay, v(-22.0, -12.0), v(-22.0, 10.0), 4.5, &[], Paint::Concrete);
        self.zline(&mut lay, v(-22.0, 10.0), v(-22.0, 32.0), 6.5, &[Door(21.0, 2.4)], sheet);
        self.zline(&mut lay, v(-26.0, -48.0), v(-26.0, -32.0), 4.0, &[Window(-40.0, 8, -1.0)], fence);
        self.zline(&mut lay, v(-26.0, -32.0), v(-26.0, 32.0), 4.0, &[Window(-24.0, 7, -1.0), Window(-8.0, 7, -1.0), Window(8.0, 7, -1.0), Window(24.0, 7, -1.0)], Paint::Concrete);
        self.zline(&mut lay, v(-26.0, 32.0), v(-26.0, 44.0), 7.5, &[Window(38.0, 5, -1.0)], sheet);
        // The rolling mill along the north.
        self.zline(&mut lay, v(-26.0, 32.0), v(-22.0, 32.0), 7.5, &[Door(-24.0, 2.4)], sheet);
        self.zline(&mut lay, v(-22.0, 32.0), v(14.0, 32.0), 7.5, &[Door(-4.0, 2.4)], sheet);
        self.zline(&mut lay, v(14.0, 32.0), v(44.0, 32.0), 9.0, &[Door(24.0, 2.4)], brick);
        self.zline(&mut lay, v(44.0, 32.0), v(60.0, 32.0), 7.5, &[Door(52.0, 2.4)], sheet);
        self.zline(&mut lay, v(-26.0, 44.0), v(14.0, 44.0), 7.5, &[Window(-20.0, 5, 1.0), Window(-8.0, 5, 1.0), Window(4.0, 5, 1.0)], sheet);
        self.zline(&mut lay, v(14.0, 44.0), v(60.0, 44.0), 10.0, &[Door(36.0, 2.4), Door(54.0, 2.4)], brick);
        // The east edge: siding, scrap yard, mill end, casting floor.
        self.zline(&mut lay, v(60.0, -48.0), v(60.0, -32.0), 4.0, &[Window(-40.0, 8, 1.0)], fence);
        self.zline(&mut lay, v(60.0, -32.0), v(60.0, 32.0), 4.0, &[Window(-20.0, 6, 1.0), Window(0.0, 6, 1.0), Window(20.0, 6, 1.0)], fence);
        self.zline(&mut lay, v(60.0, 32.0), v(60.0, 44.0), 7.5, &[Window(38.0, 5, 1.0)], sheet);
        self.zline(&mut lay, v(60.0, 44.0), v(60.0, 72.0), 10.0, &[Window(58.0, 9, 1.0)], brick);
        // The scrap yard's west fence, and the furnace hall's east wall.
        self.zline(&mut lay, v(44.0, -32.0), v(44.0, 8.0), 4.0, &[Window(-20.0, 6, -1.0), Window(-4.0, 6, -1.0)], fence);
        self.zline(&mut lay, v(44.0, 8.0), v(44.0, 32.0), 9.0, &[Door(26.0, 2.4)], brick);
        // The rail siding along the south.
        self.zline(&mut lay, v(-26.0, -32.0), v(-22.0, -32.0), 4.0, &[Door(-24.0, 2.4)], fence);
        self.zline(&mut lay, v(-22.0, -32.0), v(12.0, -32.0), 4.5, &[Door(-10.0, 2.4)], brick);
        self.zline(&mut lay, v(12.0, -32.0), v(44.0, -32.0), 4.0, &[Window(22.0, 8, 1.0), Window(36.0, 8, 1.0)], fence);
        self.zline(&mut lay, v(44.0, -32.0), v(60.0, -32.0), 4.0, &[Door(52.0, 2.4)], fence);
        self.zline(&mut lay, v(-26.0, -48.0), v(60.0, -48.0), 4.0, &[Window(-16.0, 8, -1.0), Window(4.0, 8, -1.0), Window(24.0, 8, -1.0), Window(46.0, 8, -1.0)], fence);
        // The casting floor, last of all.
        self.zline(&mut lay, v(14.0, 44.0), v(14.0, 72.0), 10.0, &[Window(52.0, 9, -1.0), Window(64.0, 9, -1.0)], brick);
        self.zline(&mut lay, v(14.0, 72.0), v(60.0, 72.0), 10.0, &[Window(24.0, 9, 1.0), Window(40.0, 9, 1.0), Window(52.0, 9, 1.0)], brick);

        // Buyable ways through: the core first, then out to the ring round it, then the casting floor.
        let doors = [
            (v(0.0, 10.0), true, (0, 1), 750, false),
            (v(12.0, -2.0), false, (0, 2), 750, true),
            (v(0.0, -12.0), true, (0, 4), 1000, false),
            (v(14.0, 21.0), false, (1, 3), 1000, false),
            (v(19.0, 8.0), true, (2, 3), 1000, true),
            (v(-22.0, -22.0), false, (4, 7), 1000, true),
            (v(-22.0, 21.0), false, (1, 7), 1000, false),
            (v(-4.0, 32.0), true, (1, 5), 1250, false),
            (v(24.0, 32.0), true, (3, 5), 1250, false),
            (v(44.0, 26.0), false, (3, 6), 1250, true),
            (v(-24.0, 32.0), true, (7, 5), 1000, false),
            (v(-24.0, -32.0), true, (7, 8), 1000, true),
            (v(-10.0, -32.0), true, (4, 8), 1250, false),
            (v(52.0, -32.0), true, (8, 6), 1250, true),
            (v(52.0, 32.0), true, (6, 5), 1000, false),
            (v(36.0, 44.0), true, (5, 9), 1500, false),
            (v(54.0, 44.0), true, (5, 9), 1500, true),
        ];
        for (at, along_x, zones, price, debris) in doors {
            self.buy_door(&mut lay, at, along_x, 2.4, zones, price, debris);
        }

        // Roofs: decor only (nothing climbs up there), with skylights over the furnace.
        self.decor(Aabb::span(Vec3::new(-22.2, g + 6.5, 9.8), Vec3::new(14.2, g + 6.75, 32.2)), Paint::Iron);
        self.decor(Aabb::span(Vec3::new(11.8, g + 4.0, -12.2), Vec3::new(32.2, g + 4.25, 8.2)), Paint::Dark);
        self.decor(Aabb::span(Vec3::new(13.8, g + 9.0, 7.8), Vec3::new(44.2, g + 9.3, 32.2)), Paint::Iron);
        self.decor(Aabb::span(Vec3::new(-26.2, g + 4.0, -32.2), Vec3::new(-21.8, g + 4.2, 32.2)), Paint::Dark);
        self.decor(Aabb::span(Vec3::new(-26.2, g + 7.5, 31.8), Vec3::new(60.2, g + 7.75, 44.2)), Paint::Iron);
        self.decor(Aabb::span(Vec3::new(13.8, g + 10.0, 43.8), Vec3::new(60.2, g + 10.3, 72.2)), Paint::Dark);
        // Roof trusses in the warehouse, the hall and the mill.
        for x in [-16.0, -8.0, 0.0, 8.0] {
            self.decor(Aabb::span(Vec3::new(x - 0.12, g + 5.9, 10.0), Vec3::new(x + 0.12, g + 6.5, 32.0)), Paint::Dark);
        }
        for x in [20.0, 26.0, 32.0, 38.0] {
            self.decor(Aabb::span(Vec3::new(x - 0.15, g + 8.2, 8.0), Vec3::new(x + 0.15, g + 9.0, 32.0)), Paint::Dark);
        }
        for k in 0..8 {
            let x = -20.0 + k as f32 * 11.0;
            self.decor(Aabb::span(Vec3::new(x - 0.15, g + 6.9, 32.0), Vec3::new(x + 0.15, g + 7.5, 44.0)), Paint::Dark);
        }

        self.foundry_courtyard(&mut lay, rng);
        self.foundry_warehouse(&mut lay, rng);
        self.foundry_offices(&mut lay, rng);
        self.foundry_furnace(&mut lay, rng);
        self.foundry_dock(&mut lay, rng);
        self.foundry_mill(&mut lay);
        self.foundry_yard(&mut lay);
        self.foundry_tunnel(&mut lay);
        self.foundry_siding(&mut lay);
        self.foundry_casting(&mut lay);
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
        Self::wall_buy(lay, Vec2::new(-21.8, 15.0), Vec2::X, gun::PDW, 1100, 1);
        let m = self.machine(lay, Vec2::new(-12.5, 10.2), Vec2::Y, 1, perk_id::IRON_HIDE, 2.2);
        lay.perks.push(m);
        let c = self.machine(lay, Vec2::new(6.0, 31.8), Vec2::NEG_Y, 1, 0, 0.0);
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
        let c = self.machine(lay, Vec2::new(28.0, 8.2), Vec2::Y, 3, 0, 0.0);
        lay.crates.push(c);
        self.relay(lay, Vec2::new(43.8, 15.5), Vec2::NEG_X, 3, g);
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

    /// The rolling mill: one long hall the length of the north side, roller tables down one
    /// wall and coils down the other, an arc grate in front of its middle window.
    fn foundry_mill(&mut self, lay: &mut HordeLayout) {
        let g = lay.ground;
        for (x0, x1) in [(12.0, 22.0), (26.0, 32.0), (40.0, 50.0)] {
            self.solid(Aabb::span(Vec3::new(x0, g, 40.6), Vec3::new(x1, g + 1.0, 42.0)), Paint::Iron);
            let mut x = x0 + 0.3;
            while x < x1 - 0.2 {
                self.decor(Aabb::span(Vec3::new(x, g + 1.0, 40.7), Vec3::new(x + 0.3, g + 1.12, 41.9)), Paint::Steel);
                x += 1.0;
            }
        }
        for (x, z) in [(18.0, 34.2), (30.0, 34.2), (47.0, 34.6)] {
            self.column(Vec2::new(x, z), 0.8, g, g + 1.6, Paint::Copper);
        }
        self.crates(Vec2::new(-20.0, 34.0), true);
        // The overhead crane: rails along both walls and a bridge across.
        for z in [32.7, 43.3] {
            self.decor(Aabb::span(Vec3::new(-25.8, g + 6.0, z - 0.2), Vec3::new(59.8, g + 6.3, z + 0.2)), Paint::Gold);
        }
        self.decor(Aabb::span(Vec3::new(19.5, g + 5.7, 32.5), Vec3::new(20.5, g + 6.0, 43.5)), Paint::Gold);
        self.decor(Aabb::span(Vec3::new(19.8, g + 3.8, 37.6), Vec3::new(20.2, g + 5.7, 38.0)), Paint::Dark);
        for x in [-18.0, -6.0, 6.0, 18.0, 30.0, 42.0, 54.0] {
            self.hanging_light(Vec2::new(x, 38.0), g + 7.5, Paint::Glow);
        }
        Self::wall_buy(lay, Vec2::new(4.0, 32.2), Vec2::Y, gun::LEVER, 1000, 5);
        Self::wall_buy(lay, Vec2::new(40.0, 32.2), Vec2::Y, gun::DRUM_SMG, 1300, 5);
        let m = self.machine(lay, Vec2::new(-14.0, 43.8), Vec2::NEG_Y, 5, perk_id::SCRAP_MAGNET, 2.2);
        lay.perks.push(m);
        let c = self.machine(lay, Vec2::new(-16.0, 32.2), Vec2::Y, 5, 0, 0.0);
        lay.crates.push(c);
        self.relay(lay, Vec2::new(9.0, 43.8), Vec2::NEG_Y, 5, g);
        self.horde_trap(lay, (-12.0, -4.0, 35.0, 42.6), Vec2::new(-10.0, 32.2), Vec2::Y, 5);
    }

    /// The scrap yard: a fenced lot of wrecks and a car crusher, open to the sky.
    fn foundry_yard(&mut self, lay: &mut HordeLayout) {
        let g = lay.ground;
        // A wrecked car, scrap piles, the crusher in the corner.
        self.solid(Aabb::span(Vec3::new(46.5, g, 6.0), Vec3::new(50.0, g + 1.2, 11.0)), Paint::Coral);
        self.solid(Aabb::span(Vec3::new(47.0, g + 1.2, 7.2), Vec3::new(49.5, g + 1.9, 9.6)), Paint::Rust);
        self.decor(Aabb::span(Vec3::new(46.9, g + 1.3, 9.6), Vec3::new(49.6, g + 1.8, 9.65)), Paint::Screen);
        for (x0, x1, z0, z1, h) in [(52.0, 56.0, -9.0, -5.0, 1.7), (47.0, 51.0, -18.0, -14.0, 1.2), (53.0, 57.0, 22.0, 26.0, 1.4)] {
            self.solid(Aabb::span(Vec3::new(x0, g, z0), Vec3::new(x1, g + h, z1)), Paint::Rust);
            self.decor(Aabb::span(Vec3::new(x0 + 0.5, g + h, z0 + 0.5), Vec3::new(x1 - 0.8, g + h + 0.5, z1 - 0.6)), Paint::Iron);
        }
        self.solid(Aabb::span(Vec3::new(55.0, g, -31.8), Vec3::new(59.8, g + 3.0, -25.0)), Paint::Iron);
        self.decor(Aabb::span(Vec3::new(55.4, g + 3.0, -31.4), Vec3::new(59.4, g + 3.3, -25.4)), Paint::Gold);
        self.decor(Aabb::span(Vec3::new(54.96, g + 1.0, -30.0), Vec3::new(55.0, g + 2.2, -26.5)), Paint::Spark);
        // Floodlight masts.
        for (x, z) in [(45.0, -30.8), (59.0, 30.8), (45.0, 15.0)] {
            self.decor(Aabb::span(Vec3::new(x - 0.1, g, z - 0.1), Vec3::new(x + 0.1, g + 7.0, z + 0.1)), Paint::Dark);
            self.decor(Aabb::span(Vec3::new(x - 0.5, g + 7.0, z - 0.3), Vec3::new(x + 0.5, g + 7.4, z + 0.3)), Paint::Glow);
        }
        Self::wall_buy(lay, Vec2::new(59.8, -12.0), Vec2::NEG_X, gun::CQB, 1200, 6);
        let c = self.machine(lay, Vec2::new(59.8, 10.0), Vec2::NEG_X, 6, 0, 0.0);
        lay.crates.push(c);
        self.relay(lay, Vec2::new(44.2, -12.0), Vec2::X, 6, g);
    }

    /// The cable tunnel: a tight concrete run down the whole west side, pipes overhead, and an
    /// arc grate halfway along.
    fn foundry_tunnel(&mut self, lay: &mut HordeLayout) {
        let g = lay.ground;
        for x in [-25.6, -22.4] {
            self.decor(Aabb::span(Vec3::new(x - 0.15, g + 3.0, -32.0), Vec3::new(x + 0.15, g + 3.3, 32.0)), Paint::Copper);
        }
        self.decor(Aabb::span(Vec3::new(-24.3, g + 3.6, -32.0), Vec3::new(-23.7, g + 3.7, 32.0)), Paint::Dark);
        let mut z = -28.0;
        while z < 30.0 {
            self.hanging_light(Vec2::new(-24.0, z), g + 4.0, if (z as i32 / 8) % 2 == 0 { Paint::Glow } else { Paint::Spark });
            // Bulkhead ribs.
            self.decor(Aabb::span(Vec3::new(-25.8, g + 3.4, z + 3.9), Vec3::new(-22.2, g + 4.0, z + 4.1)), Paint::Dark);
            z += 8.0;
        }
        Self::wall_buy(lay, Vec2::new(-22.2, 12.0), Vec2::NEG_X, gun::SHOTGUN, 1200, 7);
        self.horde_trap(lay, (-25.6, -22.4, -4.0, 4.0), Vec2::new(-22.2, -6.0), Vec2::NEG_X, 7);
    }

    /// The rail siding: a train of boxcars on the southern track, long lanes either side.
    fn foundry_siding(&mut self, lay: &mut HordeLayout) {
        let g = lay.ground;
        for z in [-42.0, -36.5] {
            self.flat(-25.8, 59.8, z - 1.1, z + 1.1, g + 0.008, Paint::Dark);
            for dz in [-0.72, 0.72] {
                self.decor(Aabb::span(Vec3::new(-25.8, g, z + dz - 0.05), Vec3::new(59.8, g + 0.14, z + dz + 0.05)), Paint::Steel);
            }
        }
        for (k, (x0, x1)) in [(-14.0, -5.0), (-2.0, 7.0), (10.0, 19.0), (24.0, 33.0), (37.0, 46.0)].into_iter().enumerate() {
            let paint = [Paint::Coral, Paint::Rust, Paint::Teal, Paint::Rust, Paint::Coral][k];
            self.solid(Aabb::span(Vec3::new(x0, g, -43.4), Vec3::new(x1, g + 3.2, -40.6)), paint);
            self.decor(Aabb::span(Vec3::new(x0 - 0.05, g + 3.2, -43.5), Vec3::new(x1 + 0.05, g + 3.35, -40.5)), Paint::Dark);
            let mid = (x0 + x1) * 0.5;
            for z in [-43.42, -40.58] {
                self.decor(Aabb::span(Vec3::new(mid - 1.2, g + 0.5, z - 0.02), Vec3::new(mid + 1.2, g + 2.8, z + 0.02)), Paint::Dark);
            }
        }
        for p in [Vec2::new(-24.8, -46.8), Vec2::new(58.8, -46.8), Vec2::new(16.0, -33.2)] {
            self.lamp(p);
        }
        Self::wall_buy(lay, Vec2::new(6.0, -32.2), Vec2::NEG_Y, gun::SNIPER, 1750, 8);
        let c = self.machine(lay, Vec2::new(35.0, -47.8), Vec2::Y, 8, 0, 0.0);
        lay.crates.push(c);
        self.relay(lay, Vec2::new(-6.0, -47.8), Vec2::Y, 8, g);
    }

    /// The casting floor: moulds and a ladle on the floor, a catwalk deck along the north wall
    /// (stairs at both ends) with the reforge up on it.
    fn foundry_casting(&mut self, lay: &mut HordeLayout) {
        let g = lay.ground;
        let top = g + 3.2;
        self.deck(22.0, 54.0, 64.0, 71.8, g, top, 64.0, (24.0, 52.0), &[30.0, 38.0, 46.0]);
        self.stairs_z(22.0, 24.0, 64.0, -1.0, g, top);
        self.stairs_z(52.0, 54.0, 64.0, -1.0, g, top);
        for (x, z) in [(26.0, 50.0), (48.0, 50.0), (26.0, 56.0), (48.0, 56.0)] {
            self.solid(Aabb::span(Vec3::new(x - 1.5, g, z - 1.0), Vec3::new(x + 1.5, g + 0.8, z + 1.0)), Paint::Iron);
            self.decor(Aabb::span(Vec3::new(x - 1.2, g + 0.8, z - 0.7), Vec3::new(x + 1.2, g + 0.82, z + 0.7)), Paint::Glow);
        }
        self.column(Vec2::new(37.0, 51.0), 1.3, g, g + 1.8, Paint::Rust);
        self.decor(Aabb::new(Vec3::new(37.0, g + 1.82, 51.0), Vec3::new(0.9, 0.02, 0.9)), Paint::Spark);
        for (x, z) in [(26.0, 58.0), (48.0, 58.0), (37.0, 47.0)] {
            self.hanging_light(Vec2::new(x, z), g + 10.0, Paint::Glow);
        }
        Self::wall_buy(lay, Vec2::new(14.2, 58.0), Vec2::X, gun::MINIGUN, 2500, 9);
        let m = self.machine(lay, Vec2::new(59.8, 50.0), Vec2::NEG_X, 9, perk_id::SPARE_SOCKET, 2.2);
        lay.perks.push(m);
        let f = self.machine_on(lay, Vec2::new(38.0, 71.8), Vec2::NEG_Y, 9, 0, 1.6, top);
        lay.forge = Some(f);
        self.relay(lay, Vec2::new(30.0, 71.8), Vec2::NEG_Y, 9, top);
        let c = self.machine(lay, Vec2::new(24.0, 44.2), Vec2::Y, 9, 0, 0.0);
        lay.crates.push(c);
    }

    /// The dead works outside the walls: chimneys, silos, a fence line, scrap heaps.
    fn foundry_outside(&mut self, rng: &mut Rng) {
        for (x, z, h) in [(-46.0, 60.0, 22.0), (82.0, 40.0, 26.0), (82.0, -20.0, 18.0), (-46.0, -66.0, 20.0)] {
            self.column(Vec2::new(x, z), 1.4, self.height(x, z), self.height(x, z) + h, Paint::Rust);
            self.decor(Aabb::new(Vec3::new(x, self.height(x, z) + h + 0.2, z), Vec3::new(1.1, 0.2, 1.1)), Paint::Spark);
        }
        for (x, z) in [(-50.0, 8.0), (80.0, 72.0)] {
            self.silo(Vec2::new(x, z), 3.5, 12.0, Paint::Iron, None);
        }
        // Scrap heaps and dead trees, well clear of the husks' lanes in.
        let clear = |p: Vec2| p.x > -42.0 && p.x < 76.0 && p.y > -64.0 && p.y < 88.0;
        for _ in 0..28 {
            let p = Vec2::new(rng.range(-88.0, 88.0), rng.range(-88.0, 88.0));
            if clear(p) {
                continue;
            }
            self.props.push(Prop { kind: PropKind::Ruin, pos: Vec3::new(p.x, self.height(p.x, p.y), p.y), yaw: rng.range(0.0, TAU), scale: rng.range(0.8, 1.4) });
        }
        for _ in 0..40 {
            let p = Vec2::new(rng.range(-88.0, 88.0), rng.range(-88.0, 88.0));
            if clear(p) {
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
        let v = Vec2::new;
        lay.zones = vec![
            HordeZone { name: "LANDING PAD", rect: (-10.0, 10.0, -10.0, 10.0) },
            HordeZone { name: "BOARDWALK", rect: (-36.0, -10.0, -7.0, 10.0) },
            HordeZone { name: "GREENHOUSE", rect: (-14.0, 14.0, 10.0, 30.0) },
            HordeZone { name: "SPECIMEN LAB", rect: (10.0, 34.0, -12.0, 10.0) },
            HordeZone { name: "PUMP HOUSE", rect: (-26.0, -2.0, -32.0, -10.0) },
            HordeZone { name: "COLD STORE", rect: (-36.0, -14.0, 10.0, 30.0) },
            HordeZone { name: "GLASS GALLERY", rect: (-40.0, 44.0, 30.0, 40.0) },
            HordeZone { name: "OBSERVATORY", rect: (14.0, 44.0, 10.0, 30.0) },
            HordeZone { name: "PIPE TUNNEL", rect: (-40.0, -36.0, -32.0, 30.0) },
            HordeZone { name: "REED YARD", rect: (-40.0, 54.0, -48.0, -32.0) },
            HordeZone { name: "HANGAR", rect: (34.0, 54.0, -32.0, 10.0) },
        ];
        let panel = Paint::Shell;
        let metal = Paint::Steel;
        let glass = Paint::Teal;
        let fence = Paint::Wood;
        let floors = [Paint::Dark, Paint::Leaf, Paint::Bone, Paint::Metal, Paint::Concrete, Paint::Steel, Paint::Steel, Paint::Metal, Paint::Concrete, Paint::Leaf, Paint::Concrete];
        for (zone, paint) in lay.zones.clone().iter().zip(floors) {
            let (x0, x1, z0, z1) = zone.rect;
            self.flat(x0, x1, z0, z1, g + 0.002, paint);
        }

        // The old core round the pad.
        self.zline(&mut lay, v(-10.0, -10.0), v(-10.0, 10.0), 3.4, &[Door(1.5, 2.4)], metal);
        self.zline(&mut lay, v(-36.0, 10.0), v(14.0, 10.0), 4.2, &[Door(0.0, 2.4), Door(-25.0, 2.4)], panel);
        self.zline(&mut lay, v(10.0, -12.0), v(10.0, 10.0), 3.8, &[Door(0.0, 2.4)], metal);
        // The pump house sits off to the south-west, so the pad's south wall looks out on the fen.
        self.zline(&mut lay, v(-26.0, -10.0), v(-2.0, -10.0), 5.0, &[Door(-6.0, 2.4)], Paint::Concrete);
        self.zline(&mut lay, v(-2.0, -10.0), v(10.0, -10.0), 3.4, &[Window(0.2, 0, -1.0), Window(4.5, 0, -1.0), Window(8.5, 0, -1.0)], metal);
        self.zline(&mut lay, v(-14.0, 10.0), v(-14.0, 30.0), 4.2, &[Door(20.0, 2.4)], panel);
        self.zline(&mut lay, v(-36.0, -7.0), v(-10.0, -7.0), 3.4, &[Window(-33.0, 1, -1.0), Window(-28.5, 1, -1.0)], metal);
        self.zline(&mut lay, v(10.0, -12.0), v(34.0, -12.0), 3.8, &[Window(16.0, 3, -1.0), Window(22.0, 3, -1.0), Window(28.0, 3, -1.0)], metal);
        self.zline(&mut lay, v(-26.0, -32.0), v(-26.0, -10.0), 5.0, &[Window(-20.0, 4, -1.0), Window(-13.0, 4, -1.0)], Paint::Concrete);
        self.zline(&mut lay, v(-2.0, -32.0), v(-2.0, -10.0), 5.0, &[Window(-24.0, 4, 1.0)], Paint::Concrete);
        // The pipe tunnel down the west side.
        self.zline(&mut lay, v(-36.0, -32.0), v(-36.0, -7.0), 3.4, &[], Paint::Concrete);
        self.zline(&mut lay, v(-36.0, -7.0), v(-36.0, 10.0), 3.4, &[Door(1.5, 2.4)], metal);
        self.zline(&mut lay, v(-36.0, 10.0), v(-36.0, 30.0), 4.2, &[], panel);
        self.zline(&mut lay, v(-40.0, -48.0), v(-40.0, -32.0), 4.0, &[Window(-40.0, 9, -1.0)], fence);
        self.zline(&mut lay, v(-40.0, -32.0), v(-40.0, 30.0), 3.4, &[Window(-24.0, 8, -1.0), Window(-10.0, 8, -1.0), Window(6.0, 8, -1.0), Window(20.0, 8, -1.0)], Paint::Concrete);
        self.zline(&mut lay, v(-40.0, 30.0), v(-40.0, 40.0), 4.6, &[Window(35.0, 6, -1.0)], glass);
        // The glass gallery along the north.
        self.zline(&mut lay, v(-40.0, 30.0), v(-36.0, 30.0), 4.6, &[Door(-38.0, 2.4)], panel);
        self.zline(&mut lay, v(-36.0, 30.0), v(14.0, 30.0), 4.6, &[Door(-25.0, 2.4), Door(-7.0, 2.4)], panel);
        self.zline(&mut lay, v(14.0, 30.0), v(44.0, 30.0), 7.0, &[Door(17.0, 2.4)], metal);
        self.zline(&mut lay, v(-40.0, 40.0), v(44.0, 40.0), 4.6, &[Window(-28.0, 6, 1.0), Window(-8.0, 6, 1.0), Window(12.0, 6, 1.0), Window(32.0, 6, 1.0)], glass);
        self.zline(&mut lay, v(44.0, 30.0), v(44.0, 40.0), 4.6, &[Window(35.0, 6, 1.0)], glass);
        // The observatory and the hangar to the east.
        self.zline(&mut lay, v(44.0, 10.0), v(44.0, 30.0), 7.0, &[Window(15.0, 7, 1.0), Window(25.0, 7, 1.0)], metal);
        self.zline(&mut lay, v(14.0, 10.0), v(14.0, 30.0), 7.0, &[], glass);
        self.zline(&mut lay, v(14.0, 10.0), v(44.0, 10.0), 7.0, &[Door(22.0, 2.4), Door(39.0, 2.4)], metal);
        self.zline(&mut lay, v(44.0, 10.0), v(54.0, 10.0), 6.0, &[Window(49.0, 10, 1.0)], metal);
        self.zline(&mut lay, v(34.0, -32.0), v(34.0, 10.0), 6.0, &[Door(-7.0, 2.4), Window(-22.0, 10, -1.0)], metal);
        self.zline(&mut lay, v(54.0, -32.0), v(54.0, 10.0), 6.0, &[Window(-22.0, 10, 1.0), Window(-6.0, 10, 1.0)], metal);
        // The reed yard along the south.
        self.zline(&mut lay, v(54.0, -48.0), v(54.0, -32.0), 4.0, &[Window(-40.0, 9, 1.0)], fence);
        self.zline(&mut lay, v(-40.0, -32.0), v(-36.0, -32.0), 4.0, &[Door(-38.0, 2.4)], fence);
        self.zline(&mut lay, v(-36.0, -32.0), v(-26.0, -32.0), 4.0, &[Window(-31.0, 9, 1.0)], fence);
        self.zline(&mut lay, v(-26.0, -32.0), v(-2.0, -32.0), 5.0, &[Door(-10.0, 2.4)], Paint::Concrete);
        self.zline(&mut lay, v(-2.0, -32.0), v(34.0, -32.0), 4.0, &[Window(8.0, 9, 1.0), Window(22.0, 9, 1.0)], fence);
        self.zline(&mut lay, v(34.0, -32.0), v(54.0, -32.0), 6.0, &[Door(44.0, 2.4)], metal);
        self.zline(&mut lay, v(-40.0, -48.0), v(54.0, -48.0), 4.0, &[Window(-26.0, 9, -1.0), Window(-6.0, 9, -1.0), Window(16.0, 9, -1.0), Window(40.0, 9, -1.0)], fence);

        let doors = [
            (v(-10.0, 1.5), false, (0, 1), 750, true),
            (v(0.0, 10.0), true, (0, 2), 1000, false),
            (v(10.0, 0.0), false, (0, 3), 750, false),
            (v(-6.0, -10.0), true, (0, 4), 1250, false),
            (v(-25.0, 10.0), true, (1, 5), 1000, true),
            (v(-14.0, 20.0), false, (5, 2), 1000, false),
            (v(-36.0, 1.5), false, (1, 8), 1000, true),
            (v(-38.0, 30.0), true, (8, 6), 1250, false),
            (v(-38.0, -32.0), true, (8, 9), 1250, true),
            (v(-25.0, 30.0), true, (5, 6), 1250, false),
            (v(-7.0, 30.0), true, (2, 6), 1250, false),
            (v(17.0, 30.0), true, (6, 7), 1500, false),
            (v(22.0, 10.0), true, (3, 7), 1500, false),
            (v(39.0, 10.0), true, (10, 7), 1500, true),
            (v(34.0, -7.0), false, (3, 10), 1250, false),
            (v(-10.0, -32.0), true, (4, 9), 1250, false),
            (v(44.0, -32.0), true, (10, 9), 1250, true),
        ];
        for (at, along_x, zones, price, debris) in doors {
            self.buy_door(&mut lay, at, along_x, 2.4, zones, price, debris);
        }

        // Roofs: the greenhouse and the gallery are lattices of glass, the rest flat.
        self.decor(Aabb::span(Vec3::new(-36.2, g + 4.2, 9.8), Vec3::new(-13.8, g + 4.45, 30.2)), Paint::Dark);
        self.decor(Aabb::span(Vec3::new(9.8, g + 3.8, -12.2), Vec3::new(34.2, g + 4.05, 10.2)), Paint::Dark);
        self.decor(Aabb::span(Vec3::new(-26.2, g + 5.0, -32.2), Vec3::new(-1.8, g + 5.25, -9.8)), Paint::Iron);
        self.decor(Aabb::span(Vec3::new(-40.2, g + 3.4, -32.2), Vec3::new(-35.8, g + 3.6, 30.2)), Paint::Dark);
        self.decor(Aabb::span(Vec3::new(13.8, g + 7.0, 9.8), Vec3::new(44.2, g + 7.3, 30.2)), Paint::Dark);
        self.decor(Aabb::span(Vec3::new(33.8, g + 6.0, -32.2), Vec3::new(54.2, g + 6.25, 10.2)), Paint::Iron);
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
        let mut x = -40.0;
        while x <= 44.0 {
            self.decor(Aabb::span(Vec3::new(x - 0.1, g + 4.5, 30.0), Vec3::new(x + 0.1, g + 4.75, 40.0)), Paint::Dark);
            x += 6.0;
        }
        for z in [30.0, 35.0, 40.0] {
            self.decor(Aabb::span(Vec3::new(-40.0, g + 4.5, z - 0.1), Vec3::new(44.0, g + 4.75, z + 0.1)), Paint::Dark);
        }

        self.fen_pad(&mut lay, rng);
        self.fen_boardwalk(&mut lay, rng);
        self.fen_greenhouse(&mut lay, rng);
        self.fen_lab(&mut lay, rng);
        self.fen_pump(&mut lay, rng);
        self.fen_cold(&mut lay, rng);
        self.fen_gallery(&mut lay);
        self.fen_observatory(&mut lay);
        self.fen_tunnel(&mut lay);
        self.fen_yard(&mut lay, rng);
        self.fen_hangar(&mut lay);
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
        // Rails along the walk (thin, walk-through).
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
        self.relay(lay, Vec2::new(-30.0, 9.8), Vec2::NEG_Y, 1, g);
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
        let c = self.machine(lay, Vec2::new(16.0, 9.8), Vec2::NEG_Y, 3, 0, 0.0);
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
        Self::wall_buy(lay, Vec2::new(-25.8, -16.5), Vec2::X, gun::BATTLE_RIFLE, 1600, 4);
        let p = self.machine(lay, Vec2::new(-15.0, -31.8), Vec2::Y, 4, 0, 0.0);
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
        let c = self.machine(lay, Vec2::new(-18.0, 29.8), Vec2::NEG_Y, 5, 0, 0.0);
        lay.crates.push(c);
        let _ = rng;
    }

    /// The glass gallery: one long hall of specimen cases the width of the station, an arc
    /// grate in front of its west windows.
    fn fen_gallery(&mut self, lay: &mut HordeLayout) {
        let g = lay.ground;
        for (x0, x1) in [(-34.0, -30.0), (-22.0, -16.0), (6.0, 11.0), (24.0, 29.0), (36.0, 40.0)] {
            self.solid(Aabb::span(Vec3::new(x0, g, 34.4), Vec3::new(x1, g + 1.0, 35.6)), Paint::Bone);
            self.decor(Aabb::span(Vec3::new(x0 + 0.2, g + 1.0, 34.6), Vec3::new(x1 - 0.2, g + 1.5, 35.4)), Paint::Aether);
            self.decor(Aabb::span(Vec3::new(x0, g + 1.5, 34.4), Vec3::new(x1, g + 1.56, 35.6)), Paint::Steel);
        }
        for x in [-30.0, -18.0, -6.0, 6.0, 18.0, 30.0, 40.0] {
            self.hanging_light(Vec2::new(x, 35.0), g + 4.5, Paint::Aether);
        }
        Self::wall_buy(lay, Vec2::new(-18.0, 39.8), Vec2::NEG_Y, gun::LEVER, 1000, 6);
        Self::wall_buy(lay, Vec2::new(30.0, 30.2), Vec2::Y, gun::DRUM_SMG, 1300, 6);
        let m = self.machine(lay, Vec2::new(22.0, 39.8), Vec2::NEG_Y, 6, perk_id::SCRAP_MAGNET, 2.2);
        lay.perks.push(m);
        let c = self.machine(lay, Vec2::new(40.0, 39.8), Vec2::NEG_Y, 6, 0, 0.0);
        lay.crates.push(c);
        self.relay(lay, Vec2::new(2.0, 39.8), Vec2::NEG_Y, 6, g);
        self.horde_trap(lay, (-12.0, -4.0, 31.0, 38.6), Vec2::new(-16.0, 30.2), Vec2::Y, 6);
    }

    /// The observatory: a tall dome room, the telescope on its plinth, and an upper deck along
    /// the north wall (stairs at both ends) with the reforge on it.
    fn fen_observatory(&mut self, lay: &mut HordeLayout) {
        let g = lay.ground;
        let top = g + 3.2;
        self.deck(20.0, 42.0, 24.0, 29.8, g, top, 24.0, (22.0, 40.0), &[26.0, 31.0, 36.0]);
        self.stairs_z(20.0, 22.0, 24.0, -1.0, g, top);
        self.stairs_z(40.0, 42.0, 24.0, -1.0, g, top);
        // The telescope: a plinth and a long tube pointing up at the roof hatch.
        self.solid(Aabb::span(Vec3::new(27.5, g, 12.5), Vec3::new(30.5, g + 1.2, 15.5)), Paint::Steel);
        self.column(Vec2::new(29.0, 14.0), 0.5, g + 1.2, g + 2.4, Paint::Dark);
        for k in 0..5 {
            let y = g + 2.4 + k as f32 * 0.7;
            self.decor(Aabb::new(Vec3::new(29.0, y, 14.0 + k as f32 * 0.35), Vec3::new(0.45, 0.36, 0.45)), if k == 4 { Paint::Aether } else { Paint::Bone });
        }
        self.decor(Aabb::span(Vec3::new(26.0, g + 6.95, 13.0), Vec3::new(32.0, g + 7.05, 17.0)), Paint::Blue);
        for (x, z) in [(18.0, 14.0), (38.0, 14.0), (31.0, 20.0)] {
            self.hanging_light(Vec2::new(x, z), g + 7.0, Paint::Violet);
        }
        Self::wall_buy(lay, Vec2::new(14.2, 19.0), Vec2::X, gun::SNIPER, 1750, 7);
        let m = self.machine(lay, Vec2::new(43.8, 17.5), Vec2::NEG_X, 7, perk_id::SPARE_SOCKET, 2.2);
        lay.perks.push(m);
        let f = self.machine_on(lay, Vec2::new(31.0, 29.8), Vec2::NEG_Y, 7, 0, 1.6, top);
        lay.forge = Some(f);
        self.relay(lay, Vec2::new(25.0, 29.8), Vec2::NEG_Y, 7, top);
    }

    /// The pipe tunnel: a tight run down the west side, pipes along both walls, an arc grate.
    fn fen_tunnel(&mut self, lay: &mut HordeLayout) {
        let g = lay.ground;
        for x in [-39.6, -36.4] {
            self.decor(Aabb::span(Vec3::new(x - 0.15, g + 2.5, -32.0), Vec3::new(x + 0.15, g + 2.8, 30.0)), Paint::Copper);
            self.decor(Aabb::span(Vec3::new(x - 0.1, g + 2.9, -32.0), Vec3::new(x + 0.1, g + 3.1, 30.0)), Paint::Teal);
        }
        let mut z = -28.0;
        while z < 28.0 {
            self.hanging_light(Vec2::new(-38.0, z), g + 3.4, Paint::Green);
            z += 9.0;
        }
        Self::wall_buy(lay, Vec2::new(-36.2, 14.0), Vec2::NEG_X, gun::CQB, 1200, 8);
        self.horde_trap(lay, (-39.6, -36.4, -6.0, 2.0), Vec2::new(-36.2, -8.0), Vec2::NEG_X, 8);
    }

    /// The reed yard: an open mudflat along the south, sluice kerbs and a beached skiff.
    fn fen_yard(&mut self, lay: &mut HordeLayout, rng: &mut Rng) {
        let g = lay.ground;
        for (x0, x1) in [(-34.0, -20.0), (-14.0, 2.0), (10.0, 30.0), (36.0, 50.0)] {
            self.solid(Aabb::span(Vec3::new(x0, g, -40.5), Vec3::new(x1, g + 0.6, -39.5)), Paint::Concrete);
            self.decor(Aabb::span(Vec3::new(x0, g + 0.01, -39.5), Vec3::new(x1, g + 0.03, -38.0)), Paint::Dark);
        }
        self.solid(Aabb::span(Vec3::new(0.0, g, -46.5), Vec3::new(6.0, g + 1.3, -43.5)), Paint::Rust);
        self.decor(Aabb::span(Vec3::new(1.0, g + 1.3, -46.0), Vec3::new(3.5, g + 2.2, -44.0)), Paint::White);
        for _ in 0..14 {
            let p = Vec2::new(rng.range(-38.0, 52.0), rng.range(-38.0, -34.0));
            self.props.push(Prop { kind: PropKind::Bush, pos: Vec3::new(p.x, g, p.y), yaw: rng.range(0.0, TAU), scale: rng.range(0.8, 1.3) });
        }
        for p in [Vec2::new(-38.8, -46.8), Vec2::new(52.8, -46.8), Vec2::new(14.0, -33.2), Vec2::new(-20.0, -46.8)] {
            self.lamp(p);
        }
        Self::wall_buy(lay, Vec2::new(0.0, -32.2), Vec2::NEG_Y, gun::BURST, 1200, 9);
        let c = self.machine(lay, Vec2::new(-16.0, -47.8), Vec2::Y, 9, 0, 0.0);
        lay.crates.push(c);
        self.relay(lay, Vec2::new(26.0, -47.8), Vec2::Y, 9, g);
    }

    /// The hangar: a tall shed with a hover skiff up on blocks.
    fn fen_hangar(&mut self, lay: &mut HordeLayout) {
        let g = lay.ground;
        self.solid(Aabb::span(Vec3::new(46.0, g, -28.0), Vec3::new(51.0, g + 1.4, -20.0)), Paint::Teal);
        self.solid(Aabb::span(Vec3::new(47.0, g + 1.4, -26.0), Vec3::new(50.0, g + 2.4, -22.0)), Paint::White);
        self.decor(Aabb::span(Vec3::new(46.9, g + 1.6, -22.05), Vec3::new(50.1, g + 2.2, -22.0)), Paint::Screen);
        self.crates(Vec2::new(37.0, -29.5), true);
        self.crates(Vec2::new(52.0, -2.0), false);
        self.decor(Aabb::span(Vec3::new(34.0, g + 5.0, -14.2), Vec3::new(54.0, g + 5.3, -13.8)), Paint::Gold);
        for (x, z) in [(40.0, -20.0), (48.0, -8.0), (40.0, 2.0)] {
            self.hanging_light(Vec2::new(x, z), g + 6.0, Paint::Glow);
        }
        Self::wall_buy(lay, Vec2::new(34.2, 4.0), Vec2::X, gun::MINIGUN, 2500, 10);
        let c = self.machine(lay, Vec2::new(46.0, 9.8), Vec2::NEG_Y, 10, 0, 0.0);
        lay.crates.push(c);
        self.relay(lay, Vec2::new(53.8, 3.0), Vec2::NEG_X, 10, g);
    }

    /// The fen outside: dead trees, reeds, a radio mast, a sunk boat.
    fn fen_outside(&mut self, rng: &mut Rng) {
        let clear = |p: Vec2| p.x > -54.0 && p.x < 68.0 && p.y > -62.0 && p.y < 54.0;
        for _ in 0..90 {
            let p = Vec2::new(rng.range(-74.0, 74.0), rng.range(-74.0, 74.0));
            if clear(p) {
                continue;
            }
            let h = self.height(p.x, p.y);
            let kind = if h > self.water + 0.2 && rng.chance(0.45) { PropKind::Tree } else { PropKind::Bush };
            let scale = if kind == PropKind::Tree { rng.range(3.4, 5.6) } else { rng.range(0.9, 1.6) };
            self.props.push(Prop { kind, pos: Vec3::new(p.x, h.max(self.water - 0.2), p.y), yaw: rng.range(0.0, TAU), scale });
        }
        let mast = Vec2::new(64.0, 62.0);
        let h = self.height(mast.x, mast.y).max(self.water);
        self.column(mast, 0.3, h, h + 24.0, Paint::Steel);
        self.decor(Aabb::new(Vec3::new(mast.x, h + 24.2, mast.y), Vec3::new(0.25, 0.25, 0.25)), Paint::Red);
        for (x, z) in [(-62.0, -30.0), (30.0, -68.0)] {
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
            let area: f32 = lay.zones.iter().map(|z| (z.rect.1 - z.rect.0) * (z.rect.3 - z.rect.2)).sum();
            println!("{what}: {} blocks ({} solid), {} zones, {area:.0} m2, {} windows, {} doors, {} buys", w.blocks.len(), w.blocks.iter().filter(|b| b.solid).count(), lay.zones.len(), lay.windows.len(), lay.doors.len(), lay.buys.len());
            // Budgets: a few thousand blocks at most.
            assert!(w.blocks.len() > 300 && w.blocks.len() < 4000, "{what}: {} blocks", w.blocks.len());
            // Big: two to three times the floor the first versions had (about 3000 m2).
            assert!(area > 6500.0 && area < 10_000.0, "{what}: {area:.0} m2 of zones");
            assert!(lay.zones.len() >= 10 && lay.doors.len() >= 15, "{what}");
            assert!(lay.windows.len() >= 30, "{what}: {} windows", lay.windows.len());
            assert!(lay.buys.len() >= 15 && lay.perks.len() == 7 && lay.crates.len() >= 6, "{what}");
            assert!(lay.forge.is_some() && lay.power.is_some(), "{what}");
            assert!(lay.traps.len() >= 2 && lay.relays.len() >= 4 && lay.relays.len() <= 8, "{what}");
            // Zones never overlap, and each one has a door or two (so there is always a way round).
            for (i, a) in lay.zones.iter().enumerate() {
                for b in &lay.zones[i + 1..] {
                    let overlap = (a.rect.1.min(b.rect.1) - a.rect.0.max(b.rect.0)).min(a.rect.3.min(b.rect.3) - a.rect.2.max(b.rect.2));
                    assert!(overlap <= 0.0, "{what}: {} and {} overlap", a.name, b.name);
                }
                let doors = lay.doors.iter().filter(|d| d.zones.0 as usize == i || d.zones.1 as usize == i).count();
                assert!(doors >= 2, "{what}: {} is a dead end ({doors} door)", a.name);
            }
            // The perks are spread out: no zone has more than one machine.
            for z in 0..lay.zones.len() {
                assert!(lay.perks.iter().filter(|m| m.zone as usize == z).count() <= 1, "{what}: perks crowd {}", lay.zones[z].name);
            }
            assert!(lay.crates.iter().map(|c| c.zone).collect::<std::collections::HashSet<_>>().len() >= 6, "{what}: the crate moves between zones");
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
            for (i, m) in lay.machines().enumerate() {
                clear(m.pos, &format!("machine {i}"));
                assert_eq!(lay.zone_at(m.pos.xz()), Some(m.zone), "{what}: machine {i} at {:?}", m.pos);
                // Standing on something: the ground, or a deck.
                let floor = s.tops_at(m.pos.x, m.pos.z).into_iter().any(|t| (t - m.pos.y).abs() < 0.05) || (m.pos.y - lay.ground).abs() < 0.05;
                assert!(floor, "{what}: machine {i} floats at {:?}", m.pos);
            }
            // The reforge is up on a deck.
            let forge = lay.forge.unwrap();
            assert!(forge.pos.y > lay.ground + 2.5, "{what}: the reforge should be up on the deck");
            for (i, trap) in lay.traps.iter().enumerate() {
                let (x0, x1, z0, z1) = trap.floor;
                for p in [Vec2::new(x0 + 0.3, z0 + 0.3), Vec2::new(x1 - 0.3, z1 - 0.3), Vec2::new((x0 + x1) * 0.5, (z0 + z1) * 0.5)] {
                    assert_eq!(lay.zone_at(p), Some(trap.zone), "{what}: trap {i}'s floor leaves its zone");
                    clear(Vec3::new(p.x, lay.ground, p.y), &format!("trap {i} floor"));
                }
                assert!(trap.panel.pos.xz().distance(Vec2::new((x0 + x1) * 0.5, (z0 + z1) * 0.5)) < 10.0, "{what}: trap {i}'s panel is far from it");
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

    /// Writes each horde map's blocks and layout as plain text (for drawing overviews):
    /// `OPNIX_HORDE_DUMP=dir cargo test -p oz_core dump_horde_maps -- --ignored`.
    #[test]
    #[ignore]
    fn dump_horde_maps() {
        let Ok(dir) = std::env::var("OPNIX_HORDE_DUMP") else { return };
        for layout in Layout::HORDE {
            let w = World::generate(WorldRecipe::preset(layout, Biome::IronShoals, Weather::Clear));
            let lay = w.horde.as_ref().unwrap();
            let mut out = String::new();
            let m = |kind: &str, m: &HordeMachine, out: &mut String| out.push_str(&format!("machine {kind} {} {} {} {} {}\n", m.pos.x, m.pos.y, m.pos.z, m.zone, m.kind));
            for b in &w.blocks {
                let (lo, hi) = (b.aabb.center - b.aabb.half, b.aabb.center + b.aabb.half);
                out.push_str(&format!("block {} {} {} {} {} {} {} {:?}\n", b.solid as u8, lo.x, lo.y, lo.z, hi.x, hi.y, hi.z, b.paint));
            }
            for z in &lay.zones {
                out.push_str(&format!("zone {} {} {} {} {}\n", z.rect.0, z.rect.1, z.rect.2, z.rect.3, z.name.replace(' ', "_")));
            }
            for d in &lay.doors {
                out.push_str(&format!("door {} {} {} {} {} {}\n", d.aabb.center.x, d.aabb.center.z, d.price, d.debris as u8, d.zones.0, d.zones.1));
            }
            for win in &lay.windows {
                out.push_str(&format!("window {} {} {} {} {}\n", win.center.x, win.center.z, win.spawn.x, win.spawn.z, win.zone));
            }
            for b in &lay.buys {
                out.push_str(&format!("buy {} {} {} {}\n", b.pos.x, b.pos.z, b.weapon, b.price));
            }
            for p in &lay.perks {
                m("perk", p, &mut out);
            }
            for c in &lay.crates {
                m("crate", c, &mut out);
            }
            for r in &lay.relays {
                m("relay", r, &mut out);
            }
            m("forge", lay.forge.as_ref().unwrap(), &mut out);
            m("power", lay.power.as_ref().unwrap(), &mut out);
            for t in &lay.traps {
                out.push_str(&format!("trap {} {} {} {}\n", t.floor.0, t.floor.1, t.floor.2, t.floor.3));
                m("panel", &t.panel, &mut out);
            }
            out.push_str(&format!("spawn {} {}\nhalf {}\n", w.spawn.x, w.spawn.y, w.recipe.half_extent));
            std::fs::write(format!("{dir}/{layout:?}.txt"), out).unwrap();
        }
    }
}
