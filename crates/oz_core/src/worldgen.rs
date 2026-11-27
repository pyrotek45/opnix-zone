//! Procedural maps.
//!
//! A [`World`] is generated from a [`WorldRecipe`] and is fully deterministic:
//! host and clients only exchange the recipe, never geometry. The current
//! generator builds archipelagos (one home island, outlying islets joined by
//! sandbars). New generators only need to fill in the same [`World`] struct to
//! work with the rest of the engine.

use crate::{collide::Aabb, economy::Ore, model::Paint, noise, rng::Rng};
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
    pub const ALL: [Biome; 5] = [
        Biome::EmberAtoll,
        Biome::BlackwaterCaldera,
        Biome::FrostbiteStation,
        Biome::IronShoals,
        Biome::SunkenReach,
    ];

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
    /// A compact island town: a street grid of solid buildings and cover, with two team bases.
    /// Different every seed.
    Town,
    /// Fixed map: three lanes between the bases - a tight alley, the main street through
    /// the fountain plaza, and a long market lane watched by a three-storey tower.
    Crossroads,
    /// Fixed map: a container yard - corridors of stacked shipping containers, a big
    /// warehouse in the middle, offices on the flanks.
    Dockyard,
}

impl Layout {
    /// Built as a town (flat ground, streets, team bases).
    pub fn is_town(self) -> bool {
        !matches!(self, Layout::Archipelago)
    }

    /// Versus map choices: the random town, then the fixed maps.
    pub const VERSUS: [Layout; 3] = [Layout::Town, Layout::Crossroads, Layout::Dockyard];

    pub fn title(self) -> &'static str {
        match self {
            Layout::Archipelago => "ARCHIPELAGO",
            Layout::Town => "RANDOM TOWN",
            Layout::Crossroads => "CROSSROADS",
            Layout::Dockyard => "DOCKYARD",
        }
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
    pub const ALL: [Weather; 4] = [
        Weather::Clear,
        Weather::Overcast,
        Weather::Rain,
        Weather::Fog,
    ];

    /// A seed's natural weather (mostly clear).
    pub fn from_seed(seed: u64) -> Weather {
        [
            Weather::Clear,
            Weather::Clear,
            Weather::Overcast,
            Weather::Rain,
            Weather::Fog,
            Weather::Clear,
        ][(seed % 6) as usize]
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
        Self {
            seed,
            biome,
            half_extent: 160.0,
            cell: 2.0,
            main_radius: 46.0,
            islets: 5,
            layout: Layout::Archipelago,
            weather: Weather::from_seed(seed),
        }
    }

    pub fn town(seed: u64, biome: Biome) -> Self {
        Self {
            seed,
            biome,
            half_extent: 96.0,
            cell: 2.0,
            main_radius: 68.0,
            islets: 0,
            layout: Layout::Town,
            weather: Weather::from_seed(seed),
        }
    }

    /// A fixed map: the same streets every time (its own seed), in any biome and weather.
    pub fn preset(layout: Layout, biome: Biome, weather: Weather) -> Self {
        let seed = match layout {
            Layout::Crossroads => 0xC055_2045,
            Layout::Dockyard => 0xD0C5_1D0C,
            _ => 7,
        };
        Self {
            layout,
            weather,
            ..Self::town(seed, biome)
        }
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
/// come from this one list, so they can never disagree.
#[derive(Clone, Copy, Debug)]
pub struct Block {
    pub aabb: Aabb,
    pub paint: Paint,
    pub solid: bool,
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
        if self.turned {
            Vec3::new(h.z, h.y, h.x)
        } else {
            h
        }
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
    /// Tufts of grass (render only).
    pub grass: Vec<Vec3>,
    /// The top of the fountain's spout, if the map has one.
    pub fountain: Option<Vec3>,
    /// Flag pole tops (flags wave in the wind).
    pub flags: Vec<Vec3>,
    /// Breakable street furniture.
    pub clutter: Vec<Clutter>,
    side: usize,
    heights: Vec<f32>,
}

pub const WATER_LEVEL: f32 = -0.35;
/// Height of low cover (walls, barriers, parapets): above a crouching robot's head, below a vault.
pub const COVER_HEIGHT: f32 = 1.2;
/// Window sills: crouch below one to hide; vault through it.
pub const WINDOW_SILL: f32 = 1.15;
/// How deep a player may wade before the sea pushes back.
pub const WADE_DEPTH: f32 = 0.65;
const SEABED: f32 = -1.4;
const BASE_RADIUS: f32 = 15.0;

impl World {
    pub fn generate(recipe: WorldRecipe) -> World {
        let mut rng = Rng::new(recipe.seed);
        let mut islands = vec![Island {
            center: Vec2::ZERO,
            radius: recipe.main_radius,
            phase: [rng.range(0.0, TAU), rng.range(0.0, TAU)],
            falloff: 11.0,
        }];
        let start = rng.range(0.0, TAU);
        for i in 0..recipe.islets {
            let angle = start + (i as f32 + rng.range(-0.22, 0.22)) * TAU / recipe.islets as f32;
            let radius = rng.range(15.0, 23.0);
            let distance = recipe.main_radius + radius + rng.range(26.0, 44.0);
            islands.push(Island {
                center: Vec2::from_angle(angle) * distance,
                radius,
                phase: [rng.range(0.0, TAU), rng.range(0.0, TAU)],
                falloff: 7.0,
            });
        }

        let side = (2.0 * recipe.half_extent / recipe.cell).round() as usize + 1;
        let mut world = World {
            recipe,
            water: WATER_LEVEL,
            islands,
            props: Vec::new(),
            blocks: Vec::new(),
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
            grass: Vec::new(),
            fountain: None,
            flags: Vec::new(),
            clutter: Vec::new(),
        };
        world.heights = (0..side * side)
            .map(|i| {
                let (ix, iz) = (i % side, i / side);
                world.raw_height(
                    ix as f32 * recipe.cell - recipe.half_extent,
                    iz as f32 * recipe.cell - recipe.half_extent,
                )
            })
            .collect();

        match recipe.layout {
            Layout::Archipelago => {
                world.pads = (0..6)
                    .map(|i| Vec2::from_angle(TAU * (i as f32 + 0.5) / 6.0) * 10.0)
                    .collect();
                world.scatter(&mut rng.fork());
            }
            Layout::Town | Layout::Crossroads | Layout::Dockyard => {
                world.build_town(&mut rng.fork())
            }
        }
        world
    }

    /// Analytic terrain function; sampled once into the grid.
    fn raw_height(&self, x: f32, z: f32) -> f32 {
        let seed = self.recipe.seed;
        let p = Vec2::new(x, z);
        let mut best = SEABED + 0.25 * noise::fbm(seed ^ 0xbed, x * 0.03, z * 0.03, 2);
        for (index, island) in self.islands.iter().enumerate() {
            let d = p - island.center;
            let (r, theta) = (d.length(), d.y.atan2(d.x));
            let edge = island.radius
                * (1.0
                    + 0.085 * (theta * 3.0 + island.phase[0]).sin()
                    + 0.05 * (theta * 5.0 + island.phase[1]).cos());
            let shore = ((edge - r) / island.falloff).clamp(-1.0, 1.0);
            if self.recipe.layout.is_town() {
                // Towns sit on a nearly flat plateau so streets read as streets.
                best = best.max(
                    shore * (1.35 + 0.12 * noise::value(seed ^ 0x70, x * 0.09, z * 0.09)) - 0.15,
                );
                continue;
            }
            let mut relief = 0.55 * noise::fbm(seed ^ 0x51ab, x * 0.07, z * 0.07, 3)
                + 0.25 * noise::value(seed ^ 0x77, x * 0.21, z * 0.21);
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
                let offset = (p - along * t).length()
                    + 1.5 * noise::value(seed ^ 0xca5e, x * 0.11, z * 0.11);
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
        let (gx, gz) = (
            ((x + half) / cell).clamp(0.0, max + 0.9999),
            ((z + half) / cell).clamp(0.0, max + 0.9999),
        );
        let (ix, iz) = (gx.floor() as usize, gz.floor() as usize);
        let (u, v) = (gx - ix as f32, gz - iz as f32);
        let at = |dx: usize, dz: usize| self.heights[(iz + dz) * self.side + ix + dx];
        let (a, b, c, d) = (at(0, 0), at(1, 0), at(0, 1), at(1, 1));
        if u + v <= 1.0 {
            a + (b - a) * u + (c - a) * v
        } else {
            d + (c - d) * (1.0 - u) + (b - d) * (1.0 - v)
        }
    }

    /// Every collision box on the map: solid architecture plus props.
    pub fn solids(&self) -> impl Iterator<Item = Aabb> + '_ {
        let architecture = self.blocks.iter().filter(|b| b.solid).map(|b| b.aabb);
        let props = self.props.iter().filter_map(|p| {
            p.solid_half()
                .map(|half| Aabb::new(p.pos + Vec3::Y * half.y, half))
        });
        architecture.chain(props)
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
        let mut d = 0.2;
        while d < max {
            let p = origin + dir * d;
            if p.y <= self.height(p.x, p.z) {
                return d;
            }
            d += 0.3;
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
            self.place_deposit(
                rng,
                home,
                if ore == Ore::Iron {
                    (19.0, 34.0)
                } else {
                    (30.0, 40.0)
                },
                ore,
            );
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
        let place = |world: &mut World,
                     rng: &mut Rng,
                     kind: PropKind,
                     count: usize,
                     min_height: f32,
                     scale: (f32, f32)| {
            for _ in 0..count {
                let island = islands[if rng.chance(0.55) {
                    0
                } else {
                    rng.below(islands.len())
                }];
                let p = island.center
                    + Vec2::from_angle(rng.range(0.0, TAU)) * rng.range(0.0, island.radius * 0.93);
                let y = world.height(p.x, p.y);
                let crowded = world
                    .deposits
                    .iter()
                    .any(|d| Vec2::new(d.pos.x, d.pos.z).distance(p) < 3.5);
                if y < min_height
                    || !clear_of_base(p)
                    || crowded
                    || (p.x.abs() < 4.0 && p.y > 0.0 && p.y < 30.0)
                {
                    continue;
                }
                world.props.push(Prop {
                    kind,
                    pos: Vec3::new(p.x, y, p.y),
                    yaw: rng.range(0.0, TAU),
                    scale: rng.range(scale.0, scale.1),
                });
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
        const LOT: f32 = 21.0;
        match self.recipe.layout {
            Layout::Crossroads => self.crossroads(rng),
            Layout::Dockyard => self.dockyard(rng),
            _ => self.random_lots(rng, LOT),
        }
        self.finish_town(rng);
    }

    /// The random town: a 5 x 5 grid of lots around an open plaza.
    fn random_lots(&mut self, rng: &mut Rng, lot: f32) {
        const LOT: f32 = 21.0;
        let _ = lot;
        for gx in -2i32..=2 {
            for gz in -2i32..=2 {
                if (gx, gz) == (0, 0) {
                    continue; // The plaza stays open.
                }
                let center = Vec2::new(gx as f32 * LOT, gz as f32 * LOT);
                let roll = rng.unit();
                if roll < 0.12 {
                    self.courtyard(rng, center);
                } else if roll < 0.42 {
                    // Two narrow houses with an alley between them.
                    for side in [-1.0, 1.0] {
                        let half = Vec2::new(rng.range(3.6, 6.2), rng.range(2.9, 3.4));
                        let at = center + Vec2::new(rng.range(-0.8, 0.8), side * 4.2);
                        self.building(rng, at, half, 1);
                    }
                } else {
                    let half = Vec2::new(rng.range(4.4, 6.6), rng.range(4.2, 6.6));
                    let floors = if half.x > 5.6 && half.y > 5.0 && rng.chance(0.4) {
                        3
                    } else if rng.chance(0.55) {
                        2
                    } else {
                        1
                    };
                    let at = center + Vec2::new(rng.range(-1.0, 1.0), rng.range(-1.0, 1.0));
                    self.building(rng, at, half, floors);
                }
            }
        }
    }

    /// Fixed map: three lanes. North alley (z = 24): two rows of tight two-storey houses.
    /// Main street (z = 0): the plaza and fountain, parked cars. South market (z = -26): open,
    /// low walls, overwatched by a three-storey tower. Cross streets at x = -31, 0, 31.
    fn crossroads(&mut self, rng: &mut Rng) {
        let blocks = [-42.0, -15.5, 15.5, 42.0];
        for (i, x) in blocks.into_iter().enumerate() {
            let inner = i == 1 || i == 2;
            // Between main street and the alley, and on the alley's far side.
            self.building(
                rng,
                Vec2::new(x, 10.5),
                Vec2::new(if inner { 7.5 } else { 6.0 }, 4.2),
                2,
            );
            self.building(
                rng,
                Vec2::new(x, 32.0),
                Vec2::new(6.0, 4.0),
                if inner { 2 } else { 1 },
            );
            // Between main street and the market.
            self.building(
                rng,
                Vec2::new(x, -11.0),
                Vec2::new(if inner { 7.0 } else { 6.0 }, 3.6),
                1,
            );
        }
        // The tower over the market, and quieter blocks at the back.
        self.building(rng, Vec2::new(0.0, -38.0), Vec2::new(6.2, 5.6), 3);
        for x in [-40.0, -20.0, 20.0, 40.0] {
            self.building(
                rng,
                Vec2::new(x, -44.0),
                Vec2::new(5.5, 4.5),
                if x.abs() < 30.0 { 2 } else { 1 },
            );
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
        let colours = [
            Paint::Rust,
            Paint::Teal,
            Paint::Coral,
            Paint::Gold,
            Paint::Steel,
            Paint::Blue,
        ];
        let container = |world: &mut World, at: Vec2, along_x: bool, high: bool, rng: &mut Rng| {
            let half = if along_x {
                Vec3::new(3.05, 1.3, 1.22)
            } else {
                Vec3::new(1.22, 1.3, 3.05)
            };
            let g = world.height(at.x, at.y);
            for level in 0..if high { 2 } else { 1 } {
                let paint = *rng.pick(&colours).unwrap_or(&Paint::Rust);
                let c = Vec3::new(at.x, g + half.y + level as f32 * 2.6, at.y);
                world.solid(Aabb::new(c, half), paint);
                // Corrugation ribs and a door end.
                let rib = if along_x {
                    Vec3::new(0.04, half.y * 0.95, half.z + 0.03)
                } else {
                    Vec3::new(half.x + 0.03, half.y * 0.95, 0.04)
                };
                for k in -3..=3 {
                    let offset = if along_x {
                        Vec3::X * k as f32 * 0.85
                    } else {
                        Vec3::Z * k as f32 * 0.85
                    };
                    world.decor(Aabb::new(c + offset, rib), Paint::Dark);
                }
            }
        };
        for (x, z0, z1) in [
            (-18.0, -40.0, -14.0),
            (-18.0, 14.0, 40.0),
            (18.0, -40.0, -14.0),
            (18.0, 14.0, 40.0),
        ] {
            let mut z = z0;
            while z < z1 {
                let high = rng.chance(0.4);
                container(self, Vec2::new(x, z), false, high, rng);
                z += 6.6 + if rng.chance(0.3) { 3.0 } else { 0.0 };
            }
        }
        for (x0, x1, z) in [
            (-46.0, -24.0, -34.0),
            (-46.0, -24.0, 36.0),
            (24.0, 46.0, -34.0),
            (24.0, 46.0, 36.0),
            (-12.0, 12.0, 26.0),
            (-12.0, 12.0, -26.0),
        ] {
            let mut x = x0;
            while x < x1 {
                let high = rng.chance(0.3);
                container(self, Vec2::new(x, z), true, high, rng);
                x += 6.6 + if rng.chance(0.35) { 3.2 } else { 0.0 };
            }
        }
        // Loose single containers for cover in the open.
        for (x, z, along) in [
            (-30.0, 6.0, false),
            (30.0, -6.0, false),
            (-8.0, 14.0, true),
            (8.0, -14.0, true),
        ] {
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
            let yaw = if matches!(kind, PropKind::Rock) {
                rng.below(4) as f32 * std::f32::consts::FRAC_PI_2
            } else {
                0.0
            };
            self.props.push(Prop {
                kind,
                pos: Vec3::new(p.x, self.height(p.x, p.y), p.y),
                yaw,
                scale,
            });
        }
        // Concrete barriers in the streets: chest-high to a crouching robot, low enough to vault.
        let mut placed = 0;
        for _ in 0..120 {
            if placed >= 16 {
                break;
            }
            let p = Vec2::new(rng.range(-50.0, 50.0), rng.range(-50.0, 50.0));
            let along_x = rng.chance(0.5);
            let half = if along_x {
                Vec2::new(1.3, 0.28)
            } else {
                Vec2::new(0.28, 1.3)
            };
            if self.occupied(p, 2.4) || p.length() < 6.0 {
                continue;
            }
            let g = self.height(p.x, p.y);
            self.solid(
                Aabb::span(
                    Vec3::new(p.x - half.x, g - 0.1, p.y - half.y),
                    Vec3::new(p.x + half.x, g + COVER_HEIGHT, p.y + half.y),
                ),
                Paint::Concrete,
            );
            self.decor(
                Aabb::span(
                    Vec3::new(
                        p.x - half.x - 0.02,
                        g + COVER_HEIGHT - 0.12,
                        p.y - half.y - 0.02,
                    ),
                    Vec3::new(
                        p.x + half.x + 0.02,
                        g + COVER_HEIGHT - 0.04,
                        p.y + half.y + 0.02,
                    ),
                ),
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
                self.props.push(Prop {
                    kind: PropKind::Palm,
                    pos: Vec3::new(p.x, self.height(p.x, p.y), p.y),
                    yaw: 0.0,
                    scale: rng.range(2.8, 5.0),
                });
            }
        }
        for team in 0..2 {
            let x = if team == 0 { -58.0 } else { 58.0 };
            self.team_spawns[team] = (0..6)
                .map(|i| {
                    Vec2::new(
                        x + (i % 2) as f32 * 2.5 * if team == 0 { 1.0 } else { -1.0 },
                        -12.5 + i as f32 * 5.0,
                    )
                })
                .collect();
            // A little sandbagged cover in front of each base.
            let front = if team == 0 { -50.0 } else { 50.0 };
            for z in [-9.0, 0.0, 9.0] {
                let y = self.height(front, z);
                self.solid(
                    Aabb::span(
                        Vec3::new(front - 0.4, y - 0.2, z - 1.6),
                        Vec3::new(front + 0.4, y + COVER_HEIGHT, z + 1.6),
                    ),
                    Paint::Sand,
                );
            }
        }
        self.spawn = self.team_spawns[0][2];
    }

    fn solid(&mut self, aabb: Aabb, paint: Paint) {
        self.blocks.push(Block {
            aabb,
            paint,
            solid: true,
        });
    }

    fn decor(&mut self, aabb: Aabb, paint: Paint) {
        self.blocks.push(Block {
            aabb,
            paint,
            solid: false,
        });
    }

    /// A band `out` thick around the outside of a footprint (plinths, floor-line trims):
    /// four strips hugging the walls, never a slab through the rooms inside.
    #[allow(clippy::too_many_arguments)]
    fn ring(
        &mut self,
        x0: f32,
        x1: f32,
        z0: f32,
        z1: f32,
        y0: f32,
        y1: f32,
        out: f32,
        paint: Paint,
    ) {
        self.decor(
            Aabb::span(
                Vec3::new(x0 - out, y0, z0 - out),
                Vec3::new(x1 + out, y1, z0),
            ),
            paint,
        );
        self.decor(
            Aabb::span(
                Vec3::new(x0 - out, y0, z1),
                Vec3::new(x1 + out, y1, z1 + out),
            ),
            paint,
        );
        self.decor(
            Aabb::span(Vec3::new(x0 - out, y0, z0), Vec3::new(x0, y1, z1)),
            paint,
        );
        self.decor(
            Aabb::span(Vec3::new(x1, y0, z0), Vec3::new(x1 + out, y1, z1)),
            paint,
        );
    }

    /// Is anything solid within `margin` of this point?
    fn occupied(&self, p: Vec2, margin: f32) -> bool {
        self.solids().any(|b| b.touches(p.x, p.y, margin))
    }

    /// A wall along one axis, with openings cut into it.
    /// `fixed` is the wall's extent across its thickness; openings are (start, end, bottom, top) along the wall.
    #[allow(clippy::too_many_arguments)]
    fn wall(
        &mut self,
        along_x: bool,
        fixed: (f32, f32),
        start: f32,
        length: f32,
        base: f32,
        height: f32,
        openings: &[(f32, f32, f32, f32)],
        paint: Paint,
    ) {
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
            let aabb = if along_x {
                Aabb::span(
                    Vec3::new(a, base + y0, fixed.0),
                    Vec3::new(b, base + y1, fixed.1),
                )
            } else {
                Aabb::span(
                    Vec3::new(fixed.0, base + y0, a),
                    Vec3::new(fixed.1, base + y1, b),
                )
            };
            self.solid(aabb, paint);
        }
        // Remember every opening: doors get doors, windows get glass.
        for (u0, u1, y0, y1) in openings.iter().copied() {
            let mid = start + (u0 + u1) * 0.5;
            let across = (fixed.0 + fixed.1) * 0.5;
            let center = if along_x {
                Vec3::new(mid, base + y0, across)
            } else {
                Vec3::new(across, base + y0, mid)
            };
            if y0 <= 0.0 {
                self.doors.push(DoorSpec {
                    center,
                    along_x,
                    width: u1 - u0,
                    height: y1 - y0,
                });
            } else {
                self.windows.push(WindowSpec {
                    center,
                    along_x,
                    width: u1 - u0,
                    height: y1 - y0,
                });
            }
        }
        // Sills under windows read better with a dark ledge, and a lintel over each opening.
        for (u0, u1, _, y1) in openings.iter().copied() {
            let (a, b) = (start + u0 - 0.08, start + u1 + 0.08);
            let lintel = if along_x {
                Aabb::span(
                    Vec3::new(a, base + y1, fixed.0 - 0.05),
                    Vec3::new(b, base + y1 + 0.12, fixed.1 + 0.05),
                )
            } else {
                Aabb::span(
                    Vec3::new(fixed.0 - 0.05, base + y1, a),
                    Vec3::new(fixed.1 + 0.05, base + y1 + 0.12, b),
                )
            };
            self.decor(lintel, Paint::Dark);
        }
        for (u0, u1, y0, _) in openings.iter().copied().filter(|o| o.2 > 0.0) {
            let (a, b) = (start + u0, start + u1);
            let ledge = if along_x {
                Aabb::span(
                    Vec3::new(a, base + y0 - 0.06, fixed.0 - 0.06),
                    Vec3::new(b, base + y0, fixed.1 + 0.06),
                )
            } else {
                Aabb::span(
                    Vec3::new(fixed.0 - 0.06, base + y0 - 0.06, a),
                    Vec3::new(fixed.1 + 0.06, base + y0, b),
                )
            };
            self.decor(ledge, Paint::Dark);
        }
    }

    /// Openings for one side of one storey: maybe a door, and windows wherever they fit.
    fn openings(
        rng: &mut Rng,
        length: f32,
        door: bool,
        windows: bool,
    ) -> Vec<(f32, f32, f32, f32)> {
        let mut out = Vec::new();
        let mut taken: Vec<(f32, f32)> = Vec::new();
        if door {
            let center = rng.range(1.6, length - 1.6);
            out.push((center - 0.85, center + 0.85, 0.0, 2.35));
            taken.push((center - 1.3, center + 1.3));
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

    /// One building: foundation, walls with doors and windows, up to three
    /// storeys joined by switchback stairs, interior rooms, roof, detailing,
    /// and sometimes a ladder up to the roof.
    fn building(&mut self, rng: &mut Rng, c: Vec2, half: Vec2, floors: u32) {
        const T: f32 = 0.3;
        const STOREY: f32 = 3.2;
        const STEPS: usize = 11;
        const RUN: f32 = 0.42;
        const LANE: f32 = 1.2;
        let (hx, hz) = (half.x, half.y);
        let (x0, x1, z0, z1) = (c.x - hx, c.x + hx, c.y - hz, c.y + hz);
        let mut g = f32::MIN;
        for (x, z) in [(x0, z0), (x1, z0), (x0, z1), (x1, z1), (c.x, c.y)] {
            g = g.max(self.height(x, z));
        }
        let g = g + 0.12;
        let paint = *rng
            .pick(&[
                Paint::Concrete,
                Paint::Rust,
                Paint::Bone,
                Paint::Teal,
                Paint::Steel,
                Paint::Sand,
            ])
            .unwrap_or(&Paint::Concrete);
        let trim = if matches!(paint, Paint::Bone | Paint::Sand | Paint::Concrete) {
            Paint::Steel
        } else {
            Paint::Bone
        };
        let roof_y = g + floors as f32 * STOREY;

        // Foundation and floor.
        self.solid(
            Aabb::span(Vec3::new(x0, g - 1.0, z0), Vec3::new(x1, g, z1)),
            Paint::Concrete,
        );
        self.decor(
            Aabb::span(
                Vec3::new(x0 + T, g, z0 + T),
                Vec3::new(x1 - T, g + 0.012, z1 - T),
            ),
            Paint::Wood,
        );

        // Doors go on up to two sides; buildings with stairs keep the stair walls (north, and south above two storeys) solid.
        let mut doors = [false; 4];
        doors[rng.below(if floors > 1 { 3 } else { 4 })] = true;
        if rng.chance(0.6) {
            doors[rng.below(if floors > 1 { 3 } else { 4 })] = true;
        }
        for floor in 0..floors {
            let base = g + floor as f32 * STOREY;
            let ground = floor == 0;
            // South (-z) and north (+z) walls run along X and own the corners.
            let south = Self::openings(rng, 2.0 * hx, ground && doors[0], true);
            self.wall(
                true,
                (z0, z0 + T),
                x0,
                2.0 * hx,
                base,
                STOREY,
                &south,
                paint,
            );
            let north = Self::openings(rng, 2.0 * hx, ground && doors[3], floor > 0 || floors == 1);
            self.wall(
                true,
                (z1 - T, z1),
                x0,
                2.0 * hx,
                base,
                STOREY,
                &north,
                paint,
            );
            let west = Self::openings(rng, 2.0 * hz - 2.0 * T, ground && doors[1], true);
            self.wall(
                false,
                (x0, x0 + T),
                z0 + T,
                2.0 * hz - 2.0 * T,
                base,
                STOREY,
                &west,
                paint,
            );
            let east = Self::openings(rng, 2.0 * hz - 2.0 * T, ground && doors[2], true);
            self.wall(
                false,
                (x1 - T, x1),
                z0 + T,
                2.0 * hz - 2.0 * T,
                base,
                STOREY,
                &east,
                paint,
            );
            // A trim band at each floor line, on the outside of the walls only.
            self.ring(
                x0,
                x1,
                z0,
                z1,
                base + STOREY - 0.14,
                base + STOREY,
                0.05,
                Paint::Dark,
            );
            // Ceiling lamp.
            self.decor(
                Aabb::new(
                    Vec3::new(c.x, base + STOREY - 0.3, c.y - hz * 0.3),
                    Vec3::new(0.35, 0.03, 0.35),
                ),
                Paint::Glow,
            );
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
                let (a, b) = if k % 2 == 0 {
                    (lx0 + i as f32 * RUN, lx0 + (i + 1) as f32 * RUN)
                } else {
                    (lx1 - (i + 1) as f32 * RUN, lx1 - i as f32 * RUN)
                };
                let step = Aabb::span(Vec3::new(a, base, lz0), Vec3::new(b, top, lz1));
                self.solid(
                    step,
                    if i % 2 == 0 {
                        Paint::Wood
                    } else {
                        Paint::Steel
                    },
                );
            }
        }
        // Floor slabs, each with a hole above the flight that arrives through it.
        for k in 1..floors {
            let top = g + k as f32 * STOREY;
            let (hx0, hx1, hz0, hz1) = lane(k - 1);
            // The hole runs the length of the flight, one step short so the top step meets solid floor.
            let (hx0, hx1) = if (k - 1) % 2 == 0 {
                (hx0, hx1 - RUN * 0.5)
            } else {
                (hx0 + RUN * 0.5, hx1)
            };
            let pieces = [
                (x0, x1, z0, hz0 - 0.05),
                (x0, x1, hz1 + 0.05, z1),
                (x0, hx0 - 0.05, hz0 - 0.05, hz1 + 0.05),
                (hx1 + 0.05, x1, hz0 - 0.05, hz1 + 0.05),
            ];
            for (a0, a1, b0, b1) in pieces {
                if a1 - a0 > 0.05 && b1 - b0 > 0.05 {
                    self.solid(
                        Aabb::span(Vec3::new(a0, top - 0.25, b0), Vec3::new(a1, top, b1)),
                        Paint::Concrete,
                    );
                    self.decor(
                        Aabb::span(
                            Vec3::new(a0.max(x0 + T), top, b0.max(z0 + T)),
                            Vec3::new(a1.min(x1 - T), top + 0.012, b1.min(z1 - T)),
                        ),
                        Paint::Wood,
                    );
                }
            }
            // Railing along the open side of the hole; the high end is left open as the way in.
            let rail_z = if (k - 1) % 2 == 0 {
                hz0 - 0.08
            } else {
                hz1 + 0.08
            };
            let (r0, r1) = if (k - 1) % 2 == 0 {
                (hx0, hx1 - 1.0)
            } else {
                (hx0 + 1.0, hx1)
            };
            self.solid(
                Aabb::span(
                    Vec3::new(r0, top, rail_z - 0.04),
                    Vec3::new(r1, top + 0.95, rail_z + 0.04),
                ),
                Paint::Dark,
            );
            self.decor(
                Aabb::span(
                    Vec3::new(r0, top + 0.92, rail_z - 0.06),
                    Vec3::new(r1, top + 1.0, rail_z + 0.06),
                ),
                Paint::Gold,
            );
            // Something to hide behind up here.
            let p = Vec2::new(rng.range(x0 + 1.5, x1 - 1.5), c.y + rng.range(-0.8, 0.8));
            self.solid(
                Aabb::span(
                    Vec3::new(p.x - 0.55, top, p.y - 0.55),
                    Vec3::new(p.x + 0.55, top + 1.0, p.y + 0.55),
                ),
                Paint::Teal,
            );
        }

        // Ground-floor rooms: a partition with a doorway, clear of the stairs.
        let partition = x0 + T + 0.4 + span + 1.3;
        if floors > 1 && x1 - partition > 2.6 {
            let length = 2.0 * hz - 2.0 * T - LANE - 0.1;
            let door = Self::openings(rng, length, true, false);
            self.wall(
                false,
                (partition, partition + 0.2),
                z0 + T,
                length,
                g,
                STOREY - 0.25,
                &door,
                trim,
            );
        }

        // Furniture on the ground floor: a counter or table, away from the doors and stairs.
        for _ in 0..2 {
            let p = Vec2::new(
                rng.range(x0 + 1.6, x1 - 1.6),
                rng.range(z0 + 1.6, z1 - 1.6 - if floors > 1 { 1.4 } else { 0.0 }),
            );
            if (p.x - partition).abs() < 1.2 {
                continue;
            }
            let size = if rng.chance(0.5) {
                Vec2::new(rng.range(0.7, 1.4), 0.4)
            } else {
                Vec2::new(0.5, 0.5)
            };
            self.solid(
                Aabb::span(
                    Vec3::new(p.x - size.x, g, p.y - size.y),
                    Vec3::new(p.x + size.x, g + 0.95, p.y + size.y),
                ),
                Paint::Wood,
            );
        }

        // Detailing: a dark plinth, corner pilasters, a drainpipe, the odd air-conditioning unit.
        let o = 0.06;
        self.ring(x0, x1, z0, z1, g, g + 0.4, o, Paint::Dark);
        // Pilasters stand proud of the walls and are cover: solid, so nobody walks into them.
        for (x, z) in [(x0, z0), (x1, z0), (x0, z1), (x1, z1)] {
            self.solid(
                Aabb::new(
                    Vec3::new(x, g + (roof_y - g) * 0.5, z),
                    Vec3::new(0.2, (roof_y - g) * 0.5, 0.2),
                ),
                trim,
            );
        }
        // The drainpipe is too thin to hide behind: walk-through, like every pole and wire.
        self.decor(
            Aabb::span(
                Vec3::new(x1 + 0.02, g, z1 - 0.6),
                Vec3::new(x1 + 0.14, roof_y, z1 - 0.48),
            ),
            Paint::Steel,
        );
        for floor in 0..floors {
            if rng.chance(0.35) {
                let y = g + floor as f32 * STOREY + 2.4;
                let z = rng.range(z0 + 1.0, z1 - 1.0);
                self.solid(
                    Aabb::span(
                        Vec3::new(x0 - 0.45, y, z - 0.35),
                        Vec3::new(x0, y + 0.45, z + 0.35),
                    ),
                    Paint::Metal,
                );
                self.decor(
                    Aabb::span(
                        Vec3::new(x0 - 0.47, y + 0.1, z - 0.2),
                        Vec3::new(x0 - 0.45, y + 0.35, z + 0.2),
                    ),
                    Paint::Dark,
                );
            }
        }

        // Roof, parapet and rooftop clutter.
        self.solid(
            Aabb::span(
                Vec3::new(x0 - 0.15, roof_y, z0 - 0.15),
                Vec3::new(x1 + 0.15, roof_y + 0.3, z1 + 0.15),
            ),
            Paint::Dark,
        );
        // Parapets along two sides: a ledge to crouch behind up there (the ladders come up the other sides).
        self.solid(
            Aabb::span(
                Vec3::new(x0 - 0.15, roof_y + 0.3, z0 - 0.15),
                Vec3::new(x1 + 0.15, roof_y + 0.3 + COVER_HEIGHT, z0 + 0.1),
            ),
            paint,
        );
        self.solid(
            Aabb::span(
                Vec3::new(x0 - 0.15, roof_y + 0.3, z1 - 0.1),
                Vec3::new(x1 + 0.15, roof_y + 0.3 + COVER_HEIGHT, z1 + 0.15),
            ),
            paint,
        );
        let unit = Vec2::new(rng.range(x0 + 1.0, x1 - 1.0), rng.range(z0 + 1.0, z1 - 1.0));
        // The rule for scenery: anything you could hide behind is solid (this air-conditioning
        // unit is rooftop cover); anything too thin to hide behind - masts, poles, wires - you walk through.
        self.solid(
            Aabb::new(
                Vec3::new(unit.x, roof_y + 0.7, unit.y),
                Vec3::new(0.6, 0.4, 0.45),
            ),
            Paint::Metal,
        );
        if rng.chance(0.4) {
            self.decor(
                Aabb::new(
                    Vec3::new(c.x + hx * 0.5, roof_y + 1.6, c.y),
                    Vec3::new(0.04, 1.3, 0.04),
                ),
                Paint::Metal,
            );
            self.decor(
                Aabb::new(
                    Vec3::new(c.x + hx * 0.5, roof_y + 2.95, c.y),
                    Vec3::new(0.08, 0.08, 0.08),
                ),
                Paint::Red,
            );
        } else if rng.chance(0.5) {
            // A flag on a pole; it waves in the wind.
            self.decor(
                Aabb::new(
                    Vec3::new(c.x - hx * 0.5, roof_y + 2.0, c.y),
                    Vec3::new(0.05, 1.7, 0.05),
                ),
                Paint::Metal,
            );
            self.flags
                .push(Vec3::new(c.x - hx * 0.5, roof_y + 3.55, c.y));
        }

        // A ladder up a side wall without a door, onto the roof.
        if rng.chance(0.55) && !(doors[1] && doors[2]) {
            let east = !doors[2] && (doors[1] || rng.chance(0.5));
            let (x, out) = if east {
                (x1 + 0.22, 1.0)
            } else {
                (x0 - 0.22, -1.0)
            };
            // Beside the south corner, where walls never have windows.
            let z = z0 + 0.75;
            let foot = Vec3::new(x, self.height(x, z), z);
            for side in [-0.3, 0.3] {
                self.decor(
                    Aabb::span(
                        Vec3::new(x - 0.04, foot.y, z + side - 0.04),
                        Vec3::new(x + 0.04, roof_y + 1.0, z + side + 0.04),
                    ),
                    Paint::Iron,
                );
            }
            let mut y = foot.y + 0.3;
            while y < roof_y + 0.6 {
                self.decor(
                    Aabb::new(Vec3::new(x, y, z), Vec3::new(0.03, 0.03, 0.3)),
                    Paint::Iron,
                );
                y += 0.35;
            }
            self.ladders.push(Ladder {
                foot,
                top: roof_y + 0.3,
                out: Vec2::new(out, 0.0),
            });
        }
    }

    /// The town square: a tiered fountain (good cover) ringed by benches' worth of lamps and trees.
    fn plaza(&mut self, rng: &mut Rng) {
        let g = self.height(0.0, 0.0);
        // Basin: four low walls with water inside, a pillar and a raised bowl.
        let (r, wall) = (3.2, 0.35);
        for (cx, cz, hx, hz) in [
            (0.0, r, r, wall),
            (0.0, -r, r, wall),
            (r, 0.0, wall, r - wall),
            (-r, 0.0, wall, r - wall),
        ] {
            self.solid(
                Aabb::new(Vec3::new(cx, g + 0.35, cz), Vec3::new(hx, 0.45, hz)),
                Paint::Concrete,
            );
            self.decor(
                Aabb::new(
                    Vec3::new(cx, g + 0.83, cz),
                    Vec3::new(hx + 0.05, 0.04, hz + 0.05),
                ),
                Paint::Bone,
            );
        }
        self.decor(
            Aabb::new(
                Vec3::new(0.0, g + 0.45, 0.0),
                Vec3::new(r - wall, 0.05, r - wall),
            ),
            Paint::Blue,
        );
        self.solid(
            Aabb::new(Vec3::new(0.0, g + 1.0, 0.0), Vec3::new(0.45, 1.0, 0.45)),
            Paint::Bone,
        );
        self.solid(
            Aabb::new(Vec3::new(0.0, g + 2.05, 0.0), Vec3::new(1.2, 0.08, 1.2)),
            Paint::Concrete,
        );
        self.decor(
            Aabb::new(Vec3::new(0.0, g + 2.12, 0.0), Vec3::new(1.0, 0.03, 1.0)),
            Paint::Blue,
        );
        self.solid(
            Aabb::new(Vec3::new(0.0, g + 2.5, 0.0), Vec3::new(0.12, 0.4, 0.12)),
            Paint::Bone,
        );
        self.fountain = Some(Vec3::new(0.0, g + 2.95, 0.0));
        // Lamps around the square.
        for i in 0..6 {
            let a = i as f32 * TAU / 6.0 + 0.3;
            let p = Vec2::from_angle(a) * 8.0;
            self.props.push(Prop {
                kind: PropKind::Lamp,
                pos: Vec3::new(p.x, self.height(p.x, p.y), p.y),
                yaw: a,
                scale: 1.0,
            });
        }
        let _ = rng;
    }

    /// Bins, barrels, benches, mailboxes, crate stacks and parked cars along the streets.
    fn street_clutter(&mut self, rng: &mut Rng) {
        let kinds = [
            (ClutterKind::Bin, 14),
            (ClutterKind::Barrel, 10),
            (ClutterKind::Bench, 8),
            (ClutterKind::Mailbox, 6),
            (ClutterKind::Crates, 8),
            (ClutterKind::Car, 6),
        ];
        for (kind, count) in kinds {
            let mut placed = 0;
            for _ in 0..count * 12 {
                if placed >= count {
                    break;
                }
                // Streets run between the lots on the 21 m grid.
                let along = rng.range(-55.0, 55.0);
                let street = (rng.below(5) as f32 - 2.0) * 21.0
                    + 10.5 * if rng.chance(0.5) { 1.0 } else { -1.0 };
                let across = rng.range(-2.6, 2.6);
                let on_x = rng.chance(0.5);
                let p = if on_x {
                    Vec2::new(along, street + across)
                } else {
                    Vec2::new(street + across, along)
                };
                let turned = !on_x;
                let half = kind.half();
                let reach = half.x.max(half.z) + 0.8;
                if p.x.abs() > 54.0
                    || p.length() < 6.0
                    || self.occupied(p, reach)
                    || self
                        .clutter
                        .iter()
                        .any(|c| Vec2::new(c.pos.x, c.pos.z).distance(p) < reach + 1.5)
                {
                    continue;
                }
                self.clutter.push(Clutter {
                    kind,
                    pos: Vec3::new(p.x, self.height(p.x, p.y), p.y),
                    turned,
                });
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
                let p = Vec2::new(gx as f32 * 21.0 + 10.5, gz as f32 * 21.0 + 10.5)
                    + Vec2::new(rng.range(-1.0, 1.0), rng.range(-1.0, 1.0));
                if p.length() < 7.0 || self.occupied(p, 1.2) || p.x.abs() > 52.0 {
                    continue;
                }
                self.props.push(Prop {
                    kind: PropKind::Tree,
                    pos: Vec3::new(p.x, self.height(p.x, p.y), p.y),
                    yaw: rng.range(0.0, TAU),
                    scale: rng.range(3.6, 5.2),
                });
            }
        }
        for _ in 0..40 {
            let p = Vec2::new(rng.range(-60.0, 60.0), rng.range(-60.0, 60.0));
            if self.occupied(p, 0.9) || p.length() < 5.0 {
                continue;
            }
            self.props.push(Prop {
                kind: PropKind::Bush,
                pos: Vec3::new(p.x, self.height(p.x, p.y), p.y),
                yaw: rng.range(0.0, TAU),
                scale: rng.range(0.7, 1.2),
            });
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
        for (dx, dz, long_x) in [
            (-6.0, 0.0, false),
            (6.0, 0.0, false),
            (0.0, -6.0, true),
            (0.0, 6.0, true),
        ] {
            if rng.chance(0.75) {
                let half = if long_x {
                    Vec3::new(3.0, COVER_HEIGHT * 0.5, 0.25)
                } else {
                    Vec3::new(0.25, COVER_HEIGHT * 0.5, 3.0)
                };
                self.solid(
                    Aabb::new(Vec3::new(c.x + dx, g + COVER_HEIGHT * 0.5, c.y + dz), half),
                    Paint::Concrete,
                );
            }
        }
        self.solid(
            Aabb::new(Vec3::new(c.x, g + 0.4, c.y), Vec3::new(1.2, 0.5, 1.2)),
            Paint::Concrete,
        );
        // Water in the planter: a skin on top, nothing to sink into.
        self.decor(
            Aabb::new(Vec3::new(c.x, g + 0.915, c.y), Vec3::new(0.9, 0.02, 0.9)),
            Paint::Blue,
        );
    }

    fn place_deposit(&mut self, rng: &mut Rng, island: Island, band: (f32, f32), ore: Ore) {
        for _ in 0..40 {
            let p =
                island.center + Vec2::from_angle(rng.range(0.0, TAU)) * rng.range(band.0, band.1);
            let y = self.height(p.x, p.y);
            let spaced = self
                .deposits
                .iter()
                .all(|d| Vec2::new(d.pos.x, d.pos.z).distance(p) > 7.0);
            if y > 0.45 && spaced {
                let amount = match ore {
                    Ore::Iron => 420.0,
                    Ore::Copper => 320.0,
                    Ore::Aether => 200.0,
                } * rng.range(0.8, 1.3);
                self.deposits.push(Deposit {
                    pos: Vec3::new(p.x, y, p.y),
                    ore,
                    amount,
                });
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
        let look = self.recipe.biome.look();
        let mut out = TerrainChunk::default();
        let mut rng = Rng::new(self.recipe.seed ^ ((cx as u64) << 20) ^ cz as u64);
        let (cell, half) = (self.recipe.cell, self.recipe.half_extent);
        for iz in cz * chunk_cells..((cz + 1) * chunk_cells).min(self.side - 1) {
            for ix in cx * chunk_cells..((cx + 1) * chunk_cells).min(self.side - 1) {
                let p = |dx: usize, dz: usize| {
                    Vec3::new(
                        (ix + dx) as f32 * cell - half,
                        self.heights[(iz + dz) * self.side + ix + dx],
                        (iz + dz) as f32 * cell - half,
                    )
                };
                let (a, b, c, d) = (p(0, 0), p(1, 0), p(0, 1), p(1, 1));
                for [a, b, c] in [[a, c, b], [b, c, d]] {
                    let normal = (b - a).cross(c - a).normalize();
                    let y = (a.y + b.y + c.y) / 3.0;
                    let base = if y < 0.74 {
                        look.sand
                    } else if y > 3.3 || normal.y < 0.82 {
                        look.high
                    } else {
                        look.grass
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
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// RGBA8 sRGB pixels for a skybox cubemap, faces in +X, -X, +Y, -Y, +Z, -Z order.
pub fn sky_cubemap(look: &BiomeLook, night: bool, size: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((size * size * 6 * 4) as usize);
    let sun = Vec3::new(-0.54, 0.21, 0.81).normalize();
    let (horizon, zenith) = if night {
        (Vec3::new(0.10, 0.13, 0.22), Vec3::new(0.012, 0.018, 0.05))
    } else {
        (Vec3::from(look.horizon), Vec3::from(look.zenith))
    };
    let sea = if night {
        Vec3::new(0.03, 0.06, 0.10)
    } else {
        Vec3::from(look.water) * 1.25
    };
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
                    let h =
                        noise::value(0x57a2, cell.x * 12.9898 + cell.z * 4.1414, cell.y * 78.233);
                    if d.y > 0.02 && h > 0.965 {
                        rgb = rgb.lerp(Vec3::new(0.85, 0.92, 1.0), ((h - 0.965) * 28.0).min(1.0));
                    }
                    if angle > 0.9975 {
                        rgb = Vec3::new(0.82, 0.88, 0.96);
                    } else {
                        rgb = rgb.lerp(
                            Vec3::new(0.35, 0.45, 0.62),
                            ((angle - 0.93) / 0.07).clamp(0.0, 1.0).powi(3) * 0.4,
                        );
                    }
                } else {
                    rgb = rgb.lerp(
                        Vec3::new(1.0, 0.79, 0.49),
                        ((angle - 0.82) / 0.18).clamp(0.0, 1.0).powi(3) * 0.65,
                    );
                    let streak = (d.x * 17.0 + d.z * 7.0).sin()
                        + (d.x * 32.0 - d.z * 15.0).sin() * 0.45
                        + (d.y * 75.0).sin() * 0.9;
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
        assert!(
            a.deposits.len() >= 20,
            "expected plenty of ore, got {}",
            a.deposits.len()
        );
        assert!(a.deposits.iter().any(|d| d.ore == Ore::Aether));
    }

    #[test]
    fn sandbars_connect_every_islet() {
        let w = world();
        for island in &w.islands[1..] {
            let steps = 200;
            for i in 0..=steps {
                let p = island.center * (i as f32 / steps as f32);
                assert!(
                    w.walkable(p.x, p.y),
                    "gap in sandbar at {p:?} (h={})",
                    w.height(p.x, p.y)
                );
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

    #[test]
    fn town_has_enterable_buildings_and_clear_team_bases() {
        let w = World::generate(WorldRecipe::town(9, Biome::IronShoals));
        let solids: Vec<Aabb> = w.solids().collect();
        assert!(w.blocks.len() > 600, "only {} blocks", w.blocks.len());
        // Two-storey houses have stair steps one rise apart.
        let steps = w
            .blocks
            .iter()
            .filter(|b| {
                b.solid && (b.aabb.half.x - 0.21).abs() < 1e-3 && (b.aabb.half.z - 0.6).abs() < 1e-3
            })
            .count();
        assert!(steps >= 11, "no staircases found");
        for team in 0..2 {
            assert_eq!(w.team_spawns[team].len(), 6);
            for spawn in &w.team_spawns[team] {
                assert!(
                    w.walkable(spawn.x, spawn.y),
                    "spawn {spawn:?} is not on land"
                );
                assert!(
                    solids.iter().all(|b| !b.touches(spawn.x, spawn.y, 0.4)),
                    "spawn {spawn:?} is inside something"
                );
            }
        }
        // The plaza is open around its fountain, which is cover.
        assert!(w.fountain.is_some());
        assert!(
            w.walkable(0.0, 6.0) && solids.iter().all(|b| !b.touches(0.0, 6.0, 1.0)),
            "the plaza must be open"
        );
        assert!(!w.doors.is_empty() && !w.windows.is_empty() && !w.grass.is_empty());
    }

    #[test]
    fn fixed_maps_are_fixed_and_playable() {
        for layout in [Layout::Crossroads, Layout::Dockyard] {
            let a = World::generate(WorldRecipe::preset(
                layout,
                Biome::IronShoals,
                Weather::Clear,
            ));
            let b = World::generate(WorldRecipe::preset(
                layout,
                Biome::EmberAtoll,
                Weather::Rain,
            ));
            // The same streets whatever the biome or weather.
            assert_eq!(
                a.blocks.len(),
                b.blocks.len(),
                "{layout:?} changed with the biome"
            );
            let solids: Vec<Aabb> = a.solids().collect();
            assert!(a.blocks.len() > 400, "{layout:?} is nearly empty");
            for team in 0..2 {
                for spawn in &a.team_spawns[team] {
                    assert!(
                        a.walkable(spawn.x, spawn.y),
                        "{layout:?} spawn {spawn:?} is not on land"
                    );
                    assert!(
                        solids.iter().all(|s| !s.touches(spawn.x, spawn.y, 0.4)),
                        "{layout:?} spawn {spawn:?} is inside something"
                    );
                }
            }
        }
    }

    #[test]
    fn chunks_cover_the_grid() {
        let w = world();
        let n = w.chunks_per_side(32);
        let total: usize = (0..n)
            .flat_map(|z| (0..n).map(move |x| (x, z)))
            .map(|(x, z)| w.chunk(x, z, 32).positions.len())
            .sum();
        assert_eq!(total, (w.side - 1) * (w.side - 1) * 6);
        assert_eq!(
            sky_cubemap(&Biome::EmberAtoll.look(), true, 8).len(),
            8 * 8 * 6 * 4
        );
    }
}
