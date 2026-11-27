//! `oz_core` - engine-agnostic building blocks for Opnix Zone shooters.
//!
//! Nothing in here depends on Bevy. Everything is plain data plus pure
//! functions, so it can be unit-tested headlessly and shared between the
//! authoritative simulation, the client, and any future tooling.
//!
//! * [`rng`], [`noise`] - deterministic randomness.
//! * [`collide`] - static collision boxes for walls, floors and props.
//! * [`model`] - block models: every visual asset is a list of painted primitives.
//! * [`worldgen`] - procedural maps (heightfield, props, ore, spawn points).
//! * [`arsenal`] - weapons, attachments, mods and the stat pipeline.
//! * [`content`] - enemy / structure / upgrade definitions and the [`content::Catalog`].
//! * [`kit`] - a ready-made arsenal games can reuse.
//! * [`economy`] - shared stock and costs.
//! * [`synth`] - procedural sound effects.

pub mod arsenal;
pub mod collide;
pub mod content;
pub mod economy;
pub mod kit;
pub mod model;
pub mod noise;
pub mod rng;
pub mod synth;

pub mod worldgen;

pub use glam;
