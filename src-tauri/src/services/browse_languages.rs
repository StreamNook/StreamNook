//! Which stream languages Discover and browse show: one rule for every platform.
//!
//! The setting is `discovery_languages` (Twitch's language codes, `tl` for
//! Filipino). Empty means the device's language; `any` turns the filter off;
//! anything else is the list as chosen. Twitch's recommendations and every Kick
//! browse read go through `current()`, so the setting means the same thing on
//! every surface.

use crate::models::settings::AppState;
use tauri::Manager;

/// The setting's value that turns the language filter off.
pub const ANY: &str = "any";
/// Kick takes one language per request, so a longer list costs a call each.
pub const KICK_MAX_LANGUAGES: usize = 3;

/// The languages a stored `discovery_languages` value asks for: empty when
/// nothing should be filtered.
pub fn resolve(stored: &[String], device: Option<&str>) -> Vec<String> {
    if stored.iter().any(|l| l.eq_ignore_ascii_case(ANY)) {
        return Vec::new();
    }
    if stored.is_empty() {
        return device.and_then(device_code).into_iter().collect();
    }
    let mut out: Vec<String> = Vec::new();
    for l in stored {
        let l = l.trim().to_ascii_lowercase();
        if !l.is_empty() && !out.contains(&l) {
            out.push(l);
        }
    }
    out
}

/// A device locale (`en-US`, `zh-Hant-HK`, `fil-PH`, `nb_NO`) as one of the
/// setting's codes.
pub fn device_code(locale: &str) -> Option<String> {
    let parts: Vec<String> = locale
        .split(['-', '_', '.', '@'])
        .map(|p| p.to_ascii_lowercase())
        .collect();
    let primary = parts.first().filter(|p| p.len() >= 2 && p.len() <= 3)?;
    let code = match primary.as_str() {
        "zh" if parts.iter().any(|p| p == "hk" || p == "yue") => "zh-hk",
        "yue" => "zh-hk",
        "fil" => "tl",
        "nb" | "nn" => "no",
        "c" | "posix" => return None,
        other => other,
    };
    Some(code.to_string())
}

/// The setting's codes as Kick spells them, at most `KICK_MAX_LANGUAGES`.
/// `other` has no Kick equivalent and is left out.
pub fn kick_codes(languages: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in languages {
        let code = match l.as_str() {
            "tl" => "fil",
            "zh-hk" => "zh",
            "other" => continue,
            other => other,
        };
        if !out.iter().any(|c| c == code) {
            out.push(code.to_string());
        }
        if out.len() == KICK_MAX_LANGUAGES {
            break;
        }
    }
    out
}

/// The stored setting, untouched.
pub fn stored(state: &AppState) -> Vec<String> {
    let Ok(settings) = state.settings.lock() else { return Vec::new() };
    settings
        .extra
        .get("discovery_languages")
        .and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok())
        .unwrap_or_default()
}

/// The device's language as the setting would spell it.
pub fn device() -> Option<String> {
    sys_locale::get_locale().and_then(|l| device_code(&l))
}

/// The languages to show right now.
pub fn current() -> Vec<String> {
    let stored = crate::services::providers::app_handle()
        .and_then(|app| app.try_state::<AppState>().map(|s| stored(s.inner())))
        .unwrap_or_default();
    resolve_stored(&stored)
}

/// `resolve` with the device's language.
pub fn resolve_stored(stored: &[String]) -> Vec<String> {
    let device = if stored.is_empty() { sys_locale::get_locale() } else { None };
    resolve(stored, device.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn empty_follows_the_device_language() {
        assert_eq!(resolve(&[], Some("en-US")), v(&["en"]));
        assert_eq!(resolve(&[], Some("pt_BR.UTF-8")), v(&["pt"]));
        assert_eq!(resolve(&[], None), Vec::<String>::new());
    }

    #[test]
    fn any_turns_the_filter_off() {
        assert!(resolve(&v(&["any"]), Some("en-US")).is_empty());
        assert!(resolve(&v(&["en", "any"]), Some("en-US")).is_empty());
    }

    #[test]
    fn a_chosen_list_wins_over_the_device() {
        assert_eq!(resolve(&v(&["ES", "en", "es"]), Some("ja-JP")), v(&["es", "en"]));
    }

    #[test]
    fn device_locales_map_to_the_setting_codes() {
        assert_eq!(device_code("zh-Hant-HK").as_deref(), Some("zh-hk"));
        assert_eq!(device_code("zh-Hans-CN").as_deref(), Some("zh"));
        assert_eq!(device_code("fil-PH").as_deref(), Some("tl"));
        assert_eq!(device_code("nb_NO").as_deref(), Some("no"));
        assert_eq!(device_code("C").as_deref(), None);
        assert_eq!(device_code("").as_deref(), None);
    }

    #[test]
    fn kick_spells_filipino_fil_and_takes_three() {
        assert_eq!(kick_codes(&v(&["tl", "en"])), v(&["fil", "en"]));
        assert_eq!(kick_codes(&v(&["other", "en", "es", "pt", "fr"])), v(&["en", "es", "pt"]));
        assert_eq!(kick_codes(&v(&["zh", "zh-hk"])), v(&["zh"]));
    }
}
