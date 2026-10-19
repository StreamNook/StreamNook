//! The desktop notification for a mention, a reply to you, or one of your
//! highlight phrases, while StreamNook is not the window in use.
//! `mention_inbox` decides what counts and whether to announce it (setting,
//! streamer mode, the per-chat cooldown); this only builds and delivers it.
//!
//! `chat_rules` already decides what a mention is, the same judgment the
//! mention sound (`mention_ping`) uses. This turns the ones worth a notice into
//! a `MirroredNotice` and hands it to `notification_mirror::deliver`, which
//! shows it as a system notification or a MultiChat card, or nowhere when the
//! main window is in use. The notice opens the chat at that message and, as a
//! Windows notification on Twitch, takes a reply posted as a threaded reply.

use crate::commands::notification_mirror::{MirroredNotice, NoticeAction, NoticeReply};
use crate::models::chat_layout::ChatMessage;
use crate::services::mention_inbox::{open_key, room_key, MentionKind};

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
        action: Some(NoticeAction::Mention { id: msg.id.clone(), channel: open_key(&msg.provider, &msg.channel) }),
        reply: twitch.then(|| NoticeReply::Chat {
            channel,
            broadcaster_id: msg.tags.get("room-id").cloned(),
            parent_id: msg.id.clone(),
            to_name: name,
        }),
    }
}

/// Show the notification for an entry `mention_inbox` decided to announce.
pub fn deliver(msg: &ChatMessage, kind: MentionKind) {
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
    use std::collections::HashMap;

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
        // The click opens the chat by its own case; the room key stays lowercase.
        assert_eq!(n.action, Some(NoticeAction::Mention { id: "k-1".into(), channel: "kick:SomeSlug".into() }));
        assert_eq!(n.reply, None);
    }
}
