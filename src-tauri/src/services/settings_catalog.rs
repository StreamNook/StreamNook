//! Every settings row the phones show, with the paths it writes and the
//! words people search for it by. Settings search ranks rows against a query;
//! the "changed from default" review lists the rows whose paths differ from a
//! fresh install. The rows live in `settings_catalog.json` beside this file.

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct CatalogRow {
    /// The settings page: `player`, `adFree`, `chat`, `moderation`, `theme`, `notifications`, `cache`, `support`.
    pub page: String,
    pub section: String,
    pub title: String,
    #[serde(default)]
    pub paths: Vec<String>,
    /// Drawn nested under another row.
    #[serde(default)]
    pub nested: bool,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    /// The brand whose logo stands in for its name in the title (`7tv`, `ffz`, `bttv`).
    #[serde(default)]
    pub logo: Option<String>,
    /// For a nested row, the title of the row it is drawn under, so a search
    /// hit can say where it lives ("Size" under "GIFs" is not the emote "Size").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub under: Option<String>,
    /// Not drawn on the iPhone, where it does nothing yet: search and the
    /// changed review skip it there. Its saved value is kept, and Android and
    /// desktop still show it.
    #[serde(default, skip_serializing)]
    pub hidden_on_ios: bool,
    /// What this build must have for the row to be drawn at all: a
    /// `client_capabilities` field (`combined_chat`), `watch:<platform>`
    /// for a row that only touches that platform, or `alert:<type>` for a
    /// row about an alert type (`services::alert_types`) this build must
    /// send. `|` between names means any
    /// one of them. Search and the changed review skip the row on a build
    /// without it, on every platform, so they hide exactly what the pages hide.
    #[serde(default, skip_serializing)]
    pub needs: Option<String>,
}

/// Whether this build has the capability `need` names.
fn build_has(need: &str) -> bool {
    if let Some(platform) = need.strip_prefix("watch:") {
        return crate::services::account_roster::can_watch(platform);
    }
    if let Some(alert) = need.strip_prefix("alert:") {
        return crate::services::alert_types::works(alert);
    }
    crate::services::client_capabilities::has(need)
}

/// Whether a row is drawn on this platform's settings pages, given what the
/// build has (`has` answers one capability name).
fn shown(row: &CatalogRow, ios: bool, has: &dyn Fn(&str) -> bool) -> bool {
    if ios && row.hidden_on_ios {
        return false;
    }
    match row.needs.as_deref() {
        Some(needs) => needs.split('|').map(str::trim).any(|n| has(n)),
        None => true,
    }
}

const IOS: bool = cfg!(target_os = "ios");

#[derive(Deserialize)]
struct Catalog {
    rows: Vec<CatalogRow>,
}

/// A nested row sits under the nearest row above it on the same page and
/// section that is not nested itself.
fn link_nested(rows: &mut [CatalogRow]) {
    let mut parent: Option<(String, String, String)> = None;
    for row in rows.iter_mut() {
        if !row.nested {
            parent = Some((row.page.clone(), row.section.clone(), row.title.clone()));
            continue;
        }
        row.under = parent
            .as_ref()
            .filter(|(page, section, _)| *page == row.page && *section == row.section)
            .map(|(_, _, title)| title.clone());
    }
}

static ROWS: Lazy<Vec<CatalogRow>> = Lazy::new(|| {
    let mut rows = serde_json::from_str::<Catalog>(include_str!("settings_catalog.json")).map(|c| c.rows).unwrap_or_default();
    link_nested(&mut rows);
    rows
});

pub fn rows() -> &'static [CatalogRow] {
    &ROWS
}

/// The order pages are listed in, so ties read top to bottom like the You tab.
const PAGE_ORDER: &[&str] = &["player", "adFree", "chat", "moderation", "theme", "notifications", "cache", "support"];

fn page_name(page: &str) -> &'static str {
    match page {
        "player" => "player",
        "adFree" => "ad-free ad free",
        "chat" => "chat",
        "moderation" => "moderation mod",
        "theme" => "theme",
        "notifications" => "notifications",
        "cache" => "cache",
        _ => "support",
    }
}

fn normalize(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '&' || c == '!' || c == '@' { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn has_word_prefix(haystack: &str, token: &str) -> bool {
    haystack.split(' ').any(|w| w.starts_with(token))
}

/// How well one row answers the query: every word must land somewhere; a hit
/// in the title counts most, then an alias, the section, the page and the
/// description. Zero means no match.
fn score(row: &CatalogRow, query: &str) -> u32 {
    let tokens: Vec<&str> = query.split(' ').filter(|t| !t.is_empty()).collect();
    if tokens.is_empty() {
        return 0;
    }
    let title = normalize(&row.title);
    let aliases: Vec<String> = row.aliases.iter().map(|a| normalize(a)).collect();
    let section = normalize(&row.section);
    let page = page_name(&row.page);
    let detail = normalize(&row.detail);
    let paths = row.paths.join(" ").replace(['.', '_'], " ");
    let mut total: u32 = 0;
    for token in &tokens {
        let best = [
            (has_word_prefix(&title, token), 30),
            (title.contains(token), 20),
            (aliases.iter().any(|a| has_word_prefix(a, token)), 16),
            (has_word_prefix(&section, token), 10),
            (has_word_prefix(page, token), 6),
            (has_word_prefix(&detail, token), 5),
            (paths.contains(token), 3),
        ]
        .iter()
        .filter(|(hit, _)| *hit)
        .map(|(_, points)| *points)
        .max();
        match best {
            Some(points) => total += points,
            None => return 0,
        }
    }
    if title == query {
        total += 60;
    } else if title.starts_with(query) {
        total += 40;
    }
    if aliases.iter().any(|a| a == query) {
        total += 30;
    }
    if row.nested {
        total = total.saturating_sub(2);
    }
    total
}

/// The rows that answer `query`, best first (at most `limit`).
pub fn search(query: &str, limit: usize) -> Vec<CatalogRow> {
    search_on(query, limit, IOS, &build_has)
}

fn search_on(query: &str, limit: usize, ios: bool, has: &dyn Fn(&str) -> bool) -> Vec<CatalogRow> {
    let query = normalize(query);
    let mut hits: Vec<(u32, usize, &CatalogRow)> = rows()
        .iter()
        .enumerate()
        .filter(|(_, row)| shown(row, ios, has))
        .filter_map(|(i, row)| {
            let s = score(row, &query);
            (s > 0).then_some((s, i, row))
        })
        .collect();
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    hits.into_iter().take(limit).map(|(_, _, row)| row.clone()).collect()
}

/// Whether a row's paths include a changed leaf, or hold one, or sit inside one.
fn row_changed(row: &CatalogRow, changed: &[String]) -> bool {
    row.paths.iter().any(|path| {
        changed.iter().any(|c| c == path || c.starts_with(&format!("{path}.")) || path.starts_with(&format!("{c}.")))
    })
}

/// The rows whose settings differ from a fresh install, in page order.
pub fn changed_rows(changed: &[String]) -> Vec<CatalogRow> {
    changed_rows_on(changed, IOS, &build_has)
}

fn changed_rows_on(changed: &[String], ios: bool, has: &dyn Fn(&str) -> bool) -> Vec<CatalogRow> {
    let mut rows: Vec<(usize, usize, CatalogRow)> = rows()
        .iter()
        .enumerate()
        .filter(|(_, row)| shown(row, ios, has) && row_changed(row, changed))
        .map(|(i, row)| (PAGE_ORDER.iter().position(|p| *p == row.page).unwrap_or(PAGE_ORDER.len()), i, row.clone()))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    rows.into_iter().map(|(_, _, row)| row).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(rows: &[CatalogRow]) -> Vec<&str> {
        rows.iter().map(|r| r.title.as_str()).collect()
    }

    #[test]
    fn the_catalog_loads_and_names_real_settings() {
        assert!(rows().len() > 150);
        assert!(rows().iter().all(|r| PAGE_ORDER.contains(&r.page.as_str())));
        let fresh = serde_json::to_value(crate::models::settings::Settings::default()).unwrap();
        let mut known = fresh.clone();
        crate::commands::settings_fields::fill_absent(&mut known, &crate::commands::settings_fields::frontend_owned_defaults());
        // Every path's top-level group is a real settings key (lists and maps may be absent).
        for row in rows() {
            for path in &row.paths {
                let top = path.split('.').next().unwrap();
                assert!(
                    known.get(top).is_some() || matches!(top, "custom_themes" | "chat_highlights" | "chat_filters" | "chat_commands" | "chat_customization" | "font" | "font_custom" | "glass_transparency"),
                    "{} has an unknown path {path}",
                    row.title
                );
            }
        }
    }

    #[test]
    fn search_finds_rows_by_title_alias_and_section() {
        assert_eq!(search("text size", 3)[0].title, "Text size");
        assert_eq!(search("ads", 3)[0].title, "Ad-free playback");
        assert!(titles(&search("pip", 5)).contains(&"Background play"));
        assert!(titles(&search("nightbot", 5)).contains(&"Known bots"));
        assert!(search("zzzz nothing", 5).is_empty());
        assert!(search("   ", 5).is_empty());
    }

    #[test]
    fn nested_rows_name_the_row_they_sit_under() {
        let gif = rows().iter().find(|r| r.paths.iter().any(|p| p == "chat_design.gif_scale")).unwrap();
        assert_eq!(gif.under.as_deref(), Some("GIFs"));
        let emote = rows().iter().find(|r| r.paths.iter().any(|p| p == "chat_design.emote_scale")).unwrap();
        assert_eq!(emote.under, None);
        assert!(rows().iter().filter(|r| r.nested).all(|r| r.under.is_some()), "a nested row has no row above it");
    }

    #[test]
    fn changed_rows_follow_prefixes() {
        let rows = changed_rows(&["chat_design.font_size".into(), "chat_blend.platforms.kick".into()]);
        let t = titles(&rows);
        assert!(t.contains(&"Text size"));
        assert!(t.contains(&"Platforms"));
        assert!(!t.contains(&"Spacing"));
    }

    fn everything(_: &str) -> bool {
        true
    }

    fn nothing(_: &str) -> bool {
        false
    }

    #[test]
    fn rows_hidden_on_the_iphone_stay_for_other_platforms() {
        assert!(titles(&search_on("scrollbar marks", 5, false, &everything)).contains(&"Scrollbar marks"));
        assert!(!titles(&search_on("scrollbar marks", 5, true, &everything)).contains(&"Scrollbar marks"));
        let changed = ["chat_blend.platforms.kick".to_string(), "chat_design.font_size".to_string()];
        assert!(titles(&changed_rows_on(&changed, false, &everything)).contains(&"Platforms"));
        let ios = changed_rows_on(&changed, true, &everything);
        assert!(!titles(&ios).contains(&"Platforms"));
        assert!(titles(&ios).contains(&"Text size"));
        // The flag never reaches a client.
        let row = rows().iter().find(|r| r.hidden_on_ios).unwrap();
        assert!(serde_json::to_value(row).unwrap().get("hidden_on_ios").is_none());
    }

    #[test]
    fn rows_a_build_lacks_are_hidden_from_search_and_changed() {
        // A build without combined chat (a phone) finds none of its rows, on
        // Android as well as the iPhone.
        assert!(titles(&search_on("merged feed", 5, false, &everything)).contains(&"Merged feed"));
        assert!(!titles(&search_on("merged feed", 5, false, &nothing)).contains(&"Merged feed"));
        assert!(!titles(&search_on("scrollbar marks", 5, false, &nothing)).contains(&"Scrollbar marks"));
        let changed = ["chat_blend.platforms.kick".to_string(), "chat_design.font_size".to_string()];
        let lacking = changed_rows_on(&changed, false, &nothing);
        assert!(!titles(&lacking).contains(&"Platforms"));
        assert!(titles(&lacking).contains(&"Text size"), "a row with no needs always shows");

        // One capability at a time: only the rows that need it go.
        let no_youtube = |n: &str| n != "watch:youtube";
        assert!(!titles(&search_on("currency", 5, false, &no_youtube)).contains(&"Currency"));
        assert!(titles(&search_on("merged feed", 5, false, &no_youtube)).contains(&"Merged feed"));
        // `|` is any of: pictures show while either platform can be watched.
        let tiktok_only = |n: &str| n == "watch:tiktok";
        assert!(titles(&search_on("pictures in chat", 5, false, &tiktok_only)).contains(&"Pictures in chat"));
        assert!(!titles(&search_on("pictures in chat", 5, false, &nothing)).contains(&"Pictures in chat"));

        // The flag never reaches a client either.
        let row = rows().iter().find(|r| r.needs.is_some()).unwrap();
        assert!(serde_json::to_value(row).unwrap().get("needs").is_none());
    }

    #[test]
    fn every_need_names_a_real_capability() {
        let known = ["combined_chat", "chat_dock", "scrollbar_marks", "spell_dictionary"];
        for row in rows() {
            for need in row.needs.iter().flat_map(|n| n.split('|')) {
                let alert = need.strip_prefix("alert:").is_some_and(|id| crate::services::alert_types::ids().contains(&id));
                let ok = known.contains(&need)
                    || alert
                    || matches!(need, "watch:youtube" | "watch:tiktok" | "watch:twitch" | "watch:kick");
                assert!(ok, "{} needs an unknown capability {need}", row.title);
            }
        }
    }

    #[test]
    fn alert_rows_follow_the_alert_list() {
        // The iPhone sends mentions and go-lives: search finds those rows and none of the others.
        let iphone = |n: &str| match n.strip_prefix("alert:") {
            Some(id) => crate::services::alert_types::works_on(id, "ios"),
            None => true,
        };
        let android = |n: &str| match n.strip_prefix("alert:") {
            Some(id) => crate::services::alert_types::works_on(id, "android"),
            None => true,
        };
        for title in ["Mentions and replies", "Live channels", "Which channels"] {
            assert!(titles(&search_on(&title.to_lowercase(), 5, true, &iphone)).contains(&title), "{title} on iPhone");
        }
        for title in ["Live channels", "Which channels", "Drops", "New badges"] {
            assert!(titles(&search_on(&title.to_lowercase(), 5, false, &android)).contains(&title), "{title} on Android");
        }
        for title in ["Drops", "New badges"] {
            assert!(!titles(&search_on(&title.to_lowercase(), 5, true, &iphone)).contains(&title), "{title} on iPhone");
        }
        // Every type a phone lists has its row on the Notifications page, by the same title.
        for platform in ["android", "ios"] {
            for alert in crate::services::alert_types::for_platform(platform) {
                let row = rows().iter().find(|r| r.paths.contains(&alert.setting)).unwrap_or_else(|| panic!("no row for {}", alert.id));
                assert_eq!((row.page.as_str(), row.section.as_str(), row.title.as_str()), ("notifications", "Notify me about", alert.title.as_str()));
                assert_eq!(row.detail, alert.detail, "{} words differ", alert.id);
                assert_eq!(row.needs.as_deref(), Some(format!("alert:{}", alert.id).as_str()));
            }
        }
    }

    #[test]
    fn this_build_answers_needs_from_rust() {
        assert_eq!(build_has("combined_chat"), cfg!(desktop));
        assert_eq!(build_has("watch:youtube"), cfg!(desktop));
        assert!(build_has("watch:kick"));
    }

    #[test]
    fn rows_every_phone_draws_are_never_hidden() {
        for title in ["Wallet balance", "Resume VODs", "VOD positions"] {
            assert!(titles(&search_on(&title.to_lowercase(), 5, true, &nothing)).contains(&title), "{title}");
        }
        let wallet = rows().iter().find(|r| r.title == "Wallet balance").unwrap();
        assert!(!wallet.detail.to_lowercase().contains("hover"), "phones have no hover");
    }

    #[test]
    fn keep_tinted_sits_under_highlight_on_every_phone() {
        for ios in [false, true] {
            let hit = search_on("keep tinted", 3, ios, &nothing);
            assert_eq!(hit[0].title, "Keep tinted");
            assert_eq!(hit[0].paths, ["chat_design.mention_hold"]);
            assert_eq!(hit[0].under.as_deref(), Some("Highlight"));
        }
    }

    #[test]
    fn moderation_rows_live_on_their_own_page() {
        // Every moderation row sits on the Moderation page, none on Chat.
        for row in rows() {
            let moderation = row.paths.iter().any(|p| {
                p.starts_with("moderation.") || p.starts_with("chat_design.mod_slider") || p == "chat_design.mod_hold_buttons"
            });
            assert_eq!(moderation, row.page == "moderation", "{} is on {}", row.title, row.page);
        }
        for ios in [false, true] {
            let swipe = search_on("swipe to moderate", 3, ios, &nothing);
            assert_eq!((swipe[0].page.as_str(), swipe[0].section.as_str()), ("moderation", "Moderating Messages"));
            let hold = search_on("hold menu buttons", 3, ios, &nothing);
            assert_eq!((hold[0].page.as_str(), hold[0].paths[0].as_str()), ("moderation", "chat_design.mod_hold_buttons"));
            let lengths = search_on("timeout lengths", 3, ios, &nothing);
            assert_eq!((lengths[0].page.as_str(), lengths[0].section.as_str()), ("moderation", "User Cards"));
        }
        // Changed settings lists them after Chat and before Theme.
        let changed = ["chat_design.mod_slider_timeouts".to_string(), "chat_design.font_size".to_string(), "theme".to_string()];
        let pages: Vec<String> = changed_rows_on(&changed, false, &nothing).into_iter().map(|r| r.page).collect();
        let at = |page: &str| pages.iter().position(|p| p == page).unwrap();
        assert!(at("chat") < at("moderation") && at("moderation") < at("theme"), "{pages:?}");
        assert_eq!(rows().iter().find(|r| r.title == "Steps").unwrap().paths.len(), 3);
    }

    #[test]
    fn pinned_messages_rows_show_on_every_phone() {
        for ios in [false, true] {
            let style = search_on("hide pinned", 3, ios, &nothing);
            assert_eq!((style[0].section.as_str(), style[0].title.as_str()), ("Pinned Messages", "Style"), "ios {ios}");
            assert_eq!(style[0].under.as_deref(), Some("Collapsed"));
            let changed = changed_rows_on(&["chat_design.pinned_collapsed_style".to_string()], ios, &nothing);
            assert_eq!(titles(&changed), ["Style"], "ios {ios}");
        }
    }
    #[test]
    fn keep_screen_on_is_searchable_in_the_chats_tab_section() {
        let hit = search_on("keep screen on", 3, false, &nothing);
        assert_eq!(hit[0].title, "Keep screen on");
        assert_eq!(hit[0].section, "Chats Tab");
    }
}
