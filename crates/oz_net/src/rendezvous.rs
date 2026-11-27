//! Join codes: finding each other on the internet without a server of our own.
//!
//! A host gets a short code like `K7Q4MZ`. Host and joiner both post where
//! they can be reached (LAN address, the public address STUN reported, a
//! UPnP-forwarded port) to a topic named after the code on a public
//! publish/subscribe board - by default the free <https://ntfy.sh>, or any
//! ntfy-compatible server set in the options. Each reads the other's post
//! and starts sending packets to every address it lists; the host "punches"
//! toward the joiner so its router lets the joiner's packets in. After that
//! the board is not needed: the game traffic goes directly between players.
//!
//! The board work happens on a background thread so a slow network never
//! stalls a frame. Posts only carry addresses, never game data.

use std::{
    collections::HashSet,
    io,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const DEFAULT_BOARD: &str = "https://ntfy.sh";
/// Unambiguous characters (no 0/O, 1/I/L).
const ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";
const CODE_LEN: usize = 6;

/// A public message board: post a line to a topic, read recent lines back.
pub trait Board: Send + Sync + 'static {
    fn post(&self, topic: &str, message: &str) -> io::Result<()>;
    /// Messages posted to `topic` in the last `minutes`, oldest first.
    fn read(&self, topic: &str, minutes: u32) -> io::Result<Vec<String>>;
}

/// An ntfy server (<https://ntfy.sh> or self-hosted).
pub struct Ntfy {
    pub base: String,
}

impl Board for Ntfy {
    fn post(&self, topic: &str, message: &str) -> io::Result<()> {
        ureq::post(&format!("{}/{topic}", self.base.trim_end_matches('/')))
            .timeout(Duration::from_secs(6))
            .send_string(message)
            .map(|_| ())
            .map_err(io::Error::other)
    }

    fn read(&self, topic: &str, minutes: u32) -> io::Result<Vec<String>> {
        let url = format!(
            "{}/{topic}/raw?poll=1&since={minutes}m",
            self.base.trim_end_matches('/')
        );
        let body = ureq::get(&url)
            .timeout(Duration::from_secs(6))
            .call()
            .map_err(io::Error::other)?
            .into_string()?;
        Ok(body
            .lines()
            .map(str::to_string)
            .filter(|l| !l.is_empty())
            .collect())
    }
}

/// An in-process board, for tests and same-machine play.
#[derive(Clone, Default)]
pub struct MemoryBoard(Arc<Mutex<Vec<(String, String)>>>);

impl Board for MemoryBoard {
    fn post(&self, topic: &str, message: &str) -> io::Result<()> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("poisoned"))?
            .push((topic.into(), message.into()));
        Ok(())
    }

    fn read(&self, topic: &str, _: u32) -> io::Result<Vec<String>> {
        Ok(self
            .0
            .lock()
            .map_err(|_| io::Error::other("poisoned"))?
            .iter()
            .filter(|(t, _)| t == topic)
            .map(|(_, m)| m.clone())
            .collect())
    }
}

/// A fresh random join code.
pub fn new_code() -> String {
    let mut x = super::random_u64();
    (0..CODE_LEN)
        .map(|_| {
            let c = ALPHABET[(x % ALPHABET.len() as u64) as usize] as char;
            x /= ALPHABET.len() as u64;
            c
        })
        .collect()
}

/// Uppercases and strips spaces and dashes; `None` if it cannot be a code.
pub fn normalize_code(text: &str) -> Option<String> {
    let code: String = text
        .chars()
        .filter(|c| !matches!(c, ' ' | '-'))
        .map(|c| c.to_ascii_uppercase())
        .collect();
    (code.len() == CODE_LEN && code.bytes().all(|b| ALPHABET.contains(&b))).then_some(code)
}

/// Shown to players as `K7Q-4MZ`.
pub fn pretty_code(code: &str) -> String {
    if code.len() == CODE_LEN {
        format!("{}-{}", &code[..3], &code[3..])
    } else {
        code.to_string()
    }
}

fn topic(code: &str) -> String {
    format!("opnix-zone-v1-{}", code.to_ascii_lowercase())
}

fn list(addrs: &[SocketAddr]) -> String {
    addrs
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn parse_list(text: &str) -> Vec<SocketAddr> {
    text.split(',')
        .filter_map(|a| a.trim().parse().ok())
        .collect()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// What the game thread and the board thread share.
#[derive(Default)]
struct Shared {
    /// Where we can be reached (set by the game).
    ours: Vec<SocketAddr>,
    /// Where the other side says it can be reached (set by the board thread).
    theirs: Vec<SocketAddr>,
    /// Joiners announced since the last take (host side).
    joiners: Vec<Vec<SocketAddr>>,
    /// Last problem talking to the board, for the UI.
    trouble: Option<String>,
    /// The host's post was found (joiner side).
    found: bool,
    stop: bool,
}

/// One side of a rendezvous; dropping it stops the board thread.
pub struct Rendezvous {
    pub code: String,
    shared: Arc<Mutex<Shared>>,
}

impl Rendezvous {
    /// Host side: publishes our addresses under `code` and collects joiners to punch toward.
    pub fn host(board: Arc<dyn Board>, code: String) -> Self {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (thread_shared, topic) = (shared.clone(), topic(&code));
        std::thread::spawn(move || {
            let mut posted = (String::new(), 0u64);
            let mut seen: HashSet<String> = HashSet::new();
            loop {
                let ours = match thread_shared.lock() {
                    Ok(s) if s.stop => return,
                    Ok(s) => s.ours.clone(),
                    Err(_) => return,
                };
                // Re-post when our addresses change, and now and then so the post stays fresh.
                let message = format!("OZH {}", list(&ours));
                if !ours.is_empty() && (message != posted.0 || now_secs() > posted.1 + 600) {
                    match board.post(&topic, &message) {
                        Ok(()) => posted = (message, now_secs()),
                        Err(e) => report(&thread_shared, format!("{e}")),
                    }
                }
                match board.read(&topic, 30) {
                    Ok(lines) => {
                        let mut fresh = Vec::new();
                        for line in lines {
                            if let Some(rest) = line.strip_prefix("OZC ")
                                && seen.insert(rest.to_string())
                                && let Some((_, addrs)) = rest.split_once(' ')
                            {
                                fresh.push(parse_list(addrs));
                            }
                        }
                        if let Ok(mut s) = thread_shared.lock() {
                            s.joiners.extend(fresh);
                            s.trouble = None;
                        }
                    }
                    Err(e) => report(&thread_shared, format!("{e}")),
                }
                std::thread::sleep(Duration::from_millis(1500));
            }
        });
        Self { code, shared }
    }

    /// Joiner side: posts our addresses and watches for the host's.
    pub fn join(board: Arc<dyn Board>, code: String) -> Self {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let (thread_shared, topic) = (shared.clone(), topic(&code));
        let nonce = super::random_u64();
        std::thread::spawn(move || {
            let mut posted = String::new();
            let mut last_post = 0u64;
            loop {
                let ours = match thread_shared.lock() {
                    Ok(s) if s.stop => return,
                    Ok(s) => s.ours.clone(),
                    Err(_) => return,
                };
                let message = format!("OZC {nonce:x} {}", list(&ours));
                // Post again when our addresses change, or every few seconds so a host that
                // started listening late still hears us.
                if !ours.is_empty() && (message != posted || now_secs() > last_post + 5) {
                    match board.post(&topic, &message) {
                        Ok(()) => {
                            posted = message;
                            last_post = now_secs();
                        }
                        Err(e) => report(&thread_shared, format!("{e}")),
                    }
                }
                match board.read(&topic, 12 * 60) {
                    Ok(lines) => {
                        let host = lines
                            .iter()
                            .rev()
                            .find_map(|l| l.strip_prefix("OZH "))
                            .map(parse_list);
                        if let Ok(mut s) = thread_shared.lock() {
                            if let Some(host) = host {
                                s.theirs = host;
                                s.found = true;
                            }
                            s.trouble = None;
                        }
                    }
                    Err(e) => report(&thread_shared, format!("{e}")),
                }
                std::thread::sleep(Duration::from_millis(1000));
            }
        });
        Self { code, shared }
    }

    /// Tells the board thread where we can be reached.
    pub fn set_ours(&self, addrs: Vec<SocketAddr>) {
        if let Ok(mut s) = self.shared.lock()
            && s.ours != addrs
        {
            s.ours = addrs;
        }
    }

    /// Joiner: the host's addresses, once its post is found.
    pub fn host_addrs(&self) -> Vec<SocketAddr> {
        self.shared
            .lock()
            .map(|s| s.theirs.clone())
            .unwrap_or_default()
    }

    pub fn found_host(&self) -> bool {
        self.shared.lock().is_ok_and(|s| s.found)
    }

    /// Host: joiners who announced themselves since the last call.
    pub fn take_joiners(&self) -> Vec<Vec<SocketAddr>> {
        self.shared
            .lock()
            .map(|mut s| std::mem::take(&mut s.joiners))
            .unwrap_or_default()
    }

    /// The last error from the board, if the latest attempt failed.
    pub fn trouble(&self) -> Option<String> {
        self.shared.lock().ok().and_then(|s| s.trouble.clone())
    }
}

impl Drop for Rendezvous {
    fn drop(&mut self) {
        if let Ok(mut s) = self.shared.lock() {
            s.stop = true;
        }
    }
}

fn report(shared: &Arc<Mutex<Shared>>, error: String) {
    if let Ok(mut s) = shared.lock() {
        s.trouble = Some(error);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Client, Host, HostEvent};
    use std::time::Instant;

    #[test]
    fn codes_are_short_unambiguous_and_forgiving() {
        let code = new_code();
        assert_eq!(code.len(), CODE_LEN);
        assert_eq!(
            normalize_code(&pretty_code(&code).to_lowercase()),
            Some(code.clone())
        );
        assert_eq!(normalize_code("k7q 4mz"), Some("K7Q4MZ".into()));
        assert_eq!(normalize_code("192.168.1.4"), None);
        assert_eq!(normalize_code("IOU0L1"), None);
    }

    #[test]
    fn a_join_code_connects_two_players() {
        let board: Arc<dyn Board> = Arc::new(MemoryBoard::default());
        let mut host: Host<u32, u32> = Host::bind("127.0.0.1:0").unwrap();
        let host_side = Rendezvous::host(board.clone(), new_code());
        // The host advertises a dead address first, then the real one.
        host_side.set_ours(vec![
            "127.0.0.1:9".parse().unwrap(),
            host.local_addr().unwrap(),
        ]);

        let mut client: Client<u32, u32> = Client::open().unwrap();
        let joiner = Rendezvous::join(board, host_side.code.clone());
        joiner.set_ours(vec![SocketAddr::from((
            [127, 0, 0, 1],
            client.local_port(),
        ))]);
        client.send(&7);

        let start = Instant::now();
        let (mut punched, mut delivered) = (0, false);
        loop {
            client.add_candidates(&joiner.host_addrs());
            for addrs in host_side.take_joiners() {
                for addr in addrs {
                    host.punch(addr);
                    punched += 1;
                }
            }
            client.poll();
            delivered |= host
                .poll()
                .into_iter()
                .any(|e| matches!(e, HostEvent::Message(_, 7)));
            if delivered && punched > 0 {
                break;
            }
            assert!(start.elapsed() < Duration::from_secs(10), "never met");
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(joiner.found_host());
    }
}
