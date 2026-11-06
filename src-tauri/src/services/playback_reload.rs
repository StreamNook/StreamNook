//! Automatic live reloads, counted per channel.
//!
//! The player reloads a live stream on its own when it stalls (the frozen-
//! playhead watchdog) or gives up (the watch session's `Resume`). Each reload
//! fully resets the relay, so when the cause sits upstream the same thing
//! happens again: viewers saw the stream "keep refreshing" every half minute
//! until they stopped it themselves. This counts those reloads and, on the
//! second inside `WINDOW`, plays the channel through the stable whole-segment
//! path instead of the low-latency origin for `HOLD_OFF`, and holds HEVC and
//! AV1 off a Twitch channel for as long: those always take the low-latency
//! origin, so only H.264 reaches the stable path. Every automatic
//! reload is logged at warn level with its reason and count, so a user's log
//! shows the loop without Detailed log on.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;

use crate::services::stream_server::UpstreamProfile;

/// Reloads closer together than this are one bout of trouble.
const WINDOW: Duration = Duration::from_secs(5 * 60);
/// How long the low-latency origin stays off once reloads repeat.
const HOLD_OFF: Duration = Duration::from_secs(10 * 60);
/// The reload, within `WINDOW`, that turns low latency off.
const HOLD_OFF_AT: usize = 2;

static HISTORY: Lazy<Mutex<HashMap<String, VecDeque<Instant>>>> = Lazy::new(Default::default);

/// Record an automatic reload of `login` at `now`; returns how many fell
/// inside `WINDOW`, this one included.
fn record(history: &mut HashMap<String, VecDeque<Instant>>, login: &str, now: Instant) -> usize {
    let times = history.entry(login.to_ascii_lowercase()).or_default();
    while times.front().is_some_and(|t| now.duration_since(*t) > WINDOW) {
        times.pop_front();
    }
    times.push_back(now);
    times.len()
}

/// What one automatic reload decides.
#[derive(Debug, PartialEq, Eq)]
struct Decision {
    /// Reloads of the channel inside `WINDOW`, this one included.
    count: usize,
    /// The low-latency origin is held off.
    hold: bool,
    /// HEVC and AV1 are held off the channel too.
    hold_codecs: bool,
}

/// Decide one automatic reload of `login` at `now` from the raw state: the
/// channel the solo relay serves and its upstream profile. Only the Twitch
/// channel the relay serves holds codecs, so a provider stream with the same
/// login never touches the Twitch channel's selection.
fn decide(
    history: &mut HashMap<String, VecDeque<Instant>>,
    login: &str,
    now: Instant,
    solo: Option<&str>,
    profile: UpstreamProfile,
) -> Decision {
    let count = record(history, login, now);
    let hold = count >= HOLD_OFF_AT;
    let hold_codecs = hold
        && profile == UpstreamProfile::Twitch
        && solo.is_some_and(|c| c.eq_ignore_ascii_case(login));
    Decision { count, hold, hold_codecs }
}

/// The warn line for one automatic reload.
fn reload_message(login: &str, reason: &str, d: &Decision) -> String {
    let mut line = format!(
        "[Playback] automatic reload of {login} ({reason}), {} in the last {} min",
        d.count,
        WINDOW.as_secs() / 60
    );
    if d.hold {
        line.push_str(&format!(
            "; low latency off for {} min, playing the stable stream",
            HOLD_OFF.as_secs() / 60
        ));
    }
    if d.hold_codecs {
        line.push_str(&format!(
            "; HEVC and AV1 held off {login} for {} min",
            HOLD_OFF.as_secs() / 60
        ));
    }
    line
}

/// Note one automatic reload. Returns true when the low-latency origin is
/// now held off, so the reload about to happen uses the stable path.
pub fn note(login: &str, reason: &str) -> bool {
    let d = {
        let mut history = HISTORY.lock().unwrap_or_else(|e| e.into_inner());
        decide(
            &mut history,
            login,
            Instant::now(),
            crate::services::stream_server::solo_channel().as_deref(),
            crate::services::stream_server::upstream_profile(),
        )
    };
    if d.hold {
        crate::services::ll_origin::hold_off(HOLD_OFF);
    }
    if d.hold_codecs {
        crate::services::twitch_resolver::hold_codecs(login, HOLD_OFF);
    }
    log::warn!("{}", reload_message(login, reason, &d));
    d.hold
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reloads_count_inside_the_window_per_channel() {
        let mut h = HashMap::new();
        let t0 = Instant::now();
        assert_eq!(record(&mut h, "Caseoh_", t0), 1);
        assert_eq!(record(&mut h, "caseoh_", t0 + Duration::from_secs(40)), 2);
        assert_eq!(record(&mut h, "xqc", t0 + Duration::from_secs(41)), 1, "per channel");
        // Past the window the old ones no longer count.
        assert_eq!(record(&mut h, "caseoh_", t0 + WINDOW + Duration::from_secs(10)), 2);
        assert_eq!(record(&mut h, "caseoh_", t0 + WINDOW * 3), 1);
    }

    #[test]
    fn each_reload_decides_its_holds_from_the_raw_state() {
        let twitch = UpstreamProfile::Twitch;
        let t0 = Instant::now();
        let mut h = HashMap::new();
        let d = decide(&mut h, "Caseoh_", t0, Some("caseoh_"), twitch);
        assert_eq!(d, Decision { count: 1, hold: false, hold_codecs: false });
        let d = decide(&mut h, "caseoh_", t0 + Duration::from_secs(30), Some("caseoh_"), twitch);
        assert_eq!(d, Decision { count: 2, hold: true, hold_codecs: true });
        let d = decide(&mut h, "caseoh_", t0 + Duration::from_secs(60), Some("caseoh_"), twitch);
        assert_eq!(d, Decision { count: 3, hold: true, hold_codecs: true }, "every holding reload renews");

        let mut h = HashMap::new();
        decide(&mut h, "xqc", t0, Some("pokimane"), twitch);
        let d = decide(&mut h, "xqc", t0, Some("pokimane"), twitch);
        assert!(d.hold && !d.hold_codecs, "another channel on the relay");

        let mut h = HashMap::new();
        decide(&mut h, "xqc", t0, None, twitch);
        let d = decide(&mut h, "xqc", t0, None, twitch);
        assert!(d.hold && !d.hold_codecs, "no live channel on the relay");

        let mut h = HashMap::new();
        decide(&mut h, "xqc", t0, Some("xqc"), UpstreamProfile::GenericHls);
        let d = decide(&mut h, "xqc", t0, Some("xqc"), UpstreamProfile::GenericHls);
        assert!(d.hold && !d.hold_codecs, "a provider stream");
    }

    #[test]
    fn the_reload_message_names_each_hold() {
        let plain = Decision { count: 1, hold: false, hold_codecs: false };
        assert_eq!(
            reload_message("xqc", "stall", &plain),
            "[Playback] automatic reload of xqc (stall), 1 in the last 5 min"
        );
        let both = Decision { count: 2, hold: true, hold_codecs: true };
        assert_eq!(
            reload_message("xqc", "wedge", &both),
            "[Playback] automatic reload of xqc (wedge), 2 in the last 5 min; low latency off for 10 min, \
             playing the stable stream; HEVC and AV1 held off xqc for 10 min"
        );
    }

    /// `src` with CRLF normalized and cut at its first test module.
    fn production(src: &str) -> String {
        let src = src.replace("\r\n", "\n");
        match src.find("#[cfg(test)]") {
            Some(i) => src[..i].to_string(),
            None => src,
        }
    }

    /// The text of the function starting at `head`, to its closing brace at
    /// column 0.
    fn body_of<'a>(src: &'a str, head: &str) -> &'a str {
        let start = src.find(head).unwrap_or_else(|| panic!("{head} not found"));
        let end = src[start..].find("\n}\n").map_or(src.len(), |e| start + e);
        &src[start..end]
    }

    /// Whether every needle appears in order, each searched after the last.
    fn in_order(src: &str, needles: &[&str]) -> bool {
        let mut at = 0;
        for needle in needles {
            match src[at..].find(needle) {
                Some(i) => at += i + needle.len(),
                None => return false,
            }
        }
        true
    }

    #[test]
    fn note_passes_the_raw_reads_and_applies_both_holds() {
        let src = production(include_str!("playback_reload.rs"));
        let note = body_of(&src, "pub fn note(");
        assert!(
            in_order(
                note,
                &[
                    "decide(",
                    "crate::services::stream_server::solo_channel().as_deref()",
                    "crate::services::stream_server::upstream_profile()",
                    "if d.hold {",
                    "crate::services::ll_origin::hold_off(HOLD_OFF);",
                    "if d.hold_codecs {",
                    "crate::services::twitch_resolver::hold_codecs(login, HOLD_OFF);",
                ]
            ),
            "{note}"
        );
    }

    #[test]
    fn the_viewers_quality_pick_ends_the_codec_hold() {
        let src = production(include_str!("../commands/streaming.rs"));
        let change = body_of(&src, "pub async fn change_stream_quality(");
        assert!(in_order(change, &["tr::release_codec_hold(", "start_stream("]), "{change}");
    }

    #[test]
    fn a_held_tile_is_promoted_with_a_fresh_resolve() {
        // multi_nook.rs has test modules above this function: slice it, no cut.
        let src = include_str!("../commands/multi_nook.rs").replace("\r\n", "\n");
        let promote = body_of(&src, "pub async fn promote_multi_nook_tile(");
        assert!(
            in_order(
                promote,
                &["promotion.capped.is_some()", "tr::codec_held(", "return Ok(None);", "start_promoted_relay("]
            ),
            "{promote}"
        );
    }

    #[test]
    fn the_quality_menu_skips_session_failures_only() {
        let src = production(include_str!("twitch_resolver.rs"));
        let build = body_of(&src, "fn build(\n");
        assert!(build.contains("quality_names(&menu_variants(&master, &channel))"), "{build}");
        let menu = body_of(&src, "fn menu_variants(");
        assert!(menu.contains("failed_in_session(channel)") && !menu.contains("failed_for("), "{menu}");
    }
}
