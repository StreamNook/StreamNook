//! Escape a relay that is serving nothing but ads.
//!
//! Stripping handles a leaked ad seamlessly, with no reload and no gap. It
//! can't help when the relay's whole window is ads: there is no real segment
//! left to serve. That is the rare case this handles, by re-resolving the same
//! channel through a region we haven't tried yet and hot-swapping the relay's
//! upstream underneath the player.
//!
//! Guardrails carried over from 7.8.6: fire only on a debounced confirmation
//! (one unlucky poll is not an ad window), and hold a cooldown afterwards so a
//! slow connection can't be thrashed. Only a relayed stream ever arms this:
//! an entitled or direct stream has nowhere else to go.

use log::{error, info, warn};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;

use crate::services::twitch_auth_service::TwitchAuthService;

/// Consecutive all-ad polls before escalating.
const DEBOUNCE_POLLS: u32 = 2;
/// Minimum gap between two escalations.
const COOLDOWN: Duration = Duration::from_secs(30);
/// Hard ceiling on one escalation attempt. Everything inside it is network work
/// on a phone, and `TwitchAuthService::get_token` in particular waits on a
/// oneshot fed from the Android UI thread with no timeout of its own, so a
/// backgrounded webview could park an escalation forever and, without this,
/// leave `in_flight` set and escalation dead for the rest of the stream.
const ATTEMPT_BUDGET: Duration = Duration::from_secs(45);

/// Everything needed to re-resolve the stream now playing.
struct ActiveStream {
    channel: String,
    quality: String,
    /// The user's relay preference list (or the bundled pool).
    configured: Vec<String>,
    /// True when `configured` is the user's own override, in which case the
    /// bundled pool is not a legitimate fallback.
    user_supplied: bool,
    /// Bases already known to be serving ads for this stream.
    tried: Vec<String>,
    auth: TwitchAuthService,
    last_pivot: Option<Instant>,
    in_flight: bool,
}

static ACTIVE: Lazy<Mutex<Option<ActiveStream>>> = Lazy::new(|| Mutex::new(None));
static EMPTY_POLLS: AtomicU32 = AtomicU32::new(0);

/// Bumped every time the armed stream changes. An escalation captures it before
/// its first await and refuses to swap the upstream if it has moved, which is
/// what stops a slow re-resolve from landing on a channel the viewer switched
/// to in the meantime.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// The guarded state is fully reconstructible by the next `arm`, so a poisoned
/// lock is not worth killing playback over: `maybe_trigger` runs on every
/// playlist poll, and an `unwrap` here would panic the relay's request handler
/// once a second for the rest of the session.
fn active() -> MutexGuard<'static, Option<ActiveStream>> {
    ACTIVE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Arm escalation for a stream that resolved through `base`.
pub fn arm(
    channel: String,
    quality: String,
    configured: Vec<String>,
    user_supplied: bool,
    base: String,
    auth: TwitchAuthService,
) {
    GENERATION.fetch_add(1, Ordering::SeqCst);
    let previous = active().replace(ActiveStream {
        channel,
        quality,
        configured,
        user_supplied,
        tried: vec![base],
        auth,
        last_pivot: None,
        in_flight: false,
    });
    // Drop the old record's `TwitchAuthService` outside the critical section.
    drop(previous);
    EMPTY_POLLS.store(0, Ordering::Relaxed);
}

/// Disarm (stream stopped, or this stream isn't using a relay).
pub fn disarm() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
    let previous = active().take();
    drop(previous);
    EMPTY_POLLS.store(0, Ordering::Relaxed);
}

/// Reset the in-flight flag and start a cooldown, so a failed escalation
/// doesn't hammer re-resolution while ads keep leaking.
fn end_with_cooldown(generation: u64) {
    if let Some(a) = active().as_mut() {
        // Only if we are still the armed stream; otherwise this would put a
        // cooldown on someone else's channel.
        if GENERATION.load(Ordering::SeqCst) == generation {
            a.in_flight = false;
            a.last_pivot = Some(Instant::now());
        }
    }
    EMPTY_POLLS.store(0, Ordering::Relaxed);
}

/// Called once per served media playlist with "stripping left nothing playable".
/// Non-blocking: the escalation runs on its own task.
pub fn maybe_trigger(all_ads: bool) {
    if !all_ads {
        EMPTY_POLLS.store(0, Ordering::Relaxed);
        return;
    }
    if EMPTY_POLLS.fetch_add(1, Ordering::Relaxed) + 1 < DEBOUNCE_POLLS {
        return;
    }
    let go = {
        let mut guard = active();
        match guard.as_mut() {
            Some(a) if !a.in_flight && a.last_pivot.is_none_or(|t| t.elapsed() >= COOLDOWN) => {
                a.in_flight = true;
                true
            }
            _ => false,
        }
    };
    if go {
        let generation = GENERATION.load(Ordering::SeqCst);
        tokio::spawn(async move {
            // The budget is the backstop that guarantees `in_flight` is cleared
            // no matter how the attempt ends, including a hung await.
            if tokio::time::timeout(ATTEMPT_BUDGET, run(generation))
                .await
                .is_err()
            {
                warn!("[AdBypass] escalation timed out; standing down for the cooldown");
                end_with_cooldown(generation);
            }
        });
    }
}

/// True while `generation` is still the armed stream AND a live relay session is
/// serving it. Checked before the work and again before the swap.
fn still_current(generation: u64) -> bool {
    GENERATION.load(Ordering::SeqCst) == generation
        && crate::services::stream_server::solo_session_active()
}

async fn run(generation: u64) {
    if !still_current(generation) {
        return;
    }

    // Prefer regions this stream hasn't been served ads by. Once every base has
    // been tried, let the race pick anything again rather than giving up.
    let (channel, quality, preferred, avoid, user_supplied, auth) = {
        let guard = active();
        let Some(a) = guard.as_ref() else { return };
        let untried: Vec<String> = a
            .configured
            .iter()
            .filter(|b| !a.tried.contains(b))
            .cloned()
            .collect();
        let preferred = if untried.is_empty() {
            a.configured.clone()
        } else {
            untried
        };
        (
            a.channel.clone(),
            a.quality.clone(),
            preferred,
            a.tried.clone(),
            a.user_supplied,
            a.auth.clone(),
        )
    };

    // Re-fetch the viewer's own master too: its above-relay tiers are signed
    // URLs, so the ones from the original resolve are no use by now.
    let oauth = auth.get_token().await.ok();
    let core = crate::services::twitch_resolver::resolve_live(&channel, oauth.as_deref(), &quality)
        .await
        .ok();

    let Some((resolved, base)) = super::resolve_through_relay(
        &channel,
        &quality,
        core.as_ref(),
        &preferred,
        &avoid,
        user_supplied,
    )
    .await
    else {
        warn!("[AdBypass] {} found no clean relay to escalate to", channel);
        end_with_cooldown(generation);
        return;
    };

    // Last check before anything observable happens. The work above takes
    // seconds, which is long enough for the viewer to have opened a different
    // channel; swapping now would repoint their player at the old one.
    if !still_current(generation) {
        info!(
            "[AdBypass] {} escalation finished after the viewer moved on; discarding it",
            channel
        );
        return;
    }

    match crate::services::stream_server::swap_upstream(resolved.url.clone()).await {
        Ok(()) => {
            crate::services::auth_proxy::set_status(resolved.status.clone());
            {
                let mut guard = active();
                if let Some(a) = guard.as_mut() {
                    if GENERATION.load(Ordering::SeqCst) == generation {
                        if !a.tried.contains(&base) {
                            a.tried.push(base);
                        }
                        a.last_pivot = Some(Instant::now());
                        a.in_flight = false;
                    }
                }
            }
            EMPTY_POLLS.store(0, Ordering::Relaxed);
            info!(
                "[AdBypass] {} escalated to region {:?} after an all-ad window",
                channel, resolved.status.proxy_region
            );
        }
        Err(e) => {
            error!("[AdBypass] {} upstream swap failed: {}", channel, e);
            end_with_cooldown(generation);
        }
    }
}
