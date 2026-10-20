use crate::services::badge_service::{
    BadgeDetail, BadgeService, SelectedTwitchBadge, ThirdPartyGalleryBadge, UserBadgesResponse,
};
use crate::services::twitch_service::TwitchService;
use log::debug;
use std::sync::Arc;
use tokio::sync::RwLock;

// Global badge service instance
lazy_static::lazy_static! {
    static ref BADGE_SERVICE: Arc<RwLock<Option<BadgeService>>> = Arc::new(RwLock::new(None));
}

/// Initialize the badge service (called on app start)
pub async fn initialize_badge_service() {
    let client_id = env!("TWITCH_APP_CLIENT_ID").to_string();
    let service = BadgeService::new(client_id);

    // No per-client Helix prefetch of global badges on startup. get_user_badges
    // self-heals when the cache is empty (it lazily fetches on the first badge
    // resolution), so idle clients never call Helix for badges.

    // Pre-fetch third-party badge databases (FFZ/Chatterino/etc — not Helix)
    let _ = service.fetch_third_party_badges().await;

    *BADGE_SERVICE.write().await = Some(service);
    debug!("[BadgeService] Service initialized and pre-warmed");
    // Rows a native shell got before now wore no chat-client badges.
    crate::services::late_third_party::flush().await;
}

/// Whether the service (and so the chat-client badge feeds) is up; rows stamped
/// before then wear none of those badges.
pub fn third_party_ready() -> bool {
    BADGE_SERVICE.try_read().is_ok_and(|guard| guard.is_some())
}

/// The third-party badges a chatter wears, without waiting on the service
/// (see `BadgeService::worn_third_party_now`); empty before it is ready.
pub fn third_party_worn_now(user_id: &str) -> Vec<crate::services::badge_service::UserBadge> {
    let Ok(guard) = BADGE_SERVICE.try_read() else { return Vec::new() };
    guard.as_ref().map(|s| s.worn_third_party_now(user_id)).unwrap_or_default()
}

pub async fn get_service() -> Result<Arc<RwLock<Option<BadgeService>>>, String> {
    Ok(BADGE_SERVICE.clone())
}

/// The global badge a Twitch user selected, the one chat shows for them in
/// every channel; `None` when they wear none. Profile cards show this.
#[tauri::command]
pub async fn get_selected_twitch_badge(user_id: String) -> Result<Option<SelectedTwitchBadge>, String> {
    let service_lock = get_service().await?;
    {
        let service_guard = service_lock.read().await;
        if service_guard.is_none() {
            drop(service_guard);
            initialize_badge_service().await;
        }
    }
    let service_guard = service_lock.read().await;
    let service = service_guard
        .as_ref()
        .ok_or_else(|| "Badge service failed to initialize".to_string())?;
    service.selected_badge(&user_id).await
}

/// Get user badges for a specific channel (for normal chat - display badges only)
#[tauri::command]
pub async fn get_user_badges_unified(
    user_id: String,
    username: String,
    channel_id: String,
    channel_name: String,
) -> Result<UserBadgesResponse, String> {
    let service_lock = get_service().await?;

    // Auto-initialize if not ready yet
    {
        let service_guard = service_lock.read().await;
        if service_guard.is_none() {
            drop(service_guard);
            initialize_badge_service().await;
        }
    }

    let service_guard = service_lock.read().await;
    let service = service_guard
        .as_ref()
        .ok_or_else(|| "Badge service failed to initialize".to_string())?;

    let token = TwitchService::get_token()
        .await
        .map_err(|e| format!("Failed to get token: {}", e))?;

    service
        .get_user_badges(&user_id, &username, &channel_id, &channel_name, &token)
        .await
}

/// Get user badges with full earned badge collection (for profile overlay)
#[tauri::command]
pub async fn get_user_badges_with_earned_unified(
    user_id: String,
    username: String,
    channel_id: String,
    channel_name: String,
) -> Result<UserBadgesResponse, String> {
    let service_lock = get_service().await?;

    // Auto-initialize if not ready yet
    {
        let service_guard = service_lock.read().await;
        if service_guard.is_none() {
            drop(service_guard);
            initialize_badge_service().await;
        }
    }

    let service_guard = service_lock.read().await;
    let service = service_guard
        .as_ref()
        .ok_or_else(|| "Badge service failed to initialize".to_string())?;

    let token = TwitchService::get_token()
        .await
        .map_err(|e| format!("Failed to get token: {}", e))?;

    service
        .get_user_badges_with_earned(&user_id, &username, &channel_id, &channel_name, &token)
        .await
}

/// Resolve ONLY a user's real chat-client (third-party) badges — their actual
/// BTTV / FFZ / Chatterino / Homies / Chatsen / Chatty / DankChat badges — from
/// the prefetched provider databases. Cache-only (no Twitch token, no channel,
/// no network), so chat can call it once per chatter without the per-user round
/// trip the full unified path needs. display_badges / earned_badges come back
/// empty.
#[tauri::command]
pub async fn get_third_party_badges_for_user_unified(
    user_id: String,
) -> Result<UserBadgesResponse, String> {
    let service_lock = get_service().await?;

    // Auto-initialize if not ready yet
    {
        let service_guard = service_lock.read().await;
        if service_guard.is_none() {
            drop(service_guard);
            initialize_badge_service().await;
        }
    }

    let service_guard = service_lock.read().await;
    let service = service_guard
        .as_ref()
        .ok_or_else(|| "Badge service failed to initialize".to_string())?;

    Ok(service.get_third_party_badges_only(&user_id).await)
}

/// Parse a badge string from IRC tags (e.g., "subscriber/12,premium/1")
#[tauri::command]
pub async fn parse_badge_string(badge_string: String) -> Result<Vec<String>, String> {
    let service_lock = get_service().await?;
    let service_guard = service_lock.read().await;

    let service = service_guard
        .as_ref()
        .ok_or_else(|| "Badge service not initialized".to_string())?;

    Ok(service.parse_badge_string(&badge_string))
}

/// Pre-fetch global badges (can be called manually or on app start)
#[tauri::command]
pub async fn prefetch_global_badges_unified() -> Result<(), String> {
    let service_lock = get_service().await?;
    let service_guard = service_lock.read().await;

    let service = service_guard
        .as_ref()
        .ok_or_else(|| "Badge service not initialized".to_string())?;

    let token = TwitchService::get_token()
        .await
        .map_err(|e| format!("Failed to get token: {}", e))?;

    service.fetch_global_badges(&token).await
}

/// Pre-fetch channel badges for a specific channel
#[tauri::command]
pub async fn prefetch_channel_badges_unified(channel_id: String) -> Result<(), String> {
    let service_lock = get_service().await?;
    let service_guard = service_lock.read().await;

    let service = service_guard
        .as_ref()
        .ok_or_else(|| "Badge service not initialized".to_string())?;

    let token = TwitchService::get_token()
        .await
        .map_err(|e| format!("Failed to get token: {}", e))?;

    service.fetch_channel_badges(&channel_id, &token).await
}

/// Pre-fetch third-party badge databases (FFZ, Chatterino, Homies)
#[tauri::command]
pub async fn prefetch_third_party_badges() -> Result<(), String> {
    let service_lock = get_service().await?;
    let service_guard = service_lock.read().await;

    let service = service_guard
        .as_ref()
        .ok_or_else(|| "Badge service not initialized".to_string())?;

    service.fetch_third_party_badges().await
}

/// Clear all badge caches
#[tauri::command]
pub async fn clear_badge_cache_unified() -> Result<(), String> {
    let service_lock = get_service().await?;
    let service_guard = service_lock.read().await;

    let service = service_guard
        .as_ref()
        .ok_or_else(|| "Badge service not initialized".to_string())?;

    service.clear_cache().await;
    Ok(())
}

/// Clear badge cache for a specific channel
#[tauri::command]
pub async fn clear_channel_badge_cache_unified(channel_id: String) -> Result<(), String> {
    let service_lock = get_service().await?;
    let service_guard = service_lock.read().await;

    let service = service_guard
        .as_ref()
        .ok_or_else(|| "Badge service not initialized".to_string())?;

    service.clear_channel_cache(&channel_id).await;
    Ok(())
}

/// Get the current user's global badge collection (all earned global badges)
/// Returns a list of badge IDs in "set_id/version" format (e.g., "bungie_ally_badge/1")
/// Used for cross-referencing badge drop ownership. Only answers for the
/// signed-in user (the Drops token's owner); errors for anyone else.
#[tauri::command]
pub async fn get_global_badge_collection(
    user_id: String,
    username: String,
) -> Result<Vec<String>, String> {
    let service_lock = get_service().await?;

    // Auto-initialize if not ready yet
    {
        let service_guard = service_lock.read().await;
        if service_guard.is_none() {
            drop(service_guard);
            initialize_badge_service().await;
        }
    }

    let service_guard = service_lock.read().await;
    let service = service_guard
        .as_ref()
        .ok_or_else(|| "Badge service not initialized".to_string())?;

    service
        .fetch_current_user_global_collection(&user_id, &username)
        .await
}

/// Get the full distinct badge set for every third-party chat client (FFZ,
/// BetterTTV, Chatterino, Homies, Chatsen, Chatty, DankChat) for the browse gallery.
/// Pass the current user's Twitch ID to flag which badges they own.
#[tauri::command]
pub async fn get_all_third_party_badges(
    viewer_user_id: Option<String>,
) -> Result<Vec<ThirdPartyGalleryBadge>, String> {
    let service_lock = get_service().await?;

    // Auto-initialize if not ready yet
    {
        let service_guard = service_lock.read().await;
        if service_guard.is_none() {
            drop(service_guard);
            initialize_badge_service().await;
        }
    }

    let service_guard = service_lock.read().await;
    let service = service_guard
        .as_ref()
        .ok_or_else(|| "Badge service failed to initialize".to_string())?;

    // Make sure the databases are warm (no-op if cache is fresh).
    let _ = service.fetch_third_party_badges().await;

    Ok(service
        .get_all_third_party_badges(viewer_user_id.as_deref())
        .await)
}

/// What the badge spotlight shows for a clicked third-party badge (FFZ,
/// BetterTTV, Chatterino, Homies, Moltorino, Chatsen, Chatty, DankChat):
/// provider name and site, and whatever the in-memory feeds know about the
/// badge (title, art, description, holder count, whether the viewer holds
/// it). Accepts any of the badge id spaces (`ffz-3`, `ffz:ffz-3`, `3`); a
/// badge the feeds do not list still answers with the provider's details.
#[tauri::command]
pub async fn get_badge_detail(
    provider: String,
    id: String,
    title: Option<String>,
    viewer_user_id: Option<String>,
) -> Result<BadgeDetail, String> {
    let service_lock = get_service().await?;
    {
        let service_guard = service_lock.read().await;
        if service_guard.is_none() {
            drop(service_guard);
            initialize_badge_service().await;
        }
    }
    let service_guard = service_lock.read().await;
    let service = service_guard
        .as_ref()
        .ok_or_else(|| "Badge service failed to initialize".to_string())?;
    service
        .get_badge_detail(&provider, &id, title.as_deref(), viewer_user_id.as_deref())
        .await
}

/// Resolve a single user's BetterTTV Pro loyalty badge (the per-user badge a Pro
/// subscriber enables). This is separate from the contributor badge feed above:
/// BTTV only exposes Pro badges over its live-update WebSocket, so this does an
/// on-demand `broadcast_me` lookup (cached, negative answers included). Returns
/// `None` when the user has no Pro badge or the socket didn't answer in time.
/// Called by the profile card in parallel with the main profile fetch so it
/// never blocks the card from rendering.
#[tauri::command]
pub async fn get_bttv_pro_badge(
    user_id: String,
) -> Result<Option<crate::services::bttv_pro_service::BttvProBadge>, String> {
    Ok(crate::services::bttv_pro_service::resolve_bttv_pro_badge(&user_id).await)
}

/// Every distinct BetterTTV Pro loyalty badge design StreamNook knows: the ones
/// this install resolved (persisted), then the shared list streamnook.app
/// builds from every profile lookup. The BetterTTV gallery tab renders one tile
/// per URL so discovered loyalty tiers are visible to everyone, not just the
/// account that owns them.
#[tauri::command]
pub async fn get_discovered_bttv_pro_badges() -> Result<Vec<String>, String> {
    Ok(crate::services::bttv_pro_service::all_known_bttv_pro_badges().await)
}

/// The primary account's badge standing: what it owns, every badge's earn
/// window, and the badges it is missing that are earnable right now. Answers
/// from cache at once; a due refresh runs in the background and is announced
/// with `badge-standing-changed`.
#[tauri::command]
pub async fn get_badge_standing(
    app_handle: crate::rt::AppHandle,
    force: Option<bool>,
) -> Result<crate::services::badge_standing::BadgeStanding, String> {
    Ok(crate::services::badge_standing::get_standing(&app_handle, force.unwrap_or(false)).await)
}

/// The phones' badge wall (services/badge_wall.rs): the global catalogue with
/// owned badges marked, each tile's status and sheet copy, in `sort`
/// (`newest`, `available`, `soon`, `usage`, `oldest`), and the missing strip.
/// `force` re-reads the account's collection in the background.
#[tauri::command]
pub async fn get_badge_wall(
    app_handle: crate::rt::AppHandle,
    sort: Option<String>,
    force: Option<bool>,
) -> Result<crate::services::badge_wall::BadgeWall, String> {
    let sort = crate::services::badge_wall::Sort::parse(sort.as_deref().unwrap_or("newest"));
    Ok(crate::services::badge_wall::get(&app_handle, sort, force.unwrap_or(false)).await)
}

/// One badge's earn window, for a surface that shows a single badge.
#[tauri::command]
pub async fn get_badge_window(
    set_id: String,
    version: String,
) -> Result<Option<Vec<crate::services::badge_window::WindowRun>>, String> {
    Ok(crate::services::badge_standing::window_for(&set_id, &version).await)
}

/// The live Drops campaign that awards a badge, when one is running, so a
/// badge page can offer the channels it is earned on. None when no campaign
/// matches or the Drops list cannot be read (Drops not signed in).
#[tauri::command]
pub async fn get_badge_drop_campaign(
    state: tauri::State<'_, crate::models::settings::AppState>,
    set_id: String,
    version: String,
) -> Result<Option<crate::models::drops::DropCampaign>, String> {
    Ok(live_badge_campaign(&state, &set_id, &version).await.map(|(c, _)| c))
}

/// How one badge is earned and where: what to do, for how long, on which
/// channels and in which category, plus a sentence saying all of it. The
/// viewer's live Drops list answers first; the relay's campaign requirement
/// covers anyone not signed in to Drops.
#[tauri::command]
pub async fn get_badge_earn_guide(
    state: tauri::State<'_, crate::models::settings::AppState>,
    set_id: String,
    version: String,
) -> Result<crate::services::badge_earn::EarnGuide, String> {
    use crate::services::universal_cache_service::{get_cached_item, CacheType};

    let live = live_badge_campaign(&state, &set_id, &version)
        .await
        .and_then(|(c, title)| crate::services::badge_earn::campaign_requirement(&c, title.as_deref()));
    let meta = get_cached_item(CacheType::Badge, &format!("metadata:{set_id}-v{version}"))
        .await
        .ok()
        .flatten()
        .and_then(|c| {
            serde_json::from_value::<crate::commands::badge_metadata::BadgeMetadataCached>(c.data).ok()
        });
    Ok(crate::services::badge_earn::earn_guide(
        meta.as_ref().and_then(|m| m.enrichment.as_ref()),
        meta.as_ref().and_then(|m| m.more_info.as_deref()),
        live,
    ))
}

/// The live Drops campaign awarding a badge, with the badge's title.
async fn live_badge_campaign(
    state: &crate::models::settings::AppState,
    set_id: &str,
    version: &str,
) -> Option<(crate::models::drops::DropCampaign, Option<String>)> {
    let title = crate::commands::badges::get_cached_global_badges()
        .await
        .ok()
        .flatten()
        .and_then(|catalog| {
            catalog
                .data
                .into_iter()
                .find(|s| s.set_id == set_id)?
                .versions
                .into_iter()
                .find(|v| v.id == version)
                .map(|v| v.title)
        });
    // The cached list at any age first: a fresh fetch resets the live progress
    // map, which a badge page has no business doing.
    let mut campaigns = {
        let drops = state.drops_service.lock().await;
        match drops.cached_campaigns_snapshot().await {
            Some(c) => c,
            None => match drops.get_all_active_campaigns_cached().await {
                Ok(c) => c,
                Err(e) => {
                    debug!("[Badges] No Drops list for {set_id}: {e}");
                    return None;
                }
            },
        }
    };
    let now = chrono::Utc::now();
    campaigns.retain(|c| c.end_at > now);
    let campaign =
        crate::services::drops_overview::campaign_for_badge(&campaigns, set_id, title.as_deref())
            .cloned()?;
    Some((campaign, title))
}

/// Whether a badge's earn window is open right now. Prefers the enrichment's
/// ISO window (authoritative campaign data) over the payload's `status`, which
/// is only a snapshot of when the relay sent it.
///
/// Campaign windows only: a toast or an Android push must never fire off a
/// prose window, because year-less prose ("Dec 1-12") re-matches every year.
///
/// Crate-visible so the Android background poll classifies a drop exactly as
/// this path does. Duplicating the window logic would let the two drift, and a
/// drop treated as available by one and not the other is the kind of
/// disagreement nobody notices until it is wrong.
pub(crate) fn is_window_open(
    badge: &crate::services::badge_polling_service::BadgeNotification,
) -> bool {
    use crate::services::badge_polling_service::BadgeNotificationStatus;
    use crate::services::badge_window::{campaign_runs, status_at, WindowStatus};

    match campaign_runs(badge.enrichment.as_ref()) {
        Some(runs) => {
            status_at(&runs, chrono::Utc::now().timestamp_millis()) == WindowStatus::Available
        }
        None => matches!(badge.status, BadgeNotificationStatus::Available),
    }
}

/// Merge a drop into the gallery and store its writeup for the More Info panel.
/// The amend event carries real ids because the detail panel filters on them.
async fn apply_drop(
    app_handle: &crate::rt::AppHandle,
    badge: &crate::services::badge_polling_service::BadgeNotification,
) {
    use tauri::Emitter;

    let _ = crate::commands::badges::merge_pushed_badge_into_global_cache(badge).await;
    if let Some(enrichment) = &badge.enrichment {
        crate::commands::badge_metadata::store_enrichment_metadata(
            &badge.badge_set_id,
            &badge.badge_version,
            enrichment,
        )
        .await;
    }
    let _ = app_handle.emit(
        "badge-metadata-amended",
        serde_json::json!({
            "badge_set_id": badge.badge_set_id,
            "badge_version": badge.badge_version,
        }),
    );
}

/// Store a drop's earn window again when the feed has already recorded the
/// drop but the cache never kept the window. Android's notification worker
/// used to record drops as seen while its window writes never reached disk,
/// and an unchanged re-push is otherwise skipped, so those badges stayed
/// windowless (never Available, never in the missing strip) for good. Only the
/// enrichment is written: no gallery merge, no toast. True when it wrote one.
pub(crate) async fn heal_enrichment(
    badge: &crate::services::badge_polling_service::BadgeNotification,
) -> bool {
    let Some(enrichment) = &badge.enrichment else {
        return false;
    };
    let key = format!("metadata:{}-v{}", badge.badge_set_id, badge.badge_version);
    // The same enrichment, not just any: a lost write could also be an update
    // (the relay adding a window to a drop it first sent without one), leaving
    // the older copy in place.
    let kept = || {
        matches!(
            crate::services::universal_cache_service::peek_cached_entry(&key),
            Ok(Some(entry)) if entry.metadata.source == crate::commands::badge_metadata::ENRICHMENT_SOURCE
                && entry.data.get("enrichment") == Some(enrichment)
        )
    };
    if kept() {
        return false;
    }
    crate::commands::badge_metadata::store_enrichment_metadata(
        &badge.badge_set_id,
        &badge.badge_version,
        enrichment,
    )
    .await;
    // An enrichment with nothing in it is skipped by the store; reporting it
    // as healed would rebuild open surfaces on every frame for nothing.
    kept()
}

/// Merge back into the gallery any drop the feed recorded whose gallery entry
/// was lost the same way as its window (see `heal_enrichment`): a badge absent
/// from the gallery has no tile and can never reach the missing strip. Reads
/// the gallery once for the whole batch. True when it merged one.
async fn heal_gallery(badges: &[crate::services::badge_polling_service::BadgeNotification]) -> bool {
    let Ok(Some(gallery)) = crate::commands::badges::get_cached_global_badges().await else {
        return false;
    };
    let present: std::collections::HashSet<(&str, &str)> = gallery
        .data
        .iter()
        .flat_map(|s| s.versions.iter().map(move |v| (s.set_id.as_str(), v.id.as_str())))
        .collect();
    let mut merged = false;
    for badge in badges {
        if !present.contains(&(badge.badge_set_id.as_str(), badge.badge_version.as_str())) {
            merged |= crate::commands::badges::merge_pushed_badge_into_global_cache(badge)
                .await
                .unwrap_or(false);
        }
    }
    merged
}

/// Ingest a batch of drops from the real-time feed (the badge WebSocket, or its
/// `latest.json` poll fallback).
///
/// Drops are classified, not deduped, because the relay re-pushes under the
/// same id: first sighting toasts, an opening window toasts once, a corrected
/// writeup stores silently, an unchanged re-push does nothing.
#[tauri::command]
pub async fn ingest_badge_drops(
    app_handle: crate::rt::AppHandle,
    badges: Vec<crate::services::badge_polling_service::BadgeNotification>,
) -> Result<(), String> {
    use crate::services::badge_polling_service::{self as feed, FeedAction};
    use tauri::Emitter;

    if badges.is_empty() {
        return Ok(());
    }

    // Startup fires the poll fallback and the socket's history frame at once,
    // both carrying the same drops. Serialized so a badge is classified against
    // the previous batch's recorded state, not concurrently with it.
    static INGEST_LOCK: once_cell::sync::Lazy<tokio::sync::Mutex<()>> =
        once_cell::sync::Lazy::new(|| tokio::sync::Mutex::new(()));
    let _guard = INGEST_LOCK.lock().await;

    // Fresh install: record the relay's backlog as already-seen, no toasts.
    let seeding = feed::is_empty().await;

    let _ = crate::commands::badges::prune_invalid_global_badges().await;

    let mut stored_any = if seeding { false } else { heal_gallery(&badges).await };
    for badge in &badges {
        let feed_id = badge.feed_id();
        let hash = badge.content_hash();
        let available = is_window_open(badge);

        let action = if seeding {
            FeedAction::SilentStore
        } else {
            feed::classify(&feed_id, &hash, available).await
        };

        if !action.stores() {
            stored_any |= heal_enrichment(badge).await;
            continue;
        }

        apply_drop(&app_handle, badge).await;
        stored_any = true;

        // The toast and the stored notification row read the resolved window,
        // so a row keeps classifying itself correctly as time passes. Set on the
        // emitted copy only: the relay's payload, and so `content_hash`, never
        // carries it.
        let mut announced = badge.clone();
        announced.window = crate::services::badge_window::resolve(
            badge.date_info.as_deref(),
            badge.enrichment.as_ref(),
        )
        .map(|w| w.runs);

        match action {
            FeedAction::NotifyNew => {
                let _ = app_handle.emit("badge-notification", vec![announced.clone()]);
                if available {
                    let _ = app_handle.emit("badge-available", vec![announced.clone()]);
                }
            }
            FeedAction::NotifyAvailable => {
                let _ = app_handle.emit("badge-notification", vec![announced.clone()]);
                let _ = app_handle.emit("badge-available", vec![announced.clone()]);
            }
            // Corrections refresh open surfaces via apply_drop; no toast.
            FeedAction::SilentStore | FeedAction::Skip => {}
        }

        // After the store, so a failed store retries on the next frame.
        feed::record(&feed_id, &hash, action, available).await;
    }

    feed::persist().await;

    // One event per stored batch, so an open gallery rebuilds once rather than
    // once per drop (a startup batch carries up to fifty).
    if stored_any {
        let _ = app_handle.emit("badge-standing-changed", ());
    }
    Ok(())
}

#[cfg(test)]
mod window_tests {
    use super::is_window_open;
    use crate::services::badge_polling_service::{BadgeNotification, BadgeNotificationStatus};
    use chrono::{DateTime, Utc};
    use serde_json::json;

    /// The body `is_window_open` had before the window classifier existed,
    /// kept so the two can be compared case by case.
    fn legacy_is_window_open(badge: &BadgeNotification) -> bool {
        let iso = |key: &str| -> Option<DateTime<Utc>> {
            badge
                .enrichment
                .as_ref()?
                .get(key)?
                .as_str()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|d| d.with_timezone(&Utc))
        };
        match (iso("starts_utc"), iso("ends_utc")) {
            (None, None) => matches!(badge.status, BadgeNotificationStatus::Available),
            (start, end) => {
                let now = Utc::now();
                start.map(|s| now >= s).unwrap_or(true) && end.map(|e| now <= e).unwrap_or(true)
            }
        }
    }

    fn drop_with(
        status: BadgeNotificationStatus,
        date_info: Option<&str>,
        enrichment: Option<serde_json::Value>,
    ) -> BadgeNotification {
        BadgeNotification {
            badge_name: "Test".into(),
            badge_set_id: "test".into(),
            badge_version: "1".into(),
            badge_image_url: String::new(),
            badge_description: None,
            status,
            date_info: date_info.map(String::from),
            enrichment,
            window: None,
        }
    }

    #[test]
    fn classifies_every_campaign_shape_exactly_as_before() {
        let past = "2020-01-01T00:00:00Z";
        let future = "2099-01-01T00:00:00Z";
        let windows = [
            json!({ "starts_utc": past, "ends_utc": future }),
            json!({ "starts_utc": future, "ends_utc": "2099-02-01T00:00:00Z" }),
            json!({ "starts_utc": past, "ends_utc": "2020-02-01T00:00:00Z" }),
            json!({ "starts_utc": past }),
            json!({ "ends_utc": future }),
            json!({ "starts_utc": "not a date", "ends_utc": future }),
            json!({ "starts_utc": "2026-07-24T07:00Z" }),
            json!({}),
        ];
        for status in [
            BadgeNotificationStatus::New,
            BadgeNotificationStatus::Available,
            BadgeNotificationStatus::ComingSoon,
        ] {
            for w in &windows {
                let badge = drop_with(status, None, Some(w.clone()));
                assert_eq!(is_window_open(&badge), legacy_is_window_open(&badge), "{w}");
            }
            let bare = drop_with(status, None, None);
            assert_eq!(is_window_open(&bare), legacy_is_window_open(&bare));
        }
    }

    #[test]
    fn a_prose_only_window_falls_back_to_the_payload_status() {
        let badge = drop_with(BadgeNotificationStatus::New, Some("Dec 1-12"), None);
        assert!(!is_window_open(&badge));
        let badge = drop_with(BadgeNotificationStatus::Available, Some("Dec 1-12"), None);
        assert!(is_window_open(&badge));
    }

    #[test]
    fn the_emitted_window_never_changes_the_content_hash() {
        let badge = drop_with(
            BadgeNotificationStatus::Available,
            Some("2026-09-24T12:00:00Z - 2026-10-10T11:59:59Z"),
            Some(json!({ "starts_utc": "2026-09-24T12:00:00Z" })),
        );
        let before = badge.content_hash();
        let json = serde_json::to_value(&badge).unwrap();
        assert!(json.get("window").is_none(), "an unset window is not serialized");
        let mut announced = badge.clone();
        announced.window = crate::services::badge_window::resolve(
            badge.date_info.as_deref(),
            badge.enrichment.as_ref(),
        )
        .map(|w| w.runs);
        assert!(announced.window.is_some());
        assert_eq!(badge.content_hash(), before);
    }
}

/// Badge cache counts for the resource line, without waiting on the service lock.
pub fn cache_counts() -> Option<(usize, usize)> {
    BADGE_SERVICE.try_read().ok().and_then(|g| g.as_ref().and_then(|s| s.cache_counts()))
}
