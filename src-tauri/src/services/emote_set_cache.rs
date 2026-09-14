//! On-disk per-channel emote dictionary cache.
//!
//! The emote IMAGE bytes are cached elsewhere (universal_cache_service), keyed by
//! emote id. That cache cannot tell chat "this word is an emote": it has no names
//! and no per-channel grouping. This module persists the per-channel name->emote
//! dictionary (the whole EmoteSet) so the chat tokenizer can load disk-first. The
//! payoff: chat recognizes a channel's emotes instantly on join with no network
//! round-trip, keeps working while 7TV is slow or down, and survives restarts.
//! It is the dictionary counterpart to the prefetch's image cache, and mirrors
//! Chatterino7's readProviderEmotesCache / writeProviderEmotesCache.
//!
//! Files live at <cache_dir>/emote_sets/<channel_id>.json, one per channel.

use anyhow::{Context, Result};
use log::{debug, warn};
use once_cell::sync::Lazy;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::services::cache_service::get_cache_dir;
use crate::services::emote_service::EmoteSet;

/// Format version of the stored dictionary. Bumped when the composition rule
/// changes in a way that makes every existing file wrong, so `save`'s
/// don't-shrink guard steps aside and the first authoritative fetch rewrites
/// them. Old files still LOAD (a stale seed beats none for the seconds until
/// that fetch lands); they just stop being protected.
///
/// - 2 (2026-09-07): 7TV rows are the channel set plus globals, keyed by name.
///   Version 1 files carry ~250 trending rows that were never the channel's and
///   are missing every channel alias the trending or global row won by id, so
///   they are strictly larger AND strictly wrong, which is exactly the case the
///   don't-shrink guard would otherwise defend forever.
const DICTIONARY_VERSION: u32 = 2;

// Process-lifetime map of sanitized channel id -> (7TV emote count, format
// version) of the stored set, so save()'s don't-shrink check does not need to
// re-read and re-parse the whole file (thousands of emotes) on every save.
// Seeded as sets pass through load()/write_set(); a miss falls back to one load().
static SEVEN_TV_COUNTS: Lazy<Mutex<HashMap<String, (usize, u32)>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn record_count(channel_id: &str, count: usize, version: u32) {
    let key = sanitize_id(channel_id);
    if key.is_empty() {
        return;
    }
    if let Ok(mut map) = SEVEN_TV_COUNTS.lock() {
        map.insert(key, (count, version));
    }
}

fn known_count(channel_id: &str) -> Option<(usize, u32)> {
    let key = sanitize_id(channel_id);
    if key.is_empty() {
        return None;
    }
    SEVEN_TV_COUNTS.lock().ok().and_then(|m| m.get(&key).copied())
}

/// Wrapper persisted to disk: the set, when it was written, and the format
/// version it was written under. `set` round-trips through EmoteSet's own
/// serde, so the 7TV array stays under the "7tv" key exactly like the live API
/// shape. Files written before versioning deserialize as version 0.
#[derive(Deserialize)]
struct StoredEmoteSet {
    #[allow(dead_code)]
    saved_at: u64,
    #[serde(default)]
    version: u32,
    set: EmoteSet,
}

fn store_dir() -> Result<PathBuf> {
    let dir = get_cache_dir()?.join("emote_sets");
    if !dir.exists() {
        fs::create_dir_all(&dir).context("Failed to create emote_sets cache directory")?;
    }
    Ok(dir)
}

/// channel_id is a Twitch numeric user id, but sanitize defensively so a
/// malformed id can never escape the directory. Also keys SEVEN_TV_COUNTS, so
/// two raw ids mapping to the same file share one count.
fn sanitize_id(channel_id: &str) -> String {
    channel_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect()
}

/// Path for a channel's dictionary file.
fn path_for(channel_id: &str) -> Result<PathBuf> {
    let safe = sanitize_id(channel_id);
    if safe.is_empty() {
        anyhow::bail!("empty/invalid channel id");
    }
    Ok(store_dir()?.join(format!("{}.json", safe)))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn read_stored(channel_id: &str) -> Option<StoredEmoteSet> {
    let path = path_for(channel_id).ok()?;
    let bytes = match fs::read(&path) {
        Ok(b) => b,
        Err(_) => return None, // no file yet, the normal cold path
    };
    match serde_json::from_slice::<StoredEmoteSet>(&bytes) {
        Ok(stored) => {
            record_count(channel_id, stored.set.seven_tv.len(), stored.version);
            Some(stored)
        }
        Err(e) => {
            warn!(
                "[EmoteSetCache] failed to parse {} (treating as absent): {}",
                path.display(),
                e
            );
            None
        }
    }
}

/// Load a channel's stored emote set, if present. Returns it regardless of age
/// or format version: a stale dictionary is far better than none for chat
/// recognition, and callers refresh in the background. Returns None when there
/// is no file or it can't be parsed (a corrupt file is treated as absent and
/// gets overwritten on next save).
pub fn load(channel_id: &str) -> Option<EmoteSet> {
    read_stored(channel_id).map(|stored| stored.set)
}

/// Persist a channel's emote set, but NEVER let a deficient fetch shrink a good
/// stored set. If a current-format file already exists with MORE 7TV emotes
/// than `set`, keep the existing one. A failed or partial live fetch
/// (globals-only from a tripped circuit breaker, or a timed-out channel set)
/// therefore cannot overwrite a healthy dictionary. Growth (a streamer added
/// emotes), first writes, and rewrites of an OLDER-format file always go
/// through. Use this when the set's completeness is uncertain (e.g. the AFK
/// prefetch). For a fetch known to be authoritative, use [`save_force`].
pub fn save(channel_id: &str, set: &EmoteSet) {
    // Consult the process-lifetime count map; only a miss pays the full file
    // read + parse (which records the count for next time).
    let existing = match known_count(channel_id) {
        Some(c) => Some(c),
        None => read_stored(channel_id).map(|s| (s.set.seven_tv.len(), s.version)),
    };
    if let Some((existing_count, existing_version)) = existing {
        if existing_version >= DICTIONARY_VERSION && set.seven_tv.len() < existing_count {
            debug!(
                "[EmoteSetCache] keeping stored set for {} (7TV {} >= incoming {}), not shrinking",
                channel_id,
                existing_count,
                set.seven_tv.len()
            );
            return;
        }
    }
    write_set(channel_id, set);
}

/// Persist a channel's emote set unconditionally, even if it is smaller than what
/// is on disk. Use ONLY when the caller knows the fetch was authoritative (the
/// 7TV channel endpoint definitively answered), so a legitimate emote removal is
/// written through instead of being blocked by the don't-shrink guard.
pub fn save_force(channel_id: &str, set: &EmoteSet) {
    write_set(channel_id, set);
}

fn write_set(channel_id: &str, set: &EmoteSet) {
    let path = match path_for(channel_id) {
        Ok(p) => p,
        Err(e) => {
            warn!("[EmoteSetCache] skip save, bad channel id {}: {}", channel_id, e);
            return;
        }
    };
    // Serialize by reference (no clone of the set) via a transient JSON value.
    let value = serde_json::json!({
        "saved_at": now_secs(),
        "version": DICTIONARY_VERSION,
        "set": set
    });
    match serde_json::to_vec(&value) {
        Ok(bytes) => {
            if let Err(e) = fs::write(&path, &bytes) {
                warn!("[EmoteSetCache] failed to write {}: {}", path.display(), e);
            } else {
                record_count(channel_id, set.seven_tv.len(), DICTIONARY_VERSION);
                debug!(
                    "[EmoteSetCache] saved {} ({} 7TV emotes, {} bytes)",
                    channel_id,
                    set.seven_tv.len(),
                    bytes.len()
                );
            }
        }
        Err(e) => warn!("[EmoteSetCache] failed to serialize {}: {}", channel_id, e),
    }
}
