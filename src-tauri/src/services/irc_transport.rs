//! Transport layer for the Twitch IRC connection.
//!
//! Twitch serves the same IRC protocol over raw TCP (port 6667) and over
//! WebSocket (`wss://irc-ws.chat.twitch.tv`, port 443). Port 6667 is dropped
//! on a meaningful number of home and corporate networks (ISP, router,
//! antivirus), so the connect path races the two: the preferred transport
//! starts immediately, the other only after a short stagger or once the
//! preferred one has failed. Healthy TCP wins long before the stagger, so
//! those users see no extra traffic and no extra latency.
//!
//! What was learned is kept in a small pure policy (`TransportPolicy`) and,
//! only for users moved off TCP, in a hint file so later launches skip the
//! stagger. The hint carries a reason and expires, so a lifted block or a
//! one-off bad night on the network does not pin anyone to TLS forever.

use crate::services::cache_service;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use log::{debug, warn};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::{connect_async_with_config, MaybeTlsStream, WebSocketStream};

pub const IRC_SERVER: &str = "irc.chat.twitch.tv";
pub const IRC_PORT: u16 = 6667;
/// No `:443` suffix on purpose: tungstenite would then send `Host: host:443`,
/// while browsers omit the default port. Keep the browser shape.
pub const IRC_WS_URL: &str = "wss://irc-ws.chat.twitch.tv";

/// How long the preferred transport gets on its own before the other one is
/// also started. A healthy TCP connect completes in well under this, so the
/// second transport never starts for those users.
pub const TRANSPORT_FALLBACK_DELAY: std::time::Duration = std::time::Duration::from_millis(2000);
/// Whole-race deadline. Same value the TCP-only connect used.
pub const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
/// Consecutive connect-ok-but-handshake-failed outcomes on one transport
/// before the next attempt uses the other. Two, so a single server-side blip
/// does not move a healthy user off TCP.
const HANDSHAKE_SWAP_THRESHOLD: u8 = 2;
/// A hint earned because TCP could not connect is re-checked weekly; one
/// earned because TCP connected but the handshake died (a firewall that
/// blackholes data) costs two 30 s handshake failures to re-learn, so it is
/// kept longer.
const HINT_TTL_CONNECT_MS: u64 = 7 * 24 * 60 * 60 * 1000;
const HINT_TTL_HANDSHAKE_MS: u64 = 30 * 24 * 60 * 60 * 1000;
const HINT_FILE: &str = "irc_transport.json";

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IrcTransport {
    #[default]
    Tcp,
    WebSocket,
}

impl IrcTransport {
    pub fn other(self) -> Self {
        match self {
            IrcTransport::Tcp => IrcTransport::WebSocket,
            IrcTransport::WebSocket => IrcTransport::Tcp,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            IrcTransport::Tcp => "tcp",
            IrcTransport::WebSocket => "websocket",
        }
    }
}

// ---------------------------------------------------------------------------
// Writer / reader
// ---------------------------------------------------------------------------

/// Write half of the IRC connection. Every caller sends whole `\r\n`-terminated
/// IRC lines, so one method covers both transports.
pub enum IrcWriter {
    Tcp(WriteHalf<TcpStream>),
    Ws(SplitSink<WsStream, Message>),
}

impl IrcWriter {
    /// Send one IRC line (caller includes the trailing `\r\n`). Always flushed:
    /// TCP flush is a no-op, the WebSocket sink needs it.
    pub async fn send_line(&mut self, line: &str) -> std::io::Result<()> {
        match self {
            IrcWriter::Tcp(w) => {
                w.write_all(line.as_bytes()).await?;
                w.flush().await
            }
            IrcWriter::Ws(sink) => sink.send(Message::text(line)).await.map_err(ws_to_io),
        }
    }

    /// Close the write side. TCP sends FIN; WebSocket sends a Close frame.
    /// Either way later sends fail and the session ends when the server
    /// answers or the read timeout fires.
    pub async fn shutdown(&mut self) -> std::io::Result<()> {
        match self {
            IrcWriter::Tcp(w) => w.shutdown().await,
            IrcWriter::Ws(sink) => sink.close().await.map_err(ws_to_io),
        }
    }
}

/// Read half of the IRC connection with `BufReader::read_line` semantics:
/// appends one line including its terminator, returns 0 at end of stream.
pub enum IrcReader {
    Tcp(BufReader<ReadHalf<TcpStream>>),
    Ws {
        stream: SplitStream<WsStream>,
        pending: VecDeque<String>,
    },
}

impl IrcReader {
    pub async fn read_line(&mut self, buf: &mut String) -> std::io::Result<usize> {
        match self {
            IrcReader::Tcp(r) => r.read_line(buf).await,
            IrcReader::Ws { stream, pending } => loop {
                if let Some(line) = pending.pop_front() {
                    buf.push_str(&line);
                    return Ok(line.len());
                }
                match stream.next().await {
                    None => return Ok(0),
                    Some(Ok(Message::Close(_))) => return Ok(0),
                    Some(Ok(Message::Text(text))) => split_irc_frame(text.as_str(), pending),
                    // The library answers WebSocket-level Ping itself on the
                    // next read; Twitch's IRC-level PING arrives as text and
                    // is handled upstream like on TCP.
                    Some(Ok(_)) => continue,
                    Some(Err(tungstenite::Error::ConnectionClosed))
                    | Some(Err(tungstenite::Error::AlreadyClosed))
                    | Some(Err(tungstenite::Error::Protocol(
                        tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
                    ))) => return Ok(0),
                    Some(Err(tungstenite::Error::Io(e))) => return Err(e),
                    Some(Err(e)) => return Err(std::io::Error::other(e)),
                }
            },
        }
    }
}

fn ws_to_io(e: tungstenite::Error) -> std::io::Error {
    match e {
        tungstenite::Error::Io(e) => e,
        tungstenite::Error::ConnectionClosed
        | tungstenite::Error::AlreadyClosed
        | tungstenite::Error::Protocol(_) => std::io::Error::new(std::io::ErrorKind::BrokenPipe, e),
        other => std::io::Error::other(other),
    }
}

/// Split one WebSocket text frame into IRC lines, each re-terminated with
/// `\r\n`. Twitch packs several lines per frame. An unterminated tail is
/// treated as a line and logged so field logs can prove whether it happens.
pub fn split_irc_frame(text: &str, out: &mut VecDeque<String>) {
    let mut rest = text;
    while !rest.is_empty() {
        match rest.find('\n') {
            Some(i) => {
                let line = rest[..i].trim_end_matches('\r');
                if !line.is_empty() {
                    out.push_back(format!("{}\r\n", line));
                }
                rest = &rest[i + 1..];
            }
            None => {
                let line = rest.trim_end_matches('\r');
                if !line.is_empty() {
                    debug!(
                        "[IRC Transport] frame without trailing CRLF ({} bytes)",
                        line.len()
                    );
                    out.push_back(format!("{}\r\n", line));
                }
                rest = "";
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Connect race
// ---------------------------------------------------------------------------

type ConnectResult = anyhow::Result<(IrcReader, IrcWriter)>;
type ConnectFuture = Pin<Box<dyn Future<Output = ConnectResult> + Send>>;

async fn connect_one(transport: IrcTransport) -> ConnectResult {
    match transport {
        IrcTransport::Tcp => {
            let stream = TcpStream::connect((IRC_SERVER, IRC_PORT)).await?;
            let (r, w) = tokio::io::split(stream);
            Ok((IrcReader::Tcp(BufReader::new(r)), IrcWriter::Tcp(w)))
        }
        IrcTransport::WebSocket => {
            // Third argument disables Nagle so CAP / PASS / NICK do not each
            // wait for the previous frame's ACK.
            let (ws, _resp) = connect_async_with_config(IRC_WS_URL, None, true)
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            let (sink, stream) = ws.split();
            Ok((
                IrcReader::Ws {
                    stream,
                    pending: VecDeque::new(),
                },
                IrcWriter::Ws(sink),
            ))
        }
    }
}

fn boxed(transport: IrcTransport) -> ConnectFuture {
    Box::pin(connect_one(transport))
}

/// Outcome of the race.
pub struct Connected {
    pub reader: IrcReader,
    pub writer: IrcWriter,
    pub transport: IrcTransport,
    /// The TCP error when TCP failed before WebSocket won this race. (A TCP
    /// attempt still pending at that moment is left to finish in the
    /// background and reports its own verdict.)
    pub tcp_failed: Option<String>,
}

/// Race the preferred transport against the other one. See module docs.
/// `Err(String)` carries a human-readable reason for the lifecycle log.
pub async fn connect_transport() -> Result<Connected, String> {
    let preferred = preferred_transport();
    let fallback = preferred.other();
    let deadline = tokio::time::Instant::now() + CONNECT_TIMEOUT;

    let mut first: Option<ConnectFuture> = Some(boxed(preferred));
    let mut second: Option<ConnectFuture> = None;
    let mut second_started = false;
    let mut first_err: Option<String> = None;
    let mut second_err: Option<String> = None;
    let stagger = tokio::time::sleep(TRANSPORT_FALLBACK_DELAY);
    tokio::pin!(stagger);

    // Every branch future is dropped before its handler runs, so the handlers
    // may replace `first` / `second` freely.
    loop {
        tokio::select! {
            r = async {
                match first.as_mut() {
                    Some(f) => f.await,
                    None => std::future::pending().await,
                }
            }, if first.is_some() => {
                first = None;
                match r {
                    Ok((reader, writer)) => {
                        let tcp_failed = if preferred == IrcTransport::WebSocket { second_err.take() } else { None };
                        if let Some(e) = tcp_failed.as_deref() {
                            note_tcp_unreachable(e);
                        }
                        return Ok(Connected { reader, writer, transport: preferred, tcp_failed });
                    }
                    Err(e) => {
                        first_err = Some(e.to_string());
                        if !second_started {
                            second_started = true;
                            second = Some(boxed(fallback));
                        }
                        if second.is_none() {
                            break;
                        }
                    }
                }
            }
            r = async {
                match second.as_mut() {
                    Some(f) => f.await,
                    None => std::future::pending().await,
                }
            }, if second.is_some() => {
                second = None;
                match r {
                    Ok((reader, writer)) => {
                        let tcp_failed = if fallback == IrcTransport::WebSocket { first_err.take() } else { None };
                        if let Some(e) = tcp_failed.as_deref() {
                            note_tcp_unreachable(e);
                        }
                        // WebSocket won while TCP is still pending: let TCP
                        // finish on its own so we learn whether it is slow or
                        // dead, without delaying this session. Bounded by the
                        // race deadline; it owns no socket unless TCP connects,
                        // and then drops it at once.
                        if fallback == IrcTransport::WebSocket {
                            if let Some(tcp) = first.take() {
                                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                                tokio::spawn(async move {
                                    match tokio::time::timeout(remaining, tcp).await {
                                        Ok(Ok(_conn)) => debug!("[IRC Transport] tcp verdict: slow but reachable; preference unchanged"),
                                        Ok(Err(e)) => note_tcp_unreachable(&e.to_string()),
                                        Err(_) => note_tcp_unreachable("connect timed out"),
                                    }
                                });
                            }
                        }
                        return Ok(Connected { reader, writer, transport: fallback, tcp_failed });
                    }
                    Err(e) => {
                        second_err = Some(e.to_string());
                        if first.is_none() {
                            break;
                        }
                    }
                }
            }
            _ = &mut stagger, if !second_started => {
                second_started = true;
                second = Some(boxed(fallback));
            }
            _ = tokio::time::sleep_until(deadline) => {
                // Dropping `first` / `second` closes any half-open socket.
                // Nothing is learned: there is no WebSocket evidence.
                return Err("connect timed out".to_string());
            }
        }
    }

    let (tcp_msg, ws_msg) = match preferred {
        IrcTransport::Tcp => (first_err, second_err),
        IrcTransport::WebSocket => (second_err, first_err),
    };
    Err(format!(
        "connect failed: tcp: {}; websocket: {}",
        tcp_msg.unwrap_or_else(|| "not attempted".into()),
        ws_msg.unwrap_or_else(|| "not attempted".into())
    ))
}

// ---------------------------------------------------------------------------
// Policy (pure) and its I/O wrappers
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HintReason {
    /// TCP could not connect while WebSocket could.
    Connect,
    /// TCP connected but the IRC handshake never completed, twice.
    Handshake,
}

impl HintReason {
    fn ttl_ms(self) -> u64 {
        match self {
            HintReason::Connect => HINT_TTL_CONNECT_MS,
            HintReason::Handshake => HINT_TTL_HANDSHAKE_MS,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HintFile {
    pub transport: IrcTransport,
    pub reason: HintReason,
    pub saved_at_ms: u64,
}

impl HintFile {
    fn is_fresh(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.saved_at_ms) < self.reason.ttl_ms()
    }
}

/// What the wrappers should write to disk after a policy call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Persist {
    pub transport: IrcTransport,
    pub reason: HintReason,
}

/// Everything the transport layer knows, with no I/O. Each method returns the
/// persistence action it wants, so the rules are unit-testable end to end.
#[derive(Debug, Default)]
pub struct TransportPolicy {
    preferred: IrcTransport,
    /// What the hint file holds (as loaded, or as last written).
    stored: Option<HintFile>,
    /// Why TCP is not trusted in this process, if it is not.
    tcp_suspect: Option<HintReason>,
    ws_authenticated: bool,
    handshake_fails: u8,
}

impl TransportPolicy {
    pub fn preferred(&self) -> IrcTransport {
        self.preferred
    }

    pub fn load(&mut self, hint: Option<HintFile>, now_ms: u64) {
        self.stored = hint;
        if let Some(h) = hint {
            if h.transport == IrcTransport::WebSocket && h.is_fresh(now_ms) {
                self.preferred = IrcTransport::WebSocket;
                self.tcp_suspect = Some(h.reason);
            }
        }
    }

    /// TCP failed in a race that WebSocket connected in.
    pub fn tcp_unreachable(&mut self, now_ms: u64) -> Option<Persist> {
        self.tcp_suspect = Some(HintReason::Connect);
        self.preferred = IrcTransport::WebSocket;
        self.handshake_fails = 0;
        if self.ws_authenticated {
            self.persist_if_needed(IrcTransport::WebSocket, HintReason::Connect, now_ms)
        } else {
            None
        }
    }

    /// Connect succeeded on `t` but the handshake did not reach `001` for a
    /// transient reason. Returns the new preferred transport on a swap.
    pub fn handshake_failed(&mut self, t: IrcTransport) -> Option<IrcTransport> {
        if t != self.preferred {
            return None;
        }
        self.handshake_fails += 1;
        if self.handshake_fails < HANDSHAKE_SWAP_THRESHOLD {
            return None;
        }
        self.handshake_fails = 0;
        self.preferred = t.other();
        if self.preferred == IrcTransport::WebSocket {
            self.tcp_suspect = Some(HintReason::Handshake);
        }
        Some(self.preferred)
    }

    /// The session reached `001` on `t`.
    pub fn authenticated(&mut self, t: IrcTransport, now_ms: u64) -> Option<Persist> {
        self.handshake_fails = 0;
        match t {
            IrcTransport::Tcp => {
                self.tcp_suspect = None;
                self.ws_authenticated = false;
                self.preferred = IrcTransport::Tcp;
                match self.stored {
                    Some(h) if h.transport == IrcTransport::WebSocket => {
                        self.persist_if_needed(IrcTransport::Tcp, HintReason::Connect, now_ms)
                    }
                    _ => None,
                }
            }
            IrcTransport::WebSocket => {
                self.ws_authenticated = true;
                match self.tcp_suspect {
                    Some(reason) => {
                        self.preferred = IrcTransport::WebSocket;
                        self.persist_if_needed(IrcTransport::WebSocket, reason, now_ms)
                    }
                    // A race win over a slow TCP: TCP stays primary.
                    None => None,
                }
            }
        }
    }

    fn persist_if_needed(
        &mut self,
        t: IrcTransport,
        reason: HintReason,
        now_ms: u64,
    ) -> Option<Persist> {
        let already = matches!(
            self.stored,
            Some(h) if h.transport == t && (t == IrcTransport::Tcp || h.is_fresh(now_ms))
        );
        if already {
            return None;
        }
        self.stored = Some(HintFile {
            transport: t,
            reason,
            saved_at_ms: now_ms,
        });
        Some(Persist {
            transport: t,
            reason,
        })
    }
}

static POLICY: Mutex<TransportPolicy> = Mutex::new(TransportPolicy {
    preferred: IrcTransport::Tcp,
    stored: None,
    tcp_suspect: None,
    ws_authenticated: false,
    handshake_fails: 0,
});
static HINT_LOADED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn policy() -> std::sync::MutexGuard<'static, TransportPolicy> {
    POLICY.lock().unwrap_or_else(|p| p.into_inner())
}

pub fn preferred_transport() -> IrcTransport {
    policy().preferred()
}

/// TCP failed in a race that WebSocket connected in (or the verdict task
/// after a WebSocket win). Never called on both-failed races.
pub fn note_tcp_unreachable(reason: &str) {
    let (changed, persist) = {
        let mut p = policy();
        let before = p.preferred();
        let persist = p.tcp_unreachable(now_ms());
        (before != IrcTransport::WebSocket, persist)
    };
    if changed {
        crate::services::irc_service::record_lifecycle(&format!(
            "tcp 6667 unreachable ({}); switching to websocket transport",
            reason
        ));
    }
    if let Some(p) = persist {
        write_hint(p);
    }
}

pub fn note_handshake_failure(t: IrcTransport) {
    let swapped = policy().handshake_failed(t);
    if let Some(next) = swapped {
        crate::services::irc_service::record_lifecycle(&format!(
            "handshake failed {} times on {}; next attempt uses {}",
            HANDSHAKE_SWAP_THRESHOLD,
            t.label(),
            next.label()
        ));
    }
}

pub fn note_authenticated(t: IrcTransport) {
    let persist = policy().authenticated(t, now_ms());
    if let Some(p) = persist {
        write_hint(p);
    }
}

fn hint_path() -> anyhow::Result<std::path::PathBuf> {
    Ok(cache_service::get_app_data_dir()?.join(HINT_FILE))
}

/// Load the persisted preference once per process. Blocking I/O runs on the
/// blocking pool; call it from the supervisor before its first token fetch,
/// never from the connect path.
pub async fn load_hint() {
    if HINT_LOADED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let loaded = tokio::task::spawn_blocking(|| -> Option<HintFile> {
        let path = hint_path().ok()?;
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str::<HintFile>(&text).ok()
    })
    .await
    .ok()
    .flatten();
    let preferred = {
        let mut p = policy();
        p.load(loaded, now_ms());
        p.preferred()
    };
    if preferred == IrcTransport::WebSocket {
        crate::services::irc_service::record_lifecycle(
            "transport hint: websocket (tcp 6667 was unusable on an earlier run)",
        );
    }
}

/// Write the hint. The policy has already decided this is a real change.
fn write_hint(p: Persist) {
    let file = HintFile {
        transport: p.transport,
        reason: p.reason,
        saved_at_ms: now_ms(),
    };
    tokio::task::spawn_blocking(move || {
        let Ok(path) = hint_path() else { return };
        match serde_json::to_string(&file) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    warn!("[IRC Transport] could not save transport hint: {}", e);
                } else {
                    crate::services::irc_service::record_lifecycle(&format!(
                        "transport preference saved: {} ({:?})",
                        file.transport.label(),
                        file.reason
                    ));
                }
            }
            Err(e) => warn!("[IRC Transport] could not encode transport hint: {}", e),
        }
    });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        let mut q = VecDeque::new();
        split_irc_frame(text, &mut q);
        q.into_iter().collect()
    }

    #[test]
    fn split_frame_multi_line() {
        let got = lines(":tmi.twitch.tv 001 x :Welcome\r\n:tmi.twitch.tv 002 x :Your host\r\n");
        assert_eq!(
            got,
            vec![
                ":tmi.twitch.tv 001 x :Welcome\r\n".to_string(),
                ":tmi.twitch.tv 002 x :Your host\r\n".to_string()
            ]
        );
    }

    #[test]
    fn split_frame_unterminated_tail_is_a_line() {
        assert_eq!(
            lines("PING :tmi.twitch.tv"),
            vec!["PING :tmi.twitch.tv\r\n".to_string()]
        );
    }

    #[test]
    fn split_frame_bare_newline_and_empty_segments() {
        assert_eq!(
            lines("A\nB\r\n\r\n"),
            vec!["A\r\n".to_string(), "B\r\n".to_string()]
        );
        assert!(lines("").is_empty());
        assert!(lines("\r\n").is_empty());
    }

    #[test]
    fn hint_file_round_trip_and_wire_form() {
        let h = HintFile {
            transport: IrcTransport::WebSocket,
            reason: HintReason::Connect,
            saved_at_ms: 42,
        };
        let json = serde_json::to_string(&h).unwrap();
        assert_eq!(
            json,
            r#"{"transport":"websocket","reason":"connect","saved_at_ms":42}"#
        );
        assert_eq!(serde_json::from_str::<HintFile>(&json).unwrap(), h);
    }

    #[test]
    fn transport_other_is_an_involution() {
        for t in [IrcTransport::Tcp, IrcTransport::WebSocket] {
            assert_eq!(t.other().other(), t);
        }
    }

    // ---- policy scenarios ----------------------------------------------

    const NOW: u64 = 1_000_000_000_000;
    const DAY: u64 = 24 * 60 * 60 * 1000;

    fn ws_hint(reason: HintReason, age_ms: u64) -> HintFile {
        HintFile {
            transport: IrcTransport::WebSocket,
            reason,
            saved_at_ms: NOW - age_ms,
        }
    }

    #[test]
    fn policy_healthy_tcp_never_persists() {
        let mut p = TransportPolicy::default();
        assert_eq!(p.preferred(), IrcTransport::Tcp);
        assert_eq!(p.authenticated(IrcTransport::Tcp, NOW), None);
        assert_eq!(p.authenticated(IrcTransport::Tcp, NOW), None);
        assert_eq!(p.preferred(), IrcTransport::Tcp);
    }

    #[test]
    fn policy_syn_drop_persists_after_ws_authenticates() {
        let mut p = TransportPolicy::default();
        assert_eq!(
            p.tcp_unreachable(NOW),
            None,
            "no persist before a WebSocket login"
        );
        assert_eq!(p.preferred(), IrcTransport::WebSocket);
        assert_eq!(
            p.authenticated(IrcTransport::WebSocket, NOW),
            Some(Persist {
                transport: IrcTransport::WebSocket,
                reason: HintReason::Connect
            })
        );
        assert_eq!(
            p.authenticated(IrcTransport::WebSocket, NOW),
            None,
            "written once"
        );
    }

    #[test]
    fn policy_verdict_after_auth_persists() {
        let mut p = TransportPolicy::default();
        assert_eq!(p.authenticated(IrcTransport::WebSocket, NOW), None);
        assert_eq!(
            p.preferred(),
            IrcTransport::Tcp,
            "a race win alone keeps TCP primary"
        );
        assert_eq!(
            p.tcp_unreachable(NOW),
            Some(Persist {
                transport: IrcTransport::WebSocket,
                reason: HintReason::Connect
            })
        );
        assert_eq!(p.preferred(), IrcTransport::WebSocket);
    }

    #[test]
    fn policy_slow_tcp_learns_nothing() {
        let mut p = TransportPolicy::default();
        assert_eq!(p.authenticated(IrcTransport::WebSocket, NOW), None);
        assert_eq!(p.preferred(), IrcTransport::Tcp);
        assert_eq!(p.authenticated(IrcTransport::Tcp, NOW), None);
    }

    #[test]
    fn policy_offline_then_back_online_is_silent() {
        let mut p = TransportPolicy::default();
        // Both transports failed: connect_transport calls nothing.
        assert_eq!(p.authenticated(IrcTransport::Tcp, NOW), None);
        assert_eq!(p.preferred(), IrcTransport::Tcp);
    }

    #[test]
    fn policy_two_handshake_failures_swap_and_persist_on_ws_login() {
        let mut p = TransportPolicy::default();
        assert_eq!(p.handshake_failed(IrcTransport::Tcp), None);
        assert_eq!(
            p.handshake_failed(IrcTransport::Tcp),
            Some(IrcTransport::WebSocket)
        );
        assert_eq!(p.preferred(), IrcTransport::WebSocket);
        assert_eq!(
            p.authenticated(IrcTransport::WebSocket, NOW),
            Some(Persist {
                transport: IrcTransport::WebSocket,
                reason: HintReason::Handshake
            })
        );
    }

    #[test]
    fn policy_single_blip_does_not_swap() {
        let mut p = TransportPolicy::default();
        assert_eq!(p.handshake_failed(IrcTransport::Tcp), None);
        assert_eq!(p.authenticated(IrcTransport::Tcp, NOW), None);
        assert_eq!(
            p.handshake_failed(IrcTransport::Tcp),
            None,
            "counter was reset"
        );
        assert_eq!(p.preferred(), IrcTransport::Tcp);
    }

    #[test]
    fn policy_handshake_failure_on_non_preferred_is_ignored() {
        let mut p = TransportPolicy::default();
        assert_eq!(p.handshake_failed(IrcTransport::WebSocket), None);
        assert_eq!(p.handshake_failed(IrcTransport::WebSocket), None);
        assert_eq!(p.preferred(), IrcTransport::Tcp);
    }

    #[test]
    fn policy_fresh_ws_hint_is_used_and_not_rewritten() {
        let mut p = TransportPolicy::default();
        p.load(Some(ws_hint(HintReason::Connect, DAY)), NOW);
        assert_eq!(p.preferred(), IrcTransport::WebSocket);
        assert_eq!(p.authenticated(IrcTransport::WebSocket, NOW), None);
    }

    #[test]
    fn policy_expired_hint_tries_tcp_first_and_refreshes_when_still_blocked() {
        let mut p = TransportPolicy::default();
        p.load(Some(ws_hint(HintReason::Connect, 8 * DAY)), NOW);
        assert_eq!(p.preferred(), IrcTransport::Tcp);
        assert_eq!(p.tcp_unreachable(NOW), None);
        assert_eq!(
            p.authenticated(IrcTransport::WebSocket, NOW),
            Some(Persist {
                transport: IrcTransport::WebSocket,
                reason: HintReason::Connect
            })
        );
        let mut q = TransportPolicy::default();
        q.load(Some(ws_hint(HintReason::Handshake, 8 * DAY)), NOW);
        assert_eq!(
            q.preferred(),
            IrcTransport::WebSocket,
            "handshake hints live 30 days"
        );
    }

    #[test]
    fn policy_ws_hint_with_broken_ws_returns_to_tcp() {
        let mut p = TransportPolicy::default();
        p.load(Some(ws_hint(HintReason::Connect, DAY)), NOW);
        assert_eq!(p.handshake_failed(IrcTransport::WebSocket), None);
        assert_eq!(
            p.handshake_failed(IrcTransport::WebSocket),
            Some(IrcTransport::Tcp)
        );
        assert_eq!(
            p.authenticated(IrcTransport::Tcp, NOW),
            Some(Persist {
                transport: IrcTransport::Tcp,
                reason: HintReason::Connect
            })
        );
        assert_eq!(p.authenticated(IrcTransport::Tcp, NOW), None);
    }

    #[test]
    fn policy_block_lifted_after_expiry_clears_hint() {
        let mut p = TransportPolicy::default();
        p.load(Some(ws_hint(HintReason::Connect, 8 * DAY)), NOW);
        assert_eq!(
            p.authenticated(IrcTransport::Tcp, NOW),
            Some(Persist {
                transport: IrcTransport::Tcp,
                reason: HintReason::Connect
            })
        );
    }

    /// Real network: anonymous IRC handshake over the WebSocket gateway. Proves
    /// TLS, the Host header without :443, frame splitting and 001 detection.
    /// `cargo test live_wss_anonymous_handshake -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_wss_anonymous_handshake() {
        let (mut reader, mut writer) = connect_one(IrcTransport::WebSocket)
            .await
            .expect("wss connect");
        writer
            .send_line("CAP REQ :twitch.tv/tags twitch.tv/commands\r\n")
            .await
            .unwrap();
        writer.send_line("NICK justinfan12345\r\n").await.unwrap();
        let mut line = String::new();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            line.clear();
            let n = tokio::time::timeout_at(deadline, reader.read_line(&mut line))
                .await
                .expect("timed out waiting for 001")
                .expect("read");
            assert!(n > 0, "closed before 001");
            assert!(
                line.ends_with("\r\n"),
                "line not CRLF-terminated: {:?}",
                line
            );
            println!("<< {}", line.trim_end());
            if line.contains(" 001 ") {
                break;
            }
        }
        writer.shutdown().await.unwrap();
    }
}
