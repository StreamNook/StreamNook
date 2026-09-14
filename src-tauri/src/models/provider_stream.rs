//! Provider-tagged stream rows for the multi-platform browse surface.
//!
//! `ProviderStream` is deliberately field-compatible with the Helix-shaped
//! `TwitchStream` (`user_login`, `user_name`, `viewer_count`, `thumbnail_url`,
//! `started_at`, `game_name`, `tags`) so the frontend renders Twitch and
//! provider cards through the same components with no adapter layer. The
//! additions are `provider`, the composite `key`, and `watch_url` (the
//! canonical platform URL handed to `start_stream`, which dispatches on it).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderStream {
    /// "kick" | "youtube" | "tiktok" (matches ProviderId on the frontend).
    pub provider: String,
    /// Composite "provider:channel" key (see `services::providers::key`).
    pub key: String,
    /// Platform stream/video id; empty when the platform doesn't expose one.
    #[serde(default)]
    pub id: String,
    /// Platform user id: numeric-as-string (Kick/TikTok) or UC id (YouTube).
    #[serde(default)]
    pub user_id: String,
    /// Slug / @handle / UC id; what chat and playback address.
    pub user_login: String,
    /// Display-cased name.
    pub user_name: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub viewer_count: u32,
    #[serde(default)]
    pub game_id: String,
    /// Category name; empty for platforms without categories (TikTok).
    #[serde(default)]
    pub game_name: String,
    /// Box art for `game_name`, when the platform ships it on the stream row.
    /// Kick does, which is what lets the category grid be built from the
    /// directory instead of a second request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category_thumbnail: Option<String>,
    /// Direct thumbnail URL (no Twitch-style {width} templating).
    #[serde(default)]
    pub thumbnail_url: String,
    /// ISO-UTC start time; empty when the platform doesn't expose it.
    #[serde(default)]
    pub started_at: String,
    #[serde(default)]
    pub profile_image_url: Option<String>,
    pub is_live: bool,
    /// Canonical platform watch URL; `start_stream` dispatches on this.
    pub watch_url: String,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

/// One page of browse/search results plus an opaque continuation cursor.
#[derive(Debug, Clone, Serialize)]
pub struct StreamPage {
    pub streams: Vec<ProviderStream>,
    pub cursor: Option<String>,
}

/// A browsable category on a platform (Twitch's "games" equivalent).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCategory {
    pub provider: String,
    /// Platform category id, as a string so it can ride the same routing as slugs.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub thumbnail: String,
    /// Viewers currently watching this category, where the platform reports it.
    #[serde(default)]
    pub viewer_count: u32,
    /// Live channels currently in this category, where known.
    #[serde(default)]
    pub channel_count: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct CategoryPage {
    pub categories: Vec<ProviderCategory>,
    pub cursor: Option<String>,
}
