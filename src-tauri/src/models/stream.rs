use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TwitchStream {
    pub id: String,
    pub user_id: String,
    pub user_name: String,
    pub user_login: String,
    pub title: String,
    pub viewer_count: u32,
    #[serde(default)]
    pub game_id: String,
    pub game_name: String,
    pub thumbnail_url: String,
    pub started_at: String,
    #[serde(default)]
    pub broadcaster_type: Option<String>,
    #[serde(default)]
    pub has_shared_chat: Option<bool>,
    #[serde(default)]
    pub profile_image_url: Option<String>,
    #[serde(default)]
    pub is_live: Option<bool>,
    // Free-form stream tags from the Helix streams response (e.g. "English",
    // "Speedrun"). Powers the category tag filter.
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    // Broadcast language in Helix form ("en", "fr", "zh-hk"). GQL paths must
    // normalize their enum spelling (FR, ZH_HK) to this form.
    #[serde(default)]
    pub language: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CategoryTag {
    pub id: String,
    pub localized_name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CategoryInfo {
    pub id: String,
    pub name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub followers_count: Option<u32>,
    #[serde(alias = "boxArtURL")]
    pub box_art_url: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<CategoryTag>>,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GqlGameResponse {
    pub game: Option<CategoryInfo>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GqlDataResponse {
    pub data: Option<GqlGameResponse>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TwitchClip {
    pub id: String,
    pub url: String,
    pub embed_url: String,
    pub broadcaster_id: String,
    pub broadcaster_name: String,
    /// Broadcaster LOGIN (lowercase), distinct from the display name. Chat replay
    /// keys a channel's third-party emote set off the login, and Helix clips only
    /// carry the display name, so this stays None on that path and the frontend
    /// resolves it from broadcaster_id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broadcaster_login: Option<String>,
    pub creator_id: String,
    pub creator_name: String,
    pub video_id: String,
    pub game_id: String,
    /// Category name, resolved inline by the GQL clip path (Helix only carries
    /// game_id, so this stays None there and the frontend resolves it lazily).
    /// Omitted from JSON when absent so it reads as `undefined`, not `null`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_name: Option<String>,
    pub language: String,
    pub title: String,
    pub view_count: u32,
    pub created_at: String,
    pub thumbnail_url: String,
    pub duration: f32,
    pub vod_offset: Option<u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TwitchVideo {
    pub id: String,
    pub stream_id: Option<String>,
    pub user_id: String,
    pub user_login: String,
    pub user_name: String,
    pub title: String,
    pub description: String,
    pub created_at: String,
    pub published_at: String,
    pub url: String,
    pub thumbnail_url: String,
    pub viewable: String,
    pub view_count: u32,
    pub language: String,
    #[serde(rename = "type")]
    pub video_type: String,
    pub duration: String,
    /// "recording" while the broadcast is still live (the VOD is still
    /// growing), "recorded" once finished. Absent from Helix rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Length in whole seconds, the number `duration` is formatted from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length_seconds: Option<u32>,
    /// The viewer's stored watch position for this video, joined on by
    /// `vod_progress_service::attach`. Absent when never watched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<crate::services::vod_progress_service::VodProgressSummary>,
}

/// Aggregated reaction ("likes") counts for one clip, from Twitch's clip
/// reactions feature. `total` is every reaction type summed (LIKE/LOVE/HYPE/
/// LAUGH/SAD); `like` is just the LIKE reaction. Keyed by the clip slug (`id`).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ClipReactions {
    pub id: String,
    pub total: u32,
    pub like: u32,
}
