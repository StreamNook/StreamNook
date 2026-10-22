//! Overlay files: a stream overlay's look, saved to a file one streamer can hand
//! to another.
//!
//! The file carries the style and a suggested name, nothing else. The overlay's
//! link id, its channels, its per-channel blocked users and anything about the
//! account stay behind, so opening a shared file can never point someone at the
//! sender's chat or reach the sender's published overlay. Opening a file always
//! makes a NEW draft overlay; it gets its own link the first time it is
//! published.
//!
//! Every field is checked against the overlay style's real shape (the
//! `OverlayStyle` interface in src/components/overlay/overlayConfig.ts): wrong
//! types, unknown keys and values outside the builder's limits are dropped or
//! pulled into range, and the whole file is size-capped. The same pass runs on
//! export, so a file written here is already clean. A test keeps the field table
//! in step with the TypeScript interface.

use crate::rt::{AppHandle, Rt, Window};
use serde::Serialize;
use serde_json::{Map, Value};

/// The `format` value every overlay file carries.
pub const OVERLAY_FILE_FORMAT: &str = "streamnook-overlay";
/// The newest file version this build reads and the one it writes.
pub const OVERLAY_FILE_VERSION: u64 = 1;
/// File extension, without the dot.
pub const OVERLAY_FILE_EXTENSION: &str = "snoverlay";
/// Hard cap on a file's size. A full style is a few KB; the server's own cap
/// on a stored style is 16 KB.
pub const MAX_OVERLAY_FILE_BYTES: usize = 64 * 1024;

const MAX_NAME_CHARS: usize = 60;
const DEFAULT_NAME: &str = "Imported overlay";
/// Longest hidden phrase or command filter a file carries
/// (`OVERLAY_FILE_TEXT_ENTRY_MAX` in overlayConfig.ts). The builder caps new
/// entries lower, but older ones can be longer: they travel whole up to this
/// length, and a longer one is left out and reported. The web builder reads
/// files with the same rule (`profileSync.ts`).
const TEXT_ENTRY_MAX: usize = 1000;
/// Most entries a phrase or command list keeps.
const MAX_LIST_ENTRIES: usize = 100;

const PROVIDERS: &[&str] = &["twitch", "kick", "youtube", "tiktok"];
const EVENT_CATEGORIES: &[&str] = &[
    "subscription",
    "gift",
    "raid",
    "cheer",
    "milestone",
    "redemption",
    "follow",
    "announcement",
];
const ANIMATIONS: &[&str] = &["none", "sheen", "pulse", "chase"];
const ALIGNS: &[&str] = &["left", "center", "right"];

/// Style keys that exist but never travel in a file: `blockedUsers` is keyed by
/// the sender's own channels and lists people they blocked, and `profileName`
/// is the published copy of the overlay's name (the file has its own `name`).
const NEVER_EXPORTED: &[&str] = &["blockedUsers", "profileName"];

/// What one style field must be.
#[derive(Clone, Copy)]
enum Kind {
    Bool,
    /// A number pulled into [min, max]; `step` > 0 rounds to that step.
    Num { min: f64, max: f64, step: f64 },
    /// One of a fixed set of strings.
    Choice(&'static [&'static str]),
    /// A CSS color. `allow_empty` = '' means "use the default".
    Color { allow_empty: bool },
    /// A CSS font-family list.
    Font,
    /// '' or a three-letter ISO currency code.
    Currency,
    /// Strings from a fixed set.
    ChoiceList(&'static [&'static str]),
    /// Short ids (`[a-z0-9_-]`), e.g. badge providers or badge categories.
    IdList { max_items: usize },
    /// Free text entries, trimmed and de-duplicated (see `list_text`).
    TextList,
    /// `provider:category` keys.
    ProviderEvents,
    /// `{ value, mode }` command filters.
    CommandFilters,
    /// Category -> template text.
    EventTemplates,
}

/// Every key of `OverlayStyle` that a file may carry, with its shape. Kept in
/// step with the TypeScript interface by `style_fields_match_the_typescript_type`.
const FIELDS: &[(&str, Kind)] = &[
    ("width", Kind::Num { min: 160.0, max: 900.0, step: 1.0 }),
    ("height", Kind::Num { min: 80.0, max: 1600.0, step: 1.0 }),
    ("sources", Kind::ChoiceList(PROVIDERS)),
    ("sourceTag", Kind::Choice(&["none", "dot", "label", "icon"])),
    ("fontFamily", Kind::Font),
    ("fontSize", Kind::Num { min: 10.0, max: 48.0, step: 0.0 }),
    ("lineHeight", Kind::Num { min: 1.0, max: 2.2, step: 0.0 }),
    ("messageGap", Kind::Num { min: 0.0, max: 28.0, step: 0.0 }),
    ("emoteScale", Kind::Num { min: 0.5, max: 3.0, step: 0.0 }),
    ("badgeScale", Kind::Num { min: 0.5, max: 2.5, step: 0.0 }),
    ("showBadges", Kind::Bool),
    ("showTimestamps", Kind::Bool),
    ("bodyTextColor", Kind::Color { allow_empty: false }),
    ("textShadow", Kind::Bool),
    ("textShadowColor", Kind::Color { allow_empty: false }),
    ("textShadowSize", Kind::Num { min: 0.0, max: 12.0, step: 0.0 }),
    ("textShadowOpacity", Kind::Num { min: 0.0, max: 1.0, step: 0.0 }),
    ("shadowOnText", Kind::Bool),
    ("shadowOnNames", Kind::Bool),
    ("shadowOnBadges", Kind::Bool),
    ("shadowDarkText", Kind::Bool),
    ("shadowDarkNames", Kind::Bool),
    ("shadowDarkBadges", Kind::Bool),
    ("textAlign", Kind::Choice(ALIGNS)),
    ("fontWeight", Kind::Num { min: 300.0, max: 700.0, step: 100.0 }),
    ("textItalic", Kind::Bool),
    ("textStrikethrough", Kind::Bool),
    ("background", Kind::Choice(&["transparent", "solid"])),
    ("backgroundColor", Kind::Color { allow_empty: false }),
    ("backgroundOpacity", Kind::Num { min: 0.0, max: 1.0, step: 0.0 }),
    ("eventStyle", Kind::Choice(&["plain", "streamnook", "outline"])),
    ("eventFill", Kind::Bool),
    ("eventAnimation", Kind::Choice(ANIMATIONS)),
    ("eventAnimateRepeat", Kind::Bool),
    ("cheerDisplay", Kind::Choice(&["message", "event"])),
    ("seventvEmoteChanges", Kind::Bool),
    ("hideBots", Kind::Bool),
    ("hiddenEvents", Kind::ChoiceList(EVENT_CATEGORIES)),
    ("hiddenProviderEvents", Kind::ProviderEvents),
    ("superchatCurrency", Kind::Currency),
    ("hideCommands", Kind::Bool),
    ("commandFilters", Kind::CommandFilters),
    ("showPaints", Kind::Bool),
    ("showThirdPartyBadges", Kind::Bool),
    ("showAtmospheres", Kind::Bool),
    ("hiddenBadgeProviders", Kind::IdList { max_items: 32 }),
    ("hiddenTwitchBadgeCategories", Kind::IdList { max_items: 32 }),
    ("ffzCustomRoleBadges", Kind::Bool),
    ("emojiStyle", Kind::Choice(&["system", "apple", "google", "twitter", "facebook"])),
    ("showAvatars", Kind::Bool),
    ("showAtSign", Kind::Bool),
    ("readableNameColors", Kind::Bool),
    ("showReplies", Kind::Bool),
    ("replyStyle", Kind::Choice(&["full", "mention", "off"])),
    ("linkStyle", Kind::Choice(&["accent", "plain"])),
    ("linkColor", Kind::Color { allow_empty: true }),
    ("linkUnderline", Kind::Bool),
    ("showPersonalEmotes", Kind::Bool),
    ("eventTemplates", Kind::EventTemplates),
    ("firstTimeStyle", Kind::Choice(&["off", "twitch", "streamnook"])),
    ("firstTimeFill", Kind::Bool),
    ("firstTimeAnimation", Kind::Choice(ANIMATIONS)),
    ("firstTimeAnimateRepeat", Kind::Bool),
    ("direction", Kind::Choice(&["newBottom", "newTop"])),
    ("entrance", Kind::Choice(&["none", "fade", "slide", "drift", "rise", "pop", "stamp"])),
    ("bubble", Kind::Bool),
    ("bubbleShape", Kind::Choice(&["rounded", "pill", "speech"])),
    ("bubbleRadius", Kind::Num { min: 0.0, max: 24.0, step: 1.0 }),
    ("bubbleColor", Kind::Color { allow_empty: false }),
    ("bubbleOpacity", Kind::Num { min: 0.05, max: 1.0, step: 0.0 }),
    ("firstTimeColor", Kind::Color { allow_empty: true }),
    ("eventOutlineColor", Kind::Color { allow_empty: true }),
    ("hidePhrases", Kind::TextList),
    ("maxMessageAgeSec", Kind::Num { min: 0.0, max: 600.0, step: 1.0 }),
    ("maxMessageLines", Kind::Num { min: 0.0, max: 6.0, step: 1.0 }),
    ("restoreOnReload", Kind::Bool),
    ("loadRecentChat", Kind::Bool),
    ("modCommands", Kind::Bool),
    ("giantEmotes", Kind::Bool),
    ("giantEmoteAlign", Kind::Choice(&["left", "center", "right", "inline"])),
    ("showGifs", Kind::Bool),
    ("gifScale", Kind::Num { min: 0.5, max: 2.0, step: 0.0 }),
    ("showUploadedImages", Kind::Bool),
    ("uploadedImageScale", Kind::Num { min: 0.5, max: 3.0, step: 0.0 }),
    ("uploadedImagesFrom", Kind::Choice(&["trusted", "everyone"])),
];

fn has_control(s: &str) -> bool {
    s.chars().any(char::is_control)
}

fn clean_text(v: &Value, max_chars: usize) -> Option<String> {
    let s = v.as_str()?.trim();
    if s.is_empty() || has_control(s) {
        return None;
    }
    Some(s.chars().take(max_chars).collect())
}

/// One phrase or command text: trimmed, no control characters, at most
/// `TEXT_ENTRY_MAX` characters. `Some("")` is an empty entry (skipped quietly),
/// `None` one that is left out and reported.
fn list_text(v: Option<&Value>) -> Option<String> {
    let s = v?.as_str()?.trim();
    if s.is_empty() {
        return Some(String::new());
    }
    if has_control(s) || s.chars().count() > TEXT_ENTRY_MAX {
        return None;
    }
    Some(s.to_string())
}

fn is_color(s: &str) -> bool {
    if s.len() > 64 {
        return false;
    }
    if let Some(hex) = s.strip_prefix('#') {
        return matches!(hex.len(), 3 | 4 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit());
    }
    let lower = s.to_ascii_lowercase();
    for prefix in ["rgba(", "rgb(", "hsla(", "hsl("] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            return rest.ends_with(')')
                && rest[..rest.len() - 1]
                    .chars()
                    .all(|c| c.is_ascii_digit() || matches!(c, ' ' | ',' | '.' | '%' | '/' | '-'));
        }
    }
    false
}

/// A font-family list the overlay can put in CSS as is: a bounded length, no
/// control characters, and none of the characters that end a declaration or
/// open a function (`url(`, `var(`).
fn is_font_family(s: &str) -> bool {
    !s.trim().is_empty()
        && s.chars().count() <= 200
        && !has_control(s)
        && !s.chars().any(|c| matches!(c, ';' | '{' | '}' | '<' | '>' | '\\' | '(' | ')'))
}

fn is_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 32 && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// One field's value in the shape the overlay expects, or None to drop it.
/// `lossy` is set when a list kept the field but left an entry out that the
/// streamer should hear about.
fn sanitize_field(kind: Kind, v: &Value, lossy: &mut bool) -> Option<Value> {
    match kind {
        Kind::Bool => v.as_bool().map(Value::Bool),
        Kind::Num { min, max, step } => {
            let n = v.as_f64().filter(|n| n.is_finite())?;
            let mut n = n.clamp(min, max);
            if step > 0.0 {
                n = ((n / step).round() * step).clamp(min, max);
            }
            serde_json::Number::from_f64(n).map(|num| {
                // Whole numbers go out as integers, the way the builder writes them.
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    Value::from(n as i64)
                } else {
                    Value::Number(num)
                }
            })
        }
        Kind::Choice(set) => v.as_str().filter(|s| set.contains(s)).map(|s| Value::from(s)),
        Kind::Color { allow_empty } => {
            let s = v.as_str()?.trim();
            if s.is_empty() {
                return allow_empty.then(|| Value::from(""));
            }
            is_color(s).then(|| Value::from(s))
        }
        Kind::Font => v.as_str().filter(|s| is_font_family(s)).map(|s| Value::from(s.trim())),
        Kind::Currency => {
            let s = v.as_str()?.trim();
            (s.is_empty() || (s.len() == 3 && s.chars().all(|c| c.is_ascii_uppercase()))).then(|| Value::from(s))
        }
        Kind::ChoiceList(set) => {
            let mut out: Vec<Value> = Vec::new();
            for item in v.as_array()? {
                if let Some(s) = item.as_str().filter(|s| set.contains(s)) {
                    if !out.iter().any(|o| o == s) {
                        out.push(Value::from(s));
                    }
                }
            }
            Some(Value::Array(out))
        }
        Kind::IdList { max_items } => {
            let mut out: Vec<Value> = Vec::new();
            for item in v.as_array()? {
                if let Some(s) = item.as_str().filter(|s| is_id(s)) {
                    if !out.iter().any(|o| o == s) {
                        out.push(Value::from(s));
                    }
                }
                if out.len() >= max_items {
                    break;
                }
            }
            Some(Value::Array(out))
        }
        Kind::TextList => {
            let mut out: Vec<Value> = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for item in v.as_array()? {
                match list_text(Some(item)) {
                    None => *lossy = true,
                    Some(s) if !s.is_empty() && seen.insert(s.to_lowercase()) => out.push(Value::from(s)),
                    Some(_) => {}
                }
                if out.len() >= MAX_LIST_ENTRIES {
                    break;
                }
            }
            Some(Value::Array(out))
        }
        Kind::ProviderEvents => {
            let mut out: Vec<Value> = Vec::new();
            for item in v.as_array()? {
                let Some(s) = item.as_str() else { continue };
                let Some((p, c)) = s.split_once(':') else { continue };
                if PROVIDERS.contains(&p) && EVENT_CATEGORIES.contains(&c) && !out.iter().any(|o| o == s) {
                    out.push(Value::from(s));
                }
            }
            Some(Value::Array(out))
        }
        Kind::CommandFilters => {
            let mut out: Vec<Value> = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for item in v.as_array()? {
                let obj = item.as_object();
                match obj.and_then(|o| list_text(o.get("value"))) {
                    None => *lossy = true,
                    Some(value) if !value.is_empty() => {
                        let mode = match obj.and_then(|o| o.get("mode")).and_then(Value::as_str) {
                            Some("exact") => "exact",
                            _ => "prefix",
                        };
                        if seen.insert(format!("{mode}:{}", value.to_lowercase())) {
                            out.push(serde_json::json!({ "value": value, "mode": mode }));
                        }
                    }
                    Some(_) => {}
                }
                if out.len() >= MAX_LIST_ENTRIES {
                    break;
                }
            }
            Some(Value::Array(out))
        }
        Kind::EventTemplates => {
            let mut out = Map::new();
            for (k, val) in v.as_object()? {
                if EVENT_CATEGORIES.contains(&k.as_str()) {
                    if let Some(text) = clean_text(val, 200) {
                        out.insert(k.clone(), Value::from(text));
                    }
                }
            }
            Some(Value::Object(out))
        }
    }
}

/// A style object reduced to the fields an overlay file may carry, each in the
/// shape the overlay expects. Returns the clean style and the keys that were
/// left out, whole (unknown, never exported, or invalid) or in part (a list
/// entry that could not be used).
pub fn sanitize_style(raw: &Value) -> (Map<String, Value>, Vec<String>) {
    let mut style = Map::new();
    let mut dropped = Vec::new();
    let Some(obj) = raw.as_object() else {
        return (style, dropped);
    };
    for (key, value) in obj {
        match FIELDS.iter().find(|(k, _)| k == key) {
            Some((_, kind)) => {
                let mut lossy = false;
                match sanitize_field(*kind, value, &mut lossy) {
                    Some(clean) => {
                        style.insert(key.clone(), clean);
                        if lossy {
                            dropped.push(key.clone());
                        }
                    }
                    None => dropped.push(key.clone()),
                }
            }
            None => dropped.push(key.clone()),
        }
    }
    dropped.sort();
    (style, dropped)
}

/// An overlay name fit for the profile list: trimmed, no control characters,
/// bounded, never empty.
pub fn sanitize_name(raw: Option<&Value>) -> String {
    let name: String = raw
        .and_then(Value::as_str)
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(MAX_NAME_CHARS)
        .collect();
    let name = name.trim().to_string();
    if name.is_empty() {
        DEFAULT_NAME.to_string()
    } else {
        name
    }
}

/// The file's text for an overlay.
pub fn build_overlay_file(name: &str, style: &Value) -> String {
    let (clean, _) = sanitize_style(style);
    let doc = serde_json::json!({
        "format": OVERLAY_FILE_FORMAT,
        "version": OVERLAY_FILE_VERSION,
        "name": sanitize_name(Some(&Value::from(name))),
        "style": Value::Object(clean),
    });
    serde_json::to_string_pretty(&doc).unwrap_or_default()
}

/// What opening an overlay file gives the builder.
#[derive(Debug, Serialize, PartialEq)]
pub struct ImportedOverlay {
    pub name: String,
    pub style: Map<String, Value>,
    /// Style keys in the file that were left out, whole or in part.
    pub dropped: Vec<String>,
}

/// Read an overlay file's text. Refuses anything that is not an overlay file,
/// a file from a newer format version, and anything over the size cap.
pub fn parse_overlay_file(text: &str) -> Result<ImportedOverlay, String> {
    if text.len() > MAX_OVERLAY_FILE_BYTES {
        return Err("That file is too large to be an overlay file.".into());
    }
    let doc: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|_| "That file is not a StreamNook overlay file.".to_string())?;
    let obj = doc.as_object().ok_or("That file is not a StreamNook overlay file.")?;
    if obj.get("format").and_then(Value::as_str) != Some(OVERLAY_FILE_FORMAT) {
        return Err("That file is not a StreamNook overlay file.".into());
    }
    let version = obj.get("version").and_then(Value::as_u64).ok_or("That overlay file has no version.")?;
    if version == 0 {
        return Err("That overlay file has no version.".into());
    }
    if version > OVERLAY_FILE_VERSION {
        return Err("That overlay file was made by a newer StreamNook. Update StreamNook to open it.".into());
    }
    let style = obj.get("style").filter(|s| s.is_object()).ok_or("That overlay file has no settings in it.")?;
    let (style, dropped) = sanitize_style(style);
    Ok(ImportedOverlay { name: sanitize_name(obj.get("name")), style, dropped })
}

/// A file name for an overlay: the name with the characters Windows refuses
/// taken out, plus the extension.
fn file_name_for(name: &str) -> String {
    let stem: String = sanitize_name(Some(&Value::from(name)))
        .chars()
        .map(|c| if matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { ' ' } else { c })
        .collect();
    let stem = stem.trim().trim_end_matches('.');
    let stem = if stem.is_empty() { "overlay" } else { stem };
    format!("{stem}.{OVERLAY_FILE_EXTENSION}")
}

/// A file dialog, modal to the window that asked on Windows and macOS, so the
/// builder cannot change underneath it while it is open.
#[cfg(not(target_os = "ios"))]
fn file_dialog(app: &AppHandle, window: &Window) -> tauri_plugin_dialog::FileDialogBuilder<Rt> {
    use tauri_plugin_dialog::DialogExt;
    let dialog = app.dialog().file();
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    let dialog = dialog.set_parent(window);
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let _ = window;
    dialog
}

/// Save an overlay's settings to a file the streamer picks. Returns the file
/// name written, or None when the save dialog was cancelled.
#[cfg(not(target_os = "ios"))]
#[tauri::command]
pub async fn export_overlay_file(app: AppHandle, window: Window, name: String, style: Value) -> Result<Option<String>, String> {
    let text = build_overlay_file(&name, &style);
    let (tx, rx) = tokio::sync::oneshot::channel();
    file_dialog(&app, &window)
        .set_title("Export overlay")
        .set_file_name(file_name_for(&name))
        .add_filter("StreamNook overlay", &[OVERLAY_FILE_EXTENSION])
        .save_file(move |picked| {
            let _ = tx.send(picked);
        });
    let Some(picked) = rx.await.map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let mut path = picked.into_path().map_err(|e| e.to_string())?;
    if path.extension().is_none() {
        path.set_extension(OVERLAY_FILE_EXTENSION);
    }
    tokio::fs::write(&path, text)
        .await
        .map_err(|e| format!("Could not save the overlay file: {e}"))?;
    Ok(Some(path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()))
}

#[cfg(target_os = "ios")]
#[tauri::command]
pub async fn export_overlay_file(_app: AppHandle, _window: Window, _name: String, _style: Value) -> Result<Option<String>, String> {
    Err(crate::ios_ffi::not_wired("Exporting overlay files"))
}

/// Open an overlay file the streamer picks and return its checked settings.
/// None when the open dialog was cancelled.
#[cfg(not(target_os = "ios"))]
#[tauri::command]
pub async fn import_overlay_file(app: AppHandle, window: Window) -> Result<Option<ImportedOverlay>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    file_dialog(&app, &window)
        .set_title("Import overlay")
        .add_filter("StreamNook overlay", &[OVERLAY_FILE_EXTENSION, "json"])
        .pick_file(move |picked| {
            let _ = tx.send(picked);
        });
    let Some(picked) = rx.await.map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let path = picked.into_path().map_err(|e| e.to_string())?;
    let size = tokio::fs::metadata(&path)
        .await
        .map_err(|e| format!("Could not open that file: {e}"))?
        .len();
    if size > MAX_OVERLAY_FILE_BYTES as u64 {
        return Err("That file is too large to be an overlay file.".into());
    }
    let bytes = tokio::fs::read(&path).await.map_err(|e| format!("Could not open that file: {e}"))?;
    let text = String::from_utf8(bytes).map_err(|_| "That file is not a StreamNook overlay file.".to_string())?;
    parse_overlay_file(&text).map(Some)
}

#[cfg(target_os = "ios")]
#[tauri::command]
pub async fn import_overlay_file(_app: AppHandle, _window: Window) -> Result<Option<ImportedOverlay>, String> {
    Err(crate::ios_ffi::not_wired("Importing overlay files"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const OVERLAY_CONFIG_TS: &str =
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../src/components/overlay/overlayConfig.ts"));

    /// Field names of `export interface OverlayStyle { ... }`.
    fn ts_style_keys() -> Vec<String> {
        let start = OVERLAY_CONFIG_TS
            .find("export interface OverlayStyle {")
            .expect("OverlayStyle interface not found");
        let body = &OVERLAY_CONFIG_TS[start..];
        let end = body.find("\n}").expect("OverlayStyle interface never closes");
        body[..end]
            .lines()
            .skip(1)
            .filter_map(|line| {
                let rest = line.strip_prefix("  ")?;
                if rest.starts_with(' ') {
                    return None;
                }
                let name: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                let after = rest[name.len()..].trim_start_matches('?');
                (!name.is_empty() && after.starts_with(':')).then_some(name)
            })
            .collect()
    }

    #[test]
    fn style_fields_match_the_typescript_type() {
        let ts = ts_style_keys();
        assert!(ts.len() > 50, "parsed too few OverlayStyle keys: {ts:?}");
        for key in &ts {
            let known = FIELDS.iter().any(|(k, _)| k == key) || NEVER_EXPORTED.contains(&key.as_str());
            assert!(known, "OverlayStyle.{key} is not in overlay_file.rs FIELDS (or NEVER_EXPORTED)");
        }
        for (key, _) in FIELDS {
            assert!(ts.iter().any(|k| k == key), "FIELDS has {key}, which OverlayStyle no longer has");
        }
    }

    fn sample_style() -> Value {
        json!({
            "width": 420,
            "height": 700,
            "sources": ["twitch", "kick"],
            "sourceTag": "icon",
            "fontFamily": "'Bebas Neue', sans-serif",
            "fontSize": 18,
            "lineHeight": 1.5,
            "fontWeight": 600,
            "bodyTextColor": "#ffeedd",
            "textShadowColor": "rgba(0, 0, 0, 0.8)",
            "linkColor": "",
            "commandFilters": [{ "value": "!", "mode": "prefix" }, { "value": "!title", "mode": "exact" }],
            "hidePhrases": ["spoiler"],
            "hiddenProviderEvents": ["twitch:raid"],
            "eventTemplates": { "raid": "{username} brought {viewers}" },
            "superchatCurrency": "EUR",
            "bubble": true,
            "entrance": "pop",
            "blockedUsers": { "twitch:sender": ["someone"] },
            "profileName": "Sender's overlay",
        })
    }

    #[test]
    fn round_trip_keeps_the_style_and_drops_account_data() {
        let text = build_overlay_file("My overlay", &sample_style());
        let doc: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(doc["format"], OVERLAY_FILE_FORMAT);
        assert_eq!(doc["version"], OVERLAY_FILE_VERSION);
        assert!(doc["style"].get("blockedUsers").is_none());
        assert!(doc["style"].get("profileName").is_none());
        assert!(doc.get("id").is_none() && doc.get("sources").is_none() && doc.get("channels").is_none());

        let imported = parse_overlay_file(&text).unwrap();
        assert_eq!(imported.name, "My overlay");
        assert!(imported.dropped.is_empty(), "{:?}", imported.dropped);
        let mut expected = sample_style().as_object().unwrap().clone();
        expected.remove("blockedUsers");
        expected.remove("profileName");
        assert_eq!(imported.style, expected);
    }

    #[test]
    fn invalid_values_are_dropped_or_clamped() {
        let raw = json!({
            "format": OVERLAY_FILE_FORMAT,
            "version": 1,
            "name": "  \u{0007}Bad\u{0000} name ",
            "style": {
                "width": 99999,
                "fontSize": "huge",
                "fontWeight": 650,
                "sourceTag": "banner",
                "fontFamily": "x; background: url(https://evil)",
                "bodyTextColor": "red; position: fixed",
                "linkColor": "",
                "superchatCurrency": "euro",
                "sources": ["twitch", "myspace", "twitch"],
                "hiddenProviderEvents": ["twitch:raid", "twitch:hack", "nope"],
                "hiddenBadgeProviders": ["ffz", "<script>"],
                "eventTemplates": { "raid": "hi", "takeover": "x" },
                "maxMessageLines": 2.6,
                "mystery": true,
            }
        });
        let imported = parse_overlay_file(&raw.to_string()).unwrap();
        let s = &imported.style;
        assert_eq!(imported.name, "Bad name");
        assert_eq!(s["width"], 900);
        assert_eq!(s["fontWeight"], 700);
        assert_eq!(s["linkColor"], "");
        assert_eq!(s["sources"], json!(["twitch"]));
        assert_eq!(s["hiddenProviderEvents"], json!(["twitch:raid"]));
        assert_eq!(s["hiddenBadgeProviders"], json!(["ffz"]));
        assert_eq!(s["eventTemplates"], json!({ "raid": "hi" }));
        assert_eq!(s["maxMessageLines"], 3);
        for key in ["fontSize", "sourceTag", "fontFamily", "bodyTextColor", "superchatCurrency", "mystery"] {
            assert!(s.get(key).is_none(), "{key} should be dropped");
            assert!(imported.dropped.iter().any(|d| d == key), "{key} should be reported");
        }
    }

    #[test]
    fn refuses_files_that_are_not_overlay_files() {
        assert!(parse_overlay_file("not json").is_err());
        assert!(parse_overlay_file("[]").is_err());
        assert!(parse_overlay_file(&json!({ "format": "other", "version": 1, "style": {} }).to_string()).is_err());
        assert!(parse_overlay_file(&json!({ "format": OVERLAY_FILE_FORMAT, "style": {} }).to_string()).is_err());
        assert!(parse_overlay_file(&json!({ "format": OVERLAY_FILE_FORMAT, "version": 1 }).to_string()).is_err());
        let newer = json!({ "format": OVERLAY_FILE_FORMAT, "version": OVERLAY_FILE_VERSION + 1, "style": {} });
        assert!(parse_overlay_file(&newer.to_string()).unwrap_err().contains("newer"));
        let big = format!("{{\"format\":\"{OVERLAY_FILE_FORMAT}\",\"pad\":\"{}\"}}", "x".repeat(MAX_OVERLAY_FILE_BYTES));
        assert!(parse_overlay_file(&big).unwrap_err().contains("too large"));
    }

    #[test]
    fn empty_name_falls_back_and_file_names_are_safe() {
        let imported = parse_overlay_file(&json!({ "format": OVERLAY_FILE_FORMAT, "version": 1, "style": {} }).to_string()).unwrap();
        assert_eq!(imported.name, DEFAULT_NAME);
        assert_eq!(file_name_for("a/b:c*?"), "a b c.snoverlay");
        assert_eq!(file_name_for("   "), "Imported overlay.snoverlay");
    }

    /// The same fixture the web half tests: every setting changed from its
    /// default, every choice option, both ends of every range.
    const EVERY_SETTING: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../src/components/overlay/builder/fixtures/overlayEverySetting.json"
    ));

    /// Equal as the overlay reads them: JSON numbers compare by value, so `2`
    /// and `2.0` are the same setting.
    fn same_setting(a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
            (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same_setting(p, q)),
            (Value::Object(x), Value::Object(y)) => {
                x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same_setting(v, w)))
            }
            _ => a == b,
        }
    }

    #[test]
    fn every_setting_survives_a_file_unchanged() {
        let fixture: Value = serde_json::from_str(EVERY_SETTING).unwrap();
        let base = fixture["base"].as_object().unwrap().clone();
        let mut styles = vec![("every setting changed".to_string(), base.clone())];
        for (i, variant) in fixture["variants"].as_array().unwrap().iter().enumerate() {
            let mut style = base.clone();
            style.extend(variant.as_object().unwrap().clone());
            styles.push((format!("variant {}", i + 1), style));
        }
        for key in ts_style_keys() {
            if !NEVER_EXPORTED.contains(&key.as_str()) {
                assert!(base.contains_key(&key), "overlayEverySetting.json is missing {key}");
            }
        }
        for (label, style) in styles {
            let text = build_overlay_file("Every setting", &Value::Object(style.clone()));
            let imported = parse_overlay_file(&text).unwrap();
            assert!(imported.dropped.is_empty(), "{label}: dropped {:?}", imported.dropped);
            assert_eq!(imported.style.len(), style.len(), "{label}: setting count changed");
            for (key, value) in &style {
                let back = imported.style.get(key).unwrap_or(&Value::Null);
                assert!(same_setting(value, back), "{label}: {key} went out as {value} and came back as {back}");
            }
        }
    }

    #[test]
    fn text_entry_cap_matches_the_typescript_constant() {
        let marker = "export const OVERLAY_FILE_TEXT_ENTRY_MAX = ";
        let at = OVERLAY_CONFIG_TS.find(marker).expect("OVERLAY_FILE_TEXT_ENTRY_MAX not found") + marker.len();
        let digits: String = OVERLAY_CONFIG_TS[at..].chars().take_while(|c| c.is_ascii_digit() || *c == '_').filter(|c| *c != '_').collect();
        assert_eq!(digits.parse::<usize>().unwrap(), TEXT_ENTRY_MAX);
    }

    /// The same inputs and results as the web half's test
    /// (builder/overlayFileEntries.test.ts), so a file opens the same on both.
    #[test]
    fn long_list_entries_travel_whole_up_to_the_cap() {
        let legacy = "r".repeat(300);
        let at_cap = "p".repeat(TEXT_ENTRY_MAX);
        let too_long = "q".repeat(TEXT_ENTRY_MAX + 1);
        let long_command = format!("!{}", "y".repeat(TEXT_ENTRY_MAX));
        let raw = json!({
            "hidePhrases": [legacy.clone(), at_cap.clone(), too_long, "  ", "Spoiler", "spoiler", "tab\there", 7],
            "commandFilters": [
                { "value": long_command, "mode": "exact" },
                { "value": "!ok", "mode": "exact" },
                { "value": " ", "mode": "prefix" },
                { "mode": "prefix" },
                "!legacy",
            ],
        });
        let (style, dropped) = sanitize_style(&raw);
        assert_eq!(style["hidePhrases"], json!([legacy, at_cap, "Spoiler"]));
        assert_eq!(style["commandFilters"], json!([{ "value": "!ok", "mode": "exact" }]));
        assert_eq!(dropped, vec!["commandFilters".to_string(), "hidePhrases".to_string()]);

        let clean = json!({ "hidePhrases": ["  a  ", "", "b"], "commandFilters": [{ "value": "!", "mode": "prefix" }] });
        let (style, dropped) = sanitize_style(&clean);
        assert_eq!(style["hidePhrases"], json!(["a", "b"]));
        assert!(dropped.is_empty(), "{dropped:?}");
    }
}
