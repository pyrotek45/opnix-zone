//! Scavenge: everyone spawns with a pistol and a spare magazine; everything else is
//! found. Weapons (with whatever parts they come with), ammo, gear and armour lie
//! around the town and come back a while after they are taken. Anyone who goes down
//! drops their gun and some ammo, so the fighting keeps everyone supplied.
//!
//! Walk over ammo, gear and armour to take them. Walk over a gun to take it while a
//! hand is free (you carry two); with both full, press Use to swap the gun you are
//! holding for it - yours drops where you stand.
//!
//! The other versus modes have no loot, but the fallen there sometimes drop a box of
//! ammo (always when their killer is running dry), so nobody is left with an empty gun.

use crate::{
    net::NetIds,
    player::{Arsenal, Health, Player, PlayerInput},
    session::{Content, Map, MatchEntity, Statics},
    sim_dt,
    state::{CommandEvent, FxEvent, MatchState, Phase, PlayerCommand, Rules, SimRng},
};
use bevy::prelude::*;
use oz_core::{
    arsenal::{SLOT_COUNT, options, resolve},
    synth::Sfx,
};

/// What a pickup gives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Loot {
    Weapon { index: u8, parts: [u8; SLOT_COUNT], ammo: u16, reserve: u16 },
    Ammo,
    /// A catalog gear index.
    Gear(u8),
    Armor,
    /// A killstreak supply crate (`killstreak::SupplyCrate`): not taken like loot.
    Supply { team: u8, uses: u8 },
}

/// Something lying on the ground to take.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct Pickup {
    pub loot: Loot,
    /// Dropped by a fallen player: gone after this many seconds. Placed loot does not expire.
    pub life: Option<f32>,
}

/// A place loot appears, and when it comes back after being taken.
#[derive(Component, Clone, Copy, Debug)]
pub struct LootSpot {
    pub at: Vec3,
    pub wait: f32,
    pub stocked: Option<Entity>,
}

/// Marks a downed player whose gun has already been dropped.
#[derive(Component)]
pub struct Dropped;

/// How many places loot appears.
const SPOTS: usize = 46;
/// Seconds before an emptied spot is restocked.
const RESTOCK: f32 = 35.0;
/// How long a dropped gun stays.
const DROP_LIFE: f32 = 45.0;
/// Reach for picking things up.
const REACH: f32 = 1.4;
/// Armour raises maximum health to this.
pub const ARMORED: f32 = 150.0;

/// Loot runs in scavenge, and in terrorist hunt (fallen hostiles drop their guns and ammo).
pub fn scavenge(rules: Option<Res<Rules>>) -> bool {
    rules.is_some_and(|rules| rules.loot || rules.hunt)
}

/// What appears at a spot: mostly guns (with a random set of parts), then ammo, gear, armour.
fn roll(content: &Content, rng: &mut SimRng) -> Loot {
    let guns: Vec<u8> = (1..content.weapons.len() as u8).filter(|i| content.weapons[*i as usize].frame.is_some()).collect();
    let r = rng.0.unit();
    if r < 0.45 && !guns.is_empty() {
        let index = guns[rng.0.below(guns.len())];
        let def = &content.weapons[index as usize];
        let mut parts = def.defaults;
        for (slot, part) in parts.iter_mut().enumerate() {
            let choices = options(def, &content.attachments, slot);
            if rng.0.chance(0.45) && !choices.is_empty() {
                *part = choices[rng.0.below(choices.len())].0 as u8;
            }
        }
        let state = oz_core::arsenal::WeaponState { equipped: parts, ..Default::default() };
        let mag = resolve(def, &state, &content.attachments).mag;
        Loot::Weapon { index, parts, ammo: mag, reserve: mag * 2 }
    } else if r < 0.75 {
        Loot::Ammo
    } else if r < 0.9 && !content.gear.is_empty() {
        Loot::Gear(rng.0.below(content.gear.len()) as u8)
    } else {
        Loot::Armor
    }
}

/// Authority: lays the loot spots out once per match (after the lobby).
pub fn place_spots(mut commands: Commands, map: Res<Map>, statics: Res<Statics>, mut rng: ResMut<SimRng>, spots: Query<(), With<LootSpot>>, state: Res<MatchState>, rules: Res<Rules>) {
    // Loot lying about town is scavenge only; hunt just has what hostiles drop.
    if !rules.loot || !spots.is_empty() || state.phase == Phase::Lobby {
        return;
    }
    for _ in 0..SPOTS {
        let at = crate::versus::hunt_post(&map, &statics, &mut rng);
        // The spot does not care whose base it is near; just keep it off the spawn pads.
        commands.spawn((MatchEntity, LootSpot { at, wait: rng.0.range(0.0, 2.0), stocked: None }));
    }
}

/// Puts the kit for scavenge on a player: a pistol, one spare magazine, no gear.
pub fn strip(content: &oz_core::content::Catalog, player: &mut Player, arsenal: &mut Arsenal) {
    let pistol = content.weapons.iter().position(|w| w.frame.is_some()).unwrap_or(0);
    for (index, (weapon, def)) in arsenal.weapons.iter_mut().zip(&content.weapons).enumerate() {
        weapon.owned = index == pistol;
        if weapon.owned {
            weapon.equipped = def.defaults;
            weapon.ammo = def.mag;
            weapon.reserve = def.mag;
        }
    }
    arsenal.current = pistol as u8;
    arsenal.reload = 0.0;
    player.gear_left = 0;
}

/// Authority: restocks spots, ages dropped guns, drops the guns of the fallen, and hands
/// out whatever players walk over (or ask for with Use).
#[allow(clippy::too_many_arguments)]
pub fn run_loot(
    time: Res<Time>,
    content: Res<Content>,
    state: Res<MatchState>,
    mut rng: ResMut<SimRng>,
    mut ids: ResMut<NetIds>,
    mut commands: Commands,
    mut spots: Query<&mut LootSpot>,
    mut pickups: Query<(Entity, &Transform, &mut Pickup)>,
    mut players: Query<(Entity, &mut Player, &Transform, &mut PlayerInput, &mut Arsenal, &mut Health, Has<Dropped>), Without<Pickup>>,
    mut orders: EventReader<CommandEvent>,
    mut fx: EventWriter<FxEvent>,
) {
    let dt = sim_dt(&time);
    if state.phase == Phase::Lobby {
        return;
    }
    // Restock empty spots.
    for mut spot in &mut spots {
        if spot.stocked.is_some_and(|e| pickups.get(e).is_ok()) {
            continue;
        }
        if spot.stocked.take().is_some() {
            spot.wait = RESTOCK;
        }
        spot.wait -= dt;
        if spot.wait <= 0.0 {
            let loot = roll(&content, &mut rng);
            spot.stocked = Some(commands.spawn((MatchEntity, ids.next(), Pickup { loot, life: None }, Transform::from_translation(spot.at))).id());
        }
    }
    // Dropped guns do not lie around forever.
    for (entity, _, mut pickup) in &mut pickups {
        if let Some(life) = pickup.life.as_mut() {
            *life -= dt;
            if *life <= 0.0 {
                commands.entity(entity).despawn();
            }
        }
    }
    let drop_at = |commands: &mut Commands, ids: &mut NetIds, at: Vec3, loot: Loot| {
        commands.spawn((MatchEntity, ids.next(), Pickup { loot, life: Some(DROP_LIFE) }, Transform::from_translation(at)));
    };
    // The fallen drop what they held (anything but the starting pistol) and some ammo.
    for (entity, _, transform, _, arsenal, health, dropped) in &mut players {
        if health.alive() {
            if dropped {
                commands.entity(entity).remove::<Dropped>();
            }
            continue;
        }
        if dropped {
            continue;
        }
        commands.entity(entity).insert(Dropped);
        let index = arsenal.current as usize;
        if let (Some(weapon), Some(def)) = (arsenal.weapons.get(index), content.weapons.get(index))
            && index != 0
            && def.frame.is_some()
        {
            drop_at(&mut commands, &mut ids, transform.translation + Vec3::new(0.3, 0.0, 0.0), Loot::Weapon { index: index as u8, parts: weapon.equipped, ammo: weapon.ammo, reserve: weapon.reserve });
        }
        drop_at(&mut commands, &mut ids, transform.translation - Vec3::new(0.3, 0.0, 0.0), Loot::Ammo);
    }

    // Use near a gun with both hands full: swap.
    let swaps: Vec<u8> = orders.read().filter(|o| o.command == PlayerCommand::Pickup).map(|o| o.player).collect();

    for (_, mut player, transform, mut input, mut arsenal, mut health, _) in &mut players {
        if !health.alive() {
            continue;
        }
        let feet = transform.translation;
        let wants_swap = swaps.contains(&player.id);
        let mut best_swap: Option<(f32, Entity)> = None;
        for (entity, at, pickup) in &pickups {
            let offset = at.translation - feet;
            if offset.xz().length() > REACH || offset.y.abs() > 1.6 {
                continue;
            }
            let owned: Vec<usize> = (0..arsenal.weapons.len()).filter(|i| arsenal.owns(*i)).collect();
            let taken = match pickup.loot {
                Loot::Ammo => take_ammo(&content, &mut arsenal).then(|| "AMMO".to_string()),
                Loot::Armor => (health.max < ARMORED).then(|| {
                    health.max = ARMORED;
                    health.hp = (health.hp + 50.0).min(ARMORED);
                    "ARMOUR PLATES".to_string()
                }),
                Loot::Gear(index) => content.gear.get(index as usize).and_then(|gear| {
                    (player.gear != index || player.gear_left < gear.count).then(|| {
                        player.gear = index;
                        player.gear_left = gear.count;
                        gear.name.to_string()
                    })
                }),
                Loot::Supply { .. } => None,
                Loot::Weapon { index, parts, ammo, reserve } => {
                    let i = index as usize;
                    let Some(def) = content.weapons.get(i) else { continue };
                    if owned.contains(&i) {
                        // The same gun again: strip it for ammo.
                        let weapon = &mut arsenal.weapons[i];
                        (weapon.reserve < def.reserve.max(def.mag * 4)).then(|| {
                            weapon.reserve = (weapon.reserve + ammo + reserve).min(def.reserve.max(def.mag * 4));
                            format!("{} AMMO", def.name)
                        })
                    } else if owned.len() < 2 {
                        let weapon = &mut arsenal.weapons[i];
                        weapon.owned = true;
                        weapon.equipped = parts;
                        weapon.ammo = ammo;
                        weapon.reserve = reserve;
                        input.weapon = index;
                        Some(def.name.to_string())
                    } else {
                        if wants_swap && best_swap.is_none_or(|(d, _)| offset.length() < d) {
                            best_swap = Some((offset.length(), entity));
                        }
                        None
                    }
                }
            };
            if let Some(what) = taken {
                commands.entity(entity).despawn();
                fx.write(FxEvent::tell(player.id, format!("+ {what}")));
                fx.write(FxEvent::Sound { sfx: Sfx::Buy, at: Some(feet) });
                break;
            }
        }
        // Swap the gun in hand for the one on the floor.
        if let Some((_, entity)) = best_swap
            && let Ok((_, _, pickup)) = pickups.get(entity)
            && let Loot::Weapon { index, parts, ammo, reserve } = pickup.loot
        {
            let current = arsenal.current as usize;
            if let Some(held) = arsenal.weapons.get(current).copied() {
                arsenal.weapons[current].owned = false;
                drop_at(&mut commands, &mut ids, feet + Vec3::Y * 0.1, Loot::Weapon { index: current as u8, parts: held.equipped, ammo: held.ammo, reserve: held.reserve });
            }
            let weapon = &mut arsenal.weapons[index as usize];
            weapon.owned = true;
            weapon.equipped = parts;
            weapon.ammo = ammo;
            weapon.reserve = reserve;
            arsenal.current = index;
            arsenal.reload = 0.0;
            input.weapon = index;
            commands.entity(entity).despawn();
            let name = content.weapons.get(index as usize).map_or("", |d| d.name);
            fx.write(FxEvent::tell(player.id, format!("+ {name}")));
            fx.write(FxEvent::Sound { sfx: Sfx::Swap, at: Some(feet) });
        }
    }
}

/// An ammo box: two magazines more for every gun carried, up to what it can hold.
/// False if every gun is already full.
fn take_ammo(content: &Content, arsenal: &mut Arsenal) -> bool {
    let mut any = false;
    for (weapon, def) in arsenal.weapons.iter_mut().zip(&content.weapons) {
        let cap = def.reserve.max(def.mag * 4);
        if weapon.owned && !def.infinite_reserve && weapon.reserve < cap {
            weapon.reserve = (weapon.reserve + def.mag.max(1) * 2).min(cap);
            any = true;
        }
    }
    any
}

/// Is a gun carried running dry: less than a spare magazine left behind the one loaded?
/// (Sidearms with endless reserve do not count: nobody wants to be left with only those.)
pub fn low_on_ammo(content: &Content, arsenal: &Arsenal) -> bool {
    arsenal.weapons.iter().zip(&content.weapons).any(|(weapon, def)| weapon.owned && !def.infinite_reserve && def.mag > 0 && weapon.reserve < def.mag)
}

/// Versus modes without loot: the fallen sometimes drop ammo.
pub fn ammo_drops(rules: Option<Res<Rules>>) -> bool {
    rules.is_some_and(|rules| rules.pvp && !rules.loot && !rules.hunt)
}

/// Chance someone killed drops an ammo box when their killer is not short of ammo.
pub const AMMO_DROP_CHANCE: f32 = 0.25;
/// Most dropped ammo boxes lying about at once.
const AMMO_DROPS: usize = 6;
/// How long a dropped ammo box lies before it goes.
const AMMO_LIFE: f32 = 30.0;

/// Authority, versus modes without loot: a kill sometimes leaves an ammo box where the
/// victim fell, always when the killer is low on ammo. Never more than a few at once:
/// past that the oldest one goes.
#[allow(clippy::too_many_arguments)]
pub fn drop_ammo(
    content: Res<Content>,
    state: Res<MatchState>,
    mut rng: ResMut<SimRng>,
    mut ids: ResMut<NetIds>,
    mut commands: Commands,
    mut kills: EventReader<FxEvent>,
    players: Query<(&Player, &Transform, &Arsenal)>,
    boxes: Query<(Entity, &Pickup)>,
) {
    let killed: Vec<(u8, u8)> = kills.read().filter_map(|e| if let FxEvent::Killed { victim, by, .. } = *e { Some((victim, by)) } else { None }).collect();
    if state.phase == Phase::Lobby || killed.is_empty() {
        return;
    }
    let mut lying: Vec<(Entity, f32)> = boxes.iter().filter_map(|(e, p)| (p.loot == Loot::Ammo).then_some((e, p.life?))).collect();
    for (victim, by) in killed {
        let Some((_, at, _)) = players.iter().find(|(p, ..)| p.id == victim) else { continue };
        let short = players.iter().find(|(p, ..)| p.id == by).is_some_and(|(_, _, arsenal)| low_on_ammo(&content, arsenal));
        if !short && !rng.0.chance(AMMO_DROP_CHANCE) {
            continue;
        }
        if lying.len() >= AMMO_DROPS {
            // Make room: the one with the least time left goes.
            lying.sort_by(|a, b| b.1.total_cmp(&a.1));
            if let Some((oldest, _)) = lying.pop() {
                commands.entity(oldest).despawn();
            }
        }
        let entity = commands.spawn((MatchEntity, ids.next(), Pickup { loot: Loot::Ammo, life: Some(AMMO_LIFE) }, Transform::from_translation(at.translation + Vec3::Y * 0.1))).id();
        lying.push((entity, AMMO_LIFE));
    }
}

/// Authority, versus modes without loot: ages the dropped ammo boxes and hands them to
/// whoever walks over one with room for it.
pub fn take_dropped_ammo(
    time: Res<Time>,
    content: Res<Content>,
    mut commands: Commands,
    mut boxes: Query<(Entity, &Transform, &mut Pickup)>,
    mut players: Query<(&Player, &Transform, &mut Arsenal, &Health)>,
    mut fx: EventWriter<FxEvent>,
) {
    let dt = sim_dt(&time);
    for (entity, at, mut pickup) in &mut boxes {
        let (Loot::Ammo, Some(life)) = (pickup.loot, pickup.life.as_mut()) else { continue };
        *life -= dt;
        if *life <= 0.0 {
            commands.entity(entity).despawn();
            continue;
        }
        let taker = players.iter_mut().find(|(_, feet, arsenal, health)| {
            let offset = at.translation - feet.translation;
            health.alive() && offset.xz().length() < REACH && offset.y.abs() < 1.6 && arsenal.weapons.iter().zip(&content.weapons).any(|(w, d)| w.owned && !d.infinite_reserve && w.reserve < d.reserve.max(d.mag * 4))
        });
        if let Some((player, feet, mut arsenal, _)) = taker {
            take_ammo(&content, &mut arsenal);
            commands.entity(entity).despawn();
            fx.write(FxEvent::tell(player.id, "+ AMMO".to_string()));
            fx.write(FxEvent::Sound { sfx: Sfx::Buy, at: Some(feet.translation) });
        }
    }
}

/// Where a bot with empty hands should go: the nearest gun on the ground.
pub fn nearest_gun<'a>(from: Vec3, pickups: impl Iterator<Item = (&'a Transform, &'a Pickup)>) -> Option<Vec3> {
    pickups
        .filter(|(_, p)| matches!(p.loot, Loot::Weapon { .. }))
        .map(|(t, _)| t.translation)
        .min_by(|a, b| a.distance_squared(from).total_cmp(&b.distance_squared(from)))
}

/// Packs loot for a snapshot: (sub, aux, 24 bits of parts).
pub fn encode(loot: &Loot) -> (u8, u8, u32) {
    match *loot {
        Loot::Ammo => (0, 0, 0),
        Loot::Armor => (1, 0, 0),
        Loot::Gear(index) => (2, index, 0),
        Loot::Weapon { index, parts, .. } => (3, index, parts.iter().enumerate().fold(0u32, |bits, (i, p)| bits | ((*p as u32 & 7) << (i * 3)))),
        Loot::Supply { team, uses } => (4, team, uses as u32),
    }
}

pub fn decode(sub: u8, aux: u8, packed: u32) -> Loot {
    match sub {
        0 => Loot::Ammo,
        1 => Loot::Armor,
        2 => Loot::Gear(aux),
        4 => Loot::Supply { team: aux, uses: (packed & 0xff) as u8 },
        _ => Loot::Weapon { index: aux, parts: std::array::from_fn(|i| ((packed >> (i * 3)) & 7) as u8), ammo: 0, reserve: 0 },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loot_round_trips_through_a_snapshot() {
        for loot in [Loot::Ammo, Loot::Armor, Loot::Gear(3), Loot::Supply { team: 1, uses: 2 }, Loot::Weapon { index: 7, parts: [1, 0, 2, 3, 0, 1, 4], ammo: 0, reserve: 0 }] {
            let (sub, aux, packed) = encode(&loot);
            assert_eq!(decode(sub, aux, packed), loot);
        }
    }
}
