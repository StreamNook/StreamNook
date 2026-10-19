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
//!
//! The thumb can land anywhere on the row, so the room right of it varies.
//! The cancel zone and the step size stay the same from press to press, so the
//! hand learns them; what gives way when the room is short is, in order: the
//! longest timeouts (down to the first), then the step size, then the cancel
//! zone (never below `DEAD_FLOOR_PT`), then Delete. Ban always stays last and
//! reachable. A press with no room even for that gets no steps, only "Start
//! further left", so nothing can land past the edge of the screen.

use serde::Serialize;

use crate::services::moderation::{slider_steps_for, ModStep, StepAction};

/// Pull this far before the first step; less is the cancel zone, as wide as
/// desktop's so a reply-length swipe never deletes.
const DEAD_ZONE_PT: f32 = 72.0;
/// A short room narrows the cancel zone last, and never below this.
const DEAD_FLOOR_PT: f32 = 48.0;
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
/// Room kept free at the row's right edge, where a thumb cannot reach.
const RIGHT_MARGIN_PT: f32 = 16.0;

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

fn ends_in_ban(steps: &[ModStep]) -> bool {
    steps.last().is_some_and(|s| s.action == StepAction::Ban)
}

/// The ruler for `steps` with this cancel zone and step size: (starts, end),
/// each a pull from where the thumb landed.
pub fn layout(steps: &[ModStep], dead: f32, spacing: f32) -> (Vec<f32>, f32) {
    let last = steps.len().saturating_sub(1);
    let ban = ends_in_ban(steps);
    let starts: Vec<f32> = steps
        .iter()
        .enumerate()
        .map(|(i, _)| {
            let at = if ban && i == last { i + BAN_GAP_STEPS } else { i };
            dead + at as f32 * spacing
        })
        .collect();
    let last_zone = if ban { (1.5 * spacing).max(BAN_ZONE_MIN_PT) } else { spacing };
    let end = starts.last().map_or(dead, |s| s + last_zone);
    (starts, end)
}

/// The steps that fit `room` points right of the thumb, with the cancel zone
/// and step size they get (see the module notes for what gives way first).
/// `None` when not even the first timeout and Ban fit.
pub fn fit(steps: &[ModStep], room: f32, length: &str) -> Option<(Vec<ModStep>, f32, f32)> {
    let fits = |kept: &[ModStep], dead: f32, spacing: f32| layout(kept, dead, spacing).1 + RIGHT_MARGIN_PT <= room;
    let mut kept = steps.to_vec();
    let (mut dead, mut spacing) = (DEAD_ZONE_PT, step_room(length));
    // The longest timeouts first, keeping the first.
    while !fits(&kept, dead, spacing) && kept.iter().filter(|s| s.action == StepAction::Timeout).count() > 1 {
        let longest = kept.iter().rposition(|s| s.action == StepAction::Timeout).expect("counted above");
        kept.remove(longest);
    }
    loop {
        while !fits(&kept, dead, spacing) && spacing > STEP_FLOOR_PT {
            spacing = (spacing - 1.0).max(STEP_FLOOR_PT);
        }
        while !fits(&kept, dead, spacing) && dead > DEAD_FLOOR_PT {
            dead = (dead - 1.0).max(DEAD_FLOOR_PT);
        }
        if fits(&kept, dead, spacing) {
            return (!kept.is_empty()).then_some((kept, dead, spacing));
        }
        // Delete goes last of all, while a timeout or Ban is left.
        match kept.iter().position(|s| s.action == StepAction::Delete) {
            Some(i) if kept.len() > 1 => {
                kept.remove(i);
            }
            _ => return None,
        }
    }
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
    let room = row_width - start;
    let quick_seconds = steps.iter().find_map(|s| (s.action == StepAction::Timeout).then_some(s.seconds).flatten()).unwrap_or(600);
    // Too near the right edge for any step: the pull runs with no steps and
    // says where to start instead, so a release there does nothing.
    let (kept, dead, spacing, cancel_words) = match fit(steps, room, length) {
        Some((kept, dead, spacing)) => (kept, dead, spacing, "Let go to cancel"),
        None => (Vec::new(), DEAD_FLOOR_PT, STEP_FLOOR_PT, "Start further left to moderate"),
    };
    let (starts, end) = if kept.is_empty() {
        (Vec::new(), (room - RIGHT_MARGIN_PT).max(1.0))
    } else {
        layout(&kept, dead, spacing)
    };
    TouchSlider {
        enabled: enabled && !steps.is_empty(),
        style: match style {
            "rail" | "capsule" => style.to_string(),
            _ => "preview".to_string(),
        },
        steps: touch_steps(&kept, name.trim()),
        dead,
        spacing,
        starts,
        end,
        ban_dwell_ms: BAN_DWELL_MS,
        ban_still: BAN_STILL_PT,
        hysteresis: HYSTERESIS_PT,
        leash: LEASH_PT,
        engage: ENGAGE_PT,
        cancel_words: cancel_words.into(),
        hold_ban_words: "Hold for Ban, let go to cancel".into(),
        nudge_words: "Swipe right on a message to moderate it".into(),
        start,
        quick_seconds,
    }
}

/// The slider for a press `start_x` points from the left of a message
/// `row_width` points wide, from the viewer's saved steps, Length and Style;
/// `target_name` names the chatter in the chip. With the message's sender
/// (`target_id`, `target_badges`) and the room (`channel`, its login) the
/// steps are only those Twitch allows on it (`moderation::message_reach`):
/// Delete alone on the viewer's own message, none on the broadcaster's.
#[tauri::command]
pub async fn get_touch_mod_slider(
    provider: Option<String>,
    row_width: f64,
    start_x: Option<f64>,
    target_name: Option<String>,
    target_id: Option<String>,
    target_badges: Option<Vec<String>>,
    channel: Option<String>,
) -> TouchSlider {
    let mut steps = slider_steps_for(provider.as_deref(), None);
    let mut reachable = true;
    if let (Some(target), Some(channel)) = (target_id.filter(|t| !t.is_empty()), channel.filter(|c| !c.is_empty())) {
        use crate::services::moderation::{message_reach, steps_within, MessageReach};
        let role = crate::services::chat_send_limiter::budget(&channel).role;
        let viewer = crate::services::chatter_card::own_user_id().await;
        let reach = message_reach(role, &viewer, &target, &target_badges.unwrap_or_default());
        steps = steps_within(steps, reach);
        reachable = reach != MessageReach::Nothing;
    }
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
        enabled && reachable,
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

    fn default_steps() -> Vec<ModStep> {
        slider_steps(Platform::Twitch, &SliderConfig { delete: true, timeouts: crate::services::moderation::DEFAULT_SLIDER_TIMEOUTS.to_vec(), ban: true })
    }

    #[test]
    fn wherever_the_thumb_lands_ban_stays_reachable_and_the_ruler_keeps_its_sizes() {
        for width in [393.0, 402.0] {
            for start in [16.0, 120.0, 200.0] {
                let s = build(&default_steps(), width, start, "", "medium", "preview", true);
                let room = width - start;
                let ban = s.steps.len() - 1;
                assert_eq!(s.steps[ban].label, "Ban", "Ban stays last ({width}, {start})");
                assert_eq!(s.steps[0].label, "Delete", "Delete stays ({width}, {start})");
                assert_eq!(s.steps[1].seconds, Some(60), "the first timeout stays ({width}, {start})");
                assert!(s.starts[ban] + BAN_ZONE_MIN_PT <= room, "Ban fits ({width}, {start}): {:?}", s.starts);
                assert!(s.end + RIGHT_MARGIN_PT <= room, "the pull stops in the room ({width}, {start})");
                assert_eq!(s.dead, 72.0, "the cancel zone keeps its size ({width}, {start})");
            }
        }
        // Landing near the left, the step size is the Length setting's, every time.
        let a = build(&default_steps(), 393.0, 16.0, "", "medium", "preview", true);
        let b = build(&default_steps(), 393.0, 60.0, "", "medium", "preview", true);
        assert_eq!((a.spacing, b.spacing), (34.0, 34.0));
        // The longest timeouts give way first.
        let labels: Vec<&str> = a.steps.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels.first(), Some(&"Delete"));
        assert!(!labels.contains(&"1w"), "1w is the first to go: {labels:?}");
    }

    #[test]
    fn too_near_the_right_edge_the_pull_has_no_steps_and_says_where_to_start() {
        let s = build(&default_steps(), 393.0, 300.0, "", "medium", "preview", true);
        assert!(s.steps.is_empty());
        assert!(s.enabled, "the pull still shows its chip");
        assert_eq!(s.cancel_words, "Start further left to moderate");
        assert!(s.end <= 93.0);
        assert_eq!(step_at(&[], s.dead, s.spacing, 80.0), None);
    }

    #[test]
    fn a_narrow_row_squeezes_the_steps_but_never_below_the_floor() {
        let s = build(&steps(), 200.0, 0.0, "", "long", "rail", true);
        assert_eq!(s.dead, 72.0);
        assert_eq!(s.steps.len(), 3, "Delete, 1m and Ban");
        assert!(s.spacing >= STEP_FLOOR_PT && s.spacing < STEP_PT_LONG);
        assert!(s.end + RIGHT_MARGIN_PT <= 200.0);
        assert_eq!(s.style, "rail");
        let odd = build(&steps(), 390.0, 0.0, "", "medium", "sideways", true);
        assert_eq!(odd.style, "preview");
    }

    #[test]
    fn the_cancel_zone_the_gap_before_ban_and_ban() {
        let st = steps();
        let (dead, spacing) = (72.0, 34.0);
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
