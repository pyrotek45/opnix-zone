//! Starting and ending matches, and the registry of game modes.

use crate::{
    combat::{Grid, Solid},
    net::{NetIds, NetLink},
    player::{Arsenal, Health, LocalPlayer, Mover, Player, PlayerInput, spawn_point},
    state::{MatchPhase, MatchState, NetRole, Phase, Rules, SimRng},
    structures::OreNode,
    weapons::grant_weapon,
};
use bevy::prelude::*;
use oz_core::{arsenal::WeaponState, content::Catalog, rng::Rng, worldgen::World as GameMap};
use std::sync::Arc;

/// The active game's content. Cheap to clone, shared with background tasks if needed.
#[derive(Resource, Clone, Deref)]
pub struct Content(pub Arc<Catalog>);

/// The generated map for the current match.
#[derive(Resource, Clone, Deref)]
pub struct Map(pub Arc<GameMap>);

/// Everything that belongs to one match; despawned when it ends.
#[derive(Component)]
pub struct MatchEntity;

/// A walk-up terminal. `kind` indexes `Catalog::stations`.
#[derive(Component, Clone, Copy)]
pub struct Station {
    pub kind: u16,
}

/// A game mode as the engine sees it: a name, default rules, and content.
/// Its behaviour comes from a director plugin that the game registers separately.
pub struct ModeDef {
    pub name: &'static str,
    pub blurb: &'static str,
    /// Planned modes are listed in the menu but cannot be started yet.
    pub playable: bool,
    pub rules: fn(seed: u64) -> Rules,
    pub catalog: fn(&Rules) -> Catalog,
}

#[derive(Resource, Default)]
pub struct Modes(pub Vec<ModeDef>);

pub struct MatchSetup {
    pub rules: Rules,
    pub role: NetRole,
    pub link: NetLink,
    pub player_name: String,
    /// Player id of this machine's player; `None` for a dedicated server.
    pub local_player: Option<u8>,
}

impl MatchSetup {
    pub fn offline(rules: Rules, player_name: impl Into<String>) -> Self {
        Self { rules, role: NetRole::Offline, link: NetLink::None, player_name: player_name.into(), local_player: Some(0) }
    }
}

/// Put a setup here and the next frame starts that match.
#[derive(Resource, Default)]
pub struct PendingMatch(pub Option<MatchSetup>);

pub fn new_arsenal(catalog: &Catalog, rules: &Rules) -> Arsenal {
    let mut arsenal = Arsenal { weapons: vec![WeaponState::default(); catalog.weapons.len()], ..default() };
    for index in 0..catalog.weapons.len() {
        if rules.starter_weapons & (1 << index) != 0 {
            grant_weapon(&mut arsenal, &catalog.weapons, index);
        }
    }
    arsenal.current = (0..catalog.weapons.len()).find(|i| arsenal.owns(*i)).unwrap_or(0) as u8;
    arsenal
}

/// The components every player entity has, on every machine.
pub fn player_bundle(catalog: &Catalog, rules: &Rules, map: &GameMap, id: u8, name: String) -> impl Bundle {
    let arsenal = new_arsenal(catalog, rules);
    (
        MatchEntity,
        Player { id, name, respawn: 0.0, calm: 0.0 },
        PlayerInput { weapon: arsenal.current, ..default() },
        arsenal,
        Health::full(100.0),
        Mover::default(),
        Transform::from_translation(spawn_point(map, id)),
    )
}

pub fn start_pending_match(
    mut pending: ResMut<PendingMatch>,
    modes: Res<Modes>,
    mut commands: Commands,
    mut next: ResMut<NextState<MatchPhase>>,
    current: Res<State<MatchPhase>>,
) {
    if *current.get() != MatchPhase::None {
        return;
    }
    let Some(setup) = pending.0.take() else { return };
    let Some(mode) = modes.0.get(setup.rules.mode as usize) else {
        warn!("unknown game mode {}", setup.rules.mode);
        return;
    };
    let rules = setup.rules;
    let catalog = Arc::new((mode.catalog)(&rules));
    let map = Arc::new(GameMap::generate(rules.recipe));
    let mut ids = NetIds::default();

    // Static furniture is rebuilt from the recipe on every machine; it is never replicated.
    for prop in &map.props {
        if let Some(half) = prop.solid_half() {
            commands.spawn((MatchEntity, Solid { half }, Transform::from_translation(prop.pos)));
        }
    }
    for (index, pad) in map.pads.iter().enumerate().take(catalog.stations.len()) {
        let facing = Quat::from_rotation_y(f32::atan2(pad.x - map.core.x, pad.y - map.core.y));
        commands.spawn((
            MatchEntity,
            Station { kind: index as u16 },
            Solid { half: Vec3::new(0.75, 1.1, 0.75) },
            Transform::from_translation(map.ground(pad.x, pad.y)).with_rotation(facing),
        ));
    }
    if rules.objective_hp > 0.0 {
        commands.spawn((MatchEntity, Solid { half: Vec3::new(1.5, 0.45, 1.5) }, Transform::from_translation(map.ground(map.core.x, map.core.y))));
    }

    if setup.role != NetRole::Client && rules.mining {
        for deposit in &map.deposits {
            commands.spawn((
                MatchEntity,
                ids.next(),
                OreNode { ore: deposit.ore, amount: deposit.amount, max: deposit.amount },
                Solid { half: Vec3::new(0.8, 0.7, 0.8) },
                Transform::from_translation(deposit.pos),
            ));
        }
    }
    if let Some(id) = setup.local_player {
        let mut player = commands.spawn((player_bundle(&catalog, &rules, &map, id, setup.player_name.clone()), LocalPlayer));
        if setup.role != NetRole::Client {
            player.insert(ids.next());
        }
    }

    commands.insert_resource(MatchState {
        phase: Phase::Day,
        day: 1,
        timer: rules.day_seconds,
        daylight: 1.0,
        objective_hp: rules.objective_hp,
        objective_max: rules.objective_hp,
        stock: rules.start_stock,
        upgrades: vec![0; catalog.upgrades.len()],
        ..default()
    });
    commands.insert_resource(Grid::new(rules.recipe.half_extent));
    commands.insert_resource(SimRng(Rng::new(rules.recipe.seed ^ 0xc0ffee)));
    commands.insert_resource(Content(catalog));
    commands.insert_resource(Map(map));
    commands.insert_resource(rules);
    commands.insert_resource(ids);
    commands.insert_resource(setup.role);
    commands.insert_resource(setup.link);
    next.set(MatchPhase::Active);
}

/// Ends the current match and returns to the idle state (menus).
pub fn end_match(next: &mut NextState<MatchPhase>) {
    next.set(MatchPhase::None);
}

pub fn cleanup_match(mut commands: Commands, entities: Query<Entity, With<MatchEntity>>) {
    for entity in &entities {
        commands.entity(entity).despawn();
    }
    commands.insert_resource(NetLink::None);
    commands.insert_resource(NetRole::Offline);
}
