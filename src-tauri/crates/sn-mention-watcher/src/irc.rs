//! Anonymous Twitch chat connections: as many as the watched set needs, at
//! most 200 channels each, every JOIN paced by the one process-wide bucket.
//!
//! Live channels are joined before offline ones, across every connection: a
//! connection joins an offline channel only while no connection has a live
//! one waiting (`LiveGate`), so after a restart the live chats are covered
//! first and the offline ones fill in behind them.
//!
//! Each connection owns the list of channels it should be in. When it drops
//! (or Twitch asks it to reconnect), it opens a new socket, waits out a
//! backoff, and joins its whole list again through the bucket, live first.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, Notify};
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
    /// A channel to be in, and whether it is live.
    Join(String, bool),
    /// A channel still waiting to be joined went live: it moves up.
    Promote(String),
    Part(String),
}

/// How many live joins are waiting, across every connection. Offline joins
/// wait while it is above zero.
#[derive(Default)]
pub struct LiveGate {
    waiting: AtomicUsize,
    idle: Notify,
}

impl LiveGate {
    fn add(&self, n: usize) {
        self.waiting.fetch_add(n, Ordering::SeqCst);
    }

    fn sub(&self, n: usize) {
        if n == 0 {
            return;
        }
        if self.waiting.fetch_sub(n, Ordering::SeqCst) == n {
            self.idle.notify_waiters();
        }
    }

    fn is_idle(&self) -> bool {
        self.waiting.load(Ordering::SeqCst) == 0
    }
}

/// One connection's channels waiting to be joined: live ones first. Every live
/// entry is counted in the gate while it waits.
struct Queue {
    live: VecDeque<String>,
    offline: VecDeque<String>,
    gate: Arc<LiveGate>,
}

impl Queue {
    fn new(gate: Arc<LiveGate>) -> Self {
        Queue { live: VecDeque::new(), offline: VecDeque::new(), gate }
    }

    fn push(&mut self, channel: String, live: bool) {
        if live {
            self.live.push_back(channel);
            self.gate.add(1);
        } else {
            self.offline.push_back(channel);
        }
    }

    /// Moves a waiting offline channel up to the live queue. False when it was not waiting.
    fn promote(&mut self, channel: &str) -> bool {
        match self.offline.iter().position(|c| c == channel) {
            Some(i) => {
                self.offline.remove(i);
                self.push(channel.to_string(), true);
                true
            }
            None => false,
        }
    }

    fn remove(&mut self, channel: &str) {
        if let Some(i) = self.live.iter().position(|c| c == channel) {
            self.live.remove(i);
            self.gate.sub(1);
        }
        if let Some(i) = self.offline.iter().position(|c| c == channel) {
            self.offline.remove(i);
        }
    }

    /// Whether this connection has a join it may make now.
    fn ready(&self) -> bool {
        !self.live.is_empty() || (!self.offline.is_empty() && self.gate.is_idle())
    }

    /// Waiting only on other connections' live joins.
    fn held(&self) -> bool {
        self.live.is_empty() && !self.offline.is_empty() && !self.gate.is_idle()
    }

    /// The next channel to join: a live one, else an offline one when no live
    /// join waits anywhere.
    fn next(&mut self) -> Option<String> {
        if let Some(c) = self.live.pop_front() {
            self.gate.sub(1);
            return Some(c);
        }
        if self.gate.is_idle() {
            return self.offline.pop_front();
        }
        None
    }

    /// Back to the front, after a write failed.
    fn unpop(&mut self, channel: String, live: bool) {
        if live {
            self.live.push_front(channel);
            self.gate.add(1);
        } else {
            self.offline.push_front(channel);
        }
    }

    fn clear(&mut self) {
        let n = self.live.len();
        self.live.clear();
        self.offline.clear();
        self.gate.sub(n);
    }
}

impl Drop for Queue {
    fn drop(&mut self) {
        self.clear();
    }
}

struct Conn {
    tx: mpsc::UnboundedSender<Cmd>,
    channels: HashSet<String>,
}

/// Which connection holds which channel, and the connections themselves.
pub struct Pool {
    conns: Vec<Conn>,
    /// Channel login to (connection, live).
    assigned: HashMap<String, (usize, bool)>,
    bucket: bucket::Shared,
    gate: Arc<LiveGate>,
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

/// What `apply` does to move from what is assigned to what is wanted.
#[derive(Debug, Default, PartialEq)]
struct Plan {
    part: Vec<String>,
    /// Assigned and still waiting offline, now live.
    promote: Vec<String>,
    /// New channels, in `wanted`'s order (live first).
    join: Vec<(String, bool)>,
}

fn plan(assigned: &HashMap<String, (usize, bool)>, wanted: &[(String, bool)]) -> Plan {
    let want: HashMap<&str, bool> = wanted.iter().map(|(c, live)| (c.as_str(), *live)).collect();
    let mut part: Vec<String> = assigned.keys().filter(|c| !want.contains_key(c.as_str())).cloned().collect();
    part.sort();
    let mut promote: Vec<String> = assigned
        .iter()
        .filter(|(c, (_, was_live))| !*was_live && want.get(c.as_str()) == Some(&true))
        .map(|(c, _)| c.clone())
        .collect();
    promote.sort();
    let mut seen = HashSet::new();
    let join = wanted
        .iter()
        .filter(|(c, _)| !assigned.contains_key(c) && seen.insert(c.clone()))
        .cloned()
        .collect();
    Plan { part, promote, join }
}

impl Pool {
    pub fn new(lines: mpsc::Sender<Privmsg>, health: Arc<Health>) -> Self {
        Pool {
            conns: Vec::new(),
            assigned: HashMap::new(),
            bucket: bucket::Shared::new(),
            gate: Arc::new(LiveGate::default()),
            lines,
            health,
        }
    }

    /// Be in exactly these channels (lowercase logins, live first): part the
    /// rest, move up any waiting channel that went live, join the new ones.
    pub fn apply(&mut self, wanted: &[(String, bool)]) {
        let plan = plan(&self.assigned, wanted);
        for channel in plan.part {
            if let Some((i, _)) = self.assigned.remove(&channel) {
                self.conns[i].channels.remove(&channel);
                let _ = self.conns[i].tx.send(Cmd::Part(channel));
            }
        }
        for channel in plan.promote {
            if let Some((i, live)) = self.assigned.get_mut(&channel) {
                *live = true;
                let _ = self.conns[*i].tx.send(Cmd::Promote(channel));
            }
        }
        // A channel that went offline keeps its place; only new ones are queued.
        for (channel, live) in wanted {
            if let Some((_, was)) = self.assigned.get_mut(channel) {
                *was = *live;
            }
        }
        let (joining, kept) = (plan.join.len(), self.assigned.len());
        for (channel, live) in plan.join {
            let loads: Vec<usize> = self.conns.iter().map(|c| c.channels.len()).collect();
            let i = pick(&loads).unwrap_or_else(|| self.open());
            self.conns[i].channels.insert(channel.clone());
            self.assigned.insert(channel.clone(), (i, live));
            let _ = self.conns[i].tx.send(Cmd::Join(channel, live));
        }
        if joining > 0 {
            tracing::info!("irc: {} channels to join, {} kept, {} connections", joining, kept, self.conns.len());
        }
    }

    fn open(&mut self) -> usize {
        let (tx, rx) = mpsc::unbounded_channel();
        let id = self.conns.len();
        tokio::spawn(run_conn(id, rx, self.bucket.clone(), self.gate.clone(), self.lines.clone(), self.health.clone()));
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
    gate: Arc<LiveGate>,
    lines: mpsc::Sender<Privmsg>,
    health: Arc<Health>,
) {
    // What this connection should be in, and whether each is live, whatever
    // the socket's state.
    let mut want: HashMap<String, bool> = HashMap::new();
    let mut backoff = Backoff::default();
    loop {
        // Take commands that came in while the socket was down.
        while let Ok(cmd) = cmds.try_recv() {
            match cmd {
                Cmd::Join(c, live) => {
                    want.insert(c, live);
                }
                Cmd::Promote(c) => {
                    if let Some(live) = want.get_mut(&c) {
                        *live = true;
                    }
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
        let mut queue = Queue::new(gate.clone());
        let mut order: Vec<(&String, &bool)> = want.iter().collect();
        order.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
        for (c, live) in order {
            queue.push(c.clone(), *live);
        }
        let mut joined: HashSet<String> = HashSet::new();
        let mut last_rx = Instant::now();
        let mut keepalive = tokio::time::interval(KEEPALIVE);
        keepalive.tick().await;
        let mut quick = false;
        let mut closed = false;
        loop {
            tokio::select! {
                cmd = cmds.recv() => match cmd {
                    Some(Cmd::Join(c, live)) => {
                        if want.insert(c.clone(), live).is_none() {
                            queue.push(c, live);
                        }
                    }
                    Some(Cmd::Promote(c)) => {
                        if let Some(live) = want.get_mut(&c) {
                            *live = true;
                        }
                        queue.promote(&c);
                    }
                    Some(Cmd::Part(c)) => {
                        want.remove(&c);
                        queue.remove(&c);
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
                // Offline joins here are held while another connection has a
                // live one waiting; this wakes them when the last one goes.
                _ = gate.idle.notified(), if queue.held() => {},
                _ = bucket.acquire(), if queue.ready() => {
                    if let Some(c) = queue.next() {
                        if want.contains_key(&c) && !joined.contains(&c) {
                            if write.send(Message::Text(format!("JOIN #{c}"))).await.is_err() {
                                let live = want.get(&c).copied().unwrap_or(false);
                                queue.unpop(c, live);
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
        // Its waiting live joins no longer hold the others back while it is down.
        queue.clear();
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

    fn w(list: &[(&str, bool)]) -> Vec<(String, bool)> {
        list.iter().map(|(c, l)| (c.to_string(), *l)).collect()
    }

    #[test]
    fn a_plan_parts_what_left_promotes_what_went_live_and_joins_the_new_live_first() {
        let mut assigned = HashMap::new();
        assigned.insert("kept".to_string(), (0, true));
        assigned.insert("gone".to_string(), (0, false));
        assigned.insert("waking".to_string(), (1, false));
        let p = plan(&assigned, &w(&[("newlive", true), ("kept", true), ("waking", true), ("newoff", false), ("newlive", true)]));
        assert_eq!(p.part, vec!["gone".to_string()]);
        assert_eq!(p.promote, vec!["waking".to_string()]);
        // Never twice, and in the wanted order (live first).
        assert_eq!(p.join, w(&[("newlive", true), ("newoff", false)]));
    }

    #[test]
    fn a_big_set_spreads_over_connections_once_each() {
        // 7,600 channels as the Worker sends them: live first.
        let wanted: Vec<(String, bool)> = (0..7_600).map(|i| (format!("ch{i:05}"), i < 2_000)).collect();
        let first = plan(&HashMap::new(), &wanted);
        assert_eq!(first.join.len(), 7_600);
        assert!(first.join[..2_000].iter().all(|(_, live)| *live));
        assert!(first.join[2_000..].iter().all(|(_, live)| !*live));
        let mut loads: Vec<usize> = Vec::new();
        let mut at: HashMap<String, usize> = HashMap::new();
        for (c, _) in &first.join {
            let i = pick(&loads).unwrap_or_else(|| {
                loads.push(0);
                loads.len() - 1
            });
            loads[i] += 1;
            assert!(at.insert(c.clone(), i).is_none(), "{c} assigned twice");
        }
        assert_eq!(loads.len(), 38);
        assert!(loads.iter().all(|n| *n <= PER_CONNECTION));
        // A second pass with everything assigned changes nothing.
        let assigned: HashMap<String, (usize, bool)> =
            wanted.iter().map(|(c, l)| (c.clone(), (at[c], *l))).collect();
        assert_eq!(plan(&assigned, &wanted), Plan::default());
    }

    #[test]
    fn offline_joins_wait_while_any_live_join_waits_anywhere() {
        let gate = Arc::new(LiveGate::default());
        let mut a = Queue::new(gate.clone());
        let mut b = Queue::new(gate.clone());
        a.push("aoff".into(), false);
        b.push("blive".into(), true);
        assert!(!a.ready() && a.held());
        assert_eq!(a.next(), None);
        assert!(b.ready());
        assert_eq!(b.next(), Some("blive".into()));
        assert!(gate.is_idle());
        assert!(a.ready() && !a.held());
        assert_eq!(a.next(), Some("aoff".into()));
    }

    #[test]
    fn live_first_within_a_connection_and_a_promotion_jumps_the_queue() {
        let gate = Arc::new(LiveGate::default());
        let mut q = Queue::new(gate.clone());
        q.push("off1".into(), false);
        q.push("off2".into(), false);
        q.push("live1".into(), true);
        assert!(q.promote("off2"));
        assert!(!q.promote("nowhere"));
        assert_eq!(q.next(), Some("live1".into()));
        assert_eq!(q.next(), Some("off2".into()));
        assert_eq!(q.next(), Some("off1".into()));
        assert_eq!(q.next(), None);
        assert!(gate.is_idle());
    }

    #[test]
    fn the_gate_counts_only_what_still_waits() {
        let gate = Arc::new(LiveGate::default());
        {
            let mut q = Queue::new(gate.clone());
            q.push("a".into(), true);
            q.push("b".into(), true);
            q.remove("a");
            assert!(!gate.is_idle());
            let c = q.next().unwrap();
            q.unpop(c, true);
            assert!(!gate.is_idle());
            // A connection that goes down stops holding the others.
        }
        assert!(gate.is_idle());
    }
}
