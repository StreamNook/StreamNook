use crate::models::settings::AppState;
use crate::services::twitch_service::TwitchService;
use anyhow::Result;
use log::{debug, error};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveNotification {
    pub streamer_name: String,
    pub streamer_login: String,
    pub streamer_avatar: Option<String>,
    pub game_name: Option<String>,
    pub game_image: Option<String>,
    pub stream_title: Option<String>,
    pub stream_url: String,
    #[serde(default)]
    pub is_test: bool,
    /// Which watcher raised this. `None` = the follow poller (every existing
    /// emitter). `Some("favorite")` = the favourites sweep, which the frontend
    /// gates on its own setting — a channel you favourited but don't follow
    /// must not be silenced by the follows toggle, and vice versa.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

pub struct LiveNotificationService {
    currently_live: Arc<RwLock<HashSet<String>>>,
    running: Arc<RwLock<bool>>,
    /// The first observed list seeds `currently_live` without notifying, so
    /// enabling the feature mid-session never announces everyone already live.
    seeded: AtomicBool,
}

impl LiveNotificationService {
    pub fn new() -> Self {
        Self {
            currently_live: Arc::new(RwLock::new(HashSet::new())),
            running: Arc::new(RwLock::new(false)),
            seeded: AtomicBool::new(false),
        }
    }

    /// Arm the service. The followed-streams poll that used to live here
    /// moved to `services::home_snapshot`, which calls `observe` with every
    /// fresh list, so one Helix call a minute serves notifications, Sidebar
    /// and Home instead of three separate fetches.
    pub async fn start(&self, _app_handle: AppHandle, _app_state: AppState) -> Result<()> {
        let mut running = self.running.write().await;
        *running = true;
        Ok(())
    }

    /// Diff a fresh followed-live list against the last one and notify for
    /// every channel that just went live. Fed by the Home snapshot poll.
    pub async fn observe(
        &self,
        app_handle: &AppHandle,
        app_state: &AppState,
        streams: &[crate::models::stream::TwitchStream],
    ) {
        if !*self.running.read().await {
            return;
        }
        let notifications_enabled = match app_state.settings.lock() {
            Ok(settings) => settings.live_notifications.enabled,
            Err(_) => false,
        };
        if !notifications_enabled {
            return;
        }
        let mut live_set = self.currently_live.write().await;
        if !self.seeded.swap(true, Ordering::SeqCst) {
            for stream in streams {
                live_set.insert(stream.user_login.clone());
            }
            return;
        }
        let mut new_live_streamers = Vec::new();
        for stream in streams {
            if !live_set.contains(&stream.user_login) {
                live_set.insert(stream.user_login.clone());
                new_live_streamers.push(stream.clone());
            }
        }
        let current_live_logins: HashSet<String> =
            streams.iter().map(|s| s.user_login.clone()).collect();
        live_set.retain(|login| current_live_logins.contains(login));
        drop(live_set);
        for stream in new_live_streamers {
            if let Err(e) = Self::send_notification(app_handle, &stream).await {
                error!("Failed to send live notification: {}", e);
            }
        }
    }

    pub async fn stop(&self) -> Result<()> {
        let mut running = self.running.write().await;
        *running = false;
        Ok(())
    }

    async fn send_notification(
        app_handle: &AppHandle,
        stream: &crate::models::stream::TwitchStream,
    ) -> Result<()> {
        // Fetch streamer avatar
        let streamer_avatar = match TwitchService::get_user_by_login(&stream.user_login).await {
            Ok(user) => user.profile_image_url,
            Err(_) => None,
        };

        // Get game image if game name is available
        let game_image = if !stream.game_name.is_empty() {
            Self::get_game_box_art(&stream.game_name).await.ok()
        } else {
            None
        };

        let notification = LiveNotification {
            streamer_name: stream.user_name.clone(),
            streamer_login: stream.user_login.clone(),
            streamer_avatar,
            game_name: Some(stream.game_name.clone()),
            game_image,
            stream_title: Some(stream.title.clone()),
            stream_url: format!("https://twitch.tv/{}", stream.user_login),
            is_test: false,
            source: None,
        };

        // Emit event to frontend (for in-app notifications)
        app_handle.emit("streamer-went-live", &notification)?;

        debug!(
            "[In-App Notification] {} is now live!",
            notification.streamer_name
        );

        Ok(())
    }

    async fn get_game_box_art(game_name: &str) -> Result<String> {
        let token = TwitchService::get_token().await?;
        let client = crate::services::http::client().clone();

        // Search for the game
        let url = format!(
            "https://api.twitch.tv/helix/games?name={}",
            urlencoding::encode(game_name)
        );

        let response = client
            .get(&url)
            .header("Client-Id", env!("TWITCH_APP_CLIENT_ID"))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        if let Some(data) = response.get("data").and_then(|d| d.as_array()) {
            if let Some(game) = data.first() {
                if let Some(box_art_url) = game.get("box_art_url").and_then(|u| u.as_str()) {
                    // Replace template variables with actual dimensions (285x380 box art)
                    let image_url = box_art_url
                        .replace("{width}", "285")
                        .replace("{height}", "380");
                    return Ok(image_url);
                }
            }
        }

        Err(anyhow::anyhow!("Game box art not found"))
    }
}
