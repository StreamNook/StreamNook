use crate::services::badge_service::{BadgeService, ThirdPartyGalleryBadge, UserBadgesResponse};
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
}

pub async fn get_service() -> Result<Arc<RwLock<Option<BadgeService>>>, String> {
    Ok(BADGE_SERVICE.clone())
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

/// Every distinct BetterTTV Pro loyalty badge image URL StreamNook has resolved
/// so far, across all users (persisted). The BetterTTV gallery tab renders one
/// tile per URL so discovered loyalty tiers are visible to everyone, not just
/// the account that owns them.
#[tauri::command]
pub async fn get_discovered_bttv_pro_badges() -> Result<Vec<String>, String> {
    Ok(crate::services::bttv_pro_service::get_discovered_bttv_pro_badges())
}

/// The primary account's badge standing: what it owns, every badge's earn
/// window, and the badges it is missing that are earnable right now. Answers
/// from cache at once; a due refresh runs in the background and is announced
/// with `badge-standing-changed`.
#[tauri::command]
pub async fn get_badge_standing(
    app_handle: tauri::AppHandle,
    force: Option<bool>,
) -> Result<crate::services::badge_standing::BadgeStanding, String> {
    Ok(crate::services::badge_standing::get_standing(&app_handle, force.unwrap_or(false)).await)
}

/// One badge's earn window, for a surface that shows a single badge.
#[tauri::command]
pub async fn get_badge_window(
    set_id: String,
    version: String,
) -> Result<Option<Vec<crate::services::badge_window::WindowRun>>, String> {
    Ok(crate::services::badge_standing::window_for(&set_id, &version).await)
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
    app_handle: &tauri::AppHandle,
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

/// Ingest a batch of drops from the real-time feed (the badge WebSocket, or its
/// `latest.json` poll fallback).
///
/// Drops are classified, not deduped, because the relay re-pushes under the
/// same id: first sighting toasts, an opening window toasts once, a corrected
/// writeup stores silently, an unchanged re-push does nothing.
#[tauri::command]
pub async fn ingest_badge_drops(
    app_handle: tauri::AppHandle,
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

    let mut stored_any = false;
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
