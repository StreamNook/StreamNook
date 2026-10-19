//! Field-level settings edits for the settings pages.
//!
//! Every settings page used to build a whole group (`chat_design`,
//! `video_player`, ...) from its own copy and save it, so a stale copy wrote
//! back keys another surface had just changed. These commands change one
//! dotted path at a time, or one entry of a list, on the canonical settings
//! Rust holds, and answer which paths differ from a fresh install so a page
//! can draw its reset arrows without a copy of the defaults.

use crate::commands::settings::commit_settings_patch;
use crate::models::settings::{AppState, Settings};
use crate::rt::AppHandle;
use serde_json::{Map, Value};
use tauri::State;

/// What a settings page draws from: every value (frontend-owned groups filled
/// with their defaults where absent) and the dotted paths that differ from a
/// fresh install.
#[derive(Clone, Debug, serde::Serialize)]
pub struct SettingsView {
    pub values: Value,
    /// Leaf paths whose value differs from the default. A list is one leaf.
    /// A page's row shows its reset arrow when one of its paths equals, or
    /// is a prefix of, one of these.
    pub changed: Vec<String>,
}

/// The defaults of the groups the frontend owns (they ride `extra`, so
/// `Settings::default()` has no value for them). These are the values the
/// pages fell back to when a key was absent; declaring them here lets every
/// shell reset them and compare against them without its own copy.
pub fn frontend_owned_defaults() -> Value {
    serde_json::json!({
        "show_polls": true,
        "show_predictions": true,
        "chat_overlay_order": "prediction-first",
        "show_channel_point_redemptions": true,
        "collapse_gift_subs": true,
        "clip_chat_replay": true,
        "auto_claim_points_watching": true,
        "oled_accent": "#a064ff",
        // Empty follows the device language (services/browse_languages.rs).
        "discovery_languages": [],
        "fullscreen_chat": {
            "phone_layout": "beside",
            "opacity": 55,
            "width": 340,
            "side": "auto",
        },
        "chat_events": {
            "superchat_currency": "",
            "event_style": "cards",
            "event_outline_color": "",
            "event_animation": "none",
            "event_animate_repeat": false,
            "cheer_display": "card",
            "hidden_provider_events": [],
        },
        "chat_render": {
            "smooth_scroll_on_resume": true,
            "message_buffer_cap": 100,
        },
        "moderation": {
            "timeout_presets": [1, 600, 3600, 86400],
            "card_actions": [],
        },
        "message_repeat": {
            "mode": "off",
            "match": "normalized",
            "threshold": 2,
            "window_seconds": 60,
            "color": "#8b8b8b",
            "exempt_privileged": true,
            "keep_all_when_moderator": true,
        },
        "chat_filters": {
            "hide_bots": false,
            "hide_commands": false,
        },
        "chat_highlights": {
            "appearance": {
                "display_style": "standard",
                "opacity": 20,
            },
            // What `chat_rules` reads for a rule never saved.
            "built_in": {
                "first_time_chatter": {
                    "enabled": true, "color": "#a855f7", "style": "wash", "fill": false,
                    "animation": "none", "animate_repeat": false,
                },
                "returning_chatter": { "enabled": false, "color": "#22d3ee" },
                "self_message": { "enabled": false, "color": "#facc15" },
                "raider": { "enabled": false, "color": "#ef4444" },
            },
        },
        // Typed groups whose optional fields stay absent until chosen.
        "chat_design": {
            "polls_start_collapsed": false,
            "pinned_start_collapsed": true,
            "mention_sound": "",
            "mention_sound_volume": 100,
            "mention_sound_replies": true,
            "mention_weight": "medium",
            "mention_italic": "inherit",
            "mention_style": "plain",
            "chat_dock_switcher": "menu",
        },
        "cosmetics": {
            "paint_shadows": "all",
        },
        // The composer keys `ChatInputSettings` does not name.
        "chat_input": {
            "bypass_duplicate": false,
            "spellcheck_enabled": true,
            "spellcheck_custom_words": [],
            "hide_placeholder": false,
            "hide_command_button": false,
            "hide_emote_button": false,
            "hide_points_balance": false,
            "show_points_balance_inline": false,
            "emote_tab_complete_enabled": true,
            "emote_tab_complete_match_mode": "starts_with",
            "emote_tab_complete_include_chatters": true,
        },
    })
}

/// The defaults a settings page compares against and resets to: Rust's
/// `Settings::default()` with the frontend-owned defaults filled in.
pub fn default_settings_value() -> Value {
    let mut base = serde_json::to_value(Settings::default()).unwrap_or_else(|_| Value::Object(Map::new()));
    fill_absent(&mut base, &frontend_owned_defaults());
    base
}

/// Copies every key of `defaults` that `target` lacks (or holds as null),
/// recursing into objects both sides hold.
pub fn fill_absent(target: &mut Value, defaults: &Value) {
    let (Some(target), Some(defaults)) = (target.as_object_mut(), defaults.as_object()) else {
        return;
    };
    for (key, default) in defaults {
        match target.get_mut(key) {
            Some(existing) if !existing.is_null() => {
                if existing.is_object() && default.is_object() {
                    fill_absent(existing, default);
                }
            }
            _ => {
                target.insert(key.clone(), default.clone());
            }
        }
    }
}

/// The value at a dotted path, if every step exists.
pub fn get_path<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(root, |node, step| node.as_object()?.get(step))
}

/// Sets the value at a dotted path, creating objects on the way; `null`
/// removes the last step. A step through a non-object replaces it with one.
pub fn set_path(root: &mut Value, path: &str, value: Value) {
    let steps: Vec<&str> = path.split('.').filter(|s| !s.is_empty()).collect();
    let Some((last, parents)) = steps.split_last() else { return };
    let mut node = root;
    for step in parents {
        if !node.is_object() {
            *node = Value::Object(Map::new());
        }
        node = node.as_object_mut().expect("an object").entry(step.to_string()).or_insert(Value::Null);
        if !node.is_object() {
            *node = Value::Object(Map::new());
        }
    }
    if !node.is_object() {
        *node = Value::Object(Map::new());
    }
    let obj = node.as_object_mut().expect("an object");
    if value.is_null() {
        obj.remove(*last);
    } else {
        obj.insert(last.to_string(), value);
    }
}

/// Settings equality the way a page reads it: an absent key equals null,
/// numbers compare by value (1 and 1.0 match), arrays element by element and
/// objects key by key in any order.
pub fn json_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => match (x.as_f64(), y.as_f64()) {
            (Some(x), Some(y)) => (x - y).abs() < 1e-9,
            _ => x == y,
        },
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(x, y)| json_eq(x, y)),
        (Value::Object(x), Value::Object(y)) => x
            .keys()
            .chain(y.keys())
            .all(|k| json_eq(x.get(k).unwrap_or(&Value::Null), y.get(k).unwrap_or(&Value::Null))),
        _ => a == b,
    }
}

/// Every leaf path where `current` differs from `default`. Objects held on
/// both sides are walked; anything else (a list, a value, an object on one
/// side only) is one leaf.
pub fn changed_paths(current: &Value, default: &Value) -> Vec<String> {
    fn walk(current: &Value, default: &Value, prefix: &str, out: &mut Vec<String>) {
        if let (Some(c), Some(d)) = (current.as_object(), default.as_object()) {
            let mut keys: Vec<&String> = c.keys().chain(d.keys()).collect();
            keys.sort();
            keys.dedup();
            for key in keys {
                let path = if prefix.is_empty() { key.clone() } else { format!("{prefix}.{key}") };
                walk(c.get(key).unwrap_or(&Value::Null), d.get(key).unwrap_or(&Value::Null), &path, out);
            }
        } else if !json_eq(current, default) && !prefix.is_empty() {
            out.push(prefix.to_string());
        }
    }
    let mut out = Vec::new();
    walk(current, default, "", &mut out);
    out
}

/// The settings with dotted-path `changes` applied, as the top-level patch
/// that carries them: one entry per top-level key a change touched, holding
/// that key's whole new value (or null when the change removed it).
pub fn patch_for_changes(current: &Settings, changes: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let mut root = serde_json::to_value(current).map_err(|e| e.to_string())?;
    let mut touched: Vec<String> = Vec::new();
    for (path, value) in changes {
        let top = path.split('.').next().unwrap_or_default().to_string();
        if top.is_empty() {
            return Err("an empty settings path".to_string());
        }
        set_path(&mut root, path, value.clone());
        if !touched.contains(&top) {
            touched.push(top);
        }
    }
    let mut patch = Map::new();
    for top in touched {
        patch.insert(top.clone(), root.get(&top).cloned().unwrap_or(Value::Null));
    }
    Ok(patch)
}

/// Read the values and the changed paths a settings page draws from.
#[tauri::command]
pub async fn get_settings_view(state: State<'_, AppState>) -> Result<SettingsView, String> {
    let current = {
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        serde_json::to_value(&*settings).map_err(|e| e.to_string())?
    };
    Ok(settings_view(current))
}

/// The view of a serialized settings object (values filled, changed paths).
pub fn get_settings_view_from(current: Value) -> SettingsView {
    settings_view(current)
}

fn settings_view(mut current: Value) -> SettingsView {
    // An absent frontend-owned key reads as its default, so it is filled
    // before comparing: only a value someone chose shows as changed.
    fill_absent(&mut current, &frontend_owned_defaults());
    let changed = changed_paths(&current, &default_settings_value());
    SettingsView { values: current, changed }
}

/// Set one or more settings by dotted path (`chat_design.font_size`), each a
/// read-modify-write on the canonical settings: every other key of the group
/// is kept as Rust holds it. A null value removes the key.
#[tauri::command]
pub async fn set_settings_fields(
    app: AppHandle,
    changes: Map<String, Value>,
    source: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if changes.is_empty() {
        return Ok(());
    }
    commit_settings_patch(&app, &state, source, |current| patch_for_changes(current, &changes))
}

/// Put the given dotted paths back to a fresh install's values (removing the
/// key where a fresh install has none), as one write.
#[tauri::command]
pub async fn reset_settings(
    app: AppHandle,
    paths: Vec<String>,
    source: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    let defaults = default_settings_value();
    let changes: Map<String, Value> = paths
        .into_iter()
        .map(|path| {
            let value = get_path(&defaults, &path).cloned().unwrap_or(Value::Null);
            (path, value)
        })
        .collect();
    commit_settings_patch(&app, &state, source, |current| patch_for_changes(current, &changes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn set_path_creates_and_removes() {
        let mut root = json!({ "a": { "b": 1 } });
        set_path(&mut root, "a.c.d", json!(true));
        assert_eq!(root, json!({ "a": { "b": 1, "c": { "d": true } } }));
        set_path(&mut root, "a.b", Value::Null);
        assert_eq!(root, json!({ "a": { "c": { "d": true } } }));
        assert_eq!(get_path(&root, "a.c.d"), Some(&json!(true)));
        assert_eq!(get_path(&root, "a.x.d"), None);
    }

    #[test]
    fn json_eq_reads_like_a_page() {
        assert!(json_eq(&json!(1), &json!(1.0)));
        assert!(json_eq(&json!({ "a": null }), &json!({})));
        assert!(json_eq(&json!({ "a": 1, "b": 2 }), &json!({ "b": 2, "a": 1 })));
        assert!(!json_eq(&json!([1, 2]), &json!([2, 1])));
    }

    #[test]
    fn changed_paths_lists_leaves() {
        let current = json!({ "chat_design": { "font_size": 18, "show_badges": true }, "list": [1], "extra": { "x": 1 } });
        let default = json!({ "chat_design": { "font_size": 14.0, "show_badges": true }, "list": [1] });
        assert_eq!(changed_paths(&current, &default), vec!["chat_design.font_size", "extra"]);
    }

    #[test]
    fn a_fresh_install_differs_nowhere() {
        let fresh = serde_json::to_value(Settings::default()).unwrap();
        let view = settings_view(fresh);
        assert!(view.changed.is_empty(), "a fresh install shows reset arrows on {:?}", view.changed);
        assert_eq!(get_path(&view.values, "message_repeat.threshold"), Some(&json!(2)));
    }

    #[test]
    fn a_field_change_keeps_the_rest_of_its_group() {
        let current = Settings::default();
        let before = current.chat_design.show_badges;
        let mut changes = Map::new();
        changes.insert("chat_design.font_size".into(), json!(20));
        changes.insert("fullscreen_chat.side".into(), json!("left"));
        let patch = patch_for_changes(&current, &changes).unwrap();
        assert_eq!(patch.len(), 2);
        assert_eq!(get_path(&patch["chat_design"], "font_size"), Some(&json!(20)));
        assert_eq!(get_path(&patch["chat_design"], "show_badges"), Some(&json!(before)));
        assert_eq!(patch["fullscreen_chat"], json!({ "side": "left" }));
    }
}
