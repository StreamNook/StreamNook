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
//!
//! Each line carries the generation of the table it was remembered under (0 is
//! no table at all), and a room's generation rises with every table put in
//! place. Only a line from an older generation is ever repaired, and once a
//! table lands every remembered line counts as covered by it, so no line is
//! sent twice for the same table. Leaving a channel forgets the room.

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
    /// The room's table generation when the line was remembered; 0 is none.
    generation: u64,
}

#[derive(Default)]
struct Room {
    /// How many tables have been put in place for the room; 0 is none yet.
    generation: u64,
    lines: VecDeque<Line>,
}

fn rooms() -> &'static Mutex<HashMap<String, Room>> {
    static ROOMS: OnceLock<Mutex<HashMap<String, Room>>> = OnceLock::new();
    ROOMS.get_or_init(Default::default)
}

/// Remember a chat line for `channel` (lowercase, no `#`, the parse-table key).
pub(crate) fn remember(channel: &str, raw: &str) {
    if channel.is_empty() {
        return;
    }
    let Ok(mut map) = rooms().lock() else { return };
    let now = Instant::now();
    // Rooms left behind (a channel browsed past) lose their lines once their
    // newest line is too old to repair, and drop out unless a table is in
    // place for them. Checked only when a room is added, so a line for a room
    // already open stays a single push.
    if map.len() > 32 && !map.contains_key(channel) {
        map.retain(|_, r| {
            if r.lines.back().is_some_and(|l| now.duration_since(l.at) <= KEEP_FOR) {
                return true;
            }
            r.lines.clear();
            r.generation > 0
        });
    }
    let room = map.entry(channel.to_string()).or_default();
    push_line(&mut room.lines, room.generation, raw, now);
}

/// Drop everything remembered for `room`, as when the channel is left. A
/// rejoin starts clean: its lines are remembered under the table it installs.
pub(crate) fn forget(room: &str) {
    if let Ok(mut map) = rooms().lock() {
        map.remove(room);
    }
}

fn push_line(q: &mut VecDeque<Line>, generation: u64, raw: &str, now: Instant) {
    while q.front().is_some_and(|l| now.duration_since(l.at) > KEEP_FOR) || q.len() >= KEEP {
        q.pop_front();
    }
    q.push_back(Line { at: now, raw: raw.to_string(), generation });
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

/// The lines of `q` the change from `old` to `new` (generation `generation`)
/// repairs, each once, and how long ago the oldest of them arrived. A line
/// remembered with no table is compared against none; a line from an older
/// table is compared against `old`, and skipped when `old` is not known. A
/// line already under `generation` is never repaired.
fn lines_to_repair(
    q: &VecDeque<Line>,
    generation: u64,
    new: &EmoteLookup,
    old: Option<&EmoteLookup>,
    now: Instant,
) -> (Vec<String>, Duration) {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut oldest = Duration::ZERO;
    let mut out = Vec::new();
    for l in q {
        let age = now.duration_since(l.at);
        if age > KEEP_FOR
            || l.generation >= generation
            || is_partner_line(&l.raw)
            || !seen.insert(l.raw.as_str())
        {
            continue;
        }
        let before = match (l.generation, old) {
            (0, _) => None,
            (_, Some(o)) => Some(o),
            (_, None) => continue,
        };
        if gains_emote(irc_trailing_text(&l.raw), new, before) {
            oldest = oldest.max(age);
            out.push(l.raw.clone());
        }
    }
    (out, oldest)
}

/// Put `new` in place for `channel`: the lines it repairs and how long ago the
/// oldest arrived. Every remembered line then counts as covered by it.
fn install_table(
    channel: &str,
    old: Option<&EmoteLookup>,
    new: &EmoteLookup,
    now: Instant,
) -> Option<(Vec<String>, Duration)> {
    let Ok(mut map) = rooms().lock() else { return None };
    let room = map.entry(channel.to_string()).or_default();
    room.generation += 1;
    let found = lines_to_repair(&room.lines, room.generation, new, old, now);
    let generation = room.generation;
    for l in room.lines.iter_mut() {
        l.generation = generation;
    }
    Some(found)
}

/// `channel`'s parse table went from `old` to `new`.
pub(crate) fn table_changed(channel: &str, old: Option<Arc<EmoteLookup>>, new: Arc<EmoteLookup>) {
    let Some((lines, oldest)) = install_table(channel, old.as_deref(), &new, Instant::now()) else {
        return;
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
        push_line(&mut q, 0, &mine, now);
        push_line(&mut q, 0, &mine, now);
        push_line(&mut q, 0, "@id=2;room-id=11;source-room-id=22 :b!b@b.tmi.twitch.tv PRIVMSG #chan :Clap", now);
        let (lines, _) = lines_to_repair(&q, 1, &new, None, now);
        assert_eq!(lines, vec![mine]);
    }

    #[test]
    fn the_ring_keeps_only_the_newest_lines() {
        let mut q = VecDeque::new();
        let now = Instant::now();
        for i in 0..(KEEP + 10) {
            push_line(&mut q, 0, &line(&format!("m{i}")), now);
        }
        assert_eq!(q.len(), KEEP);
        assert!(q.back().is_some_and(|l| l.raw.ends_with(&format!("m{}", KEEP + 9))));
    }

    #[test]
    fn old_lines_are_not_repaired() {
        let new = lookup(vec![emote("2", "Clap")]);
        let mut q = VecDeque::new();
        let then = Instant::now();
        push_line(&mut q, 0, &line("Clap"), then);
        let later = then + KEEP_FOR + Duration::from_secs(1);
        let (lines, _) = lines_to_repair(&q, 1, &new, None, later);
        assert!(lines.is_empty());
    }

    fn install(channel: &str, old: Option<&EmoteLookup>, new: &EmoteLookup) -> Vec<String> {
        install_table(channel, old, new, Instant::now()).map(|(lines, _)| lines).unwrap_or_default()
    }

    #[test]
    fn lines_with_no_table_are_repaired_once() {
        let room = "restamp_no_table";
        let clap = lookup(vec![emote("2", "Clap")]);
        let mine = line("hi Clap");
        remember(room, &mine);
        remember(room, &line("plain words"));
        assert_eq!(install(room, None, &clap), vec![mine]);
        assert!(install(room, Some(&*clap), &clap).is_empty());
    }

    #[test]
    fn a_newer_table_repairs_lines_that_use_what_it_adds() {
        let room = "restamp_newer_table";
        let first = lookup(vec![emote("1", "KEKW")]);
        let second = lookup(vec![emote("1", "KEKW"), emote("2", "Clap")]);
        assert!(install(room, None, &first).is_empty());
        let kept = line("hi KEKW");
        let added = line("hi Clap");
        remember(room, &kept);
        remember(room, &added);
        assert_eq!(install(room, Some(&*first), &second), vec![added]);
        assert!(install(room, Some(&*second), &second).is_empty());
    }

    #[test]
    fn lines_under_the_current_table_are_never_repaired() {
        let room = "restamp_current_table";
        let clap = lookup(vec![emote("2", "Clap")]);
        assert!(install(room, None, &clap).is_empty());
        remember(room, &line("hi Clap"));
        assert!(install(room, Some(&*clap), &clap).is_empty());
        assert!(install(room, None, &clap).is_empty());
    }

    #[test]
    fn leaving_and_rejoining_with_the_same_table_sends_nothing() {
        let room = "restamp_part_rejoin";
        let clap = lookup(vec![emote("2", "Clap")]);
        assert!(install(room, None, &clap).is_empty());
        remember(room, &line("hi Clap"));
        forget(room);
        assert!(install(room, None, &clap).is_empty());
        remember(room, &line("again Clap"));
        assert!(install(room, Some(&*clap), &clap).is_empty());
    }

    #[test]
    fn a_table_dropped_without_forgetting_never_resends_its_lines() {
        let room = "restamp_dropped_table";
        let clap = lookup(vec![emote("2", "Clap")]);
        assert!(install(room, None, &clap).is_empty());
        remember(room, &line("hi Clap"));
        assert!(install(room, None, &clap).is_empty());
    }

    #[test]
    fn forget_drops_the_room() {
        let room = "restamp_forget";
        remember(room, &line("hi Clap"));
        assert!(rooms().lock().unwrap().contains_key(room));
        forget(room);
        assert!(!rooms().lock().unwrap().contains_key(room));
        let clap = lookup(vec![emote("2", "Clap")]);
        assert!(install(room, None, &clap).is_empty());
    }
}
