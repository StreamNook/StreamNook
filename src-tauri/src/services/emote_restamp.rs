//! Rust decides which words are emotes once, when a line is parsed, and a row
//! parsed as text stays text. When a channel's emote table was not in place
//! yet (a first visit, a provider that answered late, a phone back from the
//! background) or changes later (an emote added mid-stream), the rows already
//! drawn would keep showing those emotes as plain words.
//!
//! Every chat line is remembered here for a few minutes. When a channel's
//! table is put in place or changes, the lines with a word that now names an
//! emote (or a different one) are parsed again and patched in place in every
//! window by the `HISTORY_SEGMENTS` frame every client already applies by row
//! id. A removal never un-draws an emote that is already showing.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::services::irc_service::{irc_trailing_text, EmoteLookup, IrcService};

/// Lines kept per channel. A busy room passes this in seconds, which is fine:
/// a table that lands seconds late repairs what is still on screen.
const KEEP: usize = 300;
/// How long a line stays repairable.
const KEEP_FOR: Duration = Duration::from_secs(180);

struct Line {
    at: Instant,
    raw: String,
}

fn rooms() -> &'static Mutex<HashMap<String, VecDeque<Line>>> {
    static ROOMS: OnceLock<Mutex<HashMap<String, VecDeque<Line>>>> = OnceLock::new();
    ROOMS.get_or_init(Default::default)
}

/// Remember a chat line for `channel` (lowercase, no `#`, the parse-table key).
pub(crate) fn remember(channel: &str, raw: &str) {
    if channel.is_empty() {
        return;
    }
    let Ok(mut map) = rooms().lock() else { return };
    let now = Instant::now();
    // Rooms left behind (a channel browsed past) drop out once their newest
    // line is too old to repair, so the map stays the size of what is open.
    if map.len() > 32 {
        map.retain(|_, q| q.back().is_some_and(|l| now.duration_since(l.at) <= KEEP_FOR));
    }
    push_line(map.entry(channel.to_string()).or_default(), raw, now);
}

fn push_line(q: &mut VecDeque<Line>, raw: &str, now: Instant) {
    while q.front().is_some_and(|l| now.duration_since(l.at) > KEEP_FOR) || q.len() >= KEEP {
        q.pop_front();
    }
    q.push_back(Line { at: now, raw: raw.to_string() });
}

/// True when some word of `text` names an emote in `new` that `old` did not
/// name, or named as a different emote. A removal never counts.
pub(crate) fn gains_emote(text: &str, new: &EmoteLookup, old: Option<&EmoteLookup>) -> bool {
    text.split_whitespace().any(|word| match new.get(word) {
        Some(now) => old.and_then(|o| o.get(word)).map_or(true, |before| before.id != now.id),
        None => false,
    })
}

/// The value of one IRC tag on a raw line, if present.
fn tag<'a>(raw: &'a str, name: &str) -> Option<&'a str> {
    let tags = raw.strip_prefix('@')?.split(' ').next()?;
    tags.split(';').find_map(|t| {
        let (k, v) = t.split_once('=')?;
        (k == name).then_some(v)
    })
}

/// A Shared Chat partner's row parses with its own room's table and has its
/// own repair path (`restamp_partner_rows`); the home table never changes it.
fn is_partner_line(raw: &str) -> bool {
    match (tag(raw, "source-room-id"), tag(raw, "room-id")) {
        (Some(source), Some(room)) => !source.is_empty() && source != room,
        _ => false,
    }
}

/// The lines of `q` the change from `old` to `new` repairs, each once, and
/// how long ago the oldest of them arrived.
fn lines_to_repair(
    q: &VecDeque<Line>,
    new: &EmoteLookup,
    old: Option<&EmoteLookup>,
    now: Instant,
) -> (Vec<String>, Duration) {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut oldest = Duration::ZERO;
    let mut out = Vec::new();
    for l in q {
        let age = now.duration_since(l.at);
        if age > KEEP_FOR || is_partner_line(&l.raw) || !seen.insert(l.raw.as_str()) {
            continue;
        }
        if gains_emote(irc_trailing_text(&l.raw), new, old) {
            oldest = oldest.max(age);
            out.push(l.raw.clone());
        }
    }
    (out, oldest)
}

/// `channel`'s parse table went from `old` to `new`.
pub(crate) fn table_changed(channel: &str, old: Option<Arc<EmoteLookup>>, new: Arc<EmoteLookup>) {
    let (lines, oldest) = {
        let Ok(map) = rooms().lock() else { return };
        let Some(q) = map.get(channel) else { return };
        lines_to_repair(q, &new, old.as_deref(), Instant::now())
    };
    if lines.is_empty() {
        return;
    }
    // Tables are rebuilt from sync code too (tests among them); repair needs
    // the runtime and is simply skipped without one.
    let Ok(rt) = tokio::runtime::Handle::try_current() else { return };
    let key = channel.to_string();
    rt.spawn(async move { IrcService::restamp_live_rows(&key, lines, oldest).await });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::emote_service::{Emote, EmoteProvider, EmoteSet};

    fn emote(id: &str, name: &str) -> Emote {
        Emote {
            id: id.into(),
            name: name.into(),
            url: format!("https://cdn.7tv.app/emote/{id}/1x.avif"),
            provider: EmoteProvider::SevenTV,
            is_zero_width: None,
            local_url: None,
            emote_type: None,
            owner_id: None,
            owner_name: None,
            original_name: None,
            owner_ref: None,
            width: None,
            height: None,
            modifier_flags: None,
            ffz_sub_only: None,
            animated: None,
        }
    }

    fn lookup(emotes: Vec<Emote>) -> Arc<EmoteLookup> {
        let mut set = EmoteSet::new();
        set.seven_tv = emotes;
        EmoteLookup::build(&set)
    }

    fn line(text: &str) -> String {
        format!("@id=1;room-id=11 :a!a@a.tmi.twitch.tv PRIVMSG #chan :{text}")
    }

    #[test]
    fn a_channel_emote_beats_any_global_of_the_same_name() {
        let mut global = emote("g", "EZ");
        global.emote_type = Some(crate::services::emote_match::GLOBAL_EMOTE_TYPE.into());
        let mut channel = emote("b", "EZ");
        channel.provider = EmoteProvider::BTTV;
        let mut set = EmoteSet::new();
        set.seven_tv = vec![global];
        set.bttv = vec![channel];
        assert_eq!(EmoteLookup::build(&set).get("EZ").map(|e| e.id.as_str()), Some("b"));
    }

    #[test]
    fn inside_the_channel_group_7tv_beats_bttv() {
        let mut bttv = emote("b", "Clap");
        bttv.provider = EmoteProvider::BTTV;
        let mut set = EmoteSet::new();
        set.seven_tv = vec![emote("s", "Clap")];
        set.bttv = vec![bttv];
        assert_eq!(EmoteLookup::build(&set).get("Clap").map(|e| e.id.as_str()), Some("s"));
    }

    #[test]
    fn an_added_emote_repairs_and_a_kept_one_does_not() {
        let before = lookup(vec![emote("1", "KEKW")]);
        let after = lookup(vec![emote("1", "KEKW"), emote("2", "Clap")]);
        assert!(gains_emote("hi Clap", &after, Some(&*before)));
        assert!(!gains_emote("hi KEKW", &after, Some(&*before)));
        assert!(gains_emote("hi KEKW", &after, None));
    }

    #[test]
    fn a_changed_emote_repairs_and_a_removed_one_does_not() {
        let before = lookup(vec![emote("1", "EZ"), emote("3", "Gone")]);
        let after = lookup(vec![emote("2", "EZ")]);
        assert!(gains_emote("EZ", &after, Some(&*before)));
        assert!(!gains_emote("Gone", &after, Some(&*before)));
    }

    #[test]
    fn partner_and_repeated_lines_are_skipped() {
        let new = lookup(vec![emote("2", "Clap")]);
        let mut q = VecDeque::new();
        let now = Instant::now();
        let mine = line("Clap");
        push_line(&mut q, &mine, now);
        push_line(&mut q, &mine, now);
        push_line(&mut q, "@id=2;room-id=11;source-room-id=22 :b!b@b.tmi.twitch.tv PRIVMSG #chan :Clap", now);
        let (lines, _) = lines_to_repair(&q, &new, None, now);
        assert_eq!(lines, vec![mine]);
    }

    #[test]
    fn the_ring_keeps_only_the_newest_lines() {
        let mut q = VecDeque::new();
        let now = Instant::now();
        for i in 0..(KEEP + 10) {
            push_line(&mut q, &line(&format!("m{i}")), now);
        }
        assert_eq!(q.len(), KEEP);
        assert!(q.back().is_some_and(|l| l.raw.ends_with(&format!("m{}", KEEP + 9))));
    }

    #[test]
    fn old_lines_are_not_repaired() {
        let new = lookup(vec![emote("2", "Clap")]);
        let mut q = VecDeque::new();
        let then = Instant::now();
        push_line(&mut q, &line("Clap"), then);
        let later = then + KEEP_FOR + Duration::from_secs(1);
        let (lines, _) = lines_to_repair(&q, &new, None, later);
        assert!(lines.is_empty());
    }
}
