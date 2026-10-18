//! A moderator's warning to the signed-in viewer, and acknowledging it.
//!
//! A warned viewer cannot chat in that channel until they acknowledge the
//! warning. Twitch tells the viewer three ways, and all three land here:
//!
//! - PubSub `chatrooms-user-v1.<viewer>` pushes `user_moderation_action`
//!   with `action: "warn"` (id and reason) the moment a moderator warns, and
//!   `"acknowledge_warning"` once it is acknowledged anywhere (twitch.tv too);
//! - a send refused with IRC NOTICE `msg_warned`, or a Helix send whose drop
//!   reason says to acknowledge the warning, for a warning that came before
//!   the app was listening;
//! - GQL `chatModeratorStrikeStatus` then fills in the reason for that case.
//!
//! The pending warning rides the room's words (`room_modes::RoomModeView`
//! `warning`), which every message box already reads, so a phone and the
//! desktop draw the same lock and the same button. Acknowledging is Twitch's
//! own `AcknowledgeChatWarning` mutation, sent as the viewer with the
//! twitch.tv web session: no Helix endpoint exists for it and no extra
//! permission is asked for.
//!
//! Keyed by the channel's user id (the room id).

use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

const ACK_HASH: &str = "f97404a69caf9d152118bae17e962eca27c87c8a85224538173b3dfcd6c9df60";

const STATUS_QUERY: &str = r#"query ChatWarningStatus($channelID: ID!, $userID: ID!) {
  chatModeratorStrikeStatus(channelID: $channelID, userID: $userID) {
    warningDetails { id reason createdAt chatRulesCited }
  }
}"#;

/// What Twitch says when a warned viewer tries to chat, over IRC and in a
/// Helix drop reason alike ("... Acknowledge the Warning in browser ...").
const REFUSAL_HINT: &str = "acknowledge the warning";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    /// Twitch's warning id; empty while only a refused send told us.
    pub id: String,
    pub reason: Option<String>,
    /// Rules the moderator cited.
    pub rules: Vec<String>,
}

/// What the message box shows while a warning is pending.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WarningView {
    /// "A moderator warned you".
    pub title: String,
    /// The reason in quotes and what to do, in one or two sentences.
    pub detail: String,
    /// "Acknowledge".
    pub action: String,
    /// The disabled field's placeholder: "Acknowledge the warning to chat".
    pub prompt: String,
    /// A word or two for a narrow box: "Warned".
    pub short: String,
}

static WARNINGS: OnceLock<Mutex<HashMap<String, Warning>>> = OnceLock::new();

fn warnings() -> std::sync::MutexGuard<'static, HashMap<String, Warning>> {
    WARNINGS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

pub fn pending(room_id: &str) -> Option<Warning> {
    warnings().get(room_id).cloned()
}

/// The words for a pending warning in `room_id`, if any.
pub fn view(room_id: &str) -> Option<WarningView> {
    pending(room_id).map(|w| view_of(&w))
}

pub fn view_of(w: &Warning) -> WarningView {
    let mut detail = String::new();
    if let Some(reason) = w.reason.as_deref() {
        detail.push_str(&format!("\u{201C}{reason}\u{201D} "));
    }
    if !w.rules.is_empty() {
        detail.push_str(&format!("Rules: {}. ", w.rules.join(", ")));
    }
    detail.push_str("You can chat again after you acknowledge it.");
    WarningView {
        title: "A moderator warned you".into(),
        detail,
        action: "Acknowledge".into(),
        prompt: "Acknowledge the warning to chat".into(),
        short: "Warned".into(),
    }
}

/// Store or clear `room_id`'s warning. Returns whether anything changed, so
/// the caller only rewords the room when it did.
fn set(room_id: &str, warning: Option<Warning>) -> bool {
    let mut map = warnings();
    match warning {
        Some(w) => {
            // A refusal carries no id or reason: keep what is already known.
            if w.id.is_empty() {
                if map.contains_key(room_id) {
                    return false;
                }
            }
            let changed = map.get(room_id) != Some(&w);
            map.insert(room_id.to_string(), w);
            changed
        }
        None => map.remove(room_id).is_some(),
    }
}

/// Every pending warning belongs to the account that was signed in.
pub fn reset_account() {
    warnings().clear();
}

/// Read one PubSub `chatrooms-user-v1` message. Returns the room id whose
/// warning changed. Pure, for tests.
pub fn apply_pubsub(message: &Value, own_user_id: &str) -> Option<String> {
    if message.get("type").and_then(Value::as_str) != Some("user_moderation_action") {
        return None;
    }
    let data = message.get("data")?;
    let room_id = data.get("channel_id").and_then(Value::as_str)?.to_string();
    if let Some(target) = data.get("target_id").and_then(Value::as_str) {
        if !own_user_id.is_empty() && target != own_user_id {
            return None;
        }
    }
    let changed = match data.get("action").and_then(Value::as_str)? {
        "warn" => set(
            &room_id,
            Some(Warning {
                id: data.get("id").and_then(Value::as_str).unwrap_or_default().to_string(),
                reason: data.get("reason").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string),
                rules: Vec::new(),
            }),
        ),
        "acknowledge_warning" => set(&room_id, None),
        _ => false,
    };
    changed.then_some(room_id)
}

/// Whether a send refusal (NOTICE id or drop-reason text) means "warned".
pub fn is_warned_refusal(msg_id: Option<&str>, text: Option<&str>) -> bool {
    msg_id == Some("msg_warned") || text.is_some_and(|t| t.to_lowercase().contains(REFUSAL_HINT))
}

/// Read a `chatModeratorStrikeStatus` answer. `Some(None)` means Twitch says
/// there is no pending warning; `None` means no usable answer. Pure, for tests.
pub fn status_from(json: &Value) -> Option<Option<Warning>> {
    let status = json.pointer("/data/chatModeratorStrikeStatus")?;
    if status.is_null() {
        return None;
    }
    let details = status.get("warningDetails").filter(|d| !d.is_null());
    Some(details.map(|d| Warning {
        id: d.get("id").and_then(Value::as_str).unwrap_or_default().to_string(),
        reason: d.get("reason").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string),
        rules: d
            .get("chatRulesCited")
            .and_then(Value::as_array)
            .map(|r| r.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default(),
    }))
}

async fn own_user_id() -> Option<String> {
    if let Some(account) = crate::services::account_store::AccountStore::primary() {
        return Some(account.user_id);
    }
    crate::services::twitch_service::TwitchService::get_user_info().await.ok().map(|u| u.id)
}

/// Reword every open chat of `room_id` after its warning changed.
async fn reword(room_id: &str) {
    for channel in crate::services::chat_send_limiter::channels_with_room_id(room_id) {
        crate::services::irc_service::reship_room_state(&channel).await;
    }
}

/// A PubSub `chatrooms-user-v1` message arrived.
pub async fn on_pubsub(message: Value) {
    let own = own_user_id().await.unwrap_or_default();
    if let Some(room_id) = apply_pubsub(&message, &own) {
        reword(&room_id).await;
    }
}

/// A send to `channel` (login) was refused. When the refusal means "warned",
/// mark the room and ask Twitch for the reason.
pub async fn on_refusal(channel: &str, msg_id: Option<&str>, text: Option<&str>) {
    if !is_warned_refusal(msg_id, text) {
        return;
    }
    let Some(room_id) = crate::services::chat_send_limiter::room_id_of(channel) else {
        return;
    };
    let fresh = set(&room_id, Some(Warning { id: String::new(), reason: None, rules: Vec::new() }));
    if fresh {
        reword(&room_id).await;
    }
    refresh(&room_id).await;
}

/// Ask Twitch whether a warning is pending in `room_id`, and reword the room
/// if the answer changed anything.
pub async fn refresh(room_id: &str) {
    let (Some(token), Some(me)) = (crate::services::twitch_web_gql::app_web_session().await, own_user_id().await) else {
        return;
    };
    let body = json!({
        "operationName": "ChatWarningStatus",
        "query": STATUS_QUERY,
        "variables": { "channelID": room_id, "userID": me },
    });
    let answer = match crate::services::twitch_web_gql::post(Some(&token), body, "chat-warning").await {
        Ok(json) => status_from(&json),
        Err(e) => {
            log::warn!("[chat-warning] status: {e}");
            None
        }
    };
    if let Some(warning) = answer {
        if set(room_id, warning) {
            reword(room_id).await;
        }
    }
}

/// Acknowledge `room_id`'s warning as the viewer. On success the room's
/// message box unlocks at once; PubSub's echo then finds nothing to change.
pub async fn acknowledge(room_id: &str) -> Result<(), String> {
    let token = crate::services::twitch_web_gql::app_web_session()
        .await
        .ok_or_else(|| "Sign in to Twitch to acknowledge the warning".to_string())?;
    let body = json!({
        "operationName": "AcknowledgeChatWarning",
        "variables": { "input": { "channelID": room_id } },
        "extensions": { "persistedQuery": { "version": 1, "sha256Hash": ACK_HASH } },
    });
    let json = crate::services::twitch_web_gql::post(Some(&token), body, "chat-warning-ack").await?;
    if let Some(err) = ack_error(&json) {
        return Err(err);
    }
    if set(room_id, None) {
        reword(room_id).await;
    }
    Ok(())
}

/// Twitch's refusal of an acknowledgement in words, or `None` when it went
/// through. Pure, for tests.
pub fn ack_error(json: &Value) -> Option<String> {
    if let Some(errors) = json.get("errors").and_then(Value::as_array).filter(|e| !e.is_empty()) {
        let first = errors[0].get("message").and_then(Value::as_str).unwrap_or("unknown error");
        return Some(if first.contains("PersistedQueryNotFound") {
            "Twitch changed how warnings are acknowledged. Acknowledge it on twitch.tv for now".to_string()
        } else {
            format!("Twitch did not take the acknowledgement: {first}")
        });
    }
    let payload = json.pointer("/data/acknowledgeChatWarning");
    if let Some(code) = payload.and_then(|p| p.get("error")).filter(|e| !e.is_null()) {
        let code = code.get("code").and_then(Value::as_str).unwrap_or("error");
        return Some(format!("Twitch did not take the acknowledgement ({code})"));
    }
    None
}

#[tauri::command]
pub async fn acknowledge_chat_warning(room_id: String) -> Result<(), String> {
    acknowledge(room_id.trim()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pubsub_warn_then_acknowledge() {
        let room = "pubsub-room-1";
        let warn = json!({ "type": "user_moderation_action", "data": {
            "action": "warn", "channel_id": room, "target_id": "9", "id": "w1", "reason": " keep it friendly " } });
        assert_eq!(apply_pubsub(&warn, "9").as_deref(), Some(room));
        let w = pending(room).unwrap();
        assert_eq!(w.id, "w1");
        assert_eq!(w.reason.as_deref(), Some("keep it friendly"));
        assert_eq!(view(room).unwrap().detail, "\u{201C}keep it friendly\u{201D} You can chat again after you acknowledge it.");
        // The same push again changes nothing.
        assert_eq!(apply_pubsub(&warn, "9"), None);
        let ack = json!({ "type": "user_moderation_action", "data": {
            "action": "acknowledge_warning", "channel_id": room, "target_id": "9" } });
        assert_eq!(apply_pubsub(&ack, "9").as_deref(), Some(room));
        assert!(pending(room).is_none());
    }

    #[test]
    fn pubsub_for_someone_else_is_ignored() {
        let warn = json!({ "type": "user_moderation_action", "data": {
            "action": "warn", "channel_id": "pubsub-room-2", "target_id": "8", "id": "w1" } });
        assert_eq!(apply_pubsub(&warn, "9"), None);
        assert_eq!(apply_pubsub(&json!({ "type": "other" }), "9"), None);
    }

    #[test]
    fn a_refusal_marks_without_losing_a_known_reason() {
        let room = "refusal-room";
        assert!(set(room, Some(Warning { id: "w2".into(), reason: Some("spam".into()), rules: vec![] })));
        assert!(!set(room, Some(Warning { id: String::new(), reason: None, rules: vec![] })));
        assert_eq!(pending(room).unwrap().reason.as_deref(), Some("spam"));
        assert!(set(room, None));
    }

    #[test]
    fn refusal_texts() {
        assert!(is_warned_refusal(Some("msg_warned"), None));
        assert!(is_warned_refusal(None, Some("You received a Warning from a moderator in this channel. Acknowledge the Warning in browser to continue chatting in this channel.")));
        assert!(!is_warned_refusal(Some("msg_slowmode"), Some("slow down")));
    }

    #[test]
    fn strike_status() {
        let none = json!({ "data": { "chatModeratorStrikeStatus": { "warningDetails": null } } });
        assert_eq!(status_from(&none), Some(None));
        let some = json!({ "data": { "chatModeratorStrikeStatus": { "warningDetails": {
            "id": "w3", "reason": "no slurs", "createdAt": "2026-10-18T00:00:00Z", "chatRulesCited": ["Be kind"] } } } });
        let w = status_from(&some).unwrap().unwrap();
        assert_eq!(w.rules, vec!["Be kind".to_string()]);
        assert_eq!(view_of(&w).detail, "\u{201C}no slurs\u{201D} Rules: Be kind. You can chat again after you acknowledge it.");
        assert_eq!(status_from(&json!({ "errors": [{ "message": "unauthenticated" }], "data": { "chatModeratorStrikeStatus": null } })), None);
    }

    #[test]
    fn acknowledgement_errors() {
        assert_eq!(ack_error(&json!({ "data": { "acknowledgeChatWarning": { "error": null } } })), None);
        assert!(ack_error(&json!({ "errors": [{ "message": "PersistedQueryNotFound" }] })).unwrap().contains("twitch.tv"));
        assert!(ack_error(&json!({ "data": { "acknowledgeChatWarning": { "error": { "code": "NOT_WARNED" } } } })).unwrap().contains("NOT_WARNED"));
    }
}
