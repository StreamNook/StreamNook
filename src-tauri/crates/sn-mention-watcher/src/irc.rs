//! Anonymous Twitch chat connections: as many as the live set needs, at most
//! 200 channels each, every JOIN paced by the one process-wide bucket.
//!
//! Each connection owns the list of channels it should be in. When it drops
//! (or Twitch asks it to reconnect), it opens a new socket, waits out a
//! backoff, and joins its whole list again through the bucket.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::backoff::Backoff;
use crate::bucket;
use crate::health::Health;
use crate::parse::{parse, Line, Privmsg};

pub const URL: &str = "wss://irc-ws.chat.twitch.tv:443";
pub const PER_CONNECTION: usize = 200;
/// A socket with no traffic for this long is presumed dead.
const SILENCE: Duration = Duration::from_secs(300);
/// Ask for a PONG this often when nothing else arrives.
const KEEPALIVE: Duration = Duration::from_secs(60);

#[derive(Debug)]
enum Cmd {
    Join(String),
    Part(String),
}

struct Conn {
    tx: mpsc::UnboundedSender<Cmd>,
    channels: HashSet<String>,
}

/// Which connection holds which channel, and the connections themselves.
pub struct Pool {
    conns: Vec<Conn>,
    assigned: HashMap<String, usize>,
    bucket: bucket::Shared,
    lines: mpsc::Sender<Privmsg>,
    health: Arc<Health>,
}

/// Where a new channel goes: the emptiest connection with room, if any.
fn pick(loads: &[usize]) -> Option<usize> {
    loads
        .iter()
        .enumerate()
        .filter(|(_, n)| **n < PER_CONNECTION)
        .min_by_key(|(_, n)| **n)
        .map(|(i, _)| i)
}

impl Pool {
    pub fn new(lines: mpsc::Sender<Privmsg>, health: Arc<Health>) -> Self {
        Pool {
            conns: Vec::new(),
            assigned: HashMap::new(),
            bucket: bucket::Shared::new(),
            lines,
            health,
        }
    }

    /// Be in exactly these channels (lowercase logins): part the rest, join
    /// the new ones.
    pub fn apply(&mut self, desired: &HashSet<String>) {
        let gone: Vec<String> = self.assigned.keys().filter(|c| !desired.contains(*c)).cloned().collect();
        for channel in gone {
            if let Some(i) = self.assigned.remove(&channel) {
                self.conns[i].channels.remove(&channel);
                let _ = self.conns[i].tx.send(Cmd::Part(channel));
            }
        }
        let mut new: Vec<&String> = desired.iter().filter(|c| !self.assigned.contains_key(*c)).collect();
        new.sort();
        let (joining, kept) = (new.len(), self.assigned.len());
        for channel in new {
            let loads: Vec<usize> = self.conns.iter().map(|c| c.channels.len()).collect();
            let i = pick(&loads).unwrap_or_else(|| self.open());
            self.conns[i].channels.insert(channel.clone());
            self.assigned.insert(channel.clone(), i);
            let _ = self.conns[i].tx.send(Cmd::Join(channel.clone()));
        }
        if joining > 0 {
            tracing::info!("irc: {} channels to join, {} kept, {} connections", joining, kept, self.conns.len());
        }
    }

    fn open(&mut self) -> usize {
        let (tx, rx) = mpsc::unbounded_channel();
        let id = self.conns.len();
        tokio::spawn(run_conn(id, rx, self.bucket.clone(), self.lines.clone(), self.health.clone()));
        self.conns.push(Conn { tx, channels: HashSet::new() });
        id
    }
}

fn nick() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("justinfan{}", 10_000 + nanos % 89_999)
}

/// Twitch refused or throttled a join: slow every connection down.
fn is_join_pushback(msg_id: &str) -> bool {
    matches!(msg_id, "msg_ratelimit" | "msg_channel_blocked" | "tos_ban") || msg_id.contains("rate")
}

async fn run_conn(
    id: usize,
    mut cmds: mpsc::UnboundedReceiver<Cmd>,
    bucket: bucket::Shared,
    lines: mpsc::Sender<Privmsg>,
    health: Arc<Health>,
) {
    // What this connection should be in, whatever the socket's state.
    let mut want: HashSet<String> = HashSet::new();
    let mut backoff = Backoff::default();
    loop {
        // Take commands that came in while the socket was down.
        while let Ok(cmd) = cmds.try_recv() {
            match cmd {
                Cmd::Join(c) => {
                    want.insert(c);
                }
                Cmd::Part(c) => {
                    want.remove(&c);
                }
            }
        }
        let socket = match tokio_tungstenite::connect_async(URL).await {
            Ok((socket, _)) => socket,
            Err(e) => {
                let wait = backoff.next();
                tracing::warn!("irc {id}: could not connect ({e}); again in {}s", wait.as_secs());
                tokio::time::sleep(wait).await;
                continue;
            }
        };
        let (mut write, mut read) = socket.split();
        let hello = ["PASS SCHMOOPIIE".to_string(), format!("NICK {}", nick()), "CAP REQ :twitch.tv/tags twitch.tv/commands".to_string()];
        let mut greeted = true;
        for line in hello {
            if write.send(Message::Text(line)).await.is_err() {
                greeted = false;
                break;
            }
        }
        if !greeted {
            tokio::time::sleep(backoff.next()).await;
            continue;
        }
        health.connected.fetch_add(1, Ordering::Relaxed);
        tracing::info!("irc {id}: connected, joining {} channels", want.len());
        let mut pending: VecDeque<String> = want.iter().cloned().collect();
        let mut joined: HashSet<String> = HashSet::new();
        let mut last_rx = Instant::now();
        let mut keepalive = tokio::time::interval(KEEPALIVE);
        keepalive.tick().await;
        let mut quick = false;
        let mut closed = false;
        loop {
            tokio::select! {
                cmd = cmds.recv() => match cmd {
                    Some(Cmd::Join(c)) => {
                        if want.insert(c.clone()) {
                            pending.push_back(c);
                        }
                    }
                    Some(Cmd::Part(c)) => {
                        want.remove(&c);
                        if joined.remove(&c) {
                            health.channels.fetch_sub(1, Ordering::Relaxed);
                            if write.send(Message::Text(format!("PART #{c}"))).await.is_err() {
                                break;
                            }
                        }
                    }
                    None => {
                        closed = true;
                        break;
                    }
                },
                _ = bucket.acquire(), if !pending.is_empty() => {
                    if let Some(c) = pending.pop_front() {
                        if want.contains(&c) && !joined.contains(&c) {
                            if write.send(Message::Text(format!("JOIN #{c}"))).await.is_err() {
                                pending.push_front(c);
                                break;
                            }
                            joined.insert(c);
                            health.channels.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                },
                frame = read.next() => {
                    let text = match frame {
                        Some(Ok(Message::Text(text))) => text,
                        Some(Ok(Message::Ping(data))) => {
                            last_rx = Instant::now();
                            let _ = write.send(Message::Pong(data)).await;
                            continue;
                        }
                        Some(Ok(Message::Close(_))) | None => break,
                        Some(Ok(_)) => continue,
                        Some(Err(e)) => {
                            tracing::warn!("irc {id}: read failed: {e}");
                            break;
                        }
                    };
                    last_rx = Instant::now();
                    backoff.reset();
                    let mut reconnect = false;
                    for raw in text.split("\r\n").filter(|l| !l.is_empty()) {
                        match parse(raw) {
                            Line::Privmsg(m) => {
                                if lines.try_send(*m).is_err() {
                                    tracing::warn!("irc {id}: the matcher is behind; a message was skipped");
                                }
                            }
                            Line::Ping(arg) => {
                                let _ = write.send(Message::Text(format!("PONG :{arg}"))).await;
                            }
                            Line::Reconnect => reconnect = true,
                            Line::Notice(msg_id, channel, text) => {
                                tracing::info!("irc {id}: notice {msg_id} in #{channel}: {text}");
                                if is_join_pushback(&msg_id) {
                                    bucket.drain().await;
                                }
                            }
                            Line::Other => {}
                        }
                    }
                    if reconnect {
                        tracing::info!("irc {id}: Twitch asked for a reconnect");
                        quick = true;
                        break;
                    }
                },
                _ = keepalive.tick() => {
                    if last_rx.elapsed() > SILENCE {
                        tracing::warn!("irc {id}: silent for {}s; reconnecting", SILENCE.as_secs());
                        break;
                    }
                    if write.send(Message::Text("PING :sn-mention-watcher".to_string())).await.is_err() {
                        break;
                    }
                }
            }
        }
        health.connected.fetch_sub(1, Ordering::Relaxed);
        health.channels.fetch_sub(joined.len(), Ordering::Relaxed);
        let _ = write.close().await;
        if closed {
            return;
        }
        let wait = if quick { Duration::from_secs(1) } else { backoff.next() };
        tracing::warn!("irc {id}: disconnected from {} channels; reconnecting in {}s", joined.len(), wait.as_secs());
        tokio::time::sleep(wait).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_channel_goes_to_the_emptiest_connection_with_room() {
        assert_eq!(pick(&[]), None);
        assert_eq!(pick(&[5, 2, 9]), Some(1));
        assert_eq!(pick(&[PER_CONNECTION, PER_CONNECTION]), None);
        assert_eq!(pick(&[PER_CONNECTION, 199]), Some(1));
    }

    #[test]
    fn a_nick_is_an_anonymous_justinfan() {
        let n = nick();
        assert!(n.starts_with("justinfan"));
        let digits: u32 = n["justinfan".len()..].parse().unwrap();
        assert!((10_000..100_000).contains(&digits));
    }

    #[test]
    fn rate_notices_slow_the_joins() {
        assert!(is_join_pushback("msg_ratelimit"));
        assert!(!is_join_pushback("msg_channel_suspended"));
    }
}
