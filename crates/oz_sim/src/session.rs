//! Starting and ending matches, and the registry of game modes.

use crate::{
    combat::Grid,
    nav::Nav,
    net::{NetIds, NetLink},
    player::{Arsenal, Health, LocalPlayer, Mover, Player, PlayerInput, spawn_point},
    state::{MatchPhase, MatchState, NetRole, Phase, Rules, SimRng},
    structures::OreNode,
    weapons::grant_weapon,
};
use bevy::prelude::*;
use oz_core::{
    arsenal::WeaponState,
    collide::{Aabb, StaticWorld},
    content::Catalog,
    rng::Rng,
    worldgen::World as GameMap,
};
use std::sync::Arc;

/// The active game's content. Cheap to clone, shared with background tasks if needed.
#[derive(Resource, Clone, Deref)]
pub struct Content(pub Arc<Catalog>);

/// The generated map for the current match.
#[derive(Resource, Clone, Deref)]
pub struct Map(pub Arc<GameMap>);

/// Every static collision box: architecture, props, stations, the objective
/// and ore. Built identically on every machine from the map recipe.
#[derive(Resource, Clone, Deref)]
pub struct Statics(pub Arc<StaticWorld>);

/// Station footprint, shared by collision and placement checks.
pub const STATION_HALF: Vec3 = Vec3::new(0.75, 1.1, 0.75);
const CORE_HALF: Vec3 = Vec3::new(1.5, 0.45, 1.5);
const ORE_HALF: Vec3 = Vec3::new(0.8, 0.7, 0.8);

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
    pub local_team: u8,
}

impl MatchSetup {
    pub fn offline(rules: Rules, player_name: impl Into<String>) -> Self {
        Self {
            rules,
            role: NetRole::Offline,
            link: NetLink::None,
            player_name: player_name.into(),
            local_player: Some(0),
            local_team: 0,
        }
    }
}

/// Put a setup here and the next frame starts that match.
#[derive(Resource, Default)]
pub struct PendingMatch(pub Option<MatchSetup>);

pub fn new_arsenal(catalog: &Catalog, rules: &Rules) -> Arsenal {
    let mut arsenal = Arsenal {
        weapons: catalog.weapons.iter().map(WeaponState::fresh).collect(),
        ..default()
    };
    for index in 0..catalog.weapons.len() {
        if rules.starter_weapons & (1 << index) != 0 {
            grant_weapon(&mut arsenal, &catalog.weapons, index);
        }
    }
    arsenal.current = (0..catalog.weapons.len())
        .find(|i| arsenal.owns(*i))
        .unwrap_or(0) as u8;
    arsenal
}

/// The components every player entity has, on every machine.
pub fn player_bundle(
    catalog: &Catalog,
    rules: &Rules,
    map: &GameMap,
    id: u8,
    name: String,
    team: u8,
) -> impl Bundle {
    let arsenal = new_arsenal(catalog, rules);
    let team = if rules.pvp { team.min(1) } else { 0 };
    let mut player = Player::new(id, name, team);
    player.gear_left = catalog.gear.first().map_or(0, |gear| gear.count);
    // Start out facing the middle of the map rather than the sea.
    let spawn = spawn_point(map, id, rules.pvp.then_some(team));
    let to_center = map.core - spawn.xz();
    let yaw = f32::atan2(-to_center.x, -to_center.y);
    (
        MatchEntity,
        player,
        PlayerInput {
            weapon: arsenal.current,
            yaw,
            ..default()
        },
        arsenal,
        Health::full(100.0),
        Mover::default(),
        Transform::from_translation(spawn).with_rotation(Quat::from_rotation_y(yaw)),
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
    let Some(setup) = pending.0.take() else {
        return;
    };
    let Some(mode) = modes.0.get(setup.rules.mode as usize) else {
        warn!("unknown game mode {}", setup.rules.mode);
        return;
    };
    let rules = setup.rules;
    let catalog = Arc::new((mode.catalog)(&rules));
    let map = Arc::new(GameMap::generate(rules.recipe));
    let mut ids = NetIds::default();

    // Static collision is rebuilt from the recipe on every machine; it is never replicated.
    let mut statics = StaticWorld::new(rules.recipe.half_extent);
    for aabb in map.solids() {
        statics.add(aabb);
    }
    let standing = |at: Vec3, half: Vec3| Aabb::new(at + Vec3::Y * half.y, half);
    for (index, pad) in map.pads.iter().enumerate().take(catalog.stations.len()) {
        let facing = Quat::from_rotation_y(f32::atan2(pad.x - map.core.x, pad.y - map.core.y));
        let at = map.ground(pad.x, pad.y);
        statics.add(standing(at, STATION_HALF));
        commands.spawn((
            MatchEntity,
            Station { kind: index as u16 },
            Transform::from_translation(at).with_rotation(facing),
        ));
    }
    if rules.objective_hp > 0.0 {
        statics.add(standing(map.ground(map.core.x, map.core.y), CORE_HALF));
    }
    if rules.mining {
        for deposit in &map.deposits {
            statics.add(standing(deposit.pos, ORE_HALF));
        }
    }
    // Bots need a navigation grid; only the authority runs them.
    if setup.role != NetRole::Client {
        commands.insert_resource(Nav::build(&map, &statics));
    }

    if setup.role != NetRole::Client && rules.mining {
        for deposit in &map.deposits {
            commands.spawn((
                MatchEntity,
                ids.next(),
                OreNode {
                    ore: deposit.ore,
                    amount: deposit.amount,
                    max: deposit.amount,
                },
                Transform::from_translation(deposit.pos),
            ));
        }
    }
    if let Some(id) = setup.local_player {
        let mut player = commands.spawn((
            player_bundle(
                &catalog,
                &rules,
                &map,
                id,
                setup.player_name.clone(),
                setup.local_team,
            ),
            LocalPlayer,
        ));
        if setup.role != NetRole::Client {
            player.insert(ids.next());
        }
    }

    commands.insert_resource(MatchState {
        phase: if rules.pvp { Phase::Lobby } else { Phase::Day },
        team_size: rules.team_size,
        day: 1,
        timer: rules.day_seconds,
        daylight: 1.0,
        objective_hp: rules.objective_hp,
        objective_max: rules.objective_hp,
        stock: rules.start_stock,
        upgrades: vec![0; catalog.upgrades.len()],
        weather: rules.recipe.weather,
        ..default()
    });
    commands.insert_resource(Grid::new(rules.recipe.half_extent));
    commands.insert_resource(SimRng(Rng::new(rules.recipe.seed ^ 0xc0ffee)));
    commands.insert_resource(Content(catalog));
    commands.insert_resource(Map(map));
    commands.insert_resource(Statics(Arc::new(statics)));
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
    commands.remove_resource::<Nav>();
    commands.insert_resource(crate::lagcomp::LagHistory::default());
    commands.insert_resource(crate::net::ServerClock::default());
    commands.insert_resource(NetLink::None);
    commands.insert_resource(NetRole::Offline);
}
