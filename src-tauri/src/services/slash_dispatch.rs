//! The chat slash command dispatcher: one place that reads a typed `/command`
//! line, runs it, and answers what the composer should do next. Desktop,
//! Android and the iPhone all call it through `run_slash_command`.
//!
//! # The answer ([`Outcome`])
//!
//! - `handled`: false means this is not a command StreamNook runs; the caller
//!   sends the text as typed (Twitch still runs `/me`).
//! - `send`: text the caller sends as an ordinary chat message in the room
//!   through its normal send path (custom command expansions, `/say`, `/shrug`,
//!   `/lurk`, `/squishy`, `/reply`, `/pin <message>`). `reply_to` names the
//!   parent message id for `/reply`; `pin_after_send` asks the send path to pin
//!   the message once Twitch has given it an id (`/pin <message>`).
//! - `clear_draft`: the composer empties its field.
//! - `toasts`: `{ text, kind }` with kind `success`, `error` or `info`.
//! - `notices`: lines printed in this room's chat that only the viewer sees.
//! - `ui`: one page action Rust cannot do itself. The caller applies `ui`
//!   before printing `notices` (so `/clearmessages` prints its line into the
//!   emptied chat); kinds that only start something need not be awaited.
//!
//! # `ui` kinds (tag `kind`, other fields beside it)
//!
//! - `open_user_card { user_id, login, display_name }`: `/user`, `/usercard`.
//! - `clear_local_chat`: `/clearmessages` empties this room's visible rows.
//! - `open_poll_composer { channel, composer }`: desktop `/poll`, `/prediction`
//!   (`composer` is "poll" or "prediction").
//! - `create_clip`: `/clip [title]` clips the room it was typed in.
//! - `identify_song`: `/song` listens to the player's audio.
//! - `reload_stream`: `/reload` restarts the playing stream and its chat.
//! - `reload_chat`: `/reconnect` empties and reconnects this room's chat.
//! - `refresh_emotes`: `/refresh` already refetched in Rust; the page drops its
//!   own copy of the room's emote set.
//! - `open_chat_window { window, channel, channel_id }`: desktop `/overlay`
//!   (`window` "overlay") and `/popup` (`window` "multichat").
//! - `open_channel_points { channel }`: desktop `/redeem`.
//! - `open_search`: desktop `/search`.
//! - `local_view { view, on }`: desktop `/localmod`, `/localsub`, `/localascii`
//!   and their `off` forms (`view` "mod", "sub" or "ascii").
//! - `dock_join { login, channel_id, display_name, avatar_url }` and
//!   `dock_part { login }`: desktop `/join`, `/part` (the chat dock).
//! - `speak { text }`: `/tts`.
//! - `window_effect { effect }`: `/barrelroll` and `/party`.
//! - `screenshot`: desktop `/screenshot` (needs the video element's rectangle).
//!
//! # Scripts and later work
//!
//! `/chain`, `/foreach`, `/runin` and `/timer` run their lines here. A line
//! inside a script sends through the chat send limiter as the signed-in account
//! (`chat_send_paced`), and its notices go straight into the room's chat as a
//! local NOTICE frame ([`post_room_notice`]), the same frame every window and
//! the iPhone already read. Work that ends after the call returns (a `/timer`,
//! a `/nuke` window closing, `/massunban` progress, a reminder, the end of a
//! `/spam` run on a phone) reports the same way.
//!
//! Every Twitch, GQL, file and settings call goes through [`Backend`], whose
//! real implementation (`commands::slash_backend`) calls the existing commands
//! by name with the arguments the page used to send, so the tests below watch
//! exactly what a command would do.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;
use serde_json::{json, Value};

use crate::services::chat_history::SweepEntry;
use crate::services::chat_send_limiter::Role;
use crate::services::command_catalog::{Builtin, UserCommand, BUILTINS};
use crate::services::moderation::{self, ModAction};
use crate::services::nuke;
use crate::services::reminder_service::{self, RemindParse};

pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The msg-id of a NOTICE frame StreamNook writes into a room itself.
pub const LOCAL_NOTICE_ID: &str = "streamnook_notice";

// ─────────────────────────────── the answer ───────────────────────────────

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToastKind {
    Success,
    Error,
    Info,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    pub text: String,
    pub kind: ToastKind,
}

/// A page action Rust cannot take. See the module header.
#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Ui {
    OpenUserCard { user_id: String, login: String, display_name: String },
    ClearLocalChat,
    OpenPollComposer { channel: String, composer: String },
    /// Clip the room the line was typed in (`channel` is its login, `kick:slug` for Kick),
    /// titled with the words after `/clip` (empty: the stream's title).
    CreateClip { channel: String, channel_id: String, title: String },
    IdentifySong,
    ReloadStream,
    ReloadChat,
    RefreshEmotes,
    OpenChatWindow { window: String, channel: String, channel_id: Option<String> },
    OpenChannelPoints { channel: String },
    OpenSearch,
    LocalView { view: String, on: bool },
    DockJoin { login: String, channel_id: String, display_name: String, avatar_url: Option<String> },
    DockPart { login: String },
    Speak { text: String },
    WindowEffect { effect: String },
    Screenshot,
}

/// What the composer does after a typed line. See the module header.
#[derive(Serialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    pub handled: bool,
    pub send: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub pin_after_send: bool,
    pub clear_draft: bool,
    pub toasts: Vec<Toast>,
    pub notices: Vec<String>,
    pub ui: Option<Ui>,
}

impl Outcome {
    fn handled() -> Self {
        Outcome { handled: true, clear_draft: true, ..Default::default() }
    }
    fn not_handled() -> Self {
        Outcome::default()
    }
    fn toast(&mut self, text: impl Into<String>, kind: ToastKind) {
        self.toasts.push(Toast { text: text.into(), kind });
    }
    fn ok(&mut self, text: impl Into<String>) {
        self.toast(text, ToastKind::Success);
    }
    fn err(&mut self, text: impl Into<String>) {
        self.toast(text, ToastKind::Error);
    }
    fn info(&mut self, text: impl Into<String>) {
        self.toast(text, ToastKind::Info);
    }
    fn say(&mut self, line: impl Into<String>) {
        self.notices.push(line.into());
    }
    fn say_all(&mut self, lines: Vec<String>) {
        self.notices.extend(lines);
    }
}

/// Which app is asking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Desktop,
    Android,
    Ios,
}

impl Platform {
    pub fn parse(s: &str) -> Platform {
        match s {
            "android" => Platform::Android,
            "ios" => Platform::Ios,
            _ => Platform::Desktop,
        }
    }
    pub fn phone(self) -> bool {
        self != Platform::Desktop
    }
}

/// The signed-in account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Me {
    pub id: String,
    pub login: String,
    pub display_name: String,
}

/// Everything the dispatcher asks of the app. The real one is
/// `commands::slash_backend::TauriBackend`; tests use a fake.
pub trait Backend: Send + Sync + 'static {
    /// Run an app command by the name and camelCase arguments the page used.
    fn invoke<'a>(&'a self, cmd: &'a str, args: Value) -> BoxFut<'a, Result<Value, String>>;
    fn me(&self) -> Option<Me>;
    /// The account's standing in a room (from its own chat badges there).
    fn role(&self, channel: &str) -> Role;
    /// The settings as JSON (the same object the page held).
    fn settings(&self) -> Value;
    /// (login, display name, user id) of whoever whispered last.
    fn last_whisper(&self) -> Option<(String, String, String)>;
    /// The room's held messages sent at or after `since_ms`, oldest first.
    fn history(&self, channel: &str, since_ms: i64) -> Vec<SweepEntry>;
    /// The newest held message id `login` sent in the room.
    fn latest_message_id(&self, channel: &str, login: &str) -> Option<String>;
    /// Whether a chat window holds the room right now.
    fn is_open<'a>(&'a self, channel: &'a str) -> BoxFut<'a, bool>;
    /// Print a line in the room's chat that only the viewer sees.
    fn post_notice<'a>(&'a self, channel: &'a str, text: String) -> BoxFut<'a, ()>;
    fn now_ms(&self) -> i64;
    fn new_id(&self) -> String;
}

/// The room a line runs in.
#[derive(Clone)]
pub struct Ctx {
    pub b: Arc<dyn Backend>,
    /// Bare lowercase Twitch login, or a provider room key (`kick:slug`).
    pub channel: String,
    /// The room's Twitch user id ("" when not known).
    pub broadcaster_id: String,
    pub platform: Platform,
}

// ─────────────────────────── the command table ───────────────────────────

fn dev_build() -> bool {
    cfg!(debug_assertions)
}

/// The built-in a name runs (aliases included), as this build lists them.
pub fn find_builtin(name: &str) -> Option<&'static Builtin> {
    let n = resolve_alias(name);
    BUILTINS.iter().find(|b| b.name == n && (dev_build() || !b.dev_only()))
}

/// The command a typed name runs: itself, or what its alias points at.
pub fn resolve_alias(name: &str) -> String {
    let n = name.to_lowercase();
    BUILTINS
        .iter()
        .find(|b| b.aliases.iter().any(|a| a.eq_ignore_ascii_case(&n)))
        .map(|b| b.name.to_string())
        .unwrap_or(n)
}

/// The built-ins custom commands have always yielded to. Every other built-in
/// name and alias came later, after people had saved custom commands, so a
/// saved custom command of the same name keeps running in its place.
const BUILT_IN_FIRST: &[&str] = &[
    "mods", "vips", "color", "block", "unblock", "disconnect", "w", "gift", "vote", "ban", "timeout", "unban",
    "monitor", "unmonitor", "restrict", "unrestrict", "clear", "slow", "slowoff", "followers", "followersoff",
    "subscribers", "subscribersoff", "uniquechat", "uniquechatoff", "emoteonly", "emoteonlyoff", "announce",
    "shoutout", "poll", "deletepoll", "commercial", "goal", "prediction", "raid", "unraid", "marker",
    "clearmessages", "openurl", "popout", "popup", "r", "clip", "chatters", "ignore", "settitle", "setgame",
    "overlay", "uptime", "song", "remind", "usercard", "user", "banid", "nuke", "undo", "endpoll", "cancelpoll",
    "lockprediction", "cancelprediction", "completeprediction", "pin", "unpin", "blockterm", "unblockterm",
    "refresh", "reload", "me", "help",
];

/// Whether a saved custom command named `typed` runs instead of the built-in.
pub fn custom_command_wins(typed: &str) -> bool {
    !BUILT_IN_FIRST.contains(&typed.to_lowercase().as_str())
}

fn user_commands(settings: &Value) -> Vec<UserCommand> {
    crate::services::command_catalog::user_commands(settings.get("chat_commands"))
}

/// An enabled custom command by trigger. `require_slash` picks slash commands
/// (true) or plain-text triggers (false).
pub fn find_user_command(trigger: &str, commands: &[UserCommand], require_slash: bool) -> Option<UserCommand> {
    let t = trigger.to_lowercase();
    commands
        .iter()
        .find(|c| c.enabled && c.trigger.to_lowercase() == t && (c.require_slash != Some(false)) == require_slash)
        .cloned()
}

/// A plain-text trigger at the start of `message` (or, when the command allows
/// it, at the end), with the other words as its arguments.
pub fn match_plain_text_command(message: &str, commands: &[UserCommand]) -> Option<(UserCommand, Vec<String>)> {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lowered = trimmed.to_lowercase();
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    let first = words.first().map(|w| w.to_lowercase()).unwrap_or_default();
    let last = words.last().map(|w| w.to_lowercase()).unwrap_or_default();
    for c in commands {
        if !c.enabled || c.require_slash != Some(false) {
            continue;
        }
        let t = c.trigger.to_lowercase();
        if t.is_empty() {
            continue;
        }
        if first == t {
            return Some((c.clone(), words[1..].iter().map(|s| s.to_string()).collect()));
        }
        if c.also_match_suffix == Some(true) && last == t && lowered != t {
            return Some((c.clone(), words[..words.len() - 1].iter().map(|s| s.to_string()).collect()));
        }
    }
    None
}

/// The lines `/help [command]` prints. A phone lists only what works there.
pub fn help_lines(arg: Option<&str>, phone: bool) -> Vec<String> {
    let name = arg.unwrap_or("").trim().trim_start_matches('/').to_lowercase();
    if name.is_empty() {
        let mut by_category: Vec<(&str, Vec<String>)> = Vec::new();
        for c in BUILTINS.iter().filter(|b| dev_build() || !b.dev_only()) {
            if phone && c.desktop_only() {
                continue;
            }
            match by_category.iter_mut().find(|(cat, _)| *cat == c.category) {
                Some((_, list)) => list.push(format!("/{}", c.name)),
                None => by_category.push((c.category, vec![format!("/{}", c.name)])),
            }
        }
        let mut lines = vec![if phone {
            "Commands (try /help <command> for one of them, or the command button left of the emote picker):"
        } else {
            "Commands (try /help <command> for one of them, or Commands in the emote menu, the smiley in the message box):"
        }
        .to_string()];
        lines.extend(by_category.into_iter().map(|(cat, names)| format!("{cat}: {}", names.join(" "))));
        return lines;
    }
    let Some(def) = find_builtin(&name) else {
        return vec![format!("There is no /{name} command. /help lists them all.")];
    };
    if phone && def.desktop_only() {
        return vec![format!("/{} works in the desktop app.", def.name)];
    }
    let mut lines = vec![format!("{}: {}", def.usage, def.description)];
    if !def.details.is_empty() {
        lines.push(def.details.to_string());
    }
    if !def.aliases.is_empty() {
        lines.push(format!(
            "Also works as {}",
            def.aliases.iter().map(|a| format!("/{a}")).collect::<Vec<_>>().join(", ")
        ));
    }
    if !def.examples.is_empty() {
        lines.push(format!("For example: {}", def.examples.join("  ")));
    }
    lines
}

// ─────────────────────────────── parsing ───────────────────────────────

/// A login from what was typed: `@Name`, `#name` or `name`, lowercased.
pub fn clean_login(raw: &str) -> String {
    raw.trim().trim_start_matches(['@', '#']).to_lowercase()
}

/// The typed name with its first `@` removed, case kept (what the older
/// moderation commands always did).
fn strip_first_at(raw: &str) -> String {
    raw.replacen('@', "", 1)
}

/// The leading integer of `raw`, the way JavaScript's `parseInt` reads it.
pub fn js_parse_int(raw: &str) -> Option<i64> {
    let s = raw.trim_start();
    let (neg, digits) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let end = digits.find(|c: char| !c.is_ascii_digit()).unwrap_or(digits.len());
    if end == 0 {
        return None;
    }
    let n: i64 = digits[..end].parse().unwrap_or(i64::MAX);
    Some(if neg { -n } else { n })
}

/// "30s", "5m", "1h30m", "2d", "1w", or a bare number of seconds, as
/// milliseconds. Units go in that order, each at most once.
pub fn parse_delay(raw: &str) -> Option<u64> {
    let s = raw.trim().to_lowercase();
    if s.is_empty() {
        return None;
    }
    if s.chars().all(|c| c.is_ascii_digit()) {
        return Some(s.parse::<u64>().unwrap_or(u64::MAX / 1000).saturating_mul(1000));
    }
    let mut rest = s.as_str();
    let mut total: u64 = 0;
    let mut any = false;
    for (unit, ms) in [('w', 604_800_000u64), ('d', 86_400_000), ('h', 3_600_000), ('m', 60_000), ('s', 1000)] {
        let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
        if end > 0 && rest[end..].starts_with(unit) {
            let n: u64 = rest[..end].parse().unwrap_or(u64::MAX);
            total = total.saturating_add(n.saturating_mul(ms));
            rest = &rest[end + 1..];
            any = true;
        }
    }
    (any && rest.is_empty()).then_some(total)
}

/// `/followers [duration]` in minutes: "10m", "2h", "1d", "1w", "1mo", or a
/// bare number of minutes. None where a unit came with no number.
pub fn followers_minutes(raw: &str) -> Option<i64> {
    let n = js_parse_int(raw);
    if raw.ends_with("mo") {
        n.map(|n| n * 60 * 24 * 30)
    } else if raw.ends_with('m') {
        n
    } else if raw.ends_with('h') {
        n.map(|n| n * 60)
    } else if raw.ends_with('d') {
        n.map(|n| n * 60 * 24)
    } else if raw.ends_with('w') {
        n.map(|n| n * 60 * 24 * 7)
    } else {
        Some(n.unwrap_or(0))
    }
}

/// A `/setting set` value: JSON when it reads as JSON, text otherwise.
pub fn parse_setting_value(raw: &str) -> Value {
    let t = raw.trim();
    serde_json::from_str(t).unwrap_or_else(|_| Value::String(t.to_string()))
}

/// "empty", "list", "number", "string", "boolean" or "object".
fn kind_of(v: Option<&Value>) -> &'static str {
    match v {
        None | Some(Value::Null) => "empty",
        Some(Value::Array(_)) => "list",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Object(_)) => "object",
    }
}

fn get_path<'a>(root: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    let mut cur = root;
    for k in keys {
        cur = cur.as_object()?.get(*k)?;
    }
    Some(cur)
}

fn with_path(root: &Value, keys: &[&str], value: Value) -> Value {
    let mut out = root.clone();
    let mut cur = &mut out;
    for k in &keys[..keys.len() - 1] {
        if !cur.is_object() {
            *cur = json!({});
        }
        let obj = cur.as_object_mut().expect("an object");
        let next = obj.entry(k.to_string()).or_insert_with(|| json!({}));
        if !next.is_object() {
            *next = json!({});
        }
        cur = next;
    }
    if !cur.is_object() {
        *cur = json!({});
    }
    cur.as_object_mut().expect("an object").insert(keys[keys.len() - 1].to_string(), value);
    out
}

/// JSON the way the page printed it: whole numbers without a fraction.
pub fn js_stringify(v: &Value) -> String {
    fn norm(v: &Value) -> Value {
        match v {
            Value::Number(n) => match n.as_f64() {
                Some(f) if n.is_f64() && f.fract() == 0.0 && f.abs() < 9.0e15 => json!(f as i64),
                _ => v.clone(),
            },
            Value::Array(a) => Value::Array(a.iter().map(norm).collect()),
            Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), norm(v))).collect()),
            _ => v.clone(),
        }
    }
    norm(v).to_string()
}

/// How many chatters either Helix chatters answer names.
pub fn count_chatters(res: &Value) -> Option<i64> {
    if let Some(total) = res.get("total").and_then(Value::as_i64) {
        return Some(total);
    }
    if let Some(data) = res.get("data").and_then(Value::as_array) {
        return Some(data.len() as i64);
    }
    if let Some(groups) = res.as_array() {
        let mut n = 0;
        for g in groups {
            if let Some(obj) = g.as_object() {
                n += obj.values().filter_map(Value::as_array).map(|a| a.len() as i64).sum::<i64>();
            }
        }
        return Some(n);
    }
    None
}

/// What `/pin` should do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinTarget {
    Message { message_id: String, duration_seconds: Option<u64> },
    Send { text: String },
}

fn pin_duration(token: Option<&str>) -> Option<u64> {
    let token = token?;
    let (digits, unit) = match token.chars().last()? {
        c @ ('s' | 'm' | 'h') => (&token[..token.len() - 1], Some(c)),
        _ => (token, None),
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let n: u64 = digits.parse().ok()?;
    if n == 0 {
        return None;
    }
    Some(n * match unit {
        Some('m') => 60,
        Some('h') => 3600,
        _ => 1,
    })
}

fn looks_like_message_id(token: &str) -> bool {
    let parts: Vec<&str> = token.split('-').collect();
    parts.len() == 5
        && [8, 4, 4, 4, 12].iter().zip(&parts).all(|(n, p)| p.len() == *n && p.chars().all(|c| c.is_ascii_hexdigit()))
}

/// `/pin <id> [duration]`, `/pin @user [duration]` (their latest message), or
/// `/pin <text>` (send it and pin it). A name means "their latest message"
/// only when nothing but a duration follows it.
pub fn resolve_pin_args(args: &[&str], latest_from: impl Fn(&str) -> Option<String>) -> Result<PinTarget, String> {
    let tokens: Vec<&str> = args.iter().copied().filter(|t| !t.is_empty()).collect();
    let Some((&first, rest)) = tokens.split_first() else {
        return Err("/pin needs a message id, a username or a message to send".to_string());
    };
    if looks_like_message_id(first) {
        return Ok(PinTarget::Message { message_id: first.to_string(), duration_seconds: pin_duration(rest.first().copied()) });
    }
    let name_only = rest.is_empty() || (rest.len() == 1 && pin_duration(Some(rest[0])).is_some());
    if name_only {
        let name = first.strip_prefix('@').unwrap_or(first);
        let found = if name.is_empty() { None } else { latest_from(name) };
        if let Some(id) = found {
            return Ok(PinTarget::Message { message_id: id, duration_seconds: pin_duration(rest.first().copied()) });
        }
        if first.starts_with('@') {
            return Err(format!("No recent message from {} in this chat", if name.is_empty() { first } else { name }));
        }
    }
    Ok(PinTarget::Send { text: tokens.join(" ") })
}

// ─────────────────────────── custom command templates ───────────────────────────

/// What a custom command's placeholders read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TemplateContext {
    pub user_name: String,
    pub user_id: String,
    pub channel_name: String,
    pub channel_id: String,
    pub stream_title: String,
    pub stream_game: String,
    pub stream_uptime: String,
    pub args: Vec<String>,
}

/// An expanded template and the 1-based argument positions it needed but did
/// not get.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expansion {
    pub text: String,
    pub missing_args: Vec<u64>,
}

fn dotted(key: &str, ctx: &TemplateContext) -> Option<String> {
    Some(match key.to_lowercase().as_str() {
        "user" | "user.name" => ctx.user_name.clone(),
        "user.id" => ctx.user_id.clone(),
        "channel" | "channel.name" => ctx.channel_name.clone(),
        "channel.id" => ctx.channel_id.clone(),
        "stream.title" => ctx.stream_title.clone(),
        "stream.game" => ctx.stream_game.clone(),
        "stream.uptime" => ctx.stream_uptime.clone(),
        _ => return None,
    })
}

/// Expand a custom command template in one pass, so an inserted value is never
/// read as a placeholder:
///
/// - `{N}` the Nth argument, `{N+}` the Nth onwards, `{*}` all of them
/// - `{{` and `}}` literal braces
/// - `{user}` / `{user.name}`, `{user.id}`, `{channel}` / `{channel.name}`,
///   `{channel.id}`, `{stream.title}`, `{stream.game}`, `{stream.uptime}`
///
/// An unknown `{name}` stays as typed. Runs of spaces left by empty values
/// collapse to one and the ends are trimmed.
pub fn expand_user_command(template: &str, ctx: &TemplateContext) -> Expansion {
    let mut out = String::with_capacity(template.len());
    let mut missing: Vec<u64> = Vec::new();
    let b = template.as_bytes();
    let mut i = 0;
    while i < template.len() {
        let rest = &template[i..];
        if rest.starts_with("{{") {
            out.push('{');
            i += 2;
            continue;
        }
        if rest.starts_with("}}") {
            out.push('}');
            i += 2;
            continue;
        }
        if b[i] == b'{' {
            let inner_end = rest[1..].find('}').map(|e| e + 1);
            if let Some(close) = inner_end {
                let inner = &rest[1..close];
                let digits_end = inner.find(|c: char| !c.is_ascii_digit()).unwrap_or(inner.len());
                // {N+}
                if digits_end > 0 && &inner[digits_end..] == "+" {
                    let start: u64 = inner[..digits_end].parse().unwrap_or(u64::MAX);
                    if start < 1 || start > ctx.args.len() as u64 {
                        missing.push(start);
                    } else {
                        out.push_str(&ctx.args[(start - 1) as usize..].join(" "));
                    }
                    i += close + 1;
                    continue;
                }
                // {N}
                if digits_end > 0 && digits_end == inner.len() {
                    let n: u64 = inner.parse().unwrap_or(u64::MAX);
                    match n.checked_sub(1).and_then(|k| ctx.args.get(k as usize)).filter(|v| !v.is_empty()) {
                        Some(v) => out.push_str(v),
                        None => missing.push(n),
                    }
                    i += close + 1;
                    continue;
                }
                if inner == "*" {
                    out.push_str(&ctx.args.join(" "));
                    i += close + 1;
                    continue;
                }
                let is_name = inner.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
                    && inner.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.');
                if is_name {
                    match dotted(inner, ctx) {
                        Some(v) => out.push_str(&v),
                        None => out.push_str(&rest[..=close]),
                    }
                    i += close + 1;
                    continue;
                }
            }
        }
        let ch = rest.chars().next().expect("not empty");
        out.push(ch);
        i += ch.len_utf8();
    }
    // Runs of spaces or tabs left by empty values become one space.
    let mut collapsed = String::with_capacity(out.len());
    let mut run = String::new();
    for c in out.chars() {
        if c == ' ' || c == '\t' {
            run.push(c);
        } else {
            if run.chars().count() >= 2 {
                collapsed.push(' ');
            } else {
                collapsed.push_str(&run);
            }
            run.clear();
            collapsed.push(c);
        }
    }
    if run.chars().count() >= 2 {
        collapsed.push(' ');
    } else {
        collapsed.push_str(&run);
    }
    missing.sort_by_key(|n| n.to_string());
    missing.dedup();
    Expansion { text: collapsed.trim().to_string(), missing_args: missing }
}

/// A stream's age as "2h 14m" or "14m"; "" with no start time.
pub fn format_uptime(started_at: Option<&str>, now_ms: i64) -> String {
    let Some(start) = started_at.and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()) else {
        return String::new();
    };
    let elapsed = now_ms - start.timestamp_millis();
    if elapsed <= 0 {
        return "0m".to_string();
    }
    let minutes = elapsed / 60_000;
    let (h, m) = (minutes / 60, minutes % 60);
    if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}

/// The room's live stream (Helix `streams` row), or None when offline.
async fn room_stream(b: &dyn Backend, channel: &str) -> Option<Value> {
    if channel.is_empty() || channel.contains(':') {
        return None;
    }
    b.invoke("get_stream", json!({ "login": channel })).await.ok().filter(|v| !v.is_null())
}

/// The room's live stream for /uptime: `None` when Twitch answered that it is
/// not live, the failure when the check itself failed.
async fn uptime_stream(b: &dyn Backend, channel: &str) -> Result<Option<Value>, String> {
    if channel.is_empty() || channel.contains(':') {
        return Ok(None);
    }
    let stream = b.invoke("get_stream", json!({ "login": channel })).await?;
    Ok(Some(stream).filter(|s| s["started_at"].as_str().is_some_and(|t| !t.is_empty())))
}

/// The placeholders for a template run in `channel` (the room it was typed
/// in, never the stream playing elsewhere). The room is looked up only when
/// the template names it.
pub async fn template_context(b: &dyn Backend, channel: &str, channel_id: &str, args: Vec<String>, template: &str) -> TemplateContext {
    let me = b.me();
    let mut ctx = TemplateContext {
        user_name: me.as_ref().map(|m| if m.display_name.is_empty() { m.login.clone() } else { m.display_name.clone() }).unwrap_or_default(),
        user_id: me.map(|m| m.id).unwrap_or_default(),
        channel_name: channel.to_string(),
        channel_id: channel_id.to_string(),
        args,
        ..Default::default()
    };
    let lower = template.to_lowercase();
    if !lower.contains("{channel") && !lower.contains("{stream") {
        return ctx;
    }
    match room_stream(b, channel).await {
        Some(s) => {
            let name = s["user_name"].as_str().or_else(|| s["user_login"].as_str()).filter(|n| !n.is_empty());
            if let Some(name) = name {
                ctx.channel_name = name.to_string();
            }
            if ctx.channel_id.is_empty() {
                ctx.channel_id = s["user_id"].as_str().unwrap_or_default().to_string();
            }
            ctx.stream_title = s["title"].as_str().unwrap_or_default().to_string();
            ctx.stream_game = s["game_name"].as_str().unwrap_or_default().to_string();
            ctx.stream_uptime = format_uptime(s["started_at"].as_str(), b.now_ms());
        }
        None => {
            if let Some(user) = lookup(b, channel).await {
                ctx.channel_name = user.display_name.clone();
                if ctx.channel_id.is_empty() {
                    ctx.channel_id = user.id;
                }
            }
        }
    }
    ctx
}

/// A reminder's template expanded for the room it posts to.
pub async fn expand_for_room(template: &str, channel: &str, channel_id: &str) -> String {
    match crate::commands::slash_backend::global() {
        Some(b) => {
            let ctx = template_context(&*b, channel, channel_id, Vec::new(), template).await;
            expand_user_command(template, &ctx).text
        }
        None => expand_user_command(template, &TemplateContext { channel_name: channel.to_string(), ..Default::default() }).text,
    }
}

/// A local line in `channel`'s chat: a NOTICE frame with [`LOCAL_NOTICE_ID`],
/// which desktop, Android and the iPhone show as a system row in that room.
pub async fn post_room_notice(channel: &str, text: &str) {
    let key = if channel.contains(':') { channel.to_string() } else { channel.trim_start_matches('#').to_lowercase() };
    if key.is_empty() || text.is_empty() {
        return;
    }
    crate::services::irc_service::post_channel_frame(&key, notice_frame(&key, text)).await;
}

/// The frame [`post_room_notice`] sends.
pub fn notice_frame(channel: &str, text: &str) -> String {
    json!({ "type": "NOTICE", "channel": channel, "msg_id": LOCAL_NOTICE_ID, "message": text }).to_string()
}

// ─────────────────────────────── lookups ───────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
struct User {
    id: String,
    login: String,
    display_name: String,
    avatar: Option<String>,
}

/// Someone's account, `None` when Twitch has no such account, or why the lookup
/// failed (signed out, offline, a Helix refusal): never taken for "not found".
async fn find_user(b: &dyn Backend, login: &str) -> Result<Option<User>, String> {
    let v = match b.invoke("get_user_by_login", json!({ "login": login })).await {
        Ok(v) => v,
        Err(e) if e.contains("not found") => return Ok(None),
        Err(e) => return Err(e),
    };
    let Some(id) = v["id"].as_str().map(str::to_string) else { return Ok(None) };
    Ok(Some(User {
        id,
        login: v["login"].as_str().unwrap_or(login).to_string(),
        display_name: v["display_name"].as_str().unwrap_or(login).to_string(),
        avatar: v["profile_image_url"].as_str().map(str::to_string),
    }))
}

/// The account, or the line that says why there is none: `missing` when Twitch
/// has no such account, the failure when the lookup itself failed.
async fn user_or_why(b: &dyn Backend, who: &str, missing: String) -> Result<User, String> {
    match find_user(b, who).await {
        Ok(Some(user)) => Ok(user),
        Ok(None) => Err(missing),
        Err(e) => Err(format!("Couldn't look up {who}: {e}")),
    }
}

/// Someone's account when the lookup worked and found one.
async fn lookup(b: &dyn Backend, login: &str) -> Option<User> {
    find_user(b, login).await.ok().flatten()
}

fn is_broadcaster_of(ctx: &Ctx) -> bool {
    ctx.b.me().is_some_and(|m| !m.login.is_empty() && m.login.to_lowercase() == ctx.channel.to_lowercase())
}

fn is_moderator_of(ctx: &Ctx) -> bool {
    matches!(ctx.b.role(&ctx.channel), Role::Moderator | Role::Broadcaster)
}

// ─────────────────────────── confirmations and timers ───────────────────────────

const CONFIRM_WINDOW_MS: i64 = 20_000;
const MAX_SCRIPT_DEPTH: u32 = 3;
const MAX_FOREACH: usize = 20;
const MAX_CHAIN: usize = 10;
const MAX_TIMERS: usize = 20;
const MAX_TIMER_MS: u64 = 24 * 3600 * 1000;

static CONFIRMS: OnceLock<Mutex<HashMap<String, i64>>> = OnceLock::new();
static TIMERS: OnceLock<Mutex<HashMap<String, HashMap<u64, tauri::async_runtime::JoinHandle<()>>>>> = OnceLock::new();
static NEXT_TIMER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn confirms() -> std::sync::MutexGuard<'static, HashMap<String, i64>> {
    CONFIRMS.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap_or_else(|e| e.into_inner())
}

fn timers() -> std::sync::MutexGuard<'static, HashMap<String, HashMap<u64, tauri::async_runtime::JoinHandle<()>>>> {
    TIMERS.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap_or_else(|e| e.into_inner())
}

/// Ask for the same command twice within 20 seconds before something hard to
/// undo. True means "not confirmed yet" (the prompt was printed). Only a line
/// typed in the chat box counts: from a script the command is refused, so a
/// script can never confirm itself.
fn needs_confirmation(ctx: &Ctx, o: &mut Outcome, key: &str, prompt: &str, depth: u32, name: &str) -> bool {
    if depth > 0 {
        let what = if name.is_empty() { "This command".to_string() } else { format!("/{name}") };
        o.say(format!(
            "{what} asks you to confirm, so it can't run from /chain, /foreach, /timer or /runin. Type it in the chat box."
        ));
        return true;
    }
    let now = ctx.b.now_ms();
    let full = format!("{}|{key}", ctx.channel);
    let mut map = confirms();
    if let Some(at) = map.get(&full).copied() {
        if now - at < CONFIRM_WINDOW_MS {
            map.remove(&full);
            return false;
        }
    }
    map.insert(full, now);
    o.say(format!("{prompt} Run the same command again within 20 seconds to confirm."));
    true
}

// ─────────────────────────────── sending ───────────────────────────────

/// What a script line leaves for the line that ran it.
#[derive(Default)]
struct ScriptOut {
    toasts: Vec<Toast>,
    ui: Option<Ui>,
}

/// Where a line about a send to `channel` is seen: in that chat when a window
/// holds it, else as a toast.
async fn send_notice(ctx: &Ctx, channel: &str, text: String, out: &mut ScriptOut) {
    if ctx.b.is_open(channel).await {
        ctx.b.post_notice(channel, text).await;
    } else {
        out.toasts.push(Toast { text, kind: ToastKind::Error });
    }
}

/// Send a scripted line to `ctx`'s room: it waits for a slot from the chat
/// send limiter, then goes out as the signed-in account (needs no open chat).
async fn send_paced(ctx: &Ctx, text: &str, reply_to: Option<&str>, pin: bool, out: &mut ScriptOut) -> bool {
    let key = if ctx.channel.contains(':') { ctx.channel.clone() } else { ctx.channel.to_lowercase() };
    if ctx.b.me().is_none() {
        out.toasts.push(Toast { text: "Sign in to Twitch to send messages".into(), kind: ToastKind::Error });
        return false;
    }
    let mut args = json!({
        "channel": key,
        "channelId": if ctx.broadcaster_id.is_empty() { Value::Null } else { json!(ctx.broadcaster_id) },
        "text": text,
    });
    if let Some(r) = reply_to {
        args["replyTo"] = json!(r);
    }
    if pin {
        args["pinAfterSend"] = json!(true);
    }
    match ctx.b.invoke("chat_send_paced", args).await {
        Ok(v) => {
            let sent = v["is_sent"].as_bool().unwrap_or(false);
            if !sent {
                let reason = v["drop_reason"].as_str().unwrap_or("Twitch dropped it").to_string();
                send_notice(ctx, &key, format!("Not sent to {key}: {reason}"), out).await;
            }
            sent
        }
        Err(refusal) => {
            let shown = if key.is_empty() { "chat".to_string() } else { key.clone() };
            send_notice(ctx, &key, format!("Not sent to {shown}: {refusal}"), out).await;
            false
        }
    }
}

/// One line of a script (`/chain`, `/foreach`, `/timer`, `/runin`) in `ctx`'s
/// room. Text and commands that go to chat as text (`/me`, or one nothing
/// here knows) are sent paced; a command's notices go into the room at once.
fn run_script_line(ctx: Ctx, line: String, depth: u32) -> BoxFut<'static, ScriptOut> {
    Box::pin(async move {
        let mut out = ScriptOut::default();
        if !line.starts_with('/') {
            send_paced(&ctx, &line, None, false, &mut out).await;
            return out;
        }
        let o = dispatch(ctx.clone(), line.clone(), depth).await;
        if !o.handled {
            if let Some((provider, _)) = ctx.channel.split_once(':') {
                // Another platform would post the command as literal text.
                let cmd = line[1..].split_whitespace().next().unwrap_or("").to_lowercase();
                ctx.b.post_notice(&ctx.channel, format!("/{cmd} isn't available on {}.", provider_label(provider))).await;
                return out;
            }
            send_paced(&ctx, &line, None, false, &mut out).await;
            return out;
        }
        for n in o.notices {
            ctx.b.post_notice(&ctx.channel, n).await;
        }
        out.toasts.extend(o.toasts);
        out.ui = o.ui;
        if let Some(text) = o.send {
            send_paced(&ctx, &text, o.reply_to.as_deref(), o.pin_after_send, &mut out).await;
        }
        out
    })
}

/// A platform's name as chat shows it.
fn provider_label(provider: &str) -> String {
    match provider {
        "kick" => "Kick".to_string(),
        "youtube" => "YouTube".to_string(),
        "tiktok" => "TikTok".to_string(),
        "twitch" => "Twitch".to_string(),
        other => {
            let mut c = other.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
        }
    }
}

fn merge(o: &mut Outcome, s: ScriptOut) {
    o.toasts.extend(s.toasts);
    if s.ui.is_some() {
        o.ui = s.ui;
    }
}

// ─────────────────────────────── dispatch ───────────────────────────────

/// Run one typed line in `ctx`'s room. `depth` counts how deep inside
/// `/chain`, `/foreach`, `/timer` or `/runin` it runs (0 when typed).
pub fn dispatch(ctx: Ctx, text: String, depth: u32) -> BoxFut<'static, Outcome> {
    Box::pin(async move {
        if !text.starts_with('/') {
            return plain_text(&ctx, &text, depth).await;
        }
        let body = text[1..].trim();
        let words: Vec<&str> = body.split_whitespace().collect();
        let typed_word = words.first().copied().unwrap_or("");
        let typed = typed_word.to_lowercase();
        let command = resolve_alias(&typed);
        let args: Vec<&str> = words.iter().skip(1).copied().collect();
        let raw_rest = &body[typed_word.len()..];
        let rest = raw_rest.trim();
        let mut o = Outcome::handled();
        let settings = ctx.b.settings();

        // Saved custom commands, where they win the name.
        if custom_command_wins(&typed) {
            if let Some(uc) = find_user_command(&typed, &user_commands(&settings), true) {
                let a: Vec<String> = args.iter().map(|s| s.to_string()).collect();
                let tctx = template_context(&*ctx.b, &ctx.channel, &ctx.broadcaster_id, a, &uc.expansion).await;
                let ex = expand_user_command(&uc.expansion, &tctx);
                if !ex.missing_args.is_empty() {
                    o.err(format!(
                        "/{typed} expects argument{} {}",
                        if ex.missing_args.len() > 1 { "s" } else { "" },
                        ex.missing_args.iter().map(|i| format!("{{{i}}}")).collect::<Vec<_>>().join(", ")
                    ));
                    return o;
                }
                if ex.text.is_empty() {
                    o.err(format!("/{typed} expanded to an empty message"));
                    return o;
                }
                o.send = Some(ex.text);
                return o;
            }
        }

        let def = find_builtin(&command);
        if ctx.platform.phone() && (def.is_some_and(|d| d.desktop_only()) || command == "poll" || command == "prediction") {
            o.info(format!("/{typed} works in the desktop app"));
            return o;
        }

        // Kick, YouTube and other rooms run only what needs no Twitch API.
        if ctx.channel.contains(':') && !PLATFORM_NEUTRAL.contains(&command.as_str()) {
            return Outcome::not_handled();
        }

        let call = Call { ctx: &ctx, typed: &typed, command: &command, args: &args, rest, raw_rest, depth };
        match core(&call, &mut o).await {
            Ok(Flow::Done) => o,
            Ok(Flow::PassThrough) => Outcome::not_handled(),
            Ok(Flow::Usage) => match def {
                Some(d) => {
                    if !ctx.channel.is_empty() {
                        o.say(format!("Usage: {}", d.usage));
                    }
                    o.info(format!("Usage: {}", d.usage));
                    o
                }
                None => Outcome::not_handled(),
            },
            Ok(Flow::NotMine) => match extended(&call, &mut o).await {
                Ok(true) => o,
                Ok(false) => Outcome::not_handled(),
                Err(e) => {
                    log::warn!("[Commands] /{typed} failed: {e}");
                    o.err(if e.is_empty() { format!("/{typed} failed") } else { e });
                    o
                }
            },
            Err(e) => {
                log::warn!("[Command Handler] Failed to execute /{command}: {e}");
                o.err(format!("Command failed: {e}"));
                o
            }
        }
    })
}

/// A plain-text trigger (a custom command saved without the slash).
async fn plain_text(ctx: &Ctx, text: &str, depth: u32) -> Outcome {
    let settings = ctx.b.settings();
    let Some((cmd, args)) = match_plain_text_command(text, &user_commands(&settings)) else {
        return Outcome::not_handled();
    };
    let mut o = Outcome::handled();
    let tctx = template_context(&*ctx.b, &ctx.channel, &ctx.broadcaster_id, args, &cmd.expansion).await;
    let ex = expand_user_command(&cmd.expansion, &tctx);
    if !ex.missing_args.is_empty() {
        o.err(format!(
            "Trigger \"{}\" expects argument{} {}",
            cmd.trigger,
            if ex.missing_args.len() > 1 { "s" } else { "" },
            ex.missing_args.iter().map(|i| format!("{{{i}}}")).collect::<Vec<_>>().join(", ")
        ));
        return o;
    }
    if ex.text.is_empty() {
        o.err(format!("Trigger \"{}\" expanded to an empty message", cmd.trigger));
        return o;
    }
    // The expansion runs as if it had been typed: it may start a command.
    if ex.text.starts_with('/') {
        let inner = dispatch(ctx.clone(), ex.text.clone(), depth).await;
        if inner.handled {
            return inner;
        }
    }
    o.send = Some(ex.text);
    o
}

enum Flow {
    Done,
    /// A listed command missing what it needs: print its usage.
    Usage,
    /// Goes to chat as typed (`/me`).
    PassThrough,
    NotMine,
}

struct Call<'a> {
    ctx: &'a Ctx,
    typed: &'a str,
    command: &'a str,
    args: &'a [&'a str],
    /// Everything after the command word, trimmed.
    rest: &'a str,
    /// Everything after the command word, spacing kept.
    raw_rest: &'a str,
    depth: u32,
}

impl Call<'_> {
    fn arg(&self, i: usize) -> &str {
        self.args.get(i).copied().unwrap_or("")
    }
    fn from(&self, i: usize) -> String {
        self.args.get(i..).map(|a| a.join(" ")).unwrap_or_default()
    }
    fn b(&self) -> &dyn Backend {
        &*self.ctx.b
    }
    fn id(&self) -> &str {
        &self.ctx.broadcaster_id
    }
    fn login(&self) -> &str {
        &self.ctx.channel
    }
}

fn opt(s: &str) -> Value {
    if s.is_empty() {
        Value::Null
    } else {
        json!(s)
    }
}

/// Look someone up and run `cmd` on them, toasting `done`: `User <name> not
/// found` when there is no such account.
async fn on_user(
    c: &Call<'_>,
    o: &mut Outcome,
    username: &str,
    cmd: &str,
    args: impl FnOnce(&User) -> Value,
    done: String,
) -> Result<(), String> {
    match user_or_why(c.b(), username, format!("User {username} not found")).await {
        Ok(user) => {
            c.b().invoke(cmd, args(&user)).await?;
            o.ok(done);
        }
        Err(why) => o.err(why),
    }
    Ok(())
}

/// A Twitch moderation action's toast, in the words `services::moderation`
/// gives every surface.
fn mod_words(action: &ModAction, name: &str) -> String {
    moderation::outcome_for(moderation::Platform::Twitch, action, name).message
}

/// The commands of the original table.
async fn core(c: &Call<'_>, o: &mut Outcome) -> Result<Flow, String> {
    let b = c.b();
    let id = c.id().to_string();
    let login = c.login().to_string();
    let modes = |settings: Value| json!({ "broadcasterId": id, "settings": settings });
    match c.command {
        "spam" | "pyramid" => {
            let raw = c.raw_rest.to_string();
            match b
                .invoke(
                    "run_chat_burst",
                    json!({ "kind": c.command, "args": raw, "channelLogin": login, "channelId": id, "announceEnd": c.ctx.platform.phone() }),
                )
                .await
            {
                Ok(reply) => o.say(reply["message"].as_str().unwrap_or("").to_string()),
                Err(e) => o.err(e),
            }
        }
        "song" => {
            o.say("Listening to identify the song...");
            o.ui = Some(Ui::IdentifySong);
        }
        "ban" => {
            if c.args.is_empty() {
                return Ok(Flow::Usage);
            }
            let username = clean_login(c.arg(0));
            let reason = c.from(1);
            let done = mod_words(&ModAction::Ban, &username);
            on_user(c, o, &username, "ban_user", |u| {
                json!({ "broadcasterId": id, "targetUserId": u.id, "duration": null, "reason": opt(&reason) })
            }, done)
            .await?;
        }
        "timeout" => {
            if c.args.is_empty() {
                return Ok(Flow::Usage);
            }
            let username = clean_login(c.arg(0));
            let (secs, reason) = moderation::typed_timeout(&c.from(1));
            let reason = reason.unwrap_or_default();
            // What Twitch will really apply (it stops at 14 days), said the way
            // every other moderation surface says it ("Timed out bob for 10m").
            let secs = moderation::applied_timeout(moderation::Platform::Twitch, secs);
            let done = mod_words(&ModAction::Timeout { seconds: secs }, &username);
            on_user(c, o, &username, "ban_user", |u| {
                json!({ "broadcasterId": id, "targetUserId": u.id, "duration": secs, "reason": opt(&reason) })
            }, done)
            .await?;
        }
        "unban" | "untimeout" => {
            if c.args.is_empty() {
                return Ok(Flow::Usage);
            }
            let username = clean_login(c.arg(0));
            let done = mod_words(&ModAction::Unban { timeout: c.command == "untimeout" }, &username);
            on_user(c, o, &username, "unban_user", |u| json!({ "broadcasterId": id, "targetUserId": u.id }), done).await?;
        }
        "clear" => {
            b.invoke("clear_chat", json!({ "broadcasterId": id })).await?;
            o.ok("Chat cleared");
        }
        "slow" => {
            let secs = (if c.args.is_empty() { Some(30) } else { js_parse_int(c.arg(0)) }).unwrap_or(30);
            b.invoke("update_chat_settings", modes(json!({ "slow_mode": true, "slow_mode_wait_time": secs }))).await?;
            o.ok(format!("Slow mode enabled ({secs}s)"));
        }
        "slowoff" => {
            b.invoke("update_chat_settings", modes(json!({ "slow_mode": false }))).await?;
            o.ok("Slow mode disabled");
        }
        "followers" => {
            let raw = if c.arg(0).is_empty() { "0" } else { c.arg(0) };
            let minutes = followers_minutes(raw).map(Value::from).unwrap_or(Value::Null);
            b.invoke("update_chat_settings", modes(json!({ "follower_mode": true, "follower_mode_duration": minutes }))).await?;
            o.ok("Followers-only mode enabled");
        }
        "followersoff" => {
            b.invoke("update_chat_settings", modes(json!({ "follower_mode": false }))).await?;
            o.ok("Followers-only mode disabled");
        }
        "subscribers" | "subscribersoff" | "uniquechat" | "uniquechatoff" | "emoteonly" | "emoteonlyoff" => {
            let (field, on, done) = match c.command {
                "subscribers" => ("subscriber_mode", true, "Subscribers-only mode enabled"),
                "subscribersoff" => ("subscriber_mode", false, "Subscribers-only mode disabled"),
                "uniquechat" => ("unique_chat_mode", true, "Unique chat mode enabled"),
                "uniquechatoff" => ("unique_chat_mode", false, "Unique chat mode disabled"),
                "emoteonly" => ("emote_mode", true, "Emote-only mode enabled"),
                _ => ("emote_mode", false, "Emote-only mode disabled"),
            };
            let mut s = serde_json::Map::new();
            s.insert(field.to_string(), json!(on));
            b.invoke("update_chat_settings", modes(Value::Object(s))).await?;
            o.ok(done);
        }
        "announce" | "announceblue" | "announcegreen" | "announceorange" | "announcepurple" => {
            if c.args.is_empty() {
                return Ok(Flow::Usage);
            }
            let color = match c.command {
                "announce" => "primary",
                "announceblue" => "blue",
                "announcegreen" => "green",
                "announceorange" => "orange",
                _ => "purple",
            };
            b.invoke("send_chat_announcement", json!({ "broadcasterId": id, "message": c.from(0), "color": color })).await?;
        }
        "shoutout" | "mod" | "unmod" | "vip" | "unvip" | "raid" | "block" | "unblock" => {
            if c.args.is_empty() {
                return Ok(Flow::Usage);
            }
            let username = strip_first_at(c.arg(0));
            let (cmd, done, with_broadcaster) = match c.command {
                "shoutout" => ("send_shoutout", format!("Shoutout given to {username}"), true),
                "mod" => ("add_channel_moderator", format!("{username} is now a moderator"), true),
                "unmod" => ("remove_channel_moderator", format!("{username} is no longer a moderator"), true),
                "vip" => ("add_channel_vip", format!("{username} is now a VIP"), true),
                "unvip" => ("remove_channel_vip", format!("{username} is no longer a VIP"), true),
                "raid" => ("start_raid", format!("Raiding {username}"), true),
                "block" => ("block_user", format!("Blocked {username}"), false),
                _ => ("unblock_user", format!("Unblocked {username}"), false),
            };
            on_user(c, o, &username, cmd, |u| {
                if with_broadcaster {
                    json!({ "broadcasterId": id, "targetUserId": u.id })
                } else {
                    json!({ "targetUserId": u.id })
                }
            }, done)
            .await?;
        }
        "commercial" => {
            let length = (if c.args.is_empty() { Some(30) } else { js_parse_int(c.arg(0)) }).unwrap_or(30);
            b.invoke("start_commercial", json!({ "broadcasterId": id, "length": length })).await?;
            o.ok(format!("Started {length}s commercial"));
        }
        "unraid" => {
            b.invoke("cancel_raid", json!({ "broadcasterId": id })).await?;
            o.ok("Raid canceled");
        }
        "marker" => {
            b.invoke("create_stream_marker", json!({ "userId": id, "description": opt(&c.from(0)) })).await?;
            o.ok("Stream marker created");
        }
        "color" => {
            if c.args.is_empty() {
                o.info("Usage: /color <color name or hex>");
                return Ok(Flow::Done);
            }
            let Some(me) = b.me().filter(|m| !m.id.is_empty()) else {
                o.err("You must be logged in to change your color");
                return Ok(Flow::Done);
            };
            let color = c.arg(0);
            match b.invoke("update_user_chat_color", json!({ "targetUserId": me.id, "color": color })).await {
                Ok(_) => o.ok(format!("Chat color changed to {color}")),
                Err(e) => o.err(format!("Failed to change color: {e}")),
            }
        }
        "w" => {
            if c.args.len() >= 2 {
                let username = strip_first_at(c.arg(0));
                let message = c.from(1);
                let done = format!("Whisper sent to {username}");
                on_user(c, o, &username, "send_whisper", |u| json!({ "toUserId": u.id, "message": message }), done).await?;
            } else if c.args.len() == 1 {
                o.info("Usage: /w <username> <message>");
            } else {
                return Ok(Flow::Usage);
            }
        }
        "disconnect" => match b.invoke("stop_chat", json!({})).await {
            Ok(v) => o.info(if v.as_bool().unwrap_or(false) {
                "Disconnected from chat"
            } else {
                "Chat stayed connected because another chat window is still using it"
            }),
            Err(e) => {
                log::warn!("[Command Handler] Disconnect failed: {e}");
                o.err(format!("Couldn't disconnect from chat: {e}"));
            }
        },
        "mods" | "vips" => {
            // Read signed in (the web session, then the rewards sign-in, then
            // anonymous), every page of the list. A list Twitch would not give
            // is said as such, with who of the group is in chat now, never as
            // "none".
            if login.is_empty() {
                o.err(format!("/{}: open a channel first", c.command));
                return Ok(Flow::Done);
            }
            let lines = b.invoke("slash_role_list", json!({ "channel": login, "role": c.command })).await?;
            o.say_all(as_lines(lines));
        }
        "monitor" | "unmonitor" | "restrict" | "unrestrict" => {
            if c.args.is_empty() {
                o.info(format!("Usage: /{} <username>", c.command));
                return Ok(Flow::Done);
            }
            let username = strip_first_at(c.arg(0));
            let (status, done) = match c.command {
                "monitor" => ("ACTIVE_MONITORING", format!("Now monitoring {username}")),
                "unmonitor" => ("NO_TREATMENT", format!("Stopped monitoring {username}")),
                "restrict" => ("RESTRICTED", format!("Restricted {username}")),
                _ => ("NO_TREATMENT", format!("Unrestricted {username}")),
            };
            on_user(c, o, &username, "update_suspicious_user_status", |u| {
                json!({ "broadcasterId": id, "targetUserId": u.id, "status": status })
            }, done)
            .await?;
        }
        "warn" => {
            if c.args.len() < 2 {
                o.info("Usage: /warn <username> <reason>");
                return Ok(Flow::Done);
            }
            let username = strip_first_at(c.arg(0));
            let reason = c.from(1);
            let done = format!("Warning sent to {username}");
            on_user(c, o, &username, "warn_chat_user", |u| {
                json!({ "broadcasterId": id, "targetUserId": u.id, "reason": reason })
            }, done)
            .await?;
        }
        "shield" | "shieldoff" => {
            let on = c.command == "shield";
            b.invoke("update_shield_mode", json!({ "broadcasterId": id, "isActive": on })).await?;
            o.ok(if on { "Shield mode activated" } else { "Shield mode deactivated" });
        }
        "me" => return Ok(Flow::PassThrough),
        "clearmessages" => {
            o.ui = Some(Ui::ClearLocalChat);
            o.say("Cleared messages in this chat (visual only).");
        }
        "openurl" => {
            if c.args.is_empty() {
                o.info("Usage: /openurl <url>");
                return Ok(Flow::Done);
            }
            let target = c.arg(0);
            let candidate = if target.contains("://") { target.to_string() } else { format!("https://{target}") };
            match url::Url::parse(&candidate) {
                Ok(parsed) if !matches!(parsed.scheme(), "http" | "https") => {
                    o.err(format!("Refused to open {}: URL", parsed.scheme()));
                }
                Ok(parsed) => {
                    if let Err(e) = b.invoke("open_url", json!({ "url": parsed.as_str() })).await {
                        o.err(format!("Couldn't open {target}: {e}"));
                    }
                }
                Err(_) => o.err(format!("Not a valid URL: {target}")),
            }
        }
        "popout" => {
            let channel = strip_first_at(if c.arg(0).is_empty() { login.as_str() } else { c.arg(0) }).to_lowercase();
            if channel.is_empty() {
                o.info("Usage: /popout [channel]");
                return Ok(Flow::Done);
            }
            b.invoke("open_url", json!({ "url": format!("https://www.twitch.tv/popout/{channel}/chat?popout=") })).await?;
        }
        "r" => {
            let text = c.from(0).trim().to_string();
            let Some((last_login, last_name, last_id)) = b.last_whisper() else {
                o.info("No whisper to reply to yet");
                return Ok(Flow::Done);
            };
            if text.is_empty() {
                o.info(format!("Usage: /r <message>  (replies to {last_name})"));
                return Ok(Flow::Done);
            }
            let to = if last_id.is_empty() { lookup(b, &last_login).await.map(|u| u.id) } else { Some(last_id) };
            let Some(to) = to else {
                o.err(format!("Could not find {last_name}"));
                return Ok(Flow::Done);
            };
            b.invoke("send_whisper", json!({ "toUserId": to, "message": text })).await?;
            o.ok(format!("Whisper sent to {last_name}"));
        }
        "clip" => {
            o.ui = Some(Ui::CreateClip {
                channel: login.to_string(),
                channel_id: id.to_string(),
                title: c.from(0).trim().to_string(),
            })
        }
        "chatters" => {
            if id.is_empty() || login.is_empty() {
                o.say("No channel open.");
                return Ok(Flow::Done);
            }
            match b.invoke("get_channel_chatters", json!({ "broadcasterId": id, "channelLogin": login })).await {
                Ok(res) => o.say(match count_chatters(&res) {
                    None => "Chatter count unavailable for this channel.".to_string(),
                    Some(n) => format!(
                        "{} chatter{} connected.",
                        crate::services::slash_commands::group(n),
                        if n == 1 { "" } else { "s" }
                    ),
                }),
                Err(e) => {
                    log::warn!("[Command Handler] /chatters failed: {e}");
                    o.err(format!("Couldn't count the chatters: {e}"));
                }
            }
        }
        "ignore" => {
            let target = strip_first_at(c.arg(0)).to_lowercase();
            if target.is_empty() {
                o.info("Usage: /ignore <user>");
                return Ok(Flow::Done);
            }
            b.invoke("set_chat_user_hidden", json!({ "name": target, "channelKey": null, "hidden": true })).await?;
            o.say(format!("Hiding {target} everywhere. Undo in Settings > Chat > Hidden Messages."));
        }
        "settitle" | "setgame" => {
            let value = c.from(0).trim().to_string();
            let title = c.command == "settitle";
            if value.is_empty() || id.is_empty() {
                o.info(if title { "Usage: /settitle <title>" } else { "Usage: /setgame <category>" });
                return Ok(Flow::Done);
            }
            let args = if title {
                json!({ "broadcasterId": id, "title": value, "gameName": null })
            } else {
                json!({ "broadcasterId": id, "title": null, "gameName": value })
            };
            match b.invoke("update_channel_info", args).await {
                Ok(_) => o.ok(if title { "Stream title updated".to_string() } else { format!("Category set to {value}") }),
                Err(e) => o.err(e),
            }
        }
        "overlay" | "popup" => {
            let channel = strip_first_at(if c.arg(0).is_empty() { login.as_str() } else { c.arg(0) }).to_lowercase();
            if channel.is_empty() {
                o.info(format!("Usage: /{} [channel]", c.command));
                return Ok(Flow::Done);
            }
            let channel_id = if channel == login && !id.is_empty() {
                Some(id.clone())
            } else {
                lookup(b, &channel).await.map(|u| u.id)
            };
            let window = if c.command == "overlay" { "overlay" } else { "multichat" };
            o.ui = Some(Ui::OpenChatWindow { window: window.to_string(), channel, channel_id });
        }
        // A failed check is said as such: "offline" only when Twitch answered that it is.
        "uptime" => match uptime_stream(b, &login).await {
            Err(e) => o.err(format!("Couldn't check whether this channel is live: {e}")),
            Ok(None) => o.say("This channel is offline."),
            Ok(Some(s)) => {
                let uptime = format_uptime(s["started_at"].as_str(), b.now_ms());
                let name = s["user_name"]
                    .as_str()
                    .filter(|n| !n.is_empty())
                    .or_else(|| s["user_login"].as_str().filter(|n| !n.is_empty()))
                    .unwrap_or("This channel");
                o.say(format!("{name} has been live for {uptime}."));
            }
        },
        "user" | "usercard" => {
            if c.args.is_empty() {
                o.info(format!("Usage: /{} <user>", c.command));
                return Ok(Flow::Done);
            }
            let username = strip_first_at(c.arg(0));
            match user_or_why(b, &username, format!("User {username} not found")).await {
                Ok(u) => o.ui = Some(Ui::OpenUserCard { user_id: u.id, login: u.login, display_name: u.display_name }),
                Err(why) => o.err(why),
            }
        }
        "refresh" => {
            if login.is_empty() || id.is_empty() {
                o.err("/refresh: no active channel");
                return Ok(Flow::Done);
            }
            match b.invoke("refresh_emotes", json!({ "channel": login, "channelId": id })).await {
                Ok(total) => {
                    o.ui = Some(Ui::RefreshEmotes);
                    o.say(format!("Emotes refreshed ({} loaded).", total.as_i64().unwrap_or(0)));
                }
                Err(e) => o.err(format!("/refresh failed: {e}")),
            }
        }
        "reload" => o.ui = Some(Ui::ReloadStream),
        "remind" => remind(c, o).await?,
        "nuke" => nuke_command(c, o).await,
        "undo" => {
            if login.is_empty() {
                o.err("/undo: no active channel");
                return Ok(Flow::Done);
            }
            if !is_moderator_of(c.ctx) {
                o.err("/undo is a moderator-only command");
                return Ok(Flow::Done);
            }
            let Some(plan) = nuke::take_last(&login) else {
                o.say("/undo: no nuke to undo on this channel.");
                return Ok(Flow::Done);
            };
            if plan.action == nuke::NukeAction::Delete {
                let n: usize = plan.affected.iter().map(|a| a.message_ids.len()).sum();
                o.say(format!("/undo: cannot restore {n} deleted message(s). Twitch does not allow message un-delete."));
                return Ok(Flow::Done);
            }
            let mut unbanned = 0;
            for a in &plan.affected {
                match b.invoke("unban_user", json!({ "broadcasterId": plan.broadcaster_id, "targetUserId": a.user_id })).await {
                    Ok(_) => unbanned += 1,
                    Err(e) => log::warn!("[undo] failed to unban {}: {e}", a.username),
                }
            }
            o.say(format!(
                "/undo reversed nuke on \"{}\": unbanned {unbanned}/{} user(s).",
                plan.source,
                plan.affected.len()
            ));
        }
        "banid" => {
            if c.args.is_empty() {
                o.info("Usage: /banid <userID> [reason]");
                return Ok(Flow::Done);
            }
            let target = c.arg(0);
            if !target.chars().all(|ch| ch.is_ascii_digit()) {
                o.err("User ID must be numeric. Did you mean /ban?");
                return Ok(Flow::Done);
            }
            let reason = c.from(1);
            match b
                .invoke("ban_user", json!({ "broadcasterId": id, "targetUserId": target, "duration": null, "reason": opt(&reason) }))
                .await
            {
                Ok(_) => o.ok(format!("Banned user ID {target}")),
                Err(e) => o.err(format!("Ban failed: {e}")),
            }
        }
        "poll" | "prediction" => {
            o.ui = Some(Ui::OpenPollComposer { channel: login.clone(), composer: c.command.to_string() });
        }
        "endpoll" | "cancelpoll" => {
            if id.is_empty() {
                o.err(format!("/{}: no active channel", c.command));
                return Ok(Flow::Done);
            }
            // Twitch reads and ends polls only on the channel owner's own token.
            if b.role(&login) != Role::Broadcaster {
                o.err(format!("/{}: only the broadcaster can end a poll on Twitch", c.command));
                return Ok(Flow::Done);
            }
            let active = match b.invoke("get_active_poll", json!({ "broadcasterId": id })).await {
                Ok(active) => active,
                Err(e) => {
                    o.err(format!("Couldn't read the running poll: {e}"));
                    return Ok(Flow::Done);
                }
            };
            let Some(poll_id) = active["id"].as_str().filter(|s| !s.is_empty()).map(str::to_string) else {
                o.err("No poll is running right now");
                return Ok(Flow::Done);
            };
            let end = c.command == "endpoll";
            let status = if end { "TERMINATED" } else { "ARCHIVED" };
            match b.invoke("end_poll", json!({ "broadcasterId": id, "pollId": poll_id, "status": status })).await {
                Ok(_) => o.ok(if end { "Poll ended" } else { "Poll canceled" }),
                Err(e) => o.err(e),
            }
        }
        "lockprediction" | "cancelprediction" | "completeprediction" => {
            if id.is_empty() {
                o.err(format!("/{}: no active channel", c.command));
                return Ok(Flow::Done);
            }
            let action = c.command.replacen("prediction", "", 1);
            let outcome = c.from(0).trim().to_string();
            match b.invoke("finish_prediction", json!({ "broadcasterId": id, "action": action, "outcome": opt(&outcome) })).await {
                Ok(done) => o.ok(done.as_str().unwrap_or("").to_string()),
                Err(e) => o.err(e),
            }
        }
        "blockterm" | "unblockterm" => {
            if id.is_empty() || login.is_empty() {
                o.err(format!("/{}: no active channel", c.command));
                return Ok(Flow::Done);
            }
            if !is_moderator_of(c.ctx) {
                o.err(format!("/{} is a moderator-only command", c.command));
                return Ok(Flow::Done);
            }
            let phrase = c.from(0).trim().to_string();
            if phrase.is_empty() {
                o.err(format!("/{} needs a phrase", c.command));
                return Ok(Flow::Done);
            }
            let block = c.command == "blockterm";
            let cmd = if block { "add_blocked_term" } else { "remove_blocked_term" };
            match b.invoke(cmd, json!({ "broadcasterId": id, "text": phrase })).await {
                Ok(_) => o.ok(if block { format!("Blocked \"{phrase}\"") } else { format!("Unblocked \"{phrase}\"") }),
                Err(e) => o.err(e),
            }
        }
        "pin" => {
            if id.is_empty() || login.is_empty() {
                o.err("/pin: no active channel");
                return Ok(Flow::Done);
            }
            if !is_moderator_of(c.ctx) {
                o.err("/pin is a moderator-only command");
                return Ok(Flow::Done);
            }
            match resolve_pin_args(c.args, |who| b.latest_message_id(&login, who)) {
                Err(e) => o.err(e),
                Ok(PinTarget::Send { text }) => {
                    if b.me().is_none() {
                        o.err("/pin: sign in to send a message");
                        return Ok(Flow::Done);
                    }
                    // Sent through the composer's own send path, which pins it
                    // once Twitch has given it an id.
                    o.send = Some(text);
                    o.pin_after_send = true;
                }
                Ok(PinTarget::Message { message_id, duration_seconds }) => {
                    match b
                        .invoke(
                            "pin_chat_message",
                            json!({ "broadcasterId": id, "messageId": message_id, "durationSeconds": duration_seconds }),
                        )
                        .await
                    {
                        Ok(_) => o.ok(match duration_seconds {
                            Some(s) => format!("Pinned for {s}s"),
                            None => "Message pinned".to_string(),
                        }),
                        Err(e) => o.err(e),
                    }
                }
            }
        }
        "unpin" => {
            if id.is_empty() || login.is_empty() {
                o.err("/unpin: no active channel");
                return Ok(Flow::Done);
            }
            if !is_moderator_of(c.ctx) {
                o.err("/unpin is a moderator-only command");
                return Ok(Flow::Done);
            }
            match b.invoke("unpin_chat_message", json!({ "broadcasterId": id })).await {
                Ok(_) => o.ok("Message unpinned"),
                Err(e) => o.err(e),
            }
        }
        _ => return Ok(Flow::NotMine),
    }
    Ok(Flow::Done)
}

async fn remind(c: &Call<'_>, o: &mut Outcome) -> Result<(), String> {
    let b = c.b();
    let settings = b.settings();
    let reminders: Vec<Value> = settings["reminders"]["reminders"].as_array().cloned().unwrap_or_default();
    let write = |list: Vec<Value>| b.invoke("patch_settings", json!({ "patch": { "reminders": { "reminders": list } } }));
    match reminder_service::parse_remind_command(c.args, c.login(), c.id(), &b.new_id()) {
        RemindParse::Error(e) => o.info(e),
        RemindParse::Help => o.say_all(reminder_service::remind_help_lines()),
        RemindParse::List => o.say_all(reminder_service::format_reminder_list(&reminders)),
        RemindParse::Clear => {
            if reminders.is_empty() {
                o.say("You have no reminders to clear.");
            } else {
                write(Vec::new()).await?;
                let n = reminders.len();
                o.say(format!("Cleared {n} reminder{}.", if n == 1 { "" } else { "s" }));
            }
        }
        RemindParse::Manage { op, index } => {
            let i = index - 1;
            if i >= reminders.len() {
                o.say(format!("There's no reminder #{index}. Run /remind list to see them."));
                return Ok(());
            }
            if op == "remove" {
                let list = reminders.iter().enumerate().filter(|(k, _)| *k != i).map(|(_, r)| r.clone()).collect();
                write(list).await?;
                o.say(format!("Removed reminder #{index}."));
                return Ok(());
            }
            let enabled = if op == "toggle" { !reminders[i]["enabled"].as_bool().unwrap_or(false) } else { op == "enable" };
            let mut list = reminders.clone();
            list[i]["enabled"] = json!(enabled);
            write(list).await?;
            o.say(format!("Reminder #{index} {}.", if enabled { "enabled" } else { "disabled" }));
        }
        RemindParse::SendNow { message, repeat } => {
            let posted = !c.login().is_empty()
                && b
                    .invoke("post_reminder", json!({ "channel": c.login(), "channelId": opt(c.id()), "message": message, "repeat": repeat }))
                    .await
                    .is_ok();
            if !posted {
                o.err("/remind now: no active channel to post in");
            }
        }
        RemindParse::Create { reminder, summary } => {
            let n = reminders.len();
            let mut list = reminders;
            list.push(reminder);
            write(list).await?;
            let summary = if summary.is_empty() { "Reminder set".to_string() } else { summary };
            o.say(format!("{summary} (now #{}). /remind list to manage.", n + 1));
        }
    }
    Ok(())
}

async fn nuke_command(c: &Call<'_>, o: &mut Outcome) {
    let b = c.b();
    let login = c.login().to_string();
    if login.is_empty() {
        o.err("/nuke: no active channel");
        return;
    }
    if !is_moderator_of(c.ctx) {
        o.err("/nuke is a moderator-only command");
        return;
    }
    let raw = c.from(0);
    if raw.trim().eq_ignore_ascii_case("stop") {
        let stopped = nuke::stop(&login);
        if stopped > 0 {
            o.ok(format!("/nuke: stopped {stopped} active window(s)"));
        } else {
            o.info("/nuke: nothing running on this channel");
        }
        return;
    }
    let parsed = match nuke::parse_nuke_args(&raw) {
        Ok(p) => p,
        Err(e) => {
            o.err(e);
            return;
        }
    };
    if !b.is_open(&login).await {
        o.say("/nuke: no chat connected for this channel.");
        o.ok("/nuke: 0 message(s), 0 user(s)");
        return;
    }
    let now = b.now_ms();
    let entries = b.history(&login, now - parsed.past_seconds as i64 * 1000);
    let (matched, affected) = nuke::sweep(&entries, &parsed, now - parsed.past_seconds as i64 * 1000);
    let reason = nuke::reason_from(&b.settings());
    let action = parsed.action;
    let calls = affected.iter().map(|a| {
        let reason = reason.clone();
        let id = c.id().to_string();
        async move {
            let results: Vec<Result<Value, String>> = match action {
                nuke::NukeAction::Delete => {
                    let mut out = Vec::new();
                    for m in &a.message_ids {
                        out.push(b.invoke("delete_chat_message", json!({ "broadcasterId": id, "messageId": m })).await);
                    }
                    out
                }
                nuke::NukeAction::Ban => vec![
                    b.invoke("ban_user", json!({ "broadcasterId": id, "targetUserId": a.user_id, "duration": null, "reason": reason }))
                        .await,
                ],
                nuke::NukeAction::Timeout(s) => vec![
                    b.invoke("ban_user", json!({ "broadcasterId": id, "targetUserId": a.user_id, "duration": s, "reason": reason }))
                        .await,
                ],
            };
            let failure = results.into_iter().find_map(Result::err);
            if let Some(e) = &failure {
                log::warn!("[nuke] action failed for {}: {e}", a.username);
            }
            failure
        }
    });
    // Each user's first failure, so the toast never reads as a clean sweep when it was not.
    let failures: Vec<String> = futures::future::join_all(calls).await.into_iter().flatten().collect();
    let users = affected.len();
    let record = nuke::remember(&login, c.id(), &parsed, affected, now);
    if parsed.future_seconds > 0 {
        let backend = c.ctx.b.clone();
        let channel = login.clone();
        let secs = parsed.future_seconds;
        let task = tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
            if let Some((source, extra)) = nuke::close(&channel, record) {
                backend.post_notice(&channel, nuke::closed_line(&source, extra)).await;
            }
        });
        nuke::set_closer(record, task);
    }
    o.say(nuke::ran_line(&parsed, matched, users));
    match failures.first() {
        None => o.ok(format!("/nuke: {matched} message(s), {users} user(s)")),
        Some(first) => o.err(format!("/nuke: {} of {users} user(s) failed: {first}", failures.len())),
    }
}

/// The extended commands that work in any platform's chat.
pub const PLATFORM_NEUTRAL: &[&str] = &[
    "help", "copy", "echo", "say", "tts", "shrug", "lurk", "squishy", "barrelroll", "party", "exporttext", "openfile",
    "proc", "screenshot", "setting", "timer", "chain", "foreach", "debug-env", "debug-chat", "debug-eventsub",
    "debug-logs", "debug-logging", "openstreamhighlights",
];

const SQUISHY: &str =
    "notsquishY WHEN YOU NEED HIM notsquishY IN A JIFFY notsquishY USE THIS EMOTE notsquishY TO SUMMON SQUISHY notsquishY";

/// The table's usage line for an extended command.
fn ext_usage(o: &mut Outcome, command: &str, fallback: &str) {
    let usage = find_builtin(command).map(|d| d.usage).unwrap_or(fallback);
    o.info(format!("Usage: {usage}"));
}

fn need_channel(c: &Call<'_>, o: &mut Outcome) -> bool {
    if !c.id().is_empty() && !c.login().is_empty() {
        return true;
    }
    o.err(format!("/{}: open a channel first", c.typed));
    false
}

fn need_staff(c: &Call<'_>, o: &mut Outcome) -> bool {
    if is_broadcaster_of(c.ctx) || is_moderator_of(c.ctx) {
        return true;
    }
    o.err(format!("/{} is for moderators and the broadcaster", c.typed));
    false
}

fn need_broadcaster(c: &Call<'_>, o: &mut Outcome) -> bool {
    if is_broadcaster_of(c.ctx) {
        return true;
    }
    o.err(format!("/{} is for the broadcaster of this channel", c.typed));
    false
}

fn as_lines(v: Value) -> Vec<String> {
    match v {
        Value::Array(a) => a.into_iter().map(|l| l.as_str().map(str::to_string).unwrap_or_else(|| l.to_string())).collect(),
        Value::String(s) => vec![s],
        Value::Null => vec![],
        other => vec![other.to_string()],
    }
}

/// The CSS color names a browser accepts.
const CSS_COLORS: &[&str] = &[
    "aliceblue", "antiquewhite", "aqua", "aquamarine", "azure", "beige", "bisque", "black", "blanchedalmond", "blue",
    "blueviolet", "brown", "burlywood", "cadetblue", "chartreuse", "chocolate", "coral", "cornflowerblue", "cornsilk",
    "crimson", "cyan", "darkblue", "darkcyan", "darkgoldenrod", "darkgray", "darkgreen", "darkgrey", "darkkhaki",
    "darkmagenta", "darkolivegreen", "darkorange", "darkorchid", "darkred", "darksalmon", "darkseagreen",
    "darkslateblue", "darkslategray", "darkslategrey", "darkturquoise", "darkviolet", "deeppink", "deepskyblue",
    "dimgray", "dimgrey", "dodgerblue", "firebrick", "floralwhite", "forestgreen", "fuchsia", "gainsboro",
    "ghostwhite", "gold", "goldenrod", "gray", "green", "greenyellow", "grey", "honeydew", "hotpink", "indianred",
    "indigo", "ivory", "khaki", "lavender", "lavenderblush", "lawngreen", "lemonchiffon", "lightblue", "lightcoral",
    "lightcyan", "lightgoldenrodyellow", "lightgray", "lightgreen", "lightgrey", "lightpink", "lightsalmon",
    "lightseagreen", "lightskyblue", "lightslategray", "lightslategrey", "lightsteelblue", "lightyellow", "lime",
    "limegreen", "linen", "magenta", "maroon", "mediumaquamarine", "mediumblue", "mediumorchid", "mediumpurple",
    "mediumseagreen", "mediumslateblue", "mediumspringgreen", "mediumturquoise", "mediumvioletred", "midnightblue",
    "mintcream", "mistyrose", "moccasin", "navajowhite", "navy", "oldlace", "olive", "olivedrab", "orange",
    "orangered", "orchid", "palegoldenrod", "palegreen", "paleturquoise", "palevioletred", "papayawhip", "peachpuff",
    "peru", "pink", "plum", "powderblue", "purple", "rebeccapurple", "red", "rosybrown", "royalblue", "saddlebrown",
    "salmon", "sandybrown", "seagreen", "seashell", "sienna", "silver", "skyblue", "slateblue", "slategray",
    "slategrey", "snow", "springgreen", "steelblue", "tan", "teal", "thistle", "tomato", "transparent", "turquoise",
    "violet", "wheat", "white", "whitesmoke", "yellow", "yellowgreen",
];

/// Whether `/setcolor` can use `value` as a name color.
pub fn is_color(value: &str) -> bool {
    let v = value.trim().to_ascii_lowercase();
    let hex = v.strip_prefix('#').unwrap_or(&v);
    if hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return true;
    }
    if let Some(h) = v.strip_prefix('#') {
        if matches!(h.len(), 3 | 4 | 8) && h.chars().all(|c| c.is_ascii_hexdigit()) {
            return true;
        }
    }
    if ["rgb(", "rgba(", "hsl(", "hsla(", "hwb(", "lab(", "lch(", "oklab(", "oklch("].iter().any(|p| v.starts_with(p)) && v.ends_with(')') {
        return true;
    }
    CSS_COLORS.contains(&v.as_str())
}

fn is_off_word(v: &str) -> bool {
    matches!(v.to_ascii_lowercase().as_str(), "off" | "none" | "reset" | "clear")
}

/// The commands added after the original table. Ok(false) means "not mine".
async fn extended(c: &Call<'_>, o: &mut Outcome) -> Result<bool, String> {
    let b = c.b();
    let id = c.id().to_string();
    let login = c.login().to_string();
    let lines = |cmd: &'static str, args: Value| async move { b.invoke(cmd, args).await.map(as_lines) };
    match c.command {
        "help" => o.say_all(help_lines(c.args.first().copied(), c.ctx.platform.phone())),
        "vote" => {
            if !need_channel(c, o) {
                return Ok(true);
            }
            if c.rest.is_empty() {
                ext_usage(o, "vote", "/vote <number or option>");
                return Ok(true);
            }
            match b.invoke("slash_vote", json!({ "channelId": id, "choice": c.rest })).await {
                Ok(v) => o.say_all(as_lines(v)),
                Err(e) => o.err(e),
            }
        }
        "gift" => {
            if !need_channel(c, o) {
                return Ok(true);
            }
            if is_broadcaster_of(c.ctx) {
                o.info("You can't gift subs in your own channel");
                return Ok(true);
            }
            let qty = js_parse_int(c.arg(0)).filter(|q| *q > 0);
            b.invoke(
                "open_subscribe_window",
                json!({ "channelLogin": login, "title": format!("Gift subs to {login}"), "provider": "twitch" }),
            )
            .await?;
            o.say(match qty {
                Some(q) => format!("Opened {login}'s sub page. Choose \"Gift a sub\" and pick {q}. Payment happens on Twitch."),
                None => format!("Opened {login}'s sub page. Choose \"Gift a sub\" there. Payment happens on Twitch."),
            });
        }
        "rules" | "follows" | "viewers" | "founders" => {
            let channel = Some(clean_login(c.arg(0))).filter(|s| !s.is_empty()).unwrap_or_else(|| login.clone());
            if channel.is_empty() {
                o.err(format!("/{}: open a channel first", c.command));
                return Ok(true);
            }
            let cmd = match c.command {
                "rules" => "slash_chat_rules",
                "follows" => "slash_follower_count",
                "viewers" => "slash_viewer_count",
                _ => "slash_founders",
            };
            o.say_all(lines(cmd, json!({ "channel": channel })).await?);
        }
        "followage" => {
            let a = clean_login(c.arg(0));
            let second = clean_login(c.arg(1));
            let channel = if second.is_empty() { login.clone() } else { second };
            if channel.is_empty() {
                o.err("/followage: open a channel first");
                return Ok(true);
            }
            o.say_all(lines("slash_followage", json!({ "channel": channel, "user": opt(&a) })).await?);
        }
        "accountage" => {
            let typed_user = clean_login(c.arg(0));
            let user = if typed_user.is_empty() { b.me().map(|m| m.login.to_lowercase()).unwrap_or_default() } else { typed_user };
            if user.is_empty() {
                o.info("Usage: /accountage <user>");
                return Ok(true);
            }
            o.say_all(lines("slash_account_age", json!({ "user": user })).await?);
        }
        "namehistory" => {
            let user = clean_login(c.arg(0));
            if user.is_empty() {
                ext_usage(o, "namehistory", "/namehistory <user>");
                return Ok(true);
            }
            o.say_all(lines("slash_name_history", json!({ "user": user })).await?);
        }
        "logs" => {
            let who = clean_login(c.arg(0));
            if who.is_empty() {
                ext_usage(o, "logs", "/logs <user>");
                return Ok(true);
            }
            if login.is_empty() {
                o.err("/logs: open a channel first");
                return Ok(true);
            }
            o.say_all(lines("slash_recent_messages", json!({ "channel": login, "login": who, "limit": 10 })).await?);
        }
        "reply" => {
            let who = clean_login(c.arg(0));
            let text = c.from(1).trim().to_string();
            if who.is_empty() || text.is_empty() {
                ext_usage(o, "reply", "/reply <@user> <message>");
                return Ok(true);
            }
            if !need_channel(c, o) {
                return Ok(true);
            }
            let hit = b.invoke("slash_last_message", json!({ "channel": login, "login": who })).await?;
            let Some(parent) = hit["id"].as_str().filter(|s| !s.is_empty()).map(str::to_string) else {
                o.err(format!("No recent message from {who} in this chat to reply to"));
                return Ok(true);
            };
            if b.me().is_none() {
                o.err("Sign in to Twitch to reply");
                return Ok(true);
            }
            o.send = Some(text);
            o.reply_to = Some(parent);
        }
        "redeem" => {
            if need_channel(c, o) {
                o.ui = Some(Ui::OpenChannelPoints { channel: login.clone() });
            }
        }
        "copy" => {
            if c.rest.is_empty() {
                ext_usage(o, "copy", "/copy <text>");
                return Ok(true);
            }
            b.invoke("copy_text", json!({ "text": c.rest })).await?;
            o.ok("Copied");
        }
        "unignore" => {
            let target = clean_login(c.arg(0));
            if target.is_empty() {
                ext_usage(o, "unignore", "/unignore <user>");
                return Ok(true);
            }
            b.invoke("set_chat_user_hidden", json!({ "name": target, "channelKey": null, "hidden": false })).await?;
            o.say(format!("{target} is no longer hidden."));
        }
        "join" => {
            let who = clean_login(c.arg(0));
            if who.is_empty() {
                ext_usage(o, "join", "/join <channel>");
                return Ok(true);
            }
            let user = match user_or_why(b, &who, format!("There is no Twitch channel named {who}")).await {
                Ok(user) => user,
                Err(why) => {
                    o.err(why);
                    return Ok(true);
                }
            };
            let shown = if user.display_name.is_empty() { user.login.clone() } else { user.display_name.clone() };
            o.ui = Some(Ui::DockJoin { login: user.login, channel_id: user.id, display_name: shown.clone(), avatar_url: user.avatar });
            o.ok(format!("Joined {shown}'s chat in the dock"));
        }
        "part" => {
            let who = clean_login(c.arg(0));
            o.ui = Some(Ui::DockPart { login: if who.is_empty() { login.clone() } else { who } });
        }
        "reconnect" => {
            if login.is_empty() {
                o.err("/reconnect: open a channel first");
                return Ok(true);
            }
            // The refresh empties this chat, so the notice comes after it.
            o.info("Reconnecting chat...");
            o.ui = Some(Ui::ReloadChat);
            o.say("Chat reconnected.");
        }
        "echo" => {
            if c.rest.is_empty() {
                ext_usage(o, "echo", "/echo <text>");
                return Ok(true);
            }
            o.say(c.rest);
        }
        "say" => {
            if c.rest.is_empty() {
                ext_usage(o, "say", "/say <text>");
                return Ok(true);
            }
            o.send = Some(c.rest.to_string());
        }
        "search" => o.ui = Some(Ui::OpenSearch),
        "setcolor" => {
            let who = clean_login(c.arg(0));
            let value = c.arg(1).trim().to_string();
            if who.is_empty() || value.is_empty() {
                ext_usage(o, "setcolor", "/setcolor <user> <color|off>");
                return Ok(true);
            }
            let user = match user_or_why(b, &who, format!("There is no Twitch user named {who}")).await {
                Ok(user) => user,
                Err(why) => {
                    o.err(why);
                    return Ok(true);
                }
            };
            let off = is_off_word(&value);
            if !off && !is_color(&value) {
                o.err(format!("\"{value}\" is not a color. Use a name like orange or a hex like #ff8800"));
                return Ok(true);
            }
            let color = if off {
                None
            } else if value.len() == 6 && value.chars().all(|ch| ch.is_ascii_hexdigit()) {
                Some(format!("#{value}"))
            } else {
                Some(value.clone())
            };
            b.invoke(
                "set_chat_user_override",
                json!({ "userId": user.id, "username": user.login, "field": "color", "value": color }),
            )
            .await?;
            o.say(match color {
                None => format!("{}'s name color is back to their own.", user.display_name),
                Some(col) => format!("{}'s name now shows in {col} for you.", user.display_name),
            });
        }
        "setname" => {
            let who = clean_login(c.arg(0));
            let nickname = c.from(1).trim().to_string();
            if who.is_empty() || nickname.is_empty() {
                ext_usage(o, "setname", "/setname <user> <nickname|off>");
                return Ok(true);
            }
            let user = match user_or_why(b, &who, format!("There is no Twitch user named {who}")).await {
                Ok(user) => user,
                Err(why) => {
                    o.err(why);
                    return Ok(true);
                }
            };
            let off = is_off_word(&nickname);
            b.invoke(
                "set_chat_user_override",
                json!({ "userId": user.id, "username": user.login, "field": "nickname", "value": if off { Value::Null } else { json!(nickname) } }),
            )
            .await?;
            o.say(if off {
                format!("{} shows under their own name again.", user.display_name)
            } else {
                format!("{} now shows as \"{nickname}\" for you.", user.display_name)
            });
        }
        "addstreamhighlight" => {
            if login.is_empty() {
                o.err("/addstreamhighlight: open a channel first");
                return Ok(true);
            }
            o.say_all(lines("slash_add_stream_highlight", json!({ "channel": login, "comment": opt(c.rest) })).await?);
        }
        "openstreamhighlights" => {
            b.invoke("slash_open_stream_highlights", json!({})).await?;
        }
        "favorite" | "unfavorite" => favorite(c, o, c.command == "favorite").await?,
        "tts" => {
            if c.rest.is_empty() {
                ext_usage(o, "tts", "/tts <text>");
                return Ok(true);
            }
            o.ui = Some(Ui::Speak { text: c.rest.to_string() });
        }
        "screenshot" => o.ui = Some(Ui::Screenshot),
        "7tv" => seventv(c, o).await?,
        "emoteinfo" => {
            if login.is_empty() {
                o.err("Open a channel first");
                return Ok(true);
            }
            let counts = match b.invoke("emote_counts", json!({ "channel": login, "channelId": opt(&id) })).await {
                Ok(counts) => counts,
                Err(e) => {
                    o.err(format!("Couldn't read this channel's emotes: {e}"));
                    return Ok(true);
                }
            };
            if !counts.is_object() {
                o.say("Emotes for this channel have not loaded yet.");
                return Ok(true);
            }
            let providers: &[(&str, &str, &str)] = &[
                ("twitchemotes", "twitch", "Twitch"),
                ("bttv", "bttv", "BTTV"),
                ("bttvemotes", "bttv", "BTTV"),
                ("seventv", "7tv", "7TV"),
                ("ffz", "ffz", "FFZ"),
                ("ffzglobal", "ffz", "FFZ"),
            ];
            let pick = |name: &str| providers.iter().find(|(n, _, _)| *n == name).map(|(_, k, l)| (*k, *l));
            let only = pick(c.typed).or_else(|| pick(&clean_login(c.arg(0))));
            let rows: Vec<(&str, &str)> = match only {
                Some(one) => vec![one],
                None => vec![("twitch", "Twitch"), ("bttv", "BTTV"), ("7tv", "7TV"), ("ffz", "FFZ")],
            };
            let parts: Vec<String> = rows
                .iter()
                .map(|(k, l)| format!("{l} {}", crate::services::slash_commands::group(counts[*k].as_i64().unwrap_or(0))))
                .collect();
            o.say(format!("Emotes loaded for {login}: {}. /refresh fetches them again.", parts.join(", ")));
        }
        "streamlink" => {
            if login.is_empty() {
                o.err("/streamlink: open a channel first");
                return Ok(true);
            }
            o.say_all(lines("slash_open_streamlink", json!({ "channel": login, "quality": opt(c.arg(0)) })).await?);
        }
        "raw" => {
            if c.rest.is_empty() {
                ext_usage(o, "raw", "/raw <IRC line>");
                return Ok(true);
            }
            b.invoke("slash_send_raw_irc", json!({ "line": c.rest })).await?;
            o.say(format!("Sent: {}", c.rest));
        }
        "fakemsg" => {
            if c.rest.is_empty() {
                ext_usage(o, "fakemsg", "/fakemsg <IRC PRIVMSG line>");
                return Ok(true);
            }
            b.invoke("slash_inject_fake_message", json!({ "line": c.rest })).await?;
        }
        "debug-env" => {
            let text = |cmd: &'static str| async move {
                b.invoke(cmd, json!({})).await.ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| "?".to_string())
            };
            let (version, system, settings_dir, log_dir) =
                (text("get_app_version").await, text("get_system_info").await, text("get_settings_dir").await, text("get_chat_log_dir").await);
            o.say(format!(
                "StreamNook {version} ({} build) on {system}",
                if dev_build() { "development" } else { "release" }
            ));
            o.say(format!("Settings: {settings_dir}"));
            o.say(format!("Chat logs: {log_dir}"));
        }
        "debug-chat" => {
            let log = as_lines(b.invoke("get_chat_lifecycle_log", json!({})).await?);
            let tail: Vec<String> = log.iter().skip(log.len().saturating_sub(8)).cloned().collect();
            if tail.is_empty() {
                o.say("No chat connection events yet.");
            } else {
                o.say("Recent chat connection events:");
                o.say_all(tail);
            }
        }
        "debug-eventsub" => {
            let connected = b.invoke("is_eventsub_connected", json!({})).await.ok().and_then(|v| v.as_bool()).unwrap_or(false);
            let session = b.invoke("get_eventsub_session_id", json!({})).await.ok().and_then(|v| v.as_str().map(str::to_string));
            o.say(if connected {
                format!("EventSub is connected (session {}).", session.unwrap_or_else(|| "unknown".to_string()))
            } else {
                "EventSub is not connected.".to_string()
            });
        }
        "debug-logs" => {
            b.invoke("open_logs_folder", json!({})).await?;
        }
        "debug-logging" => {
            let v = c.arg(0).to_lowercase();
            if v != "on" && v != "off" {
                ext_usage(o, "debug-logging", "/debug-logging <on|off>");
                return Ok(true);
            }
            b.invoke("patch_settings", json!({ "patch": { "error_reporting_enabled": v == "on" } })).await?;
            o.say(if v == "on" { "Detailed logging is on." } else { "Detailed logging is off." });
        }
        "debug-limits" => {
            if login.is_empty() {
                o.err("Open a channel first");
                return Ok(true);
            }
            let bud = b.invoke("chat_send_budget", json!({ "channel": login })).await?;
            let slow = bud["slow_seconds"].as_u64().unwrap_or(0);
            o.say(format!(
                "You are a {} here: {} messages per {}s ({} used), at least {} ms apart{}.",
                bud["role"].as_str().unwrap_or("viewer"),
                bud["per_window"],
                bud["window_seconds"],
                bud["used_in_window"],
                bud["spacing_ms"],
                if slow > 0 { format!(", slow mode {slow}s") } else { String::new() }
            ));
        }
        "setting" => setting(c, o).await?,
        "proc" => {
            if c.rest.is_empty() {
                ext_usage(o, "proc", "/proc <program> [arguments]");
                return Ok(true);
            }
            o.say_all(lines("slash_run_program", json!({ "commandLine": c.rest })).await?);
        }
        "exporttext" => {
            let file = c.arg(0);
            let text = c.from(1);
            if file.is_empty() || text.trim().is_empty() {
                ext_usage(o, "exporttext", "/exporttext <file> <text>");
                return Ok(true);
            }
            let path = b.invoke("slash_export_text", json!({ "file": file, "text": text })).await?;
            o.say(format!("Added to {}", path.as_str().unwrap_or("")));
        }
        "openfile" => {
            b.invoke("slash_open_export", json!({ "file": opt(c.arg(0)) })).await?;
        }
        "timer" => timer(c, o),
        "runin" => {
            let who = clean_login(c.arg(0));
            let line = c.from(1).trim().to_string();
            if who.is_empty() || line.is_empty() {
                ext_usage(o, "runin", "/runin <channel> <command or message>");
                return Ok(true);
            }
            if c.depth >= MAX_SCRIPT_DEPTH {
                o.err("Commands can only nest three deep");
                return Ok(true);
            }
            let user = match user_or_why(b, &who, format!("There is no Twitch channel named {who}")).await {
                Ok(user) => user,
                Err(why) => {
                    o.err(why);
                    return Ok(true);
                }
            };
            let there = Ctx {
                b: c.ctx.b.clone(),
                channel: user.login.to_lowercase(),
                broadcaster_id: user.id,
                platform: c.ctx.platform,
            };
            let s = run_script_line(there, line, c.depth + 1).await;
            merge(o, s);
        }
        "foreach" => {
            let items: Vec<String> = c.arg(0).split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect();
            let template = c.from(1).trim().to_string();
            if items.is_empty() || template.is_empty() {
                ext_usage(o, "foreach", "/foreach <a,b,c> <command with $>");
                return Ok(true);
            }
            if !template.contains('$') {
                o.err("Put $ where each item goes, for example /foreach a,b /shoutout $");
                return Ok(true);
            }
            if items.len() > MAX_FOREACH {
                o.err(format!("/foreach takes up to {MAX_FOREACH} items"));
                return Ok(true);
            }
            if c.depth >= MAX_SCRIPT_DEPTH {
                o.err("Commands can only nest three deep");
                return Ok(true);
            }
            for item in items {
                let s = run_script_line(c.ctx.clone(), template.replace('$', &item), c.depth + 1).await;
                merge(o, s);
            }
        }
        "chain" => {
            let steps: Vec<String> = c.rest.split('|').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect();
            if steps.len() < 2 {
                ext_usage(o, "chain", "/chain <command> | <command>");
                return Ok(true);
            }
            if steps.len() > MAX_CHAIN {
                o.err(format!("/chain takes up to {MAX_CHAIN} steps"));
                return Ok(true);
            }
            if c.depth >= MAX_SCRIPT_DEPTH {
                o.err("Commands can only nest three deep");
                return Ok(true);
            }
            for step in steps {
                let s = run_script_line(c.ctx.clone(), step, c.depth + 1).await;
                merge(o, s);
            }
        }
        "shrug" => {
            let shrug = "¯\\_(ツ)_/¯";
            o.send = Some(if c.rest.is_empty() { shrug.to_string() } else { format!("{} {shrug}", c.rest) });
        }
        "lurk" => {
            // A random line. /me is Twitch's; Kick and YouTube would post it as
            // the letters "/me".
            o.send = Some(crate::services::slash_commands::lurk(Some(c.rest), !login.contains(':')));
        }
        "squishy" => o.send = Some(SQUISHY.to_string()),
        "barrelroll" | "party" => o.ui = Some(Ui::WindowEffect { effect: c.command.to_string() }),
        "localmod" | "localmodoff" | "localsub" | "localsuboff" | "localascii" | "localasciioff" => {
            if login.is_empty() {
                o.err("Open a channel first");
                return Ok(true);
            }
            let (view, on, label) = match c.command {
                "localmod" => ("mod", true, "Showing only messages from moderators and the broadcaster"),
                "localmodoff" => ("mod", false, "Moderator-only view off"),
                "localsub" => ("sub", true, "Showing only messages from subscribers, moderators and the broadcaster"),
                "localsuboff" => ("sub", false, "Subscriber-only view off"),
                "localascii" => ("ascii", true, "Showing only messages written in plain ASCII"),
                _ => ("ascii", false, "ASCII-only view off"),
            };
            o.ui = Some(Ui::LocalView { view: view.to_string(), on });
            o.say(format!("{label}. Only you see this."));
        }
        "purge" => {
            let who = clean_login(c.arg(0));
            if who.is_empty() {
                ext_usage(o, "purge", "/purge <user> [reason]");
                return Ok(true);
            }
            if !need_channel(c, o) || !need_staff(c, o) {
                return Ok(true);
            }
            let user = match user_or_why(b, &who, format!("There is no Twitch user named {who}")).await {
                Ok(user) => user,
                Err(why) => {
                    o.err(why);
                    return Ok(true);
                }
            };
            let reason = c.from(1).trim().to_string();
            b.invoke("ban_user", json!({ "broadcasterId": id, "targetUserId": user.id, "duration": 1, "reason": opt(&reason) }))
                .await?;
            o.ok(format!("Purged {}'s messages", user.display_name));
        }
        "delete" => {
            if c.arg(0).is_empty() {
                ext_usage(o, "delete", "/delete <message id | @user>");
                return Ok(true);
            }
            if !need_channel(c, o) || !need_staff(c, o) {
                return Ok(true);
            }
            let done = b.invoke("slash_delete_message", json!({ "broadcasterId": id, "channel": login, "target": c.arg(0) })).await?;
            o.ok(done.as_str().unwrap_or("").to_string());
        }
        "automod" => {
            let (verb, message_id) = match c.typed {
                "automod_approve" => ("allow".to_string(), c.args.first().copied()),
                "automod_deny" => ("deny".to_string(), c.args.first().copied()),
                _ => (c.arg(0).to_lowercase(), c.args.get(1).copied()),
            };
            let allow = matches!(verb.as_str(), "allow" | "approve" | "yes");
            let deny = matches!(verb.as_str(), "deny" | "reject" | "no");
            if !allow && !deny {
                ext_usage(o, "automod", "/automod <allow|deny> [message id]");
                return Ok(true);
            }
            if !need_channel(c, o) || !need_staff(c, o) {
                return Ok(true);
            }
            o.say_all(
                lines("slash_automod", json!({ "channel": login, "messageId": message_id.map(Value::from).unwrap_or(Value::Null), "allow": allow }))
                    .await?,
            );
        }
        "lowtrust" => {
            if !need_channel(c, o) || !need_staff(c, o) {
                return Ok(true);
            }
            o.say_all(lines("slash_low_trust", json!({ "channel": login })).await?);
        }
        "modlogs" => {
            if !need_channel(c, o) || !need_staff(c, o) {
                return Ok(true);
            }
            let on = !b.settings()["show_mod_logs"].as_bool().unwrap_or(false);
            b.invoke("patch_settings", json!({ "patch": { "show_mod_logs": on } })).await?;
            o.ok(if on { "Mod log shown" } else { "Mod log hidden" });
        }
        "requests" => {
            if !need_channel(c, o) || !need_staff(c, o) {
                return Ok(true);
            }
            b.invoke("open_url", json!({ "url": format!("https://www.twitch.tv/popout/{login}/reward-queue") })).await?;
        }
        "goal" => {
            if !need_channel(c, o) || !need_broadcaster(c, o) {
                return Ok(true);
            }
            b.invoke("open_url", json!({ "url": format!("https://dashboard.twitch.tv/u/{login}/community/goals") })).await?;
            o.say("Opened your Creator Goals page. Twitch only lets goals be made and edited there.");
        }
        "sharedchat" => {
            if !need_channel(c, o) {
                return Ok(true);
            }
            o.say_all(lines("slash_shared_chat", json!({ "broadcasterId": id, "channel": login })).await?);
            if is_broadcaster_of(c.ctx) {
                b.invoke("open_url", json!({ "url": format!("https://dashboard.twitch.tv/u/{login}/stream-manager") })).await?;
                o.say("Opened your Stream Manager: Shared Chat starts from Stream Together there.");
            }
        }
        "raidbrowser" => o.say_all(lines("slash_raid_candidates", json!({})).await?),
        "raidsend" => {
            if !need_channel(c, o) || !need_broadcaster(c, o) {
                return Ok(true);
            }
            b.invoke("slash_raid_now", json!({ "broadcasterId": id })).await?;
            o.ok("Raid sent");
        }
        "massunban" => {
            if !need_channel(c, o) || !need_broadcaster(c, o) {
                return Ok(true);
            }
            if c.arg(0).eq_ignore_ascii_case("stop") {
                let stopped = b.invoke("slash_mass_unban_stop", json!({ "broadcasterId": id })).await?;
                o.say(if stopped.as_bool().unwrap_or(false) { "Stopping the mass unban..." } else { "No mass unban is running." });
                return Ok(true);
            }
            if needs_confirmation(
                c.ctx,
                o,
                &format!("massunban:{id}"),
                "This lifts EVERY permanent ban in your channel and cannot be undone.",
                c.depth,
                c.typed,
            ) {
                return Ok(true);
            }
            b.invoke("slash_mass_unban_start", json!({ "broadcasterId": id, "channel": login })).await?;
            o.say("Mass unban started, about three a second. /massunban stop ends it.");
        }
        "leadmod" | "unleadmod" => {
            let who = clean_login(c.arg(0));
            if who.is_empty() {
                ext_usage(o, c.command, "/leadmod <user>");
                return Ok(true);
            }
            if !need_channel(c, o) || !need_broadcaster(c, o) {
                return Ok(true);
            }
            o.say_all(
                lines("slash_lead_mod", json!({ "broadcasterId": id, "target": who, "assign": c.command == "leadmod" })).await?,
            );
        }
        "editor" | "uneditor" => {
            let who = clean_login(c.arg(0));
            if who.is_empty() {
                ext_usage(o, c.command, "/editor <user>");
                return Ok(true);
            }
            if !need_channel(c, o) || !need_broadcaster(c, o) {
                return Ok(true);
            }
            let add = c.command == "editor";
            if add
                && needs_confirmation(
                    c.ctx,
                    o,
                    &format!("editor:{who}"),
                    &format!("Editors can change your stream title, category, ads and more. Make {who} an editor?"),
                    c.depth,
                    c.typed,
                )
            {
                return Ok(true);
            }
            o.say_all(lines("slash_editor", json!({ "broadcasterId": id, "target": who, "add": add })).await?);
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn timer(c: &Call<'_>, o: &mut Outcome) {
    let key = if c.login().is_empty() { "_".to_string() } else { c.login().to_string() };
    if c.arg(0).eq_ignore_ascii_case("stop") {
        let stopped = timers().remove(&key).unwrap_or_default();
        let n = stopped.len();
        for (_, task) in stopped {
            task.abort();
        }
        o.say(if n > 0 { format!("Canceled {n} timer{}.", if n == 1 { "" } else { "s" }) } else { "No timers are waiting here.".to_string() });
        return;
    }
    let ms = parse_delay(c.arg(0));
    let line = c.from(1).trim().to_string();
    let Some(ms) = ms.filter(|_| !line.is_empty()) else {
        ext_usage(o, "timer", "/timer <delay> <command or message>");
        return;
    };
    if ms > MAX_TIMER_MS {
        o.err("A timer can wait up to 24 hours");
        return;
    }
    if c.depth >= MAX_SCRIPT_DEPTH {
        o.err("Timers can only nest three deep");
        return;
    }
    if timers().get(&key).map(HashMap::len).unwrap_or(0) >= MAX_TIMERS {
        o.err(format!("At most {MAX_TIMERS} timers can wait at once"));
        return;
    }
    let timer_id = NEXT_TIMER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let ctx = c.ctx.clone();
    let depth = c.depth + 1;
    let run_line = line.clone();
    let room = key.clone();
    let task = tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
        if let Some(set) = timers().get_mut(&room) {
            set.remove(&timer_id);
        }
        // Nobody is waiting on a timer's answer: toasts become lines in the room.
        let s = run_script_line(ctx.clone(), run_line, depth).await;
        for t in s.toasts {
            ctx.b.post_notice(&ctx.channel, t.text).await;
        }
    });
    timers().entry(key).or_default().insert(timer_id, task);
    o.say(format!("Runs in {}: {line}  (/timer stop cancels)", c.arg(0)));
}

async fn setting(c: &Call<'_>, o: &mut Outcome) -> Result<(), String> {
    let b = c.b();
    let verb = c.arg(0).to_lowercase();
    let path = c.arg(1);
    let keys: Vec<&str> = path.split('.').filter(|k| !k.is_empty()).collect();
    if verb.is_empty() || keys.is_empty() || !["get", "set", "reset"].contains(&verb.as_str()) {
        ext_usage(o, "setting", "/setting <get|set|reset> <name> [value]");
        return Ok(());
    }
    let settings = b.settings();
    let current = get_path(&settings, &keys).cloned();
    let defaults = b.invoke("get_default_settings", json!({})).await.unwrap_or(Value::Null);
    let fallback = get_path(&defaults, &keys).cloned();
    let name = keys.join(".");
    if current.is_none() && fallback.is_none() {
        o.err(format!("There is no setting called {name}"));
        return Ok(());
    }
    if verb == "get" {
        let shown = js_stringify(current.as_ref().unwrap_or(&Value::Null));
        let shown = if shown.chars().count() > 300 { format!("{}...", shown.chars().take(300).collect::<String>()) } else { shown };
        o.say(format!("{name} = {shown}"));
        return Ok(());
    }
    let next = if verb == "reset" {
        match fallback {
            Some(f) => f,
            None => {
                o.err(format!("{name} has no default to go back to"));
                return Ok(());
            }
        }
    } else {
        let value = c.from(2);
        if c.args.len() < 3 {
            ext_usage(o, "setting", "/setting set <name> <value>");
            return Ok(());
        }
        let next = parse_setting_value(&value);
        let want = kind_of(current.as_ref().or(fallback.as_ref()));
        if want != "empty" && kind_of(Some(&next)) != want {
            o.err(format!("{name} takes a {want}, not a {}", kind_of(Some(&next))));
            return Ok(());
        }
        next
    };
    let top = keys[0];
    let updated = with_path(&settings, &keys, next.clone());
    let mut patch = serde_json::Map::new();
    patch.insert(top.to_string(), updated[top].clone());
    b.invoke("patch_settings", json!({ "patch": Value::Object(patch) })).await?;
    o.say(format!("{name} = {}", js_stringify(&next)));
    Ok(())
}

async fn favorite(c: &Call<'_>, o: &mut Outcome, add: bool) -> Result<(), String> {
    let b = c.b();
    let who = Some(clean_login(c.arg(0))).filter(|s| !s.is_empty()).unwrap_or_else(|| c.login().to_string());
    if who.is_empty() {
        o.err(format!("/{}: name a channel or open one", c.typed));
        return Ok(());
    }
    let user = match user_or_why(b, &who, format!("There is no Twitch channel named {who}")).await {
        Ok(user) => user,
        Err(why) => {
            o.err(why);
            return Ok(());
        }
    };
    let settings = b.settings();
    let mut favorites: Vec<Value> = settings["favorite_streamers"].as_array().cloned().unwrap_or_default();
    let mut identities: Vec<Value> = settings["favorite_channels"].as_array().cloned().unwrap_or_default();
    let is_fav = favorites.iter().any(|f| f.as_str() == Some(user.id.as_str()));
    if is_fav == add {
        o.info(if add {
            format!("{} is already a favorite", user.display_name)
        } else {
            format!("{} is not a favorite", user.display_name)
        });
        return Ok(());
    }
    identities.retain(|f| f["id"].as_str() != Some(user.id.as_str()));
    if add {
        favorites.push(json!(user.id));
        identities.push(json!({
            "id": user.id,
            "provider": "twitch",
            "channel": user.login,
            "display_name": user.display_name,
            "avatar": user.avatar,
            "added_at": chrono::DateTime::from_timestamp_millis(b.now_ms()).unwrap_or_default().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        }));
    } else {
        favorites.retain(|f| f.as_str() != Some(user.id.as_str()));
    }
    b.invoke("patch_settings", json!({ "patch": { "favorite_streamers": favorites, "favorite_channels": identities } })).await?;
    if add {
        let _ = b.invoke("refresh_favorites", json!({ "key": user.id })).await;
    }
    o.ok(if add { format!("{} added to favorites", user.display_name) } else { format!("{} removed from favorites", user.display_name) });
    Ok(())
}

// ─────────────────────────────── /7tv ───────────────────────────────

const SEVENTV_EXPIRED: &str = "Your 7TV session has expired. Reconnect your 7TV account.";
const SEVENTV_IDENTITY: &str =
    "id mainConnection { platform platformId platformUsername platformDisplayName platformAvatarUrl } style { activeEmoteSetId }";

async fn seventv_gql(b: &dyn Backend, query: String, variables: Value, op: &str) -> Result<Value, String> {
    let res = b
        .invoke("seventv_graphql_authed", json!({ "query": query, "variables": variables, "operationName": op, "accountId": null }))
        .await
        .map_err(|msg| if msg.contains("SESSION_EXPIRED") || msg.contains("Not authenticated") { SEVENTV_EXPIRED.to_string() } else { msg })?;
    if let Some(first) = res["errors"].as_array().and_then(|e| e.first()) {
        if first["extensions"]["code"].as_str() == Some("LOGIN_REQUIRED") {
            return Err(SEVENTV_EXPIRED.to_string());
        }
        return Err(first["message"].as_str().filter(|m| !m.is_empty()).unwrap_or("A 7TV request failed.").to_string());
    }
    Ok(res["data"].clone())
}

/// A channel the signed-in 7TV account can act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SevenTvChannel {
    pub username: String,
    pub display_name: String,
    pub twitch_id: Option<String>,
    pub manage_emotes: bool,
    pub active_set: Option<String>,
}

fn seventv_channel(u: &Value, manage_emotes: bool) -> SevenTvChannel {
    let conn = &u["mainConnection"];
    let id = u["id"].as_str().unwrap_or_default();
    let username = conn["platformUsername"].as_str().filter(|s| !s.is_empty()).unwrap_or(id).to_string();
    let display_name = conn["platformDisplayName"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| conn["platformUsername"].as_str().filter(|s| !s.is_empty()))
        .unwrap_or(id)
        .to_string();
    SevenTvChannel {
        username,
        display_name,
        twitch_id: (conn["platform"].as_str() == Some("TWITCH")).then(|| conn["platformId"].as_str().unwrap_or_default().to_string()),
        manage_emotes,
        active_set: u["style"]["activeEmoteSetId"].as_str().filter(|s| !s.is_empty()).map(str::to_string),
    }
}

/// The signed-in 7TV user's own channel, then every channel they edit.
pub fn seventv_editable(data: &Value) -> Vec<SevenTvChannel> {
    let me = &data["users"]["me"];
    if me.is_null() {
        return Vec::new();
    }
    let mut out = vec![seventv_channel(me, true)];
    for ed in me["editorFor"].as_array().into_iter().flatten() {
        if ed["user"].is_null() {
            continue;
        }
        let p = &ed["permissions"];
        let manage = p["emoteSet"]["manage"].as_bool().unwrap_or(false)
            || p["emoteSet"]["admin"].as_bool().unwrap_or(false)
            || p["superAdmin"].as_bool().unwrap_or(false);
        out.push(seventv_channel(&ed["user"], manage));
    }
    out
}

/// (id, default name) of each directory emote in a search answer.
fn seventv_search_rows(data: &Value) -> Vec<(String, String)> {
    data["emotes"]["search"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| (e["id"].as_str().unwrap_or_default().to_string(), e["defaultName"].as_str().unwrap_or_default().to_string()))
        .collect()
}

async fn seventv_search(b: &dyn Backend, query: &str, per_page: u32) -> Result<Vec<(String, String)>, String> {
    let gql = "query SearchEmotes($query: String, $sort: Sort!, $filters: Filters, $page: Int, $perPage: Int) { emotes { search(query: $query, sort: $sort, filters: $filters, page: $page, perPage: $perPage) { items { id defaultName flags { animated defaultZeroWidth nsfw } owner { mainConnection { platformDisplayName platformUsername } } } totalCount pageCount } } }";
    let sort_by = if query.is_empty() { "TRENDING_WEEKLY" } else { "TOP_ALL_TIME" };
    let vars = json!({ "query": opt(query), "sort": { "sortBy": sort_by, "order": "DESCENDING" }, "filters": null, "page": 1, "perPage": per_page });
    Ok(seventv_search_rows(&seventv_gql(b, gql.to_string(), vars, "SearchEmotes").await?))
}

/// An emote id out of a raw id or a 7tv.app emote link.
pub fn seventv_emote_id(input: &str) -> Option<String> {
    let t = input.trim();
    if let Some(pos) = t.find("emotes/") {
        let id: String = t[pos + 7..].chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        if !id.is_empty() {
            return Some(id);
        }
    }
    (t.len() >= 20 && t.chars().all(|c| c.is_ascii_alphanumeric())).then(|| t.to_string())
}

fn emote_ref(emote_id: &str, alias: Option<&str>) -> Value {
    match alias {
        Some(a) => json!({ "emoteId": emote_id, "alias": a }),
        None => json!({ "emoteId": emote_id }),
    }
}

async fn seventv(c: &Call<'_>, o: &mut Outcome) -> Result<(), String> {
    let b = c.b();
    let verb = c.arg(0).to_lowercase();
    if verb == "search" {
        let q = c.from(1).trim().to_string();
        if q.is_empty() {
            ext_usage(o, "7tv", "/7tv search <name>");
            return Ok(());
        }
        let rows = seventv_search(b, &q, 8).await?;
        if rows.is_empty() {
            o.say(format!("No 7TV emotes match \"{q}\"."));
        } else {
            let list: Vec<String> = rows.iter().map(|(id, name)| format!("{name} ({id})")).collect();
            o.say(format!("7TV emotes for \"{q}\": {}. Add one with /7tv add <id>.", list.join(", ")));
        }
        return Ok(());
    }
    if !["add", "remove", "rename"].contains(&verb.as_str()) {
        ext_usage(o, "7tv", "/7tv <search|add|remove|rename> ...");
        return Ok(());
    }
    let login = c.login().to_string();
    if login.is_empty() {
        o.err("Open a channel first");
        return Ok(());
    }
    let editable = format!(
        "query EditableChannels {{ users {{ me {{ {SEVENTV_IDENTITY} editorFor {{ state permissions {{ emoteSet {{ admin manage }} user {{ manageEditors }} }} user {{ {SEVENTV_IDENTITY} }} }} }} }} }}"
    );
    let channels = seventv_editable(&seventv_gql(b, editable, Value::Null, "EditableChannels").await?);
    let target = channels
        .into_iter()
        .find(|ch| ch.username.to_lowercase() == login || (!c.id().is_empty() && ch.twitch_id.as_deref() == Some(c.id())));
    let Some(target) = target.filter(|t| t.manage_emotes) else {
        o.err(format!("You can't edit {login}'s 7TV emotes. Sign in to 7TV with an editor account in Settings"));
        return Ok(());
    };
    let Some(set_id) = target.active_set.clone() else {
        o.err(format!("{login} has no active 7TV emote set"));
        return Ok(());
    };
    if verb == "add" {
        let input = c.arg(1);
        let alias = c.args.get(2).copied();
        if input.is_empty() {
            ext_usage(o, "7tv", "/7tv add <emote id, link or name> [alias]");
            return Ok(());
        }
        let mut emote: Option<(String, String)> = None;
        if let Some(eid) = seventv_emote_id(input) {
            let gql = "query ResolveEmote($id: Id!) { emotes { emote(id: $id) { id defaultName flags { animated defaultZeroWidth nsfw } owner { mainConnection { platformDisplayName platformUsername } } } } }";
            let data = seventv_gql(b, gql.to_string(), json!({ "id": eid }), "ResolveEmote").await?;
            let e = &data["emotes"]["emote"];
            if let Some(found) = e["id"].as_str() {
                emote = Some((found.to_string(), e["defaultName"].as_str().unwrap_or_default().to_string()));
            }
        }
        if emote.is_none() {
            let rows = seventv_search(b, input, 20).await?;
            emote = rows
                .iter()
                .find(|(_, name)| name == input)
                .or_else(|| rows.iter().find(|(_, name)| name.to_lowercase() == input.to_lowercase()))
                .cloned();
        }
        let Some((emote_id, default_name)) = emote else {
            o.err(format!("No 7TV emote called \"{input}\". Try /7tv search {input}"));
            return Ok(());
        };
        let gql = "mutation AddEmote($setId: Id!, $emote: EmoteSetEmoteId!, $zeroWidth: Boolean, $override: Boolean) { emoteSets { emoteSet(id: $setId) { addEmote(id: $emote, zeroWidth: $zeroWidth, overrideConflicts: $override) { id } } } }";
        seventv_gql(b, gql.to_string(), json!({ "setId": set_id, "emote": emote_ref(&emote_id, alias), "zeroWidth": null, "override": null }), "AddEmote")
            .await?;
        o.say(format!("Added {} to {}'s 7TV emotes.", alias.unwrap_or(&default_name), target.display_name));
        return Ok(());
    }
    let name = c.arg(1);
    if name.is_empty() {
        ext_usage(o, "7tv", "/7tv <emote name>");
        return Ok(());
    }
    let gql = "query SetEmotes($id: Id!, $page: Int!, $perPage: Int!, $query: String) { emoteSets { emoteSet(id: $id) { id name capacity emotes(query: $query, page: $page, perPage: $perPage) { items { alias addedAt addedById flags { zeroWidth } emote { id defaultName flags { animated } } } totalCount pageCount } } } }";
    let data = seventv_gql(b, gql.to_string(), json!({ "id": set_id, "page": 1, "perPage": 50, "query": name }), "SetEmotes").await?;
    let items: Vec<(String, String)> = data["emoteSets"]["emoteSet"]["emotes"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|it| (it["emote"]["id"].as_str().unwrap_or_default().to_string(), it["alias"].as_str().unwrap_or_default().to_string()))
        .collect();
    let in_set = items
        .iter()
        .find(|(_, alias)| alias == name)
        .or_else(|| items.iter().find(|(_, alias)| alias.to_lowercase() == name.to_lowercase()))
        .cloned();
    let Some((emote_id, alias)) = in_set else {
        o.err(format!("{name} is not in {}'s 7TV emotes", target.display_name));
        return Ok(());
    };
    if verb == "remove" {
        let gql = "mutation RemoveEmote($setId: Id!, $emote: EmoteSetEmoteId!) { emoteSets { emoteSet(id: $setId) { removeEmote(id: $emote) { id } } } }";
        seventv_gql(b, gql.to_string(), json!({ "setId": set_id, "emote": emote_ref(&emote_id, Some(&alias)) }), "RemoveEmote").await?;
        o.say(format!("Removed {alias} from {}'s 7TV emotes.", target.display_name));
        return Ok(());
    }
    let next = c.arg(2);
    if next.is_empty() {
        ext_usage(o, "7tv", "/7tv rename <emote name> <new name>");
        return Ok(());
    }
    let gql = "mutation RenameEmote($setId: Id!, $emote: EmoteSetEmoteId!, $alias: String!) { emoteSets { emoteSet(id: $setId) { updateEmoteAlias(id: $emote, alias: $alias) { alias } } } }";
    seventv_gql(b, gql.to_string(), json!({ "setId": set_id, "emote": emote_ref(&emote_id, Some(&alias)), "alias": next }), "RenameEmote")
        .await?;
    o.say(format!("Renamed {alias} to {next}."));
    Ok(())
}

#[cfg(test)]
mod tests;
