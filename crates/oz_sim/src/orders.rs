//! Validates and applies [`PlayerCommand`]s. Purchases, builds and upgrades
//! all pass through here on the authority, so single-player and online play
//! share one code path and clients can never grant themselves anything.

use crate::{
    combat::Solid,
    net::NetIds,
    player::{Arsenal, Health, Player},
    session::{Content, Map, MatchEntity},
    state::{CommandEvent, FxEvent, MatchState, PlayerCommand, Rules},
    structures::{OreNode, Structure, oriented_half, snap_yaw},
    weapons::grant_weapon,
};
use bevy::prelude::*;
use oz_core::{
    arsenal::resolve,
    content::{StructureKind, UpgradeEffect},
    economy::Cost,
    synth::Sfx,
};

pub const BUILD_REACH: f32 = 10.0;
pub const DRILL_SNAP: f32 = 3.5;
const MAX_STRUCTURES: usize = 120;

#[allow(clippy::too_many_arguments)]
pub fn process_commands(
    mut events: EventReader<CommandEvent>,
    content: Res<Content>,
    map: Res<Map>,
    statics: Res<crate::session::Statics>,
    rules: Res<Rules>,
    mut state: ResMut<MatchState>,
    mut ids: ResMut<NetIds>,
    mut commands: Commands,
    mut players: Query<(&mut Player, &Transform, &mut Arsenal, &mut Health)>,
    solids: Query<(Entity, &Transform, &Solid)>,
    structures: Query<&Structure>,
    ores: Query<(Entity, &Transform), With<OreNode>>,
    mut fx: EventWriter<FxEvent>,
) {
    for event in events.read() {
        let id = event.player;
        let deny = |fx: &mut EventWriter<FxEvent>, why: &str| {
            fx.write(FxEvent::tell(id, why));
            fx.write(FxEvent::Cue { to: id, sfx: Sfx::UiDeny });
        };
        // A new name: cleaned up, and told apart from everyone else's.
        if let PlayerCommand::Rename(wanted) = &event.command {
            let wanted = crate::versus::clean_name(wanted);
            let others: Vec<String> = players.iter().filter(|(p, ..)| p.id != id).map(|(p, ..)| p.name.clone()).collect();
            let name = crate::versus::unique_name(&wanted, &others.iter().map(String::as_str).collect::<Vec<_>>());
            if let Some((mut player, ..)) = players.iter_mut().find(|(player, ..)| player.id == id)
                && !name.is_empty()
                && player.name != name
            {
                fx.write(FxEvent::notice(format!("{} IS NOW {name}", player.name)));
                player.name = name;
            }
            continue;
        }
        let Some((mut player, transform, mut arsenal, _)) = players.iter_mut().find(|(player, ..)| player.id == id) else { continue };
        let pay = |state: &mut MatchState, cost: &Cost| state.stock.pay(cost);

        let done: Option<String> = match event.command.clone() {
            PlayerCommand::BuyWeapon(weapon) => {
                let Some(def) = content.weapons.get(weapon as usize) else { continue };
                if arsenal.owns(weapon as usize) {
                    deny(&mut fx, "ALREADY OWNED");
                    continue;
                }
                if !pay(&mut state, &def.price) {
                    deny(&mut fx, "NOT ENOUGH RESOURCES");
                    continue;
                }
                grant_weapon(&mut arsenal, &content.weapons, weapon as usize);
                Some(format!("{} ACQUIRED", def.name))
            }
            PlayerCommand::BuyAmmo(weapon) => {
                let Some(def) = content.weapons.get(weapon as usize) else { continue };
                let cap = def.mag * 8;
                if !arsenal.owns(weapon as usize) || def.infinite_reserve || def.mag == 0 || arsenal.weapons[weapon as usize].reserve >= cap {
                    deny(&mut fx, "AMMO NOT NEEDED");
                    continue;
                }
                if !pay(&mut state, &Cost::credits(def.ammo_price)) {
                    deny(&mut fx, "NOT ENOUGH CREDITS");
                    continue;
                }
                let slot = &mut arsenal.weapons[weapon as usize];
                slot.reserve = (slot.reserve + def.mag * 2).min(cap);
                Some(format!("{} AMMO +{}", def.name, def.mag * 2))
            }
            PlayerCommand::EquipPart { weapon, slot, variant } => {
                let (w, s, v) = (weapon as usize, slot as usize, variant as usize);
                let name = oz_core::arsenal::options(&content.weapons.get(w).cloned().unwrap_or_else(oz_core::kit::blank_weapon), &content.attachments, s)
                    .into_iter()
                    .find(|(i, ..)| *i == v)
                    .map_or("PART", |(_, name, ..)| name);
                // Versus loadouts customise any weapon for free; co-op players pay for parts on guns they own.
                if w >= arsenal.weapons.len() || (!arsenal.owns(w) && !rules.pvp) {
                    deny(&mut fx, "WEAPON NOT OWNED");
                    continue;
                }
                let Some(price) = oz_core::arsenal::part_cost(&content.weapons[w], &content.attachments, s, v) else {
                    deny(&mut fx, "THAT PART DOES NOT FIT");
                    continue;
                };
                if !arsenal.weapons[w].has_part(s, v) && !rules.pvp {
                    if !pay(&mut state, &Cost::credits(price)) {
                        deny(&mut fx, "NOT ENOUGH CREDITS");
                        continue;
                    }
                    arsenal.weapons[w].purchased[s] |= 1 << v;
                }
                arsenal.weapons[w].equipped[s] = variant;
                // A smaller magazine returns the excess rounds to the reserve.
                let mag = resolve(&content.weapons[w], &arsenal.weapons[w], &content.attachments).mag;
                let state = &mut arsenal.weapons[w];
                if state.ammo > mag {
                    state.reserve += state.ammo - mag;
                    state.ammo = mag;
                }
                Some(format!("{name} EQUIPPED"))
            }
            PlayerCommand::BuyMod { weapon, kind } => {
                let w = weapon as usize;
                if !arsenal.owns(w) || !content.mods.contains(&kind) {
                    deny(&mut fx, "WEAPON NOT OWNED");
                    continue;
                }
                let level = arsenal.weapons[w].mod_level(kind);
                let mut trial = arsenal.weapons[w];
                if !trial.add_mod(kind) {
                    deny(&mut fx, if level > 0 { "MOD AT MAX LEVEL" } else { "ALL MOD SLOTS FULL" });
                    continue;
                }
                if !pay(&mut state, &kind.cost(level)) {
                    deny(&mut fx, "NOT ENOUGH RESOURCES");
                    continue;
                }
                arsenal.weapons[w] = trial;
                Some(format!("{} MOD LEVEL {}", kind.name(), level + 1))
            }
            PlayerCommand::Build { kind, x, z, yaw } => {
                let Some(def) = content.structures.get(kind as usize) else { continue };
                let yaw = snap_yaw(yaw);
                let half = oriented_half(def.half, yaw);
                let mut at = Vec3::new(x, 0.0, z);
                let mut node = None;
                if matches!(def.kind, StructureKind::Drill { .. }) {
                    let taken = |candidate: Entity| structures.iter().any(|s| s.node == Some(candidate));
                    let nearest = ores
                        .iter()
                        .filter(|(entity, _)| !taken(*entity))
                        .map(|(entity, t)| (t.translation.xz().distance(at.xz()), entity, t.translation))
                        .min_by(|a, b| a.0.total_cmp(&b.0));
                    match nearest {
                        Some((d, entity, pos)) if d <= DRILL_SNAP => {
                            node = Some(entity);
                            at = pos;
                        }
                        _ => {
                            deny(&mut fx, "DRILLS MUST BE PLACED ON A FREE ORE DEPOSIT");
                            continue;
                        }
                    }
                }
                at.y = map.height(at.x, at.z);
                let footprint = half + Vec3::new(0.15, 0.0, 0.15);
                let drill_site = node.map(|_| at);
                let blocked = statics.overlaps(at + Vec3::Y * (half.y + 0.25), Vec3::new(footprint.x, half.y, footprint.z), drill_site)
                    || solids.iter().any(|(_, t, solid)| (t.translation.x - at.x).abs() < solid.half.x + footprint.x && (t.translation.z - at.z).abs() < solid.half.z + footprint.z);
                let near_core = at.xz().distance(map.core) < 3.2;
                if transform.translation.distance(at) > BUILD_REACH || at.y < map.water + 0.15 || blocked || near_core {
                    deny(&mut fx, "CANNOT BUILD THERE");
                    continue;
                }
                if structures.iter().count() >= MAX_STRUCTURES {
                    deny(&mut fx, "STRUCTURE LIMIT REACHED");
                    continue;
                }
                if !pay(&mut state, &def.cost) {
                    deny(&mut fx, "NOT ENOUGH RESOURCES");
                    continue;
                }
                commands.spawn((
                    MatchEntity,
                    ids.next(),
                    Structure::new(kind, node),
                    Health::full(def.hp),
                    Solid { half },
                    Transform::from_translation(at).with_rotation(Quat::from_rotation_y(yaw)),
                ));
                fx.write(FxEvent::Sound { sfx: Sfx::Build, at: Some(at) });
                Some(format!("{} ONLINE", def.name))
            }
            PlayerCommand::Upgrade(index) => {
                let Some(def) = content.upgrades.get(index as usize) else { continue };
                let level = state.upgrades.get(index as usize).copied().unwrap_or(def.max_level);
                if level >= def.max_level {
                    deny(&mut fx, "UPGRADE AT MAX LEVEL");
                    continue;
                }
                if !pay(&mut state, &def.cost.scaled(level as u32)) {
                    deny(&mut fx, "NOT ENOUGH RESOURCES");
                    continue;
                }
                state.upgrades[index as usize] += 1;
                match def.effect {
                    UpgradeEffect::ObjectiveHull => {
                        let max = rules.objective_hp * content.bonus(&state.upgrades, UpgradeEffect::ObjectiveHull);
                        state.objective_hp += max - state.objective_max;
                        state.objective_max = max;
                    }
                    UpgradeEffect::PlayerArmor => {
                        let max = 100.0 * content.bonus(&state.upgrades, UpgradeEffect::PlayerArmor);
                        for (_, _, _, mut health) in &mut players {
                            health.hp += max - health.max;
                            health.max = max;
                        }
                    }
                    _ => {}
                }
                fx.write(FxEvent::notice(format!("{} UPGRADED TO LEVEL {}", def.name, level + 1)));
                None
            }
            PlayerCommand::Loadout(loadout) => {
                let valid = |index: u8| (index as usize) < content.weapons.len();
                if !rules.pvp || !valid(loadout.primary) || !valid(loadout.secondary) || loadout.primary == loadout.secondary {
                    deny(&mut fx, "INVALID LOADOUT");
                    continue;
                }
                // In the lobby and on fresh spawns the class swaps at once; otherwise it applies on the next life.
                if player.life < 10.0 || state.phase == crate::state::Phase::Lobby {
                    crate::player::apply_loadout(&content, &mut player, &mut arsenal, loadout);
                    None
                } else {
                    player.loadout = Some(loadout);
                    Some("LOADOUT APPLIES ON YOUR NEXT SPAWN".to_string())
                }
            }
            PlayerCommand::Lobby(_) => continue,
            PlayerCommand::Chat(text) => {
                let text: String = text.chars().filter(|c| c.is_ascii_graphic() || *c == ' ').take(80).collect::<String>().trim().to_uppercase();
                if !text.is_empty() {
                    fx.write(FxEvent::Chat { name: player.name.clone(), text });
                }
                continue;
            }
            // Mode actions belong to the mode's director, which reads the same events.
            PlayerCommand::Mode(_) => continue,
            // Handled by `killstreak::use_streaks`.
            PlayerCommand::UseStreak => continue,
            // Handled by `breakables::work_doors`.
            PlayerCommand::Door(_) | PlayerCommand::Pickup | PlayerCommand::Rename(_) => continue,
        };
        fx.write(FxEvent::Cue { to: id, sfx: Sfx::Buy });
        if let Some(text) = done {
            fx.write(FxEvent::tell(id, text));
        }
    }
}
