//! The mention ping: whether a live chat message that mentions you plays your
//! mention sound.
//!
//! Rust already decides what a mention is (`chat_rules`), and it sees every
//! joined channel, docked and hidden ones included, so the decision lives here
//! too: the setting, streamer mode, a per-channel cooldown and the age of the
//! message. A message that passes becomes one small `chat-mention-ping` event
//! to every window that could play it; the first page to claim it
//! (`mention_ping_claim`) plays the sound it names, so it plays once. Off, this
//! is one read of an empty slot per mention.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use serde::Serialize;
use tauri::{Emitter, Manager};

use crate::models::chat_layout::ChatMessage;
use crate::models::settings::Settings;

/// The event that plays the mention sound, sent to every window that could play it.
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
struct Tone {
    sound: String,
    volume: u32,
}

/// What a mention and a reply to you each play; `None` is silent.
#[derive(Clone, Debug, PartialEq)]
struct PingConfig {
    mention: Option<Tone>,
    reply: Option<Tone>,
}

impl PingConfig {
    /// The tone `msg` plays: a reply to you plays the reply sound, else a
    /// mention plays the mention sound. A reply that also names you falls
    /// back to the mention sound when replies are silent.
    fn tone(&self, mentioned: bool, reply_to_me: bool) -> Option<&Tone> {
        let reply = if reply_to_me { self.reply.as_ref() } else { None };
        reply.or(if mentioned { self.mention.as_ref() } else { None })
    }
}

fn tone(sound: Option<&str>, volume: Option<u32>) -> Option<Tone> {
    let sound = sound.map(str::trim).unwrap_or("");
    if sound.is_empty() || sound == "none" {
        return None;
    }
    Some(Tone {
        sound: sound.to_string(),
        volume: volume.unwrap_or(100).min(200),
    })
}

#[derive(Clone, Debug, Serialize)]
struct PingPayload<'a> {
    /// What a page claims with `mention_ping_claim`.
    id: u64,
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
    let mention = tone(design.mention_sound.as_deref(), design.mention_sound_volume);
    let reply = match design.reply_sound.as_deref().map(str::trim) {
        Some(own) if !own.is_empty() => tone(Some(own), design.reply_sound_volume),
        // Follows the mention sound, unless the older switch turned it off.
        _ if design.mention_sound_replies.unwrap_or(true) => mention.clone(),
        _ => None,
    };
    if mention.is_none() && reply.is_none() {
        return None;
    }
    Some(PingConfig { mention, reply })
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

const MULTICHAT_PREFIX: &str = "multichat-";
const OVERLAY_PREFIX: &str = "overlay-";

/// A page claims a ping within this, or it is not news any more.
const CLAIM_WINDOW: Duration = Duration::from_secs(3);
/// At most this many pings wait for a claim; the cooldowns keep it near one.
const UNCLAIMED_KEPT: usize = 8;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
/// Pings sent and not claimed yet, oldest first: `(id, sent)`.
static UNCLAIMED: Lazy<Mutex<VecDeque<(u64, Instant)>>> = Lazy::new(|| Mutex::new(VecDeque::new()));

/// Every open window that could play the ping: main, each MultiChat window and
/// each chat overlay. Closing main while a popout is open destroys it to free
/// its memory, so any of them may be the one alive; all of them get the ping
/// and the first to claim it plays it. With none open it goes to main, the
/// iPhone core's only target. No window getters: they wait on the UI thread,
/// and this runs on the chat read loop.
fn players(labels: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out: Vec<String> = labels
        .into_iter()
        .filter(|l| l == "main" || l.starts_with(MULTICHAT_PREFIX) || l.starts_with(OVERLAY_PREFIX))
        .collect();
    if out.is_empty() {
        out.push("main".to_string());
    }
    out
}

/// Note a ping as sent and return its id.
fn issue(now: Instant, unclaimed: &mut VecDeque<(u64, Instant)>) -> u64 {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    unclaimed.retain(|(_, sent)| now.duration_since(*sent) < CLAIM_WINDOW);
    while unclaimed.len() >= UNCLAIMED_KEPT {
        unclaimed.pop_front();
    }
    unclaimed.push_back((id, now));
    id
}

/// True for the first claim of a ping still within `CLAIM_WINDOW`, false for
/// every other.
fn take(id: u64, now: Instant, unclaimed: &mut VecDeque<(u64, Instant)>) -> bool {
    let Some(at) = unclaimed.iter().position(|(i, _)| *i == id) else {
        return false;
    };
    unclaimed
        .remove(at)
        .is_some_and(|(_, sent)| now.duration_since(sent) < CLAIM_WINDOW)
}

/// A page heard ping `id`. True: this page plays it. False: another page
/// already does, or it is stale.
#[tauri::command]
pub fn mention_ping_claim(id: u64) -> bool {
    take(id, Instant::now(), &mut UNCLAIMED.lock().unwrap_or_else(|e| e.into_inner()))
}

/// The tone `msg`, just received live, plays, if any. Pure apart from the
/// clock and the cooldown table it is handed, so it can be tested.
fn should_ping<'c>(
    config: &'c PingConfig,
    mentioned: bool,
    reply_to_me: bool,
    channel: &str,
    sent_at: Option<i64>,
    now_ms: i64,
    now: Instant,
    last: &mut (Option<Instant>, HashMap<String, Instant>),
) -> Option<&'c Tone> {
    let tone = config.tone(mentioned, reply_to_me)?;
    if sent_at.is_some_and(|t| now_ms - t > MAX_AGE_MS) {
        return None;
    }
    if last.0.is_some_and(|t| now.duration_since(t) < GLOBAL_COOLDOWN) {
        return None;
    }
    if last
        .1
        .get(channel)
        .is_some_and(|t| now.duration_since(*t) < CHANNEL_COOLDOWN)
    {
        return None;
    }
    last.0 = Some(now);
    // The table only ever holds channels that pinged within the cooldown.
    last.1.retain(|_, t| now.duration_since(*t) < CHANNEL_COOLDOWN);
    last.1.insert(channel.to_string(), now);
    Some(tone)
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
    let tone = match LAST.lock() {
        Ok(mut last) => should_ping(
            &config,
            mentioned,
            reply_to_me,
            &channel,
            sent_at_ms(msg),
            chrono::Utc::now().timestamp_millis(),
            Instant::now(),
            &mut last,
        )
        .cloned(),
        Err(_) => None,
    };
    let Some(tone) = tone else {
        return;
    };
    if let Some(app) = crate::services::providers::app_handle() {
        let id = issue(Instant::now(), &mut UNCLAIMED.lock().unwrap_or_else(|e| e.into_inner()));
        let payload = PingPayload {
            id,
            channel: &channel,
            sound: &tone.sound,
            volume: tone.volume,
        };
        for target in players(app.webview_windows().into_keys()) {
            let _ = app.emit_to(target.as_str(), EVENT, &payload);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boop() -> Option<Tone> {
        Some(Tone { sound: "boop".into(), volume: 100 })
    }

    fn cfg(replies: bool) -> PingConfig {
        PingConfig {
            mention: boop(),
            reply: if replies { boop() } else { None },
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn fires(
        c: &PingConfig,
        mentioned: bool,
        reply_to_me: bool,
        channel: &str,
        sent_at: Option<i64>,
        now_ms: i64,
        now: Instant,
        last: &mut (Option<Instant>, HashMap<String, Instant>),
    ) -> bool {
        should_ping(c, mentioned, reply_to_me, channel, sent_at, now_ms, now, last).is_some()
    }

    #[test]
    fn mention_pings_once_per_channel_cooldown() {
        let c = cfg(true);
        let mut last = (None, HashMap::new());
        let t0 = Instant::now();
        assert!(fires(&c, true, false, "a", None, 0, t0, &mut last));
        // Same channel inside its cooldown: quiet.
        let t1 = t0 + Duration::from_secs(1);
        assert!(!fires(&c, true, false, "a", None, 0, t1, &mut last));
        // Another channel past the global gap: pings.
        assert!(fires(&c, true, false, "b", None, 0, t1, &mut last));
        // The first channel again after its cooldown.
        let t2 = t0 + CHANNEL_COOLDOWN + Duration::from_millis(1);
        assert!(fires(&c, true, false, "a", None, 0, t2, &mut last));
    }

    #[test]
    fn global_gap_merges_simultaneous_channels() {
        let c = cfg(true);
        let mut last = (None, HashMap::new());
        let t0 = Instant::now();
        assert!(fires(&c, true, false, "a", None, 0, t0, &mut last));
        assert!(!fires(&c, true, false, "b", None, 0, t0 + Duration::from_millis(100), &mut last));
    }

    #[test]
    fn replies_follow_their_setting_and_old_messages_stay_quiet() {
        let mut last = (None, HashMap::new());
        let t0 = Instant::now();
        assert!(!fires(&cfg(false), false, true, "a", None, 0, t0, &mut last));
        assert!(fires(&cfg(true), false, true, "a", None, 0, t0, &mut last));
        let mut last = (None, HashMap::new());
        assert!(!fires(&cfg(true), true, false, "a", Some(0), MAX_AGE_MS + 1, t0, &mut last));
        assert!(fires(&cfg(true), true, false, "a", Some(0), MAX_AGE_MS, t0, &mut last));
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
        assert_eq!(c.mention.as_ref().unwrap().volume, 200);
        assert_eq!(c.reply, c.mention, "replies follow the mention sound by default");
    }

    #[test]
    fn a_reply_sound_of_its_own_plays_for_replies_only() {
        let mut s = Settings::default();
        s.chat_design.reply_sound = Some("chime".into());
        s.chat_design.reply_sound_volume = Some(40);
        let c = config_from(&s).expect("a reply sound alone still pings");
        assert!(c.tone(true, false).is_none(), "no mention sound set");
        let t = c.tone(false, true).unwrap();
        assert_eq!((t.sound.as_str(), t.volume), ("chime", 40));

        s.chat_design.mention_sound = Some("boop".into());
        let c = config_from(&s).unwrap();
        assert_eq!(c.tone(true, false).unwrap().sound, "boop");
        assert_eq!(c.tone(true, true).unwrap().sound, "chime", "a reply plays the reply sound");
    }

    #[test]
    fn silent_replies_old_switch_and_none_both_hold() {
        let mut s = Settings::default();
        s.chat_design.mention_sound = Some("boop".into());
        s.chat_design.mention_sound_replies = Some(false);
        let c = config_from(&s).unwrap();
        assert!(c.reply.is_none(), "the older switch still silences replies");
        assert_eq!(
            c.tone(true, true).unwrap().sound,
            "boop",
            "a reply that names you still pings as a mention"
        );

        s.chat_design.mention_sound_replies = None;
        s.chat_design.reply_sound = Some("none".into());
        assert!(config_from(&s).unwrap().reply.is_none());
    }

    #[test]
    fn every_window_that_can_play_gets_it_and_none_means_main() {
        let open = ["main", "multichat-default", "overlay-a", "profile-x", "emote-palette", "multichat-w2"];
        let mut got = players(open.iter().map(|s| s.to_string()));
        got.sort();
        assert_eq!(got, ["main", "multichat-default", "multichat-w2", "overlay-a"]);
        assert_eq!(players(["profile-x".to_string()]), ["main"], "nothing that can play: main, as on the iPhone");
        assert_eq!(players(Vec::<String>::new()), ["main"]);
    }

    #[test]
    fn a_ping_is_claimed_once_and_only_while_fresh() {
        let mut q = VecDeque::new();
        let t0 = Instant::now();
        let a = issue(t0, &mut q);
        assert!(take(a, t0, &mut q), "the first page to claim plays it");
        assert!(!take(a, t0, &mut q), "every other page stays quiet");
        let b = issue(t0, &mut q);
        assert!(!take(b, t0 + CLAIM_WINDOW, &mut q), "a claim after the window is stale");
        assert!(!take(u64::MAX, t0, &mut q), "an id never sent");
    }

    #[test]
    fn unclaimed_pings_stay_bounded() {
        let mut q = VecDeque::new();
        let t0 = Instant::now();
        for _ in 0..UNCLAIMED_KEPT * 3 {
            issue(t0, &mut q);
        }
        assert_eq!(q.len(), UNCLAIMED_KEPT);
    }
}
