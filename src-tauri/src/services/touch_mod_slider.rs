//! The mod slider on a phone: swipe right on a message to pick Delete, a
//! timeout or Ban, and let go to apply it through `moderate_chatter`.
//!
//! It is the desktop slider's gesture with a finger's sizes. Rust lays out
//! the ruler once per press, for the room right of where the thumb landed
//! (where the cancel zone ends, where each step starts, where the pull stops),
//! and says every word the chip shows with the chatter's name in it, so the
//! iPhone and Android draw and decide the same thing. The phones only follow
//! the finger and look up the step under it with `step_at`'s rule.
//!
//! The rules, as on desktop:
//! - a pull short of the dead zone is the cancel zone ("Let go to cancel");
//! - steps run mildest first: Delete, the timeouts shortest first, Ban;
//! - Ban sits one step-length past the last timeout, its zone has an end
//!   (the pull stops there), and it arms only after the finger rests in it
//!   for `ban_dwell_ms`; a release there before it arms cancels, so a flick
//!   past the end neither bans nor lands on the longest timeout;
//! - a step changes only `hysteresis` past its edge;
//! - a finger more than `leash` above or below the message cancels.

use serde::Serialize;

use crate::services::moderation::{slider_steps_for, ModStep, StepAction};

/// Pull this far before the first step; less is the cancel zone, as wide as
/// desktop's so a reply-length swipe never deletes. A narrow room takes a
/// quarter of its width instead.
const DEAD_ZONE_PT: f32 = 72.0;
/// Room per step by the Length setting, wider than desktop's for a finger.
const STEP_PT_SHORT: f32 = 28.0;
const STEP_PT_MEDIUM: f32 = 34.0;
const STEP_PT_LONG: f32 = 42.0;
/// A row too narrow for the steps squeezes them, never below this.
const STEP_FLOOR_PT: f32 = 20.0;
/// Ban sits this many step-lengths past the last timeout.
const BAN_GAP_STEPS: usize = 1;
/// Ban's zone is at least this long.
const BAN_ZONE_MIN_PT: f32 = 40.0;
/// Ban arms after the finger rests this long in its zone.
const BAN_DWELL_MS: u32 = 350;
/// Moving this far inside Ban's zone starts the rest over.
const BAN_STILL_PT: f32 = 10.0;
/// A step changes only this far past its edge.
const HYSTERESIS_PT: f32 = 4.0;
/// A finger this far above or below the message cancels a release.
const LEASH_PT: f32 = 120.0;
/// The horizontal pull that claims the touch from the list's scroll.
const ENGAGE_PT: f32 = 10.0;
/// Room kept free at the row's right edge.
const RIGHT_MARGIN_PT: f32 = 24.0;

/// One step as the chip and the track show it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TouchStep {
    pub action: StepAction,
    pub seconds: Option<u32>,
    /// "Delete", "10m", "Ban".
    pub label: String,
    /// What the chip says while it is armed: "Delete message", "Time out 10m", "Ban".
    pub chip: String,
    /// The step's color between the theme's warning (0) and error (1); Delete
    /// is a calm gray (`None`).
    pub tone: Option<f32>,
}

/// The slider for one press.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TouchSlider {
    /// Swipe to moderate is on.
    pub enabled: bool,
    /// "preview", "rail" or "capsule".
    pub style: String,
    pub steps: Vec<TouchStep>,
    /// Where the cancel zone ends, as a pull in points.
    pub dead: f32,
    /// Room per step.
    pub spacing: f32,
    /// Where each step's zone starts, as a pull in points.
    pub starts: Vec<f32>,
    /// The pull stops here.
    pub end: f32,
    pub ban_dwell_ms: u32,
    pub ban_still: f32,
    pub hysteresis: f32,
    pub leash: f32,
    pub engage: f32,
    /// "Let go to cancel".
    pub cancel_words: String,
    /// "Hold for Ban, let go to cancel" while Ban's rest runs (a release then cancels).
    pub hold_ban_words: String,
    /// The one-time hint: "Swipe right on a message to moderate it".
    pub nudge_words: String,
    /// Where the thumb landed, from the row's left edge: the ruler starts here.
    pub start: f32,
    /// The long-press fan's one-tap timeout: the first timeout step.
    pub quick_seconds: u32,
}

fn step_room(length: &str) -> f32 {
    match length {
        "short" => STEP_PT_SHORT,
        "long" => STEP_PT_LONG,
        _ => STEP_PT_MEDIUM,
    }
}

/// Step-lengths the steps span: one per step, plus the gap before Ban.
fn step_units(steps: &[ModStep]) -> f32 {
    let ends_in_ban = steps.last().is_some_and(|s| s.action == StepAction::Ban);
    steps.len() as f32 + if ends_in_ban { BAN_GAP_STEPS as f32 + 0.5 } else { 0.0 }
}

/// The ruler for `steps` with `room` points right of the thumb: (dead,
/// spacing, starts, end), each a pull from where the thumb landed.
pub fn layout(steps: &[ModStep], room: f32, length: &str) -> (f32, f32, Vec<f32>, f32) {
    let dead = DEAD_ZONE_PT.min((room * 0.25).round()).max(1.0);
    let units = step_units(steps).max(1.0);
    let fit = (room - dead - RIGHT_MARGIN_PT) / units;
    let spacing = step_room(length).min(fit).max(STEP_FLOOR_PT);
    let last = steps.len().saturating_sub(1);
    let starts: Vec<f32> = steps
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let at = if s.action == StepAction::Ban && i == last { i + BAN_GAP_STEPS } else { i };
            dead + at as f32 * spacing
        })
        .collect();
    let last_zone = if steps.last().is_some_and(|s| s.action == StepAction::Ban) {
        (1.5 * spacing).max(BAN_ZONE_MIN_PT)
    } else {
        spacing
    };
    let end = starts.last().map_or(dead, |s| s + last_zone);
    (dead, spacing, starts, end)
}

/// The step under a pull of `slide` points: `None` in the cancel zone. The
/// gap before Ban keeps showing the step before it.
pub fn step_at(steps: &[ModStep], dead: f32, spacing: f32, slide: f32) -> Option<usize> {
    if slide < dead || steps.is_empty() {
        return None;
    }
    let unit = ((slide - dead) / spacing).floor() as usize;
    let last = steps.len() - 1;
    if steps[last].action != StepAction::Ban {
        return Some(unit.min(last));
    }
    if unit >= last + BAN_GAP_STEPS {
        return Some(last);
    }
    if last == 0 {
        return None;
    }
    Some(unit.min(last - 1))
}

/// The step to show, keeping `current` until the pull is `hysteresis` past
/// its edge.
pub fn with_hysteresis(starts: &[f32], current: Option<usize>, next: Option<usize>, slide: f32, hysteresis: f32) -> Option<usize> {
    match (current, next) {
        (Some(cur), Some(nx)) if nx > cur => (slide >= starts[nx] + hysteresis).then_some(nx).or(Some(cur)),
        (Some(cur), Some(nx)) if nx < cur => (slide <= starts[cur] - hysteresis).then_some(nx).or(Some(cur)),
        _ => next,
    }
}

fn touch_steps(steps: &[ModStep], name: &str) -> Vec<TouchStep> {
    let timeouts = steps.iter().filter(|s| s.action == StepAction::Timeout).count();
    let mut nth = 0usize;
    steps
        .iter()
        .map(|s| {
            let (chip, tone) = match s.action {
                StepAction::Delete if name.is_empty() => ("Delete message".to_string(), None),
                StepAction::Delete => (format!("Delete {name}\u{2019}s message"), None),
                StepAction::Ban if name.is_empty() => ("Ban".to_string(), Some(1.0)),
                StepAction::Ban => (format!("Ban {name}"), Some(1.0)),
                StepAction::Timeout => {
                    let tone = if timeouts > 1 { nth as f32 / (timeouts - 1) as f32 * 0.65 } else { 0.0 };
                    nth += 1;
                    let chip = if name.is_empty() {
                        format!("Time out {}", s.label)
                    } else {
                        format!("Time out {name} {}", s.label)
                    };
                    (chip, Some(tone))
                }
            };
            TouchStep { action: s.action, seconds: s.seconds, label: s.label.clone(), chip, tone }
        })
        .collect()
}

/// The slider for a press `start` points from the left of a row `row_width`
/// points wide, on `name`'s message. Pure, for tests.
pub fn build(steps: &[ModStep], row_width: f32, start: f32, name: &str, length: &str, style: &str, enabled: bool) -> TouchSlider {
    let start = start.clamp(0.0, (row_width - 1.0).max(0.0));
    let (dead, spacing, starts, end) = layout(steps, row_width - start, length);
    let quick_seconds = steps.iter().find_map(|s| (s.action == StepAction::Timeout).then_some(s.seconds).flatten()).unwrap_or(600);
    TouchSlider {
        enabled: enabled && !steps.is_empty(),
        style: match style {
            "rail" | "capsule" => style.to_string(),
            _ => "preview".to_string(),
        },
        steps: touch_steps(steps, name.trim()),
        dead,
        spacing,
        starts,
        end,
        ban_dwell_ms: BAN_DWELL_MS,
        ban_still: BAN_STILL_PT,
        hysteresis: HYSTERESIS_PT,
        leash: LEASH_PT,
        engage: ENGAGE_PT,
        cancel_words: "Let go to cancel".into(),
        hold_ban_words: "Hold for Ban, let go to cancel".into(),
        nudge_words: "Swipe right on a message to moderate it".into(),
        start,
        quick_seconds,
    }
}

/// The slider for a press `start_x` points from the left of a message
/// `row_width` points wide, from the viewer's saved steps, Length and Style;
/// `target_name` names the chatter in the chip.
#[tauri::command]
pub fn get_touch_mod_slider(
    provider: Option<String>,
    row_width: f64,
    start_x: Option<f64>,
    target_name: Option<String>,
) -> TouchSlider {
    let steps = slider_steps_for(provider.as_deref(), None);
    let (length, style, enabled) = crate::services::providers::setting(|s| {
        (s.chat_design.mod_slider_length.clone(), s.chat_design.mod_slider_style.clone(), s.chat_design.mod_slider)
    })
    .unwrap_or_else(|| ("medium".into(), "preview".into(), true));
    build(
        &steps,
        row_width as f32,
        start_x.unwrap_or(0.0) as f32,
        target_name.as_deref().unwrap_or(""),
        &length,
        &style,
        enabled,
    )
}

/// More timeouts than this stop fitting the slider (desktop's limit too).
const MAX_TIMEOUTS: usize = 8;

/// The slider's timeouts after adding or removing `seconds`: sorted, no
/// repeats, at most eight, 1 second to 14 days. `None` when the change is
/// refused (out of range, the list full, or the last one removed). Pure, for tests.
pub fn edited_timeouts(current: &[u32], op: &str, seconds: u32) -> Option<Vec<u32>> {
    let mut list: Vec<u32> = current.to_vec();
    match op {
        "add" => {
            if !(1..=1_209_600).contains(&seconds) || list.len() >= MAX_TIMEOUTS {
                return None;
            }
            if !list.contains(&seconds) {
                list.push(seconds);
            }
        }
        "remove" => {
            list.retain(|s| *s != seconds);
            if list.is_empty() {
                return None;
            }
        }
        _ => return None,
    }
    list.sort_unstable();
    list.dedup();
    Some(list)
}

/// Add or remove one of the slider's timeouts (`chat_design.mod_slider_timeouts`).
/// Answers the list as stored.
#[tauri::command]
pub async fn edit_mod_slider_timeouts(
    app: crate::rt::AppHandle,
    op: String,
    seconds: u32,
    state: tauri::State<'_, crate::models::settings::AppState>,
) -> Result<Vec<u32>, String> {
    let mut stored = Vec::new();
    crate::commands::settings::commit_settings_patch(&app, &state, None, |current| {
        let Some(list) = edited_timeouts(&current.chat_design.mod_slider_timeouts, &op, seconds) else {
            return Err(match op.as_str() {
                "remove" => "Keep at least one timeout length.".to_string(),
                _ => "Up to eight lengths, each 1 second to 14 days.".to_string(),
            });
        };
        stored = list.clone();
        let mut changes = serde_json::Map::new();
        changes.insert("chat_design.mod_slider_timeouts".into(), serde_json::json!(list));
        crate::commands::settings_fields::patch_for_changes(current, &changes)
    })?;
    Ok(stored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::moderation::{slider_steps, Platform, SliderConfig};

    #[test]
    fn timeouts_edit_one_at_a_time() {
        assert_eq!(edited_timeouts(&[600, 60], "add", 2_700), Some(vec![60, 600, 2_700]));
        assert_eq!(edited_timeouts(&[60, 600], "add", 600), Some(vec![60, 600]));
        assert_eq!(edited_timeouts(&[60], "add", 0), None);
        assert_eq!(edited_timeouts(&[1, 2, 3, 4, 5, 6, 7, 8], "add", 9), None);
        assert_eq!(edited_timeouts(&[60, 600], "remove", 60), Some(vec![600]));
        assert_eq!(edited_timeouts(&[60], "remove", 60), None, "the last one stays");
    }

    fn steps() -> Vec<ModStep> {
        slider_steps(Platform::Twitch, &SliderConfig { delete: true, timeouts: vec![60, 600, 3_600], ban: true })
    }

    #[test]
    fn the_ruler_ends_in_bans_zone_one_step_past_the_last_timeout() {
        let s = build(&steps(), 406.0, 16.0, "", "medium", "preview", true);
        // The room right of the thumb is 390.
        assert_eq!(s.start, 16.0);
        assert_eq!(s.dead, 72.0);
        assert_eq!(s.spacing, 34.0);
        // Delete, 1m, 10m, 1h, then a gap, then Ban.
        assert_eq!(s.starts, vec![72.0, 106.0, 140.0, 174.0, 242.0]);
        assert_eq!(s.end, 242.0 + 51.0);
        assert_eq!(s.quick_seconds, 60);
        assert_eq!(s.steps[2].chip, "Time out 10m");
        assert_eq!(s.steps[0].tone, None);
        assert_eq!(s.steps[4].tone, Some(1.0));
        assert_eq!(s.steps[3].tone, Some(0.65));
        assert_eq!(s.hold_ban_words, "Hold for Ban, let go to cancel");
    }

    #[test]
    fn the_chip_names_the_chatter() {
        let s = build(&steps(), 390.0, 20.0, " Gus ", "medium", "preview", true);
        assert_eq!(s.steps[0].chip, "Delete Gus\u{2019}s message");
        assert_eq!(s.steps[2].chip, "Time out Gus 10m");
        assert_eq!(s.steps[4].chip, "Ban Gus");
    }

    #[test]
    fn a_thumb_landing_mid_row_still_reaches_ban() {
        // The ruler fits the room right of the thumb, so every step stays reachable.
        let s = build(&steps(), 393.0, 200.0, "", "medium", "preview", true);
        assert!(s.end <= 193.0, "Ban zone ends inside the room: {}", s.end);
        assert!(s.spacing >= STEP_FLOOR_PT);
    }

    #[test]
    fn a_narrow_row_squeezes_the_steps_but_never_below_the_floor() {
        let s = build(&steps(), 200.0, 0.0, "", "long", "rail", true);
        assert_eq!(s.dead, 50.0);
        assert!(s.spacing >= STEP_FLOOR_PT);
        assert_eq!(s.style, "rail");
        let odd = build(&steps(), 390.0, 0.0, "", "medium", "sideways", true);
        assert_eq!(odd.style, "preview");
    }

    #[test]
    fn the_cancel_zone_the_gap_before_ban_and_ban() {
        let st = steps();
        let (dead, spacing, _, _) = layout(&st, 390.0, "medium");
        assert_eq!(step_at(&st, dead, spacing, 10.0), None);
        assert_eq!(step_at(&st, dead, spacing, 76.0), Some(0));
        assert_eq!(step_at(&st, dead, spacing, 176.0), Some(3));
        // The gap before Ban still shows 1h.
        assert_eq!(step_at(&st, dead, spacing, 216.0), Some(3));
        assert_eq!(step_at(&st, dead, spacing, 246.0), Some(4));
        let ban_only = vec![st[4].clone()];
        assert_eq!(step_at(&ban_only, dead, spacing, 76.0), None, "Ban alone needs its gap");
    }

    #[test]
    fn hysteresis_holds_a_step_near_its_edge() {
        let starts = vec![56.0, 90.0, 124.0];
        assert_eq!(with_hysteresis(&starts, Some(0), Some(1), 91.0, 4.0), Some(0));
        assert_eq!(with_hysteresis(&starts, Some(0), Some(1), 95.0, 4.0), Some(1));
        assert_eq!(with_hysteresis(&starts, Some(1), Some(0), 88.0, 4.0), Some(1));
        assert_eq!(with_hysteresis(&starts, Some(1), Some(0), 85.0, 4.0), Some(0));
        assert_eq!(with_hysteresis(&starts, Some(1), None, 20.0, 4.0), None);
    }

    #[test]
    fn no_steps_means_no_slider() {
        assert!(!build(&[], 390.0, 0.0, "", "medium", "preview", true).enabled);
        assert!(!build(&steps(), 390.0, 0.0, "", "medium", "preview", false).enabled);
    }
}
