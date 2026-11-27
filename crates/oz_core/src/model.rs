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
    Dark,
    Metal,
    Gold,
    Wood,
    Teal,
    Coral,
    Purple,
    Shell,
    Bone,
    Steel,
    Rock,
    Leaf,
    LeafLight,
    Concrete,
    Rust,
    Sand,
    Iron,
    Copper,
    // Unlit / emissive paints.
    Glow,
    Spark,
    Red,
    Green,
    Blue,
    Violet,
    Aether,
    White,
    /// A dark, faintly glowing display surface (robot faces, terminals).
    Screen,
}

impl Paint {
    pub const ALL: [Paint; 27] = [
        Paint::Dark,
        Paint::Metal,
        Paint::Gold,
        Paint::Wood,
        Paint::Teal,
        Paint::Coral,
        Paint::Purple,
        Paint::Shell,
        Paint::Bone,
        Paint::Steel,
        Paint::Rock,
        Paint::Leaf,
        Paint::LeafLight,
        Paint::Concrete,
        Paint::Rust,
        Paint::Sand,
        Paint::Iron,
        Paint::Copper,
        Paint::Glow,
        Paint::Spark,
        Paint::Red,
        Paint::Green,
        Paint::Blue,
        Paint::Violet,
        Paint::Aether,
        Paint::White,
        Paint::Screen,
    ];

    /// sRGB colour as `0xRRGGBB`.
    pub fn hex(self) -> u32 {
        match self {
            Paint::Dark => 0x26354b,
            Paint::Metal => 0x78939b,
            Paint::Gold => 0xf4ae61,
            Paint::Wood => 0x78504b,
            Paint::Teal => 0x216d73,
            Paint::Coral => 0xda7766,
            Paint::Purple => 0x8354a1,
            Paint::Shell => 0x396b64,
            Paint::Bone => 0xd7c1a0,
            Paint::Steel => 0x3b4c60,
            Paint::Rock => 0x637477,
            Paint::Leaf => 0x26635b,
            Paint::LeafLight => 0x6eaa64,
            Paint::Concrete => 0xd3a97c,
            Paint::Rust => 0xba654d,
            Paint::Sand => 0xe3b06e,
            Paint::Iron => 0x8a8f9c,
            Paint::Copper => 0xd9824a,
            Paint::Glow => 0xffcb65,
            Paint::Spark => 0xffedbb,
            Paint::Red => 0xff535e,
            Paint::Green => 0x7fffa0,
            Paint::Blue => 0x63ddff,
            Paint::Violet => 0xc58bff,
            Paint::Aether => 0x9df7e4,
            Paint::White => 0xffffff,
            Paint::Screen => 0x0c2430,
        }
    }

    pub fn rgb(self) -> [f32; 3] {
        let h = self.hex();
        [
            (h >> 16) as u8 as f32 / 255.0,
            (h >> 8) as u8 as f32 / 255.0,
            h as u8 as f32 / 255.0,
        ]
    }

    /// Unlit paints ignore lighting and read as glowing, even at night.
    pub fn unlit(self) -> bool {
        matches!(
            self,
            Paint::Glow
                | Paint::Spark
                | Paint::Red
                | Paint::Green
                | Paint::Blue
                | Paint::Violet
                | Paint::Aether
                | Paint::White
                | Paint::Screen
        )
    }
}

/// Procedural animation hint for a part. Interpreted by the client only.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Anim {
    None,
    /// Swings back and forth while the owner moves; `phase` offsets the cycle (0..1).
    Leg {
        phase: f32,
    },
    /// Continuous spin around local Y, radians per second.
    Spin {
        speed: f32,
    },
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

/// A list of parts in model space: +Y up, -Z forward, origin at the feet.
#[derive(Clone, Debug, Default)]
pub struct BlockModel {
    pub parts: Vec<Part>,
}

impl BlockModel {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn part(mut self, shape: Shape, paint: Paint, pos: [f32; 3], size: [f32; 3]) -> Self {
        self.parts.push(Part {
            shape,
            paint,
            pos: pos.into(),
            size: size.into(),
            rot: Quat::IDENTITY,
            anim: Anim::None,
        });
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

    /// Keeps only the parts whose animation tag passes `keep`.
    pub fn filtered(&self, keep: impl Fn(Anim) -> bool) -> BlockModel {
        BlockModel {
            parts: self
                .parts
                .iter()
                .copied()
                .filter(|p| keep(p.anim))
                .collect(),
        }
    }

    /// Rotates the most recently added part (XYZ Euler, degrees).
    pub fn rotated(mut self, degrees: [f32; 3]) -> Self {
        if let Some(part) = self.parts.last_mut() {
            part.rot = Quat::from_euler(
                EulerRot::YXZ,
                degrees[1].to_radians(),
                degrees[0].to_radians(),
                degrees[2].to_radians(),
            );
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
    pub fn pair(
        mut self,
        shape: Shape,
        paint: Paint,
        pos: [f32; 3],
        size: [f32; 3],
        anim: Option<f32>,
    ) -> Self {
        for (side, offset) in [(1.0, 0.0), (-1.0, 0.5)] {
            let anim = anim.map_or(Anim::None, |phase| Anim::Leg {
                phase: phase + offset,
            });
            self.parts.push(Part {
                shape,
                paint,
                pos: Vec3::new(pos[0] * side, pos[1], pos[2]),
                size: size.into(),
                rot: Quat::IDENTITY,
                anim,
            });
        }
        self
    }

    /// Appends another model with an offset; handy for composing attachments.
    pub fn merged(mut self, other: &BlockModel, offset: [f32; 3]) -> Self {
        let offset = Vec3::from(offset);
        self.parts.extend(other.parts.iter().map(|p| Part {
            pos: p.pos + offset,
            ..*p
        }));
        self
    }

    pub fn scaled(mut self, factor: f32) -> Self {
        for part in &mut self.parts {
            part.pos *= factor;
            part.size *= factor;
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_composes() {
        let m = BlockModel::new()
            .cube(Paint::Dark, [0.0; 3], [1.0; 3])
            .pair(
                Shape::Cube,
                Paint::Metal,
                [0.5, 0.0, 0.0],
                [0.1; 3],
                Some(0.0),
            );
        assert_eq!(m.parts.len(), 3);
        assert_eq!(m.parts[1].pos.x, -m.parts[2].pos.x);
        assert_eq!(Paint::White.rgb(), [1.0; 3]);
    }
}
