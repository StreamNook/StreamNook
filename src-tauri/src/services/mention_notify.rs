//! A notification when someone mentions you, replies to you, or uses one of
//! your highlight phrases, in a chat you have joined while StreamNook is not
//! the window in use. `mention_inbox::on_verdict` decides what counts and calls
//! this once per newly filed entry.
//!
//! `chat_rules` already decides what a mention is, the same judgment the
//! mention sound (`mention_ping`) uses. This turns the ones worth a notice into
//! a `MirroredNotice` and hands it to `notification_mirror::deliver`, which
//! shows it as a system notification or a MultiChat card, or nowhere when the
//! main window is in use. The notice opens the chat at that message and, as a
//! Windows notification on Twitch, takes a reply posted as a threaded reply.

use std::collections::HashMap;
use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;

use crate::commands::notification_mirror::{MirroredNotice, NoticeAction, NoticeReply};
use crate::models::chat_layout::ChatMessage;
use crate::services::mention_inbox::{room_key, MentionKind};
use crate::models::settings::Settings;

/// One channel notifies at most this often, so a chat that keeps @-ing you is
/// one notice, not a stack.
const CHANNEL_COOLDOWN: Duration = Duration::from_secs(20);
/// A message older than this when it arrives is replay, not news.
const MAX_AGE_MS: i64 = 15_000;

static ENABLED: RwLock<bool> = RwLock::new(false);
static LAST: Lazy<Mutex<HashMap<String, Instant>>> = Lazy::new(|| Mutex::new(HashMap::new()));

/// Re-read the setting. Called on load and on every save, with `mention_ping`.
pub fn refresh(settings: &Settings) {
    let on = settings.live_notifications.enabled && settings.live_notifications.show_mention_notifications;
    if let Ok(mut g) = ENABLED.write() {
        *g = on;
    }
}

fn sent_at_ms(msg: &ChatMessage) -> Option<i64> {
    msg.tags.get("tmi-sent-ts").and_then(|v| v.parse::<i64>().ok())
}

/// Whether this live message earns a notice now. Pure apart from the clock
/// and the cooldown table it is handed.
fn should_notify(
    channel: &str,
    sent_at: Option<i64>,
    now_ms: i64,
    now: Instant,
    last: &mut HashMap<String, Instant>,
) -> bool {
    if sent_at.is_some_and(|t| now_ms - t > MAX_AGE_MS) {
        return false;
    }
    if last.get(channel).is_some_and(|t| now.duration_since(*t) < CHANNEL_COOLDOWN) {
        return false;
    }
    last.retain(|_, t| now.duration_since(*t) < CHANNEL_COOLDOWN);
    last.insert(channel.to_string(), now);
    true
}

/// The notice for `msg`. `avatar` is the sender's picture when it was found.
fn notice_for(msg: &ChatMessage, kind: MentionKind, avatar: Option<String>) -> MirroredNotice {
    let room = room_key(&msg.provider, &msg.channel);
    let channel = room.split_once(':').map(|(_, slug)| slug.to_string()).unwrap_or_else(|| room.clone());
    let name = if msg.display_name.is_empty() { msg.username.clone() } else { msg.display_name.clone() };
    let title = match kind {
        MentionKind::Reply => format!("{name} replied to you in {channel}"),
        MentionKind::Mention => format!("{name} mentioned you in {channel}"),
        MentionKind::Word => format!("{name} in {channel}"),
    };
    // Only Twitch takes a threaded reply from the notification.
    let twitch = !room.contains(':');
    MirroredNotice {
        id: format!("mention-{}", msg.id),
        kind: "mention".into(),
        title,
        body: msg.content.clone(),
        image_url: avatar,
        round_image: true,
        action_label: Some("Open chat".into()),
        action: Some(NoticeAction::Mention { id: msg.id.clone(), channel: room.clone() }),
        reply: twitch.then(|| NoticeReply::Chat {
            channel,
            broadcaster_id: msg.tags.get("room-id").cloned(),
            parent_id: msg.id.clone(),
            to_name: name,
        }),
    }
}

/// Called once for each newly filed inbox entry.
pub fn on_live_message(msg: &ChatMessage, kind: MentionKind) {
    if msg.id.is_empty() {
        return;
    }
    if !ENABLED.read().map(|g| *g).unwrap_or(false) {
        return;
    }
    if crate::services::streamer_mode::StreamerMode::is_active() {
        return;
    }
    let channel = msg.channel.trim_start_matches('#').to_lowercase();
    let fire = match LAST.lock() {
        Ok(mut last) => should_notify(
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
    let Some(app) = crate::services::providers::app_handle() else { return };
    let msg = msg.clone();
    tauri::async_runtime::spawn(async move {
        let avatar = if msg.provider.is_empty() || msg.provider.eq_ignore_ascii_case("twitch") {
            match crate::services::twitch_service::TwitchService::get_user_by_id(&msg.user_id).await {
                Ok(user) => user.profile_image_url,
                Err(_) => None,
            }
        } else {
            None
        };
        crate::commands::notification_mirror::deliver(app, notice_for(&msg, kind, avatar)).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_notice_per_channel_per_cooldown_and_never_for_replay() {
        let mut last = HashMap::new();
        let t0 = Instant::now();
        let now_ms = 1_000_000;
        assert!(should_notify("a", Some(now_ms - 1_000), now_ms, t0, &mut last));
        assert!(!should_notify("a", Some(now_ms), now_ms, t0 + Duration::from_secs(5), &mut last));
        assert!(should_notify("b", None, now_ms, t0 + Duration::from_secs(5), &mut last));
        assert!(should_notify("a", None, now_ms, t0 + CHANNEL_COOLDOWN, &mut last));
        assert!(!should_notify("c", Some(now_ms - MAX_AGE_MS - 1), now_ms, t0, &mut last));
    }

    #[test]
    fn the_notice_opens_the_message_and_replies_in_its_thread() {
        let mut msg = ChatMessage {
            id: "msg-1".into(),
            user_id: "42".into(),
            username: "viewer".into(),
            display_name: "Viewer".into(),
            color: None,
            badges: vec![],
            timestamp: "0".into(),
            content: "@you hi".into(),
            provider: "twitch".into(),
            channel: "#SomeChannel".into(),
            emotes: vec![],
            tags: HashMap::new(),
            layout: Default::default(),
            segments: vec![],
            metadata: Default::default(),
        };
        msg.tags.insert("room-id".into(), "777".into());
        let n = notice_for(&msg, MentionKind::Reply, None);
        assert_eq!(n.title, "Viewer replied to you in somechannel");
        assert_eq!(
            n.action,
            Some(NoticeAction::Mention { id: "msg-1".into(), channel: "somechannel".into() })
        );
        assert_eq!(
            n.reply,
            Some(NoticeReply::Chat {
                channel: "somechannel".into(),
                broadcaster_id: Some("777".into()),
                parent_id: "msg-1".into(),
                to_name: "Viewer".into(),
            })
        );
        assert_eq!(notice_for(&msg, MentionKind::Mention, None).title, "Viewer mentioned you in somechannel");
        assert_eq!(notice_for(&msg, MentionKind::Word, None).title, "Viewer in somechannel");
    }

    #[test]
    fn a_kick_mention_opens_its_room_and_takes_no_reply() {
        let msg = ChatMessage {
            id: "k-1".into(),
            user_id: "9".into(),
            username: "kicker".into(),
            display_name: "Kicker".into(),
            color: None,
            badges: vec![],
            timestamp: "0".into(),
            content: "hi brandon".into(),
            provider: "kick".into(),
            channel: "SomeSlug".into(),
            emotes: vec![],
            tags: HashMap::new(),
            layout: Default::default(),
            segments: vec![],
            metadata: Default::default(),
        };
        let n = notice_for(&msg, MentionKind::Mention, None);
        assert_eq!(n.title, "Kicker mentioned you in someslug");
        assert_eq!(n.action, Some(NoticeAction::Mention { id: "k-1".into(), channel: "kick:someslug".into() }));
        assert_eq!(n.reply, None);
    }
}
