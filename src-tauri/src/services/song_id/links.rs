//! The song link finder: a song's own page on each music service, found from
//! its ISRC and Apple Music id, and a search page on every service where no
//! page was found.
//!
//! Where each link comes from:
//! * Apple Music: Shazam's hub link, else the song page for its Apple id.
//! * Deezer: its API by ISRC, else a text search that names exactly one
//!   recording. Deezer also gives the track's length (the reference the
//!   YouTube Music match is held to), a cover when Shazam had none, and on the
//!   text path an ISRC.
//! * YouTube Music and YouTube: YouTube Music's own web search (anonymous,
//!   songs only), matched strictly on title, first artist, version words and
//!   length. The same video is the YouTube link.
//! * Beatport: its search page, taken only on an equal ISRC.
//! * Spotify and Tidal: streamnook.app's song-links function by ISRC (both need
//!   a secret the app never holds).
//!
//! Every lookup fails closed to a search link. Each host keeps a minimum gap
//! between calls and a breaker that opens for five minutes after three
//! failures in a row. A resolve is `complete` only when every lookup that
//! applied answered cleanly (found, a clean miss, or streamnook.app not
//! offering it yet), so a song cut short by its time budget or by a host that
//! is down is looked up again later (history.rs decides when).

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use url::Url;

use super::history::fold;
use super::services;
use super::shazam::{IsrcFrom, Provider};

/// The services a song's links are shown for, in their order.
pub const DISPLAY: &[&str] = &[
    "Spotify",
    "Apple Music",
    "YouTube Music",
    "YouTube",
    "Deezer",
    "Tidal",
    "Beatport",
    "SoundCloud",
];

/// The services whose direct link the finder makes from the ISRC or the
/// song's words. When Shazam's ISRC replaces one a text search found, these
/// go (history.rs `merge_links`); Shazam's own links and Apple Music stay.
pub const RESOLVER_MADE: &[&str] = &[
    "Deezer",
    "YouTube Music",
    "YouTube",
    "Beatport",
    "Spotify",
    "Tidal",
];

/// A full desktop Chrome user agent. YouTube Music's web search and
/// Beatport's pages answer a browser; Beatport refuses plain agents (403).
pub const CHROME_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36";
/// The YouTube Music web client the search speaks as.
const YTM_CLIENT_VERSION: &str = "1.20251020.01.00";
/// YouTube Music's "Songs" search filter.
const YTM_SONGS_ONLY: &str = "EgWKAQIIAWoKEAkQBRAKEAMQBA==";
/// Two lengths this close are one recording's.
const SAME_LENGTH_SECS: u32 = 5;
/// The largest answer read from any host.
const MAX_BODY: usize = 4 * 1024 * 1024;
/// A call that takes longer than this has failed (the time budget usually
/// ends the wait first, which is no failure).
const CALL_TIMEOUT: Duration = Duration::from_secs(10);
const FAILURES_TO_OPEN: u32 = 3;
const OPEN_FOR: Duration = Duration::from_secs(5 * 60);
/// Deezer text search hits read.
const DEEZER_HITS: u32 = 10;

const JSON_HEADERS: &[(&str, &str)] = &[("Accept", "application/json")];
const YTM_HEADERS: &[(&str, &str)] = &[
    ("Accept", "*/*"),
    ("Accept-Language", "en-US,en;q=0.9"),
    ("Origin", "https://music.youtube.com"),
    ("Referer", "https://music.youtube.com/"),
];
const PAGE_HEADERS: &[(&str, &str)] = &[
    (
        "Accept",
        "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
    ),
    ("Accept-Language", "en-US,en;q=0.9"),
];

/// What a resolve starts from: the song as named, what is known about its
/// identity, and the links it already has (Shazam's hub, or the kept song's).
#[derive(Debug, Clone, PartialEq)]
pub struct ResolveInput {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub isrc: Option<String>,
    pub isrc_from: Option<IsrcFrom>,
    pub apple_id: Option<String>,
    pub known: Vec<Provider>,
}

/// What a resolve found: every link (direct first, in `DISPLAY` order, then
/// search pages, then other services kept before), the ISRC and its source,
/// a cover from Deezer, and whether every lookup answered cleanly.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub providers: Vec<Provider>,
    pub isrc: Option<String>,
    pub isrc_from: Option<IsrcFrom>,
    pub cover: Option<String>,
    pub complete: bool,
}

/// Find the song's links for the refresh runner. Half of `budget` goes to
/// Deezer, half to the lookups that run together after it (YouTube Music,
/// Beatport, streamnook.app); each lookup's share starts once its host's gate
/// lets it through, so waiting for a gate never eats it.
pub async fn resolve(input: &ResolveInput, budget: Duration) -> Resolved {
    resolve_on(&NET, input, Budget::PerLookup(budget)).await
}

/// Find the song's links at identify time: everything ends `budget` from
/// now, gate waits included, so the identify answer never waits past it
/// (Deezer has until half of it). Lookups cut short are finished by a
/// refresh after the song is filed.
pub async fn resolve_within(input: &ResolveInput, budget: Duration) -> Resolved {
    resolve_on(&NET, input, Budget::Total(budget)).await
}

/// How a resolve spends its time.
#[derive(Debug, Clone, Copy)]
enum Budget {
    /// Half each to Deezer and to the lookups after it, counted from the
    /// moment each host's gate lets it through.
    PerLookup(Duration),
    /// One end for everything, this long from now.
    Total(Duration),
}

impl From<Duration> for Budget {
    fn from(budget: Duration) -> Self {
        Budget::PerLookup(budget)
    }
}

/// How long one lookup may take.
#[derive(Debug, Clone, Copy)]
enum Time {
    /// This long from the moment its host's gate lets it through.
    Share(Duration),
    /// Until this moment, its gate wait included.
    Until(tokio::time::Instant),
}

impl From<Duration> for Time {
    fn from(share: Duration) -> Self {
        Time::Share(share)
    }
}

// ------------------------------------------------------------------ links

/// An ISRC as kept: 2 letters, 3 letters or digits, 7 digits, uppercase.
pub fn valid_isrc(raw: &str) -> Option<String> {
    let code = raw.trim().to_ascii_uppercase();
    let b = code.as_bytes();
    let ok = b.len() == 12
        && b[..2].iter().all(u8::is_ascii_uppercase)
        && b[2..5]
            .iter()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        && b[5..].iter().all(u8::is_ascii_digit);
    ok.then_some(code)
}

/// An Apple Music song id: 1 to 12 digits.
pub fn valid_apple_id(raw: &str) -> Option<String> {
    let id = raw.trim();
    (!id.is_empty() && id.len() <= 12 && id.bytes().all(|b| b.is_ascii_digit()))
        .then(|| id.to_string())
}

/// A YouTube video id: 11 of letters, digits, `_` and `-`.
fn valid_youtube_id(raw: &str) -> Option<String> {
    (raw.len() == 11
        && raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
    .then(|| raw.to_string())
}

/// A numeric id (Deezer's, Beatport's), from a JSON number or digit string.
fn number_id(v: &Value) -> Option<String> {
    match v {
        Value::Number(n) => n.as_u64().map(|n| n.to_string()),
        Value::String(s) => {
            (!s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()))
                .then(|| s.clone())
        }
        _ => None,
    }
}

fn is_https_web(url: &str) -> bool {
    url.len() <= 2048
        && Url::parse(url).is_ok_and(|u| u.scheme() == "https" && u.host_str().is_some())
}

/// A Spotify track page, as streamnook.app should send it.
fn spotify_track_url(raw: &str) -> Option<String> {
    let url = Url::parse(raw).ok()?;
    if url.scheme() != "https" || url.host_str() != Some("open.spotify.com") || url.port().is_some()
    {
        return None;
    }
    let mut parts = url.path_segments()?;
    let (Some("track"), Some(id), None) = (parts.next(), parts.next(), parts.next()) else {
        return None;
    };
    (id.len() == 22 && id.bytes().all(|b| b.is_ascii_alphanumeric()))
        .then(|| format!("https://open.spotify.com/track/{id}"))
}

/// A Tidal track page, as streamnook.app should send it.
fn tidal_track_url(raw: &str) -> Option<String> {
    let url = Url::parse(raw).ok()?;
    if url.scheme() != "https" || url.host_str() != Some("tidal.com") || url.port().is_some() {
        return None;
    }
    let mut parts = url.path_segments()?;
    let (Some("browse"), Some("track"), Some(id), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    number_id(&Value::String(id.to_string()))
        .map(|id| format!("https://tidal.com/browse/track/{id}"))
}

/// The most a search's words take in its address, encoded. A title in a
/// script that encodes to 9 bytes a character (CJK) would otherwise make an
/// address too long to show as a QR code, and no search needs more.
const SEARCH_WORDS_MAX: usize = 200;

/// `words` encoded for an address, cut to `SEARCH_WORDS_MAX` bytes between
/// two characters (never inside one's escapes), with no space left at the cut.
fn encoded_words(words: &str) -> String {
    let mut out = String::new();
    let mut buf = [0u8; 4];
    for ch in words.trim().chars() {
        let piece = urlencoding::encode(ch.encode_utf8(&mut buf));
        if out.len() + piece.len() > SEARCH_WORDS_MAX {
            break;
        }
        out.push_str(&piece);
    }
    while out.ends_with("%20") {
        out.truncate(out.len() - 3);
    }
    out
}

/// The search page for the song's words on `service`, for every service on
/// `DISPLAY` but Beatport (a search there is noise for most songs).
pub fn search_link(service: &str, artist: &str, title: &str) -> Option<Provider> {
    let q = encoded_words(&format!("{artist} {title}"));
    let url = match service {
        "Spotify" => format!("https://open.spotify.com/search/{q}"),
        "Apple Music" => format!("https://music.apple.com/us/search?term={q}"),
        "YouTube Music" => format!("https://music.youtube.com/search?q={q}"),
        "YouTube" => format!("https://www.youtube.com/results?search_query={q}"),
        "Deezer" => format!("https://www.deezer.com/search/{q}"),
        "Tidal" => format!("https://tidal.com/search?q={q}"),
        "SoundCloud" => format!("https://soundcloud.com/search?q={q}"),
        _ => return None,
    };
    Some(Provider {
        name: service.to_string(),
        url,
        search: true,
    })
}

/// The Apple Music song id an Apple Music link names: its `?i=` (an album
/// page opened at the song) or the id after `/song/`.
pub fn apple_id_of(url: &str) -> Option<String> {
    if !services::is_link_to("Apple Music", url) || services::is_search_page(url) {
        return None;
    }
    let parsed = Url::parse(url).ok()?;
    if let Some((_, id)) = parsed.query_pairs().find(|(k, _)| k == "i") {
        return valid_apple_id(&id);
    }
    let parts: Vec<&str> = parsed.path_segments()?.collect();
    let at = parts.iter().position(|p| *p == "song")?;
    parts[at + 1..].iter().rev().find_map(|p| valid_apple_id(p))
}

/// Direct links first in `DISPLAY` order, then search pages in `DISPLAY`
/// order, then services off that list (Amazon Music and Pandora from songs
/// kept before) in the order they were kept.
pub fn sort_providers(p: &mut [Provider]) {
    p.sort_by_key(|x| match DISPLAY.iter().position(|d| *d == x.name) {
        Some(i) if !x.search => (0, i),
        Some(i) => (1, i),
        None => (2, 0),
    });
}

/// The services a lookup can give a direct link for, for a song with (or
/// without) an ISRC and a known Apple id. Beatport and SoundCloud are never
/// among them: Beatport is filled only on an equal ISRC, SoundCloud never.
pub fn fillable(has_isrc: bool, apple_id_known: bool) -> Vec<&'static str> {
    DISPLAY
        .iter()
        .copied()
        .filter(|service| match *service {
            "YouTube Music" | "YouTube" | "Deezer" => true,
            "Spotify" | "Tidal" => has_isrc,
            "Apple Music" => apple_id_known,
            _ => false,
        })
        .collect()
}

/// Put `link` in its service's slot: a direct link fills a missing or search
/// slot and never replaces another direct link; a search page only fills a
/// missing slot.
pub fn put(providers: &mut Vec<Provider>, link: Provider) {
    match providers.iter().position(|p| p.name == link.name) {
        Some(i) if !providers[i].search => {}
        Some(i) => {
            if !link.search {
                providers[i] = link;
            }
        }
        None => providers.push(link),
    }
}

fn has_direct(providers: &[Provider], service: &str) -> bool {
    providers.iter().any(|p| p.name == service && !p.search)
}

fn direct(service: &str, url: String) -> Provider {
    Provider {
        name: service.to_string(),
        url,
        search: false,
    }
}

// --------------------------------------------------------------- matching

/// Words of `s`: runs of letters and digits.
fn words(s: &str) -> Vec<&str> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect()
}

/// Text compared whole: folded (history.rs `fold`), "&" read as "and",
/// punctuation dropped.
fn plain(raw: &str) -> String {
    words(&fold(raw).replace('&', " and ")).join(" ")
}

/// The first credited name of an artist line. " x " splits last, inside the
/// first name only, so "Lil Nas X & Billy Ray Cyrus" stays "lil nas x".
fn primary_artist(raw: &str) -> String {
    let s = fold(raw);
    let cut = [",", "&", " and ", " feat", " ft.", " ft "]
        .iter()
        .filter_map(|sep| s.find(sep))
        .min()
        .unwrap_or(s.len());
    let first = s[..cut].trim();
    words(first.split(" x ").next().unwrap_or(first)).join(" ")
}

/// A title as matching compares it: its words without credits ("feat. X",
/// "(with X)") and without the bracketed or dashed parts that name a
/// version, and that version's words as a set.
#[derive(Debug, PartialEq, Eq)]
struct Title {
    core: String,
    versions: BTreeSet<&'static str>,
}

/// Words that start a credit inside a title.
const CREDIT: &[&str] = &["feat", "ft", "featuring"];
/// Words that start a credit in a bracketed or dashed part.
const ASIDE_CREDIT: &[&str] = &["feat", "ft", "featuring", "with", "prod", "produced"];

#[derive(Clone, Copy, PartialEq)]
enum PartKind {
    /// The title itself (and plain text after a bracket).
    Head,
    /// A bracketed part, or one after " - ".
    Aside,
}

/// The title split into its head and its asides.
fn title_parts(text: &str) -> Vec<(PartKind, String)> {
    let text: String = text
        .chars()
        .map(|c| {
            if c == '\u{2013}' || c == '\u{2014}' {
                '-'
            } else {
                c
            }
        })
        .collect();
    let mut parts = Vec::new();
    let mut plain = String::new();
    let mut group = String::new();
    let mut depth = 0usize;
    let mut dashed = false;
    let flush = |plain: &mut String, dashed: &mut bool, parts: &mut Vec<(PartKind, String)>| {
        let text = std::mem::take(plain);
        let mut pieces = text.split(" - ");
        if let Some(first) = pieces.next() {
            let kind = if *dashed {
                PartKind::Aside
            } else {
                PartKind::Head
            };
            parts.push((kind, first.to_string()));
        }
        for piece in pieces {
            *dashed = true;
            parts.push((PartKind::Aside, piece.to_string()));
        }
    };
    for c in text.chars() {
        match c {
            '(' | '[' | '{' => {
                if depth == 0 {
                    flush(&mut plain, &mut dashed, &mut parts);
                } else {
                    group.push(' ');
                }
                depth += 1;
            }
            ')' | ']' | '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    parts.push((PartKind::Aside, std::mem::take(&mut group)));
                } else {
                    group.push(' ');
                }
            }
            _ if depth > 0 => group.push(c),
            _ => plain.push(c),
        }
    }
    if depth > 0 {
        parts.push((PartKind::Aside, group));
    }
    flush(&mut plain, &mut dashed, &mut parts);
    parts
}

/// The version words in `ws`, as one set: "radio edit" and "sped up" are
/// one word each, any "remaster..." is "remaster".
fn version_words(ws: &[&str], out: &mut BTreeSet<&'static str>) {
    let mut i = 0;
    while i < ws.len() {
        let next = ws.get(i + 1).copied();
        let found = match (ws[i], next) {
            ("radio", Some("edit")) => {
                i += 1;
                Some("radio edit")
            }
            ("sped", Some("up")) => {
                i += 1;
                Some("sped up")
            }
            ("live", _) => Some("live"),
            ("remix", _) => Some("remix"),
            ("edit", _) => Some("edit"),
            ("instrumental", _) => Some("instrumental"),
            ("acoustic", _) => Some("acoustic"),
            ("slowed", _) => Some("slowed"),
            ("karaoke", _) => Some("karaoke"),
            ("mix", _) => Some("mix"),
            (w, _) if w.starts_with("remaster") => Some("remaster"),
            _ => None,
        };
        if let Some(word) = found {
            out.insert(word);
        }
        i += 1;
    }
}

fn title_of(raw: &str) -> Title {
    let text = fold(raw).replace('&', " and ");
    let mut core: Vec<String> = Vec::new();
    let mut versions = BTreeSet::new();
    for (kind, part) in title_parts(&text) {
        let mut ws = words(&part);
        if ws.is_empty() {
            continue;
        }
        match kind {
            PartKind::Head => {
                if let Some(at) = ws.iter().skip(1).position(|w| CREDIT.contains(w)) {
                    ws.truncate(at + 1);
                }
                version_words(&ws, &mut versions);
                core.extend(ws.iter().map(|w| w.to_string()));
            }
            PartKind::Aside => {
                if ASIDE_CREDIT.contains(&ws[0]) {
                    continue;
                }
                let mut own = BTreeSet::new();
                version_words(&ws, &mut own);
                if own.is_empty() {
                    core.extend(ws.iter().map(|w| w.to_string()));
                } else {
                    versions.extend(own);
                }
            }
        }
    }
    Title {
        core: core.join(" "),
        versions,
    }
}

/// Whether a hit names our song: the same title (core and version words) and
/// the same first artist.
fn same_song(ours: &Title, our_artist: &str, title: &str, artist: &str) -> bool {
    let first = primary_artist(our_artist);
    !ours.core.is_empty()
        && !first.is_empty()
        && *ours == title_of(title)
        && first == primary_artist(artist)
}

// ----------------------------------------------------------------- Deezer

#[derive(Debug, Clone, PartialEq)]
struct DeezerTrack {
    id: String,
    title: String,
    artist: String,
    album: Option<String>,
    secs: Option<u32>,
    rank: u64,
    cover: Option<String>,
    isrc: Option<String>,
}

fn deezer_track(v: &Value) -> Option<DeezerTrack> {
    Some(DeezerTrack {
        id: number_id(v.get("id")?)?,
        title: v.get("title")?.as_str()?.to_string(),
        artist: v
            .pointer("/artist/name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        album: v
            .pointer("/album/title")
            .and_then(Value::as_str)
            .map(str::to_string),
        secs: v
            .get("duration")
            .and_then(Value::as_u64)
            .filter(|s| *s > 0)
            .and_then(|s| u32::try_from(s).ok()),
        rank: v.get("rank").and_then(Value::as_u64).unwrap_or(0),
        cover: v
            .pointer("/album/cover_xl")
            .and_then(Value::as_str)
            .filter(|u| is_https_web(u))
            .map(str::to_string),
        isrc: v.get("isrc").and_then(Value::as_str).and_then(valid_isrc),
    })
}

/// Deezer's error answer (HTTP 200 with an `error` object): "no data" (800)
/// and anything final is a clean miss; a refusal for quota (4) or a busy
/// service (700) is a failure.
fn deezer_error(answer: &Value) -> Option<Parsed<DeezerTrack>> {
    let error = answer.get("error")?;
    match error.get("code").and_then(Value::as_u64) {
        Some(4) | Some(700) => Some(Parsed::Bad),
        _ => Some(Parsed::Miss),
    }
}

fn parse_deezer_track(reply: &Reply) -> Parsed<DeezerTrack> {
    let Some(answer) = reply.json() else {
        return Parsed::Bad;
    };
    if let Some(error) = deezer_error(&answer) {
        return error;
    }
    deezer_track(&answer).map_or(Parsed::Bad, Parsed::Found)
}

fn parse_deezer_search(reply: &Reply, input: &ResolveInput) -> Parsed<DeezerTrack> {
    let Some(answer) = reply.json() else {
        return Parsed::Bad;
    };
    if let Some(error) = deezer_error(&answer) {
        return error;
    }
    let Some(data) = answer.get("data").and_then(Value::as_array) else {
        return Parsed::Bad;
    };
    let hits: Vec<DeezerTrack> = data.iter().filter_map(deezer_track).collect();
    pick_deezer(&hits, input).map_or(Parsed::Miss, Parsed::Found)
}

/// The one text search hit that is our song: the only exact one; else the
/// only one on our album; else, when the candidates all last within
/// `SAME_LENGTH_SECS` of each other, the highest ranked. Anything less clear
/// takes nothing, since a guess would write the song's identity.
fn pick_deezer(hits: &[DeezerTrack], input: &ResolveInput) -> Option<DeezerTrack> {
    let ours = title_of(&input.title);
    let exact: Vec<&DeezerTrack> = hits
        .iter()
        .filter(|h| same_song(&ours, &input.artist, &h.title, &h.artist))
        .collect();
    if exact.len() <= 1 {
        return exact.first().map(|h| (*h).clone());
    }
    let album = input.album.as_deref().map(plain).filter(|a| !a.is_empty());
    let on_album: Vec<&DeezerTrack> = exact
        .iter()
        .copied()
        .filter(|h| album.is_some() && h.album.as_deref().map(plain) == album)
        .collect();
    let pool = if on_album.is_empty() { exact } else { on_album };
    if pool.len() == 1 {
        return Some(pool[0].clone());
    }
    let lengths: Vec<u32> = pool.iter().map(|h| h.secs).collect::<Option<_>>()?;
    let (shortest, longest) = (lengths.iter().min()?, lengths.iter().max()?);
    if longest - shortest > SAME_LENGTH_SECS {
        return None;
    }
    pool.into_iter()
        .reduce(|best, h| if h.rank > best.rank { h } else { best })
        .cloned()
}

// ---------------------------------------------------------- YouTube Music

#[derive(Debug, Clone, PartialEq)]
struct YtItem {
    id: String,
    title: String,
    artist: String,
    secs: Option<u32>,
}

/// "5:50" or "1:02:03" in seconds.
fn clock_secs(s: &str) -> Option<u32> {
    let parts: Vec<&str> = s.split(':').collect();
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    let mut total: u32 = 0;
    for part in parts {
        if part.is_empty() || part.len() > 2 || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        total = total.checked_mul(60)?.checked_add(part.parse().ok()?)?;
    }
    Some(total)
}

/// The first object under `key`, depth first, at most `depth` levels down.
fn first_key<'a>(v: &'a Value, key: &str, depth: usize) -> Option<&'a Value> {
    if depth == 0 {
        return None;
    }
    match v {
        Value::Object(map) => map
            .get(key)
            .or_else(|| map.values().find_map(|x| first_key(x, key, depth - 1))),
        Value::Array(items) => items.iter().find_map(|x| first_key(x, key, depth - 1)),
        _ => None,
    }
}

/// The songs of a search answer: None when it is no search answer at all,
/// empty when it found nothing.
fn ytm_items(answer: &Value) -> Option<Vec<YtItem>> {
    let results = answer.pointer("/contents/tabbedSearchResultsRenderer")?;
    let Some(shelf) = first_key(results, "musicShelfRenderer", 12) else {
        return Some(Vec::new());
    };
    Some(
        shelf
            .get("contents")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|i| i.get("musicResponsiveListItemRenderer"))
                    .filter_map(ytm_item)
                    .collect()
            })
            .unwrap_or_default(),
    )
}

/// One song row: its title (first column), "Artist • Album • 5:50" (second
/// column) and its video id.
fn ytm_item(row: &Value) -> Option<YtItem> {
    let columns = row.get("flexColumns")?.as_array()?;
    let text = |i: usize| -> Option<String> {
        let runs = columns
            .get(i)?
            .pointer("/musicResponsiveListItemFlexColumnRenderer/text/runs")?
            .as_array()?;
        Some(
            runs.iter()
                .filter_map(|run| run.get("text").and_then(Value::as_str))
                .collect(),
        )
    };
    let title = text(0)?;
    let line = text(1)?;
    let mut parts: Vec<&str> = line.split(" \u{2022} ").map(str::trim).collect();
    if parts.len() >= 4 && parts[0].eq_ignore_ascii_case("song") {
        parts.remove(0);
    }
    let secs = if parts.len() >= 2 {
        parts.last().and_then(|p| clock_secs(p))
    } else {
        None
    };
    let id = row
        .pointer("/playlistItemData/videoId")
        .and_then(Value::as_str)
        .or_else(|| {
            columns
                .first()?
                .pointer("/musicResponsiveListItemFlexColumnRenderer/text/runs/0/navigationEndpoint/watchEndpoint/videoId")?
                .as_str()
        })
        .and_then(valid_youtube_id)?;
    Some(YtItem {
        id,
        title,
        artist: parts.first()?.to_string(),
        secs,
    })
}

/// The row that is our song. With a reference length: the first that is our
/// song by title, first artist and version words, and lasts within
/// `SAME_LENGTH_SECS` of it when it shows a length. Without one: the only
/// row whose whole title and artist read exactly as ours.
fn pick_video<'a>(
    items: &'a [YtItem],
    input: &ResolveInput,
    reference: Option<u32>,
) -> Option<&'a YtItem> {
    match reference {
        Some(want) => {
            let ours = title_of(&input.title);
            items.iter().find(|it| {
                same_song(&ours, &input.artist, &it.title, &it.artist)
                    && it.secs.is_none_or(|s| s.abs_diff(want) <= SAME_LENGTH_SECS)
            })
        }
        None => {
            let (title, artist) = (plain(&input.title), plain(&input.artist));
            if title.is_empty() || artist.is_empty() {
                return None;
            }
            let mut exact = items
                .iter()
                .filter(|it| plain(&it.title) == title && plain(&it.artist) == artist);
            let first = exact.next()?;
            exact.next().is_none().then_some(first)
        }
    }
}

fn parse_ytm(reply: &Reply, input: &ResolveInput, reference: Option<u32>) -> Parsed<String> {
    let Some(answer) = reply.json() else {
        return Parsed::Bad;
    };
    let Some(items) = ytm_items(&answer) else {
        return Parsed::Bad;
    };
    pick_video(&items, input, reference).map_or(Parsed::Miss, |it| Parsed::Found(it.id.clone()))
}

// --------------------------------------------------------------- Beatport

/// The JSON a Next.js page carries in its `__NEXT_DATA__` script.
fn next_data(html: &str) -> Option<Value> {
    let at = html.find("id=\"__NEXT_DATA__\"")?;
    let open = at + html[at..].find('>')? + 1;
    let close = open + html[open..].find("</script>")?;
    serde_json::from_str(&html[open..close]).ok()
}

fn parse_beatport(reply: &Reply, isrc: &str) -> Parsed<String> {
    if !reply.ok() {
        return Parsed::Bad;
    }
    let html = String::from_utf8_lossy(&reply.body);
    let Some(data) = next_data(&html) else {
        return Parsed::Bad;
    };
    let tracks = data
        .pointer("/props/pageProps/dehydratedState/queries")
        .and_then(Value::as_array)
        .and_then(|queries| {
            queries
                .iter()
                .find(|q| q.pointer("/queryKey/0").and_then(Value::as_str) == Some("search-tracks"))
        })
        .and_then(|q| q.pointer("/state/data/data"))
        .and_then(Value::as_array);
    let Some(tracks) = tracks else {
        return Parsed::Bad;
    };
    tracks
        .iter()
        .find(|t| {
            t.get("isrc")
                .and_then(Value::as_str)
                .and_then(valid_isrc)
                .as_deref()
                == Some(isrc)
        })
        .and_then(|t| t.get("track_id").and_then(number_id))
        .map_or(Parsed::Miss, |id| {
            Parsed::Found(format!("https://www.beatport.com/track/-/{id}"))
        })
}

// --------------------------------------------------------- streamnook.app

/// The site's answer: each service's link when found, and whether both
/// answers were final (`found`, `miss` or `unconfigured`).
#[derive(Debug, Clone, PartialEq)]
struct SiteLinks {
    spotify: Option<String>,
    tidal: Option<String>,
    clean: bool,
}

/// One service's state in the site's answer: its link, and whether the
/// answer is final. `unconfigured` (its keys are not set yet) is final until
/// the next check; `error` and anything this build does not know are not.
fn site_service(v: &Value, check: fn(&str) -> Option<String>) -> (Option<String>, bool) {
    match v.get("state").and_then(Value::as_str) {
        Some("found") => (v.get("url").and_then(Value::as_str).and_then(check), true),
        Some("miss") | Some("unconfigured") => (None, true),
        _ => (None, false),
    }
}

/// streamnook.app's answer. The function not deployed yet answers through
/// the site's page fallback (200 HTML) or the relay's 404, and any 2xx that
/// is not JSON with both services in it reads the same: not there yet, a
/// clean answer that never counts against the host.
fn parse_site(reply: &Reply) -> Parsed<SiteLinks> {
    if reply.status == 404 {
        return Parsed::Unavailable;
    }
    if !reply.ok() {
        return Parsed::Bad;
    }
    if !reply.is_json() {
        return Parsed::Unavailable;
    }
    let Ok(answer) = serde_json::from_slice::<Value>(&reply.body) else {
        return Parsed::Unavailable;
    };
    let (Some(spotify), Some(tidal)) = (
        answer.get("spotify").filter(|v| v.is_object()),
        answer.get("tidal").filter(|v| v.is_object()),
    ) else {
        return Parsed::Unavailable;
    };
    let (spotify, spotify_final) = site_service(spotify, spotify_track_url);
    let (tidal, tidal_final) = site_service(tidal, tidal_track_url);
    Parsed::Found(SiteLinks {
        spotify,
        tidal,
        clean: spotify_final && tidal_final,
    })
}

// ------------------------------------------------------------------ hosts

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Host {
    Deezer = 0,
    YouTubeMusic = 1,
    Beatport = 2,
    Site = 3,
}

impl Host {
    const ALL: [Host; 4] = [Host::Deezer, Host::YouTubeMusic, Host::Beatport, Host::Site];

    /// The least time between two calls to the host.
    fn gap(self) -> Duration {
        match self {
            Host::Deezer => Duration::from_millis(120),
            Host::YouTubeMusic | Host::Beatport => Duration::from_secs(1),
            Host::Site => Duration::from_millis(100),
        }
    }
}

/// One host's gate and breaker. Every method takes the time, so a test can
/// move it.
#[derive(Debug)]
struct HostState {
    gap: Duration,
    next_at: Option<Instant>,
    fails: u32,
    open_until: Option<Instant>,
}

impl HostState {
    fn new(gap: Duration) -> HostState {
        HostState {
            gap,
            next_at: None,
            fails: 0,
            open_until: None,
        }
    }

    fn is_open(&self, now: Instant) -> bool {
        self.open_until.is_some_and(|until| now < until)
    }

    /// When a call asked for at `now` may start; the next one waits a gap
    /// after it.
    fn reserve(&mut self, now: Instant) -> Instant {
        let start = self.next_at.map_or(now, |at| at.max(now));
        self.next_at = Some(start + self.gap);
        start
    }

    /// Count an answer: a failure toward the breaker, a good answer resets
    /// the count. Three failures in a row open it for `OPEN_FOR`.
    fn record(&mut self, ok: bool, now: Instant) {
        if ok {
            self.fails = 0;
            return;
        }
        self.fails += 1;
        if self.fails >= FAILURES_TO_OPEN {
            self.fails = 0;
            self.open_until = Some(now + OPEN_FOR);
        }
    }
}

type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// One call to a host.
#[derive(Debug, Clone)]
struct Request {
    url: String,
    /// A JSON body, sent as a POST.
    json: Option<Value>,
    headers: &'static [(&'static str, &'static str)],
}

/// A host's answer, read whole.
#[derive(Debug, Clone)]
struct Reply {
    status: u16,
    content_type: Option<String>,
    body: Vec<u8>,
}

impl Reply {
    fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    fn is_json(&self) -> bool {
        self.content_type
            .as_deref()
            .and_then(|t| t.split(';').next())
            .map(str::trim)
            .is_some_and(|t| t == "application/json" || t.ends_with("+json"))
    }

    /// The body as JSON, from a 2xx answer.
    fn json(&self) -> Option<Value> {
        if !self.ok() {
            return None;
        }
        serde_json::from_slice(&self.body).ok()
    }
}

#[derive(Debug, Clone)]
enum Sent {
    Reply(Reply),
    /// No answer: the connection failed, or the call's own timeout passed.
    Failed,
}

/// The network, behind a trait so tests can stand in for every host.
trait Http: Send + Sync {
    fn send(&self, host: Host, request: Request) -> BoxFut<'_, Sent>;
}

/// What one parser made of an answer.
enum Parsed<T> {
    Found(T),
    Miss,
    /// streamnook.app does not offer it yet.
    Unavailable,
    /// Not an answer the host gives: counts as a failure.
    Bad,
}

/// One lookup's outcome.
#[derive(Debug, PartialEq)]
enum Look<T> {
    Found(T),
    Miss,
    Unavailable,
    /// An error answer, no answer, or one that did not parse.
    Failed,
    /// The time budget ran out first. Never a failure.
    Cut,
    /// The host's breaker is open. Not looked up, never a failure.
    Open,
}

impl<T> Look<T> {
    /// Whether the lookup gave a final answer.
    fn clean(&self) -> bool {
        matches!(self, Look::Found(_) | Look::Miss | Look::Unavailable)
    }
}

/// The hosts, each with its gate and breaker.
struct Net {
    http: Arc<dyn Http>,
    hosts: Mutex<Vec<HostState>>,
}

static NET: LazyLock<Net> = LazyLock::new(|| Net::new(Arc::new(Live)));

impl Net {
    fn new(http: Arc<dyn Http>) -> Net {
        Net {
            http,
            hosts: Mutex::new(Host::ALL.iter().map(|h| HostState::new(h.gap())).collect()),
        }
    }

    fn state<R>(&self, host: Host, f: impl FnOnce(&mut HostState) -> R) -> R {
        let mut hosts = self.hosts.lock().unwrap_or_else(|p| p.into_inner());
        f(&mut hosts[host as usize])
    }

    /// One call: wait for the host's gate, then give it its `time` (a share
    /// from the gate, or until a set end). A host whose breaker is open is not
    /// called, nor one whose gate opens only after the end.
    async fn look<T>(
        &self,
        host: Host,
        request: Request,
        time: Time,
        parse: impl FnOnce(&Reply) -> Parsed<T>,
    ) -> Look<T> {
        let start = self.state(host, |s| {
            let now = Instant::now();
            if s.is_open(now) {
                return Err(Look::Open);
            }
            let opens = s.next_at.map_or(now, |at| at.max(now));
            if let Time::Until(end) = time {
                if opens >= end.into_std() {
                    return Err(Look::Cut);
                }
            }
            Ok(s.reserve(now))
        });
        let start = match start {
            Ok(start) => start,
            Err(look) => return look,
        };
        tokio::time::sleep_until(tokio::time::Instant::from_std(start)).await;
        if self.state(host, |s| s.is_open(Instant::now())) {
            return Look::Open;
        }
        let call = self.http.send(host, request);
        let answered = match time {
            Time::Share(share) => tokio::time::timeout(share, call).await,
            Time::Until(end) => tokio::time::timeout_at(end, call).await,
        };
        let parsed = match answered {
            Err(_) => return Look::Cut,
            Ok(Sent::Failed) => Parsed::Bad,
            Ok(Sent::Reply(reply)) => parse(&reply),
        };
        let counted = match &parsed {
            Parsed::Found(_) | Parsed::Miss => Some(true),
            Parsed::Bad => Some(false),
            Parsed::Unavailable => None,
        };
        if let Some(ok) = counted {
            self.state(host, |s| s.record(ok, Instant::now()));
            if !ok {
                log::debug!("[SongLinks] {host:?} did not answer");
            }
        }
        match parsed {
            Parsed::Found(t) => Look::Found(t),
            Parsed::Miss => Look::Miss,
            Parsed::Unavailable => Look::Unavailable,
            Parsed::Bad => Look::Failed,
        }
    }

    async fn deezer_by_isrc(&self, isrc: &str, time: impl Into<Time>) -> Look<DeezerTrack> {
        let request = Request {
            url: format!("https://api.deezer.com/track/isrc:{isrc}"),
            json: None,
            headers: JSON_HEADERS,
        };
        self.look(Host::Deezer, request, time.into(), parse_deezer_track)
            .await
    }

    async fn deezer_by_text(
        &self,
        input: &ResolveInput,
        time: impl Into<Time>,
    ) -> Look<DeezerTrack> {
        let request = Request {
            url: format!(
                "https://api.deezer.com/search?q={}&limit={DEEZER_HITS}",
                urlencoding::encode(&words_of(input))
            ),
            json: None,
            headers: JSON_HEADERS,
        };
        self.look(Host::Deezer, request, time.into(), |r| {
            parse_deezer_search(r, input)
        })
        .await
    }

    async fn youtube_music(
        &self,
        input: &ResolveInput,
        reference: Option<u32>,
        time: impl Into<Time>,
    ) -> Look<String> {
        let request = Request {
            url: "https://music.youtube.com/youtubei/v1/search?prettyPrint=false".to_string(),
            json: Some(json!({
                "context": { "client": {
                    "clientName": "WEB_REMIX",
                    "clientVersion": YTM_CLIENT_VERSION,
                    "hl": "en",
                    "gl": "US"
                } },
                "query": words_of(input),
                "params": YTM_SONGS_ONLY
            })),
            headers: YTM_HEADERS,
        };
        self.look(Host::YouTubeMusic, request, time.into(), |r| {
            parse_ytm(r, input, reference)
        })
        .await
    }

    async fn beatport(
        &self,
        input: &ResolveInput,
        isrc: &str,
        time: impl Into<Time>,
    ) -> Look<String> {
        let request = Request {
            url: format!(
                "https://www.beatport.com/search/tracks?q={}",
                urlencoding::encode(&words_of(input))
            ),
            json: None,
            headers: PAGE_HEADERS,
        };
        self.look(Host::Beatport, request, time.into(), |r| {
            parse_beatport(r, isrc)
        })
        .await
    }

    async fn site(&self, isrc: &str, time: impl Into<Time>) -> Look<SiteLinks> {
        let request = Request {
            url: format!("https://streamnook.app/api/v1/song-links?isrc={isrc}"),
            json: None,
            headers: JSON_HEADERS,
        };
        self.look(Host::Site, request, time.into(), parse_site)
            .await
    }
}

/// The words a search asks for: artist and title.
fn words_of(input: &ResolveInput) -> String {
    format!("{} {}", input.artist.trim(), input.title.trim())
        .trim()
        .to_string()
}

async fn resolve_on(net: &Net, input: &ResolveInput, budget: impl Into<Budget>) -> Resolved {
    // Deezer's time, then the time of the lookups that run after it.
    let (first, rest) = match budget.into() {
        Budget::PerLookup(budget) => (Time::Share(budget / 2), Time::Share(budget / 2)),
        Budget::Total(budget) => {
            let now = tokio::time::Instant::now();
            (Time::Until(now + budget / 2), Time::Until(now + budget))
        }
    };
    let mut providers: Vec<Provider> = Vec::new();
    for p in &input.known {
        put(&mut providers, p.clone());
    }
    let mut isrc = input.isrc.as_deref().and_then(valid_isrc);
    let mut isrc_from = isrc
        .as_ref()
        .map(|_| input.isrc_from.unwrap_or(IsrcFrom::Text));
    if let Some(id) = input.apple_id.as_deref().and_then(valid_apple_id) {
        put(
            &mut providers,
            direct(
                "Apple Music",
                format!("https://music.apple.com/us/song/{id}"),
            ),
        );
    }

    let wants_video =
        !(has_direct(&providers, "YouTube Music") && has_direct(&providers, "YouTube"));
    let mut complete = true;
    let mut reference = None;
    let mut cover = None;
    // Deezer first: its length is what the YouTube Music match is held to,
    // and on the text path it may name the ISRC the rest look up by.
    if !has_direct(&providers, "Deezer") || isrc.is_none() || wants_video {
        let look = match &isrc {
            Some(code) => net.deezer_by_isrc(code, first).await,
            None => net.deezer_by_text(input, first).await,
        };
        complete &= look.clean();
        if let Look::Found(track) = look {
            put(
                &mut providers,
                direct(
                    "Deezer",
                    format!("https://www.deezer.com/track/{}", track.id),
                ),
            );
            reference = track.secs;
            cover = track.cover;
            if isrc.is_none() {
                if let Some(code) = track.isrc {
                    isrc = Some(code);
                    isrc_from = Some(IsrcFrom::Text);
                }
            }
        }
    }

    let code = isrc.clone().unwrap_or_default();
    let wants_beatport = isrc.is_some() && !has_direct(&providers, "Beatport");
    let wants_site =
        isrc.is_some() && !(has_direct(&providers, "Spotify") && has_direct(&providers, "Tidal"));
    let (video, beatport, site) = tokio::join!(
        async {
            if wants_video {
                Some(net.youtube_music(input, reference, rest).await)
            } else {
                None
            }
        },
        async {
            if wants_beatport {
                Some(net.beatport(input, &code, rest).await)
            } else {
                None
            }
        },
        async {
            if wants_site {
                Some(net.site(&code, rest).await)
            } else {
                None
            }
        },
    );
    if let Some(look) = video {
        complete &= look.clean();
        if let Look::Found(id) = look {
            put(
                &mut providers,
                direct(
                    "YouTube Music",
                    format!("https://music.youtube.com/watch?v={id}"),
                ),
            );
            put(
                &mut providers,
                direct("YouTube", format!("https://www.youtube.com/watch?v={id}")),
            );
        }
    }
    if let Some(look) = beatport {
        complete &= look.clean();
        if let Look::Found(url) = look {
            put(&mut providers, direct("Beatport", url));
        }
    }
    if let Some(look) = site {
        complete &= look.clean();
        if let Look::Found(links) = look {
            complete &= links.clean;
            if let Some(url) = links.spotify {
                put(&mut providers, direct("Spotify", url));
            }
            if let Some(url) = links.tidal {
                put(&mut providers, direct("Tidal", url));
            }
        }
    }

    for service in DISPLAY {
        if !providers.iter().any(|p| p.name == *service) {
            if let Some(link) = search_link(service, &input.artist, &input.title) {
                providers.push(link);
            }
        }
    }
    sort_providers(&mut providers);
    Resolved {
        providers,
        isrc,
        isrc_from,
        cover,
        complete,
    }
}

// -------------------------------------------------------------- the network

struct Live;

impl Http for Live {
    fn send(&self, host: Host, request: Request) -> BoxFut<'_, Sent> {
        Box::pin(live_send(host, request))
    }
}

static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(CALL_TIMEOUT)
        .build()
        .expect("Failed to build the song link client")
});

/// StreamNook's own agent, for Deezer and streamnook.app.
fn app_agent() -> &'static str {
    static AGENT: LazyLock<String> = LazyLock::new(|| {
        format!(
            "StreamNook/{}",
            crate::services::client_identity::build_version()
        )
    });
    &AGENT
}

async fn live_send(host: Host, request: Request) -> Sent {
    let agent = match host {
        Host::Deezer | Host::Site => app_agent(),
        Host::YouTubeMusic | Host::Beatport => CHROME_UA,
    };
    let mut builder = match &request.json {
        Some(body) => HTTP.post(&request.url).json(body),
        None => HTTP.get(&request.url),
    };
    builder = builder
        .header(reqwest::header::USER_AGENT, agent)
        .timeout(CALL_TIMEOUT);
    for (name, value) in request.headers {
        builder = builder.header(*name, *value);
    }
    // streamnook.app goes by the route that reaches it (direct or relay).
    let sent = match host {
        Host::Site => crate::services::sn_transport::send(builder).await,
        _ => builder.send().await,
    };
    let Ok(mut response) = sent else {
        return Sent::Failed;
    };
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_ascii_lowercase);
    let mut body = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if body.len() + chunk.len() > MAX_BODY {
                    return Sent::Failed;
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(_) => return Sent::Failed,
        }
    }
    Sent::Reply(Reply {
        status,
        content_type,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEEZER_HIT: &str = include_str!("fixtures/deezer-isrc-hit.json");
    const DEEZER_MISS: &str = include_str!("fixtures/deezer-isrc-miss.json");
    const DEEZER_SEARCH: &str = include_str!("fixtures/deezer-search.json");
    const YTM_SEARCH: &str = include_str!("fixtures/ytm-search.json");
    const BEATPORT: &str = include_str!("fixtures/beatport-next-data.json");
    const SPOTIFY_TRACK: &str = "https://open.spotify.com/track/0123456789abcdefghijkl";
    const TIDAL_TRACK: &str = "https://tidal.com/browse/track/12345678";

    fn reply(status: u16, content_type: &str, body: &str) -> Reply {
        Reply {
            status,
            content_type: Some(content_type.to_string()),
            body: body.as_bytes().to_vec(),
        }
    }

    fn json_reply(body: &str) -> Reply {
        reply(200, "application/json; charset=utf-8", body)
    }

    fn beatport_page(data: &str) -> Reply {
        reply(
            200,
            "text/html; charset=utf-8",
            &format!(
                "<!doctype html><html><body><div id=\"__next\"></div><script id=\"__NEXT_DATA__\" type=\"application/json\">{data}</script></body></html>"
            ),
        )
    }

    fn site_reply(spotify: Value, tidal: Value) -> Reply {
        json_reply(&json!({ "spotify": spotify, "tidal": tidal }).to_string())
    }

    fn input(title: &str, artist: &str, album: Option<&str>) -> ResolveInput {
        ResolveInput {
            title: title.into(),
            artist: artist.into(),
            album: album.map(str::to_string),
            isrc: None,
            isrc_from: None,
            apple_id: None,
            known: Vec::new(),
        }
    }

    fn kiss_of_life() -> ResolveInput {
        ResolveInput {
            isrc: Some("GBBBM9200027".into()),
            isrc_from: Some(IsrcFrom::Shazam),
            apple_id: Some("1440764166".into()),
            ..input("Kiss of Life", "Sade", Some("Love Deluxe"))
        }
    }

    fn found<T>(parsed: Parsed<T>) -> Option<T> {
        match parsed {
            Parsed::Found(t) => Some(t),
            _ => None,
        }
    }

    fn kind<T>(parsed: &Parsed<T>) -> &'static str {
        match parsed {
            Parsed::Found(_) => "found",
            Parsed::Miss => "miss",
            Parsed::Unavailable => "unavailable",
            Parsed::Bad => "bad",
        }
    }

    // ---------------------------------------------------------------- ids

    #[test]
    fn ids_are_validated_before_any_link_is_built() {
        assert_eq!(
            valid_isrc(" gbbbm9200027 ").as_deref(),
            Some("GBBBM9200027")
        );
        assert_eq!(valid_isrc("QM6MZ1422245").as_deref(), Some("QM6MZ1422245"));
        for bad in [
            "GBBBM920002",
            "GBBBM92000270",
            "G1BBM9200027",
            "GBBBM92000A7",
            "GB-BM9200027",
            "",
        ] {
            assert_eq!(valid_isrc(bad), None, "{bad}");
        }
        assert_eq!(valid_apple_id("950742158").as_deref(), Some("950742158"));
        for bad in ["", "95074215a", "1234567890123", "-1"] {
            assert_eq!(valid_apple_id(bad), None, "{bad}");
        }
        assert_eq!(
            valid_youtube_id("4MOzk7-6j-I").as_deref(),
            Some("4MOzk7-6j-I")
        );
        for bad in ["4MOzk7-6j-", "4MOzk7-6j-I1", "4MOzk7-6j-!", "4MOzk7/6j-I"] {
            assert_eq!(valid_youtube_id(bad), None, "{bad}");
        }
        assert_eq!(
            number_id(&json!(1030607572u64)).as_deref(),
            Some("1030607572")
        );
        assert_eq!(number_id(&json!("14957156")).as_deref(), Some("14957156"));
        assert_eq!(number_id(&json!("1/../2")), None);
        assert_eq!(number_id(&json!(-4)), None);
        assert_eq!(
            spotify_track_url(SPOTIFY_TRACK).as_deref(),
            Some(SPOTIFY_TRACK)
        );
        for bad in [
            "https://open.spotify.com/track/short",
            "https://open.spotify.com/album/0123456789abcdefghijkl",
            "http://open.spotify.com/track/0123456789abcdefghijkl",
            "https://open.spotify.com.x.example/track/0123456789abcdefghijkl",
            "https://open.spotify.com/track/0123456789abcdefghijkl/x",
        ] {
            assert_eq!(spotify_track_url(bad), None, "{bad}");
        }
        assert_eq!(tidal_track_url(TIDAL_TRACK).as_deref(), Some(TIDAL_TRACK));
        for bad in [
            "https://tidal.com/browse/track/12a",
            "https://listen.tidal.com/browse/track/1",
            "https://tidal.com/browse/album/1",
        ] {
            assert_eq!(tidal_track_url(bad), None, "{bad}");
        }
    }

    #[test]
    fn apple_ids_come_from_apple_song_links_only() {
        for (url, id) in [
            (
                "https://music.apple.com/us/album/nuevayol/1787022393?i=1787022572",
                Some("1787022572"),
            ),
            (
                "https://music.apple.com/us/song/950742158",
                Some("950742158"),
            ),
            (
                "https://music.apple.com/us/song/kiss-of-life/1440764166",
                Some("1440764166"),
            ),
            (
                "https://music.apple.com/us/album/love-deluxe/1440764000",
                None,
            ),
            ("https://music.apple.com/us/search?term=Sade", None),
            ("https://open.spotify.com/track/1?i=5", None),
            ("https://music.apple.com/us/album/x/1?i=12x", None),
        ] {
            assert_eq!(apple_id_of(url).as_deref(), id, "{url}");
        }
    }

    // -------------------------------------------------------------- order

    #[test]
    fn direct_links_come_first_then_searches_then_other_services() {
        let mut p = vec![
            search_link("Deezer", "A", "B").unwrap(),
            direct("Pandora", "https://www.pandora.com/x".into()),
            direct("Deezer", "https://www.deezer.com/track/1".into()),
            search_link("Spotify", "A", "B").unwrap(),
            direct("Amazon Music", "https://music.amazon.com/x".into()),
            direct("Apple Music", "https://music.apple.com/us/song/1".into()),
            search_link("SoundCloud", "A", "B").unwrap(),
        ];
        sort_providers(&mut p);
        let order: Vec<(&str, bool)> = p.iter().map(|p| (p.name.as_str(), p.search)).collect();
        assert_eq!(
            order,
            vec![
                ("Apple Music", false),
                ("Deezer", false),
                ("Spotify", true),
                ("Deezer", true),
                ("SoundCloud", true),
                ("Pandora", false),
                ("Amazon Music", false)
            ]
        );
    }

    #[test]
    fn a_slot_takes_a_direct_link_over_a_search_and_keeps_its_direct_one() {
        let mut p = vec![search_link("Spotify", "A", "B").unwrap()];
        put(&mut p, search_link("Spotify", "C", "D").unwrap());
        assert!(
            p[0].url.contains("A%20B"),
            "a search never replaces a search"
        );
        put(&mut p, direct("Spotify", SPOTIFY_TRACK.into()));
        assert_eq!(p[0].url, SPOTIFY_TRACK);
        put(
            &mut p,
            direct("Spotify", "https://open.spotify.com/track/other".into()),
        );
        put(&mut p, search_link("Spotify", "A", "B").unwrap());
        assert_eq!((p.len(), p[0].url.as_str()), (1, SPOTIFY_TRACK));
    }

    #[test]
    fn the_services_a_lookup_can_fill_follow_what_the_song_has() {
        assert_eq!(
            fillable(false, false),
            vec!["YouTube Music", "YouTube", "Deezer"]
        );
        assert_eq!(
            fillable(true, true),
            vec![
                "Spotify",
                "Apple Music",
                "YouTube Music",
                "YouTube",
                "Deezer",
                "Tidal"
            ]
        );
        for has in [false, true] {
            assert!(!fillable(has, has).contains(&"Beatport"));
            assert!(!fillable(has, has).contains(&"SoundCloud"));
        }
    }

    // ----------------------------------------------------------- matching

    fn versions(raw: &str) -> Vec<&'static str> {
        title_of(raw).versions.into_iter().collect()
    }

    #[test]
    fn titles_compare_without_credits_and_with_their_version_words() {
        assert_eq!(title_of("Kiss of Life").core, "kiss of life");
        let live = title_of("Kiss of Life (Live 2011)");
        assert_eq!(
            (live.core.as_str(), versions("Kiss of Life (Live 2011)")),
            ("kiss of life", vec!["live"])
        );
        assert_ne!(title_of("Kiss of Life"), live);
        assert_eq!(
            title_of("Turn On The Lights again.. (feat. Future & Fred again..)"),
            title_of("Turn On The Lights again..")
        );
        assert_eq!(title_of("Song feat. Someone"), title_of("Song"));
        assert_eq!(title_of("Song (with Someone)"), title_of("Song"));
        assert_eq!(versions("Song - Remastered 2011"), vec!["remaster"]);
        assert_eq!(title_of("Song - Remastered 2011").core, "song");
        assert_eq!(versions("Song (Radio Edit)"), vec!["radio edit"]);
        assert_ne!(title_of("Song (Radio Edit)"), title_of("Song (Edit)"));
        assert_eq!(versions("Song (Sped Up)"), vec!["sped up"]);
        assert_eq!(title_of("Rock & Roll"), title_of("Rock and Roll"));
        assert_eq!(title_of("Don't Stop!"), title_of("Dont Stop"));
        assert_eq!(
            title_of("Caf\u{e9} \u{2013} Live"),
            title_of("Cafe (Live)"),
            "an en dash splits a version off like a hyphen"
        );
        assert_eq!(
            title_of("Marea (we've lost dancing)").core,
            "marea weve lost dancing",
            "a bracket that is no credit or version is part of the title"
        );
        assert_eq!(
            primary_artist("Swedish House Mafia, Future & Fred again.."),
            "swedish house mafia"
        );
        assert_eq!(
            primary_artist("Simon & Garfunkel"),
            primary_artist("Simon and Garfunkel")
        );
        assert_eq!(
            primary_artist("Lil Nas X feat. Billy Ray Cyrus"),
            "lil nas x"
        );
        assert_eq!(primary_artist("Lil Nas X & Billy Ray Cyrus"), "lil nas x");
        assert_eq!(primary_artist("Lil Nas X"), "lil nas x");
        assert_eq!(primary_artist("A x B"), "a");
        assert_eq!(primary_artist("Beyoncé"), "beyonce");
    }

    // ------------------------------------------------------------- Deezer

    #[test]
    fn deezer_by_isrc_reads_a_hit_a_miss_and_a_refusal() {
        let hit = found(parse_deezer_track(&json_reply(DEEZER_HIT))).unwrap();
        assert_eq!(hit.id, "1030607572");
        assert_eq!(hit.secs, Some(349));
        assert_eq!(hit.isrc.as_deref(), Some("GBBBM9200027"));
        assert!(hit
            .cover
            .as_deref()
            .unwrap()
            .starts_with("https://cdn-images.dzcdn.net/"));
        assert_eq!(kind(&parse_deezer_track(&json_reply(DEEZER_MISS))), "miss");
        let refusal = r#"{"error":{"type":"Exception","message":"Quota limit exceeded","code":4}}"#;
        assert_eq!(kind(&parse_deezer_track(&json_reply(refusal))), "bad");
        assert_eq!(
            kind(&parse_deezer_track(&reply(500, "text/html", "oops"))),
            "bad"
        );
        assert_eq!(
            kind(&parse_deezer_track(&reply(200, "text/html", "<html>"))),
            "bad"
        );
        assert_eq!(kind(&parse_deezer_track(&json_reply("{}"))), "bad");
    }

    #[test]
    fn deezers_text_search_breaks_a_tie_by_album_then_by_rank() {
        let pick = |album: Option<&str>| {
            found(parse_deezer_search(
                &json_reply(DEEZER_SEARCH),
                &input("Kiss of Life", "Sade", album),
            ))
            .map(|t| (t.id, t.secs, t.isrc))
        };
        assert_eq!(
            pick(Some("Love Deluxe")),
            Some(("1030607572".into(), Some(349), Some("GBBBM9200027".into())))
        );
        assert_eq!(
            pick(Some("The Best of Sade")),
            Some(("1030652052".into(), Some(250), Some("GBBBM9200026".into())))
        );
        // Two exact hits that last differently and no album to choose by: a
        // guess would write the wrong ISRC, so nothing is taken.
        assert_eq!(pick(None), None);
        assert_eq!(pick(Some("Lovers Rock")), None);

        // Two exact hits within 5 s of each other: the higher ranked.
        let close = json!({ "data": [
            { "id": 1, "title": "One", "artist": { "name": "A" }, "album": { "title": "X" }, "duration": 250, "rank": 10 },
            { "id": 2, "title": "One", "artist": { "name": "A" }, "album": { "title": "Y" }, "duration": 253, "rank": 20 },
            { "id": 3, "title": "One (Live)", "artist": { "name": "A" }, "duration": 400, "rank": 99 }
        ] });
        let hit = found(parse_deezer_search(
            &json_reply(&close.to_string()),
            &input("One", "A", None),
        ))
        .unwrap();
        assert_eq!(hit.id, "2");
        let one = json!({ "data": [{ "id": 5, "title": "One", "artist": { "name": "A feat. B" }, "duration": 1, "rank": 1 }] });
        assert_eq!(
            found(parse_deezer_search(
                &json_reply(&one.to_string()),
                &input("One", "A", None)
            ))
            .map(|t| t.id),
            Some("5".into())
        );
        assert_eq!(
            kind(&parse_deezer_search(
                &json_reply(r#"{"data":[]}"#),
                &input("One", "A", None)
            )),
            "miss"
        );
        assert_eq!(
            kind(&parse_deezer_search(
                &json_reply(r#"{"total":0}"#),
                &input("One", "A", None)
            )),
            "bad"
        );
    }

    // ------------------------------------------------------ YouTube Music

    fn ytm() -> Vec<YtItem> {
        ytm_items(&serde_json::from_str(YTM_SEARCH).unwrap()).unwrap()
    }

    #[test]
    fn youtube_musics_search_answer_reads_into_song_rows() {
        let items = ytm();
        assert_eq!(items.len(), 5);
        assert_eq!(
            items[0],
            YtItem {
                id: "4MOzk7-6j-I".into(),
                title: "Kiss of Life".into(),
                artist: "Sade".into(),
                secs: Some(350)
            }
        );
        assert_eq!(
            (items[1].id.as_str(), items[1].secs),
            ("uEcKk2U_U7A", Some(252))
        );
        assert_eq!(items[3].artist, "KISS OF LIFE");
        assert_eq!(clock_secs("1:02:03"), Some(3723));
        assert_eq!(clock_secs("350"), None);
        assert_eq!(clock_secs("5:5x"), None);
        assert!(ytm_items(&json!({ "error": { "code": 400 } })).is_none());
        assert_eq!(
            ytm_items(&json!({ "contents": { "tabbedSearchResultsRenderer": { "tabs": [] } } })),
            Some(Vec::new())
        );
    }

    #[test]
    fn youtube_music_matches_strictly() {
        let items = ytm();
        let sade = input("Kiss of Life", "Sade", None);
        let id = |items: &[YtItem], input: &ResolveInput, reference| {
            pick_video(items, input, reference).map(|it| it.id.clone())
        };
        // Deezer's 349 s against YouTube Music's 5:50.
        assert_eq!(id(&items, &sade, Some(349)).as_deref(), Some("4MOzk7-6j-I"));
        // The best-of edit (4:12) is another recording.
        assert_eq!(id(&items[1..], &sade, Some(349)), None);
        assert_eq!(id(&items, &sade, Some(250)).as_deref(), Some("uEcKk2U_U7A"));
        // A live recording is never the studio one.
        assert_eq!(
            id(
                &items,
                &input("Kiss of Life (Live)", "Sade", None),
                Some(350)
            ),
            None
        );
        // With no length to hold it to: only a single exact row.
        assert_eq!(
            id(&items, &sade, None),
            None,
            "two rows read exactly as ours"
        );
        assert_eq!(id(&items[..1], &sade, None).as_deref(), Some("4MOzk7-6j-I"));
        assert_eq!(
            id(&items, &input("Tell Me", "KISS OF LIFE", None), None).as_deref(),
            Some("brBJpDMypx4")
        );
        assert_eq!(kind(&parse_ytm(&json_reply("[]"), &sade, None)), "bad");
        assert_eq!(
            found(parse_ytm(&json_reply(YTM_SEARCH), &sade, Some(349))).as_deref(),
            Some("4MOzk7-6j-I")
        );
    }

    // ------------------------------------------------------------ Beatport

    #[test]
    fn beatport_is_taken_only_on_an_equal_isrc() {
        assert_eq!(
            found(parse_beatport(&beatport_page(BEATPORT), "GBAHS2100041")).as_deref(),
            Some("https://www.beatport.com/track/-/14957156")
        );
        assert_eq!(
            kind(&parse_beatport(&beatport_page(BEATPORT), "GBBBM9200027")),
            "miss"
        );
        assert_eq!(
            kind(&parse_beatport(
                &reply(200, "text/html", "<html></html>"),
                "GBAHS2100041"
            )),
            "bad"
        );
        assert_eq!(
            kind(&parse_beatport(
                &reply(403, "text/html", "denied"),
                "GBAHS2100041"
            )),
            "bad"
        );
        assert_eq!(
            kind(&parse_beatport(
                &beatport_page(r#"{"props":{}}"#),
                "GBAHS2100041"
            )),
            "bad"
        );
    }

    // ------------------------------------------------------ streamnook.app

    #[test]
    fn the_sites_answer_carries_each_services_state() {
        let links = found(parse_site(&site_reply(
            json!({ "state": "found", "url": SPOTIFY_TRACK }),
            json!({ "state": "miss" }),
        )))
        .unwrap();
        assert_eq!(
            links,
            SiteLinks {
                spotify: Some(SPOTIFY_TRACK.into()),
                tidal: None,
                clean: true
            }
        );
        let links = found(parse_site(&site_reply(
            json!({ "state": "error" }),
            json!({ "state": "found", "url": TIDAL_TRACK }),
        )))
        .unwrap();
        assert_eq!(
            (links.tidal.as_deref(), links.clean),
            (Some(TIDAL_TRACK), false)
        );
        let links = found(parse_site(&site_reply(
            json!({ "state": "unconfigured" }),
            json!({ "state": "unconfigured" }),
        )))
        .unwrap();
        assert!(links.clean && links.spotify.is_none() && links.tidal.is_none());
        let links = found(parse_site(&site_reply(
            json!({ "state": "found", "url": "https://x.example/track/1" }),
            json!({ "state": "later" }),
        )))
        .unwrap();
        assert_eq!((links.spotify, links.clean), (None, false));
    }

    #[test]
    fn the_sites_page_fallback_and_partial_answers_are_not_there_yet() {
        let spa = reply(
            200,
            "text/html; charset=utf-8",
            "<!doctype html><div id=\"root\"></div>",
        );
        assert_eq!(kind(&parse_site(&spa)), "unavailable");
        assert_eq!(
            kind(&parse_site(&json_reply(r#"{"spotify":{"state":"miss"}}"#))),
            "unavailable"
        );
        assert_eq!(kind(&parse_site(&json_reply("not json"))), "unavailable");
        assert_eq!(
            kind(&parse_site(&reply(
                404,
                "application/json",
                r#"{"error":"relay_not_allowed"}"#
            ))),
            "unavailable"
        );
        assert_eq!(
            kind(&parse_site(&reply(429, "application/json", "{}"))),
            "bad"
        );
        assert_eq!(
            kind(&parse_site(&reply(
                503,
                "application/json",
                r#"{"error":"busy"}"#
            ))),
            "bad"
        );
    }

    // ----------------------------------------------------------- breakers

    #[test]
    fn a_breaker_opens_after_three_failures_and_closes_after_five_minutes() {
        let t0 = Instant::now();
        let mut s = HostState::new(Duration::ZERO);
        s.record(false, t0);
        s.record(true, t0);
        s.record(false, t0);
        s.record(false, t0);
        assert!(!s.is_open(t0), "a good answer started the count again");
        s.record(false, t0);
        assert!(s.is_open(t0));
        assert!(s.is_open(t0 + OPEN_FOR - Duration::from_millis(1)));
        assert!(!s.is_open(t0 + OPEN_FOR));
    }

    #[test]
    fn a_gate_spaces_calls_by_its_gap() {
        let t0 = Instant::now();
        let gap = Duration::from_millis(120);
        let mut s = HostState::new(gap);
        assert_eq!(s.reserve(t0), t0);
        assert_eq!(s.reserve(t0), t0 + gap);
        assert_eq!(
            s.reserve(t0 + Duration::from_secs(5)),
            t0 + Duration::from_secs(5)
        );
    }

    // --------------------------------------------------------- whole runs

    type Rule = Box<dyn Fn(Host, &Request) -> (Sent, Duration) + Send + Sync>;

    struct Fake {
        rule: Rule,
        calls: Mutex<Vec<(Host, String)>>,
    }

    impl Http for Fake {
        fn send(&self, host: Host, request: Request) -> BoxFut<'_, Sent> {
            self.calls.lock().unwrap().push((host, request.url.clone()));
            let (out, delay) = (self.rule)(host, &request);
            Box::pin(async move {
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
                out
            })
        }
    }

    impl Fake {
        fn calls_to(&self, host: Host) -> usize {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(h, _)| *h == host)
                .count()
        }
    }

    /// A net over fake hosts, with no gaps unless a test sets them.
    fn fake_net(
        rule: impl Fn(Host, &Request) -> (Sent, Duration) + Send + Sync + 'static,
    ) -> (Net, Arc<Fake>) {
        let fake = Arc::new(Fake {
            rule: Box::new(rule),
            calls: Mutex::new(Vec::new()),
        });
        let net = Net {
            http: fake.clone(),
            hosts: Mutex::new(
                Host::ALL
                    .iter()
                    .map(|_| HostState::new(Duration::ZERO))
                    .collect(),
            ),
        };
        (net, fake)
    }

    /// Sade's "Kiss of Life" on every host, as recorded.
    fn every_host_answers(
        site: Reply,
    ) -> impl Fn(Host, &Request) -> (Sent, Duration) + Send + Sync + 'static {
        move |host, _| {
            let out = match host {
                Host::Deezer => json_reply(DEEZER_HIT),
                Host::YouTubeMusic => json_reply(YTM_SEARCH),
                Host::Beatport => beatport_page(BEATPORT),
                Host::Site => site.clone(),
            };
            (Sent::Reply(out), Duration::ZERO)
        }
    }

    fn found_site() -> Reply {
        site_reply(
            json!({ "state": "found", "url": SPOTIFY_TRACK }),
            json!({ "state": "found", "url": TIDAL_TRACK }),
        )
    }

    fn links_of(r: &Resolved) -> Vec<(&str, bool)> {
        r.providers
            .iter()
            .map(|p| (p.name.as_str(), p.search))
            .collect()
    }

    #[tokio::test]
    async fn a_song_with_its_isrc_gets_direct_links_first() {
        let (net, fake) = fake_net(every_host_answers(found_site()));
        let r = resolve_on(&net, &kiss_of_life(), Duration::from_secs(3)).await;
        assert_eq!(
            links_of(&r),
            vec![
                ("Spotify", false),
                ("Apple Music", false),
                ("YouTube Music", false),
                ("YouTube", false),
                ("Deezer", false),
                ("Tidal", false),
                ("SoundCloud", true)
            ],
            "Beatport has no such ISRC and no search link"
        );
        let url = |name: &str| {
            r.providers
                .iter()
                .find(|p| p.name == name)
                .unwrap()
                .url
                .as_str()
        };
        assert_eq!(
            url("Apple Music"),
            "https://music.apple.com/us/song/1440764166"
        );
        assert_eq!(
            url("YouTube Music"),
            "https://music.youtube.com/watch?v=4MOzk7-6j-I"
        );
        assert_eq!(
            url("YouTube"),
            "https://www.youtube.com/watch?v=4MOzk7-6j-I"
        );
        assert_eq!(url("Deezer"), "https://www.deezer.com/track/1030607572");
        assert_eq!(url("Spotify"), SPOTIFY_TRACK);
        assert!(r.complete);
        assert_eq!(
            (r.isrc.as_deref(), r.isrc_from),
            (Some("GBBBM9200027"), Some(IsrcFrom::Shazam))
        );
        assert!(r.cover.is_some());
        assert_eq!(fake.calls_to(Host::Beatport), 1);
        for p in &r.providers {
            assert!(
                services::is_link_to(&p.name, &p.url),
                "{} -> {}",
                p.name,
                p.url
            );
        }
    }

    #[tokio::test]
    async fn a_song_without_an_isrc_takes_one_from_a_clear_text_hit() {
        let (net, fake) = fake_net(|host, request: &Request| {
            let out = match host {
                Host::Deezer if request.url.contains("/search?") => json_reply(DEEZER_SEARCH),
                Host::YouTubeMusic => json_reply(YTM_SEARCH),
                Host::Beatport => beatport_page(BEATPORT),
                Host::Site => found_site(),
                _ => reply(500, "text/plain", "unexpected"),
            };
            (Sent::Reply(out), Duration::ZERO)
        });
        let r = resolve_on(
            &net,
            &input("Kiss of Life", "Sade", Some("Love Deluxe")),
            Duration::from_secs(3),
        )
        .await;
        assert_eq!(
            (r.isrc.as_deref(), r.isrc_from),
            (Some("GBBBM9200027"), Some(IsrcFrom::Text))
        );
        assert!(r.complete);
        assert!(r
            .providers
            .iter()
            .any(|p| p.name == "YouTube Music" && !p.search));
        assert_eq!(
            fake.calls_to(Host::Site),
            1,
            "the found ISRC asks for Spotify and Tidal"
        );

        // Without the album to choose by, no ISRC and no Deezer link.
        let (net, fake) = fake_net(|host, request: &Request| {
            let out = match host {
                Host::Deezer if request.url.contains("/search?") => json_reply(DEEZER_SEARCH),
                Host::YouTubeMusic => json_reply(YTM_SEARCH),
                _ => reply(500, "text/plain", "unexpected"),
            };
            (Sent::Reply(out), Duration::ZERO)
        });
        let r = resolve_on(
            &net,
            &input("Kiss of Life", "Sade", None),
            Duration::from_secs(3),
        )
        .await;
        assert_eq!(r.isrc, None);
        assert!(r.complete, "a clean miss everywhere");
        assert!(r.providers.iter().all(|p| p.search), "{:?}", links_of(&r));
        assert_eq!(fake.calls_to(Host::Site) + fake.calls_to(Host::Beatport), 0);
    }

    #[tokio::test]
    async fn a_budget_cut_is_not_complete_and_counts_against_no_host() {
        let others = every_host_answers(found_site());
        let (net, _) = fake_net(move |host, request: &Request| match host {
            Host::Deezer => (
                Sent::Reply(json_reply(DEEZER_HIT)),
                Duration::from_millis(600),
            ),
            _ => others(host, request),
        });
        let r = resolve_on(&net, &kiss_of_life(), Duration::from_millis(300)).await;
        assert!(!r.complete);
        assert!(r.providers.iter().any(|p| p.name == "Deezer" && p.search));
        assert_eq!(net.state(Host::Deezer, |s| s.fails), 0);
        assert!(!net.state(Host::Deezer, |s| s.is_open(Instant::now())));
    }

    #[tokio::test]
    async fn each_lookups_budget_starts_after_its_hosts_gate() {
        let (net, _) = fake_net(|host, _: &Request| match host {
            Host::Deezer => (
                Sent::Reply(json_reply(DEEZER_HIT)),
                Duration::from_millis(40),
            ),
            _ => (Sent::Reply(json_reply(DEEZER_MISS)), Duration::ZERO),
        });
        // Another lookup took Deezer's gate: this one waits 300 ms for it.
        net.state(Host::Deezer, |s| {
            s.next_at = Some(Instant::now() + Duration::from_millis(300))
        });
        let began = Instant::now();
        let look = net
            .deezer_by_isrc("GBBBM9200027", Duration::from_millis(150))
            .await;
        assert!(began.elapsed() >= Duration::from_millis(300));
        assert!(
            matches!(look, Look::Found(ref t) if t.id == "1030607572"),
            "{look:?}"
        );
    }

    #[tokio::test]
    async fn at_identify_time_a_slow_gate_never_holds_the_answer_past_its_budget() {
        let others = every_host_answers(found_site());
        let (net, fake) = fake_net(move |host, request: &Request| match host {
            // streamnook.app answers after the budget.
            Host::Site => (Sent::Reply(found_site()), Duration::from_secs(2)),
            _ => others(host, request),
        });
        // Another lookup holds Deezer's gate past the whole budget, and
        // YouTube Music's past half of it.
        net.state(Host::Deezer, |s| {
            s.next_at = Some(Instant::now() + Duration::from_secs(2))
        });
        net.state(Host::YouTubeMusic, |s| {
            s.next_at = Some(Instant::now() + Duration::from_millis(200))
        });
        let began = Instant::now();
        let r = resolve_on(
            &net,
            &kiss_of_life(),
            Budget::Total(Duration::from_millis(300)),
        )
        .await;
        let took = began.elapsed();
        assert!(took >= Duration::from_millis(290), "{took:?}");
        assert!(took < Duration::from_millis(600), "{took:?}");
        assert!(!r.complete);
        // A gate that opens only after the end is not waited for or called.
        assert_eq!(fake.calls_to(Host::Deezer), 0);
        // One that opens before it is still called.
        assert_eq!(fake.calls_to(Host::YouTubeMusic), 1);
        // A cut is no failure.
        assert_eq!(net.state(Host::Site, |s| s.fails), 0);
    }

    #[tokio::test]
    async fn the_refresh_runner_still_waits_for_a_gate_and_gives_each_lookup_its_share() {
        let (net, fake) = fake_net(every_host_answers(found_site()));
        net.state(Host::Deezer, |s| {
            s.next_at = Some(Instant::now() + Duration::from_millis(300))
        });
        let began = Instant::now();
        let r = resolve_on(&net, &kiss_of_life(), Duration::from_millis(200)).await;
        assert!(began.elapsed() >= Duration::from_millis(300));
        assert_eq!(fake.calls_to(Host::Deezer), 1);
        assert!(r.complete, "{:?}", links_of(&r));
    }

    #[test]
    fn a_long_title_cuts_the_search_words_between_two_characters() {
        let title = "あ".repeat(140);
        let link = search_link("Spotify", "アーティスト", &title).unwrap();
        let q = link
            .url
            .strip_prefix("https://open.spotify.com/search/")
            .unwrap();
        assert!(q.len() <= SEARCH_WORDS_MAX, "{}", q.len());
        // Nine bytes a character: whole characters only, nothing half-escaped.
        let words = urlencoding::decode(q).unwrap();
        assert!(format!("アーティスト {title}").starts_with(words.as_ref()));
        assert!(words.chars().count() > 10);
        assert!(!words.ends_with(' '));
        // A short song keeps every word.
        assert_eq!(
            search_link("Spotify", "Sade", "Kiss of Life").unwrap().url,
            "https://open.spotify.com/search/Sade%20Kiss%20of%20Life"
        );
        // A cut that lands after a space leaves none at the end.
        assert_eq!(
            encoded_words(&format!("{} bc", "a".repeat(197))),
            "a".repeat(197)
        );
    }

    #[tokio::test]
    async fn an_open_breaker_is_not_clean_calls_nothing_and_gives_the_search_link() {
        let (net, fake) = fake_net(every_host_answers(found_site()));
        net.state(Host::Deezer, |s| {
            s.open_until = Some(Instant::now() + Duration::from_secs(60))
        });
        let r = resolve_on(&net, &kiss_of_life(), Duration::from_secs(3)).await;
        assert!(!r.complete);
        assert_eq!(fake.calls_to(Host::Deezer), 0);
        assert!(r.providers.iter().any(|p| p.name == "Deezer" && p.search));
        assert_eq!(net.state(Host::Deezer, |s| s.fails), 0);
    }

    #[tokio::test]
    async fn three_failures_open_a_hosts_breaker() {
        let (net, fake) = fake_net(|_, _: &Request| {
            (
                Sent::Reply(reply(503, "text/plain", "busy")),
                Duration::ZERO,
            )
        });
        for _ in 0..3 {
            assert_eq!(
                net.deezer_by_isrc("GBBBM9200027", Duration::from_secs(1))
                    .await,
                Look::Failed
            );
        }
        assert_eq!(
            net.deezer_by_isrc("GBBBM9200027", Duration::from_secs(1))
                .await,
            Look::Open
        );
        assert_eq!(fake.calls_to(Host::Deezer), 3);
        // No answer at all counts the same.
        let (net, _) = fake_net(|_, _: &Request| (Sent::Failed, Duration::ZERO));
        assert_eq!(
            net.site("GBBBM9200027", Duration::from_secs(1)).await,
            Look::Failed
        );
        assert_eq!(net.state(Host::Site, |s| s.fails), 1);
    }

    #[tokio::test]
    async fn the_sites_page_fallback_is_clean_and_never_trips_its_breaker() {
        let spa = reply(200, "text/html", "<!doctype html><div id=\"root\"></div>");
        let (net, fake) = fake_net(every_host_answers(spa));
        for _ in 0..4 {
            let r = resolve_on(&net, &kiss_of_life(), Duration::from_secs(3)).await;
            assert!(r.complete, "the song is stamped and asked again in 7 days");
            assert!(r.providers.iter().any(|p| p.name == "Spotify" && p.search));
        }
        assert_eq!(fake.calls_to(Host::Site), 4, "never opened");
        assert_eq!(net.state(Host::Site, |s| s.fails), 0);
        let (partial, _) = fake_net(every_host_answers(json_reply(
            r#"{"spotify":{"state":"found","url":"x"}}"#,
        )));
        assert!(
            resolve_on(&partial, &kiss_of_life(), Duration::from_secs(3))
                .await
                .complete
        );
    }

    #[tokio::test]
    async fn the_sites_states_decide_whether_the_song_is_complete() {
        for (site, complete) in [
            (
                reply(404, "application/json", r#"{"error":"relay_not_allowed"}"#),
                true,
            ),
            (
                site_reply(
                    json!({ "state": "unconfigured" }),
                    json!({ "state": "unconfigured" }),
                ),
                true,
            ),
            (
                site_reply(json!({ "state": "miss" }), json!({ "state": "miss" })),
                true,
            ),
            (
                site_reply(json!({ "state": "error" }), json!({ "state": "miss" })),
                false,
            ),
            (reply(429, "application/json", "{}"), false),
        ] {
            let (net, _) = fake_net(every_host_answers(site.clone()));
            let r = resolve_on(&net, &kiss_of_life(), Duration::from_secs(3)).await;
            assert_eq!(
                r.complete,
                complete,
                "{:?}",
                String::from_utf8_lossy(&site.body)
            );
        }
    }

    #[tokio::test]
    async fn links_a_song_already_has_skip_their_lookups() {
        let (net, fake) = fake_net(every_host_answers(found_site()));
        let mut song = kiss_of_life();
        song.known = vec![
            direct(
                "YouTube Music",
                "https://music.youtube.com/watch?v=4MOzk7-6j-I".into(),
            ),
            direct(
                "YouTube",
                "https://www.youtube.com/watch?v=4MOzk7-6j-I".into(),
            ),
            direct("Deezer", "https://www.deezer.com/track/1030607572".into()),
            direct("Spotify", SPOTIFY_TRACK.into()),
            direct("Tidal", TIDAL_TRACK.into()),
            direct("Pandora", "https://www.pandora.com/x".into()),
        ];
        let r = resolve_on(&net, &song, Duration::from_secs(3)).await;
        assert!(r.complete);
        assert_eq!(
            fake.calls_to(Host::Deezer)
                + fake.calls_to(Host::YouTubeMusic)
                + fake.calls_to(Host::Site),
            0
        );
        assert_eq!(fake.calls_to(Host::Beatport), 1);
        assert_eq!(r.providers.last().map(|p| p.name.as_str()), Some("Pandora"));
    }
}
