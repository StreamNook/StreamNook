//! The emotes the viewer used most recently, learned from the messages they
//! type and send (`send_chat_message`), so the picker can offer them first
//! ("Recently used" on its Favorites tab). Paced and scripted sends do not
//! count: they are not someone reaching for an emote.
//!
//! A word counts when the room's emote set (or the viewer's 7TV personal set)
//! has an emote of exactly that name. The list is newest first, one entry per
//! emote, at most `MAX`, kept in `recent_emotes.json` in the cache folder.

use std::collections::VecDeque;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::services::emote_service::{Emote, EmoteProvider, EmoteSet};

/// How many emotes the list remembers.
pub const MAX: usize = 30;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentEmote {
    pub id: String,
    pub name: String,
}

static RECENT: Mutex<Option<VecDeque<RecentEmote>>> = Mutex::new(None);

fn file() -> Option<std::path::PathBuf> {
    crate::services::cache_service::get_cache_dir().ok().map(|d| d.join("recent_emotes.json"))
}

fn with_list<R>(f: impl FnOnce(&mut VecDeque<RecentEmote>) -> R) -> R {
    let mut guard = RECENT.lock().unwrap_or_else(|e| e.into_inner());
    let list = guard.get_or_insert_with(|| {
        file()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice::<VecDeque<RecentEmote>>(&bytes).ok())
            .unwrap_or_default()
    });
    f(list)
}

/// The emotes `text` uses, in the order they appear, each once.
pub fn emotes_in(text: &str, find: impl Fn(&str) -> Option<RecentEmote>) -> Vec<RecentEmote> {
    let mut out: Vec<RecentEmote> = Vec::new();
    for word in text.split_whitespace() {
        if let Some(e) = find(word) {
            out.retain(|o| o.id != e.id);
            out.push(e);
        }
    }
    out
}

/// Puts `used` at the front (the last one used first), dropping older copies
/// and anything past `MAX`.
pub fn remember(list: &mut VecDeque<RecentEmote>, used: Vec<RecentEmote>) {
    for e in used {
        list.retain(|o| o.id != e.id);
        list.push_front(e);
    }
    list.truncate(MAX);
}

fn lookup(set: &EmoteSet, word: &str) -> Option<RecentEmote> {
    set.twitch
        .iter()
        .chain(&set.seven_tv)
        .chain(&set.ffz)
        .chain(&set.bttv)
        .chain(&set.kick)
        .find(|e: &&Emote| e.name == word)
        .map(|e| RecentEmote { id: e.id.clone(), name: e.name.clone() })
}

/// Learns the emotes a message the viewer sent in `channel` used.
pub async fn record(channel: &str, text: &str) {
    let personal = crate::services::account_store::AccountStore::primary().and_then(|me| {
        crate::services::irc_service::with_personal_emotes(&me.user_id, |map| map.clone())
    });
    let used = crate::services::irc_service::with_channel_emotes(channel, |set| {
        emotes_in(text, |word| {
            personal
                .as_ref()
                .and_then(|p| p.get(word))
                .map(|e| RecentEmote { id: e.id.clone(), name: e.name.clone() })
                .or_else(|| lookup(set, word))
        })
    })
    .await
    .unwrap_or_default();
    if used.is_empty() {
        return;
    }
    let snapshot = with_list(|list| {
        remember(list, used);
        list.clone()
    });
    if let (Some(path), Ok(bytes)) = (file(), serde_json::to_vec(&snapshot)) {
        if let Err(e) = std::fs::write(&path, bytes) {
            log::debug!("[recent_emotes] could not save: {e}");
        }
    }
}

/// The remembered emote ids, newest first.
pub fn ids() -> Vec<String> {
    with_list(|list| list.iter().map(|e| e.id.clone()).collect())
}

/// Most distinct words of one message looked up for emote usage.
const MAX_UNIQUE_WORDS: usize = 40;
/// Longest word, in characters, looked up for emote usage.
const MAX_WORD_LEN: usize = 40;

/// One emote a sent message used, with how many times it appeared.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmoteUse {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub url: String,
    pub count: i64,
}

fn provider_name(provider: &EmoteProvider) -> &'static str {
    match provider {
        EmoteProvider::Twitch => "twitch",
        EmoteProvider::BTTV => "bttv",
        EmoteProvider::SevenTV => "7tv",
        EmoteProvider::FFZ => "ffz",
        EmoteProvider::Kick => "kick",
    }
}

fn use_of(e: &Emote) -> EmoteUse {
    EmoteUse {
        id: e.id.clone(),
        name: e.name.clone(),
        provider: provider_name(&e.provider).to_string(),
        url: e.url.clone(),
        count: 1,
    }
}

/// The emotes `text` uses, each once with its occurrence count, in order of
/// first appearance. Only the first `MAX_UNIQUE_WORDS` distinct words of at most
/// `MAX_WORD_LEN` characters are looked up.
pub fn count_uses(text: &str, find: impl Fn(&str) -> Option<EmoteUse>) -> Vec<EmoteUse> {
    let mut looked: Vec<(&str, Option<EmoteUse>)> = Vec::new();
    for word in text.split_whitespace() {
        if word.chars().count() > MAX_WORD_LEN || looked.iter().any(|(w, _)| *w == word) {
            continue;
        }
        if looked.len() >= MAX_UNIQUE_WORDS {
            break;
        }
        looked.push((word, find(word)));
    }
    let mut out: Vec<EmoteUse> = Vec::new();
    for word in text.split_whitespace() {
        let Some(Some(found)) = looked.iter().find(|(w, _)| *w == word).map(|(_, f)| f) else {
            continue;
        };
        match out.iter_mut().find(|o| o.id == found.id) {
            Some(o) => o.count += 1,
            None => out.push(found.clone()),
        }
    }
    out
}

/// The emotes a message sent in the joined chat `channel` used: the viewer's
/// 7TV personal set first, then the channel's set, as `record` resolves them.
pub async fn used_in(channel: &str, text: &str) -> Vec<EmoteUse> {
    let personal = crate::services::account_store::AccountStore::primary().and_then(|me| {
        crate::services::irc_service::with_personal_emotes(&me.user_id, |map| map.clone())
    });
    crate::services::irc_service::with_channel_emotes(channel, |set| {
        count_uses(text, |word| {
            personal
                .as_ref()
                .and_then(|p| p.get(word))
                .map(use_of)
                .or_else(|| find_for_tally(set, word))
        })
    })
    .await
    .unwrap_or_default()
}

/// The emotes a message used, resolved against `set` alone.
pub fn used_in_set(set: &EmoteSet, text: &str) -> Vec<EmoteUse> {
    count_uses(text, |word| find_for_tally(set, word))
}

/// Resolves a word for the usage tally when several providers share a name:
/// 7TV first, then FFZ, BTTV, Twitch and Kick.
fn find_for_tally(set: &EmoteSet, word: &str) -> Option<EmoteUse> {
    set.seven_tv
        .iter()
        .chain(&set.ffz)
        .chain(&set.bttv)
        .chain(&set.twitch)
        .chain(&set.kick)
        .find(|e: &&Emote| e.name == word)
        .map(use_of)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(id: &str, name: &str) -> RecentEmote {
        RecentEmote { id: id.into(), name: name.into() }
    }

    #[test]
    fn only_known_names_count_once_each() {
        let find = |w: &str| match w {
            "Kappa" => Some(e("1", "Kappa")),
            "LULW" => Some(e("2", "LULW")),
            _ => None,
        };
        let used = emotes_in("Kappa hi LULW Kappa kappa", find);
        assert_eq!(used, vec![e("2", "LULW"), e("1", "Kappa")], "the last use decides the order");
    }

    fn u(id: &str, name: &str) -> EmoteUse {
        EmoteUse { id: id.into(), name: name.into(), provider: "7tv".into(), url: "u".into(), count: 1 }
    }

    fn known(w: &str) -> Option<EmoteUse> {
        match w {
            "KEKW" => Some(u("1", "KEKW")),
            "LULW" => Some(u("2", "LULW")),
            _ => None,
        }
    }

    #[test]
    fn occurrences_are_counted_per_emote() {
        let used = count_uses("KEKW hi LULW KEKW KEKW kekw", known);
        assert_eq!(used.len(), 2);
        assert_eq!((used[0].id.as_str(), used[0].count), ("1", 3));
        assert_eq!((used[1].id.as_str(), used[1].count), ("2", 1));
    }

    #[test]
    fn only_forty_unique_words_are_looked_up() {
        let mut text: String = (0..40).map(|i| format!("w{i} ")).collect();
        text.push_str("KEKW");
        assert!(count_uses(&text, known).is_empty());
        let mut text: String = (0..39).map(|i| format!("w{i} ")).collect();
        text.push_str("KEKW KEKW");
        assert_eq!(count_uses(&text, known)[0].count, 2);
    }

    #[test]
    fn words_over_forty_characters_are_skipped() {
        let long = "x".repeat(41);
        let find = |w: &str| if w == long { Some(u("9", "long")) } else { known(w) };
        let used = count_uses(&format!("{long} KEKW"), find);
        assert_eq!(used.len(), 1);
        assert_eq!(used[0].id, "1");
    }

    fn emote(id: &str, provider: EmoteProvider) -> Emote {
        Emote {
            id: id.into(),
            name: "Same".into(),
            url: "u".into(),
            provider,
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

    #[test]
    fn a_shared_name_is_credited_7tv_then_ffz_then_bttv_then_twitch() {
        let mut set = EmoteSet {
            twitch: vec![emote("tw", EmoteProvider::Twitch)],
            bttv: vec![emote("bt", EmoteProvider::BTTV)],
            seven_tv: vec![emote("7", EmoteProvider::SevenTV)],
            ffz: vec![emote("ff", EmoteProvider::FFZ)],
            kick: Vec::new(),
            seven_tv_ok: true,
        };
        let credited = |set: &EmoteSet| used_in_set(set, "Same")[0].id.clone();
        assert_eq!(credited(&set), "7");
        set.seven_tv.clear();
        assert_eq!(credited(&set), "ff");
        set.ffz.clear();
        assert_eq!(credited(&set), "bt");
        set.bttv.clear();
        assert_eq!(credited(&set), "tw");
    }

    #[test]
    fn the_newest_goes_first_and_the_list_is_capped() {
        let mut list: VecDeque<RecentEmote> = (0..MAX).map(|i| e(&i.to_string(), "x")).collect();
        remember(&mut list, vec![e("5", "x"), e("new", "y")]);
        assert_eq!(list.len(), MAX);
        assert_eq!(list[0].id, "new");
        assert_eq!(list[1].id, "5");
        assert_eq!(list.iter().filter(|r| r.id == "5").count(), 1);
    }
}
