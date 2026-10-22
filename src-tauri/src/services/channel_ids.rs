//! A Twitch channel's id by login, without leaning on Helix when it can be
//! helped. Everything a room needs after a join (its emote table, 7TV live
//! updates, Shared Chat, the mod view) hangs off this id, and a Helix lookup
//! that failed while a phone woke its radio, or while a token refreshed, used
//! to leave the room without any of it for the rest of the session.
//!
//! Sources, in order: the id IRC itself named for the room (`room-id` on its
//! ROOMSTATE), the id this device saw for the login before, then Helix. A
//! saved id is checked by the room's next ROOMSTATE, which arrives within a
//! second of the join.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use tokio::sync::broadcast;

use crate::services::twitch_service::TwitchService;

/// Logins kept on disk. One line each; dropping the excess forgets channels
/// last seen long ago, which only costs one Helix lookup.
const SAVED_CAP: usize = 5000;
const PROVIDER: &str = "login_ids";
const SAVED_KEY: &str = "all";

/// The waits between attempts once a lookup has failed: each one also ends
/// early when IRC names the room.
const PATIENT_WAITS: [Duration; 4] = [
    Duration::from_secs(2),
    Duration::from_secs(5),
    Duration::from_secs(10),
    Duration::from_secs(30),
];

#[derive(Default)]
struct Ids {
    /// What IRC or Helix said this run.
    live: HashMap<String, String>,
    /// The saved list, loaded on first use.
    saved: Option<HashMap<String, String>>,
    /// Logins answered from the saved list and not confirmed yet.
    unconfirmed: HashSet<String>,
}

fn ids() -> &'static Mutex<Ids> {
    static IDS: OnceLock<Mutex<Ids>> = OnceLock::new();
    IDS.get_or_init(Default::default)
}

fn learned() -> &'static broadcast::Sender<(String, String)> {
    static TX: OnceLock<broadcast::Sender<(String, String)>> = OnceLock::new();
    TX.get_or_init(|| broadcast::channel(64).0)
}

fn saved(state: &mut Ids) -> &mut HashMap<String, String> {
    state.saved.get_or_insert_with(|| {
        crate::services::emote_set_cache::load_provider(PROVIDER, SAVED_KEY).unwrap_or_default()
    })
}

/// Record `login`'s id. True when it corrects an id handed out from the saved
/// list (the login now belongs to someone else), so the caller can redo what
/// it set up with the old one.
pub(crate) fn note(login: &str, id: &str) -> bool {
    let login = login.trim_start_matches('#').to_lowercase();
    if login.is_empty() || id.is_empty() {
        return false;
    }
    let (corrected, to_save) = {
        let Ok(mut state) = ids().lock() else { return false };
        let handed = state.unconfirmed.remove(&login);
        state.live.insert(login.clone(), id.to_string());
        let map = saved(&mut state);
        let before = map.insert(login.clone(), id.to_string());
        let corrected = handed && before.as_deref().is_some_and(|b| b != id);
        let to_save = if before.as_deref() == Some(id) {
            None
        } else {
            if map.len() > SAVED_CAP {
                let excess: Vec<String> =
                    map.keys().filter(|k| **k != login).take(map.len() - SAVED_CAP).cloned().collect();
                for k in excess {
                    map.remove(&k);
                }
            }
            Some(map.clone())
        };
        (corrected, to_save)
    };
    let _ = learned().send((login, id.to_string()));
    // Tests never touch the real saved list in the app data folder.
    if cfg!(test) {
        return corrected;
    }
    if let Some(map) = to_save {
        // Off the caller's task: this runs on the chat read loop.
        let write = move || crate::services::emote_set_cache::save_provider(PROVIDER, SAVED_KEY, &map);
        match tokio::runtime::Handle::try_current() {
            Ok(rt) => {
                rt.spawn_blocking(write);
            }
            Err(_) => write(),
        }
    }
    corrected
}

/// The id known without asking anyone: what IRC or Helix said this run, else
/// the saved one.
pub(crate) fn known(login: &str) -> Option<String> {
    let login = login.trim_start_matches('#').to_lowercase();
    let Ok(mut state) = ids().lock() else { return None };
    if let Some(id) = state.live.get(&login) {
        return Some(id.clone());
    }
    let id = saved(&mut state).get(&login).cloned()?;
    state.unconfirmed.insert(login);
    Some(id)
}

/// One Helix lookup, remembered on success.
async fn ask_helix(login: &str) -> Option<String> {
    match TwitchService::get_user_by_login(login).await {
        Ok(user) => {
            note(login, &user.id);
            Some(user.id)
        }
        Err(e) => {
            log::warn!("[ChannelIds] Helix lookup for {login} failed: {e}");
            None
        }
    }
}

/// The id now: known, else one Helix lookup. Never sleeps, so a pane waiting
/// on a join is never held up by a retry.
pub(crate) async fn resolve(login: &str) -> Option<String> {
    if let Some(id) = known(login) {
        return Some(id);
    }
    ask_helix(login).await
}

/// The id, waiting for it when it is not known: between Helix attempts, IRC
/// naming the room ends the wait at once. `wanted` is checked before every
/// attempt, so a room left meanwhile stops the search. None after about 47 s.
pub(crate) async fn resolve_patiently(login: &str, wanted: impl Fn() -> bool) -> Option<String> {
    let login = login.trim_start_matches('#').to_lowercase();
    let mut heard = learned().subscribe();
    if let Some(id) = resolve(&login).await {
        return Some(id);
    }
    for wait in PATIENT_WAITS {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            match tokio::time::timeout_at(deadline, heard.recv()).await {
                Ok(Ok((l, id))) if l == login => return Some(id),
                Ok(Ok(_)) => {}
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => {
                    if let Some(id) = ids().lock().ok().and_then(|s| s.live.get(&login).cloned()) {
                        return Some(id);
                    }
                }
                Ok(Err(broadcast::error::RecvError::Closed)) | Err(_) => break,
            }
        }
        if !wanted() {
            return None;
        }
        if let Some(id) = ask_helix(&login).await {
            return Some(id);
        }
    }
    None
}

#[cfg(test)]
pub(crate) fn reset_for_test() {
    if let Ok(mut state) = ids().lock() {
        *state = Ids { saved: Some(HashMap::new()), ..Ids::default() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests share one process-wide table.
    static LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn irc_beats_the_saved_id_and_corrects_it() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_test();
        if let Ok(mut s) = ids().lock() {
            saved(&mut s).insert("chan".into(), "1".into());
        }
        assert_eq!(known("#Chan").as_deref(), Some("1"));
        // IRC names a different owner: the handed-out id was wrong.
        assert!(note("chan", "2"));
        assert_eq!(known("chan").as_deref(), Some("2"));
        // Confirming the same id again corrects nothing.
        assert!(!note("chan", "2"));
    }

    #[test]
    fn a_confirmed_id_is_not_a_correction() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_test();
        if let Ok(mut s) = ids().lock() {
            saved(&mut s).insert("room".into(), "7".into());
        }
        assert_eq!(known("room").as_deref(), Some("7"));
        assert!(!note("room", "7"));
    }

    #[test]
    fn the_saved_list_stays_bounded() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset_for_test();
        if let Ok(mut s) = ids().lock() {
            let map = saved(&mut s);
            for i in 0..SAVED_CAP {
                map.insert(format!("u{i}"), i.to_string());
            }
        }
        note("newest", "99999");
        let s = ids().lock().unwrap();
        let map = s.saved.as_ref().unwrap();
        assert_eq!(map.len(), SAVED_CAP);
        assert_eq!(map.get("newest").map(String::as_str), Some("99999"));
    }
}
