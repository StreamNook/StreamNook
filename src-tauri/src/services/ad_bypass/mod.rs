//! Ad-free live playback for Android.
//!
//! # Why this exists
//!
//! Ad handling shipped inside the core relay through v7.8.6. `99bd2e1` took it
//! out: the core became ad-neutral and resolution became a plugin hook
//! (`playback.resolve`). On desktop that works. On Android it can't: a plugin
//! is a spawned native child process talking stdio JSON-RPC, and the platform's
//! W^X sandbox forbids executing an installed binary, so the hook can never be
//! filled and the phone has had no ad path at all.
//!
//! This module brings 7.8.6's proven behavior forward for Android only. Desktop
//! keeps its ad-neutral core and its plugin model, unchanged: the whole module
//! is compiled for Android only, and the relay reaches it through gated lines
//! that do not exist in a desktop build.
//!
//! # Shape
//!
//! - `proxies`: the bundled relay pool and the user's override.
//! - `master`: race the pool, validate the answer, splice the above-1080p
//!   tiers back in from the viewer's own authenticated master.
//! - `filter`: strip ad segments out of every served media playlist.
//! - `pivot`: when a whole window is ads, re-resolve through another region.
//!
//! # The two seams it occupies
//!
//! Resolve happens once per stream, in `commands::streaming`, in the same place
//! a resolution-owning plugin would be asked. Serving happens on every playlist
//! poll, in `stream_server`, strictly before `hls_projection::stabilize` (which
//! records segment URLs, so feeding it ad segments would poison it).
//!
//! # Low latency
//!
//! The LL-HLS origin synthesizes its own playlist and serves it before the
//! relay's filter seam is ever reached, and it activates on CMAF (HEVC/AV1)
//! regardless of the experimental setting. The two cannot both own the
//! playlist, so `LlOrigin::start` returns inactive while `active()` is true.

// Android builds only. The unit tests run against the Android target too (built
// with `cargo test --target aarch64-linux-android` and executed on the device),
// so there is no reason to let this compile into a desktop test build and every
// reason not to: a desktop `cargo test` is the one place a supposedly frozen
// build could still break.
#![cfg(target_os = "android")]

pub mod filter;
pub mod master;
pub mod pivot;
pub mod proxies;

use log::{debug, info};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::models::settings::VideoPlayerSettings;
use crate::services::twitch_auth_service::TwitchAuthService;
use crate::services::twitch_resolver::{self as tr, ResolvedLive};
use master::sorted_heights;

/// The user's setting, mirrored where the relay can read it. The relay's
/// playlist handler runs far from any Tauri `State`, so the authoritative value
/// is copied here on every live resolve (which is also the only moment it can
/// change what happens to a stream).
static ENABLED: AtomicBool = AtomicBool::new(true);

/// True while the stream now playing was resolved through a relay. Distinct
/// from `ENABLED`: an entitled viewer (Turbo or a channel sub) is already
/// ad-free through Twitch itself and never routes through a relay, and a stream
/// whose relays all failed fell back to the direct master.
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Whether ad-free playback is switched on.
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Whether the stream now playing is being served through a relay. This is what
/// makes ad-free playback and the LL-HLS origin mutually exclusive.
///
/// Only live starts touch it, so a VOD or clip opened after a relayed live
/// stream reads the stale `true`. That is harmless: a VOD carries no
/// `#EXT-X-TWITCH-PREFETCH` hints, so the origin declines it either way.
pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// Resolve a live channel through a relay, ad-free.
///
/// Mirrors the `playback.resolve` plugin contract exactly, because it sits in
/// the same seam: `core` is the core resolver's own result, and returning
/// `None` means "the core resolution serves". It declines in four cases: the
/// feature is off, the viewer is entitled (already ad-free, and routing them
/// through a relay would cost them their above-1080p tiers), every relay
/// failed, or the relay's master turned out to be unusable. An ad-bearing
/// stream beats no stream, which is 7.8.6's rule and Streamlink's before it.
pub async fn resolve_master(
    channel: &str,
    quality: &str,
    core: Option<&ResolvedLive>,
    settings: &VideoPlayerSettings,
    auth: TwitchAuthService,
) -> Option<ResolvedLive> {
    // What Twitch offered this viewer and what it held back, logged on EVERY
    // live resolve including the entitled one. This is the only spot on a
    // shipped build's log that has the viewer's own master in hand: the
    // region-unlock path inside `fetch_auth_master` reports failure at `debug!`
    // and success not at all, so without this a missing 1440p tier is
    // indistinguishable from an unlock that ran and failed.
    if let Some(c) = core {
        info!(
            "[AdBypass] {} viewer ladder={:?} withheld={:?} (mode={}, entitled={})",
            channel,
            sorted_heights(&c.master),
            master::withheld_reasons(&c.master),
            c.status.mode,
            c.status.entitled
        );
    }

    ENABLED.store(settings.ad_bypass_enabled, Ordering::Relaxed);
    if !settings.ad_bypass_enabled {
        stand_down();
        return None;
    }
    if core.map(|r| r.status.entitled).unwrap_or(false) {
        debug!("[AdBypass] {} is entitled; no relay needed", channel);
        stand_down();
        return None;
    }

    let (configured, user_supplied) = configured_bases(settings);
    let resolved =
        resolve_through_relay(channel, quality, core, &configured, &[], !user_supplied).await;
    let Some((resolved, base)) = resolved else {
        stand_down();
        return None;
    };

    info!(
        "[AdBypass] {} resolved ad-free through {} (region={:?})",
        channel, base, resolved.status.proxy_region
    );
    ACTIVE.store(true, Ordering::Relaxed);
    pivot::arm(
        channel.to_string(),
        quality.to_string(),
        configured,
        user_supplied,
        base,
        auth,
    );
    Some(resolved)
}

/// The relay bases to prefer, and whether they came from the user rather than
/// the bundled pool (which decides whether falling back to the pool is allowed).
fn configured_bases(settings: &VideoPlayerSettings) -> (Vec<String>, bool) {
    let custom = proxies::parse_bases(&settings.ad_bypass_proxies);
    if custom.is_empty() {
        (proxies::bundled(), false)
    } else {
        (custom, true)
    }
}

/// Race `preferred` (falling back to the rest of the bundled pool), graft the
/// viewer's above-1080p tiers onto the winner, and select the requested
/// quality. Returns the resolution and the winning base. Shared by the initial
/// resolve and the pivot's re-resolve.
pub(crate) async fn resolve_through_relay(
    channel: &str,
    quality: &str,
    core: Option<&ResolvedLive>,
    preferred: &[String],
    avoid: &[String],
    allow_bundled_fallback: bool,
) -> Option<(ResolvedLive, String)> {
    let (base, relay_master) =
        match master::fetch_with_fallback(channel, preferred, avoid, allow_bundled_fallback).await {
            Ok(win) => win,
            Err(e) => {
                debug!("[AdBypass] {} no relay answered ({})", channel, e);
                return None;
            }
        };

    // The core result already holds the viewer's authenticated master, so the
    // splice costs nothing extra. Logged out (or a failed core resolve) simply
    // means there are no tiers to graft.
    //
    // The relay's own ladder, to sit beside the viewer ladder logged at resolve
    // time. Together they say whether a missing tier was never in the viewer's
    // master or was there and did not survive the splice.
    info!(
        "[AdBypass] {} relay ladder={:?} via {}",
        channel,
        sorted_heights(&relay_master),
        base
    );
    let full_master = match core {
        Some(c) => master::splice(&relay_master, &c.master),
        None => relay_master,
    };
    let region = proxies::region_for_base(&base);

    match tr::resolve_from_master(channel, full_master, quality, Some(base.clone()), region) {
        Ok(mut resolved) => {
            // `resolve_from_master` stamps the plugin provenance; this is the
            // in-core Android path, so say so for the UI's playback badge.
            resolved.status.mode = "proxy".to_string();
            Some((resolved, base))
        }
        Err(e) => {
            debug!("[AdBypass] {} relay master unusable ({})", channel, e);
            None
        }
    }
}

/// Give the stream back to the normal path: no relay in use, no pivot armed.
fn stand_down() {
    ACTIVE.store(false, Ordering::Relaxed);
    pivot::disarm();
}

/// Strip leaked ad segments out of a served media playlist, and escalate to a
/// different relay region when stripping leaves nothing playable.
///
/// Returns the playlist to serve: the original text byte-for-byte whenever the
/// feature is off or nothing was dropped, so an ad-free poll is untouched.
/// Filtering runs whether or not a relay is in use, because a leaked ad on an
/// entitled stream is worth removing too. The escalation only fires for a relayed
/// stream, since that is the only one with somewhere else to go.
pub fn filter_and_escalate(playlist: &str) -> String {
    // Only a media playlist is in scope. The relay routes every non-`.ts` body
    // through here, which includes CMAF segments that happen to decode as UTF-8
    // and any error page a CDN answers 200 with. Letting those through would
    // also let them reset the escalation debounce, which is counted in polls.
    if !enabled() || !playlist.contains("#EXTINF:") {
        return playlist.to_string();
    }
    let (filtered, outcome) = filter::filter_ad_segments(playlist);
    // Escalation is only meaningful for a stream that is actually on a relay;
    // an entitled or direct stream has nowhere else to go.
    if active() {
        pivot::maybe_trigger(outcome.all_ads());
    }
    if outcome.dropped == 0 {
        return playlist.to_string();
    }
    debug!(
        "[AdBypass] stripped {} ad segment(s); {} real remain",
        outcome.dropped, outcome.real
    );
    filtered
}
