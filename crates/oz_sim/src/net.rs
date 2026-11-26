//! Host/client replication.
//!
//! The model is deliberately simple and cheap:
//!
//! * The host owns the simulation. Clients send their input (and their own
//!   position - movement is owner-authoritative so it feels instant) plus
//!   [`PlayerCommand`]s for anything that changes shared state.
//! * The host broadcasts quantised [`Snapshot`]s (17 bytes per entity) at
//!   20 Hz, dropping to 10 Hz for very large battles. Slow-changing entities
//!   (structures, ore) only travel in periodic "full" snapshots or when the
//!   set changes.
//! * Every snapshot is stamped with the host's clock. Clients keep a
//!   [`ServerClock`] and play remote entities back [`INTERPOLATION`] seconds
//!   behind it from a small buffer, which is smooth even with jitter. Inputs
//!   carry that playback time, so the host can rewind for lag compensation
//!   (see [`crate::lagcomp`]).
//! * Quick taps between input sends are latched so no shot is ever lost, and
//!   both sides drop connections that go silent.
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
    state::{CommandEvent, FxEvent, MatchPhase, MatchState, NetRole, PlayerCommand, Rules},
    structures::{OreNode, Structure},
};
use bevy::{platform::collections::HashMap, prelude::*};
use oz_core::{content::ShotLook, economy::Ore};
use oz_net::{
    Client, ClientEvent, Host, HostEvent, PeerId,
    lan::{Beacon, PortStatus, open_port},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    f32::consts::TAU,
    sync::{Arc, Mutex},
    time::Duration,
};

pub const PROTOCOL: u32 = 3;
/// How long the host remembers someone who dropped, so they can reconnect into the same seat.
pub const REJOIN_WINDOW: f32 = 600.0;
/// How far behind the host's clock clients play back remote entities.
pub const INTERPOLATION: f32 = 0.1;
/// How far behind the host plays back remote players' reported positions.
const HOST_INTERPOLATION: f32 = 0.05;
/// Connections silent for this long are treated as gone.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(6);
const HOST_TIMEOUT: Duration = Duration::from_secs(10);
const PING_EVERY: f32 = 1.0;
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
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> NetId {
        self.last += 1;
        NetId(self.last)
    }
}

/// Recent positions of an entity driven over the network: (time, position, yaw), oldest first.
#[derive(Component, Clone, Debug, Default)]
pub struct Remote {
    samples: VecDeque<(f32, Vec3, f32)>,
}

impl Remote {
    pub fn at(time: f32, pos: Vec3, yaw: f32) -> Self {
        let mut remote = Self::default();
        remote.push(time, pos, yaw);
        remote
    }

    pub fn push(&mut self, time: f32, pos: Vec3, yaw: f32) {
        // Out-of-order or same-time samples replace the newest instead of reordering.
        if self.samples.back().is_some_and(|(t, ..)| *t >= time) {
            self.samples.pop_back();
        }
        self.samples.push_back((time, pos, yaw));
        while self.samples.len() > 12 {
            self.samples.pop_front();
        }
    }

    pub fn latest(&self) -> Option<(Vec3, f32)> {
        self.samples.back().map(|(_, pos, yaw)| (*pos, *yaw))
    }

    /// Position and yaw at `time`, interpolated between the samples around it.
    pub fn sample(&self, time: f32) -> Option<(Vec3, f32)> {
        let after = self.samples.iter().position(|(t, ..)| *t >= time);
        match after {
            None => self.latest(),
            Some(0) => self.samples.front().map(|(_, p, y)| (*p, *y)),
            Some(i) => {
                let ((ta, pa, ya), (tb, pb, yb)) = (self.samples[i - 1], self.samples[i]);
                // Teleports (respawns) snap instead of sliding across the map.
                if pa.distance(pb) > 8.0 {
                    return Some((pb, yb));
                }
                let k = ((time - ta) / (tb - ta).max(1e-4)).clamp(0.0, 1.0);
                let turn = (yb - ya + std::f32::consts::PI).rem_euclid(TAU) - std::f32::consts::PI;
                Some((pa.lerp(pb, k), ya + turn * k))
            }
        }
    }
}

/// Client: estimate of the host's clock (`host time = local time + offset`).
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct ServerClock {
    pub offset: f32,
    pub synced: bool,
}

impl ServerClock {
    pub fn now(&self, local: f32) -> f32 {
        local + self.offset
    }

    /// The moment of the world a client is currently showing.
    pub fn view(&self, local: f32) -> f32 {
        self.now(local) - INTERPOLATION
    }

    /// Feeds a snapshot timestamp. Fast arrivals pull the estimate forward at once;
    /// slow ones (network hiccups) only nudge it back, so jitter never makes time jump backwards.
    fn observe(&mut self, server_time: f32, local: f32) {
        let sample = server_time - local;
        if !self.synced || sample > self.offset {
            self.offset = sample;
            self.synced = true;
        } else {
            self.offset += (sample - self.offset) * 0.02;
        }
    }
}

/// Round-trip time to the host in milliseconds (clients), for display.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct Ping(pub u16);

#[derive(Serialize, Deserialize, Debug)]
pub enum C2S {
    /// `token` identifies this install across reconnects (0 = none).
    Hello { protocol: u32, name: String, token: u64 },
    /// `view` is the server time of the world the client is showing (for lag compensation).
    Input { input: PlayerInput, pos: Vec3, view: f32 },
    Command(PlayerCommand),
    /// `sent` comes back in a pong; `rtt` reports the last measured round trip in ms.
    Ping { sent: f32, rtt: u16 },
}

#[derive(Serialize, Deserialize, Debug)]
pub enum S2C {
    Welcome { player: u8, team: u8, rules: Rules },
    Refused(String),
    Snapshot(Snapshot),
    You(Private),
    Fx(Vec<FxEvent>),
    Pong { sent: f32 },
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

/// 17 bytes on the wire.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct EntState {
    pub id: u32,
    pub kind: u8,
    /// Player id, enemy kind, projectile look, structure kind or ore type.
    pub sub: u8,
    pub pos: [i16; 3],
    pub yaw: u8,
    /// Look pitch (players), quantised to i8.
    pub pitch: i8,
    /// Health fraction, 0..=255.
    pub hp: u8,
    /// Kind-specific byte: attack / shot counters, current weapon.
    pub aux: u8,
    /// Players: presentation flags (stance, aiming, reloading...).
    pub flags: u8,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Snapshot {
    /// The host's clock when this snapshot was taken.
    pub time: f32,
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

fn pack_pitch(pitch: f32) -> i8 {
    (pitch / 1.5 * 127.0).clamp(-127.0, 127.0) as i8
}

fn unpack_pitch(pitch: i8) -> f32 {
    pitch as f32 / 127.0 * 1.5
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
    /// The port players connect to.
    pub port: u16,
    /// Answers "who is hosting?" on the local network.
    beacon: Option<Beacon>,
    /// Automatic router port forwarding, if attempted.
    pub router: Option<Arc<Mutex<PortStatus>>>,
    /// Peer -> reconnect token.
    tokens: HashMap<PeerId, u64>,
    /// Players who dropped recently, kept so they can take their seat back.
    departed: Vec<Departed>,
}

/// Everything needed to put a reconnecting player back where they were.
#[derive(Clone, Debug)]
struct Departed {
    token: u64,
    id: u8,
    team: u8,
    kills: u16,
    deaths: u16,
    perk: u8,
    gear: u8,
    loadout: Option<crate::player::Loadout>,
    arsenal: Arsenal,
    left_at: f32,
}

impl HostLink {
    pub fn router_status(&self) -> Option<PortStatus> {
        self.router.as_ref().and_then(|status| status.lock().ok().map(|s| s.clone()))
    }
}

pub struct ClientLink {
    pub client: Client<S2C, C2S>,
    pub player: u8,
    /// Where we connected and who we are, for reconnecting.
    pub address: String,
    pub name: String,
    pub token: u64,
    send_timer: f32,
    ping_timer: f32,
    rtt: u16,
    /// Buttons pressed since the last input send, so short taps are never lost.
    latched: u8,
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
/// Loopback-only hosts (tests, local experiments) skip LAN discovery and router setup.
pub fn host_setup(rules: Rules, player_name: impl Into<String>, addr: &str, dedicated: bool) -> std::io::Result<MatchSetup> {
    let host = Host::bind(addr)?;
    let bound = host.local_addr()?;
    let public = !bound.ip().is_loopback();
    let link = NetLink::Host(HostLink {
        host,
        peers: HashMap::default(),
        send_timer: 0.0,
        full_timer: 0.0,
        port: bound.port(),
        beacon: if public { Beacon::bind() } else { None },
        router: public.then(|| open_port(bound.port())),
        tokens: HashMap::default(),
        departed: Vec::new(),
    });
    Ok(MatchSetup { rules, role: NetRole::Host, link, player_name: player_name.into(), local_player: (!dedicated).then_some(0), local_team: 0 })
}

/// A connection that is waiting for the host's welcome.
#[derive(Resource)]
pub struct Joining {
    client: Option<Client<S2C, C2S>>,
    name: String,
    address: String,
    token: u64,
    waited: f32,
}

/// Set when a client loses its host unexpectedly: where to reconnect.
#[derive(Resource, Clone, Debug)]
pub struct LostHost {
    pub address: String,
    pub name: String,
    pub token: u64,
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
    join_as(commands, addr, name, 0);
}

/// Joins with a reconnect token: a host that remembers the token gives back the same seat, team and score.
pub fn join_as(commands: &mut Commands, addr: &str, name: impl Into<String>, token: u64) {
    let name = name.into();
    match Client::connect(addr) {
        Ok(mut client) => {
            client.send(&C2S::Hello { protocol: PROTOCOL, name: name.clone(), token });
            commands.insert_resource(Joining { client: Some(client), name, address: addr.to_string(), token, waited: 0.0 });
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
            ClientEvent::Message(S2C::Welcome { player, team, rules }) => outcome = Some(Ok((player, team, rules))),
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
        Ok((player, team, rules)) => {
            // Take the socket out of the resource we are about to drop.
            let name = joining.name.clone();
            let Some(client) = joining.client.take() else { return };
            let link = NetLink::Client(ClientLink {
                client,
                player,
                address: joining.address.clone(),
                name: name.clone(),
                token: joining.token,
                send_timer: 0.0,
                ping_timer: 0.0,
                rtt: 0,
                latched: 0,
                entities: HashMap::default(),
            });
            pending.0 = Some(MatchSetup { rules, role: NetRole::Client, link, player_name: name, local_player: Some(player), local_team: team });
            commands.insert_resource(JoinStatus::Idle);
        }
        Err(why) => commands.insert_resource(JoinStatus::Failed(why)),
    }
}

/// Host: accept players, apply their input, forward their commands.
#[allow(clippy::too_many_arguments)]
pub fn host_receive(
    time: Res<Time>,
    mut link: ResMut<NetLink>,
    content: Res<Content>,
    map: Res<Map>,
    rules: Res<Rules>,
    modes: Res<crate::session::Modes>,
    state: Res<MatchState>,
    mut ids: ResMut<NetIds>,
    mut commands: Commands,
    mut players: Query<(Entity, &mut Player, &mut PlayerInput, &mut Remote, &Arsenal)>,
    // Everyone not driven over the network (local player, bots); remote players come from `players`.
    everyone: Query<(&Player, Has<crate::bots::Bot>), Without<Remote>>,
    mut orders: EventWriter<CommandEvent>,
    mut fx: EventWriter<FxEvent>,
) {
    let NetLink::Host(link) = &mut *link else { return };
    let now = time.elapsed_secs();
    if let Some(beacon) = &link.beacon {
        let mode = modes.0.get(rules.mode as usize).map_or("MATCH", |m| m.name);
        let host_name = state.roster.iter().find(|r| r.id == 0).map_or("DEDICATED SERVER", |r| r.name.as_str());
        let humans = state.roster.iter().filter(|r| !r.bot).count() as u8;
        beacon.poll(link.port, host_name, mode, humans);
    }
    // Peers that went quiet are dropped as if they had disconnected.
    let stale: Vec<PeerId> = link.peers.keys().copied().filter(|peer| link.host.silent_for(*peer).is_some_and(|d| d > HOST_TIMEOUT)).collect();
    let mut events = link.host.poll();
    for peer in stale {
        link.host.kick(peer);
        events.push(HostEvent::Disconnected(peer));
    }
    for event in events {
        match event {
            HostEvent::Connected(_) => {}
            HostEvent::Message(peer, C2S::Hello { protocol, name, token }) => {
                if protocol != PROTOCOL {
                    link.host.send(peer, &S2C::Refused("VERSION MISMATCH".into()));
                    continue;
                }
                // The same install connecting again while its old connection lingers: drop the old one.
                if token != 0 {
                    let old: Vec<PeerId> = link.tokens.iter().filter(|(p, t)| **t == token && **p != peer).map(|(p, _)| *p).collect();
                    for old_peer in old {
                        link.host.kick(old_peer);
                        link.tokens.remove(&old_peer);
                        if let Some(id) = link.peers.remove(&old_peer)
                            && let Some((entity, player, _, _, arsenal)) = players.iter().find(|(_, player, ..)| player.id == id) {
                                link.departed.push(Departed::from_player(token, player, arsenal, now));
                                commands.entity(entity).despawn();
                            }
                    }
                }
                link.departed.retain(|d| now - d.left_at < REJOIN_WINDOW);
                let back = (token != 0).then(|| link.departed.iter().position(|d| d.token == token)).flatten().map(|i| link.departed.remove(i));
                let free = |id: u8| !link.peers.values().any(|used| *used == id);
                let id = match back.as_ref().map(|d| d.id).filter(|id| free(*id)) {
                    Some(id) => id,
                    None => {
                        let Some(id) = (1..crate::bots::FIRST_BOT_ID).find(|id| free(*id)) else {
                            link.host.send(peer, &S2C::Refused("SERVER FULL".into()));
                            continue;
                        };
                        id
                    }
                };
                let mut name: String = name.chars().filter(|c| !c.is_control()).take(16).collect::<String>().trim().to_string();
                if name.is_empty() {
                    name = format!("PLAYER {id}");
                }
                // Two players with the same name get told apart.
                if everyone.iter().map(|(p, _)| &p.name).chain(players.iter().map(|(_, p, ..)| &p.name)).any(|n| *n == name) {
                    name = format!("{} {id}", name.chars().take(13).collect::<String>());
                }
                let team = match &back {
                    Some(seat) if rules.pvp => seat.team,
                    _ => crate::versus::team_for_newcomer(everyone.iter().map(|(p, bot)| (p.team, bot)).chain(players.iter().map(|(_, p, ..)| (p.team, false)))),
                };
                let bundle = player_bundle(&content, &rules, &map, id, name.clone(), team);
                let spawn = crate::player::spawn_point(&map, id, rules.pvp.then_some(team));
                let entity = commands.spawn((bundle, ids.next(), Remote::at(now, spawn, 0.0), crate::lagcomp::ViewTime(now))).id();
                if let Some(seat) = back.clone() {
                    let gear_count = content.gear.get(seat.gear as usize).map_or(0, |g| g.count);
                    commands.entity(entity).queue(move |mut entity: EntityWorldMut| seat.restore(&mut entity, gear_count));
                }
                link.peers.insert(peer, id);
                if token != 0 {
                    link.tokens.insert(peer, token);
                }
                link.full_timer = 0.0;
                link.host.send(peer, &S2C::Welcome { player: id, team, rules: rules.clone() });
                fx.write(FxEvent::notice(if back.is_some() { format!("{name} RECONNECTED") } else { format!("{name} JOINED") }));
            }
            HostEvent::Message(peer, message) => {
                let Some(&id) = link.peers.get(&peer) else { continue };
                match message {
                    C2S::Input { input, pos, view } => {
                        if let Some((entity, _, mut current, mut remote, _)) = players.iter_mut().find(|(_, player, ..)| player.id == id) {
                            *current = input;
                            if pos.is_finite() {
                                remote.push(now, pos, input.yaw);
                            }
                            if view.is_finite() {
                                commands.entity(entity).insert(crate::lagcomp::ViewTime(view));
                            }
                        }
                    }
                    C2S::Ping { sent, rtt } => {
                        link.host.send(peer, &S2C::Pong { sent });
                        if let Some((_, mut player, ..)) = players.iter_mut().find(|(_, player, ..)| player.id == id) {
                            player.ping = rtt;
                        }
                    }
                    C2S::Command(command) => {
                        orders.write(CommandEvent { player: id, command });
                    }
                    C2S::Hello { .. } => {}
                }
            }
            HostEvent::Disconnected(peer) => {
                let token = link.tokens.remove(&peer).unwrap_or(0);
                let Some(id) = link.peers.remove(&peer) else { continue };
                if let Some((entity, player, _, _, arsenal)) = players.iter().find(|(_, player, ..)| player.id == id) {
                    fx.write(FxEvent::notice(format!("{} LEFT", player.name)));
                    if token != 0 {
                        link.departed.push(Departed::from_player(token, player, arsenal, now));
                    }
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
    players: Query<(&NetId, &Player, &Transform, &Health, &Arsenal, &PlayerInput)>,
    enemies: Query<(&NetId, &Enemy, &Transform, &Health)>,
    (shots, mines): (Query<(&NetId, &Projectile, &Transform)>, Query<(&NetId, &crate::combat::Mine, &Transform)>),
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
    for (id, player, transform, health, arsenal, input) in &players {
        let pitch = pack_pitch(input.pitch);
        entities.push(EntState { id: id.0, kind: kind::PLAYER, sub: player.id, pos: q.pack(transform.translation), yaw: pack_yaw(transform.rotation), pitch, hp: fraction(health), aux: (arsenal.current & 0x3f) | (player.team << 6), flags: player.flags });
    }
    for (id, enemy, transform, health) in &enemies {
        entities.push(EntState { id: id.0, kind: kind::ENEMY, sub: enemy.kind as u8, pos: q.pack(transform.translation), yaw: pack_yaw(transform.rotation), pitch: 0, hp: fraction(health), aux: enemy.attacks, flags: 0 });
    }
    for (id, shot, transform) in &shots {
        // aux = owner + 1, so clients can hide copies of projectiles they already predicted.
        entities.push(EntState { id: id.0, kind: kind::PROJECTILE, sub: shot.look as u8, pos: q.pack(transform.translation), yaw: pack_yaw(transform.rotation), pitch: 0, hp: 255, aux: shot.owner.map_or(0, |o| o.saturating_add(1)), flags: 0 });
    }
    // Mines travel as projectiles that never move.
    for (id, _, transform) in &mines {
        entities.push(EntState { id: id.0, kind: kind::PROJECTILE, sub: ShotLook::Mine as u8, pos: q.pack(transform.translation), yaw: 0, pitch: 0, hp: 255, aux: 0, flags: 0 });
    }
    if full {
        for (id, structure, transform, health) in &structures {
            entities.push(EntState { id: id.0, kind: kind::STRUCTURE, sub: structure.kind as u8, pos: q.pack(transform.translation), yaw: pack_yaw(transform.rotation), pitch: 0, hp: fraction(health), aux: structure.shots, flags: 0 });
        }
        for (id, node, transform) in &ores {
            let left = ((node.amount / node.max.max(1.0)) * 255.0).ceil() as u8;
            entities.push(EntState { id: id.0, kind: kind::ORE, sub: node.ore as u8, pos: q.pack(transform.translation), yaw: 0, pitch: 0, hp: left, aux: 0, flags: 0 });
        }
    }
    link.host.broadcast(&S2C::Snapshot(Snapshot { time: time.elapsed_secs(), full, state: state.clone(), entities }));

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
    time: Res<Time>,
    mut clock: ResMut<ServerClock>,
    mut ping: ResMut<Ping>,
    mut link: ResMut<NetLink>,
    content: Res<Content>,
    rules: Res<Rules>,
    mut state: ResMut<MatchState>,
    mut commands: Commands,
    mut local: Query<(&mut Health, &mut Arsenal, &mut Player, &mut Transform), With<LocalPlayer>>,
    mut remotes: Query<(&mut Remote, Option<&mut Health>, Option<&mut Enemy>, Option<&mut Structure>, Option<&mut OreNode>, Option<&mut PlayerInput>, Option<&mut Player>), Without<LocalPlayer>>,
    mut fx: EventWriter<FxEvent>,
    mut next: ResMut<NextState<MatchPhase>>,
    mut was_dead: Local<bool>,
) {
    let NetLink::Client(link) = &mut *link else { return };
    let q = Quant::new(&rules);
    let local_now = time.elapsed_secs();
    let mut events = link.client.poll();
    if link.client.silent_for() > CLIENT_TIMEOUT && link.client.is_connected() {
        events.push(ClientEvent::Disconnected);
    }
    for event in events {
        match event {
            ClientEvent::Message(S2C::Pong { sent }) => {
                link.rtt = ((local_now - sent).max(0.0) * 1000.0) as u16;
                ping.0 = link.rtt;
            }
            ClientEvent::Message(S2C::Snapshot(snapshot)) => {
                clock.observe(snapshot.time, local_now);
                *state = snapshot.state.clone();
                let mut seen = bevy::platform::collections::HashSet::with_capacity(snapshot.entities.len());
                for e in &snapshot.entities {
                    seen.insert(e.id);
                    if e.kind == kind::PLAYER && e.sub == link.player {
                        continue;
                    }
                    let (pos, yaw) = (q.unpack(e.pos), unpack_yaw(e.yaw));
                    if let Some((entity, _)) = link.entities.get(&e.id) {
                        let Ok((mut remote, health, enemy, structure, ore, input, player)) = remotes.get_mut(*entity) else { continue };
                        remote.push(snapshot.time, pos, yaw);
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
                            input.pitch = unpack_pitch(e.pitch);
                            input.stance = e.flags & crate::player::flags::STANCE;
                            input.lean = crate::player::flags::lean(e.flags);
                        }
                        if let Some(mut player) = player {
                            player.flags = e.flags;
                            player.team = e.aux >> 6;
                            if let Some(entry) = snapshot.state.roster.iter().find(|r| r.id == player.id)
                                && player.name != entry.name {
                                    player.name = entry.name.clone();
                                }
                        }
                        continue;
                    }
                    let base = (MatchEntity, NetId(e.id), Remote::at(snapshot.time, pos, yaw), Transform::from_translation(pos).with_rotation(Quat::from_rotation_y(yaw)));
                    let hp = e.hp as f32 / 255.0;
                    let entity = match e.kind {
                        kind::PLAYER => commands
                            .spawn((
                                base,
                                Player { flags: e.flags, ..Player::new(e.sub, format!("PLAYER {}", e.sub + 1), e.aux >> 6) },
                                PlayerInput { weapon: e.aux & 0x3f, pitch: unpack_pitch(e.pitch), stance: e.flags & crate::player::flags::STANCE, lean: crate::player::flags::lean(e.flags), ..default() },
                                Health { hp: 100.0 * hp, max: 100.0 },
                            ))
                            .id(),
                        kind::ENEMY => {
                            let Some(def) = content.enemies.get(e.sub as usize) else { continue };
                            commands.spawn((base, Enemy { kind: e.sub as u16, attacks: e.aux, ..default() }, Health { hp: def.hp * hp, max: def.hp })).id()
                        }
                        kind::PROJECTILE => {
                            let shot = Projectile { velocity: Vec3::ZERO, gravity: 0.0, damage: 0.0, splash: 0.0, look: ShotLook::from_index(e.sub), owner: e.aux.checked_sub(1), team: 255, life: 1.0, effect: crate::combat::Impact::Blast };
                            commands.spawn((base, shot)).id()
                        }
                        kind::STRUCTURE => {
                            let Some(def) = content.structures.get(e.sub as usize) else { continue };
                            let mut structure = Structure::new(e.sub as u16, None);
                            structure.shots = e.aux;
                            commands.spawn((base, structure, Health { hp: def.hp * hp, max: def.hp }, Solid { half: crate::structures::oriented_half(def.half, yaw) })).id()
                        }
                        _ => commands.spawn((base, OreNode { ore: Ore::from_index(e.sub), amount: hp, max: 1.0 })).id(),
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
                commands.insert_resource(LostHost { address: link.address.clone(), name: link.name.clone(), token: link.token });
                let why = if link.client.silent_for() > CLIENT_TIMEOUT { "THE HOST STOPPED RESPONDING" } else { "THE HOST CLOSED THE MATCH" };
                commands.insert_resource(JoinStatus::Failed(why.into()));
                crate::session::end_match(&mut next);
                break;
            }
        }
    }
}

/// Client: send input at a fixed rate, and commands immediately.
pub fn client_send(
    time: Res<Time>,
    clock: Res<ServerClock>,
    mut link: ResMut<NetLink>,
    local: Query<(&PlayerInput, &Transform), With<LocalPlayer>>,
    mut orders: EventReader<CommandEvent>,
) {
    let NetLink::Client(link) = &mut *link else { return };
    for order in orders.read() {
        link.client.send(&C2S::Command(order.command.clone()));
    }
    let Ok((input, transform)) = local.single() else { return };
    link.latched |= input.buttons;
    let now = time.elapsed_secs();
    link.ping_timer -= time.delta_secs();
    if link.ping_timer <= 0.0 {
        link.ping_timer = PING_EVERY;
        link.client.send(&C2S::Ping { sent: now, rtt: link.rtt });
    }
    link.send_timer -= time.delta_secs();
    if link.send_timer <= 0.0 {
        link.send_timer = INPUT_RATE;
        // Anything pressed since the last send counts, so a quick tap still fires.
        let sent = PlayerInput { buttons: input.buttons | link.latched, ..*input };
        link.latched = 0;
        link.client.send(&C2S::Input { input: sent, pos: transform.translation, view: clock.view(now) });
    }
}

/// Plays networked entities back from their sample buffers, a little in the past.
pub fn smooth_remote(time: Res<Time>, role: Res<NetRole>, clock: Res<ServerClock>, mut remotes: Query<(&mut Transform, &Remote)>) {
    let now = time.elapsed_secs();
    let playback = if *role == NetRole::Client { clock.view(now) } else { now - HOST_INTERPOLATION };
    for (mut transform, remote) in &mut remotes {
        if let Some((pos, yaw)) = remote.sample(playback) {
            transform.translation = pos;
            transform.rotation = Quat::from_rotation_y(yaw);
        }
    }
}

impl Departed {
    fn from_player(token: u64, player: &Player, arsenal: &Arsenal, now: f32) -> Self {
        Self {
            token,
            id: player.id,
            team: player.team,
            kills: player.kills,
            deaths: player.deaths,
            perk: player.perk,
            gear: player.gear,
            loadout: player.loadout,
            arsenal: arsenal.clone(),
            left_at: now,
        }
    }

    /// Puts the remembered score, class and weapons onto a fresh player.
    fn restore(&self, entity: &mut EntityWorldMut, gear_count: u8) {
        if let Some(mut player) = entity.get_mut::<Player>() {
            player.kills = self.kills;
            player.deaths = self.deaths;
            player.perk = self.perk;
            player.gear = self.gear;
            player.gear_left = gear_count;
            player.loadout = self.loadout;
        }
        let current = self.arsenal.current;
        if let Some(mut arsenal) = entity.get_mut::<Arsenal>() {
            *arsenal = Arsenal { cooldown: 0.0, reload: 0.0, burst: 0, trigger: false, ..self.arsenal.clone() };
        }
        if let Some(mut input) = entity.get_mut::<PlayerInput>() {
            input.weapon = current;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_state_is_compact() {
        let state = EntState { id: 1, kind: 1, sub: 2, pos: [1, 2, 3], yaw: 4, pitch: -3, hp: 5, aux: 6, flags: 7 };
        assert_eq!(bincode::serialize(&state).unwrap().len(), 17);
    }

    #[test]
    fn remote_playback_interpolates_and_snaps_on_teleport() {
        let mut remote = Remote::at(0.0, Vec3::ZERO, 0.0);
        remote.push(0.1, Vec3::new(1.0, 0.0, 0.0), 6.2);
        let (pos, yaw) = remote.sample(0.05).unwrap();
        assert!((pos.x - 0.5).abs() < 1e-4);
        // The short way round from 0 to 6.2 rad is backwards through zero.
        assert!(yaw < 0.0 && yaw > -0.1);
        remote.push(0.2, Vec3::new(50.0, 0.0, 0.0), 0.0);
        assert_eq!(remote.sample(0.15).unwrap().0.x, 50.0);
        assert_eq!(remote.sample(9.0).unwrap().0.x, 50.0);
    }

    #[test]
    fn server_clock_prefers_fast_arrivals() {
        let mut clock = ServerClock::default();
        clock.observe(10.0, 1.0);
        assert_eq!(clock.offset, 9.0);
        // A late packet barely moves the estimate; an early one wins outright.
        clock.observe(10.5, 1.8);
        assert!(clock.offset < 9.0 && clock.offset > 8.98);
        clock.observe(11.0, 1.9);
        assert!((clock.offset - 9.1).abs() < 1e-4);
    }

    #[test]
    fn quantisation_round_trips() {
        let q = Quant { xz: 160.0 / 32_000.0 };
        let p = Vec3::new(-151.37, 3.21, 88.04);
        assert!(q.unpack(q.pack(p)).distance(p) < 0.02);
        for pitch in [-1.4, 0.0, 0.7] {
            assert!((unpack_pitch(pack_pitch(pitch)) - pitch).abs() < 0.02);
        }
        for yaw in [0.0, 1.0, 3.0, 6.0] {
            let back = unpack_yaw(pack_yaw(Quat::from_rotation_y(yaw)));
            assert!((back - yaw).abs() < TAU / 200.0, "{yaw} -> {back}");
        }
    }
}
