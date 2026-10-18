//! "Your clips": the clips this account made in StreamNook, newest first.
//!
//! Twitch has no list of the clips a viewer made across channels, so the core
//! keeps its own: `clip_create` records each clip it makes here, and the phones'
//! "Your clips" screens read `my_clips_list`. One JSON file per signed-in account
//! (the same owner as Continue Watching), a shared `anon` file when signed out.
//! Local only; nothing syncs. Bounded at `CAP` clips.
//!
//! A clip renders for a few seconds after it is made, so its thumbnail arrives
//! later (`set_thumbnail`, from the render poll in `clip_create`).

use crate::services::cache_service::get_app_data_dir;
use crate::services::vod_progress_service;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

const STORE_DIR: &str = "my_clips";
const CAP: usize = 300;

/// Serializes reads and writes of the account file.
static LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MyClip {
    /// Twitch's clip slug.
    pub slug: String,
    pub title: String,
    pub channel_id: String,
    pub channel_login: String,
    pub channel_name: String,
    /// Unix milliseconds.
    pub made_at: i64,
    pub duration_seconds: f64,
    /// "live" or "vod".
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
}

impl MyClip {
    /// The link people open: the channel's own clip page.
    pub fn url(&self) -> String {
        clip_url(&self.channel_login, &self.slug)
    }
}

pub fn clip_url(login: &str, slug: &str) -> String {
    if login.is_empty() {
        format!("https://clips.twitch.tv/{slug}")
    } else {
        format!("https://www.twitch.tv/{}/clip/{slug}", login.to_lowercase())
    }
}

fn storage_path() -> Result<PathBuf, String> {
    let owner = vod_progress_service::owner_id().unwrap_or_else(|| "anon".to_string());
    let dir = get_app_data_dir()
        .map_err(|e| format!("Failed to resolve app data dir: {e}"))?
        .join(STORE_DIR);
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create my_clips directory: {e}"))?;
    Ok(dir.join(format!("{owner}.json")))
}

fn read(path: &PathBuf) -> Vec<MyClip> {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write(path: &PathBuf, clips: &[MyClip]) -> Result<(), String> {
    let json = serde_json::to_string(clips).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Put a new clip first; a slug already listed is replaced, and the list keeps `CAP`.
fn insert(clips: &mut Vec<MyClip>, clip: MyClip) {
    clips.retain(|c| c.slug != clip.slug);
    clips.insert(0, clip);
    clips.truncate(CAP);
}

pub fn record(clip: MyClip) -> Result<(), String> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = storage_path()?;
    let mut clips = read(&path);
    insert(&mut clips, clip);
    write(&path, &clips)
}

/// Newest first.
pub fn list() -> Vec<MyClip> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    storage_path().map(|p| read(&p)).unwrap_or_default()
}

pub fn set_thumbnail(slug: &str, url: &str) -> Result<(), String> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = storage_path()?;
    let mut clips = read(&path);
    let Some(clip) = clips.iter_mut().find(|c| c.slug == slug) else {
        return Ok(());
    };
    clip.thumbnail_url = Some(url.to_string());
    write(&path, &clips)
}

pub fn forget(slug: &str) -> Result<(), String> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = storage_path()?;
    let mut clips = read(&path);
    clips.retain(|c| c.slug != slug);
    write(&path, &clips)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(slug: &str) -> MyClip {
        MyClip {
            slug: slug.to_string(),
            title: "t".to_string(),
            channel_id: "1".to_string(),
            channel_login: "Jynxzi".to_string(),
            channel_name: "Jynxzi".to_string(),
            made_at: 0,
            duration_seconds: 30.0,
            source: "live".to_string(),
            thumbnail_url: None,
        }
    }

    #[test]
    fn newest_first_without_repeats() {
        let mut clips = vec![clip("a"), clip("b")];
        insert(&mut clips, clip("b"));
        assert_eq!(clips.iter().map(|c| c.slug.as_str()).collect::<Vec<_>>(), ["b", "a"]);
    }

    #[test]
    fn the_list_keeps_its_cap() {
        let mut clips = Vec::new();
        for i in 0..CAP + 5 {
            insert(&mut clips, clip(&i.to_string()));
        }
        assert_eq!(clips.len(), CAP);
        assert_eq!(clips[0].slug, (CAP + 4).to_string());
    }

    #[test]
    fn links_open_the_channel_clip_page() {
        assert_eq!(clip("Abc").url(), "https://www.twitch.tv/jynxzi/clip/Abc");
        assert_eq!(clip_url("", "Abc"), "https://clips.twitch.tv/Abc");
    }
}
