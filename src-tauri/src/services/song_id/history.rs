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
//! atomic writer (flushed, then renamed) right after each change. A song is
//! found a few times an hour at most, so there is nothing to debounce, and a
//! phone killed in the background loses nothing. Read on first use and let go
//! of again `IDLE_UNLOAD` after the last, so the app holds no songs while
//! nobody looks. A file that is there but cannot be read is never written
//! over; one that does not parse is moved aside as `song_history.json.bad-<ms>`.
//!
//! Versions: a field added later must be optional, so every app reads every
//! file of `FILE_VERSION`. A file whose `version` is higher was written by a
//! newer StreamNook: this one shows what it can read of it and never writes
//! over it or moves it aside, so going back to an older build and forward
//! again loses nothing.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::Emitter;

use super::services;
use super::shazam::{Provider, SongMatch};
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
/// host, so no link is ever drawn with a logo it does not go to.
fn web_providers(providers: &[Provider]) -> Vec<Provider> {
    providers
        .iter()
        .filter(|p| is_web_url(&p.url) && services::is_link_to(p.name.trim(), &p.url))
        .map(|p| Provider {
            name: p.name.trim().to_string(),
            url: p.url.clone(),
        })
        .collect()
}

/// A match as the history keeps it: bounded text and web links only.
fn kept_match(song: &SongMatch) -> SongMatch {
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
        providers: web_providers(&song.providers),
        seed_url: None,
        key: None,
        links_are_searches: song.links_are_searches,
    }
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
    pub providers: Vec<Provider>,
    /// `providers` are search pages, replaced by the first real links found.
    pub links_are_searches: bool,
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
    /// found.
    fn refresh_from(&mut self, song: &SongMatch) {
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
        // Search pages never replace a track's own links; real links replace
        // search pages.
        if !song.providers.is_empty() && (!song.links_are_searches || self.links_are_searches) {
            self.providers = song.providers.clone();
            self.links_are_searches = song.links_are_searches;
        }
    }

    /// A stored row made safe to show: web links only, known platforms only,
    /// places newest first and bounded, and its search words built.
    fn tidy(&mut self) {
        self.album_art = web_url(&self.album_art);
        self.shazam_url = service_url(&self.shazam_url, "Shazam");
        self.song_link = service_url(&self.song_link, "song.link");
        self.providers = web_providers(&self.providers);
        for sighting in &mut self.sightings {
            sighting.source = sighting.source.take().and_then(SongSource::cleaned);
        }
        self.sightings.sort_by(|a, b| b.at_ms.cmp(&a.at_ms));
        self.sightings.truncate(MAX_SIGHTINGS);
        self.count = self.count.max(1);
        self.haystack = haystack_of(self);
    }

    fn row(&self) -> SongRow {
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
        }
    }
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
}

/// What changed, for a window showing the list or a song.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Change {
    Filed { key: String },
    Removed { key: String },
    Cleared,
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
            entry.refresh_from(&song);
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

    fn page(&self, query: &str, offset: usize, limit: usize) -> SongPage {
        let terms = terms_of(query);
        let limit = limit.clamp(1, MAX_PAGE);
        let mut matched = 0;
        let mut rows = Vec::new();
        for song in &self.songs {
            if !terms.iter().all(|t| song.haystack.contains(t.as_str())) {
                continue;
            }
            if matched >= offset && rows.len() < limit {
                rows.push(song.row());
            }
            matched += 1;
        }
        let (mixed_platforms, channels) = self.places();
        SongPage {
            rows,
            matched,
            total: self.songs.len(),
            mixed_platforms,
            channels,
        }
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

/// The song's identity in the history: artist and title folded (`fold`), so
/// the same song matched twice is one row.
pub fn song_key(artist: &str, title: &str) -> String {
    format!("{}\u{1f}{}", fold(artist), fold(title))
}

/// Text as the history compares it: lowercase with Latin accents folded
/// (`fold_search`), apostrophes dropped so "Don't", "Don’t" and "dont" are one
/// word, and every run of spacing made one space.
fn fold(s: &str) -> String {
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
    path: PathBuf,
    used: Instant,
}

static STORE: Mutex<Option<Loaded>> = Mutex::new(None);
/// A change is in memory that the last write did not get to disk.
static DIRTY: AtomicBool = AtomicBool::new(false);
/// The unloader task is running.
static UNLOADER: AtomicBool = AtomicBool::new(false);

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

fn with_history<R>(f: impl FnOnce(&mut History, &Path) -> R) -> Result<R, String> {
    let out = {
        let mut guard = lock_store();
        if guard.is_none() {
            let path = get_app_data_dir()
                .map_err(|e| format!("Failed to resolve app data dir: {e}"))?
                .join(FILE);
            sweep_temps(&path);
            let history = History::from_stored(read_file(&path)?);
            if history.read_only {
                log::warn!(
                    "[SongHistory] {} was saved by a newer StreamNook; shown, not changed",
                    path.display()
                );
            }
            *guard = Some(Loaded {
                history,
                path,
                used: Instant::now(),
            });
        }
        let loaded = guard.as_mut().expect("seeded above");
        loaded.used = Instant::now();
        f(&mut loaded.history, &loaded.path)
    };
    schedule_unload();
    Ok(out)
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

/// File a match heard now on `source`. Returns the song's key.
pub fn record(
    app: &AppHandle,
    song: &SongMatch,
    source: Option<SongSource>,
) -> Result<String, String> {
    let at_ms = now_ms();
    let key = change(|h| (h.file(song, source, at_ms), true))?;
    let _ = app.emit(CHANGED_EVENT, Change::Filed { key: key.clone() });
    Ok(key)
}

/// The rows matching every word of `query`, from `offset`.
pub fn page(query: &str, offset: usize, limit: usize) -> Result<SongPage, String> {
    with_history(|h, _| h.page(query, offset, limit))
}

/// One song, by key.
pub fn get(key: &str) -> Result<Option<SongEntry>, String> {
    with_history(|h, _| h.songs.iter().find(|s| s.key == key).cloned())
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

/// Forget every song.
pub fn clear(app: &AppHandle) -> Result<(), String> {
    change(|h| {
        h.songs.clear();
        h.removed = None;
        ((), true)
    })?;
    let _ = app.emit(CHANGED_EVENT, Change::Cleared);
    Ok(())
}

/// Write a change an earlier write did not get to disk. The exit path calls
/// it; a no-op when nothing is pending.
pub fn flush_now() -> Result<(), String> {
    if !DIRTY.load(Ordering::Acquire) {
        return Ok(());
    }
    let guard = lock_store();
    let Some(loaded) = guard.as_ref() else {
        return Ok(());
    };
    persist(&loaded.history, &loaded.path)
}

/// One unloader pass over `slot`: let the songs go once they sat unused for
/// `idle_after` with nothing left to write. `write` is the disk write
/// (`write_file`), passed in so a test can fail it. Returns how long to wait
/// before the next pass, or None once nothing is held.
fn unload_pass(
    slot: &mut Option<Loaded>,
    idle_after: Duration,
    dirty: &AtomicBool,
    write: impl Fn(&Path, &History) -> Result<(), String>,
) -> Option<Duration> {
    let idle = slot.as_ref()?.used.elapsed();
    if idle < idle_after {
        return Some(idle_after - idle);
    }
    if dirty.load(Ordering::Acquire) {
        if let Some(loaded) = slot.as_ref() {
            if write(&loaded.path, &loaded.history).is_ok() {
                dirty.store(false, Ordering::Release);
            }
        }
        if dirty.load(Ordering::Acquire) {
            // Keep the songs until a write lands; try again in a while.
            return Some(IDLE_UNLOAD);
        }
    }
    *slot = None;
    None
}

fn unload_step() -> Option<Duration> {
    let mut guard = lock_store();
    unload_pass(&mut guard, IDLE_UNLOAD, &DIRTY, write_file)
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
            seed_url: None,
            key: None,
            links_are_searches: false,
        }
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
        first.providers = vec![Provider {
            name: "Spotify".into(),
            url: "https://open.spotify.com/track/1".into(),
        }];
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
        real.providers = vec![Provider {
            name: "Spotify".into(),
            url: "https://open.spotify.com/track/1".into(),
        }];
        let mut search = song("A", "One");
        search.providers = vec![Provider {
            name: "Spotify".into(),
            url: "https://open.spotify.com/search/A%20One".into(),
        }];
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
            Provider {
                name: "Spotify".into(),
                url: "https://open.spotify.com/track/1".into(),
            },
            Provider {
                name: "Evil".into(),
                url: "javascript:alert(1)".into(),
            },
            Provider {
                name: " ".into(),
                url: "https://example.com".into(),
            },
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
        let link = |name: &str, url: &str| Provider {
            name: name.into(),
            url: url.into(),
        };
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
        e.providers = vec![Provider {
            name: "X".into(),
            url: "ms-settings:".into(),
        }];
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
            path: PathBuf::from("unused"),
            used,
        })
    }

    #[test]
    fn the_unloader_waits_out_the_idle_time_then_lets_go() {
        let dirty = AtomicBool::new(false);
        let ok = |_: &Path, _: &History| Ok(());
        let mut slot = held(Instant::now());
        assert!(unload_pass(&mut slot, Duration::from_secs(60), &dirty, ok).is_some());
        assert!(slot.is_some());
        assert_eq!(unload_pass(&mut slot, Duration::ZERO, &dirty, ok), None);
        assert!(slot.is_none());
        assert_eq!(
            unload_pass(&mut slot, Duration::ZERO, &dirty, ok),
            None,
            "nothing held"
        );
    }

    #[test]
    fn the_unloader_keeps_songs_a_write_has_not_saved() {
        let dirty = AtomicBool::new(true);
        let mut slot = held(Instant::now());
        let fail = |_: &Path, _: &History| Err("disk full".to_string());
        assert_eq!(
            unload_pass(&mut slot, Duration::ZERO, &dirty, fail),
            Some(IDLE_UNLOAD)
        );
        assert!(slot.is_some() && dirty.load(Ordering::Acquire));
        let ok = |_: &Path, _: &History| Ok(());
        assert_eq!(unload_pass(&mut slot, Duration::ZERO, &dirty, ok), None);
        assert!(slot.is_none() && !dirty.load(Ordering::Acquire));
    }
}
