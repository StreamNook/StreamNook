//! The one compact count format every shell shows: "352", "1.2K", "12.3K",
//! "353K", "1.2M". Category tiles on both phones and the desktop read the line
//! Rust stamps on each category (`viewers_line`) instead of each formatting the
//! number its own way: Android's tile said "324.6K" where the iPhone's said
//! "320K" for the same count.

/// A count in the compact form. Below 1,000 it is the number itself. From
/// there it takes a unit (K, M, B) with one decimal while the value is under
/// 100 of that unit and a whole number from 100 up, and never shows a trailing
/// ".0". A value that rounds up to the next unit takes that unit ("1M", never
/// "1000K").
pub fn compact(n: u64) -> String {
    if n < 1_000 {
        return n.to_string();
    }
    const UNITS: [(u64, &str); 3] = [(1_000, "K"), (1_000_000, "M"), (1_000_000_000, "B")];
    // The unit the number is in, then one step up if rounding carried it to
    // 1,000 of that unit (999,500 is "1M", not "1000K").
    let mut i = UNITS.iter().rposition(|&(size, _)| n >= size).unwrap_or(0);
    let mut shown = rounded(n as f64 / UNITS[i].0 as f64);
    if shown >= 1_000.0 && i + 1 < UNITS.len() {
        i += 1;
        shown = rounded(n as f64 / UNITS[i].0 as f64);
    }
    format!("{}{}", trim(shown), UNITS[i].1)
}

/// One decimal under 100, whole from 100 up.
fn rounded(value: f64) -> f64 {
    if value < 99.95 { (value * 10.0).round() / 10.0 } else { value.round() }
}

/// "12.0" -> "12", "12.3" -> "12.3".
fn trim(value: f64) -> String {
    let s = format!("{value:.1}");
    s.strip_suffix(".0").map(str::to_string).unwrap_or(s)
}

/// A category's viewers as its tile words it: "No live streams" at 0,
/// "1 viewer", else the compact count and "viewers". `None` when the source
/// did not count them (Helix answers carry no viewers), so the tile shows
/// nothing rather than a wrong zero.
pub fn viewers_line(viewers: Option<u64>) -> Option<String> {
    viewers.map(|n| match n {
        0 => "No live streams".to_string(),
        1 => "1 viewer".to_string(),
        n => format!("{} viewers", compact(n)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_counts_are_the_number() {
        assert_eq!(compact(0), "0");
        assert_eq!(compact(7), "7");
        assert_eq!(compact(999), "999");
    }

    #[test]
    fn one_decimal_under_a_hundred_whole_from_there() {
        assert_eq!(compact(1_000), "1K");
        assert_eq!(compact(1_234), "1.2K");
        assert_eq!(compact(12_345), "12.3K");
        assert_eq!(compact(99_940), "99.9K");
        assert_eq!(compact(99_950), "100K");
        assert_eq!(compact(324_600), "325K");
        assert_eq!(compact(353_000), "353K");
        assert_eq!(compact(1_250_000), "1.3M");
        assert_eq!(compact(12_000_000), "12M");
        assert_eq!(compact(2_500_000_000), "2.5B");
    }

    #[test]
    fn a_count_that_rounds_up_takes_the_next_unit() {
        assert_eq!(compact(999_499), "999K");
        assert_eq!(compact(999_500), "1M");
        assert_eq!(compact(999_950), "1M");
        assert_eq!(compact(999_999_999), "1B");
    }

    #[test]
    fn never_a_trailing_point_zero() {
        for n in [1_000, 2_000, 10_000, 100_000, 1_000_000, 20_000_000] {
            assert!(!compact(n).contains(".0"), "{n} -> {}", compact(n));
        }
    }

    #[test]
    fn categories_carry_the_line_they_are_served_with() {
        use crate::models::provider_stream::ProviderCategory;
        use crate::services::twitch_service::CategoryRow;
        let row = |viewers| CategoryRow { id: "1".into(), name: "Just Chatting".into(), box_art_url: String::new(), viewer_count: viewers };
        let counted = serde_json::to_value(row(Some(353_123))).unwrap();
        assert_eq!(counted["viewers_line"], "353K viewers");
        assert_eq!(counted["viewer_count"], 353_123);
        // Helix answers carry no viewers: neither the number nor a line.
        let uncounted = serde_json::to_value(row(None)).unwrap();
        assert!(uncounted.get("viewers_line").is_none() && uncounted.get("viewer_count").is_none());
        // A row read back from a cache that already holds the line still parses.
        let back: CategoryRow = serde_json::from_value(counted).unwrap();
        assert_eq!(back.viewer_count, Some(353_123));
        let kick = ProviderCategory {
            provider: "kick".into(),
            id: "15".into(),
            name: "Just Chatting".into(),
            thumbnail: String::new(),
            viewer_count: 0,
            channel_count: 0,
        };
        assert_eq!(serde_json::to_value(kick).unwrap()["viewers_line"], "No live streams");
    }

    #[test]
    fn the_tile_line() {
        assert_eq!(viewers_line(None), None);
        assert_eq!(viewers_line(Some(0)).as_deref(), Some("No live streams"));
        assert_eq!(viewers_line(Some(1)).as_deref(), Some("1 viewer"));
        assert_eq!(viewers_line(Some(353_123)).as_deref(), Some("353K viewers"));
        assert_eq!(viewers_line(Some(12_345)).as_deref(), Some("12.3K viewers"));
    }
}
