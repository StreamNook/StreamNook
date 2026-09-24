//! Hype trains for every channel something is showing, one poll per channel.
//!
//! The main window's stream, each MultiChat pane and the blended MultiChat view
//! all show a hype-train banner, and each used to run its own copy of the same
//! adaptive poll, so one channel open in two places was polled twice. Surfaces
//! now say which channel they are showing; Rust polls each channel once for all
//! of them and announces `hype-train://update { login, train, level_changed }`.
//!
//! A watch belongs to a window. Watches whose window has closed are dropped on
//! the next poll, so a popout that closes without unmounting cannot keep a
//! channel polled for the rest of the session.
//!
//! The poll is Twitch's GQL (no auth, any channel, unaffected by the EventSub
//! hype-train withdrawal): 15 s idle, 3 s while a train runs, 1 s when the next
//! level is close so the level-up lands when it happens.

use serde_json::json;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

pub const UPDATE_EVENT: &str = "hype-train://update";

/// A watch owned by Rust itself rather than a window (the watch session).
pub const INTERNAL_OWNER: &str = "@internal";

const IDLE_POLL: Duration = Duration::from_secs(15);
const ACTIVE_POLL: Duration = Duration::from_secs(3);
const IMMINENT_POLL: Duration = Duration::from_secs(1);
const IMMINENT_RATIO: f64 = 0.85;

struct Watch {
    login: String,
    name: String,
    /// Window label -> how many surfaces in it show this channel.
    owners: HashMap<String, usize>,
    task: Option<tauri::async_runtime::JoinHandle<()>>,
}

static WATCHES: Mutex<Option<HashMap<String, Watch>>> = Mutex::new(None);

fn with_watches<R>(f: impl FnOnce(&mut HashMap<String, Watch>) -> R) -> R {
    let mut guard = WATCHES.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(HashMap::new))
}

fn key_of(login: &str) -> String {
    login.trim().to_lowercase()
}

/// Start showing `login`'s hype train in `owner` (a window label, or
/// INTERNAL_OWNER). The broadcaster id is looked up when not known.
pub fn watch(app: &AppHandle, owner: &str, login: &str, channel_id: Option<String>, name: &str) {
    let key = key_of(login);
    if key.is_empty() {
        return;
    }
    let start = with_watches(|w| {
        let entry = w.entry(key.clone()).or_insert_with(|| Watch {
            login: key.clone(),
            name: if name.is_empty() { key.clone() } else { name.to_string() },
            owners: HashMap::new(),
            task: None,
        });
        *entry.owners.entry(owner.to_string()).or_insert(0) += 1;
        entry.task.is_none()
    });
    if start {
        let task = tauri::async_runtime::spawn(poll(app.clone(), key.clone(), channel_id));
        let orphaned = with_watches(|w| match w.get_mut(&key) {
            Some(entry) if entry.task.is_none() => {
                entry.task = Some(task);
                None
            }
            _ => Some(task),
        });
        if let Some(task) = orphaned {
            task.abort();
        }
    }
}

/// Stop showing `login`'s hype train in `owner`.
pub fn unwatch(owner: &str, login: &str) {
    let key = key_of(login);
    let stopped = with_watches(|w| {
        let entry = w.get_mut(&key)?;
        if let Some(count) = entry.owners.get_mut(owner) {
            *count -= 1;
            if *count == 0 {
                entry.owners.remove(owner);
            }
        }
        if entry.owners.is_empty() {
            w.remove(&key).and_then(|e| e.task)
        } else {
            None
        }
    });
    if let Some(task) = stopped {
        task.abort();
    }
}

/// Drop the owners whose window is gone. True while anyone still watches.
fn still_watched(app: &AppHandle, key: &str) -> bool {
    with_watches(|w| {
        let Some(entry) = w.get_mut(key) else { return false };
        entry
            .owners
            .retain(|owner, _| owner == INTERNAL_OWNER || app.get_webview_window(owner).is_some());
        if entry.owners.is_empty() {
            w.remove(key);
            return false;
        }
        true
    })
}

async fn resolve_id(login: &str) -> Option<String> {
    crate::services::twitch_service::TwitchService::get_user_by_login(login)
        .await
        .ok()
        .map(|u| u.id)
}

async fn poll(app: AppHandle, key: String, channel_id: Option<String>) {
    let mut channel_id = channel_id.filter(|id| !id.is_empty());
    let mut previous_level = 0;
    let mut had_train = false;
    loop {
        if !still_watched(&app, &key) {
            return;
        }
        let mut next = IDLE_POLL;
        if channel_id.is_none() {
            channel_id = resolve_id(&key).await;
        }
        if let Some(id) = channel_id.clone() {
            if let Ok(status) = crate::commands::hype_train::get_hype_train_status(id.clone(), key.clone()).await {
                let name = with_watches(|w| w.get(&key).map(|e| e.name.clone())).unwrap_or_else(|| key.clone());
                if status.is_active && status.level >= 1 {
                    let level_changed = status.level > previous_level || !had_train;
                    previous_level = status.level;
                    had_train = true;
                    let imminent = status.goal > 0
                        && f64::from(status.progress) / f64::from(status.goal) > IMMINENT_RATIO;
                    next = if imminent { IMMINENT_POLL } else { ACTIVE_POLL };
                    let _ = app.emit(
                        UPDATE_EVENT,
                        json!({
                            "login": key,
                            "level_changed": level_changed,
                            "train": {
                                "id": status.id.clone().unwrap_or_default(),
                                "broadcaster_user_id": id,
                                "broadcaster_user_login": key,
                                "broadcaster_user_name": name,
                                "level": status.level,
                                "total": status.total,
                                "progress": status.progress,
                                "goal": status.goal,
                                "top_contributions": [],
                                "started_at": status.started_at.clone().unwrap_or_default(),
                                "expires_at": status.expires_at.clone().unwrap_or_default(),
                                "is_golden_kappa": status.is_golden_kappa,
                            },
                        }),
                    );
                } else if had_train {
                    had_train = false;
                    previous_level = 0;
                    let _ = app.emit(UPDATE_EVENT, json!({ "login": key, "level_changed": false, "train": null }));
                }
            }
        }
        tokio::time::sleep(next).await;
    }
}

/// Everyone watching (diagnostics): login -> owner count.
pub fn watched() -> Vec<(String, usize)> {
    with_watches(|w| w.values().map(|e| (e.login.clone(), e.owners.values().sum())).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owners(login: &str) -> Option<usize> {
        with_watches(|w| w.get(login).map(|e| e.owners.values().sum()))
    }

    #[test]
    fn watches_are_counted_per_owner_and_released_to_zero() {
        // Registered without a running app: exercise the bookkeeping directly.
        with_watches(|w| {
            let entry = w.entry("chan".into()).or_insert_with(|| Watch {
                login: "chan".into(),
                name: "Chan".into(),
                owners: HashMap::new(),
                task: None,
            });
            *entry.owners.entry("main".into()).or_insert(0) += 2;
            *entry.owners.entry("multichat-1".into()).or_insert(0) += 1;
        });
        assert_eq!(owners("chan"), Some(3));
        unwatch("main", "Chan");
        assert_eq!(owners("chan"), Some(2));
        unwatch("multichat-1", "chan");
        unwatch("main", "chan");
        assert_eq!(owners("chan"), None);
        // Releasing what was never watched is harmless.
        unwatch("main", "chan");
        assert!(watched().iter().all(|(l, _)| l != "chan"));
    }
}
