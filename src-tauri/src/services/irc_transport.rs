//! Transport layer for the Twitch IRC connection.
//!
//! Twitch serves the same IRC protocol over WebSocket
//! (`wss://irc-ws.chat.twitch.tv`, the route twitch.tv itself uses) and as
//! raw IRC over TLS (`irc.chat.twitch.tv:443`, the route Chatterino-family
//! clients use). The two are different hostnames on different servers, so
//! each is a real second route when the other is blocked or degraded. Both
//! are encrypted: plain text (port 6667) is never used, because it exposes
//! the OAuth token on every connect and some networks inspect unencrypted
//! traffic and cut the connection every minute or so. A network that breaks
//! TLS to Twitch breaks video, sign-in and the API too, so a plain-text chat
//! route would rescue nobody.
//!
//! The connect path races the two: the preferred transport starts at once,
//! the other only after a short stagger or as soon as the preferred one has
//! failed. A healthy preferred transport wins long before the stagger, so
//! those users see no extra traffic and no extra latency.
//!
//! What was learned is kept in a small pure policy (`TransportPolicy`) and,
//! only while the fallback is in use, in a hint file so later launches skip
//! the default. The hint carries a reason and expires, so a lifted block or a
//! one-off bad night on the network does not pin anyone to the fallback.
//!
//! Besides failed connects and handshakes, the policy learns from sessions
//! that signed in fine and then went silent: two such sessions on one
//! transport within half an hour (the first proven not to be an outage by an
//! immediate clean reconnect) move the connection to the other transport.

use crate::services::cache_service;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use log::{debug, warn};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::time::Duration;
use tokio::io::{
    AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, ReadHalf, WriteHalf,
};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::{connect_async_with_config, MaybeTlsStream, WebSocketStream};

pub const IRC_SERVER: &str = "irc.chat.twitch.tv";
/// Raw IRC over TLS. Twitch documents 6697 as well; 443 is what reference
/// clients default to and what restrictive firewalls let through.
pub const IRC_TLS_PORT: u16 = 443;
/// No `:443` suffix on purpose: tungstenite would then send `Host: host:443`,
/// while browsers omit the default port. Keep the browser shape.
pub const IRC_WS_URL: &str = "wss://irc-ws.chat.twitch.tv";

/// The transport every session starts on unless the policy has learned
/// otherwise.
pub const DEFAULT_TRANSPORT: IrcTransport = IrcTransport::WebSocket;

/// How long the preferred transport gets on its own before the other one is
/// also started. A healthy WebSocket connect (TLS included) completes in a few
/// hundred ms, so the fallback never starts for those users; the margin keeps
/// a slow but working default from losing the race on every connect.
pub const TRANSPORT_FALLBACK_DELAY: Duration = Duration::from_millis(3000);
/// Whole-race deadline.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// Consecutive connect-ok-but-handshake-failed outcomes on one transport
/// before the next attempt uses the other. Two, so a single server-side blip
/// does not move a healthy user.
const HANDSHAKE_SWAP_THRESHOLD: u8 = 2;
/// A confirmed silent session counts toward a move for this long.
const STALL_WINDOW_MS: u64 = 30 * 60 * 1000;
/// Hint lifetimes. Retrying the default after a failed connect costs at most
/// one stagger, so that hint is short. A handshake blackhole or a transport
/// that keeps going silent costs a minute or more of dead chat to re-learn,
/// so those are kept longer.
const HINT_TTL_CONNECT_MS: u64 = 24 * 60 * 60 * 1000;
const HINT_TTL_HANDSHAKE_MS: u64 = 7 * 24 * 60 * 60 * 1000;
const HINT_TTL_STALL_MS: u64 = 7 * 24 * 60 * 60 * 1000;
const HINT_FILE: &str = "irc_transport_v2.json";
/// The TCP-default era's hint. Its `tcp` value meant "TCP works again", which
/// reads backwards now, so it is never loaded, only removed.
const LEGACY_HINT_FILE: &str = "irc_transport.json";

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// The raw IRC byte stream: TLS over TCP in production, plain TCP only in
/// tests that talk to a local server.
pub trait IrcByteStream: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> IrcByteStream for T {}
pub type IrcStream = Box<dyn IrcByteStream>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IrcTransport {
    Irc,
    WebSocket,
}

impl IrcTransport {
    pub fn other(self) -> Self {
        match self {
            IrcTransport::Irc => IrcTransport::WebSocket,
            IrcTransport::WebSocket => IrcTransport::Irc,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            IrcTransport::Irc => "irc",
            IrcTransport::WebSocket => "websocket",
        }
    }

    /// Route and host, for lifecycle lines a support reader can act on.
    pub fn describe(self) -> &'static str {
        match self {
            IrcTransport::Irc => "irc over tls (irc.chat.twitch.tv)",
            IrcTransport::WebSocket => "websocket (irc-ws.chat.twitch.tv)",
        }
    }

    fn idx(self) -> usize {
        match self {
            IrcTransport::Irc => 0,
            IrcTransport::WebSocket => 1,
        }
    }
}

// ---------------------------------------------------------------------------
// Writer / reader
// ---------------------------------------------------------------------------

/// Write half of the IRC connection. Every caller sends whole `\r\n`-terminated
/// IRC lines, so one method covers both transports.
pub enum IrcWriter {
    Irc(WriteHalf<IrcStream>),
    Ws(SplitSink<WsStream, Message>),
}

impl IrcWriter {
    /// Send one IRC line (caller includes the trailing `\r\n`). Always flushed:
    /// both the TLS stream and the WebSocket sink buffer until then.
    pub async fn send_line(&mut self, line: &str) -> std::io::Result<()> {
        match self {
            IrcWriter::Irc(w) => {
                w.write_all(line.as_bytes()).await?;
                w.flush().await
            }
            IrcWriter::Ws(sink) => sink.send(Message::text(line)).await.map_err(ws_to_io),
        }
    }

    /// Close the write side. TLS sends close_notify then FIN; WebSocket sends
    /// a Close frame.
    /// Either way later sends fail and the session ends when the server
    /// answers or the read timeout fires.
    pub async fn shutdown(&mut self) -> std::io::Result<()> {
        match self {
            IrcWriter::Irc(w) => w.shutdown().await,
            IrcWriter::Ws(sink) => sink.close().await.map_err(ws_to_io),
        }
    }
}

/// Read half of the IRC connection with `BufReader::read_line` semantics:
/// appends one line including its terminator, returns 0 at end of stream.
pub enum IrcReader {
    Irc(BufReader<ReadHalf<IrcStream>>),
    Ws {
        stream: SplitStream<WsStream>,
        pending: VecDeque<String>,
    },
}

impl IrcReader {
    pub async fn read_line(&mut self, buf: &mut String) -> std::io::Result<usize> {
        match self {
            IrcReader::Irc(r) => r.read_line(buf).await,
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
                    // is handled upstream like on raw IRC.
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

/// Where each transport connects. Production uses `Endpoints::twitch()`;
/// tests point both at local servers.
#[derive(Clone, Debug)]
pub struct Endpoints {
    pub irc_host: String,
    pub irc_port: u16,
    pub ws_url: String,
    /// Tests only: talk plain TCP to a local IRC stand-in. Production has no
    /// way to reach a plain-text route.
    #[cfg(test)]
    pub irc_plain: bool,
}

impl Endpoints {
    pub fn twitch() -> Self {
        Endpoints {
            irc_host: IRC_SERVER.to_string(),
            irc_port: IRC_TLS_PORT,
            ws_url: IRC_WS_URL.to_string(),
            #[cfg(test)]
            irc_plain: false,
        }
    }
}

fn connect_one(transport: IrcTransport, ep: &Endpoints) -> ConnectFuture {
    match transport {
        IrcTransport::Irc => {
            let host = ep.irc_host.clone();
            let port = ep.irc_port;
            #[cfg(test)]
            let plain = ep.irc_plain;
            #[cfg(not(test))]
            let plain = false;
            Box::pin(async move {
                let tcp = TcpStream::connect((host.as_str(), port)).await?;
                // CAP / PASS / NICK go out back to back.
                let _ = tcp.set_nodelay(true);
                let stream: IrcStream = if plain {
                    Box::new(tcp)
                } else {
                    tls_connect(&host, tcp).await?
                };
                let (r, w) = tokio::io::split(stream);
                Ok((IrcReader::Irc(BufReader::new(r)), IrcWriter::Irc(w)))
            })
        }
        IrcTransport::WebSocket => {
            let url = ep.ws_url.clone();
            Box::pin(async move {
                // Third argument disables Nagle so CAP / PASS / NICK do not
                // each wait for the previous frame's ACK.
                let (ws, _resp) = connect_async_with_config(url.as_str(), None, true)
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
            })
        }
    }
}

/// TLS over an open TCP connection, with the same TLS stack and trust the
/// WebSocket uses on this platform: the OS's own (schannel, Security.framework,
/// OpenSSL) on desktop, so a corporate or antivirus CA the system trusts works
/// on both routes alike.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
async fn tls_connect(host: &str, tcp: TcpStream) -> anyhow::Result<IrcStream> {
    let connector = tokio_native_tls::native_tls::TlsConnector::new()?;
    let tls = tokio_native_tls::TlsConnector::from(connector)
        .connect(host, tcp)
        .await?;
    Ok(Box::new(tls))
}

/// Android and iOS: rustls with the bundled Mozilla roots, as the WebSocket
/// uses, so the cross-compiles never link OpenSSL.
#[cfg(any(target_os = "android", target_os = "ios"))]
async fn tls_connect(host: &str, tcp: TcpStream) -> anyhow::Result<IrcStream> {
    use tokio_rustls::rustls;
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_root_certificates(roots)
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from(host.to_string())?;
    let tls = tokio_rustls::TlsConnector::from(std::sync::Arc::new(config))
        .connect(name, tcp)
        .await?;
    Ok(Box::new(tls))
}

/// Outcome of the race.
pub struct Connected {
    pub reader: IrcReader,
    pub writer: IrcWriter,
    pub transport: IrcTransport,
    /// The other transport's error, when it failed before this one won.
    pub other_failed: Option<String>,
}

/// A won race, plus the preferred transport's connect when the other one won
/// while it was still in flight, so the caller can learn whether it was slow
/// or dead without delaying this session.
struct RaceWin {
    connected: Connected,
    preferred_pending: Option<ConnectFuture>,
}

/// Race `preferred` against the other transport. No policy side effects, so
/// tests can drive it against local servers. `Err` is a human-readable reason.
async fn race(
    ep: &Endpoints,
    preferred: IrcTransport,
    stagger: Duration,
    deadline: tokio::time::Instant,
) -> Result<RaceWin, String> {
    let fallback = preferred.other();
    let mut first: Option<ConnectFuture> = Some(connect_one(preferred, ep));
    let mut second: Option<ConnectFuture> = None;
    let mut second_started = false;
    let mut first_err: Option<String> = None;
    let mut second_err: Option<String> = None;
    let stagger = tokio::time::sleep(stagger);
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
                        return Ok(RaceWin {
                            connected: Connected { reader, writer, transport: preferred, other_failed: second_err.take() },
                            preferred_pending: None,
                        });
                    }
                    Err(e) => {
                        first_err = Some(e.to_string());
                        if !second_started {
                            second_started = true;
                            second = Some(connect_one(fallback, ep));
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
                        return Ok(RaceWin {
                            connected: Connected { reader, writer, transport: fallback, other_failed: first_err.take() },
                            preferred_pending: first.take(),
                        });
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
                second = Some(connect_one(fallback, ep));
            }
            _ = tokio::time::sleep_until(deadline) => {
                // Dropping `first` / `second` closes any half-open socket.
                return Err("connect timed out".to_string());
            }
        }
    }

    let (irc_msg, ws_msg) = match preferred {
        IrcTransport::Irc => (first_err, second_err),
        IrcTransport::WebSocket => (second_err, first_err),
    };
    Err(format!(
        "connect failed: irc over tls: {}; websocket: {}",
        irc_msg.unwrap_or_else(|| "not attempted".into()),
        ws_msg.unwrap_or_else(|| "not attempted".into())
    ))
}

/// Connect to Twitch over whichever transport the policy prefers, racing the
/// other one, and feed what the race taught into the policy.
pub async fn connect_transport() -> Result<Connected, String> {
    let preferred = preferred_transport();
    let deadline = tokio::time::Instant::now() + CONNECT_TIMEOUT;
    match race(
        &Endpoints::twitch(),
        preferred,
        TRANSPORT_FALLBACK_DELAY,
        deadline,
    )
    .await
    {
        Ok(win) => {
            let connected = win.connected;
            if let Some(e) = connected.other_failed.as_deref() {
                note_unreachable(connected.transport.other(), e);
            }
            // The other transport won while the preferred one was still
            // connecting: let it finish on its own so we learn whether it is
            // slow or dead. Bounded by the race deadline; it owns no socket
            // unless it connects, and then drops it at once.
            if let Some(pending) = win.preferred_pending {
                tokio::spawn(async move {
                    match tokio::time::timeout_at(deadline, pending).await {
                        Ok(Ok(_conn)) => debug!(
                            "[IRC Transport] {} verdict: slow but reachable; preference unchanged",
                            preferred.label()
                        ),
                        Ok(Err(e)) => note_unreachable(preferred, &e.to_string()),
                        Err(_) => note_unreachable(preferred, "connect timed out"),
                    }
                });
            }
            Ok(connected)
        }
        Err(e) => {
            note_connect_failed();
            Err(e)
        }
    }
}

// ---------------------------------------------------------------------------
// Policy (pure) and its I/O wrappers
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HintReason {
    /// The default could not connect while the fallback could.
    Connect,
    /// The default connected but the IRC handshake never completed, twice.
    Handshake,
    /// The default signed in fine and then went silent, twice in a row.
    Stall,
}

impl HintReason {
    fn ttl_ms(self) -> u64 {
        match self {
            HintReason::Connect => HINT_TTL_CONNECT_MS,
            HintReason::Handshake => HINT_TTL_HANDSHAKE_MS,
            HintReason::Stall => HINT_TTL_STALL_MS,
        }
    }
}

/// On disk only while the fallback is in use.
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

/// What the wrappers should do to the hint file after a policy call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Persist {
    Save(HintFile),
    Clear,
}

/// What a silent session end did to the policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StallOutcome {
    /// Not the preferred transport, or moving is settled for this run.
    Ignored,
    /// First silence: it becomes a strike once the next connect on the same
    /// transport signs in without a failure in between (so not an outage).
    Noted,
    /// Second silence within the window: the next connect uses this one.
    Moved(IrcTransport),
    /// Both transports went silent in this run: back to the default for good.
    Settled(IrcTransport),
}

/// Everything the transport layer knows, with no I/O. Each method returns the
/// persistence action it wants, so the rules are unit-testable end to end.
#[derive(Debug)]
pub struct TransportPolicy {
    preferred: IrcTransport,
    /// What the hint file holds (as loaded, or as last written).
    stored: Option<HintFile>,
    /// The transport this process moved away from, and why.
    suspect: Option<(IrcTransport, HintReason)>,
    last_authenticated: Option<IrcTransport>,
    handshake_fails: u8,
    /// A silent session end awaiting proof that the network was up.
    stall_pending: Option<IrcTransport>,
    /// A confirmed silent session end: transport and when.
    stall_strike: Option<(IrcTransport, u64)>,
    /// Transports this process already left because they kept going silent.
    left_for_stall: [bool; 2],
    stall_settled: bool,
}

impl Default for TransportPolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl TransportPolicy {
    pub const fn new() -> Self {
        TransportPolicy {
            preferred: DEFAULT_TRANSPORT,
            stored: None,
            suspect: None,
            last_authenticated: None,
            handshake_fails: 0,
            stall_pending: None,
            stall_strike: None,
            left_for_stall: [false, false],
            stall_settled: false,
        }
    }

    pub fn preferred(&self) -> IrcTransport {
        self.preferred
    }

    pub fn load(&mut self, hint: Option<HintFile>, now_ms: u64) {
        self.stored = hint;
        if let Some(h) = hint {
            if h.transport != DEFAULT_TRANSPORT && h.is_fresh(now_ms) {
                self.preferred = h.transport;
                self.suspect = Some((DEFAULT_TRANSPORT, h.reason));
            }
        }
    }

    fn move_to(&mut self, t: IrcTransport) {
        if self.preferred != t {
            self.preferred = t;
            self.handshake_fails = 0;
            self.stall_pending = None;
            self.stall_strike = None;
        }
    }

    /// `t` failed to connect in a race the other transport connected in.
    pub fn unreachable(&mut self, t: IrcTransport, now_ms: u64) -> Option<Persist> {
        let to = t.other();
        self.suspect = Some((t, HintReason::Connect));
        self.move_to(to);
        if self.last_authenticated != Some(to) {
            // Persisted once `to` signs in (see `authenticated`).
            return None;
        }
        if to == DEFAULT_TRANSPORT {
            self.clear()
        } else {
            self.save(to, HintReason::Connect, now_ms)
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
        self.suspect = Some((t, HintReason::Handshake));
        self.move_to(t.other());
        Some(self.preferred)
    }

    /// A session on `t` signed in, then went silent until it ended.
    pub fn session_deaf(&mut self, t: IrcTransport, now_ms: u64) -> StallOutcome {
        if t != self.preferred || self.stall_settled {
            self.stall_pending = None;
            return StallOutcome::Ignored;
        }
        let confirmed = matches!(
            self.stall_strike,
            Some((s, at)) if s == t && now_ms.saturating_sub(at) < STALL_WINDOW_MS
        );
        if !confirmed {
            self.stall_strike = None;
            self.stall_pending = Some(t);
            return StallOutcome::Noted;
        }
        if self.left_for_stall[t.other().idx()] {
            // Both went silent in this run: the network, not the transport.
            // Settle on the default rather than bounce between them.
            self.stall_settled = true;
            self.suspect = None;
            self.move_to(DEFAULT_TRANSPORT);
            return StallOutcome::Settled(DEFAULT_TRANSPORT);
        }
        self.left_for_stall[t.idx()] = true;
        self.suspect = Some((t, HintReason::Stall));
        self.move_to(t.other());
        StallOutcome::Moved(self.preferred)
    }

    /// A connect attempt failed outright (or the token fetch did): the last
    /// silent session may have been an outage, so it does not count.
    pub fn connect_failed(&mut self) {
        self.stall_pending = None;
    }

    /// The session reached `001` on `t`.
    pub fn authenticated(&mut self, t: IrcTransport, now_ms: u64) -> Option<Persist> {
        self.handshake_fails = 0;
        self.last_authenticated = Some(t);
        if self.stall_pending.take() == Some(t) {
            self.stall_strike = Some((t, now_ms));
        }
        if t != self.preferred {
            // A race win over a slow preferred transport teaches nothing.
            return None;
        }
        if t == DEFAULT_TRANSPORT {
            self.suspect = None;
            return self.clear();
        }
        match self.suspect {
            Some((s, reason)) if s == DEFAULT_TRANSPORT => self.save(t, reason, now_ms),
            _ => None,
        }
    }

    fn clear(&mut self) -> Option<Persist> {
        self.stored.take().map(|_| Persist::Clear)
    }

    fn save(&mut self, t: IrcTransport, reason: HintReason, now_ms: u64) -> Option<Persist> {
        let already = matches!(
            self.stored,
            Some(h) if h.transport == t && h.is_fresh(now_ms)
        );
        if already {
            return None;
        }
        let file = HintFile {
            transport: t,
            reason,
            saved_at_ms: now_ms,
        };
        self.stored = Some(file);
        Some(Persist::Save(file))
    }
}

static POLICY: Mutex<TransportPolicy> = Mutex::new(TransportPolicy::new());
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

/// `t` failed in a race the other transport connected in (or the verdict
/// task after the other one won). Never called on both-failed races.
pub fn note_unreachable(t: IrcTransport, reason: &str) {
    let (changed, persist) = {
        let mut p = policy();
        let before = p.preferred();
        let persist = p.unreachable(t, now_ms());
        (before != p.preferred(), persist)
    };
    if changed {
        crate::services::irc_service::record_lifecycle(&format!(
            "{} unreachable ({}); switching to {}",
            t.describe(),
            reason,
            t.other().describe()
        ));
    }
    if let Some(p) = persist {
        write_persist(p);
    }
}

pub fn note_handshake_failure(t: IrcTransport) {
    let swapped = policy().handshake_failed(t);
    if let Some(next) = swapped {
        crate::services::irc_service::record_lifecycle(&format!(
            "handshake failed {} times on {}; next attempt uses {}",
            HANDSHAKE_SWAP_THRESHOLD,
            t.describe(),
            next.describe()
        ));
    }
}

/// A session on `t` signed in and later ended after going silent.
pub fn note_session_deaf(t: IrcTransport) {
    let outcome = policy().session_deaf(t, now_ms());
    let line = match outcome {
        StallOutcome::Ignored => return,
        StallOutcome::Noted => format!(
            "{} went silent after sign-in; if it happens again within 30 min, chat moves to {}",
            t.describe(),
            t.other().describe()
        ),
        StallOutcome::Moved(to) => format!(
            "{} went silent twice within 30 min; next connection uses {}",
            t.describe(),
            to.describe()
        ),
        StallOutcome::Settled(to) => format!(
            "both transports went silent in this run; staying on {}",
            to.describe()
        ),
    };
    crate::services::irc_service::record_lifecycle(&line);
}

/// A connect attempt (or the token fetch before it) failed outright.
pub fn note_connect_failed() {
    policy().connect_failed();
}

pub fn note_authenticated(t: IrcTransport) {
    let persist = policy().authenticated(t, now_ms());
    if let Some(p) = persist {
        write_persist(p);
    }
}

fn app_data_file(name: &str) -> anyhow::Result<std::path::PathBuf> {
    Ok(cache_service::get_app_data_dir()?.join(name))
}

/// Load the persisted preference once per process. Blocking I/O runs on the
/// blocking pool; call it from the supervisor before its first token fetch,
/// never from the connect path.
pub async fn load_hint() {
    if HINT_LOADED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let loaded = tokio::task::spawn_blocking(|| -> Option<HintFile> {
        if let Ok(legacy) = app_data_file(LEGACY_HINT_FILE) {
            let _ = std::fs::remove_file(legacy);
        }
        let text = std::fs::read_to_string(app_data_file(HINT_FILE).ok()?).ok()?;
        serde_json::from_str::<HintFile>(&text).ok()
    })
    .await
    .ok()
    .flatten();
    let (preferred, reason) = {
        let mut p = policy();
        p.load(loaded, now_ms());
        (p.preferred(), loaded.map(|h| h.reason))
    };
    if preferred != DEFAULT_TRANSPORT {
        crate::services::irc_service::record_lifecycle(&format!(
            "transport hint: {} ({:?}; {} was unusable on an earlier run)",
            preferred.describe(),
            reason.unwrap_or(HintReason::Connect),
            DEFAULT_TRANSPORT.describe()
        ));
    }
}

/// Apply the hint change. The policy has already decided it is a real change.
fn write_persist(p: Persist) {
    tokio::task::spawn_blocking(move || {
        let Ok(path) = app_data_file(HINT_FILE) else {
            return;
        };
        match p {
            Persist::Save(file) => match serde_json::to_string(&file) {
                Ok(json) => {
                    if let Err(e) = std::fs::write(&path, json) {
                        warn!("[IRC Transport] could not save transport hint: {}", e);
                    } else {
                        crate::services::irc_service::record_lifecycle(&format!(
                            "transport preference saved: {} ({:?})",
                            file.transport.describe(),
                            file.reason
                        ));
                    }
                }
                Err(e) => warn!("[IRC Transport] could not encode transport hint: {}", e),
            },
            Persist::Clear => match std::fs::remove_file(&path) {
                Ok(()) => crate::services::irc_service::record_lifecycle(&format!(
                    "transport preference cleared: back to {}",
                    DEFAULT_TRANSPORT.describe()
                )),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => warn!("[IRC Transport] could not clear transport hint: {}", e),
            },
        }
    });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use IrcTransport::{Irc, WebSocket as Ws};

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
            transport: Irc,
            reason: HintReason::Stall,
            saved_at_ms: 42,
        };
        let json = serde_json::to_string(&h).unwrap();
        assert_eq!(
            json,
            r#"{"transport":"irc","reason":"stall","saved_at_ms":42}"#
        );
        assert_eq!(serde_json::from_str::<HintFile>(&json).unwrap(), h);
    }

    #[test]
    fn transport_other_is_an_involution() {
        for t in [Irc, Ws] {
            assert_eq!(t.other().other(), t);
        }
    }

    #[test]
    fn default_policy_matches_the_static() {
        assert_eq!(TransportPolicy::default().preferred(), DEFAULT_TRANSPORT);
        assert_eq!(DEFAULT_TRANSPORT, Ws);
        assert_ne!(HINT_FILE, LEGACY_HINT_FILE);
    }

    // ---- policy scenarios ----------------------------------------------

    const NOW: u64 = 1_000_000_000_000;
    const MIN: u64 = 60 * 1000;
    const DAY: u64 = 24 * 60 * MIN;

    fn irc_hint(reason: HintReason, age_ms: u64) -> HintFile {
        HintFile {
            transport: Irc,
            reason,
            saved_at_ms: NOW - age_ms,
        }
    }

    fn saved(t: IrcTransport, reason: HintReason, at: u64) -> Option<Persist> {
        Some(Persist::Save(HintFile {
            transport: t,
            reason,
            saved_at_ms: at,
        }))
    }

    #[test]
    fn healthy_default_never_persists() {
        let mut p = TransportPolicy::new();
        assert_eq!(p.preferred(), Ws);
        assert_eq!(p.authenticated(Ws, NOW), None);
        assert_eq!(p.authenticated(Ws, NOW), None);
        assert_eq!(p.preferred(), Ws);
    }

    #[test]
    fn dead_websocket_moves_to_irc_and_persists_after_irc_login() {
        let mut p = TransportPolicy::new();
        assert_eq!(p.unreachable(Ws, NOW), None, "no persist before a login");
        assert_eq!(p.preferred(), Irc);
        assert_eq!(
            p.authenticated(Irc, NOW),
            saved(Irc, HintReason::Connect, NOW)
        );
        assert_eq!(p.authenticated(Irc, NOW), None, "written once");
    }

    #[test]
    fn verdict_after_irc_login_persists() {
        let mut p = TransportPolicy::new();
        assert_eq!(p.authenticated(Irc, NOW), None);
        assert_eq!(p.preferred(), Ws, "a race win alone keeps the default");
        assert_eq!(p.unreachable(Ws, NOW), saved(Irc, HintReason::Connect, NOW));
        assert_eq!(p.preferred(), Irc);
    }

    #[test]
    fn slow_websocket_learns_nothing() {
        let mut p = TransportPolicy::new();
        assert_eq!(p.authenticated(Irc, NOW), None);
        assert_eq!(p.preferred(), Ws);
        assert_eq!(p.authenticated(Ws, NOW), None);
    }

    #[test]
    fn dead_fallback_keeps_default_and_writes_nothing() {
        let mut p = TransportPolicy::new();
        assert_eq!(p.unreachable(Irc, NOW), None);
        assert_eq!(p.preferred(), Ws);
        assert_eq!(p.authenticated(Ws, NOW), None);
    }

    #[test]
    fn two_handshake_failures_swap_and_persist_on_irc_login() {
        let mut p = TransportPolicy::new();
        assert_eq!(p.handshake_failed(Ws), None);
        assert_eq!(p.handshake_failed(Ws), Some(Irc));
        assert_eq!(
            p.authenticated(Irc, NOW),
            saved(Irc, HintReason::Handshake, NOW)
        );
    }

    #[test]
    fn single_handshake_blip_does_not_swap() {
        let mut p = TransportPolicy::new();
        assert_eq!(p.handshake_failed(Ws), None);
        assert_eq!(p.authenticated(Ws, NOW), None);
        assert_eq!(p.handshake_failed(Ws), None, "counter was reset");
        assert_eq!(p.preferred(), Ws);
    }

    #[test]
    fn handshake_failure_on_non_preferred_is_ignored() {
        let mut p = TransportPolicy::new();
        assert_eq!(p.handshake_failed(Irc), None);
        assert_eq!(p.handshake_failed(Irc), None);
        assert_eq!(p.preferred(), Ws);
    }

    #[test]
    fn fresh_irc_hint_is_used_and_not_rewritten() {
        let mut p = TransportPolicy::new();
        p.load(Some(irc_hint(HintReason::Connect, MIN)), NOW);
        assert_eq!(p.preferred(), Irc);
        assert_eq!(p.authenticated(Irc, NOW), None);
    }

    #[test]
    fn expired_hint_tries_default_and_clears_on_login() {
        let mut p = TransportPolicy::new();
        p.load(Some(irc_hint(HintReason::Connect, 2 * DAY)), NOW);
        assert_eq!(p.preferred(), Ws);
        assert_eq!(p.authenticated(Ws, NOW), Some(Persist::Clear));
        assert_eq!(p.authenticated(Ws, NOW), None, "cleared once");

        let mut q = TransportPolicy::new();
        q.load(Some(irc_hint(HintReason::Handshake, 2 * DAY)), NOW);
        assert_eq!(q.preferred(), Irc, "handshake hints live a week");
        let mut r = TransportPolicy::new();
        r.load(Some(irc_hint(HintReason::Stall, 2 * DAY)), NOW);
        assert_eq!(r.preferred(), Irc, "stall hints live a week");
    }

    #[test]
    fn expired_hint_still_blocked_saves_again() {
        let mut p = TransportPolicy::new();
        p.load(Some(irc_hint(HintReason::Connect, 2 * DAY)), NOW);
        assert_eq!(p.unreachable(Ws, NOW), None);
        assert_eq!(
            p.authenticated(Irc, NOW),
            saved(Irc, HintReason::Connect, NOW)
        );
    }

    #[test]
    fn irc_hint_with_broken_irc_returns_to_default_and_clears() {
        let mut p = TransportPolicy::new();
        p.load(Some(irc_hint(HintReason::Connect, MIN)), NOW);
        assert_eq!(p.handshake_failed(Irc), None);
        assert_eq!(p.handshake_failed(Irc), Some(Ws));
        assert_eq!(p.authenticated(Ws, NOW), Some(Persist::Clear));
    }

    #[test]
    fn a_stored_default_hint_is_ignored_and_cleared() {
        let mut p = TransportPolicy::new();
        p.load(
            Some(HintFile {
                transport: Ws,
                reason: HintReason::Connect,
                saved_at_ms: NOW,
            }),
            NOW,
        );
        assert_eq!(p.preferred(), Ws);
        assert_eq!(p.authenticated(Ws, NOW), Some(Persist::Clear));
    }

    // ---- silent sessions -------------------------------------------------

    #[test]
    fn silent_twice_with_a_clean_reconnect_between_moves_and_persists() {
        let mut p = TransportPolicy::new();
        assert_eq!(p.authenticated(Ws, NOW), None);
        assert_eq!(p.session_deaf(Ws, NOW + MIN), StallOutcome::Noted);
        assert_eq!(p.authenticated(Ws, NOW + MIN), None);
        assert_eq!(p.session_deaf(Ws, NOW + 2 * MIN), StallOutcome::Moved(Irc));
        assert_eq!(p.preferred(), Irc);
        assert_eq!(
            p.authenticated(Irc, NOW + 2 * MIN),
            saved(Irc, HintReason::Stall, NOW + 2 * MIN)
        );
    }

    #[test]
    fn an_outage_between_silences_is_not_a_strike() {
        let mut p = TransportPolicy::new();
        assert_eq!(p.session_deaf(Ws, NOW), StallOutcome::Noted);
        p.connect_failed();
        assert_eq!(p.authenticated(Ws, NOW + MIN), None);
        assert_eq!(p.session_deaf(Ws, NOW + 2 * MIN), StallOutcome::Noted);
        assert_eq!(p.preferred(), Ws);
    }

    #[test]
    fn a_strike_expires_after_the_window() {
        let mut p = TransportPolicy::new();
        assert_eq!(p.session_deaf(Ws, NOW), StallOutcome::Noted);
        assert_eq!(p.authenticated(Ws, NOW), None);
        assert_eq!(p.session_deaf(Ws, NOW + 31 * MIN), StallOutcome::Noted);
        assert_eq!(p.preferred(), Ws);
    }

    #[test]
    fn a_reconnect_on_the_other_transport_does_not_confirm() {
        let mut p = TransportPolicy::new();
        assert_eq!(p.session_deaf(Ws, NOW), StallOutcome::Noted);
        assert_eq!(p.authenticated(Irc, NOW), None, "slow-race win on irc");
        assert_eq!(p.session_deaf(Ws, NOW + MIN), StallOutcome::Noted);
    }

    #[test]
    fn silence_on_the_non_preferred_transport_is_ignored() {
        let mut p = TransportPolicy::new();
        assert_eq!(p.session_deaf(Irc, NOW), StallOutcome::Ignored);
        assert_eq!(p.authenticated(Irc, NOW), None);
        assert_eq!(p.session_deaf(Irc, NOW), StallOutcome::Ignored);
        assert_eq!(p.preferred(), Ws);
    }

    #[test]
    fn both_transports_silent_settles_on_default_for_the_run() {
        let mut p = TransportPolicy::new();
        assert_eq!(p.session_deaf(Ws, NOW), StallOutcome::Noted);
        p.authenticated(Ws, NOW);
        assert_eq!(p.session_deaf(Ws, NOW), StallOutcome::Moved(Irc));
        assert_eq!(
            p.authenticated(Irc, NOW),
            saved(Irc, HintReason::Stall, NOW)
        );
        assert_eq!(p.session_deaf(Irc, NOW), StallOutcome::Noted);
        assert_eq!(p.authenticated(Irc, NOW), None);
        assert_eq!(p.session_deaf(Irc, NOW), StallOutcome::Settled(Ws));
        assert_eq!(p.preferred(), Ws);
        assert_eq!(p.authenticated(Ws, NOW), Some(Persist::Clear));
        assert_eq!(p.session_deaf(Ws, NOW), StallOutcome::Ignored);
        p.authenticated(Ws, NOW);
        assert_eq!(p.session_deaf(Ws, NOW), StallOutcome::Ignored);
        assert_eq!(p.preferred(), Ws);
    }

    #[test]
    fn an_irc_user_whose_irc_goes_silent_returns_to_default() {
        let mut p = TransportPolicy::new();
        p.load(Some(irc_hint(HintReason::Connect, MIN)), NOW);
        assert_eq!(p.session_deaf(Irc, NOW), StallOutcome::Noted);
        p.authenticated(Irc, NOW);
        assert_eq!(p.session_deaf(Irc, NOW), StallOutcome::Moved(Ws));
        assert_eq!(p.authenticated(Ws, NOW), Some(Persist::Clear));
    }

    // ---- connect race against local servers ------------------------------

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::net::TcpListener;

    /// Local plain-TCP stand-in for the IRC route: accepts connections and
    /// holds them open, counting them.
    async fn tcp_server() -> (u16, Arc<AtomicUsize>) {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        let count = Arc::new(AtomicUsize::new(0));
        let c = count.clone();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((s, _)) = l.accept().await {
                c.fetch_add(1, Ordering::SeqCst);
                held.push(s);
            }
        });
        (port, count)
    }

    /// Completes WebSocket handshakes and holds the sockets open.
    async fn ws_server() -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((s, _)) = l.accept().await {
                if let Ok(ws) = tokio_tungstenite::accept_async(s).await {
                    held.push(ws);
                }
            }
        });
        port
    }

    /// A port nothing listens on.
    async fn dead_port() -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        l.local_addr().unwrap().port()
    }

    fn endpoints(irc_port: u16, ws_port: u16) -> Endpoints {
        Endpoints {
            irc_host: "127.0.0.1".into(),
            irc_port,
            ws_url: format!("ws://127.0.0.1:{}", ws_port),
            irc_plain: true,
        }
    }

    fn deadline(secs: u64) -> tokio::time::Instant {
        tokio::time::Instant::now() + Duration::from_secs(secs)
    }

    #[tokio::test]
    async fn race_healthy_websocket_wins_and_irc_never_starts() {
        let (tcp, tcp_accepts) = tcp_server().await;
        let ws = ws_server().await;
        let win = race(
            &endpoints(tcp, ws),
            Ws,
            Duration::from_millis(500),
            deadline(10),
        )
        .await
        .unwrap_or_else(|e| panic!("race failed: {}", e));
        assert_eq!(win.connected.transport, Ws);
        assert!(win.connected.other_failed.is_none());
        assert!(win.preferred_pending.is_none());
        tokio::time::sleep(Duration::from_millis(800)).await;
        assert_eq!(tcp_accepts.load(Ordering::SeqCst), 0, "fallback started");
    }

    #[tokio::test]
    async fn race_dead_websocket_falls_back_without_waiting_for_the_stagger() {
        let (tcp, _) = tcp_server().await;
        let ws = dead_port().await;
        let started = std::time::Instant::now();
        let win = race(
            &endpoints(tcp, ws),
            Ws,
            Duration::from_secs(10),
            deadline(15),
        )
        .await
        .unwrap_or_else(|e| panic!("race failed: {}", e));
        assert_eq!(win.connected.transport, Irc);
        assert!(win.connected.other_failed.is_some());
        assert!(win.preferred_pending.is_none());
        assert!(
            started.elapsed() < Duration::from_secs(8),
            "waited for the stagger"
        );
    }

    #[tokio::test]
    async fn race_slow_websocket_loses_after_the_stagger_and_leaves_a_verdict() {
        let (tcp, _) = tcp_server().await;
        // Accepts TCP but never answers the WebSocket upgrade.
        let (ws, _) = tcp_server().await;
        let win = race(
            &endpoints(tcp, ws),
            Ws,
            Duration::from_millis(200),
            deadline(10),
        )
        .await
        .unwrap_or_else(|e| panic!("race failed: {}", e));
        assert_eq!(win.connected.transport, Irc);
        assert!(win.connected.other_failed.is_none());
        assert!(win.preferred_pending.is_some(), "no verdict future");
    }

    #[tokio::test]
    async fn race_both_dead_names_both() {
        let tcp = dead_port().await;
        let ws = dead_port().await;
        let err = match race(
            &endpoints(tcp, ws),
            Ws,
            Duration::from_millis(200),
            deadline(15),
        )
        .await
        {
            Ok(_) => panic!("connected to nothing"),
            Err(e) => e,
        };
        assert!(err.starts_with("connect failed: irc over tls: "), "{}", err);
        assert!(err.contains("; websocket: "), "{}", err);
        assert!(!err.contains("not attempted"), "{}", err);
    }

    #[tokio::test]
    async fn race_prefers_irc_when_told_to() {
        let (tcp, _) = tcp_server().await;
        let ws = ws_server().await;
        let win = race(
            &endpoints(tcp, ws),
            Irc,
            Duration::from_millis(500),
            deadline(10),
        )
        .await
        .unwrap_or_else(|e| panic!("race failed: {}", e));
        assert_eq!(win.connected.transport, Irc);
    }

    /// Anonymous read-only handshake to `001` on an already connected pair.
    async fn anonymous_welcome(mut reader: IrcReader, mut writer: IrcWriter) {
        writer
            .send_line("CAP REQ :twitch.tv/tags twitch.tv/commands\r\n")
            .await
            .unwrap();
        writer.send_line("NICK justinfan54321\r\n").await.unwrap();
        let mut line = String::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            line.clear();
            let n = tokio::time::timeout_at(deadline, reader.read_line(&mut line))
                .await
                .expect("timed out waiting for 001")
                .expect("read");
            assert!(n > 0, "closed before 001");
            if line.contains(" 001 ") {
                break;
            }
        }
        writer.shutdown().await.unwrap();
    }

    /// Real network: the production race prefers WebSocket and signs in over
    /// it; with the WebSocket endpoint dead it falls back at once to IRC over
    /// TLS on irc.chat.twitch.tv:443, which proves the TLS route end to end.
    /// `cargo test live_race_default_and_fallback -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_race_default_and_fallback() {
        let started = std::time::Instant::now();
        let win = race(
            &Endpoints::twitch(),
            DEFAULT_TRANSPORT,
            TRANSPORT_FALLBACK_DELAY,
            deadline(15),
        )
        .await
        .unwrap_or_else(|e| panic!("race failed: {}", e));
        println!(
            "default: {:?} in {:?}",
            win.connected.transport,
            started.elapsed()
        );
        assert_eq!(win.connected.transport, Ws);
        assert!(win.preferred_pending.is_none());
        anonymous_welcome(win.connected.reader, win.connected.writer).await;

        let dead = dead_port().await;
        let ep = Endpoints {
            ws_url: format!("ws://127.0.0.1:{}", dead),
            ..Endpoints::twitch()
        };
        let started = std::time::Instant::now();
        let win = race(
            &ep,
            DEFAULT_TRANSPORT,
            TRANSPORT_FALLBACK_DELAY,
            deadline(15),
        )
        .await
        .unwrap_or_else(|e| panic!("race failed: {}", e));
        println!(
            "fallback: {:?} in {:?} (websocket error: {:?})",
            win.connected.transport,
            started.elapsed(),
            win.connected.other_failed
        );
        assert_eq!(win.connected.transport, Irc);
        assert!(win.connected.other_failed.is_some());
        anonymous_welcome(win.connected.reader, win.connected.writer).await;
    }

    /// Real network: anonymous IRC handshake over the WebSocket gateway. Proves
    /// TLS, the Host header without :443, frame splitting and 001 detection.
    /// `cargo test live_wss_anonymous_handshake -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_wss_anonymous_handshake() {
        let (mut reader, mut writer) = connect_one(Ws, &Endpoints::twitch())
            .await
            .expect("wss connect");
        writer
            .send_line("CAP REQ :twitch.tv/tags twitch.tv/commands\r\n")
            .await
            .unwrap();
        writer.send_line("NICK justinfan12345\r\n").await.unwrap();
        let mut line = String::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
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
