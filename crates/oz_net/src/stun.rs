//! Just enough STUN (RFC 5389) to ask "what is my address on the internet?".
//!
//! The request goes out of the game socket itself, so the answer is the exact
//! public address and port the router mapped for it - the one a friend's
//! packets must be aimed at.

use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs, UdpSocket},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const SERVERS: [&str; 4] = [
    "stun.l.google.com:19302",
    "stun.cloudflare.com:3478",
    "stun1.l.google.com:19302",
    "stun.nextcloud.com:3478",
];
const COOKIE: u32 = 0x2112_A442;
const ATTEMPTS: u32 = 8;

pub struct Stun {
    /// Filled in by a background lookup (DNS can block).
    servers: Arc<Mutex<Vec<SocketAddr>>>,
    transaction: [u8; 12],
    next: Instant,
    tries: u32,
    pub public: Option<SocketAddr>,
}

impl Stun {
    pub fn start() -> Self {
        let servers = Arc::new(Mutex::new(Vec::new()));
        let shared = servers.clone();
        std::thread::spawn(move || {
            let found: Vec<SocketAddr> = SERVERS
                .iter()
                .filter_map(|s| s.to_socket_addrs().ok())
                .flatten()
                .filter(|a| a.is_ipv4())
                .collect();
            if let Ok(mut slot) = shared.lock() {
                *slot = found;
            }
        });
        let id = super::random_u64().to_le_bytes();
        let more = super::random_u64().to_le_bytes();
        let mut transaction = [0u8; 12];
        transaction[..8].copy_from_slice(&id);
        transaction[8..].copy_from_slice(&more[..4]);
        Self {
            servers,
            transaction,
            next: Instant::now(),
            tries: 0,
            public: None,
        }
    }

    pub fn request(&self) -> [u8; 20] {
        let mut packet = [0u8; 20];
        packet[0..2].copy_from_slice(&0x0001u16.to_be_bytes()); // Binding request
        packet[4..8].copy_from_slice(&COOKIE.to_be_bytes());
        packet[8..20].copy_from_slice(&self.transaction);
        packet
    }

    /// Sends (and re-sends) requests until one is answered.
    pub fn poll(&mut self, socket: &UdpSocket) {
        if self.public.is_some() || self.tries >= ATTEMPTS || Instant::now() < self.next {
            return;
        }
        let servers = self.servers.lock().map(|s| s.clone()).unwrap_or_default();
        if servers.is_empty() {
            return;
        }
        self.tries += 1;
        self.next = Instant::now() + Duration::from_millis(500 * self.tries as u64);
        let request = self.request();
        for server in servers {
            let _ = socket.send_to(&request, server);
        }
    }

    /// Takes a datagram if it is our STUN answer.
    pub fn handle(&mut self, packet: &[u8]) -> bool {
        match parse_response(packet, &self.transaction) {
            Some(found) => {
                self.public.get_or_insert(found);
                true
            }
            None => packet.len() >= 20 && packet[4..8] == COOKIE.to_be_bytes(),
        }
    }
}

/// The mapped address in a binding success response for `transaction`.
pub fn parse_response(packet: &[u8], transaction: &[u8; 12]) -> Option<SocketAddr> {
    if packet.len() < 20
        || packet[0..2] != [0x01, 0x01]
        || packet[4..8] != COOKIE.to_be_bytes()
        || packet[8..20] != transaction[..]
    {
        return None;
    }
    let length = u16::from_be_bytes([packet[2], packet[3]]) as usize;
    let body = packet.get(20..20 + length)?;
    let mut at = 0;
    let mut plain = None;
    while at + 4 <= body.len() {
        let kind = u16::from_be_bytes([body[at], body[at + 1]]);
        let len = u16::from_be_bytes([body[at + 2], body[at + 3]]) as usize;
        let value = body.get(at + 4..at + 4 + len)?;
        match kind {
            0x0020 => return decode(value, true, transaction),
            0x0001 => plain = decode(value, false, transaction),
            _ => {}
        }
        at += 4 + len.div_ceil(4) * 4;
    }
    plain
}

fn decode(value: &[u8], xor: bool, transaction: &[u8; 12]) -> Option<SocketAddr> {
    if value.len() < 8 {
        return None;
    }
    let cookie = COOKIE.to_be_bytes();
    let mut port = u16::from_be_bytes([value[2], value[3]]);
    if xor {
        port ^= (COOKIE >> 16) as u16;
    }
    let ip = match value[1] {
        0x01 => {
            let mut b = [value[4], value[5], value[6], value[7]];
            if xor {
                for (i, byte) in b.iter_mut().enumerate() {
                    *byte ^= cookie[i];
                }
            }
            IpAddr::V4(Ipv4Addr::from(b))
        }
        0x02 if value.len() >= 20 => {
            let mut b: [u8; 16] = value[4..20].try_into().ok()?;
            if xor {
                let mask: Vec<u8> = cookie.iter().chain(transaction.iter()).copied().collect();
                for (i, byte) in b.iter_mut().enumerate() {
                    *byte ^= mask[i];
                }
            }
            IpAddr::V6(Ipv6Addr::from(b))
        }
        _ => return None,
    };
    Some(SocketAddr::new(ip, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_xor_mapped_address() {
        let transaction = [7u8; 12];
        let (ip, port) = (Ipv4Addr::new(203, 0, 113, 9), 40_123u16);
        let cookie = COOKIE.to_be_bytes();
        let mut attr = vec![0x00, 0x20, 0x00, 0x08, 0x00, 0x01];
        attr.extend_from_slice(&(port ^ (COOKIE >> 16) as u16).to_be_bytes());
        attr.extend(ip.octets().iter().zip(cookie).map(|(a, c)| a ^ c));
        let mut packet = vec![0x01, 0x01];
        packet.extend_from_slice(&(attr.len() as u16).to_be_bytes());
        packet.extend_from_slice(&cookie);
        packet.extend_from_slice(&transaction);
        packet.extend_from_slice(&attr);
        assert_eq!(
            parse_response(&packet, &transaction),
            Some(SocketAddr::new(IpAddr::V4(ip), port))
        );
        assert_eq!(
            parse_response(&packet, &[0u8; 12]),
            None,
            "someone else's transaction"
        );
    }
}
