//! Channel points arrive in bursts (a watch tick on several channels, a bonus
//! claim right after it), and one notification per earn is noise. Earns are
//! gathered until QUIET passes with none, then announced once as
//! `channel-points-summary`: the total, what each channel gave (busiest first),
//! what each reason gave, and the last balance seen.
//!
//! This used to run inside the notification centre component, so it only
//! existed while that window did. Every earn still goes out as
//! `channel-points-earned` for the surfaces that track balances.

use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use crate::rt::AppHandle;
use tauri::{Emitter, Listener};

pub const SUMMARY_EVENT: &str = "channel-points-summary";
const QUIET: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Earned {
    pub name: String,
    pub points: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Summary {
    pub total_points: i64,
    /// Named channels, busiest first.
    pub channels: Vec<Earned>,
    /// Reason codes as Twitch sends them (WATCH, CLAIM, ...), in first-seen order.
    pub reasons: Vec<Earned>,
    /// The reason of the first earn in the burst.
    pub first_reason: String,
    pub last_balance: Option<i64>,
}

/// One earn. Paths report the same channel differently (the chest claim knows
/// only the login, the socket and the poll carry the display name), so the
/// channel is grouped by `key` and shown by the best `name` any of its earns had.
#[derive(Debug, Clone)]
struct Earn {
    /// Channel id, else the lowercased login or name. None = no channel named.
    key: Option<String>,
    name: Option<String>,
    /// `name` is a real display name, not a bare login.
    display: bool,
    points: i64,
    reason: String,
}

#[derive(Default)]
struct Burst {
    events: Vec<Earn>,
    last_balance: Option<i64>,
    generation: u64,
}

static BURST: Mutex<Option<Burst>> = Mutex::new(None);

fn summarize(burst: &Burst) -> Option<Summary> {
    if burst.events.is_empty() {
        return None;
    }
    // (key, name is a display name, earned)
    let mut by_channel: Vec<(&str, bool, Earned)> = Vec::new();
    let mut by_reason: Vec<Earned> = Vec::new();
    let mut total = 0;
    for earn in &burst.events {
        total += earn.points;
        if let (Some(key), Some(name)) = (earn.key.as_deref(), earn.name.as_ref()) {
            match by_channel.iter_mut().find(|(k, _, _)| *k == key) {
                Some((_, display, e)) => {
                    e.points += earn.points;
                    if earn.display && !*display {
                        e.name = name.clone();
                        *display = true;
                    }
                }
                None => by_channel.push((key, earn.display, Earned { name: name.clone(), points: earn.points })),
            }
        }
        let code = earn.reason.to_uppercase();
        match by_reason.iter_mut().find(|e| e.name == code) {
            Some(e) => e.points += earn.points,
            None => by_reason.push(Earned { name: code, points: earn.points }),
        }
    }
    // Stable: equal amounts keep first-seen order.
    by_channel.sort_by(|a, b| b.2.points.cmp(&a.2.points));
    Some(Summary {
        total_points: total,
        channels: by_channel.into_iter().map(|(_, _, e)| e).collect(),
        reasons: by_reason,
        first_reason: burst.events[0].reason.clone(),
        last_balance: burst.last_balance,
    })
}

fn earn_from(text: &dyn Fn(&str) -> Option<String>, points: i64) -> Earn {
    let login = text("channel_login");
    let display_name = text("channel_display_name");
    // A display name equal to the login (the claim path sends the login in
    // both fields) is no better than the login.
    let display = display_name.as_ref().is_some_and(|d| Some(d) != login.as_ref());
    let name = display_name.or_else(|| login.clone());
    let key = text("channel_id").or_else(|| login.or_else(|| name.clone()).map(|s| s.to_lowercase()));
    Earn {
        key,
        name,
        display,
        points,
        reason: text("reason").unwrap_or_else(|| "watch".into()),
    }
}

fn record(app: &AppHandle, payload: &serde_json::Value) {
    let points = payload.get("points").and_then(|v| v.as_i64()).unwrap_or(0);
    if points <= 0 {
        return;
    }
    let text = |k: &str| payload.get(k).and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(String::from);
    let earn = earn_from(&text, points);
    let generation = {
        let mut guard = BURST.lock().unwrap_or_else(|e| e.into_inner());
        let burst = guard.get_or_insert_with(Burst::default);
        burst.events.push(earn);
        if let Some(balance) = payload.get("balance").and_then(|v| v.as_i64()).filter(|b| *b > 0) {
            burst.last_balance = Some(balance);
        }
        burst.generation += 1;
        burst.generation
    };
    // Each earn restarts the quiet period; only the last timer of a burst finds
    // its generation still current and announces.
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(QUIET).await;
        let summary = {
            let mut guard = BURST.lock().unwrap_or_else(|e| e.into_inner());
            let Some(burst) = guard.as_mut().filter(|b| b.generation == generation) else { return };
            let summary = summarize(burst);
            *burst = Burst { generation: burst.generation, ..Default::default() };
            summary
        };
        if let Some(summary) = summary {
            let _ = app.emit(SUMMARY_EVENT, summary);
        }
    });
}

/// Start gathering earns.
pub fn init(app: &AppHandle) {
    let handle = app.clone();
    app.listen("channel-points-earned", move |event| {
        if let Ok(payload) = serde_json::from_str::<serde_json::Value>(event.payload()) {
            record(&handle, &payload);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An earn as the event carries it: `fields` are (key, value) payload pairs.
    fn earn(fields: &[(&str, &str)], points: i64) -> Earn {
        let text = |k: &str| fields.iter().find(|(f, _)| *f == k).map(|(_, v)| v.to_string());
        earn_from(&text, points)
    }

    fn burst(events: &[(Option<&str>, i64, &str)], balance: Option<i64>) -> Burst {
        Burst {
            events: events
                .iter()
                .map(|(c, p, r)| {
                    let mut fields = vec![("reason", *r)];
                    if let Some(c) = c {
                        fields.push(("channel_login", c));
                    }
                    earn(&fields, *p)
                })
                .collect(),
            last_balance: balance,
            generation: 1,
        }
    }

    #[test]
    fn one_channel_reported_by_login_and_by_display_name_is_one_row() {
        // The chest claim sends the login in both name fields; the socket's
        // watch earn carries the display name. Same channel id.
        let claim = earn(
            &[("channel_id", "42"), ("channel_login", "hutchmf"), ("channel_display_name", "hutchmf"), ("reason", "claim")],
            50,
        );
        let watch = earn(
            &[("channel_id", "42"), ("channel_login", "hutchmf"), ("channel_display_name", "HutchMF"), ("reason", "WATCH")],
            10,
        );
        let s = summarize(&Burst { events: vec![claim, watch], last_balance: None, generation: 1 }).unwrap();
        assert_eq!(s.channels, vec![Earned { name: "HutchMF".into(), points: 60 }]);
    }

    #[test]
    fn a_burst_sums_by_channel_busiest_first_and_by_reason() {
        let s = summarize(&burst(
            &[(Some("ninja"), 10, "WATCH"), (Some("poki"), 50, "CLAIM"), (Some("ninja"), 10, "watch"), (None, 5, "RAID")],
            Some(1200),
        ))
        .unwrap();
        assert_eq!(s.total_points, 75);
        assert_eq!(
            s.channels,
            vec![Earned { name: "poki".into(), points: 50 }, Earned { name: "ninja".into(), points: 20 }]
        );
        assert_eq!(
            s.reasons,
            vec![
                Earned { name: "WATCH".into(), points: 20 },
                Earned { name: "CLAIM".into(), points: 50 },
                Earned { name: "RAID".into(), points: 5 },
            ]
        );
        assert_eq!(s.first_reason, "WATCH");
        assert_eq!(s.last_balance, Some(1200));
    }

    #[test]
    fn nothing_gathered_announces_nothing() {
        assert!(summarize(&Burst::default()).is_none());
    }
}
