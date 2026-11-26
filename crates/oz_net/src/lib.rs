//! `oz_net` - a small host/client message transport.
//!
//! The engine only needs "send typed messages to peers, receive typed messages
//! back", so that is the whole API: [`Host`] accepts connections and
//! [`Client`] joins one. Both are non-blocking and are polled once per frame.
//! Messages are any `serde` types, encoded with bincode in length-prefixed
//! frames over TCP (with `TCP_NODELAY`).
//!
//! Because gameplay never touches sockets directly, the wire layer can later
//! be swapped (UDP, relay, Steam sockets) without changing any game code.

use serde::{Serialize, de::DeserializeOwned};
use std::{
    io::{self, ErrorKind, Read, Write},
    marker::PhantomData,
    net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs},
    time::Duration,
};

pub const DEFAULT_PORT: u16 = 27_650;
/// Frames larger than this are treated as a protocol violation.
const MAX_FRAME: usize = 4 * 1024 * 1024;

pub type PeerId = u32;

/// One framed, non-blocking connection.
struct Link {
    stream: TcpStream,
    inbox: Vec<u8>,
    outbox: Vec<u8>,
    closed: bool,
}

impl Link {
    fn new(stream: TcpStream) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        stream.set_nodelay(true)?;
        Ok(Self { stream, inbox: Vec::new(), outbox: Vec::new(), closed: false })
    }

    fn queue<T: Serialize>(&mut self, message: &T) {
        let Ok(body) = bincode::serialize(message) else { return };
        self.queue_raw(&body);
    }

    fn queue_raw(&mut self, body: &[u8]) {
        self.outbox.extend_from_slice(&(body.len() as u32).to_le_bytes());
        self.outbox.extend_from_slice(body);
    }

    /// Writes as much pending data as the socket accepts.
    fn flush(&mut self) {
        while !self.outbox.is_empty() && !self.closed {
            match self.stream.write(&self.outbox) {
                Ok(0) => self.closed = true,
                Ok(n) => drop(self.outbox.drain(..n)),
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(_) => self.closed = true,
            }
        }
    }

    /// Reads everything available and decodes complete frames.
    fn receive<T: DeserializeOwned>(&mut self, mut deliver: impl FnMut(T)) {
        let mut buffer = [0u8; 16 * 1024];
        loop {
            match self.stream.read(&mut buffer) {
                Ok(0) => {
                    self.closed = true;
                    break;
                }
                Ok(n) => self.inbox.extend_from_slice(&buffer[..n]),
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(_) => {
                    self.closed = true;
                    break;
                }
            }
        }
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
                Ok(message) => deliver(message),
                Err(_) => self.closed = true,
            }
            cursor += 4 + length;
        }
        self.inbox.drain(..cursor);
    }
}

pub enum HostEvent<In> {
    Connected(PeerId),
    Disconnected(PeerId),
    Message(PeerId, In),
}

/// The listening side. `In` is what clients send, `Out` is what the host sends.
pub struct Host<In, Out> {
    listener: TcpListener,
    peers: Vec<(PeerId, Link)>,
    next_id: PeerId,
    _marker: PhantomData<fn(In, Out)>,
}

impl<In: DeserializeOwned, Out: Serialize> Host<In, Out> {
    pub fn bind(addr: impl ToSocketAddrs) -> io::Result<Self> {
        let listener = TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        Ok(Self { listener, peers: Vec::new(), next_id: 1, _marker: PhantomData })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Accepts connections, reads messages, flushes writes. Call once per frame.
    pub fn poll(&mut self) -> Vec<HostEvent<In>> {
        let mut events = Vec::new();
        while let Ok((stream, _)) = self.listener.accept() {
            if let Ok(link) = Link::new(stream) {
                self.peers.push((self.next_id, link));
                events.push(HostEvent::Connected(self.next_id));
                self.next_id += 1;
            }
        }
        for (id, link) in &mut self.peers {
            link.receive(|message| events.push(HostEvent::Message(*id, message)));
            link.flush();
            if link.closed {
                events.push(HostEvent::Disconnected(*id));
            }
        }
        self.peers.retain(|(_, link)| !link.closed);
        events
    }

    pub fn send(&mut self, peer: PeerId, message: &Out) {
        if let Some((_, link)) = self.peers.iter_mut().find(|(id, _)| *id == peer) {
            link.queue(message);
            link.flush();
        }
    }

    /// Serialises once and sends to every peer.
    pub fn broadcast(&mut self, message: &Out) {
        let Ok(body) = bincode::serialize(message) else { return };
        for (_, link) in &mut self.peers {
            link.queue_raw(&body);
            link.flush();
        }
    }

    pub fn kick(&mut self, peer: PeerId) {
        self.peers.retain(|(id, _)| *id != peer);
    }
}

pub enum ClientEvent<In> {
    Message(In),
    Disconnected,
}

/// The joining side. `In` is what the host sends, `Out` is what this client sends.
pub struct Client<In, Out> {
    link: Link,
    reported_close: bool,
    _marker: PhantomData<fn(In, Out)>,
}

impl<In: DeserializeOwned, Out: Serialize> Client<In, Out> {
    pub fn connect(addr: impl ToSocketAddrs) -> io::Result<Self> {
        let mut last_error = io::Error::new(ErrorKind::InvalidInput, "address did not resolve");
        for candidate in addr.to_socket_addrs()? {
            match TcpStream::connect_timeout(&candidate, Duration::from_secs(4)) {
                Ok(stream) => return Ok(Self { link: Link::new(stream)?, reported_close: false, _marker: PhantomData }),
                Err(e) => last_error = e,
            }
        }
        Err(last_error)
    }

    pub fn poll(&mut self) -> Vec<ClientEvent<In>> {
        let mut events = Vec::new();
        self.link.receive(|message| events.push(ClientEvent::Message(message)));
        self.link.flush();
        if self.link.closed && !self.reported_close {
            self.reported_close = true;
            events.push(ClientEvent::Disconnected);
        }
        events
    }

    pub fn send(&mut self, message: &Out) {
        self.link.queue(message);
        self.link.flush();
    }

    pub fn is_connected(&self) -> bool {
        !self.link.closed
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
    use std::time::Instant;

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    enum Up {
        Hello(String),
        Blob(Vec<u8>),
    }

    #[derive(Serialize, Deserialize, Debug, PartialEq)]
    struct Down(u32);

    fn pump<T>(mut step: impl FnMut() -> Option<T>) -> T {
        let start = Instant::now();
        loop {
            if let Some(value) = step() {
                return value;
            }
            assert!(start.elapsed() < Duration::from_secs(5), "timed out");
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
    fn addresses_get_a_default_port() {
        assert_eq!(normalize_address(""), format!("127.0.0.1:{DEFAULT_PORT}"));
        assert_eq!(normalize_address("example.org"), format!("example.org:{DEFAULT_PORT}"));
        assert_eq!(normalize_address(" 10.0.0.2:9000 "), "10.0.0.2:9000");
    }
}
