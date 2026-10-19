//! One-entry edits to the lists inside settings groups: trusted link sites,
//! the spellcheck dictionary, hidden command patterns, highlight and ignored
//! phrases, highlighted users and badges, custom commands, and the toggle
//! sets (hidden event kinds, hidden badge services, scrollbar mark kinds).
//!
//! Each edit is a read-modify-write on the list Rust holds, so a page never
//! sends a whole group built from its own copy. New entries, and the clean-up
//! a typed value needs (a site reduced to its host, a word lowercased, a
//! trigger stripped), are made here so every shell stores the same thing.

use crate::commands::settings::commit_settings_patch;
use crate::commands::settings_fields::{get_path, patch_for_changes};
use crate::models::settings::{AppState, Settings};
use crate::rt::AppHandle;
use serde_json::{json, Map, Value};
use tauri::State;

/// One change to a list. Entries with an `id` are found by it; the rest by
/// position or value.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ListEdit {
    /// Append `item`, or a fresh entry when the list makes its own.
    Add {
        #[serde(default)]
        item: Option<Value>,
    },
    Remove {
        #[serde(default)]
        index: Option<usize>,
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        value: Option<Value>,
    },
    /// Merge `patch` into one entry's fields.
    Update {
        #[serde(default)]
        index: Option<usize>,
        #[serde(default)]
        id: Option<String>,
        patch: Map<String, Value>,
    },
    /// Move one entry to position `to` (order is match priority).
    Move {
        #[serde(default)]
        index: Option<usize>,
        #[serde(default)]
        id: Option<String>,
        to: usize,
    },
    /// Add `value` to a set, or take it out when it is there.
    Toggle { value: Value },
}

/// The kinds of list the pages edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Trusted link sites: a host, lowercased, without `www.`.
    Hosts,
    /// Spellcheck words: the first word typed, lowercased.
    Words,
    /// Hidden command patterns `{ value, mode }`, duplicates allowed.
    CommandFilters,
    /// Entries with an `id`, made here when added without one.
    Entries(Entries),
    /// A set toggled one value at a time, kept in `order` when one is given.
    Set(&'static [&'static str]),
    /// Twitch logins toggled one at a time, trimmed and lowercased.
    Logins,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Entries {
    HighlightPhrases,
    IgnoredPhrases,
    HighlightUsers,
    HighlightBadges,
    UserCommands,
    /// Renamed in place; added and removed with their files (`add_custom_sound`).
    CustomSounds,
    /// The viewer's own user-card mod buttons (`services::chatter_card`).
    CardActions,
}

/// The scrollbar mark kinds, in the order a saved list keeps, so a list
/// toggled back equals the default.
pub const MARK_KINDS: &[&str] =
    &["mention", "highlight", "pinned", "announcement", "sub", "redeem", "raid", "first_time"];

fn kind_of(path: &str) -> Option<Kind> {
    Some(match path {
        "chat_design.link_preview_trusted_domains" => Kind::Hosts,
        "chat_input.spellcheck_custom_words" => Kind::Words,
        "chat_filters.command_filters" => Kind::CommandFilters,
        "chat_highlights.phrases" => Kind::Entries(Entries::HighlightPhrases),
        "chat_filters.ignored_phrases" => Kind::Entries(Entries::IgnoredPhrases),
        "chat_highlights.users" => Kind::Entries(Entries::HighlightUsers),
        "chat_highlights.badges" => Kind::Entries(Entries::HighlightBadges),
        "chat_commands.user_commands" => Kind::Entries(Entries::UserCommands),
        "chat_events.hidden_provider_events" | "chat_design.hidden_badge_providers" => Kind::Set(&[]),
        "chat_design.scrollbar_mark_kinds" => Kind::Set(MARK_KINDS),
        "live_notifications.muted_live_channels" | "mentions.muted_channels" => Kind::Logins,
        "chat_highlights.custom_sounds" => Kind::Entries(Entries::CustomSounds),
        "moderation.card_actions" => Kind::Entries(Entries::CardActions),
        _ => return None,
    })
}

/// A trusted site as stored: the URL's host, lowercased, `www.` dropped.
/// None when the text is not a URL or host.
pub fn trustable_host(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let full = if raw.starts_with("http") { raw.to_string() } else { format!("https://{raw}") };
    let url = url::Url::parse(&full).ok()?;
    let host = url.host_str()?.to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    (!host.is_empty()).then_some(host)
}

/// A dictionary word as stored: the first word typed, lowercased.
pub fn dictionary_word(raw: &str) -> Option<String> {
    raw.split_whitespace().next().map(str::to_lowercase)
}

/// A custom command trigger as stored: no leading slash, only letters,
/// digits and underscores, lowercased.
pub fn command_trigger(raw: &str) -> String {
    raw.strip_prefix('/')
        .unwrap_or(raw)
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect::<String>()
        .to_lowercase()
}

fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// A fresh entry for a list that makes its own.
fn new_entry(kind: Entries) -> Value {
    match kind {
        Entries::HighlightPhrases => json!({
            "id": new_id(), "pattern": "", "enabled": true, "case_sensitive": false,
            "whole_word": true, "is_regex": false, "color": "#fbbf24", "sound_id": null,
            "cooldown_seconds": 3,
        }),
        Entries::IgnoredPhrases => json!({
            "id": new_id(), "pattern": "", "enabled": true, "case_sensitive": false,
            "whole_word": false, "is_regex": false,
        }),
        Entries::HighlightUsers => json!({ "id": new_id(), "enabled": true, "username": "", "color": "#22d3ee" }),
        Entries::HighlightBadges => json!({
            "id": new_id(), "enabled": true, "badge_key": "", "color": "#a855f7", "label": "",
        }),
        Entries::UserCommands => json!({
            "id": new_id(), "trigger": "", "expansion": "", "description": "", "enabled": true,
            "require_slash": true, "also_match_suffix": false,
        }),
        Entries::CustomSounds => json!({ "id": new_id(), "name": "", "path": "" }),
        Entries::CardActions => json!({ "id": new_id(), "label": "", "kind": "timeout", "seconds": 600, "reason": "" }),
    }
}

/// The clean-up a typed field needs before it is stored.
fn clean_entry(kind: Entries, entry: &mut Map<String, Value>) {
    let text = |entry: &Map<String, Value>, key: &str| entry.get(key).and_then(Value::as_str).map(str::to_string);
    match kind {
        Entries::HighlightUsers => {
            if let Some(name) = text(entry, "username") {
                entry.insert("username".into(), json!(name.trim().to_lowercase()));
            }
        }
        Entries::HighlightBadges => {
            if let Some(key) = text(entry, "badge_key") {
                entry.insert("badge_key".into(), json!(key.trim()));
            }
        }
        Entries::UserCommands => {
            if let Some(trigger) = text(entry, "trigger") {
                entry.insert("trigger".into(), json!(command_trigger(&trigger)));
            }
        }
        Entries::HighlightPhrases => {
            if let Some(cooldown) = entry.get("cooldown_seconds") {
                let seconds = cooldown.as_f64().map(|s| s.round().clamp(0.0, 60.0) as u64).unwrap_or(3);
                entry.insert("cooldown_seconds".into(), json!(seconds));
            }
        }
        Entries::CardActions => {
            if let Some(label) = text(entry, "label") {
                entry.insert("label".into(), json!(label.trim().chars().take(24).collect::<String>()));
            }
            let kind = text(entry, "kind").filter(|k| ["timeout", "ban", "unban", "warn"].contains(&k.as_str()));
            entry.insert("kind".into(), json!(kind.unwrap_or_else(|| "timeout".into())));
            if let Some(seconds) = entry.get("seconds").and_then(Value::as_f64) {
                entry.insert("seconds".into(), json!((seconds.round() as u64).clamp(1, 1_209_600)));
            }
            if let Some(reason) = text(entry, "reason") {
                entry.insert("reason".into(), json!(reason.trim().chars().take(500).collect::<String>()));
            }
        }
        Entries::IgnoredPhrases | Entries::CustomSounds => {}
    }
}

fn position(list: &[Value], index: Option<usize>, id: Option<&str>) -> Option<usize> {
    match id {
        Some(id) => list.iter().position(|e| e.get("id").and_then(Value::as_str) == Some(id)),
        None => index.filter(|i| *i < list.len()),
    }
}

/// The list after `edit`, or an error the page can show. `Ok(None)` means the
/// edit changed nothing (a duplicate, an empty word).
fn apply(kind: Kind, list: &[Value], edit: ListEdit) -> Result<Option<Vec<Value>>, String> {
    let mut list = list.to_vec();
    match edit {
        ListEdit::Add { item } => match kind {
            Kind::Hosts => {
                let raw = item.as_ref().and_then(Value::as_str).unwrap_or_default();
                let host = trustable_host(raw).ok_or_else(|| "That is not a site address.".to_string())?;
                if list.iter().any(|v| v.as_str() == Some(host.as_str())) {
                    return Ok(None);
                }
                list.push(json!(host));
            }
            Kind::Words => {
                let raw = item.as_ref().and_then(Value::as_str).unwrap_or_default();
                let Some(word) = dictionary_word(raw) else { return Ok(None) };
                if list.iter().any(|v| v.as_str().map(str::to_lowercase).as_deref() == Some(word.as_str())) {
                    return Ok(None);
                }
                list.push(json!(word));
            }
            Kind::CommandFilters => {
                let item = item.unwrap_or(Value::Null);
                let value: String = item.get("value").and_then(Value::as_str).unwrap_or_default().trim().chars().take(40).collect();
                if value.is_empty() {
                    return Ok(None);
                }
                let mode = if item.get("mode").and_then(Value::as_str) == Some("exact") { "exact" } else { "prefix" };
                list.push(json!({ "value": value, "mode": mode }));
            }
            Kind::Entries(entries) => {
                let mut entry = match item {
                    Some(Value::Object(seed)) => {
                        let mut fresh = new_entry(entries).as_object().cloned().unwrap_or_default();
                        fresh.extend(seed);
                        fresh
                    }
                    _ => new_entry(entries).as_object().cloned().unwrap_or_default(),
                };
                clean_entry(entries, &mut entry);
                list.push(Value::Object(entry));
            }
            Kind::Set(_) | Kind::Logins => return Err("toggle a set's values instead".to_string()),
        },
        ListEdit::Remove { index, id, value } => {
            let at = match (&value, kind) {
                (Some(value), _) => list.iter().position(|v| v == value),
                _ => position(&list, index, id.as_deref()),
            };
            let Some(at) = at else { return Ok(None) };
            list.remove(at);
        }
        ListEdit::Update { index, id, patch } => {
            let Kind::Entries(entries) = kind else { return Err("entries of this list are not edited in place".to_string()) };
            let Some(at) = position(&list, index, id.as_deref()) else { return Ok(None) };
            let Some(entry) = list[at].as_object_mut() else { return Ok(None) };
            for (key, value) in patch {
                if key == "id" {
                    continue;
                }
                if value.is_null() {
                    entry.remove(&key);
                } else {
                    entry.insert(key, value);
                }
            }
            clean_entry(entries, entry);
        }
        ListEdit::Move { index, id, to } => {
            let Some(at) = position(&list, index, id.as_deref()) else { return Ok(None) };
            let to = to.min(list.len().saturating_sub(1));
            if at == to {
                return Ok(None);
            }
            let entry = list.remove(at);
            list.insert(to, entry);
        }
        ListEdit::Toggle { value } if kind == Kind::Logins => {
            let login = value.as_str().unwrap_or_default().trim().to_lowercase();
            if login.is_empty() {
                return Ok(None);
            }
            let before = list.len();
            list.retain(|v| v.as_str() != Some(login.as_str()));
            if list.len() == before {
                list.push(json!(login));
            }
        }
        ListEdit::Toggle { value } => {
            let Kind::Set(order) = kind else { return Err("this list is not a set".to_string()) };
            if let Some(at) = list.iter().position(|v| v == &value) {
                list.remove(at);
            } else {
                list.push(value);
            }
            if !order.is_empty() {
                let rank = |v: &Value| v.as_str().and_then(|s| order.iter().position(|o| *o == s)).unwrap_or(order.len());
                list.sort_by_key(rank);
            }
        }
    }
    Ok(Some(list))
}

/// The list at `path` with one edit applied, as the dotted-path change that
/// stores it.
fn list_change(current: &Settings, path: &str, edit: ListEdit) -> Result<Option<Vec<Value>>, String> {
    let kind = kind_of(path).ok_or_else(|| format!("{path} is not an editable list"))?;
    let root = serde_json::to_value(current).map_err(|e| e.to_string())?;
    let list = get_path(&root, path).and_then(Value::as_array).cloned().unwrap_or_else(|| {
        // An unsaved mark-kinds list reads as the default set.
        if path == "chat_design.scrollbar_mark_kinds" {
            crate::models::settings::Settings::default().chat_design.scrollbar_mark_kinds.iter().map(|k| json!(k)).collect()
        } else {
            Vec::new()
        }
    });
    apply(kind, &list, edit)
}

/// Apply one edit to a settings list (see `ListEdit`) and answer the list as
/// stored. A refused value (a site that is not an address) is an error with a
/// sentence the page can show.
#[tauri::command]
pub async fn edit_settings_list(
    app: AppHandle,
    path: String,
    edit: ListEdit,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let mut stored = None;
    commit_settings_patch(&app, &state, None, |current| {
        let Some(next) = list_change(current, &path, edit)? else {
            return Ok(Map::new());
        };
        stored = Some(Value::Array(next.clone()));
        let mut changes = Map::new();
        changes.insert(path.clone(), Value::Array(next));
        patch_for_changes(current, &changes)
    })?;
    match stored {
        Some(list) => Ok(list),
        None => {
            let settings = state.settings.lock().map_err(|e| e.to_string())?;
            let root = serde_json::to_value(&*settings).map_err(|e| e.to_string())?;
            Ok(get_path(&root, &path).cloned().unwrap_or_else(|| json!([])))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(path: &str, list: Value, edit: Value) -> Option<Vec<Value>> {
        let edit: ListEdit = serde_json::from_value(edit).unwrap();
        apply(kind_of(path).unwrap(), list.as_array().unwrap(), edit).unwrap()
    }

    #[test]
    fn hosts_are_reduced_and_deduplicated() {
        assert_eq!(trustable_host("https://www.Example.com/path"), Some("example.com".into()));
        assert_eq!(trustable_host("clips.twitch.tv"), Some("clips.twitch.tv".into()));
        assert_eq!(trustable_host("   "), None);
        assert_eq!(trustable_host("not a host"), None);
        let path = "chat_design.link_preview_trusted_domains";
        assert_eq!(run(path, json!([]), json!({"op": "add", "item": "WWW.foo.dev"})), Some(vec![json!("foo.dev")]));
        assert_eq!(run(path, json!(["foo.dev"]), json!({"op": "add", "item": "foo.dev"})), None);
        assert_eq!(run(path, json!(["a.com", "b.com"]), json!({"op": "remove", "value": "a.com"})), Some(vec![json!("b.com")]));
    }

    #[test]
    fn words_keep_the_first_word() {
        let path = "chat_input.spellcheck_custom_words";
        assert_eq!(run(path, json!([]), json!({"op": "add", "item": "  PogChamp lul"})), Some(vec![json!("pogchamp")]));
        assert_eq!(run(path, json!(["Pogchamp"]), json!({"op": "add", "item": "pogchamp"})), None);
        assert_eq!(run(path, json!([]), json!({"op": "add", "item": "   "})), None);
    }

    #[test]
    fn command_filters_trim_and_allow_repeats() {
        let path = "chat_filters.command_filters";
        let one = run(path, json!([]), json!({"op": "add", "item": {"value": " !drops ", "mode": "exact"}})).unwrap();
        assert_eq!(one, vec![json!({"value": "!drops", "mode": "exact"})]);
        let two = run(path, Value::Array(one), json!({"op": "add", "item": {"value": "!drops"}})).unwrap();
        assert_eq!(two.len(), 2);
        assert_eq!(two[1]["mode"], "prefix");
    }

    #[test]
    fn entries_are_made_cleaned_moved_and_removed() {
        let path = "chat_commands.user_commands";
        let list = run(path, json!([]), json!({"op": "add"})).unwrap();
        let id = list[0]["id"].as_str().unwrap().to_string();
        assert_eq!(list[0]["require_slash"], true);
        let list = run(path, Value::Array(list), json!({"op": "update", "id": id, "patch": {"trigger": "/Lurk Now!"}})).unwrap();
        assert_eq!(list[0]["trigger"], "lurknow");
        let list = run(path, Value::Array(list), json!({"op": "add"})).unwrap();
        let moved = run(path, Value::Array(list.clone()), json!({"op": "move", "id": id, "to": 1})).unwrap();
        assert_eq!(moved[1]["id"], json!(id));
        let left = run(path, Value::Array(moved), json!({"op": "remove", "id": id})).unwrap();
        assert_eq!(left.len(), 1);

        let users = run("chat_highlights.users", json!([]), json!({"op": "add", "item": {"username": " Bob "}})).unwrap();
        assert_eq!(users[0]["username"], "bob");
        assert_eq!(users[0]["color"], "#22d3ee");
        let phrases = run("chat_highlights.phrases", json!([]), json!({"op": "add"})).unwrap();
        assert_eq!(phrases[0]["whole_word"], true);
        assert_eq!(phrases[0]["cooldown_seconds"], 3);
    }

    #[test]
    fn mark_kinds_keep_their_order() {
        let path = "chat_design.scrollbar_mark_kinds";
        let off = run(path, json!(["mention", "highlight", "pinned"]), json!({"op": "toggle", "value": "highlight"})).unwrap();
        assert_eq!(off, vec![json!("mention"), json!("pinned")]);
        let back = run(path, Value::Array(off), json!({"op": "toggle", "value": "highlight"})).unwrap();
        assert_eq!(back, vec![json!("mention"), json!("highlight"), json!("pinned")]);
        let muted = run("live_notifications.muted_live_channels", json!(["xqc"]), json!({"op": "toggle", "value": " XQC "})).unwrap();
        assert!(muted.is_empty());
        let muted = run("live_notifications.muted_live_channels", json!([]), json!({"op": "toggle", "value": "Shroud"})).unwrap();
        assert_eq!(muted, vec![json!("shroud")]);
        let sets = run("chat_events.hidden_provider_events", json!([]), json!({"op": "toggle", "value": "kick:follows"})).unwrap();
        assert_eq!(sets, vec![json!("kick:follows")]);
    }

    #[test]
    fn unknown_lists_are_refused() {
        assert!(kind_of("accounts").is_none());
    }
}
