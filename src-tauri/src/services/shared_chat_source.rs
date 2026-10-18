//! Where a Twitch Shared Chat row came from, worked out once per source room:
//! the partner channel's login, name, face and its own color, so every chat
//! surface marks a partner's rows the same way (the channel's face leads the
//! row, its color edges it) and no view looks users up itself.
//!
//! One anonymous GQL read per room id (`user(id:)` with `primaryColorHex`, the
//! color the channel picked on Twitch), cached for the session; a channel with
//! no color gets a stable hue from its id. The color is served raw and floored
//! per theme by `floored`, so a pale channel color still reads on a light
//! theme and a dark one on a dark theme.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::services::twitch_service::TwitchService;

/// A failed lookup is retried after this long, not on every row.
const RETRY_AFTER: Duration = Duration::from_secs(60);

/// The channel a partner's row came from.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SharedSource {
    pub id: String,
    pub login: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// The channel's color as `#rrggbb`, floored for the theme it is served to.
    pub color: String,
}

enum Entry {
    Known(SharedSource),
    Failed(Instant),
}

fn cache() -> &'static Mutex<HashMap<String, Entry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The partner channel behind `room_id`, its color floored for a dark or light
/// theme. None while Twitch cannot say who it is.
pub async fn source(room_id: &str, dark: bool) -> Option<SharedSource> {
    if room_id.is_empty() {
        return None;
    }
    {
        let cache = cache().lock().await;
        match cache.get(room_id) {
            Some(Entry::Known(found)) => return Some(themed(found.clone(), dark)),
            Some(Entry::Failed(at)) if at.elapsed() < RETRY_AFTER => return None,
            _ => {}
        }
    }
    let read = lookup(room_id).await;
    let mut cache = cache().lock().await;
    match read {
        Some(found) => {
            cache.insert(room_id.to_string(), Entry::Known(found.clone()));
            Some(themed(found, dark))
        }
        None => {
            cache.insert(room_id.to_string(), Entry::Failed(Instant::now()));
            None
        }
    }
}

/// The partner channel behind `room_id` if it is already known, without
/// waiting: for code that stamps a row as it arrives. A room not yet known is
/// looked up in the background, so its later rows carry it.
pub fn cached(room_id: &str, dark: bool) -> Option<SharedSource> {
    if room_id.is_empty() {
        return None;
    }
    if let Ok(cache) = cache().try_lock() {
        match cache.get(room_id) {
            Some(Entry::Known(found)) => return Some(themed(found.clone(), dark)),
            Some(Entry::Failed(at)) if at.elapsed() < RETRY_AFTER => return None,
            _ => {}
        }
    }
    warm(vec![room_id.to_string()]);
    None
}

/// Looks the given rooms up in the background (a Shared Chat session's
/// partners as the chat joins it), so their first rows already carry them.
pub fn warm(room_ids: Vec<String>) {
    if room_ids.is_empty() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        for id in room_ids {
            let _ = source(&id, true).await;
        }
    });
}

fn themed(mut found: SharedSource, dark: bool) -> SharedSource {
    found.color = floored(&found.color, dark);
    found
}

async fn lookup(room_id: &str) -> Option<SharedSource> {
    let body = serde_json::json!({
        "operationName": "StreamNookSharedSource",
        "query": "query StreamNookSharedSource($id: ID!) { user(id: $id) { id login displayName profileImageURL(width: 70) primaryColorHex } }",
        "variables": { "id": room_id },
    });
    let resp = TwitchService::gql_public_read(body)
        .await
        .map_err(|e| log::debug!("[SharedSource] {room_id}: {e}"))
        .ok()?;
    from_answer(room_id, &resp)
}

/// The source from Twitch's answer, its color raw (or the id's stable hue).
fn from_answer(room_id: &str, resp: &serde_json::Value) -> Option<SharedSource> {
    let user = resp.pointer("/data/user").filter(|u| u.is_object())?;
    let text = |key: &str| user.get(key).and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(str::to_string);
    let login = text("login")?;
    let color = text("primaryColorHex")
        .and_then(|hex| parse_hex(&hex))
        .map(|(r, g, b)| hex_of(r, g, b))
        .unwrap_or_else(|| stable_color(room_id));
    Some(SharedSource {
        id: text("id").unwrap_or_else(|| room_id.to_string()),
        display_name: text("displayName").unwrap_or_else(|| login.clone()),
        login,
        avatar_url: text("profileImageURL"),
        color,
    })
}

/// A hue picked from the id, the same every time, at a saturation and
/// lightness that read as a channel color on both themes before flooring.
pub fn stable_color(id: &str) -> String {
    // FNV-1a: stable across runs and platforms, unlike the std hasher.
    let mut h: u32 = 0x811c9dc5;
    for b in id.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    let (r, g, b) = hsl_to_rgb((h % 360) as f64, 0.65, 0.55);
    hex_of(r, g, b)
}

/// `hex` moved toward white (dark theme) or black (light theme) until it has
/// at least 3:1 contrast against that theme's chat background, so a channel's
/// edge and ring always read. Unreadable input comes back as given.
pub fn floored(hex: &str, dark: bool) -> String {
    let Some((r, g, b)) = parse_hex(hex) else { return hex.to_string() };
    let bg = if dark { (14, 14, 16) } else { (247, 247, 248) };
    let target = if dark { 255.0 } else { 0.0 };
    let (mut fr, mut fg, mut fb) = (r as f64, g as f64, b as f64);
    for _ in 0..40 {
        if contrast((fr, fg, fb), bg) >= 3.0 {
            break;
        }
        fr += (target - fr) * 0.1;
        fg += (target - fg) * 0.1;
        fb += (target - fb) * 0.1;
    }
    hex_of(fr.round() as u8, fg.round() as u8, fb.round() as u8)
}

fn parse_hex(hex: &str) -> Option<(u8, u8, u8)> {
    let h = hex.trim().trim_start_matches('#');
    if h.len() != 6 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let n = u32::from_str_radix(h, 16).ok()?;
    Some(((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

fn hex_of(r: u8, g: u8, b: u8) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn luminance((r, g, b): (f64, f64, f64)) -> f64 {
    let lin = |c: f64| {
        let c = c / 255.0;
        if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

fn contrast(fg: (f64, f64, f64), bg: (u8, u8, u8)) -> f64 {
    let a = luminance(fg);
    let b = luminance((bg.0 as f64, bg.1 as f64, bg.2 as f64));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match h as u32 {
        0..=59 => (c, x, 0.0),
        60..=119 => (x, c, 0.0),
        120..=179 => (0.0, c, x),
        180..=239 => (0.0, x, c),
        240..=299 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let to = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (to(r), to(g), to(b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_answer_gives_the_channel_and_its_own_color() {
        let resp = json!({ "data": { "user": {
            "id": "110690086", "login": "myth", "displayName": "Myth",
            "profileImageURL": "https://static-cdn.jtvnw.net/x-70x70.png", "primaryColorHex": "1F69FF"
        } } });
        let s = from_answer("110690086", &resp).unwrap();
        assert_eq!((s.login.as_str(), s.display_name.as_str(), s.color.as_str()), ("myth", "Myth", "#1f69ff"));
        assert_eq!(s.avatar_url.as_deref(), Some("https://static-cdn.jtvnw.net/x-70x70.png"));
    }

    #[test]
    fn no_color_takes_the_ids_stable_hue_and_no_user_is_none() {
        let resp = json!({ "data": { "user": { "id": "42", "login": "keem", "primaryColorHex": null } } });
        let s = from_answer("42", &resp).unwrap();
        assert_eq!(s.color, stable_color("42"));
        assert_eq!(stable_color("42"), stable_color("42"));
        assert_ne!(stable_color("42"), stable_color("43"));
        assert_eq!(s.display_name, "keem");
        assert!(from_answer("7", &json!({ "data": { "user": null } })).is_none());
    }

    #[test]
    fn the_floor_gives_three_to_one_on_both_themes() {
        for hex in ["#1f69ff", "#000000", "#202020", "#ffffff", "#f0e68c", "#ff8a3d", &stable_color("9")] {
            for dark in [true, false] {
                let out = floored(hex, dark);
                let (r, g, b) = parse_hex(&out).unwrap();
                let bg = if dark { (14, 14, 16) } else { (247, 247, 248) };
                assert!(contrast((r as f64, g as f64, b as f64), bg) >= 3.0, "{hex} dark={dark} -> {out}");
            }
        }
        // A color that already reads is left alone.
        assert_eq!(floored("#57b8ff", true), "#57b8ff");
        assert_eq!(floored("nope", true), "nope");
    }
}
