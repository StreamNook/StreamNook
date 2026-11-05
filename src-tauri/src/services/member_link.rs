//! The page link a StreamNook member's profile shows: `streamnook.app/<login>`.
//!
//! The login is always the member's own Twitch login for their Twitch id, never
//! a chat platform's. The reserved words (first path segments that belong to the
//! site) are the committed `src/utils/reservedSlugs.json` plus whatever the live
//! update manifest lists (`client_config`), with live entries kept only when they
//! are shaped like a login, so a broken list can never un-reserve a word. A
//! reserved login's page lives at `/u/<login>`; `href` always uses that form
//! (the site sends a member's `/u/` link to the root form), so opening a link is
//! right even when the two lists drift. The accent is the member's rank color
//! lifted until it reads on the plate's own floor over a white page and over a
//! black one.

use crate::services::cosmetics_registry as registry;
use crate::services::profile_view::member_rank;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::LazyLock;

/// The plate's own floor as `[r, g, b, alpha]` (alpha 235 of 255, 0.92), the same
/// on every platform and never affected by glass settings or theme light.
pub const PLATE_FLOOR: [u8; 4] = [14, 14, 18, 235];
/// The plate's fixed text colors, pinned by the contrast test.
pub const PREFIX_HEX: &str = "#9aa3ad";
pub const ARROW_HEX: &str = "#9aa3ad";
/// WCAG ratios: text on the plate, and the arrow glyph (non-text).
pub const TEXT_RATIO: f64 = 4.5;
pub const ARROW_RATIO: f64 = 3.0;

const SITE: &str = "streamnook.app";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PageLink {
    /// What opening the plate loads: always `https://streamnook.app/u/<login>`.
    pub href: String,
    /// The canonical page, shown and copied.
    pub url: String,
    /// `streamnook.app/` or, for a reserved login, `streamnook.app/u/`.
    pub prefix: String,
    /// The login, lowercase.
    pub name: String,
    /// `#rrggbb`, readable on the plate.
    pub accent: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PageLinkAnswer {
    /// `member`, `not_member` or `not_ready`.
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<PageLink>,
}

#[derive(Deserialize)]
struct Reserved {
    slugs: Vec<String>,
}

static COMMITTED: LazyLock<HashSet<String>> = LazyLock::new(|| {
    serde_json::from_str::<Reserved>(include_str!("../../../src/utils/reservedSlugs.json"))
        .expect("reservedSlugs.json")
        .slugs
        .into_iter()
        .map(|s| s.to_lowercase())
        .collect()
});

/// `^[a-z0-9_]{1,25}$`, the site's `VALID_LOGIN`.
pub fn is_valid_login(login: &str) -> bool {
    (1..=25).contains(&login.len())
        && login
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn live_slugs() -> Vec<String> {
    crate::services::client_config::peek()
        .map(|c| c.reserved_slugs)
        .unwrap_or_default()
}

/// Whether a lowercase login is a word the site keeps for itself.
fn is_reserved_in(login: &str, live: &[String]) -> bool {
    COMMITTED.contains(login)
        || live.iter().any(|s| {
            let s = s.to_lowercase();
            is_valid_login(&s) && s == login
        })
}

/// The link for a member with this rank number and Twitch login; None when the
/// login is not shaped like one.
pub fn link_for(number: u32, login: &str, live: &[String]) -> Option<PageLink> {
    let name = login.trim().to_lowercase();
    if !is_valid_login(&name) {
        return None;
    }
    let prefix = if is_reserved_in(&name, live) {
        format!("{SITE}/u/")
    } else {
        format!("{SITE}/")
    };
    Some(PageLink {
        href: format!("https://{SITE}/u/{name}"),
        url: format!("https://{prefix}{name}"),
        accent: plate_accent(&member_rank(number).accent),
        prefix,
        name,
    })
}

/// The member's page link: None for a non-member or a login that is not valid.
pub fn page_link(member_id: &str, login: &str) -> Option<PageLink> {
    let number = registry::member_number(member_id)?;
    link_for(number, login, &live_slugs())
}

/// The three-state answer the desktop card asks for: `not_ready` while the member
/// list has not loaded or the login is not known yet.
pub fn answer_for(
    loaded: bool,
    number: Option<u32>,
    login: &str,
    live: &[String],
) -> PageLinkAnswer {
    fn answer(state: &str, link: Option<PageLink>) -> PageLinkAnswer {
        PageLinkAnswer {
            state: state.to_string(),
            link,
        }
    }
    if !loaded {
        return answer("not_ready", None);
    }
    let Some(number) = number else {
        return answer("not_member", None);
    };
    if login.trim().is_empty() {
        return answer("not_ready", None);
    }
    match link_for(number, login, live) {
        Some(link) => answer("member", Some(link)),
        None => answer("not_member", None),
    }
}

pub fn link_answer(member_id: &str, login: &str) -> PageLinkAnswer {
    answer_for(
        registry::members_loaded(),
        registry::member_number(member_id),
        login,
        &live_slugs(),
    )
}

// ---- Contrast ------------------------------------------------------------------

fn parse_hex(hex: &str) -> Option<[u8; 3]> {
    let h = hex.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

fn to_hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn channel(v: u8) -> f64 {
    let s = f64::from(v) / 255.0;
    if s <= 0.03928 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG relative luminance.
pub fn luminance(c: [u8; 3]) -> f64 {
    0.2126 * channel(c[0]) + 0.7152 * channel(c[1]) + 0.0722 * channel(c[2])
}

/// WCAG contrast ratio of two opaque colors.
pub fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// The plate's floor laid over an opaque page color.
pub fn plate_over(page: [u8; 3]) -> [u8; 3] {
    let a = f64::from(PLATE_FLOOR[3]) / 255.0;
    let mix =
        |floor: u8, under: u8| (f64::from(floor) * a + f64::from(under) * (1.0 - a)).round() as u8;
    [
        mix(PLATE_FLOOR[0], page[0]),
        mix(PLATE_FLOOR[1], page[1]),
        mix(PLATE_FLOOR[2], page[2]),
    ]
}

/// `hex` lightened toward white, in 2% steps, until it reads at `ratio` on the
/// plate's floor composited over `floor_over` (the page color under the plate).
/// A color that already reads comes back unchanged (lowercase); white is the end
/// of the walk. A string that is not `#rrggbb` is returned as given.
pub fn lift_to_contrast(hex: &str, floor_over: [u8; 3], ratio: f64) -> String {
    let Some(base) = parse_hex(hex) else {
        return hex.to_string();
    };
    let plate = plate_over(floor_over);
    if contrast(base, plate) >= ratio {
        return to_hex(base);
    }
    for step in 1..=50 {
        let t = f64::from(step) / 50.0;
        let mixed = [0, 1, 2]
            .map(|i| (f64::from(base[i]) + (255.0 - f64::from(base[i])) * t).round() as u8);
        if contrast(mixed, plate) >= ratio {
            return to_hex(mixed);
        }
    }
    "#ffffff".to_string()
}

/// The accent a rank color shows as on the plate: readable over the floor on a
/// white page and on a black one.
pub fn plate_accent(hex: &str) -> String {
    let over_white = lift_to_contrast(hex, [255, 255, 255], TEXT_RATIO);
    lift_to_contrast(&over_white, [0, 0, 0], TEXT_RATIO)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn link(number: u32, login: &str) -> Option<PageLink> {
        link_for(number, login, &[])
    }

    #[test]
    fn a_plain_login_links_to_the_root_form_and_opens_the_u_form() {
        let l = link(7, "Tyler").unwrap();
        assert_eq!(l.name, "tyler");
        assert_eq!(l.href, "https://streamnook.app/u/tyler");
        assert_eq!(l.url, "https://streamnook.app/tyler");
        assert_eq!(l.prefix, "streamnook.app/");
        assert_eq!(l.accent, "#fbbf24");
    }

    #[test]
    fn a_reserved_login_shows_and_copies_the_u_form() {
        let l = link(300, "help").unwrap();
        assert_eq!(l.prefix, "streamnook.app/u/");
        assert_eq!(l.url, "https://streamnook.app/u/help");
        assert_eq!(l.href, "https://streamnook.app/u/help");
        // Reserved words match in any case.
        assert_eq!(link(300, "HELP").unwrap().prefix, "streamnook.app/u/");
    }

    #[test]
    fn a_login_that_is_not_shaped_like_one_has_no_link() {
        assert!(link(7, "").is_none());
        assert!(link(7, "has space").is_none());
        assert!(link(7, "dot.dot").is_none());
        assert!(link(7, "한국어").is_none());
        assert!(link(7, &"a".repeat(26)).is_none());
        assert!(link(7, &"a".repeat(25)).is_some());
        assert!(link(7, "a_1").is_some());
    }

    #[test]
    fn a_live_entry_adds_a_word_and_a_malformed_one_never_removes_one() {
        let live = vec![
            "Brandnew".to_string(),
            "bad word".to_string(),
            "".to_string(),
            "x".repeat(40),
        ];
        assert_eq!(
            link_for(7, "brandnew", &live).unwrap().prefix,
            "streamnook.app/u/"
        );
        // The committed list still holds with a broken live list.
        assert_eq!(
            link_for(7, "help", &live).unwrap().prefix,
            "streamnook.app/u/"
        );
        assert_eq!(
            link_for(7, "tyler", &live).unwrap().prefix,
            "streamnook.app/"
        );
        assert!(is_reserved_in("help", &[]));
    }

    #[test]
    fn the_three_state_answer() {
        let live: Vec<String> = Vec::new();
        assert_eq!(
            answer_for(false, Some(7), "tyler", &live).state,
            "not_ready"
        );
        assert_eq!(answer_for(false, None, "tyler", &live).state, "not_ready");
        let none = answer_for(true, None, "tyler", &live);
        assert_eq!(
            (none.state.as_str(), none.link.is_none()),
            ("not_member", true)
        );
        assert_eq!(answer_for(true, Some(7), "", &live).state, "not_ready");
        assert_eq!(answer_for(true, Some(7), "  ", &live).state, "not_ready");
        assert_eq!(
            answer_for(true, Some(7), "not valid", &live).state,
            "not_member"
        );
        let member = answer_for(true, Some(7), "Tyler", &live);
        assert_eq!(member.state, "member");
        assert_eq!(member.link.unwrap().name, "tyler");
        let wire = serde_json::to_value(answer_for(true, None, "x", &live)).unwrap();
        assert_eq!(wire, serde_json::json!({ "state": "not_member" }));
    }

    #[test]
    fn every_rank_accent_reads_on_the_plate_over_white_and_over_black() {
        let over_white = plate_over([255, 255, 255]);
        let over_black = plate_over([0, 0, 0]);
        for number in [1, 7, 50, 200, 1000] {
            let rank = member_rank(number);
            let accent = link(number, "tyler").unwrap().accent;
            let rgb = parse_hex(&accent).expect("a #rrggbb accent");
            assert!(
                contrast(rgb, over_white) >= TEXT_RATIO,
                "{} {accent} over white",
                rank.tier
            );
            assert!(
                contrast(rgb, over_black) >= TEXT_RATIO,
                "{} {accent} over black",
                rank.tier
            );
        }
        // Ethereal violet is too dark as is, so it is lifted; amber already reads and is kept.
        assert_ne!(link(1, "tyler").unwrap().accent, "#8b5cf6");
        assert_eq!(link(7, "tyler").unwrap().accent, "#fbbf24");
    }

    #[test]
    fn the_fixed_plate_colors_read_on_the_floor() {
        let prefix = parse_hex(PREFIX_HEX).unwrap();
        let arrow = parse_hex(ARROW_HEX).unwrap();
        for page in [[255, 255, 255], [0, 0, 0]] {
            let plate = plate_over(page);
            assert!(
                contrast(prefix, plate) >= TEXT_RATIO,
                "prefix over {page:?}"
            );
            assert!(contrast(arrow, plate) >= ARROW_RATIO, "arrow over {page:?}");
        }
        assert_eq!(PLATE_FLOOR, [14, 14, 18, 235]);
    }

    #[test]
    fn lifting_keeps_a_readable_color_and_walks_a_dark_one_toward_white() {
        assert_eq!(lift_to_contrast("#FFFFFF", [255, 255, 255], 4.5), "#ffffff");
        let lifted = lift_to_contrast("#202020", [0, 0, 0], 4.5);
        assert!(contrast(parse_hex(&lifted).unwrap(), plate_over([0, 0, 0])) >= 4.5);
        assert_eq!(
            lift_to_contrast("not a color", [0, 0, 0], 4.5),
            "not a color"
        );
        assert_eq!(lift_to_contrast("#000000", [0, 0, 0], 100.0), "#ffffff");
    }

    #[test]
    fn the_committed_reserved_list_is_clean() {
        let slugs: Vec<String> =
            serde_json::from_str::<Reserved>(include_str!("../../../src/utils/reservedSlugs.json"))
                .unwrap()
                .slugs;
        assert!(!slugs.is_empty());
        let mut seen = HashSet::new();
        for s in &slugs {
            assert_eq!(s, &s.to_lowercase(), "{s} must be lowercase");
            assert!(is_valid_login(s), "{s} must be shaped like a login");
            assert!(seen.insert(s.clone()), "{s} is listed twice");
        }
    }

    /// The site's checkout: `SN_SITE_DIR`, else `C:/streamnook.app`, else
    /// `/mnt/c/streamnook.app`.
    fn site_dir() -> Option<PathBuf> {
        if let Ok(dir) = std::env::var("SN_SITE_DIR") {
            let path = PathBuf::from(dir);
            if path.exists() {
                return Some(path);
            }
        }
        ["C:/streamnook.app", "/mnt/c/streamnook.app"]
            .iter()
            .map(PathBuf::from)
            .find(|p| p.exists())
    }

    /// The words in the site's `RESERVED_SLUGS` set, in its order.
    fn site_slugs(ts: &str) -> Vec<String> {
        let start = ts.find("new Set([").expect("RESERVED_SLUGS set") + "new Set([".len();
        let end = start + ts[start..].find("])").expect("RESERVED_SLUGS set end");
        ts[start..end]
            .lines()
            .map(|l| l.split("//").next().unwrap_or("").trim())
            .filter(|l| !l.is_empty())
            .flat_map(|l| {
                l.split('\'')
                    .skip(1)
                    .step_by(2)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    #[test]
    fn the_site_list_is_read_word_by_word_in_order() {
        let ts = "export const RESERVED_SLUGS = new Set([\n  // Pages\n  'api',\n  'u', // short\n\n  'help',\n]);\n";
        assert_eq!(site_slugs(ts), vec!["api", "u", "help"]);
    }

    #[test]
    fn the_committed_reserved_list_equals_the_sites() {
        let Some(dir) = site_dir() else {
            assert!(
                std::env::var_os("CI").is_some(),
                "the site checkout was not found: set SN_SITE_DIR to it (or CI to skip this check)"
            );
            return;
        };
        let ts = std::fs::read_to_string(dir.join("src/lib/reservedSlugs.ts"))
            .expect("the site's src/lib/reservedSlugs.ts")
            .replace("\r\n", "\n");
        let committed: Vec<String> =
            serde_json::from_str::<Reserved>(include_str!("../../../src/utils/reservedSlugs.json"))
                .unwrap()
                .slugs;
        assert_eq!(
            committed,
            site_slugs(&ts),
            "src/utils/reservedSlugs.json must equal the site's RESERVED_SLUGS"
        );
    }
}
