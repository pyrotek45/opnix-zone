//! Ground-level navigation for AI: a walkability grid built from the map and
//! its static collision, A* search, and line-of-sight path smoothing.
//!
//! The grid is built once per match on the authority. Searches reuse scratch
//! buffers (generation-stamped, so nothing is cleared between searches) and a
//! per-frame budget keeps a crowd of bots from all searching at once.

use crate::player::ground_under;
use bevy::prelude::*;
use oz_core::{collide::StaticWorld, worldgen::World as GameMap};
use std::{cmp::Reverse, collections::BinaryHeap};

const CELL: f32 = 1.0;
const CLEARANCE: f32 = 0.32;
const STRAIGHT: u32 = 10;
const DIAGONAL: u32 = 14;
const MAX_EXPANDED: usize = 40_000;

#[derive(Resource)]
pub struct Nav {
    half: f32,
    side: usize,
    walk: Vec<bool>,
    /// Ground height per cell, used to reject cliffs between neighbours.
    floor: Vec<f32>,
}

/// Reusable A* buffers.
#[derive(Resource, Default)]
pub struct NavScratch {
    stamp: Vec<u32>,
    generation: u32,
    cost: Vec<u32>,
    came: Vec<u32>,
    closed: Vec<u32>,
}

/// Searches allowed this frame; refilled every frame.
#[derive(Resource)]
pub struct NavBudget(pub u32);

impl Default for NavBudget {
    fn default() -> Self {
        Self(3)
    }
}

pub fn refill_budget(mut budget: ResMut<NavBudget>) {
    budget.0 = 3;
}

impl Nav {
    pub fn build(map: &GameMap, statics: &StaticWorld) -> Nav {
        let half = map.recipe.half_extent - 4.0;
        let side = (2.0 * half / CELL) as usize;
        let mut walk = vec![false; side * side];
        let mut floor = vec![0.0; side * side];
        for z in 0..side {
            for x in 0..side {
                let p = Vec2::new(-half + (x as f32 + 0.5) * CELL, -half + (z as f32 + 0.5) * CELL);
                let i = z * side + x;
                if !map.walkable(p.x, p.y) {
                    continue;
                }
                let terrain = map.height(p.x, p.y);
                let ground = ground_under(map, statics, p.x, p.y, terrain);
                floor[i] = ground;
                walk[i] = !statics.blocked(p.x, p.y, ground, 1.7, CLEARANCE);
            }
        }
        Nav { half, side, walk, floor }
    }

    pub fn walkable_cells(&self) -> usize {
        self.walk.iter().filter(|w| **w).count()
    }

    fn index(&self, p: Vec2) -> Option<usize> {
        let x = ((p.x + self.half) / CELL).floor();
        let z = ((p.y + self.half) / CELL).floor();
        (x >= 0.0 && z >= 0.0 && (x as usize) < self.side && (z as usize) < self.side).then(|| z as usize * self.side + x as usize)
    }

    fn center(&self, i: usize) -> Vec2 {
        Vec2::new(-self.half + ((i % self.side) as f32 + 0.5) * CELL, -self.half + ((i / self.side) as f32 + 0.5) * CELL)
    }

    pub fn walkable(&self, p: Vec2) -> bool {
        self.index(p).is_some_and(|i| self.walk[i])
    }

    /// The closest walkable cell within a few metres.
    fn nearest(&self, p: Vec2) -> Option<usize> {
        let i = self.index(p)?;
        if self.walk[i] {
            return Some(i);
        }
        let (cx, cz) = ((i % self.side) as i32, (i / self.side) as i32);
        for radius in 1..6i32 {
            let mut best: Option<(f32, usize)> = None;
            for dz in -radius..=radius {
                for dx in -radius..=radius {
                    if dx.abs() != radius && dz.abs() != radius {
                        continue;
                    }
                    let (x, z) = (cx + dx, cz + dz);
                    if x < 0 || z < 0 || x >= self.side as i32 || z >= self.side as i32 {
                        continue;
                    }
                    let j = z as usize * self.side + x as usize;
                    if self.walk[j] {
                        let d = self.center(j).distance_squared(p);
                        if best.is_none_or(|(bd, _)| d < bd) {
                            best = Some((d, j));
                        }
                    }
                }
            }
            if let Some((_, j)) = best {
                return Some(j);
            }
        }
        None
    }

    /// Can you walk straight from `a` to `b` without leaving walkable cells?
    pub fn line_clear(&self, a: Vec2, b: Vec2) -> bool {
        let span = b - a;
        let steps = (span.length() / (CELL * 0.35)).ceil().max(1.0) as usize;
        let mut last_floor: Option<f32> = None;
        for i in 0..=steps {
            let p = a + span * (i as f32 / steps as f32);
            let Some(cell) = self.index(p) else { return false };
            if !self.walk[cell] {
                return false;
            }
            if let Some(prev) = last_floor
                && (self.floor[cell] - prev).abs() > 0.8 {
                    return false;
                }
            last_floor = Some(self.floor[cell]);
        }
        true
    }

    /// A* from `from` to `to`, then smoothed to as few straight legs as possible.
    pub fn path(&self, scratch: &mut NavScratch, from: Vec2, to: Vec2) -> Option<Vec<Vec2>> {
        let (start, goal) = (self.nearest(from)?, self.nearest(to)?);
        if start == goal {
            return Some(vec![to]);
        }
        let n = self.walk.len();
        if scratch.stamp.len() != n {
            *scratch = NavScratch { stamp: vec![0; n], generation: 0, cost: vec![0; n], came: vec![0; n], closed: vec![0; n] };
        }
        scratch.generation = scratch.generation.wrapping_add(1).max(1);
        let generation = scratch.generation;
        let side = self.side as i32;
        let goal_xy = ((goal % self.side) as i32, (goal / self.side) as i32);
        let heuristic = |i: usize| {
            let (dx, dz) = (((i % self.side) as i32 - goal_xy.0).unsigned_abs(), ((i / self.side) as i32 - goal_xy.1).unsigned_abs());
            STRAIGHT * dx.max(dz) + (DIAGONAL - STRAIGHT) * dx.min(dz)
        };

        let mut open = BinaryHeap::new();
        scratch.stamp[start] = generation;
        scratch.cost[start] = 0;
        scratch.came[start] = start as u32;
        open.push(Reverse((heuristic(start), start as u32)));
        let mut expanded = 0;
        let mut found = false;
        while let Some(Reverse((_, current))) = open.pop() {
            let current = current as usize;
            if scratch.closed[current] == generation {
                continue;
            }
            scratch.closed[current] = generation;
            if current == goal {
                found = true;
                break;
            }
            expanded += 1;
            if expanded > MAX_EXPANDED {
                break;
            }
            let (cx, cz) = ((current % self.side) as i32, (current / self.side) as i32);
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
                let (x, z) = (cx + dx, cz + dz);
                if x < 0 || z < 0 || x >= side || z >= side {
                    continue;
                }
                let next = (z * side + x) as usize;
                if !self.walk[next] || (self.floor[next] - self.floor[current]).abs() > 0.8 {
                    continue;
                }
                let diagonal = dx != 0 && dz != 0;
                // No cutting corners past walls.
                if diagonal && (!self.walk[(cz * side + x) as usize] || !self.walk[(z * side + cx) as usize]) {
                    continue;
                }
                let cost = scratch.cost[current] + if diagonal { DIAGONAL } else { STRAIGHT };
                if scratch.stamp[next] != generation || cost < scratch.cost[next] {
                    scratch.stamp[next] = generation;
                    scratch.cost[next] = cost;
                    scratch.came[next] = current as u32;
                    open.push(Reverse((cost + heuristic(next), next as u32)));
                }
            }
        }
        if !found {
            return None;
        }

        let mut cells = vec![goal];
        let mut at = goal;
        while at != start {
            at = scratch.came[at] as usize;
            cells.push(at);
        }
        cells.reverse();

        // String-pull: from each anchor, jump to the furthest cell still in straight sight.
        let points: Vec<Vec2> = cells.iter().map(|i| self.center(*i)).collect();
        let mut smooth = Vec::new();
        let mut anchor = from;
        let mut i = 0;
        while i < points.len() {
            let mut furthest = i;
            for j in (i..points.len()).rev() {
                if self.line_clear(anchor, points[j]) {
                    furthest = j;
                    break;
                }
            }
            anchor = points[furthest];
            smooth.push(anchor);
            i = furthest + 1;
        }
        if self.walkable(to) && smooth.last().is_some_and(|last| self.line_clear(*last, to)) {
            smooth.push(to);
        }
        Some(smooth)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oz_core::{
        collide::Aabb,
        worldgen::{Biome, WorldRecipe},
    };

    #[test]
    fn finds_a_way_around_a_wall_and_through_the_town() {
        let map = GameMap::generate(WorldRecipe::town(9, Biome::IronShoals));
        let mut statics = StaticWorld::new(map.recipe.half_extent);
        for aabb in map.solids() {
            statics.add(aabb);
        }
        let nav = Nav::build(&map, &statics);
        let mut scratch = NavScratch::default();
        assert!(nav.walkable_cells() > 5_000);
        let (a, b) = (map.team_spawns[0][2], map.team_spawns[1][2]);
        let path = nav.path(&mut scratch, a, b).expect("bases must be connected");
        assert!(path.len() >= 2 && path.last().unwrap().distance(b) < 1.5);
        // Every leg of the smoothed path is walkable in a straight line.
        let mut at = a;
        for p in &path {
            assert!(nav.line_clear(at, *p) || at.distance(*p) < 1.5, "blocked leg {at:?} -> {p:?}");
            at = *p;
        }

        // A plain wall: the path must go around it.
        let flat = GameMap::generate(WorldRecipe::town(3, Biome::IronShoals));
        let mut walls = StaticWorld::new(flat.recipe.half_extent);
        walls.add(Aabb::span(Vec3::new(-0.5, 0.0, -20.0), Vec3::new(0.5, 6.0, 20.0)));
        let nav = Nav::build(&flat, &walls);
        let route = nav.path(&mut scratch, Vec2::new(-6.0, 0.0), Vec2::new(6.0, 0.0)).unwrap();
        assert!(route.iter().any(|p| p.y.abs() > 19.0), "path should detour around the wall end: {route:?}");
    }
}
