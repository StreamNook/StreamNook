//! The small notices a person can switch off one by one (Settings >
//! Notifications > Small notices): confirmations like "Copied" or "Welcome
//! back", never an error or anything that needs a decision. Each has an id the
//! frontend's call sites pass to `addToast` (`notice`) and its own switch,
//! `live_notifications.notice_<id>`, on by default. Desktop and Android draw
//! the section from this one list, in its order and with its words; a notice
//! a platform never shows is left out there. The list lives in
//! `notice_types.json` beside this file, which the frontend's id list is
//! tested against.

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct NoticeType {
    pub id: String,
    /// The row's name, the same on every platform that lists it.
    pub title: String,
    pub detail: String,
    /// The dotted settings path the notice's switch writes.
    pub setting: String,
    /// Where this notice shows: `desktop`, `android`, `ios`.
    #[serde(skip_serializing)]
    pub platforms: Vec<String>,
}

#[derive(Deserialize)]
struct List {
    notices: Vec<NoticeType>,
}

static NOTICES: Lazy<Vec<NoticeType>> = Lazy::new(|| {
    serde_json::from_str::<List>(include_str!("notice_types.json")).map(|l| l.notices).unwrap_or_default()
});

/// The notices that show on `platform`, in the list's order.
pub fn for_platform(platform: &str) -> Vec<NoticeType> {
    NOTICES.iter().filter(|t| t.platforms.iter().any(|p| p == platform)).cloned().collect()
}

/// The notices this build shows.
pub fn notice_types() -> Vec<NoticeType> {
    for_platform(crate::services::alert_types::PLATFORM)
}

/// Whether this build shows the notice `id`.
pub fn shows(id: &str) -> bool {
    shows_on(id, crate::services::alert_types::PLATFORM)
}

pub fn shows_on(id: &str, platform: &str) -> bool {
    NOTICES.iter().any(|t| t.id == id && t.platforms.iter().any(|p| p == platform))
}

/// Every id the list knows.
#[cfg(test)]
pub fn ids() -> Vec<&'static str> {
    NOTICES.iter().map(|t| t.id.as_str()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids_on(platform: &str) -> Vec<String> {
        for_platform(platform).into_iter().map(|t| t.id).collect()
    }

    #[test]
    fn the_list_loads_with_unique_ids_and_words() {
        assert!(NOTICES.len() >= 6);
        let mut seen = std::collections::HashSet::new();
        assert!(NOTICES.iter().all(|t| seen.insert(t.id.clone())), "an id repeats");
        for t in NOTICES.iter() {
            assert!(!t.title.is_empty() && !t.detail.is_empty(), "{} is missing words", t.id);
            assert!(!t.platforms.is_empty(), "{} shows nowhere", t.id);
            assert!(t.platforms.iter().all(|p| matches!(p.as_str(), "desktop" | "android" | "ios")), "{} names an unknown platform", t.id);
            assert!(!t.title.contains('\u{2014}') && !t.detail.contains('\u{2014}'), "no em dashes");
        }
    }

    #[test]
    fn each_notice_writes_its_own_switch_on_by_default() {
        let fresh = serde_json::to_value(crate::models::settings::Settings::default()).unwrap();
        for t in NOTICES.iter() {
            assert_eq!(t.setting, format!("live_notifications.notice_{}", t.id), "{} names its switch", t.id);
            let (group, key) = t.setting.split_once('.').unwrap();
            assert_eq!(fresh.get(group).and_then(|g| g.get(key)), Some(&serde_json::Value::Bool(true)), "{} is not a saved switch that starts on", t.id);
        }
        // A settings file from before the switches existed shows every notice.
        let old: crate::models::settings::LiveNotificationSettings =
            serde_json::from_value(serde_json::json!({ "enabled": true, "play_sound": true })).unwrap();
        assert!(old.notice_welcome_back && old.notice_drop_claimed && old.notice_copied);
        assert!(old.notice_quality_changed && old.notice_stream_restarted && old.notice_vod_resumed);
    }

    #[test]
    fn each_platform_lists_only_what_it_shows() {
        assert_eq!(
            ids_on("desktop"),
            ["welcome_back", "drop_claimed", "copied", "quality_changed", "stream_restarted", "vod_resumed"]
        );
        // The phone greets nobody on launch and its player raises none of the
        // quality, restart or resume notices.
        assert_eq!(ids_on("android"), ["drop_claimed", "copied"]);
        assert!(ids_on("ios").is_empty());
        assert!(shows_on("copied", "android") && !shows_on("welcome_back", "android"));
    }

    #[test]
    fn the_platform_list_never_reaches_a_client() {
        let row = serde_json::to_value(&NOTICES[0]).unwrap();
        assert!(row.get("platforms").is_none());
        assert!(row.get("setting").is_some() && row.get("title").is_some());
    }
}
