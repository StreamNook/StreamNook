use crate::models::chat_layout::ChatMessage;
use crate::models::settings::AppState;
use crate::services::chat_logger_service::ChatLoggerService;
use crate::services::chat_service::{ChatService, SendResult};
use crate::services::irc_service::IrcService;
use crate::services::providers::{registry, SendCapability, SendOutcome};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::State;

/// The folder chat logs are written to right now (the custom folder when one
/// is set, else the default under the app data dir), for the settings UI.
/// While logging is enabled the folder is created, so opening it always works.
#[tauri::command]
pub async fn get_chat_log_dir(state: State<'_, AppState>) -> Result<String, String> {
    let (folder, enabled) = state
        .settings
        .lock()
        .map(|s| (s.chat_logging.folder.clone(), s.chat_logging.enabled))
        .map_err(|_| "settings unavailable".to_string())?;
    let Some(dir) = ChatLoggerService::resolve_dir(&folder) else {
        return Ok(String::new());
    };
    if enabled {
        let _ = std::fs::create_dir_all(&dir);
    }
    Ok(dir.to_string_lossy().to_string())
}

/// `claim` (default true) marks the calling window as a real chat consumer
/// that will later balance itself with `leave_chat_channel`. Pass false for
/// ensure-only calls (the stream-start warm-up) so the channel can still PART
/// once its actual consumers are gone. `reattach` (default false) is for the
/// reconnect path, whose store still holds channels: it suppresses the sweep
/// of this window's recorded claims that a fresh first-acquire start performs
/// (that sweep is what garbage-collects claims left behind by a previous JS
/// context of the same window, e.g. before a webview reload).
#[tauri::command]
pub async fn start_chat(
    channel: String,
    claim: Option<bool>,
    reattach: Option<bool>,
    window: tauri::Window,
    state: State<'_, AppState>,
) -> Result<u16, String> {
    ChatService::start(
        &channel,
        &state,
        claim.unwrap_or(true),
        reattach.unwrap_or(false),
        window.label(),
    )
    .await
    .map_err(|e| e.to_string())
}

/// Connect a non-Twitch provider's chat for this window. Brings up (or reuses)
/// the shared local-WS bridge and returns its port so the frontend attaches to
/// the same socket the Twitch path uses. Errors if no adapter is registered for
/// the provider yet. Twitch keeps its own `start_chat` path.
#[tauri::command]
pub async fn provider_chat_connect(
    provider: String,
    channel: String,
    window: tauri::Window,
) -> Result<u16, String> {
    let port = IrcService::ensure_local_ws_bridge()
        .await
        .map_err(|e| e.to_string())?;
    match registry().await.get(&provider) {
        Some(p) => {
            p.connect(&channel, window.label())
                .await
                .map_err(|e| e.to_string())?;
            Ok(port)
        }
        None => Err(format!("provider '{}' is not available yet", provider)),
    }
}

/// The shared local-WS bridge's port, with no platform connect in the way.
/// A window whose first channel's own connect can fail (a YouTube or TikTok
/// channel that is not live, or a Twitch start that did not come up) still gets
/// its socket, so every other channel in it keeps chatting.
#[tauri::command]
pub async fn chat_bridge_port() -> Result<u16, String> {
    IrcService::ensure_local_ws_bridge()
        .await
        .map_err(|e| e.to_string())
}

/// Drop this window's claim on a non-Twitch source; the adapter disconnects when
/// the last consumer leaves.
#[tauri::command]
pub async fn provider_chat_disconnect(
    provider: String,
    channel: String,
    window: tauri::Window,
) -> Result<(), String> {
    if let Some(p) = registry().await.get(&provider) {
        p.disconnect(&channel, window.label())
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Send a message to a non-Twitch source via its adapter.
#[tauri::command]
pub async fn provider_send_message(
    provider: String,
    channel: String,
    text: String,
    reply_to: Option<String>,
) -> Result<SendOutcome, String> {
    match registry().await.get(&provider) {
        Some(p) => p
            .send(&channel, &text, reply_to.as_deref())
            .await
            .map_err(|e| e.to_string()),
        None => Ok(SendOutcome {
            message_id: None,
            is_sent: false,
            drop_reason: Some("provider unavailable".into()),
        }),
    }
}

/// Whether the user can currently send to a non-Twitch source (read-only,
/// sendable, or needs sign-in). Defaults to read-only when no adapter exists.
#[tauri::command]
pub async fn provider_send_capability(
    provider: String,
    channel: String,
) -> Result<SendCapability, String> {
    Ok(match registry().await.get(&provider) {
        Some(p) => p.send_capability(&channel).await,
        None => SendCapability::ReadOnly,
    })
}

/// Injected JS in the hidden Kick resolver webview reports the resolved chatroom
/// id here (the Cloudflare-clearing fallback for the channel-id lookup).
#[tauri::command]
pub async fn report_kick_chatroom(
    label: String,
    chatroom_id: u64,
    sub_badges: Vec<crate::services::providers::kick::KickSubBadge>,
    meta: Option<crate::services::providers::kick::KickChannelMeta>,
) {
    let subs = sub_badges.into_iter().map(|b| (b.months, b.src)).collect();
    crate::services::providers::kick::resolve_pending(&label, chatroom_id, subs, meta).await;
}

/// The Kick account-sync webview reports the user's followed + subscribed
/// channels here, read from the website session in page context.
#[tauri::command]
pub async fn report_kick_follows(
    label: String,
    report: crate::services::providers::kick_account::KickImportReport,
) {
    crate::services::providers::kick_account::resolve_pending(&label, report).await;
}

/// The resolver webview reports the channel's native Kick emotes here, separately
/// from the chrome above (their fetch is slower, so it must not delay name /
/// viewers / uptime / chat connect).
#[tauri::command]
pub async fn report_kick_emotes(
    label: String,
    native_emotes: Vec<crate::services::providers::kick_emotes::KickNativeEmoteEntry>,
) {
    crate::services::providers::kick::resolve_emotes_pending(&label, native_emotes).await;
}

/// Live Kick channel metadata (viewers / uptime start_time / title / avatar)
/// captured during channel resolve, for the MultiChat chrome. Returns null until
/// the channel has been resolved.
#[tauri::command]
pub fn get_kick_channel_meta(
    slug: String,
) -> Option<crate::services::providers::kick::KickChannelMeta> {
    crate::services::providers::kick::channel_meta(&slug)
}

/// Live YouTube channel metadata (channel name / title / viewers / uptime start /
/// avatar) scraped from the watch page during resolve, for the MultiChat chrome.
/// `slug` is the source identifier (an `@handle`, `UC…` channel id, or video id).
/// Returns null until the live video has been resolved.
#[tauri::command]
pub async fn get_youtube_channel_meta(
    slug: String,
) -> Option<crate::services::providers::youtube::YouTubeChannelMeta> {
    // Re-resolves past the freshness window instead of serving the cache blindly:
    // the MultiChat viewer counter polls this, and an ageless read meant it showed
    // the count captured when chat resolved, unchanged, forever.
    crate::services::providers::youtube_media::channel_meta_refreshed(&slug).await
}

/// The `UC…` channel id behind a legacy `/c/NAME` or `/user/NAME` YouTube link.
/// `path` is `c/NAME` or `user/NAME`, as the frontend's link parser returns it;
/// the error is a line fit for the add box that asked.
#[tauri::command]
pub async fn resolve_youtube_legacy_channel(path: String) -> Result<String, String> {
    crate::services::providers::youtube_media::resolve_legacy_channel(&path)
        .await
        .map_err(|e| e.to_string())
}

/// The slug Kick knows a typed channel name by, trying both spellings a name
/// with `_` or `-` can have. When Kick can't be reached the name comes back as
/// given; the error is a line fit for the add box that asked.
#[tauri::command]
pub async fn resolve_kick_slug(name: String) -> Result<String, String> {
    crate::services::providers::kick::resolve_slug(&name).await
}

/// Live TikTok creator metadata (name / title / viewers / avatar) resolved from the
/// profile page + webcast room info, for the MultiChat chrome. `slug` is the TikTok
/// handle. Returns null until the LIVE has been resolved.
#[tauri::command]
pub fn get_tiktok_channel_meta(
    slug: String,
) -> Option<crate::services::providers::tiktok::TikTokChannelMeta> {
    crate::services::providers::tiktok::channel_meta(&slug)
}

/// Sign in to TikTok in the login overlay. The session plays age-restricted
/// LIVEs and reads who the account follows is live; see `tiktok_auth_service`
/// for where it stops.
#[tauri::command]
pub async fn tiktok_connect(state: tauri::State<'_, crate::models::settings::AppState>) -> Result<(), String> {
    crate::services::tiktok_auth_service::connect()
        .await
        .map_err(|e| e.to_string())?;
    // Look NOW rather than at the poller's next 90 s tick, so the Following tab
    // fills as the sign-in window closes.
    if let Some(app) = crate::services::providers::app_handle() {
        let state_for_refresh = (*state).clone();
        tauri::async_runtime::spawn(async move {
            crate::services::provider_live_service::refresh_provider(app, state_for_refresh, "tiktok").await;
        });
    }
    Ok(())
}

/// Sign out of TikTok, clearing the session, the sign-in profile and what was
/// read with it. Follows made in StreamNook itself are the user's own and stay.
#[tauri::command]
pub async fn tiktok_disconnect() -> Result<(), String> {
    crate::services::tiktok_auth_service::disconnect().await;
    Ok(())
}

#[tauri::command]
pub async fn tiktok_is_connected() -> bool {
    crate::services::tiktok_auth_service::is_connected()
}

/// Sign into YouTube (webview-session): opens a login window, harvests the session
/// cookies once the user finishes, so send/moderation can drive the private API.
#[tauri::command]
pub async fn youtube_connect() -> Result<(), String> {
    crate::services::youtube_auth_service::connect()
        .await
        .map_err(|e| e.to_string())
}

/// Sign out of YouTube. Same policy as Kick: credentials and imported channels
/// go, an open player and read-only chat stay. The auth service already signs the
/// web profile out and clears the moderation cache.
// `async` on purpose, here and on the Kick twins below: a non-async command
// runs on the UI thread, and these touch the OS keychain (which can block on
// a SecurityAgent prompt), clear a whole webview profile and write settings.
// On macOS that froze the window for as long as the keychain took to answer.
// Async commands that borrow `State` must return a `Result`.
#[tauri::command]
pub async fn youtube_disconnect(state: State<'_, AppState>) -> Result<(), String> {
    crate::services::youtube_auth_service::disconnect().await;
    if let Err(e) = crate::commands::provider_browse::clear_imported_follows("youtube", &state) {
        log::warn!("[youtube] could not clear imported follows: {}", e);
    }
    Ok(())
}

// The first call lazily loads the session from the keychain; keep that off the
// UI thread too.
#[tauri::command]
pub async fn youtube_is_connected() -> bool {
    crate::services::youtube_auth_service::is_connected()
}

/// The connected YouTube account's name for the Connections UI, or null. Fetches it
/// once (then cached) so an already-connected session gets its name without a reconnect.
#[tauri::command]
pub async fn youtube_account_name() -> Option<String> {
    crate::services::youtube_auth_service::account_name_lazy().await
}

/// Re-read WHICH YouTube channel the signed-in session acts as. True when it
/// changed, so the caller can re-import that channel's subscriptions.
///
/// Every in-app YouTube window (the sign-in overlay, and the `/join` membership
/// panel) opens youtube.com in the app's own YouTube profile, and YouTube puts a
/// full account switcher in both. A user with a brand account can therefore change
/// channel in there, and that switch leaves NO trace in the cookie jar, so nothing
/// in the app noticed it. Called when such a window closes.
#[tauri::command]
pub async fn youtube_refresh_identity() -> bool {
    crate::services::youtube_auth_service::resync_identity().await
}

/// Delete a single YouTube chat message (`message_id` is the live-chat item id).
/// `channel` is the source identifier (the same key the chat slice uses).
#[tauri::command]
pub async fn youtube_delete_message(channel: String, message_id: String) -> Result<(), String> {
    crate::services::providers::youtube::delete_message(&channel, &message_id)
        .await
        .map_err(|e| e.to_string())
}

/// Time out (`duration_seconds` Some — YouTube's fixed timeout) or permanently ban
/// (`duration_seconds` None) a user by their channel id on `channel`'s stream.
#[tauri::command]
pub async fn youtube_ban_user(
    channel: String,
    target_channel_id: String,
    duration_seconds: Option<u32>,
) -> Result<(), String> {
    crate::services::providers::youtube::ban_user(&channel, &target_channel_id, duration_seconds)
        .await
        .map_err(|e| e.to_string())
}

/// Lift a ban / hide on a YouTube user by their channel id.
#[tauri::command]
pub async fn youtube_unban_user(channel: String, target_channel_id: String) -> Result<(), String> {
    crate::services::providers::youtube::unban_user(&channel, &target_channel_id)
        .await
        .map_err(|e| e.to_string())
}

/// Whether the connected YouTube account can moderate this channel's chat (gates the
/// mod controls). Probes a message's context menu; cached after the first answer.
#[tauri::command]
pub async fn youtube_can_moderate(channel: String) -> bool {
    crate::services::providers::youtube::can_moderate(&channel).await
}

/// Connect a Kick account (OAuth, Authorization Code + PKCE) so the user can send
/// Kick chat. Opens the browser to id.kick.com and waits for the loopback redirect.
#[tauri::command]
pub async fn kick_connect() -> Result<(), String> {
    crate::services::kick_auth_service::connect()
        .await
        .map_err(|e| e.to_string())
}

/// Sign out of Kick: credentials, the kick.com browser session, and the channels
/// that session imported.
///
/// Deliberately does NOT stop an open Kick player or an already-joined read-only
/// chat. Kick streams play fine signed out, so tearing playback down here would
/// destroy the thing the user is currently watching to make a point about
/// account state. Sending goes read-only, Following becomes the login wall, and
/// Twitch (and YouTube) are untouched.
#[tauri::command]
pub async fn kick_disconnect(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    crate::services::kick_auth_service::disconnect();
    // The OAuth token and the site session are two different credentials in two
    // different places. Clearing only the first left kick.com still logged in, so
    // "Disconnect" then "Connect" silently reused the old session. Deleting the
    // profile's folder did not reliably clear the second either: the browser
    // behind the profile can outlive its window and keep the session. So the
    // clear goes through a live webview on it (on macOS, Kick's cookies only;
    // see `sign_in_profile`).
    #[cfg(desktop)]
    crate::services::providers::kick_account::clear_site_session(&app).await;
    // The phone keeps the kick.com session in the overlay's app-global jar, not
    // a profile directory. Expire only Kick's cookies so Twitch stays signed in.
    #[cfg(target_os = "android")]
    crate::twitch_login_plugin::expire_cookies(&app, &["https://kick.com", "https://id.kick.com"]);
    if let Err(e) = crate::commands::provider_browse::clear_imported_follows("kick", &state) {
        log::warn!("[Kick] could not clear imported follows: {}", e);
    }
    Ok(())
}

#[tauri::command]
pub async fn kick_is_connected() -> bool {
    crate::services::kick_auth_service::is_connected()
}

/// The connected Kick account's username (for the Connections UI), or null.
#[tauri::command]
pub async fn kick_account_name() -> Option<String> {
    crate::services::kick_auth_service::account_name().await
}

/// Ban (omit duration) or time out (duration in minutes) a Kick user. Addressed by
/// numeric Kick user ids: the channel's broadcaster id + the target chatter's id.
#[tauri::command]
pub async fn kick_ban_user(
    broadcaster_user_id: u64,
    target_user_id: u64,
    duration_minutes: Option<u32>,
    reason: Option<String>,
) -> Result<(), String> {
    crate::services::providers::kick::ban_user(
        broadcaster_user_id,
        target_user_id,
        duration_minutes,
        reason,
    )
    .await
    .map_err(|e| e.to_string())
}

/// Lift a ban / timeout on a Kick user.
#[tauri::command]
pub async fn kick_unban_user(broadcaster_user_id: u64, target_user_id: u64) -> Result<(), String> {
    crate::services::providers::kick::unban_user(broadcaster_user_id, target_user_id)
        .await
        .map_err(|e| e.to_string())
}

/// Delete a single Kick chat message by its id.
#[tauri::command]
pub async fn kick_delete_message(message_id: String) -> Result<(), String> {
    crate::services::providers::kick::delete_message(&message_id)
        .await
        .map_err(|e| e.to_string())
}

/// Whether the connected Kick account can moderate this channel, so the mod
/// controls can be shown to a moderator who has not spoken yet. One cached probe
/// per channel; the frontend keeps the badge heuristic as a fallback.
/// The connected Kick account's standing in this channel: mod rights, how long
/// they have followed, and sub tenure. One cached request answers all three, so
/// the composer can tell whether a followers-only rule actually gates YOU.
#[tauri::command]
pub async fn kick_viewer_state(
    channel: String,
) -> crate::services::providers::kick::KickViewerState {
    crate::services::providers::kick::viewer_state(&channel).await
}

/// Recent Kick chat for seeding the pane on join, oldest-first.
///
/// Kick's socket only carries NEW traffic, so without this the pane opened empty
/// and an offline channel stayed empty. Returns an empty list on any failure:
/// scrollback is a nicety, never a reason to fail opening a channel.
#[tauri::command]
pub async fn kick_chat_history(
    channel: String,
) -> Vec<crate::models::chat_layout::ChatMessage> {
    // Backfill goes through the same rule engine as live rows (ignores drop,
    // highlights stamp), so a hidden user's scrollback is hidden too.
    let rules = crate::services::chat_rules::ChatRules::snapshot();
    crate::services::providers::kick::chat_history(&channel)
        .await
        .into_iter()
        .filter_map(|mut m| {
            if crate::services::chat_rules::ChatRules::evaluate(&mut m, &rules).drop {
                None
            } else {
                m.metadata.from_backfill = true;
                Some(m)
            }
        })
        .collect()
}

#[tauri::command]
pub async fn kick_can_moderate(channel: String) -> bool {
    crate::services::providers::kick::can_moderate(&channel).await
}

/// A Kick channel's 7TV emotes (channel set + 7TV globals) as an EmoteSet, for the
/// emote picker — parity with Twitch's `fetch_channel_emotes`.
#[tauri::command]
pub async fn get_kick_channel_emotes(slug: String) -> crate::services::emote_service::EmoteSet {
    crate::services::providers::kick_emotes::channel_emote_set(&slug).await
}

/// A YouTube channel's 7TV emotes (channel set + 7TV globals) as an EmoteSet,
/// for the emote picker. Separate from `get_youtube_channel_emojis`, which serves
/// YouTube's OWN channel emoji: a channel can have either, both, or neither.
#[tauri::command]
pub async fn get_youtube_channel_emotes(
    channel: String,
) -> crate::services::emote_service::EmoteSet {
    crate::services::providers::youtube_emotes::channel_emote_set(&channel).await
}

#[tauri::command]
pub async fn stop_chat() -> Result<(), String> {
    ChatService::stop().await.map_err(|e| e.to_string())
}

/// Who is signed in on a platform: display name, profile picture, and the id
/// that platform's chat identifies them by.
///
/// One call rather than a `*_account_name` plus a separate avatar lookup, because
/// all three come out of the same upstream response. Empty when not connected.
///
/// `id` is Kick's numeric account id or YouTube's `UC…` channel id — the same
/// value those platforms stamp on a chat message, which is what lets a member's
/// StreamNook cosmetics find them there. `None` is normal and not an error: a
/// YouTube account can have no channel at all.
#[derive(serde::Serialize, Default)]
pub struct PlatformAccountInfo {
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub id: Option<String>,
    /// The @handle, where the platform has one distinct from the display name.
    /// TikTok's: nicknames there are often stylized past recognition, and the
    /// handle is what says which account it is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
}

#[tauri::command]
pub async fn platform_account_info(provider: String) -> PlatformAccountInfo {
    match provider.as_str() {
        "kick" => {
            let (name, avatar_url) = crate::services::kick_auth_service::account_identity().await;
            // Read AFTER account_identity, which is what fills it from the same
            // response it took the name and picture out of.
            let id = crate::services::kick_auth_service::account_id();
            PlatformAccountInfo {
                name,
                avatar_url,
                id,
                handle: None,
            }
        }
        "youtube" => {
            let (name, avatar_url) =
                crate::services::youtube_auth_service::account_identity().await;
            let id = crate::services::youtube_auth_service::account_channel_id();
            PlatformAccountInfo {
                name,
                avatar_url,
                id,
                handle: None,
            }
        }
        "tiktok" => {
            let (name, avatar_url) = crate::services::tiktok_auth_service::account_identity().await;
            PlatformAccountInfo {
                name,
                avatar_url,
                // Read after account_identity, which is what records both.
                id: crate::services::tiktok_auth_service::account_id(),
                handle: crate::services::tiktok_auth_service::account_handle(),
            }
        }
        _ => PlatformAccountInfo::default(),
    }
}

/// Which StreamNook member, if any, is behind each of these chat identities.
///
/// The frontend collects the non-Twitch chatters it has seen and asks in waves;
/// this answers from a process-wide cache and looks up only what is genuinely
/// unknown. Keys with no claim are absent from the result, which is the ordinary
/// case rather than an error: most chatters are not members.
///
/// Rust owns this rather than each webview doing its own, so a MultiChat popout
/// costs nothing extra and the phone gets it for free.
///
/// Keys it could not answer (a failed request, an open circuit breaker, or more
/// than one request's worth) come back in `unresolved`, separately from keys that
/// simply have no claim, so the caller knows to ask about them again.
#[tauri::command]
pub async fn resolve_member_ids(keys: Vec<String>) -> crate::services::member_alias::ResolveOutcome {
    crate::services::member_alias::resolve(keys).await
}

/// Forget every cached claim, so the next sighting of each chatter re-asks.
///
/// Called after the local user links or unlinks a platform account: their own
/// claim just changed, and a "nobody" cached a moment earlier would otherwise
/// keep their badge off their own messages until the app restarted.
#[tauri::command]
pub fn invalidate_member_aliases() {
    crate::services::member_alias::invalidate();
}

/// Check whether the connected platform sessions are still accepted, and sign out
/// any that have been revoked. Returns the provider ids that were signed out.
///
/// This is the ONLY thing worth polling about a platform account: `kick_is_connected`
/// / `youtube_is_connected` are pure in-memory reads that never notice a session
/// dying server-side, so polling those told us nothing while this told us nothing
/// at all. Deliberately low-frequency and driven from ONE place per window — a
/// revoked token is rare, and each check is a real network round trip.
///
/// Providers that aren't connected are skipped entirely, so a Twitch-only user
/// pays nothing.
#[tauri::command]
pub async fn validate_platform_sessions(app: tauri::AppHandle) -> Vec<String> {
    use tauri::Emitter;
    // All three at once. Each is a real round trip, and YouTube's can re-read
    // its cookies through a hidden page, which must not hold up the other two.
    let (kick, youtube, tiktok) = tokio::join!(
        async {
            crate::services::kick_auth_service::is_connected()
                && crate::services::kick_auth_service::validate_session().await == Some(false)
        },
        async {
            crate::services::youtube_auth_service::is_connected()
                && crate::services::youtube_auth_service::validate_session().await == Some(false)
        },
        async {
            crate::services::tiktok_auth_service::is_connected()
                && crate::services::tiktok_auth_service::validate_session().await == Some(false)
        },
    );
    let signed_out: Vec<String> = [("kick", kick), ("youtube", youtube), ("tiktok", tiktok)]
        .into_iter()
        .filter(|(_, out)| *out)
        .map(|(provider, _)| provider.to_string())
        .collect();

    // Tell every window, so a popout's composer goes read-only without needing a
    // poll of its own.
    if !signed_out.is_empty() {
        let _ = app.emit("platform-account-changed", signed_out.clone());
    }
    signed_out
}

/// Cold-restart the chat backend, tearing down the shared local-WS bridge even
/// when providers are riding it. This is the watchdog's escalation path, for a
/// task that is alive but wedged; `stop_chat` is the user-intent path and
/// deliberately preserves the bridge for other providers.
#[tauri::command]
pub async fn restart_chat_bridge() -> Result<(), String> {
    ChatService::restart_bridge().await.map_err(|e| e.to_string())
}

/// Recent IRC connection-lifecycle events (connects, auth results, session
/// drops, deferred JOINs), pullable from a packaged build for support.
#[tauri::command]
pub async fn get_chat_lifecycle_log() -> Result<Vec<String>, String> {
    Ok(crate::services::irc_service::lifecycle_snapshot())
}

/// Dev-only: force-FIN the IRC socket to exercise the reconnect path.
#[tauri::command]
pub async fn debug_break_chat_socket() -> Result<(), String> {
    if !cfg!(debug_assertions) {
        return Err("debug builds only".into());
    }
    crate::services::irc_service::IrcService::debug_shutdown_socket()
        .await
        .map_err(|e| e.to_string())
}

/// Dev-only: raw PART with no bookkeeping — simulates the server silently
/// dropping a JOIN, for exercising the refresh probe and nudge-ladder recovery.
#[tauri::command]
pub async fn debug_unjoin_channel(channel: String) -> Result<(), String> {
    if !cfg!(debug_assertions) {
        return Err("debug builds only".into());
    }
    crate::services::irc_service::IrcService::debug_send_part(&channel)
        .await
        .map_err(|e| e.to_string())
}

/// Frontend stale-watchdog stage 1: re-issue JOINs for the desired channels so
/// a healthy connection re-acks (resetting the frontend's stale timer) and a
/// deaf or un-JOINed one is flushed out for the stage-2 escalation. Returns how
/// many channels were nudged.
#[tauri::command]
pub async fn nudge_chat_channels() -> Result<usize, String> {
    crate::services::irc_service::IrcService::nudge_channels()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn send_chat_message(
    message: String,
    reply_parent_msg_id: Option<String>,
    target_channel: Option<String>,
    broadcaster_id: Option<String>,
    sender_id: Option<String>,
    sender_account_id: Option<String>,
) -> Result<SendResult, String> {
    ChatService::send_message(
        &message,
        reply_parent_msg_id.as_deref(),
        target_channel.as_deref(),
        broadcaster_id.as_deref(),
        sender_id.as_deref(),
        sender_account_id.as_deref(),
    )
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn join_chat_channel(
    channel: String,
    window: tauri::Window,
    state: State<'_, AppState>,
) -> Result<(), String> {
    ChatService::join_channel(&channel, &state, window.label())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn leave_chat_channel(channel: String, window: tauri::Window) -> Result<(), String> {
    ChatService::leave_channel(&channel, window.label())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn start_multi_chat(
    channels: Vec<String>,
    window: tauri::Window,
    state: State<'_, AppState>,
) -> Result<u16, String> {
    if channels.is_empty() {
        return Err("No channels provided".to_string());
    }

    // Start with the first channel
    let port = ChatService::start(&channels[0], &state, true, false, window.label())
        .await
        .map_err(|e| e.to_string())?;

    // Join the rest (each call also populates the per-channel emote cache so
    // 7TV/FFZ/BTTV emotes render for these channels too)
    for channel in channels.iter().skip(1) {
        ChatService::join_channel(channel, &state, window.label())
            .await
            .unwrap_or_else(|e| {
                log::error!(
                    "[IRC Chat] Failed to join additional channel {}: {}",
                    channel,
                    e
                );
            });
    }

    Ok(port)
}

/// Parse historical IRC messages (from IVR API) through the Rust backend
/// Layout is handled by the browser - we just parse the message structure
#[tauri::command]
pub async fn parse_historical_messages(
    messages: Vec<String>,
    channel_name: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<ChatMessage>, String> {
    // Don't block the backfill on the channel emote fetch. We used to await it so
    // BTTV/7TV/FFZ emotes could be matched during parse, but that put a slow
    // provider (a down 7TV) directly in front of the recent-messages display and
    // left chat blank for seconds. Instead parse immediately with whatever emotes
    // are already cached (warm on any repeat visit) so chat populates fast like
    // Twitch, and warm the cache in the BACKGROUND for live messages and the next
    // visit. Tradeoff: on the first visit to a channel in a session third-party
    // emotes in the short backfill may render as text until the cache fills;
    // Twitch emotes (carried in the IRC tags) always render.
    if let Some(channel) = channel_name {
        let emote_service = state.emote_service.clone();
        tokio::spawn(async move {
            IrcService::fetch_and_store_emotes(&channel, emote_service).await;
        });
    }

    Ok(IrcService::parse_historical_messages(messages).await)
}

/// Join backfill: how long a non-windowed recent-messages answer is reused.
/// The main window and a MultiChat pane ask for the same channel within the
/// same second; a rejoin inside the window is served without a round trip.
const HISTORY_CACHE_TTL: Duration = Duration::from_secs(30);
/// Mirror page size; robotty caps at 800.
const HISTORY_DEFAULT_LIMIT: u32 = 100;
const HISTORY_MAX_LIMIT: u32 = 800;
const RECENT_MESSAGES_BASE: &str = "https://recent-messages.robotty.de/api/v2/recent-messages";

static HISTORY_CACHE: OnceLock<Mutex<HashMap<String, (Instant, Vec<String>)>>> = OnceLock::new();

fn history_cache() -> &'static Mutex<HashMap<String, (Instant, Vec<String>)>> {
    HISTORY_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached_history(channel: &str) -> Option<Vec<String>> {
    let cache = history_cache().lock().ok()?;
    cache
        .get(channel)
        .filter(|(at, _)| at.elapsed() < HISTORY_CACHE_TTL)
        .map(|(_, lines)| lines.clone())
}

fn store_history(channel: &str, lines: Vec<String>) {
    if let Ok(mut cache) = history_cache().lock() {
        cache.retain(|_, (at, _)| at.elapsed() < HISTORY_CACHE_TTL);
        cache.insert(channel.to_string(), (Instant::now(), lines));
    }
}

/// Twitch logins: letters, digits, underscore. Anything else never reaches
/// the mirror URL.
fn valid_channel_login(login: &str) -> bool {
    !login.is_empty()
        && login.len() <= 25
        && login.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Stamp a mirror line as history so the parser and the rows can tell it
/// from live traffic (the tag the page used to add itself).
fn mark_historical(line: &str) -> String {
    match line.strip_prefix('@') {
        Some(rest) => format!("@historical=1;{rest}"),
        None => format!("@historical=1 {line}"),
    }
}

async fn fetch_recent_messages(
    channel: &str,
    limit: u32,
    after_ms: Option<u64>,
    before_ms: Option<u64>,
) -> Vec<String> {
    let mut url = format!(
        "{RECENT_MESSAGES_BASE}/{channel}?limit={}&hide_moderation_messages=true&hide_moderated_messages=true",
        limit.clamp(1, HISTORY_MAX_LIMIT)
    );
    if let Some(a) = after_ms {
        url.push_str(&format!("&after={a}"));
    }
    if let Some(b) = before_ms {
        url.push_str(&format!("&before={b}"));
    }
    let resp = match crate::services::http::client().get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            log::warn!("[ChatHistory] {channel}: mirror request failed: {e}");
            return Vec::new();
        }
    };
    if !resp.status().is_success() {
        log::warn!("[ChatHistory] {channel}: mirror answered {}", resp.status());
        return Vec::new();
    }
    let body: serde_json::Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            log::warn!("[ChatHistory] {channel}: mirror body unreadable: {e}");
            return Vec::new();
        }
    };
    body.get("messages")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.as_str())
                .map(mark_historical)
                .collect()
        })
        .unwrap_or_default()
}

/// The join backfill, fetched and parsed by Rust in one call: the recent
/// messages mirror (recent-messages.robotty.de) tagged `historical=1`, then
/// the same parse the live path uses. Until 2026-09-07 the page fetched the
/// mirror itself, serially after the badge cache init, so history landed one
/// to two seconds after the first live rows and visibly prepended; now the
/// store starts this call at acquire time, in parallel with the IRC join,
/// and paints history and the held live tail together. `limit`, `after_ms`
/// and `before_ms` bound a reconnect backfill (never cached). A mirror error
/// is an empty answer, as before: a channel with no history is still a
/// channel.
#[tauri::command]
pub async fn load_channel_history(
    channel: String,
    limit: Option<u32>,
    after_ms: Option<u64>,
    before_ms: Option<u64>,
    state: State<'_, AppState>,
) -> Result<Vec<ChatMessage>, String> {
    let key = channel.trim().trim_start_matches('#').to_lowercase();
    if !valid_channel_login(&key) {
        return Err(format!("bad_channel: {channel}"));
    }
    let windowed = limit.is_some() || after_ms.is_some() || before_ms.is_some();
    let started = Instant::now();
    let raw = match if windowed { None } else { cached_history(&key) } {
        Some(lines) => lines,
        None => {
            let lines =
                fetch_recent_messages(&key, limit.unwrap_or(HISTORY_DEFAULT_LIMIT), after_ms, before_ms)
                    .await;
            if !windowed {
                store_history(&key, lines.clone());
            }
            lines
        }
    };
    // Same background emote warm as parse_historical_messages: never in front
    // of the rows, a down provider must not blank the chat.
    let emote_service = state.emote_service.clone();
    let warm = key.clone();
    tokio::spawn(async move {
        IrcService::fetch_and_store_emotes(&warm, emote_service).await;
    });
    let parsed = IrcService::parse_historical_messages(raw).await;
    log::debug!(
        "[ChatHistory] {key}: {} rows in {} ms{}",
        parsed.len(),
        started.elapsed().as_millis(),
        if windowed { " (windowed)" } else { "" }
    );
    Ok(parsed)
}

#[cfg(test)]
mod history_tests {
    use super::*;

    #[test]
    fn historical_tag_lands_in_the_tag_block() {
        assert_eq!(
            mark_historical("@id=1;user-id=2 :a!a@a.tmi.twitch.tv PRIVMSG #c :hi"),
            "@historical=1;id=1;user-id=2 :a!a@a.tmi.twitch.tv PRIVMSG #c :hi"
        );
        assert_eq!(
            mark_historical(":a!a@a.tmi.twitch.tv PRIVMSG #c :hi"),
            "@historical=1 :a!a@a.tmi.twitch.tv PRIVMSG #c :hi"
        );
    }

    #[test]
    fn channel_logins_are_validated_before_the_url() {
        assert!(valid_channel_login("summit1g"));
        assert!(valid_channel_login("a_b_1"));
        assert!(!valid_channel_login(""));
        assert!(!valid_channel_login("a/b"));
        assert!(!valid_channel_login("a?limit=1"));
        assert!(!valid_channel_login("x".repeat(26).as_str()));
    }
}

/// Diagnostic breadcrumb from the Kick resolver's injected script. A bare
/// "timed out" says nothing about WHERE it stalled — challenge never cleared,
/// fetch 403'd, or the script never ran at all — so the script reports each
/// attempt and this lands it in the log.
#[tauri::command]
pub fn report_kick_resolve_diag(label: String, note: String) {
    log::warn!("[Kick][resolve:{}] {}", label, note);
}

/// The emoji a YouTube channel offers in its live chat, custom ones first.
///
/// Mirrors `get_kick_channel_emotes`. YouTube has no emote-set endpoint, but its
/// live_chat page ships the full list inline (its own picker needs it), so the
/// chat adapter caches it when it resolves the chat and this hands it over.
#[tauri::command]
pub fn get_youtube_channel_emojis(
    channel: String,
) -> Vec<crate::services::providers::youtube::YouTubeEmoji> {
    crate::services::providers::youtube::channel_emojis(&channel)
}

/// The row for a message the user is about to send, built by the same parser
/// as received messages (see `IrcService::build_own_message`). The composer
/// shows it at once; the echo upgrades it in place by id.
#[tauri::command]
pub async fn build_own_chat_message(
    message: crate::services::irc_service::OwnMessage,
) -> Option<ChatMessage> {
    IrcService::build_own_message(message).await
}
