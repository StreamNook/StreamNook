//! Emote tables that came back partial, and the rows parsed with them.
//!
//! A 7TV channel lookup that fails leaves a table holding the globals only (or
//! nothing). Such a table is never stored as the channel's real set: it is
//! served only while nothing better is held, and it is fetched again in the
//! background at `DELAYS` while the channel is still open, one loop per channel,
//! until a whole answer lands. Kick, YouTube and Shared Chat partner tables all
//! use this.
//!
//! Kick and YouTube bake 7TV words into a row's segments when the row is
//! parsed, so rows parsed before the whole table landed would keep their 7TV
//! words as text for good. Those rows are held here (a bounded few per channel)
//! and, once a better table is installed, baked again and patched in place with
//! the same `HISTORY_SEGMENTS` frame (id, segments, parent_segments, view) the
//! Twitch backlog re-stamp sends, which every client already applies by id.

use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};

use crate::models::chat_layout::{ChatMessage, MessageSegment};

/// When a partial table is fetched again: 15 s, 45 s, 2 min and 5 min after it
/// came back partial.
pub(crate) const DELAYS: [Duration; 4] = [
    Duration::from_secs(15),
    Duration::from_secs(45),
    Duration::from_secs(120),
    Duration::from_secs(300),
];

/// The most rows held per channel while its table is not whole.
const MAX_HELD: usize = 300;

/// What a 7TV lookup answered. Only a 404 means "there is none"; every other
/// failure (another 4xx, a 5xx, a timeout, a dropped connection, an unreadable
/// body) is a failed fetch, never an empty set.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum Lookup {
    Found,
    None,
    Failed,
}

/// How a 7TV lookup's HTTP status reads.
pub(crate) fn classify(status: reqwest::StatusCode) -> Lookup {
    if status == reqwest::StatusCode::NOT_FOUND {
        Lookup::None
    } else if status.is_success() {
        Lookup::Found
    } else {
        Lookup::Failed
    }
}

/// GET a 7TV document. `Ok(None)` only for a 404; anything else that is not a
/// readable success is `Err`.
pub(crate) async fn get_json(client: &reqwest::Client, url: &str, timeout: Duration) -> Result<Option<Value>, ()> {
    let resp = client.get(url).timeout(timeout).send().await.map_err(|_| ())?;
    match classify(resp.status()) {
        Lookup::None => Ok(None),
        Lookup::Failed => Err(()),
        Lookup::Found => resp.json().await.map(Some).map_err(|_| ()),
    }
}

/// How far a held table can be trusted, worst first.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
pub(crate) enum Standing {
    /// A fetch failed somewhere (the globals, the channel lookup or its set).
    Partial,
    /// The saved copy of an earlier whole answer, seeded at first use.
    Saved,
    /// A whole answer fetched this run.
    Whole,
}

/// What to do with a freshly fetched table.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum Install {
    /// Put it in place; `save` writes it to disk too.
    Replace { save: bool },
    /// Keep what is held.
    Keep,
}

/// A whole answer always replaces what is held (a smaller one is what a
/// removal looks like) and is the only answer ever saved. A partial one is
/// used only while nothing better is held: nothing at all, or a smaller
/// partial one. A saved copy of an earlier whole answer beats it.
pub(crate) fn decide(held: Option<(Standing, usize)>, whole: bool, fetched_len: usize) -> Install {
    if whole {
        return Install::Replace { save: true };
    }
    match held {
        None => Install::Replace { save: false },
        Some((Standing::Partial, len)) if fetched_len > len => Install::Replace { save: false },
        Some(_) => Install::Keep,
    }
}

/// The keys a retry loop runs for, so a key never gets two.
pub(crate) struct Retrying(OnceLock<Mutex<HashSet<String>>>);

impl Retrying {
    pub(crate) const fn new() -> Self {
        Self(OnceLock::new())
    }

    fn keys(&self) -> &Mutex<HashSet<String>> {
        self.0.get_or_init(Default::default)
    }

    /// Takes `key`; false when a loop already runs for it.
    pub(crate) fn claim(&self, key: &str) -> bool {
        self.keys().lock().map(|mut k| k.insert(key.to_string())).unwrap_or(false)
    }

    pub(crate) fn release(&self, key: &str) {
        if let Ok(mut k) = self.keys().lock() {
            k.remove(key);
        }
    }

    pub(crate) fn running(&self, key: &str) -> bool {
        self.keys().lock().map(|k| k.contains(key)).unwrap_or(false)
    }
}

/// Gives the key back however the loop ends.
struct Claim(&'static Retrying, String);

impl Drop for Claim {
    fn drop(&mut self) {
        self.0.release(&self.1);
    }
}

/// Waits each delay and tries again while `in_use` holds, until `attempt`
/// reports a whole table. True when one landed.
pub(crate) async fn run<F, Fut>(delays: &[Duration], in_use: impl Fn() -> bool, mut attempt: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    for delay in delays {
        tokio::time::sleep(*delay).await;
        if !in_use() {
            return false;
        }
        if attempt().await {
            return true;
        }
    }
    false
}

/// Starts the retry loop for `key` unless one already runs (or there is no
/// runtime to run it on). True when this call started it.
pub(crate) fn spawn<F, Fut>(
    guard: &'static Retrying,
    key: String,
    in_use: impl Fn() -> bool + Send + 'static,
    attempt: F,
) -> bool
where
    F: FnMut() -> Fut + Send + 'static,
    Fut: Future<Output = bool> + Send + 'static,
{
    if tokio::runtime::Handle::try_current().is_err() || !guard.claim(&key) {
        return false;
    }
    // Taken before the task, so a task dropped unpolled still gives it back.
    let claim = Claim(guard, key);
    tokio::spawn(async move {
        let whole = run(&DELAYS, in_use, attempt).await;
        log::debug!("[EmoteRetry] {}: retries ended ({})", claim.1, if whole { "whole" } else { "still partial" });
    });
    true
}

/// Rows parsed while a channel's table was not whole, keyed by the table's key.
pub(crate) struct HeldRows(OnceLock<Mutex<HashMap<String, VecDeque<ChatMessage>>>>);

impl HeldRows {
    pub(crate) const fn new() -> Self {
        Self(OnceLock::new())
    }

    fn rows(&self) -> &Mutex<HashMap<String, VecDeque<ChatMessage>>> {
        self.0.get_or_init(Default::default)
    }

    /// Keeps a copy of a row as the viewer got it.
    pub(crate) fn hold(&self, key: &str, msg: &ChatMessage) {
        if let Ok(mut map) = self.rows().lock() {
            let rows = map.entry(key.to_string()).or_default();
            if rows.iter().any(|m| m.id == msg.id) {
                return;
            }
            if rows.len() >= MAX_HELD {
                rows.pop_front();
            }
            rows.push_back(msg.clone());
        }
    }

    pub(crate) fn drop_key(&self, key: &str) {
        if let Ok(mut map) = self.rows().lock() {
            map.remove(key);
        }
    }

    pub(crate) fn len(&self, key: &str) -> usize {
        self.rows().lock().map(|m| m.get(key).map_or(0, VecDeque::len)).unwrap_or(0)
    }

    /// Bakes the held rows again with `bake` and returns the patch rows (per
    /// chat channel) for those that changed. The held copies take the new
    /// segments; they are let go when `whole`, since nothing will change them
    /// again.
    pub(crate) fn repair(
        &self,
        key: &str,
        whole: bool,
        bake: impl Fn(Vec<MessageSegment>) -> Vec<MessageSegment>,
    ) -> HashMap<String, Vec<Value>> {
        let mut out: HashMap<String, Vec<Value>> = HashMap::new();
        let Ok(mut map) = self.rows().lock() else { return out };
        let Some(rows) = map.get_mut(key) else { return out };
        for msg in rows.iter_mut() {
            let segments = bake(msg.segments.clone());
            let parent = msg
                .metadata
                .reply_info
                .as_ref()
                .and_then(|r| r.parent_segments.clone())
                .map(&bake);
            let body_changed = !same(&segments, &msg.segments);
            let parent_changed = match (&parent, msg.metadata.reply_info.as_ref().and_then(|r| r.parent_segments.as_ref())) {
                (Some(new), Some(old)) => !same(new, old),
                _ => false,
            };
            if !body_changed && !parent_changed {
                continue;
            }
            msg.segments = segments;
            if let (Some(new), Some(info)) = (parent, msg.metadata.reply_info.as_mut()) {
                info.parent_segments = Some(new);
            }
            if msg.metadata.view.is_some() {
                crate::services::chat_row_view::stamp_view(msg);
            }
            let mut row = json!({
                "id": msg.id,
                "segments": msg.segments,
                "parent_segments": msg.metadata.reply_info.as_ref().and_then(|r| r.parent_segments.as_ref()),
            });
            if let Some(view) = &msg.metadata.view {
                row["view"] = json!(view);
            }
            out.entry(msg.channel.clone()).or_default().push(row);
        }
        if whole {
            map.remove(key);
        }
        out
    }
}

fn same(a: &[MessageSegment], b: &[MessageSegment]) -> bool {
    serde_json::to_value(a).ok() == serde_json::to_value(b).ok()
}

/// Sends the patch rows `repair` produced to each chat they belong to.
pub(crate) async fn send_patches(patches: HashMap<String, Vec<Value>>) {
    for (channel, rows) in patches {
        log::debug!("[EmoteRetry] {channel}: {} rows now wear the channel's whole emote set", rows.len());
        let frame = json!({ "type": "HISTORY_SEGMENTS", "channel": channel, "rows": rows });
        crate::services::irc_service::post_channel_frame(&channel, frame.to_string()).await;
    }
}

/// A 7TV emote as a table hands it to `bake`.
pub(crate) struct Baked {
    pub id: String,
    pub url: String,
    pub zero_width: bool,
}

/// Bakes 7TV words into `segments` with `lookup`: Text segments are scanned
/// word by word (whitespace kept exactly), and a 7TV emote segment the table
/// now names differently (a channel emote over the global of the same name)
/// takes the table's emote. Every other segment (the platform's own emotes and
/// emoji, links, personal emotes) is left alone, as is a 7TV emote the table
/// does not name.
pub(crate) fn bake(segments: Vec<MessageSegment>, lookup: impl Fn(&str) -> Option<Baked>) -> Vec<MessageSegment> {
    let mut out: Vec<MessageSegment> = Vec::with_capacity(segments.len());
    for seg in segments {
        if let MessageSegment::Text { content } = &seg {
            let mut buf = String::new();
            for (is_space, word) in whitespace_runs(content) {
                if is_space {
                    buf.push_str(word);
                    continue;
                }
                match lookup(word) {
                    Some(e) => {
                        if !buf.is_empty() {
                            out.push(MessageSegment::Text { content: std::mem::take(&mut buf) });
                        }
                        out.push(emote_segment(word, e));
                    }
                    None => buf.push_str(word),
                }
            }
            if !buf.is_empty() {
                out.push(MessageSegment::Text { content: buf });
            }
            continue;
        }
        let swapped = match &seg {
            MessageSegment::Emote { content, emote_id, emote_url, is_personal, .. }
                if *is_personal != Some(true) && is_seventv_url(emote_url) =>
            {
                lookup(content)
                    .filter(|e| emote_id.as_deref() != Some(e.id.as_str()))
                    .map(|e| emote_segment(content, e))
            }
            _ => None,
        };
        out.push(swapped.unwrap_or(seg));
    }
    // Neighbouring Text segments (a word that stayed text beside one that did
    // too) read the same as one.
    let mut merged: Vec<MessageSegment> = Vec::with_capacity(out.len());
    for seg in out {
        if let (Some(MessageSegment::Text { content: prev }), MessageSegment::Text { content }) = (merged.last_mut(), &seg) {
            prev.push_str(content);
            continue;
        }
        merged.push(seg);
    }
    merged
}

fn emote_segment(word: &str, e: Baked) -> MessageSegment {
    MessageSegment::Emote {
        content: word.to_string(),
        emote_id: Some(e.id),
        emote_url: e.url,
        is_zero_width: Some(e.zero_width),
        modifier_flags: None,
        is_personal: None,
    }
}

fn is_seventv_url(url: &str) -> bool {
    url.contains("cdn.7tv.app/emote/")
}

/// `text` cut into alternating runs of whitespace and non-whitespace, in order,
/// each tagged with whether it is whitespace. Joined back they are `text`.
fn whitespace_runs(text: &str) -> Vec<(bool, &str)> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut current: Option<bool> = None;
    for (i, c) in text.char_indices() {
        let space = c.is_whitespace();
        match current {
            Some(kind) if kind == space => {}
            Some(kind) => {
                out.push((kind, &text[start..i]));
                start = i;
                current = Some(space);
            }
            None => current = Some(space),
        }
    }
    if let Some(kind) = current {
        out.push((kind, &text[start..]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn text(s: &str) -> MessageSegment {
        MessageSegment::Text { content: s.to_string() }
    }

    fn seventv(word: &str, id: &str) -> MessageSegment {
        MessageSegment::Emote {
            content: word.to_string(),
            emote_id: Some(id.to_string()),
            emote_url: format!("https://cdn.7tv.app/emote/{id}/2x.webp"),
            is_zero_width: Some(false),
            modifier_flags: None,
            is_personal: None,
        }
    }

    fn table(word: &str) -> Option<Baked> {
        match word {
            "OMEGALUL" => Some(Baked { id: "chan1".into(), url: "https://cdn.7tv.app/emote/chan1/2x.webp".into(), zero_width: false }),
            "EZ" => Some(Baked { id: "glob1".into(), url: "https://cdn.7tv.app/emote/glob1/2x.webp".into(), zero_width: false }),
            _ => None,
        }
    }

    fn json_of(segments: &[MessageSegment]) -> Value {
        serde_json::to_value(segments).unwrap()
    }

    #[test]
    fn only_a_404_means_the_channel_has_none() {
        use reqwest::StatusCode;
        assert_eq!(classify(StatusCode::OK), Lookup::Found);
        assert_eq!(classify(StatusCode::NOT_FOUND), Lookup::None);
        for failed in [
            StatusCode::BAD_REQUEST,
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
            StatusCode::GONE,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::BAD_GATEWAY,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::GATEWAY_TIMEOUT,
        ] {
            assert_eq!(classify(failed), Lookup::Failed, "{failed} is a failed fetch, not an empty set");
        }
    }

    #[test]
    fn a_partial_answer_is_used_only_when_nothing_better_is_held_and_never_saved() {
        // Nothing held: the partial answer beats nothing, but is not saved.
        assert_eq!(decide(None, false, 45), Install::Replace { save: false });
        // A saved copy of an earlier whole answer, or a whole one, beats it.
        assert_eq!(decide(Some((Standing::Saved, 999)), false, 45), Install::Keep);
        assert_eq!(decide(Some((Standing::Whole, 999)), false, 45), Install::Keep);
        assert_eq!(decide(Some((Standing::Saved, 10)), false, 45), Install::Keep, "even a smaller saved copy");
        // Partial over partial: only a fuller one.
        assert_eq!(decide(Some((Standing::Partial, 45)), false, 45), Install::Keep);
        assert_eq!(decide(Some((Standing::Partial, 0)), false, 45), Install::Replace { save: false });
        // A whole answer always lands and is the only one saved, smaller or not.
        assert_eq!(decide(Some((Standing::Whole, 999)), true, 998), Install::Replace { save: true });
        assert_eq!(decide(None, true, 45), Install::Replace { save: true });
    }

    #[test]
    fn the_retry_schedule_is_15_45_120_300() {
        let secs: Vec<u64> = DELAYS.iter().map(Duration::as_secs).collect();
        assert_eq!(secs, vec![15, 45, 120, 300]);
    }

    #[test]
    fn one_retry_loop_per_key() {
        static GUARD: Retrying = Retrying::new();
        assert!(GUARD.claim("chan"));
        assert!(!GUARD.claim("chan"), "a second loop for the same key is refused");
        assert!(GUARD.claim("other"));
        assert!(GUARD.running("chan"));
        GUARD.release("chan");
        assert!(!GUARD.running("chan"));
        assert!(GUARD.claim("chan"), "free again once the loop ended");
    }

    #[tokio::test]
    async fn retries_stop_on_a_whole_answer() {
        let tries = Arc::new(AtomicUsize::new(0));
        let t = tries.clone();
        let delays = [Duration::ZERO; 4];
        let whole = run(&delays, || true, move || {
            let t = t.clone();
            async move { t.fetch_add(1, Ordering::SeqCst) + 1 == 2 }
        })
        .await;
        assert!(whole);
        assert_eq!(tries.load(Ordering::SeqCst), 2, "no try after the whole answer");
    }

    #[tokio::test]
    async fn retries_stop_when_the_channel_is_left_and_give_up_after_the_last_delay() {
        let tries = Arc::new(AtomicUsize::new(0));
        let t = tries.clone();
        let delays = [Duration::ZERO; 4];
        let whole = run(&delays, || false, move || {
            let t = t.clone();
            async move {
                t.fetch_add(1, Ordering::SeqCst);
                true
            }
        })
        .await;
        assert!(!whole);
        assert_eq!(tries.load(Ordering::SeqCst), 0, "a channel no longer open is not fetched");

        let t = tries.clone();
        let whole = run(&delays, || true, move || {
            let t = t.clone();
            async move {
                t.fetch_add(1, Ordering::SeqCst);
                false
            }
        })
        .await;
        assert!(!whole);
        assert_eq!(tries.load(Ordering::SeqCst), 4, "one try per delay, then it stops");
    }

    #[tokio::test]
    async fn a_spawned_loop_holds_its_key_until_it_ends() {
        static GUARD: Retrying = Retrying::new();
        assert!(spawn(&GUARD, "k".into(), || false, || async { true }));
        assert!(!spawn(&GUARD, "k".into(), || false, || async { true }), "one loop per key");
        // The first delay is 15 s; the loop is still waiting, so the key stays held.
        assert!(GUARD.running("k"));
    }

    #[test]
    fn bake_turns_words_into_emotes_and_keeps_whitespace() {
        let out = bake(vec![text("hi  OMEGALUL\tthere EZ")], table);
        assert_eq!(
            json_of(&out),
            json_of(&[text("hi  "), seventv("OMEGALUL", "chan1"), text("\tthere "), seventv("EZ", "glob1")])
        );
    }

    #[test]
    fn bake_swaps_a_global_for_the_channel_emote_and_leaves_other_emotes() {
        let kick_native = MessageSegment::Emote {
            content: "KEKW".into(),
            emote_id: Some("37225".into()),
            emote_url: "https://files.kick.com/emotes/37225/fullsize".into(),
            is_zero_width: None,
            modifier_flags: None,
            is_personal: None,
        };
        let personal = MessageSegment::Emote {
            content: "OMEGALUL".into(),
            emote_id: Some("mine".into()),
            emote_url: "https://cdn.7tv.app/emote/mine/2x.webp".into(),
            is_zero_width: Some(false),
            modifier_flags: None,
            is_personal: Some(true),
        };
        let input = vec![seventv("OMEGALUL", "oldglobal"), text(" "), kick_native.clone(), text(" "), personal.clone(), text(" "), seventv("gone", "x")];
        let out = bake(input, table);
        assert_eq!(
            json_of(&out),
            json_of(&[seventv("OMEGALUL", "chan1"), text(" "), kick_native, text(" "), personal, text(" "), seventv("gone", "x")])
        );
    }

    fn row(id: &str, channel: &str, segments: Vec<MessageSegment>) -> ChatMessage {
        let mut msg: ChatMessage = serde_json::from_value(json!({
            "id": id,
            "user_id": "1",
            "username": "a",
            "display_name": "A",
            "color": null,
            "badges": [],
            "timestamp": "",
            "content": "",
            "tags": {},
            "provider": "kick",
            "channel": channel,
        }))
        .expect("a row");
        msg.segments = segments;
        msg
    }

    #[test]
    fn held_rows_are_patched_only_when_they_change_and_let_go_once_whole() {
        let held = HeldRows::new();
        held.hold("slug", &row("r1", "kick:slug", vec![text("look OMEGALUL")]));
        held.hold("slug", &row("r2", "kick:slug", vec![text("no emotes here")]));
        held.hold("slug", &row("r1", "kick:slug", vec![text("look OMEGALUL")]));
        assert_eq!(held.len("slug"), 2, "a row is held once");

        // A partial table that still lacks the channel emote changes nothing.
        let patches = held.repair("slug", false, |s| bake(s, |_| None));
        assert!(patches.is_empty());
        assert_eq!(held.len("slug"), 2, "still held while the table is partial");

        let patches = held.repair("slug", true, |s| bake(s, table));
        let rows = patches.get("kick:slug").expect("patched in its own chat");
        assert_eq!(rows.len(), 1, "only the row whose emotes changed");
        assert_eq!(rows[0]["id"], "r1");
        assert_eq!(rows[0]["segments"], json_of(&[text("look "), seventv("OMEGALUL", "chan1")]));
        assert_eq!(held.len("slug"), 0, "let go once the table is whole");
    }

    #[test]
    fn held_rows_stay_bounded() {
        let held = HeldRows::new();
        for i in 0..(MAX_HELD + 25) {
            held.hold("busy", &row(&format!("r{i}"), "kick:busy", vec![text("x")]));
        }
        assert_eq!(held.len("busy"), MAX_HELD);
        held.drop_key("busy");
        assert_eq!(held.len("busy"), 0);
    }
}
