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
    /// Set when a lobby reloads on new rules (see [`relaunch`]): who stays, and the
    /// lobby's settings. `None` for a fresh match.
    pub carry: Option<Carry>,
}

impl MatchSetup {
    pub fn offline(rules: Rules, player_name: impl Into<String>) -> Self {
        Self { rules, role: NetRole::Offline, link: NetLink::None, player_name: player_name.into(), local_player: Some(0), local_team: 0, carry: None }
    }
}

/// A human who keeps their seat through a relaunch (bots are made afresh for the new rules).
#[derive(Clone, Debug, PartialEq)]
pub struct Seat {
    pub id: u8,
    pub name: String,
    pub team: u8,
}

/// What a lobby keeps when it reloads on new rules or a new map.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Carry {
    /// Remote humans, spawned by the authority straight into their old ids.
    pub seats: Vec<Seat>,
    pub time_mode: u8,
    pub weather_mode: u8,
    /// Back from a co-op match: the host's co-op pick (mode, map) the lobby still shows.
    pub pick: Option<(u8, u8)>,
}

/// Authority: a guest who kept their seat through a relaunch and is still building the new
/// world. The lobby will not start until everyone is in (or [`LOADING_WAIT`] runs out).
#[derive(Component, Clone, Copy, Debug)]
pub struct Loading {
    pub waited: f32,
}

/// Longest the host's START waits for a guest still loading in.
pub const LOADING_WAIT: f32 = 20.0;

/// Client: tells the host this machine has built the world of the latest relaunch.
pub fn announce_arrival(link: Res<NetLink>, local: Query<&Player, With<LocalPlayer>>, mut orders: EventWriter<crate::state::CommandEvent>) {
    if let (NetLink::Client(link), Ok(player)) = (&*link, local.single()) {
        orders.write(crate::state::CommandEvent { player: player.id, command: crate::state::PlayerCommand::Arrived(link.epoch) });
    }
}

/// Authority: a guest is in once they say so for this relaunch; nobody waits forever.
pub fn mark_arrived(mut commands: Commands, time: Res<Time<Real>>, link: Res<NetLink>, mut orders: EventReader<crate::state::CommandEvent>, mut loading: Query<(Entity, &Player, &mut Loading)>) {
    let epoch = match &*link {
        NetLink::Host(host) => host.epoch,
        _ => 0,
    };
    for order in orders.read() {
        if let crate::state::PlayerCommand::Arrived(at) = order.command
            && at == epoch
            && let Some((entity, ..)) = loading.iter().find(|(_, p, _)| p.id == order.player)
        {
            commands.entity(entity).remove::<Loading>();
        }
    }
    for (entity, _, mut wait) in &mut loading {
        wait.waited += time.delta_secs();
        if wait.waited > LOADING_WAIT {
            commands.entity(entity).remove::<Loading>();
        }
    }
}

/// Put a setup here and the next frame starts that match.
#[derive(Resource, Default)]
pub struct PendingMatch(pub Option<MatchSetup>);

/// Authority: put rules here to reload the lobby on them - a new mode or map, or the
/// rematch after a match. The connection and every human (id, name, team) carry over;
/// the world, bots, scores and everything else start fresh.
#[derive(Resource, Default)]
pub struct Relaunch(pub Option<Rules>);

/// True while the current match came from a relaunch (menus stay in the lobby).
#[derive(Resource, Default)]
pub struct Relaunched(pub bool);

/// Authority: tears the match down and queues it again on the requested rules, keeping the
/// connection and the humans. Guests are told first, so they rebuild the same world.
#[allow(clippy::too_many_arguments)]
pub fn relaunch(
    mut request: ResMut<Relaunch>,
    mut link: ResMut<NetLink>,
    role: Res<NetRole>,
    old: Res<Rules>,
    state: Res<MatchState>,
    players: Query<(&Player, Has<LocalPlayer>, Has<crate::bots::Bot>)>,
    mut pending: ResMut<PendingMatch>,
    mut next: ResMut<NextState<MatchPhase>>,
    mut back: ResMut<crate::versus::LobbyReturn>,
) {
    let Some(rules) = request.0.take() else { return };
    // Back from a co-op match to the lobby it started from: the same teams and the same pick.
    let saved = if !old.pvp && rules.pvp { back.0.take().filter(|saved| saved.rules == rules) } else { None };
    // (id, name, team, this machine's player)
    let mut humans: Vec<(u8, String, u8, bool)> = players.iter().filter(|(.., bot)| !bot).map(|(p, local, _)| (p.id, p.name.clone(), p.team, local)).collect();
    let mut link = std::mem::take(&mut *link);
    if let NetLink::Host(host) = &link {
        // Someone who said hello this very frame has no player yet; keep their seat anyway.
        for id in host.peers.values() {
            if !humans.iter().any(|h| h.0 == *id) {
                humans.push((*id, format!("PLAYER {id}"), 0, false));
            }
        }
    }
    humans.sort_by_key(|h| h.0);
    for (index, human) in humans.iter_mut().enumerate() {
        // Hunt and co-op put every human on the squad; leaving one deals them out to both teams
        // (or back to the teams they had in the lobby before a co-op match).
        let before = saved.as_ref().and_then(|saved| saved.teams.iter().find(|(id, _)| *id == human.0)).map(|(_, team)| *team);
        human.2 = if rules.hunt || !rules.pvp {
            0
        } else if let Some(team) = before {
            team.min(1)
        } else if old.hunt || !old.pvp {
            (index % 2) as u8
        } else {
            human.2.min(1)
        };
    }
    if let NetLink::Host(host) = &mut link {
        let teams: Vec<(u8, u8)> = humans.iter().map(|h| (h.0, h.2)).collect();
        crate::net::announce_relaunch(host, &rules, &teams);
    }
    let local = humans.iter().find(|h| h.3).cloned();
    let seats = humans.iter().filter(|h| !h.3).map(|(id, name, team, _)| Seat { id: *id, name: name.clone(), team: *team }).collect();
    pending.0 = Some(MatchSetup {
        rules,
        role: *role,
        link,
        player_name: local.as_ref().map_or_else(|| "SERVER".to_string(), |l| l.1.clone()),
        local_player: local.as_ref().map(|l| l.0),
        local_team: local.as_ref().map_or(0, |l| l.2),
        carry: Some(Carry { seats, time_mode: state.time_mode, weather_mode: state.weather_mode, pick: saved.map(|saved| saved.pick) }),
    });
    end_match(&mut next);
}

pub fn new_arsenal(catalog: &Catalog, rules: &Rules) -> Arsenal {
    let mut arsenal = Arsenal { weapons: catalog.weapons.iter().map(WeaponState::fresh).collect(), ..default() };
    for index in 0..catalog.weapons.len() {
        // Only the first 32 weapons can be starters (a catalog may hold more).
        if 1u32.checked_shl(index as u32).is_some_and(|bit| rules.starter_weapons & bit != 0) {
            grant_weapon(&mut arsenal, &catalog.weapons, index);
        }
    }
    arsenal.current = (0..catalog.weapons.len()).find(|i| arsenal.owns(*i)).unwrap_or(0) as u8;
    arsenal
}

/// The components every player entity has, on every machine.
pub fn player_bundle(catalog: &Catalog, rules: &Rules, map: &GameMap, id: u8, name: String, team: u8) -> impl Bundle {
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
        PlayerInput { weapon: arsenal.current, yaw, ..default() },
        arsenal,
        Health::full(100.0),
        Mover::default(),
        Transform::from_translation(spawn).with_rotation(Quat::from_rotation_y(yaw)),
    )
}

pub fn start_pending_match(
    time: Res<Time>,
    mut pending: ResMut<PendingMatch>,
    modes: Res<Modes>,
    mut commands: Commands,
    mut next: ResMut<NextState<MatchPhase>>,
    current: Res<State<MatchPhase>>,
) {
    if *current.get() != MatchPhase::None {
        return;
    }
    let Some(mut setup) = pending.0.take() else { return };
    let Some(mode) = modes.0.get(setup.rules.mode as usize) else {
        warn!("unknown game mode {}", setup.rules.mode);
        return;
    };
    let rules = setup.rules;
    let catalog = Arc::new((mode.catalog)(&rules));
    let map = Arc::new(GameMap::generate(rules.recipe));
    let mut ids = NetIds::default();

    // Static collision is rebuilt from the recipe on every machine; it is never replicated.
    let mut statics = map.statics();
    let standing = |at: Vec3, half: Vec3| Aabb::new(at + Vec3::Y * half.y, half);
    for (index, pad) in map.pads.iter().enumerate().take(catalog.stations.len()) {
        let facing = Quat::from_rotation_y(f32::atan2(pad.x - map.core.x, pad.y - map.core.y));
        let at = map.ground(pad.x, pad.y);
        statics.add(standing(at, STATION_HALF));
        commands.spawn((MatchEntity, Station { kind: index as u16 }, Transform::from_translation(at).with_rotation(facing)));
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
                OreNode { ore: deposit.ore, amount: deposit.amount, max: deposit.amount },
                Transform::from_translation(deposit.pos),
            ));
        }
    }
    if let Some(id) = setup.local_player {
        let mut player = commands.spawn((player_bundle(&catalog, &rules, &map, id, setup.player_name.clone(), setup.local_team), LocalPlayer));
        if setup.role != NetRole::Client {
            player.insert(ids.next());
        }
    }
    // A relaunch: everyone who was connected keeps their seat.
    let carry = setup.carry.clone().unwrap_or_default();
    if setup.role != NetRole::Client {
        let now = time.elapsed_secs();
        for seat in &carry.seats {
            let spawn = spawn_point(&map, seat.id, rules.pvp.then_some(seat.team));
            let mut guest = commands.spawn((player_bundle(&catalog, &rules, &map, seat.id, seat.name.clone(), seat.team), ids.next(), crate::net::remote_seat(now, spawn)));
            // A guest still connected rebuilds this world too; until they say so they are loading.
            if matches!(setup.link, NetLink::Host(ref host) if host.peers.values().any(|id| *id == seat.id)) {
                guest.insert(Loading { waited: 0.0 });
            }
        }
    }
    if let NetLink::Host(host) = &mut setup.link {
        host.resend_soon();
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
        // The lobby's weather setting is for versus; co-op maps keep their own look.
        weather: match carry.weather_mode {
            n @ 1..=4 if rules.pvp => oz_core::worldgen::Weather::ALL[(n - 1) as usize],
            _ => rules.recipe.weather,
        },
        time_mode: carry.time_mode,
        weather_mode: carry.weather_mode,
        next_mode: carry.pick.map_or(rules.mode, |pick| pick.0),
        next_map: carry.pick.map_or_else(|| crate::versus::map_choice(&rules), |pick| pick.1),
        ..default()
    });
    commands.insert_resource(Relaunched(setup.carry.is_some()));
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
