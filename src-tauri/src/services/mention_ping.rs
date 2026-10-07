//! The mention ping: whether a live chat message that mentions you plays your
//! mention sound.
//!
//! Rust already decides what a mention is (`chat_rules`), and it sees every
//! joined channel, docked and hidden ones included, so the decision lives here
//! too: the setting, streamer mode, a per-channel cooldown and the age of the
//! message. A message that passes becomes one small `chat-mention-ping` event
//! to the main window, which only plays the sound it names. Off, this is one
//! read of an empty slot per mention.

use std::collections::HashMap;
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use serde::Serialize;
use tauri::Emitter;

use crate::models::chat_layout::ChatMessage;
use crate::models::settings::Settings;

/// The main window's event that plays the mention sound.
pub const EVENT: &str = "chat-mention-ping";

/// One channel pings at most this often: a chat that keeps @-ing you should
/// not turn into a metronome.
const CHANNEL_COOLDOWN: Duration = Duration::from_secs(3);
/// And the whole app at most this often, so several channels at once still
/// read as one ping.
const GLOBAL_COOLDOWN: Duration = Duration::from_millis(800);
/// A message older than this when it arrives is replay, not news.
const MAX_AGE_MS: i64 = 5_000;

#[derive(Clone, Debug, PartialEq)]
struct PingConfig {
    sound: String,
    volume: u32,
    replies: bool,
}

#[derive(Clone, Debug, Serialize)]
struct PingPayload<'a> {
    channel: &'a str,
    sound: &'a str,
    /// Percent of the sound's own level.
    volume: u32,
}

static CONFIG: Lazy<RwLock<Option<PingConfig>>> = Lazy::new(|| RwLock::new(None));
static LAST: Lazy<Mutex<(Option<Instant>, HashMap<String, Instant>)>> =
    Lazy::new(|| Mutex::new((None, HashMap::new())));

fn config_from(settings: &Settings) -> Option<PingConfig> {
    let design = &settings.chat_design;
    let sound = design.mention_sound.as_deref().map(str::trim).unwrap_or("");
    if sound.is_empty() || sound == "none" {
        return None;
    }
    Some(PingConfig {
        sound: sound.to_string(),
        volume: design.mention_sound_volume.unwrap_or(100).min(200),
        replies: design.mention_sound_replies.unwrap_or(true),
    })
}

/// Re-read the mention sound settings. Called on load and on every save.
pub fn refresh(settings: &Settings) {
    let next = config_from(settings);
    if let Ok(mut g) = CONFIG.write() {
        if *g != next {
            *g = next;
            if g.is_none() {
                if let Ok(mut last) = LAST.lock() {
                    *last = (None, HashMap::new());
                }
            }
        }
    }
}

/// The message's server send time, when Twitch stamped one.
fn sent_at_ms(msg: &ChatMessage) -> Option<i64> {
    msg.tags.get("tmi-sent-ts").and_then(|v| v.parse::<i64>().ok())
}

/// Whether `msg`, just received live, plays the mention sound. Pure apart from
/// the clock and the cooldown table it is handed, so it can be tested.
fn should_ping(
    config: &PingConfig,
    mentioned: bool,
    reply_to_me: bool,
    channel: &str,
    sent_at: Option<i64>,
    now_ms: i64,
    now: Instant,
    last: &mut (Option<Instant>, HashMap<String, Instant>),
) -> bool {
    if !(mentioned || (reply_to_me && config.replies)) {
        return false;
    }
    if sent_at.is_some_and(|t| now_ms - t > MAX_AGE_MS) {
        return false;
    }
    if last.0.is_some_and(|t| now.duration_since(t) < GLOBAL_COOLDOWN) {
        return false;
    }
    if last
        .1
        .get(channel)
        .is_some_and(|t| now.duration_since(*t) < CHANNEL_COOLDOWN)
    {
        return false;
    }
    last.0 = Some(now);
    // The table only ever holds channels that pinged within the cooldown.
    last.1.retain(|_, t| now.duration_since(*t) < CHANNEL_COOLDOWN);
    last.1.insert(channel.to_string(), now);
    true
}

/// Called for each live message after the rule engine has judged it.
pub fn on_live_message(msg: &ChatMessage, mentioned: bool, reply_to_me: bool) {
    if !mentioned && !reply_to_me {
        return;
    }
    let Some(config) = CONFIG.read().ok().and_then(|g| g.clone()) else {
        return;
    };
    if crate::services::streamer_mode::StreamerMode::is_active() {
        return;
    }
    let channel = msg.channel.trim_start_matches('#').to_lowercase();
    let fire = match LAST.lock() {
        Ok(mut last) => should_ping(
            &config,
            mentioned,
            reply_to_me,
            &channel,
            sent_at_ms(msg),
            chrono::Utc::now().timestamp_millis(),
            Instant::now(),
            &mut last,
        ),
        Err(_) => false,
    };
    if !fire {
        return;
    }
    if let Some(app) = crate::services::providers::app_handle() {
        let _ = app.emit_to(
            "main",
            EVENT,
            PingPayload {
                channel: &channel,
                sound: &config.sound,
                volume: config.volume,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(replies: bool) -> PingConfig {
        PingConfig {
            sound: "boop".into(),
            volume: 100,
            replies,
        }
    }

    #[test]
    fn mention_pings_once_per_channel_cooldown() {
        let c = cfg(true);
        let mut last = (None, HashMap::new());
        let t0 = Instant::now();
        assert!(should_ping(&c, true, false, "a", None, 0, t0, &mut last));
        // Same channel inside its cooldown: quiet.
        let t1 = t0 + Duration::from_secs(1);
        assert!(!should_ping(&c, true, false, "a", None, 0, t1, &mut last));
        // Another channel past the global gap: pings.
        assert!(should_ping(&c, true, false, "b", None, 0, t1, &mut last));
        // The first channel again after its cooldown.
        let t2 = t0 + CHANNEL_COOLDOWN + Duration::from_millis(1);
        assert!(should_ping(&c, true, false, "a", None, 0, t2, &mut last));
    }

    #[test]
    fn global_gap_merges_simultaneous_channels() {
        let c = cfg(true);
        let mut last = (None, HashMap::new());
        let t0 = Instant::now();
        assert!(should_ping(&c, true, false, "a", None, 0, t0, &mut last));
        assert!(!should_ping(&c, true, false, "b", None, 0, t0 + Duration::from_millis(100), &mut last));
    }

    #[test]
    fn replies_follow_their_setting_and_old_messages_stay_quiet() {
        let mut last = (None, HashMap::new());
        let t0 = Instant::now();
        assert!(!should_ping(&cfg(false), false, true, "a", None, 0, t0, &mut last));
        assert!(should_ping(&cfg(true), false, true, "a", None, 0, t0, &mut last));
        let mut last = (None, HashMap::new());
        assert!(!should_ping(&cfg(true), true, false, "a", Some(0), MAX_AGE_MS + 1, t0, &mut last));
        assert!(should_ping(&cfg(true), true, false, "a", Some(0), MAX_AGE_MS, t0, &mut last));
    }

    #[test]
    fn no_sound_means_no_config() {
        let mut s = Settings::default();
        assert!(config_from(&s).is_none());
        s.chat_design.mention_sound = Some("  ".into());
        assert!(config_from(&s).is_none());
        s.chat_design.mention_sound = Some("file:abc".into());
        s.chat_design.mention_sound_volume = Some(500);
        let c = config_from(&s).unwrap();
        assert_eq!(c.volume, 200);
        assert!(c.replies);
    }
}
