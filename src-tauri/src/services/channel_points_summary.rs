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
use tauri::{AppHandle, Emitter, Listener};

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

#[derive(Default)]
struct Burst {
    events: Vec<(Option<String>, i64, String)>,
    last_balance: Option<i64>,
    generation: u64,
}

static BURST: Mutex<Option<Burst>> = Mutex::new(None);

fn summarize(burst: &Burst) -> Option<Summary> {
    if burst.events.is_empty() {
        return None;
    }
    let mut by_channel: Vec<Earned> = Vec::new();
    let mut by_reason: Vec<Earned> = Vec::new();
    let mut total = 0;
    for (channel, points, reason) in &burst.events {
        total += points;
        if let Some(name) = channel {
            match by_channel.iter_mut().find(|e| &e.name == name) {
                Some(e) => e.points += points,
                None => by_channel.push(Earned { name: name.clone(), points: *points }),
            }
        }
        let code = reason.to_uppercase();
        match by_reason.iter_mut().find(|e| e.name == code) {
            Some(e) => e.points += points,
            None => by_reason.push(Earned { name: code, points: *points }),
        }
    }
    // Stable: equal amounts keep first-seen order.
    by_channel.sort_by(|a, b| b.points.cmp(&a.points));
    Some(Summary {
        total_points: total,
        channels: by_channel,
        reasons: by_reason,
        first_reason: burst.events[0].2.clone(),
        last_balance: burst.last_balance,
    })
}

fn record(app: &AppHandle, payload: &serde_json::Value) {
    let points = payload.get("points").and_then(|v| v.as_i64()).unwrap_or(0);
    if points <= 0 {
        return;
    }
    let text = |k: &str| payload.get(k).and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(String::from);
    let channel = text("channel_display_name").or_else(|| text("channel_login"));
    let reason = text("reason").unwrap_or_else(|| "watch".into());
    let generation = {
        let mut guard = BURST.lock().unwrap_or_else(|e| e.into_inner());
        let burst = guard.get_or_insert_with(Burst::default);
        burst.events.push((channel, points, reason));
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

    fn burst(events: &[(Option<&str>, i64, &str)], balance: Option<i64>) -> Burst {
        Burst {
            events: events.iter().map(|(c, p, r)| (c.map(String::from), *p, r.to_string())).collect(),
            last_balance: balance,
            generation: 1,
        }
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
