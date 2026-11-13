//! The people whispered to in the last 24 hours, per sending account: the
//! local 40-a-day check Twitch enforces, and the set the automation cap's
//! local half reads (D285, D286).
//!
//! One process-wide store, loaded by whichever comes first (the engine's init
//! or a cold `cold_reply`) and flushed to `automation_whisper_recipients.json`
//! (tmp then rename, `.bad` on an unreadable file) by both, so a later flush
//! never drops a recipient the other path booked. It never touches
//! `automation_counts.json`. Each recipient is keyed by its latest send, so a
//! repeat whisper refreshes it. Your own whispers are recorded, never refused.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

pub const FILE: &str = "automation_whisper_recipients.json";
pub const DAY_MS: i64 = 24 * 3_600_000;
/// Twitch's own limit: new people an account may whisper in 24 hours.
pub const TWITCH_PER_DAY: usize = 40;

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Recipients {
    pub version: u32,
    /// sender account id -> recipient id -> wall ms of the latest send.
    pub by_sender: HashMap<String, HashMap<String, i64>>,
}

impl Recipients {
    fn prune(&mut self, now: i64) {
        for map in self.by_sender.values_mut() {
            map.retain(|_, at| now - *at < DAY_MS);
        }
        self.by_sender.retain(|_, map| !map.is_empty());
    }

    /// People `sender` whispered to inside 24 h.
    pub fn counted(&self, sender: &str, now: i64) -> usize {
        self.by_sender
            .get(sender)
            .map_or(0, |m| m.values().filter(|at| now - **at < DAY_MS).count())
    }

    /// Whether `recipient` already counts for `sender` at `now`.
    pub fn has(&self, sender: &str, recipient: &str, now: i64) -> bool {
        self.by_sender
            .get(sender)
            .and_then(|m| m.get(recipient))
            .is_some_and(|at| now - *at < DAY_MS)
    }

    /// The oldest counted send plus 24 h: when a place frees.
    pub fn frees_at(&self, sender: &str, now: i64) -> Option<i64> {
        self.by_sender
            .get(sender)?
            .values()
            .filter(|at| now - **at < DAY_MS)
            .min()
            .map(|at| at + DAY_MS)
    }

    /// Book one send. An automated send to a new person is refused at the
    /// Twitch limit; your own is always recorded.
    pub fn book(
        &mut self,
        sender: &str,
        recipient: &str,
        now: i64,
        automated: bool,
    ) -> Result<(), String> {
        self.prune(now);
        if automated
            && !self.has(sender, recipient, now)
            && self.counted(sender, now) >= TWITCH_PER_DAY
        {
            return Err("Twitch allows whispering 40 new people a day".to_string());
        }
        self.by_sender
            .entry(sender.to_string())
            .or_default()
            .insert(recipient.to_string(), now);
        Ok(())
    }

    pub fn to_json(&self) -> String {
        let mut out = self.clone();
        out.version = 1;
        serde_json::to_string(&out).unwrap_or_default()
    }

    pub fn from_json(text: &str) -> Result<Recipients, String> {
        serde_json::from_str(text).map_err(|e| e.to_string())
    }
}

/// The process-wide state: the loaded store and the folder it flushes to.
/// Under `cfg(test)` it is thread-local, so parallel tests never share a store
/// or a folder, and with no test folder set nothing is read or written (a
/// whisper test never touches the real app data folder).
#[derive(Default)]
struct State {
    store: Option<Recipients>,
    dir: Option<PathBuf>,
    writes: u64,
}

#[cfg(not(test))]
static STATE: Mutex<Option<State>> = Mutex::new(None);

#[cfg(test)]
thread_local! {
    static TEST_STATE: std::cell::RefCell<State> = std::cell::RefCell::new(State::default());
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    #[cfg(not(test))]
    {
        let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
        f(guard.get_or_insert_with(State::default))
    }
    #[cfg(test)]
    {
        TEST_STATE.with(|s| f(&mut s.borrow_mut()))
    }
}

/// The folder the file lives in: the app data dir; under `cfg(test)` only the
/// folder a test set.
fn real_dir(state: &State) -> Option<PathBuf> {
    #[cfg(test)]
    {
        state.dir.clone()
    }
    #[cfg(not(test))]
    {
        state
            .dir
            .clone()
            .or_else(|| crate::services::twitch_service::get_app_data_dir().ok())
    }
}

#[cfg(test)]
pub(crate) fn set_dir_for_tests(path: PathBuf) {
    with(|s| {
        *s = State {
            store: None,
            dir: Some(path),
            writes: 0,
        }
    });
}

#[cfg(test)]
pub(crate) fn forget_loaded_for_tests() {
    with(|s| s.store = None);
}

fn ensure_loaded_in(state: &mut State) {
    if state.store.is_none() {
        let loaded = real_dir(state).map(|d| load_from(&d)).unwrap_or_default();
        state.store = Some(loaded);
    }
}

/// Load the store once (a second call keeps what is in memory).
pub fn ensure_loaded() {
    with(ensure_loaded_in);
}

fn load_from(dir: &Path) -> Recipients {
    let path = dir.join(FILE);
    match std::fs::read_to_string(&path) {
        Ok(text) => Recipients::from_json(&text).unwrap_or_else(|_| {
            let _ = std::fs::rename(&path, dir.join(format!("{FILE}.bad")));
            Recipients::default()
        }),
        Err(_) => Recipients::default(),
    }
}

/// Write the file (a per-write temp name, then rename), holding the state's
/// lock so two writers never share a temp file or write an older copy last.
fn flush_in(state: &mut State) {
    let (Some(dir), Some(store)) = (real_dir(state), state.store.clone()) else {
        return;
    };
    state.writes += 1;
    let tmp = dir.join(format!(
        "{FILE}.{}.{}.tmp",
        std::process::id(),
        state.writes
    ));
    let ok =
        std::fs::write(&tmp, store.to_json()).and_then(|_| std::fs::rename(&tmp, dir.join(FILE)));
    if let Err(e) = ok {
        let _ = std::fs::remove_file(&tmp);
        log::warn!("[Automations] could not save the whisper recipients: {e}");
    }
}

/// Write the file now. Both the engine and a cold process call it.
pub fn flush() {
    with(flush_in);
}

/// Before a send: an automated send to a new person is refused at Twitch's 40
/// a day; your own is never refused. Changes nothing (the booking comes after
/// the send, `book_recipient`), and runs under `SEND_LOCK`, so no other whisper
/// books between the check and the booking.
pub fn check_recipient(
    sender: &str,
    recipient: &str,
    now_ms: i64,
    automated: bool,
) -> Result<(), String> {
    with(|s| {
        ensure_loaded_in(s);
        let store = s.store.as_ref().expect("loaded above");
        if automated
            && !store.has(sender, recipient, now_ms)
            && store.counted(sender, now_ms) >= TWITCH_PER_DAY
        {
            Err("Twitch allows whispering 40 new people a day".to_string())
        } else {
            Ok(())
        }
    })
}

/// After a send that was Sent or may have been delivered (Unknown): record the
/// recipient by its latest send. A refused send books nothing. Every whisper
/// path calls this: `run_queued_classified` (send, retry, `/r`, inline reply)
/// and `cold_reply`.
pub fn book_recipient(sender: &str, recipient: &str, now_ms: i64) {
    with(|s| {
        ensure_loaded_in(s);
        if let Some(store) = s.store.as_mut() {
            let _ = store.book(sender, recipient, now_ms, false);
        }
        flush_in(s);
    });
}

/// The store's own read for the engine's local checks.
pub fn snapshot() -> Recipients {
    with(|s| {
        ensure_loaded_in(s);
        s.store.clone().unwrap_or_default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: i64 = 3_600_000;

    #[test]
    fn the_41st_automated_recipient_is_refused_and_your_own_is_recorded() {
        let mut r = Recipients::default();
        for i in 0..40 {
            r.book("me", &format!("u{i}"), 1_000, true).unwrap();
        }
        assert!(r.book("me", "u40", 1_000, true).is_err());
        assert!(
            r.book("me", "u0", 2_000, true).is_ok(),
            "a repeat recipient is not new"
        );
        assert!(
            r.book("me", "u40", 3_000, false).is_ok(),
            "your own is recorded, never refused"
        );
        assert_eq!(r.counted("me", 3_000), 41);
    }

    #[test]
    fn a_repeat_refreshes_so_a_recipient_at_23h_still_counts_at_25h() {
        let mut r = Recipients::default();
        r.book("me", "a", 0, true).unwrap();
        r.book("me", "a", 23 * H, true).unwrap();
        assert!(r.has("me", "a", 25 * H));
        assert!(!r.has("me", "a", 48 * H));
        assert_eq!(r.frees_at("me", 25 * H), Some(47 * H));
    }

    #[test]
    fn with_no_test_folder_nothing_touches_the_real_app_data_dir() {
        // A fresh thread has no folder: the check and the booking work in memory only.
        assert!(check_recipient("me", "a", 1_000, true).is_ok());
        book_recipient("me", "a", 1_000);
        assert!(snapshot().has("me", "a", 2_000));
        assert!(with(|s| real_dir(s)).is_none());
    }

    #[test]
    fn the_check_never_books_and_a_sent_send_does() {
        set_dir_for_tests(std::env::temp_dir().join("sn-recipients-unused"));
        for i in 0..40 {
            book_recipient("me", &format!("u{i}"), 1_000);
        }
        assert!(check_recipient("me", "new", 2_000, true).is_err());
        assert!(!snapshot().has("me", "new", 2_000), "the check never books");
        assert!(
            check_recipient("me", "new", 2_000, false).is_ok(),
            "your own is never refused"
        );
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join("sn-recipients-unused"));
    }

    #[test]
    fn it_survives_a_restart_and_reads_a_bad_file_aside() {
        let dir = std::env::temp_dir().join(format!(
            "sn-recipients-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        set_dir_for_tests(dir.clone());
        book_recipient("me", "a", 1_000);
        forget_loaded_for_tests();
        assert!(snapshot().has("me", "a", 2_000));
        std::fs::write(dir.join(FILE), "not json").unwrap();
        forget_loaded_for_tests();
        assert_eq!(snapshot(), Recipients::default());
        assert!(dir.join(format!("{FILE}.bad")).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_cold_flush_never_writes_the_counts_file_and_leaves_no_temp() {
        let dir = std::env::temp_dir().join(format!(
            "sn-recipients-cold-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        set_dir_for_tests(dir.clone());
        std::fs::write(dir.join("automation_counts.json"), "{\"clock\":1}").unwrap();
        book_recipient("me", "a", 1_000);
        assert_eq!(
            std::fs::read_to_string(dir.join("automation_counts.json")).unwrap(),
            "{\"clock\":1}"
        );
        assert!(std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .all(|e| !e.file_name().to_string_lossy().ends_with(".tmp")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
