//! Reading and editing the links between one streamer's channels on different
//! platforms. The rules live in `services::channel_link_service`; this layer
//! only locks, persists and answers.
//!
//! Both commands take `(provider, channel)` as a PAIR rather than a pre-joined
//! composite key. A second place that joins and splits keys is a second codec,
//! and the app has already shipped bugs from one of those drifting from the
//! other.

use crate::models::settings::{AppState, ChannelLinkGroup, LinkMember};
use crate::services::channel_link_service as links;
use serde::{Deserialize, Serialize};
use tauri::State;

/// What the chat panel needs in one answer: the group, and the OTHER platforms'
/// channels for the one being watched. `companions` depends on which member you
/// are asking from, so it is derived here rather than left to the caller to
/// work out a second time.
#[derive(Serialize)]
pub struct ChannelLinkView {
    pub group: Option<ChannelLinkGroup>,
    pub companions: Vec<LinkMember>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkAction {
    /// This channel is the same streamer.
    Link,
    /// It is not (or no longer) the same streamer.
    Unlink,
    /// It is not this streamer, and never suggest it again.
    Dismiss,
}

fn view(settings: &crate::models::settings::Settings, provider: &str, channel: &str) -> ChannelLinkView {
    ChannelLinkView {
        group: links::group_for(settings, provider, channel),
        companions: links::companions_of(settings, provider, channel),
    }
}

/// The streamer that owns this channel, as seen FROM this channel.
#[tauri::command]
pub async fn get_channel_links(
    provider: String,
    channel: String,
    state: State<'_, AppState>,
) -> Result<ChannelLinkView, String> {
    let settings = state.settings.lock().map_err(|e| e.to_string())?;
    Ok(view(&settings, &provider, &channel))
}

/// Attach, detach, or refuse a platform for the streamer that owns
/// `(provider, channel)`.
#[tauri::command]
pub async fn update_channel_link(
    provider: String,
    channel: String,
    member: LinkMember,
    action: LinkAction,
    state: State<'_, AppState>,
) -> Result<ChannelLinkView, String> {
    if channel.trim().is_empty() || member.channel.trim().is_empty() {
        return Err("channel is required".to_string());
    }
    let (snapshot, result) = {
        let mut settings = state.settings.lock().map_err(|e| e.to_string())?;
        match action {
            LinkAction::Link => {
                links::link(&mut settings, &provider, &channel, member.clone());
            }
            LinkAction::Unlink => {
                links::unlink(&mut settings, &member.provider, &member.channel);
            }
            LinkAction::Dismiss => {
                links::dismiss(&mut settings, &provider, &channel, &member.provider, &member.channel);
            }
        }
        (settings.clone(), view(&settings, &provider, &channel))
    };
    crate::commands::settings::write_settings_to_disk(&snapshot)?;
    Ok(result)
}

/// Look for this streamer on another platform, and emit a suggestion if one
/// turns up. Kick only — see `channel_link_service::probe` for why YouTube is
/// never searched in the background.
///
/// Answers immediately and does the lookup on a task, so opening a stream never
/// waits on another platform's API. The result arrives on the `channel-links`
/// event.
#[tauri::command]
pub async fn probe_channel_links(
    provider: String,
    channel: String,
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let settings = {
        let s = state.settings.lock().map_err(|e| e.to_string())?;
        s.clone()
    };
    tauri::async_runtime::spawn(async move {
        crate::services::channel_link_service::probe(&app, &settings, &provider, &channel).await;
    });
    Ok(())
}
