//! Links into the app itself, for the update notes: `streamnook://settings/...`
//! lands on a settings tab or row and `streamnook://open/...` opens a page.
//!
//! Only these forms resolve, and only to places that open something to look at:
//! a link never runs an action (no sign-out, no clearing, no stream controls).
//! Anything else answers None and the click does nothing.
//!
//! - `streamnook://settings/<Tab>` and `streamnook://settings/<Tab>/<Row title>`,
//!   path segments percent-encoded (`Italic%20%2Fme%20messages`).
//! - `streamnook://open/<page>`, a page from [`AppPage`].

use serde::Serialize;

/// The desktop Settings tabs a link may name, as the dialog spells them.
const SETTINGS_TABS: &[&str] = &[
    "Profile",
    "Interface",
    "Player",
    "Chat",
    "Moderation",
    "Overlay",
    "Theme",
    "Integrations",
    "Plugins",
    "Notifications",
    "Automations",
    "Cache",
    "Command Palette",
    "Keybindings",
    "Backup",
    "Support",
    "Analytics",
];

/// The longest row title a link may carry; real titles are a few words.
const MAX_ROW_LEN: usize = 120;

/// A page a link may open.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppPage {
    Drops,
    Whispers,
    Badges,
    EmoteSets,
    Multichat,
    Songs,
    Profile,
    Home,
}

impl AppPage {
    fn from_slug(slug: &str) -> Option<Self> {
        Some(match slug {
            "drops" => Self::Drops,
            "whispers" => Self::Whispers,
            "badges" => Self::Badges,
            "emote-sets" => Self::EmoteSets,
            "multichat" => Self::Multichat,
            "songs" => Self::Songs,
            "profile" => Self::Profile,
            "home" => Self::Home,
            _ => return None,
        })
    }
}

/// Where a link goes.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppLink {
    /// A settings tab, and the row to land on when the link names one.
    Settings { tab: String, row: Option<String> },
    Open { page: AppPage },
}

/// Read a link into where it goes, or None for anything that is not one of the
/// forms above.
pub fn parse(link: &str) -> Option<AppLink> {
    let url = url::Url::parse(link.trim()).ok()?;
    if url.scheme() != "streamnook" || url.fragment().is_some() {
        return None;
    }
    // Only `settings/Automations?rule=<id>` carries a query; it lands on that
    // rule as the row `rule:<id>`.
    let query = url.query();
    let segments: Vec<String> = url
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).map(decode).collect::<Option<Vec<_>>>())
        .unwrap_or(Some(Vec::new()))?;
    match (url.host_str()?, segments.as_slice()) {
        ("open", [page]) if query.is_none() => AppPage::from_slug(page).map(|page| AppLink::Open { page }),
        ("settings", [tab]) => {
            let tab = settings_tab(tab)?;
            let row = match query {
                None => None,
                Some(q) if tab == "Automations" => Some(rule_link_row(q)?),
                Some(_) => return None,
            };
            Some(AppLink::Settings { tab, row })
        }
        ("settings", [tab, row]) if query.is_none() => {
            let row = row.trim();
            if row.is_empty() || row.chars().count() > MAX_ROW_LEN || row.chars().any(char::is_control) {
                return None;
            }
            settings_tab(tab).map(|tab| AppLink::Settings { tab, row: Some(row.to_string()) })
        }
        _ => None,
    }
}

/// `rule=<id>` (one pair, an id of 1 to 64 letters, digits, `_` or `-`) as the
/// row `rule:<id>`.
fn rule_link_row(query: &str) -> Option<String> {
    let id = query.strip_prefix("rule=")?;
    let ok = !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    ok.then(|| format!("rule:{id}"))
}

fn decode(segment: &str) -> Option<String> {
    urlencoding::decode(segment).ok().map(|s| s.into_owned())
}

fn settings_tab(name: &str) -> Option<String> {
    SETTINGS_TABS.iter().find(|t| t.eq_ignore_ascii_case(name.trim())).map(|t| t.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_settings_link_names_its_tab_and_row() {
        assert_eq!(
            parse("streamnook://settings/Chat/Italic%20%2Fme%20messages"),
            Some(AppLink::Settings { tab: "Chat".into(), row: Some("Italic /me messages".into()) })
        );
        assert_eq!(
            parse("streamnook://settings/command%20palette"),
            Some(AppLink::Settings { tab: "Command Palette".into(), row: None })
        );
    }

    #[test]
    fn a_rule_link_lands_on_its_row() {
        assert_eq!(
            parse("streamnook://settings/Automations?rule=ab12-cd_3"),
            Some(AppLink::Settings { tab: "Automations".into(), row: Some("rule:ab12-cd_3".into()) })
        );
        assert_eq!(parse("streamnook://settings/automations"), Some(AppLink::Settings { tab: "Automations".into(), row: None }));
        for bad in [
            "streamnook://settings/Chat?rule=x",
            "streamnook://settings/Automations?rule=",
            "streamnook://settings/Automations?rule=a%20b",
            "streamnook://settings/Automations?rule=a&x=1",
            "streamnook://settings/Automations?x=1",
            "streamnook://settings/Automations/Row?rule=x",
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn an_open_link_names_a_page() {
        assert_eq!(parse("streamnook://open/drops"), Some(AppLink::Open { page: AppPage::Drops }));
        assert_eq!(parse("streamnook://open/emote-sets"), Some(AppLink::Open { page: AppPage::EmoteSets }));
    }

    #[test]
    fn anything_else_goes_nowhere() {
        for link in [
            "https://streamnook.app/open/drops",
            "streamnook://open/sign-out",
            "streamnook://open/drops/extra",
            "streamnook://open/drops?then=clear",
            "streamnook://settings/Nowhere",
            "streamnook://settings/Chat/%20",
            "streamnook://settings/Chat/a/b",
            "streamnook://watch/xqc",
            "streamnook://notify?a=x",
            "not a link",
        ] {
            assert_eq!(parse(link), None, "{link}");
        }
    }

    #[test]
    fn the_tabs_match_the_settings_dialog() {
        let source = include_str!("../../../src/stores/AppStore.ts");
        let line = source.lines().find(|l| l.starts_with("export type SettingsTab =")).expect("SettingsTab");
        let mut named: Vec<&str> = line.split('\'').skip(1).step_by(2).collect();
        let mut ours = SETTINGS_TABS.to_vec();
        named.sort_unstable();
        ours.sort_unstable();
        assert_eq!(named, ours);
    }
}
