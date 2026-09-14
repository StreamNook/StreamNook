//! `StreamSource`: the watch/browse facet of a platform adapter, sibling of
//! `ChatProvider`. Chat and watch stay independently implementable: TikTok has
//! chat without login AND playback without login, but a future platform might
//! ship only one facet. Implementations live in `{kick,youtube,tiktok}_media.rs`
//! and register alongside their chat adapters in `registry()`.

use crate::models::provider_stream::{CategoryPage, ProviderStream, StreamPage};
use anyhow::Result;
use async_trait::async_trait;

/// How the resolved stream is delivered, which decides the player path:
/// `Hls` rides the localhost HLS relay; `Flv` rides the streaming FLV relay
/// (mpegts.js on the frontend); `Mp4` is a direct `video.src` URL; `LocalHls`
/// is HLS the adapter is ALREADY serving from localhost, so it must be handed
/// to the player untouched rather than proxied a second time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackKind {
    Hls,
    LocalHls,
    Flv,
    Mp4,
}

/// One selectable rendition. For HLS platforms this is a variant from the
/// master playlist; for TikTok it is a discrete per-quality URL.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PlaybackQuality {
    /// Menu label ("1080p60", "720p", "sd", "audio_only").
    pub name: String,
    /// Media-playlist URL (HLS) or direct stream URL (FLV) for this rendition.
    pub url: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub bandwidth: Option<u64>,
}

/// Result of a watch-time resolution, shaped to fill `StreamStartResult`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ResolvedPlayback {
    pub kind: PlaybackKind,
    /// The selected rendition's URL (what the relay should serve).
    pub url: String,
    /// The quality actually served (closest match, mirrors twitch_resolver).
    pub quality: String,
    /// Quality menu, best-first.
    pub qualities: Vec<PlaybackQuality>,
}

/// What a platform adapter supports in this build. The frontend renders
/// browse/search/follow affordances from this instead of hard-coding platforms.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct SourceCaps {
    pub playback: bool,
    pub directory: bool,
    pub search: bool,
    /// Platform-side followed list (vs. the app-local follow list).
    pub native_follows: bool,
    pub live_check: bool,
}

#[async_trait]
pub trait StreamSource: Send + Sync {
    /// Stable provider id ("kick", "youtube", ...). Matches `ChatProvider::id`.
    fn id(&self) -> &'static str;

    fn caps(&self) -> SourceCaps;

    /// Resolve a channel to a playable URL at watch time. One call per watch is
    /// the budget for expensive paths (Kick's hidden-webview resolve); cache
    /// within the platform token/URL lifetime, never longer.
    ///
    /// `stream_id` names who this playback belongs to: the grid's cell id, or
    /// `stream_server::SOLO_STREAM_ID` for the main player. Adapters that serve
    /// through a per-stream local relay key their state on it; the rest ignore
    /// it. Without it a second stream silently repoints the first one's relay.
    async fn resolve_playback(
        &self,
        stream_id: &str,
        channel: &str,
        quality: &str,
    ) -> Result<ResolvedPlayback>;

    /// The quality menu WITHOUT starting playback.
    ///
    /// The default resolves and throws the url away, which is what every caller
    /// did before this existed. YouTube overrides it because resolving there
    /// STARTS a local relay session: enumerating the menu used to restart
    /// playback on the tallest rendition underneath a stream that was already
    /// running.
    async fn qualities(&self, channel: &str) -> Result<Vec<PlaybackQuality>> {
        self.resolve_playback("", channel, "best")
            .await
            .map(|r| r.qualities)
    }

    /// Channel metadata WITHOUT connecting chat. Cache-first: read the caches
    /// the chat adapters keep warm; refresh lazily with a TTL.
    async fn channel_meta(&self, channel: &str) -> Result<ProviderStream>;

    /// Browse: live directory, optionally scoped to a category id/name.
    /// Required, not defaulted: a default body here once masked YouTube's real
    /// implementations sitting outside the trait impl block, with every browse
    /// surface answering "not supported" while the code looked present.
    async fn directory(
        &self,
        category: Option<&str>,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<StreamPage>;

    async fn search(&self, query: &str) -> Result<StreamPage>;

    /// Browsable categories, most-watched first. This is what the Categories tab
    /// shows; `directory(Some(category_id), ..)` then lists the streams inside
    /// one. A platform with no category system implements this with an explicit
    /// `Err` and the UI falls back to a plain live grid.
    async fn categories(&self, cursor: Option<&str>, limit: u32) -> Result<CategoryPage>;

    /// Which of these channels are live right now (the who's-live poll).
    /// Implementations batch where the platform allows and stagger where not.
    async fn live_check(&self, channels: &[String]) -> Result<Vec<ProviderStream>>;

    /// Platform-side followed-live list. Only when `caps().native_follows`.
    async fn followed_live(&self) -> Result<Vec<ProviderStream>>;
}
