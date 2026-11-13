//! A rule's Notify action: an in-app toast and sound in one window, and the
//! system notification where it belongs.
//!
//! The in-app part goes to every window that could play it (main, MultiChat
//! windows, chat overlays); the first one to claim the notification's id
//! plays it (the mention ping's pattern). With no such window, or when none
//! claims it within 3 s, the system part shows instead. The system part
//! (desktop only) also shows at once when the rule asks for it and the main
//! window is not focused on screen. Streamer mode shows nothing.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use super::activity::{EntryKind, NewEntry};
use super::{lock, Engine};

const MULTICHAT_PREFIX: &str = "multichat-";
const OVERLAY_PREFIX: &str = "overlay-";
/// Notifications waiting for a claim at most.
const UNCLAIMED_KEPT: usize = 16;

/// What a Notify action asks for.
#[derive(Clone, Debug, PartialEq)]
pub struct NotifyRequest {
    pub rule_id: Option<String>,
    pub title: String,
    pub body: String,
    pub toast: bool,
    /// The rule asks for the system notification (desktop).
    pub system: bool,
    pub sound: Option<String>,
    pub volume: f32,
    /// The chat it is about (a click opens its stream).
    pub channel: String,
    /// A click opens Settings > Automations instead (the breaker's notice).
    pub settings_link: bool,
}

/// `automations://notify`'s payload.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct NotifyPayload {
    pub id: u64,
    pub title: String,
    pub body: String,
    pub sound: Option<String>,
    pub volume: f32,
    pub toast: bool,
    pub channel: String,
}

/// Where one notification goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Route {
    Nothing,
    /// The system part only.
    System,
    /// The in-app part to the players; `system_now` also shows the system part at once.
    Players { system_now: bool },
}

/// Pure routing. `in_app` is whether the request has an in-app part (a
/// toast or a sound); `desktop` whether a system part exists at all.
pub(crate) fn route(streamer: bool, has_players: bool, in_app: bool, system: bool, main_focused: bool, desktop: bool) -> Route {
    if streamer {
        return Route::Nothing;
    }
    if !in_app || !has_players {
        return if desktop && (system || in_app) { Route::System } else { Route::Nothing };
    }
    Route::Players { system_now: desktop && system && !main_focused }
}

/// Every window that could play an in-app notification. Main in the tray is
/// left out while another window can play; with none open there is no
/// player (the system part shows instead).
pub(crate) fn players(labels: impl IntoIterator<Item = String>, main_in_tray: bool) -> Vec<String> {
    let mut out: Vec<String> = labels
        .into_iter()
        .filter(|l| l == "main" || l.starts_with(MULTICHAT_PREFIX) || l.starts_with(OVERLAY_PREFIX))
        .collect();
    if main_in_tray && out.iter().any(|l| l != "main") {
        out.retain(|l| l != "main");
    }
    out.sort();
    out
}

/// Notification ids waiting for a player to claim them.
pub struct Notifier {
    next: AtomicU64,
    unclaimed: Mutex<VecDeque<(u64, Instant)>>,
}

impl Default for Notifier {
    fn default() -> Self {
        Notifier::new()
    }
}

impl Notifier {
    pub fn new() -> Notifier {
        Notifier { next: AtomicU64::new(1), unclaimed: Mutex::new(VecDeque::new()) }
    }

    pub(crate) fn issue(&self, now: Instant, window: Duration) -> u64 {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut unclaimed = lock(&self.unclaimed);
        unclaimed.retain(|(_, at)| now.duration_since(*at) < window);
        while unclaimed.len() >= UNCLAIMED_KEPT {
            unclaimed.pop_front();
        }
        unclaimed.push_back((id, now));
        id
    }

    /// True for the first claim of `id` within the window; false otherwise.
    pub fn claim(&self, id: u64, window: Duration) -> bool {
        let mut unclaimed = lock(&self.unclaimed);
        let Some(at) = unclaimed.iter().position(|(i, _)| *i == id) else { return false };
        unclaimed.remove(at).is_some_and(|(_, sent)| sent.elapsed() < window)
    }
}

/// Show one notification.
pub(crate) async fn deliver(engine: &Arc<Engine>, request: NotifyRequest) {
    let streamer = engine.local.streamer_mode();
    let targets = if streamer { Vec::new() } else { engine.local.notify_targets() };
    let in_app = request.toast || request.sound.is_some();
    let focused = engine.local.main_focused_on_screen();
    let decided = route(streamer, !targets.is_empty(), in_app, request.system, focused, cfg!(desktop));
    let log = |kind: EntryKind, text: String| NewEntry {
        kind,
        rule_id: request.rule_id.clone().unwrap_or_default(),
        rule_name: request.rule_id.as_ref().and_then(|id| engine.compiled().rule(id).map(|r| r.rule.name.clone())).unwrap_or_default(),
        channel: if request.channel.is_empty() {
            String::new()
        } else {
            crate::services::chat_rules::channel_filter_key("twitch", &request.channel)
        },
        text,
    };
    match decided {
        Route::Nothing => {
            if streamer {
                engine.log(log(EntryKind::Skipped, "Skipped: streamer mode".into()));
            }
        }
        Route::System => {
            let id = engine.notifier.next.fetch_add(1, Ordering::Relaxed);
            engine.local.system_notice(&request, id);
            engine.log(log(EntryKind::Acted, format!("Notified you: {}", request.title)));
        }
        Route::Players { system_now } => {
            let window = engine.timings.notify_confirm;
            let id = engine.notifier.issue(Instant::now(), window);
            let payload = NotifyPayload {
                id,
                title: request.title.clone(),
                body: request.body.clone(),
                sound: request.sound.clone(),
                volume: request.volume,
                toast: request.toast,
                channel: request.channel.clone(),
            };
            engine.local.emit_notify(&targets, &payload);
            if system_now {
                engine.local.system_notice(&request, id);
            }
            engine.log(log(EntryKind::Acted, format!("Notified you: {}", request.title)));
            if cfg!(desktop) && !system_now {
                let task = engine.clone();
                engine.spawn(async move {
                    let engine = task;
                    tokio::time::sleep(window).await;
                    // Still unclaimed: no window played it.
                    if engine.notifier.claim(id, Duration::MAX) {
                        engine.local.system_notice(&request, id);
                    }
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::testkit::*;
    use super::*;

    fn request() -> NotifyRequest {
        NotifyRequest {
            rule_id: None,
            title: "Raid".into(),
            body: "Someone raided".into(),
            toast: true,
            system: false,
            sound: None,
            volume: 1.0,
            channel: "chan".into(),
            settings_link: false,
        }
    }

    #[test]
    fn routing_table() {
        assert_eq!(route(true, true, true, true, false, true), Route::Nothing);
        assert_eq!(route(false, false, true, false, false, true), Route::System);
        assert_eq!(route(false, true, true, false, false, true), Route::Players { system_now: false });
        assert_eq!(route(false, true, true, true, false, true), Route::Players { system_now: true });
        assert_eq!(route(false, true, true, true, true, true), Route::Players { system_now: false });
        assert_eq!(route(false, false, true, true, false, false), Route::Nothing, "phones have no system part");
    }

    #[test]
    fn players_leave_out_main_in_the_tray_while_another_window_can_play() {
        let labels = || ["main", "multichat-1", "settings", "overlay-chat"].map(String::from);
        assert_eq!(players(labels(), false), vec!["main", "multichat-1", "overlay-chat"]);
        assert_eq!(players(labels(), true), vec!["multichat-1", "overlay-chat"]);
        assert_eq!(players(["main".to_string()], true), vec!["main"]);
        assert!(players(["settings".to_string()], false).is_empty());
    }

    #[tokio::test]
    async fn no_player_shows_the_system_part_at_once() {
        let kit = Kit::new();
        kit.local.set_targets(vec![]);
        deliver(&kit.engine, request()).await;
        assert!(kit.local.notifications().is_empty());
        assert_eq!(kit.local.system_notices().len(), usize::from(cfg!(desktop)));
    }

    #[tokio::test]
    async fn an_unconfirmed_toast_falls_back_to_the_system_part() {
        let kit = Kit::new();
        kit.local.set_targets(vec!["main".into()]);
        deliver(&kit.engine, request()).await;
        assert_eq!(kit.local.notifications().len(), 1);
        assert!(kit.local.system_notices().is_empty());
        tokio::time::sleep(kit.engine.timings.notify_confirm + Duration::from_millis(100)).await;
        assert_eq!(kit.local.system_notices().len(), usize::from(cfg!(desktop)));
    }

    #[tokio::test]
    async fn a_confirmed_toast_shows_no_system_part() {
        let kit = Kit::new();
        kit.local.set_targets(vec!["main".into()]);
        deliver(&kit.engine, request()).await;
        let id = kit.local.notifications()[0].id;
        assert!(kit.engine.notifier.claim(id, kit.engine.timings.notify_confirm));
        assert!(!kit.engine.notifier.claim(id, kit.engine.timings.notify_confirm), "claimed once");
        tokio::time::sleep(kit.engine.timings.notify_confirm + Duration::from_millis(100)).await;
        assert!(kit.local.system_notices().is_empty());
    }

    #[tokio::test]
    async fn the_live_notification_setting_does_not_gate_it() {
        let kit = Kit::new();
        kit.local.set_targets(vec![]);
        kit.local.set_live_notifications_off();
        deliver(&kit.engine, NotifyRequest { system: true, ..request() }).await;
        assert_eq!(kit.local.system_notices().len(), usize::from(cfg!(desktop)));
    }

    #[tokio::test]
    async fn streamer_mode_skips_both_parts() {
        let kit = Kit::new();
        kit.local.set_streamer(true);
        kit.local.set_targets(vec!["main".into()]);
        deliver(&kit.engine, NotifyRequest { system: true, ..request() }).await;
        assert!(kit.local.notifications().is_empty());
        assert!(kit.local.system_notices().is_empty());
        assert!(kit.activity_texts().iter().any(|t| t == "Skipped: streamer mode"));
    }
}
