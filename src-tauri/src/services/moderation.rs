//! One door for chat moderation on every platform.
//!
//! The page used to pick the platform API itself in seven places (the hover
//! dock, the drag buckets, the user card, slash commands, /nuke, keyboard
//! moderation and the phone's mod sheet), each with its own copy of Kick's
//! round-to-minutes rule and its own toast wording. Now a surface names the
//! chatter and the action, and this module decides which API answers, what
//! the platform will really apply, and how to say what happened.
//!
//! It also owns the timeout steps a surface offers (the hover dock's list and
//! the row slider), because what a step means depends on the platform: Kick
//! only takes whole minutes up to a week, YouTube has a single fixed timeout,
//! Twitch stops at 14 days.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::services::twitch_service::TwitchService;

/// Twitch's longest timeout; anything longer has to be a ban.
pub const TWITCH_MAX_TIMEOUT_SECS: u32 = 1_209_600;
/// Kick takes timeouts in whole minutes, 1 to 10080 (one week).
pub const KICK_MAX_TIMEOUT_MINUTES: u32 = 10_080;
/// YouTube offers one timeout length, whatever was asked for.
pub const YOUTUBE_TIMEOUT_SECS: u32 = 300;

/// The hover dock's timeouts when the viewer has not set their own.
pub const DEFAULT_DOCK_TIMEOUTS: [u32; 4] = [1, 600, 3_600, 86_400];
/// The slider's timeouts by default (between its Delete and Ban steps).
pub const DEFAULT_SLIDER_TIMEOUTS: [u32; 7] = [60, 300, 600, 1_800, 3_600, 86_400, 604_800];
/// The line buttons' timeouts by default (between their Delete and Ban). Kept
/// short: the buttons sit on every line, so each one costs line width.
pub const DEFAULT_LINE_TIMEOUTS: [u32; 1] = [600];
/// More timeouts than this stop fitting the dock and the slider.
const MAX_TIMEOUTS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Twitch,
    Kick,
    YouTube,
}

impl Platform {
    /// The page's provider id. Anything unknown is Twitch, the app's home
    /// platform, which is also what every caller assumed before.
    pub fn from_provider(provider: Option<&str>) -> Self {
        match provider.map(str::to_ascii_lowercase).as_deref() {
            Some("kick") => Platform::Kick,
            Some("youtube") => Platform::YouTube,
            _ => Platform::Twitch,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ModAction {
    Timeout {
        seconds: u32,
    },
    Ban,
    /// Lifts a ban or a timeout (one API on every platform). `timeout` only
    /// changes the words: "Removed the timeout on X" rather than "Unbanned X".
    Unban {
        #[serde(default)]
        timeout: bool,
    },
    Delete {
        #[serde(rename = "messageId")]
        message_id: String,
    },
    /// A Twitch warning the chatter must acknowledge before they can chat
    /// again. The request's `reason` is what they see, so one is required.
    Warn,
}

/// Who to act on, where, and how. Ids are the platform's own: a Twitch user
/// id, a numeric Kick id, a YouTube channel id.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModRequest {
    #[serde(default)]
    pub provider: Option<String>,
    /// The channel owner's id (Twitch and Kick).
    #[serde(default)]
    pub broadcaster_id: Option<String>,
    /// The channel slug (YouTube). A `youtube:` prefix is accepted.
    #[serde(default)]
    pub channel: Option<String>,
    /// The chatter. Not needed to delete a message.
    #[serde(default)]
    pub target_user_id: Option<String>,
    /// The chatter's display name, for the words only.
    #[serde(default)]
    pub target_name: Option<String>,
    pub action: ModAction,
    #[serde(default)]
    pub reason: Option<String>,
    /// With no reason given, use the first saved ban reason.
    #[serde(default)]
    pub use_saved_reason: bool,
    /// The words after the name in a typed `/timeout` ("10m spam", "spam").
    /// With a Timeout action they set its length and reason (`typed_timeout`).
    #[serde(default)]
    pub typed: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModOutcome {
    /// The timeout the platform really applied, in seconds.
    pub applied_seconds: Option<u32>,
    /// What happened, ready for a toast.
    pub message: String,
    /// A ban or timeout, which an Unban reverses.
    pub undoable: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum StepAction {
    Delete,
    Timeout,
    Ban,
}

/// One step of the dock's timeout list or the slider, mildest first.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModStep {
    pub action: StepAction,
    /// The timeout as the platform applies it; `None` for Delete and Ban.
    pub seconds: Option<u32>,
    pub label: String,
}

impl ModStep {
    fn timeout(seconds: u32) -> Self {
        ModStep { action: StepAction::Timeout, seconds: Some(seconds), label: duration_label(seconds) }
    }
    fn delete() -> Self {
        ModStep { action: StepAction::Delete, seconds: None, label: "Delete".to_string() }
    }
    fn ban() -> Self {
        ModStep { action: StepAction::Ban, seconds: None, label: "Ban".to_string() }
    }
}

/// Which steps the slider offers. The order is fixed (Delete, the timeouts
/// shortest first, Ban) so further right always means harsher.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SliderConfig {
    pub delete: bool,
    pub timeouts: Vec<u32>,
    pub ban: bool,
}

/// The timeout `platform` will really apply when asked for `seconds`.
pub fn applied_timeout(platform: Platform, seconds: u32) -> u32 {
    match platform {
        Platform::Twitch => seconds.clamp(1, TWITCH_MAX_TIMEOUT_SECS),
        Platform::Kick => kick_minutes(seconds) * 60,
        Platform::YouTube => YOUTUBE_TIMEOUT_SECS,
    }
}

/// Kick minutes for a timeout of `seconds`: rounded UP (a 70 s timeout must not
/// quietly become 60 s), at least one minute (zero would lift it), at most a week.
pub fn kick_minutes(seconds: u32) -> u32 {
    seconds.div_ceil(60).clamp(1, KICK_MAX_TIMEOUT_MINUTES)
}

/// A short label in the largest unit that fits: one part when it divides
/// evenly ("10m", "1d", "2w"), otherwise the two largest parts ("1m 30s",
/// "1h 30m").
pub fn duration_label(seconds: u32) -> String {
    const UNITS: [(u32, &str); 5] = [(604_800, "w"), (86_400, "d"), (3_600, "h"), (60, "m"), (1, "s")];
    if let Some((size, unit)) = UNITS.iter().find(|(size, _)| seconds >= *size) {
        if seconds % size == 0 {
            return format!("{}{}", seconds / size, unit);
        }
    }
    let mut parts = Vec::new();
    let mut rest = seconds;
    for (size, unit) in UNITS {
        if rest >= size {
            parts.push(format!("{}{}", rest / size, unit));
            rest %= size;
        }
        if parts.len() == 2 {
            break;
        }
    }
    if parts.is_empty() {
        "0s".to_string()
    } else {
        parts.join(" ")
    }
}

/// The timeouts `platform` applies for `list`, shortest first, with the ones
/// it would merge (Kick's sub-minute lengths, YouTube's single length) folded
/// together. Out-of-range entries are dropped.
fn applied_timeouts(platform: Platform, list: &[u32]) -> Vec<u32> {
    let mut applied: Vec<u32> = list
        .iter()
        .copied()
        .filter(|s| (1..=TWITCH_MAX_TIMEOUT_SECS).contains(s))
        .take(MAX_TIMEOUTS)
        .map(|s| applied_timeout(platform, s))
        .collect();
    applied.sort_unstable();
    applied.dedup();
    applied
}

/// The hover dock's timeouts on `platform`, then Ban. An empty or unusable
/// list falls back to the defaults.
pub fn timeout_steps(platform: Platform, presets: &[u32]) -> Vec<ModStep> {
    let mut timeouts = applied_timeouts(platform, presets);
    if timeouts.is_empty() {
        timeouts = applied_timeouts(platform, &DEFAULT_DOCK_TIMEOUTS);
    }
    let mut steps: Vec<ModStep> = timeouts.into_iter().map(ModStep::timeout).collect();
    steps.push(ModStep::ban());
    steps
}

/// The slider's steps on `platform`: Delete, the timeouts it applies, Ban,
/// each only when turned on. Empty when everything is off.
pub fn slider_steps(platform: Platform, config: &SliderConfig) -> Vec<ModStep> {
    let mut steps = Vec::new();
    if config.delete {
        steps.push(ModStep::delete());
    }
    steps.extend(applied_timeouts(platform, &config.timeouts).into_iter().map(ModStep::timeout));
    if config.ban {
        steps.push(ModStep::ban());
    }
    steps
}

/// The slider as the viewer set it up (typed `chat_design` fields).
fn saved_slider_config() -> SliderConfig {
    crate::services::providers::setting(|s| SliderConfig {
        delete: s.chat_design.mod_slider_delete,
        timeouts: s.chat_design.mod_slider_timeouts.clone(),
        ban: s.chat_design.mod_slider_ban,
    })
    .unwrap_or_else(|| SliderConfig {
        delete: true,
        timeouts: DEFAULT_SLIDER_TIMEOUTS.to_vec(),
        ban: true,
    })
}

/// The slider's steps for `provider`, from `config` when given (the page's
/// copy, which can be a moment ahead of the saved one mid-edit), else saved.
pub fn slider_steps_for(provider: Option<&str>, config: Option<SliderConfig>) -> Vec<ModStep> {
    let config = config.unwrap_or_else(saved_slider_config);
    slider_steps(Platform::from_provider(provider), &config)
}

/// The viewer's timeout list (`moderation.timeout_presets`, seconds). The
/// moderation group is untyped on this side, so it is read off the catch-all.
fn saved_presets() -> Vec<u32> {
    crate::services::providers::setting(|s| {
        s.extra
            .get("moderation")
            .and_then(|m| m.get("timeout_presets"))
            .and_then(|v| v.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|n| n.as_f64())
                    .filter(|n| n.is_finite() && *n >= 1.0)
                    .map(|n| n.min(u32::MAX as f64) as u32)
                    .collect()
            })
            .unwrap_or_default()
    })
    .unwrap_or_default()
}

fn saved_reason() -> Option<String> {
    crate::services::providers::setting(|s| {
        s.extra
            .get("moderation")
            .and_then(|m| m.get("saved_ban_reasons"))
            .and_then(|v| v.as_array())
            .and_then(|list| list.first())
            .and_then(|r| r.as_str())
            .map(|r| r.trim().to_string())
            .filter(|r| !r.is_empty())
    })
    .flatten()
}

/// The steps for `provider`, from `presets` when given, else the saved list.
pub fn steps_for(provider: Option<&str>, presets: Option<Vec<u32>>) -> Vec<ModStep> {
    let list = presets.unwrap_or_else(saved_presets);
    timeout_steps(Platform::from_provider(provider), &list)
}

fn name_of(req: &ModRequest) -> String {
    req.target_name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "that chatter".to_string())
}

/// What a successful action did, in words.
pub fn outcome_for(platform: Platform, action: &ModAction, name: &str) -> ModOutcome {
    match action {
        ModAction::Timeout { seconds } => {
            let applied = applied_timeout(platform, *seconds);
            ModOutcome {
                applied_seconds: Some(applied),
                message: format!("Timed out {} for {}", name, duration_label(applied)),
                undoable: true,
            }
        }
        // YouTube calls its ban "hide user from channel".
        ModAction::Ban => ModOutcome {
            applied_seconds: None,
            message: if platform == Platform::YouTube {
                format!("Hid {} from chat", name)
            } else {
                format!("Banned {}", name)
            },
            undoable: true,
        },
        ModAction::Unban { timeout } => ModOutcome {
            applied_seconds: None,
            message: if *timeout {
                format!("Removed the timeout on {}", name)
            } else if platform == Platform::YouTube {
                format!("{} can chat again", name)
            } else {
                format!("Unbanned {}", name)
            },
            undoable: false,
        },
        ModAction::Delete { .. } => ModOutcome {
            applied_seconds: None,
            message: if name == "that chatter" {
                "Deleted the message".to_string()
            } else {
                format!("Deleted a message from {}", name)
            },
            undoable: false,
        },
        ModAction::Warn => ModOutcome {
            applied_seconds: None,
            message: format!("Warned {}", name),
            undoable: false,
        },
    }
}

fn need<'a>(value: &'a Option<String>, what: &str) -> Result<&'a str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow!("Missing {} for this moderation action", what))
}

fn kick_id(value: &Option<String>, what: &str) -> Result<u64> {
    need(value, what)?
        .parse::<u64>()
        .map_err(|_| anyhow!("That Kick {} is not a number", what))
}

fn youtube_slug(req: &ModRequest) -> Result<String> {
    let raw = need(&req.channel, "channel")?;
    Ok(raw.strip_prefix("youtube:").unwrap_or(raw).to_string())
}

/// The words after the name in a typed `/timeout` as the seconds and the
/// reason, read the same on every app: a leading length (plain seconds, or
/// 30s, 10m, 1h30m, 2d, 1w) sets the time; without one it is Twitch's ten
/// minutes and every word is the reason.
pub fn typed_timeout(words: &str) -> (u32, Option<String>) {
    let words = words.trim();
    let (first, rest) = words.split_once(char::is_whitespace).unwrap_or((words, ""));
    let (seconds, reason) = match crate::services::slash_dispatch::parse_delay(first) {
        Some(ms) => ((ms / 1000).max(1), rest.trim()),
        None => (600, words),
    };
    (u32::try_from(seconds).unwrap_or(u32::MAX), Some(reason.to_string()).filter(|r| !r.is_empty()))
}

/// Folds a typed `/timeout` tail into the request's length and reason. A
/// reason the page gave on its own still wins.
fn settle_typed(req: &mut ModRequest) {
    if let (ModAction::Timeout { seconds }, Some(words)) = (&mut req.action, req.typed.take()) {
        let (typed_seconds, typed_reason) = typed_timeout(&words);
        *seconds = typed_seconds;
        req.reason = req.reason.take().or(typed_reason);
    }
}

/// Run one moderation action on the right platform. Errors carry the
/// platform's own reason ("Twitch refused the ban: ...").
pub async fn moderate(mut req: ModRequest) -> Result<ModOutcome> {
    settle_typed(&mut req);
    let platform = Platform::from_provider(req.provider.as_deref());
    let reason = req
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_string)
        .or_else(|| if req.use_saved_reason { saved_reason() } else { None });

    match (&req.action, platform) {
        (ModAction::Timeout { seconds }, Platform::Twitch) => {
            TwitchService::ban_user(
                need(&req.broadcaster_id, "channel")?,
                need(&req.target_user_id, "chatter")?,
                Some(applied_timeout(platform, *seconds)),
                reason.as_deref(),
            )
            .await?
        }
        (ModAction::Ban, Platform::Twitch) => {
            TwitchService::ban_user(
                need(&req.broadcaster_id, "channel")?,
                need(&req.target_user_id, "chatter")?,
                None,
                reason.as_deref(),
            )
            .await?
        }
        (ModAction::Unban { .. }, Platform::Twitch) => {
            TwitchService::unban_user(
                need(&req.broadcaster_id, "channel")?,
                need(&req.target_user_id, "chatter")?,
            )
            .await?
        }
        (ModAction::Delete { message_id }, Platform::Twitch) => {
            TwitchService::delete_chat_message(need(&req.broadcaster_id, "channel")?, message_id).await?
        }
        (ModAction::Timeout { seconds }, Platform::Kick) => {
            crate::services::providers::kick::ban_user(
                kick_id(&req.broadcaster_id, "channel")?,
                kick_id(&req.target_user_id, "chatter")?,
                Some(kick_minutes(*seconds)),
                reason,
            )
            .await?
        }
        (ModAction::Ban, Platform::Kick) => {
            crate::services::providers::kick::ban_user(
                kick_id(&req.broadcaster_id, "channel")?,
                kick_id(&req.target_user_id, "chatter")?,
                None,
                reason,
            )
            .await?
        }
        (ModAction::Unban { .. }, Platform::Kick) => {
            crate::services::providers::kick::unban_user(
                kick_id(&req.broadcaster_id, "channel")?,
                kick_id(&req.target_user_id, "chatter")?,
            )
            .await?
        }
        (ModAction::Delete { message_id }, Platform::Kick) => {
            crate::services::providers::kick::delete_message(message_id).await?
        }
        (ModAction::Timeout { .. }, Platform::YouTube) => {
            crate::services::providers::youtube::ban_user(
                &youtube_slug(&req)?,
                need(&req.target_user_id, "chatter")?,
                Some(YOUTUBE_TIMEOUT_SECS),
            )
            .await?
        }
        (ModAction::Ban, Platform::YouTube) => {
            crate::services::providers::youtube::ban_user(
                &youtube_slug(&req)?,
                need(&req.target_user_id, "chatter")?,
                None,
            )
            .await?
        }
        (ModAction::Unban { .. }, Platform::YouTube) => {
            crate::services::providers::youtube::unban_user(
                &youtube_slug(&req)?,
                need(&req.target_user_id, "chatter")?,
            )
            .await?
        }
        (ModAction::Delete { message_id }, Platform::YouTube) => {
            crate::services::providers::youtube::delete_message(&youtube_slug(&req)?, message_id).await?
        }
        (ModAction::Warn, Platform::Twitch) => {
            let reason = reason.as_deref().ok_or_else(|| anyhow!("A warning needs a reason they will see"))?;
            TwitchService::warn_chat_user(
                need(&req.broadcaster_id, "channel")?,
                need(&req.target_user_id, "chatter")?,
                reason,
            )
            .await?
        }
        (ModAction::Warn, _) => return Err(anyhow!("Warnings are a Twitch feature")),
    }

    Ok(outcome_for(platform, &req.action, &name_of(&req)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_warning_is_worded_and_cannot_be_undone() {
        let action: ModAction = serde_json::from_value(serde_json::json!({ "kind": "warn" })).unwrap();
        let out = outcome_for(Platform::Twitch, &action, "gus");
        assert_eq!(out.message, "Warned gus");
        assert!(!out.undoable);
    }

    #[tokio::test]
    async fn a_warning_without_a_reason_or_off_twitch_is_refused_before_any_call() {
        let req = |provider: &str, reason: Option<&str>| ModRequest {
            provider: Some(provider.into()),
            broadcaster_id: Some("1".into()),
            channel: None,
            target_user_id: Some("2".into()),
            target_name: None,
            action: ModAction::Warn,
            reason: reason.map(str::to_string),
            use_saved_reason: false,
            typed: None,
        };
        assert!(moderate(req("twitch", Some("  "))).await.unwrap_err().to_string().contains("needs a reason"));
        assert!(moderate(req("kick", Some("spam"))).await.unwrap_err().to_string().contains("Twitch feature"));
    }

    fn labels(steps: &[ModStep]) -> Vec<&str> {
        steps.iter().map(|s| s.label.as_str()).collect()
    }

    fn default_slider() -> SliderConfig {
        SliderConfig { delete: true, timeouts: DEFAULT_SLIDER_TIMEOUTS.to_vec(), ban: true }
    }

    #[test]
    fn dock_steps_follow_the_list_and_end_in_a_ban() {
        let steps = timeout_steps(Platform::Twitch, &[]);
        assert_eq!(labels(&steps), ["1s", "10m", "1h", "1d", "Ban"]);
        assert_eq!(steps.last().unwrap().action, StepAction::Ban);
        // Sorted, deduped, unusable entries dropped.
        let custom = timeout_steps(Platform::Twitch, &[3_600, 0, 30, 3_600, 2_000_000]);
        assert_eq!(labels(&custom), ["30s", "1h", "Ban"]);
    }

    #[test]
    fn slider_runs_mildest_to_harshest() {
        let steps = slider_steps(Platform::Twitch, &default_slider());
        assert_eq!(labels(&steps), ["Delete", "1m", "5m", "10m", "30m", "1h", "1d", "1w", "Ban"]);
        assert_eq!(steps[0].action, StepAction::Delete);
        assert_eq!(steps[0].seconds, None);
        // Whatever order the list was saved in, timeouts sit shortest first
        // between Delete and Ban.
        let shuffled = SliderConfig { delete: false, timeouts: vec![86_400, 60], ban: true };
        assert_eq!(labels(&slider_steps(Platform::Twitch, &shuffled)), ["1m", "1d", "Ban"]);
        let none = SliderConfig { delete: false, timeouts: vec![], ban: false };
        assert!(slider_steps(Platform::Twitch, &none).is_empty());
    }

    #[test]
    fn kick_merges_sub_minute_steps_and_caps_at_a_week() {
        let steps = timeout_steps(Platform::Kick, &[1, 30, 60, 90, 1_209_600]);
        assert_eq!(labels(&steps), ["1m", "2m", "1w", "Ban"]);
        assert_eq!(kick_minutes(1), 1);
        assert_eq!(kick_minutes(61), 2);
        assert_eq!(kick_minutes(10_000_000), KICK_MAX_TIMEOUT_MINUTES);
        assert_eq!(applied_timeout(Platform::Kick, 70), 120);
    }

    #[test]
    fn line_buttons_default_to_delete_ten_minutes_ban() {
        // The line buttons ask for their steps through the slider's list builder.
        let line = SliderConfig { delete: true, timeouts: DEFAULT_LINE_TIMEOUTS.to_vec(), ban: true };
        assert_eq!(labels(&slider_steps(Platform::Twitch, &line)), ["Delete", "10m", "Ban"]);
        assert_eq!(labels(&slider_steps(Platform::Kick, &line)), ["Delete", "10m", "Ban"]);
        assert_eq!(labels(&slider_steps(Platform::YouTube, &line)), ["Delete", "5m", "Ban"]);
    }

    #[test]
    fn youtube_has_one_timeout() {
        let steps = slider_steps(Platform::YouTube, &default_slider());
        assert_eq!(labels(&steps), ["Delete", "5m", "Ban"]);
    }

    #[test]
    fn steps_serialize_for_the_page() {
        let v = serde_json::to_value(ModStep::timeout(600)).unwrap();
        assert_eq!(v, serde_json::json!({ "action": "timeout", "seconds": 600, "label": "10m" }));
        let cfg: SliderConfig =
            serde_json::from_value(serde_json::json!({ "delete": true, "timeouts": [60], "ban": false })).unwrap();
        assert_eq!(cfg, SliderConfig { delete: true, timeouts: vec![60], ban: false });
    }

    #[test]
    fn labels_read_like_people_write_them() {
        assert_eq!(duration_label(1), "1s");
        assert_eq!(duration_label(90), "1m 30s");
        assert_eq!(duration_label(5_400), "1h 30m");
        assert_eq!(duration_label(86_400), "1d");
        assert_eq!(duration_label(1_209_600), "2w");
    }

    #[test]
    fn outcome_words_report_what_the_platform_applied() {
        let o = outcome_for(Platform::Kick, &ModAction::Timeout { seconds: 30 }, "Ann");
        assert_eq!(o.applied_seconds, Some(60));
        assert_eq!(o.message, "Timed out Ann for 1m");
        assert!(o.undoable);
        let yt = outcome_for(Platform::YouTube, &ModAction::Timeout { seconds: 86_400 }, "Bo");
        assert_eq!(yt.message, "Timed out Bo for 5m");
        let lift = outcome_for(Platform::Twitch, &ModAction::Unban { timeout: true }, "Cy");
        assert_eq!(lift.message, "Removed the timeout on Cy");
        assert!(!lift.undoable);
    }

    #[test]
    fn requests_read_the_page_shape() {
        let req: ModRequest = serde_json::from_value(serde_json::json!({
            "provider": "kick",
            "broadcasterId": "12",
            "targetUserId": "34",
            "targetName": "Dee",
            "action": { "kind": "timeout", "seconds": 600 },
            "useSavedReason": true
        }))
        .unwrap();
        assert_eq!(Platform::from_provider(req.provider.as_deref()), Platform::Kick);
        assert!(matches!(req.action, ModAction::Timeout { seconds: 600 }));
        assert!(req.use_saved_reason);
        let del: ModRequest = serde_json::from_value(serde_json::json!({
            "action": { "kind": "delete", "messageId": "m1" }
        }))
        .unwrap();
        assert!(matches!(del.action, ModAction::Delete { ref message_id } if message_id == "m1"));
        let unban: ModRequest =
            serde_json::from_value(serde_json::json!({ "action": { "kind": "unban" } })).unwrap();
        assert!(matches!(unban.action, ModAction::Unban { timeout: false }));
    }

    #[test]
    fn a_typed_timeout_reads_the_same_on_every_app() {
        assert_eq!(typed_timeout("5m spam links"), (300, Some("spam links".to_string())));
        assert_eq!(typed_timeout("1h30m"), (5400, None));
        assert_eq!(typed_timeout("45 caps"), (45, Some("caps".to_string())));
        assert_eq!(typed_timeout("spam links"), (600, Some("spam links".to_string())));
        assert_eq!(typed_timeout("10x"), (600, Some("10x".to_string())));
        assert_eq!(typed_timeout(""), (600, None));
    }

    #[test]
    fn a_typed_tail_sets_the_requests_length_and_reason() {
        let mut req: ModRequest = serde_json::from_value(serde_json::json!({
            "provider": "kick",
            "action": { "kind": "timeout", "seconds": 600 },
            "typed": "2h being rude"
        }))
        .unwrap();
        settle_typed(&mut req);
        assert!(matches!(req.action, ModAction::Timeout { seconds: 7200 }));
        assert_eq!(req.reason.as_deref(), Some("being rude"));
        assert!(req.typed.is_none());
    }
}
