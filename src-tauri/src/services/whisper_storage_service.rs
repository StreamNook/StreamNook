use log::debug;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use tauri::AppHandle;
use tauri::Manager;

/// Per-account whisper files live under this directory, one `<owner_id>.json`
/// each, so signing out or switching accounts never surfaces another profile's
/// imported whispers.
const WHISPERS_DIR: &str = "whispers";
/// The pre-scoping single global file. Adopted once into the active account's
/// file on first load after the update, then retired.
const LEGACY_WHISPERS_FILE: &str = "whispers.json";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StoredWhisper {
    pub id: String,
    pub from_user_id: String,
    pub from_user_login: String,
    pub from_user_name: String,
    pub to_user_id: String,
    pub to_user_login: String,
    pub to_user_name: String,
    pub message: String,
    pub timestamp: i64,
    pub is_sent: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StoredConversation {
    pub user_id: String,
    pub user_login: String,
    pub user_name: String,
    pub profile_image_url: Option<String>,
    pub messages: Vec<StoredWhisper>,
    pub last_message_timestamp: i64,
    pub unread_count: i32,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct WhisperStorage {
    pub conversations: HashMap<String, StoredConversation>,
    pub version: i32,
}

impl WhisperStorage {
    fn empty() -> Self {
        WhisperStorage {
            conversations: HashMap::new(),
            version: 1,
        }
    }
}

/// One owner's in-memory archive plus where it flushes to.
struct OwnerStore {
    path: PathBuf,
    storage: WhisperStorage,
    dirty: bool,
}

// In-memory archives keyed by sanitized owner id, each lazily seeded from disk
// on first touch (the session's one read per owner). Mutations land here; a
// debounced background task flushes dirty owners (universal-cache pattern).
static STORE: OnceLock<Mutex<HashMap<String, OwnerStore>>> = OnceLock::new();
static ANY_DIRTY: AtomicBool = AtomicBool::new(false);
static FLUSH_TASK_STARTED: AtomicBool = AtomicBool::new(false);

fn store() -> &'static Mutex<HashMap<String, OwnerStore>> {
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn mark_dirty() {
    ANY_DIRTY.store(true, Ordering::Release);
    ensure_flush_task();
}

fn ensure_flush_task() {
    if FLUSH_TASK_STARTED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        // No runtime yet: reset the flag so a later write retries; ANY_DIRTY
        // stays set, so nothing is lost (the exit flush is the last-resort net).
        if tokio::runtime::Handle::try_current().is_err() {
            FLUSH_TASK_STARTED.store(false, Ordering::SeqCst);
            return;
        }
        tokio::spawn(async {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                if !ANY_DIRTY.load(Ordering::Acquire) {
                    continue;
                }
                let flushed = tokio::task::spawn_blocking(WhisperStorageService::flush_now).await;
                match flushed {
                    Ok(Ok(())) => {}
                    // flush_now restored ANY_DIRTY itself on failure.
                    Ok(Err(e)) => {
                        debug!("[WhisperStorage] debounced flush failed (will retry): {}", e)
                    }
                    Err(_) => ANY_DIRTY.store(true, Ordering::Release),
                }
            }
        });
    }
}

pub struct WhisperStorageService;

impl WhisperStorageService {
    /// Keep ids filesystem-safe. Twitch ids are numeric, but guard anyway so a
    /// malformed owner id can never escape the whispers directory.
    fn sanitize_owner(owner_id: &str) -> String {
        owner_id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect()
    }

    /// Path to a specific account's whispers file:
    /// `<app_data>/whispers/<owner_id>.json`.
    fn get_storage_path(app_handle: &AppHandle, owner_id: &str) -> Result<PathBuf, String> {
        let app_data_dir = app_handle
            .path()
            .app_data_dir()
            .map_err(|e| format!("Failed to get app data directory: {}", e))?;
        let dir = app_data_dir.join(WHISPERS_DIR);
        if !dir.exists() {
            fs::create_dir_all(&dir)
                .map_err(|e| format!("Failed to create whispers directory: {}", e))?;
        }
        Ok(dir.join(format!("{}.json", Self::sanitize_owner(owner_id))))
    }

    /// One-time adoption of the pre-scoping global `whispers.json`. Those
    /// whispers belong to whoever is signed in now, so move them under this
    /// owner and retire the global file so other accounts can't inherit them.
    /// Only reached from a first seed (no owner file, no memory entry yet).
    fn adopt_legacy_global(
        app_handle: &AppHandle,
        owner_path: &PathBuf,
    ) -> Result<Option<WhisperStorage>, String> {
        let app_data_dir = app_handle
            .path()
            .app_data_dir()
            .map_err(|e| format!("Failed to get app data directory: {}", e))?;
        let legacy = app_data_dir.join(LEGACY_WHISPERS_FILE);
        if !legacy.exists() {
            return Ok(None);
        }
        let contents = fs::read_to_string(&legacy)
            .map_err(|e| format!("Failed to read legacy whispers file: {}", e))?;
        let storage: WhisperStorage = serde_json::from_str(&contents)
            .map_err(|e| format!("Failed to parse legacy whispers file: {}", e))?;
        let adopted = serde_json::to_string(&storage)
            .map_err(|e| format!("Failed to serialize adopted whispers: {}", e))?;
        fs::write(owner_path, adopted)
            .map_err(|e| format!("Failed to write adopted whispers: {}", e))?;
        // Retire the global file so the next account doesn't adopt it too.
        let _ = fs::rename(&legacy, app_data_dir.join("whispers.json.migrated"));
        debug!(
            "[WhisperStorage] Adopted {} legacy conversations into the active account",
            storage.conversations.len()
        );
        Ok(Some(storage))
    }

    /// Lock the store, seed this owner's archive from disk on first touch, and
    /// run `f` on it. Callers handle the empty-owner guard themselves.
    fn with_owner<R>(
        app_handle: &AppHandle,
        owner_id: &str,
        f: impl FnOnce(&mut OwnerStore) -> R,
    ) -> Result<R, String> {
        let key = Self::sanitize_owner(owner_id);
        let mut guard = store().lock().map_err(|e| e.to_string())?;
        if !guard.contains_key(&key) {
            let path = Self::get_storage_path(app_handle, owner_id)?;
            let storage = if path.exists() {
                let contents = fs::read_to_string(&path)
                    .map_err(|e| format!("Failed to read whispers file: {}", e))?;
                serde_json::from_str(&contents)
                    .map_err(|e| format!("Failed to parse whispers file: {}", e))?
            } else if let Some(adopted) = Self::adopt_legacy_global(app_handle, &path)? {
                adopted
            } else {
                debug!("[WhisperStorage] No whispers for this account, starting empty");
                WhisperStorage::empty()
            };
            guard.insert(
                key.clone(),
                OwnerStore {
                    path,
                    storage,
                    dirty: false,
                },
            );
        }
        Ok(f(guard.get_mut(&key).expect("seeded above")))
    }

    /// Synchronous write of every dirty owner archive, for shutdown paths.
    pub fn flush_now() -> Result<(), String> {
        if !ANY_DIRTY.swap(false, Ordering::AcqRel) {
            return Ok(());
        }
        let mut guard = match store().lock() {
            Ok(g) => g,
            Err(e) => {
                ANY_DIRTY.store(true, Ordering::Release);
                return Err(e.to_string());
            }
        };
        let mut first_err: Option<String> = None;
        for entry in guard.values_mut() {
            if !entry.dirty {
                continue;
            }
            let json = match serde_json::to_string(&entry.storage) {
                Ok(j) => j,
                Err(e) => {
                    first_err.get_or_insert(format!("Failed to serialize whispers: {}", e));
                    continue;
                }
            };
            match fs::write(&entry.path, json) {
                Ok(()) => entry.dirty = false,
                Err(e) => {
                    first_err.get_or_insert(format!("Failed to write whispers file: {}", e));
                }
            }
        }
        if guard.values().any(|s| s.dirty) {
            ANY_DIRTY.store(true, Ordering::Release);
        }
        match first_err {
            None => Ok(()),
            Some(e) => Err(e),
        }
    }

    /// Load one account's whispers. Empty owner (signed out) returns nothing so
    /// no account's whispers are shown.
    pub fn load_whispers(app_handle: &AppHandle, owner_id: &str) -> Result<WhisperStorage, String> {
        if owner_id.trim().is_empty() {
            return Ok(WhisperStorage::empty());
        }
        Self::with_owner(app_handle, owner_id, |o| o.storage.clone())
    }

    /// Save one account's whispers. Empty owner is a no-op so signed-out state
    /// never writes a file.
    pub fn save_whispers(
        app_handle: &AppHandle,
        owner_id: &str,
        storage: &WhisperStorage,
    ) -> Result<(), String> {
        if owner_id.trim().is_empty() {
            return Ok(());
        }
        Self::with_owner(app_handle, owner_id, |o| {
            o.storage = storage.clone();
            o.dirty = true;
        })?;
        mark_dirty();
        Ok(())
    }

    /// Save a single conversation (incremental update).
    pub fn save_conversation(
        app_handle: &AppHandle,
        owner_id: &str,
        user_id: &str,
        conversation: &StoredConversation,
    ) -> Result<(), String> {
        if owner_id.trim().is_empty() {
            return Ok(());
        }
        Self::with_owner(app_handle, owner_id, |o| {
            o.storage
                .conversations
                .insert(user_id.to_string(), conversation.clone());
            o.dirty = true;
        })?;
        mark_dirty();
        Ok(())
    }

    /// Append a single message to a conversation, de-duped by message id.
    pub fn append_message(
        app_handle: &AppHandle,
        owner_id: &str,
        user_id: &str,
        message: StoredWhisper,
    ) -> Result<(), String> {
        if owner_id.trim().is_empty() {
            return Err(format!("Conversation with user {} does not exist", user_id));
        }
        Self::with_owner(app_handle, owner_id, |o| {
            match o.storage.conversations.get_mut(user_id) {
                Some(conversation) => {
                    if !conversation.messages.iter().any(|m| m.id == message.id) {
                        conversation.last_message_timestamp = message.timestamp;
                        conversation.messages.push(message);
                    }
                    o.dirty = true;
                    Ok(())
                }
                None => Err(format!("Conversation with user {} does not exist", user_id)),
            }
        })??;
        mark_dirty();
        Ok(())
    }

    /// Delete a conversation.
    pub fn delete_conversation(
        app_handle: &AppHandle,
        owner_id: &str,
        user_id: &str,
    ) -> Result<(), String> {
        if owner_id.trim().is_empty() {
            return Ok(());
        }
        Self::with_owner(app_handle, owner_id, |o| {
            o.storage.conversations.remove(user_id);
            o.dirty = true;
        })?;
        mark_dirty();
        Ok(())
    }

    /// Storage file path for one account (debugging/export).
    pub fn get_storage_file_path(app_handle: &AppHandle, owner_id: &str) -> Result<String, String> {
        let path = Self::get_storage_path(app_handle, owner_id)?;
        Ok(path.to_string_lossy().to_string())
    }
}
