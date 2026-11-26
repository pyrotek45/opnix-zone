//! Procedural maps.
//!
//! A [`World`] is generated from a [`WorldRecipe`] and is fully deterministic:
//! host and clients only exchange the recipe, never geometry. The current
//! generator builds archipelagos (one home island, outlying islets joined by
//! sandbars). New generators only need to fill in the same [`World`] struct to
//! work with the rest of the engine.

use crate::{economy::Ore, noise, rng::Rng};
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
                title: "EMBER ATOLL", blurb: "Warm-water defense perimeter and the first beacon site.",
                sand: [0.89, 0.69, 0.43], grass: [0.37, 0.62, 0.43], high: [0.39, 0.47, 0.43],
                horizon: [1.0, 0.65, 0.47], zenith: [0.16, 0.35, 0.55], water: [0.16, 0.49, 0.54], fog: [0.65, 0.68, 0.67],
                palms: 90, ruins: 6,
            },
            Biome::BlackwaterCaldera => BiomeLook {
                title: "BLACKWATER CALDERA", blurb: "Volcanic ridges, dark terrain and glowing industrial markers.",
                sand: [0.42, 0.30, 0.27], grass: [0.30, 0.27, 0.26], high: [0.24, 0.20, 0.21],
                horizon: [0.85, 0.32, 0.18], zenith: [0.16, 0.12, 0.21], water: [0.12, 0.17, 0.22], fog: [0.45, 0.33, 0.30],
                palms: 0, ruins: 14,
            },
            Biome::FrostbiteStation => BiomeLook {
                title: "FROSTBITE STATION", blurb: "Frozen terrain, blue sky and biting wind.",
                sand: [0.74, 0.81, 0.86], grass: [0.82, 0.90, 0.95], high: [0.62, 0.74, 0.84],
                horizon: [0.67, 0.82, 0.92], zenith: [0.21, 0.38, 0.63], water: [0.20, 0.42, 0.58], fog: [0.72, 0.80, 0.86],
                palms: 0, ruins: 10,
            },
            Biome::IronShoals => BiomeLook {
                title: "IRON SHOALS", blurb: "Industrial reef with taller cover and concrete pylons.",
                sand: [0.60, 0.56, 0.50], grass: [0.41, 0.50, 0.50], high: [0.40, 0.44, 0.49],
                horizon: [0.83, 0.68, 0.45], zenith: [0.18, 0.26, 0.34], water: [0.15, 0.37, 0.43], fog: [0.60, 0.62, 0.60],
                palms: 8, ruins: 22,
            },
            Biome::SunkenReach => BiomeLook {
                title: "SUNKEN REACH", blurb: "Low broken shores and flooded ruins near the signal.",
                sand: [0.50, 0.60, 0.50], grass: [0.30, 0.56, 0.48], high: [0.29, 0.42, 0.45],
                horizon: [0.55, 0.42, 0.68], zenith: [0.08, 0.16, 0.33], water: [0.10, 0.30, 0.36], fog: [0.45, 0.50, 0.58],
                palms: 40, ruins: 16,
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
    Town,
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
}

impl WorldRecipe {
    pub fn archipelago(seed: u64, biome: Biome) -> Self {
        Self { seed, biome, half_extent: 160.0, cell: 2.0, main_radius: 46.0, islets: 5, layout: Layout::Archipelago }
    }

    pub fn town(seed: u64, biome: Biome) -> Self {
        Self { seed, biome, half_extent: 96.0, cell: 2.0, main_radius: 68.0, islets: 0, layout: Layout::Town }
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
    /// A solid building; its size comes from `Prop::half`.
    Building,
}

#[derive(Clone, Copy, Debug)]
pub struct Prop {
    pub kind: PropKind,
    /// Ground position.
    pub pos: Vec3,
    pub yaw: f32,
    pub scale: f32,
    /// Explicit collision half-extents for [`PropKind::Building`].
    pub half: Vec3,
}

impl Prop {
    /// Axis-aligned collision half-extents, centred at `pos + half.y`. `None` = no collision.
    pub fn solid_half(&self) -> Option<Vec3> {
        let s = self.scale;
        match self.kind {
            PropKind::Palm => Some(Vec3::new(0.26, s * 0.5, 0.26)),
            PropKind::Rock => Some(Vec3::new(s * 0.75, s * 0.45, s * 0.65)),
            PropKind::Ruin => Some(Vec3::new(1.6 * s, 0.9 * s, 0.35 * s)),
            PropKind::Pylon => Some(Vec3::new(0.7, 1.9 * s, 0.7)),
            PropKind::Crate => Some(Vec3::new(0.62, 0.55, 0.62)),
            PropKind::Building => Some(self.half),
        }
    }
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
    side: usize,
    heights: Vec<f32>,
}

pub const WATER_LEVEL: f32 = -0.35;
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
            recipe, water: WATER_LEVEL, islands, props: Vec::new(), deposits: Vec::new(), landings: Vec::new(),
            core: Vec2::ZERO, spawn: Vec2::new(0.0, 6.0), pads: Vec::new(), team_spawns: [Vec::new(), Vec::new()], side, heights: Vec::new(),
        };
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
            Layout::Town => world.build_town(&mut rng.fork()),
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
            let edge = island.radius * (1.0 + 0.085 * (theta * 3.0 + island.phase[0]).sin() + 0.05 * (theta * 5.0 + island.phase[1]).cos());
            let shore = ((edge - r) / island.falloff).clamp(-1.0, 1.0);
            if self.recipe.layout == Layout::Town {
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
            self.place_deposit(rng, home, if ore == Ore::Iron { (19.0, 34.0) } else { (30.0, 40.0) }, ore);
        }
        for index in 1..self.islands.len() {
            let island = self.islands[index];
            for i in 0..4 {
                let ore = match (index + i) % 4 { 0 => Ore::Iron, 1 | 2 => Ore::Copper, _ => Ore::Aether };
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
                world.props.push(Prop { kind, pos: Vec3::new(p.x, y, p.y), yaw: rng.range(0.0, TAU), scale: rng.range(scale.0, scale.1), half: Vec3::ZERO });
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

    /// A street grid of buildings with cover in the lanes and a base on each side.
    fn build_town(&mut self, rng: &mut Rng) {
        const BLOCK: f32 = 21.0;
        let look = self.recipe.biome.look();
        for gx in -2i32..=2 {
            for gz in -2i32..=2 {
                let center = Vec2::new(gx as f32 * BLOCK, gz as f32 * BLOCK);
                if (gx, gz) == (0, 0) {
                    continue; // The plaza stays open.
                }
                // Some lots hold two smaller buildings, which opens alleys through the block.
                let split = rng.chance(0.35);
                for part in 0..if split { 2 } else { 1 } {
                    let half = Vec3::new(rng.range(3.6, 6.4), rng.range(1.7, 3.6), if split { rng.range(2.2, 2.9) } else { rng.range(3.6, 6.4) });
                    let offset = if split { Vec2::new(rng.range(-0.8, 0.8), if part == 0 { -3.6 } else { 3.6 }) } else { Vec2::new(rng.range(-1.0, 1.0), rng.range(-1.0, 1.0)) };
                    let p = center + offset;
                    self.props.push(Prop { kind: PropKind::Building, pos: Vec3::new(p.x, self.height(p.x, p.y) - 0.3, p.y), yaw: 0.0, scale: rng.range(0.0, 1.0), half });
                }
            }
        }
        // Cover in the streets and the plaza.
        let blocked = |world: &World, p: Vec2, margin: f32| world.props.iter().any(|prop| {
            let half = prop.solid_half().unwrap_or(Vec3::ZERO);
            (prop.pos.x - p.x).abs() < half.x + margin && (prop.pos.z - p.y).abs() < half.z + margin
        });
        for i in 0..70 {
            let p = Vec2::new(rng.range(-60.0, 60.0), rng.range(-58.0, 58.0));
            if blocked(self, p, 1.4) || p.x.abs() > 52.0 {
                continue;
            }
            let kind = match i % 5 { 0 | 1 => PropKind::Crate, 2 => PropKind::Ruin, 3 => PropKind::Rock, _ => PropKind::Pylon };
            let scale = match kind { PropKind::Ruin => rng.range(0.6, 0.8), PropKind::Rock => rng.range(0.8, 1.3), PropKind::Pylon => rng.range(0.7, 1.0), _ => 1.0 };
            self.props.push(Prop { kind, pos: Vec3::new(p.x, self.height(p.x, p.y), p.y), yaw: rng.range(0.0, TAU), scale, half: Vec3::ZERO });
        }
        for _ in 0..look.palms / 3 {
            let p = Vec2::from_angle(rng.range(0.0, TAU)) * rng.range(56.0, 64.0);
            if !blocked(self, p, 1.0) && p.x.abs() < 50.0 {
                self.props.push(Prop { kind: PropKind::Palm, pos: Vec3::new(p.x, self.height(p.x, p.y), p.y), yaw: 0.0, scale: rng.range(2.8, 5.0), half: Vec3::ZERO });
            }
        }
        for team in 0..2 {
            let x = if team == 0 { -58.0 } else { 58.0 };
            self.team_spawns[team] = (0..6).map(|i| Vec2::new(x + (i % 2) as f32 * 2.5 * if team == 0 { 1.0 } else { -1.0 }, -12.5 + i as f32 * 5.0)).collect();
        }
        self.spawn = self.team_spawns[0][2];
    }

    fn place_deposit(&mut self, rng: &mut Rng, island: Island, band: (f32, f32), ore: Ore) {
        for _ in 0..40 {
            let p = island.center + Vec2::from_angle(rng.range(0.0, TAU)) * rng.range(band.0, band.1);
            let y = self.height(p.x, p.y);
            let spaced = self.deposits.iter().all(|d| Vec2::new(d.pos.x, d.pos.z).distance(p) > 7.0);
            if y > 0.45 && spaced {
                let amount = match ore { Ore::Iron => 420.0, Ore::Copper => 320.0, Ore::Aether => 200.0 } * rng.range(0.8, 1.3);
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
        let look = self.recipe.biome.look();
        let mut out = TerrainChunk::default();
        let mut rng = Rng::new(self.recipe.seed ^ ((cx as u64) << 20) ^ cz as u64);
        let (cell, half) = (self.recipe.cell, self.recipe.half_extent);
        for iz in cz * chunk_cells..((cz + 1) * chunk_cells).min(self.side - 1) {
            for ix in cx * chunk_cells..((cx + 1) * chunk_cells).min(self.side - 1) {
                let p = |dx: usize, dz: usize| {
                    Vec3::new((ix + dx) as f32 * cell - half, self.heights[(iz + dz) * self.side + ix + dx], (iz + dz) as f32 * cell - half)
                };
                let (a, b, c, d) = (p(0, 0), p(1, 0), p(0, 1), p(1, 1));
                for [a, b, c] in [[a, c, b], [b, c, d]] {
                    let normal = (b - a).cross(c - a).normalize();
                    let y = (a.y + b.y + c.y) / 3.0;
                    let base = if y < 0.74 { look.sand } else if y > 3.3 || normal.y < 0.82 { look.high } else { look.grass };
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
    let (horizon, zenith) = if night {
        (Vec3::new(0.10, 0.13, 0.22), Vec3::new(0.012, 0.018, 0.05))
    } else {
        (Vec3::from(look.horizon), Vec3::from(look.zenith))
    };
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

    #[test]
    fn town_has_buildings_and_clear_team_bases() {
        let w = World::generate(WorldRecipe::town(9, Biome::IronShoals));
        let buildings: Vec<_> = w.props.iter().filter(|p| p.kind == PropKind::Building).collect();
        assert!(buildings.len() >= 24, "only {} buildings", buildings.len());
        for team in 0..2 {
            assert_eq!(w.team_spawns[team].len(), 6);
            for spawn in &w.team_spawns[team] {
                assert!(w.walkable(spawn.x, spawn.y), "spawn {spawn:?} is not on land");
                for prop in &w.props {
                    let half = prop.solid_half().unwrap();
                    let inside = (prop.pos.x - spawn.x).abs() < half.x + 0.4 && (prop.pos.z - spawn.y).abs() < half.z + 0.4;
                    assert!(!inside, "spawn {spawn:?} is inside a {:?}", prop.kind);
                }
            }
        }
        assert!(w.walkable(0.0, 0.0), "the plaza must be open");
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
