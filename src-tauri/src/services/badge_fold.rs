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

#[derive(Deserialize)]
struct Rules {
    role_sets: Vec<String>,
    platform_providers: Vec<String>,
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

/// Indices into the row's badges, each list in chat order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BadgeFold {
    pub shown: Vec<usize>,
    pub hidden: Vec<usize>,
}

/// Folds a row's badges for `limit` (0 = show all). Nothing folds unless two
/// or more would hide (a "+1" chip is as wide as its badge). Kept: the
/// chatter's role badge (else their first platform badge), their 7TV badge and
/// their StreamNook badge, in that priority, filled from the rest in chat order.
pub fn fold_badges(kinds: &[FoldKind], limit: usize) -> BadgeFold {
    if limit == 0 || kinds.len() <= limit + 1 {
        return BadgeFold {
            shown: (0..kinds.len()).collect(),
            hidden: Vec::new(),
        };
    }
    let first = |kind: FoldKind| kinds.iter().position(|k| *k == kind);
    let mut picks: Vec<usize> = Vec::with_capacity(limit);
    let platform = first(FoldKind::Role).or_else(|| first(FoldKind::Platform));
    for i in [
        platform,
        first(FoldKind::Seventv),
        first(FoldKind::Streamnook),
    ]
    .into_iter()
    .flatten()
    {
        if picks.len() < limit {
            picks.push(i);
        }
    }
    for i in 0..kinds.len() {
        if picks.len() >= limit {
            break;
        }
        if !picks.contains(&i) {
            picks.push(i);
        }
    }
    picks.sort_unstable();
    let hidden = (0..kinds.len()).filter(|i| !picks.contains(i)).collect();
    BadgeFold {
        shown: picks,
        hidden,
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
                    hidden: c.hidden
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
}
