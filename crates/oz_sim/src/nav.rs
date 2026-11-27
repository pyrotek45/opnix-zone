//! Navigation for AI on every floor: a layered walkability grid built from
//! the map and its static collision, A* search, and line-of-sight path
//! smoothing.
//!
//! Each half-metre column of the map holds one node per surface you can
//! stand on there: the ground, a stair step, an upper floor, a rooftop.
//! Neighbouring nodes connect when the height difference is a step you can
//! walk, so stairs join floors and walls with a floor above them do not.
//!
//! The grid is built once per match on the authority. Searches reuse scratch
//! buffers (generation-stamped, so nothing is cleared between searches) and a
//! per-frame budget keeps a crowd of bots from all searching at once.

use bevy::prelude::*;
use oz_core::{collide::StaticWorld, worldgen::World as GameMap};
use std::{cmp::Reverse, collections::BinaryHeap};

const CELL: f32 = 0.5;
const CLEARANCE: f32 = 0.3;
/// Tallest height change between neighbouring nodes: two stair steps can fall in one half-metre.
const MAX_RISE: f32 = 0.65;
const HEADROOM: f32 = 1.7;
const STRAIGHT: u32 = 10;
const DIAGONAL: u32 = 14;
const MAX_EXPANDED: usize = 150_000;

#[derive(Resource)]
pub struct Nav {
    half: f32,
    side: usize,
    /// Nodes of column `c` are `start[c]..start[c + 1]`, lowest first.
    start: Vec<u32>,
    /// Standing height of each node.
    floor: Vec<f32>,
    /// Column of each node.
    column: Vec<u32>,
    /// Ladders: (foot node, roof node, climb cost). Bots climb up them, and come back down
    /// the short ones by dropping off the edge.
    links: Vec<(u32, u32, u32, bool)>,
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
        let mut start = Vec::with_capacity(side * side + 1);
        let mut floor = Vec::new();
        let mut column = Vec::new();
        let mut heights: Vec<f32> = Vec::new();
        for z in 0..side {
            for x in 0..side {
                start.push(floor.len() as u32);
                let p = Vec2::new(-half + (x as f32 + 0.5) * CELL, -half + (z as f32 + 0.5) * CELL);
                if !map.walkable(p.x, p.y) {
                    continue;
                }
                let terrain = map.height(p.x, p.y).max(map.water - oz_core::worldgen::WADE_DEPTH);
                heights.clear();
                heights.push(terrain);
                heights.extend(statics.tops_at(p.x, p.y).into_iter().filter(|top| *top > terrain - 0.05));
                heights.sort_by(f32::total_cmp);
                // Surfaces closer together than a step are one surface: you stand on the higher.
                let mut merged: Vec<f32> = Vec::new();
                for h in heights.iter().copied() {
                    match merged.last_mut() {
                        Some(last) if h - *last < oz_core::collide::STEP => *last = h,
                        _ => merged.push(h),
                    }
                }
                for h in merged {
                    if !statics.blocked(p.x, p.y, h, HEADROOM, CLEARANCE) {
                        floor.push(h);
                        column.push((z * side + x) as u32);
                    }
                }
            }
        }
        start.push(floor.len() as u32);
        let mut nav = Nav { half, side, start, floor, column, links: Vec::new() };
        for ladder in &map.ladders {
            let out = Vec3::new(ladder.out.x, 0.0, ladder.out.y);
            let foot = ladder.foot + out * crate::player::LADDER_STANDOFF;
            let roof = Vec3::new(ladder.foot.x, ladder.top, ladder.foot.z) - out * 0.9;
            let bottom = nav.column_at(foot.xz()).and_then(|c| nav.nodes(c).min_by(|a, b| (nav.floor[*a] - foot.y).abs().total_cmp(&(nav.floor[*b] - foot.y).abs())));
            let top = nav.column_at(roof.xz()).and_then(|c| nav.nodes(c).find(|n| (nav.floor[*n] - ladder.top).abs() < 0.4));
            if let (Some(bottom), Some(top)) = (bottom, top)
                && (nav.floor[bottom] - foot.y).abs() < 0.8
            {
                let climb = ((ladder.top - foot.y).max(0.0) / CELL) as u32 * STRAIGHT * 2 + STRAIGHT;
                // Dropping back down is only sensible off a low roof.
                let drop = ladder.top - foot.y < 4.5;
                nav.links.push((bottom as u32, top as u32, climb, drop));
            }
        }
        nav
    }

    pub fn walkable_cells(&self) -> usize {
        self.floor.len()
    }

    fn column_at(&self, p: Vec2) -> Option<usize> {
        let x = ((p.x + self.half) / CELL).floor();
        let z = ((p.y + self.half) / CELL).floor();
        (x >= 0.0 && z >= 0.0 && (x as usize) < self.side && (z as usize) < self.side).then(|| z as usize * self.side + x as usize)
    }

    fn nodes(&self, column: usize) -> std::ops::Range<usize> {
        self.start[column] as usize..self.start[column + 1] as usize
    }

    fn center(&self, node: usize) -> Vec3 {
        let c = self.column[node] as usize;
        Vec3::new(-self.half + ((c % self.side) as f32 + 0.5) * CELL, self.floor[node], -self.half + ((c / self.side) as f32 + 0.5) * CELL)
    }

    /// The node in `column` you can step to from height `from`, if any.
    fn step_into(&self, column: usize, from: f32) -> Option<usize> {
        self.nodes(column).filter(|n| (self.floor[*n] - from).abs() <= MAX_RISE).min_by(|a, b| (self.floor[*a] - from).abs().total_cmp(&(self.floor[*b] - from).abs()))
    }

    /// Is there a surface to stand on near this point (at about its height)?
    pub fn walkable(&self, p: Vec3) -> bool {
        self.column_at(p.xz()).is_some_and(|c| self.nodes(c).any(|n| (self.floor[n] - p.y).abs() < 1.0))
    }

    /// The closest node within a few metres, preferring the same floor.
    fn nearest(&self, p: Vec3) -> Option<usize> {
        let c = self.column_at(p.xz())?;
        let (cx, cz) = ((c % self.side) as i32, (c / self.side) as i32);
        let score = |n: usize| {
            let at = self.center(n);
            let dy = at.y - p.y;
            // Being under the point is fine (we fall); being well above it is not.
            at.xz().distance_squared(p.xz()) + if dy > 0.6 { dy * dy * 25.0 } else { dy * dy * 2.0 }
        };
        let mut best: Option<(f32, usize)> = None;
        for radius in 0..12i32 {
            for dz in -radius..=radius {
                for dx in -radius..=radius {
                    if dx.abs() != radius && dz.abs() != radius {
                        continue;
                    }
                    let (x, z) = (cx + dx, cz + dz);
                    if x < 0 || z < 0 || x >= self.side as i32 || z >= self.side as i32 {
                        continue;
                    }
                    for n in self.nodes(z as usize * self.side + x as usize) {
                        let s = score(n);
                        if best.is_none_or(|(bs, _)| s < bs) {
                            best = Some((s, n));
                        }
                    }
                }
            }
            // Anything found within this ring beats anything further out.
            if best.is_some_and(|(s, _)| s < ((radius + 1) as f32 * CELL).powi(2)) {
                break;
            }
        }
        best.map(|(_, n)| n)
    }

    /// Can you walk straight from `a` to `b` (staying on walkable ground, stepping
    /// only up and down steps)?
    pub fn line_clear(&self, a: Vec3, b: Vec3) -> bool {
        let Some(mut node) = self.nearest(a) else { return false };
        let span = b.xz() - a.xz();
        let steps = (span.length() / (CELL * 0.35)).ceil().max(1.0) as usize;
        for i in 1..=steps {
            let p = a.xz() + span * (i as f32 / steps as f32);
            let Some(column) = self.column_at(p) else { return false };
            if self.column[node] as usize == column {
                continue;
            }
            match self.step_into(column, self.floor[node]) {
                Some(next) => node = next,
                None => return false,
            }
        }
        (self.floor[node] - b.y).abs() < 1.2
    }

    /// A* from `from` to `to`, then smoothed to as few straight legs as possible.
    pub fn path(&self, scratch: &mut NavScratch, from: Vec3, to: Vec3) -> Option<Vec<Vec3>> {
        let (start, goal) = (self.nearest(from)?, self.nearest(to)?);
        if start == goal {
            return Some(vec![to]);
        }
        let n = self.floor.len();
        if scratch.stamp.len() != n {
            *scratch = NavScratch { stamp: vec![0; n], generation: 0, cost: vec![0; n], came: vec![0; n], closed: vec![0; n] };
        }
        scratch.generation = scratch.generation.wrapping_add(1).max(1);
        let generation = scratch.generation;
        let side = self.side as i32;
        let goal_column = self.column[goal] as usize;
        let goal_xz = ((goal_column % self.side) as i32, (goal_column / self.side) as i32);
        let goal_y = self.floor[goal];
        let heuristic = |node: usize| {
            let c = self.column[node] as usize;
            let (dx, dz) = (((c % self.side) as i32 - goal_xz.0).unsigned_abs(), ((c / self.side) as i32 - goal_xz.1).unsigned_abs());
            // Changing floors costs at least the climb.
            let climb = ((self.floor[node] - goal_y).abs() / CELL * STRAIGHT as f32 * 0.5) as u32;
            STRAIGHT * dx.max(dz) + (DIAGONAL - STRAIGHT) * dx.min(dz) + climb
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
            let here = self.floor[current];
            let c = self.column[current] as usize;
            let (cx, cz) = ((c % self.side) as i32, (c / self.side) as i32);
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
                let (x, z) = (cx + dx, cz + dz);
                if x < 0 || z < 0 || x >= side || z >= side {
                    continue;
                }
                let Some(next) = self.step_into((z * side + x) as usize, here) else { continue };
                let diagonal = dx != 0 && dz != 0;
                // No cutting corners past walls.
                if diagonal && (self.step_into((cz * side + x) as usize, here).is_none() || self.step_into((z * side + cx) as usize, here).is_none()) {
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
            // Up a ladder (or back down a short one).
            for &(bottom, top, climb, drop) in &self.links {
                let next = if bottom as usize == current {
                    top as usize
                } else if drop && top as usize == current {
                    bottom as usize
                } else {
                    continue;
                };
                let cost = scratch.cost[current] + climb;
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

        let mut nodes = vec![goal];
        let mut at = goal;
        while at != start {
            at = scratch.came[at] as usize;
            nodes.push(at);
        }
        nodes.reverse();

        // String-pull: from each anchor, jump to the furthest node still in straight sight.
        let points: Vec<Vec3> = nodes.iter().map(|n| self.center(*n)).collect();
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
        let ground = |p: Vec2| Vec3::new(p.x, map.height(p.x, p.y), p.y);
        let (a, b) = (ground(map.team_spawns[0][2]), ground(map.team_spawns[1][2]));
        let path = nav.path(&mut scratch, a, b).expect("bases must be connected");
        assert!(path.len() >= 2 && path.last().unwrap().xz().distance(b.xz()) < 1.5);
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
        let at = |x: f32| Vec3::new(x, flat.height(x, 0.0), 0.0);
        let route = nav.path(&mut scratch, at(-6.0), at(6.0)).unwrap();
        assert!(route.iter().any(|p| p.z.abs() > 19.0), "path should detour around the wall end: {route:?}");
    }

    #[test]
    fn bots_can_cross_every_map() {
        use oz_core::worldgen::{Layout, Theme, Weather};
        let mut recipes: Vec<WorldRecipe> = Layout::VERSUS.into_iter().filter(|l| *l != Layout::Town).map(|l| WorldRecipe::preset(l, Biome::IronShoals, Weather::Clear)).collect();
        recipes.extend(Theme::ALL.map(|t| WorldRecipe::themed(21, Biome::IronShoals, t)));
        for recipe in recipes {
            let what = format!("{:?} {:?}", recipe.layout, oz_core::worldgen::Theme::from_seed(recipe.seed));
            let map = GameMap::generate(recipe);
            let mut statics = StaticWorld::new(map.recipe.half_extent);
            for aabb in map.solids() {
                statics.add(aabb);
            }
            let nav = Nav::build(&map, &statics);
            let mut scratch = NavScratch::default();
            let ground = |p: Vec2| Vec3::new(p.x, map.height(p.x, p.y), p.y);
            let (a, b) = (ground(map.team_spawns[0][2]), ground(map.team_spawns[1][2]));
            let path = nav.path(&mut scratch, a, b).unwrap_or_else(|| panic!("{what}: the bases are not connected"));
            assert!(path.last().unwrap().xz().distance(b.xz()) < 1.5, "{what}");
            // Indoors, bots sweep the waypoints: nearly all of them must be reachable.
            if !map.waypoints.is_empty() {
                let mut reached = 0;
                let sample: Vec<Vec3> = map.waypoints.iter().step_by((map.waypoints.len() / 24).max(1)).copied().collect();
                for p in &sample {
                    if nav.path(&mut scratch, a, *p).is_some_and(|path| path.last().unwrap().distance(*p) < 1.5) {
                        reached += 1;
                    }
                }
                assert!(reached * 10 >= sample.len() * 9, "{what}: only {reached} of {} waypoints reachable", sample.len());
            }
        }
    }

    #[test]
    fn ladders_lead_up_onto_roofs() {
        // Find a town with a ladder, and path from its foot to the roof it reaches.
        for seed in 1..40 {
            let map = GameMap::generate(WorldRecipe::town(seed, Biome::IronShoals));
            let Some(ladder) = map.ladders.first().copied() else { continue };
            let mut statics = StaticWorld::new(map.recipe.half_extent);
            for aabb in map.solids() {
                statics.add(aabb);
            }
            let nav = Nav::build(&map, &statics);
            if nav.links.is_empty() {
                continue;
            }
            let out = Vec3::new(ladder.out.x, 0.0, ladder.out.y);
            let from = ladder.foot + out * 3.0;
            let to = Vec3::new(ladder.foot.x, ladder.top, ladder.foot.z) - out * 1.5;
            let path = nav.path(&mut NavScratch::default(), from, to).expect("a way up");
            assert!(path.iter().any(|p| p.y > ladder.top - 0.5), "the path should reach the roof: {path:?}");
            return;
        }
        panic!("no town had a usable ladder");
    }

    #[test]
    fn stairs_lead_to_the_upper_floors() {
        let map = GameMap::generate(WorldRecipe::town(9, Biome::IronShoals));
        let mut statics = StaticWorld::new(map.recipe.half_extent);
        for aabb in map.solids() {
            statics.add(aabb);
        }
        let nav = Nav::build(&map, &statics);
        let mut scratch = NavScratch::default();
        // Upper floors: a storey over the ground with a roof still above them.
        let upstairs: Vec<Vec3> = (0..nav.floor.len())
            .filter(|n| {
                let column = nav.nodes(nav.column[*n] as usize);
                let below = column.clone().any(|m| (nav.floor[*n] - nav.floor[m] - 3.2).abs() < 0.2);
                let above = column.clone().any(|m| nav.floor[m] > nav.floor[*n] + 2.5);
                below && above
            })
            .map(|n| nav.center(n))
            .collect();
        assert!(upstairs.len() > 50, "no upper floors in the nav grid");
        let start = map.team_spawns[0][2];
        let start = Vec3::new(start.x, map.height(start.x, start.y), start.y);
        // Most upstairs rooms can be walked to (a few nodes are cut off, like the top of a cupboard).
        let mut reached = 0;
        for target in upstairs.iter().step_by(upstairs.len() / 12) {
            if let Some(path) = nav.path(&mut scratch, start, *target) {
                let end = *path.last().unwrap();
                if end.distance(*target) < 1.0 {
                    reached += 1;
                    // The route climbs: somewhere along it the height goes up a floor.
                    assert!(path.iter().any(|p| p.y > start.y + 2.5));
                }
            }
        }
        assert!(reached >= 6, "only {reached} of 12 upstairs spots were reachable");
    }
}
