//! A channel's videos and clips as ready-to-draw rows, for the phones' Channel
//! sheet. One door over the two GQL lists: the videos already carry the
//! viewer's saved position (`vod_progress_service::attach`), and every label a
//! row shows (length, age, views, who clipped it) is worded here, so both
//! phones read the same words and never format a number themselves.

use crate::models::stream::{TwitchClip, TwitchVideo};
use crate::services::twitch_service::TwitchService;
use serde::Serialize;

/// Rows per page.
const PAGE: u32 = 20;
/// A saved position this close to the end counts as watched.
const WATCHED_FRACTION: f64 = 0.95;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MediaRow {
    /// "video" or "clip".
    pub kind: &'static str,
    /// The video id or the clip slug.
    pub id: String,
    pub title: String,
    pub thumbnail_url: String,
    /// What `start_stream` (videos) or the clip sheet (clips) opens.
    pub url: String,
    pub length_label: String,
    pub age_label: String,
    pub views_label: String,
    /// The category (videos), or "Clipped by <name>" (clips).
    pub byline: Option<String>,
    /// How far the viewer got, 0 to 1; none when never opened.
    pub progress: Option<f64>,
    pub watched: bool,
    /// The broadcast is still live, so its video is still growing.
    pub recording: bool,
    /// A clip's parent video and where in it the clip starts, for "Watch full broadcast".
    pub vod_id: Option<String>,
    pub vod_offset: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaPage {
    pub rows: Vec<MediaRow>,
    pub cursor: Option<String>,
}

/// `kind` "videos" takes `filter` archive | highlight | upload and `sort` recent | popular;
/// "clips" takes `filter` 24h | 7d | 30d | all.
pub async fn page(
    channel_id: &str,
    kind: &str,
    filter: Option<&str>,
    sort: Option<&str>,
    cursor: Option<&str>,
) -> anyhow::Result<MediaPage> {
    let now = chrono::Utc::now().timestamp_millis();
    if kind == "clips" {
        let (clips, cursor) =
            TwitchService::get_clips_by_broadcaster(channel_id, PAGE, cursor, Some(filter.unwrap_or("30d"))).await?;
        return Ok(MediaPage { rows: clips.iter().map(|c| clip_row(c, now)).collect(), cursor });
    }
    let sort = if sort == Some("popular") { "views" } else { "time" };
    let (mut videos, cursor) =
        TwitchService::get_user_videos(channel_id, sort, Some(filter.unwrap_or("archive")), PAGE, cursor).await?;
    crate::services::vod_progress_service::attach(&mut videos);
    Ok(MediaPage { rows: videos.iter().map(|v| video_row(v, now)).collect(), cursor })
}

fn video_row(v: &TwitchVideo, now_ms: i64) -> MediaRow {
    let length = v.length_seconds.map(f64::from).unwrap_or(0.0);
    let progress = v.progress.as_ref().and_then(|p| {
        let total = if p.duration_secs > 0.0 { p.duration_secs } else { length };
        (total > 0.0).then(|| (p.position_secs / total).clamp(0.0, 1.0))
    });
    let watched = v.progress.as_ref().is_some_and(|p| p.completed) || progress.is_some_and(|f| f >= WATCHED_FRACTION);
    MediaRow {
        kind: "video",
        id: v.id.clone(),
        title: v.title.clone(),
        thumbnail_url: v.thumbnail_url.clone(),
        url: format!("https://www.twitch.tv/videos/{}", v.id),
        length_label: video_length(length),
        age_label: age(&v.created_at, now_ms),
        views_label: views(u64::from(v.view_count)),
        byline: v.game_name.clone().filter(|g| !g.is_empty()),
        progress: if watched { Some(1.0) } else { progress },
        watched,
        recording: v.status.as_deref() == Some("recording"),
        vod_id: None,
        vod_offset: None,
    }
}

fn clip_row(c: &TwitchClip, now_ms: i64) -> MediaRow {
    MediaRow {
        kind: "clip",
        id: c.id.clone(),
        title: c.title.clone(),
        thumbnail_url: c.thumbnail_url.clone(),
        url: c.url.clone(),
        length_label: clip_length(f64::from(c.duration)),
        age_label: age(&c.created_at, now_ms),
        views_label: views(u64::from(c.view_count)),
        byline: Some(c.creator_name.clone()).filter(|n| !n.is_empty()).map(|n| format!("Clipped by {n}")),
        progress: None,
        watched: false,
        recording: false,
        vod_id: Some(c.video_id.clone()).filter(|id| !id.is_empty()),
        vod_offset: c.vod_offset,
    }
}

/// "4h 12m", "47m", "0:45".
pub fn video_length(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as u64;
    match (s / 3600, (s % 3600) / 60) {
        (0, 0) => format!("0:{:02}", s),
        (0, m) => format!("{m}m"),
        (h, m) => format!("{h}h {m:02}m"),
    }
}

/// "0:28", "1:05".
pub fn clip_length(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// "812K views", "1.1M views", "1 view".
pub fn views(n: u64) -> String {
    let number = match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => compact(n as f64 / 1_000.0, "K"),
        _ => compact(n as f64 / 1_000_000.0, "M"),
    };
    format!("{number} {}", if n == 1 { "view" } else { "views" })
}

/// One decimal under ten ("1.1M"), whole numbers above ("812K").
fn compact(x: f64, unit: &str) -> String {
    if x < 9.95 {
        let s = format!("{x:.1}");
        format!("{}{unit}", s.trim_end_matches(".0"))
    } else {
        // "999K" rather than "1000K" at the top of the thousands.
        let whole = x.round() as u64;
        format!("{}{unit}", if unit == "K" { whole.min(999) } else { whole })
    }
}

/// "just now", "5 minutes ago", "yesterday", "3 weeks ago", "2 years ago".
pub fn age(rfc3339: &str, now_ms: i64) -> String {
    let Ok(at) = chrono::DateTime::parse_from_rfc3339(rfc3339) else {
        return String::new();
    };
    age_ms(at.timestamp_millis(), now_ms)
}

/// `age` from Unix milliseconds.
pub fn age_ms(at_ms: i64, now_ms: i64) -> String {
    let secs = ((now_ms - at_ms) / 1000).max(0);
    let (n, unit) = match secs {
        0..=59 => return "just now".to_string(),
        60..=3_599 => (secs / 60, "minute"),
        3_600..=86_399 => (secs / 3_600, "hour"),
        86_400..=172_799 => return "yesterday".to_string(),
        172_800..=604_799 => (secs / 86_400, "day"),
        604_800..=2_591_999 => (secs / 604_800, "week"),
        2_592_000..=31_535_999 => (secs / 2_592_000, "month"),
        _ => (secs / 31_536_000, "year"),
    };
    format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths() {
        assert_eq!(video_length(15_120.0), "4h 12m");
        assert_eq!(video_length(3_600.0), "1h 00m");
        assert_eq!(video_length(2_820.0), "47m");
        assert_eq!(video_length(45.0), "0:45");
        assert_eq!(clip_length(28.4), "0:28");
        assert_eq!(clip_length(65.0), "1:05");
    }

    #[test]
    fn view_counts() {
        assert_eq!(views(1), "1 view");
        assert_eq!(views(999), "999 views");
        assert_eq!(views(1_234), "1.2K views");
        assert_eq!(views(9_960), "10K views");
        assert_eq!(views(999_700), "999K views");
        assert_eq!(views(812_400), "812K views");
        assert_eq!(views(1_100_000), "1.1M views");
        assert_eq!(views(2_000_000), "2M views");
    }

    #[test]
    fn ages() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-18T12:00:00Z").unwrap().timestamp_millis();
        assert_eq!(age("2026-10-18T11:59:30Z", now), "just now");
        assert_eq!(age("2026-10-18T11:59:00Z", now), "1 minute ago");
        assert_eq!(age("2026-10-18T07:00:00Z", now), "5 hours ago");
        assert_eq!(age("2026-10-17T10:00:00Z", now), "yesterday");
        assert_eq!(age("2026-10-14T12:00:00Z", now), "4 days ago");
        assert_eq!(age("2026-09-27T12:00:00Z", now), "3 weeks ago");
        assert_eq!(age("2026-07-18T12:00:00Z", now), "3 months ago");
        assert_eq!(age("2024-10-18T12:00:00Z", now), "2 years ago");
        assert_eq!(age("garbage", now), "");
    }
}
