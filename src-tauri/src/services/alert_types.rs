//! Every kind of alert StreamNook sends, with the setting that switches it,
//! the OS category it posts under (the Android notification channel, the
//! iPhone's notification category) and the platforms where it really
//! notifies. Both phones draw their Notifications section and the setup
//! wizard's Notifications step from this one list, in its order and with its
//! words. A type a platform cannot send is left out there, never shown as
//! missing: a type joins a platform by adding it to that type's platforms.
//! The types live in `alert_types.json` beside this file.

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct AlertType {
    /// `live`, `favorite_live`, `mentions`, `drops`, `badges`.
    pub id: String,
    /// The row's name, the same on every phone.
    pub title: String,
    pub detail: String,
    /// The dotted settings path the type's switch writes.
    pub setting: String,
    /// The OS category the alert posts under.
    pub channel: String,
    /// Offered in the setup wizard's Notifications step.
    pub setup: bool,
    /// Where this type really notifies: `desktop`, `android`, `ios`.
    #[serde(skip_serializing)]
    pub platforms: Vec<String>,
}

#[derive(Deserialize)]
struct List {
    types: Vec<AlertType>,
}

static TYPES: Lazy<Vec<AlertType>> =
    Lazy::new(|| serde_json::from_str::<List>(include_str!("alert_types.json")).map(|l| l.types).unwrap_or_default());

/// This build's platform, as the list names it.
pub const PLATFORM: &str = if cfg!(target_os = "ios") {
    "ios"
} else if cfg!(target_os = "android") {
    "android"
} else {
    "desktop"
};

/// The types that notify on `platform`, in the list's order.
pub fn for_platform(platform: &str) -> Vec<AlertType> {
    TYPES.iter().filter(|t| t.platforms.iter().any(|p| p == platform)).cloned().collect()
}

/// The types this build notifies about.
pub fn alert_types() -> Vec<AlertType> {
    for_platform(PLATFORM)
}

/// Whether this build sends the alert type `id`.
pub fn works(id: &str) -> bool {
    works_on(id, PLATFORM)
}

pub fn works_on(id: &str, platform: &str) -> bool {
    TYPES.iter().any(|t| t.id == id && t.platforms.iter().any(|p| p == platform))
}

/// Every id the list knows, works anywhere or not.
#[cfg(test)]
pub fn ids() -> Vec<&'static str> {
    TYPES.iter().map(|t| t.id.as_str()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids_on(platform: &str) -> Vec<String> {
        for_platform(platform).into_iter().map(|t| t.id).collect()
    }

    #[test]
    fn the_list_loads_with_unique_ids() {
        assert!(TYPES.len() >= 5);
        let mut seen = std::collections::HashSet::new();
        assert!(TYPES.iter().all(|t| seen.insert(t.id.clone())), "an id repeats");
        for t in TYPES.iter() {
            assert!(!t.title.is_empty() && !t.detail.is_empty() && !t.channel.is_empty(), "{} is missing words", t.id);
            assert!(t.platforms.iter().all(|p| matches!(p.as_str(), "desktop" | "android" | "ios")), "{} names an unknown platform", t.id);
            assert!(!t.title.contains('\u{2014}') && !t.detail.contains('\u{2014}'), "no em dashes");
        }
    }

    #[test]
    fn every_setting_is_a_real_live_notifications_switch() {
        let fresh = serde_json::to_value(crate::models::settings::Settings::default()).unwrap();
        for t in TYPES.iter() {
            let (group, key) = t.setting.split_once('.').expect("a dotted path");
            let value = fresh.get(group).and_then(|g| g.get(key));
            assert!(value.is_some_and(|v| v.is_boolean()), "{} writes {}, which is not a saved switch", t.id, t.setting);
        }
    }

    #[test]
    fn each_phone_lists_only_what_it_sends() {
        // Android polls for go-lives and posts drops, badges and mentions.
        assert_eq!(ids_on("android"), ["live", "mentions", "drops", "badges"]);
        // The iPhone posts mentions while it runs; go-lives arrive by push (services/push_alerts.rs).
        assert_eq!(ids_on("ios"), ["live", "mentions"]);
        assert!(works_on("mentions", "ios") && works_on("live", "ios"));
        assert!(!works_on("drops", "ios") && !works_on("badges", "ios"));
        assert!(works_on("favorite_live", "desktop") && !works_on("favorite_live", "android"));
    }

    #[test]
    fn the_wizard_offers_go_live_and_mentions() {
        let setup: Vec<String> = TYPES.iter().filter(|t| t.setup).map(|t| t.id.clone()).collect();
        assert_eq!(setup, ["live", "mentions"]);
        let android: Vec<String> = for_platform("android").into_iter().filter(|t| t.setup).map(|t| t.id).collect();
        assert_eq!(android, ["live", "mentions"]);
        let ios: Vec<String> = for_platform("ios").into_iter().filter(|t| t.setup).map(|t| t.id).collect();
        assert_eq!(ios, ["live", "mentions"]);
    }

    #[test]
    fn the_platform_list_never_reaches_a_client() {
        let row = serde_json::to_value(&TYPES[0]).unwrap();
        assert!(row.get("platforms").is_none());
        assert!(row.get("setting").is_some() && row.get("channel").is_some());
    }
}
