//! What each window's chat socket receives.
//!
//! Every chat frame reaches the local bridge once and is offered to every
//! window's socket. Each socket keeps its own route, set by the window over
//! the same socket in a few short text lines:
//!
//! - `SUBS:<key>\t<key>...` the channels this window holds (replaces), and
//!   `SUB:<key>` / `UNSUB:<key>` as that changes. Until a window has sent
//!   `SUBS` it receives everything, as before, so nothing is lost while it
//!   connects.
//! - `PARK:<cap>:<key>` holds a channel back: the window keeps it but shows it
//!   nowhere. Its frames wait here, in order, up to `cap` (the window's own
//!   buffer size), and when any of them mention the viewer the window gets a
//!   count each second instead (`PARKED_TICK:<mentions>:<rows>:<key>`, sent
//!   whenever either moved) so tab badges and "new messages" marks keep up.
//!   `cap` counts chat rows that do not mention the viewer. Mention rows are
//!   never pushed out by plain rows, and deletions and patches wait in order
//!   without taking a row's place; each of those two is bounded on its own.
//! - `UNPARK:<key>` sends what waited, in order, then `PARK_END:<key>`, then
//!   live frames again. If more arrived than were kept, `PARK_GAP:<key>` goes
//!   first so the window keeps the rows it already shows and fills the span
//!   between them and the backlog from history instead of showing a hole.
//! - A state frame (the room's modes, the account's own badges and color
//!   there) is never held back: it says how the channel stands, not what
//!   happened in it, and Twitch sends it only once per JOIN, so a busy
//!   channel's backlog pushed it out of the ring and the chat never learned
//!   the account moderates it.
//!
//! The rule everywhere is to deliver when unsure: a frame with no channel (a
//! status line, a frame whose channel is not known) always goes through.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

/// One frame on the bridge: the text the page parses, the channel it belongs
/// to when known, whether it is a chat row (counted while held back) that
/// raised a ping for the viewer (an @ mention, a reply or a highlight phrase),
/// and whether it is channel state (never held back).
#[derive(Clone, Debug)]
pub struct BridgeFrame {
    pub channel: Option<Arc<str>>,
    pub text: Arc<str>,
    pub row: bool,
    pub mention: bool,
    pub state: bool,
}

impl BridgeFrame {
    /// A frame for every window (status lines, frames of unknown channel).
    pub fn all(text: impl Into<Arc<str>>) -> Self {
        Self { channel: None, text: text.into(), row: false, mention: false, state: false }
    }

    /// A non-row frame for one channel (room state, a deletion, a timeout).
    pub fn channel(channel: &str, text: impl Into<Arc<str>>) -> Self {
        Self { channel: Some(channel_key(channel)), text: text.into(), row: false, mention: false, state: false }
    }

    /// A chat row for one channel.
    pub fn row(channel: &str, text: impl Into<Arc<str>>, mention: bool) -> Self {
        Self { channel: Some(channel_key(channel)), text: text.into(), row: true, mention, state: false }
    }

    /// The channel when known, else a frame for every window.
    pub fn maybe_channel(channel: Option<&str>, text: impl Into<Arc<str>>) -> Self {
        match channel {
            Some(c) if !c.trim().is_empty() => Self::channel(c, text),
            _ => Self::all(text),
        }
    }

    /// The same frame, marked as channel state: delivered at once even while
    /// the channel is held back. Only for frames that replace what the page
    /// knew (ROOMSTATE, USER_BADGES, USER_COLOR), never for rows or deletions,
    /// which must stay in order with the rows around them.
    pub fn as_state(mut self) -> Self {
        self.state = true;
        self
    }
}

impl From<String> for BridgeFrame {
    fn from(text: String) -> Self {
        Self::all(text)
    }
}

/// The key a channel's frames are routed by: the page's slice key. A Twitch
/// channel is its bare lowercase login (`#xqc` and `xqc` are `xqc`); a
/// provider channel arrives already composite (`kick:slug`) and is lowercased
/// the way the page stores it.
pub fn channel_key(raw: &str) -> Arc<str> {
    Arc::from(raw.trim().trim_start_matches('#').to_lowercase())
}

/// What a held frame is, for deciding what may be dropped when the ring is full.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// A chat row that does not mention the viewer.
    Row,
    /// A chat row that mentions the viewer.
    Mention,
    /// Anything else held in order (a deletion, a timeout, a segment patch).
    Other,
}

#[derive(Debug)]
struct Entry {
    kind: Kind,
    text: Arc<str>,
}

/// A held-back channel's frames in arrival order. `cap` bounds the chat rows
/// that are not mentions; mention rows and the other frames are bounded
/// separately by `MAX_HELD` each, so a busy channel can push plain rows out but
/// never a mention or the deletion that applies to a row still held.
#[derive(Debug, Default)]
struct Held {
    cap: usize,
    ring: VecDeque<Entry>,
    dropped: bool,
    mentions: u32,
    rows: u32,
    /// Rows of either kind in `ring`.
    row_count: usize,
    mention_count: usize,
    other_count: usize,
}

impl Held {
    fn push(&mut self, kind: Kind, text: Arc<str>) {
        match kind {
            Kind::Row => self.row_count += 1,
            Kind::Mention => {
                self.row_count += 1;
                self.mention_count += 1;
            }
            Kind::Other => self.other_count += 1,
        }
        self.ring.push_back(Entry { kind, text });
        while self.row_count > self.cap {
            if !self.drop_oldest(Kind::Row) {
                break;
            }
        }
        if self.mention_count > MAX_HELD {
            self.drop_oldest(Kind::Mention);
        }
        if self.other_count > MAX_HELD {
            self.drop_oldest(Kind::Other);
        }
    }

    /// Remove the oldest held frame of this kind. Returns whether there was one.
    fn drop_oldest(&mut self, kind: Kind) -> bool {
        let Some(i) = self.ring.iter().position(|e| e.kind == kind) else {
            return false;
        };
        self.ring.remove(i);
        match kind {
            Kind::Row => self.row_count -= 1,
            Kind::Mention => {
                self.row_count -= 1;
                self.mention_count -= 1;
            }
            Kind::Other => self.other_count -= 1,
        }
        self.dropped = true;
        true
    }
}

/// One socket's route. Pure: the socket task feeds it frames and the window's
/// lines, and sends what it returns.
#[derive(Debug, Default)]
pub struct SocketRoute {
    /// The window has said what it holds; before that, everything goes.
    filtered: bool,
    subs: HashSet<String>,
    held: HashMap<String, Held>,
}

/// Upper bound on a held-back ring, whatever a window asks for.
const MAX_HELD: usize = 2000;

impl SocketRoute {
    /// A line from the window. Returns frames to send now (a backlog).
    pub fn on_line(&mut self, line: &str) -> Vec<Arc<str>> {
        if let Some(list) = line.strip_prefix("SUBS:") {
            self.filtered = true;
            self.subs = list.split('\t').filter(|k| !k.is_empty()).map(|k| channel_key(k).to_string()).collect();
            self.held.retain(|k, _| self.subs.contains(k));
        } else if let Some(k) = line.strip_prefix("SUB:") {
            self.subs.insert(channel_key(k).to_string());
        } else if let Some(k) = line.strip_prefix("UNSUB:") {
            let k = channel_key(k).to_string();
            self.subs.remove(&k);
            self.held.remove(&k);
        } else if let Some(rest) = line.strip_prefix("PARK:") {
            if let Some((cap, k)) = rest.split_once(':') {
                let cap = cap.parse::<usize>().unwrap_or(500).clamp(1, MAX_HELD);
                let k = channel_key(k).to_string();
                self.held.entry(k).or_insert_with(|| Held { cap, ..Default::default() }).cap = cap;
            }
        } else if let Some(k) = line.strip_prefix("UNPARK:") {
            let k = channel_key(k).to_string();
            if let Some(h) = self.held.remove(&k) {
                let mut out = Vec::with_capacity(h.ring.len() + 2);
                if h.dropped {
                    out.push(Arc::from(format!("PARK_GAP:{k}")));
                }
                out.extend(h.ring.into_iter().map(|e| e.text));
                out.push(Arc::from(format!("PARK_END:{k}")));
                return out;
            }
        }
        Vec::new()
    }

    /// A frame off the bridge. Returns it when the window should get it now.
    pub fn on_frame(&mut self, f: &BridgeFrame) -> Option<Arc<str>> {
        let Some(ch) = f.channel.as_deref() else {
            return Some(f.text.clone());
        };
        if let Some(h) = self.held.get_mut(ch) {
            if f.state {
                return Some(f.text.clone());
            }
            let kind = match (f.row, f.mention) {
                (true, true) => Kind::Mention,
                (true, false) => Kind::Row,
                (false, _) => Kind::Other,
            };
            h.push(kind, f.text.clone());
            if f.row {
                h.rows = h.rows.saturating_add(1);
                if f.mention {
                    h.mentions = h.mentions.saturating_add(1);
                }
            }
            return None;
        }
        if !self.filtered || self.subs.contains(ch) {
            return Some(f.text.clone());
        }
        None
    }

    /// The rows and mentions in held-back channels since the last call, as lines.
    pub fn take_ticks(&mut self) -> Vec<Arc<str>> {
        let mut out = Vec::new();
        for (k, h) in self.held.iter_mut() {
            if h.mentions > 0 || h.rows > 0 {
                out.push(Arc::from(format!("PARKED_TICK:{}:{}:{}", h.mentions, h.rows, k)));
                h.mentions = 0;
                h.rows = 0;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(v: Vec<Arc<str>>) -> Vec<String> {
        v.into_iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn keys_match_the_pages_slice_keys() {
        assert_eq!(&*channel_key("#XQC"), "xqc");
        assert_eq!(&*channel_key("Kick:SomeSlug"), "kick:someslug");
    }

    #[test]
    fn a_window_gets_everything_until_it_says_what_it_holds() {
        let mut r = SocketRoute::default();
        assert!(r.on_frame(&BridgeFrame::row("xqc", "a", false)).is_some());
        r.on_line("SUBS:shroud");
        assert!(r.on_frame(&BridgeFrame::row("xqc", "b", false)).is_none(), "another window's channel");
        assert!(r.on_frame(&BridgeFrame::row("shroud", "c", false)).is_some());
        r.on_line("SUB:xqc");
        assert!(r.on_frame(&BridgeFrame::row("#XQC", "d", false)).is_some());
        r.on_line("UNSUB:xqc");
        assert!(r.on_frame(&BridgeFrame::row("xqc", "e", false)).is_none());
    }

    #[test]
    fn frames_without_a_channel_always_go_through() {
        let mut r = SocketRoute::default();
        r.on_line("SUBS:");
        assert_eq!(r.on_frame(&BridgeFrame::all("HEARTBEAT")).as_deref(), Some("HEARTBEAT"));
        assert!(r.on_frame(&BridgeFrame::maybe_channel(None, "x")).is_some());
    }

    #[test]
    fn a_held_back_chat_waits_in_order_and_comes_back_whole() {
        let mut r = SocketRoute::default();
        r.on_line("SUBS:xqc\tshroud");
        r.on_line("PARK:10:xqc");
        assert!(r.on_frame(&BridgeFrame::row("xqc", "1", false)).is_none());
        assert!(r.on_frame(&BridgeFrame::channel("xqc", "del")).is_none());
        assert!(r.on_frame(&BridgeFrame::row("xqc", "2", true)).is_none());
        assert!(r.on_frame(&BridgeFrame::row("shroud", "s", false)).is_some(), "other chats flow");
        assert_eq!(texts(r.take_ticks()), vec!["PARKED_TICK:1:2:xqc"]);
        assert!(r.take_ticks().is_empty(), "counts reset");
        assert_eq!(texts(r.on_line("UNPARK:xqc")), vec!["1", "del", "2", "PARK_END:xqc"]);
        assert!(r.on_frame(&BridgeFrame::row("xqc", "3", false)).is_some(), "live again");
    }

    #[test]
    fn an_overflowing_backlog_says_so_first() {
        let mut r = SocketRoute::default();
        r.on_line("SUBS:xqc");
        r.on_line("PARK:2:xqc");
        for t in ["1", "2", "3"] {
            r.on_frame(&BridgeFrame::row("xqc", t, false));
        }
        assert_eq!(texts(r.on_line("UNPARK:xqc")), vec!["PARK_GAP:xqc", "2", "3", "PARK_END:xqc"]);
    }

    #[test]
    fn dropping_a_channel_drops_its_backlog() {
        let mut r = SocketRoute::default();
        r.on_line("SUBS:xqc");
        r.on_line("PARK:5:xqc");
        r.on_frame(&BridgeFrame::row("xqc", "1", false));
        r.on_line("UNSUB:xqc");
        assert!(r.on_line("UNPARK:xqc").is_empty());
    }

    #[test]
    fn a_held_back_chat_still_learns_how_it_stands() {
        let mut r = SocketRoute::default();
        r.on_line("SUBS:xqc");
        r.on_line("PARK:10:xqc");
        assert!(r.on_frame(&BridgeFrame::row("xqc", "1", false)).is_none());
        let badges = BridgeFrame::channel("xqc", "USER_BADGES:#xqc:moderator/1").as_state();
        assert_eq!(r.on_frame(&badges).as_deref(), Some("USER_BADGES:#xqc:moderator/1"));
        assert_eq!(texts(r.take_ticks()), vec!["PARKED_TICK:0:1:xqc"], "state is not a new message");
        assert!(r.on_frame(&BridgeFrame::channel("xqc", "del")).is_none(), "deletions stay in order");
        assert_eq!(texts(r.on_line("UNPARK:xqc")), vec!["1", "del", "PARK_END:xqc"]);
    }

    #[test]
    fn a_full_backlog_keeps_its_mentions_and_deletions() {
        let mut r = SocketRoute::default();
        r.on_line("SUBS:xqc");
        r.on_line("PARK:3:xqc");
        let mut sent = 0;
        for t in ["r1", "r2", "m1", "r3", "r4", "r5", "m2", "r6", "r7", "r8", "r9", "r10"] {
            let mention = t.starts_with('m');
            r.on_frame(&BridgeFrame::row("xqc", t, mention));
            sent += 1;
            if sent % 3 == 0 {
                r.on_frame(&BridgeFrame::channel("xqc", format!("del{}", sent / 3)));
            }
        }
        assert_eq!(texts(r.take_ticks()), vec!["PARKED_TICK:2:12:xqc"], "every arrival is counted");
        assert_eq!(
            texts(r.on_line("UNPARK:xqc")),
            vec!["PARK_GAP:xqc", "m1", "del1", "del2", "m2", "del3", "r10", "del4", "PARK_END:xqc"],
            "both mentions, the newest plain row and all four deletions, in arrival order",
        );
    }

    #[test]
    fn deletions_take_no_row_slot() {
        let mut r = SocketRoute::default();
        r.on_line("SUBS:xqc");
        r.on_line("PARK:2:xqc");
        r.on_frame(&BridgeFrame::row("xqc", "1", false));
        for i in 0..5 {
            r.on_frame(&BridgeFrame::channel("xqc", format!("del{i}")));
        }
        r.on_frame(&BridgeFrame::row("xqc", "2", false));
        assert_eq!(
            texts(r.on_line("UNPARK:xqc")),
            vec!["1", "del0", "del1", "del2", "del3", "del4", "2", "PARK_END:xqc"],
            "nothing was dropped, so no gap",
        );
    }

    #[test]
    fn a_mention_survives_a_full_ring_of_mentions() {
        let mut r = SocketRoute::default();
        r.on_line("SUBS:xqc");
        r.on_line("PARK:2:xqc");
        for t in ["m1", "m2", "m3"] {
            r.on_frame(&BridgeFrame::row("xqc", t, true));
        }
        r.on_frame(&BridgeFrame::row("xqc", "plain", false));
        assert_eq!(texts(r.on_line("UNPARK:xqc")), vec!["PARK_GAP:xqc", "m1", "m2", "m3", "PARK_END:xqc"]);
    }

    #[test]
    fn rows_that_pinged_are_kept_and_a_phrase_hit_that_did_not_is_plain() {
        let mut r = SocketRoute::default();
        r.on_line("SUBS:xqc");
        r.on_line("PARK:3:xqc");
        // A phrase hit with its ping on arrives flagged; with pings off it does not.
        r.on_frame(&BridgeFrame::row("xqc", "phrase1", true));
        r.on_frame(&BridgeFrame::row("xqc", "silent", false));
        r.on_frame(&BridgeFrame::row("xqc", "phrase2", true));
        for i in 0..10 {
            r.on_frame(&BridgeFrame::row("xqc", format!("p{i}"), false));
        }
        assert_eq!(texts(r.take_ticks()), vec!["PARKED_TICK:2:13:xqc"]);
        assert_eq!(
            texts(r.on_line("UNPARK:xqc")),
            vec!["PARK_GAP:xqc", "phrase1", "phrase2", "p9", "PARK_END:xqc"],
            "both rows that pinged stay, the silent phrase hit is pushed out like any plain row",
        );
    }

    #[test]
    fn the_other_frames_and_the_mentions_are_each_bounded() {
        let mut r = SocketRoute::default();
        r.on_line("SUBS:xqc");
        r.on_line("PARK:5:xqc");
        for i in 0..MAX_HELD + 1 {
            r.on_frame(&BridgeFrame::channel("xqc", format!("d{i}")));
        }
        let out = texts(r.on_line("UNPARK:xqc"));
        assert_eq!(out.len(), MAX_HELD + 2, "gap, the newest bound's worth, end");
        assert_eq!(out[0], "PARK_GAP:xqc");
        assert_eq!(out[1], "d1", "the oldest went first");

        r.on_line("PARK:5:xqc");
        for i in 0..MAX_HELD + 1 {
            r.on_frame(&BridgeFrame::row("xqc", format!("m{i}"), true));
        }
        let out = texts(r.on_line("UNPARK:xqc"));
        assert_eq!(out.len(), MAX_HELD + 2);
        assert_eq!(out[0], "PARK_GAP:xqc");
        assert_eq!(out[1], "m1");
    }

    #[test]
    fn state_never_falls_off_a_full_backlog() {
        let mut r = SocketRoute::default();
        r.on_line("SUBS:xqc");
        r.on_line("PARK:2:xqc");
        let badges = BridgeFrame::channel("xqc", "USER_BADGES:#xqc:moderator/1").as_state();
        assert!(r.on_frame(&badges).is_some());
        for t in ["1", "2", "3"] {
            r.on_frame(&BridgeFrame::row("xqc", t, false));
        }
        assert_eq!(texts(r.on_line("UNPARK:xqc")), vec!["PARK_GAP:xqc", "2", "3", "PARK_END:xqc"]);
    }
}
