//! Which platforms the All view gathers live streams from.
//!
//! Some viewers only watch Twitch and do not want YouTube, Kick or TikTok in
//! their search results or their suggestions. They turn a platform off once,
//! from the search bar's platform picker or the sidebar's Recommended header,
//! and from then on the All view neither searches nor fetches it: search
//! results, Home's Discover grid and the sidebar's Recommended list leave it
//! out.
//!
//! It never hides what the viewer chose to see: followed channels, favourites,
//! notifications, and a platform picked on its own on the rail all stay.
//!
//! The choice is one list of platform ids, kept under `KEY` in the settings'
//! catch-all and written only through `set`, so a window's older copy of the
//! settings never sends it back.

use crate::models::settings::Settings;
use std::collections::HashSet;

/// The settings key the list of platforms left out is kept under.
pub const KEY: &str = "suggest_platforms_off";

/// The platforms the picker offers, in the order it draws them.
pub const PICKABLE: [&str; 4] = ["twitch", "youtube", "kick", "tiktok"];

/// Known platform ids only, each once, in picker order. Every platform off
/// would leave nothing to search or suggest, so that reads as none off.
pub fn clean<I, S>(off: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let wanted: HashSet<String> = off.into_iter().map(|p| p.as_ref().trim().to_lowercase()).collect();
    let kept: Vec<String> = PICKABLE
        .iter()
        .filter(|p| wanted.contains(**p))
        .map(|p| p.to_string())
        .collect();
    if kept.len() == PICKABLE.len() {
        return Vec::new();
    }
    kept
}

/// The platforms left out, as saved.
pub fn read(settings: &Settings) -> Vec<String> {
    let saved = settings
        .extra
        .get(KEY)
        .and_then(|v| v.as_array())
        .map(|items| items.iter().filter_map(|v| v.as_str()).map(str::to_string).collect::<Vec<_>>())
        .unwrap_or_default();
    clean(saved)
}

/// Save `off` as the platforms left out. Answers the list as stored, and
/// whether it changed.
pub fn write(settings: &mut Settings, off: &[String]) -> (Vec<String>, bool) {
    let next = clean(off);
    if read(settings) == next {
        return (next, false);
    }
    if next.is_empty() {
        settings.extra.remove(KEY);
    } else {
        settings.extra.insert(KEY.to_string(), serde_json::json!(next));
    }
    (next, true)
}

/// Whether the All view searches and suggests `provider`.
pub fn shows(off: &[String], provider: &str) -> bool {
    !off.iter().any(|p| p == provider)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_ids_and_repeats_are_dropped_and_order_is_the_pickers() {
        assert_eq!(clean(["Kick", "rumble", "youtube", "kick"]), vec!["youtube", "kick"]);
    }

    #[test]
    fn every_platform_off_reads_as_none_off() {
        assert!(clean(["twitch", "youtube", "kick", "tiktok"]).is_empty());
    }

    #[test]
    fn the_choice_round_trips_through_the_settings_catch_all() {
        let mut settings = Settings::default();
        assert!(read(&settings).is_empty());
        let (stored, changed) = write(&mut settings, &["youtube".into(), "tiktok".into(), "kick".into()]);
        assert!(changed);
        assert_eq!(stored, vec!["youtube", "kick", "tiktok"]);
        let json = serde_json::to_value(&settings).unwrap();
        assert_eq!(json[KEY], serde_json::json!(["youtube", "kick", "tiktok"]), "saved at the top level");
        let back: Settings = serde_json::from_value(json).unwrap();
        assert_eq!(read(&back), vec!["youtube", "kick", "tiktok"]);
        assert!(!write(&mut settings, &["tiktok".into(), "kick".into(), "youtube".into()]).1, "same set, no change");
        let (stored, changed) = write(&mut settings, &[]);
        assert!(changed && stored.is_empty());
        assert!(!settings.extra.contains_key(KEY), "everything on leaves no key behind");
    }

    #[test]
    fn shows_reads_the_list() {
        let off = vec!["youtube".to_string()];
        assert!(shows(&off, "twitch"));
        assert!(!shows(&off, "youtube"));
    }
}
