//! The engine's seam to the rest of the app: settings, the shared chat
//! connection, liveness, notes, notifications and window events.
//!
//! Nothing here reaches Twitch's send paths or streamnook.app (that is
//! `outward`). The seam exists so every engine test runs against a fake with
//! no IRC connection, no window and no settings file.

use async_trait::async_trait;
use serde::Serialize;

use crate::models::automations::{AutomationPrefs, Automations};
use crate::rt::AppHandle;
use crate::services::channel_state::ChannelState;

use super::activity::Entry;
use super::badges::Probe;
use std::collections::BTreeSet;

use super::notify::{NotifyPayload, NotifyRequest};
use super::own_topics::{OwnState, TopicKind};
use super::speak::SpeakPayload;
use super::StatusSnapshot;

/// The account store's account-change guard, as a seam of its own: the
/// removal hook needs it even when no engine (and so no `Local`) runs.
#[async_trait]
pub trait AccountChanges: Send + Sync {
    /// Waits for linked-account token refreshes already running, and blocks
    /// new ones until the answer is dropped.
    async fn begin_account_change(&self) -> AccountChange;
}

/// An account-change guard, held by a `LeavingGuard` until the door returns.
pub type AccountChange = Box<dyn std::any::Any + Send>;

/// The real account-change seam. It needs no app, so the removal hook uses it
/// when no engine runs.
pub struct StoreChanges;

#[async_trait]
impl AccountChanges for StoreChanges {
    async fn begin_account_change(&self) -> AccountChange {
        Box::new(crate::services::account_store::AccountStore::begin_account_change().await)
    }
}

#[async_trait]
pub trait Local: AccountChanges {
    /// The automations group and the preferences, as the live settings hold them.
    fn settings(&self) -> (Automations, AutomationPrefs);
    /// The Reminders list the status carries (P5). Empty by default.
    fn reminders(&self) -> Vec<crate::services::reminder_service::convert::ReminderSummary> {
        Vec::new()
    }
    /// A reminder (or a rule with `only_while_open`) may post in `login`'s chat: some label other
    /// than the engine's holds it, or the stream warm-up joined it. The default is the first half
    /// (what a fake has); the real one adds the warm-up.
    async fn open_for_reminders(&self, login: &str) -> bool {
        self.holders(login).await.iter().any(|h| h != super::AUTOMATIONS_LABEL)
    }
    /// The Twitch rooms `label` holds.
    async fn chats_held_by(&self, label: &str) -> Vec<String>;
    /// Every label holding `channel`.
    async fn holders(&self, channel: &str) -> Vec<String>;
    /// The shared IRC connection is up.
    async fn chat_connected(&self) -> bool;
    /// Chat recovery is rebuilding the connection right now.
    fn recovery_cycling(&self) -> bool;
    /// Start the shared connection with `first` claimed under the engine's
    /// label (`claim` and `reattach` both true).
    async fn chat_start(&self, first: &str) -> Result<(), String>;
    /// Claim and join `channel` under the engine's label.
    async fn chat_join(&self, channel: &str) -> Result<(), String>;
    /// Let go of the engine's claim on `channel`.
    async fn chat_leave(&self, channel: &str);
    /// Claim and open `channel` of another platform (`kick`, `youtube`,
    /// `tiktok`) under the engine's label.
    async fn provider_join(&self, provider: &str, channel: &str) -> Result<(), String>;
    /// Let go of the engine's claim on a channel of another platform.
    async fn provider_leave(&self, provider: &str, channel: &str);
    /// The channels `label` holds on another platform, as its adapter
    /// stores them (lowercase).
    async fn provider_held_by(&self, provider: &str, label: &str) -> Vec<String>;
    /// Whether a label other than `label` holds `channel` on that platform.
    async fn provider_held_by_other_than(&self, provider: &str, channel: &str, label: &str) -> bool;
    /// Hold a live-only liveness watch on `login`.
    async fn watch_live(&self, login: &str, room_id: &str);
    async fn unwatch_live(&self, login: &str);
    /// Whether `login` is live (None: not known, read as not live).
    async fn is_live(&self, login: &str) -> Option<bool>;
    /// Title, game and uptime for `{title}`, `{game}` and `{uptime}`.
    async fn channel_info(&self, login: &str) -> Option<ChannelState>;
    /// The login of this device's current Twitch watch.
    fn current_watch(&self) -> Option<String>;
    /// A chat line only you see.
    async fn room_notice(&self, login: &str, text: &str);
    fn streamer_mode(&self) -> bool;
    /// The windows that could play a notification (none: nobody can).
    fn notify_targets(&self) -> Vec<String>;
    /// The main window is on screen and focused.
    fn main_focused_on_screen(&self) -> bool;
    fn emit_notify(&self, targets: &[String], payload: &NotifyPayload);
    /// The system notification (desktop only; elsewhere nothing).
    fn system_notice(&self, request: &NotifyRequest, id: u64);
    fn emit_activity(&self, watchers: &[String], entries: &[Entry]);
    fn emit_status(&self, snapshot: &StatusSnapshot);
    /// One check of a badge link: GET with no redirect followed and a 5 s
    /// timeout (the caller has already checked the link with `check_url`).
    async fn probe_link(&self, url: &str) -> Probe;
    /// Refresh the badge gallery from Twitch.
    async fn refresh_gallery(&self) -> Result<(), String>;
    /// Store run = Off (the breaker).
    async fn turn_off(&self);
    /// What USERSTATE last said `as_login` is in `channel` (`chat_send_limiter::known_role`).
    fn own_role(&self, as_login: &str, channel: &str) -> Option<crate::services::chat_send_limiter::Role>;
    /// People the whisper store says `owner` has whispered with (`whisper_store::knows`), for the To picker.
    fn whisper_people(&self, owner_id: &str) -> Vec<WhisperPerson>;

    // ---- P3 (default bodies: a fake or a build with no whisper socket hears nothing) ----

    /// The own-channel topics `owner_id`'s rules need; replaces the last set.
    /// The whisper session subscribes them while it is up.
    async fn hold_own_topics(&self, _owner_id: &str, _kinds: &BTreeSet<TopicKind>) {}
    /// Delete the own-channel subscriptions (with the token that made them).
    async fn release_own_topics(&self, _owner_id: &str) {}
    /// What the whisper session holds for the engine.
    fn own_topic_state(&self) -> OwnState {
        OwnState::default()
    }
    /// Twitch's clock for a local wall time (the whisper socket measures the offset).
    fn twitch_now(&self, wall_ms: i64) -> i64 {
        wall_ms
    }
    /// A Speak line to the players (`automations://speak`; global on iOS).
    fn emit_speak(&self, _targets: &[String], _payload: &SpeakPayload) {}
}

/// One person the Whisper action's picker lists.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct WhisperPerson {
    pub user_id: String,
    pub login: String,
    pub name: String,
}

#[derive(Serialize)]
struct ActivityPayload<'a> {
    entries: &'a [Entry],
}

/// The seam the running app uses.
pub struct RealLocal {
    app: AppHandle,
    spawn: super::Spawn,
}

impl RealLocal {
    pub fn new(app: AppHandle, spawn: super::Spawn) -> RealLocal {
        RealLocal { app, spawn }
    }
}

const LABEL: &str = super::AUTOMATIONS_LABEL;

#[async_trait]
impl Local for RealLocal {
    fn settings(&self) -> (Automations, AutomationPrefs) {
        use tauri::Manager;
        let state = self.app.state::<crate::models::settings::AppState>();
        let s = state.settings.lock().unwrap_or_else(|e| e.into_inner());
        (s.automations.clone(), s.automation_prefs.clone())
    }

    fn reminders(&self) -> Vec<crate::services::reminder_service::convert::ReminderSummary> {
        use tauri::Manager;
        let state = self.app.state::<crate::models::settings::AppState>();
        let settings = state.settings.lock().unwrap_or_else(|e| e.into_inner()).clone();
        crate::services::reminder_service::convert::summaries(&settings)
    }

    async fn open_for_reminders(&self, login: &str) -> bool {
        crate::services::irc_service::IrcService::open_for_reminders(login).await
    }

    async fn chats_held_by(&self, label: &str) -> Vec<String> {
        crate::services::irc_service::IrcService::channels_held_by(label).await
    }

    async fn holders(&self, channel: &str) -> Vec<String> {
        crate::services::irc_service::IrcService::holders(channel).await
    }

    async fn chat_connected(&self) -> bool {
        crate::services::irc_service::IrcService::is_connected().await
    }

    fn recovery_cycling(&self) -> bool {
        crate::services::chat_recovery::is_cycling()
    }

    async fn chat_start(&self, first: &str) -> Result<(), String> {
        use tauri::Manager;
        let state = self.app.state::<crate::models::settings::AppState>();
        crate::services::chat_service::ChatService::start(first, state.inner(), true, true, LABEL)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn chat_join(&self, channel: &str) -> Result<(), String> {
        use tauri::Manager;
        let state = self.app.state::<crate::models::settings::AppState>();
        crate::services::chat_service::ChatService::join_channel(channel, state.inner(), LABEL)
            .await
            .map_err(|e| e.to_string())
    }

    async fn chat_leave(&self, channel: &str) {
        if let Err(e) = crate::services::chat_service::ChatService::leave_channel(channel, LABEL).await {
            log::debug!("[Automations] leave {channel}: {e}");
        }
    }

    async fn provider_join(&self, provider: &str, channel: &str) -> Result<(), String> {
        let registry = crate::services::providers::registry().await;
        let Some(adapter) = registry.get(provider) else { return Err(format!("No {provider} chat on this device")) };
        adapter.connect(channel, LABEL).await.map_err(|e| e.to_string())
    }

    async fn provider_leave(&self, provider: &str, channel: &str) {
        let registry = crate::services::providers::registry().await;
        if let Some(adapter) = registry.get(provider) {
            if let Err(e) = adapter.disconnect(channel, LABEL).await {
                log::debug!("[Automations] leave {provider}:{channel}: {e}");
            }
        }
    }

    async fn provider_held_by(&self, provider: &str, label: &str) -> Vec<String> {
        match crate::services::providers::registry().await.get(provider) {
            Some(adapter) => adapter.channels_held_by(label).await,
            None => Vec::new(),
        }
    }

    async fn provider_held_by_other_than(&self, provider: &str, channel: &str, label: &str) -> bool {
        match crate::services::providers::registry().await.get(provider) {
            Some(adapter) => adapter.held_by_other_than(channel, label).await,
            None => false,
        }
    }

    async fn watch_live(&self, login: &str, room_id: &str) {
        let _ = crate::services::channel_state::watch(login, room_id, true).await;
    }

    async fn unwatch_live(&self, login: &str) {
        crate::services::channel_state::unwatch(login, true).await;
    }

    async fn is_live(&self, login: &str) -> Option<bool> {
        crate::services::channel_state::is_live(login).await
    }

    async fn channel_info(&self, login: &str) -> Option<ChannelState> {
        crate::services::channel_state::get(login).await
    }

    fn current_watch(&self) -> Option<String> {
        crate::services::watch_session::current_twitch_watch().map(|(login, _)| login)
    }

    async fn room_notice(&self, login: &str, text: &str) {
        crate::services::slash_dispatch::post_room_notice(login, text).await;
    }

    fn streamer_mode(&self) -> bool {
        crate::services::streamer_mode::StreamerMode::is_active()
    }

    fn notify_targets(&self) -> Vec<String> {
        #[cfg(target_os = "ios")]
        {
            vec!["main".to_string()]
        }
        #[cfg(not(target_os = "ios"))]
        {
            use tauri::Manager;
            let labels = self.app.webview_windows().into_keys();
            super::notify::players(labels, crate::services::app_exit::main_in_tray())
        }
    }

    fn main_focused_on_screen(&self) -> bool {
        #[cfg(desktop)]
        {
            use tauri::Manager;
            self.app.get_webview_window("main").is_some_and(|w| {
                w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(true) && w.is_focused().unwrap_or(false)
            })
        }
        #[cfg(not(desktop))]
        {
            true
        }
    }

    fn emit_notify(&self, targets: &[String], payload: &NotifyPayload) {
        use tauri::Emitter;
        #[cfg(target_os = "ios")]
        {
            let _ = targets;
            let _ = self.app.emit("automations://notify", payload);
        }
        #[cfg(not(target_os = "ios"))]
        for target in targets {
            let _ = self.app.emit_to(target.as_str(), "automations://notify", payload);
        }
    }

    fn system_notice(&self, request: &NotifyRequest, id: u64) {
        #[cfg(desktop)]
        {
            use crate::commands::notification_mirror::{MirroredNotice, NoticeAction};
            let (action_label, action) = if request.settings_link {
                (Some("Open".to_string()), Some(NoticeAction::Settings { tab: "Automations".into() }))
            } else if request.channel.is_empty() {
                (None, None)
            } else {
                (Some("Watch".to_string()), Some(NoticeAction::Watch { channel: request.channel.clone() }))
            };
            let notice = MirroredNotice {
                id: format!("automation-{id}"),
                kind: "automation".into(),
                title: request.title.clone(),
                body: request.body.clone(),
                quote: None,
                image_url: None,
                round_image: false,
                action_label,
                action,
                reply: None,
            };
            let app = self.app.clone();
            (self.spawn)(Box::pin(async move {
                crate::services::system_notify::show_with(app, notice, None).await;
            }));
        }
        #[cfg(not(desktop))]
        {
            let _ = (request, id);
        }
    }

    fn emit_activity(&self, watchers: &[String], entries: &[Entry]) {
        use tauri::Emitter;
        let payload = ActivityPayload { entries };
        #[cfg(target_os = "ios")]
        {
            let _ = watchers;
            let _ = self.app.emit("automations://activity", &payload);
        }
        #[cfg(not(target_os = "ios"))]
        for label in watchers {
            let _ = self.app.emit_to(label.as_str(), "automations://activity", &payload);
        }
    }

    fn emit_status(&self, snapshot: &StatusSnapshot) {
        use tauri::Emitter;
        let _ = self.app.emit("automations://status", snapshot);
    }

    async fn probe_link(&self, url: &str) -> Probe {
        probe(url, std::time::Duration::from_secs(5)).await
    }

    async fn refresh_gallery(&self) -> Result<(), String> {
        crate::commands::badges::refresh_global_badges().await.map(|_| ())
    }

    async fn turn_off(&self) {
        let result = crate::commands::account_automations::edit(&self.app, |a| {
            Ok(crate::commands::account_automations::apply_run(a, crate::models::automations::RunState::Off))
        });
        if let Err(e) = result {
            log::warn!("[Automations] could not store Off after the breaker: {e}");
        }
    }

    fn own_role(&self, as_login: &str, channel: &str) -> Option<crate::services::chat_send_limiter::Role> {
        crate::services::chat_send_limiter::known_role(as_login, channel)
    }

    fn whisper_people(&self, owner_id: &str) -> Vec<WhisperPerson> {
        let Some(scope) = crate::services::whisper_sync::current_scope().filter(|s| s.owner == owner_id) else { return Vec::new() };
        let rows = crate::services::whisper_store::read_index(&scope, |idx| {
            idx.conversations.iter().map(|c| (c.user_id.clone(), c.user_login.clone(), c.user_name.clone())).collect::<Vec<_>>()
        })
        .unwrap_or_default();
        rows.into_iter()
            .filter(|(id, _, _)| !id.is_empty() && crate::services::whisper_store::knows(&scope, id))
            .map(|(user_id, login, name)| WhisperPerson { user_id, login, name })
            .collect()
    }

    async fn hold_own_topics(&self, owner_id: &str, kinds: &BTreeSet<TopicKind>) {
        crate::services::whisper_service::hold_own_topics(owner_id, kinds);
    }

    async fn release_own_topics(&self, owner_id: &str) {
        crate::services::whisper_service::release_own_topics(owner_id).await;
    }

    fn own_topic_state(&self) -> OwnState {
        crate::services::whisper_service::own_topic_state()
    }

    fn twitch_now(&self, wall_ms: i64) -> i64 {
        crate::services::whisper_sync::twitch_now(wall_ms)
    }

    fn emit_speak(&self, targets: &[String], payload: &SpeakPayload) {
        use tauri::Emitter;
        #[cfg(target_os = "ios")]
        {
            let _ = targets;
            let _ = self.app.emit("automations://speak", payload);
        }
        #[cfg(not(target_os = "ios"))]
        for target in targets {
            let _ = self.app.emit_to(target.as_str(), "automations://speak", payload);
        }
    }
}

#[async_trait]
impl AccountChanges for RealLocal {
    async fn begin_account_change(&self) -> AccountChange {
        StoreChanges.begin_account_change().await
    }
}

/// GET `url` once, never following a redirect (a 3xx is its own status, not
/// ready). A link `check_url` refuses is never requested.
pub(crate) async fn probe(url: &str, timeout: std::time::Duration) -> Probe {
    if super::badges::check_url(url).is_err() {
        return Probe::Failed;
    }
    let Ok(client) = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .build()
    else {
        return Probe::Failed;
    };
    match client.get(url).send().await {
        Ok(resp) => Probe::Status(resp.status().as_u16()),
        Err(_) => Probe::Failed,
    }
}
