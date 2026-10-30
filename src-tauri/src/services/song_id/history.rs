//! Your songs: every song the user identified, newest heard first, kept until
//! they remove it.
//!
//! Filed where the match is made (`commands::song_id::identify_song`), so the
//! player's music button, /song in any chat window and the phone all keep what
//! they found without asking for it. A song is one row however often it is
//! heard: hearing it again moves it to the top, counts it and notes where
//! (`sightings`, newest first). Pages (`page`) and one song's detail (`get`)
//! are what the WebView reads; search, order and paging happen here.
//!
//! Storage: `<app_data>/song_history.json`, written whole through the cache's
//! atomic writer (flushed, then renamed) right after each change the user
//! makes. A song is found a few times an hour at most, so there is nothing to
//! debounce, and a phone killed in the background loses nothing. Links a
//! background refresh finds (links.rs) are the one exception: they are held as
//! a pending change and written once, `DEFERRED_WRITE` later, by the unloader,
//! or at exit. Read on first use and let go of again `IDLE_UNLOAD` after the
//! last, so the app holds no songs while nobody looks. A file that is there
//! but cannot be read is never written over; one that does not parse is moved
//! aside as `song_history.json.bad-<ms>`.
//!
//! Cover colors (cover_tint.rs) live beside the songs in their own file,
//! loaded and let go with them under the same lock, with their own writer
//! and dirty flag. They are a cache: they never fail a load, never keep the
//! songs in memory, and never ride on the history's writes.
//!
//! Versions: a field added later must be optional, so every app reads every
//! file of `FILE_VERSION`. A file whose `version` is higher was written by a
//! newer StreamNook: this one shows what it can read of it and never writes
//! over it or moves it aside, so going back to an older build and forward
//! again loses nothing.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::Emitter;

use super::cover_tint::{self, TintEntry, TintView, Tints};
use super::links;
use super::services;
use super::shazam::{IsrcFrom, Provider, SongMatch};
use crate::rt::AppHandle;
use crate::services::cache_service::get_app_data_dir;
use crate::services::providers::key::is_provider_id;
use crate::services::universal_cache_service::write_atomic_with;
use crate::utils::text_fold::fold_search;

const FILE: &str = "song_history.json";
const FILE_VERSION: u32 = 1;
/// Distinct songs kept. Past this the one heard longest ago goes.
pub const MAX_SONGS: usize = 10_000;
/// Places a song was heard, kept per song, newest first.
pub const MAX_SIGHTINGS: usize = 10;
/// The same song on the same channel again within this long (the button
/// pressed twice, a retry) is one hearing, not two.
pub const REPEAT_WINDOW_MS: i64 = 15 * 60 * 1000;
/// How long the songs stay in memory after their last use.
const IDLE_UNLOAD: Duration = Duration::from_secs(60);
/// Rows per page, at most.
pub const MAX_PAGE: usize = 100;
/// Search words used, at most, and characters kept per word.
const MAX_TERMS: usize = 8;
const MAX_TERM_CHARS: usize = 64;
/// Characters of a query read at all (the words past `MAX_TERMS` go unused).
const MAX_QUERY_CHARS: usize = MAX_TERMS * (MAX_TERM_CHARS + 1);
/// Characters kept of a channel's login or name, or a link's service name.
const MAX_FIELD: usize = 120;
/// Characters kept of a title, an artist or an album.
const MAX_TEXT: usize = 300;
/// Characters kept of a link; a longer one is dropped, not cut.
const MAX_URL: usize = 2048;
/// How far into a broadcast or video a hearing is kept as being; past this
/// it is not a real position.
const MAX_POSITION_SECS: u32 = 30 * 24 * 60 * 60;
/// Why a change is refused while the file is a newer app's.
const NEWER_FILE: &str =
    "Your songs were saved by a newer StreamNook, so this one only shows them.";
/// Payload: `Change`. Every window hears it, since any of them may show the list.
pub const CHANGED_EVENT: &str = "song-history-changed";
/// Time bounds a page groups its songs by, at most.
const MAX_BOUNDS: usize = 64;
/// Songs waiting for a link refresh, at most.
const REFRESH_CAP: usize = 100;
/// The link refresh's time budget (links.rs splits it between its two steps;
/// each lookup's share starts after its host's gate).
const REFRESH_BUDGET: Duration = Duration::from_secs(6);
/// How long links a refresh found wait in memory before they are written.
const DEFERRED_WRITE: Duration = Duration::from_secs(30);
/// A song whose links were last checked longer ago than this is checked
/// again when it is opened, while a service the finder can fill lacks a link.
const STALE_LINKS_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// Where a song was playing: the channel the captured player showed, and the
/// stream, video or clip it showed then. Every field past `media` came later
/// and is missing from hearings kept before it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct SongSource {
    pub provider: String,
    pub channel_login: String,
    pub channel_name: Option<String>,
    /// "live", "video" or "clip".
    pub media: Option<String>,
    /// The stream's, video's or clip's title at the time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The category it was in ("Just Chatting").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// The channel's picture, a small copy when its host draws one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    /// How far in it was heard: the broadcast's run time when live, the
    /// video's own time on a VOD.
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "lenient_secs"
    )]
    pub position_secs: Option<u32>,
}

/// A position as any number, a fraction cut off. Anything that is no real
/// position (negative, too far, not a number) is left out, so it never costs
/// the rest of the source.
fn lenient_secs<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u32>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(d)?;
    Ok(value
        .and_then(|v| v.as_f64())
        .filter(|s| s.is_finite() && *s >= 0.0 && *s <= f64::from(MAX_POSITION_SECS))
        .map(|s| s as u32))
}

impl SongSource {
    /// The source as stored: a known platform, trimmed and bounded fields, or
    /// None when it names no channel.
    fn cleaned(self) -> Option<SongSource> {
        let provider = self.provider.trim().to_ascii_lowercase();
        if !is_provider_id(&provider) {
            return None;
        }
        let channel_login = clip(self.channel_login.trim());
        if channel_login.is_empty() {
            return None;
        }
        let channel_name = self
            .channel_name
            .map(|n| clip(n.trim()))
            .filter(|n| !n.is_empty());
        let media = self
            .media
            .map(|m| m.trim().to_ascii_lowercase())
            .filter(|m| matches!(m.as_str(), "live" | "video" | "clip"));
        let title = self
            .title
            .map(|t| clip_to(t.trim(), MAX_TEXT))
            .filter(|t| !t.is_empty());
        let category = self
            .category
            .map(|c| clip(c.trim()))
            .filter(|c| !c.is_empty());
        let avatar = web_url(&self.avatar).map(|url| small_avatar(&url).unwrap_or(url));
        // A clip's own time says nothing about where in the broadcast it was.
        let position_secs = self
            .position_secs
            .filter(|s| *s <= MAX_POSITION_SECS && media.as_deref() != Some("clip"));
        Some(SongSource {
            provider,
            channel_login,
            channel_name,
            media,
            title,
            category,
            avatar,
            position_secs,
        })
    }

    fn same_channel(&self, other: &SongSource) -> bool {
        self.provider == other.provider
            && self
                .channel_login
                .eq_ignore_ascii_case(&other.channel_login)
    }
}

fn clip(s: &str) -> String {
    clip_to(s, MAX_FIELD)
}

fn clip_to(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// Whether the history keeps `url` as a link: an http(s) address of a sane
/// length. Shazam's and Odesli's answers, and the file on disk, are not
/// trusted to hold only those.
fn is_web_url(url: &str) -> bool {
    url.len() <= MAX_URL && (url.starts_with("https://") || url.starts_with("http://"))
}

fn web_url(url: &Option<String>) -> Option<String> {
    url.as_ref().filter(|u| is_web_url(u)).cloned()
}

/// `url` when it is a web link to the service called `name` (services.rs).
fn service_url(url: &Option<String>, name: &str) -> Option<String> {
    web_url(url).filter(|u| services::is_link_to(name, u))
}

/// The links kept: each named for a service and going to that service's own
/// host, so no link is ever drawn with a logo it does not go to. A search
/// page is marked as one whatever it came marked as.
fn web_providers(providers: &[Provider]) -> Vec<Provider> {
    providers
        .iter()
        .filter(|p| is_web_url(&p.url) && services::is_link_to(p.name.trim(), &p.url))
        .map(|p| Provider {
            name: p.name.trim().to_string(),
            url: p.url.clone(),
            search: p.search || services::is_search_page(&p.url),
        })
        .collect()
}

/// One link per service: the first direct one, else the first search page.
fn one_per_service(providers: Vec<Provider>) -> Vec<Provider> {
    let mut out: Vec<Provider> = Vec::with_capacity(providers.len());
    for p in providers {
        links::put(&mut out, p);
    }
    out
}

/// What older builds read: every link is a search page.
fn all_searches(providers: &[Provider]) -> bool {
    !providers.is_empty() && providers.iter().all(|p| p.search)
}

/// A match as the history keeps it: bounded text, web links only, one per
/// service with the track's own first, and a valid ISRC and Apple id.
fn kept_match(song: &SongMatch) -> SongMatch {
    let mut providers = web_providers(&song.providers);
    if song.links_are_searches {
        for p in &mut providers {
            p.search = true;
        }
    }
    let mut providers = one_per_service(providers);
    links::sort_providers(&mut providers);
    let isrc = song.isrc.as_deref().and_then(links::valid_isrc);
    SongMatch {
        title: clip_to(song.title.trim(), MAX_TEXT),
        artist: clip_to(song.artist.trim(), MAX_TEXT),
        album: song
            .album
            .as_deref()
            .map(|a| clip_to(a.trim(), MAX_TEXT))
            .filter(|a| !a.is_empty()),
        album_art: web_url(&song.album_art),
        shazam_url: service_url(&song.shazam_url, "Shazam"),
        song_link: service_url(&song.song_link, "song.link"),
        links_are_searches: all_searches(&providers),
        providers,
        key: None,
        isrc_from: isrc.as_ref().map(|_| song.isrc_from.unwrap_or(IsrcFrom::Text)),
        isrc,
        apple_id: song.apple_id.as_deref().and_then(links::valid_apple_id),
        links_complete: song.links_complete,
    }
}

/// A value of a field added later, or None when this build cannot read it:
/// a newer build's value never costs the rest of the song (or the file).
fn lenient<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = Option::<serde_json::Value>::deserialize(d)?;
    Ok(value.and_then(|v| serde_json::from_value(v).ok()))
}

fn lenient_ms<'de, D: serde::Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(d)?;
    Ok(value
        .and_then(|v| v.as_i64())
        .filter(|ms| *ms >= 0)
        .unwrap_or(0))
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

/// One time a song was heard.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Sighting {
    pub at_ms: i64,
    pub source: Option<SongSource>,
}

/// One song in the history, with everything its detail shows. Every field has
/// a default, so a row missing one still reads.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct SongEntry {
    pub key: String,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub album_art: Option<String>,
    pub shazam_url: Option<String>,
    pub song_link: Option<String>,
    /// The track's own links first (links.rs `DISPLAY` order), then search
    /// pages, then services off that list in the order they were kept.
    pub providers: Vec<Provider>,
    /// Every provider is a search page. Derived from `providers` and written
    /// for older builds, which read only this.
    pub links_are_searches: bool,
    /// The recording's ISRC, when one is known.
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "lenient"
    )]
    pub isrc: Option<String>,
    /// Where `isrc` came from; a text search's never replaces Shazam's.
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "lenient"
    )]
    pub isrc_from: Option<IsrcFrom>,
    /// When a link lookup last answered cleanly for every service it asked
    /// (ms); 0 is never, which asks for a refresh when the song is opened.
    #[serde(skip_serializing_if = "is_zero", deserialize_with = "lenient_ms")]
    pub links_checked_ms: i64,
    pub first_ms: i64,
    pub last_ms: i64,
    /// Times heard (repeats inside `REPEAT_WINDOW_MS` count once).
    pub count: u32,
    pub sightings: Vec<Sighting>,
    /// The folded words search looks in. Rebuilt on load, never stored.
    #[serde(skip)]
    haystack: String,
}

impl SongEntry {
    /// Take what a newer match found: it names the song (Shazam corrects
    /// titles over time), and a field it left out keeps what an earlier match
    /// found. Its links go through the one merge the link refresh uses.
    fn refresh_from(&mut self, song: &SongMatch, now_ms: i64) {
        if !song.title.is_empty() {
            self.title = song.title.clone();
        }
        if !song.artist.is_empty() {
            self.artist = song.artist.clone();
        }
        if song.album.is_some() {
            self.album = song.album.clone();
        }
        if song.album_art.is_some() {
            self.album_art = song.album_art.clone();
        }
        if song.shazam_url.is_some() {
            self.shazam_url = song.shazam_url.clone();
        }
        if song.song_link.is_some() {
            self.song_link = song.song_link.clone();
        }
        let found = links::Resolved {
            providers: song.providers.clone(),
            isrc: song.isrc.clone(),
            isrc_from: song.isrc_from,
            cover: None,
            complete: song.links_complete,
        };
        merge_links(self, &found, now_ms);
    }

    /// A stored row made safe to show: web links only, known platforms only,
    /// one link per service with the track's own first, places newest first
    /// and bounded, and its search words built. A row kept before links
    /// carried their own search mark gets it from the old whole-row flag.
    fn tidy(&mut self) {
        self.album_art = web_url(&self.album_art);
        self.shazam_url = service_url(&self.shazam_url, "Shazam");
        self.song_link = service_url(&self.song_link, "song.link");
        let mut providers = web_providers(&self.providers);
        if self.links_are_searches {
            for p in &mut providers {
                p.search = true;
            }
        }
        self.providers = one_per_service(providers);
        links::sort_providers(&mut self.providers);
        self.links_are_searches = all_searches(&self.providers);
        self.isrc = self.isrc.as_deref().and_then(links::valid_isrc);
        if self.isrc.is_none() {
            self.isrc_from = None;
        } else if self.isrc_from.is_none() {
            self.isrc_from = Some(IsrcFrom::Text);
        }
        for sighting in &mut self.sightings {
            sighting.source = sighting.source.take().and_then(SongSource::cleaned);
        }
        self.sightings.sort_by(|a, b| b.at_ms.cmp(&a.at_ms));
        self.sightings.truncate(MAX_SIGHTINGS);
        self.count = self.count.max(1);
        self.haystack = haystack_of(self);
    }

    fn row(&self, tints: &BTreeMap<String, TintEntry>) -> SongRow {
        let thumb = self.album_art.as_deref().and_then(thumb_of);
        SongRow {
            key: self.key.clone(),
            title: self.title.clone(),
            artist: self.artist.clone(),
            art: thumb.clone().or_else(|| self.album_art.clone()),
            art_full: thumb.and(self.album_art.clone()),
            last_ms: self.last_ms,
            count: self.count,
            heard: self.sightings.iter().find_map(|s| s.source.clone()),
            tint: self.tint(tints),
        }
    }

    /// The cover's colors as the page reads them: none yet while a color may
    /// still come, `main: null` for a grey cover, a cover that cannot be read
    /// or no cover at all.
    fn tint(&self, tints: &BTreeMap<String, TintEntry>) -> Option<TintView> {
        match self.album_art.as_deref() {
            None => Some(TintView::GREY),
            Some(art) => cover_tint::view(tints.get(art)),
        }
    }

    /// The cover when it has no color to show yet and is worth fetching.
    fn untinted_cover(&self, tints: &BTreeMap<String, TintEntry>, now_ms: i64) -> Option<String> {
        let art = self.album_art.as_ref()?;
        (!cover_tint::usable(tints.get(art), now_ms)).then(|| art.clone())
    }

    fn has_direct(&self, service: &str) -> bool {
        self.providers
            .iter()
            .any(|p| p.name == service && !p.search)
    }

    /// The Apple Music id its Apple link names.
    fn apple_id(&self) -> Option<String> {
        self.providers
            .iter()
            .find(|p| p.name == "Apple Music" && !p.search)
            .and_then(|p| links::apple_id_of(&p.url))
    }
}

/// Whether opening `song` should look its links up again: never checked
/// cleanly, or checked over `STALE_LINKS_MS` ago while a service the finder
/// can fill for this song has no link of its own. Beatport and SoundCloud
/// never ask (the finder fills Beatport only by ISRC, SoundCloud never).
fn needs_refresh(song: &SongEntry, read_only: bool, now_ms: i64) -> bool {
    if read_only {
        return false;
    }
    if song.links_checked_ms == 0 {
        return true;
    }
    if now_ms - song.links_checked_ms <= STALE_LINKS_MS {
        return false;
    }
    links::fillable(song.isrc.is_some(), song.apple_id().is_some())
        .iter()
        .any(|service| !song.has_direct(service))
}

/// The one merge of found links into a kept song, for a refresh and for a
/// song identified again: a direct link fills a missing or search slot of
/// its service and never replaces or removes another direct link, and a
/// search page only fills a missing slot. An ISRC from Shazam fills a
/// missing one or replaces one a text search found; when that one differs,
/// the links the finder made from it go and the song is checked again. A
/// text search's ISRC never replaces any. `links_checked_ms` moves forward
/// only on a complete lookup. Returns whether anything changed.
fn merge_links(song: &mut SongEntry, found: &links::Resolved, now_ms: i64) -> bool {
    let before = (
        song.providers.clone(),
        song.isrc.clone(),
        song.isrc_from,
        song.links_checked_ms,
        song.album_art.clone(),
    );
    let mut replaced = false;
    if let Some(code) = found.isrc.as_deref().and_then(links::valid_isrc) {
        match (found.isrc_from, song.isrc.as_deref()) {
            (_, None) => {
                song.isrc = Some(code);
                song.isrc_from = Some(found.isrc_from.unwrap_or(IsrcFrom::Text));
            }
            (Some(IsrcFrom::Shazam), Some(old)) if song.isrc_from != Some(IsrcFrom::Shazam) => {
                if old != code {
                    song.providers
                        .retain(|p| p.search || !links::RESOLVER_MADE.contains(&p.name.as_str()));
                    song.links_checked_ms = 0;
                    replaced = true;
                }
                song.isrc = Some(code);
                song.isrc_from = Some(IsrcFrom::Shazam);
            }
            _ => {}
        }
    }
    for p in web_providers(&found.providers) {
        links::put(&mut song.providers, p);
    }
    links::sort_providers(&mut song.providers);
    song.links_are_searches = all_searches(&song.providers);
    if found.complete && !replaced {
        song.links_checked_ms = song.links_checked_ms.max(now_ms);
    }
    if song.album_art.is_none() {
        song.album_art = web_url(&found.cover);
    }
    before
        != (
            song.providers.clone(),
            song.isrc.clone(),
            song.isrc_from,
            song.links_checked_ms,
            song.album_art.clone(),
        )
}

/// One row of the list.
#[derive(Serialize, Clone, Debug)]
pub struct SongRow {
    pub key: String,
    pub title: String,
    pub artist: String,
    /// The cover, a small copy when its host can draw one (`thumb_of`).
    pub art: Option<String>,
    /// The full cover when `art` is a small copy of it, for a host that
    /// refused the size.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub art_full: Option<String>,
    pub last_ms: i64,
    pub count: u32,
    /// The latest place it was heard that named a channel.
    pub heard: Option<SongSource>,
    /// The cover's colors (`SongEntry::tint`); left out while one may come.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tint: Option<TintView>,
}

/// Where one time group starts: `bound` indexes the page's bounds as sorted
/// here (their count for "Older"), `start` is the first matched song in it.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Group {
    pub bound: usize,
    pub start: usize,
}

/// A page of the list.
#[derive(Serialize, Clone, Debug)]
pub struct SongPage {
    pub rows: Vec<SongRow>,
    /// Songs the query matches (every song with no query).
    pub matched: usize,
    /// Songs held.
    pub total: usize,
    /// Places heard name more than one platform: rows then mark theirs.
    pub mixed_platforms: bool,
    /// Channels the songs held were heard on.
    pub channels: usize,
    /// The time groups of every matched song, newest first; empty when the
    /// page sent no bounds.
    pub groups: Vec<Group>,
}

/// One song as its detail shows it: the kept song and its cover's colors.
#[derive(Serialize, Clone, Debug)]
pub struct SongDetail {
    #[serde(flatten)]
    pub entry: SongEntry,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tint: Option<TintView>,
}

/// A song opened: its detail, and the work opening it asks for.
pub struct Opened {
    pub detail: SongDetail,
    /// Its cover, when it has no color yet.
    pub cover: Option<String>,
    /// Its links should be looked up again (`needs_refresh`).
    pub refresh: bool,
}

/// A match filed: its key, and its cover when it has no color yet.
pub struct Recorded {
    pub key: String,
    pub cover: Option<String>,
}

/// What changed, for a window showing the list or a song.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Change {
    Filed { key: String },
    Removed { key: String },
    Cleared,
    /// These songs' covers got their colors.
    Tinted { keys: Vec<String> },
}

fn first_version() -> u32 {
    1
}

#[derive(Deserialize)]
struct SongFile {
    #[serde(default = "first_version")]
    version: u32,
    #[serde(default)]
    songs: Vec<SongEntry>,
}

/// Only the version, to tell a newer app's file from a damaged one.
#[derive(Deserialize)]
struct FileHead {
    #[serde(default = "first_version")]
    version: u32,
}

/// A newer app's file, read row by row: a row this app cannot read is left
/// out of the list and stays in the file, which is never written.
#[derive(Deserialize)]
struct NewerFile {
    #[serde(default)]
    songs: Vec<serde_json::Value>,
}

#[derive(Serialize)]
struct SongFileOut<'a> {
    version: u32,
    songs: &'a [SongEntry],
}

/// What the file holds: its songs, and whether a newer StreamNook wrote it.
struct Stored {
    songs: Vec<SongEntry>,
    newer: bool,
}

/// The songs, newest heard first.
#[derive(Default)]
struct History {
    songs: Vec<SongEntry>,
    /// The song the last Remove took out, for Undo. Never stored.
    removed: Option<SongEntry>,
    /// The file is a newer app's: shown, never written over.
    read_only: bool,
}

impl History {
    fn from_stored(stored: Stored) -> Self {
        let mut songs = stored.songs;
        songs.sort_by(|a, b| b.last_ms.cmp(&a.last_ms));
        let mut seen = HashSet::new();
        songs.retain(|s| {
            !s.key.is_empty() && !s.title.trim().is_empty() && seen.insert(s.key.clone())
        });
        songs.truncate(MAX_SONGS);
        for song in &mut songs {
            song.tidy();
        }
        History {
            songs,
            removed: None,
            read_only: stored.newer,
        }
    }

    /// File one match heard at `at_ms`. Returns its key.
    fn file(&mut self, song: &SongMatch, source: Option<SongSource>, at_ms: i64) -> String {
        let song = kept_match(song);
        let key = song_key(&song.artist, &song.title);
        let source = source.and_then(SongSource::cleaned);
        let sighting = Sighting {
            at_ms,
            source: source.clone(),
        };
        if let Some(i) = self.songs.iter().position(|s| s.key == key) {
            let mut entry = self.songs.remove(i);
            entry.refresh_from(&song, at_ms);
            let repeat = entry.sightings.first().is_some_and(|last| {
                at_ms - last.at_ms < REPEAT_WINDOW_MS
                    && match (&last.source, &source) {
                        (Some(a), Some(b)) => a.same_channel(b),
                        (None, None) => true,
                        _ => false,
                    }
            });
            if repeat {
                entry.sightings[0] = sighting;
            } else {
                entry.count = entry.count.saturating_add(1);
                entry.sightings.insert(0, sighting);
                entry.sightings.truncate(MAX_SIGHTINGS);
            }
            entry.last_ms = entry.last_ms.max(at_ms);
            entry.haystack = haystack_of(&entry);
            self.songs.insert(0, entry);
        } else {
            let mut entry = SongEntry {
                key: key.clone(),
                title: song.title.clone(),
                artist: song.artist.clone(),
                album: song.album.clone(),
                album_art: song.album_art.clone(),
                shazam_url: song.shazam_url.clone(),
                song_link: song.song_link.clone(),
                providers: song.providers.clone(),
                links_are_searches: song.links_are_searches,
                isrc: song.isrc.clone(),
                isrc_from: song.isrc_from,
                links_checked_ms: if song.links_complete { at_ms } else { 0 },
                first_ms: at_ms,
                last_ms: at_ms,
                count: 1,
                sightings: vec![sighting],
                haystack: String::new(),
            };
            entry.haystack = haystack_of(&entry);
            self.songs.insert(0, entry);
            self.songs.truncate(MAX_SONGS);
        }
        key
    }

    #[cfg(test)]
    fn page(&self, query: &str, offset: usize, limit: usize) -> SongPage {
        self.page_in(query, offset, limit, &[], &BTreeMap::new(), 0).0
    }

    /// One page of the songs matching `query`, with the time groups of every
    /// matched song for `bounds` (`Groups`), and the covers of its rows that
    /// have no color yet (at most `cover_tint::MAX_PER_PAGE`).
    fn page_in(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
        bounds: &[i64],
        tints: &BTreeMap<String, TintEntry>,
        now_ms: i64,
    ) -> (SongPage, Vec<String>) {
        let terms = terms_of(query);
        let limit = limit.clamp(1, MAX_PAGE);
        let bounds = sorted_bounds(bounds);
        let mut groups = Groups::default();
        let mut matched = 0;
        let mut rows = Vec::new();
        let mut covers: Vec<String> = Vec::new();
        for song in &self.songs {
            if !terms.iter().all(|t| song.haystack.contains(t.as_str())) {
                continue;
            }
            groups.add(&bounds, song.last_ms, matched);
            if matched >= offset && rows.len() < limit {
                rows.push(song.row(tints));
                if let Some(cover) = song.untinted_cover(tints, now_ms) {
                    if covers.len() < cover_tint::MAX_PER_PAGE && !covers.contains(&cover) {
                        covers.push(cover);
                    }
                }
            }
            matched += 1;
        }
        let (mixed_platforms, channels) = self.places();
        let page = SongPage {
            rows,
            matched,
            total: self.songs.len(),
            mixed_platforms,
            channels,
            groups: groups.out,
        };
        (page, covers)
    }

    /// Whether the places heard name more than one platform, and how many
    /// channels they name (a login in any case is one channel).
    fn places(&self) -> (bool, usize) {
        let mut first: Option<&str> = None;
        let mut mixed = false;
        let mut channels: HashSet<(&str, String)> = HashSet::new();
        for source in self
            .songs
            .iter()
            .flat_map(|s| &s.sightings)
            .filter_map(|s| s.source.as_ref())
        {
            match first {
                None => first = Some(source.provider.as_str()),
                Some(p) if p != source.provider => mixed = true,
                _ => {}
            }
            channels.insert((
                source.provider.as_str(),
                source.channel_login.to_ascii_lowercase(),
            ));
        }
        (mixed, channels.len())
    }

    /// Take one song out, kept aside for Undo. False when it was not there.
    fn remove(&mut self, key: &str) -> bool {
        let Some(at) = self.songs.iter().position(|s| s.key == key) else {
            return false;
        };
        self.removed = Some(self.songs.remove(at));
        true
    }

    /// Put back the song the last Remove took out, when `key` names it.
    fn restore(&mut self, key: &str) -> bool {
        match self.removed.take() {
            Some(entry) if entry.key == key => {
                self.put_back(entry);
                true
            }
            other => {
                self.removed = other;
                false
            }
        }
    }

    fn put_back(&mut self, entry: SongEntry) {
        if let Some(now) = self.songs.iter_mut().find(|s| s.key == entry.key) {
            // Heard again after it was removed: the one row keeps both.
            now.first_ms = now.first_ms.min(entry.first_ms);
            now.count = now.count.saturating_add(entry.count);
            now.sightings.extend(entry.sightings);
            now.sightings.sort_by(|a, b| b.at_ms.cmp(&a.at_ms));
            now.sightings.truncate(MAX_SIGHTINGS);
            now.haystack = haystack_of(now);
            return;
        }
        // Back where it sat in the newest-first order.
        let at = self
            .songs
            .iter()
            .position(|s| s.last_ms < entry.last_ms)
            .unwrap_or(self.songs.len());
        self.songs.insert(at, entry);
        self.songs.truncate(MAX_SONGS);
    }
}

/// The page's time bounds as groups use them: newest first, each once, at
/// most `MAX_BOUNDS`.
fn sorted_bounds(raw: &[i64]) -> Vec<i64> {
    let mut bounds = raw.to_vec();
    bounds.sort_unstable_by(|a, b| b.cmp(a));
    bounds.dedup();
    bounds.truncate(MAX_BOUNDS);
    bounds
}

/// Time groups built while walking the matched songs newest first. A song's
/// own bucket is the first bound it was heard at or after (the bounds' count
/// for "Older"); its group is never earlier than the one before it, so a
/// song filed under a clock that jumped back stays in the group above it.
#[derive(Default)]
struct Groups {
    out: Vec<Group>,
}

impl Groups {
    fn add(&mut self, bounds: &[i64], last_ms: i64, index: usize) {
        if bounds.is_empty() {
            return;
        }
        let own = bounds
            .iter()
            .position(|b| last_ms >= *b)
            .unwrap_or(bounds.len());
        let bucket = match self.out.last() {
            Some(last) => last.bound.max(own),
            None => own,
        };
        if self.out.last().is_none_or(|last| last.bound != bucket) {
            self.out.push(Group {
                bound: bucket,
                start: index,
            });
        }
    }
}

/// The song's identity in the history: artist and title folded (`fold`), so
/// the same song matched twice is one row.
pub fn song_key(artist: &str, title: &str) -> String {
    format!("{}\u{1f}{}", fold(artist), fold(title))
}

/// Text as the history compares it: lowercase with Latin accents folded
/// (`fold_search`), apostrophes dropped so "Don't", "Don’t" and "dont" are one
/// word, and every run of spacing made one space. The link finder matches
/// titles from this too (links.rs).
pub(super) fn fold(s: &str) -> String {
    let folded: String = fold_search(s)
        .chars()
        .filter(|c| !is_apostrophe(*c))
        .collect();
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_apostrophe(c: char) -> bool {
    matches!(
        c,
        '\'' | '\u{2018}' | '\u{2019}' | '\u{02BC}' | '`' | '\u{00B4}'
    )
}

/// A small copy of a cover for the list. Apple's image host, where Shazam's
/// covers live, draws any size named in the file name, so a 400x400 cover is
/// asked for at 96x96: a 40 px row decodes about 37 KB instead of 640 KB.
/// None for any other host or file name.
pub fn thumb_of(url: &str) -> Option<String> {
    if !url.contains(".mzstatic.com/image/thumb/") {
        return None;
    }
    let (head, file) = url.rsplit_once('/')?;
    let (size, ext) = file.split_once('.')?;
    let dims = size.trim_end_matches(|c: char| c.is_ascii_lowercase());
    let suffix = &size[dims.len()..];
    let (w, h) = dims.split_once('x')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(w)
        || !digits(h)
        || !matches!(suffix, "" | "bb" | "cc")
        || !matches!(ext, "jpg" | "jpeg" | "png" | "webp")
    {
        return None;
    }
    Some(format!("{head}/96x96{suffix}.{ext}"))
}

/// A small copy of a channel's picture. Twitch's image host keeps each
/// picture at fixed sizes named in the file name, and Your songs draws one at
/// 40 px at most, so a 300x300 picture is kept as its 70x70 copy. None for
/// any other host or file name.
fn small_avatar(url: &str) -> Option<String> {
    const MARK: &str = "-profile_image-";
    if !url.starts_with("https://static-cdn.jtvnw.net/jtv_user_pictures/") {
        return None;
    }
    let (head, file) = url.rsplit_once(MARK)?;
    let (size, ext) = file.split_once('.')?;
    let (w, h) = size.split_once('x')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(w) || !digits(h) || !matches!(ext, "png" | "jpg" | "jpeg" | "webp") {
        return None;
    }
    Some(format!("{head}{MARK}70x70.{ext}"))
}

fn haystack_of(song: &SongEntry) -> String {
    let mut out = format!("{} {}", song.title, song.artist);
    if let Some(album) = &song.album {
        out.push(' ');
        out.push_str(album);
    }
    for source in song.sightings.iter().filter_map(|s| s.source.as_ref()) {
        out.push(' ');
        out.push_str(&source.channel_login);
        for word in [&source.channel_name, &source.category]
            .into_iter()
            .flatten()
        {
            out.push(' ');
            out.push_str(word);
        }
    }
    fold(&out)
}

fn terms_of(query: &str) -> Vec<String> {
    let query: String = query.chars().take(MAX_QUERY_CHARS).collect();
    let mut terms: Vec<String> = Vec::new();
    for word in fold(&query)
        .split(' ')
        .filter(|w| !w.is_empty())
        .take(MAX_TERMS)
    {
        let word: String = word.chars().take(MAX_TERM_CHARS).collect();
        if !terms.contains(&word) {
            terms.push(word);
        }
    }
    terms
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

struct Loaded {
    history: History,
    /// The cover colors, kept in their own file beside the songs.
    tints: Tints,
    path: PathBuf,
    used: Instant,
}

static STORE: Mutex<Option<Loaded>> = Mutex::new(None);
/// A change is in memory that the last write did not get to disk (a failed
/// write, or links a refresh found waiting for their deferred write).
static DIRTY: AtomicBool = AtomicBool::new(false);
/// The unloader task is running.
static UNLOADER: AtomicBool = AtomicBool::new(false);
/// A deferred write is waiting to run.
static DEFERRED: AtomicBool = AtomicBool::new(false);

/// The store, also after a call that panicked while holding it: one bad call
/// never closes Your songs until a restart.
fn lock_store() -> MutexGuard<'static, Option<Loaded>> {
    STORE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Temp files a write left behind when the app died between writing one and
/// renaming it (`write_atomic_with` names them `song_history.json.<id>.tmp`).
fn sweep_temps(path: &Path) {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str())) else {
        return;
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let prefix = format!("{name}.");
    for entry in entries.flatten() {
        let file = entry.file_name();
        let file = file.to_string_lossy();
        if file.starts_with(&prefix) && file.ends_with(".tmp") {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// The stored history. A missing file is an empty one. One that does not
/// parse (damaged bytes, a hand edit) is moved aside, kept for recovery, and
/// the history starts empty; a newer app's file is read as far as it can be
/// and left as it is. One that is there but cannot be read right now is an
/// error, so nothing ever writes an empty list over someone's songs.
fn read_file(path: &Path) -> Result<Stored, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Stored {
                songs: Vec::new(),
                newer: false,
            })
        }
        Err(e) => return Err(format!("Failed to read song history: {e}")),
    };
    match serde_json::from_slice::<SongFile>(&bytes) {
        Ok(file) => Ok(Stored {
            newer: file.version > FILE_VERSION,
            songs: file.songs,
        }),
        Err(e) => {
            if serde_json::from_slice::<FileHead>(&bytes)
                .is_ok_and(|head| head.version > FILE_VERSION)
            {
                let rows = serde_json::from_slice::<NewerFile>(&bytes)
                    .map(|f| f.songs)
                    .unwrap_or_default();
                let songs: Vec<SongEntry> = rows
                    .into_iter()
                    .filter_map(|row| serde_json::from_value(row).ok())
                    .collect();
                return Ok(Stored { songs, newer: true });
            }
            let aside = path.with_extension(format!("json.bad-{}", now_ms()));
            fs::rename(path, &aside).map_err(|re| {
                format!("Song history does not parse ({e}) and could not be moved aside: {re}")
            })?;
            log::warn!(
                "[SongHistory] {} did not parse ({e}); kept as {}",
                path.display(),
                aside.display()
            );
            Ok(Stored {
                songs: Vec::new(),
                newer: false,
            })
        }
    }
}

fn write_file(path: &Path, history: &History) -> Result<(), String> {
    let out = SongFileOut {
        version: FILE_VERSION,
        songs: &history.songs,
    };
    let json =
        serde_json::to_vec(&out).map_err(|e| format!("Failed to serialize song history: {e}"))?;
    // The cache's writer: on disk before it replaces the file, retried while
    // Windows holds the file (antivirus, the indexer), temp removed on failure.
    write_atomic_with(path, json, |from, to| fs::rename(from, to))
        .map_err(|e| format!("Failed to write song history: {e}"))?;
    sync_dir_of(path);
    Ok(())
}

/// The rename reaches the disk only with its folder (Windows needs no step).
#[cfg(not(windows))]
fn sync_dir_of(path: &Path) {
    if let Some(dir) = path.parent() {
        let _ = fs::File::open(dir).and_then(|d| d.sync_all());
    }
}

#[cfg(windows)]
fn sync_dir_of(_path: &Path) {}

/// Write the history, remembering a failed write so the unloader and the exit
/// path try again.
fn persist(history: &History, path: &Path) -> Result<(), String> {
    let result = write_file(path, history);
    DIRTY.store(result.is_err(), Ordering::Release);
    result
}

/// Write the cover colors: only those of covers a song or the Undo slot
/// still uses, and never while tints are off for this load.
fn save_tints(tints: &mut Tints, history: &History) -> Result<(), String> {
    prune_tints(&mut tints.map, history);
    cover_tint::write_tints_file(&tints.path, &tints.map, cover_tint::is_off())
}

/// Drop the colors of covers no song uses (the Undo slot counts as a song).
fn prune_tints(map: &mut BTreeMap<String, TintEntry>, history: &History) {
    let used: HashSet<&str> = history
        .songs
        .iter()
        .chain(history.removed.iter())
        .filter_map(|s| s.album_art.as_deref())
        .collect();
    map.retain(|url, _| used.contains(url.as_str()));
}

/// Read the songs at `path` and the cover colors beside them. The history's
/// own errors fail the load; the colors never do. Also returns whether
/// tints are off for this load (their file is a newer app's, or could not be
/// read or parsed).
fn load(path: PathBuf) -> Result<(Loaded, bool), String> {
    sweep_temps(&path);
    let history = History::from_stored(read_file(&path)?);
    if history.read_only {
        log::warn!(
            "[SongHistory] {} was saved by a newer StreamNook; shown, not changed",
            path.display()
        );
    }
    let tint_path = path.with_file_name(cover_tint::FILE);
    sweep_temps(&tint_path);
    let (map, off) = cover_tint::read_tints(&tint_path);
    let loaded = Loaded {
        history,
        tints: Tints {
            map,
            path: tint_path,
        },
        path,
        used: Instant::now(),
    };
    Ok((loaded, off))
}

fn with_store<R>(f: impl FnOnce(&mut History, &mut Tints, &Path) -> R) -> Result<R, String> {
    let out = {
        let mut guard = lock_store();
        if guard.is_none() {
            let path = get_app_data_dir()
                .map_err(|e| format!("Failed to resolve app data dir: {e}"))?
                .join(FILE);
            let (loaded, off) = load(path)?;
            cover_tint::set_off(off);
            *guard = Some(loaded);
        }
        let loaded = guard.as_mut().expect("seeded above");
        loaded.used = Instant::now();
        f(&mut loaded.history, &mut loaded.tints, &loaded.path)
    };
    schedule_unload();
    Ok(out)
}

fn with_history<R>(f: impl FnOnce(&mut History, &Path) -> R) -> Result<R, String> {
    with_store(|history, _, path| f(history, path))
}

/// One change to `h`, written to `path` when it changed anything. Refused
/// while the file is a newer app's. The write's own result comes back apart,
/// since a change kept in memory still stands.
fn apply<R>(
    h: &mut History,
    path: &Path,
    f: impl FnOnce(&mut History) -> (R, bool),
) -> Result<(R, Result<(), String>), String> {
    if h.read_only {
        return Err(NEWER_FILE.to_string());
    }
    let (out, changed) = f(h);
    let saved = if changed { persist(h, path) } else { Ok(()) };
    Ok((out, saved))
}

fn change<R>(f: impl FnOnce(&mut History) -> (R, bool)) -> Result<R, String> {
    let (out, saved) = with_history(|h, path| apply(h, path, f))??;
    if let Err(e) = saved {
        log::warn!("[SongHistory] kept in memory, not on disk yet: {e}");
    }
    Ok(out)
}

/// File a match heard now on `source`. Returns the song's key, and its cover
/// when that has no color yet.
pub fn record(
    app: &AppHandle,
    song: &SongMatch,
    source: Option<SongSource>,
) -> Result<Recorded, String> {
    let at_ms = now_ms();
    let (key, saved, cover) = with_store(|h, t, path| {
        let (key, saved) = apply(h, path, |h| (h.file(song, source, at_ms), true))?;
        let cover = h
            .songs
            .iter()
            .find(|s| s.key == key)
            .and_then(|s| s.untinted_cover(&t.map, at_ms));
        Ok::<_, String>((key, saved, cover))
    })??;
    if let Err(e) = saved {
        log::warn!("[SongHistory] kept in memory, not on disk yet: {e}");
    }
    let _ = app.emit(CHANGED_EVENT, Change::Filed { key: key.clone() });
    Ok(Recorded { key, cover })
}

/// The rows matching every word of `query`, from `offset`, with the time
/// groups for the page's `bounds`, and the covers of those rows that have no
/// color yet.
pub fn page(
    query: &str,
    offset: usize,
    limit: usize,
    bounds: Option<Vec<i64>>,
) -> Result<(SongPage, Vec<String>), String> {
    let now = now_ms();
    with_store(|h, t, _| {
        h.page_in(
            query,
            offset,
            limit,
            bounds.as_deref().unwrap_or(&[]),
            &t.map,
            now,
        )
    })
}

/// One song, by key, with what opening it asks for: its cover's colors and
/// a link refresh (`needs_refresh`).
pub fn get(key: &str) -> Result<Option<Opened>, String> {
    let now = now_ms();
    with_store(|h, t, _| {
        let song = h.songs.iter().find(|s| s.key == key)?;
        Some(Opened {
            cover: song.untinted_cover(&t.map, now),
            refresh: needs_refresh(song, h.read_only, now),
            detail: SongDetail {
                tint: song.tint(&t.map),
                entry: song.clone(),
            },
        })
    })
}

/// Take one song out, kept for an Undo. False when it was not there.
pub fn remove(app: &AppHandle, key: &str) -> Result<bool, String> {
    let removed = change(|h| {
        let removed = h.remove(key);
        (removed, removed)
    })?;
    if removed {
        let _ = app.emit(
            CHANGED_EVENT,
            Change::Removed {
                key: key.to_string(),
            },
        );
    }
    Ok(removed)
}

/// Undo the last Remove when `key` names its song. False when there is
/// nothing to put back (another song was removed since, or the songs were let
/// go of).
pub fn restore(app: &AppHandle, key: &str) -> Result<bool, String> {
    let restored = change(|h| {
        let restored = h.restore(key);
        (restored, restored)
    })?;
    if restored {
        let _ = app.emit(
            CHANGED_EVENT,
            Change::Filed {
                key: key.to_string(),
            },
        );
    }
    Ok(restored)
}

/// Forget every song, and the colors of their covers with them.
pub fn clear(app: &AppHandle) -> Result<(), String> {
    let saved = with_store(|h, t, path| {
        let ((), saved) = apply(h, path, |h| {
            h.songs.clear();
            h.removed = None;
            ((), true)
        })?;
        if !cover_tint::is_off() {
            t.map.clear();
            let written = save_tints(t, h);
            cover_tint::TINT_DIRTY.store(written.is_err(), Ordering::Release);
        }
        Ok::<_, String>(saved)
    })??;
    if let Err(e) = saved {
        log::warn!("[SongHistory] kept in memory, not on disk yet: {e}");
    }
    let _ = app.emit(CHANGED_EVENT, Change::Cleared);
    Ok(())
}

/// Tell every window these songs' covers got their colors.
pub(super) fn tinted(app: &AppHandle, keys: Vec<String>) {
    let _ = app.emit(CHANGED_EVENT, Change::Tinted { keys });
}

/// Put fetched cover colors into the live map and write the tint file once
/// (cover_tint.rs calls it per batch, off the async runtime). Returns the
/// keys of the songs whose covers got an entry.
pub(super) fn add_tints(results: Vec<(String, TintEntry)>) -> Result<Vec<String>, String> {
    with_store(|h, t, _| {
        land_tints(
            h,
            t,
            results,
            cover_tint::is_off(),
            &cover_tint::TINT_DIRTY,
            save_tints,
        )
    })
}

/// `add_tints` on a given store, with the flag and writer passed in so a
/// test can fail the write. Nothing is written while tints are off.
fn land_tints(
    history: &History,
    tints: &mut Tints,
    results: Vec<(String, TintEntry)>,
    off: bool,
    tint_dirty: &AtomicBool,
    write_tints: impl Fn(&mut Tints, &History) -> Result<(), String>,
) -> Vec<String> {
    let urls: HashSet<String> = results.iter().map(|(url, _)| url.clone()).collect();
    tints.map.extend(results);
    let keys = history
        .songs
        .iter()
        .filter(|s| s.album_art.as_ref().is_some_and(|art| urls.contains(art)))
        .map(|s| s.key.clone())
        .collect();
    if !off {
        match write_tints(tints, history) {
            Ok(()) => tint_dirty.store(false, Ordering::Release),
            Err(e) => {
                tint_dirty.store(true, Ordering::Release);
                log::warn!("[SongTints] kept in memory, not on disk yet: {e}");
            }
        }
    }
    keys
}

/// Write what an earlier write did not get to disk, and links a refresh
/// found. The exit path and the deferred write call it; a no-op when nothing
/// is pending or the songs are not loaded (it never loads them).
pub fn flush_now() -> Result<(), String> {
    if !DIRTY.load(Ordering::Acquire) && !cover_tint::TINT_DIRTY.load(Ordering::Acquire) {
        return Ok(());
    }
    let mut guard = lock_store();
    flush_slot(
        &mut guard,
        &DIRTY,
        &cover_tint::TINT_DIRTY,
        write_file,
        save_tints,
    )
}

/// `flush_now` on a given slot, flags and writers. Each file is written only
/// while its own flag is set, and a newer app's history never.
fn flush_slot(
    slot: &mut Option<Loaded>,
    dirty: &AtomicBool,
    tint_dirty: &AtomicBool,
    write: impl Fn(&Path, &History) -> Result<(), String>,
    write_tints: impl Fn(&mut Tints, &History) -> Result<(), String>,
) -> Result<(), String> {
    let Some(loaded) = slot.as_mut() else {
        return Ok(());
    };
    let mut out = Ok(());
    if tint_dirty.load(Ordering::Acquire) {
        match write_tints(&mut loaded.tints, &loaded.history) {
            Ok(()) => tint_dirty.store(false, Ordering::Release),
            Err(e) => out = Err(e),
        }
    }
    if dirty.load(Ordering::Acquire) && !loaded.history.read_only {
        match write(&loaded.path, &loaded.history) {
            Ok(()) => dirty.store(false, Ordering::Release),
            Err(e) => out = Err(e),
        }
    }
    out
}

/// One unloader pass over `slot`: let the songs go once they sat unused for
/// `idle_after` with nothing of theirs left to write. `write` and
/// `write_tints` are the disk writes (`write_file`, `save_tints`), passed in
/// so a test can fail them. The colors get one more try here and never keep
/// the songs: only the history's own `dirty` does. A newer app's history is
/// never written. Returns how long to wait before the next pass, or None
/// once nothing is held.
fn unload_pass(
    slot: &mut Option<Loaded>,
    idle_after: Duration,
    dirty: &AtomicBool,
    tint_dirty: &AtomicBool,
    write: impl Fn(&Path, &History) -> Result<(), String>,
    write_tints: impl Fn(&mut Tints, &History) -> Result<(), String>,
) -> Option<Duration> {
    let idle = slot.as_ref()?.used.elapsed();
    if idle < idle_after {
        return Some(idle_after - idle);
    }
    if let Some(loaded) = slot.as_mut() {
        if tint_dirty.load(Ordering::Acquire)
            && write_tints(&mut loaded.tints, &loaded.history).is_ok()
        {
            tint_dirty.store(false, Ordering::Release);
        }
        if dirty.load(Ordering::Acquire) {
            // Nothing this app holds may go over a newer app's file.
            if loaded.history.read_only || write(&loaded.path, &loaded.history).is_ok() {
                dirty.store(false, Ordering::Release);
            }
        }
    }
    if dirty.load(Ordering::Acquire) {
        // Keep the songs until a write lands; try again in a while.
        return Some(IDLE_UNLOAD);
    }
    tint_dirty.store(false, Ordering::Release);
    *slot = None;
    None
}

fn unload_step() -> Option<Duration> {
    let mut guard = lock_store();
    unload_pass(
        &mut guard,
        IDLE_UNLOAD,
        &DIRTY,
        &cover_tint::TINT_DIRTY,
        write_file,
        save_tints,
    )
}

fn schedule_unload() {
    if UNLOADER
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    tauri::async_runtime::spawn(async {
        while let Ok(Some(wait)) = tokio::task::spawn_blocking(unload_step).await {
            tokio::time::sleep(wait).await;
        }
        UNLOADER.store(false, Ordering::Release);
        // A use that landed while this task was ending found it still running
        // and started nothing; pick it up here.
        if lock_store().is_some() {
            schedule_unload();
        }
    });
}

/// Songs waiting for their links to be looked up again, oldest first, and
/// the one being looked up.
#[derive(Default)]
struct Refreshes {
    queued: VecDeque<String>,
    running: Option<String>,
}

impl Refreshes {
    /// Queue `key`. False when it is already queued or running, or the queue
    /// is full.
    fn push(&mut self, key: String) -> bool {
        if self.running.as_ref() == Some(&key)
            || self.queued.contains(&key)
            || self.queued.len() >= REFRESH_CAP
        {
            return false;
        }
        self.queued.push_back(key);
        true
    }

    /// The next key to look up, now the running one.
    fn next(&mut self) -> Option<String> {
        self.running = self.queued.pop_front();
        self.running.clone()
    }
}

static REFRESHES: Mutex<Refreshes> = Mutex::new(Refreshes {
    queued: VecDeque::new(),
    running: None,
});
/// The refresh runner task is running.
static REFRESHER: AtomicBool = AtomicBool::new(false);

fn refreshes() -> MutexGuard<'static, Refreshes> {
    REFRESHES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Look the links of the song `key` up again in the background, one song at
/// a time. Opening a song (`get` says when) and an identification whose
/// lookups were cut short ask for it.
pub fn queue_refresh(app: &AppHandle, key: String) {
    if refreshes().push(key) {
        start_refresher(app);
    }
}

fn start_refresher(app: &AppHandle) {
    if refreshes().queued.is_empty() {
        return;
    }
    if REFRESHER
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let next = refreshes().next();
            let Some(key) = next else {
                break;
            };
            refresh_one(&app, key).await;
        }
        REFRESHER.store(false, Ordering::Release);
        // A key queued while this task was ending found it still running.
        start_refresher(&app);
    });
}

async fn refresh_one(app: &AppHandle, key: String) {
    let asked = key.clone();
    let input = tokio::task::spawn_blocking(move || with_history(|h, _| refresh_input(h, &asked)))
        .await;
    let Ok(Ok(Some(input))) = input else {
        return;
    };
    let found = links::resolve(&input, REFRESH_BUDGET).await;
    let at = key.clone();
    let changed = tokio::task::spawn_blocking(move || {
        with_history(|h, _| apply_refresh(h, &at, &found, now_ms(), &DIRTY))
    })
    .await;
    if matches!(changed, Ok(Ok(true))) {
        arm_deferred_write(&DEFERRED, DEFERRED_WRITE);
        let _ = app.emit(CHANGED_EVENT, Change::Filed { key });
    }
}

/// What a refresh of `key` looks up: the song as it is now. None when it is
/// gone or the file is a newer app's.
fn refresh_input(h: &History, key: &str) -> Option<links::ResolveInput> {
    if h.read_only {
        return None;
    }
    let song = h.songs.iter().find(|s| s.key == key)?;
    Some(links::ResolveInput {
        title: song.title.clone(),
        artist: song.artist.clone(),
        album: song.album.clone(),
        isrc: song.isrc.clone(),
        isrc_from: song.isrc_from,
        apple_id: song.apple_id(),
        known: song.providers.clone(),
    })
}

/// Merge a refresh's links into the song `key` as it is now (`merge_links`).
/// Nothing when it is gone or the file is a newer app's. A change is held as
/// pending (`dirty`) for the deferred write. Returns whether it changed.
fn apply_refresh(
    h: &mut History,
    key: &str,
    found: &links::Resolved,
    now_ms: i64,
    dirty: &AtomicBool,
) -> bool {
    if h.read_only {
        return false;
    }
    let Some(song) = h.songs.iter_mut().find(|s| s.key == key) else {
        return false;
    };
    let changed = merge_links(song, found, now_ms);
    if changed {
        dirty.store(true, Ordering::Release);
    }
    changed
}

/// Write pending links once, `after` from now: one timer at a time, and a
/// change that lands while one waits rides on it. False when one was
/// already waiting.
fn arm_deferred_write(pending: &'static AtomicBool, after: Duration) -> bool {
    if !claim_deferred(pending) {
        return false;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(after).await;
        pending.store(false, Ordering::Release);
        if let Ok(Err(e)) = tokio::task::spawn_blocking(flush_now).await {
            log::warn!("[SongHistory] links kept in memory, not on disk yet: {e}");
        }
    });
    true
}

/// Take the one deferred-write slot. False while a write is waiting.
fn claim_deferred(pending: &AtomicBool) -> bool {
    pending
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(artist: &str, title: &str) -> SongMatch {
        SongMatch {
            title: title.to_string(),
            artist: artist.to_string(),
            album: None,
            album_art: None,
            shazam_url: None,
            song_link: None,
            providers: Vec::new(),
            key: None,
            links_are_searches: false,
            isrc: None,
            isrc_from: None,
            apple_id: None,
            links_complete: false,
        }
    }

    fn direct(name: &str, url: &str) -> Provider {
        Provider {
            name: name.into(),
            url: url.into(),
            search: false,
        }
    }

    fn searching(name: &str) -> Provider {
        links::search_link(name, "A", "One").unwrap()
    }

    fn on(login: &str) -> Option<SongSource> {
        Some(SongSource {
            provider: "twitch".into(),
            channel_login: login.into(),
            media: Some("live".into()),
            ..Default::default()
        })
    }

    fn entry(title: &str, last_ms: i64) -> SongEntry {
        let mut e = SongEntry {
            key: song_key("A", title),
            title: title.into(),
            artist: "A".into(),
            first_ms: last_ms,
            last_ms,
            count: 1,
            ..Default::default()
        };
        e.haystack = haystack_of(&e);
        e
    }

    fn history(songs: Vec<SongEntry>) -> History {
        History {
            songs,
            ..Default::default()
        }
    }

    fn stored(songs: Vec<SongEntry>) -> Stored {
        Stored {
            songs,
            newer: false,
        }
    }

    fn titles(h: &History) -> Vec<String> {
        h.songs.iter().map(|s| s.title.clone()).collect()
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sn-song-history-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_song_heard_again_is_one_row_moved_to_the_top() {
        let mut h = History::default();
        h.file(&song("Daft Punk", "Veridis Quo"), on("a"), 1_000);
        h.file(&song("Sleep Token", "Telomeres"), on("a"), 2_000);
        let key = h.file(&song("daft  punk", "VERIDIS QUO"), on("b"), 3_000);
        assert_eq!(key, song_key("Daft Punk", "Veridis Quo"));
        assert_eq!(titles(&h), vec!["VERIDIS QUO", "Telomeres"]);
        let top = &h.songs[0];
        assert_eq!((top.first_ms, top.last_ms, top.count), (1_000, 3_000, 2));
        assert_eq!(top.sightings.len(), 2);
        assert_eq!(top.sightings[0].source.as_ref().unwrap().channel_login, "b");
    }

    #[test]
    fn accents_and_apostrophes_do_not_split_a_song() {
        assert_eq!(
            song_key("Beyoncé", "Don’t Hurt Yourself"),
            song_key("beyonce", "Dont Hurt Yourself")
        );
        assert_eq!(song_key("A", "Don't"), song_key("A", "Don’t"));
        assert_ne!(song_key("A", "One"), song_key("A", "Two"));
    }

    #[test]
    fn a_repeat_on_the_same_channel_soon_after_counts_once() {
        let mut h = History::default();
        h.file(&song("A", "One"), on("x"), 0);
        h.file(&song("A", "One"), on("X"), REPEAT_WINDOW_MS - 1);
        assert_eq!((h.songs[0].count, h.songs[0].sightings.len()), (1, 1));
        assert_eq!(h.songs[0].sightings[0].at_ms, REPEAT_WINDOW_MS - 1);
        h.file(&song("A", "One"), on("x"), 2 * REPEAT_WINDOW_MS);
        assert_eq!(h.songs[0].count, 2);
        h.file(&song("A", "One"), on("y"), 2 * REPEAT_WINDOW_MS + 1);
        assert_eq!(h.songs[0].count, 3);
    }

    #[test]
    fn places_are_capped_newest_first() {
        let mut h = History::default();
        for i in 0..(MAX_SIGHTINGS as i64 + 5) {
            h.file(&song("A", "One"), on(&format!("c{i}")), i);
        }
        let s = &h.songs[0];
        assert_eq!(s.sightings.len(), MAX_SIGHTINGS);
        assert_eq!(s.count, MAX_SIGHTINGS as u32 + 5);
        assert_eq!(
            s.sightings[0].source.as_ref().unwrap().channel_login,
            format!("c{}", MAX_SIGHTINGS + 4)
        );
    }

    #[test]
    fn a_later_match_keeps_what_it_left_out() {
        let mut h = History::default();
        let mut first = song("A", "One");
        first.album = Some("Album".into());
        first.album_art = Some("https://art/1.jpg".into());
        first.providers = vec![direct("Spotify", "https://open.spotify.com/track/1")];
        h.file(&first, None, 0);
        h.file(&song("A", "One"), None, 2 * REPEAT_WINDOW_MS);
        let s = &h.songs[0];
        assert_eq!(s.album.as_deref(), Some("Album"));
        assert_eq!(s.album_art.as_deref(), Some("https://art/1.jpg"));
        assert_eq!(s.providers.len(), 1);
        assert_eq!(s.count, 2);
    }

    #[test]
    fn search_pages_never_replace_real_links() {
        let mut real = song("A", "One");
        real.providers = vec![direct("Spotify", "https://open.spotify.com/track/1")];
        let mut search = song("A", "One");
        search.providers = vec![direct(
            "Spotify",
            "https://open.spotify.com/search/A%20One",
        )];
        search.links_are_searches = true;

        let mut h = History::default();
        h.file(&real, None, 0);
        h.file(&search, None, 2 * REPEAT_WINDOW_MS);
        assert_eq!(
            h.songs[0].providers[0].url,
            "https://open.spotify.com/track/1"
        );

        let mut h = History::default();
        h.file(&search, None, 0);
        assert!(h.songs[0].links_are_searches);
        h.file(&real, None, 2 * REPEAT_WINDOW_MS);
        assert_eq!(
            h.songs[0].providers[0].url,
            "https://open.spotify.com/track/1"
        );
        assert!(!h.songs[0].links_are_searches);
    }

    #[test]
    fn only_web_links_and_bounded_text_are_kept() {
        let mut s = song("A", &"t".repeat(MAX_TEXT + 50));
        s.album_art = Some("file:///C:/Windows/x.png".into());
        s.song_link = Some(format!("https://song.link/{}", "a".repeat(MAX_URL)));
        s.shazam_url = Some("https://www.shazam.com/track/1".into());
        s.providers = vec![
            direct("Spotify", "https://open.spotify.com/track/1"),
            direct("Evil", "javascript:alert(1)"),
            direct(" ", "https://example.com"),
        ];
        let mut h = History::default();
        h.file(&s, None, 0);
        let kept = &h.songs[0];
        assert_eq!(kept.title.chars().count(), MAX_TEXT);
        assert_eq!(kept.album_art, None);
        assert_eq!(kept.song_link, None);
        assert_eq!(
            kept.shazam_url.as_deref(),
            Some("https://www.shazam.com/track/1")
        );
        assert_eq!(kept.providers.len(), 1);
    }

    #[test]
    fn a_link_is_kept_only_under_the_service_it_goes_to() {
        let link = direct;
        let mut s = song("A", "One");
        s.providers = vec![
            link(
                " Apple Music ",
                "https://geo.music.apple.com/us/album/_/1?i=2",
            ),
            link("Spotify", "https://x.example/?u=open.spotify.com"),
            link("YouTube", "https://music.youtube.com/watch?v=1"),
            link("Amazon Music", "https://www.amazon.com/dp/B0"),
            link("Listen", "https://open.spotify.com/track/1"),
        ];
        s.shazam_url = Some("https://shazam.com.x.example/track/1".into());
        s.song_link = Some("https://song.link/s/1".into());
        let mut h = History::default();
        h.file(&s, None, 0);
        let kept = &h.songs[0];
        assert_eq!(kept.providers.len(), 1);
        assert_eq!(kept.providers[0].name, "Apple Music");
        assert_eq!(kept.shazam_url, None);
        assert_eq!(kept.song_link.as_deref(), Some("https://song.link/s/1"));

        // A stored file (an older build's, or edited) is held to the same rule.
        let stored: SongEntry = serde_json::from_str(
            r#"{"key":"a\u001fone","title":"One","artist":"A","providers":[{"name":"Spotify","url":"https://open.spotify.com/track/1"},{"name":"Spotify","url":"https://open.spotify.com.x.example/"},{"name":"Amazon Music","url":"https://www.amazon.com/dp/B0"}],"song_link":"https://x.example/song.link"}"#,
        )
        .unwrap();
        let h = History::from_stored(Stored {
            songs: vec![stored],
            newer: false,
        });
        let s = &h.songs[0];
        assert_eq!(s.providers.len(), 1);
        assert_eq!(s.providers[0].url, "https://open.spotify.com/track/1");
        assert_eq!(s.song_link, None);
    }

    #[test]
    fn the_song_heard_longest_ago_goes_past_the_cap() {
        let mut h = history(
            (0..MAX_SONGS)
                .rev()
                .map(|i| entry(&format!("t{i}"), i as i64))
                .collect(),
        );
        h.file(&song("A", "new"), None, MAX_SONGS as i64);
        assert_eq!(h.songs.len(), MAX_SONGS);
        assert_eq!(h.songs[0].title, "new");
        assert!(!h.songs.iter().any(|s| s.title == "t0"));
    }

    #[test]
    fn search_needs_every_word_somewhere() {
        let mut h = History::default();
        let mut a = song("Daft Punk", "Veridis Quo");
        a.album = Some("Discovery".into());
        let kick = SongSource {
            provider: "kick".into(),
            channel_login: "adinross".into(),
            channel_name: Some("AdinRoss".into()),
            category: Some("Just Chatting".into()),
            title: Some("late night music".into()),
            ..Default::default()
        };
        h.file(&a, Some(kick), 0);
        h.file(&song("Sleep Token", "Telomeres"), on("xqc"), 1);
        assert_eq!(h.page("daft discovery", 0, 50).rows.len(), 1);
        assert_eq!(h.page("ADIN quo", 0, 50).rows.len(), 1);
        assert_eq!(h.page("xqc", 0, 50).rows[0].title, "Telomeres");
        assert_eq!(h.page("daft telomeres", 0, 50).matched, 0);
        assert_eq!(h.page("  ", 0, 50).matched, 2);
        assert_eq!(h.page("chatting", 0, 50).rows[0].title, "Veridis Quo");
        assert_eq!(
            h.page("late night", 0, 50).matched,
            0,
            "a stream's title is shown, not searched"
        );
        assert!(h.page("", 0, 50).mixed_platforms);
    }

    #[test]
    fn pages_count_the_channels_heard_on() {
        let mut h = History::default();
        assert_eq!(h.page("", 0, 50).channels, 0);
        h.file(&song("A", "One"), on("xqc"), 0);
        h.file(&song("A", "Two"), on("XQC"), 1);
        h.file(&song("A", "Three"), on("zackrawrr"), 2);
        h.file(&song("A", "Four"), None, 3);
        let kick = SongSource {
            provider: "kick".into(),
            channel_login: "xqc".into(),
            ..Default::default()
        };
        h.file(&song("A", "Five"), Some(kick), 4);
        let page = h.page("one", 0, 50);
        assert_eq!(page.matched, 1);
        assert_eq!(
            page.channels, 3,
            "every song counts, whatever matched; Kick's xqc is its own channel"
        );
    }

    #[test]
    fn search_ignores_accents_and_apostrophes() {
        let mut h = History::default();
        h.file(&song("Beyoncé", "Don’t Hurt Yourself"), None, 0);
        h.file(&song("Sigur Rós", "Hoppípolla"), None, 1);
        assert_eq!(h.page("beyonce dont", 0, 50).matched, 1);
        assert_eq!(h.page("SIGUR ros hoppipolla", 0, 50).matched, 1);
        assert_eq!(h.page("Beyoncé", 0, 50).matched, 1);
    }

    #[test]
    fn pages_count_every_match_and_return_their_slice() {
        let mut h = history((0..120).rev().map(|i| entry(&format!("t{i}"), i)).collect());
        let p = h.page("", 100, 50);
        assert_eq!((p.matched, p.total, p.rows.len()), (120, 120, 20));
        assert_eq!(p.rows[0].title, "t19");
        assert_eq!(h.page("", 0, 0).rows.len(), 1);
        assert_eq!(h.page("", 0, 500).rows.len(), MAX_PAGE);
        assert!(h.remove(&song_key("A", "t5")));
        assert!(!h.remove(&song_key("A", "t5")));
        assert_eq!(h.page("", 0, 50).total, 119);
    }

    #[test]
    fn a_removed_song_comes_back_in_its_place() {
        let mut h = history(vec![entry("c", 3), entry("b", 2), entry("a", 1)]);
        assert!(h.remove(&song_key("A", "b")));
        assert!(
            !h.restore(&song_key("A", "a")),
            "only the song that was removed comes back"
        );
        assert!(h.restore(&song_key("A", "b")));
        assert_eq!(titles(&h), vec!["c", "b", "a"]);
        assert!(!h.restore(&song_key("A", "b")), "an undo runs once");
    }

    #[test]
    fn undo_after_hearing_it_again_keeps_one_row_with_both() {
        let mut h = History::default();
        h.file(&song("A", "One"), on("x"), 0);
        let key = song_key("A", "One");
        assert!(h.remove(&key));
        h.file(&song("A", "One"), on("y"), 10);
        assert!(h.restore(&key));
        assert_eq!(h.songs.len(), 1);
        let s = &h.songs[0];
        assert_eq!((s.first_ms, s.count, s.sightings.len()), (0, 2, 2));
        assert_eq!(s.sightings[0].source.as_ref().unwrap().channel_login, "y");
    }

    #[test]
    fn a_source_must_name_a_known_platform_and_a_channel() {
        let ok = SongSource {
            provider: " Twitch ".into(),
            channel_login: " xqc ".into(),
            channel_name: Some("  ".into()),
            media: Some("LIVE".into()),
            ..Default::default()
        }
        .cleaned()
        .unwrap();
        assert_eq!(ok.provider, "twitch");
        assert_eq!(ok.channel_login, "xqc");
        assert_eq!(ok.channel_name, None);
        assert_eq!(ok.media.as_deref(), Some("live"));
        let bad_platform = SongSource {
            provider: "myspace".into(),
            channel_login: "x".into(),
            ..Default::default()
        };
        assert!(bad_platform.cleaned().is_none());
        let no_channel = SongSource {
            provider: "kick".into(),
            channel_login: " ".into(),
            ..Default::default()
        };
        assert!(no_channel.cleaned().is_none());
        let long = SongSource {
            provider: "kick".into(),
            channel_login: "a".repeat(500),
            media: Some("radio".into()),
            title: Some("t".repeat(900)),
            category: Some("c".repeat(500)),
            position_secs: Some(MAX_POSITION_SECS + 1),
            ..Default::default()
        }
        .cleaned()
        .unwrap();
        assert_eq!(long.channel_login.chars().count(), MAX_FIELD);
        assert_eq!(long.media, None);
        assert_eq!(long.title.unwrap().chars().count(), MAX_TEXT);
        assert_eq!(long.category.unwrap().chars().count(), MAX_FIELD);
        assert_eq!(long.position_secs, None);
    }

    #[test]
    fn a_source_keeps_the_stream_it_heard() {
        let live = SongSource {
            provider: "twitch".into(),
            channel_login: "xqc".into(),
            media: Some("live".into()),
            title: Some("  react andy  ".into()),
            category: Some(" ".into()),
            avatar: Some("javascript:alert(1)".into()),
            position_secs: Some(8040),
            ..Default::default()
        }
        .cleaned()
        .unwrap();
        assert_eq!(live.title.as_deref(), Some("react andy"));
        assert_eq!(live.category, None);
        assert_eq!(live.avatar, None, "only web links are kept");
        assert_eq!(live.position_secs, Some(8040));
        let clip = SongSource {
            provider: "twitch".into(),
            channel_login: "xqc".into(),
            media: Some("clip".into()),
            position_secs: Some(12),
            ..Default::default()
        }
        .cleaned()
        .unwrap();
        assert_eq!(clip.position_secs, None, "a clip's own time is no position");
        let sent = |position: &str| {
            serde_json::from_str::<SongSource>(&format!(
                r#"{{"provider":"twitch","channel_login":"x","media":"video","position_secs":{position}}}"#
            ))
            .unwrap()
            .position_secs
        };
        assert_eq!(sent("8040.7"), Some(8040));
        assert_eq!(sent("-3"), None);
        assert_eq!(sent("\"soon\""), None);
        assert_eq!(sent("null"), None);
    }

    #[test]
    fn hearings_kept_before_the_stream_fields_still_read() {
        let old: SongEntry = serde_json::from_str(
            r#"{"key":"a\u001fone","title":"One","artist":"A","sightings":[{"at_ms":5,"source":{"provider":"twitch","channel_login":"x","channel_name":"X","media":"live"}}]}"#,
        )
        .unwrap();
        let source = old.sightings[0].source.as_ref().unwrap();
        assert_eq!(
            (
                &source.title,
                &source.category,
                &source.avatar,
                source.position_secs
            ),
            (&None, &None, &None, None)
        );
        let json = serde_json::to_string(source).unwrap();
        assert!(
            !json.contains("title") && !json.contains("position_secs"),
            "missing fields stay out of the file: {json}"
        );
    }

    #[test]
    fn twitch_pictures_are_kept_small() {
        let url = |file: &str| format!("https://static-cdn.jtvnw.net/jtv_user_pictures/{file}");
        assert_eq!(
            small_avatar(&url("abc-profile_image-300x300.png")),
            Some(url("abc-profile_image-70x70.png"))
        );
        assert_eq!(
            small_avatar(&url("a-b-c-profile_image-600x600.jpeg")),
            Some(url("a-b-c-profile_image-70x70.jpeg"))
        );
        assert_eq!(small_avatar(&url("abc-banner-1920x480.png")), None);
        assert_eq!(
            small_avatar("https://files.kick.com/images/user/1/profile_image/x.webp"),
            None
        );
        let kept = SongSource {
            provider: "twitch".into(),
            channel_login: "x".into(),
            avatar: Some(url("abc-profile_image-300x300.png")),
            ..Default::default()
        }
        .cleaned()
        .unwrap();
        assert_eq!(kept.avatar, Some(url("abc-profile_image-70x70.png")));
    }

    #[test]
    fn covers_on_apples_host_are_asked_for_small() {
        assert_eq!(
            thumb_of(
                "https://is1-ssl.mzstatic.com/image/thumb/Music125/v4/ab/cd/x.jpg/400x400cc.jpg"
            )
            .as_deref(),
            Some("https://is1-ssl.mzstatic.com/image/thumb/Music125/v4/ab/cd/x.jpg/96x96cc.jpg")
        );
        assert_eq!(
            thumb_of("https://is1-ssl.mzstatic.com/image/thumb/Music/x.png/1200x1200bb.webp")
                .as_deref(),
            Some("https://is1-ssl.mzstatic.com/image/thumb/Music/x.png/96x96bb.webp")
        );
        assert_eq!(thumb_of("https://i.scdn.co/image/ab67616d0000b273"), None);
        assert_eq!(
            thumb_of("https://is1-ssl.mzstatic.com/image/thumb/Music/x.jpg/source.jpg"),
            None
        );
    }

    #[test]
    fn rows_carry_the_small_cover_and_the_full_one() {
        let mut h = History::default();
        let mut s = song("A", "One");
        s.album_art = Some("https://is1-ssl.mzstatic.com/image/thumb/M/x.jpg/400x400cc.jpg".into());
        h.file(&s, on("x"), 0);
        let page = h.page("", 0, 1);
        let row = &page.rows[0];
        assert!(row.art.as_deref().unwrap().ends_with("/96x96cc.jpg"));
        assert!(row.art_full.as_deref().unwrap().ends_with("/400x400cc.jpg"));
        assert_eq!(row.heard.as_ref().unwrap().channel_login, "x");
        assert!(!page.mixed_platforms);
    }

    #[test]
    fn a_file_that_does_not_parse_is_kept_aside_and_never_written_over() {
        let dir = temp_dir("bad");
        let path = dir.join(FILE);
        assert!(
            read_file(&path).unwrap().songs.is_empty(),
            "a missing file is an empty history"
        );
        let damaged: &[u8] = b"\xff\xfe{ not json";
        fs::write(&path, damaged).unwrap();
        let back = read_file(&path).unwrap();
        assert!(back.songs.is_empty() && !back.newer);
        assert!(!path.exists());
        let aside: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("song_history.json.bad-")
            })
            .collect();
        assert_eq!(aside.len(), 1);
        assert_eq!(fs::read(aside[0].path()).unwrap(), damaged);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_cannot_be_read_is_an_error_and_left_alone() {
        let dir = temp_dir("unreadable");
        let path = dir.join(FILE);
        fs::create_dir_all(&path).unwrap();
        assert!(read_file(&path).is_err());
        assert!(path.is_dir());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_newer_apps_file_is_shown_and_never_changed() {
        let dir = temp_dir("newer");
        let path = dir.join(FILE);
        // A row this app reads, one it cannot (a field of another type), and
        // a field it does not know.
        let text = r#"{"version":2,"songs":[{"key":"a\u001fone","title":"One","artist":"A","last_ms":5,"count":2,"mood":"x"},{"key":"b","title":7}]}"#;
        fs::write(&path, text).unwrap();
        let back = read_file(&path).unwrap();
        assert!(back.newer);
        assert_eq!(back.songs.len(), 1);
        let mut h = History::from_stored(back);
        assert!(h.read_only);
        assert_eq!(h.page("one", 0, 10).matched, 1);
        assert!(apply(&mut h, &path, |h| (h.remove("a\u{1f}one"), true)).is_err());
        assert_eq!(h.songs.len(), 1);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            text,
            "the file is left exactly as it was"
        );
        assert_eq!(names_in(&dir), vec![FILE.to_string()]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_row_missing_fields_still_reads_and_a_row_missing_its_name_is_dropped() {
        let dir = temp_dir("partial");
        let path = dir.join(FILE);
        fs::write(
            &path,
            r#"{"songs":[{"key":"a\u001fone","title":"One","artist":"A"},{"key":"x","title":""},{"key":"k","title":"Two","sightings":[{"at_ms":1,"source":{"provider":"myspace","channel_login":"z"}}]}]}"#,
        )
        .unwrap();
        let h = History::from_stored(read_file(&path).unwrap());
        assert_eq!(h.songs.len(), 2);
        assert!(h.songs.iter().all(|s| s.count == 1));
        let two = h.songs.iter().find(|s| s.title == "Two").unwrap();
        assert_eq!(
            two.sightings[0].source, None,
            "an unknown platform is not shown"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stored_row_is_made_safe_to_show() {
        let mut e = entry("One", 1);
        e.album_art = Some("file:///etc/passwd".into());
        e.providers = vec![direct("X", "ms-settings:")];
        e.count = 0;
        let h = History::from_stored(stored(vec![e]));
        let s = &h.songs[0];
        assert_eq!(
            (s.album_art.clone(), s.providers.len(), s.count),
            (None, 0, 1)
        );
    }

    #[test]
    fn a_written_history_reads_back_in_order_and_leaves_no_temp() {
        let dir = temp_dir("roundtrip");
        let path = dir.join(FILE);
        let mut h = History::default();
        h.file(&song("A", "One"), on("x"), 1);
        h.file(&song("B", "Two"), None, 2);
        write_file(&path, &h).unwrap();
        assert_eq!(names_in(&dir), vec![FILE.to_string()]);
        let back = History::from_stored(read_file(&path).unwrap());
        assert_eq!(titles(&back), vec!["Two", "One"]);
        assert_eq!(
            back.page("x", 0, 10).matched,
            1,
            "search words are rebuilt on load"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn temps_a_crash_left_are_swept_and_nothing_else() {
        let dir = temp_dir("sweep");
        let path = dir.join(FILE);
        fs::write(dir.join("song_history.json.0123abcd.tmp"), "x").unwrap();
        fs::write(dir.join("song_history.json.bad-5"), "x").unwrap();
        fs::write(dir.join("other.json.tmp"), "x").unwrap();
        sweep_temps(&path);
        assert_eq!(
            names_in(&dir),
            vec![
                "other.json.tmp".to_string(),
                "song_history.json.bad-5".to_string()
            ]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_change_writes_when_it_changed_something() {
        let dir = temp_dir("apply");
        let path = dir.join(FILE);
        let mut h = history(vec![entry("One", 1)]);
        let (removed, saved) = apply(&mut h, &path, |h| {
            let r = h.remove("nothing");
            (r, r)
        })
        .unwrap();
        assert!(!removed && saved.is_ok() && !path.exists());
        let (removed, saved) = apply(&mut h, &path, |h| {
            let r = h.remove(&song_key("A", "One"));
            (r, r)
        })
        .unwrap();
        assert!(removed && saved.is_ok() && path.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn loading_drops_duplicate_keys_keeping_the_newest() {
        let h = History::from_stored(stored(vec![entry("t1", 1), entry("t1", 5), entry("t2", 3)]));
        assert_eq!(
            h.songs.iter().map(|s| s.last_ms).collect::<Vec<_>>(),
            vec![5, 3]
        );
    }

    #[test]
    fn change_events_carry_their_kind() {
        assert_eq!(
            serde_json::to_value(Change::Filed { key: "k".into() }).unwrap(),
            serde_json::json!({ "kind": "filed", "key": "k" })
        );
        assert_eq!(
            serde_json::to_value(Change::Cleared).unwrap(),
            serde_json::json!({ "kind": "cleared" })
        );
    }

    fn held(used: Instant) -> Option<Loaded> {
        Some(Loaded {
            history: History::default(),
            tints: Tints::default(),
            path: PathBuf::from("unused"),
            used,
        })
    }

    fn ok(_: &Path, _: &History) -> Result<(), String> {
        Ok(())
    }

    fn fail(_: &Path, _: &History) -> Result<(), String> {
        Err("disk full".to_string())
    }

    fn tints_ok(_: &mut Tints, _: &History) -> Result<(), String> {
        Ok(())
    }

    fn tints_fail(_: &mut Tints, _: &History) -> Result<(), String> {
        Err("disk full".to_string())
    }

    #[test]
    fn the_unloader_waits_out_the_idle_time_then_lets_go() {
        let dirty = AtomicBool::new(false);
        let tint_dirty = AtomicBool::new(false);
        let mut slot = held(Instant::now());
        let pass = |slot: &mut Option<Loaded>, idle| {
            unload_pass(slot, idle, &dirty, &tint_dirty, ok, tints_ok)
        };
        assert!(pass(&mut slot, Duration::from_secs(60)).is_some());
        assert!(slot.is_some());
        assert_eq!(pass(&mut slot, Duration::ZERO), None);
        assert!(slot.is_none());
        assert_eq!(pass(&mut slot, Duration::ZERO), None, "nothing held");
    }

    #[test]
    fn the_unloader_keeps_songs_a_write_has_not_saved() {
        let dirty = AtomicBool::new(true);
        let tint_dirty = AtomicBool::new(false);
        let mut slot = held(Instant::now());
        assert_eq!(
            unload_pass(&mut slot, Duration::ZERO, &dirty, &tint_dirty, fail, tints_ok),
            Some(IDLE_UNLOAD)
        );
        assert!(slot.is_some() && dirty.load(Ordering::Acquire));
        assert_eq!(
            unload_pass(&mut slot, Duration::ZERO, &dirty, &tint_dirty, ok, tints_ok),
            None
        );
        assert!(slot.is_none() && !dirty.load(Ordering::Acquire));
    }

    #[test]
    fn the_two_files_dirty_flags_never_stand_in_for_each_other() {
        // A failing history write with a good tint write keeps the songs.
        let dirty = AtomicBool::new(true);
        let tint_dirty = AtomicBool::new(true);
        let mut slot = held(Instant::now());
        assert_eq!(
            unload_pass(&mut slot, Duration::ZERO, &dirty, &tint_dirty, fail, tints_ok),
            Some(IDLE_UNLOAD)
        );
        assert!(slot.is_some());
        assert!(dirty.load(Ordering::Acquire), "the songs still wait for a write");
        assert!(!tint_dirty.load(Ordering::Acquire), "the colors landed");

        // A failing tint write never keeps them: after its one more try the
        // songs go, and its flag goes with the slot.
        let dirty = AtomicBool::new(false);
        let tint_dirty = AtomicBool::new(true);
        let tries = std::cell::Cell::new(0);
        let counted_fail = |_: &mut Tints, _: &History| {
            tries.set(tries.get() + 1);
            Err("disk full".to_string())
        };
        let mut slot = held(Instant::now());
        assert_eq!(
            unload_pass(&mut slot, Duration::ZERO, &dirty, &tint_dirty, ok, counted_fail),
            None
        );
        assert!(slot.is_none());
        assert_eq!(tries.get(), 1);
        assert!(
            !tint_dirty.load(Ordering::Acquire),
            "no flag is left over for the next load"
        );
    }

    #[test]
    fn a_newer_apps_history_is_never_written_by_the_unloader_or_at_exit() {
        let writes = std::cell::Cell::new(0);
        let counted = |_: &Path, _: &History| {
            writes.set(writes.get() + 1);
            Ok(())
        };
        let read_only = |used| {
            let mut slot = held(used);
            slot.as_mut().unwrap().history.read_only = true;
            slot
        };

        let dirty = AtomicBool::new(true);
        let tint_dirty = AtomicBool::new(false);
        let mut slot = read_only(Instant::now());
        assert_eq!(
            unload_pass(&mut slot, Duration::ZERO, &dirty, &tint_dirty, counted, tints_ok),
            None,
            "nothing of this app's is kept for it either"
        );
        assert_eq!(writes.get(), 0);

        let dirty = AtomicBool::new(true);
        let mut slot = read_only(Instant::now());
        assert!(flush_slot(&mut slot, &dirty, &tint_dirty, counted, tints_ok).is_ok());
        assert_eq!(writes.get(), 0, "flush_now and the deferred write refuse it");
    }

    #[test]
    fn a_pending_change_is_written_once_at_exit_and_then_nothing_is() {
        let writes = std::cell::Cell::new(0);
        let counted = |_: &Path, _: &History| {
            writes.set(writes.get() + 1);
            Ok(())
        };
        let dirty = AtomicBool::new(true);
        let tint_dirty = AtomicBool::new(false);
        let mut slot = held(Instant::now());
        assert!(flush_slot(&mut slot, &dirty, &tint_dirty, counted, tints_ok).is_ok());
        assert!(flush_slot(&mut slot, &dirty, &tint_dirty, counted, tints_ok).is_ok());
        assert_eq!(writes.get(), 1);
        assert!(!dirty.load(Ordering::Acquire));

        // A failed write is reported and stays pending.
        let dirty = AtomicBool::new(true);
        assert!(flush_slot(&mut slot, &dirty, &tint_dirty, fail, tints_ok).is_err());
        assert!(dirty.load(Ordering::Acquire));
        let mut empty = None;
        assert!(flush_slot(&mut empty, &dirty, &tint_dirty, fail, tints_fail).is_ok());
    }

    #[test]
    fn the_update_restart_flushes_the_songs() {
        let source = include_str!("../../commands/components.rs");
        let start = source
            .find("fn flush_stores_before_hard_exit")
            .expect("the hard-exit flush exists");
        let body = &source[start..];
        let body = &body[..body.find("\n}").expect("its body ends")];
        assert!(
            body.contains("song_id::history::flush_now()"),
            "a pending song change is written before the update restart"
        );
    }

    // ---------------------------------------------------------------- groups

    fn at(last_ms: &[i64]) -> History {
        history(
            last_ms
                .iter()
                .enumerate()
                .map(|(i, ms)| entry(&format!("t{i}"), *ms))
                .collect(),
        )
    }

    fn groups(h: &History, query: &str, bounds: &[i64]) -> Vec<(usize, usize)> {
        h.page_in(query, 0, 50, bounds, &BTreeMap::new(), 0)
            .0
            .groups
            .iter()
            .map(|g| (g.bound, g.start))
            .collect()
    }

    #[test]
    fn groups_split_the_songs_by_the_pages_bounds() {
        // Today from 1000, yesterday from 500, two months back from 100.
        let bounds = [1000, 500, 100];
        let h = at(&[1500, 1000, 700, 300, 50]);
        assert_eq!(
            groups(&h, "", &bounds),
            vec![(0, 0), (1, 2), (2, 3), (3, 4)],
            "a song at a bound is in it; past the last is Older"
        );
        assert!(groups(&h, "", &[]).is_empty(), "no bounds give no groups");
        let empty = at(&[]);
        assert!(groups(&empty, "", &bounds).is_empty());
        // An empty bucket is skipped.
        assert_eq!(groups(&at(&[1500, 300]), "", &bounds), vec![(0, 0), (2, 1)]);
        // A query counts matched songs only.
        assert_eq!(groups(&h, "t3", &bounds), vec![(2, 0)]);
        let page = h.page_in("", 0, 2, &bounds, &BTreeMap::new(), 0).0;
        assert_eq!(page.rows.len(), 2);
        assert_eq!(page.groups.len(), 4, "every page carries every group");
        let json = serde_json::to_value(&page).unwrap();
        assert_eq!(json["groups"][1], serde_json::json!({ "bound": 1, "start": 2 }));
    }

    #[test]
    fn bounds_are_sorted_deduped_and_capped() {
        let h = at(&[1500, 1000, 700, 300, 50]);
        assert_eq!(
            groups(&h, "", &[100, 1000, 500, 1000]),
            groups(&h, "", &[1000, 500, 100])
        );
        let many: Vec<i64> = (0..100).map(|i| 2000 - i * 10).collect();
        assert_eq!(sorted_bounds(&many).len(), MAX_BOUNDS);
        assert_eq!(sorted_bounds(&many)[0], 2000);
        // 1500 is the 51st bound (2000 - 50 * 10); 50 is past all 64 kept.
        let g = groups(&at(&[1500, 50]), "", &many);
        assert_eq!(g, vec![(50, 0), (MAX_BOUNDS, 1)]);
    }

    #[test]
    fn a_song_from_a_clock_that_jumped_back_stays_in_the_group_above() {
        // The second song was filed under a clock that read too early.
        let h = at(&[1500, 300, 1200]);
        assert_eq!(groups(&h, "", &[1000, 500, 100]), vec![(0, 0), (2, 1)]);
    }

    // ---------------------------------------------------------------- tints

    fn color(h: f64) -> TintEntry {
        TintEntry::Color {
            main: Some([0.6, 0.15, h]),
            second: None,
        }
    }

    #[test]
    fn rows_and_the_detail_carry_the_covers_colors_in_three_states() {
        let mut tints = BTreeMap::new();
        tints.insert("https://c/colored.jpg".to_string(), color(26.0));
        tints.insert(
            "https://c/grey.jpg".to_string(),
            TintEntry::Color {
                main: None,
                second: None,
            },
        );
        tints.insert(
            "https://c/failed.jpg".to_string(),
            TintEntry::Failed { failed_ms: 5 },
        );
        let mut h = History::default();
        for (title, art) in [
            ("colored", Some("https://c/colored.jpg")),
            ("grey", Some("https://c/grey.jpg")),
            ("failed", Some("https://c/failed.jpg")),
            ("pending", Some("https://c/pending.jpg")),
            ("none", None),
        ] {
            let mut s = song("A", title);
            s.album_art = art.map(str::to_string);
            h.file(&s, None, 0);
        }
        let (page, covers) = h.page_in("", 0, 50, &[], &tints, 10);
        let tint_of = |title: &str| {
            let row = page.rows.iter().find(|r| r.title == title).unwrap();
            serde_json::to_value(row).unwrap().get("tint").cloned()
        };
        let grey = Some(serde_json::json!({ "main": null, "second": null }));
        assert_eq!(
            tint_of("colored"),
            Some(serde_json::json!({ "main": [0.6, 0.15, 26.0], "second": null }))
        );
        assert_eq!(tint_of("grey"), grey);
        assert_eq!(tint_of("failed"), grey, "nothing more will come");
        assert_eq!(tint_of("none"), grey);
        assert_eq!(tint_of("pending"), None, "left out while a color may come");
        assert_eq!(covers, vec!["https://c/pending.jpg".to_string()]);

        // A failure is fetched again after 30 days.
        let (_, covers) = h.page_in("", 0, 50, &[], &tints, 5 + cover_tint::RETRY_PERMANENT_MS);
        assert!(covers.contains(&"https://c/failed.jpg".to_string()));

        let song = h.songs.iter().find(|s| s.title == "colored").unwrap();
        let detail = SongDetail {
            tint: song.tint(&tints),
            entry: song.clone(),
        };
        let json = serde_json::to_value(&detail).unwrap();
        assert_eq!(json["title"], "colored");
        assert_eq!(json["tint"]["main"], serde_json::json!([0.6, 0.15, 26.0]));
        assert_eq!(json["links_are_searches"], false, "still written for older builds");
    }

    #[test]
    fn the_colors_kept_are_those_a_song_or_the_undo_slot_uses() {
        let mut h = History::default();
        for (title, art) in [("a", "https://c/a.jpg"), ("b", "https://c/b.jpg")] {
            let mut s = song("A", title);
            s.album_art = Some(art.to_string());
            h.file(&s, None, 0);
        }
        assert!(h.remove(&song_key("A", "b")));
        let mut map: BTreeMap<String, TintEntry> = ["a", "b", "gone"]
            .iter()
            .map(|n| (format!("https://c/{n}.jpg"), color(1.0)))
            .collect();
        prune_tints(&mut map, &h);
        assert_eq!(
            map.keys().cloned().collect::<Vec<_>>(),
            vec!["https://c/a.jpg".to_string(), "https://c/b.jpg".to_string()]
        );
    }

    #[test]
    fn landed_colors_name_their_songs_and_write_once() {
        let mut h = History::default();
        for (title, art) in [("a", "https://c/same.jpg"), ("b", "https://c/same.jpg"), ("c", "https://c/c.jpg")] {
            let mut s = song("A", title);
            s.album_art = Some(art.to_string());
            h.file(&s, None, 0);
        }
        let writes = std::cell::Cell::new(0);
        let counted = |_: &mut Tints, _: &History| {
            writes.set(writes.get() + 1);
            Ok(())
        };
        let tint_dirty = AtomicBool::new(true);
        let mut tints = Tints::default();
        let mut keys = land_tints(
            &h,
            &mut tints,
            vec![("https://c/same.jpg".to_string(), color(5.0))],
            false,
            &tint_dirty,
            counted,
        );
        keys.sort();
        assert_eq!(keys, vec![song_key("A", "a"), song_key("A", "b")]);
        assert_eq!(writes.get(), 1);
        assert!(!tint_dirty.load(Ordering::Acquire));

        // A failed write stays pending; while tints are off nothing is
        // written and nothing is marked pending.
        land_tints(&h, &mut tints, vec![], false, &tint_dirty, tints_fail);
        assert!(tint_dirty.load(Ordering::Acquire));
        let tint_dirty = AtomicBool::new(false);
        let writes_before = writes.get();
        land_tints(
            &h,
            &mut tints,
            vec![("https://c/c.jpg".to_string(), color(9.0))],
            true,
            &tint_dirty,
            counted,
        );
        assert_eq!(writes.get(), writes_before);
        assert!(!tint_dirty.load(Ordering::Acquire));
        assert!(tints.map.contains_key("https://c/c.jpg"), "still shown this load");
    }

    #[test]
    fn a_damaged_tint_file_is_moved_aside_and_never_stops_the_songs_loading() {
        let mut h = History::default();
        h.file(&song("A", "One"), None, 1);

        let damaged = temp_dir("tints-damaged");
        let path = damaged.join(FILE);
        write_file(&path, &h).unwrap();
        fs::write(damaged.join(cover_tint::FILE), b"{ not json").unwrap();
        let (loaded, off) = load(path).unwrap();
        assert_eq!(titles(&loaded.history), vec!["One"]);
        assert!(loaded.tints.map.is_empty());
        assert!(off, "tints are off for this load");
        let names = names_in(&damaged);
        assert!(!names.contains(&cover_tint::FILE.to_string()));
        assert!(names.iter().any(|n| n.starts_with("song_tints.json.bad-")));

        // A tint file that cannot be read at all is left where it is.
        let unreadable = temp_dir("tints-unreadable");
        let path = unreadable.join(FILE);
        write_file(&path, &h).unwrap();
        fs::create_dir_all(unreadable.join(cover_tint::FILE)).unwrap();
        let (loaded, off) = load(path).unwrap();
        assert!(off && loaded.tints.map.is_empty());
        assert_eq!(titles(&loaded.history), vec!["One"]);
        assert!(unreadable.join(cover_tint::FILE).is_dir());

        let _ = fs::remove_dir_all(&damaged);
        let _ = fs::remove_dir_all(&unreadable);
    }

    // ---------------------------------------------------------------- links

    fn resolved(providers: Vec<Provider>, complete: bool) -> links::Resolved {
        links::Resolved {
            providers,
            isrc: None,
            isrc_from: None,
            cover: None,
            complete,
        }
    }

    fn kept(e: &SongEntry) -> Vec<(String, bool)> {
        e.providers
            .iter()
            .map(|p| (p.name.clone(), p.search))
            .collect()
    }

    #[test]
    fn an_old_search_flagged_song_turns_into_per_link_searches() {
        let stored: SongEntry = serde_json::from_str(
            r#"{"key":"a\u001fone","title":"One","artist":"A","links_are_searches":true,"providers":[{"name":"Spotify","url":"https://open.spotify.com/search/A%20One"},{"name":"YouTube Music","url":"https://music.youtube.com/search?q=A%20One"},{"name":"Apple Music","url":"https://music.apple.com/us/search?term=A%20One"}]}"#,
        )
        .unwrap();
        // A song since Odesli closed: Shazam's hub gave a YouTube Music
        // search page that the old build kept as a direct link.
        let hub: SongEntry = serde_json::from_str(
            r#"{"key":"b\u001ftwo","title":"Two","artist":"B","providers":[{"name":"Apple Music","url":"https://music.apple.com/us/album/x/1?i=2"},{"name":"YouTube Music","url":"https://music.youtube.com/search?q=Two+B&feature=shazam"}]}"#,
        )
        .unwrap();
        let h = History::from_stored(super::tests::stored(vec![stored, hub]));
        let one = h.songs.iter().find(|s| s.title == "One").unwrap();
        assert!(one.providers.iter().all(|p| p.search));
        assert!(one.links_are_searches, "written as derived for older builds");
        assert_eq!(
            kept(one),
            vec![
                ("Spotify".to_string(), true),
                ("Apple Music".to_string(), true),
                ("YouTube Music".to_string(), true)
            ]
        );
        let json = serde_json::to_string(one).unwrap();
        assert!(json.contains(r#""search":true"#));
        let two = h.songs.iter().find(|s| s.title == "Two").unwrap();
        assert_eq!(
            kept(two),
            vec![("Apple Music".to_string(), false), ("YouTube Music".to_string(), true)]
        );
        assert!(!two.links_are_searches);
        assert_eq!(two.links_checked_ms, 0, "so opening it looks the links up");
    }

    #[test]
    fn a_direct_link_fills_a_search_slot_and_never_replaces_a_direct_one() {
        let mut e = entry("One", 1);
        e.providers = vec![
            direct("Spotify", "https://open.spotify.com/track/kept"),
            searching("YouTube Music"),
        ];
        let found = resolved(
            vec![
                direct("Spotify", "https://open.spotify.com/track/other"),
                direct("YouTube Music", "https://music.youtube.com/watch?v=4MOzk7-6j-I"),
                searching("Deezer"),
                searching("Spotify"),
            ],
            false,
        );
        assert!(merge_links(&mut e, &found, 50));
        assert_eq!(
            kept(&e),
            vec![
                ("Spotify".to_string(), false),
                ("YouTube Music".to_string(), false),
                ("Deezer".to_string(), true)
            ]
        );
        assert_eq!(e.providers[0].url, "https://open.spotify.com/track/kept");
        assert!(!merge_links(&mut e, &found, 60), "the same links again change nothing");
    }

    #[test]
    fn links_are_stamped_only_by_a_complete_lookup() {
        let mut e = entry("One", 1);
        merge_links(&mut e, &resolved(vec![searching("Deezer")], false), 50);
        assert_eq!(e.links_checked_ms, 0);
        assert!(merge_links(&mut e, &resolved(vec![], true), 60));
        assert_eq!(e.links_checked_ms, 60);
        merge_links(&mut e, &resolved(vec![], true), 40);
        assert_eq!(e.links_checked_ms, 60, "never back in time");
    }

    #[test]
    fn a_reidentification_cut_short_keeps_earlier_links_isrc_and_stamp() {
        let mut h = History::default();
        let mut first = song("Sade", "Kiss of Life");
        first.providers = vec![
            direct("Spotify", "https://open.spotify.com/track/0123456789abcdefghijkl"),
            direct("Deezer", "https://www.deezer.com/track/1030607572"),
        ];
        first.isrc = Some("GBBBM9200027".into());
        first.isrc_from = Some(IsrcFrom::Shazam);
        first.links_complete = true;
        h.file(&first, None, 1_000);
        // Heard again; this time the lookups ran out of time and Shazam gave
        // no ISRC.
        let mut again = song("Sade", "Kiss of Life");
        again.providers = std::iter::once(direct("Apple Music", "https://music.apple.com/us/song/1"))
            .chain(
                links::DISPLAY
                    .iter()
                    .filter_map(|s| links::search_link(s, "Sade", "Kiss of Life")),
            )
            .collect();
        h.file(&again, None, 2 * REPEAT_WINDOW_MS);
        let s = &h.songs[0];
        assert_eq!(s.isrc.as_deref(), Some("GBBBM9200027"));
        assert_eq!(s.links_checked_ms, 1_000);
        assert_eq!(
            s.providers
                .iter()
                .filter(|p| !p.search)
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Spotify", "Apple Music", "Deezer"]
        );
        assert_eq!(s.providers[0].url, "https://open.spotify.com/track/0123456789abcdefghijkl");
        assert!(s.providers.iter().skip(3).all(|p| p.search));
    }

    #[test]
    fn a_shazam_isrc_replaces_a_text_one_and_drops_the_links_made_from_it() {
        let mut e = entry("One", 1);
        e.isrc = Some("GBAAA0000001".into());
        e.isrc_from = Some(IsrcFrom::Text);
        e.links_checked_ms = 500;
        e.providers = vec![
            direct("Spotify", "https://open.spotify.com/track/0123456789abcdefghijkl"),
            direct("Apple Music", "https://music.apple.com/us/song/7"),
            direct("Deezer", "https://www.deezer.com/track/1"),
            direct("Beatport", "https://www.beatport.com/track/-/1"),
            searching("SoundCloud"),
        ];
        let mut found = resolved(vec![], false);
        found.isrc = Some("GBBBB0000002".into());
        found.isrc_from = Some(IsrcFrom::Shazam);
        assert!(merge_links(&mut e, &found, 900));
        assert_eq!(e.isrc.as_deref(), Some("GBBBB0000002"));
        assert_eq!(e.isrc_from, Some(IsrcFrom::Shazam));
        assert_eq!(e.links_checked_ms, 0, "the refresh rule looks again");
        assert_eq!(
            kept(&e),
            vec![("Apple Music".to_string(), false), ("SoundCloud".to_string(), true)]
        );

        // The same ISRC from Shazam only names where it came from.
        let mut e = entry("One", 1);
        e.isrc = Some("GBAAA0000001".into());
        e.isrc_from = Some(IsrcFrom::Text);
        e.links_checked_ms = 500;
        e.providers = vec![direct("Deezer", "https://www.deezer.com/track/1")];
        let mut found = resolved(vec![], true);
        found.isrc = Some("GBAAA0000001".into());
        found.isrc_from = Some(IsrcFrom::Shazam);
        merge_links(&mut e, &found, 900);
        assert_eq!(e.isrc_from, Some(IsrcFrom::Shazam));
        assert_eq!(e.providers.len(), 1);
        assert_eq!(e.links_checked_ms, 900);
    }

    #[test]
    fn a_text_isrc_never_replaces_one() {
        for from in [IsrcFrom::Shazam, IsrcFrom::Text] {
            let mut e = entry("One", 1);
            e.isrc = Some("GBAAA0000001".into());
            e.isrc_from = Some(from);
            e.providers = vec![direct("Deezer", "https://www.deezer.com/track/1")];
            let mut found = resolved(vec![], false);
            found.isrc = Some("GBBBB0000002".into());
            found.isrc_from = Some(IsrcFrom::Text);
            merge_links(&mut e, &found, 900);
            assert_eq!(e.isrc.as_deref(), Some("GBAAA0000001"));
            assert_eq!(e.isrc_from, Some(from));
            assert_eq!(e.providers.len(), 1);
        }
        // A Shazam ISRC never replaces a Shazam one either.
        let mut e = entry("One", 1);
        e.isrc = Some("GBAAA0000001".into());
        e.isrc_from = Some(IsrcFrom::Shazam);
        let mut found = resolved(vec![], false);
        found.isrc = Some("GBBBB0000002".into());
        found.isrc_from = Some(IsrcFrom::Shazam);
        merge_links(&mut e, &found, 900);
        assert_eq!(e.isrc.as_deref(), Some("GBAAA0000001"));
        // A text ISRC fills a missing one.
        let mut e = entry("One", 1);
        let mut found = resolved(vec![], false);
        found.isrc = Some("gbccc0000003".into());
        found.isrc_from = Some(IsrcFrom::Text);
        merge_links(&mut e, &found, 900);
        assert_eq!(
            (e.isrc.as_deref(), e.isrc_from),
            (Some("GBCCC0000003"), Some(IsrcFrom::Text))
        );
    }

    #[test]
    fn a_refresh_merges_into_the_song_as_it_is_now_or_not_at_all() {
        let dirty = AtomicBool::new(false);
        let found = resolved(vec![direct("Deezer", "https://www.deezer.com/track/1")], true);
        let mut h = history(vec![entry("One", 1)]);
        assert!(!apply_refresh(&mut h, "gone", &found, 5, &dirty));
        assert!(!dirty.load(Ordering::Acquire), "a removed song changes nothing");
        assert!(h.remove(&song_key("A", "One")));
        assert!(!apply_refresh(&mut h, &song_key("A", "One"), &found, 5, &dirty));
        assert!(!dirty.load(Ordering::Acquire), "the Undo slot is not refreshed");

        let mut h = history(vec![entry("One", 1)]);
        h.read_only = true;
        assert!(!apply_refresh(&mut h, &song_key("A", "One"), &found, 5, &dirty));
        assert!(!dirty.load(Ordering::Acquire), "a newer app's file is never marked");
        assert!(h.songs[0].providers.is_empty());
        assert!(refresh_input(&h, &song_key("A", "One")).is_none());

        let mut h = history(vec![entry("One", 1)]);
        assert!(apply_refresh(&mut h, &song_key("A", "One"), &found, 5, &dirty));
        assert!(dirty.load(Ordering::Acquire), "held for the deferred write");
        assert_eq!(h.songs[0].links_checked_ms, 5);
        let input = refresh_input(&h, &song_key("A", "One")).unwrap();
        assert_eq!(input.known, h.songs[0].providers);
    }

    #[test]
    fn one_deferred_write_waits_at_a_time() {
        let pending = AtomicBool::new(false);
        assert!(claim_deferred(&pending));
        assert!(!claim_deferred(&pending), "a second change rides on it");
        pending.store(false, Ordering::Release);
        assert!(claim_deferred(&pending), "after it ran, a new one can wait");
    }

    #[test]
    fn opening_a_song_asks_for_a_refresh_only_when_a_lookup_could_add_a_link() {
        let now = 100 * STALE_LINKS_MS;
        let fresh = now - 1000;
        let stale = now - STALE_LINKS_MS - 1;
        let with = |checked: i64, isrc: bool, providers: Vec<Provider>| {
            let mut e = entry("One", 1);
            e.links_checked_ms = checked;
            e.isrc = isrc.then(|| "GBBBM9200027".to_string());
            e.providers = providers;
            e
        };
        let videos_and_deezer = || {
            vec![
                direct("YouTube Music", "https://music.youtube.com/watch?v=4MOzk7-6j-I"),
                direct("YouTube", "https://www.youtube.com/watch?v=4MOzk7-6j-I"),
                direct("Deezer", "https://www.deezer.com/track/1"),
            ]
        };
        assert!(needs_refresh(&with(0, false, videos_and_deezer()), false, now));
        assert!(!needs_refresh(&with(0, false, vec![]), true, now), "never a newer app's");
        assert!(!needs_refresh(&with(fresh, false, vec![]), false, now));
        assert!(needs_refresh(&with(stale, false, vec![]), false, now));
        assert!(
            !needs_refresh(&with(stale, false, videos_and_deezer()), false, now),
            "Spotify and Tidal need an ISRC; Beatport and SoundCloud never ask"
        );
        assert!(needs_refresh(&with(stale, true, videos_and_deezer()), false, now));
        let mut every = videos_and_deezer();
        every.push(direct("Spotify", "https://open.spotify.com/track/0123456789abcdefghijkl"));
        every.push(direct("Tidal", "https://tidal.com/browse/track/1"));
        every.push(searching("Apple Music"));
        assert!(
            !needs_refresh(&with(stale, true, every), false, now),
            "an Apple search page names no Apple id to fill from"
        );
    }

    #[test]
    fn the_refresh_queue_skips_a_queued_or_running_key_and_is_capped() {
        let mut q = Refreshes::default();
        assert!(q.push("a".into()));
        assert!(!q.push("a".into()), "already queued");
        assert_eq!(q.next().as_deref(), Some("a"));
        assert!(!q.push("a".into()), "already running");
        assert!(q.push("b".into()));
        assert_eq!(q.next().as_deref(), Some("b"));
        assert!(q.push("a".into()), "a finished key can queue again");
        for i in 0..REFRESH_CAP * 2 {
            q.push(format!("k{i}"));
        }
        assert_eq!(q.queued.len(), REFRESH_CAP);
        assert_eq!(q.queued.front().map(String::as_str), Some("a"), "oldest first");
        let _ = q.next();
        assert!(q.next().is_some());
        let mut q = Refreshes::default();
        assert_eq!(q.next(), None);
        assert_eq!(q.running, None);
    }

    #[test]
    fn a_new_song_is_stamped_only_when_its_lookups_were_complete() {
        let mut h = History::default();
        let mut s = song("A", "One");
        s.links_complete = true;
        s.isrc = Some("GBBBM9200027".into());
        s.isrc_from = Some(IsrcFrom::Shazam);
        h.file(&s, None, 77);
        assert_eq!(h.songs[0].links_checked_ms, 77);
        assert_eq!(h.songs[0].isrc_from, Some(IsrcFrom::Shazam));
        let mut s = song("A", "Two");
        s.isrc = Some("bad".into());
        h.file(&s, None, 78);
        assert_eq!(h.songs[0].links_checked_ms, 0);
        assert_eq!((h.songs[0].isrc.clone(), h.songs[0].isrc_from), (None, None));
    }

    #[test]
    fn new_fields_this_build_cannot_read_never_cost_the_song() {
        let e: SongEntry = serde_json::from_str(
            r#"{"key":"a\u001fone","title":"One","artist":"A","isrc":7,"isrc_from":"apple","links_checked_ms":"soon"}"#,
        )
        .unwrap();
        assert_eq!((e.isrc, e.isrc_from, e.links_checked_ms), (None, None, 0));
        let e: SongEntry = serde_json::from_str(
            r#"{"key":"a\u001fone","title":"One","artist":"A","isrc":"GBBBM9200027","isrc_from":"text","links_checked_ms":12}"#,
        )
        .unwrap();
        assert_eq!(e.isrc_from, Some(IsrcFrom::Text));
        assert_eq!(e.links_checked_ms, 12);
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(json["isrc"], "GBBBM9200027");
        assert_eq!(json["isrc_from"], "text");
        assert_eq!(json["links_checked_ms"], 12);
        let bare = serde_json::to_string(&entry("One", 1)).unwrap();
        assert!(
            !bare.contains("isrc") && !bare.contains("links_checked_ms"),
            "unset fields stay out of the file: {bare}"
        );
    }

    #[test]
    fn the_tinted_change_names_its_songs() {
        assert_eq!(
            serde_json::to_value(Change::Tinted {
                keys: vec!["a".into(), "b".into()]
            })
            .unwrap(),
            serde_json::json!({ "kind": "tinted", "keys": ["a", "b"] })
        );
    }
}
