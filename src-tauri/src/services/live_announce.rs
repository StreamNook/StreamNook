//! The one door every "streamer went live" announcement goes through.
//!
//! Four things raise one: the follow poller, the favourites sweep, the other
//! platforms' live watcher, and the settings Test button. Before this gate the
//! notification centre decided, per window, whether an event was a repeat (a
//! channel you both follow and favourite is announced by both watchers; a stream
//! that drops and comes straight back announces itself again) and whether it
//! was the same person going live on another platform. Rust now decides once:
//!
//! - a disabled announcement type is dropped here, before it can occupy the
//!   dedupe slot a later, enabled announcement for the same channel needs;
//! - the same channel is announced at most once per GO_LIVE_DEDUPE;
//! - the same platform-free channel on a second platform within
//!   CROSS_PLATFORM_MERGE joins the first entry (`merge_into`) instead of
//!   opening a new one.
//!
//! Test announcements skip all three: the button fires every time it is pressed.

use crate::models::settings::AppState;
use crate::services::live_notification_service::LiveNotification;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

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
    /// Set when this joins an entry already open for the same person.
    pub merge_into: Option<String>,
}

struct OpenEntry {
    id: String,
    providers: Vec<String>,
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

/// Decide what one non-test announcement becomes, or None when it repeats one
/// made within the dedupe window. Sweeps both maps, so neither grows for the
/// life of the session.
fn admit(gate: &mut Gate, n: &LiveNotification, now: i64) -> Option<(String, Vec<String>, Option<String>)> {
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
            entry.at = now;
            return Some((entry.id.clone(), entry.providers.clone(), Some(entry.id.clone())));
        }
        // The same person on the same platform again after the dedupe window:
        // a new event, a new entry.
    }
    let id = format!("live-{now}-{}", n.streamer_login);
    gate.open.insert(channel, OpenEntry { id: id.clone(), providers: vec![parsed.provider.clone()], at: now });
    Some((id, vec![parsed.provider], None))
}

fn type_enabled(app: &AppHandle, n: &LiveNotification) -> bool {
    let state = app.state::<AppState>();
    let Ok(settings) = state.settings.lock() else { return true };
    let ln = &settings.live_notifications;
    ln.enabled
        && if n.source.as_deref() == Some("favorite") {
            ln.show_favorite_live_notifications
        } else {
            ln.show_live_notifications
        }
}

/// Announce that a streamer went live, unless it is a repeat or switched off.
pub fn announce(app: &AppHandle, notification: LiveNotification) {
    let now = chrono::Utc::now().timestamp_millis();
    let decided = if notification.is_test {
        let provider = crate::services::providers::key::parse_key(&notification.streamer_login).provider;
        Some((format!("live-test-{now}"), vec![provider], None))
    } else {
        if !type_enabled(app, &notification) {
            return;
        }
        let mut guard = GATE.lock().unwrap_or_else(|e| e.into_inner());
        admit(guard.get_or_insert_with(Gate::default), &notification, now)
    };
    let Some((notification_id, providers, merge_into)) = decided else { return };
    let _ = app.emit(EVENT, LiveAnnouncement { notification, notification_id, providers, merge_into });
}

#[cfg(test)]
mod tests {
    use super::*;

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
        }
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
        let (first, providers, merge) = admit(&mut g, &live("xqc"), 0).unwrap();
        assert_eq!((providers, merge), (vec!["twitch".to_string()], None));
        let (id, providers, merge) = admit(&mut g, &live("kick:xqc"), 30_000).unwrap();
        assert_eq!(id, first);
        assert_eq!(providers, vec!["twitch".to_string(), "kick".to_string()]);
        assert_eq!(merge, Some(first));
    }

    #[test]
    fn after_the_merge_window_it_is_a_new_entry() {
        let mut g = Gate::default();
        let (first, _, _) = admit(&mut g, &live("xqc"), 0).unwrap();
        let (second, providers, merge) = admit(&mut g, &live("kick:xqc"), CROSS_PLATFORM_MERGE_MS + 1).unwrap();
        assert_ne!(first, second);
        assert_eq!((providers, merge), (vec!["kick".to_string()], None));
    }

    #[test]
    fn announcements_serialize_flat_for_the_listeners() {
        let a = LiveAnnouncement {
            notification: live("xqc"),
            notification_id: "live-1-xqc".into(),
            providers: vec!["twitch".into()],
            merge_into: None,
        };
        let v = serde_json::to_value(a).unwrap();
        assert_eq!(v["streamer_login"], "xqc");
        assert_eq!(v["notification_id"], "live-1-xqc");
    }
}
