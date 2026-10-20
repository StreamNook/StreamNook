//! The StreamNook mention watcher: reads the chat of every live channel that
//! opted-in users follow, and of every offline chat they hold a lease on (one
//! they opened lately, or that was live lately), live ones joined first, with
//! one anonymous Twitch connection pool, and hands each mention of or reply to
//! them to the notify Worker.
//!
//! Configuration, from the environment:
//! - `SN_NOTIFY_URL`: the Worker's origin, e.g. `https://notify.streamnook.app`.
//! - `SN_WATCHER_SECRET`: the shared HMAC secret (the Worker's `WATCHER_SECRET`).
//! - `SN_WATCHER_HEALTH`: the health file (default `/run/sn-mention-watcher/health`).
//! - `RUST_LOG`: log filter (default `info`).
//!
//! Flow: every 60 s the signed `GET /watcher/targets` (every page of it) gives
//! the watched set; the pool joins and parts to match it, live channels first; each PRIVMSG goes through the shared rule
//! (`sn_mention_rule`) for everyone watching that channel; matches go out in
//! signed batches (`POST /watcher/mentions`).

mod backoff;
mod bucket;
mod health;
mod irc;
mod matcher;
mod outbox;
mod parse;
mod sign;
mod targets;

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use tokio::sync::mpsc;

use crate::health::Health;
use crate::targets::LiveSet;

const POLL: Duration = Duration::from_secs(60);
const DEFAULT_HEALTH: &str = "/run/sn-mention-watcher/health";

struct Config {
    base: String,
    secret: String,
    health: PathBuf,
}

fn config() -> Result<Config, String> {
    let base = std::env::var("SN_NOTIFY_URL").map_err(|_| "SN_NOTIFY_URL is not set".to_string())?;
    let base = base.trim().trim_end_matches('/').to_string();
    if !base.starts_with("https://") && !base.starts_with("http://") {
        return Err(format!("SN_NOTIFY_URL must be an http(s) origin, not {base:?}"));
    }
    let secret = std::env::var("SN_WATCHER_SECRET").map_err(|_| "SN_WATCHER_SECRET is not set".to_string())?;
    let secret = secret.trim().to_string();
    if secret.len() < 32 {
        return Err("SN_WATCHER_SECRET is shorter than 32 characters".to_string());
    }
    let health = std::env::var("SN_WATCHER_HEALTH").unwrap_or_else(|_| DEFAULT_HEALTH.to_string());
    Ok(Config { base, secret, health: PathBuf::from(health) })
}

/// One signed targets page. The signature covers the path, never the query.
async fn fetch_page(client: &reqwest::Client, cfg: &Config, page: u32) -> Result<targets::Answer, String> {
    let ts = sign::now_secs();
    let sig = sign::signature(&cfg.secret, ts, "GET", targets::PATH, b"");
    let resp = client
        .get(format!("{}{}?page={page}", cfg.base, targets::PATH))
        .header(sign::TS_HEADER, ts.to_string())
        .header(sign::SIG_HEADER, sig)
        .send()
        .await
        .map_err(|e| format!("could not reach the Worker: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("the Worker answered {status} for page {page}"));
    }
    let bytes = resp.bytes().await.map_err(|e| format!("could not read page {page}: {e}"))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("unreadable page {page}: {e}"))
}

/// One signed targets read: every page, or nothing (a failed page keeps the
/// last whole set).
async fn fetch_targets(client: &reqwest::Client, cfg: &Config) -> Result<LiveSet, String> {
    let mut answers = Vec::new();
    let mut page = 0;
    loop {
        let answer = fetch_page(client, cfg, page).await?;
        let next = answer.next;
        answers.push(answer);
        match next {
            Some(n) if n > page && n < targets::MAX_PAGES => page = n,
            _ => break,
        }
    }
    Ok(LiveSet::from_answers(answers))
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(term) => term,
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
        tokio::select! {
            _ = term.recv() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_ansi(false)
        .init();

    let cfg = match config() {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!("not starting: {e}");
            std::process::exit(2);
        }
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("sn-mention-watcher/0.1")
        .build()
        .expect("the HTTP client builds");

    let health = Arc::new(Health::default());
    let live: Arc<RwLock<LiveSet>> = Arc::new(RwLock::new(LiveSet::default()));

    // Matches to the Worker.
    let (mention_tx, mention_rx) = mpsc::channel(outbox::CAP);
    let poster = outbox::Poster { client: client.clone(), base: cfg.base.clone(), secret: cfg.secret.clone() };
    let outbox_task = tokio::spawn(outbox::run(mention_rx, poster, health.clone()));

    // Chat lines to the rule.
    let (line_tx, mut line_rx) = mpsc::channel::<parse::Privmsg>(10_000);
    {
        let live = live.clone();
        let health = health.clone();
        tokio::spawn(async move {
            while let Some(msg) = line_rx.recv().await {
                let found = {
                    let set = live.read().unwrap_or_else(|p| p.into_inner());
                    matcher::matches(&msg, &set)
                };
                for m in found {
                    if mention_tx.try_send(m).is_err() {
                        health.drop_mentions(1);
                    }
                }
            }
        });
    }

    tokio::spawn(health::run(health.clone(), cfg.health.clone()));

    let mut pool = irc::Pool::new(line_tx, health.clone());
    // Whether the last read's channels were all joined, so "all joined" is said once.
    let mut caught_up = false;
    let mut tick = tokio::time::interval(POLL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tracing::info!("watching for mentions through {}", cfg.base);
    let stop = shutdown_signal();
    tokio::pin!(stop);
    loop {
        tokio::select! {
            _ = &mut stop => break,
            _ = tick.tick() => {}
        }
        match fetch_targets(&client, &cfg).await {
            Ok(set) => {
                health.targets_ok();
                health.users.store(set.users.len(), Ordering::Relaxed);
                let wanted = set.wanted();
                let live_count = set.live_count();
                health.wanted.store(wanted.len(), Ordering::Relaxed);
                health.wanted_live.store(live_count, Ordering::Relaxed);
                tracing::debug!("targets: {} channels ({} live), {} users", wanted.len(), live_count, set.users.len());
                *live.write().unwrap_or_else(|p| p.into_inner()) = set;
                pool.apply(&wanted);
                // Join progress, live channels first: said every read until done.
                let joined = health.channels.load(Ordering::Relaxed);
                if joined < wanted.len() {
                    tracing::info!("irc: joined {} of {} channels ({} live)", joined, wanted.len(), live_count);
                    caught_up = false;
                } else if !caught_up {
                    tracing::info!("irc: all {} channels joined ({} live)", wanted.len(), live_count);
                    caught_up = true;
                }
            }
            // The last good set stays: a Worker outage parts nothing.
            Err(e) => tracing::warn!("targets: {e}; keeping the last set"),
        }
    }
    tracing::info!("stopping");
    drop(pool);
    let _ = tokio::time::timeout(Duration::from_secs(5), outbox_task).await;
}
