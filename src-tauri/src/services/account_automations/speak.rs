//! A rule's Speak action: read a line aloud in the one window that claims it.
//!
//! Streamer mode skips unless the action says "Also while streamer mode is
//! on". Every window that could play gets `automations://speak`; the first to
//! claim the id speaks it (the mention ping's pattern, the same claim as a
//! Notify card). With no player, or when none claims in time, a desktop
//! system notification shows the text (answer 9: a system notification only
//! when no window can play). When the player cannot speak (Android's
//! `{ spoken: false }`, a Linux build with no voices, an iPhone with none),
//! the player shows the text as a toast itself. Nothing is ever posted to chat.

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;

use super::activity::{EntryKind, NewEntry};
use super::notify::NotifyRequest;
use super::Engine;

/// What a Speak action asks for.
#[derive(Clone, Debug, PartialEq)]
pub struct SpeakRequest {
    pub rule_id: Option<String>,
    /// The line, already expanded and cleaned (`template::clean_speech`).
    pub text: String,
    /// Also while streamer mode is on.
    pub on_stream: bool,
    /// The channel it is about (a system notification's click opens it).
    pub channel: String,
}

/// `automations://speak`'s payload.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SpeakPayload {
    pub id: u64,
    pub text: String,
    pub channel: String,
}

/// Where one line goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SpeakRoute {
    Nothing,
    /// A desktop system notification with the text.
    System,
    /// To the players; the first to claim speaks.
    Players,
}

/// Pure routing.
pub(crate) fn route(
    streamer: bool,
    on_stream: bool,
    has_players: bool,
    desktop: bool,
) -> SpeakRoute {
    if streamer && !on_stream {
        return SpeakRoute::Nothing;
    }
    if has_players {
        SpeakRoute::Players
    } else if desktop {
        SpeakRoute::System
    } else {
        SpeakRoute::Nothing
    }
}

/// A text a page or the Android plugin may be asked to speak: one line,
/// trimmed, 1 to 300 characters. None for nothing to say.
pub fn speakable(text: &str) -> Option<String> {
    let line = super::template::clean_speech(text);
    (!line.is_empty()).then_some(line)
}

/// Speak one line.
pub(crate) async fn deliver(engine: &Arc<Engine>, request: SpeakRequest) {
    let streamer = engine.local.streamer_mode();
    // A player is wanted even in streamer mode when the action allows it.
    let targets = if streamer && !request.on_stream {
        Vec::new()
    } else {
        engine.local.notify_targets()
    };
    let decided = route(
        streamer,
        request.on_stream,
        !targets.is_empty(),
        cfg!(desktop),
    );
    let log = |kind: EntryKind, text: String| NewEntry {
        kind,
        rule_id: request.rule_id.clone().unwrap_or_default(),
        rule_name: request
            .rule_id
            .as_ref()
            .and_then(|id| engine.compiled().rule(id).map(|r| r.rule.name.clone()))
            .unwrap_or_default(),
        channel: if request.channel.is_empty() {
            String::new()
        } else {
            crate::services::chat_rules::channel_filter_key("twitch", &request.channel)
        },
        text,
    };
    let as_notice = || NotifyRequest {
        rule_id: request.rule_id.clone(),
        title: "StreamNook".into(),
        body: request.text.clone(),
        toast: false,
        system: true,
        sound: None,
        volume: 1.0,
        channel: request.channel.clone(),
        settings_link: false,
    };
    match decided {
        SpeakRoute::Nothing => {
            if streamer {
                engine.log(log(EntryKind::Skipped, "Skipped: streamer mode".into()));
            }
        }
        SpeakRoute::System => {
            let id = engine
                .notifier
                .issue(std::time::Instant::now(), Duration::from_secs(1));
            engine.local.system_notice(&as_notice(), id);
            engine.log(log(
                EntryKind::Acted,
                format!("Showed what it would say: {}", request.text),
            ));
        }
        SpeakRoute::Players => {
            let window = engine.timings.notify_confirm;
            let id = engine.notifier.issue(std::time::Instant::now(), window);
            engine.local.emit_speak(
                &targets,
                &SpeakPayload {
                    id,
                    text: request.text.clone(),
                    channel: request.channel.clone(),
                },
            );
            engine.log(log(EntryKind::Acted, format!("Spoke: {}", request.text)));
            if cfg!(desktop) {
                let engine = engine.clone();
                let notice = as_notice();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(window).await;
                    // Still unclaimed: no window spoke it.
                    if engine.notifier.claim(id, Duration::MAX) {
                        engine.local.system_notice(&notice, id);
                    }
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routing_follows_streamer_mode_players_and_the_desktop() {
        // Streamer mode skips unless the action says otherwise.
        assert_eq!(route(true, false, true, true), SpeakRoute::Nothing);
        assert_eq!(route(true, true, true, true), SpeakRoute::Players);
        assert_eq!(route(false, false, true, true), SpeakRoute::Players);
        // No player: a system notification on the desktop, nothing elsewhere.
        assert_eq!(route(false, false, false, true), SpeakRoute::System);
        assert_eq!(route(false, false, false, false), SpeakRoute::Nothing);
        assert_eq!(route(true, true, false, true), SpeakRoute::System);
    }

    #[test]
    fn a_speakable_text_is_one_trimmed_line() {
        assert_eq!(speakable("  hi \n there ").as_deref(), Some("hi there"));
        assert_eq!(speakable(" \n\t "), None);
        assert_eq!(speakable(&"b".repeat(999)).unwrap().chars().count(), 300);
    }
}
