//! One way out for every request StreamNook makes to its own hosts:
//! streamnook.app, cdn.streamnook.app and modroom.streamnook.app.
//!
//! All three sit on Cloudflare's shared free-plan addresses, and some networks
//! silently drop that whole pool. For a member on such a network every request
//! hung until the client timeout while Twitch, 7TV, FFZ and BTTV kept working,
//! so watch minutes, the version report, update checks, identity, membership
//! and art all failed with nothing on screen.
//!
//! Two routes reach the same origin:
//!
//! * **direct**: the host itself.
//! * **relay**: the `sn-relay` Supabase Edge Function, which forwards an
//!   allowlisted request to the origin and streams the answer back. Supabase is
//!   on a different network, and the same members already reach it.
//!
//! The rules:
//!
//! * Direct first, with a short connect timeout, so a dropped address costs
//!   seconds rather than the whole request timeout.
//! * Only a request that never reached the origin moves to the other route: a
//!   connect failure (refused, reset, timed out, TLS cut short), or a timeout on
//!   a GET or HEAD, which is safe to repeat. An HTTP status is an answer and is
//!   returned as it came, whatever it is.
//! * The working route is remembered per host. While a host is on the relay, a
//!   probe tries direct again in the background (after 5 minutes, doubling to
//!   30), and the first answer moves the host back. Probes only ride real
//!   traffic, so an idle app runs nothing.
//! * The relay is a transport, not a trust boundary. Method, body and the
//!   `Authorization` header go through unchanged and the origin does all auth.
//!   The app holds no relay secret; that is between the relay and the origin.
//!
//! Callers keep building requests with reqwest as before and hand the builder
//! to [`send`] (or [`send_download`] for long transfers) in place of `.send()`.
//! The request runs on this module's own client, keeping its headers, body and
//! per-request timeout.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use log::{debug, warn};
use once_cell::sync::Lazy;
use reqwest::{Client, Method, Request, RequestBuilder, Response, Url};

/// Long enough for a slow but working link to finish TCP and TLS, short enough
/// that a dropped address is found out before the caller gives up.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(6);
/// Whole-request cap for API calls, the same as `services::http::client()`.
const API_TIMEOUT: Duration = Duration::from_secs(30);
/// Long transfers have no total cap; a stall this long frees the connection.
const READ_TIMEOUT: Duration = Duration::from_secs(20);
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
const FIRST_PROBE: Duration = Duration::from_secs(5 * 60);
const MAX_PROBE_GAP: Duration = Duration::from_secs(30 * 60);
/// How often a repeating failure is allowed back into the log.
pub const LOG_EVERY: Duration = Duration::from_secs(15 * 60);
const RELAY_FUNCTION: &str = "sn-relay";

/// A StreamNook-owned host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Host {
    Site,
    Cdn,
    Modroom,
}

impl Host {
    const ALL: [Host; 3] = [Host::Site, Host::Cdn, Host::Modroom];

    pub fn name(self) -> &'static str {
        match self {
            Host::Site => "streamnook.app",
            Host::Cdn => "cdn.streamnook.app",
            Host::Modroom => "modroom.streamnook.app",
        }
    }

    /// The host an https URL on the default port points at, if it is ours.
    fn of(url: &Url) -> Option<Host> {
        if url.scheme() != "https" || url.port().is_some() {
            return None;
        }
        let host = url.host_str()?;
        Host::ALL.into_iter().find(|h| h.name() == host)
    }
}

/// Which way a request reaches the origin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Direct,
    Relay,
}

impl Route {
    pub fn as_str(self) -> &'static str {
        match self {
            Route::Direct => "direct",
            Route::Relay => "relay",
        }
    }

    fn other(self) -> Route {
        match self {
            Route::Direct => Route::Relay,
            Route::Relay => Route::Direct,
        }
    }
}

/// Lets the first of a run of failures into the log, then one line per
/// [`LOG_EVERY`], and says when the run ends. A member's log stays readable
/// through a day-long outage and still records that it happened.
#[derive(Debug, Default)]
pub struct LogThrottle {
    last: Option<Instant>,
    held: u32,
    run: u32,
}

impl LogThrottle {
    pub const fn new() -> Self {
        LogThrottle { last: None, held: 0, run: 0 }
    }

    /// A failure happened. `Some(n)` means log it now, `n` being how many were
    /// held back since the last line; `None` means stay quiet.
    pub fn hit(&mut self, now: Instant, every: Duration) -> Option<u32> {
        self.run += 1;
        match self.last {
            Some(at) if now.saturating_duration_since(at) < every => {
                self.held += 1;
                None
            }
            _ => {
                self.last = Some(now);
                Some(std::mem::take(&mut self.held))
            }
        }
    }

    /// A success happened. `Some(n)` when it ends a run of `n` failures that
    /// reached the log, which is when a recovery line is due.
    pub fn recover(&mut self) -> Option<u32> {
        let run = std::mem::take(&mut self.run);
        let logged = self.last.take().is_some();
        self.held = 0;
        (logged && run > 0).then_some(run)
    }
}

#[derive(Debug)]
struct HostState {
    route: Route,
    /// When the host moved onto the relay.
    relayed_at: Option<Instant>,
    next_probe: Option<Instant>,
    probe_gap: Duration,
    probing: bool,
    /// Requests that reached the origin on neither route.
    down: LogThrottle,
}

impl HostState {
    const fn new() -> Self {
        HostState {
            route: Route::Direct,
            relayed_at: None,
            next_probe: None,
            probe_gap: FIRST_PROBE,
            probing: false,
            down: LogThrottle::new(),
        }
    }

    /// Direct did not connect but the relay did. True when this moves the host
    /// onto the relay, which is a line for the log.
    fn direct_unreachable(&mut self, now: Instant) -> bool {
        if self.route == Route::Relay {
            return false;
        }
        self.route = Route::Relay;
        self.relayed_at = Some(now);
        self.probe_gap = FIRST_PROBE;
        self.next_probe = Some(now + FIRST_PROBE);
        true
    }

    /// Direct answered. When that moves the host back off the relay, how long it
    /// was there.
    fn direct_ok(&mut self, now: Instant) -> Option<Duration> {
        self.next_probe = None;
        if self.route != Route::Relay {
            return None;
        }
        self.route = Route::Direct;
        self.probe_gap = FIRST_PROBE;
        let since = self.relayed_at.take().unwrap_or(now);
        Some(now.saturating_duration_since(since))
    }

    /// Claims the direct probe when one is due. At most one runs per host.
    fn claim_probe(&mut self, now: Instant) -> bool {
        if self.route != Route::Relay || self.probing {
            return false;
        }
        match self.next_probe {
            Some(at) if now >= at => {
                self.probing = true;
                true
            }
            _ => false,
        }
    }

    fn probe_done(&mut self) {
        self.probing = false;
    }

    /// The probe found direct still down: wait twice as long, up to the cap.
    fn probe_failed(&mut self, now: Instant) {
        self.probing = false;
        if self.route != Route::Relay {
            return;
        }
        self.probe_gap = (self.probe_gap * 2).min(MAX_PROBE_GAP);
        self.next_probe = Some(now + self.probe_gap);
    }
}

static STATE: Mutex<[HostState; 3]> =
    Mutex::new([HostState::new(), HostState::new(), HostState::new()]);

fn with_host<R>(host: Host, f: impl FnOnce(&mut HostState) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard[host as usize])
}

/// The route requests to `host` take right now. `direct` until a request has
/// shown the host cannot be reached that way.
pub fn route(host: Host) -> Route {
    with_host(host, |s| s.route)
}

/// The relay's base URL, built from the Supabase project URL the frontend is
/// built with (`VITE_SUPABASE_URL`, read at compile time by build.rs). A build
/// without it has no second route and behaves exactly as before.
fn relay_base() -> Option<&'static Url> {
    static BASE: Lazy<Option<Url>> = Lazy::new(|| relay_base_from(option_env!("VITE_SUPABASE_URL")));
    BASE.as_ref()
}

fn relay_base_from(project: Option<&str>) -> Option<Url> {
    let project = project?.trim().trim_end_matches('/');
    let parsed = Url::parse(project).ok()?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return None;
    }
    Url::parse(&format!("{project}/functions/v1/{RELAY_FUNCTION}/")).ok()
}

/// `https://streamnook.app/api/v1/user/sync?x=1` becomes
/// `<base>/streamnook.app/api/v1/user/sync?x=1`. None for a URL that is not on
/// one of our hosts.
fn relay_url(base: &Url, url: &Url) -> Option<Url> {
    let host = Host::of(url)?;
    let mut out = base.clone();
    out.set_path(&format!("{}{}{}", base.path(), host.name(), url.path()));
    out.set_query(url.query());
    Some(out)
}

/// Whether a failed attempt never reached the origin, so the other route may
/// carry the same request without running it twice. A GET or HEAD that timed
/// out may have reached it, and is safe to repeat anyway.
fn try_other_route(is_connect: bool, is_timeout: bool, method: &Method) -> bool {
    is_connect || (is_timeout && (*method == Method::GET || *method == Method::HEAD))
}

fn worth_other_route(e: &reqwest::Error, method: &Method) -> bool {
    try_other_route(e.is_connect(), e.is_timeout(), method)
}

/// A short description of a transport failure for the log: the innermost
/// cause, never the URL or any header.
pub fn reason(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        return "timed out".to_string();
    }
    let mut innermost = None;
    let mut next = std::error::Error::source(e);
    while let Some(cause) = next {
        innermost = Some(cause);
        next = cause.source();
    }
    match innermost {
        Some(cause) => cause.to_string(),
        None if e.is_connect() => "could not connect".to_string(),
        None => "request failed".to_string(),
    }
}

fn minutes(d: Duration) -> String {
    let m = d.as_secs() / 60;
    match m {
        0 => "under a minute".to_string(),
        1 => "1 minute".to_string(),
        n => format!("{n} minutes"),
    }
}

static API_CLIENT: Lazy<Client> = Lazy::new(|| {
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(API_TIMEOUT)
        .build()
        .expect("failed to build the StreamNook API client")
});

static DOWNLOAD_CLIENT: Lazy<Client> = Lazy::new(|| {
    Client::builder()
        .user_agent("StreamNook/1.0")
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .build()
        .expect("failed to build the StreamNook download client")
});

/// Sends a request to a StreamNook host by the route that works, moving to the
/// other one if the first never reached the origin. A request to any other
/// host is sent as it is.
pub async fn send(builder: RequestBuilder) -> reqwest::Result<Response> {
    send_on(&API_CLIENT, builder).await
}

/// [`send`] for a long transfer: no total timeout, a read timeout instead, so a
/// large file on a slow link is never cut off while it is still moving.
pub async fn send_download(builder: RequestBuilder) -> reqwest::Result<Response> {
    send_on(&DOWNLOAD_CLIENT, builder).await
}

/// The URLs one StreamNook resource can be read from, preferred route first,
/// for a client this module does not run (tauri-plugin-updater tries its
/// endpoints in order).
pub fn ordered_urls(url: &str) -> Vec<Url> {
    let Ok(direct) = Url::parse(url) else {
        return Vec::new();
    };
    let Some(host) = Host::of(&direct) else {
        return vec![direct];
    };
    let Some(relayed) = relay_base().and_then(|base| relay_url(base, &direct)) else {
        return vec![direct];
    };
    match route(host) {
        Route::Direct => vec![direct, relayed],
        Route::Relay => vec![relayed, direct],
    }
}

async fn attempt(client: &Client, base: &Url, route: Route, mut request: Request) -> reqwest::Result<Response> {
    if route == Route::Relay {
        if let Some(relayed) = relay_url(base, request.url()) {
            *request.url_mut() = relayed;
        }
    }
    client.execute(request).await
}

async fn send_on(client: &'static Client, builder: RequestBuilder) -> reqwest::Result<Response> {
    let (_, request) = builder.build_split();
    let request = request?;
    let Some(host) = Host::of(request.url()) else {
        return client.execute(request).await;
    };
    let method = request.method().clone();

    let Some(base) = relay_base() else {
        let result = client.execute(request).await;
        match &result {
            Ok(_) => recovered(host, Route::Direct),
            Err(e) if worth_other_route(e, &method) => {
                let held = with_host(host, |s| s.down.hit(Instant::now(), LOG_EVERY));
                if let Some(held) = held {
                    warn!(
                        "[SnTransport] {} is unreachable ({}) and this build has no relay route{}",
                        host.name(),
                        reason(e),
                        held_note(held)
                    );
                }
            }
            Err(_) => {}
        }
        return result;
    };

    let first = route(host);
    if first == Route::Relay {
        probe_if_due(host);
    }
    // A streamed body cannot be replayed; such a request gets one route only.
    let spare = request.try_clone();
    let first_err = match attempt(client, base, first, request).await {
        Ok(resp) => {
            recovered(host, first);
            return Ok(resp);
        }
        Err(e) => e,
    };
    if !worth_other_route(&first_err, &method) {
        return Err(first_err);
    }
    let Some(spare) = spare else {
        return Err(first_err);
    };

    let second = first.other();
    let second_result = attempt(client, base, second, spare).await;
    let now = Instant::now();
    match &second_result {
        Ok(_) => match second {
            Route::Relay => {
                let moved = with_host(host, |s| {
                    let moved = s.direct_unreachable(now);
                    s.down.recover();
                    moved
                });
                if moved {
                    warn!(
                        "[SnTransport] {} is unreachable directly ({}); sending through the relay",
                        host.name(),
                        reason(&first_err)
                    );
                }
            }
            Route::Direct => {
                let away = with_host(host, |s| {
                    s.down.recover();
                    s.direct_ok(now)
                });
                warn!(
                    "[SnTransport] the relay is unreachable ({}) but {} answers directly; leaving the relay{}",
                    reason(&first_err),
                    host.name(),
                    away.map(|d| format!(" after {}", minutes(d))).unwrap_or_default()
                );
            }
        },
        Err(second_err) => {
            let direct_err = if first == Route::Direct { &first_err } else { second_err };
            let relay_err = if first == Route::Relay { &first_err } else { second_err };
            // Direct never connecting while the relay at least connected still
            // means the relay is the route to keep.
            if first == Route::Direct && !second_err.is_connect() {
                with_host(host, |s| s.direct_unreachable(now));
            }
            let held = with_host(host, |s| s.down.hit(now, LOG_EVERY));
            if let Some(held) = held {
                warn!(
                    "[SnTransport] {} is unreachable on both routes (direct: {}; relay: {}){}",
                    host.name(),
                    reason(direct_err),
                    reason(relay_err),
                    held_note(held)
                );
            }
        }
    }
    // Both failed: the caller sees the error of the route it was meant to take.
    second_result.or(Err(first_err))
}

/// A request reached the origin on `route`. Ends any logged outage.
fn recovered(host: Host, route: Route) {
    if let Some(run) = with_host(host, |s| s.down.recover()) {
        warn!(
            "[SnTransport] {} is reachable again ({}) after {} failed request{}",
            host.name(),
            route.as_str(),
            run,
            if run == 1 { "" } else { "s" }
        );
    }
}

fn held_note(held: u32) -> String {
    if held == 0 {
        String::new()
    } else {
        format!(", {held} more since the last line")
    }
}

/// While a host is on the relay, checks direct again once its probe is due.
/// A HEAD on the host root: any HTTP answer means the address is reachable.
fn probe_if_due(host: Host) {
    if !with_host(host, |s| s.claim_probe(Instant::now())) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let result = API_CLIENT
            .head(format!("https://{}/", host.name()))
            .timeout(PROBE_TIMEOUT)
            .send()
            .await;
        let now = Instant::now();
        match result {
            Ok(_) => {
                let away = with_host(host, |s| {
                    s.probe_done();
                    s.direct_ok(now)
                });
                if let Some(away) = away {
                    warn!(
                        "[SnTransport] {} is reachable directly again; leaving the relay after {}",
                        host.name(),
                        minutes(away)
                    );
                }
            }
            Err(e) => {
                with_host(host, |s| s.probe_failed(now));
                debug!("[SnTransport] {} still unreachable directly: {}", host.name(), reason(&e));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn only_our_https_hosts_are_routed() {
        assert_eq!(Host::of(&url("https://streamnook.app/api/v1/user/sync")), Some(Host::Site));
        assert_eq!(Host::of(&url("https://cdn.streamnook.app/badges/a.webp")), Some(Host::Cdn));
        assert_eq!(Host::of(&url("https://modroom.streamnook.app/badges/latest.json")), Some(Host::Modroom));
        assert_eq!(Host::of(&url("http://streamnook.app/")), None);
        assert_eq!(Host::of(&url("https://streamnook.app:8443/")), None);
        assert_eq!(Host::of(&url("https://notify.streamnook.app/")), None);
        assert_eq!(Host::of(&url("https://streamnook.app.evil.test/")), None);
        assert_eq!(Host::of(&url("https://api.twitch.tv/helix")), None);
    }

    #[test]
    fn the_relay_base_comes_from_the_supabase_project_url() {
        let base = relay_base_from(Some("https://abc.supabase.co/")).unwrap();
        assert_eq!(base.as_str(), "https://abc.supabase.co/functions/v1/sn-relay/");
        assert!(relay_base_from(None).is_none());
        assert!(relay_base_from(Some("")).is_none());
        assert!(relay_base_from(Some("http://abc.supabase.co")).is_none());
        assert!(relay_base_from(Some("not a url")).is_none());
    }

    #[test]
    fn a_relayed_url_keeps_host_path_and_query() {
        let base = relay_base_from(Some("https://abc.supabase.co")).unwrap();
        assert_eq!(
            relay_url(&base, &url("https://streamnook.app/api/v1/user/sync")).unwrap().as_str(),
            "https://abc.supabase.co/functions/v1/sn-relay/streamnook.app/api/v1/user/sync"
        );
        assert_eq!(
            relay_url(&base, &url("https://streamnook.app/api/v1/cosmetics/resolve?keys=a%2Cb,c")).unwrap().as_str(),
            "https://abc.supabase.co/functions/v1/sn-relay/streamnook.app/api/v1/cosmetics/resolve?keys=a%2Cb,c"
        );
        assert_eq!(
            relay_url(&base, &url("https://cdn.streamnook.app/badges/M%C3%B6bius.webp?v=2")).unwrap().as_str(),
            "https://abc.supabase.co/functions/v1/sn-relay/cdn.streamnook.app/badges/M%C3%B6bius.webp?v=2"
        );
        assert!(relay_url(&base, &url("https://example.com/x")).is_none());
    }

    #[test]
    fn only_requests_that_never_reached_the_origin_change_route() {
        // A connect failure never reached it, whatever the method.
        assert!(try_other_route(true, false, &Method::POST));
        assert!(try_other_route(true, true, &Method::POST));
        assert!(try_other_route(true, false, &Method::GET));
        // A timeout after connecting may have run a write: only reads repeat.
        assert!(try_other_route(false, true, &Method::GET));
        assert!(try_other_route(false, true, &Method::HEAD));
        assert!(!try_other_route(false, true, &Method::POST));
        assert!(!try_other_route(false, true, &Method::PUT));
        assert!(!try_other_route(false, true, &Method::DELETE));
        // Anything else (a body that failed to decode, a redirect loop) did reach it.
        assert!(!try_other_route(false, false, &Method::GET));
    }

    #[test]
    fn the_route_moves_to_the_relay_and_back() {
        let t0 = Instant::now();
        let mut s = HostState::new();
        assert_eq!(s.route, Route::Direct);
        assert!(s.direct_unreachable(t0), "the first failure is a route change");
        assert!(!s.direct_unreachable(t0), "the second is not");
        assert_eq!(s.route, Route::Relay);
        assert_eq!(s.direct_ok(t0 + Duration::from_secs(600)), Some(Duration::from_secs(600)));
        assert_eq!(s.route, Route::Direct);
        assert_eq!(s.direct_ok(t0), None, "already direct: nothing to report");
    }

    #[test]
    fn probes_wait_their_turn_run_one_at_a_time_and_back_off() {
        let t0 = Instant::now();
        let mut s = HostState::new();
        assert!(!s.claim_probe(t0), "no probe while direct works");
        s.direct_unreachable(t0);
        assert!(!s.claim_probe(t0 + Duration::from_secs(60)), "not due yet");
        let due = t0 + FIRST_PROBE;
        assert!(s.claim_probe(due));
        assert!(!s.claim_probe(due), "one probe at a time");
        s.probe_failed(due);
        assert_eq!(s.probe_gap, FIRST_PROBE * 2);
        assert!(!s.claim_probe(due + FIRST_PROBE));
        assert!(s.claim_probe(due + FIRST_PROBE * 2));
        for _ in 0..10 {
            s.probe_failed(due);
        }
        assert_eq!(s.probe_gap, MAX_PROBE_GAP, "capped");
        s.probe_done();
        assert!(s.direct_ok(due).is_some());
        assert_eq!(s.probe_gap, FIRST_PROBE, "a later outage starts over");
        assert!(!s.claim_probe(due + MAX_PROBE_GAP), "back on direct: no probes");
    }

    #[test]
    fn the_throttle_logs_the_first_failure_then_every_interval_then_recovery() {
        let t0 = Instant::now();
        let every = Duration::from_secs(900);
        let mut t = LogThrottle::new();
        assert_eq!(t.recover(), None, "nothing to recover from");
        assert_eq!(t.hit(t0, every), Some(0), "first failure logs");
        assert_eq!(t.hit(t0 + Duration::from_secs(60), every), None);
        assert_eq!(t.hit(t0 + Duration::from_secs(120), every), None);
        assert_eq!(t.hit(t0 + every, every), Some(2), "next line counts the quiet ones");
        assert_eq!(t.hit(t0 + every + Duration::from_secs(1), every), None);
        assert_eq!(t.recover(), Some(5), "recovery names the whole run");
        assert_eq!(t.recover(), None, "once");
        assert_eq!(t.hit(t0 + every + Duration::from_secs(2), every), Some(0), "a new run logs at once");
    }

    #[test]
    fn the_updater_gets_the_preferred_route_first() {
        // This build may or may not carry a Supabase URL; both shapes are valid.
        let urls = ordered_urls("https://streamnook.app/api/v1/update/tauri");
        assert!(!urls.is_empty() && urls.len() <= 2);
        assert!(urls.iter().any(|u| u.as_str() == "https://streamnook.app/api/v1/update/tauri"));
        assert_eq!(ordered_urls("https://example.com/x").len(), 1);
        assert!(ordered_urls("not a url").is_empty());
    }

    /// The API client's timeouts without the system proxy, so a proxy on the
    /// test machine cannot answer for loopback.
    fn loopback_client() -> Client {
        Client::builder()
            .no_proxy()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(API_TIMEOUT)
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn a_refused_connection_counts_as_never_reaching_the_origin() {
        // Bind then drop a listener: the port is closed, so connecting is refused.
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let err = loopback_client()
            .post(format!("http://127.0.0.1:{port}/"))
            .send()
            .await
            .unwrap_err();
        assert!(worth_other_route(&err, &Method::POST), "refused: {err:?}");
        assert!(!reason(&err).contains("127.0.0.1"), "no URL in the log line");
    }

    #[tokio::test]
    async fn a_silent_server_is_a_timeout_that_only_reads_repeat() {
        // Accepts the connection and never answers.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let hold = tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((sock, _)) = listener.accept().await {
                held.push(sock);
            }
        });
        let err = loopback_client()
            .post(format!("http://127.0.0.1:{port}/"))
            .timeout(Duration::from_millis(200))
            .send()
            .await
            .unwrap_err();
        assert!(err.is_timeout() && !err.is_connect(), "{err:?}");
        assert!(!worth_other_route(&err, &Method::POST));
        assert!(worth_other_route(&err, &Method::GET));
        assert_eq!(reason(&err), "timed out");
        hold.abort();
    }
}
