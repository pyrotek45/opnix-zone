//! Writes a map's layout as plain text for drawing a top-down overview:
//! `OZ_MAP_DUMP=file cargo test -p oz_core --test dump_map -- --ignored`.
use oz_core::worldgen::{Biome, Layout, Weather, World, WorldRecipe};
use std::fmt::Write;

#[test]
#[ignore]
fn dump_map() {
    let Ok(path) = std::env::var("OZ_MAP_DUMP") else { return };
    let w = World::generate(WorldRecipe::preset(Layout::Mountainside, Biome::IronShoals, Weather::Clear));
    let mut out = String::new();
    for (paint, t) in &w.paving {
        let _ = writeln!(out, "pave {:?} {} {} {} {} {} {}", paint, t[0].x, t[0].z, t[1].x, t[1].z, t[2].x, t[2].z);
    }
    for b in w.blocks.iter().filter(|b| b.solid) {
        let (lo, hi) = (b.aabb.center - b.aabb.half, b.aabb.center + b.aabb.half);
        let g = w.height(b.aabb.center.x, b.aabb.center.z);
        let _ = writeln!(out, "block {:?} {} {} {} {} {} {}", b.paint, lo.x, lo.z, hi.x, hi.z, lo.y - g, hi.y - g);
    }
    for c in &w.clutter {
        let h = c.half();
        let _ = writeln!(out, "clutter {:?} {} {} {} {}", c.kind, c.pos.x, c.pos.z, h.x, h.z);
    }
    for p in &w.props {
        let _ = writeln!(out, "prop {:?} {} {} {}", p.kind, p.pos.x, p.pos.z, p.scale);
    }
    for d in &w.doors {
        let _ = writeln!(out, "door {} {} {}", d.center.x, d.center.z, d.along_x);
    }
    for l in &w.ladders {
        let _ = writeln!(out, "ladder {} {}", l.foot.x, l.foot.z);
    }
    for (team, spawns) in w.team_spawns.iter().enumerate() {
        for s in spawns {
            let _ = writeln!(out, "spawn {team} {} {}", s.x, s.y);
        }
    }
    let _ = writeln!(out, "extent {} {}", w.recipe.half_extent, w.arena);
    std::fs::write(path, out).unwrap();
}
