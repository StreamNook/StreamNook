//! Kick's browse + watch adapter (`StreamSource`).
//!
//! Two very different data paths, deliberately kept apart:
//!
//! * **Browse / search / who's-live** go through the OFFICIAL `api.kick.com`
//!   public API, which has no Cloudflare in front of it and works with either
//!   the connected user's OAuth token or an app (client-credentials) token. This
//!   is the only path allowed in polling loops.
//! * **Playback** needs `playback_url`, which lives ONLY on the internal
//!   `kick.com/api/v2/channels/{slug}` endpoint behind Cloudflare. That means one
//!   hidden-webview resolve per watch (the same resolver the chat adapter uses,
//!   so a channel you're already chatting in costs nothing extra). The resulting
//!   IVS master IS plain-fetchable, so only that one hop is expensive.
//!
//! Verified against the live API on 2026-08-20; the traps found there are noted
//! at each call site.

use crate::models::provider_stream::{CategoryPage, ProviderCategory, ProviderStream, StreamPage};
use crate::services::browse_languages;
use crate::services::kick_auth_service;
use crate::services::providers::hls_master;
use crate::services::providers::key::make_key;
use crate::services::providers::kick;
use crate::services::providers::source::{
    PlaybackKind, PlaybackQuality, ResolvedPlayback, SourceCaps, StreamSource,
};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const API: &str = "https://api.kick.com/public";
/// Kick's `channels` endpoint takes at most 50 slugs per call.
const CHANNELS_BATCH: usize = 50;
/// Kick documents each `slug` as "max 25 characters", and its usernames are
/// capped at 25 too. A follow list can still hold a longer one (seen: a
/// 32-character hex slug), and sending it fails its whole batch with
/// 400 "Invalid request", so it never leaves the app.
const SLUG_MAX: usize = 25;
/// Extra requests one sweep may spend per batch splitting a refused batch:
/// enough to isolate one bad slug among 50, few enough that an endpoint
/// refusing everything costs a bounded handful per sweep.
const SPLITS_PER_BATCH: usize = 12;

/// Slugs Kick refused (too long, or a 400 on their own). Later sweeps leave
/// them out instead of splitting a batch again every minute.
static REFUSED_SLUGS: Lazy<Mutex<HashSet<String>>> = Lazy::new(|| Mutex::new(HashSet::new()));
/// How long a parsed master stays usable for quality switching before we
/// re-resolve. Short, because the master URL carries a signed, expiring token.
const MASTER_TTL: Duration = Duration::from_secs(110);

static HTTP: Lazy<reqwest::Client> = Lazy::new(|| {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
});

/// A parsed master, kept so a quality switch re-selects from memory instead of
/// paying for another multi-second hidden-webview resolve.
struct CachedMaster {
    qualities: Vec<PlaybackQuality>,
    at: Instant,
}

static MASTERS: Lazy<Mutex<HashMap<String, CachedMaster>>> = Lazy::new(|| Mutex::new(HashMap::new()));

// --- Official API response shapes ------------------------------------------

#[derive(Deserialize)]
struct ApiEnvelope<T> {
    #[serde(default = "Vec::new")]
    data: Vec<T>,
}

/// `GET /public/v1/channels?slug=a&slug=b` — the browse tile + live state for
/// channels addressed BY SLUG, which is what our follow list stores.
#[derive(Deserialize)]
struct ChannelRow {
    #[serde(default)]
    broadcaster_user_id: Option<u64>,
    #[serde(default)]
    slug: String,
    #[serde(default)]
    stream_title: Option<String>,
    #[serde(default)]
    category: Option<CategoryRow>,
    #[serde(default)]
    stream: Option<StreamRow>,
}

#[derive(Deserialize)]
struct StreamRow {
    #[serde(default)]
    is_live: bool,
    #[serde(default)]
    viewer_count: u32,
    #[serde(default)]
    start_time: Option<String>,
    #[serde(default)]
    thumbnail: Option<String>,
}

#[derive(Deserialize, Clone)]
struct CategoryRow {
    #[serde(default)]
    id: Option<u64>,
    #[serde(default)]
    name: Option<String>,
    /// Kick marks this required on livestream rows, so a category grid can be
    /// built straight from the directory with no extra request.
    #[serde(default)]
    thumbnail: Option<String>,
}

/// `GET /public/v1/livestreams` (sorted) and `/public/v1/users/livestreams`.
/// v1 keeps the broadcaster fields flat; v2 nests them (see `V2LivestreamRow`).
#[derive(Deserialize)]
struct V1LivestreamRow {
    #[serde(default)]
    broadcaster_user_id: Option<u64>,
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    channel: Option<ChannelRef>,
    #[serde(default)]
    broadcaster_user: Option<BroadcasterRef>,
    #[serde(default)]
    stream_title: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    viewer_count: u32,
    #[serde(default)]
    thumbnail: Option<String>,
    #[serde(default)]
    started_at: Option<String>,
    #[serde(default)]
    category: Option<CategoryRow>,
    #[serde(default)]
    tags: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct ChannelRef {
    #[serde(default)]
    slug: Option<String>,
}

#[derive(Deserialize)]
struct BroadcasterRef {
    #[serde(default)]
    id: Option<u64>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    profile_picture: Option<String>,
}

/// `GET /public/v1/users?id=…` — display name + avatar, which the channels
/// endpoint does not return.
#[derive(Deserialize)]
struct UserRow {
    #[serde(default)]
    user_id: Option<u64>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    profile_picture: Option<String>,
}

// --- Helpers ---------------------------------------------------------------

/// `url` once per language in parallel, merged most-watched first, each
/// channel once, cut to `limit`. Fails only when every language failed.
async fn by_language(url: &str, languages: &[String], limit: usize) -> Result<Vec<ProviderStream>> {
    let calls = languages.iter().map(|lang| {
        let url = format!("{}&language={}", url, urlencoding::encode(lang));
        async move { api_get::<V1LivestreamRow>(&url).await }
    });
    let mut rows: Vec<V1LivestreamRow> = Vec::new();
    let mut failed = None;
    for result in futures::future::join_all(calls).await {
        match result {
            Ok(mut r) => rows.append(&mut r),
            Err(e) => failed = Some(e),
        }
    }
    if rows.is_empty() {
        if let Some(e) = failed {
            return Err(e);
        }
    }
    Ok(merge_by_viewers(rows.into_iter().filter_map(row_from_livestream).collect(), limit))
}

fn merge_by_viewers(mut streams: Vec<ProviderStream>, limit: usize) -> Vec<ProviderStream> {
    streams.sort_by(|a, b| b.viewer_count.cmp(&a.viewer_count));
    let mut seen = std::collections::HashSet::new();
    streams.retain(|s| seen.insert(s.key.clone()));
    streams.truncate(limit);
    streams
}

fn watch_url(slug: &str) -> String {
    format!("https://kick.com/{}", slug)
}

async fn token() -> Result<String> {
    kick_auth_service::read_token()
        .await
        .ok_or_else(|| anyhow!("Kick app credentials are not configured in this build"))
}

/// A non-success answer from the official API. Typed so a caller can tell a
/// request Kick refused (400) from one that never got an answer.
#[derive(Debug)]
struct ApiStatus {
    url: String,
    status: reqwest::StatusCode,
    body: String,
}

impl std::fmt::Display for ApiStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Kick API {} returned {}: {}", self.url, self.status, self.body)
    }
}

impl std::error::Error for ApiStatus {}

fn is_refused(e: &anyhow::Error) -> bool {
    e.downcast_ref::<ApiStatus>()
        .is_some_and(|s| s.status == reqwest::StatusCode::BAD_REQUEST)
}

async fn api_get<T: for<'de> Deserialize<'de>>(url: &str) -> Result<Vec<T>> {
    let tok = token().await?;
    let resp = HTTP
        .get(url)
        .bearer_auth(tok)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(ApiStatus {
            url: url.to_string(),
            status,
            body: body.chars().take(180).collect(),
        }
        .into());
    }
    Ok(resp.json::<ApiEnvelope<T>>().await?.data)
}

// TRAP: `slug[]=` silently returns an EMPTY array. Kick wants the parameter
// repeated: `?slug=a&slug=b`. Verified 2026-08-20.
fn channels_url(batch: &[String]) -> String {
    let query = batch
        .iter()
        .map(|c| format!("slug={}", urlencoding::encode(c)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{}/v1/channels?{}", API, query)
}

fn fits_slug_limit(slug: &str) -> bool {
    !slug.is_empty() && slug.chars().count() <= SLUG_MAX
}

/// The follow list as slugs the channels endpoint can take: lowercased, with
/// anything Kick refused before, or can never accept, left out.
fn queryable_slugs(channels: &[String]) -> Vec<String> {
    let lowered = channels.iter().map(|c| c.to_lowercase());
    let Ok(mut refused) = REFUSED_SLUGS.lock() else {
        return lowered.filter(|c| fits_slug_limit(c)).collect();
    };
    let mut out = Vec::with_capacity(channels.len());
    for slug in lowered {
        if refused.contains(&slug) {
            continue;
        }
        if !fits_slug_limit(&slug) {
            log::warn!("[Kick] '{}' is longer than any Kick slug; leaving it out of live checks", slug);
            refused.insert(slug);
            continue;
        }
        out.push(slug);
    }
    out
}

/// What one sweep over the channels endpoint got back.
struct ChannelSweep {
    rows: Vec<ChannelRow>,
    /// Slugs Kick refused on their own.
    refused: Vec<String>,
    /// Some batch was answered, so a refusal is about its slug, not the request.
    answered: bool,
    /// A refused batch ran out of splits before its bad slug was found.
    unresolved: bool,
}

/// Rows for `slugs`, `CHANNELS_BATCH` at a time. A batch Kick refuses (400) is
/// split until the slug behind it stands alone, so one bad follow costs its
/// own row instead of every row in its batch. Any other failure is an error.
async fn channels_in_batches<F, Fut>(slugs: &[String], mut fetch: F) -> Result<ChannelSweep>
where
    F: FnMut(Vec<String>) -> Fut,
    Fut: Future<Output = Result<Vec<ChannelRow>>>,
{
    let mut pending: Vec<Vec<String>> = slugs.chunks(CHANNELS_BATCH).rev().map(<[String]>::to_vec).collect();
    let mut splits_left = pending.len() * SPLITS_PER_BATCH;
    let mut sweep = ChannelSweep {
        rows: Vec::new(),
        refused: Vec::new(),
        answered: false,
        unresolved: false,
    };
    while let Some(batch) = pending.pop() {
        match fetch(batch.clone()).await {
            Ok(rows) => {
                sweep.answered = true;
                sweep.rows.extend(rows);
            }
            Err(e) if is_refused(&e) => {
                if batch.len() == 1 {
                    sweep.refused.extend(batch);
                } else if splits_left >= 2 {
                    splits_left -= 2;
                    let (first, second) = batch.split_at(batch.len() / 2);
                    pending.push(second.to_vec());
                    pending.push(first.to_vec());
                } else {
                    sweep.unresolved = true;
                }
            }
            Err(e) => return Err(e),
        }
    }
    Ok(sweep)
}

fn row_from_channel(row: ChannelRow, avatar: Option<String>, name: Option<String>) -> ProviderStream {
    let slug = row.slug.to_lowercase();
    // Kick's own live answer settles liveness. An ended stream's record lingers
    // with its last viewer count and preview, which drew as a live card with
    // stale numbers; an offline channel carries none of them.
    let stream = row.stream.filter(|s| s.is_live);
    ProviderStream {
        provider: "kick".to_string(),
        key: make_key("kick", &slug),
        id: String::new(),
        user_id: row.broadcaster_user_id.map(|i| i.to_string()).unwrap_or_default(),
        user_login: slug.clone(),
        user_name: name.unwrap_or_else(|| row.slug.clone()),
        title: row.stream_title.unwrap_or_default(),
        viewer_count: stream.as_ref().map(|s| s.viewer_count).unwrap_or(0),
        game_id: row
            .category
            .as_ref()
            .and_then(|c| c.id)
            .map(|i| i.to_string())
            .unwrap_or_default(),
        game_name: row
            .category
            .as_ref()
            .and_then(|c| c.name.clone())
            .unwrap_or_default(),
        category_thumbnail: row.category.as_ref().and_then(|c| c.thumbnail.clone()),
        thumbnail_url: stream.as_ref().and_then(|s| s.thumbnail.clone()).unwrap_or_default(),
        started_at: stream.as_ref().and_then(|s| s.start_time.clone()).unwrap_or_default(),
        profile_image_url: avatar,
        is_live: stream.is_some(),
        watch_url: watch_url(&slug),
        tags: None,
    }
}

fn row_from_livestream(row: V1LivestreamRow) -> Option<ProviderStream> {
    // v1 puts the slug at the top level, v2 nests it under `channel`.
    let slug = row
        .slug
        .or_else(|| row.channel.and_then(|c| c.slug))?
        .to_lowercase();
    let broadcaster = row.broadcaster_user;
    Some(ProviderStream {
        provider: "kick".to_string(),
        key: make_key("kick", &slug),
        id: String::new(),
        user_id: broadcaster
            .as_ref()
            .and_then(|b| b.id)
            .or(row.broadcaster_user_id)
            .map(|i| i.to_string())
            .unwrap_or_default(),
        user_login: slug.clone(),
        user_name: broadcaster
            .as_ref()
            .and_then(|b| b.username.clone())
            .unwrap_or_else(|| slug.clone()),
        title: row.stream_title.or(row.title).unwrap_or_default(),
        viewer_count: row.viewer_count,
        game_id: row
            .category
            .as_ref()
            .and_then(|c| c.id)
            .map(|i| i.to_string())
            .unwrap_or_default(),
        game_name: row
            .category
            .as_ref()
            .and_then(|c| c.name.clone())
            .unwrap_or_default(),
        category_thumbnail: row.category.as_ref().and_then(|c| c.thumbnail.clone()),
        thumbnail_url: row.thumbnail.unwrap_or_default(),
        started_at: row.started_at.unwrap_or_default(),
        profile_image_url: broadcaster.and_then(|b| b.profile_picture),
        is_live: true, // every row from a livestreams endpoint is live by definition
        watch_url: watch_url(&slug),
        tags: row.tags,
    })
}

/// Display names + avatars for broadcaster ids, which `channels` omits.
async fn users_by_id(ids: &[u64]) -> HashMap<u64, (Option<String>, Option<String>)> {
    let mut out = HashMap::new();
    if ids.is_empty() {
        return out;
    }
    for chunk in ids.chunks(CHANNELS_BATCH) {
        let query: String = chunk
            .iter()
            .map(|i| format!("id={}", i))
            .collect::<Vec<_>>()
            .join("&");
        match api_get::<UserRow>(&format!("{}/v1/users?{}", API, query)).await {
            Ok(rows) => {
                for r in rows {
                    if let Some(id) = r.user_id {
                        out.insert(id, (r.name, r.profile_picture));
                    }
                }
            }
            Err(e) => log::debug!("[Kick] user lookup failed: {}", e),
        }
    }
    out
}

pub struct KickSource;

impl KickSource {
    pub fn new() -> Self {
        Self
    }

    /// Re-resolve THIS channel from scratch and hand back a freshly signed url
    /// for the same quality.
    ///
    /// The IVS master carries a JWT that expires mid-session; when it does, the
    /// origin refuses every fetch and playback stops. `force` bypasses both the
    /// rendition cache and the channel meta, which is the only way to obtain a
    /// newly signed url. Until this existed, `force` was dead code and an expired
    /// url was terminal.
    pub async fn resign(&self, channel: &str, quality: &str) -> Result<String> {
        let slug = channel.to_lowercase();
        let qualities = self.qualities_for(&slug, true).await?;
        let (idx, _) = hls_master::select(&qualities, quality)
            .ok_or_else(|| anyhow!("no playable rendition for '{}'", slug))?;
        Ok(qualities[idx].url.clone())
    }

    /// Fetch + parse the IVS master for a channel, caching the renditions so a
    /// quality switch doesn't pay for another webview resolve.
    async fn qualities_for(&self, slug: &str, force: bool) -> Result<Vec<PlaybackQuality>> {
        if !force {
            if let Ok(cache) = MASTERS.lock() {
                if let Some(entry) = cache.get(slug) {
                    if entry.at.elapsed() < MASTER_TTL && !entry.qualities.is_empty() {
                        return Ok(entry.qualities.clone());
                    }
                }
            }
        }

        if force {
            // A stale signed master means the whole cached record is suspect.
            kick::invalidate_meta(slug);
        }
        let meta = kick::ensure_resolved(slug, true).await?;
        if !meta.is_live {
            return Err(anyhow!("{} is not live right now", slug));
        }
        let master_url = meta
            .playback_url
            .ok_or_else(|| anyhow!("Kick returned no playback URL for '{}'", slug))?;

        // The master lives on *.live-video.net, which (unlike kick.com/api) has
        // no Cloudflare gate, so a plain client is enough. Verified 2026-08-20.
        let resp = HTTP.get(&master_url).send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!(
                "Kick playlist fetch failed with {} for '{}'",
                resp.status(),
                slug
            ));
        }
        let body = resp.text().await?;
        let qualities = hls_master::parse(&body, &master_url);
        if qualities.is_empty() {
            return Err(anyhow!("Kick master playlist for '{}' had no variants", slug));
        }
        if let Ok(mut cache) = MASTERS.lock() {
            cache.insert(
                slug.to_string(),
                CachedMaster {
                    qualities: qualities.clone(),
                    at: Instant::now(),
                },
            );
        }
        Ok(qualities)
    }
}

#[async_trait]
impl StreamSource for KickSource {
    fn id(&self) -> &'static str {
        "kick"
    }

    fn caps(&self) -> SourceCaps {
        SourceCaps {
            playback: true,
            directory: true,
            // Exact-slug lookup only: the public API has no channel search.
            search: true,
            // Kick's OFFICIAL API exposes no followed-channels endpoint (their
            // own tracking issue for it is still open), so the list is imported
            // from the website session by `kick_account` and lives in
            // `Settings.provider_follows`. Liveness for those channels then runs
            // on the official API via `live_check`, which is why this stays
            // false: there is no per-poll platform call to make.
            native_follows: false,
            live_check: true,
        }
    }

    async fn resolve_playback(
        &self,
        _stream_id: &str,
        channel: &str,
        quality: &str,
    ) -> Result<ResolvedPlayback> {
        let slug = channel.to_lowercase();
        let qualities = self.qualities_for(&slug, false).await?;
        let (idx, label) = hls_master::select(&qualities, quality)
            .ok_or_else(|| anyhow!("no playable rendition for '{}'", slug))?;
        Ok(ResolvedPlayback {
            kind: PlaybackKind::Hls,
            url: qualities[idx].url.clone(),
            quality: label,
            qualities,
        })
    }

    async fn channel_meta(&self, channel: &str) -> Result<ProviderStream> {
        let slug = channel.to_lowercase();
        // The official API answers this without touching the webview resolver,
        // so the watch path can poll it cheaply while a stream is open.
        let rows: Vec<ChannelRow> = api_get(&format!(
            "{}/v1/channels?slug={}",
            API,
            urlencoding::encode(&slug)
        ))
        .await?;
        let row = rows
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("Kick channel '{}' not found", slug))?;
        let id = row.broadcaster_user_id;
        let mut named = None;
        let mut avatar = None;
        if let Some(id) = id {
            if let Some((n, a)) = users_by_id(&[id]).await.remove(&id) {
                named = n;
                avatar = a;
            }
        }
        Ok(row_from_channel(row, avatar, named))
    }

    async fn directory(
        &self,
        category: Option<&str>,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<StreamPage> {
        // v1 is deprecated but is the ONLY endpoint that honours
        // `sort=viewer_count`; v2 returns effectively unordered results, which
        // makes for a useless "browse" grid (verified 2026-08-20). Prefer v1 and
        // fall back to v2 if it ever disappears.
        let capped = limit.clamp(1, 100);
        let mut url = format!("{}/v1/livestreams?limit={}&sort=viewer_count", API, capped);
        if let Some(cat) = category.filter(|c| !c.is_empty()) {
            url.push_str(&format!("&category_id={}", urlencoding::encode(cat)));
        }
        // Kick ranks one worldwide list, so the viewer's languages are asked for
        // here. It takes one `language` per request (a repeat keeps the first,
        // `en,es` matches nothing), so each language is its own call, merged.
        let languages = browse_languages::kick_codes(&browse_languages::current());
        if !languages.is_empty() {
            match by_language(&url, &languages, capped as usize).await {
                Ok(streams) => return Ok(StreamPage { streams, cursor: None }),
                Err(e) => log::warn!("[Kick] directory by language failed ({}); asking without", e),
            }
        }
        // NOTE: v1 livestreams takes only broadcaster_user_id / category_id /
        // language / limit / sort — there is NO page or cursor parameter (checked
        // against the live spec). So v1 cannot page, and claiming otherwise would
        // make "load more" silently re-serve page one. We ask for a bigger single
        // page instead and report no cursor.
        let _ = cursor;

        match api_get::<V1LivestreamRow>(&url).await {
            Ok(rows) => {
                let streams: Vec<ProviderStream> =
                    rows.into_iter().filter_map(row_from_livestream).collect();
                // v1 pages by number, so the next cursor is simply the next page.
                // v1 has no paging parameter, so there is honestly nothing to
                // continue from; a synthesized cursor would just re-serve page one.
                let next = None;
                Ok(StreamPage {
                    streams,
                    cursor: next,
                })
            }
            Err(e) => {
                log::warn!("[Kick] sorted v1 directory failed ({}); falling back to v2 (unsorted)", e);
                let mut v2 = format!("{}/v2/livestreams?limit={}", API, capped);
                if let Some(cat) = category.filter(|c| !c.is_empty()) {
                    v2.push_str(&format!("&category_id={}", urlencoding::encode(cat)));
                }
                if let Some(cur) = cursor.filter(|c| !c.is_empty()) {
                    v2.push_str(&format!("&cursor={}", urlencoding::encode(cur)));
                }
                let rows: Vec<V1LivestreamRow> = api_get(&v2).await?;
                let mut streams: Vec<ProviderStream> =
                    rows.into_iter().filter_map(row_from_livestream).collect();
                // Sort what we were given so the grid is at least locally ordered.
                streams.sort_by(|a, b| b.viewer_count.cmp(&a.viewer_count));
                Ok(StreamPage {
                    streams,
                    cursor: None,
                })
            }
        }
    }

    /// Top categories, built by aggregating the live directory rather than
    /// listing `/public/v2/categories`.
    ///
    /// Two reasons. The v2 category list has no viewer ordering (its livestreams
    /// sibling is already known to ignore `sort`), so it would put dead
    /// categories at the top of the grid; and every livestream row is required
    /// to carry `category.{id,name,thumbnail}`, so grouping the streams we
    /// already fetch yields a *viewer-ranked* grid — the same thing Twitch's
    /// top-games endpoint gives us — for a single request and no extra API
    /// surface. The trade-off is that it only sees categories with streams in
    /// the sampled window, which is exactly what belongs on a browse grid.
    async fn categories(&self, _cursor: Option<&str>, limit: u32) -> Result<CategoryPage> {
        // Sample deeper than we display so the tail of the grid is still real.
        let page = self.directory(None, None, 100).await?;

        let mut order: Vec<String> = Vec::new();
        let mut totals: HashMap<String, ProviderCategory> = HashMap::new();
        for stream in &page.streams {
            if stream.game_id.is_empty() && stream.game_name.is_empty() {
                continue; // uncategorised
            }
            let key = if stream.game_id.is_empty() {
                stream.game_name.to_lowercase()
            } else {
                stream.game_id.clone()
            };
            let entry = totals.entry(key.clone()).or_insert_with(|| {
                order.push(key.clone());
                ProviderCategory {
                    provider: "kick".to_string(),
                    id: stream.game_id.clone(),
                    name: stream.game_name.clone(),
                    thumbnail: stream.category_thumbnail.clone().unwrap_or_default(),
                    viewer_count: 0,
                    channel_count: 0,
                }
            });
            entry.viewer_count = entry.viewer_count.saturating_add(stream.viewer_count);
            entry.channel_count += 1;
            if entry.thumbnail.is_empty() {
                if let Some(t) = &stream.category_thumbnail {
                    entry.thumbnail = t.clone();
                }
            }
        }

        let mut categories: Vec<ProviderCategory> =
            order.into_iter().filter_map(|k| totals.remove(&k)).collect();
        categories.sort_by(|a, b| b.viewer_count.cmp(&a.viewer_count));
        categories.truncate(limit.clamp(1, 100) as usize);
        Ok(CategoryPage {
            categories,
            // One aggregated sample, so there is nothing further to page through.
            cursor: None,
        })
    }

    async fn search(&self, query: &str) -> Result<StreamPage> {
        // The public API has no channel search, so this is an exact-slug jump:
        // typing a channel name takes you to that channel. Kick's own search is
        // internal (Cloudflare-gated) and not worth a webview per keystroke.
        let slug = query.trim().to_lowercase().replace(' ', "-");
        if slug.is_empty() {
            return Ok(StreamPage {
                streams: vec![],
                cursor: None,
            });
        }
        match self.channel_meta(&slug).await {
            Ok(row) => Ok(StreamPage {
                streams: vec![row],
                cursor: None,
            }),
            // "not found" is a normal empty result here, not an error.
            Err(_) => Ok(StreamPage {
                streams: vec![],
                cursor: None,
            }),
        }
    }

    async fn followed_live(&self) -> Result<Vec<ProviderStream>> {
        // Never called while `native_follows` is false: Kick follows are
        // imported by `kick_account` and swept by `live_check`.
        Err(anyhow::anyhow!("followed_live is not supported on kick"))
    }

    async fn live_check(&self, channels: &[String]) -> Result<Vec<ProviderStream>> {
        let slugs = queryable_slugs(channels);
        if slugs.is_empty() {
            return Ok(vec![]);
        }
        let sweep = channels_in_batches(&slugs, |batch: Vec<String>| async move {
            api_get::<ChannelRow>(&channels_url(&batch)).await
        })
        .await?;
        // A refusal counts only once Kick has answered something this sweep: an
        // endpoint refusing every request says nothing about any one slug.
        if sweep.answered && !sweep.refused.is_empty() {
            if let Ok(mut refused) = REFUSED_SLUGS.lock() {
                for slug in &sweep.refused {
                    if refused.insert(slug.clone()) {
                        log::warn!("[Kick] the channels API refuses '{}'; leaving it out of live checks", slug);
                    }
                }
            }
        }
        // Publishing a sweep that missed channels would report them offline, so
        // the last known list stays up instead (the next sweep skips what this
        // one learned to leave out).
        if !sweep.answered {
            return Err(anyhow!("Kick refused every channels lookup for {} followed channel(s)", slugs.len()));
        }
        if sweep.unresolved {
            return Err(anyhow!("Kick refused a channels batch this sweep could not narrow down"));
        }

        // One extra batched call fills in display names + avatars, which the
        // channels endpoint omits but every browse tile wants.
        let ids: Vec<u64> = sweep.rows.iter().filter_map(|r| r.broadcaster_user_id).collect();
        let mut users = users_by_id(&ids).await;
        let mut out = Vec::with_capacity(sweep.rows.len());
        for row in sweep.rows {
            let (name, avatar) = row
                .broadcaster_user_id
                .and_then(|id| users.remove(&id))
                .unwrap_or((None, None));
            out.push(row_from_channel(row, avatar, name));
        }

        // TRAP: the official API can report `is_live: false` for a channel
        // kick.com itself shows live (seen 2026-08-25 with a week-long session:
        // v1 said offline, the site API said live). So spot-check the channels
        // v1 called offline against the site API the website renders from. A
        // plain browser-header GET, no webview; any failure keeps v1's answer.
        let live_slugs: std::collections::HashSet<String> = out
            .iter()
            .filter(|r| r.is_live)
            .map(|r| r.user_login.clone())
            .collect();
        const V2_SPOTCHECK_MAX: usize = 12;
        let missing: Vec<String> = slugs
            .iter()
            .filter(|c| !live_slugs.contains(*c))
            .take(V2_SPOTCHECK_MAX)
            .cloned()
            .collect();
        if slugs.len() > live_slugs.len() + V2_SPOTCHECK_MAX {
            log::debug!(
                "[Kick] live spot-check covering {} of {} offline-reported channels this sweep",
                V2_SPOTCHECK_MAX,
                slugs.len() - live_slugs.len()
            );
        }
        for slug in missing {
            if let Some(row) = v2_live_row(&slug).await {
                // Drop v1's offline row for this channel so the live one stands alone.
                out.retain(|r| !(r.user_login == slug && !r.is_live));
                out.push(row);
            }
        }
        Ok(out)
    }
}

/// Ask kick.com's own site API whether `slug` is live, returning a live row when
/// it is. This is the fallback for the official API's false-offlines; `None`
/// covers "actually offline" and every failure mode alike, so the caller simply
/// keeps the official answer.
async fn v2_live_row(slug: &str) -> Option<ProviderStream> {
    let url = format!("https://kick.com/api/v2/channels/{}", urlencoding::encode(slug));
    let resp = crate::services::providers::kick::browser_get(&url, slug).await?;
    let v: serde_json::Value = resp.json().await.ok()?;
    let live = v.get("livestream")?;
    if live.is_null() {
        return None;
    }
    let s = |val: &serde_json::Value, key: &str| {
        val.get(key).and_then(|x| x.as_str()).map(|x| x.to_string())
    };
    let category = live
        .get("categories")
        .and_then(|c| c.as_array())
        .and_then(|c| c.first())
        .cloned();
    Some(ProviderStream {
        provider: "kick".to_string(),
        key: make_key("kick", slug),
        id: String::new(),
        user_id: v.get("user_id").and_then(|x| x.as_u64()).map(|i| i.to_string()).unwrap_or_default(),
        user_login: slug.to_string(),
        user_name: v
            .get("user")
            .and_then(|u| s(u, "username"))
            .unwrap_or_else(|| slug.to_string()),
        title: s(live, "session_title").unwrap_or_default(),
        viewer_count: live.get("viewer_count").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
        game_id: category
            .as_ref()
            .and_then(|c| c.get("id"))
            .and_then(|x| x.as_u64())
            .map(|i| i.to_string())
            .unwrap_or_default(),
        game_name: category.as_ref().and_then(|c| s(c, "name")).unwrap_or_default(),
        category_thumbnail: None,
        thumbnail_url: live
            .get("thumbnail")
            .and_then(|t| s(t, "url"))
            .unwrap_or_default(),
        started_at: s(live, "created_at").unwrap_or_default(),
        profile_image_url: v.get("user").and_then(|u| s(u, "profile_pic")),
        is_live: true,
        watch_url: watch_url(slug),
        tags: None,
    })
}

#[cfg(test)]
mod channel_row_tests {
    use super::*;

    fn channel(stream: serde_json::Value) -> ChannelRow {
        serde_json::from_value(serde_json::json!({
            "broadcaster_user_id": 7,
            "slug": "Shroud",
            "stream_title": "last title",
            "stream": stream,
        }))
        .unwrap()
    }

    #[test]
    fn an_ended_stream_is_an_offline_row_with_no_stale_numbers() {
        let row = row_from_channel(
            channel(serde_json::json!({ "is_live": false, "viewer_count": 812, "thumbnail": "old.webp", "start_time": "2026-10-16T10:00:00Z" })),
            None,
            None,
        );
        assert!(!row.is_live);
        assert_eq!(row.viewer_count, 0);
        assert_eq!(row.thumbnail_url, "");
        assert_eq!(row.started_at, "");
        assert_eq!(row.user_login, "shroud");
    }

    #[test]
    fn a_live_stream_keeps_its_numbers() {
        let row = row_from_channel(
            channel(serde_json::json!({ "is_live": true, "viewer_count": 812, "thumbnail": "now.webp", "start_time": "2026-10-16T10:00:00Z" })),
            None,
            None,
        );
        assert!(row.is_live);
        assert_eq!(row.viewer_count, 812);
        assert_eq!(row.thumbnail_url, "now.webp");
    }

    #[test]
    fn no_stream_record_is_offline() {
        let row = row_from_channel(channel(serde_json::Value::Null), None, None);
        assert!(!row.is_live);
    }
}

// One follow Kick will not take used to fail its batch of 50 with a 400, and
// the `?` on that batch skipped every batch after it: nobody on Kick showed live.
#[cfg(test)]
mod live_batch_tests {
    use super::*;
    use std::sync::Arc;

    const HEX_SLUG: &str = "5a1ae58d6e95443487b265aac0be1a5c";

    fn row(slug: &str) -> ChannelRow {
        serde_json::from_value(serde_json::json!({ "slug": slug })).unwrap()
    }

    fn api_error(status: reqwest::StatusCode) -> anyhow::Error {
        ApiStatus {
            url: "https://api.kick.com/public/v1/channels".into(),
            status,
            body: r#"{"data":{},"message":"Invalid request"}"#.into(),
        }
        .into()
    }

    fn follows(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("chan{}", i)).collect()
    }

    /// Answers like Kick did in the report: a batch holding any slug in `bad`
    /// is refused whole. Returns the sweep and how many requests it made.
    async fn sweep(slugs: &[String], bad: &[String]) -> (Result<ChannelSweep>, usize) {
        let calls = Arc::new(Mutex::new(0usize));
        let counter = calls.clone();
        let bad = bad.to_vec();
        let result = channels_in_batches(slugs, move |batch: Vec<String>| {
            *counter.lock().unwrap() += 1;
            let refuse = batch.iter().any(|s| bad.contains(s));
            async move {
                if refuse {
                    Err(api_error(reqwest::StatusCode::BAD_REQUEST))
                } else {
                    Ok(batch.iter().map(|s| row(s)).collect())
                }
            }
        })
        .await;
        let n = *calls.lock().unwrap();
        (result, n)
    }

    #[test]
    fn a_slug_longer_than_kick_allows_never_reaches_the_api() {
        assert!(!fits_slug_limit(HEX_SLUG));
        assert!(!fits_slug_limit(""));
        assert!(fits_slug_limit("maj3r"));
        assert!(fits_slug_limit(&"a".repeat(SLUG_MAX)));

        let long = "x".repeat(SLUG_MAX + 1);
        assert_eq!(queryable_slugs(&["MAJ3R".to_string(), long.clone()]), vec!["maj3r"]);
        assert!(REFUSED_SLUGS.lock().unwrap().contains(&long));
    }

    #[tokio::test]
    async fn one_refused_slug_costs_only_its_own_row() {
        let slugs = follows(50);
        let (result, calls) = sweep(&slugs, &["chan49".to_string()]).await;
        let sweep = result.expect("a 400 is narrowed down, not returned");
        assert_eq!(sweep.rows.len(), 49);
        assert_eq!(sweep.refused, vec!["chan49"]);
        assert!(sweep.answered && !sweep.unresolved);
        assert!(calls <= 1 + SPLITS_PER_BATCH, "{} requests", calls);
    }

    #[tokio::test]
    async fn a_refused_batch_no_longer_hides_the_batch_after_it() {
        let slugs = follows(100);
        let (result, _) = sweep(&slugs, &["chan0".to_string()]).await;
        let sweep = result.unwrap();
        assert_eq!(sweep.rows.len(), 99);
        assert!(sweep.rows.iter().any(|r| r.slug == "chan99"));
        assert_eq!(sweep.refused, vec!["chan0"]);
    }

    #[tokio::test]
    async fn bad_slugs_beyond_one_sweep_are_found_by_the_next() {
        let slugs = follows(50);
        let bad = vec!["chan3".to_string(), "chan40".to_string()];
        let (first, _) = sweep(&slugs, &bad).await;
        let first = first.unwrap();
        assert!(first.answered);
        assert!(!first.refused.is_empty(), "the first sweep isolates at least one");

        let rest: Vec<String> = slugs.iter().filter(|s| !first.refused.contains(s)).cloned().collect();
        let (second, _) = sweep(&rest, &bad).await;
        let second = second.unwrap();
        assert!(!second.unresolved);
        let mut found: Vec<String> = first.refused.into_iter().chain(second.refused).collect();
        found.sort();
        assert_eq!(found, bad);
        assert_eq!(second.rows.len(), 48);
    }

    #[tokio::test]
    async fn an_endpoint_refusing_everything_marks_no_slug_and_stays_bounded() {
        let slugs = follows(100);
        let (result, calls) = sweep(&slugs, &slugs).await;
        let sweep = result.unwrap();
        assert!(!sweep.answered, "nothing answered, so no refusal can be blamed on a slug");
        assert!(calls <= 2 + 2 * SPLITS_PER_BATCH, "{} requests", calls);
    }

    #[tokio::test]
    async fn any_other_failure_is_an_error_as_before() {
        let slugs = follows(60);
        let result = channels_in_batches(&slugs, |_batch: Vec<String>| async {
            Err::<Vec<ChannelRow>, _>(api_error(reqwest::StatusCode::INTERNAL_SERVER_ERROR))
        })
        .await;
        assert!(result.is_err());
    }
}
