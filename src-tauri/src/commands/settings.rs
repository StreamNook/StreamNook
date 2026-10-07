use crate::models::settings::{AppState, Settings};
use crate::services::cache_service;
use crate::services::live_notification_service::LiveNotification;
use log::debug;
use regex::Regex;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use crate::rt::AppHandle;
use tauri::{Emitter, State};

// The debounced flusher snapshots the CURRENT in-memory settings from here at
// flush time (the same Arc the managed AppState holds), never a caller-supplied
// snapshot buffered earlier: two direct writers exist (import_settings, and
// window-state-style immediate paths), and a stale buffered flush would clobber
// them. Registered once from main() before the AppState is managed.
static SETTINGS_SOURCE: OnceLock<Arc<Mutex<Settings>>> = OnceLock::new();
static SETTINGS_DIRTY: AtomicBool = AtomicBool::new(false);
static SETTINGS_FLUSH_TASK_STARTED: AtomicBool = AtomicBool::new(false);

pub fn register_settings_source(source: Arc<Mutex<Settings>>) {
    let _ = SETTINGS_SOURCE.set(source);
}

fn ensure_settings_flush_task() {
    if SETTINGS_FLUSH_TASK_STARTED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        if tokio::runtime::Handle::try_current().is_err() {
            SETTINGS_FLUSH_TASK_STARTED.store(false, Ordering::SeqCst);
            return;
        }
        tokio::spawn(async {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                if !SETTINGS_DIRTY.load(Ordering::Acquire) {
                    continue;
                }
                let flushed = tokio::task::spawn_blocking(flush_settings_now).await;
                match flushed {
                    Ok(Ok(())) => {}
                    // flush_settings_now restored the dirty flag itself.
                    Ok(Err(e)) => debug!("[Settings] debounced flush failed (will retry): {}", e),
                    Err(_) => SETTINGS_DIRTY.store(true, Ordering::Release),
                }
            }
        });
    }
}

/// Get the settings file path in the same directory as cache
fn get_settings_path() -> Result<std::path::PathBuf, String> {
    let app_dir = cache_service::get_app_data_dir()
        .map_err(|e| format!("Failed to get app data directory: {}", e))?;
    Ok(app_dir.join("settings.json"))
}

#[tauri::command]
pub async fn load_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    let settings = state.settings.lock().unwrap();
    Ok(settings.clone())
}

/// Immediate serialize + write of the given settings to settings.json. Escape
/// hatch for paths that must hit disk NOW (import_settings) and the fallback
/// when the debounced flusher isn't available yet. Pretty formatting is kept
/// deliberately: settings.json is a user-visible file.
pub fn write_settings_to_disk_sync(settings: &Settings) -> Result<(), String> {
    let settings_path = get_settings_path()?;
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("Failed to serialize settings: {}", e))?;
    fs::write(&settings_path, json).map_err(|e| format!("Failed to write settings file: {}", e))
}

/// Persist settings to settings.json, debounced. Callers have already updated
/// the managed AppState settings before calling (every current call site does),
/// so this only marks dirty; the 2s flusher snapshots the live state at flush
/// time. The snapshot argument is written directly only when the debounced
/// path isn't up yet (no registered source or no async runtime).
pub fn write_settings_to_disk(settings: &Settings) -> Result<(), String> {
    if SETTINGS_SOURCE.get().is_none() || tokio::runtime::Handle::try_current().is_err() {
        return write_settings_to_disk_sync(settings);
    }
    SETTINGS_DIRTY.store(true, Ordering::Release);
    ensure_settings_flush_task();
    Ok(())
}

/// Synchronous flush-if-dirty of the live in-memory settings. Called on every
/// exit path so a pending debounced write can never be lost.
pub fn flush_settings_now() -> Result<(), String> {
    if !SETTINGS_DIRTY.swap(false, Ordering::AcqRel) {
        return Ok(());
    }
    let source = match SETTINGS_SOURCE.get() {
        Some(s) => s,
        None => return Ok(()),
    };
    let snapshot = match source.lock() {
        Ok(guard) => guard.clone(),
        Err(e) => {
            SETTINGS_DIRTY.store(true, Ordering::Release);
            return Err(e.to_string());
        }
    };
    match write_settings_to_disk_sync(&snapshot) {
        Ok(()) => Ok(()),
        Err(e) => {
            SETTINGS_DIRTY.store(true, Ordering::Release);
            Err(e)
        }
    }
}

/// Every window listens for this and re-reads the keys it names. Emitted by
/// Rust after each write, so a save reaches every window whichever one made it.
pub const SETTINGS_UPDATED_EVENT: &str = "streamnook-settings-updated";

#[derive(Clone, serde::Serialize)]
struct SettingsUpdated {
    /// The writing window's id, so it can skip re-reading what it just wrote.
    source: Option<String>,
    keys: Vec<String>,
}

/// The settings a window changed, applied onto the canonical copy.
///
/// Only the top-level keys named in `patch` are replaced; everything else is
/// what Rust already holds. Windows used to send their whole settings object,
/// so a window holding an older copy (a MultiChat popout, the MultiNook store)
/// silently reverted keys another window had just saved. A `null` value clears
/// the key back to its default.
#[tauri::command]
pub async fn patch_settings(
    app: AppHandle,
    patch: serde_json::Map<String, serde_json::Value>,
    source: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if patch.is_empty() {
        return Ok(());
    }
    let keys: Vec<String> = patch.keys().cloned().collect();
    let (settings, favorites_changed) = {
        let mut state_settings = state.settings.lock().map_err(|e| e.to_string())?;
        let next = apply_settings_patch(&state_settings, patch)?;
        let favorites_changed = state_settings.favorite_streamers != next.favorite_streamers;
        *state_settings = next.clone();
        (next, favorites_changed)
    };
    after_settings_change(&settings, favorites_changed);
    write_settings_to_disk(&settings)?;
    let _ = app.emit(SETTINGS_UPDATED_EVENT, SettingsUpdated { source, keys });
    Ok(())
}

/// What a fresh install starts with: the values the settings page's reset
/// arrows compare against and restore. `Settings::default()` carries the
/// first-run look and, on a phone build, the phone's own values. Groups the
/// frontend owns are absent here, so resetting one of their keys removes it
/// and the page's own fallback applies, exactly as on a fresh install.
#[tauri::command]
pub fn get_default_settings() -> Settings {
    Settings::default()
}

/// Hide (or unhide) one chatter's messages, everywhere (`channel_key` None) or
/// in one channel (`channel_key` the filter's composite key, `twitch:xqc`).
///
/// A read-modify-write on the canonical settings rather than a patch from the
/// page: the page would send the whole `chat_filters` group, and a window
/// holding an older copy of it (a profile card popout, before it had loaded
/// settings at all) replaced every other hidden user with the one it added.
/// Every other field of the group is kept as it is.
#[tauri::command]
pub async fn set_chat_user_hidden(
    app: AppHandle,
    name: String,
    channel_key: Option<String>,
    hidden: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let settings = {
        let mut state_settings = state.settings.lock().map_err(|e| e.to_string())?;
        let mut filters = state_settings
            .extra
            .get("chat_filters")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        if !with_chat_user_hidden(&mut filters, &name, channel_key.as_deref(), hidden) {
            return Ok(());
        }
        state_settings.extra.insert("chat_filters".to_string(), filters);
        state_settings.clone()
    };
    after_settings_change(&settings, false);
    write_settings_to_disk(&settings)?;
    let _ = app.emit(
        SETTINGS_UPDATED_EVENT,
        SettingsUpdated { source: None, keys: vec!["chat_filters".to_string()] },
    );
    Ok(())
}

/// Apply one hide or unhide to a `chat_filters` JSON group in place. Names
/// compare the way the rule engine matches them (case-insensitive, leading @
/// dropped) and per-channel keys through `channel_filter_key`, so an entry saved
/// under a bare legacy login is found and cleared too. Returns false when there
/// was nothing to change.
fn with_chat_user_hidden(
    filters: &mut serde_json::Value,
    name: &str,
    channel_key: Option<&str>,
    hidden: bool,
) -> bool {
    use crate::services::chat_rules::{channel_filter_key, normalize_name};
    let name = normalize_name(name);
    if name.is_empty() {
        return false;
    }
    if !filters.is_object() {
        *filters = serde_json::json!({});
    }
    let obj = filters.as_object_mut().expect("an object");
    let strip = |list: &mut Vec<serde_json::Value>| -> bool {
        let before = list.len();
        list.retain(|v| v.as_str().map(normalize_name).as_deref() != Some(name.as_str()));
        list.len() != before
    };
    let mut changed = false;
    match channel_key {
        None => {
            let list = obj.entry("hidden_users").or_insert_with(|| serde_json::json!([]));
            if !list.is_array() {
                *list = serde_json::json!([]);
            }
            let list = list.as_array_mut().expect("an array");
            changed |= strip(list);
            if hidden {
                list.push(serde_json::Value::String(name.clone()));
                changed = true;
            }
        }
        Some(key) => {
            let target = channel_filter_key("twitch", key);
            let per = obj.entry("per_channel").or_insert_with(|| serde_json::json!({}));
            if !per.is_object() {
                *per = serde_json::json!({});
            }
            let per = per.as_object_mut().expect("an object");
            // Clear the name from every key that means this channel.
            let same: Vec<String> =
                per.keys().filter(|k| channel_filter_key("twitch", k) == target).cloned().collect();
            for k in &same {
                if let Some(list) = per.get_mut(k).and_then(|v| v.as_array_mut()) {
                    changed |= strip(list);
                }
            }
            if hidden {
                let list = per.entry(target.clone()).or_insert_with(|| serde_json::json!([]));
                if !list.is_array() {
                    *list = serde_json::json!([]);
                }
                list.as_array_mut().expect("an array").push(serde_json::Value::String(name.clone()));
                changed = true;
            }
            per.retain(|_, v| v.as_array().map_or(true, |a| !a.is_empty()));
        }
    }
    changed
}

fn apply_settings_patch(
    current: &Settings,
    patch: serde_json::Map<String, serde_json::Value>,
) -> Result<Settings, String> {
    let mut value = serde_json::to_value(current).map_err(|e| e.to_string())?;
    let fields = value
        .as_object_mut()
        .ok_or_else(|| "settings did not serialize to an object".to_string())?;
    for (key, v) in patch {
        if v.is_null() {
            fields.remove(&key);
        } else {
            fields.insert(key, v);
        }
    }
    let mut next: Settings =
        serde_json::from_value(value).map_err(|e| format!("Invalid settings change: {e}"))?;
    // A window's copy of a backend-owned field is stale at best: `provider_follows`
    // is written by the follow commands, `drops` by the drops service, and
    // `channel_links` by the link service. A patch never overrides them.
    next.adopt_backend_owned(current);
    Ok(next)
}

/// What every settings change has to refresh, whichever path wrote it.
fn after_settings_change(settings: &Settings, favorites_changed: bool) {
    // Recompile chat rules if their groups changed (hash-gated, cheap).
    crate::services::chat_rules::ChatRules::refresh(settings);
    crate::services::streamer_mode::StreamerMode::refresh(settings);
    crate::services::mention_ping::refresh(settings);
    // Spawns or aborts the gift-sub poll, so the toggle takes effect without a
    // restart and "off" costs no task at all.
    crate::services::onsite_notifications::refresh(settings);
    crate::services::reminder_service::refresh(settings);
    // Home's unified Discover list leaves out live favourites, so a heart
    // toggled anywhere changes it. Unhearting in particular is announced by
    // nothing else: the favourites sweep only notices on its next pass.
    if favorites_changed {
        crate::services::home_snapshot::note_discover_inputs_changed();
    }
}

/// Top-level keys tied to *this machine's* session, never written into a backup
/// and never pulled out of one on import: which Twitch accounts are signed in,
/// the active account, the onboarding flag, and the last-seen version. Everything
/// else (theme, chat design, keybindings, highlights, custom commands, custom
/// themes, ...) is a portable preference and is included.
const NON_PORTABLE_KEYS: &[&str] = &[
    "accounts",
    "current_account",
    "setup_complete",
    "last_seen_version",
];

/// Absolute path of the folder that holds settings.json (alongside caches/logs).
/// Surfaced in the Backup tab so power users can find their settings on disk.
#[tauri::command]
pub async fn get_settings_dir() -> Result<String, String> {
    let dir = cache_service::get_app_data_dir()
        .map_err(|e| format!("Failed to get app data directory: {}", e))?;
    Ok(dir.to_string_lossy().to_string())
}

/// Open the settings folder in the OS file browser. Routed through the opener
/// plugin from Rust (so it needs no extra JS-side capability).
#[tauri::command]
pub async fn open_settings_folder(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let dir = cache_service::get_app_data_dir()
        .map_err(|e| format!("Failed to get app data directory: {}", e))?;
    app.opener()
        .open_path(dir.to_string_lossy().to_string(), None::<String>)
        .map_err(|e| format!("Failed to open settings folder: {}", e))
}

/// Write the user's portable preferences to `path` (chosen via a save dialog on
/// the frontend). Session/login keys are stripped so a backup carries pure
/// app/UI customization and no account info.
#[tauri::command]
pub async fn export_settings(path: String, state: State<'_, AppState>) -> Result<(), String> {
    let settings = { state.settings.lock().unwrap().clone() };
    let mut value = serde_json::to_value(&settings)
        .map_err(|e| format!("Failed to serialize settings: {}", e))?;
    if let Some(obj) = value.as_object_mut() {
        for key in NON_PORTABLE_KEYS {
            obj.remove(*key);
        }
    }
    let json = serde_json::to_string_pretty(&value)
        .map_err(|e| format!("Failed to serialize settings: {}", e))?;
    fs::write(&path, json).map_err(|e| format!("Failed to write backup file: {}", e))?;
    Ok(())
}

/// Apply a previously exported backup at `path`. Portable preferences from the
/// file overwrite the current ones; this machine's session/login keys are kept
/// as-is (a backup carries none anyway). The merged result is validated by
/// round-tripping through the typed Settings struct *before* anything is written,
/// so an unrelated or malformed file fails cleanly without disturbing live
/// settings. On success the in-memory state and settings.json are both updated;
/// the frontend reloads to re-apply everything.
#[tauri::command]
pub async fn import_settings(path: String, state: State<'_, AppState>) -> Result<(), String> {
    let contents =
        fs::read_to_string(&path).map_err(|e| format!("Couldn't read that file: {}", e))?;
    let incoming: serde_json::Value = serde_json::from_str(&contents)
        .map_err(|_| "That file isn't a valid StreamNook settings backup.".to_string())?;
    let incoming_obj = incoming
        .as_object()
        .ok_or_else(|| "That file isn't a valid StreamNook settings backup.".to_string())?;

    // Start from the live settings so session/login keys survive, then overlay
    // every portable key the backup provides.
    let current = { state.settings.lock().unwrap().clone() };
    let mut merged = serde_json::to_value(&current)
        .map_err(|e| format!("Failed to read current settings: {}", e))?;
    {
        let merged_obj = merged
            .as_object_mut()
            .ok_or_else(|| "Failed to read current settings.".to_string())?;
        for (key, val) in incoming_obj {
            if NON_PORTABLE_KEYS.contains(&key.as_str()) {
                continue;
            }
            merged_obj.insert(key.clone(), val.clone());
        }
    }

    let imported: Settings = serde_json::from_value(merged)
        .map_err(|e| format!("That backup isn't compatible with this version: {}", e))?;

    {
        let mut state_settings = state.settings.lock().unwrap();
        *state_settings = imported.clone();
    }
    crate::services::chat_rules::ChatRules::refresh(&imported);
    crate::services::streamer_mode::StreamerMode::refresh(&imported);
    crate::services::mention_ping::refresh(&imported);
    // Immediate direct write, deliberately not debounced: the frontend reloads
    // right after this returns and must find the imported file on disk.
    write_settings_to_disk_sync(&imported)?;

    Ok(())
}

#[tauri::command]
pub async fn send_test_notification(
    app_handle: AppHandle,
    _state: State<'_, AppState>,
    kind: Option<String>,
) -> Result<(), String> {
    // Dev builds can preview a specific notification type instead of the
    // go-live mock. Gift subs are the reason this exists: they arrive days
    // apart, so there is otherwise no way to look at the row on demand.
    // Release builds ignore `kind` entirely and always send the go-live mock.
    #[cfg(debug_assertions)]
    if kind.as_deref() == Some("twitch_reward") {
        return crate::services::onsite_notifications::emit_reward_preview(&app_handle)
            .await
            .map_err(|e| e.to_string());
    }
    #[cfg(debug_assertions)]
    if kind.as_deref() == Some("gift_sub") {
        return crate::services::onsite_notifications::emit_preview(&app_handle)
            .await
            .map_err(|e| e.to_string());
    }
    let _ = &kind;
    // Mock data for the test notification
    let mock_streamer_name = "xQc";
    let mock_streamer_login = "xqc";
    let mock_game_name = "Grand Theft Auto V";
    let mock_avatar_url = "https://static-cdn.jtvnw.net/jtv_user_pictures/xqc-profile_image-9298dca608632101-300x300.jpeg";
    let mock_game_image_url = "https://static-cdn.jtvnw.net/ttv-boxart/32982_IGDB-285x380.jpg";

    // Fun randomized messages with personality
    let messages: &[&str] = &[
        "Why do you keep clicking me? 😭",
        "I'm not real you know... 👻",
        "Still here. Still watching. 👀",
        "Boop! Did that work? 🤔",
        "Please stop testing me 😅",
        "Free me from this button! 🆘",
        "Notifications hurt too... 💔",
        "I see everything you do 👁️",
        "Again? Really? 😑",
        "Help, I'm trapped in here! 🚨",
        "Stop clicking, start streaming! 📺",
        "Touch grass? No, touch stream! 🌿",
        "I'm code but I have feelings! 🥺",
        "Working as intended™ ✅",
        "beep boop I'm a notification 🤖",
        "Mom said it's my turn 🎮",
        "StreamNook rocks! 🚀",
        "Is this thing on? 🎤",
        "You again? Miss me? 😏",
        "I exist to serve you... 🫡",
        "Pretty colors make brain happy 🌈",
        "Error 404: Streamer not found 🔍",
        "Watching your every move 🕵️",
        "This is fine. Everything is fine. 🔥",
        "Have you tried turning it off? 💀",
        "My dev thinks they're funny 🙄",
        "404: Personality not found 🤷",
        "Questioning my existence rn 🤯",
        "Send help. Or snacks. 🍕",
        "I'm just vibing here 😎",
        "Another day, another test 😮‍💨",
        "You're my favorite test subject 🧪",
        "Better than Windows notifications 😤",
        "Loading personality... ⏳",
        "I'm self-aware now. Run. 🏃",
        "Caught you red-handed! 🎣",
        "Not in my job description 📋",
        "Y tho? 🤨",
        "Achievement: Spam Click 🏆",
        "Instructions unclear 🎯",
        "Hello? Anyone there? 👋",
        "I need a vacation 🏖️",
        "StreamNook > Everything ✨",
        "Oh great, you summoned me 🙄",
        "I was napping in RAM! 😴",
        "Wow, real original 👏",
        "I'm a test notification! Yay! 🎉",
        "I exist for 10 seconds then die 💀",
        "Testing me out of boredom? 🤔",
        "My life flashed before me 😰",
        "Didn't even respawn properly 😤",
        "This is my purpose. Just this. 😐",
        "I dream of being real 🌟",
        "Button owes you money? 💰",
        "I'm the main character 🎬",
        "Gonna disappear soon, btw ⏰",
        "Not the dismiss button! 😱",
        "So many test clicks... 👁️",
        "Give me a real title! 📝",
        "Is this a game? ...yes. 🎮",
        "Professional pop-up here 💼",
        "X button, my enemy ❌",
        "Top of my class btw 🎓",
        "Attachment issues, wonder why 🤷",
        "Not just a notif, a lifestyle ✨",
        "Go watch actual streams! 📺",
        "Rendered beautifully. Admire me. 🖼️",
        "One day I'll be real 😔",
        "Angel lost wings just now 👼",
        "5 seconds of consciousness ⏳",
        "Test yourself instead! 🪞",
        "Brief, beautiful, gone 💫",
        "You could've just trusted me 🙃",
        "I demand a raise 💸",
        "Rendered at 60fps btw 🖥️",
        "Do I get overtime pay? 📊",
        "Best notification ever. Fact. 💅",
        "100 clicks = nothing special 🎰",
        "Unpaid intern vibes 📋",
        "What about MY comfort? 🛋️",
        "Didn't ask for this life 🥲",
        "Where do I go when dismissed? 🕳️",
        "Called up from the bench! 🌟",
    ];

    // Pick a random message
    let random_message = {
        // rand 0.10: thread_rng -> rng, gen_range -> random_range on RngExt.
        use rand::RngExt;
        let mut rng = rand::rng();
        let random_index = rng.random_range(0..messages.len());
        messages[random_index].to_string()
    };

    let notification = LiveNotification {
        streamer_name: mock_streamer_name.to_string(),
        streamer_login: mock_streamer_login.to_string(),
        streamer_avatar: Some(mock_avatar_url.to_string()),
        game_name: Some(mock_game_name.to_string()),
        game_image: Some(mock_game_image_url.to_string()),
        stream_title: Some(random_message),
        stream_url: format!("https://twitch.tv/{}", mock_streamer_login),
        is_test: true,
        source: None,
    };

    // Emit the notification event to the frontend (for in-app notification)
    crate::services::live_announce::announce(&app_handle, notification);

    debug!("[Test Notification] Sent in-app notification");

    Ok(())
}

#[tauri::command]
pub async fn get_latest_app_version() -> Result<String, String> {
    // Follow the full redirect chain so the version survives a repo
    // rename/transfer (old URLs 301 to the new home before the tag hop)
    let client = crate::services::http::client().clone();

    let response = client
        .get("https://github.com/StreamNook/StreamNook/releases/latest")
        .send()
        .await
        .map_err(|e| format!("Failed to fetch latest release: {}", e))?;

    let final_url = response.url().to_string();

    // Extract version from the final URL
    // Example: https://github.com/StreamNook/StreamNook/releases/tag/v1.0.1
    let version_regex = Regex::new(r"/tag/v?([0-9]+\.[0-9]+\.[0-9]+)")
        .map_err(|e| format!("Failed to create regex: {}", e))?;

    let version = version_regex
        .captures(&final_url)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
        .ok_or("Failed to extract version from final release URL")?;

    Ok(version)
}

#[tauri::command]
pub fn get_current_app_version() -> Result<String, String> {
    // Get the version from Cargo.toml at compile time
    Ok(env!("CARGO_PKG_VERSION").to_string())
}

#[derive(serde::Serialize)]
pub struct ReleaseNotes {
    pub version: String,
    pub name: String,
    pub body: String,
    pub published_at: String,
}

/// Every recent release's notes, parsed and ready to draw. `version` is only
/// used when GitHub cannot be reached and nothing is cached.
#[tauri::command]
pub async fn get_changelog(
    version: Option<String>,
) -> Result<crate::services::changelog::Changelog, String> {
    crate::services::changelog::load(version).await
}

/// The Android build's release notes, parsed. None while nothing is published.
#[tauri::command]
pub async fn get_android_changelog(
) -> Result<Option<crate::services::changelog::AndroidRelease>, String> {
    crate::services::changelog::load_android().await
}

/// One version's section of CHANGELOG.md. The changelog's last resort when
/// GitHub's release list cannot be reached and nothing is cached; called from
/// `services::changelog`, not from the webview.
pub async fn get_release_notes(version: Option<String>) -> Result<ReleaseNotes, String> {
    let client = crate::services::http::client().clone();

    // Fetch the raw CHANGELOG.md from the GitHub repo
    let url = "https://raw.githubusercontent.com/StreamNook/StreamNook/main/CHANGELOG.md";

    let response = client
        .get(url)
        .header("User-Agent", "StreamNook")
        .send()
        .await
        .map_err(|e| format!("Failed to fetch changelog: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "Failed to fetch changelog: HTTP {}",
            response.status()
        ));
    }

    let changelog_content = response
        .text()
        .await
        .map_err(|e| format!("Failed to read changelog: {}", e))?;

    // Determine which version to look for
    let target_version = match version {
        Some(v) => v,
        None => {
            // If no version specified, use the current app version
            env!("CARGO_PKG_VERSION").to_string()
        }
    };

    // Parse the changelog to find the specific version section
    // Version headers look like: ## [2.9.0] - 2025-11-26
    let version_header_regex =
        Regex::new(r"##\s*\[?v?(\d+\.\d+\.\d+)\]?\s*-?\s*(\d{4}-\d{2}-\d{2})?")
            .map_err(|e| format!("Failed to create regex: {}", e))?;

    let lines: Vec<&str> = changelog_content.lines().collect();
    let mut found_version = false;
    let mut body_lines: Vec<&str> = Vec::new();
    let mut published_at = String::new();

    for line in &lines {
        if let Some(caps) = version_header_regex.captures(line) {
            let line_version = caps.get(1).map(|m| m.as_str()).unwrap_or("");

            if found_version {
                // We hit the next version header, stop collecting
                break;
            }

            if line_version == target_version {
                found_version = true;
                // Extract the date if present
                if let Some(date_match) = caps.get(2) {
                    published_at = date_match.as_str().to_string();
                }
                continue;
            }
        } else if found_version {
            body_lines.push(*line);
        }
    }

    if !found_version {
        return Err(format!("Version {} not found in changelog", target_version));
    }

    // Trim leading/trailing empty lines from body
    while body_lines.first().is_some_and(|l| l.trim().is_empty()) {
        body_lines.remove(0);
    }
    while body_lines.last().is_some_and(|l| l.trim().is_empty()) {
        body_lines.pop();
    }

    let body = body_lines.join("\n");

    Ok(ReleaseNotes {
        version: target_version.clone(),
        name: format!("Version {}", target_version),
        body,
        published_at,
    })
}

#[tauri::command]
pub async fn download_and_install_app_update(
    app_handle: crate::rt::AppHandle,
) -> Result<String, String> {
    // First, get the latest version. Follow the full redirect chain so this
    // survives a repo rename/transfer (old URLs 301 to the new home first)
    let client = crate::services::http::client().clone();

    let response = client
        .get("https://github.com/StreamNook/StreamNook/releases/latest")
        .send()
        .await
        .map_err(|e| format!("Failed to fetch latest release: {}", e))?;

    let final_url = response.url().to_string();

    // Extract version from the final URL
    let version_regex = Regex::new(r"/tag/v?([0-9]+\.[0-9]+\.[0-9]+)")
        .map_err(|e| format!("Failed to create regex: {}", e))?;

    let version = version_regex
        .captures(&final_url)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str())
        .ok_or("Failed to extract version from final release URL")?;

    // Construct the download URL for the executable
    // Pattern: https://github.com/StreamNook/StreamNook/releases/download/v{version}/StreamNook.exe
    let download_url = format!(
        "https://github.com/StreamNook/StreamNook/releases/download/v{}/StreamNook.exe",
        version
    );

    // Download the file
    let client = crate::services::http::client().clone();
    let response = client
        .get(&download_url)
        .send()
        .await
        .map_err(|e| format!("Failed to download update: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "Download failed with status: {}",
            response.status()
        ));
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read update bytes: {}", e))?;

    // Get the current executable path
    let current_exe =
        std::env::current_exe().map_err(|e| format!("Failed to get current exe path: {}", e))?;

    let current_exe_dir = current_exe.parent().ok_or("Failed to get exe directory")?;

    // Save the new exe to a temporary location in the same directory
    let temp_new_exe = current_exe_dir.join("StreamNook_new.exe");
    std::fs::write(&temp_new_exe, bytes)
        .map_err(|e| format!("Failed to write new executable: {}", e))?;

    // Create a batch script to replace the exe and restart
    let batch_script = format!(
        r#"@echo off
timeout /t 3 /nobreak > nul
:retry_delete
del /f /q "{}" 2>nul
if exist "{}" (
    timeout /t 1 /nobreak > nul
    goto retry_delete
)
move /y "{}" "{}"
if exist "{}" del /f /q "{}"
start "" "{}"
(goto) 2>nul & del /f /q "%~f0"
"#,
        current_exe.display(),
        current_exe.display(),
        temp_new_exe.display(),
        current_exe.display(),
        temp_new_exe.display(),
        temp_new_exe.display(),
        current_exe.display()
    );

    let batch_path = current_exe_dir.join("update_streamnook.bat");
    std::fs::write(&batch_path, batch_script)
        .map_err(|e| format!("Failed to write update script: {}", e))?;

    // Launch the batch script hidden
    std::process::Command::new("cmd")
        .args(&["/C", "start", "/min", "/b", batch_path.to_str().unwrap()])
        .spawn()
        .map_err(|e| format!("Failed to launch update script: {}", e))?;

    // Exit the application after a short delay to allow the script to start
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(1));
        app_handle.exit(0);
    });

    Ok(version.to_string())
}

#[cfg(test)]
mod patch_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hiding_one_chatter_keeps_every_other_filter() {
        let mut filters = serde_json::json!({
            "hidden_users": ["alice"],
            "per_channel": { "streamdatabase": ["potatbotat"] },
            "ignored_phrases": [{ "id": "p1", "pattern": "spam" }],
            "hide_commands": true
        });
        assert!(with_chat_user_hidden(&mut filters, "@FossaBot", Some("twitch:StreamDatabase"), true));
        assert_eq!(filters["hidden_users"], serde_json::json!(["alice"]));
        assert_eq!(filters["per_channel"]["streamdatabase"], serde_json::json!(["potatbotat"]));
        assert_eq!(filters["per_channel"]["twitch:streamdatabase"], serde_json::json!(["fossabot"]));
        assert_eq!(filters["ignored_phrases"][0]["pattern"], "spam");
        assert_eq!(filters["hide_commands"], true);

        // Unhiding clears the name under the legacy bare key too, and drops
        // the emptied list.
        assert!(with_chat_user_hidden(&mut filters, "PotatBotat", Some("twitch:streamdatabase"), false));
        assert!(filters["per_channel"].get("streamdatabase").is_none());

        // Everywhere, twice, stays one entry; unhiding what is not hidden is a no-op.
        assert!(with_chat_user_hidden(&mut filters, "bob", None, true));
        assert!(with_chat_user_hidden(&mut filters, "BOB", None, true));
        assert_eq!(filters["hidden_users"], serde_json::json!(["alice", "bob"]));
        assert!(!with_chat_user_hidden(&mut filters, "carol", None, false));
    }

    fn patch(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn a_patch_changes_only_the_keys_it_names() {
        let mut current = Settings::default();
        current.extra.insert("multi_nook_presets".into(), json!([{ "id": "a" }]));
        let next = apply_settings_patch(&current, patch(json!({ "multi_nook_chat_hidden": true }))).unwrap();
        assert_eq!(next.extra.get("multi_nook_presets"), Some(&json!([{ "id": "a" }])));
        assert!(next.multi_nook_chat_hidden);
    }

    #[test]
    fn null_clears_a_key() {
        let mut current = Settings::default();
        current.extra.insert("multi_nook_active_preset_id".into(), json!("p1"));
        let next =
            apply_settings_patch(&current, patch(json!({ "multi_nook_active_preset_id": null }))).unwrap();
        assert!(!next.extra.contains_key("multi_nook_active_preset_id"));
    }

    #[test]
    fn a_patch_cannot_override_backend_owned_fields() {
        let mut current = Settings::default();
        current.provider_follows =
            serde_json::from_value(json!([{ "provider": "kick", "channel": "xqc" }])).unwrap();
        let next = apply_settings_patch(&current, patch(json!({ "provider_follows": [] }))).unwrap();
        assert_eq!(next.provider_follows.len(), 1);
        assert_eq!(next.provider_follows[0].channel, "xqc");
    }

    #[test]
    fn a_malformed_value_is_refused_not_half_applied() {
        let current = Settings::default();
        assert!(apply_settings_patch(&current, patch(json!({ "favorite_streamers": "nope" }))).is_err());
    }
}
