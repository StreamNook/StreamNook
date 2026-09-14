//! Browse, search, who's-live and follow commands for non-Twitch platforms.
//!
//! Playback deliberately does NOT live here: it rides the existing
//! `start_stream` / `stop_stream` / `get_stream_qualities` /
//! `change_stream_quality` commands, which dispatch on the watch URL. That keeps
//! one playback entry point for every platform, so restarts, quality changes and
//! session resume need no per-platform branches on the frontend.

use crate::models::provider_stream::{CategoryPage, ProviderStream, StreamPage};
use crate::models::settings::{AppState, ProviderFollow};
use crate::services::providers::key::{make_key, normalize_channel, same_channel};
use crate::services::providers::registry;
use crate::services::providers::source::{SourceCaps, StreamSource};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::State;

async fn source_for(provider: &str) -> Result<Arc<dyn StreamSource>, String> {
    registry()
        .await
        .get_source(provider)
        .ok_or_else(|| format!("{} browsing isn't supported in this build yet", provider))
}

/// What each platform's watch/browse adapter supports in this build. The
/// frontend gates its provider pills, follow buttons and search fan-out on this
/// rather than hard-coding platform names.
#[tauri::command]
pub async fn provider_source_caps() -> HashMap<String, SourceCaps> {
    registry()
        .await
        .sources()
        .map(|(id, src)| (id.to_string(), src.caps()))
        .collect()
}

#[tauri::command]
pub async fn provider_directory(
    provider: String,
    category: Option<String>,
    cursor: Option<String>,
    limit: Option<u32>,
) -> Result<StreamPage, String> {
    source_for(&provider)
        .await?
        .directory(category.as_deref(), cursor.as_deref(), limit.unwrap_or(30))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn provider_search(provider: String, query: String) -> Result<StreamPage, String> {
    source_for(&provider)
        .await?
        .search(&query)
        .await
        .map_err(|e| e.to_string())
}

/// Channel metadata WITHOUT connecting chat, so the watch path can poll viewers
/// and title while a stream is open.
#[tauri::command]
pub async fn provider_channel_meta(
    provider: String,
    channel: String,
) -> Result<ProviderStream, String> {
    source_for(&provider)
        .await?
        .channel_meta(&channel)
        .await
        .map_err(|e| e.to_string())
}

/// The current who's-live snapshot for followed provider channels, for initial
/// paint. Steady-state updates arrive on the `provider-live-update` event.
#[tauri::command]
pub async fn get_provider_followed_live() -> Result<Vec<ProviderStream>, String> {
    Ok(crate::services::provider_live_service::snapshot().await)
}

/// The current who's-live snapshot for FAVOURITED channels, across every
/// platform, for initial paint. Steady-state updates arrive on the
/// `favorites-live-update` event.
#[tauri::command]
pub async fn get_favorite_live() -> Result<Vec<ProviderStream>, String> {
    Ok(crate::services::favorite_live_service::snapshot().await)
}

/// Sweep favourites now instead of waiting out the cadence, so a channel you
/// just favourited shows up as live immediately rather than up to a minute later.
#[tauri::command]
pub async fn refresh_favorites(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    crate::services::favorite_live_service::refresh_favorites(app, (*state).clone()).await;
    Ok(())
}

/// Which of `channels` are live now. Batched by the adapter; also the offline
/// detector for the currently-watched provider stream.
#[tauri::command]
pub async fn provider_live_check(
    provider: String,
    channels: Vec<String>,
) -> Result<Vec<ProviderStream>, String> {
    source_for(&provider)
        .await?
        .live_check(&channels)
        .await
        .map_err(|e| e.to_string())
}

/// Channel avatars for the provider stream cards currently ON SCREEN.
///
/// Only YouTube needs this, and only for CATEGORY streams: search and the
/// subscriptions feed both ship an avatar with the row, but a game's live grid
/// ships none. Resolving one costs a full channel browse, so the caller sends just
/// what it is displaying and the adapter caches per session.
///
/// Ids with no avatar are absent from the map rather than blank, so the caller
/// keeps whatever placeholder it already draws.
#[tauri::command]
pub async fn provider_channel_avatars(
    provider: String,
    channel_ids: Vec<String>,
) -> Result<std::collections::HashMap<String, String>, String> {
    match provider.as_str() {
        "youtube" => Ok(crate::services::providers::youtube_media::channel_avatars(&channel_ids).await),
        // Kick rows address the channel by SLUG, which is what its avatar
        // endpoint takes.
        "kick" => Ok(crate::services::providers::kick::channel_avatars(&channel_ids).await),
        _ => Ok(std::collections::HashMap::new()),
    }
}

/// Whether a channel sells memberships, and whether the viewer holds one.
///
/// Only YouTube needs a lookup. Kick's subscription state already rides the follow
/// list (its account sync imports it), and Twitch has its own Helix path, so both
/// answer from data the app already holds.
#[tauri::command]
pub async fn provider_membership(
    provider: String,
    channel: String,
) -> Result<crate::services::providers::youtube_subscribe::MembershipState, String> {
    if provider != "youtube" {
        return Ok(Default::default());
    }
    crate::services::providers::youtube_subscribe::membership_state(&channel)
        .await
        .map_err(|e| e.to_string())
}

/// A chatter's profile card on a non-Twitch platform.
///
/// Deliberately NOT routed through `get_user_profile_complete`: that is Helix +
/// IVR + 7TV, all of which answer about Twitch accounts. Asking it about a
/// `kick:` id would return someone else's Twitch profile or nothing at all.
///
/// `room` is the channel whose chat the user was clicked in — badges, followage
/// and sub tenure are all scoped to it, which is what the card shows.
#[tauri::command]
pub async fn kick_user_profile(
    room: String,
    username: String,
) -> Result<crate::services::providers::kick_profile::KickUserProfile, String> {
    crate::services::providers::kick_profile::fetch(&room, &username).await
}

/// A YouTube chatter's channel profile. Same rule as Kick: never Helix, never IVR.
#[tauri::command]
pub async fn youtube_user_profile(
    channel_id: String,
) -> Result<crate::services::providers::youtube_media::YouTubeUserProfile, String> {
    crate::services::providers::youtube_media::user_profile(&channel_id)
        .await
        .map_err(|e| e.to_string())
}

// --- Follows ---------------------------------------------------------------
//
// Kick and TikTok expose no followed-channels API to third parties, so
// StreamNook keeps its own list. It lives in `Settings.provider_follows` (a
// typed field, because the who's-live poller reads it) and is written through
// the same save path as every other setting.

#[tauri::command]
pub fn get_provider_follows(state: State<'_, AppState>) -> Result<Vec<ProviderFollow>, String> {
    let settings = state.settings.lock().map_err(|e| e.to_string())?;
    Ok(settings.provider_follows.clone())
}

#[tauri::command]
pub async fn provider_follow(
    provider: String,
    channel: String,
    display_name: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<ProviderFollow>, String> {
    // NOT a blanket lowercase: YouTube's video and UC ids are case-SENSITIVE, and
    // flattening one here would both fail to resolve and never match the rows the
    // subscription import wrote. Same carve-out as `key::make_key`.
    let channel = normalize_channel(&provider, &channel);
    if channel.is_empty() {
        return Err("channel is required".to_string());
    }
    // Signed in to YouTube? Then YouTube is the source of truth: subscribe THERE
    // first and only record it locally if that worked, so the app never claims to
    // follow something the account doesn't. Signed out, this is a local-only list
    // exactly as before.
    if provider == "youtube" && crate::services::youtube_auth_service::is_connected() {
        crate::services::providers::youtube_subscribe::set_subscribed(&channel, true)
            .await
            .map_err(|e| e.to_string())?;
    }
    {
        let mut settings = state.settings.lock().map_err(|e| e.to_string())?;
        // Loose match: rows written before the casing fix are lowercased on disk,
        // and an exact compare would push a SECOND row for a channel already
        // followed — which is how re-syncing produced duplicates.
        if settings
            .provider_follows
            .iter()
            .any(|f| f.provider == provider && same_channel(&provider, &f.channel, &channel))
        {
            return Ok(settings.provider_follows.clone());
        }
        settings.provider_follows.push(ProviderFollow {
            // Hand-added: no import payload to take an avatar from, so the
            // per-card resolver fills it in.
            avatar: None,
            provider: provider.clone(),
            channel: channel.clone(),
            display_name,
            user_id: None,
            added_at: chrono::Utc::now().to_rfc3339(),
            subscribed: false,
            // Added by hand here, so an account re-sync must never remove it.
            imported: false,
        });
    }
    persist(&state)?;
    log::info!("[Follows] followed {}", make_key(&provider, &channel));
    let settings = state.settings.lock().map_err(|e| e.to_string())?;
    Ok(settings.provider_follows.clone())
}

#[tauri::command]
pub async fn provider_unfollow(
    provider: String,
    channel: String,
    state: State<'_, AppState>,
) -> Result<Vec<ProviderFollow>, String> {
    let channel = normalize_channel(&provider, &channel);
    // Mirror of follow: unsubscribe on YouTube first when signed in, so the local
    // list never drifts from the account.
    if provider == "youtube" && crate::services::youtube_auth_service::is_connected() {
        crate::services::providers::youtube_subscribe::set_subscribed(&channel, false)
            .await
            .map_err(|e| e.to_string())?;
    }
    {
        let mut settings = state.settings.lock().map_err(|e| e.to_string())?;
        // Loose match, or a legacy lowercased row survives the retain and the
        // unfollow silently does nothing.
        settings
            .provider_follows
            .retain(|f| !(f.provider == provider && same_channel(&provider, &f.channel, &channel)));
    }
    persist(&state)?;
    let settings = state.settings.lock().map_err(|e| e.to_string())?;
    Ok(settings.provider_follows.clone())
}

// --- Kick account sync -----------------------------------------------------

/// Result of an account sync, for the Connections UI.
#[derive(serde::Serialize)]
pub struct SyncResult {
    pub imported: usize,
    pub subscribed: usize,
    pub follows: Vec<ProviderFollow>,
}

/// Sign in to kick.com (a website session, distinct from the OAuth connection)
/// and import the user's real follows + subscriptions.
#[tauri::command]
pub async fn kick_account_sync(
    interactive: bool,
    state: State<'_, AppState>,
) -> Result<SyncResult, String> {
    // Interactive means "sign me in": ONE window that takes consent and then
    // promotes it to a site session, so the user is never asked twice.
    // Non-interactive is the silent re-sync against an existing session.
    let report = if interactive {
        crate::services::providers::kick_account::sign_in().await
    } else {
        crate::services::providers::kick_account::import(false).await
    }
    .map_err(|e| e.to_string())?;
    if report.status != "ok" {
        return Err("Not signed in to kick.com".to_string());
    }

    let now = chrono::Utc::now().to_rfc3339();
    let subscribed = report.channels.iter().filter(|c| c.subscribed).count();
    let imported = report.channels.len();
    {
        let mut settings = state.settings.lock().map_err(|e| e.to_string())?;
        // Hand-added Kick follows and every other platform survive untouched;
        // only previously-imported Kick rows are replaced, so a channel the user
        // unfollowed on Kick disappears here too.
        settings
            .provider_follows
            .retain(|f| !(f.provider == "kick" && f.imported));

        for ch in report.channels {
            let slug = ch.slug.to_lowercase();
            if let Some(existing) = settings
                .provider_follows
                .iter_mut()
                .find(|f| f.provider == "kick" && f.channel == slug)
            {
                // Already followed by hand: keep it hand-owned, just enrich it.
                existing.subscribed = ch.subscribed;
                if existing.display_name.is_none() {
                    existing.display_name = ch.username;
                }
                // Refresh the picture even on a hand-added row: it is free here
                // and channels do change their avatar.
                if ch.profile_pic.is_some() {
                    existing.avatar = ch.profile_pic;
                }
                continue;
            }
            settings.provider_follows.push(ProviderFollow {
                provider: "kick".to_string(),
                channel: slug,
                display_name: ch.username,
                avatar: ch.profile_pic,
                user_id: None,
                added_at: now.clone(),
                subscribed: ch.subscribed,
                imported: true,
            });
        }
    }
    persist(&state)?;

    let follows = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        settings.provider_follows.clone()
    };
    log::info!("[Kick] imported {} follows ({} subscribed)", imported, subscribed);
    spawn_avatar_backfill(&state, "kick");
    // Same as YouTube: paint who's live straight away instead of waiting for the
    // next cadence tick.
    if let Some(app) = crate::services::providers::app_handle() {
        let state_for_refresh = (*state).clone();
        tauri::async_runtime::spawn(async move {
            crate::services::provider_live_service::refresh_provider(app, state_for_refresh, "kick")
                .await;
        });
    }
    Ok(SyncResult {
        imported,
        subscribed,
        follows,
    })
}

/// Whether a kick.com website session exists (drives the Connections row).
#[tauri::command]
pub async fn kick_account_is_synced() -> bool {
    crate::services::providers::kick_account::is_connected().await
}

// --- YouTube account sync --------------------------------------------------

/// Import the signed-in YouTube account's subscriptions as follows.
///
/// Sign-in alone only makes `followed_live` possible, which answers "which of my
/// subscriptions are LIVE". The Home following list is `Settings.provider_follows`,
/// so without this import the list stays empty and the connection looks like it did
/// nothing. Same contract as `kick_account_sync`, deliberately.
#[tauri::command]
pub async fn youtube_account_sync(state: State<'_, AppState>) -> Result<SyncResult, String> {
    let subs = crate::services::providers::youtube_account::subscriptions()
        .await
        .map_err(|e| e.to_string())?;

    let now = chrono::Utc::now().to_rfc3339();
    let imported = subs.len();
    {
        let mut settings = state.settings.lock().map_err(|e| e.to_string())?;
        // Hand-added YouTube follows and every other platform survive untouched;
        // only previously-imported YouTube rows are replaced, so a channel the
        // user unsubscribed from on YouTube disappears here too.
        settings
            .provider_follows
            .retain(|f| !(f.provider == "youtube" && f.imported));

        let mut repaired = 0usize;
        for sub in subs {
            // Keyed by UC id: a handle can be changed by its owner, the UC id
            // cannot, and both address chat and playback.
            //
            // Matched case-INSENSITIVELY on purpose. Hand-added rows written
            // before the casing fix were lowercased on the way in, so an exact
            // compare would miss them and push a duplicate every single sync. When
            // one is found, its casing is REPAIRED in place — never deleted, since
            // a hand-added follow is the user's data and no import would bring it
            // back.
            if let Some(existing) = settings
                .provider_follows
                .iter_mut()
                .find(|f| f.provider == "youtube" && same_channel("youtube", &f.channel, &sub.channel_id))
            {
                if existing.channel != sub.channel_id {
                    existing.channel = sub.channel_id.clone();
                    repaired += 1;
                }
                // Already followed by hand: keep it hand-owned, just enrich it.
                if existing.display_name.is_none() {
                    existing.display_name = Some(sub.display_name);
                }
                if sub.avatar.is_some() {
                    existing.avatar = sub.avatar;
                }
                continue;
            }
            settings.provider_follows.push(ProviderFollow {
                provider: "youtube".to_string(),
                channel: sub.channel_id,
                display_name: Some(sub.display_name),
                avatar: sub.avatar,
                user_id: None,
                added_at: now.clone(),
                // YouTube memberships are not exposed on the subscriptions list,
                // so this stays false rather than being guessed at.
                subscribed: false,
                imported: true,
            });
        }
        if repaired > 0 {
            log::info!(
                "[YouTube] repaired the casing of {} follow row(s) written before the fix",
                repaired
            );
        }
    }
    persist(&state)?;

    let follows = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        settings.provider_follows.clone()
    };
    log::info!("[YouTube] imported {} subscription(s)", imported);
    spawn_avatar_backfill(&state, "youtube");
    // Look NOW rather than waiting out the poller's 120s cadence. Importing the
    // list tells us who you follow, not who is live, so without this a fresh
    // sign-in shows an empty Following tab for two minutes.
    if let Some(app) = crate::services::providers::app_handle() {
        let state_for_refresh = (*state).clone();
        tauri::async_runtime::spawn(async move {
            crate::services::provider_live_service::refresh_provider(
                app,
                state_for_refresh,
                "youtube",
            )
            .await;
        });
    }
    Ok(SyncResult {
        imported,
        subscribed: 0,
        follows,
    })
}

/// Fill in avatars for follows whose import payload didn't carry one, once,
/// in the background.
///
/// Signing in is the moment we learn the whole list, so this is where the images
/// get warmed. Previously they were resolved a card at a time as the roster
/// scrolled, on every app start, which is the "wasting compute on every load"
/// this removes. Only YouTube needs it: Kick's import always carries a picture.
///
/// Deliberately bounded and spawned: it must never delay the sync returning, and
/// a large subscription list is resolved a wave at a time by the resolver's own
/// concurrency limit.
fn spawn_avatar_backfill(state: &AppState, provider: &'static str) {
    if provider != "youtube" && provider != "kick" {
        return;
    }
    let Ok(settings) = state.settings.lock() else {
        return;
    };
    let missing: Vec<String> = settings
        .provider_follows
        .iter()
        .filter(|f| f.provider == provider && f.avatar.is_none())
        .map(|f| f.channel.clone())
        .collect();
    drop(settings);
    if missing.is_empty() {
        return;
    }
    let state = state.clone();
    tauri::async_runtime::spawn(async move {
        log::info!("[{}] warming {} follow avatar(s)", provider, missing.len());
        let resolved = match provider {
            "kick" => crate::services::providers::kick::channel_avatars(&missing).await,
            // The SUBSCRIPTIONS FEED already carries every avatar, in ONE request.
            // The per-channel InnerTube browse is the fallback and costs ~3 MB
            // EACH (measured 3,055,320 bytes) — for a 254-channel list that is
            // roughly 760 MB, which is why a naive backfill only ever landed a
            // handful before failing. Only channels the feed did not mention fall
            // through to it, and that path caps itself per pass.
            _ => {
                let mut from_feed: std::collections::HashMap<String, String> =
                    match crate::services::providers::youtube_account::subscriptions().await {
                        Ok(subs) => subs
                            .into_iter()
                            .filter_map(|sub| sub.avatar.map(|a| (sub.channel_id, a)))
                            .collect(),
                        Err(e) => {
                            log::warn!("[YouTube] avatar backfill could not read the feed: {}", e);
                            Default::default()
                        }
                    };
                from_feed.retain(|id, _| missing.iter().any(|m| m == id));
                let still_missing: Vec<String> = missing
                    .iter()
                    .filter(|m| !from_feed.contains_key(*m))
                    .cloned()
                    .collect();
                if !still_missing.is_empty() {
                    log::info!(
                        "[YouTube] {} follow(s) not in the feed; resolving those individually",
                        still_missing.len()
                    );
                    from_feed.extend(
                        crate::services::providers::youtube_media::channel_avatars(&still_missing)
                            .await,
                    );
                }
                from_feed
            }
        };
        if resolved.is_empty() {
            return;
        }
        let count = resolved.len();
        // Snapshot under the lock, write outside it: the disk write blocks.
        let snapshot = {
            let Ok(mut settings) = state.settings.lock() else {
                return;
            };
            for follow in settings
                .provider_follows
                .iter_mut()
                .filter(|f| f.provider == provider && f.avatar.is_none())
            {
                if let Some(url) = resolved.get(&follow.channel) {
                    follow.avatar = Some(url.clone());
                }
            }
            settings.clone()
        };
        if let Err(e) = crate::commands::settings::write_settings_to_disk(&snapshot) {
            log::warn!("[{}] avatar backfill could not be saved: {}", provider, e);
            return;
        }
        log::info!("[{}] cached {} follow avatar(s)", provider, count);
    });
}

/// Write the settings file after a follow-list change. Kept separate so the
/// settings lock is never held across the (blocking) disk write.
/// Drop the follow rows a platform's account sync imported, leaving hand-added
/// ones alone.
///
/// Called on sign-out. Without it, disconnecting an account left its channels
/// sitting in the Following tab with no session behind them — they would never
/// resolve live again, and reconnecting would import them a second time. Rows the
/// user added by hand are theirs and survive, exactly as they survive a re-sync.
pub fn clear_imported_follows(provider: &str, state: &State<'_, AppState>) -> Result<usize, String> {
    let removed = {
        let mut settings = state.settings.lock().map_err(|e| e.to_string())?;
        let before = settings.provider_follows.len();
        settings
            .provider_follows
            .retain(|f| !(f.provider == provider && f.imported));
        before - settings.provider_follows.len()
    };
    if removed > 0 {
        persist(state)?;
        log::info!("[Follows] cleared {} imported {} row(s)", removed, provider);
    }
    Ok(removed)
}

fn persist(state: &State<'_, AppState>) -> Result<(), String> {
    let snapshot = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        settings.clone()
    };
    crate::commands::settings::write_settings_to_disk(&snapshot)
}

/// Browsable categories for a platform, most-watched first. Feeds the Categories
/// tab; `provider_directory` with the returned id then lists a category's streams.
#[tauri::command]
pub async fn provider_categories(
    provider: String,
    cursor: Option<String>,
    limit: Option<u32>,
) -> Result<CategoryPage, String> {
    let reg = registry().await;
    let source = reg
        .get_source(&provider)
        .ok_or_else(|| format!("provider '{}' has no browse support", provider))?;
    source
        .categories(cursor.as_deref(), limit.unwrap_or(40))
        .await
        .map_err(|e| e.to_string())
}
