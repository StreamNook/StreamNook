//! The mention ping: whether a live chat message that mentions you plays your
//! mention sound.
//!
//! Rust already decides what a mention is (`chat_rules`), and it sees every
//! joined channel, docked and hidden ones included, so the decision lives here
//! too: the setting, streamer mode, a per-channel cooldown and the age of the
//! message. A message that passes becomes one small `chat-mention-ping` event
//! to the one window that plays it (`pick_player`), which only plays the sound
//! it names. Off, this is one read of an empty slot per mention.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use serde::Serialize;
use tauri::{Emitter, Manager};

use crate::models::chat_layout::ChatMessage;
use crate::models::settings::Settings;

/// The event that plays the mention sound, sent to one window.
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

/// Windows whose page is listening for the ping right now.
static READY: Lazy<Mutex<BTreeSet<String>>> = Lazy::new(|| Mutex::new(BTreeSet::new()));

/// A page says whether it is listening for the ping. A command rather than an
/// event: it is never queued, and the label is the calling window's own, so a
/// page can only speak for itself.
#[tauri::command]
pub fn mention_ping_ready(webview: crate::rt::Webview, ready: bool) {
    set_ready(webview.label(), ready);
}

fn set_ready(label: &str, ready: bool) {
    let mut set = READY.lock().unwrap_or_else(|e| e.into_inner());
    if ready {
        set.insert(label.to_string());
    } else {
        set.remove(label);
    }
}

/// The page in `label` is gone or starting over, and a page that goes never
/// says so itself: not listening until it says so again. Called when a window
/// is destroyed and when a page starts loading.
pub fn forget(label: &str) {
    set_ready(label, false);
}

/// The one window that plays the ping, so it never plays twice, chosen among
/// the windows whose page listens (in label order): main whenever it does, then
/// the first MultiChat window, then the first chat overlay. Closing main while
/// a popout is open destroys it to free its memory, which is when a popout
/// plays. With nobody listening it stays main, the iPhone core's only target.
fn pick_player<'a>(ready: impl IntoIterator<Item = &'a String>) -> String {
    let mut multichat = None;
    let mut overlay = None;
    for label in ready {
        if label == "main" {
            return label.clone();
        }
        if multichat.is_none() && label.starts_with(MULTICHAT_PREFIX) {
            multichat = Some(label);
        } else if overlay.is_none() && label.starts_with(OVERLAY_PREFIX) {
            overlay = Some(label);
        }
    }
    multichat.or(overlay).map_or_else(|| "main".to_string(), String::clone)
}

/// `pick_player` over the listening windows still open. A window gone without
/// saying so (a crash, a forced close) drops out here. No window getters: they
/// wait on the UI thread, and this runs on the chat read loop.
fn player(app: &crate::rt::AppHandle) -> String {
    let open = app.webview_windows();
    let mut ready = READY.lock().unwrap_or_else(|e| e.into_inner());
    ready.retain(|label| open.contains_key(label));
    pick_player(ready.iter())
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
        let target = player(&app);
        let _ = app.emit_to(
            target.as_str(),
            EVENT,
            PingPayload {
                channel: &channel,
                sound: &tone.sound,
                volume: tone.volume,
            },
        );
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

    fn labels(v: &[&str]) -> BTreeSet<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn pick_player_main_then_multichat_then_overlay() {
        let all = labels(&["main", "multichat-default", "overlay-a"]);
        assert_eq!(pick_player(&all), "main", "main plays whenever its page listens");
        let popouts = labels(&["overlay-a", "multichat-w2", "multichat-default"]);
        assert_eq!(pick_player(&popouts), "multichat-default", "MultiChat before an overlay, first by label");
        assert_eq!(pick_player(&labels(&["overlay-b", "overlay-a"])), "overlay-a");
        assert_eq!(pick_player(&labels(&["profile-x"])), "main", "never a profile or plugin window");
        assert_eq!(pick_player(&labels(&[])), "main", "nobody listening: main, as on the iPhone");
    }

    #[test]
    fn forget_drops_a_window_until_it_says_ready_again() {
        set_ready("multichat-test-forget", true);
        assert!(READY.lock().unwrap().contains("multichat-test-forget"));
        forget("multichat-test-forget");
        assert!(!READY.lock().unwrap().contains("multichat-test-forget"));
        set_ready("multichat-test-forget", true);
        assert!(READY.lock().unwrap().contains("multichat-test-forget"));
        forget("multichat-test-forget");
    }
}
