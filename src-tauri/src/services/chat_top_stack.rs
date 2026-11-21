//! What sits at the top of a chat column, and how much of it may open.
//!
//! A poll, a prediction and the watched stream's drop progress can all want the top of chat at
//! once. Opened together on a short column (a foldable's side-docked chat, a phone in portrait)
//! they left no room to read or type, and a drop card painted over the poll's fold control. So
//! every shell asks Rust for one layout: which cards show in what order, the one card that may
//! be open (the rest are one-line strips), and the most height the whole stack may take. The
//! shells only draw it, scrolling an open card's body inside that height.

//!
//! The seven cards (SN-284): `hype`, the votes (`prediction`, `poll`), `drop`, the channel's
//! pinned message (`channel_pin`), the mention context card (`mention_context`) and the pinned
//! mentions (`mention_pins`). The stack floats over the chat on every surface and never takes
//! more than `MAX_SHARE` of it. Budget, in order: the pinned mentions' floor (the newest row
//! and a count) is reserved first and never folded; the other cards are strips, and the ones
//! that do not fit fold, lowest rank (last) first, into one "N more cards" strip; the open card
//! takes what it needs next; the pinned mentions take the rest, up to their natural height, and
//! scroll inside. On a column too short for the floor and a "N more cards" strip, the floor is
//! one line with the count inline and "N more cards" merges into that count line. Which card
//! is open never depends on heights, so a measured height can never change the choice and
//! loop. The shared cases (`tests/fixtures/chat-top-stack-cases.json`) pin all of it for every
//! shell.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The share of the chat column's height the stack may take, open card included.
pub const MAX_SHARE: f64 = 0.4;
/// Below this chat height (points on the iPhone, CSS pixels elsewhere) nothing opens by itself:
/// every card starts as a strip and the viewer opens the one they want.
pub const COMPACT_BELOW: f64 = 460.0;
/// The stack is never capped below this, so two strips always fit.
pub const MIN_HEIGHT: f64 = 76.0;

/// Every card, in the order the stack draws them top to bottom.
pub const CARD_ORDER: [&str; 7] = ["hype", "prediction", "poll", "drop", "channel_pin", "mention_context", "mention_pins"];

/// A card's measured heights per form. Shells keep the last measured height of each form and
/// send it; a missing or zero field takes the default for the card's kind.
#[derive(Deserialize, Serialize, Clone, Copy, Debug, Default, PartialEq)]
#[serde(default)]
pub struct CardHeights {
    /// The one-line strip. For `mention_pins`: the floor (newest row plus the count).
    pub strip: f64,
    /// The open body at its natural height. For `mention_pins`: every row, natural.
    pub open: f64,
    /// `mention_pins` only: the one-line form with the count inline.
    pub line: f64,
}

/// The height a strip, an open body and the count line take when a shell has not measured.
fn default_heights(kind: &str) -> CardHeights {
    match kind {
        "mention_pins" => CardHeights { strip: 64.0, open: 200.0, line: 28.0 },
        "poll" => CardHeights { strip: 36.0, open: 150.0, line: 0.0 },
        "prediction" => CardHeights { strip: 36.0, open: 170.0, line: 0.0 },
        "drop" => CardHeights { strip: 36.0, open: 120.0, line: 0.0 },
        "channel_pin" => CardHeights { strip: 36.0, open: 90.0, line: 0.0 },
        "mention_context" => CardHeights { strip: 36.0, open: 80.0, line: 0.0 },
        // The hype train has one form.
        _ => CardHeights { strip: 36.0, open: 36.0, line: 0.0 },
    }
}

/// The "N more cards" strip's height.
const MORE_HEIGHT: f64 = 28.0;

/// What a shell knows: the cards it has, and what the viewer did to them.
#[derive(Deserialize, Serialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct StackRequest {
    /// The cards present: any of `hype`, `prediction`, `poll`, `drop`, `channel_pin`,
    /// `mention_context`, `mention_pins`.
    pub cards: Vec<String>,
    /// `chat_overlay_order`: `prediction-first` (the default) or `poll-first`.
    pub order: Option<String>,
    /// `chat_design.polls_start_collapsed`.
    pub polls_start_collapsed: bool,
    /// `chat_design.predictions_start_collapsed`.
    pub predictions_start_collapsed: bool,
    /// The prediction is closed to bets and waiting for its result (`locked` or
    /// `resolving`). Nothing in it can be acted on, so it never opens by itself.
    pub prediction_waiting: bool,
    /// The card the viewer last opened, which stays the open one while it is there. The
    /// per-room open/folded store feeds this for every card kind, the channel pin's
    /// (`ChatRooms.pinsOpen`) included.
    pub opened: Option<String>,
    /// Cards the viewer folded, which never open by themselves again.
    pub folded: Vec<String>,
    /// The chat column's height.
    pub chat_height: f64,
    /// The channel's pinned message is open (`ChatRooms.pinsOpen`, every writer): the open
    /// card when the viewer opened nothing else.
    pub channel_pin_opened: bool,
    /// Each card's last measured heights, per form. A kind not named takes the default.
    pub heights: BTreeMap<String, CardHeights>,
    /// The lower third: strips only, nothing opens, the pins show as their floor.
    pub strips_only: bool,
    /// The chat this surface shows, so the answer names its room key and no shell builds one.
    pub provider: Option<String>,
    pub channel: Option<String>,
}

/// How one card is drawn.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Form {
    /// One line.
    Strip,
    /// Open, with its body (scrolling inside when it overflows).
    Open,
    /// Folded into the "N more cards" strip (or the pins' count line): drawn nowhere.
    Folded,
    /// The pinned mentions: the newest row and the count.
    Floor,
    /// The pinned mentions: every row up to the natural height, scrolling inside, newest in view.
    Full,
    /// The pinned mentions on a column too short for the floor: one line, count inline.
    CountLine,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StackSlot {
    pub kind: String,
    /// The card the stack chose to be open. Heights never change it, even when the card has
    /// no room and is folded.
    pub open: bool,
    pub form: Form,
    /// The height this card takes, whole pixels; 0 when folded.
    pub height: f64,
    /// `CountLine` only: how many cards folded into it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub more_cards: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// The "N more cards" strip: the cards folded into it, lowest rank last.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MoreCards {
    pub count: usize,
    pub kinds: Vec<String>,
    pub height: f64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StackLayout {
    /// Top to bottom, every card present (folded ones included, marked by their form).
    pub slots: Vec<StackSlot>,
    /// The most height the whole stack may take; an open card's body scrolls inside it.
    /// 0 = no cap known yet.
    pub max_height: f64,
    /// The "N more cards" strip, drawn after the last slot, when cards folded and the pins
    /// have room to say so apart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub more: Option<MoreCards>,
    /// What the whole stack takes, `more` included.
    pub height: f64,
    /// The key of the chat this surface shows (the pins' open/folded store is keyed by it);
    /// empty when the request named no chat.
    pub room_key: String,
}

fn height_of(req: &StackRequest, kind: &str) -> CardHeights {
    let d = default_heights(kind);
    let m = req.heights.get(kind).copied().unwrap_or_default();
    let pick = |measured: f64, default: f64| if measured > 0.0 { measured } else { default };
    CardHeights { strip: pick(m.strip, d.strip), open: pick(m.open, d.open).max(pick(m.strip, d.strip)), line: pick(m.line, d.line) }
}

/// Which card is open. Pure over the request without its heights: a measured height can
/// never change it. The viewer's last open card stays; else, on a column tall enough, the first
/// vote they have not folded (a poll only when polls do not start collapsed; a prediction only
/// when predictions do not, and never one waiting for its result), else the drop when it is
/// alone. The hype train, the pinned mentions and (unless opened) the channel pin and the
/// mention context never open by themselves; the lower third opens nothing.
pub fn open_card(req: &StackRequest) -> Option<String> {
    if req.strips_only {
        return None;
    }
    let has = |kind: &str| req.cards.iter().any(|c| c == kind);
    const OPENABLE: [&str; 5] = ["prediction", "poll", "drop", "channel_pin", "mention_context"];
    let by_viewer = req.opened.as_deref().filter(|k| has(k) && OPENABLE.contains(k));
    if let Some(k) = by_viewer {
        return Some(k.to_string());
    }
    if req.channel_pin_opened && has("channel_pin") {
        return Some("channel_pin".to_string());
    }
    let votes: [&str; 2] = if req.order.as_deref() == Some("poll-first") { ["poll", "prediction"] } else { ["prediction", "poll"] };
    let order: Vec<&str> = votes.into_iter().chain(["drop"]).filter(|k| has(k)).collect();
    let folded = |kind: &str| req.folded.iter().any(|f| f == kind);
    let compact = req.chat_height > 0.0 && req.chat_height < COMPACT_BELOW;
    if compact {
        return None;
    }
    order
        .iter()
        .copied()
        .find(|k| match *k {
            "poll" => !req.polls_start_collapsed && !folded(k),
            "prediction" => !req.predictions_start_collapsed && !req.prediction_waiting && !folded(k),
            // The drop opens only with no vote card beside it.
            _ => !folded(k) && order.iter().all(|o| *o == "drop"),
        })
        .map(str::to_string)
}

/// The layout for `req`: the cards in order (hype, the votes in the viewer's order, the drop,
/// the channel pin, the mention context, the pinned mentions), at most one open, every card
/// given a form and a height inside the cap.
pub fn layout(req: &StackRequest) -> StackLayout {
    let has = |kind: &str| req.cards.iter().any(|c| c == kind);
    let votes: [&str; 2] = if req.order.as_deref() == Some("poll-first") { ["poll", "prediction"] } else { ["prediction", "poll"] };
    let order: Vec<&str> = std::iter::once(CARD_ORDER[0])
        .chain(votes)
        .chain(CARD_ORDER[3..].iter().copied())
        .filter(|k| has(k))
        .collect();
    let open = open_card(req);
    let cap = (req.chat_height > 0.0).then(|| (req.chat_height * MAX_SHARE).max(MIN_HEIGHT));
    let pins = order.contains(&"mention_pins");
    let others: Vec<&str> = order.iter().copied().filter(|k| *k != "mention_pins").collect();
    let open_kind = open.as_deref();
    let strip = |k: &str| height_of(req, k).strip;
    let room_key = match req.channel.as_deref().filter(|c| !c.trim().is_empty()) {
        Some(channel) => crate::services::mention_inbox::room_key(req.provider.as_deref().unwrap_or(""), channel),
        None => String::new(),
    };

    // Pass A: the floor, a separate "N more cards" strip. Pass B (too short for both): the
    // floor is one line with the count inline and the fold merges into it.
    let floor = height_of(req, "mention_pins");
    let strips: Vec<&str> = others.iter().copied().filter(|k| Some(*k) != open_kind).collect();
    let open_min = open_kind.filter(|k| others.contains(k)).map(strip).unwrap_or(0.0);
    let all_strips: f64 = strips.iter().map(|k| strip(k)).sum();
    let floor_h = if pins { floor.strip } else { 0.0 };
    let unlimited = cap.is_none();
    let cap_v = cap.unwrap_or(f64::INFINITY);

    let fits_all = floor_h + all_strips + open_min <= cap_v;
    let pass_a = fits_all || floor_h + MORE_HEIGHT + open_min <= cap_v;
    let merged = pins && !pass_a;
    let (pins_base, reserve_more) = if merged { (floor.line, false) } else { (floor_h, true) };

    // The strips that stay: a prefix (lowest rank folds first) that leaves room for the
    // open card's strip and, when something folds, the "N more cards" strip.
    let mut kept: Vec<&str> = Vec::new();
    let mut used = pins_base + open_min;
    for (i, k) in strips.iter().enumerate() {
        let remaining_after: usize = strips.len() - i - 1;
        let more_needed = if reserve_more && remaining_after > 0 { MORE_HEIGHT } else { 0.0 };
        if used + strip(k) + more_needed <= cap_v {
            used += strip(k);
            kept.push(k);
        } else {
            break;
        }
    }
    let folded_kinds: Vec<String> = strips.iter().filter(|k| !kept.contains(k)).map(|k| (*k).to_string()).collect();
    // Under the merged form the open card folds too when it has no room at all.
    let open_folded = open_kind.is_some_and(|k| others.contains(&k)) && used > cap_v;
    let mut all_folded = folded_kinds.clone();
    if open_folded {
        all_folded.extend(open_kind.map(str::to_string));
    }
    let more_h = if !merged && !folded_kinds.is_empty() { MORE_HEIGHT } else { 0.0 };
    used += more_h;

    // The open card takes what it needs next, the pins the rest up to their natural height.
    let mut open_h = 0.0;
    if let Some(k) = open_kind.filter(|k| others.contains(k)).filter(|_| !open_folded) {
        let natural = height_of(req, k).open;
        let spare = if unlimited { f64::INFINITY } else { (cap_v - used).max(0.0) };
        open_h = (strip(k) + spare).min(natural).max(strip(k));
        used += open_h - strip(k);
    }
    let pins_h = if !pins {
        0.0
    } else if merged {
        floor.line
    } else {
        let natural = if req.strips_only { floor_h } else { floor.open.max(floor_h) };
        let spare = if unlimited { f64::INFINITY } else { (cap_v - used).max(0.0) };
        (floor_h + spare).min(natural).max(floor_h)
    };

    let mut slots: Vec<StackSlot> = Vec::new();
    for k in &order {
        let is_open = Some(*k) == open_kind;
        let slot = if *k == "mention_pins" {
            let form = if merged {
                Form::CountLine
            } else if pins_h > floor_h {
                Form::Full
            } else {
                Form::Floor
            };
            StackSlot {
                kind: (*k).to_string(),
                open: false,
                form,
                height: pins_h.round(),
                more_cards: if merged { all_folded.len() } else { 0 },
            }
        } else if all_folded.iter().any(|f| f == k) {
            StackSlot { kind: (*k).to_string(), open: is_open, form: Form::Folded, height: 0.0, more_cards: 0 }
        } else if is_open {
            StackSlot { kind: (*k).to_string(), open: true, form: Form::Open, height: open_h.round(), more_cards: 0 }
        } else {
            StackSlot { kind: (*k).to_string(), open: false, form: Form::Strip, height: strip(k).round(), more_cards: 0 }
        };
        slots.push(slot);
    }
    let more = (!merged && !folded_kinds.is_empty())
        .then(|| MoreCards { count: folded_kinds.len(), kinds: folded_kinds.clone(), height: MORE_HEIGHT });
    let height = slots.iter().map(|s| s.height).sum::<f64>() + more.as_ref().map(|m| m.height).unwrap_or(0.0);
    StackLayout {
        slots,
        max_height: cap.map(f64::round).unwrap_or(0.0),
        more,
        height,
        room_key,
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

    /// SN-264: a prediction already closed and waiting for its result starts as a strip, and
    /// predictions can start collapsed like polls. A tap still opens either.
    #[test]
    fn predictions_can_start_collapsed_and_a_waiting_one_never_opens_itself() {
        let mut waiting = req(&["prediction", "poll"], 700.0);
        waiting.prediction_waiting = true;
        assert_eq!(open_of(&layout(&waiting)), vec!["poll"], "the poll beside it opens instead");
        waiting.opened = Some("prediction".into());
        assert_eq!(open_of(&layout(&waiting)), vec!["prediction"], "a tap opens it");

        let mut collapsed = req(&["prediction"], 700.0);
        collapsed.predictions_start_collapsed = true;
        assert!(open_of(&layout(&collapsed)).is_empty());
        assert_eq!(open_of(&layout(&req(&["prediction"], 700.0))), vec!["prediction"], "the default is unchanged");
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

    // ---- The seven cards (SN-284) -----------------------------------------

    fn form_of<'a>(l: &'a StackLayout, kind: &str) -> &'a StackSlot {
        l.slots.iter().find(|s| s.kind == kind).unwrap_or_else(|| panic!("no {kind} slot"))
    }

    #[test]
    fn the_seven_cards_stack_in_one_order() {
        let l = layout(&req(&["mention_pins", "mention_context", "channel_pin", "drop", "poll", "prediction", "hype"], 1000.0));
        assert_eq!(kinds(&l), vec!["hype", "prediction", "poll", "drop", "channel_pin", "mention_context", "mention_pins"]);
        let mut r = req(&["mention_pins", "drop", "poll", "prediction", "hype"], 1000.0);
        r.order = Some("poll-first".into());
        assert_eq!(kinds(&layout(&r)), vec!["hype", "poll", "prediction", "drop", "mention_pins"]);
    }

    #[test]
    fn the_stack_never_passes_its_share_and_the_pins_floor_is_never_folded() {
        for height in [200.0, 380.0, 460.0, 700.0, 1000.0] {
            for opened in [None, Some("poll"), Some("channel_pin")] {
                let mut r = req(&["hype", "prediction", "poll", "drop", "channel_pin", "mention_context", "mention_pins"], height);
                r.opened = opened.map(str::to_string);
                let l = layout(&r);
                let cap = (height * MAX_SHARE).max(MIN_HEIGHT);
                assert!(l.height <= cap, "{height}: {} over the {cap} share", l.height);
                let pins = form_of(&l, "mention_pins");
                assert!(matches!(pins.form, Form::Floor | Form::Full | Form::CountLine), "{height}: pins are never folded");
                assert!(pins.height >= 28.0);
            }
        }
    }

    #[test]
    fn strips_past_the_cap_fold_lowest_rank_first_into_one_more_strip() {
        // 380 px: cap 152. The floor (64) leaves 88: a "N more" strip (28) and one strip (36).
        let l = layout(&req(&["hype", "poll", "drop", "channel_pin", "mention_context", "mention_pins"], 380.0));
        assert_eq!(form_of(&l, "hype").form, Form::Strip);
        for k in ["poll", "drop", "channel_pin", "mention_context"] {
            assert_eq!(form_of(&l, k).form, Form::Folded, "{k}");
        }
        let more = l.more.as_ref().expect("a more strip");
        assert_eq!(
            (more.count, more.kinds.clone()),
            (4, vec!["poll".to_string(), "drop".into(), "channel_pin".into(), "mention_context".into()])
        );
        // The pins keep at least their floor and take what is left (24 px more here).
        let pins = form_of(&l, "mention_pins");
        assert_eq!((pins.form, pins.height), (Form::Full, 88.0));
        assert!(l.height <= l.max_height);
    }

    #[test]
    fn a_column_too_short_for_the_floor_and_the_more_strip_merges_them_into_the_count_line() {
        let l = layout(&req(&["hype", "poll", "drop", "mention_pins"], 150.0));
        assert_eq!(l.max_height, MIN_HEIGHT);
        let pins = form_of(&l, "mention_pins");
        assert_eq!(pins.form, Form::CountLine);
        assert_eq!(pins.height, 28.0);
        assert!(l.more.is_none(), "merged into the count line");
        assert_eq!(pins.more_cards, 3 - form_cards(&l, Form::Strip));
        assert!(l.height <= MIN_HEIGHT);
    }

    fn form_cards(l: &StackLayout, form: Form) -> usize {
        l.slots.iter().filter(|s| s.kind != "mention_pins" && s.form == form).count()
    }

    #[test]
    fn the_open_card_takes_what_it_needs_then_the_pins_take_the_rest() {
        // 1000 px: cap 400. Poll open (150) beside a drop strip (36); the pins fill what is left.
        let l = layout(&req(&["poll", "drop", "mention_pins"], 1000.0));
        assert_eq!(open_of(&l), vec!["poll"]);
        assert_eq!(form_of(&l, "poll").height, 150.0);
        assert_eq!(form_of(&l, "drop").height, 36.0);
        let pins = form_of(&l, "mention_pins");
        assert_eq!((pins.form, pins.height), (Form::Full, 200.0), "up to their natural height");
        // A shorter cap squeezes the pins first, never the open card below its strip.
        let l = layout(&req(&["poll", "drop", "mention_pins"], 500.0));
        assert_eq!(l.max_height, 200.0);
        assert_eq!(form_of(&l, "mention_pins").form, Form::Floor);
        assert!(form_of(&l, "poll").height >= 36.0);
        assert!(l.height <= 200.0);
    }

    #[test]
    fn which_card_is_open_never_depends_on_heights() {
        let mut r = req(&["poll", "drop", "channel_pin", "mention_pins"], 700.0);
        let base = open_card(&r);
        for strip in [0.0, 10.0, 500.0] {
            for open in [0.0, 30.0, 900.0] {
                r.heights.insert("poll".into(), CardHeights { strip, open, line: 0.0 });
                r.heights.insert("mention_pins".into(), CardHeights { strip: open, open: strip, line: 12.0 });
                assert_eq!(open_card(&r), base, "{strip}/{open}");
                assert_eq!(layout(&r).slots.iter().filter(|s| s.open).count(), 1);
            }
        }
    }

    #[test]
    fn the_channel_pin_opens_from_its_store_and_the_lower_third_opens_nothing() {
        let mut r = req(&["channel_pin", "mention_pins"], 700.0);
        assert!(open_of(&layout(&r)).is_empty(), "closed unless the viewer opened it");
        r.channel_pin_opened = true;
        assert_eq!(open_of(&layout(&r)), vec!["channel_pin"]);
        r.opened = Some("mention_context".into());
        r.cards.push("mention_context".into());
        assert_eq!(open_of(&layout(&r)), vec!["mention_context"], "one open card, the viewer's last");
        r.strips_only = true;
        let l = layout(&r);
        assert!(open_of(&l).is_empty());
        assert_eq!(form_of(&l, "mention_pins").form, Form::Floor, "no scrolling in the lower third");
    }

    #[test]
    fn the_answer_names_the_chats_room_key() {
        let mut r = req(&["poll"], 700.0);
        assert_eq!(layout(&r).room_key, "");
        r.provider = Some("kick".into());
        r.channel = Some("SomeSlug".into());
        assert_eq!(layout(&r).room_key, "kick:someslug");
        r.provider = Some("twitch".into());
        r.channel = Some("XQC".into());
        assert_eq!(layout(&r).room_key, "xqc");
    }

    #[test]
    fn measured_heights_replace_the_defaults() {
        let mut r = req(&["drop", "mention_pins"], 1000.0);
        r.heights.insert("drop".into(), CardHeights { strip: 50.0, open: 0.0, line: 0.0 });
        r.heights.insert("mention_pins".into(), CardHeights { strip: 80.0, open: 120.0, line: 0.0 });
        let l = layout(&r);
        assert_eq!(form_of(&l, "mention_pins").height, 120.0);
        assert_eq!(open_of(&l), vec!["drop"], "a drop alone shows open, at its natural height");
        assert_eq!(form_of(&l, "drop").height, 120.0);
    }

    // ---- The shared case file ---------------------------------------------

    fn authored() -> Vec<(&'static str, StackRequest)> {
        let all = ["hype", "prediction", "poll", "drop", "channel_pin", "mention_context", "mention_pins"];
        let with = |cards: &[&str], height: f64, f: &dyn Fn(&mut StackRequest)| {
            let mut r = req(cards, height);
            f(&mut r);
            r
        };
        vec![
            ("seven_cards_on_a_tall_column", with(&all, 1000.0, &|_| {})),
            ("seven_cards_on_a_380_px_column", with(&all, 380.0, &|_| {})),
            ("seven_cards_on_a_320_px_column", with(&all, 320.0, &|_| {})),
            ("seven_cards_with_the_poll_opened_on_a_700_px_column", with(&all, 700.0, &|r| r.opened = Some("poll".into()))),
            ("votes_poll_first_and_a_drop", with(&["drop", "poll", "prediction"], 700.0, &|r| r.order = Some("poll-first".into()))),
            ("the_pins_alone_on_a_tall_column", with(&["mention_pins"], 900.0, &|_| {})),
            ("the_pins_alone_on_a_short_column", with(&["mention_pins"], 140.0, &|_| {})),
            ("the_pins_floor_with_a_hype_strip_at_380", with(&["hype", "mention_pins"], 380.0, &|_| {})),
            ("the_lower_third_is_strips_only", with(&["hype", "drop", "mention_pins"], 600.0, &|r| r.strips_only = true)),
            ("the_channel_pin_open_from_its_store", with(&["channel_pin", "mention_pins"], 800.0, &|r| r.channel_pin_opened = true)),
            ("n_more_cards_folds_the_lowest_ranks", with(&["hype", "poll", "drop", "channel_pin", "mention_context", "mention_pins"], 380.0, &|_| {})),
            ("merged_count_line_on_a_very_short_column", with(&["hype", "poll", "drop", "mention_pins"], 150.0, &|_| {})),
            ("measured_heights_with_two_row_pins", with(&["poll", "mention_pins"], 800.0, &|r| {
                r.heights.insert("mention_pins".into(), CardHeights { strip: 96.0, open: 260.0, line: 30.0 });
            })),
            ("a_kick_chat_names_its_room_key", with(&["poll"], 700.0, &|r| {
                r.provider = Some("kick".into());
                r.channel = Some("SomeSlug".into());
            })),
            ("no_pins_keeps_the_old_stack", with(&["drop", "poll"], 700.0, &|_| {})),
            ("an_unknown_height", with(&["poll", "mention_pins"], 0.0, &|_| {})),
        ]
    }

    #[derive(Serialize, Deserialize)]
    struct Case {
        name: String,
        request: StackRequest,
        layout: StackLayout,
    }

    fn cases_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("chat-top-stack-cases.json")
    }

    /// The case file every shell reads: compared here, rewritten only with
    /// `UPDATE_GOLDEN=1` (review the diff: it is the rule every shell follows).
    #[test]
    fn the_shared_case_file_matches_the_rule() {
        let fresh: Vec<Case> = authored()
            .into_iter()
            .map(|(name, request)| Case { name: name.to_string(), layout: layout(&request), request })
            .collect();
        if std::env::var("UPDATE_GOLDEN").is_ok_and(|v| v == "1") {
            let text = serde_json::to_string_pretty(&fresh).unwrap() + "\n";
            std::fs::create_dir_all(cases_path().parent().unwrap()).unwrap();
            std::fs::write(cases_path(), text).unwrap();
        }
        let stored: Vec<Case> =
            serde_json::from_str(&std::fs::read_to_string(cases_path()).expect("run once with UPDATE_GOLDEN=1")).expect("case file parses");
        assert!(!stored.is_empty(), "the case file holds cases");
        assert_eq!(stored.len(), fresh.len(), "a case was added or removed: regenerate with UPDATE_GOLDEN=1 and review");
        for (s, f) in stored.iter().zip(&fresh) {
            assert_eq!(s.name, f.name);
            assert_eq!(s.request, f.request, "{}: the request changed; regenerate with UPDATE_GOLDEN=1", s.name);
            assert_eq!(s.layout, layout(&s.request), "{}: the rule moved; regenerate with UPDATE_GOLDEN=1 and review the diff", s.name);
        }
    }
}
