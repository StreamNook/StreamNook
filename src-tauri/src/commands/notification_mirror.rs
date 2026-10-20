//! Where a notification goes when the main window is not the window in use.
//!
//! The main window owns every notification: it applies the settings, keeps the
//! notification centre and plays the sound. Hidden to the tray, minimized or
//! behind another app it still does all of that, so a viewer elsewhere hears
//! the ping and has nothing to look at.
//!
//! The main window hands each announced notification here as plain data, and
//! `route` decides, in one place so the viewer sees it exactly once:
//!
//! - main window in use: nowhere else;
//! - "System notifications" on: the operating system's own notification
//!   (services/system_notify.rs) whenever the main window is not in use;
//! - otherwise, only while the main window is off screen, a card in exactly
//!   one MultiChat window that is on screen (the focused one, else the first
//!   by label), so two open MultiChat windows never show the same card twice.
//!
//! The sound stays the main window's, so it never doubles either.
//!
//! A click (on the card, or on the Windows notification) comes back through
//! `activate`, which brings the main window forward and tells it which
//! notification was clicked; the action itself (open the stream, the whisper,
//! the drops page) stays with the main window, which is the one that can
//! perform it.
//!
//! A Windows notification can outlive the process that showed it (it waits in
//! Action Center), so each one also carries its action as data (`NoticeAction`)
//! and, for whispers and chat mentions, a reply target (`NoticeReply`).
//!
//! A click is a `streamnook://notify?a=...` link (`ToastArgs::to_url`), which
//! Windows opens like any other StreamNook link: it reaches the running app
//! through single-instance, or starts it. So a click works after the app closed
//! and from a portable copy, with nothing registered beyond the link scheme the
//! app already owns. A click the main window no longer knows the id of runs the
//! data instead, through a pending slot the page takes exactly once.
//!
//! A reply typed into the notification arrives through the toast's own event
//! while this process runs (a link cannot carry typed text), and is sent from
//! here without opening anything.

use std::collections::VecDeque;
use std::sync::Mutex;

use crate::rt::AppHandle;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager};

/// To the chosen MultiChat window: show this card.
pub const SHOW_EVENT: &str = "mirrored-notification";
/// To the main window: the notification with this id was clicked.
pub const ACTIVATE_EVENT: &str = "mirrored-notification-activate";
/// To the main window: a notification action is waiting in
/// `take_notice_action`.
pub const ACTION_PENDING_EVENT: &str = "notice-action-pending";

const MULTICHAT_PREFIX: &str = "multichat-";
const MAX_TITLE: usize = 120;
const MAX_BODY: usize = 240;
/// A reply typed into a notification, at most what chat and whispers accept.
const MAX_REPLY: usize = 500;
/// The key a Windows notification's reply box carries its text under.
pub const REPLY_INPUT: &str = "reply";
/// Toast arguments carry this prefix and a JSON `ToastArgs`.
const ARGS_PREFIX: &str = "sn1:";
/// `streamnook://notify?a=<base64url JSON ToastArgs>`: a notification's click.
const LINK_HOST: &str = "notify";
/// Ids of notifications the main window announced this run and still holds an
/// action for (it keeps a handful; this keeps a few more).
const PAGE_IDS_KEPT: usize = 32;

/// What clicking a notification does, as data, so it still works after a
/// restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum NoticeAction {
    /// Open this stream (a `startStream` key: a Twitch login or `provider:channel`).
    Watch { channel: String },
    /// Open the whisper thread with this person.
    Whisper { user_id: String, login: String, name: String, avatar: Option<String> },
    /// Open the drops page.
    Drops,
    /// Open the badges page.
    Badges,
    /// Show what is new in this version.
    Changelog { version: Option<String> },
    /// Open this channel (a Twitch login) with its rewards showing.
    Rewards { channel: String },
    /// Open Settings on this tab.
    Settings { tab: String },
    /// Open the chat this mentions-inbox entry is in, at its message.
    /// `channel` is the entry's `open_key` (a `startStream` key) for when the
    /// entry is gone by the time the notification is clicked.
    Mention { id: String, channel: String },
}

/// Who a reply typed into the notification goes to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum NoticeReply {
    /// A whisper back to this person.
    Whisper { user_id: String, login: String, name: String, avatar: Option<String> },
    /// A Twitch chat reply, threaded under message `parent_id` in `channel`.
    Chat { channel: String, broadcaster_id: Option<String>, parent_id: String, to_name: String },
}

impl NoticeReply {
    /// The reply box's hint.
    pub fn placeholder(&self) -> String {
        match self {
            NoticeReply::Whisper { name, .. } => format!("Whisper {name}"),
            NoticeReply::Chat { to_name, .. } => format!("Reply to {to_name}"),
        }
    }
}

/// What a toast hands back when it is used: which notification, what it does,
/// and whether this was its Send button.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToastArgs {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<NoticeAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<NoticeReply>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub send: bool,
}

impl ToastArgs {
    pub fn encode(&self) -> String {
        format!("{ARGS_PREFIX}{}", serde_json::to_string(self).unwrap_or_default())
    }

    pub fn decode(raw: &str) -> Option<Self> {
        serde_json::from_str(raw.trim().strip_prefix(ARGS_PREFIX)?).ok()
    }

    /// The click as a link Windows opens like any other StreamNook link.
    pub fn to_url(&self) -> String {
        use base64::Engine;
        let json = serde_json::to_vec(self).unwrap_or_default();
        format!(
            "streamnook://{LINK_HOST}?a={}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json)
        )
    }

    /// Read a notification link. Anything can open a `streamnook://` link, so
    /// what comes back is only ever a click: a link never sends a reply.
    pub fn from_url(url: &url::Url) -> Option<Self> {
        use base64::Engine;
        if url.scheme() != "streamnook" || url.host_str() != Some(LINK_HOST) {
            return None;
        }
        let a = url.query_pairs().find(|(k, _)| k == "a")?.1;
        let json = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(a.as_bytes()).ok()?;
        let mut args: ToastArgs = serde_json::from_slice(&json).ok()?;
        args.send = false;
        Some(args)
    }
}

/// True for a `streamnook://notify` link, which is a notification's click and
/// never a channel to watch.
pub fn is_notice_url(url: &url::Url) -> bool {
    url.scheme() == "streamnook" && url.host_str() == Some(LINK_HOST)
}

/// A notification's click arrived as a link (the running app, or the one it
/// just started). Unreadable links are dropped.
pub fn on_notice_url(app: &AppHandle, url: &url::Url) {
    match ToastArgs::from_url(url) {
        Some(args) => on_activated(app, &args.encode(), None),
        None => log::debug!("[NotificationMirror] ignored an unreadable notification link"),
    }
}

static PAGE_IDS: Lazy<Mutex<VecDeque<String>>> = Lazy::new(|| Mutex::new(VecDeque::new()));
static PENDING: Lazy<Mutex<Option<NoticeAction>>> = Lazy::new(|| Mutex::new(None));

fn remember_page_id(id: &str) {
    let mut ids = PAGE_IDS.lock().unwrap_or_else(|e| e.into_inner());
    ids.push_back(id.to_string());
    while ids.len() > PAGE_IDS_KEPT {
        ids.pop_front();
    }
}

fn page_knows(id: &str) -> bool {
    PAGE_IDS.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|k| k == id)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MirroredNotice {
    pub id: String,
    /// The notification centre's type name (`live`, `whisper`, `drops`, ...);
    /// picks the glyph when there is no picture.
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub image_url: Option<String>,
    /// Round picture (a person) rather than a square one (box art, a badge).
    #[serde(default)]
    pub round_image: bool,
    /// What a click does, in words ("Watch", "Reply"). None = not clickable.
    #[serde(default)]
    pub action_label: Option<String>,
    /// The click's action as data, for a notification used after a restart.
    #[serde(default)]
    pub action: Option<NoticeAction>,
    /// Where a reply typed into the notification goes. None = no reply box.
    #[serde(default)]
    pub reply: Option<NoticeReply>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Candidate {
    label: String,
    on_screen: bool,
    focused: bool,
}

/// What the main window is doing, as far as being seen goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Main {
    on_screen: bool,
    focused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Route {
    /// The viewer can already see it in the main window, or nowhere can.
    Nowhere,
    /// The operating system's own notification.
    System,
    /// A card in this MultiChat window.
    MultiChat(String),
}

fn route(main: Main, system_on: bool, mut candidates: Vec<Candidate>) -> Route {
    if main.on_screen && main.focused {
        return Route::Nowhere;
    }
    if system_on {
        return Route::System;
    }
    if main.on_screen {
        return Route::Nowhere;
    }
    candidates.retain(|c| c.on_screen);
    candidates.sort_by(|a, b| b.focused.cmp(&a.focused).then_with(|| a.label.cmp(&b.label)));
    candidates
        .into_iter()
        .next()
        .map_or(Route::Nowhere, |c| Route::MultiChat(c.label))
}

fn on_screen(window: &crate::rt::WebviewWindow) -> bool {
    window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(true)
}

fn clamp(text: &mut String, max: usize) {
    if text.chars().count() > max {
        *text = text.chars().take(max - 1).collect::<String>() + "\u{2026}";
    }
}

fn system_notifications_on(app: &AppHandle) -> bool {
    let state = app.state::<crate::models::settings::AppState>();
    let s = state.settings.lock().unwrap_or_else(|e| e.into_inner());
    s.live_notifications.enabled && s.live_notifications.use_native_notifications
}

/// Show `notice` wherever the viewer will see it (see `route`). Returns
/// `"system"`, the MultiChat label it went to, or None when nobody needed it.
/// The main window keeps the action for this id, so a click it can still map
/// runs its own action rather than the data.
#[tauri::command]
pub async fn mirror_notification(app: AppHandle, notice: MirroredNotice) -> Option<String> {
    let id = notice.id.clone();
    let shown = deliver(app, notice).await;
    if shown.is_some() {
        remember_page_id(&id);
    }
    shown
}

/// Where a notification raised now would go. A caller with work to do before
/// it can say anything (drawing a picture) asks first, and skips the work
/// when the answer is nowhere.
pub(crate) fn current_route(app: &AppHandle) -> Route {
    let windows = app.webview_windows();
    let main = windows
        .get("main")
        .map(|w| Main { on_screen: on_screen(w), focused: w.is_focused().unwrap_or(false) })
        .unwrap_or(Main { on_screen: false, focused: false });
    let candidates = windows
        .iter()
        .filter(|(label, _)| label.starts_with(MULTICHAT_PREFIX))
        .map(|(label, w)| Candidate {
            label: label.clone(),
            on_screen: on_screen(w),
            focused: w.is_focused().unwrap_or(false),
        })
        .collect();
    route(main, system_notifications_on(app), candidates)
}

/// Route a notification Rust raised itself (a chat mention): the same places,
/// with only its data to act on.
pub async fn deliver(app: AppHandle, notice: MirroredNotice) -> Option<String> {
    deliver_with(app, notice, None).await
}

/// `deliver`, with what more a system notification can carry. A quiet one
/// (an Action Center update that never pops up) has no MultiChat card.
pub async fn deliver_with(
    app: AppHandle,
    mut notice: MirroredNotice,
    rich: Option<crate::services::system_notify::RichToast>,
) -> Option<String> {
    clamp(&mut notice.title, MAX_TITLE);
    clamp(&mut notice.body, MAX_BODY);
    match current_route(&app) {
        Route::Nowhere => None,
        Route::System => {
            crate::services::system_notify::show_with(app, notice, rich).await;
            Some("system".into())
        }
        Route::MultiChat(_) if rich.as_ref().is_some_and(|r| r.quiet) => None,
        Route::MultiChat(target) => {
            if let Err(e) = app.emit_to(target.as_str(), SHOW_EVENT, &notice) {
                log::debug!("[NotificationMirror] emit to {target} failed: {e}");
                return None;
            }
            Some(target)
        }
    }
}

/// The Settings test: a sample system notification, whichever window is in use.
#[tauri::command]
pub async fn test_system_notification(app: AppHandle) {
    let notice = MirroredNotice {
        id: "system-test".into(),
        kind: "update".into(),
        title: "StreamNook".into(),
        body: "System notifications are on. They show while StreamNook is in the background.".into(),
        image_url: None,
        round_image: false,
        action_label: None,
        action: None,
        reply: None,
    };
    crate::services::system_notify::show(app, notice).await;
}

/// Whether the system lets StreamNook show its notifications: `granted`,
/// `denied`, or `default` while it has not asked. Only macOS asks, and with
/// `ask` it shows its prompt then; Windows and Linux always answer `granted`.
#[tauri::command]
pub async fn system_notification_permission(ask: bool) -> String {
    crate::services::system_notify::permission(ask).await.as_str().to_string()
}

/// Open the system's own notification settings, where a refusal is undone.
#[tauri::command]
pub fn open_system_notification_settings(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let url = crate::services::system_notify::settings_url().ok_or("No notification settings to open here")?;
    app.opener().open_url(url, None::<String>).map_err(|e| format!("Could not open notification settings: {e}"))
}

/// Bring the main window forward and let it run notification `id`'s action.
/// Shared by the MultiChat card and the Windows notification's click.
pub fn activate(app: &AppHandle, id: &str) {
    crate::show_main_window(app);
    if let Err(e) = app.emit_to("main", ACTIVATE_EVENT, id) {
        log::debug!("[NotificationMirror] activate emit failed: {e}");
    }
}

/// A MultiChat card was clicked. A card raised by Rust (a mention) has no
/// action in the page, so its data runs instead.
#[tauri::command]
pub fn open_mirrored_notification(app: AppHandle, id: String, action: Option<NoticeAction>) {
    if page_knows(&id) || action.is_none() {
        activate(&app, &id);
    } else {
        run_action(&app, action);
    }
}

/// What a Windows notification's arguments and typed text mean.
#[derive(Debug, PartialEq)]
enum Use {
    /// Send this text to the reply target; nothing opens.
    Reply(NoticeReply, String),
    /// The main window still holds this id's action.
    PageAction(String),
    /// Run the action from the data (or just open, when there is none).
    Data(Option<NoticeAction>),
    /// Nothing to do (an empty reply, unreadable arguments).
    Ignore,
}

fn interpret(args: &str, text: Option<String>, page_knows: impl Fn(&str) -> bool) -> Use {
    let Some(args) = ToastArgs::decode(args) else {
        return Use::Ignore;
    };
    if args.send {
        let text = text.map(|t| t.trim().to_string()).unwrap_or_default();
        return match args.reply {
            Some(target) if !text.is_empty() => Use::Reply(target, text.chars().take(MAX_REPLY).collect()),
            _ => Use::Ignore,
        };
    }
    if page_knows(&args.id) {
        Use::PageAction(args.id)
    } else {
        Use::Data(args.action)
    }
}

/// A Windows notification was used: a click (through its link) or a typed
/// reply (through the toast's own event, while this process runs).
pub fn on_activated(app: &AppHandle, args: &str, text: Option<String>) {
    let used = interpret(args, text, page_knows);
    log::info!(
        "[NotificationMirror] notification used: {}",
        match &used {
            Use::Reply(..) => "reply",
            Use::PageAction(_) => "click (page action)",
            Use::Data(Some(_)) => "click (action from data)",
            Use::Data(None) => "click (open)",
            Use::Ignore => "ignored",
        }
    );
    match used {
        Use::Reply(target, text) => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move { send_reply(app, target, text).await });
        }
        Use::PageAction(id) => activate(app, &id),
        Use::Data(action) => run_action(app, action),
        Use::Ignore => {}
    }
}

/// Open the main window and leave `action` for it. The page takes it once,
/// on this event or on mount (a click that started the app), never twice.
fn run_action(app: &AppHandle, action: Option<NoticeAction>) {
    crate::show_main_window(app);
    if let Some(action) = action {
        *PENDING.lock().unwrap_or_else(|e| e.into_inner()) = Some(action);
        let _ = app.emit_to("main", ACTION_PENDING_EVENT, ());
    }
}

/// The notification action waiting for the main window, if any.
#[tauri::command]
pub fn take_notice_action() -> Option<NoticeAction> {
    PENDING.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// Send a reply typed into a notification. A failure is said in a
/// notification of its own, since nothing else is on screen.
async fn send_reply(app: AppHandle, target: NoticeReply, text: String) {
    let result = if std::env::var_os("SN_NOTIFY_DRY_RUN").is_some() {
        log::info!("[NotificationMirror] dry run: reply {target:?} with {} chars", text.chars().count());
        Ok(())
    } else {
        match &target {
            NoticeReply::Whisper { user_id, login, name, avatar } => {
                crate::services::whisper_inbox::reply_from_notification(&app, user_id, login, name, avatar.clone(), text)
                    .await
            }
            NoticeReply::Chat { channel, broadcaster_id, parent_id, .. } => {
                send_chat_reply(&app, channel, broadcaster_id.as_deref(), parent_id, &text).await
            }
        }
    };
    if let Err(e) = result {
        log::warn!("[NotificationMirror] reply failed: {e}");
        let to = match &target {
            NoticeReply::Whisper { name, .. } => name.clone(),
            NoticeReply::Chat { to_name, .. } => to_name.clone(),
        };
        let notice = MirroredNotice {
            id: format!("reply-failed-{}", chrono::Utc::now().timestamp_millis()),
            kind: "update".into(),
            title: format!("Your reply to {to} was not sent"),
            body: e,
            image_url: None,
            round_image: false,
            action_label: None,
            action: None,
            reply: None,
        };
        crate::services::system_notify::show(app, notice).await;
    }
}

async fn send_chat_reply(
    app: &AppHandle,
    channel: &str,
    broadcaster_id: Option<&str>,
    parent_id: &str,
    text: &str,
) -> Result<(), String> {
    // A typed cheer would go out as plain text and spend nothing; cheering
    // needs the confirm card in the app.
    let state = app.state::<crate::models::settings::AppState>();
    if crate::services::bits::typed_cheer(&state.twitch_auth, text, channel, broadcaster_id, true).await.is_some() {
        return Err("Cheers are sent from the chat box in StreamNook.".into());
    }
    crate::services::chat_service::ChatService::send_message(
        text,
        Some(parent_id),
        Some(channel),
        broadcaster_id,
        None,
        None,
    )
    .await
    .map_err(|e| e.to_string())
    .and_then(|sent| match sent.is_sent {
        true => Ok(()),
        false => Err(sent.drop_reason.unwrap_or_else(|| "Twitch did not accept the message.".into())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(label: &str, on_screen: bool, focused: bool) -> Candidate {
        Candidate { label: label.into(), on_screen, focused }
    }

    const IN_USE: Main = Main { on_screen: true, focused: true };
    const BEHIND: Main = Main { on_screen: true, focused: false };
    const AWAY: Main = Main { on_screen: false, focused: false };

    #[test]
    fn nowhere_while_main_is_in_use() {
        assert_eq!(route(IN_USE, true, vec![c("multichat-default", true, true)]), Route::Nowhere);
        assert_eq!(route(IN_USE, false, vec![c("multichat-default", true, true)]), Route::Nowhere);
    }

    #[test]
    fn system_whenever_main_is_not_in_use() {
        assert_eq!(route(BEHIND, true, vec![]), Route::System);
        // One place: the system notification replaces the MultiChat card.
        assert_eq!(route(AWAY, true, vec![c("multichat-default", true, true)]), Route::System);
    }

    #[test]
    fn without_system_a_visible_main_keeps_it() {
        assert_eq!(route(BEHIND, false, vec![c("multichat-default", true, true)]), Route::Nowhere);
    }

    #[test]
    fn nowhere_when_no_multichat_is_on_screen() {
        assert_eq!(route(AWAY, false, vec![]), Route::Nowhere);
        assert_eq!(route(AWAY, false, vec![c("multichat-default", false, false)]), Route::Nowhere);
    }

    #[test]
    fn exactly_one_window_focused_first() {
        let picked = route(AWAY, false, vec![c("multichat-default", true, false), c("multichat-w2", true, true)]);
        assert_eq!(picked, Route::MultiChat("multichat-w2".into()));
    }

    #[test]
    fn unfocused_falls_back_to_first_label() {
        let picked = route(
            AWAY,
            false,
            vec![c("multichat-w2", true, false), c("multichat-default", true, false), c("multichat-w3", false, true)],
        );
        assert_eq!(picked, Route::MultiChat("multichat-default".into()));
    }

    fn chat_reply() -> NoticeReply {
        NoticeReply::Chat {
            channel: "somechannel".into(),
            broadcaster_id: Some("123".into()),
            parent_id: "abc-def".into(),
            to_name: "Viewer".into(),
        }
    }

    #[test]
    fn toast_args_round_trip_and_reject_anything_else() {
        let args = ToastArgs {
            id: "mention-1".into(),
            action: Some(NoticeAction::Watch { channel: "somechannel".into() }),
            reply: Some(chat_reply()),
            send: true,
        };
        let raw = args.encode();
        assert!(raw.starts_with("sn1:{"));
        assert_eq!(ToastArgs::decode(&raw), Some(args));
        assert_eq!(ToastArgs::decode("open"), None);
        assert_eq!(ToastArgs::decode(""), None);
        // The page's camelCase shape, as the TS side sends it.
        let from_page: NoticeAction =
            serde_json::from_str(r#"{"type":"whisper","userId":"9","login":"a","name":"A","avatar":null}"#).unwrap();
        assert!(matches!(from_page, NoticeAction::Whisper { ref user_id, .. } if user_id == "9"));
    }

    #[test]
    fn a_click_link_round_trips_but_never_sends() {
        let args = ToastArgs {
            id: "mirror-7".into(),
            action: Some(NoticeAction::Changelog { version: Some("8.8.12".into()) }),
            reply: Some(chat_reply()),
            send: true,
        };
        let url = url::Url::parse(&args.to_url()).unwrap();
        assert!(is_notice_url(&url));
        assert!(url.as_str().starts_with("streamnook://notify?a="));
        let back = ToastArgs::from_url(&url).unwrap();
        assert_eq!(back.id, "mirror-7");
        assert_eq!(back.action, args.action);
        assert!(!back.send, "a link must never send a reply");
        // A share link and junk are not notification links.
        assert!(!is_notice_url(&url::Url::parse("streamnook://watch/xqc").unwrap()));
        assert!(ToastArgs::from_url(&url::Url::parse("streamnook://notify?a=%%%").unwrap()).is_none());
        assert!(ToastArgs::from_url(&url::Url::parse("streamnook://notify").unwrap()).is_none());
    }

    #[test]
    fn a_send_needs_text_and_a_target() {
        let send = ToastArgs { id: "w".into(), action: None, reply: Some(chat_reply()), send: true }.encode();
        assert_eq!(
            interpret(&send, Some("  hi there  ".into()), |_| false),
            Use::Reply(chat_reply(), "hi there".into())
        );
        assert_eq!(interpret(&send, Some("   ".into()), |_| false), Use::Ignore);
        assert_eq!(interpret(&send, None, |_| false), Use::Ignore);
        let no_target = ToastArgs { id: "w".into(), action: None, reply: None, send: true }.encode();
        assert_eq!(interpret(&no_target, Some("hi".into()), |_| false), Use::Ignore);
        let long = "x".repeat(MAX_REPLY + 50);
        assert!(matches!(interpret(&send, Some(long), |_| false), Use::Reply(_, t) if t.chars().count() == MAX_REPLY));
    }

    #[test]
    fn a_click_runs_the_page_action_while_the_page_has_it() {
        let watch = Some(NoticeAction::Watch { channel: "x".into() });
        let click = ToastArgs { id: "mirror-1".into(), action: watch.clone(), reply: None, send: false }.encode();
        assert_eq!(interpret(&click, None, |id| id == "mirror-1"), Use::PageAction("mirror-1".into()));
        // After a restart the page knows nothing: the data runs.
        assert_eq!(interpret(&click, None, |_| false), Use::Data(watch));
        // Text typed but the body clicked: still a click.
        assert!(matches!(interpret(&click, Some("hi".into()), |_| false), Use::Data(_)));
    }

    #[test]
    fn clamp_counts_chars_not_bytes() {
        let mut s = "\u{e9}".repeat(10);
        clamp(&mut s, 5);
        assert_eq!(s.chars().count(), 5);
        assert!(s.ends_with('\u{2026}'));
        let mut short = String::from("ok");
        clamp(&mut short, 5);
        assert_eq!(short, "ok");
    }
}
