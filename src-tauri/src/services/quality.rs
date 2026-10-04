//! Twitch quality-string math: parsing, ordering, closest-match selection, and
//! the two-naming-shapes equivalence rule. Lifted out of the old
//! `streamlink_manager` when Streamlink was removed; the native resolver
//! (`twitch_resolver`) is the sole consumer. Twitch ships two shapes for the
//! same tier in the wild (`480p` vs `480p30`); these helpers reconcile them.

/// Parse the leading resolution height from a quality string (e.g. "480p30" -> 480).
/// Returns None for non-resolution qualities like "best", "worst", "audio_only".
pub fn parse_quality_height(q: &str) -> Option<u32> {
    let digits: String = q
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

/// Parse the framerate suffix from a quality string (e.g. "720p60" -> 60, "720p" -> None).
fn parse_quality_fps(q: &str) -> Option<u32> {
    let lower = q.trim().to_lowercase();
    let after_p = lower.split_once('p')?.1;
    let digits: String = after_p.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

/// Sort quality strings into the order surfaced in the player's quality menu.
/// Resolutions first, descending by height then framerate; then the sentinels
/// `best`, `audio_only`, `worst`; anything else last, alphabetically.
pub(crate) fn sort_qualities_descending(qualities: &mut [String]) {
    fn rank(q: &str) -> (u8, u32, u32, String) {
        let lower = q.trim().to_lowercase();
        if let Some(h) = parse_quality_height(&lower) {
            let fps = parse_quality_fps(&lower).unwrap_or(0);
            return (0, u32::MAX - h, u32::MAX - fps, String::new());
        }
        match lower.as_str() {
            "best" | "source" => (1, 0, 0, String::new()),
            "audio_only" | "audio-only" | "audio" => (2, 0, 0, String::new()),
            "worst" => (3, 0, 0, String::new()),
            _ => (4, 0, 0, lower),
        }
    }
    qualities.sort_by_key(|q| rank(q));
}

/// Pick the closest available quality to the requested one.
/// Tiebreak: prefer higher resolution, then closer (or higher) framerate.
pub fn pick_closest_quality(requested: &str, available: &[String]) -> Option<String> {
    if available.is_empty() {
        return None;
    }

    if let Some(exact) = available.iter().find(|q| q.eq_ignore_ascii_case(requested)) {
        return Some(exact.clone());
    }

    let req_height = match parse_quality_height(requested) {
        Some(h) => h,
        None => {
            return available
                .iter()
                .find(|q| q.eq_ignore_ascii_case("best"))
                .cloned()
                .or_else(|| available.first().cloned());
        }
    };
    let req_fps = parse_quality_fps(requested);

    let mut candidates: Vec<(&String, u32, Option<u32>)> = available
        .iter()
        .filter_map(|q| Some((q, parse_quality_height(q)?, parse_quality_fps(q))))
        .collect();

    if candidates.is_empty() {
        return available
            .iter()
            .find(|q| q.eq_ignore_ascii_case("best"))
            .cloned();
    }

    candidates.sort_by(|a, b| {
        let da = (a.1 as i64 - req_height as i64).abs();
        let db = (b.1 as i64 - req_height as i64).abs();
        da.cmp(&db)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| match (a.2, b.2, req_fps) {
                (Some(af), Some(bf), Some(rf)) => {
                    let fa = (af as i64 - rf as i64).abs();
                    let fb = (bf as i64 - rf as i64).abs();
                    fa.cmp(&fb).then_with(|| bf.cmp(&af))
                }
                (Some(af), Some(bf), None) => bf.cmp(&af),
                (Some(_), None, _) => std::cmp::Ordering::Less,
                (None, Some(_), _) => std::cmp::Ordering::Greater,
                (None, None, _) => std::cmp::Ordering::Equal,
            })
    });

    Some(candidates[0].0.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn picks_exact_when_present() {
        let avail = s(&["audio_only", "360p", "480p", "720p60", "best", "worst"]);
        assert_eq!(
            pick_closest_quality("480p", &avail).as_deref(),
            Some("480p")
        );
    }

    #[test]
    fn picks_closest_when_fps_suffix_missing() {
        let avail = s(&["audio_only", "360p", "480p", "720p60", "best", "worst"]);
        assert_eq!(
            pick_closest_quality("480p30", &avail).as_deref(),
            Some("480p")
        );
    }

    #[test]
    fn picks_closest_when_height_missing() {
        let avail = s(&["audio_only", "360p", "720p60", "best", "worst"]);
        assert_eq!(
            pick_closest_quality("480p30", &avail).as_deref(),
            Some("360p")
        );
    }

    #[test]
    fn ties_prefer_higher_resolution() {
        let avail = s(&["360p", "600p", "best"]);
        assert_eq!(
            pick_closest_quality("480p30", &avail).as_deref(),
            Some("600p")
        );
    }

    #[test]
    fn picks_matching_fps_on_tie() {
        let avail = s(&["720p30", "720p60", "best"]);
        assert_eq!(
            pick_closest_quality("480p30", &avail).as_deref(),
            Some("720p30")
        );
    }

    #[test]
    fn best_request_with_only_best_falls_through() {
        let avail = s(&["360p", "best"]);
        assert_eq!(
            pick_closest_quality("best", &avail).as_deref(),
            Some("best")
        );
    }

    #[test]
    fn empty_available_returns_none() {
        assert_eq!(pick_closest_quality("480p30", &[]), None);
    }

    #[test]
    fn picks_1080p_when_user_wants_1440p_but_stream_only_has_1080() {
        let avail = s(&["audio_only", "480p30", "720p60", "1080p60", "best", "worst"]);
        assert_eq!(
            pick_closest_quality("1440p60", &avail).as_deref(),
            Some("1080p60")
        );
    }

    #[test]
    fn picks_1440p_when_offered() {
        let avail = s(&["audio_only", "720p60", "1080p60", "1440p60", "best"]);
        assert_eq!(
            pick_closest_quality("1440p60", &avail).as_deref(),
            Some("1440p60")
        );
    }

    #[test]
    fn handles_bare_resolution_alias() {
        let avail = s(&["audio_only", "360p", "480p30", "720p60", "best"]);
        assert_eq!(
            pick_closest_quality("480", &avail).as_deref(),
            Some("480p30")
        );
    }

    #[test]
    fn dropdown_value_matches_caedrel_format() {
        let avail = s(&[
            "audio_only",
            "160p",
            "360p",
            "480p",
            "720p60",
            "1080p60",
            "worst",
            "best",
        ]);
        assert_eq!(
            pick_closest_quality("480p30", &avail).as_deref(),
            Some("480p")
        );
        assert_eq!(
            pick_closest_quality("360p30", &avail).as_deref(),
            Some("360p")
        );
        assert_eq!(
            pick_closest_quality("160p30", &avail).as_deref(),
            Some("160p")
        );
    }

    #[test]
    fn dropdown_value_matches_nickmercs_format() {
        let avail = s(&[
            "audio_only",
            "160p30",
            "360p30",
            "480p30",
            "720p60",
            "1080p60",
            "worst",
            "best",
        ]);
        assert_eq!(
            pick_closest_quality("480p30", &avail).as_deref(),
            Some("480p30")
        );
    }

    #[test]
    fn high_tier_dropdown_finds_60fps_exact() {
        let avail = s(&[
            "audio_only",
            "160p",
            "360p",
            "480p",
            "720p60",
            "1080p60",
            "best",
        ]);
        assert_eq!(
            pick_closest_quality("1080p60", &avail).as_deref(),
            Some("1080p60")
        );
        assert_eq!(
            pick_closest_quality("720p60", &avail).as_deref(),
            Some("720p60")
        );
    }

    #[test]
    fn sorts_qualities_highest_resolution_first() {
        let mut q: Vec<String> = vec![
            "1080p60",
            "1440p60",
            "160p30",
            "360p30",
            "480p30",
            "720p60",
            "audio_only",
            "best",
            "worst",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        sort_qualities_descending(&mut q);
        assert_eq!(
            q,
            vec![
                "1440p60",
                "1080p60",
                "720p60",
                "480p30",
                "360p30",
                "160p30",
                "best",
                "audio_only",
                "worst",
            ]
        );
    }

    #[test]
    fn sort_breaks_height_ties_by_fps() {
        let mut q: Vec<String> = vec!["720p30", "720p60", "1080p30", "1080p60"]
            .into_iter()
            .map(String::from)
            .collect();
        sort_qualities_descending(&mut q);
        assert_eq!(q, vec!["1080p60", "1080p30", "720p60", "720p30"]);
    }
}
