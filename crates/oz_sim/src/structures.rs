//! Player-built structures (turrets, drills, walls) and ore deposits.

use crate::{
    combat::{Grid, Hit},
    player::Health,
    session::Content,
    sim_dt,
    state::{FxEvent, MatchState},
};
use bevy::prelude::*;
use oz_core::{
    content::{StructureKind, UpgradeEffect},
    economy::Ore,
};

#[derive(Component, Clone, Copy, Debug)]
pub struct Structure {
    pub kind: u16,
    pub cooldown: f32,
    /// The deposit a drill is extracting from.
    pub node: Option<Entity>,
    /// Fractional ore not yet banked.
    pub carry: f32,
    /// Incremented per shot; replicated for client animation.
    pub shots: u8,
}

impl Structure {
    pub fn new(kind: u16, node: Option<Entity>) -> Self {
        Self { kind, cooldown: 0.5, node, carry: 0.0, shots: 0 }
    }
}

#[derive(Component, Clone, Copy, Debug)]
pub struct OreNode {
    pub ore: Ore,
    pub amount: f32,
    pub max: f32,
}

/// Collision box for a structure placed at `yaw`. Yaw snaps to quarter turns,
/// so the box stays axis-aligned and only swaps its X/Z extents.
pub fn oriented_half(half: Vec3, yaw: f32) -> Vec3 {
    let quarter_turns = (yaw / std::f32::consts::FRAC_PI_2).round() as i32;
    if quarter_turns.rem_euclid(2) == 1 { Vec3::new(half.z, half.y, half.x) } else { half }
}

pub fn snap_yaw(yaw: f32) -> f32 {
    (yaw / std::f32::consts::FRAC_PI_2).round() * std::f32::consts::FRAC_PI_2
}

pub fn run_structures(
    time: Res<Time>,
    content: Res<Content>,
    grid: Res<Grid>,
    mut state: ResMut<MatchState>,
    mut commands: Commands,
    mut structures: Query<(Entity, &mut Transform, &mut Structure, &Health)>,
    mut ores: Query<&mut OreNode>,
    mut hits: EventWriter<Hit>,
    mut fx: EventWriter<FxEvent>,
) {
    let dt = sim_dt(&time);
    let turret_bonus = content.bonus(&state.upgrades, UpgradeEffect::TurretDamage);
    let drill_bonus = content.bonus(&state.upgrades, UpgradeEffect::DrillRate);
    for (entity, mut transform, mut structure, health) in &mut structures {
        let def = &content.structures[structure.kind as usize];
        if !health.alive() {
            fx.write(FxEvent::StructureLost { at: transform.translation + Vec3::Y * def.half.y });
            commands.entity(entity).despawn();
            continue;
        }
        structure.cooldown -= dt;
        match def.kind {
            StructureKind::Turret { range, damage, interval } => {
                if structure.cooldown > 0.0 {
                    continue;
                }
                let muzzle = transform.translation + Vec3::Y * (def.half.y * 2.0 - 0.25);
                match grid.nearest(muzzle, range, None) {
                    Some(target) => {
                        let to = target.pos - muzzle;
                        transform.rotation = Quat::from_rotation_y(f32::atan2(-to.x, -to.z));
                        structure.cooldown = interval;
                        structure.shots = structure.shots.wrapping_add(1);
                        hits.write(Hit::plain(target.entity, damage * turret_bonus, None));
                        fx.write(FxEvent::Shot { by: 255, sound: 7, from: muzzle + to.normalize_or_zero() * 0.9, to: target.pos, beam: false, tracer: oz_core::model::Paint::Blue, quiet: false, local: false });
                    }
                    // Nothing in range: check again shortly instead of every frame.
                    None => structure.cooldown = 0.25,
                }
            }
            StructureKind::Drill { rate } => {
                let Some(mut node) = structure.node.and_then(|node| ores.get_mut(node).ok()) else { continue };
                let mined = (rate * drill_bonus * dt).min(node.amount);
                node.amount -= mined;
                structure.carry += mined;
                if structure.carry >= 1.0 {
                    state.stock.add_ore(node.ore, structure.carry as u32);
                    structure.carry = structure.carry.fract();
                }
            }
            StructureKind::Wall => {}
        }
    }
}
