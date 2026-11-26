//! Host/client replication.
//!
//! The model is deliberately simple and cheap:
//!
//! * The host owns the simulation. Clients send their input (and their own
//!   position - movement is owner-authoritative so it feels instant) plus
//!   [`PlayerCommand`]s for anything that changes shared state.
//! * The host broadcasts quantised [`Snapshot`]s (15 bytes per entity) at
//!   20 Hz, dropping to 10 Hz for very large battles. Slow-changing entities
//!   (structures, ore) only travel in periodic "full" snapshots or when the
//!   set changes.
//! * One-shot effects travel as [`FxEvent`] batches, so clients replay the
//!   exact same tracers, explosions and sounds as the host.
//!
//! Clients spawn the same components the host has (`Enemy`, `Structure`,
//! ...), so the presentation layer does not know or care which side it is on.

use crate::{
    combat::{Projectile, Solid},
    enemy::Enemy,
    player::{Arsenal, Health, LocalPlayer, Player, PlayerInput},
    session::{Content, Map, MatchEntity, MatchSetup, PendingMatch, player_bundle},
    sim_dt,
    state::{CommandEvent, FxEvent, MatchPhase, MatchState, NetRole, PlayerCommand, Rules},
    structures::{OreNode, Structure},
};
use bevy::{platform::collections::HashMap, prelude::*};
use oz_core::{content::ShotLook, economy::Ore};
use oz_net::{Client, ClientEvent, Host, HostEvent, PeerId};
use serde::{Deserialize, Serialize};
use std::f32::consts::TAU;

pub const PROTOCOL: u32 = 1;
const INPUT_RATE: f32 = 1.0 / 30.0;
const FULL_SNAPSHOT_EVERY: f32 = 1.0;

/// Stable identity of a replicated entity across machines.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NetId(pub u32);

#[derive(Resource, Default)]
pub struct NetIds {
    last: u32,
}

impl NetIds {
    pub fn next(&mut self) -> NetId {
        self.last += 1;
        NetId(self.last)
    }
}

/// Interpolation target for entities whose position arrives over the network.
#[derive(Component, Clone, Copy, Debug)]
pub struct Remote {
    pub pos: Vec3,
    pub yaw: f32,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum C2S {
    Hello { protocol: u32, name: String },
    Input { input: PlayerInput, pos: Vec3 },
    Command(PlayerCommand),
}

#[derive(Serialize, Deserialize, Debug)]
pub enum S2C {
    Welcome { player: u8, rules: Rules },
    Refused(String),
    Snapshot(Snapshot),
    You(Private),
    Fx(Vec<FxEvent>),
}

mod kind {
    pub const PLAYER: u8 = 0;
    pub const ENEMY: u8 = 1;
    pub const PROJECTILE: u8 = 2;
    pub const STRUCTURE: u8 = 3;
    pub const ORE: u8 = 4;

    pub fn is_static(kind: u8) -> bool {
        kind >= STRUCTURE
    }
}

/// 15 bytes on the wire.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct EntState {
    pub id: u32,
    pub kind: u8,
    /// Player id, enemy kind, projectile look, structure kind or ore type.
    pub sub: u8,
    pub pos: [i16; 3],
    pub yaw: u8,
    /// Health fraction, 0..=255.
    pub hp: u8,
    /// Kind-specific byte: attack / shot counters, current weapon.
    pub aux: u8,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Snapshot {
    /// Full snapshots also carry static entities and prune ones that vanished.
    pub full: bool,
    pub state: MatchState,
    pub entities: Vec<EntState>,
}

/// State only its owner needs.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Private {
    pub health: Health,
    pub arsenal: Arsenal,
    pub respawn: f32,
    pub team: u8,
    pub gear_left: u8,
    /// Set while dead so the client lands on the respawn point.
    pub teleport: Option<Vec3>,
}

/// Position quantisation derived from the map size (about 1 cm on a 320 m map).
#[derive(Clone, Copy)]
struct Quant {
    xz: f32,
}

impl Quant {
    const Y: f32 = 0.01;

    fn new(rules: &Rules) -> Self {
        Self { xz: rules.recipe.half_extent / 32_000.0 }
    }

    fn pack(&self, p: Vec3) -> [i16; 3] {
        [(p.x / self.xz) as i16, (p.y / Self::Y).clamp(-32_000.0, 32_000.0) as i16, (p.z / self.xz) as i16]
    }

    fn unpack(&self, p: [i16; 3]) -> Vec3 {
        Vec3::new(p[0] as f32 * self.xz, p[1] as f32 * Self::Y, p[2] as f32 * self.xz)
    }
}

fn pack_yaw(rotation: Quat) -> u8 {
    let yaw = rotation.to_euler(EulerRot::YXZ).0;
    ((yaw.rem_euclid(TAU) / TAU) * 256.0) as u8
}

fn unpack_yaw(yaw: u8) -> f32 {
    yaw as f32 / 256.0 * TAU
}

fn fraction(health: &Health) -> u8 {
    if health.alive() { ((health.fraction() * 255.0) as u8).max(1) } else { 0 }
}

pub struct HostLink {
    pub host: Host<C2S, S2C>,
    /// Peer -> player id, once they have said hello.
    pub peers: HashMap<PeerId, u8>,
    send_timer: f32,
    full_timer: f32,
}

pub struct ClientLink {
    pub client: Client<S2C, C2S>,
    pub player: u8,
    send_timer: f32,
    /// Net id -> (local entity, static?).
    entities: HashMap<u32, (Entity, bool)>,
}

#[derive(Resource, Default)]
pub enum NetLink {
    #[default]
    None,
    Host(HostLink),
    Client(ClientLink),
}

impl NetLink {
    pub fn remote_players(&self) -> usize {
        match self {
            NetLink::Host(link) => link.peers.len(),
            _ => 0,
        }
    }
}

/// Opens a listen server and returns a setup ready for [`PendingMatch`].
pub fn host_setup(rules: Rules, player_name: impl Into<String>, addr: &str, dedicated: bool) -> std::io::Result<MatchSetup> {
    let host = Host::bind(addr)?;
    let link = NetLink::Host(HostLink { host, peers: HashMap::default(), send_timer: 0.0, full_timer: 0.0 });
    Ok(MatchSetup { rules, role: NetRole::Host, link, player_name: player_name.into(), local_player: (!dedicated).then_some(0) })
}

/// A connection that is waiting for the host's welcome.
#[derive(Resource)]
pub struct Joining {
    client: Option<Client<S2C, C2S>>,
    name: String,
    waited: f32,
}

/// Outcome of the last join attempt, for menus to display.
#[derive(Resource, Default, Clone, Debug, PartialEq)]
pub enum JoinStatus {
    #[default]
    Idle,
    Connecting,
    Failed(String),
}

pub fn join(commands: &mut Commands, addr: &str, name: impl Into<String>) {
    let name = name.into();
    match Client::connect(addr) {
        Ok(mut client) => {
            client.send(&C2S::Hello { protocol: PROTOCOL, name: name.clone() });
            commands.insert_resource(Joining { client: Some(client), name, waited: 0.0 });
            commands.insert_resource(JoinStatus::Connecting);
        }
        Err(error) => commands.insert_resource(JoinStatus::Failed(format!("COULD NOT CONNECT: {error}"))),
    }
}

pub fn poll_joining(time: Res<Time>, joining: Option<ResMut<Joining>>, mut pending: ResMut<PendingMatch>, mut commands: Commands) {
    let Some(mut joining) = joining else { return };
    joining.waited += time.delta_secs();
    let mut outcome = None;
    let events = joining.client.as_mut().map(|client| client.poll()).unwrap_or_default();
    for event in events {
        match event {
            ClientEvent::Message(S2C::Welcome { player, rules }) => outcome = Some(Ok((player, rules))),
            ClientEvent::Message(S2C::Refused(why)) => outcome = Some(Err(why)),
            ClientEvent::Disconnected => outcome = Some(Err("HOST CLOSED THE CONNECTION".into())),
            ClientEvent::Message(_) => {}
        }
    }
    if joining.waited > 8.0 && outcome.is_none() {
        outcome = Some(Err("HOST DID NOT ANSWER".into()));
    }
    let Some(outcome) = outcome else { return };
    commands.remove_resource::<Joining>();
    match outcome {
        Ok((player, rules)) => {
            // Take the socket out of the resource we are about to drop.
            let name = joining.name.clone();
            let Some(client) = joining.client.take() else { return };
            let link = NetLink::Client(ClientLink { client, player, send_timer: 0.0, entities: HashMap::default() });
            pending.0 = Some(MatchSetup { rules, role: NetRole::Client, link, player_name: name, local_player: Some(player) });
            commands.insert_resource(JoinStatus::Idle);
        }
        Err(why) => commands.insert_resource(JoinStatus::Failed(why)),
    }
}

/// Host: accept players, apply their input, forward their commands.
#[allow(clippy::too_many_arguments)]
pub fn host_receive(
    mut link: ResMut<NetLink>,
    content: Res<Content>,
    map: Res<Map>,
    rules: Res<Rules>,
    mut ids: ResMut<NetIds>,
    mut commands: Commands,
    mut players: Query<(Entity, &Player, &mut PlayerInput, &mut Remote)>,
    mut orders: EventWriter<CommandEvent>,
    mut fx: EventWriter<FxEvent>,
) {
    let NetLink::Host(link) = &mut *link else { return };
    for event in link.host.poll() {
        match event {
            HostEvent::Connected(_) => {}
            HostEvent::Message(peer, C2S::Hello { protocol, name }) => {
                if protocol != PROTOCOL {
                    link.host.send(peer, &S2C::Refused("VERSION MISMATCH".into()));
                    continue;
                }
                let Some(id) = (1..crate::bots::FIRST_BOT_ID).find(|id| !link.peers.values().any(|used| used == id)) else {
                    link.host.send(peer, &S2C::Refused("SERVER FULL".into()));
                    continue;
                };
                let name: String = name.chars().filter(|c| !c.is_control()).take(16).collect();
                let bundle = player_bundle(&content, &rules, &map, id, name.clone());
                let spawn = crate::player::spawn_point(&map, id, rules.pvp.then_some(id % 2));
                commands.spawn((bundle, ids.next(), Remote { pos: spawn, yaw: 0.0 }));
                link.peers.insert(peer, id);
                link.full_timer = 0.0;
                link.host.send(peer, &S2C::Welcome { player: id, rules: rules.clone() });
                fx.write(FxEvent::notice(format!("{name} JOINED")));
            }
            HostEvent::Message(peer, message) => {
                let Some(&id) = link.peers.get(&peer) else { continue };
                match message {
                    C2S::Input { input, pos } => {
                        if let Some((_, _, mut current, mut remote)) = players.iter_mut().find(|(_, player, ..)| player.id == id) {
                            *current = input;
                            if pos.is_finite() {
                                remote.pos = pos;
                                remote.yaw = input.yaw;
                            }
                        }
                    }
                    C2S::Command(command) => {
                        orders.write(CommandEvent { player: id, command });
                    }
                    C2S::Hello { .. } => {}
                }
            }
            HostEvent::Disconnected(peer) => {
                let Some(id) = link.peers.remove(&peer) else { continue };
                if let Some((entity, player, ..)) = players.iter().find(|(_, player, ..)| player.id == id) {
                    fx.write(FxEvent::notice(format!("{} LEFT", player.name)));
                    commands.entity(entity).despawn();
                }
            }
        }
    }
}

/// Host: broadcast snapshots, private state and effects.
#[allow(clippy::too_many_arguments)]
pub fn host_send(
    time: Res<Time>,
    mut link: ResMut<NetLink>,
    rules: Res<Rules>,
    state: Res<MatchState>,
    players: Query<(&NetId, &Player, &Transform, &Health, &Arsenal, Has<Remote>)>,
    enemies: Query<(&NetId, &Enemy, &Transform, &Health)>,
    shots: Query<(&NetId, &Projectile, &Transform)>,
    structures: Query<(&NetId, &Structure, &Transform, &Health)>,
    ores: Query<(&NetId, &OreNode, &Transform)>,
    changed: Query<(), Or<(Added<Structure>, Added<OreNode>)>>,
    mut removed: RemovedComponents<Structure>,
    mut fx: EventReader<FxEvent>,
) {
    let NetLink::Host(link) = &mut *link else {
        fx.clear();
        return;
    };
    let statics_changed = !changed.is_empty() || removed.read().count() > 0;
    if link.peers.is_empty() {
        fx.clear();
        return;
    }
    let effects: Vec<FxEvent> = fx.read().cloned().collect();
    if !effects.is_empty() {
        link.host.broadcast(&S2C::Fx(effects));
    }

    let dt = time.delta_secs();
    link.send_timer -= dt;
    link.full_timer -= dt;
    if statics_changed {
        link.full_timer = 0.0;
    }
    if link.send_timer > 0.0 {
        return;
    }
    // Big battles halve the rate; interpolation on the client hides it.
    link.send_timer = if enemies.iter().len() > 400 { 0.1 } else { 0.05 };
    let full = link.full_timer <= 0.0;
    if full {
        link.full_timer = FULL_SNAPSHOT_EVERY;
    }

    let q = Quant::new(&rules);
    let mut entities = Vec::with_capacity(players.iter().len() + enemies.iter().len() + shots.iter().len());
    for (id, player, transform, health, arsenal, _) in &players {
        entities.push(EntState { id: id.0, kind: kind::PLAYER, sub: player.id, pos: q.pack(transform.translation), yaw: pack_yaw(transform.rotation), hp: fraction(health), aux: (arsenal.current & 0x3f) | (player.team << 6) });
    }
    for (id, enemy, transform, health) in &enemies {
        entities.push(EntState { id: id.0, kind: kind::ENEMY, sub: enemy.kind as u8, pos: q.pack(transform.translation), yaw: pack_yaw(transform.rotation), hp: fraction(health), aux: enemy.attacks });
    }
    for (id, shot, transform) in &shots {
        entities.push(EntState { id: id.0, kind: kind::PROJECTILE, sub: shot.look as u8, pos: q.pack(transform.translation), yaw: pack_yaw(transform.rotation), hp: 255, aux: 0 });
    }
    if full {
        for (id, structure, transform, health) in &structures {
            entities.push(EntState { id: id.0, kind: kind::STRUCTURE, sub: structure.kind as u8, pos: q.pack(transform.translation), yaw: pack_yaw(transform.rotation), hp: fraction(health), aux: structure.shots });
        }
        for (id, node, transform) in &ores {
            let left = ((node.amount / node.max.max(1.0)) * 255.0).ceil() as u8;
            entities.push(EntState { id: id.0, kind: kind::ORE, sub: node.ore as u8, pos: q.pack(transform.translation), yaw: 0, hp: left, aux: 0 });
        }
    }
    link.host.broadcast(&S2C::Snapshot(Snapshot { full, state: state.clone(), entities }));

    let peers: Vec<(PeerId, u8)> = link.peers.iter().map(|(peer, id)| (*peer, *id)).collect();
    for (peer, id) in peers {
        if let Some((_, player, transform, health, arsenal, _)) = players.iter().find(|(_, player, ..)| player.id == id) {
            // Just-spawned players are also pinned so they start at their base.
            let teleport = (!health.alive() || player.respawn > 0.0 || player.life < 0.4).then_some(transform.translation);
            link.host.send(peer, &S2C::You(Private { health: *health, arsenal: arsenal.clone(), respawn: player.respawn, team: player.team, gear_left: player.gear_left, teleport }));
        }
    }
}

/// Client: apply snapshots, private state and effects.
#[allow(clippy::too_many_arguments)]
pub fn client_receive(
    mut link: ResMut<NetLink>,
    content: Res<Content>,
    rules: Res<Rules>,
    mut state: ResMut<MatchState>,
    mut commands: Commands,
    mut local: Query<(&mut Health, &mut Arsenal, &mut Player, &mut Transform), With<LocalPlayer>>,
    mut remotes: Query<(&mut Remote, Option<&mut Health>, Option<&mut Enemy>, Option<&mut Structure>, Option<&mut OreNode>, Option<&mut PlayerInput>), Without<LocalPlayer>>,
    mut fx: EventWriter<FxEvent>,
    mut next: ResMut<NextState<MatchPhase>>,
    mut was_dead: Local<bool>,
) {
    let NetLink::Client(link) = &mut *link else { return };
    let q = Quant::new(&rules);
    for event in link.client.poll() {
        match event {
            ClientEvent::Message(S2C::Snapshot(snapshot)) => {
                *state = snapshot.state;
                let mut seen = bevy::platform::collections::HashSet::with_capacity(snapshot.entities.len());
                for e in &snapshot.entities {
                    seen.insert(e.id);
                    if e.kind == kind::PLAYER && e.sub == link.player {
                        continue;
                    }
                    let (pos, yaw) = (q.unpack(e.pos), unpack_yaw(e.yaw));
                    if let Some((entity, _)) = link.entities.get(&e.id) {
                        let Ok((mut remote, health, enemy, structure, ore, input)) = remotes.get_mut(*entity) else { continue };
                        *remote = Remote { pos, yaw };
                        if let Some(mut health) = health {
                            health.hp = health.max * e.hp as f32 / 255.0;
                        }
                        if let Some(mut enemy) = enemy {
                            enemy.attacks = e.aux;
                        }
                        if let Some(mut structure) = structure {
                            structure.shots = e.aux;
                        }
                        if let Some(mut ore) = ore {
                            ore.amount = ore.max * e.hp as f32 / 255.0;
                        }
                        if let Some(mut input) = input {
                            input.weapon = e.aux & 0x3f;
                        }
                        continue;
                    }
                    let base = (MatchEntity, NetId(e.id), Remote { pos, yaw }, Transform::from_translation(pos).with_rotation(Quat::from_rotation_y(yaw)));
                    let hp = e.hp as f32 / 255.0;
                    let entity = match e.kind {
                        kind::PLAYER => commands
                            .spawn((base, Player::new(e.sub, format!("PLAYER {}", e.sub + 1), e.aux >> 6), PlayerInput { weapon: e.aux & 0x3f, ..default() }, Health { hp: 100.0 * hp, max: 100.0 }))
                            .id(),
                        kind::ENEMY => {
                            let Some(def) = content.enemies.get(e.sub as usize) else { continue };
                            commands.spawn((base, Enemy { kind: e.sub as u16, attacks: e.aux, ..default() }, Health { hp: def.hp * hp, max: def.hp })).id()
                        }
                        kind::PROJECTILE => {
                            let shot = Projectile { velocity: Vec3::ZERO, gravity: 0.0, damage: 0.0, splash: 0.0, look: ShotLook::from_index(e.sub), owner: None, team: 255, life: 1.0 };
                            commands.spawn((base, shot)).id()
                        }
                        kind::STRUCTURE => {
                            let Some(def) = content.structures.get(e.sub as usize) else { continue };
                            let mut structure = Structure::new(e.sub as u16, None);
                            structure.shots = e.aux;
                            commands.spawn((base, structure, Health { hp: def.hp * hp, max: def.hp }, Solid { half: crate::structures::oriented_half(def.half, yaw) })).id()
                        }
                        _ => commands.spawn((base, OreNode { ore: Ore::from_index(e.sub), amount: hp, max: 1.0 }, Solid { half: Vec3::new(0.8, 0.7, 0.8) })).id(),
                    };
                    link.entities.insert(e.id, (entity, kind::is_static(e.kind)));
                }
                link.entities.retain(|id, (entity, is_static)| {
                    let keep = seen.contains(id) || (*is_static && !snapshot.full);
                    if !keep {
                        commands.entity(*entity).try_despawn();
                    }
                    keep
                });
            }
            ClientEvent::Message(S2C::You(private)) => {
                if let Ok((mut health, mut arsenal, mut player, mut transform)) = local.single_mut() {
                    *health = private.health;
                    *arsenal = private.arsenal;
                    player.respawn = private.respawn;
                    player.team = private.team;
                    player.gear_left = private.gear_left;
                    // While dead the host owns our position, so we wake up at the spawn point.
                    if let Some(at) = private.teleport {
                        transform.translation = at;
                        *was_dead = true;
                    } else if *was_dead {
                        *was_dead = false;
                    }
                }
            }
            ClientEvent::Message(S2C::Fx(effects)) => {
                fx.write_batch(effects);
            }
            ClientEvent::Message(_) => {}
            ClientEvent::Disconnected => {
                commands.insert_resource(JoinStatus::Failed("CONNECTION TO HOST LOST".into()));
                crate::session::end_match(&mut next);
            }
        }
    }
}

/// Client: send input at a fixed rate, and commands immediately.
pub fn client_send(
    time: Res<Time>,
    mut link: ResMut<NetLink>,
    local: Query<(&PlayerInput, &Transform), With<LocalPlayer>>,
    mut orders: EventReader<CommandEvent>,
) {
    let NetLink::Client(link) = &mut *link else { return };
    for order in orders.read() {
        link.client.send(&C2S::Command(order.command.clone()));
    }
    link.send_timer -= time.delta_secs();
    if link.send_timer <= 0.0 {
        link.send_timer = INPUT_RATE;
        if let Ok((input, transform)) = local.single() {
            link.client.send(&C2S::Input { input: *input, pos: transform.translation });
        }
    }
}

/// Eases networked entities toward their latest known position.
pub fn smooth_remote(time: Res<Time>, mut remotes: Query<(&mut Transform, &Remote)>) {
    let blend = 1.0 - (-14.0 * sim_dt(&time)).exp();
    for (mut transform, remote) in &mut remotes {
        if transform.translation.distance_squared(remote.pos) > 64.0 {
            transform.translation = remote.pos;
        } else {
            transform.translation = transform.translation.lerp(remote.pos, blend);
        }
        transform.rotation = transform.rotation.slerp(Quat::from_rotation_y(remote.yaw), blend);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_state_is_compact() {
        let state = EntState { id: 1, kind: 1, sub: 2, pos: [1, 2, 3], yaw: 4, hp: 5, aux: 6 };
        assert_eq!(bincode::serialize(&state).unwrap().len(), 15);
    }

    #[test]
    fn quantisation_round_trips() {
        let q = Quant { xz: 160.0 / 32_000.0 };
        let p = Vec3::new(-151.37, 3.21, 88.04);
        assert!(q.unpack(q.pack(p)).distance(p) < 0.02);
        for yaw in [0.0, 1.0, 3.0, 6.0] {
            let back = unpack_yaw(pack_yaw(Quat::from_rotation_y(yaw)));
            assert!((back - yaw).abs() < TAU / 200.0, "{yaw} -> {back}");
        }
    }
}
