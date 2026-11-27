//! Procedural maps.
//!
//! A [`World`] is generated from a [`WorldRecipe`] and is fully deterministic:
//! host and clients only exchange the recipe, never geometry. [`Layout`] picks
//! the generator: archipelagos (one home island, outlying islets joined by
//! sandbars) for co-op, and towns for versus - the random town, which rolls a
//! [`Theme`] from its seed (suburbs, desert outpost, farmland, industrial
//! town, woods park) so everything on the map belongs together, and the fixed
//! maps (Crossroads, Dockyard, Office Tower, Space Station, Mountainside).
//! Every generator fills in the same [`World`] struct to work with the rest of
//! the engine.
//!
//! The pieces live in submodules: `shapes` (building shapes, roofs,
//! silos, towers, fences, furniture), `themes` (the random town per theme),
//! `office`, `station` and `mountain` (the big fixed maps).

mod mountain;
mod office;
mod paving;
mod shapes;
mod station;
mod themes;
mod zombie_maps;

pub use themes::Theme;
pub use zombie_maps::{SILL as HORDE_SILL, HordeDoor, HordeLayout, HordeMachine, HordeVent, HordeWindow, HordeZone, WallBuy, perk_id};
use shapes::{Furnish, Plan, Roof};

use crate::{collide::Aabb, economy::Ore, model::{Hull, Paint}, noise, rng::Rng};
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::f32::consts::TAU;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Biome {
    EmberAtoll,
    BlackwaterCaldera,
    FrostbiteStation,
    IronShoals,
    SunkenReach,
}

/// Colours and flavour for a biome. All colours are sRGB in `0..1`.
#[derive(Clone, Copy, Debug)]
pub struct BiomeLook {
    pub title: &'static str,
    pub blurb: &'static str,
    pub sand: [f32; 3],
    pub grass: [f32; 3],
    pub high: [f32; 3],
    pub horizon: [f32; 3],
    pub zenith: [f32; 3],
    pub water: [f32; 3],
    pub fog: [f32; 3],
    pub palms: usize,
    pub ruins: usize,
}

impl Biome {
    pub const ALL: [Biome; 5] = [Biome::EmberAtoll, Biome::BlackwaterCaldera, Biome::FrostbiteStation, Biome::IronShoals, Biome::SunkenReach];

    pub fn look(self) -> BiomeLook {
        match self {
            Biome::EmberAtoll => BiomeLook {
                title: "EMBER ATOLL",
                blurb: "Warm-water defense perimeter and the first beacon site.",
                sand: [0.89, 0.69, 0.43],
                grass: [0.37, 0.62, 0.43],
                high: [0.39, 0.47, 0.43],
                horizon: [1.0, 0.65, 0.47],
                zenith: [0.16, 0.35, 0.55],
                water: [0.16, 0.49, 0.54],
                fog: [0.65, 0.68, 0.67],
                palms: 90,
                ruins: 6,
            },
            Biome::BlackwaterCaldera => BiomeLook {
                title: "BLACKWATER CALDERA",
                blurb: "Volcanic ridges, dark terrain and glowing industrial markers.",
                sand: [0.42, 0.30, 0.27],
                grass: [0.30, 0.27, 0.26],
                high: [0.24, 0.20, 0.21],
                horizon: [0.85, 0.32, 0.18],
                zenith: [0.16, 0.12, 0.21],
                water: [0.12, 0.17, 0.22],
                fog: [0.45, 0.33, 0.30],
                palms: 0,
                ruins: 14,
            },
            Biome::FrostbiteStation => BiomeLook {
                title: "FROSTBITE STATION",
                blurb: "Frozen terrain, blue sky and biting wind.",
                sand: [0.74, 0.81, 0.86],
                grass: [0.82, 0.90, 0.95],
                high: [0.62, 0.74, 0.84],
                horizon: [0.67, 0.82, 0.92],
                zenith: [0.21, 0.38, 0.63],
                water: [0.20, 0.42, 0.58],
                fog: [0.72, 0.80, 0.86],
                palms: 0,
                ruins: 10,
            },
            Biome::IronShoals => BiomeLook {
                title: "IRON SHOALS",
                blurb: "Industrial reef with taller cover and concrete pylons.",
                sand: [0.60, 0.56, 0.50],
                grass: [0.41, 0.50, 0.50],
                high: [0.40, 0.44, 0.49],
                horizon: [0.83, 0.68, 0.45],
                zenith: [0.18, 0.26, 0.34],
                water: [0.15, 0.37, 0.43],
                fog: [0.60, 0.62, 0.60],
                palms: 8,
                ruins: 22,
            },
            Biome::SunkenReach => BiomeLook {
                title: "SUNKEN REACH",
                blurb: "Low broken shores and flooded ruins near the signal.",
                sand: [0.50, 0.60, 0.50],
                grass: [0.30, 0.56, 0.48],
                high: [0.29, 0.42, 0.45],
                horizon: [0.55, 0.42, 0.68],
                zenith: [0.08, 0.16, 0.33],
                water: [0.10, 0.30, 0.36],
                fog: [0.45, 0.50, 0.58],
                palms: 40,
                ruins: 16,
            },
        }
    }
}

/// Which generator builds the map.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Layout {
    /// A home island ringed by ore-rich islets joined by sandbars.
    Archipelago,
    /// The random town: a big map (about 220 m across) whose [`Theme`] - suburbs, desert
    /// outpost, farmland, industrial town or woods park - is rolled from the seed and sets the
    /// buildings, colours, cover and street pattern. Different every seed.
    Town,
    /// Fixed map: three lanes between the bases - a tight alley, the main street through
    /// the fountain plaza, and a long market lane watched by a three-storey tower.
    Crossroads,
    /// Fixed map: a container yard - corridors of stacked shipping containers, a big
    /// warehouse in the middle, offices on the flanks.
    Dockyard,
    /// Fixed indoor map: a four-storey office block round an atrium - cubicles, glass-walled
    /// offices, meeting rooms, a lobby - with four stairwells, a roof and a plaza outside.
    OfficeTower,
    /// Fixed indoor map: an enclosed station - a hangar and a cargo bay at the ends, airlocks
    /// and corridors to a ring round a two-level hub, labs, quarters and a reactor.
    SpaceStation,
    /// Fixed map on a slope: terraces joined by a switchback road and stairs, cliffs to hold,
    /// a village in the valley and a lodge and radio station on the summit.
    Mountainside,
    /// Horde map: an abandoned foundry at night - a courtyard, a warehouse, offices with the
    /// generator, a furnace hall and a loading dock (see `zombie_maps`).
    CinderWorks,
    /// Horde map: a swamp research station in thick fog - a landing pad, a boardwalk, a
    /// greenhouse, a specimen lab, a pump house and a cold store.
    Blackfen,
}

impl Layout {
    /// Built as a town (flat ground, streets, team bases).
    pub fn is_town(self) -> bool {
        !matches!(self, Layout::Archipelago | Layout::CinderWorks | Layout::Blackfen)
    }

    /// Horde (round survival) maps.
    pub const HORDE: [Layout; 2] = [Layout::CinderWorks, Layout::Blackfen];

    pub fn is_horde(self) -> bool {
        matches!(self, Layout::CinderWorks | Layout::Blackfen)
    }

    /// Versus map choices: the random town, then the fixed maps.
    pub const VERSUS: [Layout; 6] = [Layout::Town, Layout::Crossroads, Layout::Dockyard, Layout::OfficeTower, Layout::SpaceStation, Layout::Mountainside];

    pub fn title(self) -> &'static str {
        match self {
            Layout::Archipelago => "ARCHIPELAGO",
            Layout::Town => "RANDOM TOWN",
            Layout::Crossroads => "CROSSROADS",
            Layout::Dockyard => "DOCKYARD",
            Layout::OfficeTower => "OFFICE TOWER",
            Layout::SpaceStation => "SPACE STATION",
            Layout::Mountainside => "MOUNTAINSIDE",
            Layout::CinderWorks => "CINDER WORKS",
            Layout::Blackfen => "BLACKFEN STATION",
        }
    }

    /// Indoor maps: a roof over nearly all of the play area.
    pub fn is_indoor(self) -> bool {
        matches!(self, Layout::OfficeTower | Layout::SpaceStation)
    }
}

/// Sky and air for a map; purely visual.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Weather {
    #[default]
    Clear,
    Overcast,
    Rain,
    Fog,
}

impl Weather {
    pub const ALL: [Weather; 4] = [Weather::Clear, Weather::Overcast, Weather::Rain, Weather::Fog];

    /// A seed's natural weather (mostly clear).
    pub fn from_seed(seed: u64) -> Weather {
        [Weather::Clear, Weather::Clear, Weather::Overcast, Weather::Rain, Weather::Fog, Weather::Clear][(seed % 6) as usize]
    }

    pub fn name(self) -> &'static str {
        match self {
            Weather::Clear => "CLEAR",
            Weather::Overcast => "OVERCAST",
            Weather::Rain => "RAIN",
            Weather::Fog => "FOG",
        }
    }
}

/// Everything needed to rebuild a map. This is what travels over the network.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorldRecipe {
    pub seed: u64,
    pub biome: Biome,
    /// The map spans `[-half_extent, half_extent]` on X and Z.
    pub half_extent: f32,
    /// Terrain grid resolution in metres.
    pub cell: f32,
    pub main_radius: f32,
    pub islets: usize,
    pub layout: Layout,
    pub weather: Weather,
}

impl WorldRecipe {
    pub fn archipelago(seed: u64, biome: Biome) -> Self {
        Self { seed, biome, half_extent: 160.0, cell: 2.0, main_radius: 46.0, islets: 5, layout: Layout::Archipelago, weather: Weather::from_seed(seed) }
    }

    /// The random town: a big square of land about 230 m across, themed by its seed.
    pub fn town(seed: u64, biome: Biome) -> Self {
        Self { seed, biome, half_extent: 150.0, cell: 2.0, main_radius: 128.0, islets: 0, layout: Layout::Town, weather: Weather::from_seed(seed) }
    }

    /// The random town with a chosen theme: the first seed from `seed` on that rolls it.
    pub fn themed(seed: u64, biome: Biome, theme: Theme) -> Self {
        let seed = (seed..).find(|s| Theme::from_seed(*s) == theme).unwrap_or(seed);
        Self::town(seed, biome)
    }

    /// A horde map with its own look: the foundry under a low night sky, the fen in fog.
    pub fn horde(layout: Layout) -> Self {
        match layout {
            Layout::Blackfen => Self::preset(layout, Biome::SunkenReach, Weather::Fog),
            _ => Self::preset(Layout::CinderWorks, Biome::BlackwaterCaldera, Weather::Overcast),
        }
    }

    /// A fixed map: the same streets every time (its own seed), in any biome and weather.
    pub fn preset(layout: Layout, biome: Biome, weather: Weather) -> Self {
        let seed = match layout {
            Layout::Crossroads => 0xC055_2045,
            Layout::Dockyard => 0xD0C5_1D0C,
            Layout::OfficeTower => 0x0FF1_CE00,
            Layout::SpaceStation => 0x57A7_1000,
            Layout::Mountainside => 0x3077_7A15,
            Layout::CinderWorks => 0xC1D3_0005,
            Layout::Blackfen => 0xB1AC_F3E0,
            _ => 7,
        };
        // The compact maps keep the original island; the others get a square of land.
        let (half_extent, main_radius) = match layout {
            Layout::Crossroads | Layout::Dockyard => (96.0, 68.0),
            Layout::OfficeTower => (100.0, 84.0),
            Layout::SpaceStation => (104.0, 96.0),
            Layout::CinderWorks | Layout::Blackfen => (72.0, 60.0),
            _ => (150.0, 128.0),
        };
        Self { layout, weather, half_extent, main_radius, ..Self::town(seed, biome) }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Island {
    pub center: Vec2,
    pub radius: f32,
    phase: [f32; 2],
    falloff: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PropKind {
    Palm,
    Rock,
    Ruin,
    Pylon,
    Crate,
    /// A broadleaf tree: trunk and a blocky crown.
    Tree,
    /// Waist-high shrubs: hide in them, they stop nothing.
    Bush,
    /// A lamp post.
    Lamp,
}

#[derive(Clone, Copy, Debug)]
pub struct Prop {
    pub kind: PropKind,
    /// Ground position.
    pub pos: Vec3,
    pub yaw: f32,
    pub scale: f32,
}

impl Prop {
    /// Axis-aligned collision half-extents, centred at `pos + half.y`. `None` = no collision.
    pub fn solid_half(&self) -> Option<Vec3> {
        let s = self.scale;
        match self.kind {
            PropKind::Palm => Some(Vec3::new(0.26, s * 0.5, 0.26)),
            // Square, so a rock turned a quarter turn keeps the same box.
            PropKind::Rock => Some(Vec3::new(s * 0.75, s * 0.45, s * 0.75)),
            PropKind::Ruin => Some(Vec3::new(1.6 * s, 0.9 * s, 0.35 * s)),
            PropKind::Pylon => Some(Vec3::new(0.7, 1.9 * s, 0.7)),
            PropKind::Crate => Some(Vec3::new(0.62, 0.55, 0.62)),
            PropKind::Tree => Some(Vec3::new(0.3, s * 0.5, 0.3)),
            PropKind::Bush => None,
            // Too thin to hide behind: walk through it.
            PropKind::Lamp => None,
        }
    }
}

/// A piece of architecture: a wall, floor, stair, railing or trim. Solid blocks
/// are also collision; the rest is decoration. Rendering and collision both
/// come from this one list, so they can never disagree - except under a sloped
/// roof (see [`World::roofs`]).
#[derive(Clone, Copy, Debug)]
pub struct Block {
    pub aabb: Aabb,
    pub paint: Paint,
    pub solid: bool,
    /// Not drawn: the stair steps under a sloped roof, which are walked on (and shot, and
    /// hidden behind) but drawn as the smooth roof over them instead.
    pub hidden: bool,
}

impl Block {
    /// The box as drawn: grown by [`draw_margin`], so a trim, plinth, sign or stripe laid
    /// flush on a wall or floor always shows in front of it instead of flickering against
    /// it. Collision uses [`Block::aabb`].
    pub fn drawn(&self) -> Aabb {
        Aabb::new(self.aabb.center, self.aabb.half + Vec3::splat(draw_margin(self.aabb.half.min_element() * 2.0, self.paint)))
    }
}

/// How much a block `thickness` thick is grown when drawn: a few millimetres, the thinner
/// the more, so wherever two faces would lie in one plane the thinner piece (the trim on
/// the wall, the stripe on the floor) is drawn just in front. Thickness picks a 1.5 mm
/// step; paint picks a 0.05 mm notch within it, so two pieces of different paint never
/// end up in one plane again.
pub fn draw_margin(thickness: f32, paint: Paint) -> f32 {
    const STEP: f32 = 0.0015;
    let by_thickness = 0.02 * (1.0 - (-0.04 / thickness.max(1e-3)).exp());
    let notch = Paint::ALL.iter().position(|p| *p == paint).unwrap_or(0) as f32 * (STEP / Paint::ALL.len() as f32);
    0.0005 + (by_thickness / STEP).round() * STEP + notch
}

/// A doorway that holds a door (it can be opened, closed and blown off).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DoorSpec {
    /// Bottom centre of the opening.
    pub center: Vec3,
    /// The wall runs along X (else Z).
    pub along_x: bool,
    pub width: f32,
    pub height: f32,
}

/// A glazed window opening.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowSpec {
    pub center: Vec3,
    pub along_x: bool,
    pub width: f32,
    pub height: f32,
}

/// Street furniture that can be shot up, knocked over or blown apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClutterKind {
    Bin,
    /// Explodes when destroyed.
    Barrel,
    Bench,
    Mailbox,
    /// A parked car: big cover, and it blows up.
    Car,
    Crates,
}

impl ClutterKind {
    /// Collision half-extents along its own axes (unrotated), centred `half.y` above the ground.
    pub fn half(self) -> Vec3 {
        match self {
            ClutterKind::Bin => Vec3::new(0.4, 0.55, 0.35),
            ClutterKind::Barrel => Vec3::new(0.32, 0.5, 0.32),
            ClutterKind::Bench => Vec3::new(0.9, 0.3, 0.3),
            ClutterKind::Mailbox => Vec3::new(0.25, 0.6, 0.25),
            ClutterKind::Car => Vec3::new(1.0, 0.75, 2.1),
            ClutterKind::Crates => Vec3::new(0.6, 0.6, 0.6),
        }
    }

    /// Damage it takes before it breaks.
    pub fn health(self) -> f32 {
        match self {
            ClutterKind::Bin => 45.0,
            ClutterKind::Barrel => 35.0,
            ClutterKind::Bench => 80.0,
            ClutterKind::Mailbox => 55.0,
            ClutterKind::Car => 420.0,
            ClutterKind::Crates => 60.0,
        }
    }

    /// (damage, radius) of the blast when it goes, if it explodes.
    pub fn blast(self) -> Option<(f32, f32)> {
        match self {
            ClutterKind::Barrel => Some((150.0, 7.5)),
            ClutterKind::Car => Some((220.0, 10.0)),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clutter {
    pub kind: ClutterKind,
    pub pos: Vec3,
    /// Turned a quarter: its X and Z extents swap.
    pub turned: bool,
}

impl Clutter {
    /// World-axis collision half-extents.
    pub fn half(&self) -> Vec3 {
        let h = self.kind.half();
        if self.turned { Vec3::new(h.z, h.y, h.x) } else { h }
    }

    pub fn center(&self) -> Vec3 {
        self.pos + Vec3::Y * self.kind.half().y
    }
}

/// A ladder up an outside wall to a roof.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ladder {
    /// Ground point at its foot.
    pub foot: Vec3,
    /// Height of the roof it reaches.
    pub top: f32,
    /// Horizontal direction from the wall out to the climber.
    pub out: Vec2,
}

#[derive(Clone, Copy, Debug)]
pub struct Deposit {
    pub pos: Vec3,
    pub ore: Ore,
    pub amount: f32,
}

/// Flat vertex data for one terrain chunk (non-indexed triangle list, linear RGB colours).
#[derive(Default)]
pub struct TerrainChunk {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 4]>,
}

pub struct World {
    pub recipe: WorldRecipe,
    pub water: f32,
    pub islands: Vec<Island>,
    pub props: Vec<Prop>,
    pub blocks: Vec<Block>,
    /// Sloped roofs and round caps, drawn smooth over the hidden stair steps that carry
    /// their collision (render only; each just covers its steps' outer corners).
    pub roofs: Vec<Hull>,
    /// Roads, pavements and painted ground: triangles laid on the terrain (render only).
    pub paving: Vec<(Paint, [Vec3; 3])>,
    pub deposits: Vec<Deposit>,
    /// Offshore points around the home island where sea creatures surface.
    pub landings: Vec<Vec2>,
    /// Where the objective (beacon core, flag, payload...) sits.
    pub core: Vec2,
    pub spawn: Vec2,
    /// Flat spots around the core for stations and other mode furniture.
    pub pads: Vec<Vec2>,
    /// Spawn points per team for versus modes (empty on co-op maps).
    pub team_spawns: [Vec<Vec2>; 2],
    pub doors: Vec<DoorSpec>,
    pub windows: Vec<WindowSpec>,
    pub ladders: Vec<Ladder>,
    /// Horde maps: zones, buyable doors, barricaded windows, wall buys and machines.
    pub horde: Option<Box<HordeLayout>>,
    /// Tufts of grass (render only).
    pub grass: Vec<Vec3>,
    /// The top of the fountain's spout, if the map has one.
    pub fountain: Option<Vec3>,
    /// Flag pole tops (flags wave in the wind).
    pub flags: Vec<Vec3>,
    /// Breakable street furniture.
    pub clutter: Vec<Clutter>,
    /// Places worth going to on the indoor maps - rooms, corridors, upper floors, the roof -
    /// where bots sweep and hostiles hold posts (empty elsewhere: anywhere in the arena will do).
    pub waypoints: Vec<Vec3>,
    /// The random town's theme (`None` on every other layout).
    pub theme: Option<Theme>,
    /// Half the side of the square the fighting happens in: bots sweep it, hostiles hold posts
    /// in it, loot lies in it.
    pub arena: f32,
    /// How buildings look on this map (wall and roof paints).
    style: shapes::Style,
    /// Streets and paths laid so far, (from, to, width), for parking cars and lining with trees.
    roads: Vec<(Vec2, Vec2, f32)>,
    /// The same roads, with their looks, until they are paved.
    lanes: Vec<paving::Lane>,
    /// Areas of ground painted so far.
    painted: usize,
    /// Where each building's doorway steps lead, and whether that side was left open.
    thresholds: Vec<(Vec3, bool)>,
    /// Building sites chosen before the ground is shaped, (centre, radius): the ground stays flat there.
    flats: Vec<(Vec2, f32)>,
    side: usize,
    heights: Vec<f32>,
}

pub const WATER_LEVEL: f32 = -0.35;
/// Height of low cover (walls, barriers, parapets): above a crouching robot's head, below a vault.
pub const COVER_HEIGHT: f32 = 1.2;
/// Window sills: crouch below one to hide; vault through it.
pub const WINDOW_SILL: f32 = 1.15;
/// A doorway: wide enough for a robot, and a door that swings in it.
pub const DOOR_WIDTH: f32 = 1.7;
/// Doorway height.
pub const DOOR_HEIGHT: f32 = 2.35;
/// Storey height of every building: eleven stair steps of 0.29 m.
pub const STOREY: f32 = 3.2;
/// How deep a player may wade before the sea pushes back.
pub const WADE_DEPTH: f32 = 0.65;
const SEABED: f32 = -1.4;
const BASE_RADIUS: f32 = 15.0;

impl World {
    pub fn generate(recipe: WorldRecipe) -> World {
        let mut rng = Rng::new(recipe.seed);
        let mut islands = vec![Island { center: Vec2::ZERO, radius: recipe.main_radius, phase: [rng.range(0.0, TAU), rng.range(0.0, TAU)], falloff: 11.0 }];
        let start = rng.range(0.0, TAU);
        for i in 0..recipe.islets {
            let angle = start + (i as f32 + rng.range(-0.22, 0.22)) * TAU / recipe.islets as f32;
            let radius = rng.range(15.0, 23.0);
            let distance = recipe.main_radius + radius + rng.range(26.0, 44.0);
            islands.push(Island { center: Vec2::from_angle(angle) * distance, radius, phase: [rng.range(0.0, TAU), rng.range(0.0, TAU)], falloff: 7.0 });
        }

        let side = (2.0 * recipe.half_extent / recipe.cell).round() as usize + 1;
        let mut world = World {
            recipe,
            water: WATER_LEVEL,
            islands,
            props: Vec::new(),
            blocks: Vec::new(),
            roofs: Vec::new(),
            deposits: Vec::new(),
            landings: Vec::new(),
            core: Vec2::ZERO,
            spawn: Vec2::new(0.0, 6.0),
            pads: Vec::new(),
            team_spawns: [Vec::new(), Vec::new()],
            side,
            heights: Vec::new(),
            doors: Vec::new(),
            windows: Vec::new(),
            ladders: Vec::new(),
            horde: None,
            grass: Vec::new(),
            fountain: None,
            flags: Vec::new(),
            clutter: Vec::new(),
            waypoints: Vec::new(),
            theme: (recipe.layout == Layout::Town).then(|| Theme::from_seed(recipe.seed)),
            arena: match recipe.layout {
                Layout::Archipelago => recipe.main_radius,
                Layout::Crossroads | Layout::Dockyard => 58.0,
                Layout::OfficeTower => 72.0,
                Layout::SpaceStation => 86.0,
                Layout::CinderWorks | Layout::Blackfen => 40.0,
                _ => 112.0,
            },
            style: shapes::Style::default(),
            roads: Vec::new(),
            lanes: Vec::new(),
            painted: 0,
            thresholds: Vec::new(),
            paving: Vec::new(),
            flats: Vec::new(),
        };
        if let Some(theme) = world.theme {
            world.style = theme.style();
            world.flats = world.plan_flats();
        }
        world.heights = (0..side * side)
            .map(|i| {
                let (ix, iz) = (i % side, i / side);
                world.raw_height(ix as f32 * recipe.cell - recipe.half_extent, iz as f32 * recipe.cell - recipe.half_extent)
            })
            .collect();

        match recipe.layout {
            Layout::Archipelago => {
                world.pads = (0..6).map(|i| Vec2::from_angle(TAU * (i as f32 + 0.5) / 6.0) * 10.0).collect();
                world.scatter(&mut rng.fork());
            }
            _ => world.build_town(&mut rng.fork()),
        }
        world.pave();
        world.tidy_clutter();
        world
    }

    /// Analytic terrain function; sampled once into the grid.
    fn raw_height(&self, x: f32, z: f32) -> f32 {
        let seed = self.recipe.seed;
        let p = Vec2::new(x, z);
        match self.recipe.layout {
            Layout::Town => return self.themed_height(x, z),
            Layout::Mountainside => return self.mountain_height(x, z),
            Layout::OfficeTower | Layout::SpaceStation => return self.square_land(x, z),
            Layout::CinderWorks | Layout::Blackfen => return self.horde_height(x, z),
            _ => {}
        }
        let mut best = SEABED + 0.25 * noise::fbm(seed ^ 0xbed, x * 0.03, z * 0.03, 2);
        for (index, island) in self.islands.iter().enumerate() {
            let d = p - island.center;
            let (r, theta) = (d.length(), d.y.atan2(d.x));
            let edge = island.radius * (1.0 + 0.085 * (theta * 3.0 + island.phase[0]).sin() + 0.05 * (theta * 5.0 + island.phase[1]).cos());
            let shore = ((edge - r) / island.falloff).clamp(-1.0, 1.0);
            if self.recipe.layout.is_town() {
                // Towns sit on a nearly flat plateau so streets read as streets.
                best = best.max(shore * (1.35 + 0.12 * noise::value(seed ^ 0x70, x * 0.09, z * 0.09)) - 0.15);
                continue;
            }
            let mut relief = 0.55 * noise::fbm(seed ^ 0x51ab, x * 0.07, z * 0.07, 3) + 0.25 * noise::value(seed ^ 0x77, x * 0.21, z * 0.21);
            if index == 0 {
                // One landmark hill, and a flat plateau for the base.
                relief += 3.6 * (-((x + 19.0).powi(2) + (z + 15.0).powi(2)) / 210.0).exp();
                relief *= ((r - BASE_RADIUS + 3.0) / 9.0).clamp(0.0, 1.0);
            } else {
                relief += 1.4 * (-(r * r) / (island.radius * island.radius * 0.3)).exp();
            }
            best = best.max(shore * (1.25 + relief.max(-0.6)) - 0.15);
            if index > 0 {
                // Sandbar back to the home island: walkable at low water.
                let along = island.center.normalize();
                let t = p.dot(along).clamp(0.0, island.center.length());
                let offset = (p - along * t).length() + 1.5 * noise::value(seed ^ 0xca5e, x * 0.11, z * 0.11);
                let ridge = ((4.5 - offset) / 3.5).clamp(0.0, 1.0);
                best = best.max(SEABED + ridge * (-SEABED - 0.38));
            }
        }
        best
    }

    /// Ground height, interpolated exactly like the rendered triangles.
    pub fn height(&self, x: f32, z: f32) -> f32 {
        let (cell, half) = (self.recipe.cell, self.recipe.half_extent);
        let max = (self.side - 2) as f32;
        let (gx, gz) = (((x + half) / cell).clamp(0.0, max + 0.9999), ((z + half) / cell).clamp(0.0, max + 0.9999));
        let (ix, iz) = (gx.floor() as usize, gz.floor() as usize);
        let (u, v) = (gx - ix as f32, gz - iz as f32);
        let at = |dx: usize, dz: usize| self.heights[(iz + dz) * self.side + ix + dx];
        let (a, b, c, d) = (at(0, 0), at(1, 0), at(0, 1), at(1, 1));
        if u + v <= 1.0 { a + (b - a) * u + (c - a) * v } else { d + (c - d) * (1.0 - u) + (b - d) * (1.0 - v) }
    }

    /// Every collision box on the map: solid architecture plus props.
    pub fn solids(&self) -> impl Iterator<Item = Aabb> + '_ {
        let architecture = self.blocks.iter().filter(|b| b.solid).map(|b| b.aabb);
        let props = self.props.iter().filter_map(|p| p.solid_half().map(|half| Aabb::new(p.pos + Vec3::Y * half.y, half)));
        architecture.chain(props)
    }

    /// Leaves that hide what is behind them (but stop no bullets): tree crowns and bushes,
    /// as spheres (centre, radius).
    pub fn foliage(&self) -> Vec<(Vec3, f32)> {
        self.props
            .iter()
            .filter_map(|p| match p.kind {
                PropKind::Tree => Some((p.pos + Vec3::Y * p.scale * 0.9, p.scale * 0.42)),
                PropKind::Bush => Some((p.pos + Vec3::Y * 0.45 * p.scale, 0.65 * p.scale)),
                _ => None,
            })
            .collect()
    }

    pub fn ground(&self, x: f32, z: f32) -> Vec3 {
        Vec3::new(x, self.height(x, z), z)
    }

    pub fn in_bounds(&self, x: f32, z: f32) -> bool {
        let limit = self.recipe.half_extent - 4.0;
        x.abs() < limit && z.abs() < limit
    }

    /// Can a walking character stand here? (Sea creatures ignore this.)
    pub fn walkable(&self, x: f32, z: f32) -> bool {
        self.in_bounds(x, z) && self.height(x, z) >= self.water - WADE_DEPTH
    }

    /// Distance along a ray until it dips below the terrain, or `max`.
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max: f32) -> f32 {
        // Fine steps close by, coarser far out (rounds now fly hundreds of metres), and done
        // as soon as a rising ray is above any terrain there could be.
        let mut d = 0.2;
        while d < max {
            let p = origin + dir * d;
            let ground = self.height(p.x, p.z);
            if p.y <= ground {
                return d;
            }
            if dir.y >= 0.0 && p.y > 60.0 {
                return max;
            }
            d += (0.3 + d * 0.012).min((p.y - ground).max(0.3));
        }
        max
    }

    fn scatter(&mut self, rng: &mut Rng) {
        let look = self.recipe.biome.look();
        let clear_of_base = |p: Vec2| p.length() > BASE_RADIUS + 3.0;

        // Ore: iron near home, copper further out, aether only on the islets.
        let home = self.islands[0];
        for i in 0..9 {
            let ore = if i < 6 { Ore::Iron } else { Ore::Copper };
            self.place_deposit(rng, home, if ore == Ore::Iron { (19.0, 34.0) } else { (30.0, 40.0) }, ore);
        }
        for index in 1..self.islands.len() {
            let island = self.islands[index];
            for i in 0..4 {
                let ore = match (index + i) % 4 {
                    0 => Ore::Iron,
                    1 | 2 => Ore::Copper,
                    _ => Ore::Aether,
                };
                self.place_deposit(rng, island, (2.0, island.radius * 0.6), ore);
            }
            self.place_deposit(rng, island, (0.0, 4.0), Ore::Aether);
        }

        let islands = self.islands.clone();
        let place = |world: &mut World, rng: &mut Rng, kind: PropKind, count: usize, min_height: f32, scale: (f32, f32)| {
            for _ in 0..count {
                let island = islands[if rng.chance(0.55) { 0 } else { rng.below(islands.len()) }];
                let p = island.center + Vec2::from_angle(rng.range(0.0, TAU)) * rng.range(0.0, island.radius * 0.93);
                let y = world.height(p.x, p.y);
                let crowded = world.deposits.iter().any(|d| Vec2::new(d.pos.x, d.pos.z).distance(p) < 3.5);
                if y < min_height || !clear_of_base(p) || crowded || (p.x.abs() < 4.0 && p.y > 0.0 && p.y < 30.0) {
                    continue;
                }
                world.props.push(Prop { kind, pos: Vec3::new(p.x, y, p.y), yaw: rng.range(0.0, TAU), scale: rng.range(scale.0, scale.1) });
            }
        };
        place(self, rng, PropKind::Palm, look.palms, 0.36, (2.8, 5.1));
        place(self, rng, PropKind::Rock, 70, 0.15, (0.4, 1.7));
        place(self, rng, PropKind::Ruin, look.ruins, 0.4, (0.8, 1.3));
        place(self, rng, PropKind::Pylon, look.ruins / 3, 0.4, (0.8, 1.4));
        place(self, rng, PropKind::Crate, 10, 0.4, (1.0, 1.0));

        // Landing points: a ring just off the home island's beach.
        for i in 0..20 {
            let dir = Vec2::from_angle(TAU * i as f32 / 20.0);
            let mut r = home.radius * 0.7;
            while r < home.radius + 30.0 && self.height(dir.x * r, dir.y * r) > self.water - 0.55 {
                r += 1.0;
            }
            self.landings.push(dir * (r + 3.0));
        }
    }

    /// A street grid of enterable buildings, cover in the lanes, and a base on each side.
    fn build_town(&mut self, rng: &mut Rng) {
        match self.recipe.layout {
            Layout::Crossroads => self.crossroads(rng),
            Layout::Dockyard => self.dockyard(rng),
            Layout::OfficeTower => return self.office_tower(rng),
            Layout::SpaceStation => return self.space_station(rng),
            Layout::Mountainside => return self.mountainside(rng),
            Layout::CinderWorks => return self.cinder_works(rng),
            Layout::Blackfen => return self.blackfen(rng),
            _ => return self.themed_town(rng),
        }
        self.finish_town(rng);
    }

    /// Fixed map: three lanes. North alley (z = 24): two rows of tight two-storey houses.
    /// Main street (z = 0): the plaza and fountain, parked cars. South market (z = -26): open,
    /// low walls, overwatched by a three-storey tower. Cross streets at x = -31, 0, 31.
    fn crossroads(&mut self, rng: &mut Rng) {
        let blocks = [-42.0, -15.5, 15.5, 42.0];
        for (i, x) in blocks.into_iter().enumerate() {
            let inner = i == 1 || i == 2;
            // Between main street and the alley, and on the alley's far side.
            self.building(rng, Vec2::new(x, 10.5), Vec2::new(if inner { 7.5 } else { 6.0 }, 4.2), 2);
            self.building(rng, Vec2::new(x, 32.0), Vec2::new(6.0, 4.0), if inner { 2 } else { 1 });
            // Between main street and the market.
            self.building(rng, Vec2::new(x, -11.0), Vec2::new(if inner { 7.0 } else { 6.0 }, 3.6), 1);
        }
        // The tower over the market, and quieter blocks at the back.
        self.building(rng, Vec2::new(0.0, -38.0), Vec2::new(6.2, 5.6), 3);
        for x in [-40.0, -20.0, 20.0, 40.0] {
            self.building(rng, Vec2::new(x, -44.0), Vec2::new(5.5, 4.5), if x.abs() < 30.0 { 2 } else { 1 });
            self.building(rng, Vec2::new(x, 47.0), Vec2::new(5.5, 4.0), 1);
        }
        // Market stalls: low courtyard walls along the south lane.
        for x in [-36.0, -12.0, 12.0, 36.0] {
            self.courtyard(rng, Vec2::new(x, -26.0));
        }
    }

    /// Fixed map: a container yard around a warehouse, with offices on the flanks.
    fn dockyard(&mut self, rng: &mut Rng) {
        // The warehouse: a big single hall in the middle.
        self.building(rng, Vec2::new(0.0, 0.0), Vec2::new(11.0, 8.0), 1);
        // Offices for overwatch.
        for (x, z) in [(-34.0, 22.0), (34.0, -22.0), (-34.0, -24.0), (34.0, 24.0)] {
            self.building(rng, Vec2::new(x, z), Vec2::new(6.0, 5.0), 2);
        }
        // Containers: rows that make corridors, some stacked two high.
        let container = |world: &mut World, at: Vec2, along_x: bool, high: bool, rng: &mut Rng| world.container(rng, at, along_x, if high { 2 } else { 1 });
        for (x, z0, z1) in [(-18.0, -40.0, -14.0), (-18.0, 14.0, 40.0), (18.0, -40.0, -14.0), (18.0, 14.0, 40.0)] {
            let mut z = z0;
            while z < z1 {
                let high = rng.chance(0.4);
                container(self, Vec2::new(x, z), false, high, rng);
                z += 6.6 + if rng.chance(0.3) { 3.0 } else { 0.0 };
            }
        }
        for (x0, x1, z) in [(-46.0, -24.0, -34.0), (-46.0, -24.0, 36.0), (24.0, 46.0, -34.0), (24.0, 46.0, 36.0), (-12.0, 12.0, 26.0), (-12.0, 12.0, -26.0)] {
            let mut x = x0;
            while x < x1 {
                let high = rng.chance(0.3);
                container(self, Vec2::new(x, z), true, high, rng);
                x += 6.6 + if rng.chance(0.35) { 3.2 } else { 0.0 };
            }
        }
        // Loose single containers for cover in the open.
        for (x, z, along) in [(-30.0, 6.0, false), (30.0, -6.0, false), (-8.0, 14.0, true), (8.0, -14.0, true)] {
            container(self, Vec2::new(x, z), along, false, rng);
        }
    }

    /// Everything a town needs whatever its layout: cover, the square, clutter, greenery, bases.
    fn finish_town(&mut self, rng: &mut Rng) {
        // Cover in the streets and the plaza.
        for i in 0..80 {
            let p = Vec2::new(rng.range(-60.0, 60.0), rng.range(-58.0, 58.0));
            if self.occupied(p, 1.6) || p.x.abs() > 54.0 {
                continue;
            }
            let kind = match i % 5 {
                0 | 1 => PropKind::Crate,
                2 => PropKind::Ruin,
                3 => PropKind::Rock,
                _ => PropKind::Pylon,
            };
            let scale = match kind {
                PropKind::Ruin => rng.range(0.6, 0.8),
                PropKind::Rock => rng.range(0.8, 1.3),
                PropKind::Pylon => rng.range(0.7, 1.0),
                _ => 1.0,
            };
            // Boxy cover stays axis-aligned to match its collision box.
            // Rocks turn only in quarter turns so their box still matches what is drawn.
            let yaw = if matches!(kind, PropKind::Rock) { rng.below(4) as f32 * std::f32::consts::FRAC_PI_2 } else { 0.0 };
            self.props.push(Prop { kind, pos: Vec3::new(p.x, self.height(p.x, p.y), p.y), yaw, scale });
        }
        // Concrete barriers in the streets: chest-high to a crouching robot, low enough to vault.
        let mut placed = 0;
        for _ in 0..120 {
            if placed >= 16 {
                break;
            }
            let p = Vec2::new(rng.range(-50.0, 50.0), rng.range(-50.0, 50.0));
            let along_x = rng.chance(0.5);
            let half = if along_x { Vec2::new(1.3, 0.28) } else { Vec2::new(0.28, 1.3) };
            if self.occupied(p, 2.4) || p.length() < 6.0 {
                continue;
            }
            let g = self.height(p.x, p.y);
            self.solid(Aabb::span(Vec3::new(p.x - half.x, g - 0.1, p.y - half.y), Vec3::new(p.x + half.x, g + COVER_HEIGHT, p.y + half.y)), Paint::Concrete);
            self.decor(
                Aabb::span(Vec3::new(p.x - half.x - 0.02, g + COVER_HEIGHT - 0.12, p.y - half.y - 0.02), Vec3::new(p.x + half.x + 0.02, g + COVER_HEIGHT - 0.04, p.y + half.y + 0.02)),
                Paint::Gold,
            );
            placed += 1;
        }
        if self.recipe.layout != Layout::Dockyard {
            self.plaza(rng);
        }
        self.street_clutter(rng);
        self.greenery(rng);
        let look = self.recipe.biome.look();
        for _ in 0..look.palms / 3 {
            let p = Vec2::from_angle(rng.range(0.0, TAU)) * rng.range(56.0, 64.0);
            if !self.occupied(p, 1.0) && p.x.abs() < 50.0 {
                self.props.push(Prop { kind: PropKind::Palm, pos: Vec3::new(p.x, self.height(p.x, p.y), p.y), yaw: 0.0, scale: rng.range(2.8, 5.0) });
            }
        }
        for team in 0..2 {
            let x = if team == 0 { -58.0 } else { 58.0 };
            self.team_spawns[team] = (0..6).map(|i| Vec2::new(x + (i % 2) as f32 * 2.5 * if team == 0 { 1.0 } else { -1.0 }, -12.5 + i as f32 * 5.0)).collect();
            // A little sandbagged cover in front of each base.
            let front = if team == 0 { -50.0 } else { 50.0 };
            for z in [-9.0, 0.0, 9.0] {
                let y = self.height(front, z);
                self.solid(Aabb::span(Vec3::new(front - 0.4, y - 0.2, z - 1.6), Vec3::new(front + 0.4, y + COVER_HEIGHT, z + 1.6)), Paint::Sand);
            }
        }
        self.spawn = self.team_spawns[0][2];
    }

    fn solid(&mut self, aabb: Aabb, paint: Paint) {
        self.blocks.push(Block { aabb, paint, solid: true, hidden: false });
    }

    fn decor(&mut self, aabb: Aabb, paint: Paint) {
        self.blocks.push(Block { aabb, paint, solid: false, hidden: false });
    }

    /// A band `out` thick around the outside of a footprint (plinths, floor-line trims):
    /// four strips hugging the walls, never a slab through the rooms inside.
    #[allow(clippy::too_many_arguments)]
    /// `skip` leaves sides out (south, west, east, north), where a wing joins on.
    fn ring(&mut self, x0: f32, x1: f32, z0: f32, z1: f32, y0: f32, y1: f32, out: f32, paint: Paint, skip: [bool; 4]) {
        if !skip[0] {
            self.decor(Aabb::span(Vec3::new(x0 - out, y0, z0 - out), Vec3::new(x1 + out, y1, z0)), paint);
        }
        if !skip[3] {
            self.decor(Aabb::span(Vec3::new(x0 - out, y0, z1), Vec3::new(x1 + out, y1, z1 + out)), paint);
        }
        if !skip[1] {
            self.decor(Aabb::span(Vec3::new(x0 - out, y0, z0), Vec3::new(x0, y1, z1)), paint);
        }
        if !skip[2] {
            self.decor(Aabb::span(Vec3::new(x1, y0, z0), Vec3::new(x1 + out, y1, z1)), paint);
        }
    }

    /// Is anything solid within `margin` of this point?
    /// Takes away any car, barrel or crate that ended up inside a wall, hedge or fence
    /// (they are placed by footprint radius, which a long car can outreach).
    fn tidy_clutter(&mut self) {
        let solids: Vec<Aabb> = self.solids().collect();
        self.clutter.retain(|item| {
            let (centre, half) = (item.center(), item.half());
            !solids.iter().any(|b| b.top() > item.pos.y + 0.45 && (0..3).all(|a| (b.center[a] - centre[a]).abs() < b.half[a] + half[a] - 0.05))
        });
    }

    fn occupied(&self, p: Vec2, margin: f32) -> bool {
        self.solids().any(|b| b.touches(p.x, p.y, margin))
    }

    /// A wall along one axis, with openings cut into it.
    /// `fixed` is the wall's extent across its thickness; openings are (start, end, bottom, top) along the wall.
    #[allow(clippy::too_many_arguments)]
    fn wall(&mut self, along_x: bool, fixed: (f32, f32), start: f32, length: f32, base: f32, height: f32, openings: &[(f32, f32, f32, f32)], paint: Paint) {
        let mut pieces: Vec<(f32, f32, f32, f32)> = Vec::new();
        let mut cursor = 0.0;
        let mut sorted = openings.to_vec();
        sorted.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (u0, u1, y0, y1) in sorted {
            pieces.push((cursor, u0, 0.0, height));
            if y0 > 0.0 {
                pieces.push((u0, u1, 0.0, y0));
            }
            if y1 < height {
                pieces.push((u0, u1, y1, height));
            }
            cursor = u1;
        }
        pieces.push((cursor, length, 0.0, height));
        for (u0, u1, y0, y1) in pieces {
            if u1 - u0 < 0.01 || y1 - y0 < 0.01 {
                continue;
            }
            let (a, b) = (start + u0, start + u1);
            let aabb =
                if along_x { Aabb::span(Vec3::new(a, base + y0, fixed.0), Vec3::new(b, base + y1, fixed.1)) } else { Aabb::span(Vec3::new(fixed.0, base + y0, a), Vec3::new(fixed.1, base + y1, b)) };
            self.solid(aabb, paint);
        }
        // Remember every opening: doors get doors, windows get glass. Openings wider than a
        // doorway (garage mouths, barn doors, archways) stay open.
        for (u0, u1, y0, y1) in openings.iter().copied() {
            let mid = start + (u0 + u1) * 0.5;
            let across = (fixed.0 + fixed.1) * 0.5;
            let center = if along_x { Vec3::new(mid, base + y0, across) } else { Vec3::new(across, base + y0, mid) };
            if y0 <= 0.0 {
                if u1 - u0 <= DOOR_WIDTH + 0.05 {
                    self.doors.push(DoorSpec { center, along_x, width: u1 - u0, height: y1 - y0 });
                }
            } else {
                self.windows.push(WindowSpec { center, along_x, width: u1 - u0, height: y1 - y0 });
            }
        }
        // Sills under windows read better with a dark ledge, and a lintel over each opening.
        for (u0, u1, _, y1) in openings.iter().copied().filter(|o| o.3 < height - 0.13) {
            let (a, b) = (start + u0 - 0.08, start + u1 + 0.08);
            let lintel = if along_x {
                Aabb::span(Vec3::new(a, base + y1, fixed.0 - 0.05), Vec3::new(b, base + y1 + 0.12, fixed.1 + 0.05))
            } else {
                Aabb::span(Vec3::new(fixed.0 - 0.05, base + y1, a), Vec3::new(fixed.1 + 0.05, base + y1 + 0.12, b))
            };
            self.decor(lintel, Paint::Dark);
        }
        for (u0, u1, y0, _) in openings.iter().copied().filter(|o| o.2 > 0.0) {
            let (a, b) = (start + u0, start + u1);
            let ledge = if along_x {
                Aabb::span(Vec3::new(a, base + y0 - 0.06, fixed.0 - 0.06), Vec3::new(b, base + y0, fixed.1 + 0.06))
            } else {
                Aabb::span(Vec3::new(fixed.0 - 0.06, base + y0 - 0.06, a), Vec3::new(fixed.1 + 0.06, base + y0, b))
            };
            self.decor(ledge, Paint::Dark);
        }
    }

    /// Openings for one side of one storey: maybe a door, and windows wherever they fit.
    fn openings(rng: &mut Rng, length: f32, door: bool, windows: bool) -> Vec<(f32, f32, f32, f32)> {
        let door = door.then(|| (rng.range(1.6, length - 1.6), DOOR_WIDTH));
        Self::openings_at(rng, length, door, windows)
    }

    /// Openings for one side of one storey: a doorway at `door` (centre along the wall, width;
    /// wider than [`DOOR_WIDTH`] is an open gateway), and windows wherever they fit around it.
    pub(crate) fn openings_at(rng: &mut Rng, length: f32, door: Option<(f32, f32)>, windows: bool) -> Vec<(f32, f32, f32, f32)> {
        let mut out = Vec::new();
        let mut taken: Vec<(f32, f32)> = Vec::new();
        if let Some((center, width)) = door {
            let lo = width * 0.5 + 0.35;
            let center = center.clamp(lo, (length - lo).max(lo));
            let top = if width > DOOR_WIDTH + 0.05 { 2.9 } else { DOOR_HEIGHT };
            out.push((center - width * 0.5, center + width * 0.5, 0.0, top));
            taken.push((center - width * 0.5 - 0.45, center + width * 0.5 + 0.45));
        }
        if windows {
            let mut u = 1.0;
            while u + 1.2 < length - 0.8 {
                let clear = taken.iter().all(|(a, b)| u + 1.2 < *a || u > *b);
                // Sills at crouch-cover height: duck below one and you are hidden, jump and you go through.
                if clear && rng.chance(0.75) {
                    out.push((u, u + 1.25, WINDOW_SILL, 2.3));
                }
                u += 2.3;
            }
        }
        out
    }

    /// One building in this map's style, with its doors and roof left to chance.
    fn building(&mut self, rng: &mut Rng, c: Vec2, half: Vec2, floors: u32) {
        self.building_plan(rng, c, half, floors, &Plan::default());
    }

    /// One building: foundation, walls with doors and windows, storeys joined by switchback
    /// stairs, interior rooms and furniture, a flat roof with parapets or a stepped pitched one,
    /// detailing, and sometimes a ladder up to the roof. `plan` fixes what is not left to chance
    /// (paint, roof, doors, sides left open to join a neighbouring wing, a balcony door).
    /// Returns (floor level, roof level).
    pub(crate) fn building_plan(&mut self, rng: &mut Rng, c: Vec2, half: Vec2, floors: u32, plan: &Plan) -> (f32, f32) {
        const T: f32 = 0.3;
        const STEPS: usize = 11;
        const RUN: f32 = 0.42;
        const LANE: f32 = 1.2;
        let (hx, hz) = (half.x, half.y);
        let (x0, x1, z0, z1) = (c.x - hx, c.x + hx, c.y - hz, c.y + hz);
        let g = plan.floor.unwrap_or_else(|| {
            let mut g = f32::MIN;
            for (x, z) in [(x0, z0), (x1, z0), (x0, z1), (x1, z1), (c.x, c.y)] {
                g = g.max(self.height(x, z));
            }
            g + 0.12
        });
        let paint = plan.walls.unwrap_or_else(|| *rng.pick(&self.style.walls).unwrap_or(&Paint::Concrete));
        let trim = plan.trim.unwrap_or(if matches!(paint, Paint::Bone | Paint::Sand | Paint::Concrete | Paint::White) { Paint::Steel } else { Paint::Bone });
        let roof = match plan.roof {
            Roof::Auto => self.style.roof(rng),
            roof => roof,
        };
        let open = plan.open;
        let roof_y = g + floors as f32 * STOREY;

        // Foundation and floor.
        self.solid(Aabb::span(Vec3::new(x0, g - 2.5, z0), Vec3::new(x1, g, z1)), Paint::Concrete);
        self.decor(Aabb::span(Vec3::new(x0 + T, g, z0 + T), Vec3::new(x1 - T, g + 0.012, z1 - T)), plan.floor_paint);

        // Doors go on up to two sides; buildings with stairs keep the stair walls (north, and south above two storeys) solid.
        // Sides: 0 south (-z), 1 west (-x), 2 east (+x), 3 north (+z). A door is (centre along the wall, width).
        let mut doors = plan.doors;
        if plan.random_doors {
            for roll in 0..2 {
                if roll == 1 && !rng.chance(0.6) {
                    break;
                }
                let side = rng.below(if floors > 1 { 3 } else { 4 });
                if doors[side].is_none() && !open[side] {
                    let center = if side == 0 || side == 3 { rng.range(x0 + 1.6, x1 - 1.6) } else { rng.range(z0 + T + 1.6, z1 - T - 1.6) };
                    doors[side] = Some((center, DOOR_WIDTH));
                }
            }
        }
        // Where each side's wall starts along its length.
        let start = [x0, z0 + T, z0 + T, x0];
        // A door too near a corner slides along its wall (as `openings_at` would slide it),
        // here, so the steps up to it are built in front of it and not of blank wall.
        let length = [2.0 * hx, 2.0 * hz - 2.0 * T, 2.0 * hz - 2.0 * T, 2.0 * hx];
        for (side, door) in doors.iter_mut().enumerate() {
            if let Some((center, width)) = door {
                let lo = *width * 0.5 + 0.35;
                *center = (*center - start[side]).clamp(lo, (length[side] - lo).max(lo)) + start[side];
            }
        }
        for floor in 0..floors {
            let base = g + floor as f32 * STOREY;
            let ground = floor == 0;
            let door_for = |side: usize| -> Option<(f32, f32)> {
                if ground {
                    doors[side].map(|(center, width)| (center - start[side], width))
                } else if plan.balcony && floor == 1 && side == 0 {
                    Some((c.x - x0, DOOR_WIDTH))
                } else {
                    None
                }
            };
            // South (-z) and north (+z) walls run along X and own the corners.
            if !open[0] {
                let south = Self::openings_at(rng, 2.0 * hx, door_for(0), plan.windows);
                self.wall(true, (z0, z0 + T), x0, 2.0 * hx, base, STOREY, &south, paint);
            }
            if !open[3] {
                let north = Self::openings_at(rng, 2.0 * hx, door_for(3), plan.windows && (floor > 0 || floors == 1));
                self.wall(true, (z1 - T, z1), x0, 2.0 * hx, base, STOREY, &north, paint);
            }
            if !open[1] {
                let west = Self::openings_at(rng, 2.0 * hz - 2.0 * T, door_for(1), plan.windows);
                self.wall(false, (x0, x0 + T), z0 + T, 2.0 * hz - 2.0 * T, base, STOREY, &west, paint);
            }
            if !open[2] {
                let east = Self::openings_at(rng, 2.0 * hz - 2.0 * T, door_for(2), plan.windows);
                self.wall(false, (x1 - T, x1), z0 + T, 2.0 * hz - 2.0 * T, base, STOREY, &east, paint);
            }
            // A trim band at each floor line, on the outside of the walls only.
            self.ring(x0, x1, z0, z1, base + STOREY - 0.14, base + STOREY, 0.05, Paint::Dark, open);
            // Ceiling lamp, flush with the ceiling (the roof slab's underside, or the next floor's).
            let ceiling = if floor + 1 == floors { STOREY } else { STOREY - 0.25 };
            self.decor(Aabb::new(Vec3::new(c.x, base + ceiling - 0.03, c.y - hz * 0.3), Vec3::new(0.35, 0.03, 0.35)), Paint::Glow);
        }
        // Steps up to any door that stands above the ground outside it.
        for (side, door) in doors.iter().enumerate() {
            let Some((center, width)) = *door else { continue };
            let (p, out) = match side {
                0 => (Vec2::new(center, z0), Vec2::NEG_Y),
                1 => (Vec2::new(x0, center), Vec2::NEG_X),
                2 => (Vec2::new(x1, center), Vec2::X),
                _ => (Vec2::new(center, z1), Vec2::Y),
            };
            self.stoop(p, out, width + 0.4, g);
            self.thresholds.push((Vec3::new(p.x, g, p.y), open[side]));
        }

        // Stairs: flight k climbs from floor k to k+1. Flights alternate between a lane along the
        // north wall (climbing east) and one along the south wall (climbing west), so every floor
        // has a solid landing beside the hole the flight below comes up through.
        let rise = STOREY / STEPS as f32;
        let span = STEPS as f32 * RUN;
        let lane = |k: u32| -> (f32, f32, f32, f32) {
            if k.is_multiple_of(2) {
                let sx = x0 + T + 0.4;
                (sx, sx + span, z1 - T - LANE, z1 - T)
            } else {
                let sx = x1 - T - 0.4;
                (sx - span, sx, z0 + T, z0 + T + LANE)
            }
        };
        for k in 0..floors.saturating_sub(1) {
            let base = g + k as f32 * STOREY;
            let (lx0, lx1, lz0, lz1) = lane(k);
            for i in 0..STEPS {
                let top = base + (i + 1) as f32 * rise;
                let (a, b) = if k % 2 == 0 { (lx0 + i as f32 * RUN, lx0 + (i + 1) as f32 * RUN) } else { (lx1 - (i + 1) as f32 * RUN, lx1 - i as f32 * RUN) };
                let step = Aabb::span(Vec3::new(a, base, lz0), Vec3::new(b, top, lz1));
                self.solid(step, if i % 2 == 0 { Paint::Wood } else { Paint::Steel });
            }
        }
        // Floor slabs, each with a hole above the flight that arrives through it.
        for k in 1..floors {
            let top = g + k as f32 * STOREY;
            let (hx0, hx1, hz0, hz1) = lane(k - 1);
            // The hole runs the length of the flight, one step short so the top step meets solid floor.
            let (hx0, hx1) = if (k - 1) % 2 == 0 { (hx0, hx1 - RUN * 0.5) } else { (hx0 + RUN * 0.5, hx1) };
            let pieces = [(x0, x1, z0, hz0 - 0.05), (x0, x1, hz1 + 0.05, z1), (x0, hx0 - 0.05, hz0 - 0.05, hz1 + 0.05), (hx1 + 0.05, x1, hz0 - 0.05, hz1 + 0.05)];
            for (a0, a1, b0, b1) in pieces {
                if a1 - a0 > 0.05 && b1 - b0 > 0.05 {
                    self.solid(Aabb::span(Vec3::new(a0, top - 0.25, b0), Vec3::new(a1, top, b1)), Paint::Concrete);
                    self.decor(Aabb::span(Vec3::new(a0.max(x0 + T), top, b0.max(z0 + T)), Vec3::new(a1.min(x1 - T), top + 0.012, b1.min(z1 - T))), plan.floor_paint);
                }
            }
            // Railing along the open side of the hole; the high end is left open as the way in.
            let rail_z = if (k - 1) % 2 == 0 { hz0 - 0.08 } else { hz1 + 0.08 };
            let (r0, r1) = if (k - 1) % 2 == 0 { (hx0, hx1 - 1.0) } else { (hx0 + 1.0, hx1) };
            self.solid(Aabb::span(Vec3::new(r0, top, rail_z - 0.04), Vec3::new(r1, top + 0.95, rail_z + 0.04)), Paint::Dark);
            self.decor(Aabb::span(Vec3::new(r0, top + 0.92, rail_z - 0.06), Vec3::new(r1, top + 1.0, rail_z + 0.06)), Paint::Gold);
            // Something to hide behind up here.
            if plan.furnish != Furnish::Bare {
                let p = Vec2::new(rng.range(x0 + 1.5, x1 - 1.5), c.y + rng.range(-0.8, 0.8));
                if plan.furnish == Furnish::Home && hz > 3.2 {
                    // A bed (low cover) and a wardrobe against the wall.
                    self.bed(Vec2::new(p.x, c.y), rng.chance(0.5), top);
                } else {
                    self.solid(Aabb::span(Vec3::new(p.x - 0.55, top, p.y - 0.55), Vec3::new(p.x + 0.55, top + 1.0, p.y + 0.55)), Paint::Teal);
                }
            }
        }

        // Ground-floor rooms: a partition with a doorway, clear of the stairs.
        let partition = x0 + T + 0.4 + span + 1.3;
        if floors > 1 && x1 - partition > 2.6 && plan.furnish != Furnish::Bare {
            let length = 2.0 * hz - 2.0 * T - LANE - 0.1;
            let door = Self::openings(rng, length, true, false);
            self.wall(false, (partition, partition + 0.2), z0 + T, length, g, STOREY - 0.25, &door, trim);
        }

        // Furniture on the ground floor, away from the doors and stairs.
        let room = (x0 + 1.6, x1 - 1.6, z0 + 1.6, z1 - 1.6 - if floors > 1 { 1.4 } else { 0.0 });
        match plan.furnish {
            Furnish::Bare => {}
            Furnish::Home => self.furnish_home(rng, room, g, (partition, floors > 1)),
            Furnish::Barn => self.furnish_barn(rng, room, g),
            Furnish::Plain => {
                for _ in 0..2 {
                    let p = Vec2::new(rng.range(room.0, room.1), rng.range(room.2, room.3));
                    if (p.x - partition).abs() < 1.2 && floors > 1 {
                        continue;
                    }
                    let size = if rng.chance(0.5) { Vec2::new(rng.range(0.7, 1.4), 0.4) } else { Vec2::new(0.5, 0.5) };
                    self.solid(Aabb::span(Vec3::new(p.x - size.x, g, p.y - size.y), Vec3::new(p.x + size.x, g + 0.95, p.y + size.y)), Paint::Wood);
                }
            }
        }

        // Detailing: a dark plinth, corner pilasters, a drainpipe, the odd air-conditioning unit.
        let o = 0.06;
        self.ring(x0, x1, z0, z1, g, g + 0.4, o, Paint::Dark, open);
        // Corner pilasters: solid, and set into the corner so they stand only a few centimetres
        // proud - a robot in cover slides along the wall right up to the corner and peeks round it.
        for (x, z, a, b) in [(x0 + 0.14, z0 + 0.14, 0, 1), (x1 - 0.14, z0 + 0.14, 0, 2), (x0 + 0.14, z1 - 0.14, 3, 1), (x1 - 0.14, z1 - 0.14, 3, 2)] {
            if !open[a] && !open[b] {
                self.solid(Aabb::new(Vec3::new(x, g + (roof_y - g) * 0.5, z), Vec3::new(0.2, (roof_y - g) * 0.5, 0.2)), trim);
            }
        }
        // The drainpipe is too thin to hide behind: walk-through, like every pole and wire.
        if !open[2] {
            self.decor(Aabb::span(Vec3::new(x1 + 0.02, g, z1 - 0.6), Vec3::new(x1 + 0.14, roof_y, z1 - 0.48)), Paint::Steel);
        }
        for floor in 0..floors {
            if rng.chance(0.35) && !open[1] && plan.windows {
                let y = g + floor as f32 * STOREY + 2.4;
                let z = rng.range(z0 + 1.0, z1 - 1.0);
                self.solid(Aabb::span(Vec3::new(x0 - 0.45, y, z - 0.35), Vec3::new(x0, y + 0.45, z + 0.35)), Paint::Metal);
                self.decor(Aabb::span(Vec3::new(x0 - 0.47, y + 0.1, z - 0.2), Vec3::new(x0 - 0.45, y + 0.35, z + 0.2)), Paint::Dark);
            }
        }

        // Roof.
        self.solid(Aabb::span(Vec3::new(x0 - 0.15, roof_y, z0 - 0.15), Vec3::new(x1 + 0.15, roof_y + 0.3, z1 + 0.15)), if plan.trim == Some(paint) { shapes::darker(paint) } else { Paint::Dark });
        match roof {
            Roof::Gable(tiles) | Roof::Hip(tiles) => {
                let along_x = hx >= hz;
                let ridge = self.stepped_roof((x0 - 0.15, x1 + 0.15, z0 - 0.15, z1 + 0.15), roof_y + 0.3, matches!(roof, Roof::Hip(_)), along_x, (tiles, paint));
                // A chimney: solid, it is cover up there.
                if plan.chimney && rng.chance(0.7) {
                    let (x, z) = if along_x { (rng.range(x0 + 1.2, x1 - 1.2), c.y + hz * 0.35) } else { (c.x + hx * 0.35, rng.range(z0 + 1.2, z1 - 1.2)) };
                    self.solid(Aabb::span(Vec3::new(x - 0.35, roof_y + 0.3, z - 0.35), Vec3::new(x + 0.35, ridge + 0.7, z + 0.35)), Paint::Rust);
                    self.decor(Aabb::span(Vec3::new(x - 0.4, ridge + 0.7, z - 0.4), Vec3::new(x + 0.4, ridge + 0.78, z + 0.4)), Paint::Dark);
                }
            }
            _ => {
                // Parapets along two sides: a ledge to crouch behind up there (the ladders come up the other sides).
                self.solid(Aabb::span(Vec3::new(x0 - 0.15, roof_y + 0.3, z0 - 0.15), Vec3::new(x1 + 0.15, roof_y + 0.3 + COVER_HEIGHT, z0 + 0.1)), paint);
                self.solid(Aabb::span(Vec3::new(x0 - 0.15, roof_y + 0.3, z1 - 0.1), Vec3::new(x1 + 0.15, roof_y + 0.3 + COVER_HEIGHT, z1 + 0.15)), paint);
                let unit = Vec2::new(rng.range(x0 + 1.0, x1 - 1.0), rng.range(z0 + 1.0, z1 - 1.0));
                // The rule for scenery: anything you could hide behind is solid (this air-conditioning
                // unit is rooftop cover); anything too thin to hide behind - masts, poles, wires - you walk through.
                self.solid(Aabb::new(Vec3::new(unit.x, roof_y + 0.7, unit.y), Vec3::new(0.6, 0.4, 0.45)), Paint::Metal);
                if rng.chance(0.4) {
                    self.decor(Aabb::new(Vec3::new(c.x + hx * 0.5, roof_y + 1.6, c.y), Vec3::new(0.04, 1.3, 0.04)), Paint::Metal);
                    self.decor(Aabb::new(Vec3::new(c.x + hx * 0.5, roof_y + 2.95, c.y), Vec3::new(0.08, 0.08, 0.08)), Paint::Red);
                } else if rng.chance(0.5) {
                    // A flag on a pole; it waves in the wind.
                    self.decor(Aabb::new(Vec3::new(c.x - hx * 0.5, roof_y + 2.0, c.y), Vec3::new(0.05, 1.7, 0.05)), Paint::Metal);
                    self.flags.push(Vec3::new(c.x - hx * 0.5, roof_y + 3.55, c.y));
                }
            }
        }

        // Balcony over the front door, reached through the upstairs door.
        if plan.balcony && floors > 1 {
            self.balcony(Vec2::new(c.x, z0), g + STOREY, trim);
        }

        // A ladder up a side wall without a door, onto the roof.
        let free = |side: usize| doors[side].is_none() && !open[side];
        if plan.ladder && rng.chance(0.55) && (free(1) || free(2)) {
            let east = free(2) && (!free(1) || rng.chance(0.5));
            let (x, out) = if east { (x1 + 0.22, 1.0) } else { (x0 - 0.22, -1.0) };
            // Beside the south corner, where walls never have windows.
            let z = z0 + 0.75;
            let foot = Vec3::new(x, self.height(x, z), z);
            for side in [-0.3, 0.3] {
                self.decor(Aabb::span(Vec3::new(x - 0.04, foot.y, z + side - 0.04), Vec3::new(x + 0.04, roof_y + 1.0, z + side + 0.04)), Paint::Iron);
            }
            let mut y = foot.y + 0.3;
            while y < roof_y + 0.6 {
                self.decor(Aabb::new(Vec3::new(x, y, z), Vec3::new(0.03, 0.03, 0.3)), Paint::Iron);
                y += 0.35;
            }
            self.ladders.push(Ladder { foot, top: roof_y + 0.3, out: Vec2::new(out, 0.0) });
        }
        (g, roof_y)
    }

    /// The town square: a tiered fountain (good cover) ringed by benches' worth of lamps and trees.
    fn plaza(&mut self, rng: &mut Rng) {
        self.plaza_at(rng, Vec2::ZERO);
    }

    /// A fountain square centred on `at`.
    pub(crate) fn plaza_at(&mut self, rng: &mut Rng, at: Vec2) {
        let g = self.height(at.x, at.y);
        let o = Vec3::new(at.x, 0.0, at.y);
        // Basin: four low walls with water inside, a pillar and a raised bowl.
        let (r, wall) = (3.2, 0.35);
        for (cx, cz, hx, hz) in [(0.0, r, r, wall), (0.0, -r, r, wall), (r, 0.0, wall, r - wall), (-r, 0.0, wall, r - wall)] {
            self.solid(Aabb::new(o + Vec3::new(cx, g + 0.35, cz), Vec3::new(hx, 0.45, hz)), Paint::Concrete);
            self.decor(Aabb::new(o + Vec3::new(cx, g + 0.83, cz), Vec3::new(hx + 0.05, 0.04, hz + 0.05)), Paint::Bone);
        }
        self.decor(Aabb::new(o + Vec3::new(0.0, g + 0.45, 0.0), Vec3::new(r - wall, 0.05, r - wall)), Paint::Blue);
        self.solid(Aabb::new(o + Vec3::new(0.0, g + 1.0, 0.0), Vec3::new(0.45, 1.0, 0.45)), Paint::Bone);
        self.solid(Aabb::new(o + Vec3::new(0.0, g + 2.05, 0.0), Vec3::new(1.2, 0.08, 1.2)), Paint::Concrete);
        self.decor(Aabb::new(o + Vec3::new(0.0, g + 2.12, 0.0), Vec3::new(1.0, 0.03, 1.0)), Paint::Blue);
        self.solid(Aabb::new(o + Vec3::new(0.0, g + 2.5, 0.0), Vec3::new(0.12, 0.4, 0.12)), Paint::Bone);
        self.fountain = Some(o + Vec3::new(0.0, g + 2.95, 0.0));
        // Lamps around the square.
        for i in 0..6 {
            let a = i as f32 * TAU / 6.0 + 0.3;
            let p = at + Vec2::from_angle(a) * 8.0;
            self.props.push(Prop { kind: PropKind::Lamp, pos: Vec3::new(p.x, self.height(p.x, p.y), p.y), yaw: a, scale: 1.0 });
        }
        let _ = rng;
    }

    /// Bins, barrels, benches, mailboxes, crate stacks and parked cars along the streets.
    fn street_clutter(&mut self, rng: &mut Rng) {
        let kinds = [(ClutterKind::Bin, 14), (ClutterKind::Barrel, 10), (ClutterKind::Bench, 8), (ClutterKind::Mailbox, 6), (ClutterKind::Crates, 8), (ClutterKind::Car, 6)];
        for (kind, count) in kinds {
            let mut placed = 0;
            for _ in 0..count * 12 {
                if placed >= count {
                    break;
                }
                // Streets run between the lots on the 21 m grid.
                let along = rng.range(-55.0, 55.0);
                let street = (rng.below(5) as f32 - 2.0) * 21.0 + 10.5 * if rng.chance(0.5) { 1.0 } else { -1.0 };
                let across = rng.range(-2.6, 2.6);
                let on_x = rng.chance(0.5);
                let p = if on_x { Vec2::new(along, street + across) } else { Vec2::new(street + across, along) };
                let turned = !on_x;
                let half = kind.half();
                let reach = half.x.max(half.z) + 0.8;
                if p.x.abs() > 54.0 || p.length() < 6.0 || self.occupied(p, reach) || self.clutter.iter().any(|c| Vec2::new(c.pos.x, c.pos.z).distance(p) < reach + 1.5) {
                    continue;
                }
                self.clutter.push(Clutter { kind, pos: Vec3::new(p.x, self.height(p.x, p.y), p.y), turned });
                placed += 1;
            }
        }
    }

    /// Trees along the streets, bushes by the walls, and grass wherever nothing is built.
    fn greenery(&mut self, rng: &mut Rng) {
        let look = self.recipe.biome.look();
        let lush = look.grass[1] > look.grass[0] && look.grass[1] > look.grass[2] * 0.9;
        // Street trees on the lot corners, where they never block a doorway.
        for gx in -2i32..=2 {
            for gz in -2i32..=2 {
                if !rng.chance(0.45) {
                    continue;
                }
                let p = Vec2::new(gx as f32 * 21.0 + 10.5, gz as f32 * 21.0 + 10.5) + Vec2::new(rng.range(-1.0, 1.0), rng.range(-1.0, 1.0));
                if p.length() < 7.0 || self.occupied(p, 1.2) || p.x.abs() > 52.0 {
                    continue;
                }
                self.props.push(Prop { kind: PropKind::Tree, pos: Vec3::new(p.x, self.height(p.x, p.y), p.y), yaw: rng.range(0.0, TAU), scale: rng.range(3.6, 5.2) });
            }
        }
        for _ in 0..40 {
            let p = Vec2::new(rng.range(-60.0, 60.0), rng.range(-60.0, 60.0));
            if self.occupied(p, 0.9) || p.length() < 5.0 {
                continue;
            }
            self.props.push(Prop { kind: PropKind::Bush, pos: Vec3::new(p.x, self.height(p.x, p.y), p.y), yaw: rng.range(0.0, TAU), scale: rng.range(0.7, 1.2) });
        }
        // Grass tufts: dense where it is green, sparse elsewhere.
        let tufts = if lush { 2600 } else { 900 };
        for _ in 0..tufts {
            let p = Vec2::new(rng.range(-70.0, 70.0), rng.range(-70.0, 70.0));
            let h = self.height(p.x, p.y);
            if h < self.water + 0.3 || self.occupied(p, 0.3) {
                continue;
            }
            self.grass.push(Vec3::new(p.x, h, p.y));
        }
    }

    /// An open lot with low walls and planters: cover without a roof.
    fn courtyard(&mut self, rng: &mut Rng, c: Vec2) {
        let g = self.height(c.x, c.y);
        for (dx, dz, long_x) in [(-6.0, 0.0, false), (6.0, 0.0, false), (0.0, -6.0, true), (0.0, 6.0, true)] {
            if rng.chance(0.75) {
                let half = if long_x { Vec3::new(3.0, COVER_HEIGHT * 0.5, 0.25) } else { Vec3::new(0.25, COVER_HEIGHT * 0.5, 3.0) };
                self.solid(Aabb::new(Vec3::new(c.x + dx, g + COVER_HEIGHT * 0.5, c.y + dz), half), Paint::Concrete);
            }
        }
        self.solid(Aabb::new(Vec3::new(c.x, g + 0.4, c.y), Vec3::new(1.2, 0.5, 1.2)), Paint::Concrete);
        // Water in the planter: a skin on top, nothing to sink into.
        self.decor(Aabb::new(Vec3::new(c.x, g + 0.915, c.y), Vec3::new(0.9, 0.02, 0.9)), Paint::Blue);
    }

    fn place_deposit(&mut self, rng: &mut Rng, island: Island, band: (f32, f32), ore: Ore) {
        for _ in 0..40 {
            let p = island.center + Vec2::from_angle(rng.range(0.0, TAU)) * rng.range(band.0, band.1);
            let y = self.height(p.x, p.y);
            let spaced = self.deposits.iter().all(|d| Vec2::new(d.pos.x, d.pos.z).distance(p) > 7.0);
            if y > 0.45 && spaced {
                let amount = match ore {
                    Ore::Iron => 420.0,
                    Ore::Copper => 320.0,
                    Ore::Aether => 200.0,
                } * rng.range(0.8, 1.3);
                self.deposits.push(Deposit { pos: Vec3::new(p.x, y, p.y), ore, amount });
                return;
            }
        }
    }

    /// Number of terrain chunks along one axis for a given chunk size (in cells).
    pub fn chunks_per_side(&self, chunk_cells: usize) -> usize {
        (self.side - 1).div_ceil(chunk_cells)
    }

    /// Flat-shaded, vertex-coloured triangles for one chunk of terrain.
    pub fn chunk(&self, cx: usize, cz: usize, chunk_cells: usize) -> TerrainChunk {
        let (sand, grass, high) = self.ground_colors();
        // Where the bare rock starts: on the mountain the terraces stay green.
        let (low, top) = if self.recipe.layout == Layout::Mountainside { (0.74, 34.0) } else { (0.74, 3.3) };
        let mut out = TerrainChunk::default();
        let mut rng = Rng::new(self.recipe.seed ^ ((cx as u64) << 20) ^ cz as u64);
        let (cell, half) = (self.recipe.cell, self.recipe.half_extent);
        for iz in cz * chunk_cells..((cz + 1) * chunk_cells).min(self.side - 1) {
            for ix in cx * chunk_cells..((cx + 1) * chunk_cells).min(self.side - 1) {
                let p = |dx: usize, dz: usize| Vec3::new((ix + dx) as f32 * cell - half, self.heights[(iz + dz) * self.side + ix + dx], (iz + dz) as f32 * cell - half);
                let (a, b, c, d) = (p(0, 0), p(1, 0), p(0, 1), p(1, 1));
                for [a, b, c] in [[a, c, b], [b, c, d]] {
                    let normal = (b - a).cross(c - a).normalize();
                    let y = (a.y + b.y + c.y) / 3.0;
                    let base = if y < low {
                        sand
                    } else if y > top || normal.y < 0.82 {
                        high
                    } else {
                        grass
                    };
                    let shade = rng.range(0.88, 1.10) * if y < self.water { 0.72 } else { 1.0 };
                    let linear = base.map(|ch| srgb_to_linear((ch * shade).clamp(0.0, 1.0)));
                    for point in [a, b, c] {
                        out.positions.push(point.to_array());
                        out.normals.push(normal.to_array());
                        out.colors.push([linear[0], linear[1], linear[2], 1.0]);
                    }
                }
            }
        }
        out
    }
}

pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// RGBA8 sRGB pixels for a skybox cubemap, faces in +X, -X, +Y, -Y, +Z, -Z order.
pub fn sky_cubemap(look: &BiomeLook, night: bool, size: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((size * size * 6 * 4) as usize);
    let sun = Vec3::new(-0.54, 0.21, 0.81).normalize();
    let (horizon, zenith) = if night { (Vec3::new(0.10, 0.13, 0.22), Vec3::new(0.012, 0.018, 0.05)) } else { (Vec3::from(look.horizon), Vec3::from(look.zenith)) };
    let sea = if night { Vec3::new(0.03, 0.06, 0.10) } else { Vec3::from(look.water) * 1.25 };
    for face in 0..6 {
        for y in 0..size {
            for x in 0..size {
                let u = 2.0 * (x as f32 + 0.5) / size as f32 - 1.0;
                let v = 2.0 * (y as f32 + 0.5) / size as f32 - 1.0;
                let d = match face {
                    0 => Vec3::new(1.0, -v, -u),
                    1 => Vec3::new(-1.0, -v, u),
                    2 => Vec3::new(u, 1.0, v),
                    3 => Vec3::new(u, -1.0, -v),
                    4 => Vec3::new(u, -v, 1.0),
                    _ => Vec3::new(-u, -v, -1.0),
                }
                .normalize();
                let elev = d.y.max(0.0);
                let mut rgb = horizon.lerp(zenith, elev.powf(0.52));
                if d.y < 0.0 {
                    rgb = rgb.lerp(sea, (-d.y * 3.0).min(1.0));
                }
                let angle = d.dot(sun);
                if night {
                    // Stars: hash the direction onto a coarse lattice.
                    let cell = (d * 90.0).floor();
                    let h = noise::value(0x57a2, cell.x * 12.9898 + cell.z * 4.1414, cell.y * 78.233);
                    if d.y > 0.02 && h > 0.965 {
                        rgb = rgb.lerp(Vec3::new(0.85, 0.92, 1.0), ((h - 0.965) * 28.0).min(1.0));
                    }
                    if angle > 0.9975 {
                        rgb = Vec3::new(0.82, 0.88, 0.96);
                    } else {
                        rgb = rgb.lerp(Vec3::new(0.35, 0.45, 0.62), ((angle - 0.93) / 0.07).clamp(0.0, 1.0).powi(3) * 0.4);
                    }
                } else {
                    rgb = rgb.lerp(Vec3::new(1.0, 0.79, 0.49), ((angle - 0.82) / 0.18).clamp(0.0, 1.0).powi(3) * 0.65);
                    let streak = (d.x * 17.0 + d.z * 7.0).sin() + (d.x * 32.0 - d.z * 15.0).sin() * 0.45 + (d.y * 75.0).sin() * 0.9;
                    if elev > 0.12 && elev < 0.46 && streak > 1.38 {
                        rgb = rgb.lerp(Vec3::new(1.0, 0.79, 0.65), 0.56);
                    }
                    if angle > 0.9962 {
                        rgb = Vec3::new(1.0, 0.94, 0.72);
                    }
                }
                pixels.extend(rgb.to_array().map(|c| (c.clamp(0.0, 1.0) * 255.0) as u8));
                pixels.push(255);
            }
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> World {
        World::generate(WorldRecipe::archipelago(0x45eab1, Biome::EmberAtoll))
    }

    #[test]
    fn base_is_dry_and_edges_are_sea() {
        let w = world();
        assert!(w.height(w.spawn.x, w.spawn.y) > w.water + 0.5);
        assert!(w.height(w.core.x, w.core.y) > w.water + 0.5);
        assert!(w.height(150.0, 150.0) < w.water);
        assert!(w.walkable(w.spawn.x, w.spawn.y) && !w.walkable(150.0, 150.0));
    }

    #[test]
    fn generation_is_deterministic() {
        let (a, b) = (world(), world());
        assert_eq!(a.heights, b.heights);
        assert_eq!(a.deposits.len(), b.deposits.len());
        assert!(a.deposits.len() >= 20, "expected plenty of ore, got {}", a.deposits.len());
        assert!(a.deposits.iter().any(|d| d.ore == Ore::Aether));
    }

    #[test]
    fn sandbars_connect_every_islet() {
        let w = world();
        for island in &w.islands[1..] {
            let steps = 200;
            for i in 0..=steps {
                let p = island.center * (i as f32 / steps as f32);
                assert!(w.walkable(p.x, p.y), "gap in sandbar at {p:?} (h={})", w.height(p.x, p.y));
            }
        }
    }

    #[test]
    fn landings_are_offshore() {
        let w = world();
        assert_eq!(w.landings.len(), 20);
        for l in &w.landings {
            assert!(w.height(l.x, l.y) < w.water, "landing {l:?} is on land");
        }
    }

    /// Nothing solid in a robot's body standing at a spawn point (ceilings far overhead and
    /// floors underfoot are fine).
    /// Pairs of drawn blocks with faces in one plane, facing the same way, overlapping, in
    /// different paints: they would flicker against each other (z-fighting).
    fn flickering(w: &World, drawn: impl Fn(&Block) -> Aabb) -> Vec<(Block, Block)> {
        let blocks: Vec<(Block, Aabb)> = w.blocks.iter().filter(|b| !b.hidden).map(|b| (*b, drawn(b))).collect();
        let mut cells: std::collections::HashMap<(i32, i32), Vec<usize>> = Default::default();
        let cell = |v: f32| (v / 8.0).floor() as i32;
        for (i, (_, b)) in blocks.iter().enumerate() {
            let (lo, hi) = (b.center - b.half, b.center + b.half);
            for cx in cell(lo.x)..=cell(hi.x) {
                for cz in cell(lo.z)..=cell(hi.z) {
                    cells.entry((cx, cz)).or_default().push(i);
                }
            }
        }
        let mut seen = std::collections::HashSet::new();
        let mut found = Vec::new();
        for list in cells.values() {
            for (n, &i) in list.iter().enumerate() {
                for &j in &list[n + 1..] {
                    let ((ba, a), (bb, b)) = (blocks[i], blocks[j]);
                    if ba.paint == bb.paint || !seen.insert((i.min(j), i.max(j))) {
                        continue;
                    }
                    let (alo, ahi, blo, bhi) = (a.center - a.half, a.center + a.half, b.center - b.half, b.center + b.half);
                    let clash = (0..3).any(|axis| {
                        let others = [(axis + 1) % 3, (axis + 2) % 3];
                        let overlap = others.iter().all(|&o| ahi[o].min(bhi[o]) - alo[o].max(blo[o]) > 1e-3);
                        overlap && ((ahi[axis] - bhi[axis]).abs() < 2e-5 || (alo[axis] - blo[axis]).abs() < 2e-5)
                    });
                    if clash {
                        found.push((ba, bb));
                    }
                }
            }
        }
        found
    }

    #[test]
    fn no_two_drawn_faces_share_a_plane() {
        let mut maps: Vec<(String, World)> = Theme::ALL.into_iter().map(|t| (format!("{t:?}"), World::generate(WorldRecipe::themed(9, Biome::IronShoals, t)))).collect();
        for layout in Layout::VERSUS.into_iter().filter(|l| *l != Layout::Town) {
            maps.push((format!("{layout:?}"), World::generate(WorldRecipe::preset(layout, Biome::IronShoals, Weather::Clear))));
        }
        for (what, w) in &maps {
            let raw = flickering(w, |b| b.aabb).len();
            let drawn = flickering(w, Block::drawn);
            eprintln!("{what}: {raw} flush pairs as built, {} as drawn", drawn.len());
            for (a, b) in drawn.iter().take(5) {
                eprintln!("  {:?} {:?} / {:?} {:?}", a.paint, a.aabb, b.paint, b.aabb);
            }
            // A pair whose faces were a hair apart as built can, by bad luck, meet as drawn.
            assert!(drawn.len() <= 3, "{what}: {} pairs of faces flicker", drawn.len());
        }
    }

    /// Blocks hanging in the air: off the ground and touching no other block.
    fn floating(w: &World) -> Vec<Block> {
        let mut cells: std::collections::HashMap<(i32, i32), Vec<usize>> = Default::default();
        let cell = |v: f32| (v / 8.0).floor() as i32;
        for (i, b) in w.blocks.iter().enumerate() {
            let (lo, hi) = (b.aabb.center - b.aabb.half, b.aabb.center + b.aabb.half);
            for cx in cell(lo.x)..=cell(hi.x) {
                for cz in cell(lo.z)..=cell(hi.z) {
                    cells.entry((cx, cz)).or_default().push(i);
                }
            }
        }
        let mut found = Vec::new();
        for (i, b) in w.blocks.iter().enumerate() {
            let (lo, hi) = (b.aabb.center - b.aabb.half, b.aabb.center + b.aabb.half);
            let ground = [(lo.x, lo.z), (hi.x, lo.z), (lo.x, hi.z), (hi.x, hi.z), (b.aabb.center.x, b.aabb.center.z)].iter().map(|(x, z)| w.height(*x, *z)).fold(f32::MIN, f32::max);
            // Holograms (the station's holo-table map) float on purpose.
            if lo.y <= ground + 0.05 || lo.y <= w.water + 0.05 || (b.paint == Paint::Blue && !b.solid) {
                continue;
            }
            let touches = (cell(lo.x)..=cell(hi.x)).flat_map(|cx| (cell(lo.z)..=cell(hi.z)).map(move |cz| (cx, cz))).any(|key| {
                cells.get(&key).is_some_and(|list| {
                    list.iter().any(|&j| {
                        let o = &w.blocks[j].aabb;
                        j != i && (0..3).all(|a| (o.center[a] - b.aabb.center[a]).abs() <= o.half[a] + b.aabb.half[a] + 0.02)
                    })
                })
            });
            if !touches {
                found.push(*b);
            }
        }
        found
    }

    #[test]
    fn nothing_hangs_in_the_air() {
        for seed in [1, 3, 9, 10, 17, 22] {
            let mut maps: Vec<(String, World)> = Theme::ALL.into_iter().map(|t| (format!("{t:?} {seed}"), World::generate(WorldRecipe::themed(seed, Biome::IronShoals, t)))).collect();
            if seed == 9 {
                for layout in Layout::VERSUS.into_iter().filter(|l| *l != Layout::Town) {
                    maps.push((format!("{layout:?}"), World::generate(WorldRecipe::preset(layout, Biome::IronShoals, Weather::Clear))));
                }
            }
            for (what, w) in &maps {
                let found = floating(w);
                let mut kinds = std::collections::HashSet::new();
                for b in found.iter().filter(|b| kinds.insert((b.paint, (b.aabb.half * 20.0).round().to_array().map(|v| v as i32)))).take(12) {
                    eprintln!("{what}: floating {:?} {} at {:?} half {:?}, ground {}", b.paint, if b.solid { "solid" } else { "decor" }, b.aabb.center, b.aabb.half, w.height(b.aabb.center.x, b.aabb.center.z));
                }
                assert!(found.is_empty(), "{what}: {} blocks hang in the air", found.len());
            }
        }
    }

    #[test]
    fn props_stand_clear_and_ladders_reach_roofs() {
        let mut maps: Vec<(String, World)> = Vec::new();
        for seed in [1, 3, 9, 17] {
            maps.extend(Theme::ALL.into_iter().map(|t| (format!("{t:?} {seed}"), World::generate(WorldRecipe::themed(seed, Biome::IronShoals, t)))));
        }
        for layout in Layout::VERSUS.into_iter().filter(|l| *l != Layout::Town) {
            maps.push((format!("{layout:?}"), World::generate(WorldRecipe::preset(layout, Biome::IronShoals, Weather::Clear))));
        }
        for (what, w) in &maps {
            // No tree, rock or crate grown into a wall: anything standing up out of the ground
            // where a prop stands must be lower than the prop's footing allows.
            for prop in &w.props {
                let Some(half) = prop.solid_half() else { continue };
                let centre = prop.pos + Vec3::Y * half.y;
                let inside = w.blocks.iter().find(|b| {
                    b.solid && b.aabb.top() > prop.pos.y + 0.45 && (0..3).all(|a| (b.aabb.center[a] - centre[a]).abs() < b.aabb.half[a] + half[a] - 0.05)
                });
                assert!(inside.is_none(), "{what}: a {:?} at {:?} grows into {:?}", prop.kind, prop.pos, inside.map(|b| b.aabb));
            }
            // The same for cars, barrels and crates.
            for item in &w.clutter {
                let (centre, half) = (item.center(), item.half());
                let inside = w.blocks.iter().find(|b| {
                    b.solid && b.aabb.top() > item.pos.y + 0.45 && (0..3).all(|a| (b.aabb.center[a] - centre[a]).abs() < b.aabb.half[a] + half[a] - 0.05)
                });
                assert!(inside.is_none(), "{what}: a {:?} at {:?} sits inside {:?}", item.kind, item.pos, inside.map(|b| b.aabb));
            }
            // Every ladder comes out on something to stand on at its top.
            for ladder in &w.ladders {
                let behind = ladder.foot - Vec3::new(ladder.out.x, 0.0, ladder.out.y) * 0.5;
                let landing = w.blocks.iter().any(|b| {
                    b.solid && (b.aabb.top() - ladder.top).abs() < 0.45 && (b.aabb.center.x - behind.x).abs() <= b.aabb.half.x + 0.3 && (b.aabb.center.z - behind.z).abs() <= b.aabb.half.z + 0.3
                });
                assert!(landing, "{what}: the ladder at {:?} reaches nothing at {}", ladder.foot, ladder.top);
            }
        }
    }

    #[test]
    fn doorway_steps_lead_to_doors() {
        for seed in [1, 3, 9, 10, 17, 22] {
            for theme in Theme::ALL {
                let w = World::generate(WorldRecipe::themed(seed, Biome::IronShoals, theme));
                for &(p, open) in &w.thresholds {
                    let door = w.doors.iter().any(|d| (d.center.x - p.x).abs() < 0.05 + if d.along_x { 0.0 } else { 0.4 } && (d.center.z - p.z).abs() < 0.05 + if d.along_x { 0.4 } else { 0.0 } && (d.center.y - p.y).abs() < 0.1);
                    // A gateway wider than a door: nothing solid in the opening.
                    let q = p + Vec3::Y;
                    let wide = w.blocks.iter().all(|b| !b.solid || ((b.aabb.center - q).abs() - b.aabb.half).max_element() > 0.0);
                    assert!(door || open || wide, "{theme:?} {seed}: steps up to a blank wall at {p:?}");
                }
            }
        }
    }

    fn spawns_are_clear(w: &World, what: &str) {
        let solids: Vec<Aabb> = w.solids().collect();
        for team in 0..2 {
            assert_eq!(w.team_spawns[team].len(), 6, "{what}");
            for spawn in &w.team_spawns[team] {
                let terrain = w.height(spawn.x, spawn.y);
                let ground = solids.iter().filter(|b| b.touches(spawn.x, spawn.y, 0.0) && b.top() < terrain + 0.5).map(|b| b.top()).fold(terrain, f32::max);
                assert!(w.walkable(spawn.x, spawn.y), "{what}: spawn {spawn:?} is not on land");
                let blocked = solids.iter().find(|b| b.touches(spawn.x, spawn.y, 0.4) && b.top() > ground + 0.05 && b.bottom() < ground + 1.8);
                assert!(blocked.is_none(), "{what}: spawn {spawn:?} is inside {blocked:?}");
            }
        }
    }

    #[test]
    fn themes_come_from_the_seed_and_every_one_turns_up() {
        let rolled: std::collections::HashSet<Theme> = (0..60).map(Theme::from_seed).collect();
        assert_eq!(rolled.len(), Theme::ALL.len());
        for theme in Theme::ALL {
            let w = World::generate(WorldRecipe::themed(5, Biome::IronShoals, theme));
            assert_eq!(w.theme, Some(theme));
        }
    }

    #[test]
    fn every_town_theme_has_enterable_buildings_and_clear_team_bases() {
        for theme in Theme::ALL {
            let w = World::generate(WorldRecipe::themed(9, Biome::IronShoals, theme));
            let what = format!("{theme:?}");
            assert!(w.blocks.len() > 1000, "{what}: only {} blocks", w.blocks.len());
            // Keep the renderer and the collision grid comfortable.
            assert!(w.blocks.len() < 12_000, "{what}: {} blocks is too many", w.blocks.len());
            // Two-storey buildings have stair steps one rise apart.
            let steps = w.blocks.iter().filter(|b| b.solid && (b.aabb.half.x - 0.21).abs() < 1e-3 && (b.aabb.half.z - 0.6).abs() < 1e-3).count();
            assert!(steps >= 11, "{what}: no staircases found");
            spawns_are_clear(&w, &what);
            // The bases are at the far ends of a big map.
            assert!(w.team_spawns[0][0].x < -95.0 && w.team_spawns[1][0].x > 95.0, "{what}: bases too close");
            assert!(!w.doors.is_empty() && !w.windows.is_empty() && !w.grass.is_empty(), "{what}");
            // Every doorway is a doorway: wide enough, tall enough.
            assert!(w.doors.iter().all(|d| d.width >= 1.6 && d.width <= DOOR_WIDTH + 0.05 && d.height >= 2.3), "{what}: odd doors");
        }
    }

    #[test]
    fn fixed_maps_are_fixed_and_playable() {
        for layout in Layout::VERSUS.into_iter().filter(|l| *l != Layout::Town) {
            let a = World::generate(WorldRecipe::preset(layout, Biome::IronShoals, Weather::Clear));
            let b = World::generate(WorldRecipe::preset(layout, Biome::EmberAtoll, Weather::Rain));
            // The same streets whatever the biome or weather.
            assert_eq!(a.blocks.len(), b.blocks.len(), "{layout:?} changed with the biome");
            assert!(a.blocks.len() > 400, "{layout:?} is nearly empty");
            assert!(a.blocks.len() < 12_000, "{layout:?}: {} blocks is too many", a.blocks.len());
            spawns_are_clear(&a, &format!("{layout:?}"));
            assert!(a.team_spawns[0][0].distance(a.team_spawns[1][0]) > 100.0, "{layout:?}: bases too close");
        }
    }

    #[test]
    fn indoor_maps_are_roofed_and_have_waypoints() {
        for layout in [Layout::OfficeTower, Layout::SpaceStation] {
            let w = World::generate(WorldRecipe::preset(layout, Biome::IronShoals, Weather::Clear));
            assert!(layout.is_indoor());
            assert!(w.waypoints.len() > 40, "{layout:?}: {} waypoints", w.waypoints.len());
            let solids: Vec<Aabb> = w.solids().collect();
            let mut roofed = 0;
            for p in &w.waypoints {
                let inside = solids.iter().any(|b| b.touches(p.x, p.z, 0.3) && b.bottom() < p.y + 1.7 && b.top() > p.y + 0.1);
                assert!(!inside, "{layout:?}: waypoint {p:?} is inside something");
                roofed += solids.iter().any(|b| b.touches(p.x, p.z, 0.0) && b.bottom() > p.y + 1.8) as usize;
            }
            assert!(roofed * 2 > w.waypoints.len(), "{layout:?}: only {roofed} of {} waypoints are under a roof", w.waypoints.len());
        }
        // The station has more than one floor: the hub's balconies.
        let w = World::generate(WorldRecipe::preset(Layout::SpaceStation, Biome::IronShoals, Weather::Clear));
        let deck = w.waypoints.iter().map(|p| p.y).fold(f32::MAX, f32::min);
        assert!(w.waypoints.iter().any(|p| p.y > deck + 3.0));
    }

    #[test]
    fn the_mountain_climbs_in_terraces() {
        let w = World::generate(WorldRecipe::preset(Layout::Mountainside, Biome::IronShoals, Weather::Clear));
        let (valley, summit) = (w.height(30.0, -86.0), w.height(30.0, 75.0));
        assert!(summit - valley > 18.0, "valley {valley}, summit {summit}");
        // Terraces are flat, cliffs are steep, ramps are gentle.
        assert!((w.height(-60.0, -5.0) - w.height(60.0, 5.0)).abs() < 0.4);
        assert!(w.height(10.0, 15.6) - w.height(10.0, 12.4) > 3.5);
        let ramp = w.height(48.0, 20.0) - w.height(48.0, 8.0);
        assert!(ramp > 2.0 && ramp < 5.0, "ramp {ramp}");
        assert!(!w.ladders.is_empty() && !w.doors.is_empty());
    }

    #[test]
    fn chunks_cover_the_grid() {
        let w = world();
        let n = w.chunks_per_side(32);
        let total: usize = (0..n).flat_map(|z| (0..n).map(move |x| (x, z))).map(|(x, z)| w.chunk(x, z, 32).positions.len()).sum();
        assert_eq!(total, (w.side - 1) * (w.side - 1) * 6);
        assert_eq!(sky_cubemap(&Biome::EmberAtoll.look(), true, 8).len(), 8 * 8 * 6 * 4);
    }
}
