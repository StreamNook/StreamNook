//! Channel points arrive in bursts (a watch tick on several channels, a bonus
//! claim right after it), and one notification per earn is noise. Earns are
//! gathered until QUIET passes with none, then announced once as
//! `channel-points-summary`: the total, what each channel gave (busiest first,
//! with its face and points look), what each reason gave, and the last
//! balance seen. The same burst goes to services/points_notify.rs, which
//! decides the system notification.
//!
//! This used to run inside the notification centre component, so it only
//! existed while that window did. Every earn still goes out as
//! `channel-points-earned` for the surfaces that track balances.

use serde::Serialize;
use std::sync::Mutex;
use std::time::Duration;
use crate::rt::AppHandle;
use tauri::{Emitter, Listener};

pub const SUMMARY_EVENT: &str = "channel-points-summary";
const QUIET: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct Earned {
    pub name: String,
    pub points: i64,
}

/// What one channel gave in a burst.
#[derive(Debug, Clone, Serialize, PartialEq, Default)]
pub struct ChannelEarned {
    pub name: String,
    pub points: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    /// The channel's face, its own name for its points and its points icon,
    /// filled in once the burst closes; absent when unknown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub points_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    /// What each reason gave on this channel, codes as Twitch sends them.
    #[serde(skip)]
    pub reasons: Vec<Earned>,
    /// The balance the channel's last earn reported.
    #[serde(skip)]
    pub balance: Option<i64>,
    /// Bonus chests in this burst.
    #[serde(skip)]
    pub chests: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Summary {
    pub total_points: i64,
    /// Named channels, busiest first.
    pub channels: Vec<ChannelEarned>,
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
    channel_id: Option<String>,
    login: Option<String>,
    name: Option<String>,
    /// `name` is a real display name, not a bare login.
    display: bool,
    points: i64,
    reason: String,
    balance: Option<i64>,
}

#[derive(Default)]
struct Burst {
    events: Vec<Earn>,
    last_balance: Option<i64>,
    generation: u64,
}

static BURST: Mutex<Option<Burst>> = Mutex::new(None);

fn add_reason(list: &mut Vec<Earned>, code: String, points: i64) {
    match list.iter_mut().find(|e| e.name == code) {
        Some(e) => e.points += points,
        None => list.push(Earned { name: code, points }),
    }
}

fn summarize(burst: &Burst) -> Option<Summary> {
    if burst.events.is_empty() {
        return None;
    }
    // (key, name is a display name, earned)
    let mut by_channel: Vec<(&str, bool, ChannelEarned)> = Vec::new();
    let mut by_reason: Vec<Earned> = Vec::new();
    let mut total = 0;
    for earn in &burst.events {
        total += earn.points;
        let code = earn.reason.to_uppercase();
        if let (Some(key), Some(name)) = (earn.key.as_deref(), earn.name.as_ref()) {
            let i = match by_channel.iter().position(|(k, _, _)| *k == key) {
                Some(i) => i,
                None => {
                    by_channel.push((key, earn.display, ChannelEarned { name: name.clone(), ..Default::default() }));
                    by_channel.len() - 1
                }
            };
            let (_, display, e) = &mut by_channel[i];
            e.points += earn.points;
            if earn.display && !*display {
                e.name = name.clone();
                *display = true;
            }
            if e.channel_id.is_none() {
                e.channel_id = earn.channel_id.clone();
            }
            if e.login.is_none() {
                e.login = earn.login.clone();
            }
            if earn.balance.is_some() {
                e.balance = earn.balance;
            }
            if code == "CLAIM" {
                e.chests += 1;
            }
            add_reason(&mut e.reasons, code.clone(), earn.points);
        }
        add_reason(&mut by_reason, code, earn.points);
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

fn earn_from(text: &dyn Fn(&str) -> Option<String>, points: i64, balance: Option<i64>) -> Earn {
    let login = text("channel_login");
    let display_name = text("channel_display_name");
    // A display name equal to the login (the claim path sends the login in
    // both fields) is no better than the login.
    let display = display_name.as_ref().is_some_and(|d| Some(d) != login.as_ref());
    let name = display_name.or_else(|| login.clone());
    let channel_id = text("channel_id");
    let key = channel_id.clone().or_else(|| login.clone().or_else(|| name.clone()).map(|s| s.to_lowercase()));
    Earn {
        key,
        channel_id,
        login: login.map(|l| l.to_lowercase()),
        name,
        display,
        points,
        reason: text("reason").unwrap_or_else(|| "watch".into()),
        balance,
    }
}

fn record(app: &AppHandle, payload: &serde_json::Value) {
    let points = payload.get("points").and_then(|v| v.as_i64()).unwrap_or(0);
    if points <= 0 {
        return;
    }
    let text = |k: &str| payload.get(k).and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(String::from);
    let balance = payload.get("balance").and_then(|v| v.as_i64()).filter(|b| *b > 0);
    let earn = earn_from(&text, points, balance);
    let generation = {
        let mut guard = BURST.lock().unwrap_or_else(|e| e.into_inner());
        let burst = guard.get_or_insert_with(Burst::default);
        burst.events.push(earn);
        if balance.is_some() {
            burst.last_balance = balance;
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
        #[cfg(desktop)]
        if let Some(mut summary) = summary {
            crate::services::points_notify::dress(&app, &mut summary.channels).await;
            let _ = app.emit(SUMMARY_EVENT, &summary);
            crate::services::points_notify::on_burst(app, summary.channels).await;
        }
        #[cfg(mobile)]
        if let Some(summary) = summary {
            let _ = app.emit(SUMMARY_EVENT, &summary);
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
        earn_from(&text, points, None)
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

    fn named(channels: &[ChannelEarned]) -> Vec<(&str, i64)> {
        channels.iter().map(|c| (c.name.as_str(), c.points)).collect()
    }

    #[test]
    fn one_channel_reported_by_login_and_by_display_name_is_one_row() {
        // The chest claim sends the login in both name fields; the socket's
        // watch earn carries the display name. Same channel id.
        let claim = earn(
            &[("channel_id", "42"), ("channel_login", "hutchmf"), ("channel_display_name", "hutchmf"), ("reason", "claim")],
            50,
        );
        let mut watch = earn(
            &[("channel_id", "42"), ("channel_login", "hutchmf"), ("channel_display_name", "HutchMF"), ("reason", "WATCH")],
            10,
        );
        watch.balance = Some(900);
        let s = summarize(&Burst { events: vec![claim, watch], last_balance: None, generation: 1 }).unwrap();
        assert_eq!(named(&s.channels), vec![("HutchMF", 60)]);
        let c = &s.channels[0];
        assert_eq!((c.channel_id.as_deref(), c.login.as_deref()), (Some("42"), Some("hutchmf")));
        assert_eq!(c.balance, Some(900));
        assert_eq!(c.chests, 1);
        assert_eq!(
            c.reasons,
            vec![Earned { name: "CLAIM".into(), points: 50 }, Earned { name: "WATCH".into(), points: 10 }]
        );
    }

    #[test]
    fn a_burst_sums_by_channel_busiest_first_and_by_reason() {
        let s = summarize(&burst(
            &[(Some("ninja"), 10, "WATCH"), (Some("poki"), 50, "CLAIM"), (Some("ninja"), 10, "watch"), (None, 5, "RAID")],
            Some(1200),
        ))
        .unwrap();
        assert_eq!(s.total_points, 75);
        assert_eq!(named(&s.channels), vec![("poki", 50), ("ninja", 20)]);
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
    fn the_page_sees_names_and_faces_but_not_the_working_parts() {
        let s = summarize(&burst(&[(Some("poki"), 50, "CLAIM")], None)).unwrap();
        let json = serde_json::to_value(&s.channels[0]).unwrap();
        assert_eq!(json, serde_json::json!({"name": "poki", "points": 50, "login": "poki"}));
    }

    #[test]
    fn nothing_gathered_announces_nothing() {
        assert!(summarize(&Burst::default()).is_none());
    }
}
