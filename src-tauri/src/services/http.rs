//! Shared HTTP clients.
//!
//! reqwest::Client owns a connection pool, TLS config, and DNS resolver. Each
//! `Client::new()` allocates a fresh one. Across StreamNook's ~80 general-API
//! call sites that's substantial wasted memory and per-call setup cost (TLS
//! handshakes don't get reused across short-lived clients).
//!
//! These shared instances are safe to clone across tasks (reqwest::Client is
//! Send + Sync internally Arc'd) and centralize timeout/UA policy. Per-call
//! configuration like Authorization headers stays on the RequestBuilder; the
//! client only holds connection-level config, so sharing doesn't leak auth.
//!
//! What does NOT belong here:
//! - Stream/HLS proxy clients (`stream_server`, `multi_nook_server`) — already
//!   shared via their own LazyLock, need Chrome UA + keepalive tuning.
//! - Browser-spoofing clients with custom UA strings (badge scrapers).
//! - Per-service instance fields with specific config (badge_service,
//!   emote_service, automation_service, profile_cache_service).

use reqwest::Client;
use std::sync::LazyLock;
use std::time::Duration;

/// Default client for general-purpose API requests. 30s timeout protects
/// against hung requests (default reqwest config has no timeout — a network
/// stall used to hang forever).
static CLIENT_DEFAULT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("Failed to build default HTTP client")
});

/// Client with redirects disabled. GitHub release endpoints respond with 302
/// plus a Location header pointing at the resolved release tag; we parse that
/// header directly rather than following.
static CLIENT_NO_REDIRECT: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("Failed to build no-redirect HTTP client")
});

/// reqwest's own defaults and nothing else: no overall timeout, no default
/// headers, the exact configuration `reqwest::Client::new()` and `reqwest::get`
/// produce. This is the drop-in for the sites that used to build one of those
/// per call: long downloads that must not be capped, callers that put their own
/// timeout on the request, and calls that were never bounded. Only the
/// connection pool and TLS setup are shared; no request changes shape. A site
/// that wants a user agent or a deadline sets it on the RequestBuilder.
static CLIENT_UNBOUNDED: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .build()
        .expect("Failed to build unbounded HTTP client")
});

pub fn client() -> &'static Client {
    &CLIENT_DEFAULT
}

pub fn client_no_redirect() -> &'static Client {
    &CLIENT_NO_REDIRECT
}

pub fn client_unbounded() -> &'static Client {
    &CLIENT_UNBOUNDED
}
