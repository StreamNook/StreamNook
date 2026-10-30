//! Cover colors for Your songs: each song's cover, fetched once, shrunk to
//! 48x48 and read for its strongest color and a second of another hue
//! (OKLCH, the rules Octo picks cover colors by). The page fits these to the
//! live theme; this module only finds them and keeps them.
//!
//! Storage: `<app_data>/song_tints.json`, keyed by the cover URL a song keeps,
//! beside the song history. history.rs holds both under one lock and loads
//! and lets go of them together, but this file has its own writer and its own
//! dirty flag, so a write of one never stands in for the other. The colors
//! are a cache: a tint file that cannot be read, does not parse or was
//! written by a newer StreamNook turns tints off until the next load, and
//! never stops the songs from loading or keeps them in memory.
//!
//! Work: `enqueue` lines covers up (the opened or just found song's first);
//! one runner fetches three at a time, each within 8 s, https only, with a
//! 2 MB cap and an image decoder held to 2048 px and 32 MiB. Results land
//! in batches (every 10, or when the queue empties) under the store's lock
//! with one write of the tint file and one `tinted` change for the page. A
//! failure that may pass (offline, a 5xx, a timeout) is retried after 5
//! minutes; one that will not (a 404, a picture that does not decode or is
//! too big) is kept in the file and retried after 30 days.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use futures::stream::{FuturesUnordered, StreamExt};
use image::{ImageError, Limits};
use serde::{Deserialize, Serialize};

use super::history;
use crate::rt::AppHandle;
use crate::services::universal_cache_service::write_atomic_with;

/// The tint file's name, beside the song history.
pub const FILE: &str = "song_tints.json";
const FILE_VERSION: u32 = 1;
/// Covers waiting, at most; past this the oldest normal ones go.
const QUEUE_CAP: usize = 200;
/// Covers fetched at once.
const AT_ONCE: usize = 3;
/// One cover's whole fetch and read.
const FETCH_TIMEOUT: Duration = Duration::from_secs(8);
/// A cover bigger than this is not read.
const MAX_BYTES: usize = 2 * 1024 * 1024;
/// The decoder's limits: a picture wider or taller than this, or needing
/// more memory, is refused before it is decoded.
const MAX_EDGE: u32 = 2048;
const MAX_ALLOC: u64 = 32 * 1024 * 1024;
/// The colors are read from a copy this small.
const SAMPLE_EDGE: u32 = 48;
/// Results written together.
const BATCH: usize = 10;
/// A failure that may pass is tried again after this long.
const RETRY_TRANSIENT: Duration = Duration::from_secs(5 * 60);
/// A failure that will not pass is tried again after this long.
pub const RETRY_PERMANENT_MS: i64 = 30 * 24 * 60 * 60 * 1000;
/// Covers one page of the list asks for, at most.
pub const MAX_PER_PAGE: usize = 50;

/// OKLCH lightness, chroma and hue (degrees), 3 decimals.
pub type Lch = [f64; 3];

/// What the tint file keeps for a cover: its colors (`main` None for a grey
/// cover), or when it could not be read for good.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(untagged)]
pub enum TintEntry {
    Failed {
        failed_ms: i64,
    },
    Color {
        main: Option<Lch>,
        second: Option<Lch>,
    },
}

impl TintEntry {
    /// Whether a stored entry holds real colors (a hand edit or a damaged
    /// row could hold anything).
    fn valid(&self) -> bool {
        let ok = |color: &Option<Lch>| {
            color.is_none_or(|[l, c, h]| {
                (0.0..=1.0).contains(&l) && (0.0..=0.5).contains(&c) && (0.0..=360.0).contains(&h)
            })
        };
        match self {
            TintEntry::Failed { failed_ms } => *failed_ms >= 0,
            TintEntry::Color { main, second } => ok(main) && ok(second),
        }
    }
}

/// A cover's colors as the page reads them.
#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
pub struct TintView {
    pub main: Option<Lch>,
    pub second: Option<Lch>,
}

impl TintView {
    pub const GREY: TintView = TintView {
        main: None,
        second: None,
    };
}

/// The page's view of a stored entry: None while there is none yet (a color
/// may still come); a failure reads as grey (nothing more will come).
pub fn view(entry: Option<&TintEntry>) -> Option<TintView> {
    match entry? {
        TintEntry::Color { main, second } => Some(TintView {
            main: *main,
            second: *second,
        }),
        TintEntry::Failed { .. } => Some(TintView::GREY),
    }
}

/// Whether a stored entry needs no fetch: a color or grey, or a failure
/// younger than `RETRY_PERMANENT_MS`.
pub fn usable(entry: Option<&TintEntry>, now_ms: i64) -> bool {
    match entry {
        None => false,
        Some(TintEntry::Color { .. }) => true,
        Some(TintEntry::Failed { failed_ms }) => now_ms - failed_ms < RETRY_PERMANENT_MS,
    }
}

/// The cover colors held with the songs.
#[derive(Default)]
pub struct Tints {
    pub map: BTreeMap<String, TintEntry>,
    pub path: PathBuf,
}

// ------------------------------------------------------------------ file

fn first_version() -> u32 {
    1
}

#[derive(Deserialize)]
struct FileHead {
    #[serde(default = "first_version")]
    version: u32,
}

#[derive(Deserialize)]
struct FileIn {
    #[serde(default)]
    tints: BTreeMap<String, serde_json::Value>,
}

#[derive(Serialize)]
struct FileOut<'a> {
    version: u32,
    tints: &'a BTreeMap<String, TintEntry>,
}

/// The entries a file holds, each read on its own: one this build cannot
/// read is left out.
fn entries(raw: BTreeMap<String, serde_json::Value>) -> BTreeMap<String, TintEntry> {
    raw.into_iter()
        .filter_map(|(url, v)| {
            let entry: TintEntry = serde_json::from_value(v).ok()?;
            entry.valid().then_some((url, entry))
        })
        .collect()
}

/// The colors kept at `path`, and whether tints are off for this load. A
/// missing file is an empty one. One that cannot be read is left alone and
/// turns tints off. A newer app's file is read as far as it can be, never
/// written and never moved. Any other file that does not parse is moved
/// aside as `song_tints.json.bad-<ms>` and turns tints off.
pub fn read_tints(path: &Path) -> (BTreeMap<String, TintEntry>, bool) {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (BTreeMap::new(), false),
        Err(e) => {
            log::warn!(
                "[SongTints] {} could not be read ({e}); colors off for now",
                path.display()
            );
            return (BTreeMap::new(), true);
        }
    };
    let head = serde_json::from_slice::<FileHead>(&bytes).ok();
    let newer = head.as_ref().is_some_and(|h| h.version > FILE_VERSION);
    match serde_json::from_slice::<FileIn>(&bytes) {
        Ok(file) if head.is_some() => {
            if newer {
                log::warn!(
                    "[SongTints] {} was saved by a newer StreamNook; read, not changed",
                    path.display()
                );
            }
            (entries(file.tints), newer)
        }
        _ if newer => (BTreeMap::new(), true),
        _ => {
            let aside = path.with_extension(format!(
                "json.bad-{}",
                chrono::Utc::now().timestamp_millis()
            ));
            match fs::rename(path, &aside) {
                Ok(()) => log::warn!(
                    "[SongTints] {} did not parse; kept as {}",
                    path.display(),
                    aside.display()
                ),
                Err(e) => log::warn!(
                    "[SongTints] {} did not parse and could not be moved aside: {e}",
                    path.display()
                ),
            }
            (BTreeMap::new(), true)
        }
    }
}

/// Write the colors to `path`, whole and atomically. Refused while tints
/// are off for this load, so a newer app's or an unreadable file is never
/// written over.
pub fn write_tints_file(
    path: &Path,
    map: &BTreeMap<String, TintEntry>,
    off: bool,
) -> Result<(), String> {
    if off {
        return Err("Cover colors are off for this load; their file is left as it is.".to_string());
    }
    let out = FileOut {
        version: FILE_VERSION,
        tints: map,
    };
    let json =
        serde_json::to_vec(&out).map_err(|e| format!("Failed to serialize cover colors: {e}"))?;
    write_atomic_with(path, json, |from, to| fs::rename(from, to))
        .map_err(|e| format!("Failed to write cover colors: {e}"))
}

// ----------------------------------------------------------------- memory

/// A changed tint map is in memory that the last write did not get to disk.
/// Cleared whenever the songs are let go of.
pub static TINT_DIRTY: AtomicBool = AtomicBool::new(false);
/// The tint file is a newer app's, or could not be read or parsed, this
/// load: nothing is queued or written.
static TINTS_OFF: AtomicBool = AtomicBool::new(false);
/// The runner task is running.
static RUNNER: AtomicBool = AtomicBool::new(false);

pub fn set_off(off: bool) {
    TINTS_OFF.store(off, Ordering::Release);
}

pub fn is_off() -> bool {
    TINTS_OFF.load(Ordering::Acquire)
}

/// How soon a cover is wanted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    /// A row of a list.
    Normal,
    /// The song just found or opened.
    High,
}

/// The work kept outside the store, so letting the songs go keeps it: the
/// queue (front first), the covers being fetched, and the covers to leave
/// alone until a time (a failure that may pass).
#[derive(Default)]
struct Memory {
    queue: VecDeque<(String, Priority)>,
    in_flight: HashSet<String>,
    transient: HashMap<String, Instant>,
}

impl Memory {
    /// Queue `urls`: none while tints are off; a cover being fetched or
    /// resting after a failure is skipped; a cover already queued moves
    /// rather than doubles and keeps the higher priority (one queued High
    /// and asked for as Normal stays where it is, ahead of every Normal
    /// one). High covers go to the very front, the newest Normal request to
    /// the front of the Normal ones, each in its order. No request takes out
    /// another's covers; past `QUEUE_CAP` the oldest Normal ones go.
    fn request(&mut self, urls: Vec<String>, priority: Priority, off: bool, now: Instant) {
        if off {
            return;
        }
        self.transient.retain(|_, until| *until > now);
        let mut fresh: Vec<String> = Vec::new();
        for url in urls {
            if url.is_empty()
                || fresh.contains(&url)
                || self.in_flight.contains(&url)
                || self.transient.contains_key(&url)
            {
                continue;
            }
            if let Some(at) = self.queue.iter().position(|(u, _)| *u == url) {
                if self.queue[at].1 > priority {
                    continue;
                }
                self.queue.remove(at);
            }
            fresh.push(url);
        }
        let at = match priority {
            Priority::High => 0,
            Priority::Normal => self
                .queue
                .iter()
                .position(|(_, p)| *p == Priority::Normal)
                .unwrap_or(self.queue.len()),
        };
        for (i, url) in fresh.into_iter().enumerate() {
            self.queue.insert(at + i, (url, priority));
        }
        while self.queue.len() > QUEUE_CAP {
            match self.queue.iter().rposition(|(_, p)| *p == Priority::Normal) {
                Some(oldest) => {
                    self.queue.remove(oldest);
                }
                None => {
                    self.queue.pop_back();
                }
            }
        }
    }

    /// The next cover to fetch, now in flight.
    fn take(&mut self) -> Option<(String, Priority)> {
        let next = self.queue.pop_front()?;
        self.in_flight.insert(next.0.clone());
        Some(next)
    }
}

static MEMORY: LazyLock<Mutex<Memory>> = LazyLock::new(Default::default);

fn memory() -> MutexGuard<'static, Memory> {
    MEMORY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Fetch the colors of these covers in the background. The caller passes
/// only covers with no usable entry (history.rs knows the map).
pub fn enqueue(app: &AppHandle, urls: Vec<String>, priority: Priority) {
    if urls.is_empty() {
        return;
    }
    memory().request(urls, priority, is_off(), Instant::now());
    start_runner(app);
}

fn start_runner(app: &AppHandle) {
    if memory().queue.is_empty() {
        return;
    }
    if RUNNER
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        run(&app).await;
        RUNNER.store(false, Ordering::Release);
        // A cover queued while this task was ending found it still running.
        start_runner(&app);
    });
}

/// Fetch until the queue is empty, `AT_ONCE` at a time, landing results in
/// batches. A High cover's result lands at once.
async fn run(app: &AppHandle) {
    let mut working = FuturesUnordered::new();
    let mut done: Vec<(String, TintEntry)> = Vec::new();
    loop {
        while working.len() < AT_ONCE {
            let next = memory().take();
            let Some((url, priority)) = next else {
                break;
            };
            working.push(async move {
                let outcome = tint_of(&url).await;
                (url, priority, outcome)
            });
        }
        let Some((url, priority, outcome)) = working.next().await else {
            break;
        };
        match outcome {
            Ok(entry) => done.push((url, entry)),
            Err(Failure::Permanent) => done.push((
                url,
                TintEntry::Failed {
                    failed_ms: chrono::Utc::now().timestamp_millis(),
                },
            )),
            Err(Failure::Transient) => {
                let mut m = memory();
                m.in_flight.remove(&url);
                m.transient.insert(url, Instant::now() + RETRY_TRANSIENT);
            }
        }
        let idle = working.is_empty() && memory().queue.is_empty();
        if !done.is_empty() && (done.len() >= BATCH || idle || priority == Priority::High) {
            land(app, std::mem::take(&mut done)).await;
        }
    }
    if !done.is_empty() {
        land(app, done).await;
    }
}

/// Put a batch into the store with one write, tell the page, and let the
/// covers out of flight. A batch the store could not take rests like a
/// failure that may pass.
async fn land(app: &AppHandle, results: Vec<(String, TintEntry)>) {
    let urls: Vec<String> = results.iter().map(|(url, _)| url.clone()).collect();
    let keys = tokio::task::spawn_blocking(move || history::add_tints(results)).await;
    {
        let mut m = memory();
        for url in &urls {
            m.in_flight.remove(url);
        }
        if !matches!(keys, Ok(Ok(_))) {
            for url in urls {
                m.transient.insert(url, Instant::now() + RETRY_TRANSIENT);
            }
        }
    }
    match keys {
        Ok(Ok(keys)) if !keys.is_empty() => history::tinted(app, keys),
        Ok(Err(e)) => log::warn!("[SongTints] colors could not be kept: {e}"),
        _ => {}
    }
}

// ------------------------------------------------------------------ fetch

/// Why a cover gave no colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Failure {
    /// May pass: tried again after `RETRY_TRANSIENT`.
    Transient,
    /// Will not pass: kept in the file, tried again after 30 days.
    Permanent,
}

/// What went wrong reading a cover's bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fetch {
    Status(u16),
    TooBig,
    OffHttps,
    Transport,
}

impl Fetch {
    fn failure(self) -> Failure {
        match self {
            Fetch::Status(status) => status_failure(status),
            Fetch::TooBig | Fetch::OffHttps => Failure::Permanent,
            Fetch::Transport => Failure::Transient,
        }
    }
}

/// A cover's HTTP error: gone for good on 404 and 410, else it may pass.
fn status_failure(status: u16) -> Failure {
    if matches!(status, 404 | 410) {
        Failure::Permanent
    } else {
        Failure::Transient
    }
}

fn is_https(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|u| u.scheme() == "https" && u.host_str().is_some())
}

/// Whether a declared length is already over the cap.
fn declared_too_big(length: Option<u64>) -> bool {
    length.is_some_and(|n| n > MAX_BYTES as u64)
}

/// Add `chunk` to `body`; false once the body would pass `cap`.
fn add_capped(body: &mut Vec<u8>, chunk: &[u8], cap: usize) -> bool {
    if body.len() + chunk.len() > cap {
        return false;
    }
    body.extend_from_slice(chunk);
    true
}

static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(FETCH_TIMEOUT)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.url().scheme() != "https" {
                attempt.error("a cover redirected off https")
            } else if attempt.previous().len() >= 5 {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .user_agent(format!(
            "StreamNook/{}",
            crate::services::client_identity::build_version()
        ))
        .build()
        .expect("Failed to build the cover client")
});

/// The bytes at `url`, at most `MAX_BYTES`.
async fn get_capped(url: &str) -> Result<Vec<u8>, Fetch> {
    let mut response = CLIENT.get(url).send().await.map_err(|e| {
        if e.is_redirect() {
            Fetch::OffHttps
        } else {
            Fetch::Transport
        }
    })?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(Fetch::Status(status));
    }
    if declared_too_big(response.content_length()) {
        return Err(Fetch::TooBig);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Fetch::Transport)? {
        if !add_capped(&mut body, &chunk, MAX_BYTES) {
            return Err(Fetch::TooBig);
        }
    }
    Ok(body)
}

/// A cover's bytes: Apple's 96 px copy when its host draws one, else (or
/// when it refuses that size) the cover as kept.
async fn fetch_cover(url: &str) -> Result<Vec<u8>, Failure> {
    if let Some(small) = history::thumb_of(url) {
        match get_capped(&small).await {
            Ok(bytes) => return Ok(bytes),
            Err(Fetch::Status(status)) if (400..500).contains(&status) => {}
            Err(e) => return Err(e.failure()),
        }
    }
    get_capped(url).await.map_err(Fetch::failure)
}

/// One cover's colors, within `FETCH_TIMEOUT`.
async fn tint_of(url: &str) -> Result<TintEntry, Failure> {
    if !is_https(url) {
        return Err(Failure::Permanent);
    }
    tokio::time::timeout(FETCH_TIMEOUT, read_cover(url))
        .await
        .unwrap_or(Err(Failure::Transient))
}

/// Fetch a cover and read its colors off the async runtime.
async fn read_cover(url: &str) -> Result<TintEntry, Failure> {
    let bytes = fetch_cover(url).await?;
    match tokio::task::spawn_blocking(move || tint_of_bytes(&bytes)).await {
        Ok(Ok(entry)) => Ok(entry),
        Ok(Err(_)) => Err(Failure::Permanent),
        Err(_) => Err(Failure::Transient),
    }
}

// ------------------------------------------------------------------ color

/// Why a cover's picture could not be read (both are for good).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CoverError {
    /// Over the decoder's size or memory limits.
    Limits,
    /// Not a picture this build reads.
    Decode,
}

/// The pixels of a 48x48 copy of the picture in `bytes`, leaving out the
/// see-through ones. One reader with limits: `decode` checks the declared
/// size and reserves the memory before it decodes anything.
fn sample(bytes: &[u8]) -> Result<Vec<[u8; 3]>, CoverError> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| CoverError::Decode)?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_EDGE);
    limits.max_image_height = Some(MAX_EDGE);
    limits.max_alloc = Some(MAX_ALLOC);
    reader.limits(limits);
    let picture = reader.decode().map_err(|e| match e {
        ImageError::Limits(_) => CoverError::Limits,
        _ => CoverError::Decode,
    })?;
    let small = picture.thumbnail(SAMPLE_EDGE, SAMPLE_EDGE).to_rgba8();
    Ok(small
        .pixels()
        .filter(|p| p.0[3] >= 128)
        .map(|p| [p.0[0], p.0[1], p.0[2]])
        .collect())
}

fn tint_of_bytes(bytes: &[u8]) -> Result<TintEntry, CoverError> {
    Ok(palette(&swatches(&sample(bytes)?)))
}

fn linear(c: f64) -> f64 {
    let v = c / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// OKLab of an sRGB color (0 to 255 per channel).
fn oklab([r, g, b]: [f64; 3]) -> [f64; 3] {
    let (r, g, b) = (linear(r), linear(g), linear(b));
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    [
        0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
    ]
}

fn lch_of(rgb: [f64; 3]) -> Lch {
    let [l, a, b] = oklab(rgb);
    let h = b.atan2(a).to_degrees();
    [l, a.hypot(b), if h < 0.0 { h + 360.0 } else { h }]
}

fn distance(x: [f64; 3], y: [f64; 3]) -> f64 {
    ((x[0] - y[0]).powi(2) + (x[1] - y[1]).powi(2) + (x[2] - y[2]).powi(2)).sqrt()
}

fn hue_gap(a: f64, b: f64) -> f64 {
    let d = (a - b).rem_euclid(360.0);
    if d > 180.0 {
        360.0 - d
    } else {
        d
    }
}

/// One of a picture's main colors: its mean color and its share of the
/// picture.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Swatch {
    rgb: [u8; 3],
    share: f64,
}

#[derive(Clone, Copy, Default)]
struct Bucket {
    n: u64,
    sum: [u64; 3],
    /// The first pixel in it, so equal counts keep the picture's order.
    first: usize,
}

impl Bucket {
    fn mean(&self) -> [f64; 3] {
        let n = self.n as f64;
        [
            self.sum[0] as f64 / n,
            self.sum[1] as f64 / n,
            self.sum[2] as f64 / n,
        ]
    }
}

/// A picture's main colors, most of the picture first: 4-bit buckets, most
/// filled first; each joins the first group whose first bucket is within
/// 0.09 in OKLab, else starts one (at most 18). Specks under 0.2% end the
/// walk. The 6 largest groups, with their mean colors and shares.
fn swatches(pixels: &[[u8; 3]]) -> Vec<Swatch> {
    let mut buckets = vec![Bucket::default(); 4096];
    for (i, p) in pixels.iter().enumerate() {
        let key = ((p[0] as usize >> 4) << 8) | ((p[1] as usize >> 4) << 4) | (p[2] as usize >> 4);
        let b = &mut buckets[key];
        if b.n == 0 {
            b.first = i;
        }
        b.n += 1;
        for c in 0..3 {
            b.sum[c] += p[c] as u64;
        }
    }
    let mut found: Vec<Bucket> = buckets.into_iter().filter(|b| b.n > 0).collect();
    found.sort_by(|a, b| b.n.cmp(&a.n).then(a.first.cmp(&b.first)));
    let total: u64 = found.iter().map(|b| b.n).sum();
    let mut groups: Vec<([f64; 3], Bucket)> = Vec::new();
    for bucket in found {
        if (bucket.n as f64) < total as f64 * 0.002 {
            break;
        }
        let seen = oklab(bucket.mean());
        match groups.iter().position(|(at, _)| distance(*at, seen) < 0.09) {
            Some(i) => {
                let group = &mut groups[i].1;
                group.n += bucket.n;
                for c in 0..3 {
                    group.sum[c] += bucket.sum[c];
                }
            }
            None if groups.len() < 18 => groups.push((seen, bucket)),
            None => {}
        }
    }
    let mut groups: Vec<Bucket> = groups.into_iter().map(|(_, g)| g).collect();
    groups.sort_by(|a, b| b.n.cmp(&a.n));
    groups
        .into_iter()
        .take(6)
        .map(|g| {
            let [r, gr, b] = g.mean();
            Swatch {
                rgb: [r.round() as u8, gr.round() as u8, b.round() as u8],
                share: g.n as f64 / total as f64,
            }
        })
        .collect()
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

fn rounded([l, c, h]: Lch) -> Lch {
    [round3(l), round3(c), round3(h)]
}

/// The cover's colors from its swatches. Colorful: chroma at least 0.02 and
/// lightness 0.12 to 0.97. `main` is the colorful swatch with the most
/// `sqrt(share) * (0.3 + 5 * chroma)` (none: a grey cover). `second` is the
/// best by the same score among colorful swatches with a share of at least
/// 0.03 and a hue at least 40 degrees from `main`.
fn palette(swatches: &[Swatch]) -> TintEntry {
    let seen: Vec<(Lch, f64)> = swatches
        .iter()
        .map(|s| {
            (
                lch_of([s.rgb[0] as f64, s.rgb[1] as f64, s.rgb[2] as f64]),
                s.share,
            )
        })
        .collect();
    let colorful: Vec<&(Lch, f64)> = seen
        .iter()
        .filter(|([l, c, _], _)| *c >= 0.02 && (0.12..=0.97).contains(l))
        .collect();
    let score = |(lch, share): &(Lch, f64)| share.sqrt() * (0.3 + lch[1] * 5.0);
    let best = |pool: Vec<&(Lch, f64)>| {
        pool.into_iter()
            .reduce(|a, b| if score(b) > score(a) { b } else { a })
            .map(|(lch, _)| *lch)
    };
    let Some(main) = best(colorful.clone()) else {
        return TintEntry::Color {
            main: None,
            second: None,
        };
    };
    let second = best(
        colorful
            .into_iter()
            .filter(|(lch, share)| *share >= 0.03 && hue_gap(lch[2], main[2]) >= 40.0)
            .collect(),
    );
    TintEntry::Color {
        main: Some(rounded(main)),
        second: second.map(rounded),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: [u8; 3] = [230, 30, 40];
    const BLUE: [u8; 3] = [30, 60, 220];
    const GREEN: [u8; 3] = [40, 180, 60];
    const ORANGE: [u8; 3] = [240, 140, 30];
    const GREY: [u8; 3] = [128, 128, 128];
    const SEPIA: [u8; 3] = [140, 128, 112];
    const PALE: [u8; 3] = [245, 240, 225];

    fn picture(parts: &[([u8; 3], usize)]) -> Vec<[u8; 3]> {
        parts
            .iter()
            .flat_map(|(rgb, n)| std::iter::repeat_n(*rgb, *n))
            .collect()
    }

    fn colors(parts: &[([u8; 3], usize)]) -> (Option<Lch>, Option<Lch>) {
        match palette(&swatches(&picture(parts))) {
            TintEntry::Color { main, second } => (main, second),
            other => panic!("{other:?}"),
        }
    }

    // ---------------------------------------------------------------- T1

    #[test]
    fn one_vivid_color_gives_its_hue() {
        let (main, second) = colors(&[(RED, 1000)]);
        let [l, c, h] = main.unwrap();
        assert!((h - 26.27).abs() < 0.1, "{h}");
        assert!(c > 0.2 && (0.5..0.7).contains(&l));
        assert_eq!(second, None);
        assert_eq!(main.unwrap(), rounded(main.unwrap()), "kept at 3 decimals");
    }

    #[test]
    fn a_grey_cover_has_no_color_and_sepia_has_one() {
        assert_eq!(colors(&[(GREY, 1000)]), (None, None));
        assert_eq!(
            colors(&[([20, 20, 20], 500), ([250, 250, 250], 500)]),
            (None, None)
        );
        let (main, _) = colors(&[(SEPIA, 1000)]);
        let [_, c, _] = main.expect("sepia is the cover's character");
        assert!((0.02..0.04).contains(&c), "{c}");
    }

    #[test]
    fn a_large_pale_area_loses_to_a_smaller_vivid_one() {
        let (main, _) = colors(&[(PALE, 700), (BLUE, 300)]);
        assert!((main.unwrap()[2] - 266.45).abs() < 0.1, "{main:?}");
    }

    #[test]
    fn the_second_color_needs_its_share_and_another_hue() {
        let (_, second) = colors(&[(RED, 960), (GREEN, 40)]);
        assert!((second.unwrap()[2] - 144.8).abs() < 0.1, "{second:?}");
        let (_, second) = colors(&[(RED, 980), (GREEN, 20)]);
        assert_eq!(second, None, "2% is too little");
        let (_, second) = colors(&[(RED, 800), (ORANGE, 200)]);
        assert_eq!(second, None, "orange is within 40 degrees of red");
    }

    #[test]
    fn specks_are_left_out_and_shares_add_up() {
        let s = swatches(&picture(&[(RED, 900), (BLUE, 99), (GREEN, 1)]));
        assert_eq!(s.len(), 2, "a 0.1% speck is no color");
        assert_eq!(s[0].rgb, RED);
        assert!((s[0].share - 0.9).abs() < 1e-9 && (s[1].share - 0.099).abs() < 1e-9);
        // Near colors join one group.
        let s = swatches(&picture(&[([230, 30, 40], 500), ([226, 34, 44], 500)]));
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].rgb, [228, 32, 42]);
    }

    #[test]
    fn a_real_picture_is_read_through_its_decoder() {
        let mut png = Vec::new();
        let img = image::RgbaImage::from_fn(64, 64, |x, _| {
            if x < 40 {
                image::Rgba([RED[0], RED[1], RED[2], 255])
            } else {
                image::Rgba([0, 0, 0, 0])
            }
        });
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let TintEntry::Color { main, .. } = tint_of_bytes(&png).unwrap() else {
            panic!("a color");
        };
        assert!(
            (main.unwrap()[2] - 26.27).abs() < 0.5,
            "see-through pixels are not black: {main:?}"
        );
    }

    // ---------------------------------------------------------------- T3

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &b in bytes {
            crc ^= b as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    0xEDB8_8320 ^ (crc >> 1)
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = (data.len() as u32).to_be_bytes().to_vec();
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
        out
    }

    /// A PNG that is only a header declaring `w` x `h`.
    fn png_header(w: u32, h: u32) -> Vec<u8> {
        let mut ihdr = w.to_be_bytes().to_vec();
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend(chunk(b"IHDR", &ihdr));
        png.extend(chunk(b"IEND", &[]));
        png
    }

    #[test]
    fn a_forged_huge_picture_is_refused_by_the_limits_before_decoding() {
        assert_eq!(sample(&png_header(20_000, 20_000)), Err(CoverError::Limits));
        assert_eq!(sample(&png_header(4096, 16)), Err(CoverError::Limits));
        assert_eq!(
            sample(&png_header(100, 100)),
            Err(CoverError::Decode),
            "a header within the limits gets as far as its missing data"
        );
        assert_eq!(sample(b"not a picture"), Err(CoverError::Decode));
    }

    #[test]
    fn only_https_covers_under_the_size_cap_are_read() {
        assert!(is_https(
            "https://is1-ssl.mzstatic.com/image/thumb/x.jpg/400x400cc.jpg"
        ));
        for url in [
            "http://x.example/a.jpg",
            "file:///C:/a.jpg",
            "data:image/png;base64,AA",
            "https:",
        ] {
            assert!(!is_https(url), "{url}");
        }
        assert!(!declared_too_big(Some(MAX_BYTES as u64)));
        assert!(declared_too_big(Some(MAX_BYTES as u64 + 1)));
        assert!(!declared_too_big(None));
        let mut body = Vec::new();
        assert!(add_capped(&mut body, &vec![0; MAX_BYTES - 10], MAX_BYTES));
        assert!(add_capped(&mut body, &[0; 10], MAX_BYTES));
        assert!(!add_capped(&mut body, &[0], MAX_BYTES));
        assert_eq!(body.len(), MAX_BYTES);
    }

    #[test]
    fn failures_that_may_pass_are_told_from_ones_that_will_not() {
        for status in [404, 410] {
            assert_eq!(status_failure(status), Failure::Permanent, "{status}");
        }
        for status in [403, 429, 500, 503, 301] {
            assert_eq!(status_failure(status), Failure::Transient, "{status}");
        }
        assert_eq!(Fetch::TooBig.failure(), Failure::Permanent);
        assert_eq!(Fetch::OffHttps.failure(), Failure::Permanent);
        assert_eq!(Fetch::Transport.failure(), Failure::Transient);
    }

    // --------------------------------------------------------- T3 and T20

    fn urls(m: &Memory) -> Vec<(&str, Priority)> {
        m.queue.iter().map(|(u, p)| (u.as_str(), *p)).collect()
    }

    fn list(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    use super::Priority::{High, Normal};

    #[test]
    fn the_queue_puts_priority_first_and_the_newest_request_ahead_of_older_ones() {
        let now = Instant::now();
        let mut m = Memory::default();
        m.request(list(&["a", "b"]), Normal, false, now);
        m.request(list(&["c", "d"]), Normal, false, now);
        m.request(list(&["p"]), High, false, now);
        assert_eq!(
            urls(&m),
            vec![
                ("p", High),
                ("c", Normal),
                ("d", Normal),
                ("a", Normal),
                ("b", Normal)
            ]
        );
        // Two requests never take out each other's covers; one asked again
        // moves rather than doubles.
        m.request(list(&["b", "e"]), Normal, false, now);
        assert_eq!(
            urls(&m),
            vec![
                ("p", High),
                ("b", Normal),
                ("e", Normal),
                ("c", Normal),
                ("d", Normal),
                ("a", Normal)
            ]
        );
        m.request(list(&["q", "q"]), High, false, now);
        assert_eq!(urls(&m)[..2], [("q", High), ("p", High)]);
        assert_eq!(m.queue.len(), 7);
    }

    #[test]
    fn a_queued_priority_cover_keeps_its_priority_when_asked_for_as_normal() {
        let now = Instant::now();
        let mut m = Memory::default();
        m.request(list(&["a"]), Normal, false, now);
        m.request(list(&["p"]), High, false, now);
        m.request(list(&["p", "b"]), Normal, false, now);
        assert_eq!(urls(&m), vec![("p", High), ("b", Normal), ("a", Normal)]);
        m.request(list(&["a"]), High, false, now);
        assert_eq!(urls(&m), vec![("a", High), ("p", High), ("b", Normal)]);
    }

    #[test]
    fn covers_in_flight_resting_or_while_tints_are_off_are_not_queued() {
        let now = Instant::now();
        let mut m = Memory::default();
        m.request(list(&["a"]), Normal, true, now);
        assert!(m.queue.is_empty(), "tints off queue nothing");
        m.request(list(&["a", "b"]), Normal, false, now);
        assert_eq!(m.take(), Some(("a".to_string(), Normal)));
        m.transient.insert("t".into(), now + RETRY_TRANSIENT);
        m.request(list(&["a", "t", "c", ""]), High, false, now);
        assert_eq!(urls(&m), vec![("c", High), ("b", Normal)]);
        // Once its rest is over, a cover can be asked for again.
        m.request(list(&["t"]), Normal, false, now + RETRY_TRANSIENT);
        assert!(urls(&m).contains(&("t", Normal)));
        assert!(m.transient.is_empty());
    }

    #[test]
    fn past_the_cap_the_oldest_normal_covers_go() {
        let now = Instant::now();
        let mut m = Memory::default();
        let first: Vec<String> = (0..150).map(|i| format!("old{i}")).collect();
        let second: Vec<String> = (0..100).map(|i| format!("new{i}")).collect();
        m.request(first, Normal, false, now);
        m.request(list(&["p"]), High, false, now);
        m.request(second, Normal, false, now);
        assert_eq!(m.queue.len(), QUEUE_CAP);
        assert_eq!(urls(&m)[0], ("p", High));
        assert_eq!(m.queue[1].0, "new0");
        assert_eq!(m.queue.back().unwrap().0, "old98", "old99 and later went");
    }

    // ---------------------------------------------------------- T2 and T30

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sn-song-tints-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_map() -> BTreeMap<String, TintEntry> {
        let mut map = BTreeMap::new();
        map.insert(
            "https://c/a.jpg".to_string(),
            TintEntry::Color {
                main: Some([0.591, 0.228, 26.27]),
                second: Some([0.674, 0.198, 144.8]),
            },
        );
        map.insert(
            "https://c/grey.jpg".to_string(),
            TintEntry::Color {
                main: None,
                second: None,
            },
        );
        map.insert(
            "https://c/gone.jpg".to_string(),
            TintEntry::Failed { failed_ms: 12 },
        );
        map
    }

    #[test]
    fn the_tint_file_reads_back_what_it_wrote() {
        let dir = temp_dir("roundtrip");
        let path = dir.join(FILE);
        assert_eq!(
            read_tints(&path),
            (BTreeMap::new(), false),
            "a missing file is empty"
        );
        write_tints_file(&path, &sample_map(), false).unwrap();
        assert_eq!(read_tints(&path), (sample_map(), false));
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(r#"{"version":1,"tints":{"#), "{text}");
        assert!(
            text.contains(r#""https://c/grey.jpg":{"main":null,"second":null}"#),
            "{text}"
        );
        assert!(
            text.contains(r#""https://c/gone.jpg":{"failed_ms":12}"#),
            "{text}"
        );
        // An entry this build cannot read is left out, the rest kept.
        fs::write(
            &path,
            r#"{"version":1,"tints":{"https://c/a.jpg":{"main":[0.5,0.1,10],"second":null},"https://c/b.jpg":{"main":[9,9,9],"second":null},"https://c/c.jpg":"x"}}"#,
        )
        .unwrap();
        let (map, off) = read_tints(&path);
        assert!(!off);
        assert_eq!(map.keys().collect::<Vec<_>>(), vec!["https://c/a.jpg"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_newer_apps_tint_file_is_read_and_never_written_or_moved() {
        let dir = temp_dir("newer");
        let path = dir.join(FILE);
        let text = r#"{"version":2,"tints":{"https://c/a.jpg":{"main":[0.5,0.1,10],"second":null,"glow":1}}}"#;
        fs::write(&path, text).unwrap();
        let (map, off) = read_tints(&path);
        assert!(off, "tints are off for this load");
        assert_eq!(map.len(), 1);
        assert!(
            write_tints_file(&path, &map, off).is_err(),
            "the writer refuses"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), text);

        // Damaged, but its version head says a newer app wrote it: left as
        // it is.
        let damaged = r#"{"version":3,"tints":"not a map"}"#;
        fs::write(&path, damaged).unwrap();
        assert_eq!(read_tints(&path), (BTreeMap::new(), true));
        assert_eq!(fs::read_to_string(&path).unwrap(), damaged);
        let names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec![FILE.to_string()], "never moved aside");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failure_is_retried_after_thirty_days() {
        let failed = TintEntry::Failed { failed_ms: 1_000 };
        assert!(usable(Some(&failed), 1_000 + RETRY_PERMANENT_MS - 1));
        assert!(!usable(Some(&failed), 1_000 + RETRY_PERMANENT_MS));
        assert!(!usable(None, 0));
        assert!(usable(
            Some(&TintEntry::Color {
                main: None,
                second: None
            }),
            i64::MAX
        ));
        assert_eq!(view(None), None);
        assert_eq!(view(Some(&failed)), Some(TintView::GREY));
    }
}
