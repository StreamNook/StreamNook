//! The phone chats (under the player and on the Chats screen) and the "Add a
//! chat" sheet. The rules live in `services::chat_tabs`; every change is
//! emitted as `chat-tabs` and answers which rooms the shell must join and leave.

use crate::models::settings::AppState;
use crate::models::stream::TwitchStream;
use crate::rt::AppHandle;
use crate::services::chat_tabs::{self, ChatRoomInput, ChatTab, ChatTabsChange, ChatTabsView};
use crate::services::providers::watch_urls::{classify, WatchTarget};
use serde::Serialize;
use std::sync::OnceLock;
use tauri::{Emitter, State};

static APP: OnceLock<AppHandle> = OnceLock::new();

fn emit(app: &AppHandle, view: &ChatTabsView) {
    let _ = APP.set(app.clone());
    if let Err(e) = app.emit(chat_tabs::EVENT, view) {
        log::debug!("[ChatTabs] emit failed: {e}");
    }
}

/// Sends a change's view, and keeps the live line of every Twitch room joined
/// or left with it: the tabs' live dots and the Chats screen's play pill read
/// it, and `channel_state` batches every watched room into one poll a minute.
/// Other platforms ride the live poller, which reads the tabs itself.
fn settle(app: &AppHandle, change: &ChatTabsChange) {
    emit(app, &change.view);
    let watch = |tab: &ChatTab| tab.provider == "twitch" && tab.channel_id.as_deref().is_some_and(|id| !id.is_empty());
    let join: Vec<ChatTab> = change.acquire.iter().filter(|t| watch(t)).cloned().collect();
    let leave: Vec<ChatTab> = change.release.iter().filter(|t| watch(t)).cloned().collect();
    if !join.is_empty() || !leave.is_empty() {
        tauri::async_runtime::spawn(async move {
            for tab in leave {
                crate::services::channel_state::unwatch(&tab.login, true).await;
            }
            for tab in join {
                let id = tab.channel_id.clone().unwrap_or_default();
                let state = crate::services::channel_state::watch(&tab.login, &id, true).await;
                if let Some(live) = crate::services::channel_state::is_live(&tab.login).await {
                    note_live(&tab.channel, live, state.viewer_count);
                }
            }
        });
    }
    crate::services::chat_dock_live::wake();
}

/// A room's live line moved (`channel_state` for Twitch, the live poller for
/// other platforms); a tab showing it is redrawn.
pub fn note_live(channel: &str, live: bool, viewer_count: Option<u64>) {
    let (change, changed) = chat_tabs::apply(|t| (Default::default(), t.note_live(channel, live, viewer_count)));
    if let (true, Some(app)) = (changed, APP.get()) {
        emit(app, &change.view);
    }
}

/// Unread mentions in a room, from the mentions hub; feeds the tab's count and
/// the Chats tab's badge.
pub fn note_mentions(channel: &str, unread: u32) {
    let (change, changed) = chat_tabs::apply(|t| (Default::default(), t.note_mentions(channel, unread)));
    if let (true, Some(app)) = (changed, APP.get()) {
        emit(app, &change.view);
    }
}

/// The open rooms and the one on screen (the saved chats, restored, at launch).
#[tauri::command]
pub fn chat_tabs_get(app: AppHandle) -> ChatTabsView {
    let _ = APP.set(app);
    chat_tabs::view()
}

/// Points the stream tab at the watched stream, or drops it with `None` when
/// the player closed (a kept one stays as one of your chats).
#[tauri::command]
pub fn chat_tabs_sync_stream(app: AppHandle, room: Option<ChatRoomInput>) -> ChatTabsChange {
    let (change, ()) = chat_tabs::apply(|t| (t.sync_stream(room.as_ref()), ()));
    settle(&app, &change);
    change
}

/// Opens a room as one of your chats and puts it on screen. A non-Twitch room
/// that knows only its slug gets its real name and picture from the platform
/// a moment later, as another `chat-tabs` event.
#[tauri::command]
pub fn chat_tabs_add(app: AppHandle, room: ChatRoomInput) -> ChatTabsChange {
    let (change, needs_meta) = chat_tabs::apply(|t| t.add(&room));
    settle(&app, &change);
    if needs_meta {
        if let Some(tab) = chat_tabs::ChatTabs::tab_for(&room, false) {
            tauri::async_runtime::spawn(async move {
                match crate::commands::provider_browse::provider_channel_meta(tab.provider.clone(), tab.login.clone())
                    .await
                {
                    Ok(meta) => {
                        let (patched, changed) = chat_tabs::apply(|t| {
                            let changed = t.patch(&tab.channel, Some(meta.user_name), meta.profile_image_url);
                            (Default::default(), changed)
                        });
                        if changed {
                            emit(&app, &patched.view);
                        }
                    }
                    Err(e) => log::debug!("[ChatTabs] no channel meta for {}: {e}", tab.channel),
                }
            });
        }
    }
    change
}

/// Closes one of your chats (on the stream tab: stops keeping it).
#[tauri::command]
pub fn chat_tabs_remove(app: AppHandle, channel: String) -> ChatTabsChange {
    let (change, ()) = chat_tabs::apply(|t| (t.remove(&channel), ()));
    settle(&app, &change);
    change
}

/// Puts an open room on screen.
#[tauri::command]
pub fn chat_tabs_set_active(app: AppHandle, channel: String) -> ChatTabsView {
    let (change, changed) = chat_tabs::apply(|t| (Default::default(), t.set_active(&channel)));
    if changed {
        emit(&app, &change.view);
    }
    change.view
}

/// Joins every chat not joined yet: the Chats screen's first look, or the
/// return from a park.
#[tauri::command]
pub fn chat_tabs_connect(app: AppHandle) -> ChatTabsChange {
    let (change, ()) = chat_tabs::apply(|t| (t.connect(), ()));
    settle(&app, &change);
    change
}

/// Leaves every chat after a while in the background with no player (Android).
#[tauri::command]
pub fn chat_tabs_park(app: AppHandle) -> ChatTabsChange {
    let (change, ()) = chat_tabs::apply(|t| (t.park(), ()));
    settle(&app, &change);
    change
}

/// The rooms on screen right now (the Chats tab and the chat under the player),
/// empty when none is. Mentions arriving in a room you are looking at are read
/// already; the mentions hub takes this set as it is.
#[tauri::command]
pub fn chat_tabs_visible(rooms: Vec<String>) {
    let rooms: Vec<String> = rooms.into_iter().map(|r| r.trim().to_lowercase()).filter(|r| !r.is_empty()).collect();
    log::debug!("[ChatTabs] on screen: {rooms:?}");
}

/// Keeps the stream's chat after the player closes, or lets it close with it.
#[tauri::command]
pub fn chat_tabs_keep(app: AppHandle, channel: String, keep: bool) -> ChatTabsView {
    let (change, changed) = chat_tabs::apply(|t| (Default::default(), t.keep(&channel, keep)));
    if changed {
        emit(&app, &change.view);
    }
    change.view
}

/// Rebuilds the Twitch chat with every room this window holds (see
/// `services::chat_recovery`). `force` is a deliberate reload, never throttled.
/// Answers the bridge's port when a cycle ran, also emitted as `chat-rebuilt`.
#[tauri::command]
pub async fn rebuild_chat(
    app: AppHandle,
    window: crate::rt::Window,
    state: State<'_, AppState>,
    force: Option<bool>,
    reason: Option<String>,
) -> Result<Option<u16>, String> {
    let reason = reason.unwrap_or_else(|| "manual reload".to_string());
    let rebuilt = crate::services::chat_recovery::rebuild(&state, window.label(), force.unwrap_or(false), &reason)
        .await
        .map_err(|e| e.to_string())?;
    Ok(rebuilt.map(|r| {
        if let Err(e) = app.emit(crate::services::chat_recovery::EVENT, &r) {
            log::debug!("[ChatRecovery] emit failed: {e}");
        }
        r.port
    }))
}

/// The phones' "add a chat" picker: your live follows, then your offline
/// follows, from the Following snapshot (no request). `platforms` keeps the
/// ones the shell can chat on; `open` names the chats open now, read from the
/// tabs when left out.
#[tauri::command]
pub async fn chat_picker(
    platforms: Option<Vec<String>>,
    open: Option<Vec<String>>,
) -> crate::services::chat_picker::ChatPicker {
    let snap = crate::services::home_snapshot::snapshot().await;
    let open: std::collections::HashSet<String> = match open {
        Some(keys) => keys.into_iter().map(|k| k.trim().to_lowercase()).collect(),
        None => chat_tabs::view().tabs.into_iter().map(|t| t.tab.channel).collect(),
    };
    crate::services::chat_picker::build(&snap.following, &snap.last_broadcasts, platforms.as_deref(), &open)
}

/// One channel the "Add a chat" sheet lists.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ChatRoomRow {
    pub provider: String,
    /// The key the room's tab would get, so the sheet can mark rooms already open.
    pub key: String,
    pub user_login: String,
    /// The numeric Twitch id; empty on other platforms.
    pub user_id: String,
    pub user_name: String,
    pub profile_image_url: Option<String>,
    pub is_live: bool,
    pub viewer_count: u64,
    pub game_name: String,
    /// The row came from a pasted link, not a search.
    pub pasted: bool,
}

impl ChatRoomRow {
    fn twitch(s: TwitchStream) -> Self {
        let login = s.user_login.to_lowercase();
        Self {
            provider: "twitch".into(),
            key: login.clone(),
            user_login: login,
            user_id: s.user_id,
            user_name: s.user_name,
            profile_image_url: s.profile_image_url,
            is_live: s.is_live.unwrap_or(false),
            viewer_count: u64::from(s.viewer_count),
            game_name: s.game_name,
            pasted: false,
        }
    }

    fn provider(provider: &str, s: crate::models::provider_stream::ProviderStream) -> Self {
        let login = crate::services::providers::key::normalize_channel(provider, &s.user_login);
        Self {
            provider: provider.into(),
            key: crate::services::providers::key::make_key(provider, &login).to_lowercase(),
            user_login: login,
            user_id: String::new(),
            user_name: s.user_name,
            profile_image_url: s.profile_image_url,
            is_live: s.is_live,
            viewer_count: u64::from(s.viewer_count),
            game_name: s.game_name,
            pasted: false,
        }
    }

    fn pasted_kick(slug: &str) -> Self {
        Self {
            provider: "kick".into(),
            key: format!("kick:{slug}"),
            user_login: slug.into(),
            user_id: String::new(),
            user_name: slug.into(),
            profile_image_url: None,
            is_live: false,
            viewer_count: 0,
            game_name: String::new(),
            pasted: true,
        }
    }
}

/// A Kick room named by a pasted kick.com link (scheme, `www.` and `@`
/// optional) or typed as `kick:slug` / `kick/slug`.
pub fn kick_room_in(query: &str) -> Option<String> {
    let q = query.trim();
    let typed = q
        .strip_prefix("kick:")
        .or_else(|| q.strip_prefix("kick/"))
        .or_else(|| q.strip_prefix("Kick:"))
        .or_else(|| q.strip_prefix("Kick/"));
    let slug = match typed {
        Some(rest) => rest.trim().trim_start_matches('@').to_lowercase(),
        None if q.to_ascii_lowercase().contains("kick.com/") => match classify(q) {
            WatchTarget::Provider { provider: "kick", channel } => channel.trim_start_matches('@').to_string(),
            _ => return None,
        },
        None => return None,
    };
    let valid = !slug.is_empty() && slug.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    valid.then_some(slug)
}

/// Live channels first, biggest audience first, then everyone offline in the
/// order the platforms answered: Twitch's channel search ranks by name, so a
/// live room would otherwise sink under offline lookalikes.
pub fn order_rooms(rows: Vec<ChatRoomRow>) -> Vec<ChatRoomRow> {
    let (mut live, offline): (Vec<_>, Vec<_>) = rows.into_iter().partition(|r| r.is_live);
    live.sort_by(|a, b| b.viewer_count.cmp(&a.viewer_count));
    live.extend(offline);
    live
}

/// The room a pasted link names, answered at once without a search.
#[tauri::command]
pub fn chat_room_from_link(query: String) -> Option<ChatRoomRow> {
    kick_room_in(&query).map(|slug| ChatRoomRow::pasted_kick(&slug))
}

/// Channels to open a chat with: Twitch and Kick searched together (offline
/// channels included, since chat is joinable either way), one platform failing
/// never emptying the other, ordered live first. A pasted Kick link answers
/// that one room without searching.
#[tauri::command]
pub async fn search_chat_rooms(state: State<'_, AppState>, query: String) -> Result<Vec<ChatRoomRow>, String> {
    if let Some(row) = chat_room_from_link(query.clone()) {
        return Ok(vec![row]);
    }
    let q = query.trim();
    if q.chars().count() < 2 {
        return Ok(Vec::new());
    }
    let state: &AppState = &state;
    let (twitch, kick) = tokio::join!(
        crate::services::twitch_service::TwitchService::search_channels(state, q),
        async {
            match crate::services::providers::registry().await.get_source("kick") {
                Some(src) => src.search(q).await.map(|page| page.streams),
                None => Ok(Vec::new()),
            }
        }
    );
    let mut rows = Vec::new();
    match twitch {
        Ok(found) => rows.extend(found.into_iter().map(ChatRoomRow::twitch)),
        Err(e) => log::warn!("[AddChat] twitch search failed: {e}"),
    }
    match kick {
        Ok(found) => rows.extend(found.into_iter().map(|s| ChatRoomRow::provider("kick", s))),
        Err(e) => log::warn!("[AddChat] kick search failed: {e}"),
    }
    Ok(order_rooms(rows))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(login: &str, live: bool, viewers: u64) -> ChatRoomRow {
        ChatRoomRow {
            provider: "twitch".into(),
            key: login.into(),
            user_login: login.into(),
            user_id: String::new(),
            user_name: login.into(),
            profile_image_url: None,
            is_live: live,
            viewer_count: viewers,
            game_name: String::new(),
            pasted: false,
        }
    }

    #[test]
    fn live_rooms_lead_by_audience_then_offline_keep_their_order() {
        let ordered = order_rooms(vec![
            row("off1", false, 0),
            row("small", true, 10),
            row("off2", false, 0),
            row("big", true, 900),
        ]);
        let logins: Vec<_> = ordered.iter().map(|r| r.user_login.as_str()).collect();
        assert_eq!(logins, ["big", "small", "off1", "off2"]);
    }

    #[test]
    fn kick_links_and_prefixes_name_a_room() {
        assert_eq!(kick_room_in("https://kick.com/xqc").as_deref(), Some("xqc"));
        assert_eq!(kick_room_in("www.kick.com/@JoeWo").as_deref(), Some("joewo"));
        assert_eq!(kick_room_in("kick.com/Some_One-1?x=1").as_deref(), Some("some_one-1"));
        assert_eq!(kick_room_in("kick:xqc").as_deref(), Some("xqc"));
        assert_eq!(kick_room_in("kick/@xqc").as_deref(), Some("xqc"));
        assert_eq!(kick_room_in("kick.com/video/abc"), None);
        assert_eq!(kick_room_in("xqc"), None);
        assert_eq!(kick_room_in("kick: two words"), None);
    }

    #[test]
    fn a_pasted_row_says_so() {
        let r = chat_room_from_link("kick.com/xqc".into()).unwrap();
        assert!(r.pasted);
        assert_eq!((r.provider.as_str(), r.key.as_str()), ("kick", "kick:xqc"));
    }
}
