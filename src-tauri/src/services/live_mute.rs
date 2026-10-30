//! Per-channel go-live alert mutes: what an entry in
//! `live_notifications.muted_live_channels` is, which go-lives it silences, and
//! Copy from Twitch (one read of the viewer's Twitch notification bells).
//!
//! A Twitch channel is stored as its bare lowercase login, which is what the
//! phones' push registration and background polls compare. Any other platform
//! is stored as `provider:channel` (`providers::key::make_key`, so a YouTube id
//! keeps its case). A Twitch login never holds `:` and every other entry starts
//! with a provider prefix, so the two shapes never meet in one comparison.
//! Reads compare without case, so a lowercased older entry still matches.

use crate::models::provider_stream::ProviderStream;
use crate::services::providers::key::{channel_ids, favorite_id, is_provider_id, make_key, parse_key};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The channel a go-live is about, as the bell and the alert's "Turn off
/// alerts" action name it to `set_live_alert`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertChannel {
    pub provider: String,
    #[serde(default)]
    pub user_id: String,
    pub user_login: String,
}

fn is_twitch(provider: &str) -> bool {
    provider.is_empty() || provider == "twitch"
}

/// The one key a Twitch channel is muted under: its bare lowercase login.
pub fn keys_for_twitch(login: &str) -> Vec<String> {
    let login = login.trim().to_lowercase();
    if login.is_empty() {
        Vec::new()
    } else {
        vec![login]
    }
}

/// Every key a mute of this channel may be stored under. Twitch: the login
/// (never the numeric id). Elsewhere: every identity the channel carries
/// (`channel_ids`), so a YouTube channel muted by its UC id is still muted
/// when a go-live names it by its `@handle`.
pub fn keys_for_channel(provider: &str, user_id: &str, user_login: &str) -> Vec<String> {
    if is_twitch(provider) {
        return keys_for_twitch(user_login);
    }
    channel_ids(provider, user_id, user_login)
}

/// The keys of a live row (the favourites sweep, the other platforms'
/// watcher): the channel's identities plus the row's own key, which is the
/// favourite or follow it was swept from.
pub fn keys_for_row(row: &ProviderStream) -> Vec<String> {
    if is_twitch(&row.provider) {
        return keys_for_twitch(&row.user_login);
    }
    let mut keys = channel_ids(&row.provider, &row.user_id, &row.user_login);
    if !row.key.is_empty() && !keys.iter().any(|k| k.eq_ignore_ascii_case(&row.key)) {
        keys.push(row.key.clone());
    }
    keys
}

/// The one key a mute writes: the Twitch login, or the channel's favourite
/// id elsewhere. `None` when the channel has no stable identity (a YouTube row
/// known only by a video id): nothing can mute it by name, so no bell shows.
pub fn canonical(provider: &str, user_id: &str, user_login: &str) -> Option<String> {
    if is_twitch(provider) {
        return keys_for_twitch(user_login).into_iter().next();
    }
    favorite_id(provider, user_id, user_login)
}

/// An entry as stored, from whatever a shell sent: a bare or `twitch:`
/// login becomes the bare lowercase login, anything else `provider:channel`.
pub fn normalize_entry(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    // A provider prefix in another case (`Kick:`, `YouTube:`) still names it.
    let raw = match raw.split_once(':') {
        Some((prefix, rest)) if is_provider_id(&prefix.trim().to_lowercase()) => {
            format!("{}:{}", prefix.trim().to_lowercase(), rest.trim())
        }
        _ => raw.to_string(),
    };
    let parsed = parse_key(&raw);
    if parsed.provider == "twitch" {
        return keys_for_twitch(&parsed.channel).into_iter().next();
    }
    if parsed.channel.trim().is_empty() {
        return None;
    }
    Some(make_key(&parsed.provider, &parsed.channel))
}

/// Whether any stored entry names any of `keys`, ignoring case.
pub fn is_muted(list: &[String], keys: &[String]) -> bool {
    list.iter().any(|entry| keys.iter().any(|k| k.eq_ignore_ascii_case(entry.trim())))
}

/// The list after setting one channel's alerts to `on`, or `None` when it is
/// already that way. On takes out every entry naming the channel; off adds
/// `canonical` once. Never a toggle: a screen that is behind sets the state it
/// shows instead of flipping it the wrong way.
pub fn set_in(list: &[String], keys: &[String], canonical: Option<&str>, on: bool) -> Option<Vec<String>> {
    if on {
        let next: Vec<String> = list
            .iter()
            .filter(|entry| !keys.iter().any(|k| k.eq_ignore_ascii_case(entry.trim())))
            .cloned()
            .collect();
        (next.len() != list.len()).then_some(next)
    } else {
        if is_muted(list, keys) {
            return None;
        }
        let canonical = canonical?;
        let mut next = list.to_vec();
        next.push(canonical.to_string());
        Some(next)
    }
}

/// The identity and mute keys of a Twitch go-live from the follow poller.
pub fn twitch_alert(user_id: &str, user_login: &str) -> (Option<AlertChannel>, Vec<String>) {
    let keys = keys_for_twitch(user_login);
    let channel = keys.first().map(|login| AlertChannel {
        provider: "twitch".to_string(),
        user_id: user_id.to_string(),
        user_login: login.clone(),
    });
    (channel, keys)
}

/// The identity and mute keys of a go-live raised from a live row. The
/// identity is left out when the channel cannot be muted by name.
pub fn row_alert(row: &ProviderStream) -> (Option<AlertChannel>, Vec<String>) {
    let keys = keys_for_row(row);
    let provider = if row.provider.is_empty() { "twitch".to_string() } else { row.provider.clone() };
    let channel = canonical(&provider, &row.user_id, &row.user_login).map(|_| AlertChannel {
        provider: provider.clone(),
        user_id: row.user_id.clone(),
        user_login: row.user_login.clone(),
    });
    (channel, keys)
}

// --- Copy from Twitch -------------------------------------------------------

/// What a copy or an undo answers when it cannot. Sent to the shells as the
/// plain rejection string (`"signed-out"`, ...).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CopyError {
    SignedOut,
    WrongAccount,
    TwitchError,
    Busy,
    Stale,
    SaveFailed,
}

/// One copy's result, for the result line and its Undo.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CopyAnswer {
    pub copy_id: u64,
    /// The logins this copy muted, in follow order (what Undo takes out).
    pub added: Vec<String>,
    /// Follows whose Twitch bell is off.
    pub bells_off: usize,
    /// Of those, already muted here.
    pub already: usize,
    /// Follows Twitch answered without a channel or a bell setting.
    pub skipped: usize,
    /// Only the first `BELL_PAGES` pages of follows were read.
    pub truncated: bool,
}

/// The read, by name, so the GQL watchdog can match it.
pub const BELLS_QUERY: &str = "query StreamNookLiveBells($after: Cursor) { currentUser { id follows(first: 100, after: $after) { pageInfo { hasNextPage } edges { cursor notificationSettings { isEnabled } node { id login displayName } } } } }";

/// Pages of 100 follows read at most (2000 follows), as the alert sheet.
pub const BELL_PAGES: usize = 20;

/// One page of follows as Copy reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BellPage {
    pub user_id: String,
    pub bells_off: Vec<String>,
    pub skipped: usize,
    pub edges: usize,
    /// The cursor of the last edge when Twitch says there are more.
    pub next: Option<String>,
}

/// Every page read for one copy.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FetchedBells {
    pub user_id: String,
    pub bells_off: Vec<String>,
    pub skipped: usize,
    pub truncated: bool,
}

fn is_unauthorized(resp: &Value) -> bool {
    if resp.get("status").and_then(Value::as_u64) == Some(401) {
        return true;
    }
    if resp.get("error").and_then(Value::as_str).is_some_and(|e| e.eq_ignore_ascii_case("unauthorized")) {
        return true;
    }
    resp.get("errors").and_then(Value::as_array).is_some_and(|errors| {
        errors.iter().any(|e| {
            e.get("message").and_then(Value::as_str).is_some_and(|m| m.to_lowercase().contains("unauthorized"))
        })
    })
}

/// Read one answer to `BELLS_QUERY`. A follow with no channel (a deleted or
/// banned account) or no bell setting is skipped and counted, never fatal.
pub fn parse_bell_page(resp: &Value) -> Result<BellPage, CopyError> {
    if is_unauthorized(resp) {
        return Err(CopyError::SignedOut);
    }
    let Some(data) = resp.get("data").filter(|d| d.is_object()) else {
        // No data and no sign-in error: Twitch refused the read itself (a
        // renamed field), which signing in again would not fix.
        return Err(CopyError::TwitchError);
    };
    let Some(user) = data.get("currentUser").filter(|u| u.is_object()) else {
        return Err(CopyError::SignedOut);
    };
    let user_id = user.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
    if user_id.is_empty() {
        return Err(CopyError::TwitchError);
    }
    let Some(follows) = user.get("follows").filter(|f| f.is_object()) else {
        return Err(CopyError::TwitchError);
    };
    let Some(edges) = follows.get("edges").and_then(Value::as_array) else {
        return Err(CopyError::TwitchError);
    };
    let mut page = BellPage { user_id, edges: edges.len(), ..BellPage::default() };
    for edge in edges {
        let login = edge
            .get("node")
            .and_then(|n| n.get("login"))
            .and_then(Value::as_str)
            .map(|l| l.trim().to_lowercase())
            .filter(|l| !l.is_empty());
        let enabled = edge.get("notificationSettings").and_then(|s| s.get("isEnabled")).and_then(Value::as_bool);
        match (login, enabled) {
            (Some(login), Some(false)) => page.bells_off.push(login),
            (Some(_), Some(true)) => {}
            _ => page.skipped += 1,
        }
    }
    let more = follows.pointer("/pageInfo/hasNextPage").and_then(Value::as_bool).unwrap_or(false);
    if more {
        page.next = edges
            .last()
            .and_then(|e| e.get("cursor"))
            .and_then(Value::as_str)
            .filter(|c| !c.is_empty())
            .map(str::to_string);
    }
    Ok(page)
}

/// Read every page of the viewer's follows with their bells, through the
/// twitch.tv web session. Nothing is written here.
pub async fn fetch_bells(token: &str) -> Result<FetchedBells, CopyError> {
    let mut out = FetchedBells::default();
    let mut after: Option<String> = None;
    for page_no in 1..=BELL_PAGES {
        let body = json!({ "query": BELLS_QUERY, "variables": { "after": after } });
        let resp = crate::services::auth_proxy::gql_query(token, body).await.map_err(|e| {
            log::warn!("[LiveMute] bells read failed: {e:#}");
            CopyError::TwitchError
        })?;
        let page = parse_bell_page(&resp)?;
        if out.user_id.is_empty() {
            out.user_id = page.user_id.clone();
        } else if out.user_id != page.user_id {
            return Err(CopyError::WrongAccount);
        }
        out.bells_off.extend(page.bells_off);
        out.skipped += page.skipped;
        match page.next {
            Some(cursor) if page.edges > 0 => {
                if page_no == BELL_PAGES {
                    out.truncated = true;
                    break;
                }
                after = Some(cursor);
            }
            _ => break,
        }
    }
    Ok(out)
}

/// The merge a copy saves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Merge {
    pub list: Vec<String>,
    pub added: Vec<String>,
    pub already: usize,
}

/// `list` with every bell-off login not already muted added, in follow order.
pub fn merge_bells(list: &[String], bells_off: &[String]) -> Merge {
    let mut merged = Merge { list: list.to_vec(), added: Vec::new(), already: 0 };
    for login in bells_off {
        let keys = keys_for_twitch(login);
        let Some(key) = keys.first() else { continue };
        if merged.added.contains(key) {
            continue;
        }
        if is_muted(&merged.list, &keys) {
            merged.already += 1;
            continue;
        }
        merged.list.push(key.clone());
        merged.added.push(key.clone());
    }
    merged
}

/// `list` without the entries an undo takes out, and how many it took.
pub fn undo_in(list: &[String], added: &[String]) -> (Vec<String>, usize) {
    let next: Vec<String> = list
        .iter()
        .filter(|entry| !added.iter().any(|a| a.eq_ignore_ascii_case(entry.trim())))
        .cloned()
        .collect();
    let removed = list.len() - next.len();
    (next, removed)
}

/// The last copy, for its Undo and for a sheet reopened after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyRecord {
    pub copy_id: u64,
    pub account_id: String,
    /// `TwitchService::account_generation()` when the copy was made: Undo and
    /// a reopened sheet hold only while it has not moved.
    pub generation: u64,
    /// What Undo still takes out: the copy's logins minus any edited since.
    pub added: Vec<String>,
    pub answer: CopyAnswer,
    pub at: Instant,
}

/// How long a reopened sheet still shows the last copy's result.
pub const LAST_COPY_FOR: Duration = Duration::from_secs(10 * 60);

static RECORD: Mutex<Option<CopyRecord>> = Mutex::new(None);
/// A copy whose save is still finishing: recorded under the settings lock,
/// made the record only once that save succeeded (`keep_pending_copy`), and
/// edited by every press meanwhile, so a press in between stays as pressed.
/// Lock order: `PENDING`, then `RECORD`.
static PENDING: Mutex<Option<CopyRecord>> = Mutex::new(None);
static NEXT_COPY: AtomicU64 = AtomicU64::new(1);

fn new_record(account_id: &str, generation: u64, answer: &mut CopyAnswer) -> CopyRecord {
    answer.copy_id = NEXT_COPY.fetch_add(1, Ordering::SeqCst);
    CopyRecord {
        copy_id: answer.copy_id,
        account_id: account_id.to_string(),
        generation,
        added: answer.added.clone(),
        answer: answer.clone(),
        at: Instant::now(),
    }
}

/// Keep a finished copy as the one Undo can take back. Returns the answer
/// with its id.
pub fn record_copy(account_id: &str, generation: u64, mut answer: CopyAnswer) -> CopyAnswer {
    let record = new_record(account_id, generation, &mut answer);
    if let Ok(mut held) = RECORD.lock() {
        *held = Some(record);
    }
    answer
}

/// A copy about to be saved: held aside until `keep_pending_copy` (its save
/// succeeded) or `drop_pending_copy` (it failed; the copy before keeps its Undo).
pub fn record_pending_copy(account_id: &str, generation: u64, mut answer: CopyAnswer) -> CopyAnswer {
    let record = new_record(account_id, generation, &mut answer);
    if let Ok(mut pending) = PENDING.lock() {
        *pending = Some(record);
    }
    answer
}

/// The pending copy saved: it becomes the record (with any press since).
pub fn keep_pending_copy(copy_id: u64) {
    let Ok(mut pending) = PENDING.lock() else { return };
    if pending.as_ref().is_some_and(|r| r.copy_id == copy_id) {
        let record = pending.take();
        if let Ok(mut held) = RECORD.lock() {
            *held = record;
        }
    }
}

/// The pending copy did not save: forget it.
pub fn drop_pending_copy(copy_id: u64) {
    if let Ok(mut pending) = PENDING.lock() {
        if pending.as_ref().is_some_and(|r| r.copy_id == copy_id) {
            *pending = None;
        }
    }
}

/// A channel edited by hand since the copy leaves its Undo: a deliberate
/// mute is never taken back by it.
pub fn forget_in(record: &mut CopyRecord, keys: &[String]) {
    record.added.retain(|a| !keys.iter().any(|k| k.eq_ignore_ascii_case(a)));
}

/// `forget_in` on the held record and on a copy still saving. `PENDING` is
/// held across both, so a copy moving from one to the other is never missed.
pub fn forget_copied(keys: &[String]) {
    let pending = PENDING.lock();
    if let Ok(mut pending) = pending {
        if let Some(record) = pending.as_mut() {
            forget_in(record, keys);
        }
        if let Ok(mut held) = RECORD.lock() {
            if let Some(record) = held.as_mut() {
                forget_in(record, keys);
            }
        }
    }
}

/// What Undo of `copy_id` takes out, or `Stale` when that copy is not the
/// last one or the account changed since (`generation` moved).
pub fn undo_check(record: Option<&CopyRecord>, copy_id: u64, generation: u64) -> Result<Vec<String>, CopyError> {
    match record {
        Some(r) if r.copy_id == copy_id && r.generation == generation => Ok(r.added.clone()),
        _ => Err(CopyError::Stale),
    }
}

/// `undo_check` on the held record.
pub fn undo_list(copy_id: u64, generation: u64) -> Result<Vec<String>, CopyError> {
    let held = RECORD.lock().map_err(|_| CopyError::Stale)?;
    undo_check(held.as_ref(), copy_id, generation)
}

/// Drop the record once its Undo saved.
pub fn clear_copy(copy_id: u64) {
    if let Ok(mut held) = RECORD.lock() {
        if held.as_ref().is_some_and(|r| r.copy_id == copy_id) {
            *held = None;
        }
    }
}

/// The last copy's answer for a sheet opened again: only while the account
/// has not changed (`generation`) and within `LAST_COPY_FOR`, with `added` as
/// Undo would take it now.
pub fn last_copy_from(record: Option<&CopyRecord>, generation: u64, age: Duration) -> Option<CopyAnswer> {
    let r = record?;
    if r.generation != generation || age >= LAST_COPY_FOR {
        return None;
    }
    let mut answer = r.answer.clone();
    answer.added = r.added.clone();
    Some(answer)
}

/// `last_copy_from` on the held record.
pub fn last_copy_answer(generation: u64) -> Option<CopyAnswer> {
    let held = RECORD.lock().ok()?;
    let record = held.as_ref()?;
    last_copy_from(Some(record), generation, record.at.elapsed())
}

static COPY_BUSY: AtomicBool = AtomicBool::new(false);

/// Held for one whole copy; a second press while it runs answers `Busy`.
/// Released on drop, so an early return never leaves Copy stuck.
pub struct CopyBusy;

impl CopyBusy {
    pub fn claim() -> Option<Self> {
        COPY_BUSY.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).ok().map(|_| CopyBusy)
    }
}

impl Drop for CopyBusy {
    fn drop(&mut self) {
        COPY_BUSY.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UC: &str = "UCabcDEF1234567890123456";

    fn row(provider: &str, key: &str, user_id: &str, login: &str) -> ProviderStream {
        serde_json::from_value(json!({
            "provider": provider, "key": key, "user_id": user_id, "user_login": login,
            "user_name": login, "is_live": true, "watch_url": ""
        }))
        .unwrap()
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn twitch_logins_and_platform_keys_never_meet() {
        assert_eq!(keys_for_twitch(" xQc "), strings(&["xqc"]));
        assert!(keys_for_twitch("  ").is_empty());
        assert_eq!(keys_for_channel("kick", "12345", "XQC"), strings(&["kick:xqc"]));
        assert!(!is_muted(&strings(&["kick:xqc"]), &keys_for_twitch("xqc")));
        assert!(!is_muted(&strings(&["xqc"]), &keys_for_channel("kick", "", "xqc")));
        for key in keys_for_channel("youtube", UC, "@handle") {
            assert!(key.starts_with("youtube:"), "{key}");
        }
    }

    #[test]
    fn muted_entries_match_in_any_case() {
        let list = strings(&["XQC", "youtube:ucabcdef1234567890123456"]);
        assert!(is_muted(&list, &keys_for_twitch("xqc")));
        assert!(is_muted(&list, &keys_for_channel("youtube", UC, "")));
        assert!(!is_muted(&list, &keys_for_twitch("shroud")));
    }

    #[test]
    fn row_keys_cover_every_identity_and_never_the_twitch_id() {
        assert_eq!(keys_for_row(&row("twitch", "xqc", "71092938", "xQc")), strings(&["xqc"]));
        let yt = keys_for_row(&row("youtube", "youtube:@handle", UC, "@handle"));
        assert!(yt.contains(&format!("youtube:{UC}")));
        assert!(yt.contains(&"youtube:@handle".to_string()));
        assert_eq!(yt.len(), 2, "the row key is not repeated");
        assert_eq!(
            keys_for_row(&row("youtube", "youtube:AGr94tpNVkw", "", "AGr94tpNVkw")),
            strings(&["youtube:AGr94tpNVkw"])
        );
        assert_eq!(keys_for_row(&row("tiktok", "tiktok:someone", "", "someone")), strings(&["tiktok:someone"]));
    }

    #[test]
    fn a_channel_with_no_stable_identity_has_no_canonical_key() {
        assert_eq!(canonical("twitch", "1", "Bob").as_deref(), Some("bob"));
        assert_eq!(canonical("kick", "12345", "XQC").as_deref(), Some("kick:xqc"));
        assert_eq!(canonical("youtube", UC, "AGr94tpNVkw"), Some(format!("youtube:{UC}")));
        assert_eq!(canonical("youtube", "", "AGr94tpNVkw"), None);
        assert_eq!(canonical("twitch", "1", "  "), None);
        assert!(row_alert(&row("youtube", "youtube:AGr94tpNVkw", "", "AGr94tpNVkw")).0.is_none());
        assert_eq!(twitch_alert("7", "xQc").0.unwrap().user_login, "xqc");
    }

    #[test]
    fn entries_are_stored_in_one_shape() {
        assert_eq!(normalize_entry(" XQC ").as_deref(), Some("xqc"));
        assert_eq!(normalize_entry("twitch:Shroud").as_deref(), Some("shroud"));
        assert_eq!(normalize_entry("Kick:XQC").as_deref(), Some("kick:xqc"));
        assert_eq!(normalize_entry(&format!("YouTube:{UC}")), Some(format!("youtube:{UC}")));
        assert_eq!(normalize_entry("   "), None);
        assert_eq!(normalize_entry("kick:"), None);
    }

    #[test]
    fn setting_a_bell_is_idempotent() {
        let keys = keys_for_channel("youtube", UC, "@handle");
        let canon = canonical("youtube", UC, "@handle");
        let off = set_in(&[], &keys, canon.as_deref(), false).unwrap();
        assert_eq!(off, vec![format!("youtube:{UC}")]);
        assert_eq!(set_in(&off, &keys, canon.as_deref(), false), None, "off twice stays off");
        let both = strings(&["youtube:@handle", "YOUTUBE:ucabcdef1234567890123456", "xqc"]);
        assert_eq!(set_in(&both, &keys, canon.as_deref(), true), Some(strings(&["xqc"])), "on takes out every key");
        assert_eq!(set_in(&strings(&["xqc"]), &keys, canon.as_deref(), true), None, "on twice stays on");
        assert_eq!(set_in(&[], &[], None, false), None, "nothing to mute by");
    }

    fn page(edges: Value, has_next: bool) -> Value {
        json!({ "data": { "currentUser": { "id": "209953493", "follows": {
            "pageInfo": { "hasNextPage": has_next }, "edges": edges } } } })
    }

    #[test]
    fn a_page_lists_only_bells_that_are_off_and_counts_what_it_skips() {
        let p = parse_bell_page(&page(
            json!([
                { "cursor": "a", "notificationSettings": { "isEnabled": false }, "node": { "id": "1", "login": "Kamet0", "displayName": "Kamet0" } },
                { "cursor": "b", "notificationSettings": { "isEnabled": true }, "node": { "id": "2", "login": "xqc", "displayName": "xQc" } },
                { "cursor": "c", "notificationSettings": null, "node": { "id": "3", "login": "gone", "displayName": "gone" } },
                { "cursor": "d", "notificationSettings": { "isEnabled": false }, "node": null }
            ]),
            true,
        ))
        .unwrap();
        assert_eq!(p.user_id, "209953493");
        assert_eq!(p.bells_off, strings(&["kamet0"]));
        assert_eq!(p.skipped, 2);
        assert_eq!(p.edges, 4);
        assert_eq!(p.next.as_deref(), Some("d"));
        assert_eq!(parse_bell_page(&page(json!([]), false)).unwrap().next, None);
    }

    #[test]
    fn signed_out_and_broken_answers_have_their_own_codes() {
        let bad_token = json!({ "error": "Unauthorized", "status": 401, "message": "The token is invalid." });
        assert_eq!(parse_bell_page(&bad_token).unwrap_err(), CopyError::SignedOut);
        assert_eq!(parse_bell_page(&json!({ "data": { "currentUser": null } })).unwrap_err(), CopyError::SignedOut);
        let unauthorized = json!({ "errors": [{ "message": "unauthorized" }], "data": { "currentUser": null } });
        assert_eq!(parse_bell_page(&unauthorized).unwrap_err(), CopyError::SignedOut);
        let renamed = json!({ "errors": [{ "message": "Cannot query field \"x\" on type \"FollowEdge\"." }] });
        assert_eq!(parse_bell_page(&renamed).unwrap_err(), CopyError::TwitchError);
        assert_eq!(parse_bell_page(&json!({ "data": { "currentUser": { "id": "1" } } })).unwrap_err(), CopyError::TwitchError);
        assert_eq!(serde_json::to_value(CopyError::WrongAccount).unwrap(), json!("wrong-account"));
        assert_eq!(serde_json::to_value(CopyError::SaveFailed).unwrap(), json!("save-failed"));
    }

    #[test]
    fn merging_keeps_follow_order_and_skips_what_is_muted() {
        let m = merge_bells(&strings(&["XQC", "kick:shroud"]), &strings(&["kamet0", "xqc", "shroud", "kamet0"]));
        assert_eq!(m.added, strings(&["kamet0", "shroud"]));
        assert_eq!(m.already, 1);
        assert_eq!(m.list, strings(&["XQC", "kick:shroud", "kamet0", "shroud"]));
    }

    #[test]
    fn undo_takes_out_only_what_the_copy_added() {
        let (next, removed) = undo_in(&strings(&["xqc", "Kamet0", "shroud"]), &strings(&["kamet0", "gone"]));
        assert_eq!(next, strings(&["xqc", "shroud"]));
        assert_eq!(removed, 1);
    }

    fn answer(added: &[&str]) -> CopyAnswer {
        CopyAnswer { copy_id: 7, added: strings(added), bells_off: 3, already: 1, skipped: 0, truncated: false }
    }

    #[test]
    fn undo_is_refused_for_another_copy_or_account_and_skips_edited_channels() {
        let rec = CopyRecord {
            copy_id: 7,
            account_id: "1".into(),
            generation: 3,
            added: strings(&["kamet0", "xqc"]),
            answer: answer(&["kamet0", "xqc"]),
            at: Instant::now(),
        };
        assert_eq!(undo_check(Some(&rec), 7, 3).unwrap(), rec.added);
        assert_eq!(undo_check(Some(&rec), 6, 3).unwrap_err(), CopyError::Stale);
        assert_eq!(undo_check(Some(&rec), 7, 4).unwrap_err(), CopyError::Stale, "the account changed");
        assert_eq!(undo_check(None, 7, 3).unwrap_err(), CopyError::Stale);
        let mut edited = rec.clone();
        forget_in(&mut edited, &keys_for_twitch("XQC"));
        assert_eq!(edited.added, strings(&["kamet0"]));
    }

    #[test]
    fn a_press_while_a_copy_saves_stays_pressed_and_a_failed_copy_keeps_the_last_undo() {
        // The only test that touches the held records.
        let first = record_copy("1", 9, answer(&["a1", "b1"]));
        let next = record_pending_copy("1", 9, answer(&["c1", "d1"]));
        forget_copied(&keys_for_twitch("C1"));
        assert_eq!(undo_list(first.copy_id, 9).unwrap(), strings(&["a1", "b1"]), "still the last saved copy");
        keep_pending_copy(next.copy_id);
        assert_eq!(undo_list(next.copy_id, 9).unwrap(), strings(&["d1"]), "the press made during the save stays");
        assert_eq!(undo_list(first.copy_id, 9).unwrap_err(), CopyError::Stale);
        let failed = record_pending_copy("1", 9, answer(&["e1"]));
        drop_pending_copy(failed.copy_id);
        keep_pending_copy(failed.copy_id);
        assert_eq!(undo_list(next.copy_id, 9).unwrap(), strings(&["d1"]), "a copy that did not save leaves the last Undo");
    }

    #[test]
    fn a_reopened_sheet_sees_the_last_copy_for_ten_minutes_on_the_same_account() {
        let mut rec = CopyRecord {
            copy_id: 7,
            account_id: "1".into(),
            generation: 3,
            added: strings(&["kamet0", "xqc"]),
            answer: answer(&["kamet0", "xqc"]),
            at: Instant::now(),
        };
        forget_in(&mut rec, &keys_for_twitch("xqc"));
        let shown = last_copy_from(Some(&rec), 3, Duration::from_secs(60)).unwrap();
        assert_eq!(shown.added, strings(&["kamet0"]), "what Undo would take now");
        assert_eq!(shown.copy_id, 7);
        assert!(last_copy_from(Some(&rec), 4, Duration::from_secs(60)).is_none(), "another account");
        assert!(last_copy_from(Some(&rec), 3, LAST_COPY_FOR).is_none());
        assert!(last_copy_from(None, 3, Duration::ZERO).is_none());
    }

    #[test]
    fn a_second_copy_while_one_runs_is_busy() {
        let first = CopyBusy::claim().expect("free at start");
        assert!(CopyBusy::claim().is_none());
        drop(first);
        assert!(CopyBusy::claim().is_some());
    }
}
