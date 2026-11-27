//! Finding games and getting reachable without any cloud service.
//!
//! * [`Beacon`] answers "is anyone hosting?" broadcasts on the local network.
//! * [`Finder`] sends that broadcast and collects the answers.
//! * [`local_ip`] is the address friends on the same network type in.
//! * [`open_port`] asks the router (UPnP) to forward the game port, in the
//!   background, and reports the internet address friends elsewhere use.

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub const DISCOVERY_PORT: u16 = 27_651;
const ASK: &[u8] = b"OPNIX?1";
const ANSWER: &str = "OPNIX!1";

/// A host advertised on the local network.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub address: SocketAddr,
    pub name: String,
    pub mode: String,
    pub players: u8,
}

/// Host side: replies to discovery broadcasts. Poll once per frame.
pub struct Beacon {
    socket: UdpSocket,
}

impl Beacon {
    /// Fails quietly (returns `None`) if another game on this machine already owns the port.
    pub fn bind() -> Option<Self> {
        let socket = UdpSocket::bind(("0.0.0.0", DISCOVERY_PORT)).ok()?;
        socket.set_nonblocking(true).ok()?;
        Some(Self { socket })
    }

    /// Answers any pending queries with this game's details.
    pub fn poll(&self, game_port: u16, name: &str, mode: &str, players: u8) {
        let mut buffer = [0u8; 64];
        while let Ok((n, from)) = self.socket.recv_from(&mut buffer) {
            if &buffer[..n] == ASK {
                let clean = |s: &str| s.replace('|', " ").chars().take(32).collect::<String>();
                let reply = format!(
                    "{ANSWER}|{game_port}|{}|{}|{players}",
                    clean(name),
                    clean(mode)
                );
                let _ = self.socket.send_to(reply.as_bytes(), from);
            }
        }
    }
}

/// Client side: broadcasts a query and gathers replies for a moment.
pub struct Finder {
    socket: UdpSocket,
    started: Instant,
    pub found: Vec<Found>,
}

impl Finder {
    pub fn start() -> std::io::Result<Self> {
        let socket = UdpSocket::bind(("0.0.0.0", 0))?;
        socket.set_broadcast(true)?;
        socket.set_nonblocking(true)?;
        for target in [Ipv4Addr::BROADCAST, Ipv4Addr::LOCALHOST] {
            let _ = socket.send_to(ASK, (target, DISCOVERY_PORT));
        }
        Ok(Self {
            socket,
            started: Instant::now(),
            found: Vec::new(),
        })
    }

    /// Reads replies. Returns false once the search window has closed.
    pub fn poll(&mut self) -> bool {
        let mut buffer = [0u8; 256];
        while let Ok((n, from)) = self.socket.recv_from(&mut buffer) {
            let text = String::from_utf8_lossy(&buffer[..n]).to_string();
            let fields: Vec<&str> = text.split('|').collect();
            if fields.len() == 5 && fields[0] == ANSWER {
                let Ok(port) = fields[1].parse::<u16>() else {
                    continue;
                };
                let found = Found {
                    address: SocketAddr::new(from.ip(), port),
                    name: fields[2].into(),
                    mode: fields[3].into(),
                    players: fields[4].parse().unwrap_or(0),
                };
                // The same host can answer on loopback and on its LAN address; keep one.
                if !self
                    .found
                    .iter()
                    .any(|f| f.name == found.name && f.address.port() == port)
                {
                    self.found.push(found);
                }
            }
        }
        self.started.elapsed() < Duration::from_millis(1500)
    }
}

/// This machine's address on the local network (what LAN friends type in).
pub fn local_ip() -> Option<IpAddr> {
    // Connecting a UDP socket sends nothing; it only asks the OS which interface it would use.
    let socket = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    socket.connect(("192.0.2.1", 9)).ok()?;
    socket
        .local_addr()
        .ok()
        .map(|a| a.ip())
        .filter(|ip| !ip.is_unspecified())
}

/// Progress of an automatic router port-forward.
#[derive(Clone, Debug, PartialEq)]
pub enum PortStatus {
    Trying,
    /// The router forwarded the port; this is the internet address to share.
    Open(IpAddr),
    /// No UPnP router answered or it refused. Manual forwarding or a VPN is needed for internet play.
    Unavailable(String),
}

/// Asks the router to forward `port` (TCP) to this machine, in a background thread.
pub fn open_port(port: u16) -> Arc<Mutex<PortStatus>> {
    let status = Arc::new(Mutex::new(PortStatus::Trying));
    let shared = status.clone();
    std::thread::spawn(move || {
        let result = forward(port);
        if let Ok(mut slot) = shared.lock() {
            *slot = result;
        }
    });
    status
}

#[cfg(feature = "upnp")]
fn forward(port: u16) -> PortStatus {
    use igd_next::{PortMappingProtocol, SearchOptions, search_gateway};
    let options = SearchOptions {
        timeout: Some(Duration::from_secs(3)),
        ..Default::default()
    };
    let gateway = match search_gateway(options) {
        Ok(gateway) => gateway,
        Err(_) => return PortStatus::Unavailable("NO UPNP ROUTER FOUND".into()),
    };
    let Some(IpAddr::V4(local)) = local_ip() else {
        return PortStatus::Unavailable("NO LOCAL ADDRESS".into());
    };
    let local = SocketAddr::new(IpAddr::V4(local), port);
    if gateway
        .add_port(
            PortMappingProtocol::UDP,
            port,
            local,
            4 * 3600,
            "Opnix Zone",
        )
        .is_err()
    {
        return PortStatus::Unavailable("ROUTER REFUSED THE PORT".into());
    }
    match gateway.get_external_ip() {
        Ok(ip) => PortStatus::Open(ip),
        Err(_) => PortStatus::Unavailable("ROUTER HAS NO INTERNET ADDRESS".into()),
    }
}

#[cfg(not(feature = "upnp"))]
fn forward(_: u16) -> PortStatus {
    PortStatus::Unavailable("BUILT WITHOUT UPNP".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_are_found_on_the_local_machine() {
        let Some(beacon) = Beacon::bind() else { return }; // Port busy on this machine: nothing to test.
        let mut finder = Finder::start().unwrap();
        let start = Instant::now();
        while finder.found.is_empty() && start.elapsed() < Duration::from_secs(2) {
            beacon.poll(27_650, "ADA'S GAME", "TEAM DEATHMATCH", 3);
            finder.poll();
            std::thread::sleep(Duration::from_millis(5));
        }
        let game = finder.found.first().expect("the beacon should answer");
        assert_eq!(
            (
                game.name.as_str(),
                game.mode.as_str(),
                game.players,
                game.address.port()
            ),
            ("ADA'S GAME", "TEAM DEATHMATCH", 3, 27_650)
        );
    }
}
