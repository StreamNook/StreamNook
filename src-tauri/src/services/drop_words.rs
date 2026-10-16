//! Short phrases for a drop that needs more than one sitting of watch time, a
//! subscription, or a random draw, worked out from Twitch's numbers
//! (services/reward_drops.rs reads them into the drop model). Every phone
//! surface prints these: the Rewards campaign sheet and the in-player bar.

use crate::models::drops::{DropProgress, TimeBasedDrop};

/// A container that gives one reward from a pool is opened, not claimed.
pub fn is_draw(drop: &TimeBasedDrop, progress: Option<&DropProgress>) -> bool {
    drop.random_of.unwrap_or(0) > 1
        || progress
            .and_then(|p| p.twitch_progress.as_ref())
            .and_then(|t| t.earned.as_ref())
            .and_then(|e| e.distribution_type.as_deref())
            == Some("POOL")
}

/// "Open" for a draw, "Claim" for anything else.
pub fn claim_label(drop: &TimeBasedDrop) -> &'static str {
    if is_draw(drop, drop.progress.as_ref()) {
        "Open"
    } else {
        "Claim"
    }
}

fn minutes_label(minutes: f64) -> String {
    if minutes >= 60.0 && minutes % 60.0 == 0.0 {
        format!("{} hr", (minutes / 60.0) as i64)
    } else {
        format!("{} min", minutes.round() as i64)
    }
}

/// "Subscribe or gift a sub", "2 subs or gift subs", "20 min a day · 3 days".
/// None for a plain watch-time drop, which its minutes already describe.
pub fn requirement_text(drop: &TimeBasedDrop) -> Option<String> {
    let subs = drop.required_subs;
    if subs > 0 {
        return Some(if subs == 1 {
            "Subscribe or gift a sub".to_string()
        } else {
            format!("{subs} subs or gift subs")
        });
    }
    let days = drop.required_days;
    if days > 1 && drop.required_minutes_watched > 0 {
        let per_day = drop.required_minutes_watched as f64 / days as f64;
        return Some(format!("{} a day · {days} days", minutes_label(per_day)));
    }
    None
}

/// Watch time still needed: "36m to watch", "8h 36m to watch", "2h to watch".
pub fn to_watch(minutes: i32) -> String {
    let m = minutes.max(0);
    if m < 60 {
        return format!("{m}m to watch");
    }
    match m % 60 {
        0 => format!("{}h to watch", m / 60),
        rest => format!("{}h {rest}m to watch", m / 60),
    }
}

/// "in 23h", "in 40m", "soon"; empty with no time.
pub fn resets_in(at_ms: Option<i64>, now_ms: i64) -> String {
    let Some(at) = at_ms else { return String::new() };
    let left = at - now_ms;
    if left <= 60_000 {
        return "soon".to_string();
    }
    let hours = left / 3_600_000;
    if hours >= 1 {
        format!("in {hours}h")
    } else {
        format!("in {}m", (left + 59_999) / 60_000)
    }
}

/// Twitch's progress on a multi-day or subscription drop as one line:
/// "You got Bulbasaur", "Ready to open", "1 of 2 subs",
/// "Day 2 of 3 · 5/20 min today · resets in 23h". None when Twitch reported
/// nothing beyond minutes.
pub fn progress_line(drop: &TimeBasedDrop, now_ms: i64) -> Option<String> {
    let progress = drop.progress.as_ref()?;
    let t = progress.twitch_progress.as_ref()?;
    if progress.is_claimed {
        return Some(match &t.earned {
            Some(e) => format!("You got {}", e.name),
            None => "Claimed".to_string(),
        });
    }
    if t.ready_to_claim {
        if is_draw(drop, Some(progress)) {
            return Some("Ready to open".to_string());
        }
        return Some(match &t.earned {
            Some(e) => format!("{} ready to claim", e.name),
            None => "Ready to claim".to_string(),
        });
    }
    let subs = drop.required_subs;
    if subs > 0 {
        let unit = if subs == 1 { "sub" } else { "subs" };
        return Some(format!("{} of {subs} {unit}", t.subs_done.min(subs)));
    }
    let days = drop.required_days;
    if days > 1 && drop.required_minutes_watched > 0 {
        let per_day = (drop.required_minutes_watched as f64 / days as f64).round() as u32;
        // Twitch counts today in days_done as soon as today's minutes are in
        // ("20/20m (Day 2)" at days_done 2); the next day opens when this
        // window closes.
        let today_done = t.minutes_today >= per_day;
        let day = if today_done { t.days_done } else { t.days_done + 1 }.clamp(1, days);
        let expires = t.window_expires_at.map(|d| d.timestamp_millis());
        let opens = resets_in(expires, now_ms);
        let mut parts = vec![format!("Day {day} of {days}")];
        if today_done {
            parts.push("done for today".to_string());
            if expires.is_some() && !opens.is_empty() && day < days {
                parts.push(format!("next day {opens}"));
            }
        } else {
            parts.push(format!("{}/{per_day} min today", t.minutes_today));
            if expires.is_some() && !opens.is_empty() {
                parts.push(format!("resets {opens}"));
            }
        }
        return Some(parts.join(" · "));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::drops::{DropBenefit, TwitchProgress};
    use chrono::{TimeZone, Utc};

    fn drop(req: i32, subs: u32, days: u32) -> TimeBasedDrop {
        TimeBasedDrop {
            id: "d".into(),
            name: "d".into(),
            required_minutes_watched: req,
            benefit_edges: vec![],
            progress: None,
            is_collectible: true,
            required_subs: subs,
            required_days: days,
            random_of: None,
            next_reward: None,
        }
    }

    fn with_twitch(mut d: TimeBasedDrop, t: TwitchProgress, claimed: bool) -> TimeBasedDrop {
        d.progress = Some(DropProgress {
            campaign_id: "c".into(),
            drop_id: d.id.clone(),
            current_minutes_watched: 0,
            required_minutes_watched: d.required_minutes_watched,
            is_claimed: claimed,
            last_updated: Utc::now(),
            drop_instance_id: None,
            twitch_progress: Some(t),
        });
        d
    }

    fn twitch() -> TwitchProgress {
        TwitchProgress {
            days_done: 0,
            minutes_today: 0,
            window_expires_at: None,
            accruing: false,
            subs_done: 0,
            ready_to_claim: false,
            earned: None,
        }
    }

    #[test]
    fn requirement_words() {
        assert_eq!(requirement_text(&drop(60, 0, 0)), None);
        assert_eq!(requirement_text(&drop(0, 1, 0)).unwrap(), "Subscribe or gift a sub");
        assert_eq!(requirement_text(&drop(0, 3, 0)).unwrap(), "3 subs or gift subs");
        assert_eq!(requirement_text(&drop(60, 0, 3)).unwrap(), "20 min a day · 3 days");
        assert_eq!(requirement_text(&drop(360, 0, 3)).unwrap(), "2 hr a day · 3 days");
    }

    #[test]
    fn watch_time_words() {
        assert_eq!(to_watch(36), "36m to watch");
        assert_eq!(to_watch(516), "8h 36m to watch");
        assert_eq!(to_watch(120), "2h to watch");
    }

    #[test]
    fn reset_words() {
        assert_eq!(resets_in(None, 0), "");
        assert_eq!(resets_in(Some(30_000), 0), "soon");
        assert_eq!(resets_in(Some(40 * 60_000 + 1), 0), "in 41m");
        assert_eq!(resets_in(Some(23 * 3_600_000 + 5), 0), "in 23h");
    }

    #[test]
    fn progress_lines() {
        let now = Utc.with_ymd_and_hms(2026, 10, 16, 12, 0, 0).unwrap();
        let expires = Utc.with_ymd_and_hms(2026, 10, 17, 11, 0, 0).unwrap();
        // Day 2 of 3 underway.
        let mut t = twitch();
        t.days_done = 1;
        t.minutes_today = 5;
        t.window_expires_at = Some(expires);
        let d = with_twitch(drop(60, 0, 3), t.clone(), false);
        assert_eq!(progress_line(&d, now.timestamp_millis()).unwrap(), "Day 2 of 3 · 5/20 min today · resets in 23h");
        // Today's minutes are in.
        t.days_done = 2;
        t.minutes_today = 20;
        let d = with_twitch(drop(60, 0, 3), t, false);
        assert_eq!(progress_line(&d, now.timestamp_millis()).unwrap(), "Day 2 of 3 · done for today · next day in 23h");
        // Subs.
        let mut t = twitch();
        t.subs_done = 1;
        let d = with_twitch(drop(0, 2, 0), t, false);
        assert_eq!(progress_line(&d, 0).unwrap(), "1 of 2 subs");
        // A ready draw opens; a claimed one names what it gave.
        let mut t = twitch();
        t.ready_to_claim = true;
        let mut d = with_twitch(drop(0, 1, 0), t.clone(), false);
        d.random_of = Some(4);
        assert_eq!(progress_line(&d, 0).unwrap(), "Ready to open");
        t.earned = Some(DropBenefit {
            id: "b".into(),
            name: "Bulbasaur".into(),
            image_url: String::new(),
            distribution_type: None,
        });
        let d = with_twitch(drop(0, 1, 0), t, true);
        assert_eq!(progress_line(&d, 0).unwrap(), "You got Bulbasaur");
        // Minutes only: nothing beyond them.
        assert_eq!(progress_line(&drop(60, 0, 0), 0), None);
    }
}
