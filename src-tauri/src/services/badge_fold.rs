//! Which of a chatter's badges a chat row shows before its "+N" chip; the rest
//! unfold from the chip. Every surface folds by this rule: the iPhone row
//! layout calls it (`ios_ffi::badge_fold_plan`), and the webview's twin
//! (`src/utils/badgeFold.ts`) reads the same rules file and runs the same cases.

use serde::Deserialize;
use std::sync::LazyLock;

/// What a badge slot is, in the row's chat order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FoldKind {
    /// A platform badge naming a role (broadcaster, staff, moderator, VIP...).
    Role,
    Platform,
    Seventv,
    Addon,
    Streamnook,
}

impl FoldKind {
    /// An add-on badge (7TV, the chat clients, StreamNook): the only kind the
    /// limit folds and the rarity tier counts. The platform's own badges (role,
    /// subscriber, global, predictions) always show and count toward neither.
    pub fn is_addon(self) -> bool {
        matches!(
            self,
            FoldKind::Seventv | FoldKind::Addon | FoldKind::Streamnook
        )
    }
}

#[derive(Deserialize)]
struct Rules {
    role_sets: Vec<String>,
    platform_providers: Vec<String>,
    /// Add-on counts at which each rarity tier starts (uncommon, rare, epic, legendary).
    tiers: Vec<usize>,
}

static RULES: LazyLock<Rules> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../src/utils/badgeFold.rules.json"))
        .expect("badgeFold.rules.json")
});

/// A badge's kind from its provider key and (for platform badges) its set name.
pub fn fold_kind(provider: &str, set_name: &str) -> FoldKind {
    match provider {
        "7tv" => FoldKind::Seventv,
        "streamnook" => FoldKind::Streamnook,
        p if RULES.platform_providers.iter().any(|x| x == p) => {
            if RULES.role_sets.iter().any(|r| r == set_name) {
                FoldKind::Role
            } else {
                FoldKind::Platform
            }
        }
        _ => FoldKind::Addon,
    }
}

/// How rare a chatter's add-on collection is: 0 plain, then uncommon, rare,
/// epic, legendary. It counts every add-on they wear, so the viewer's limit
/// never changes it (a "+2" and a "+3" of the same chatter look alike).
pub fn rarity_tier(addons: usize) -> u8 {
    RULES.tiers.iter().filter(|start| addons >= **start).count() as u8
}

/// Indices into the row's badges, each list in chat order, and the rarity tier.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BadgeFold {
    pub shown: Vec<usize>,
    pub hidden: Vec<usize>,
    pub tier: u8,
}

/// Folds a row's badges for `limit` add-ons (0 = show all). The platform's own
/// badges always show and never count. Add-ons keep the StreamNook badge first,
/// then the rest in chat order, up to `limit`; the rest fold into the chip,
/// even a single one, so the setting means exactly what it says.
pub fn fold_badges(kinds: &[FoldKind], limit: usize) -> BadgeFold {
    let addons: Vec<usize> = (0..kinds.len()).filter(|&i| kinds[i].is_addon()).collect();
    let tier = rarity_tier(addons.len());
    if limit == 0 || addons.len() <= limit {
        return BadgeFold {
            shown: (0..kinds.len()).collect(),
            hidden: Vec::new(),
            tier,
        };
    }
    let member = addons
        .iter()
        .copied()
        .filter(|&i| kinds[i] == FoldKind::Streamnook);
    let rest = addons
        .iter()
        .copied()
        .filter(|&i| kinds[i] != FoldKind::Streamnook);
    let kept: Vec<usize> = member.chain(rest).take(limit).collect();
    let shown = (0..kinds.len())
        .filter(|i| !kinds[*i].is_addon() || kept.contains(i))
        .collect();
    let hidden = addons.into_iter().filter(|i| !kept.contains(i)).collect();
    BadgeFold {
        shown,
        hidden,
        tier,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct Case {
        name: String,
        kinds: Vec<FoldKind>,
        limit: usize,
        shown: Vec<usize>,
        hidden: Vec<usize>,
        tier: u8,
    }

    #[derive(Deserialize)]
    struct File {
        cases: Vec<Case>,
    }

    #[test]
    fn every_shared_case_folds_as_the_table_says() {
        let file: File =
            serde_json::from_str(include_str!("../../../src/utils/badgeFold.rules.json")).unwrap();
        assert!(file.cases.len() >= 12);
        for c in file.cases {
            assert_eq!(
                fold_badges(&c.kinds, c.limit),
                BadgeFold {
                    shown: c.shown,
                    hidden: c.hidden,
                    tier: c.tier
                },
                "case {}",
                c.name
            );
        }
    }

    #[test]
    fn kinds_come_from_provider_and_set() {
        assert_eq!(fold_kind("twitch", "moderator"), FoldKind::Role);
        assert_eq!(fold_kind("", "vip"), FoldKind::Role);
        assert_eq!(fold_kind("kick", "subscriber"), FoldKind::Platform);
        assert_eq!(fold_kind("youtube", "owner"), FoldKind::Role);
        assert_eq!(fold_kind("7tv", ""), FoldKind::Seventv);
        assert_eq!(fold_kind("limerino", ""), FoldKind::Addon);
        assert_eq!(fold_kind("streamnook", ""), FoldKind::Streamnook);
    }

    #[test]
    fn tiers_start_at_two_three_four_and_six_add_ons() {
        let tiers: Vec<u8> = (0..=7).map(rarity_tier).collect();
        assert_eq!(tiers, vec![0, 0, 1, 2, 3, 3, 4, 4]);
    }
}
