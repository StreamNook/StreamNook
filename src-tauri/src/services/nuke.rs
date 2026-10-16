//! `/nuke` and `/undo`: mass moderation by phrase or regex.
//!
//! `/nuke <pattern> <action> <past[:future]>`
//! - pattern: a plain phrase (case-insensitive substring) or `/regex/flags`
//! - action: `delete`, `ban`, or a timeout length like `10m`
//! - past: how far back to look through the messages the app holds for the room
//! - future: optional; how long to keep acting on new matching messages
//!
//! The look-back reads `chat_history` (the same ring `/logs` and search read).
//! A future window is checked by [`observe`], which the chat rules engine calls
//! for every message it records, so the check runs next to the message parser
//! for every window at once. `/undo` lifts the bans and timeouts of the newest
//! nuke in a room; Twitch cannot bring deleted messages back.
//!
//! Parsing and matching are pure and tested here; the dispatcher
//! (`slash_dispatch`) runs the moderation calls of the look-back itself so its
//! tests can watch them.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use regex::{Regex, RegexBuilder};

use crate::services::chat_history::SweepEntry;

/// What a nuke does to each person (or message) it catches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NukeAction {
    Delete,
    Ban,
    Timeout(u64),
}

impl NukeAction {
    /// "delete", "ban" or "timeout 600s".
    pub fn describe(&self) -> String {
        match self {
            NukeAction::Delete => "delete".to_string(),
            NukeAction::Ban => "ban".to_string(),
            NukeAction::Timeout(s) => format!("timeout {s}s"),
        }
    }
}

/// A parsed `/nuke` line.
#[derive(Clone, Debug)]
pub struct ParsedNuke {
    pub pattern: Regex,
    pub source: String,
    pub action: NukeAction,
    pub past_seconds: u64,
    pub future_seconds: u64,
}

fn unit_seconds(c: char) -> Option<u64> {
    match c {
        's' => Some(1),
        'm' => Some(60),
        'h' => Some(3600),
        'd' => Some(86_400),
        _ => None,
    }
}

/// "30s", "5m", "1h", "2d": digits then exactly one unit.
pub fn parse_duration_token(tok: &str) -> Option<u64> {
    let unit = tok.chars().last()?;
    let digits = &tok[..tok.len() - unit.len_utf8()];
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let n: u64 = digits.parse().unwrap_or(u64::MAX);
    Some(n.saturating_mul(unit_seconds(unit)?))
}

/// "5m" or "5m:1m" as (past, future) seconds.
pub fn parse_window(tok: &str) -> Option<(u64, u64)> {
    match tok.split_once(':') {
        Some((past, future)) => Some((parse_duration_token(past)?, parse_duration_token(future)?)),
        None => Some((parse_duration_token(tok)?, 0)),
    }
}

pub fn parse_action_token(tok: &str) -> Option<NukeAction> {
    match tok {
        "delete" => Some(NukeAction::Delete),
        "ban" => Some(NukeAction::Ban),
        _ => parse_duration_token(tok).map(NukeAction::Timeout),
    }
}

/// `/regex/flags` (the flags `g` and `y` mean nothing per message and are
/// dropped; none left means case-insensitive) or a plain phrase matched
/// case-insensitively.
pub fn parse_pattern(raw: &str) -> Result<Regex, String> {
    if raw.len() >= 3 && raw.starts_with('/') {
        if let Some(last) = raw.rfind('/').filter(|&i| i > 1) {
            let flags = &raw[last + 1..];
            if flags.chars().all(|c| "gimuy".contains(c)) {
                let body = &raw[1..last];
                let mut kept: String = flags.chars().filter(|c| *c != 'g' && *c != 'y').collect();
                if kept.is_empty() {
                    kept.push('i');
                }
                return RegexBuilder::new(body)
                    .case_insensitive(kept.contains('i'))
                    .multi_line(kept.contains('m'))
                    .build()
                    .map_err(|e| format!("invalid regex: {e}"));
            }
        }
    }
    RegexBuilder::new(&regex::escape(raw))
        .case_insensitive(true)
        .build()
        .map_err(|e| format!("invalid pattern: {e}"))
}

/// Parse everything after `/nuke`, reading from the right: the last word is
/// the window, the one before it the action, the rest the pattern.
pub fn parse_nuke_args(args: &str) -> Result<ParsedNuke, String> {
    let tokens: Vec<&str> = args.split_whitespace().collect();
    if tokens.len() < 3 {
        return Err("usage: /nuke <pattern> <action> <past[:future]>  e.g. /nuke spam ban 5m:1m".to_string());
    }
    let window_tok = tokens[tokens.len() - 1];
    let (past, future) = parse_window(window_tok).ok_or_else(|| {
        format!("last argument must be a window like \"5m\" or \"5m:1m\", not \"{window_tok}\"")
    })?;
    let action_tok = tokens[tokens.len() - 2];
    let action = parse_action_token(action_tok).ok_or_else(|| {
        format!("second-to-last argument must be \"delete\", \"ban\", or a duration like \"10m\", not \"{action_tok}\"")
    })?;
    let source = tokens[..tokens.len() - 2].join(" ").trim().to_string();
    if source.is_empty() {
        return Err("pattern is required".to_string());
    }
    let pattern = parse_pattern(&source)?;
    Ok(ParsedNuke { pattern, source, action, past_seconds: past, future_seconds: future })
}

/// One person a nuke caught, with the messages it caught them by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Affected {
    pub user_id: String,
    pub username: String,
    pub message_ids: Vec<String>,
}

/// Everything a nuke would hit among `entries`, grouped by person in the
/// order they first matched. The count shown while typing comes from here
/// too, so the preview can never disagree with the action.
pub fn sweep(entries: &[SweepEntry], parsed: &ParsedNuke, cutoff_ms: i64) -> (usize, Vec<Affected>) {
    let mut matched = 0;
    let mut order: Vec<String> = Vec::new();
    let mut grouped: HashMap<String, Affected> = HashMap::new();
    for e in entries {
        if e.user_id.is_empty() || e.id.is_empty() || e.user_id == "tw-system" {
            continue;
        }
        if e.ts_ms < cutoff_ms || !parsed.pattern.is_match(&e.content) {
            continue;
        }
        matched += 1;
        match grouped.get_mut(&e.user_id) {
            Some(a) => a.message_ids.push(e.id.clone()),
            None => {
                order.push(e.user_id.clone());
                let username = if e.display.is_empty() { e.login.clone() } else { e.display.clone() };
                grouped.insert(
                    e.user_id.clone(),
                    Affected { user_id: e.user_id.clone(), username, message_ids: vec![e.id.clone()] },
                );
            }
        }
    }
    let affected = order.into_iter().filter_map(|id| grouped.remove(&id)).collect();
    (matched, affected)
}

/// The line printed after a nuke runs.
pub fn ran_line(parsed: &ParsedNuke, matched: usize, users: usize) -> String {
    format!(
        "/nuke pattern \"{}\" → {} on {} message(s) from {} user(s){}",
        parsed.source,
        parsed.action.describe(),
        matched,
        users,
        if parsed.future_seconds > 0 { format!(". Future window: {}s.", parsed.future_seconds) } else { ".".to_string() }
    )
}

/// The line printed when a future window closes.
pub fn closed_line(source: &str, extra: usize) -> String {
    format!("/nuke window closed for pattern \"{source}\". Caught {extra} additional user(s) during the future window.")
}

// ───────────────────────────── records and windows ─────────────────────────────

/// A nuke that ran in a room: what `/undo` reverses, and while its future
/// window is open, what new messages are checked against.
pub struct Record {
    pub id: u64,
    pub channel: String,
    pub broadcaster_id: String,
    pub source: String,
    pub pattern: Regex,
    pub action: NukeAction,
    pub ran_at_ms: i64,
    pub affected: Vec<Affected>,
    pub initial_affected: usize,
    /// 0 when no future window (or once it was stopped).
    pub future_expires_ms: i64,
    pub seen: HashSet<String>,
}

#[derive(Default)]
struct State {
    last: HashMap<String, Arc<Mutex<Record>>>,
    active: HashMap<String, Vec<Arc<Mutex<Record>>>>,
    closers: HashMap<u64, tauri::async_runtime::JoinHandle<()>>,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();
/// How many future windows are open anywhere: lets [`observe`] return at once
/// for every message when none is.
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.get_or_init(|| Mutex::new(State::default())).lock().unwrap_or_else(|e| e.into_inner())
}

fn recount(s: &State) {
    ACTIVE.store(s.active.values().map(Vec::len).sum(), Ordering::Release);
}

fn key(channel: &str) -> String {
    channel.trim_start_matches('#').to_lowercase()
}

/// Remember a nuke that just ran, and open its future window when it has one.
/// Returns the record's id.
pub fn remember(channel: &str, broadcaster_id: &str, parsed: &ParsedNuke, affected: Vec<Affected>, now_ms: i64) -> u64 {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let ch = key(channel);
    let seen = affected.iter().map(|a| a.user_id.clone()).collect();
    let record = Arc::new(Mutex::new(Record {
        id,
        channel: ch.clone(),
        broadcaster_id: broadcaster_id.to_string(),
        source: parsed.source.clone(),
        pattern: parsed.pattern.clone(),
        action: parsed.action,
        ran_at_ms: now_ms,
        initial_affected: affected.len(),
        affected,
        future_expires_ms: if parsed.future_seconds > 0 { now_ms + parsed.future_seconds as i64 * 1000 } else { 0 },
        seen,
    }));
    let mut s = state();
    s.last.insert(ch.clone(), record.clone());
    if parsed.future_seconds > 0 {
        s.active.entry(ch).or_default().push(record);
        recount(&s);
    }
    id
}

/// Keep the task that will close window `id`, so stopping it early can cancel it.
pub fn set_closer(id: u64, task: tauri::async_runtime::JoinHandle<()>) {
    state().closers.insert(id, task);
}

/// Close window `id` when its time is up. Returns the pattern and how many
/// more people it caught, or None when it was already stopped.
pub fn close(channel: &str, id: u64) -> Option<(String, usize)> {
    let ch = key(channel);
    let mut s = state();
    s.closers.remove(&id);
    let list = s.active.get_mut(&ch)?;
    let pos = list.iter().position(|r| r.lock().map(|r| r.id == id).unwrap_or(false))?;
    let record = list.remove(pos);
    if list.is_empty() {
        s.active.remove(&ch);
    }
    recount(&s);
    let r = record.lock().ok()?;
    Some((r.source.clone(), r.affected.len().saturating_sub(r.initial_affected)))
}

/// `/nuke stop`: end every open window in a room. Returns how many.
pub fn stop(channel: &str) -> usize {
    let ch = key(channel);
    let mut s = state();
    let Some(list) = s.active.remove(&ch) else { return 0 };
    for record in &list {
        if let Ok(mut r) = record.lock() {
            r.future_expires_ms = 0;
            if let Some(task) = s.closers.remove(&r.id) {
                task.abort();
            }
        }
    }
    recount(&s);
    list.len()
}

/// How many windows are open in a room.
pub fn active_count(channel: &str) -> usize {
    state().active.get(&key(channel)).map(Vec::len).unwrap_or(0)
}

/// What `/undo` needs from the newest nuke in a room, which it forgets (and
/// whose window it closes).
pub struct UndoPlan {
    pub source: String,
    pub broadcaster_id: String,
    pub action: NukeAction,
    pub affected: Vec<Affected>,
}

pub fn take_last(channel: &str) -> Option<UndoPlan> {
    let ch = key(channel);
    let mut s = state();
    let record = s.last.remove(&ch)?;
    if let Some(list) = s.active.get_mut(&ch) {
        list.retain(|r| !Arc::ptr_eq(r, &record));
        if list.is_empty() {
            s.active.remove(&ch);
        }
    }
    recount(&s);
    let r = record.lock().ok()?;
    if let Some(task) = s.closers.remove(&r.id) {
        task.abort();
    }
    Some(UndoPlan {
        source: r.source.clone(),
        broadcaster_id: r.broadcaster_id.clone(),
        action: r.action,
        affected: r.affected.clone(),
    })
}

/// The moderation call one catch needs, decided under the records' lock and
/// made after it is released.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Catch {
    pub broadcaster_id: String,
    pub action: NukeAction,
    pub user_id: String,
    pub message_id: String,
}

/// Check one new message against the room's open windows. A delete acts on
/// every matching message; a ban or timeout acts once per person per nuke.
pub fn catches_for(channel: &str, user_id: &str, display: &str, message_id: &str, content: &str, ts_ms: i64, now_ms: i64) -> Vec<Catch> {
    if user_id.is_empty() || user_id == "tw-system" || content.is_empty() || message_id.is_empty() {
        return Vec::new();
    }
    let ch = key(channel);
    let records: Vec<Arc<Mutex<Record>>> = match state().active.get(&ch) {
        Some(list) => list.clone(),
        None => return Vec::new(),
    };
    let mut out = Vec::new();
    for record in records {
        let Ok(mut r) = record.lock() else { continue };
        // Only messages sent after the nuke ran: a backfill replaying older
        // lines must not be swept again.
        if now_ms > r.future_expires_ms || ts_ms < r.ran_at_ms || !r.pattern.is_match(content) {
            continue;
        }
        let first_time = r.seen.insert(user_id.to_string());
        let catch = Catch {
            broadcaster_id: r.broadcaster_id.clone(),
            action: r.action,
            user_id: user_id.to_string(),
            message_id: message_id.to_string(),
        };
        match r.action {
            NukeAction::Delete => out.push(catch),
            _ if first_time => {
                r.affected.push(Affected {
                    user_id: user_id.to_string(),
                    username: display.to_string(),
                    message_ids: vec![message_id.to_string()],
                });
                out.push(catch);
            }
            _ => {}
        }
    }
    out
}

/// The reason a nuke's bans and timeouts carry (Settings, moderation).
pub fn reason_from(settings: &serde_json::Value) -> String {
    settings["moderation"]["nuke_reason"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("/nuke")
        .to_string()
}

/// A message the chat rules engine recorded. Cheap when no window is open
/// anywhere (one atomic read).
pub fn observe(channel_key: &str, msg: &crate::models::chat_layout::ChatMessage, ts_ms: i64) {
    if ACTIVE.load(Ordering::Acquire) == 0 || channel_key.contains(':') {
        return;
    }
    let display = if msg.display_name.is_empty() { &msg.username } else { &msg.display_name };
    let catches = catches_for(channel_key, &msg.user_id, display, &msg.id, &msg.content, ts_ms, chrono::Utc::now().timestamp_millis());
    if catches.is_empty() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let reason = crate::services::providers::app_handle()
            .and_then(|app| {
                use tauri::Manager;
                let state = app.state::<crate::models::settings::AppState>();
                let settings = state.settings.lock().ok()?.clone();
                serde_json::to_value(settings).ok()
            })
            .map(|v| reason_from(&v))
            .unwrap_or_else(|| "/nuke".to_string());
        for c in catches {
            let result = match c.action {
                NukeAction::Delete => {
                    crate::services::twitch_service::TwitchService::delete_chat_message(&c.broadcaster_id, &c.message_id).await
                }
                NukeAction::Ban => {
                    crate::services::twitch_service::TwitchService::ban_user(&c.broadcaster_id, &c.user_id, None, Some(&reason)).await
                }
                NukeAction::Timeout(secs) => {
                    let secs = u32::try_from(secs).unwrap_or(u32::MAX);
                    crate::services::twitch_service::TwitchService::ban_user(&c.broadcaster_id, &c.user_id, Some(secs), Some(&reason)).await
                }
            };
            if let Err(e) = result {
                log::warn!("[nuke] action failed for {}: {e}", c.user_id);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, user: &str, text: &str, ts: i64) -> SweepEntry {
        SweepEntry {
            id: id.into(),
            user_id: user.into(),
            login: format!("login{user}"),
            display: format!("User{user}"),
            content: text.into(),
            ts_ms: ts,
        }
    }

    #[test]
    fn durations_take_one_unit() {
        assert_eq!(parse_duration_token("30s"), Some(30));
        assert_eq!(parse_duration_token("5m"), Some(300));
        assert_eq!(parse_duration_token("2h"), Some(7200));
        assert_eq!(parse_duration_token("1d"), Some(86_400));
        assert_eq!(parse_duration_token("5"), None);
        assert_eq!(parse_duration_token("5w"), None);
        assert_eq!(parse_duration_token("1h30m"), None);
        assert_eq!(parse_duration_token("m"), None);
    }

    #[test]
    fn windows_have_an_optional_future() {
        assert_eq!(parse_window("5m"), Some((300, 0)));
        assert_eq!(parse_window("5m:1m"), Some((300, 60)));
        assert_eq!(parse_window("5m:"), None);
        assert_eq!(parse_window("x:1m"), None);
    }

    #[test]
    fn actions_are_delete_ban_or_a_length() {
        assert_eq!(parse_action_token("delete"), Some(NukeAction::Delete));
        assert_eq!(parse_action_token("ban"), Some(NukeAction::Ban));
        assert_eq!(parse_action_token("10m"), Some(NukeAction::Timeout(600)));
        assert_eq!(parse_action_token("kick"), None);
        assert_eq!(NukeAction::Timeout(600).describe(), "timeout 600s");
    }

    #[test]
    fn parses_from_the_right() {
        let p = parse_nuke_args("buy  followers now ban 5m:1m").unwrap();
        assert_eq!(p.source, "buy followers now");
        assert_eq!(p.action, NukeAction::Ban);
        assert_eq!((p.past_seconds, p.future_seconds), (300, 60));
        assert!(p.pattern.is_match("BUY FOLLOWERS NOW!!"));
    }

    #[test]
    fn says_what_is_wrong() {
        assert_eq!(
            parse_nuke_args("spam ban").unwrap_err(),
            "usage: /nuke <pattern> <action> <past[:future]>  e.g. /nuke spam ban 5m:1m"
        );
        assert_eq!(
            parse_nuke_args("spam ban soon").unwrap_err(),
            "last argument must be a window like \"5m\" or \"5m:1m\", not \"soon\""
        );
        assert_eq!(
            parse_nuke_args("spam kick 5m").unwrap_err(),
            "second-to-last argument must be \"delete\", \"ban\", or a duration like \"10m\", not \"kick\""
        );
        assert!(parse_nuke_args("/(unclosed/ ban 5m").unwrap_err().starts_with("invalid regex: "));
    }

    #[test]
    fn regexes_take_flags_and_default_to_case_insensitive() {
        let p = parse_nuke_args("/bad\\s*word/ delete 10m").unwrap();
        assert!(p.pattern.is_match("BAD   WORD"));
        let sensitive = parse_nuke_args("/Bad/m delete 10m").unwrap();
        assert!(sensitive.pattern.is_match("Bad"));
        assert!(!sensitive.pattern.is_match("bad"));
        // g and y are dropped, which leaves the default.
        let global = parse_nuke_args("/bad/g delete 10m").unwrap();
        assert!(global.pattern.is_match("BAD"));
        // A phrase that only looks like it starts a regex is a phrase.
        let phrase = parse_nuke_args("/x delete 10m").unwrap();
        assert!(phrase.pattern.is_match("say /X now"));
        // Special characters in a phrase are literal.
        let dots = parse_nuke_args("a.b delete 10m").unwrap();
        assert!(dots.pattern.is_match("A.B"));
        assert!(!dots.pattern.is_match("axb"));
    }

    #[test]
    fn a_sweep_groups_by_person_inside_the_window() {
        let p = parse_nuke_args("spam ban 5m").unwrap();
        let entries = vec![
            entry("1", "a", "spam here", 1_000),
            entry("2", "b", "nothing", 2_000),
            entry("3", "a", "more SPAM", 3_000),
            entry("4", "c", "spam too", 4_000),
            entry("5", "tw-system", "spam notice", 5_000),
            entry("6", "d", "old spam", 500),
        ];
        let (matched, affected) = sweep(&entries, &p, 900);
        assert_eq!(matched, 3);
        assert_eq!(affected.len(), 2);
        assert_eq!(affected[0].user_id, "a");
        assert_eq!(affected[0].message_ids, vec!["1", "3"]);
        assert_eq!(affected[0].username, "Usera");
        assert_eq!(affected[1].user_id, "c");
    }

    #[test]
    fn lines_read_as_before() {
        let p = parse_nuke_args("spam ban 5m:1m").unwrap();
        assert_eq!(ran_line(&p, 3, 2), "/nuke pattern \"spam\" → ban on 3 message(s) from 2 user(s). Future window: 60s.");
        let q = parse_nuke_args("spam delete 5m").unwrap();
        assert_eq!(ran_line(&q, 0, 0), "/nuke pattern \"spam\" → delete on 0 message(s) from 0 user(s).");
        assert_eq!(closed_line("spam", 4), "/nuke window closed for pattern \"spam\". Caught 4 additional user(s) during the future window.");
    }

    #[test]
    fn a_window_acts_once_per_person_and_stops() {
        let p = parse_nuke_args("raid ban 1m:1m").unwrap();
        let id = remember("winroom", "42", &p, vec![], 10_000);
        assert_eq!(active_count("WinRoom"), 1);
        // Older than the nuke: ignored.
        assert!(catches_for("winroom", "u1", "U1", "m0", "raid", 9_000, 10_500).is_empty());
        let first = catches_for("winroom", "u1", "U1", "m1", "RAID now", 11_000, 11_000);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].action, NukeAction::Ban);
        assert!(catches_for("winroom", "u1", "U1", "m2", "raid again", 12_000, 12_000).is_empty());
        assert!(catches_for("winroom", "u2", "U2", "m3", "hello", 12_000, 12_000).is_empty());
        // Expired.
        assert!(catches_for("winroom", "u3", "U3", "m4", "raid", 80_000, 80_000).is_empty());
        assert_eq!(close("winroom", id), Some(("raid".to_string(), 1)));
        assert_eq!(active_count("winroom"), 0);
        let plan = take_last("winroom").unwrap();
        assert_eq!(plan.affected.len(), 1);
        assert!(take_last("winroom").is_none());
    }

    #[test]
    fn a_delete_window_deletes_every_match_and_stop_ends_it() {
        let p = parse_nuke_args("spam delete 1m:5m").unwrap();
        remember("delroom", "42", &p, vec![], 0);
        assert_eq!(catches_for("delroom", "u1", "U1", "m1", "spam", 1, 1).len(), 1);
        assert_eq!(catches_for("delroom", "u1", "U1", "m2", "spam", 2, 2).len(), 1);
        assert_eq!(stop("delroom"), 1);
        assert_eq!(stop("delroom"), 0);
        assert!(catches_for("delroom", "u1", "U1", "m3", "spam", 3, 3).is_empty());
    }

    #[test]
    fn the_reason_comes_from_settings() {
        assert_eq!(reason_from(&serde_json::json!({})), "/nuke");
        assert_eq!(reason_from(&serde_json::json!({"moderation": {"nuke_reason": "  "}})), "/nuke");
        assert_eq!(reason_from(&serde_json::json!({"moderation": {"nuke_reason": "spam wave"}})), "spam wave");
    }
}
