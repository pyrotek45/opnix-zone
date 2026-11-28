//! Block models: every visual asset in the game is a list of painted primitives.
//!
//! A [`BlockModel`] is pure data. The client turns it into meshes; nothing is
//! ever loaded from disk. Parts can carry an [`Anim`] tag so the client can
//! animate them procedurally (walking legs, spinning rotors, recoiling barrels).

use glam::{EulerRot, Quat, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Shape {
    Cube,
    /// Faceted low-poly sphere; the signature "rock / crystal" primitive.
    Shard,
    Cylinder,
    Cone,
}

/// Shared palette. Keeping colours here gives every game the same look by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Paint {
    Dark, Metal, Gold, Wood, Teal, Coral, Purple, Shell, Bone, Steel, Rock, Leaf, LeafLight,
    Concrete, Rust, Sand, Iron, Copper,
    // Unlit / emissive paints.
    Glow, Spark, Red, Green, Blue, Violet, Aether, White,
    /// A dark, faintly glowing display surface (robot faces, terminals).
    Screen,
    // Interior and street finishes (lit), added after the rest so older indices hold.
    /// Honey-coloured floorboards.
    Planks,
    /// Pale blue-grey floor and wall tiles (kitchens, bathrooms).
    Tile,
    /// Deep wine carpet (bedrooms, offices).
    Carpet,
    /// Sea-green linoleum (shops, halls).
    Lino,
    /// Red-brown brick.
    Brick,
    /// Dark blue-grey stone and slate.
    Slate,
    /// Warm off-white trim, linen and paint.
    Cream,
    /// Dark navy fabric (sofas, upholstery, awnings).
    Navy,
    /// Mustard yellow (cushions, signs, playground paint).
    Mustard,
}

impl Paint {
    pub const ALL: [Paint; 36] = [
        Paint::Dark, Paint::Metal, Paint::Gold, Paint::Wood, Paint::Teal, Paint::Coral, Paint::Purple,
        Paint::Shell, Paint::Bone, Paint::Steel, Paint::Rock, Paint::Leaf, Paint::LeafLight,
        Paint::Concrete, Paint::Rust, Paint::Sand, Paint::Iron, Paint::Copper,
        Paint::Glow, Paint::Spark, Paint::Red, Paint::Green, Paint::Blue, Paint::Violet, Paint::Aether, Paint::White,
        Paint::Screen, Paint::Planks, Paint::Tile, Paint::Carpet, Paint::Lino, Paint::Brick, Paint::Slate, Paint::Cream, Paint::Navy,
        Paint::Mustard,
    ];

    /// sRGB colour as `0xRRGGBB`.
    pub fn hex(self) -> u32 {
        match self {
            Paint::Dark => 0x26354b, Paint::Metal => 0x78939b, Paint::Gold => 0xf4ae61,
            Paint::Wood => 0x78504b, Paint::Teal => 0x216d73, Paint::Coral => 0xda7766,
            Paint::Purple => 0x8354a1, Paint::Shell => 0x396b64, Paint::Bone => 0xd7c1a0,
            Paint::Steel => 0x3b4c60, Paint::Rock => 0x637477, Paint::Leaf => 0x26635b,
            Paint::LeafLight => 0x6eaa64, Paint::Concrete => 0xd3a97c, Paint::Rust => 0xba654d,
            Paint::Sand => 0xe3b06e, Paint::Iron => 0x8a8f9c, Paint::Copper => 0xd9824a,
            Paint::Glow => 0xffcb65, Paint::Spark => 0xffedbb, Paint::Red => 0xff535e,
            Paint::Green => 0x7fffa0, Paint::Blue => 0x63ddff, Paint::Violet => 0xc58bff,
            Paint::Aether => 0x9df7e4, Paint::White => 0xffffff, Paint::Screen => 0x0c2430,
            Paint::Planks => 0xb27a45, Paint::Tile => 0xb4cbd0, Paint::Carpet => 0x7a3149, Paint::Lino => 0x4f8a7c,
            Paint::Brick => 0x9a4636, Paint::Slate => 0x4a5562, Paint::Cream => 0xeee0c2, Paint::Navy => 0x2c3a63,
            Paint::Mustard => 0xd9a43a,
        }
    }

    pub fn rgb(self) -> [f32; 3] {
        let h = self.hex();
        [(h >> 16) as u8 as f32 / 255.0, (h >> 8) as u8 as f32 / 255.0, h as u8 as f32 / 255.0]
    }

    /// How far apart two paints look: distance between their sRGB colours (0..=441).
    pub fn contrast(self, other: Paint) -> f32 {
        let (a, b) = (self.rgb(), other.rgb());
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt() * 255.0
    }

    /// Unlit paints ignore lighting and read as glowing, even at night.
    pub fn unlit(self) -> bool {
        matches!(self, Paint::Glow | Paint::Spark | Paint::Red | Paint::Green | Paint::Blue | Paint::Violet | Paint::Aether | Paint::White | Paint::Screen)
    }
}

/// Procedural animation hint for a part. Interpreted by the client only.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Anim {
    None,
    /// Swings back and forth while the owner moves; `phase` offsets the cycle (0..1).
    Leg { phase: f32 },
    /// Continuous spin around local Y, radians per second.
    Spin { speed: f32 },
    /// Gentle scale pulse, used for cores and eyes.
    Pulse,
    /// Vertical hover wobble.
    Bob,
    /// Kicks back along +Z when the owner attacks.
    Recoil,
    /// A weapon's magazine: removed and replaced during reloads.
    Mag,
    /// A weapon's moving action: slide, bolt, pump or charging handle.
    Slide,
}

#[derive(Clone, Copy, Debug)]
pub struct Part {
    pub shape: Shape,
    pub paint: Paint,
    pub pos: Vec3,
    pub size: Vec3,
    pub rot: Quat,
    pub anim: Anim,
}

/// A solid wrapped tight round a cloud of points (OpenSCAD's `hull()`): the way to fuse
/// boxes and cylinders that meet at odd angles into one clean piece, with no gaps or
/// notches where they join. Drawn faceted, in one paint.
#[derive(Clone, Debug, PartialEq)]
pub struct Hull {
    pub paint: Paint,
    pub points: Vec<Vec3>,
    pub anim: Anim,
}

/// A list of parts in model space: +Y up, -Z forward, origin at the feet.
#[derive(Clone, Debug, Default)]
pub struct BlockModel {
    pub parts: Vec<Part>,
    /// Fused solids (see [`BlockModel::hull`]).
    pub hulls: Vec<Hull>,
}

/// Corner points of a part, in model space (a cylinder's rims, a ball's extremes).
pub fn part_points(part: &Part) -> Vec<Vec3> {
    let local: Vec<Vec3> = match part.shape {
        Shape::Cube => {
            let mut corners = Vec::with_capacity(8);
            for x in [-0.5, 0.5] {
                for y in [-0.5, 0.5] {
                    for z in [-0.5, 0.5] {
                        corners.push(Vec3::new(x, y, z));
                    }
                }
            }
            corners
        }
        Shape::Cylinder | Shape::Cone => {
            // Eight sides, like the drawn primitive; a cone's top is its point.
            let mut rims = Vec::with_capacity(16);
            for i in 0..8 {
                let a = (i as f32 + 0.5) * std::f32::consts::TAU / 8.0;
                let (x, z) = (a.cos() * 0.5, a.sin() * 0.5);
                rims.push(Vec3::new(x, -0.5, z));
                rims.push(if part.shape == Shape::Cone { Vec3::new(0.0, 0.5, 0.0) } else { Vec3::new(x, 0.5, z) });
            }
            rims
        }
        Shape::Shard => vec![Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z],
    };
    local.into_iter().map(|p| part.pos + part.rot * (p * part.size)).collect()
}

/// Outward-facing triangles of the convex hull of `points` (empty if they are flat).
pub fn convex_hull(points: &[Vec3]) -> Vec<[Vec3; 3]> {
    let eps = 1e-6;
    if points.len() < 4 {
        return Vec::new();
    }
    // A starting tetrahedron from extreme points.
    let a = 0;
    let b = (0..points.len()).max_by(|i, j| points[*i].distance_squared(points[a]).total_cmp(&points[*j].distance_squared(points[a]))).unwrap_or(0);
    let line = (points[b] - points[a]).normalize_or_zero();
    let off_line = |p: Vec3| {
        let d = p - points[a];
        (d - line * d.dot(line)).length_squared()
    };
    let c = (0..points.len()).max_by(|i, j| off_line(points[*i]).total_cmp(&off_line(points[*j]))).unwrap_or(0);
    let normal = (points[b] - points[a]).cross(points[c] - points[a]).normalize_or_zero();
    let d = (0..points.len()).max_by(|i, j| (points[*i] - points[a]).dot(normal).abs().total_cmp(&(points[*j] - points[a]).dot(normal).abs())).unwrap_or(0);
    if line == Vec3::ZERO || normal == Vec3::ZERO || (points[d] - points[a]).dot(normal).abs() < 1e-5 {
        return Vec::new();
    }
    let centre = (points[a] + points[b] + points[c] + points[d]) * 0.25;
    let facing = |f: [usize; 3]| (points[f[1]] - points[f[0]]).cross(points[f[2]] - points[f[0]]);
    let outward = |f: [usize; 3]| if facing(f).dot(points[f[0]] - centre) < 0.0 { [f[0], f[2], f[1]] } else { f };
    let mut faces: Vec<[usize; 3]> = [[a, b, c], [a, b, d], [a, c, d], [b, c, d]].into_iter().map(outward).collect();
    for (i, p) in points.iter().enumerate() {
        if [a, b, c, d].contains(&i) {
            continue;
        }
        let sees = |f: &[usize; 3]| facing(*f).normalize_or_zero().dot(*p - points[f[0]]) > eps;
        if !faces.iter().any(sees) {
            continue;
        }
        let (lit, kept): (Vec<[usize; 3]>, Vec<[usize; 3]>) = faces.into_iter().partition(sees);
        // The horizon: edges of the faces this point sees that no other seen face shares.
        let edges: Vec<(usize, usize)> = lit.iter().flat_map(|f| [(f[0], f[1]), (f[1], f[2]), (f[2], f[0])]).collect();
        faces = kept;
        for &(u, v) in &edges {
            if !edges.contains(&(v, u)) {
                faces.push([u, v, i]);
            }
        }
    }
    faces.into_iter().map(|f| [points[f[0]], points[f[1]], points[f[2]]]).collect()
}

impl BlockModel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn part(mut self, shape: Shape, paint: Paint, pos: [f32; 3], size: [f32; 3]) -> Self {
        self.parts.push(Part { shape, paint, pos: pos.into(), size: size.into(), rot: Quat::IDENTITY, anim: Anim::None });
        self
    }

    pub fn cube(self, paint: Paint, pos: [f32; 3], size: [f32; 3]) -> Self {
        self.part(Shape::Cube, paint, pos, size)
    }

    pub fn shard(self, paint: Paint, pos: [f32; 3], size: [f32; 3]) -> Self {
        self.part(Shape::Shard, paint, pos, size)
    }

    pub fn cylinder(self, paint: Paint, pos: [f32; 3], size: [f32; 3]) -> Self {
        self.part(Shape::Cylinder, paint, pos, size)
    }

    pub fn cone(self, paint: Paint, pos: [f32; 3], size: [f32; 3]) -> Self {
        self.part(Shape::Cone, paint, pos, size)
    }

    /// A box stretched between two points, like a limb or a strut.
    pub fn segment(mut self, paint: Paint, from: [f32; 3], to: [f32; 3], thickness: f32) -> Self {
        let (from, to) = (Vec3::from(from), Vec3::from(to));
        let span = to - from;
        let length = span.length().max(1e-4);
        self.parts.push(Part {
            shape: Shape::Cube,
            paint,
            pos: from + span * 0.5,
            size: Vec3::new(thickness, length, thickness),
            rot: Quat::from_rotation_arc(Vec3::Y, span / length),
            anim: Anim::None,
        });
        self
    }

    /// A flat bar in the side (YZ) plane at `x`, from `from` to `to` (each `[y, z]`), `width` across
    /// and `thick` deep. The way to draw sloped faces, raked stocks and struts.
    pub fn plate(self, paint: Paint, x: f32, from: [f32; 2], to: [f32; 2], width: f32, thick: f32) -> Self {
        let (dy, dz) = (to[0] - from[0], to[1] - from[1]);
        let length = (dy * dy + dz * dz).sqrt().max(1e-4);
        self.cube(paint, [x, (from[0] + to[0]) * 0.5, (from[1] + to[1]) * 0.5], [width, thick, length]).rotated([(-dy).atan2(dz).to_degrees(), 0.0, 0.0])
    }

    /// The model's bounding box in model space (min, max), if it has anything in it.
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let points = self.parts.iter().flat_map(part_points).chain(self.hulls.iter().flat_map(|h| h.points.iter().copied()));
        points.fold(None, |acc, p| Some(acc.map_or((p, p), |(lo, hi): (Vec3, Vec3)| (lo.min(p), hi.max(p)))))
    }

    /// Keeps only the parts whose animation tag passes `keep`.
    pub fn filtered(&self, keep: impl Fn(Anim) -> bool) -> BlockModel {
        BlockModel { parts: self.parts.iter().copied().filter(|p| keep(p.anim)).collect(), hulls: self.hulls.iter().filter(|h| keep(h.anim)).cloned().collect() }
    }

    /// Fuses the last `count` parts into one solid shrink-wrapped round them, in the first
    /// one's paint (like OpenSCAD's `hull()`). Use it where pieces meet at an angle and leave
    /// a gap or a notch - a stock's comb and toe, a grip and its frame. Only fuse parts that
    /// never move apart: an animated magazine or slide must stay a part of its own.
    pub fn hull(mut self, count: usize) -> Self {
        let from = self.parts.len().saturating_sub(count);
        let taken: Vec<Part> = self.parts.drain(from..).collect();
        if let Some(first) = taken.first() {
            let points = taken.iter().flat_map(part_points).collect();
            self.hulls.push(Hull { paint: first.paint, points, anim: first.anim });
        }
        self
    }

    /// A hull through the given points directly.
    pub fn hull_points(mut self, paint: Paint, points: &[[f32; 3]]) -> Self {
        self.hulls.push(Hull { paint, points: points.iter().map(|p| Vec3::from(*p)).collect(), anim: Anim::None });
        self
    }

    /// Rotates the most recently added part (XYZ Euler, degrees).
    pub fn rotated(mut self, degrees: [f32; 3]) -> Self {
        if let Some(part) = self.parts.last_mut() {
            part.rot = Quat::from_euler(EulerRot::YXZ, degrees[1].to_radians(), degrees[0].to_radians(), degrees[2].to_radians());
        }
        self
    }

    /// Tags the most recently added part with an animation.
    pub fn anim(mut self, anim: Anim) -> Self {
        if let Some(part) = self.parts.last_mut() {
            part.anim = anim;
        }
        self
    }

    /// Adds a mirrored pair (±x) of the same part; `phase` staggers leg animation.
    pub fn pair(mut self, shape: Shape, paint: Paint, pos: [f32; 3], size: [f32; 3], anim: Option<f32>) -> Self {
        for (side, offset) in [(1.0, 0.0), (-1.0, 0.5)] {
            let anim = anim.map_or(Anim::None, |phase| Anim::Leg { phase: phase + offset });
            self.parts.push(Part { shape, paint, pos: Vec3::new(pos[0] * side, pos[1], pos[2]), size: size.into(), rot: Quat::IDENTITY, anim });
        }
        self
    }

    /// Appends another model with an offset; handy for composing attachments.
    pub fn merged(mut self, other: &BlockModel, offset: [f32; 3]) -> Self {
        let offset = Vec3::from(offset);
        self.parts.extend(other.parts.iter().map(|p| Part { pos: p.pos + offset, ..*p }));
        self.hulls.extend(other.hulls.iter().map(|h| Hull { points: h.points.iter().map(|p| *p + offset).collect(), ..h.clone() }));
        self
    }

    pub fn scaled(mut self, factor: f32) -> Self {
        for part in &mut self.parts {
            part.pos *= factor;
            part.size *= factor;
        }
        for hull in &mut self.hulls {
            for point in &mut hull.points {
                *point *= factor;
            }
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_composes() {
        let m = BlockModel::new().cube(Paint::Dark, [0.0; 3], [1.0; 3]).pair(Shape::Cube, Paint::Metal, [0.5, 0.0, 0.0], [0.1; 3], Some(0.0));
        assert_eq!(m.parts.len(), 3);
        assert_eq!(m.parts[1].pos.x, -m.parts[2].pos.x);
        assert_eq!(Paint::White.rgb(), [1.0; 3]);
    }

    #[test]
    fn hull_wraps_a_cube_in_twelve_triangles_and_fuses_parts() {
        let cube = Part { shape: Shape::Cube, paint: Paint::Dark, pos: Vec3::ZERO, size: Vec3::ONE, rot: Quat::IDENTITY, anim: Anim::None };
        let mut points = part_points(&cube);
        points.push(Vec3::ZERO); // inside: ignored
        let tris = convex_hull(&points);
        assert_eq!(tris.len(), 12);
        for [a, b, c] in tris {
            // Every face points away from the middle.
            assert!((b - a).cross(c - a).dot(a + b + c) > 0.0);
        }
        let m = BlockModel::new().cube(Paint::Wood, [0.0; 3], [1.0; 3]).cube(Paint::Dark, [0.0, 0.0, 2.0], [0.5; 3]).hull(2);
        assert!(m.parts.is_empty());
        assert_eq!(m.hulls.len(), 1);
        assert_eq!(m.hulls[0].paint, Paint::Wood);
        assert!(convex_hull(&m.hulls[0].points).len() >= 12);
    }
}
