//! Which Atmosphere (or Cologne look) each StreamNook member shows, for every
//! window and both phone shells.
//!
//! A member picks a profile theme (`user_profile_prefs.profile_theme`: `tier`,
//! `paint`, an Atmosphere id, or the Cologne id with `+coin` / `+border`) and
//! whether it also paints behind their chat messages (`chat_atmosphere`). This
//! resolves that once per MEMBER, refuses an Atmosphere the member may not wear
//! (`cosmetics_registry::may_wear_atmosphere`), follows live changes from the
//! realtime feed, and keeps the viewer's own accounts on disk so their own rows
//! paint on the first frame of a cold launch. Chat surfaces fan a member's theme
//! out to whichever chat keys they occupy.

use crate::services::cosmetics_registry as registry;
use crate::services::streamnook_backend as backend;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Emitted with a `MemberTheme` whenever a known member's theme changes.
pub const EVENT: &str = "member-theme-changed";

/// A cache hit is served at once; past this it is re-read in the background, a
/// backstop for a live change the socket missed.
const RESOLVE_TTL: Duration = Duration::from_secs(120);
const CACHE_CAP: usize = 4000;
const OWN_FILE: &str = "own_member_themes.json";

/// A member's profile preferences, as the pages read them.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProfilePrefs {
    /// True when the theme is the 7TV paint (derived; kept for older readers).
    pub paint_theme: bool,
    pub profile_theme: String,
    /// Profile sections the member hides from others.
    pub hidden_sections: Vec<String>,
    /// Whether their Atmosphere also paints behind their chat messages.
    pub chat_atmosphere: bool,
}

impl Default for ProfilePrefs {
    fn default() -> Self {
        Self { paint_theme: false, profile_theme: "tier".into(), hidden_sections: Vec::new(), chat_atmosphere: true }
    }
}

/// The Cologne look's add-ons, read off its theme id.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cologne {
    pub coin: bool,
    pub frame: bool,
}

pub fn parse_cologne(theme: Option<&str>) -> Option<Cologne> {
    let theme = theme?;
    theme
        .starts_with(registry::COLOGNE_THEME_ID)
        .then(|| Cologne { coin: theme.contains("+coin"), frame: theme.contains("+border") })
}

/// What a member shows, resolved.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MemberTheme {
    pub member_id: String,
    /// The raw `profile_theme`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// The Atmosphere that paints, after the wear gate; None for `tier`, `paint`,
    /// Cologne and refused picks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub atmosphere_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cologne: Option<Cologne>,
    /// The member keeps the theme off their chat rows.
    pub hidden_in_chat: bool,
}

impl MemberTheme {
    /// The Atmosphere a chat row draws for this member, if any.
    pub fn chat_atmosphere(&self) -> Option<&str> {
        (!self.hidden_in_chat).then_some(self.atmosphere_id.as_deref()).flatten()
    }

    pub fn chat_cologne(&self) -> Option<Cologne> {
        (!self.hidden_in_chat).then_some(self.cologne).flatten()
    }
}

struct Entry {
    theme: MemberTheme,
    at: Instant,
}

#[derive(Default)]
struct State {
    cache: HashMap<String, Entry>,
    in_flight: HashSet<String>,
    own: HashSet<String>,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();

fn with<T>(f: impl FnOnce(&mut State) -> T) -> T {
    let mut guard = STATE.get_or_init(|| Mutex::new(State::default())).lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

/// Works a raw theme into what paints: Cologne apart, an Atmosphere only when the
/// catalog knows it and the member may wear it.
pub fn resolve_theme(member_id: &str, theme: Option<&str>, hidden_in_chat: bool, own: bool) -> MemberTheme {
    let cologne = parse_cologne(theme);
    let atmosphere_id = if cologne.is_some() {
        None
    } else {
        theme
            .and_then(registry::atmosphere)
            .map(|a| a.id)
            .filter(|id| registry::may_wear_atmosphere(member_id, Some(id), own))
    };
    MemberTheme { member_id: member_id.to_string(), theme: theme.map(String::from), atmosphere_id, cologne, hidden_in_chat }
}

fn emit(theme: &MemberTheme) {
    if let Some(app) = crate::services::providers::app_handle() {
        use tauri::Emitter;
        let _ = app.emit(EVENT, theme);
    }
}

/// Files a resolved theme; emits when it differs from what was known. Returns
/// whether it changed.
fn store(theme: MemberTheme) -> bool {
    let member = theme.member_id.clone();
    let (changed, own) = with(|s| {
        if s.cache.len() >= CACHE_CAP && !s.cache.contains_key(&member) {
            let oldest = s.cache.iter().filter(|(k, _)| !s.own.contains(*k)).min_by_key(|(_, e)| e.at).map(|(k, _)| k.clone());
            if let Some(oldest) = oldest {
                s.cache.remove(&oldest);
            }
        }
        let changed = s.cache.get(&member).map(|e| e.theme != theme).unwrap_or(true);
        s.cache.insert(member.clone(), Entry { theme: theme.clone(), at: Instant::now() });
        (changed, s.own.contains(&member))
    });
    if changed {
        if own {
            persist_own();
        }
        emit(&theme);
        crate::services::chat_row_view::post_member_frame(&theme);
    }
    changed
}

/// A member's theme when it is known, without asking the network.
pub fn peek(member_id: &str) -> Option<MemberTheme> {
    with(|s| s.cache.get(member_id).map(|e| e.theme.clone()))
}

/// A member's theme: the cached answer at once (re-read in the background past
/// the TTL), else one read of their prefs. Non-members have none. Concurrent
/// asks for one member share a read.
pub async fn resolve(member_id: &str) -> Option<MemberTheme> {
    if member_id.is_empty() || !registry::is_member(member_id) {
        return None;
    }
    let (cached, stale) = with(|s| match s.cache.get(member_id) {
        Some(e) => (Some(e.theme.clone()), e.at.elapsed() >= RESOLVE_TTL),
        None => (None, true),
    });
    if let Some(theme) = cached {
        if stale {
            let member = member_id.to_string();
            tauri::async_runtime::spawn(async move {
                fetch(&member).await;
            });
        }
        return Some(theme);
    }
    fetch(member_id).await;
    peek(member_id)
}

async fn fetch(member_id: &str) {
    if !with(|s| s.in_flight.insert(member_id.to_string())) {
        return;
    }
    let prefs = read_prefs(member_id).await;
    registry::ready(Duration::from_secs(5)).await;
    if let Ok(prefs) = prefs {
        let own = is_own(member_id);
        store(resolve_theme(member_id, Some(&prefs.profile_theme), !prefs.chat_atmosphere, own));
    }
    with(|s| s.in_flight.remove(member_id));
}

fn is_own(member_id: &str) -> bool {
    with(|s| s.own.contains(member_id))
}

async fn read_prefs(member_id: &str) -> Result<ProfilePrefs, String> {
    let rows = backend::select(
        "user_profile_prefs",
        &format!("select=paint_theme,profile_theme,hidden_sections,chat_atmosphere&twitch_user_id=eq.{}", encode(member_id)),
    )
    .await?;
    Ok(prefs_from_row(rows.first()))
}

/// The row's values with the pages' defaults; the legacy `paint_theme` flag
/// stands in for a missing `profile_theme`.
pub(crate) fn prefs_from_row(row: Option<&Value>) -> ProfilePrefs {
    let Some(row) = row else { return ProfilePrefs::default() };
    let profile_theme = row
        .get("profile_theme")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(String::from)
        .unwrap_or_else(|| if row.get("paint_theme").and_then(Value::as_bool) == Some(true) { "paint" } else { "tier" }.into());
    ProfilePrefs {
        paint_theme: profile_theme == "paint",
        hidden_sections: row
            .get("hidden_sections")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        chat_atmosphere: row.get("chat_atmosphere").and_then(Value::as_bool) != Some(false),
        profile_theme,
    }
}

/// A member's profile preferences, a failed read as an error: what they hide
/// must never show because a read blinked.
pub async fn try_profile_prefs(member_id: &str) -> Result<ProfilePrefs, String> {
    if member_id.is_empty() {
        return Ok(ProfilePrefs::default());
    }
    read_prefs(member_id).await
}

/// A member's profile preferences; the defaults when none are saved or the
/// read fails (a failure is logged).
pub async fn profile_prefs(member_id: &str) -> ProfilePrefs {
    if member_id.is_empty() {
        return ProfilePrefs::default();
    }
    read_prefs(member_id).await.unwrap_or_else(|e| {
        log::warn!("[MemberTheme] profile prefs read failed: {e}");
        ProfilePrefs::default()
    })
}

fn encode(id: &str) -> String {
    id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect()
}

// ---- Own accounts ----------------------------------------------------------------

#[derive(Serialize, Deserialize, Default)]
struct OwnFile {
    #[serde(default)]
    themes: HashMap<String, MemberTheme>,
}

fn own_path() -> Option<PathBuf> {
    crate::services::cache_service::get_app_data_dir().ok().map(|d| d.join(OWN_FILE))
}

fn persist_own() {
    let themes: HashMap<String, MemberTheme> =
        with(|s| s.own.iter().filter_map(|id| s.cache.get(id).map(|e| (id.clone(), e.theme.clone()))).collect());
    if let Some(path) = own_path() {
        if let Ok(body) = serde_json::to_vec(&OwnFile { themes }) {
            let _ = std::fs::write(path, body);
        }
    }
}

/// Marks the viewer's own accounts (primary and linked) and seeds each from the
/// copy saved last session, so their own rows paint before any read lands. Their
/// picks skip the wear gate: the server gated the write.
pub fn register_own(member_ids: &[String]) {
    let saved: OwnFile = own_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    with(|s| {
        for id in member_ids.iter().filter(|id| !id.is_empty()) {
            s.own.insert(id.clone());
            if let Some(theme) = saved.themes.get(id) {
                s.cache.entry(id.clone()).or_insert_with(|| Entry { theme: theme.clone(), at: Instant::now() - RESOLVE_TTL });
            }
        }
    });
}

// ---- Writes ------------------------------------------------------------------

/// Sets a member's profile theme. Their theme flips at once for every surface
/// and goes back if the write fails.
pub async fn set_profile_theme(member_id: &str, theme: &str) -> Result<(), String> {
    if member_id.is_empty() {
        return Err("no member".into());
    }
    let previous = peek(member_id);
    let hidden = previous.as_ref().map(|t| t.hidden_in_chat).unwrap_or(false);
    store(resolve_theme(member_id, Some(theme), hidden, true));
    let result = write_theme(member_id, theme).await;
    if result.is_err() {
        if let Some(previous) = previous {
            store(previous);
        }
    }
    result
}

async fn write_theme(member_id: &str, theme: &str) -> Result<(), String> {
    if crate::services::client_config::get().await.write_via_api {
        let body = json!({ "theme": theme });
        match crate::commands::streamnook_api::post_json("/api/cosmetics/theme", &body, None).await {
            Ok(r) if r.ok => return Ok(()),
            Ok(r) => return Err(format!("{} {}", r.status, r.body.chars().take(200).collect::<String>())),
            Err(e) => log::warn!("[MemberTheme] /api/cosmetics/theme unavailable, falling back: {e}"),
        }
    }
    backend::upsert(
        "user_profile_prefs",
        "twitch_user_id",
        json!({
            "twitch_user_id": member_id,
            "profile_theme": theme,
            "paint_theme": theme == "paint",
            "updated_at": chrono::Utc::now().to_rfc3339(),
        }),
    )
    .await
}

/// Shows or hides a member's theme on their chat rows. Through the API, with
/// the direct write only when the API cannot be reached. Returns whether it was
/// saved; the rows flip at once and flip back when it was not.
pub async fn set_chat_atmosphere(member_id: &str, show: bool) -> bool {
    if member_id.is_empty() {
        return false;
    }
    let previous = peek(member_id);
    let flipped = match previous.clone() {
        Some(t) => MemberTheme { hidden_in_chat: !show, ..t },
        None => MemberTheme { member_id: member_id.to_string(), hidden_in_chat: !show, ..Default::default() },
    };
    store(flipped);
    let body = json!({ "chat_atmosphere": show });
    let saved = match crate::commands::streamnook_api::post_json("/api/v1/profile/prefs", &body, None).await {
        Ok(r) => r.ok,
        Err(e) => {
            log::warn!("[MemberTheme] /api/v1/profile/prefs unavailable, falling back: {e}");
            backend::upsert(
                "user_profile_prefs",
                "twitch_user_id",
                json!({ "twitch_user_id": member_id, "chat_atmosphere": show, "updated_at": chrono::Utc::now().to_rfc3339() }),
            )
            .await
            .is_ok()
        }
    };
    if !saved {
        match previous {
            Some(previous) => {
                store(previous);
            }
            None => {
                store(MemberTheme { member_id: member_id.to_string(), hidden_in_chat: show, ..Default::default() });
            }
        }
    }
    saved
}

// ---- Live changes ------------------------------------------------------------

/// Follows the realtime feed: a member's prefs row changing repaints them where
/// they are known. (A catalog change re-gates through `regate`.)
pub fn start() {
    static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) || !backend::configured() {
        return;
    }
    let mut changes = backend::subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match changes.recv().await {
                Ok(backend::Change::Row { table, record, .. }) if table == "user_profile_prefs" => apply_row(&record),
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

fn apply_row(record: &Value) {
    let Some(member) = record.get("twitch_user_id").and_then(Value::as_str) else { return };
    let Some(known) = peek(member) else { return };
    let theme = record.get("profile_theme").and_then(Value::as_str).map(String::from).or(known.theme.clone());
    let hidden = match record.get("chat_atmosphere").and_then(Value::as_bool) {
        Some(show) => !show,
        None => known.hidden_in_chat,
    };
    store(resolve_theme(member, theme.as_deref(), hidden, is_own(member)));
}

/// Re-applies the wear gate to every known member, after the catalog or what
/// members own changed. Called by the registry on every change.
pub fn regate() {
    let known: Vec<(MemberTheme, bool)> =
        with(|s| s.cache.values().map(|e| (e.theme.clone(), s.own.contains(&e.theme.member_id))).collect());
    for (theme, own) in known {
        let next = resolve_theme(&theme.member_id, theme.theme.as_deref(), theme.hidden_in_chat, own);
        if next != theme {
            store(next);
        }
    }
}

#[cfg(test)]
pub(crate) fn clear_for_test() {
    with(|s| *s = State::default());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::cosmetics_registry::{entry_for_test, set_for_test, TEST_LOCK};

    fn fixture() {
        let aurora = registry::atmosphere_from_row(&json!({
            "id": "aurora", "name": "Aurora", "accent": "45, 212, 191", "swatch": "#10b981",
            "base_color": "#04070c", "motion": "aurora", "chat_edge": "none", "unlock_kind": "subscriber"
        }))
        .unwrap();
        let cologne = registry::atmosphere_from_row(&json!({
            "id": "cs2-major-cologne", "name": "Cologne", "accent": "214, 177, 92", "swatch": "#0a1322",
            "base_color": "#0a1322", "motion": "drift", "chat_edge": "none", "unlock_kind": "accolade",
            "unlock_accolade_id": "cs2_major_cologne_2026", "kind": "cologne-chrome"
        }))
        .unwrap();
        set_for_test(
            &[("10", 1), ("20", 2)],
            vec![
                entry_for_test(registry::DEFAULT_SLUG, "badge", "streamnook-logo.png", true),
                entry_for_test(registry::SUBSCRIBER_SLUG, "badge", "x.webp", false),
            ],
            &[("10", registry::SUBSCRIBER_SLUG)],
            &[],
            vec![aurora, cologne],
        );
        clear_for_test();
    }

    #[test]
    fn prefs_default_and_derive_the_legacy_paint_flag() {
        assert_eq!(prefs_from_row(None), ProfilePrefs::default());
        let legacy = prefs_from_row(Some(&json!({ "paint_theme": true })));
        assert_eq!(legacy.profile_theme, "paint");
        assert!(legacy.paint_theme);
        let row = prefs_from_row(Some(&json!({ "profile_theme": "aurora", "chat_atmosphere": false, "hidden_sections": ["roast"] })));
        assert_eq!(row.profile_theme, "aurora");
        assert!(!row.chat_atmosphere);
        assert_eq!(row.hidden_sections, vec!["roast".to_string()]);
    }

    #[test]
    fn a_theme_resolves_through_the_gate_and_cologne_apart() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fixture();
        let sub = resolve_theme("10", Some("aurora"), false, false);
        assert_eq!(sub.chat_atmosphere(), Some("aurora"));
        // Owns nothing that unlocks it: refused on a viewer's screen.
        assert_eq!(resolve_theme("20", Some("aurora"), false, false).atmosphere_id, None);
        // The viewer's own pick always paints.
        assert_eq!(resolve_theme("20", Some("aurora"), false, true).atmosphere_id.as_deref(), Some("aurora"));
        let cologne = resolve_theme("20", Some("cs2-major-cologne+coin"), false, false);
        assert_eq!(cologne.atmosphere_id, None);
        assert_eq!(cologne.cologne, Some(Cologne { coin: true, frame: false }));
        assert_eq!(resolve_theme("10", Some("tier"), false, false).atmosphere_id, None);
        // Hidden from chat: kept for the profile, not drawn on rows.
        let hidden = resolve_theme("10", Some("aurora"), true, false);
        assert_eq!(hidden.atmosphere_id.as_deref(), Some("aurora"));
        assert_eq!(hidden.chat_atmosphere(), None);
    }

    #[test]
    fn a_live_prefs_row_repaints_only_known_members() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        fixture();
        apply_row(&json!({ "twitch_user_id": "10", "profile_theme": "aurora" }));
        assert!(peek("10").is_none());
        store(resolve_theme("10", Some("tier"), false, false));
        apply_row(&json!({ "twitch_user_id": "10", "profile_theme": "aurora", "chat_atmosphere": false }));
        let now = peek("10").unwrap();
        assert_eq!(now.atmosphere_id.as_deref(), Some("aurora"));
        assert!(now.hidden_in_chat);
    }
}
