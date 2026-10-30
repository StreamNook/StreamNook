//! The one door every "streamer went live" announcement goes through.
//!
//! Four things raise one: the follow poller, the favourites sweep, the other
//! platforms' live watcher, and the settings Test button. Before this gate the
//! notification centre decided, per window, whether an event was a repeat (a
//! channel you both follow and favourite is announced by both watchers; a stream
//! that drops and comes straight back announces itself again) and whether it
//! was the same person going live on another platform. Rust now decides once:
//!
//! - a go-live dropped by `drop_reason` (its type switched off, the channel
//!   muted or hidden, its offline room open) is dropped here, before it can
//!   occupy the dedupe slot a later, enabled announcement for the same channel
//!   needs;
//! - the same channel is announced at most once per GO_LIVE_DEDUPE;
//! - the same platform-free channel on a second platform within
//!   CROSS_PLATFORM_MERGE joins the first entry (`merge_into`) instead of
//!   opening a new one.
//!
//! Test announcements skip all of it: the button fires every time it is pressed.

use crate::models::settings::{AppState, LiveNotificationSettings};
use crate::services::live_mute::{self, AlertChannel};
use crate::services::live_notification_service::LiveNotification;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use crate::rt::AppHandle;
use tauri::{Emitter, Manager};

pub const EVENT: &str = "streamer-went-live";

const GO_LIVE_DEDUPE_MS: i64 = 5 * 60_000;
/// Generous on purpose: a multicast rarely starts everywhere at once, and two
/// rows an hour apart are two events, not one.
const CROSS_PLATFORM_MERGE_MS: i64 = 10 * 60_000;

#[derive(Debug, Clone, Serialize)]
pub struct LiveAnnouncement {
    #[serde(flatten)]
    pub notification: LiveNotification,
    /// The notification-centre entry this announcement is (or opens).
    pub notification_id: String,
    /// Every platform this person is live on within the merge window.
    pub providers: Vec<String>,
    /// The channel on each of those platforms that has a stable identity,
    /// for "Turn off alerts" on a merged entry.
    pub channels: Vec<AlertChannel>,
    /// Set when this joins an entry already open for the same person.
    pub merge_into: Option<String>,
}

struct OpenEntry {
    id: String,
    providers: Vec<String>,
    channels: Vec<AlertChannel>,
    at: i64,
}

#[derive(Default)]
struct Gate {
    /// streamer_login (lowercase) -> last announced at.
    recent: HashMap<String, i64>,
    /// platform-free channel -> the entry standing for it.
    open: HashMap<String, OpenEntry>,
}

static GATE: Mutex<Option<Gate>> = Mutex::new(None);

/// What `admit` decides: the entry id, its platforms, their channels, and
/// the entry it merges into.
type Admitted = (String, Vec<String>, Vec<AlertChannel>, Option<String>);

/// Decide what one non-test announcement becomes, or None when it repeats one
/// made within the dedupe window. Sweeps both maps, so neither grows for the
/// life of the session.
fn admit(gate: &mut Gate, n: &LiveNotification, now: i64) -> Option<Admitted> {
    let login = n.streamer_login.to_lowercase();
    gate.recent.retain(|_, at| now - *at < GO_LIVE_DEDUPE_MS);
    if gate.recent.contains_key(&login) {
        return None;
    }
    gate.recent.insert(login, now);

    let parsed = crate::services::providers::key::parse_key(&n.streamer_login);
    let channel = parsed.channel.to_lowercase();
    gate.open.retain(|_, e| now - e.at < CROSS_PLATFORM_MERGE_MS);
    if let Some(entry) = gate.open.get_mut(&channel) {
        if !entry.providers.contains(&parsed.provider) {
            entry.providers.push(parsed.provider);
            if let Some(c) = &n.channel {
                entry.channels.push(c.clone());
            }
            entry.at = now;
            return Some((entry.id.clone(), entry.providers.clone(), entry.channels.clone(), Some(entry.id.clone())));
        }
        // The same person on the same platform again after the dedupe window:
        // a new event, a new entry.
    }
    let id = format!("live-{now}-{}", n.streamer_login);
    let channels: Vec<AlertChannel> = n.channel.clone().into_iter().collect();
    gate.open.insert(
        channel,
        OpenEntry { id: id.clone(), providers: vec![parsed.provider.clone()], channels: channels.clone(), at: now },
    );
    Some((id, vec![parsed.provider], channels, None))
}

/// Why a go-live raises nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropReason {
    /// The master switch, or this source's switch, is off.
    TypeOff,
    /// The channel is in `muted_live_channels`.
    Muted,
    /// The channel is hidden from follows.
    Hidden,
    /// Its offline room is open, which shows the go-live itself.
    Room,
}

/// The keys a go-live is muted by: what its emitter filled, or its
/// `streamer_login` when an emitter left them empty.
fn mute_keys_of(n: &LiveNotification) -> Vec<String> {
    if n.mute_keys.is_empty() {
        vec![n.streamer_login.trim().to_string()]
    } else {
        n.mute_keys.clone()
    }
}

/// Why `n` raises nothing, or `None` when it is announced. The only decision
/// `announce` makes; pure, so every rule is tested on raw rows.
pub fn drop_reason(
    ln: &LiveNotificationSettings,
    is_hidden: &dyn Fn(&[String]) -> bool,
    room: Option<&str>,
    n: &LiveNotification,
) -> Option<DropReason> {
    if n.is_test {
        return None;
    }
    let type_on = ln.enabled
        && if n.source.as_deref() == Some("favorite") {
            ln.show_favorite_live_notifications
        } else {
            ln.show_live_notifications
        };
    if !type_on {
        return Some(DropReason::TypeOff);
    }
    let keys = mute_keys_of(n);
    if live_mute::is_muted(&ln.muted_live_channels, &keys) {
        return Some(DropReason::Muted);
    }
    if is_hidden(&keys) {
        return Some(DropReason::Hidden);
    }
    if shown_by_room(n, room) {
        return Some(DropReason::Room);
    }
    None
}

/// Whether this is the channel whose offline room is open: the room itself
/// shows the go-live, so a notification would only repeat it.
fn shown_by_room(n: &LiveNotification, room: Option<&str>) -> bool {
    let parsed = crate::services::providers::key::parse_key(&n.streamer_login);
    parsed.provider == "twitch" && room.is_some_and(|r| parsed.channel.eq_ignore_ascii_case(r))
}

/// Announce that a streamer went live, unless it is a repeat or `drop_reason`
/// drops it.
pub fn announce(app: &AppHandle, notification: LiveNotification) {
    let now = chrono::Utc::now().timestamp_millis();
    let decided = if notification.is_test {
        let provider = crate::services::providers::key::parse_key(&notification.streamer_login).provider;
        Some((format!("live-test-{now}"), vec![provider], Vec::new(), None))
    } else {
        // Read before the settings lock: the room has its own.
        let room = crate::services::watch_session::offline_room_login();
        let reason = {
            let state = app.state::<AppState>();
            let settings = state.settings.lock().unwrap_or_else(|e| e.into_inner());
            drop_reason(
                &settings.live_notifications,
                &crate::services::hidden_channels::hides_notification_keys,
                room.as_deref(),
                &notification,
            )
        };
        if reason.is_some() {
            return;
        }
        let mut guard = GATE.lock().unwrap_or_else(|e| e.into_inner());
        admit(guard.get_or_insert_with(Gate::default), &notification, now)
    };
    let Some((notification_id, providers, channels, merge_into)) = decided else { return };
    let _ = app.emit(EVENT, LiveAnnouncement { notification, notification_id, providers, channels, merge_into });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::provider_stream::ProviderStream;
    use serde_json::json;
    use std::collections::HashSet;

    const UC: &str = "UCabcDEF1234567890123456";

    fn live(login: &str) -> LiveNotification {
        LiveNotification {
            streamer_name: login.into(),
            streamer_login: login.into(),
            streamer_avatar: None,
            game_name: None,
            game_image: None,
            stream_title: None,
            stream_url: String::new(),
            is_test: false,
            source: None,
            seventv_id: None,
            channel: None,
            mute_keys: Vec::new(),
        }
    }

    fn prow(provider: &str, key: &str, user_id: &str, login: &str) -> ProviderStream {
        serde_json::from_value(json!({
            "provider": provider, "key": key, "user_id": user_id, "user_login": login,
            "user_name": login, "is_live": true, "watch_url": ""
        }))
        .unwrap()
    }

    /// A go-live built the way the favourites sweep and the provider watcher
    /// build theirs.
    fn from_row(row: &ProviderStream, source: Option<&str>) -> LiveNotification {
        let (channel, mute_keys) = live_mute::row_alert(row);
        let mut n = live(&row.key);
        n.source = source.map(str::to_string);
        n.channel = channel;
        n.mute_keys = mute_keys;
        n
    }

    fn alerting() -> LiveNotificationSettings {
        let mut ln = LiveNotificationSettings::default();
        ln.enabled = true;
        ln.show_live_notifications = true;
        ln.show_favorite_live_notifications = true;
        ln.muted_live_channels = Vec::new();
        ln
    }

    fn never_hidden(_: &[String]) -> bool {
        false
    }

    fn decide(muted: &[&str], n: &LiveNotification) -> Option<DropReason> {
        let mut ln = alerting();
        ln.muted_live_channels = muted.iter().map(|s| s.to_string()).collect();
        drop_reason(&ln, &never_hidden, None, n)
    }

    #[test]
    fn the_open_offline_room_is_not_announced_again() {
        assert!(shown_by_room(&live("Shroud"), Some("shroud")));
        assert!(!shown_by_room(&live("shroud"), None));
        assert!(!shown_by_room(&live("xqc"), Some("shroud")));
        assert!(!shown_by_room(&live("kick:shroud"), Some("shroud")), "another platform is another channel");
    }

    #[test]
    fn a_repeat_within_the_window_is_dropped() {
        let mut g = Gate::default();
        assert!(admit(&mut g, &live("xqc"), 0).is_some());
        assert!(admit(&mut g, &live("XQC"), 60_000).is_none());
        assert!(admit(&mut g, &live("xqc"), GO_LIVE_DEDUPE_MS + 1).is_some());
    }

    #[test]
    fn the_same_person_on_another_platform_joins_the_open_entry() {
        let mut g = Gate::default();
        let (first, providers, _, merge) = admit(&mut g, &live("xqc"), 0).unwrap();
        assert_eq!((providers, merge), (vec!["twitch".to_string()], None));
        let (id, providers, _, merge) = admit(&mut g, &live("kick:xqc"), 30_000).unwrap();
        assert_eq!(id, first);
        assert_eq!(providers, vec!["twitch".to_string(), "kick".to_string()]);
        assert_eq!(merge, Some(first));
    }

    #[test]
    fn after_the_merge_window_it_is_a_new_entry() {
        let mut g = Gate::default();
        let (first, _, _, _) = admit(&mut g, &live("xqc"), 0).unwrap();
        let (second, providers, _, merge) = admit(&mut g, &live("kick:xqc"), CROSS_PLATFORM_MERGE_MS + 1).unwrap();
        assert_ne!(first, second);
        assert_eq!((providers, merge), (vec!["kick".to_string()], None));
    }

    #[test]
    fn a_merged_entry_carries_every_platform_channel() {
        let mut g = Gate::default();
        let mut a = live("xqc");
        a.channel = live_mute::twitch_alert("1", "xqc").0;
        let mut b = live("kick:xqc");
        b.channel = Some(AlertChannel { provider: "kick".into(), user_id: "9".into(), user_login: "xqc".into() });
        admit(&mut g, &a, 0).unwrap();
        let (_, _, channels, _) = admit(&mut g, &b, 1_000).unwrap();
        assert_eq!(channels.iter().map(|c| c.provider.as_str()).collect::<Vec<_>>(), vec!["twitch", "kick"]);
    }

    #[test]
    fn announcements_serialize_flat_for_the_listeners() {
        let a = LiveAnnouncement {
            notification: live("xqc"),
            notification_id: "live-1-xqc".into(),
            providers: vec!["twitch".into()],
            channels: Vec::new(),
            merge_into: None,
        };
        let v = serde_json::to_value(a).unwrap();
        assert_eq!(v["streamer_login"], "xqc");
        assert_eq!(v["notification_id"], "live-1-xqc");
        assert_eq!(v["channels"], json!([]));
    }

    #[test]
    fn the_channel_rides_along_and_the_mute_keys_stay_home() {
        let (channel, mute_keys) = live_mute::twitch_alert("71092938", "xQc");
        let mut n = live("xqc");
        n.channel = channel;
        n.mute_keys = mute_keys;
        let v = serde_json::to_value(&n).unwrap();
        assert_eq!(v["channel"]["user_login"], "xqc");
        assert_eq!(v["channel"]["provider"], "twitch");
        assert!(v.get("mute_keys").is_none());
    }

    #[test]
    fn a_muted_follow_is_dropped_by_its_login() {
        let (channel, mute_keys) = live_mute::twitch_alert("71092938", "xQc");
        let mut n = live("xQc");
        n.channel = channel;
        n.mute_keys = mute_keys;
        assert_eq!(decide(&["XQC"], &n), Some(DropReason::Muted));
        assert_eq!(decide(&[], &n), None);
        assert_eq!(decide(&["kick:xqc"], &n), None, "a Kick mute is not a Twitch mute");
    }

    #[test]
    fn every_row_shape_is_muted_by_its_own_keys() {
        let twitch_fav = from_row(&prow("twitch", "xqc", "71092938", "xqc"), Some("favorite"));
        assert_eq!(decide(&["71092938"], &twitch_fav), None, "a Twitch id is never a mute key");
        assert_eq!(decide(&["xqc"], &twitch_fav), Some(DropReason::Muted));

        let kick = from_row(&prow("kick", "kick:xqc", "12345", "xqc"), None);
        assert_eq!(decide(&["kick:XQC"], &kick), Some(DropReason::Muted));
        assert_eq!(decide(&["xqc"], &kick), None, "a Twitch mute is not a Kick mute");

        let yt_id = from_row(&prow("youtube", &format!("youtube:{UC}"), UC, UC), None);
        assert_eq!(decide(&["youtube:ucabcdef1234567890123456"], &yt_id), Some(DropReason::Muted));

        let yt_handle = from_row(&prow("youtube", "youtube:@handle", UC, "@handle"), Some("favorite"));
        assert_eq!(decide(&[format!("youtube:{UC}").as_str()], &yt_handle), Some(DropReason::Muted));
        assert_eq!(decide(&["youtube:@handle"], &yt_handle), Some(DropReason::Muted));

        let yt_video = from_row(&prow("youtube", "youtube:AGr94tpNVkw", "", "AGr94tpNVkw"), None);
        assert_eq!(decide(&["youtube:AGr94tpNVkw"], &yt_video), Some(DropReason::Muted));
        assert!(yt_video.channel.is_none(), "no stable identity, no Turn off alerts");

        let tiktok = from_row(&prow("tiktok", "tiktok:someone", "", "someone"), None);
        assert_eq!(decide(&["tiktok:someone"], &tiktok), Some(DropReason::Muted));
        assert_eq!(decide(&[], &tiktok), None);
    }

    #[test]
    fn the_test_button_fires_and_the_other_rules_hold() {
        let mut test = live("xqc");
        test.is_test = true;
        assert_eq!(decide(&["xqc"], &test), None, "Test always fires");

        let bare = live("kick:xqc");
        assert_eq!(decide(&["KICK:xqc"], &bare), Some(DropReason::Muted), "no keys: its login is the key");

        let mut favorites_off = alerting();
        favorites_off.show_favorite_live_notifications = false;
        let mut fav = live("xqc");
        fav.source = Some("favorite".into());
        assert_eq!(drop_reason(&favorites_off, &never_hidden, None, &fav), Some(DropReason::TypeOff));
        assert_eq!(drop_reason(&favorites_off, &never_hidden, None, &live("xqc")), None);

        assert_eq!(drop_reason(&alerting(), &never_hidden, Some("xqc"), &live("xqc")), Some(DropReason::Room));

        let hidden: HashSet<String> = [format!("youtube:{UC}")].into_iter().collect();
        let is_hidden = |keys: &[String]| crate::services::hidden_channels::hides(&hidden, keys);
        let yt_handle = from_row(&prow("youtube", "youtube:@Handle", UC, "@Handle"), None);
        assert_eq!(drop_reason(&alerting(), &is_hidden, None, &yt_handle), Some(DropReason::Hidden));
    }
}
