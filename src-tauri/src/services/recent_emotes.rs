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

use crate::services::emote_service::{Emote, EmoteSet};

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
