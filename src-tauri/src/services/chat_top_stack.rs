//! What sits at the top of a chat column, and how much of it may open.
//!
//! A poll, a prediction and the watched stream's drop progress can all want the top of chat at
//! once. Opened together on a short column (a foldable's side-docked chat, a phone in portrait)
//! they left no room to read or type, and a drop card painted over the poll's fold control. So
//! every shell asks Rust for one layout: which cards show in what order, the one card that may
//! be open (the rest are one-line strips), and the most height the whole stack may take. The
//! shells only draw it, scrolling an open card's body inside that height.

use serde::{Deserialize, Serialize};

/// The share of the chat column's height the stack may take, open card included.
pub const MAX_SHARE: f64 = 0.4;
/// Below this chat height (points on the iPhone, CSS pixels elsewhere) nothing opens by itself:
/// every card starts as a strip and the viewer opens the one they want.
pub const COMPACT_BELOW: f64 = 460.0;
/// The stack is never capped below this, so two strips always fit.
pub const MIN_HEIGHT: f64 = 76.0;

/// What a shell knows: the cards it has, and what the viewer did to them.
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct StackRequest {
    /// The cards present: any of `prediction`, `poll`, `drop`.
    pub cards: Vec<String>,
    /// `chat_overlay_order`: `prediction-first` (the default) or `poll-first`.
    pub order: Option<String>,
    /// `chat_design.polls_start_collapsed`.
    pub polls_start_collapsed: bool,
    /// The card the viewer last opened, which stays the open one while it is there.
    pub opened: Option<String>,
    /// Cards the viewer folded, which never open by themselves again.
    pub folded: Vec<String>,
    /// The chat column's height.
    pub chat_height: f64,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct StackSlot {
    pub kind: String,
    /// Open with its body; otherwise its one-line strip.
    pub open: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct StackLayout {
    /// Top to bottom.
    pub slots: Vec<StackSlot>,
    /// The most height the whole stack may take; an open card's body scrolls inside it.
    pub max_height: f64,
}

/// The layout for `req`. The votes come first in the viewer's order, the drop after them (it is
/// news, they ask for something). At most one card is open: the one the viewer opened, else,
/// on a column tall enough, the first vote they have not folded (a poll only when polls do not
/// start collapsed), else the drop when it is alone.
pub fn layout(req: &StackRequest) -> StackLayout {
    let has = |kind: &str| req.cards.iter().any(|c| c == kind);
    let votes: [&str; 2] = if req.order.as_deref() == Some("poll-first") { ["poll", "prediction"] } else { ["prediction", "poll"] };
    let order: Vec<&str> = votes.into_iter().chain(["drop"]).filter(|k| has(k)).collect();
    let folded = |kind: &str| req.folded.iter().any(|f| f == kind);
    let compact = req.chat_height > 0.0 && req.chat_height < COMPACT_BELOW;
    let open = req
        .opened
        .as_deref()
        .filter(|k| has(k))
        .or_else(|| {
            if compact {
                return None;
            }
            order.iter().copied().find(|k| match *k {
                "poll" => !req.polls_start_collapsed && !folded(k),
                "prediction" => !folded(k),
                // The drop opens only with no vote card beside it.
                _ => !folded(k) && order.iter().all(|o| *o == "drop"),
            })
        });
    let max_height = if req.chat_height > 0.0 { (req.chat_height * MAX_SHARE).max(MIN_HEIGHT) } else { f64::INFINITY };
    StackLayout {
        slots: order.iter().map(|k| StackSlot { kind: (*k).to_string(), open: Some(*k) == open }).collect(),
        max_height: if max_height.is_finite() { max_height.round() } else { 0.0 },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(cards: &[&str], height: f64) -> StackRequest {
        StackRequest { cards: cards.iter().map(|c| c.to_string()).collect(), chat_height: height, ..Default::default() }
    }

    fn open_of(l: &StackLayout) -> Vec<&str> {
        l.slots.iter().filter(|s| s.open).map(|s| s.kind.as_str()).collect()
    }

    fn kinds(l: &StackLayout) -> Vec<&str> {
        l.slots.iter().map(|s| s.kind.as_str()).collect()
    }

    #[test]
    fn a_poll_beside_a_drop_opens_and_the_drop_is_a_strip() {
        let l = layout(&req(&["drop", "poll"], 700.0));
        assert_eq!(kinds(&l), vec!["poll", "drop"]);
        assert_eq!(open_of(&l), vec!["poll"]);
        assert_eq!(l.max_height, 280.0);
    }

    #[test]
    fn only_one_card_is_ever_open() {
        let l = layout(&req(&["drop", "poll", "prediction"], 900.0));
        assert_eq!(kinds(&l), vec!["prediction", "poll", "drop"]);
        assert_eq!(open_of(&l), vec!["prediction"]);
        let mut r = req(&["drop", "poll", "prediction"], 900.0);
        r.order = Some("poll-first".into());
        assert_eq!(open_of(&layout(&r)), vec!["poll"]);
    }

    #[test]
    fn a_short_side_docked_column_opens_nothing_by_itself() {
        let l = layout(&req(&["drop", "poll"], 380.0));
        assert!(open_of(&l).is_empty(), "two strips only");
        assert_eq!(l.max_height, 152.0);
        let tiny = layout(&req(&["drop", "poll"], 120.0));
        assert_eq!(tiny.max_height, MIN_HEIGHT, "two strips always fit");
    }

    #[test]
    fn the_viewer_can_always_fold_and_open() {
        let mut r = req(&["drop", "poll"], 700.0);
        r.folded = vec!["poll".into()];
        assert!(open_of(&layout(&r)).is_empty(), "a folded poll stays folded, and the drop stays a strip beside it");
        r.opened = Some("drop".into());
        assert_eq!(open_of(&layout(&r)), vec!["drop"]);
        let mut short = req(&["drop", "poll"], 380.0);
        short.opened = Some("poll".into());
        assert_eq!(open_of(&layout(&short)), vec!["poll"], "a tap opens it even on a short column");
    }

    #[test]
    fn polls_can_start_collapsed() {
        let mut r = req(&["poll"], 700.0);
        r.polls_start_collapsed = true;
        assert!(open_of(&layout(&r)).is_empty());
        let mut both = req(&["poll", "prediction"], 700.0);
        both.order = Some("poll-first".into());
        both.polls_start_collapsed = true;
        assert_eq!(open_of(&layout(&both)), vec!["prediction"]);
    }

    #[test]
    fn a_drop_alone_shows_open() {
        assert_eq!(open_of(&layout(&req(&["drop"], 700.0))), vec!["drop"]);
    }

    #[test]
    fn an_opened_card_that_left_frees_the_slot() {
        let mut r = req(&["drop"], 700.0);
        r.opened = Some("poll".into());
        assert_eq!(open_of(&layout(&r)), vec!["drop"]);
    }

    #[test]
    fn unknown_cards_and_an_unknown_height_are_safe() {
        let l = layout(&req(&["banner", "poll"], 0.0));
        assert_eq!(kinds(&l), vec!["poll"]);
        assert_eq!(open_of(&l), vec!["poll"]);
        assert_eq!(l.max_height, 0.0, "0 = no cap known yet");
    }
}
