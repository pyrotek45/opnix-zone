//! `oz_net` - a small host/client message transport over UDP.
//!
//! The engine only needs "send typed messages to peers, receive typed messages
//! back", so that is the whole API: [`Host`] accepts connections and
//! [`Client`] joins one. Both are non-blocking and are polled once per frame.
//! Messages are any `serde` types, encoded with bincode.
//!
//! Underneath is one UDP socket per side with two lanes:
//!
//! * **Reliable, ordered** ([`Host::send`], [`Client::send`]): a byte stream
//!   cut into numbered segments, acknowledged with a cumulative ack plus a
//!   32-bit selective bitmap, resent on a round-trip-based timeout. Used for
//!   commands, effects and anything that must arrive.
//! * **Latest-only** ([`Host::send_latest`], ...): fragmented if large, never
//!   resent, and older messages are dropped if a newer one already arrived.
//!   Used for snapshots and pings, where only the newest state matters.
//!
//! UDP is what makes playing over the internet without a server or VPN
//! possible: both sides learn their public address from a STUN server
//! ([`Host::discover_public_address`]), swap addresses through a public
//! message board ([`rendezvous`]), and fire packets at each other
//! ([`Host::punch`]) so both routers let the other side in.

pub mod lan;
pub mod rendezvous;
mod stun;

use serde::{Serialize, de::DeserializeOwned};
use std::{
    collections::{BTreeMap, HashMap},
    io::{self, ErrorKind},
    marker::PhantomData,
    net::{SocketAddr, ToSocketAddrs, UdpSocket},
    time::{Duration, Instant},
};
use stun::Stun;

pub const DEFAULT_PORT: u16 = 27_650;
/// Frames larger than this are treated as a protocol violation.
const MAX_FRAME: usize = 4 * 1024 * 1024;
/// Payload bytes per reliable segment or latest-only fragment; keeps datagrams under common MTUs.
const SEGMENT: usize = 1_100;
const DATAGRAM: usize = 1_200;
/// Reliable segments in flight before the sender waits for acks.
const WINDOW: usize = 1_024;
/// Latest-only messages larger than this many fragments go reliable instead.
const MAX_FRAGMENTS: usize = 64;
const KEEPALIVE: Duration = Duration::from_millis(250);
const HELLO_EVERY: Duration = Duration::from_millis(100);

const MAGIC: [u8; 2] = *b"OZ";
const VERSION: u8 = 1;
mod kind {
    pub const HELLO: u8 = 1;
    pub const WELCOME: u8 = 2;
    pub const DATA: u8 = 3;
    pub const BYE: u8 = 4;
    pub const PUNCH: u8 = 5;
}
const RELIABLE: u8 = 0;
const LATEST: u8 = 1;

pub type PeerId = u32;

fn header(kind: u8, conn: u64) -> Vec<u8> {
    let mut packet = Vec::with_capacity(DATAGRAM);
    packet.extend_from_slice(&MAGIC);
    packet.push(VERSION);
    packet.push(kind);
    packet.extend_from_slice(&conn.to_le_bytes());
    packet
}

/// (kind, connection id, body) of one of our datagrams.
fn parse(packet: &[u8]) -> Option<(u8, u64, &[u8])> {
    if packet.len() < 12 || packet[..2] != MAGIC || packet[2] != VERSION {
        return None;
    }
    Some((packet[3], u64::from_le_bytes(packet[4..12].try_into().ok()?), &packet[12..]))
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Some(head)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }
    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4).map(|b| u32::from_le_bytes(b.try_into().unwrap()))
    }
}

/// A random 64-bit number from the clock and the address space; good enough for connection ids.
fn random_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
    hasher.finish().max(1)
}

struct Sent {
    data: Vec<u8>,
    first: Instant,
    last: Instant,
    resent: bool,
}

struct Partial {
    parts: Vec<Option<Vec<u8>>>,
    have: usize,
}

/// One end of a connection: both lanes, acks and timers.
struct Session {
    addr: SocketAddr,
    conn: u64,
    // Reliable sending: bytes waiting to be cut into segments, and segments not yet acknowledged.
    stream: Vec<u8>,
    next_seq: u32,
    unacked: BTreeMap<u32, Sent>,
    // Reliable receiving.
    expected: u32,
    early: BTreeMap<u32, Vec<u8>>,
    inbox: Vec<u8>,
    need_ack: bool,
    // Latest-only lane.
    next_msg: u32,
    outgoing: Vec<Vec<u8>>,
    partial: HashMap<u32, Partial>,
    newest: Option<u32>,
    latest_in: Vec<Vec<u8>>,
    // Timing.
    heard: Instant,
    last_sent: Instant,
    srtt: Option<f32>,
    closed: bool,
    /// Test hook: fraction of outgoing datagrams to drop.
    loss: f32,
    loss_state: u64,
}

impl Session {
    fn new(addr: SocketAddr, conn: u64) -> Self {
        let now = Instant::now();
        Self {
            addr,
            conn,
            stream: Vec::new(),
            next_seq: 0,
            unacked: BTreeMap::new(),
            expected: 0,
            early: BTreeMap::new(),
            inbox: Vec::new(),
            need_ack: false,
            next_msg: 0,
            outgoing: Vec::new(),
            partial: HashMap::new(),
            newest: None,
            latest_in: Vec::new(),
            heard: now,
            last_sent: now,
            srtt: None,
            closed: false,
            loss: 0.0,
            loss_state: conn,
        }
    }

    fn rto(&self) -> Duration {
        Duration::from_secs_f32(self.srtt.map_or(0.2, |rtt| (rtt * 2.0 + 0.03).clamp(0.06, 1.0)))
    }

    fn queue(&mut self, body: &[u8]) {
        self.stream.extend_from_slice(&(body.len() as u32).to_le_bytes());
        self.stream.extend_from_slice(body);
    }

    fn queue_latest(&mut self, body: &[u8]) {
        let count = body.len().div_ceil(SEGMENT).max(1);
        if count > MAX_FRAGMENTS {
            self.queue(body);
            return;
        }
        let msg = self.next_msg;
        self.next_msg = self.next_msg.wrapping_add(1);
        for (index, chunk) in body.chunks(SEGMENT).enumerate().chain(body.is_empty().then_some((0, &[][..]))) {
            let mut entry = Vec::with_capacity(chunk.len() + 11);
            entry.push(LATEST);
            entry.extend_from_slice(&msg.to_le_bytes());
            entry.extend_from_slice(&(index as u16).to_le_bytes());
            entry.extend_from_slice(&(count as u16).to_le_bytes());
            entry.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
            entry.extend_from_slice(chunk);
            self.outgoing.push(entry);
        }
    }

    fn ack_fields(&self) -> (u32, u32) {
        let mut bits = 0u32;
        for i in 0..32 {
            if self.early.contains_key(&self.expected.wrapping_add(1 + i)) {
                bits |= 1 << i;
            }
        }
        (self.expected, bits)
    }

    /// Handles one DATA body from the other side.
    fn receive(&mut self, body: &[u8]) {
        self.heard = Instant::now();
        let mut r = Reader(body);
        let (Some(ack), Some(bits)) = (r.u32(), r.u32()) else { return };
        let now = Instant::now();
        let acked = |seq: u32, unacked: &mut BTreeMap<u32, Sent>, srtt: &mut Option<f32>| {
            if let Some(sent) = unacked.remove(&seq)
                && !sent.resent {
                    let sample = now.duration_since(sent.first).as_secs_f32();
                    *srtt = Some(srtt.map_or(sample, |s| s * 0.875 + sample * 0.125));
                }
        };
        let done: Vec<u32> = self.unacked.range(..ack).map(|(seq, _)| *seq).collect();
        for seq in done {
            acked(seq, &mut self.unacked, &mut self.srtt);
        }
        for i in 0..32 {
            if bits & (1 << i) != 0 {
                acked(ack.wrapping_add(1 + i), &mut self.unacked, &mut self.srtt);
            }
        }
        while let Some(tag) = r.u8() {
            match tag {
                RELIABLE => {
                    let (Some(seq), Some(len)) = (r.u32(), r.u16()) else { return };
                    let Some(data) = r.take(len as usize) else { return };
                    self.need_ack = true;
                    if seq >= self.expected {
                        self.early.entry(seq).or_insert_with(|| data.to_vec());
                    }
                    while let Some(data) = self.early.remove(&self.expected) {
                        self.inbox.extend_from_slice(&data);
                        self.expected += 1;
                    }
                }
                LATEST => {
                    let (Some(msg), Some(index), Some(count), Some(len)) = (r.u32(), r.u16(), r.u16(), r.u16()) else { return };
                    let Some(data) = r.take(len as usize) else { return };
                    let (index, count) = (index as usize, count as usize);
                    if self.newest.is_some_and(|newest| msg.wrapping_sub(newest) as i32 <= 0) || index >= count {
                        continue;
                    }
                    let partial = self.partial.entry(msg).or_insert_with(|| Partial { parts: vec![None; count], have: 0 });
                    if partial.parts.len() != count || partial.parts[index].is_some() {
                        continue;
                    }
                    partial.parts[index] = Some(data.to_vec());
                    partial.have += 1;
                    if partial.have == count {
                        let parts = self.partial.remove(&msg).map(|p| p.parts).unwrap_or_default();
                        self.latest_in.push(parts.into_iter().flatten().flatten().collect());
                        self.newest = Some(msg);
                        self.partial.retain(|id, _| id.wrapping_sub(msg) as i32 > 0);
                    }
                }
                _ => return,
            }
        }
    }

    /// Decodes everything that has fully arrived, in order: reliable first, then latest-only.
    fn deliver<T: DeserializeOwned>(&mut self, mut out: impl FnMut(T)) {
        let mut cursor = 0;
        while self.inbox.len() - cursor >= 4 {
            let length = u32::from_le_bytes(self.inbox[cursor..cursor + 4].try_into().unwrap()) as usize;
            if length > MAX_FRAME {
                self.closed = true;
                break;
            }
            if self.inbox.len() - cursor - 4 < length {
                break;
            }
            match bincode::deserialize(&self.inbox[cursor + 4..cursor + 4 + length]) {
                Ok(message) => out(message),
                Err(_) => self.closed = true,
            }
            cursor += 4 + length;
        }
        self.inbox.drain(..cursor);
        for body in std::mem::take(&mut self.latest_in) {
            if let Ok(message) = bincode::deserialize(&body) {
                out(message);
            }
        }
    }

    fn send_raw(&mut self, socket: &UdpSocket, packet: &[u8]) {
        self.last_sent = Instant::now();
        if self.loss > 0.0 {
            // xorshift: deterministic pseudo-random loss for tests.
            self.loss_state ^= self.loss_state << 13;
            self.loss_state ^= self.loss_state >> 7;
            self.loss_state ^= self.loss_state << 17;
            if (self.loss_state % 1000) as f32 / 1000.0 < self.loss {
                return;
            }
        }
        let _ = socket.send_to(packet, self.addr);
    }

    /// Sends resends, new segments and latest-only fragments, packed into datagrams.
    fn flush(&mut self, socket: &UdpSocket) {
        let now = Instant::now();
        let rto = self.rto();
        let mut entries: Vec<Vec<u8>> = Vec::new();
        let segment = |seq: u32, data: &[u8]| {
            let mut entry = Vec::with_capacity(data.len() + 7);
            entry.push(RELIABLE);
            entry.extend_from_slice(&seq.to_le_bytes());
            entry.extend_from_slice(&(data.len() as u16).to_le_bytes());
            entry.extend_from_slice(data);
            entry
        };
        for (seq, sent) in self.unacked.iter_mut() {
            if now.duration_since(sent.last) >= rto {
                sent.last = now;
                sent.resent = true;
                entries.push(segment(*seq, &sent.data));
            }
        }
        while !self.stream.is_empty() && self.unacked.len() < WINDOW {
            let take = self.stream.len().min(SEGMENT);
            let data: Vec<u8> = self.stream.drain(..take).collect();
            let seq = self.next_seq;
            self.next_seq += 1;
            entries.push(segment(seq, &data));
            self.unacked.insert(seq, Sent { data, first: now, last: now, resent: false });
        }
        entries.append(&mut self.outgoing);

        let start = |session: &Session| {
            let mut packet = header(kind::DATA, session.conn);
            let (ack, bits) = session.ack_fields();
            packet.extend_from_slice(&ack.to_le_bytes());
            packet.extend_from_slice(&bits.to_le_bytes());
            packet
        };
        let mut packet = start(self);
        let empty = packet.len();
        for entry in entries {
            if packet.len() + entry.len() > DATAGRAM && packet.len() > empty {
                self.send_raw(socket, &packet);
                packet = start(self);
            }
            packet.extend_from_slice(&entry);
        }
        if packet.len() > empty || self.need_ack || now.duration_since(self.last_sent) > KEEPALIVE {
            self.send_raw(socket, &packet);
            self.need_ack = false;
        }
    }

    fn bye(&mut self, socket: &UdpSocket) {
        let packet = header(kind::BYE, self.conn);
        for _ in 0..3 {
            let _ = socket.send_to(&packet, self.addr);
        }
    }
}

fn bind_udp(addr: impl ToSocketAddrs) -> io::Result<UdpSocket> {
    let socket = UdpSocket::bind(addr)?;
    socket.set_nonblocking(true)?;
    Ok(socket)
}

/// Reads every waiting datagram.
fn drain(socket: &UdpSocket, mut handle: impl FnMut(&[u8], SocketAddr)) {
    let mut buffer = [0u8; 2048];
    loop {
        match socket.recv_from(&mut buffer) {
            Ok((n, from)) => handle(&buffer[..n], from),
            Err(e) if e.kind() == ErrorKind::WouldBlock => break,
            // ICMP "port unreachable" from an earlier send shows up here on some systems; skip it.
            Err(e) if matches!(e.kind(), ErrorKind::ConnectionReset | ErrorKind::ConnectionRefused | ErrorKind::Interrupted) => {}
            Err(_) => break,
        }
    }
}

pub enum HostEvent<In> {
    Connected(PeerId),
    Disconnected(PeerId),
    Message(PeerId, In),
}

/// The listening side. `In` is what clients send, `Out` is what the host sends.
pub struct Host<In, Out> {
    socket: UdpSocket,
    peers: Vec<(PeerId, Session)>,
    next_id: PeerId,
    /// Addresses to fire packets at (hole punching), until a deadline.
    punches: Vec<(SocketAddr, Instant)>,
    punch_timer: Instant,
    stun: Option<Stun>,
    loss: f32,
    _marker: PhantomData<fn(In, Out)>,
}

impl<In: DeserializeOwned, Out: Serialize> Host<In, Out> {
    pub fn bind(addr: impl ToSocketAddrs) -> io::Result<Self> {
        Ok(Self { socket: bind_udp(addr)?, peers: Vec::new(), next_id: 1, punches: Vec::new(), punch_timer: Instant::now(), stun: None, loss: 0.0, _marker: PhantomData })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Asks public STUN servers what this socket looks like from the internet.
    pub fn discover_public_address(&mut self) {
        self.stun.get_or_insert_with(Stun::start);
    }

    /// This host's address as the internet sees it, once STUN has answered.
    pub fn public_addr(&self) -> Option<SocketAddr> {
        self.stun.as_ref().and_then(|stun| stun.public)
    }

    /// Fires packets at `addr` for a few seconds so this side's router lets it in.
    pub fn punch(&mut self, addr: SocketAddr) {
        let until = Instant::now() + Duration::from_secs(6);
        match self.punches.iter_mut().find(|(a, _)| *a == addr) {
            Some(entry) => entry.1 = until,
            None => self.punches.push((addr, until)),
        }
    }

    /// Test hook: drop this fraction of every outgoing datagram.
    pub fn simulate_loss(&mut self, fraction: f32) {
        self.loss = fraction;
        for (_, session) in &mut self.peers {
            session.loss = fraction;
        }
    }

    /// Accepts connections, reads messages, sends what is queued. Call once per frame.
    pub fn poll(&mut self) -> Vec<HostEvent<In>> {
        let mut events = Vec::new();
        let (socket, peers, stun) = (&self.socket, &mut self.peers, &mut self.stun);
        let mut welcomes = Vec::new();
        drain(socket, |packet, from| {
            if let Some(stun) = stun.as_mut()
                && stun.handle(packet) {
                    return;
                }
            let Some((kind, conn, body)) = parse(packet) else { return };
            let existing = peers.iter().position(|(_, s)| s.addr == from);
            match kind {
                kind::HELLO => {
                    match existing {
                        Some(i) if peers[i].1.conn == conn => {}
                        Some(i) => {
                            // Same address, new game session: the old one is gone.
                            events.push(HostEvent::Disconnected(peers[i].0));
                            peers.remove(i);
                            welcomes.push((from, conn));
                        }
                        None => welcomes.push((from, conn)),
                    }
                    let _ = socket.send_to(&header(kind::WELCOME, conn), from);
                }
                kind::DATA => {
                    if let Some(i) = existing.filter(|i| peers[*i].1.conn == conn) {
                        peers[i].1.receive(body);
                    }
                }
                kind::BYE => {
                    if let Some(i) = existing.filter(|i| peers[*i].1.conn == conn) {
                        peers[i].1.closed = true;
                    }
                }
                _ => {}
            }
        });
        for (addr, conn) in welcomes {
            if !self.peers.iter().any(|(_, s)| s.addr == addr) {
                let mut session = Session::new(addr, conn);
                session.loss = self.loss;
                self.peers.push((self.next_id, session));
                events.push(HostEvent::Connected(self.next_id));
                self.next_id += 1;
            }
        }
        for (id, session) in &mut self.peers {
            session.deliver(|message| events.push(HostEvent::Message(*id, message)));
            if session.closed {
                events.push(HostEvent::Disconnected(*id));
            } else {
                session.flush(&self.socket);
            }
        }
        self.peers.retain(|(_, s)| !s.closed);

        let now = Instant::now();
        self.punches.retain(|(_, until)| *until > now);
        if now >= self.punch_timer && !self.punches.is_empty() {
            self.punch_timer = now + Duration::from_millis(150);
            for (addr, _) in &self.punches {
                let _ = self.socket.send_to(&header(kind::PUNCH, 0), addr);
            }
        }
        if let Some(stun) = &mut self.stun {
            stun.poll(&self.socket);
        }
        events
    }

    pub fn send(&mut self, peer: PeerId, message: &Out) {
        let Ok(body) = bincode::serialize(message) else { return };
        if let Some((_, session)) = self.peers.iter_mut().find(|(id, _)| *id == peer) {
            session.queue(&body);
        }
    }

    /// Sends a message where only the newest copy matters (it may be dropped, never resent).
    pub fn send_latest(&mut self, peer: PeerId, message: &Out) {
        let Ok(body) = bincode::serialize(message) else { return };
        if let Some((_, session)) = self.peers.iter_mut().find(|(id, _)| *id == peer) {
            session.queue_latest(&body);
        }
    }

    /// Serialises once and sends reliably to every peer.
    pub fn broadcast(&mut self, message: &Out) {
        let Ok(body) = bincode::serialize(message) else { return };
        for (_, session) in &mut self.peers {
            session.queue(&body);
        }
    }

    /// Serialises once and sends latest-only to every peer.
    pub fn broadcast_latest(&mut self, message: &Out) {
        let Ok(body) = bincode::serialize(message) else { return };
        for (_, session) in &mut self.peers {
            session.queue_latest(&body);
        }
    }

    /// Sends anything queued since the last poll right away.
    pub fn flush(&mut self) {
        for (_, session) in &mut self.peers {
            session.flush(&self.socket);
        }
    }

    /// How long since this peer last sent anything.
    pub fn silent_for(&self, peer: PeerId) -> Option<Duration> {
        self.peers.iter().find(|(id, _)| *id == peer).map(|(_, s)| s.heard.elapsed())
    }

    /// Smoothed round trip to a peer.
    pub fn round_trip(&self, peer: PeerId) -> Option<Duration> {
        self.peers.iter().find(|(id, _)| *id == peer).and_then(|(_, s)| s.srtt).map(Duration::from_secs_f32)
    }

    pub fn kick(&mut self, peer: PeerId) {
        if let Some((_, session)) = self.peers.iter_mut().find(|(id, _)| *id == peer) {
            session.flush(&self.socket);
            session.bye(&self.socket);
        }
        self.peers.retain(|(id, _)| *id != peer);
    }
}

impl<In, Out> Drop for Host<In, Out> {
    fn drop(&mut self) {
        for (_, session) in &mut self.peers {
            session.bye(&self.socket);
        }
    }
}

pub enum ClientEvent<In> {
    Message(In),
    Disconnected,
}

/// The joining side. `In` is what the host sends, `Out` is what this client sends.
///
/// A client can be created before it knows where the host is ([`Client::open`])
/// and given addresses later ([`Client::add_candidates`]); it says hello to all
/// of them until one answers.
pub struct Client<In, Out> {
    socket: UdpSocket,
    candidates: Vec<SocketAddr>,
    session: Session,
    established: bool,
    hello_timer: Instant,
    reported_close: bool,
    stun: Option<Stun>,
    _marker: PhantomData<fn(In, Out)>,
}

impl<In: DeserializeOwned, Out: Serialize> Client<In, Out> {
    /// Starts joining `addr` (any form `ToSocketAddrs` takes).
    pub fn connect(addr: impl ToSocketAddrs) -> io::Result<Self> {
        let targets: Vec<SocketAddr> = addr.to_socket_addrs()?.collect();
        if targets.is_empty() {
            return Err(io::Error::new(ErrorKind::InvalidInput, "address did not resolve"));
        }
        let mut client = Self::open()?;
        client.add_candidates(&targets);
        Ok(client)
    }

    /// A client with no host yet.
    pub fn open() -> io::Result<Self> {
        let socket = bind_udp("0.0.0.0:0")?;
        let placeholder = SocketAddr::from(([0, 0, 0, 0], 0));
        Ok(Self {
            socket,
            candidates: Vec::new(),
            session: Session::new(placeholder, random_u64()),
            established: false,
            hello_timer: Instant::now(),
            reported_close: false,
            stun: None,
            _marker: PhantomData,
        })
    }

    /// More places the host might be reachable (LAN, public, forwarded).
    pub fn add_candidates(&mut self, addrs: &[SocketAddr]) {
        for addr in addrs {
            // Loopback stands in for "this machine" when the host advertised 0.0.0.0.
            let addr = if addr.ip().is_unspecified() { SocketAddr::from(([127, 0, 0, 1], addr.port())) } else { *addr };
            if !self.candidates.contains(&addr) {
                self.candidates.push(addr);
                self.hello_timer = Instant::now();
            }
        }
    }

    pub fn discover_public_address(&mut self) {
        self.stun.get_or_insert_with(Stun::start);
    }

    pub fn public_addr(&self) -> Option<SocketAddr> {
        self.stun.as_ref().and_then(|stun| stun.public)
    }

    pub fn local_port(&self) -> u16 {
        self.socket.local_addr().map_or(0, |a| a.port())
    }

    pub fn simulate_loss(&mut self, fraction: f32) {
        self.session.loss = fraction;
    }

    pub fn poll(&mut self) -> Vec<ClientEvent<In>> {
        let mut events = Vec::new();
        let (socket, session, stun) = (&self.socket, &mut self.session, &mut self.stun);
        let established = &mut self.established;
        drain(socket, |packet, from| {
            if let Some(stun) = stun.as_mut()
                && stun.handle(packet) {
                    return;
                }
            let Some((kind, conn, body)) = parse(packet) else { return };
            match kind {
                // The connection id is a 64-bit secret, so a welcome carrying it is genuine even
                // from an address we did not list (routers can remap the host's port).
                kind::WELCOME if conn == session.conn && !*established => {
                    session.addr = from;
                    session.heard = Instant::now();
                    *established = true;
                }
                kind::DATA if *established && from == session.addr && conn == session.conn => session.receive(body),
                kind::BYE if *established && from == session.addr && conn == session.conn => session.closed = true,
                _ => {}
            }
        });
        if self.established {
            self.session.deliver(|message| events.push(ClientEvent::Message(message)));
            if !self.session.closed {
                self.session.flush(&self.socket);
            }
        } else if Instant::now() >= self.hello_timer {
            self.hello_timer = Instant::now() + HELLO_EVERY;
            let hello = header(kind::HELLO, self.session.conn);
            for addr in &self.candidates {
                let _ = self.socket.send_to(&hello, addr);
            }
        }
        if let Some(stun) = &mut self.stun {
            stun.poll(&self.socket);
        }
        if self.session.closed && !self.reported_close {
            self.reported_close = true;
            events.push(ClientEvent::Disconnected);
        }
        events
    }

    pub fn send(&mut self, message: &Out) {
        if let Ok(body) = bincode::serialize(message) {
            self.session.queue(&body);
        }
    }

    pub fn send_latest(&mut self, message: &Out) {
        if let Ok(body) = bincode::serialize(message) {
            self.session.queue_latest(&body);
        }
    }

    pub fn flush(&mut self) {
        if self.established {
            self.session.flush(&self.socket);
        }
    }

    /// How long since the host last sent anything.
    pub fn silent_for(&self) -> Duration {
        self.session.heard.elapsed()
    }

    pub fn is_connected(&self) -> bool {
        !self.session.closed
    }

    /// The host answered and the connection is up.
    pub fn is_established(&self) -> bool {
        self.established
    }

    /// Where the host answered from.
    pub fn host_addr(&self) -> Option<SocketAddr> {
        self.established.then_some(self.session.addr)
    }
}

impl<In, Out> Drop for Client<In, Out> {
    fn drop(&mut self) {
        if self.established {
            self.session.bye(&self.socket);
        }
    }
}

/// Accepts `host`, `host:port` or `ip:port`; fills in [`DEFAULT_PORT`] when missing.
pub fn normalize_address(input: &str) -> String {
    let input = input.trim();
    if input.is_empty() {
        format!("127.0.0.1:{DEFAULT_PORT}")
    } else if input.rsplit_once(':').is_some_and(|(_, port)| port.parse::<u16>().is_ok()) {
        input.to_string()
    } else {
        format!("{input}:{DEFAULT_PORT}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
    enum Up {
        Hello(String),
        Blob(Vec<u8>),
        Count(u32),
    }

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct Down(u32);

    fn pump<T>(mut step: impl FnMut() -> Option<T>) -> T {
        let start = Instant::now();
        loop {
            if let Some(value) = step() {
                return value;
            }
            assert!(start.elapsed() < Duration::from_secs(10), "timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn round_trip_including_large_frames() {
        let mut host: Host<Up, Down> = Host::bind("127.0.0.1:0").unwrap();
        let mut client: Client<Down, Up> = Client::connect(host.local_addr().unwrap()).unwrap();
        let big = vec![7u8; 300_000];
        client.send(&Up::Hello("ada".into()));
        client.send(&Up::Blob(big.clone()));

        let mut seen = Vec::new();
        let peer = pump(|| {
            client.poll();
            for event in host.poll() {
                match event {
                    HostEvent::Connected(_) => {}
                    HostEvent::Message(id, message) => seen.push((id, message)),
                    HostEvent::Disconnected(_) => panic!("unexpected disconnect"),
                }
            }
            (seen.len() == 2).then(|| seen[0].0)
        });
        assert_eq!(seen[0].1, Up::Hello("ada".into()));
        assert_eq!(seen[1].1, Up::Blob(big));

        host.send(peer, &Down(1));
        host.broadcast(&Down(2));
        let mut got = Vec::new();
        pump(|| {
            host.poll();
            for event in client.poll() {
                if let ClientEvent::Message(m) = event {
                    got.push(m);
                }
            }
            (got.len() == 2).then_some(())
        });
        assert_eq!(got, vec![Down(1), Down(2)]);

        drop(client);
        pump(|| host.poll().into_iter().any(|e| matches!(e, HostEvent::Disconnected(id) if id == peer)).then_some(()));
        assert_eq!(host.peer_count(), 0);
    }

    #[test]
    fn reliable_messages_survive_heavy_loss_in_order() {
        let mut host: Host<Up, Down> = Host::bind("127.0.0.1:0").unwrap();
        let mut client: Client<Down, Up> = Client::connect(host.local_addr().unwrap()).unwrap();
        host.simulate_loss(0.3);
        client.simulate_loss(0.3);
        for i in 0..500 {
            client.send(&Up::Count(i));
        }
        client.send(&Up::Blob(vec![3; 40_000]));
        let mut got = Vec::new();
        pump(|| {
            client.poll();
            for event in host.poll() {
                if let HostEvent::Message(_, m) = event {
                    got.push(m);
                }
            }
            (got.len() == 501).then_some(())
        });
        let expected: Vec<Up> = (0..500).map(Up::Count).chain([Up::Blob(vec![3; 40_000])]).collect();
        assert_eq!(got, expected);
    }

    #[test]
    fn latest_only_messages_arrive_newest_wins() {
        let mut host: Host<Up, Down> = Host::bind("127.0.0.1:0").unwrap();
        let mut client: Client<Down, Up> = Client::connect(host.local_addr().unwrap()).unwrap();
        let peer = pump(|| {
            client.poll();
            host.poll().into_iter().find_map(|e| if let HostEvent::Connected(id) = e { Some(id) } else { None })
        });
        // A big latest-only message is fragmented and reassembled.
        let big = Up::Blob(vec![9; 20_000]);
        client.send_latest(&big);
        let got = pump(|| {
            client.poll();
            host.poll().into_iter().find_map(|e| if let HostEvent::Message(_, m) = e { Some(m) } else { None })
        });
        assert_eq!(got, big);
        // Several in one frame: they arrive in order and none is older than the last.
        for i in 0..5 {
            host.send_latest(peer, &Down(i));
        }
        let mut last = None;
        pump(|| {
            host.poll();
            for event in client.poll() {
                if let ClientEvent::Message(Down(n)) = event {
                    assert!(last.is_none_or(|l| n > l));
                    last = Some(n);
                }
            }
            (last == Some(4)).then_some(())
        });
    }

    #[test]
    fn a_client_finds_the_host_among_candidates() {
        let mut host: Host<Up, Down> = Host::bind("127.0.0.1:0").unwrap();
        let port = host.local_addr().unwrap().port();
        let mut client: Client<Down, Up> = Client::open().unwrap();
        // Wrong guesses first, like an internet address that does not route back home.
        client.add_candidates(&["127.0.0.1:1".parse().unwrap(), SocketAddr::from(([127, 0, 0, 1], port))]);
        client.send(&Up::Hello("x".into()));
        pump(|| {
            client.poll();
            host.poll().into_iter().any(|e| matches!(e, HostEvent::Message(_, Up::Hello(_)))).then_some(())
        });
        assert!(client.is_established());
    }

    #[test]
    fn addresses_get_a_default_port() {
        assert_eq!(normalize_address(""), format!("127.0.0.1:{DEFAULT_PORT}"));
        assert_eq!(normalize_address("example.org"), format!("example.org:{DEFAULT_PORT}"));
        assert_eq!(normalize_address(" 10.0.0.2:9000 "), "10.0.0.2:9000");
    }
}
