//! What a user card shows the viewer beyond the person's profile: their own
//! private note and name color for the chatter, and, in a channel the viewer
//! moderates, the Moderate buttons and the chatter's moderation history.
//!
//! Rust decides all of it so both phones draw the same card:
//!
//! - who may be moderated: the viewer is the broadcaster or a moderator there
//!   (from their own badges), and the chatter is not the viewer, not the
//!   broadcaster, and not a moderator unless the viewer is the broadcaster;
//! - the four timeout lengths, which are the desktop hover dock's list
//!   (`moderation.timeout_presets`), so one edit changes every surface, then
//!   Ban, or Unban while a ban or timeout is in force, then Warn when the
//!   sign-in carries Twitch's warnings permission;
//! - the viewer's own buttons (`moderation.card_actions`): a label and a
//!   timeout, ban, unban or warning, with an optional saved reason;
//! - the chatter's low-trust standing as this session saw it, with the
//!   treatments the viewer can switch to (`suspicious_users::card_view`).
//!
//! Each button carries the exact action `moderate_chatter` takes, so a card
//! passes it through without knowing what it does.
//!
//! The history is a Twitch call, so a card may ask without it first
//! (`history: false`) to draw the buttons at once, then again with it, which
//! turns Ban into Unban or Lift timeout when one is in force.
//!
//! The note lives in `user_notes` and the color in
//! `chat_customization.user_overrides`, both on this device, as on desktop.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The timeout lengths when the viewer never set any (the desktop dock's).
pub const DEFAULT_LENGTHS: [u32; 4] = [1, 600, 3_600, 86_400];
/// The card shows four lengths; the dock list may be longer.
const CARD_LENGTHS: usize = 4;
/// More of the viewer's own buttons than this stop fitting the card.
pub const MAX_CUSTOM: usize = 6;
/// Twitch's longest timeout.
const TWITCH_MAX_TIMEOUT: u32 = 1_209_600;
/// The optional scope a warning needs; sign-ins from before it was asked for
/// lack it.
const WARN_SCOPE: &str = "moderator:manage:warnings";
/// How long the quick first answer waits on Twitch for the scope list before
/// leaving Warn out (the second answer waits for it).
const QUICK_SCOPE_WAIT: std::time::Duration = std::time::Duration::from_millis(800);
const FULL_SCOPE_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Name colors offered on the card: each reads on the dark themes, and the
/// set covers the hues people pick. The card adds the system picker for more.
pub const SWATCHES: [&str; 10] = [
    "#7FD3FF", "#5AA9FF", "#C49BFF", "#FF7AD9", "#FF7A90", "#FF9F43", "#FFD166", "#7BE08E", "#4FD1C5", "#E6E6E6",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonKind {
    Timeout,
    Ban,
    Unban,
    Warn,
}

/// One of the viewer's own card buttons, as saved in settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardAction {
    pub label: String,
    pub kind: ButtonKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A button on the card, ready to run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CardButton {
    pub id: String,
    /// "10m", "Ban", "Spoilers".
    pub label: String,
    pub kind: ButtonKind,
    /// What `moderate_chatter` takes as `action`.
    pub action: Value,
    /// The reason sent with it, when one was saved.
    pub reason: Option<String>,
    /// A warning needs a reason the viewer types (none was saved).
    pub asks_reason: bool,
    /// Ban: drawn in the danger ink.
    pub danger: bool,
    /// For VoiceOver: "Time out for 10 minutes".
    pub spoken: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Moderate {
    /// The four lengths, then Ban or Unban.
    pub buttons: Vec<CardButton>,
    /// The viewer's own buttons.
    pub custom: Vec<CardButton>,
    pub history: crate::services::mod_history::ModHistory,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChatterCardTools {
    pub note: Option<String>,
    pub nickname: Option<String>,
    /// The viewer's color for this chatter, when they set one.
    pub color: Option<String>,
    pub swatches: Vec<String>,
    /// Present in a channel the viewer may moderate this chatter in.
    pub moderate: Option<Moderate>,
    /// Present with `moderate`.
    pub low_trust: Option<crate::services::suspicious_users::LowTrust>,
}

/// "1s", "10m", "1h", "1d": the dock's own words for a length.
pub fn short_length(seconds: u32) -> String {
    crate::services::moderation::duration_label(seconds)
}

fn spoken_length(seconds: u32) -> String {
    crate::services::mod_history::length_words(seconds as u64)
}

/// The card's four lengths from the dock list: the first four valid ones in
/// order, topped up from the defaults when fewer were saved.
pub fn lengths_from(settings: &Value) -> Vec<u32> {
    let saved: Vec<u32> = settings["moderation"]["timeout_presets"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_u64)
                .filter(|s| (1..=TWITCH_MAX_TIMEOUT as u64).contains(s))
                .map(|s| s as u32)
                .collect()
        })
        .unwrap_or_default();
    let mut out: Vec<u32> = Vec::new();
    for s in saved.into_iter().chain(DEFAULT_LENGTHS) {
        if !out.contains(&s) {
            out.push(s);
        }
        if out.len() == CARD_LENGTHS {
            break;
        }
    }
    out
}

/// The viewer's own buttons, dropping any that could not run.
pub fn custom_from(settings: &Value) -> Vec<CardAction> {
    settings["moderation"]["card_actions"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| serde_json::from_value::<CardAction>(v.clone()).ok())
                .filter(|c| !c.label.trim().is_empty())
                .filter(|c| c.kind != ButtonKind::Timeout || c.seconds.is_some_and(|s| (1..=TWITCH_MAX_TIMEOUT).contains(&s)))
                .take(MAX_CUSTOM)
                .collect()
        })
        .unwrap_or_default()
}

fn button(id: String, label: String, kind: ButtonKind, seconds: Option<u32>, reason: Option<String>) -> CardButton {
    let reason = reason.map(|r| r.trim().to_string()).filter(|r| !r.is_empty());
    let (action, spoken) = match kind {
        ButtonKind::Timeout => {
            let s = seconds.unwrap_or(600);
            let spoken = if s == 1 { "Purge their messages".to_string() } else { format!("Time out for {}", spoken_length(s)) };
            (json!({ "kind": "timeout", "seconds": s }), spoken)
        }
        ButtonKind::Ban => (json!({ "kind": "ban" }), "Ban".to_string()),
        ButtonKind::Unban => (json!({ "kind": "unban", "timeout": seconds.is_some() }), label.clone()),
        ButtonKind::Warn => (json!({ "kind": "warn" }), "Warn: they must acknowledge it before they can chat".to_string()),
    };
    CardButton {
        id,
        asks_reason: kind == ButtonKind::Warn && reason.is_none(),
        danger: kind == ButtonKind::Ban,
        label,
        kind,
        action,
        reason,
        spoken,
    }
}

/// The Moderate buttons for a chatter whose ban state is `current`, with Warn
/// last when the viewer's sign-in may warn. Ban and Unban share one slot, so
/// a card swapping one for the other moves nothing. Pure, for tests.
pub fn buttons_for(
    lengths: &[u32],
    current: Option<&crate::services::mod_history::CurrentBan>,
    can_warn: bool,
) -> Vec<CardButton> {
    let mut out: Vec<CardButton> = lengths
        .iter()
        .map(|&s| button(format!("t{s}"), short_length(s), ButtonKind::Timeout, Some(s), None))
        .collect();
    out.push(match current {
        Some(c) if c.permanent => button("unban".into(), "Unban".into(), ButtonKind::Unban, None, None),
        Some(_) => button("unban".into(), "Lift timeout".into(), ButtonKind::Unban, Some(0), None),
        None => button("ban".into(), "Ban".into(), ButtonKind::Ban, None, None),
    });
    if can_warn {
        out.push(button("warn".into(), "Warn".into(), ButtonKind::Warn, None, None));
    }
    out
}

/// The signed-in token's answer to "may it warn", remembered per token: a
/// new sign-in, another account or a refreshed token is a new key, so the
/// answer never outlives the sign-in it was asked for.
static WARN_SCOPE_CACHE: std::sync::Mutex<Option<(u64, bool)>> = std::sync::Mutex::new(None);

fn token_key(token: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    token.hash(&mut h);
    h.finish()
}

/// Whether the signed-in Twitch account may send warnings. Asks Twitch once
/// per token, waiting at most `wait`; an answer that did not come in time is
/// not remembered, so the next card asks again.
async fn can_warn(wait: std::time::Duration) -> bool {
    use crate::services::twitch_service::{lacks_scope, TwitchService};
    let Ok(token) = TwitchService::get_token().await else {
        return false;
    };
    let key = token_key(&token);
    if let Some((k, may)) = WARN_SCOPE_CACHE.lock().ok().and_then(|c| *c) {
        if k == key {
            return may;
        }
    }
    match tokio::time::timeout(wait, TwitchService::token_scopes(&token)).await {
        Ok(Ok(granted)) => {
            let may = !lacks_scope(&granted, WARN_SCOPE);
            if let Ok(mut c) = WARN_SCOPE_CACHE.lock() {
                *c = Some((key, may));
            }
            may
        }
        _ => false,
    }
}

pub fn custom_buttons(actions: &[CardAction]) -> Vec<CardButton> {
    actions
        .iter()
        .enumerate()
        .map(|(i, a)| button(format!("c{i}"), a.label.trim().to_string(), a.kind, a.seconds, a.reason.clone()))
        .collect()
}

/// Whether the viewer may moderate the chatter, from the viewer's role in
/// the room and the chatter's last badges there. Pure, for tests.
pub fn may_moderate(viewer: crate::services::chat_send_limiter::Role, viewer_id: &str, target_id: &str, target_badges: &[String]) -> bool {
    use crate::services::chat_send_limiter::Role;
    if target_id.is_empty() || target_id == viewer_id {
        return false;
    }
    let has = |name: &str| target_badges.iter().any(|b| b == name || b.starts_with(&format!("{name}/")));
    match viewer {
        Role::Broadcaster => !has("broadcaster"),
        Role::Moderator => !has("broadcaster") && !has("moderator"),
        _ => false,
    }
}

fn settings_value() -> Value {
    use tauri::Manager;
    crate::services::providers::app_handle()
        .and_then(|app| {
            let state = app.state::<crate::models::settings::AppState>();
            let settings = state.settings.lock().ok()?.clone();
            serde_json::to_value(settings).ok()
        })
        .unwrap_or(Value::Null)
}

/// The signed-in Twitch account's user id.
pub async fn own_user_id() -> String {
    if let Some(account) = crate::services::account_store::AccountStore::primary() {
        return account.user_id;
    }
    crate::services::twitch_service::TwitchService::get_user_info().await.map(|u| u.id).unwrap_or_default()
}

/// The Moderate section for a chatter: with the history when it was fetched,
/// else with none loaded and Ban in the slot. Pure, for tests.
pub fn moderate_for(settings: &Value, history: Option<crate::services::mod_history::ModHistory>, can_warn: bool) -> Moderate {
    use crate::services::mod_history::HistoryStatus;
    let history = history.unwrap_or_else(crate::services::mod_history::ModHistory::not_loaded);
    let mut buttons = buttons_for(&lengths_from(settings), history.current.as_ref(), can_warn);
    // Twitch would not say whether a ban is in force, so Unban stays offered
    // beside Ban.
    let unknown = matches!(history.status, HistoryStatus::NoAccess | HistoryStatus::SignedOut | HistoryStatus::Failed);
    if unknown && history.current.is_none() {
        let at = buttons.iter().position(|b| b.kind == ButtonKind::Ban).map_or(buttons.len(), |i| i + 1);
        buttons.insert(at, button("unban".into(), "Unban".into(), ButtonKind::Unban, None, None));
    }
    Moderate { buttons, custom: custom_buttons(&custom_from(settings)), history }
}

/// The card's tools for `user_id` in the Twitch channel `channel` (login)
/// whose user id is `channel_id`. Other platforms get the note and color only.
/// `history: false` answers at once without the moderation history (its
/// status is `not_loaded`); absent means with it. `viewer_moderates` is the
/// shell's own reading of the viewer's badges, used while Rust has seen no
/// USERSTATE for the room yet.
#[tauri::command]
pub async fn get_chatter_card_tools(
    provider: Option<String>,
    channel: Option<String>,
    channel_id: Option<String>,
    user_id: String,
    history: Option<bool>,
    viewer_moderates: Option<bool>,
) -> ChatterCardTools {
    let provider = provider.unwrap_or_else(|| "twitch".into());
    let key = crate::services::chat_row_view::override_key(&provider, &user_id);
    let own = crate::services::chat_row_view::user_override(&provider, &user_id).unwrap_or_default();
    let note = crate::services::user_notes::UserNotes::get(&key).map(|n| n.note);
    let mut tools = ChatterCardTools {
        note,
        nickname: own.nickname,
        color: own.color,
        swatches: SWATCHES.iter().map(|s| s.to_string()).collect(),
        moderate: None,
        low_trust: None,
    };
    let (Some(channel), Some(room)) = (channel.filter(|c| !c.is_empty()), channel_id.filter(|c| !c.is_empty())) else {
        return tools;
    };
    if provider != "twitch" {
        return tools;
    }
    use crate::services::chat_send_limiter::Role;
    let mut role = crate::services::chat_send_limiter::budget(&channel).role;
    if role == Role::Viewer && viewer_moderates == Some(true) {
        role = Role::Moderator;
    }
    let badges = crate::services::chat_history::ChatHistory::last_look(&room, &user_id).map(|(b, _)| b).unwrap_or_default();
    if !may_moderate(role, &own_user_id().await, &user_id, &badges) {
        return tools;
    }
    let (history, warn) = if history.unwrap_or(true) {
        let (h, w) = tokio::join!(crate::services::mod_history::fetch(&room, &user_id), can_warn(FULL_SCOPE_WAIT));
        (Some(h), w)
    } else {
        (None, can_warn(QUICK_SCOPE_WAIT).await)
    };
    tools.moderate = Some(moderate_for(&settings_value(), history, warn));
    tools.low_trust = Some(crate::services::suspicious_users::SuspiciousUsers::card_view(&channel, &user_id));
    tools
}

/// The four lengths with `index` set to `seconds`, written back over the dock
/// list's first four (any further dock lengths are kept after them). `None`
/// when the length is out of range or already one of the four. Pure, for tests.
pub fn with_length(settings: &Value, index: usize, seconds: u32) -> Option<Vec<u32>> {
    if index >= CARD_LENGTHS || !(1..=TWITCH_MAX_TIMEOUT).contains(&seconds) {
        return None;
    }
    let mut four = lengths_from(settings);
    if four[index] != seconds && four.contains(&seconds) {
        return None;
    }
    four[index] = seconds;
    let rest: Vec<u32> = settings["moderation"]["timeout_presets"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_u64).map(|s| s as u32).skip(CARD_LENGTHS).filter(|s| !four.contains(s)).collect())
        .unwrap_or_default();
    four.extend(rest);
    Some(four)
}

/// Sets one of the card's four timeout lengths (which are the dock's first
/// four). Answers the four as stored.
#[tauri::command]
pub async fn set_card_timeout_length(
    app: crate::rt::AppHandle,
    index: usize,
    seconds: u32,
    state: tauri::State<'_, crate::models::settings::AppState>,
) -> Result<Vec<u32>, String> {
    let mut stored = Vec::new();
    crate::commands::settings::commit_settings_patch(&app, &state, None, |current| {
        let value = serde_json::to_value(current).map_err(|e| e.to_string())?;
        let Some(list) = with_length(&value, index, seconds) else {
            return Err("That length is already one of the four, or out of range (1 second to 14 days).".to_string());
        };
        stored = list.iter().take(CARD_LENGTHS).copied().collect();
        let mut changes = serde_json::Map::new();
        changes.insert("moderation.timeout_presets".into(), json!(list));
        crate::commands::settings_fields::patch_for_changes(current, &changes)
    })?;
    Ok(stored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::chat_send_limiter::Role;

    #[test]
    fn short_lengths() {
        assert_eq!(short_length(1), "1s");
        assert_eq!(short_length(600), "10m");
        assert_eq!(short_length(3_600), "1h");
        assert_eq!(short_length(86_400), "1d");
        assert_eq!(short_length(1_209_600), "2w");
        assert_eq!(short_length(90), "1m 30s");
    }

    #[test]
    fn four_lengths_from_the_dock_list() {
        assert_eq!(lengths_from(&Value::Null), DEFAULT_LENGTHS.to_vec());
        let saved = json!({ "moderation": { "timeout_presets": [60, 300, 600, 1800, 3600, 86400] } });
        assert_eq!(lengths_from(&saved), vec![60, 300, 600, 1800]);
        let short = json!({ "moderation": { "timeout_presets": [30, 0, 9_999_999, 30] } });
        assert_eq!(lengths_from(&short), vec![30, 1, 600, 3_600], "invalid and repeated ones drop, defaults top up");
    }

    #[test]
    fn ban_or_unban_follows_what_is_in_force() {
        let lengths = [1, 600, 3_600, 86_400];
        let open = buttons_for(&lengths, None, false);
        assert_eq!(open.iter().map(|b| b.label.as_str()).collect::<Vec<_>>(), ["1s", "10m", "1h", "1d", "Ban"]);
        assert_eq!(open[1].action, json!({ "kind": "timeout", "seconds": 600 }));
        assert_eq!(open[0].spoken, "Purge their messages");
        assert_eq!(open[1].spoken, "Time out for 10 minutes");
        assert!(open[4].danger);

        let banned = crate::services::mod_history::CurrentBan {
            permanent: true, ends_ms: None, by: None, reason: None, label: "Banned".into(),
        };
        let b = buttons_for(&lengths, Some(&banned), false);
        assert_eq!(b[4].label, "Unban");
        assert_eq!(b[4].action, json!({ "kind": "unban", "timeout": false }));
        let timed = crate::services::mod_history::CurrentBan { permanent: false, ends_ms: Some(1), ..banned };
        let t = buttons_for(&lengths, Some(&timed), false);
        assert_eq!(t[4].label, "Lift timeout");
        assert_eq!(t[4].action, json!({ "kind": "unban", "timeout": true }));
        assert!(!t[4].danger);
    }

    #[test]
    fn warn_comes_last_only_when_the_sign_in_may_warn() {
        let lengths = [1, 600, 3_600, 86_400];
        assert!(buttons_for(&lengths, None, false).iter().all(|b| b.kind != ButtonKind::Warn));
        let with = buttons_for(&lengths, None, true);
        assert_eq!(with.iter().map(|b| b.label.as_str()).collect::<Vec<_>>(), ["1s", "10m", "1h", "1d", "Ban", "Warn"]);
        let warn = &with[5];
        assert_eq!(warn.id, "warn");
        assert_eq!(warn.action, json!({ "kind": "warn" }), "the shape moderate_chatter reads as ModAction::Warn");
        assert!(warn.asks_reason, "Twitch needs a reason they will see");
        assert!(warn.reason.is_none());
        assert!(!warn.danger);
        let banned = crate::services::mod_history::CurrentBan {
            permanent: true, ends_ms: None, by: None, reason: None, label: "Banned".into(),
        };
        let b = buttons_for(&lengths, Some(&banned), true);
        assert_eq!(b[4].label, "Unban", "Unban keeps Ban's slot");
        assert_eq!(b[5].label, "Warn");
    }

    #[test]
    fn the_quick_answer_has_ban_and_no_history() {
        let saved = json!({ "moderation": {
            "timeout_presets": [60, 300, 600, 1800],
            "card_actions": [{ "label": "Spoilers", "kind": "timeout", "seconds": 1800 }]
        } });
        let quick = moderate_for(&saved, None, true);
        assert_eq!(quick.history.status, crate::services::mod_history::HistoryStatus::NotLoaded);
        assert!(quick.history.entries.is_empty());
        assert!(quick.history.current.is_none());
        assert!(quick.history.note.is_none());
        assert_eq!(quick.buttons.iter().map(|b| b.label.as_str()).collect::<Vec<_>>(), ["1m", "5m", "10m", "30m", "Ban", "Warn"]);
        assert_eq!(quick.custom.len(), 1);
        assert_eq!(serde_json::to_value(&quick.history).unwrap()["status"], "not_loaded");
    }

    #[test]
    fn without_an_answer_from_twitch_unban_stays_beside_ban() {
        use crate::services::mod_history::{HistoryStatus, ModHistory};
        let saved = json!({ "moderation": { "timeout_presets": [60, 300, 600, 1800] } });
        let failed = ModHistory { status: HistoryStatus::Failed, entries: vec![], current: None, note: None };
        let m = moderate_for(&saved, Some(failed), false);
        assert_eq!(m.buttons.iter().map(|b| b.label.as_str()).collect::<Vec<_>>(), ["1m", "5m", "10m", "30m", "Ban", "Unban"]);
        let quick = moderate_for(&saved, None, false);
        assert!(!quick.buttons.iter().any(|b| b.label == "Unban"), "the quick answer waits for the history");
    }

    #[test]
    fn the_viewers_own_buttons() {
        let saved = json!({ "moderation": { "card_actions": [
            { "label": "Spoilers", "kind": "timeout", "seconds": 1800, "reason": "spoilers" },
            { "label": "Warn", "kind": "warn" },
            { "label": "Bad", "kind": "timeout" },
            { "label": "  ", "kind": "ban" },
            { "label": "Nope", "kind": "explode" }
        ] } });
        let actions = custom_from(&saved);
        assert_eq!(actions.len(), 2, "no length, no label and an unknown kind drop");
        let buttons = custom_buttons(&actions);
        assert_eq!(buttons[0].reason.as_deref(), Some("spoilers"));
        assert!(!buttons[0].asks_reason);
        assert_eq!(buttons[1].action, json!({ "kind": "warn" }));
        assert!(buttons[1].asks_reason, "a warning with no saved reason asks for one");
    }

    #[test]
    fn one_length_changes_in_place() {
        assert_eq!(with_length(&Value::Null, 1, 300), Some(vec![1, 300, 3_600, 86_400]));
        let long = json!({ "moderation": { "timeout_presets": [60, 300, 600, 1800, 3600, 86400] } });
        assert_eq!(with_length(&long, 0, 30), Some(vec![30, 300, 600, 1800, 3600, 86400]), "the dock's extra lengths stay");
        assert_eq!(with_length(&long, 0, 300), None, "already one of the four");
        assert_eq!(with_length(&long, 4, 300), None);
        assert_eq!(with_length(&long, 0, 0), None);
        assert_eq!(with_length(&long, 2, 600), Some(vec![60, 300, 600, 1800, 3600, 86400]), "unchanged");
    }

    #[test]
    fn who_may_be_moderated() {
        let none: Vec<String> = vec![];
        let modb = vec!["moderator/1".to_string()];
        let caster = vec!["broadcaster/1".to_string()];
        assert!(may_moderate(Role::Moderator, "1", "2", &none));
        assert!(!may_moderate(Role::Moderator, "1", "2", &modb));
        assert!(may_moderate(Role::Broadcaster, "1", "2", &modb));
        assert!(!may_moderate(Role::Broadcaster, "1", "2", &caster));
        assert!(!may_moderate(Role::Moderator, "1", "1", &none), "never yourself");
        assert!(!may_moderate(Role::Vip, "1", "2", &none));
        assert!(!may_moderate(Role::Viewer, "1", "2", &none));
    }
}
