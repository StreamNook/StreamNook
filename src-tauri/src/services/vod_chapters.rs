//! Chapters of a Twitch VOD: the category changes over a broadcast, which GQL
//! reports through `video.moments(momentRequestType: VIDEO_CHAPTER_MARKERS)`
//! as nodes of `positionMilliseconds` / `durationMilliseconds` / `description`
//! plus the game (id, box-art template). Probed live 2026-09-21.
//!
//! Two facts shape this module.
//!
//! 1. **The chapter still running on a recording VOD has duration 0.** It is
//!    open-ended, so it runs to the VOD length (or is dropped when no length
//!    is known yet). Every chapter is also clamped to the next one's start.
//! 2. **No chapters is not "unknown".** A recording with no category change
//!    and every highlight/upload return an empty list; a single chapter says
//!    nothing a card does not, so the UI only draws from two.

use serde::Serialize;

/// Twitch's box-art URL template resolved to the size the chapter list draws
/// (3:4; 52x72 answers 200, probed 2026-09-21).
const BOX_ART_SIZE: (&str, &str) = ("52", "72");
const MAX_CHAPTERS: usize = 200;

/// One category run, in seconds from the VOD start.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Chapter {
    pub start_secs: f64,
    pub end_secs: f64,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub box_art_url: Option<String>,
}

/// One `moments` node straight off GQL (milliseconds).
#[derive(Debug, Clone, Default)]
pub struct ChapterNode {
    pub position_ms: i64,
    pub duration_ms: i64,
    pub title: String,
    pub game_id: Option<String>,
    pub box_art_url: Option<String>,
}

/// Sorted, non-overlapping chapters clamped to `length_secs`, adjacent runs
/// of the same category merged, at most `MAX_CHAPTERS`.
pub fn normalize(nodes: &[ChapterNode], length_secs: Option<u32>) -> Vec<Chapter> {
    let len = length_secs.filter(|n| *n > 0).map(|n| n as f64);
    let mut chapters: Vec<Chapter> = nodes
        .iter()
        .filter(|n| n.position_ms >= 0 && !n.title.trim().is_empty())
        .map(|n| Chapter {
            start_secs: n.position_ms as f64 / 1000.0,
            end_secs: if n.duration_ms > 0 {
                (n.position_ms + n.duration_ms) as f64 / 1000.0
            } else {
                f64::INFINITY
            },
            title: n.title.trim().to_string(),
            game_id: n.game_id.clone(),
            box_art_url: n.box_art_url.as_deref().map(resolve_box_art),
        })
        .collect();
    chapters.sort_by(|a, b| a.start_secs.total_cmp(&b.start_secs));
    for i in 0..chapters.len() {
        let next_start = chapters.get(i + 1).map(|c| c.start_secs);
        let c = &mut chapters[i];
        if let Some(ns) = next_start {
            c.end_secs = c.end_secs.min(ns);
        }
        if let Some(len) = len {
            c.end_secs = c.end_secs.min(len);
        }
    }
    chapters.retain(|c| c.end_secs.is_finite() && c.end_secs > c.start_secs);
    let mut out: Vec<Chapter> = Vec::with_capacity(chapters.len());
    for c in chapters {
        match out.last_mut() {
            Some(prev) if prev.title == c.title && c.start_secs <= prev.end_secs + 1.0 => {
                prev.end_secs = prev.end_secs.max(c.end_secs);
            }
            _ => out.push(c),
        }
    }
    out.truncate(MAX_CHAPTERS);
    out
}

fn resolve_box_art(template: &str) -> String {
    template
        .replace("{width}", BOX_ART_SIZE.0)
        .replace("{height}", BOX_ART_SIZE.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(position_ms: i64, duration_ms: i64, title: &str) -> ChapterNode {
        ChapterNode {
            position_ms,
            duration_ms,
            title: title.to_string(),
            ..Default::default()
        }
    }

    /// xqc VOD 2871438321 (43,691 s), finished: every node has a duration.
    fn xqc() -> Vec<ChapterNode> {
        vec![
            node(0, 2_040_000, "Just Chatting"),
            node(2_040_000, 41_651_000, "Grand Theft Auto V"),
        ]
    }

    /// kato, still recording (15,552 s so far): the open chapter reports
    /// duration 0.
    fn kato() -> Vec<ChapterNode> {
        vec![
            node(0, 11_314_000, "Just Chatting"),
            node(11_314_000, 2_068_000, "Mario Kart 8 Deluxe"),
            node(13_382_000, 0, "Mario Kart World"),
        ]
    }

    #[test]
    fn finished_vod_two_chapters() {
        let out = normalize(&xqc(), Some(43_691));
        assert_eq!(out.len(), 2, "{out:?}");
        assert_eq!((out[0].start_secs, out[0].end_secs), (0.0, 2040.0));
        assert_eq!(out[0].title, "Just Chatting");
        assert_eq!((out[1].start_secs, out[1].end_secs), (2040.0, 43_691.0));
        assert_eq!(out[1].title, "Grand Theft Auto V");
    }

    #[test]
    fn open_chapter_runs_to_the_length() {
        let out = normalize(&kato(), Some(15_552));
        assert_eq!(out.len(), 3, "{out:?}");
        let last = &out[2];
        assert_eq!((last.start_secs, last.end_secs), (13_382.0, 15_552.0));
        assert_eq!(last.title, "Mario Kart World");
        for pair in out.windows(2) {
            assert!(pair[0].end_secs <= pair[1].start_secs, "non-overlapping: {out:?}");
        }
    }

    #[test]
    fn open_chapter_without_a_length_is_dropped() {
        let out = normalize(&kato(), None);
        assert_eq!(out.len(), 2, "{out:?}");
        assert_eq!(out[1].title, "Mario Kart 8 Deluxe");
    }

    #[test]
    fn empty_stays_empty() {
        assert!(normalize(&[], Some(100)).is_empty());
        assert!(normalize(&[], None).is_empty());
    }

    #[test]
    fn adjacent_same_title_merges() {
        let nodes = vec![
            node(0, 600_000, "Just Chatting"),
            node(600_000, 600_000, "Just Chatting"),
            node(1_200_000, 600_000, "Grand Theft Auto V"),
        ];
        let out = normalize(&nodes, Some(1_800));
        assert_eq!(out.len(), 2, "{out:?}");
        assert_eq!((out[0].start_secs, out[0].end_secs), (0.0, 1200.0));
        assert_eq!(out[0].title, "Just Chatting");
    }

    #[test]
    fn box_art_template_resolves() {
        let nodes = vec![ChapterNode {
            position_ms: 0,
            duration_ms: 1_000,
            title: "Grand Theft Auto V".to_string(),
            game_id: Some("32982".to_string()),
            box_art_url: Some(
                "https://static-cdn.jtvnw.net/ttv-boxart/32982_IGDB-{width}x{height}.jpg".to_string(),
            ),
        }];
        let out = normalize(&nodes, Some(1));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].game_id.as_deref(), Some("32982"));
        assert_eq!(
            out[0].box_art_url.as_deref(),
            Some("https://static-cdn.jtvnw.net/ttv-boxart/32982_IGDB-52x72.jpg")
        );
    }
}
