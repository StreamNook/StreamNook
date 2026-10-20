//! A Twitch poll as every shell draws it.
//!
//! The poll itself is kept by `slash_commands` (the `/vote` cache, the viewer's
//! own vote, polls StreamNook ended) from PubSub and Rust's own follow of the
//! channel's poll. Each time the poll changes, the normalized poll is turned
//! into one typed `poll-state` push here, with the shares, the winner, the
//! deadline and every line decided, and a finished poll's card is taken away
//! after its result has shown a moment.

use log::debug;
use serde::Serialize;
use serde_json::Value;
use tauri::Emitter;

use crate::services::count_text::compact;

/// `{ channel_id, view }`; `view` is null once the card should go.
pub const EVENT: &str = "poll-state";

/// How long a finished poll's result stays on the card.
const RESULT_SHOWN_MS: u64 = 5_000;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PollView {
    pub channel_id: String,
    pub poll_id: String,
    pub title: String,
    /// `open`, `closed` (its time ran out, the final count not in yet) or `ended`.
    pub phase: &'static str,
    /// Unix ms the poll closes, while open: the shells tick to it.
    pub deadline_ms: Option<i64>,
    /// The time slot's words when there is no countdown: "Closed", "Final".
    pub badge: Option<String>,
    pub choices: Vec<ChoiceView>,
    /// A tap on a choice votes.
    pub can_vote: bool,
    pub my_choice_id: Option<String>,
    /// Under the choices: "Tap a choice to vote", "You voted Malenia".
    pub footer: String,
    /// "1,204 voters".
    pub voters_line: String,
    /// Once ended: "Malenia won · 58%", "Your pick Malenia won · 58%".
    pub result_line: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ChoiceView {
    pub id: String,
    pub title: String,
    /// Share of the votes, 0 to 100.
    pub percent: i64,
    pub votes_label: String,
    pub mine: bool,
    pub winner: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PollPush {
    pub channel_id: String,
    pub view: Option<PollView>,
}

/// Text as one line: line breaks and runs of spaces become single spaces.
pub fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn grouped(n: i64) -> String {
    let digits = n.max(0).to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The card for a normalized poll (the `poll-*` payload, with `my_choice_id`
/// stamped) at `now_ms`. None for a poll Twitch hides (deleted or canceled).
pub fn view_of(poll: &Value, now_ms: i64) -> Option<PollView> {
    let channel_id = poll["channel_id"].as_str().unwrap_or("").to_string();
    let poll_id = poll["poll_id"].as_str().filter(|s| !s.is_empty())?.to_string();
    if poll["end_reason"].as_str() == Some("archived") {
        return None;
    }
    let status = poll["status"].as_str().unwrap_or("ACTIVE");
    let ended = status != "ACTIVE" || poll["end_reason"].is_string();
    let remaining = poll["remaining_ms"].as_i64().unwrap_or(0).max(0);
    let open = !ended && remaining > 0;
    let phase = if ended { "ended" } else if open { "open" } else { "closed" };
    let mine = poll["my_choice_id"].as_str().filter(|s| !s.is_empty()).map(String::from);

    let raw: Vec<(String, String, i64)> = poll["choices"]
        .as_array()
        .map(|cs| {
            cs.iter()
                .filter_map(|c| {
                    Some((
                        c["id"].as_str().filter(|s| !s.is_empty())?.to_string(),
                        c["title"].as_str().unwrap_or("").to_string(),
                        c["total_votes"].as_i64().unwrap_or(0).max(0),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let total: i64 = raw.iter().map(|(_, _, v)| v).sum();
    // The winner is the most votes, first listed on a tie, and only once ended.
    let winner = (ended && total > 0)
        .then(|| raw.iter().enumerate().max_by_key(|(i, (_, _, v))| (*v, std::cmp::Reverse(*i))).map(|(_, c)| c.0.clone()))
        .flatten();
    let choices: Vec<ChoiceView> = raw
        .iter()
        .map(|(id, title, votes)| ChoiceView {
            id: id.clone(),
            title: title.clone(),
            percent: if total == 0 { 0 } else { ((*votes as f64 / total as f64) * 100.0).round() as i64 },
            votes_label: compact(*votes as u64),
            mine: mine.as_deref() == Some(id.as_str()),
            winner: winner.as_deref() == Some(id.as_str()),
        })
        .collect();
    let mine_title = mine.as_deref().and_then(|m| raw.iter().find(|c| c.0 == m)).map(|c| c.1.clone());
    let footer = match (&mine_title, phase) {
        (Some(t), _) => format!("You voted {t}"),
        (None, "open") if poll["channel_points_voting"].as_bool() == Some(true) => "Tap a choice to vote (free votes only)".into(),
        (None, "open") => "Tap a choice to vote".into(),
        (None, _) => "Voting is closed".into(),
    };
    let voters = poll["total_voters"].as_i64().unwrap_or(0).max(0);
    let result_line = ended.then(|| match winner.as_deref().and_then(|w| choices.iter().find(|c| c.id == w)) {
        Some(w) if w.mine => format!("Your pick {} won · {}%", w.title, w.percent),
        Some(w) => format!("{} won · {}%", w.title, w.percent),
        None => "No votes".into(),
    });
    Some(PollView {
        channel_id,
        poll_id,
        title: poll["title"].as_str().map(one_line).unwrap_or_default(),
        phase,
        deadline_ms: open.then_some(now_ms + remaining),
        badge: match phase {
            "ended" => Some("Final".into()),
            "closed" => Some("Closed".into()),
            _ => None,
        },
        can_vote: open && mine.is_none(),
        my_choice_id: mine,
        footer,
        voters_line: format!("{} {}", grouped(voters), if voters == 1 { "voter" } else { "voters" }),
        result_line,
        choices,
    })
}

fn emit(push: &PollPush) {
    if let Some(app) = crate::services::providers::app_handle() {
        if let Err(e) = app.emit(EVENT, push) {
            debug!("[Poll] emit failed: {e}");
        }
    }
}

/// Push the card for a normalized poll that just changed. A finished poll's
/// result shows a moment, then the card goes unless a newer poll took its
/// place; a deleted or canceled one goes at once.
pub fn publish(poll: &Value) {
    let Some(channel_id) = poll["channel_id"].as_str().filter(|s| !s.is_empty()).map(String::from) else { return };
    let view = view_of(poll, chrono::Utc::now().timestamp_millis());
    let ended = view.as_ref().filter(|v| v.phase == "ended").map(|v| v.poll_id.clone());
    emit(&PollPush { channel_id: channel_id.clone(), view });
    if let Some(poll_id) = ended {
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(RESULT_SHOWN_MS)).await;
            let newer = crate::services::slash_commands::active_poll(&channel_id).is_some_and(|p| p.poll_id != poll_id);
            if !newer {
                emit(&PollPush { channel_id, view: None });
            }
        });
    }
}

/// The card for the poll kept for `channel_id` now, with the viewer's vote.
pub fn current(channel_id: &str) -> Option<PollView> {
    let mut last = crate::services::slash_commands::active_poll(channel_id)?.last;
    crate::services::slash_commands::stamp_my_vote(&mut last);
    view_of(&last, chrono::Utc::now().timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: i64 = 1_700_000_000_000;

    fn poll(status: &str, remaining: i64, mine: Option<&str>) -> Value {
        json!({
            "channel_id": "c1", "poll_id": "q1", "title": "Which boss next?", "status": status,
            "remaining_ms": remaining, "total_voters": 1204, "total_votes": 1204,
            "channel_points_voting": false,
            "choices": [
                { "id": "a", "title": "Malenia", "total_votes": 698 },
                { "id": "b", "title": "Radahn", "total_votes": 373 },
                { "id": "c", "title": "Mohg", "total_votes": 133 },
            ],
            "my_choice_id": mine,
        })
    }

    #[test]
    fn an_open_poll_counts_down_and_takes_a_vote() {
        let v = view_of(&poll("ACTIVE", 42_000, None), NOW).unwrap();
        assert_eq!(v.phase, "open");
        assert_eq!(v.deadline_ms, Some(NOW + 42_000));
        assert!(v.can_vote);
        assert_eq!(v.choices.iter().map(|c| c.percent).collect::<Vec<_>>(), vec![58, 31, 11]);
        assert_eq!(v.footer, "Tap a choice to vote");
        assert_eq!(v.voters_line, "1,204 voters");
        assert!(v.choices.iter().all(|c| !c.winner), "no winner while it runs");
    }

    #[test]
    fn a_vote_marks_the_choice_and_closes_voting() {
        let v = view_of(&poll("ACTIVE", 42_000, Some("a")), NOW).unwrap();
        assert!(!v.can_vote);
        assert!(v.choices[0].mine);
        assert_eq!(v.footer, "You voted Malenia");
    }

    #[test]
    fn time_running_out_closes_it_before_the_final_count() {
        let v = view_of(&poll("ACTIVE", 0, None), NOW).unwrap();
        assert_eq!(v.phase, "closed");
        assert_eq!(v.badge.as_deref(), Some("Closed"));
        assert!(!v.can_vote);
        assert_eq!(v.footer, "Voting is closed");
    }

    #[test]
    fn an_ended_poll_names_the_winner() {
        let mut p = poll("COMPLETED", 0, Some("a"));
        p["end_reason"] = json!("completed");
        let v = view_of(&p, NOW).unwrap();
        assert_eq!(v.phase, "ended");
        assert_eq!(v.badge.as_deref(), Some("Final"));
        assert!(v.choices[0].winner);
        assert_eq!(v.result_line.as_deref(), Some("Your pick Malenia won · 58%"));
        let mut theirs = poll("COMPLETED", 0, Some("b"));
        theirs["end_reason"] = json!("terminated");
        assert_eq!(view_of(&theirs, NOW).unwrap().result_line.as_deref(), Some("Malenia won · 58%"));
    }

    #[test]
    fn the_question_reads_as_one_clean_line() {
        let mut p = poll("ACTIVE", 10_000, None);
        p["title"] = json!("  Which boss\n next?   ");
        assert_eq!(view_of(&p, NOW).unwrap().title, "Which boss next?");
    }

    #[test]
    fn a_deleted_poll_has_no_card() {
        let mut p = poll("ARCHIVED", 0, None);
        p["end_reason"] = json!("archived");
        assert!(view_of(&p, NOW).is_none());
    }

    #[test]
    fn a_poll_nobody_voted_in_has_no_winner() {
        let mut p = poll("COMPLETED", 0, None);
        p["end_reason"] = json!("completed");
        for c in p["choices"].as_array_mut().unwrap() {
            c["total_votes"] = json!(0);
        }
        let v = view_of(&p, NOW).unwrap();
        assert!(v.choices.iter().all(|c| !c.winner && c.percent == 0));
        assert_eq!(v.result_line.as_deref(), Some("No votes"));
    }

    #[test]
    fn paid_votes_on_say_only_the_free_one_is_offered() {
        let mut p = poll("ACTIVE", 10_000, None);
        p["channel_points_voting"] = json!(true);
        assert_eq!(view_of(&p, NOW).unwrap().footer, "Tap a choice to vote (free votes only)");
    }
}
