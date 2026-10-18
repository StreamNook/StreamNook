//! Twitch predictions as every shell draws them.
//!
//! The channel topic (`predictions-channel-v1`) reports the pooled totals and
//! the phase; the viewer's own topic (`predictions-user-v1`) is the only source
//! for their stake and for whether they won, lost or got their points back.
//! Rust keeps the current prediction per channel from both, so a card that
//! opens mid-prediction gets it at once (`current`), and every change goes out
//! as one typed `prediction-state` push with every word and number already
//! decided. Predicting spends points, so the shells only ever ask Rust what a
//! bet would say (`bet`) and then confirm it (`predict`).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use log::debug;
use serde::Serialize;
use serde_json::Value;
use tauri::Emitter;

use crate::services::channel_state::ChannelPoints;
use crate::services::count_text::compact;

/// `{ channel_id, view }`; `view` is null once the card should go.
pub const EVENT: &str = "prediction-state";

/// How long a finished prediction's result stays on the card.
const RESULT_SHOWN_MS: i64 = 5_000;
/// Twitch's own cap on one viewer's stake in one prediction.
const MAX_STAKE: i64 = 250_000;

#[derive(Clone, Debug, Default, PartialEq)]
struct Outcome {
    id: String,
    title: String,
    color: String,
    points: i64,
    users: i64,
}

#[derive(Clone, Debug, PartialEq)]
struct Stake {
    outcome_id: String,
    points: i64,
}

#[derive(Clone, Debug, PartialEq)]
struct SelfResult {
    /// Twitch's `WIN`, `LOSE` or `REFUND`.
    kind: String,
    points_won: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
struct Prediction {
    channel_id: String,
    id: String,
    title: String,
    /// `ACTIVE`, `LOCKED`, `RESOLVE_PENDING`, `RESOLVED`, `CANCEL_PENDING` or `CANCELED`.
    status: String,
    outcomes: Vec<Outcome>,
    window_seconds: i64,
    created_at_ms: Option<i64>,
    winning_outcome_id: Option<String>,
    stake: Option<Stake>,
    result: Option<SelfResult>,
}

impl Prediction {
    fn ended(&self) -> bool {
        matches!(self.status.as_str(), "RESOLVED" | "CANCELED")
    }

    fn deadline_ms(&self) -> Option<i64> {
        self.created_at_ms.map(|at| at + self.window_seconds * 1000)
    }

    /// Open to predictions at `now_ms`: Twitch says active and the window has not run out.
    fn open_at(&self, now_ms: i64) -> bool {
        self.status == "ACTIVE" && self.deadline_ms().is_none_or(|d| now_ms < d)
    }

    fn outcome(&self, id: &str) -> Option<&Outcome> {
        self.outcomes.iter().find(|o| o.id == id)
    }

    fn pool(&self) -> i64 {
        self.outcomes.iter().map(|o| o.points).sum()
    }
}

static STATE: OnceLock<Mutex<HashMap<String, Prediction>>> = OnceLock::new();
static LOGINS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

fn state() -> std::sync::MutexGuard<'static, HashMap<String, Prediction>> {
    STATE.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap_or_else(|e| e.into_inner())
}

fn logins() -> std::sync::MutexGuard<'static, HashMap<String, String>> {
    LOGINS.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap_or_else(|e| e.into_inner())
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn iso_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|t| t.timestamp_millis())
}

// ---- What the shells draw --------------------------------------------------------

/// One prediction card, decided in full.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PredictionView {
    pub channel_id: String,
    pub prediction_id: String,
    pub title: String,
    /// `open`, `locked` (waiting for the result), `resolving` or `ended`.
    pub phase: &'static str,
    /// Unix ms the window closes, while open: the shells tick to it.
    pub deadline_ms: Option<i64>,
    /// The time slot's words when there is no countdown: "Closed", "Ended".
    pub badge: Option<String>,
    pub outcomes: Vec<OutcomeView>,
    /// Picking a side and an amount is offered.
    pub can_predict: bool,
    /// The side the viewer already put points on; the other sides are closed to them.
    pub picked: Option<String>,
    /// "100 on Yes", or "100 on Yes · waiting for the result" once locked.
    pub status_line: Option<String>,
    /// The ending: "Yes won · you won 280 Nooks", "Canceled · points refunded".
    pub result: Option<ResultView>,
    /// "1,100 people · 52.3K Nooks".
    pub totals_line: String,
    /// The channel's name for its points ("Nooks"), else "points".
    pub points_name: String,
    pub points_icon: Option<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct OutcomeView {
    pub id: String,
    pub title: String,
    /// Twitch's color for the side, lowercase: `blue`, `pink`, others as sent.
    pub color: String,
    /// Share of the points, 0 to 100. An even split while nobody has predicted.
    pub percent: i64,
    /// What one point returns if this side wins: "1:2.78"; empty while nobody is on it.
    pub ratio: String,
    pub points_label: String,
    pub people_label: String,
    pub winner: bool,
    pub mine: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ResultView {
    /// `won`, `lost`, `refunded` or `ended` (the viewer had no known stake).
    pub tone: &'static str,
    pub line: String,
}

/// The push and the getter's answer.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PredictionPush {
    pub channel_id: String,
    pub view: Option<PredictionView>,
}

fn points_word(points: Option<&ChannelPoints>) -> String {
    points
        .and_then(|p| p.name.as_deref())
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or("points")
        .to_string()
}

fn grouped(n: i64) -> String {
    let digits = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

fn ratio(pool: i64, side: i64) -> String {
    if side <= 0 {
        return String::new();
    }
    let r = pool as f64 / side as f64;
    let s = format!("{r:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    format!("1:{s}")
}

fn percents(outcomes: &[Outcome]) -> Vec<i64> {
    let pool: i64 = outcomes.iter().map(|o| o.points).sum();
    if pool <= 0 {
        let n = outcomes.len().max(1) as i64;
        return outcomes.iter().map(|_| (100.0 / n as f64).round() as i64).collect();
    }
    outcomes.iter().map(|o| ((o.points as f64 / pool as f64) * 100.0).round() as i64).collect()
}

fn view_of(p: &Prediction, points: Option<&ChannelPoints>, now_ms: i64) -> PredictionView {
    let word = points_word(points);
    let open = p.open_at(now_ms);
    let phase = match p.status.as_str() {
        _ if open => "open",
        "RESOLVED" | "CANCELED" => "ended",
        "RESOLVE_PENDING" | "CANCEL_PENDING" => "resolving",
        _ => "locked",
    };
    let pool = p.pool();
    let shares = percents(&p.outcomes);
    let winner = p.winning_outcome_id.as_deref().filter(|_| p.status == "RESOLVED");
    let picked = p.stake.as_ref().map(|s| s.outcome_id.clone());
    let outcomes = p
        .outcomes
        .iter()
        .zip(shares)
        .map(|(o, percent)| OutcomeView {
            id: o.id.clone(),
            title: o.title.clone(),
            color: o.color.to_ascii_lowercase(),
            percent,
            ratio: ratio(pool, o.points),
            points_label: compact(o.points.max(0) as u64),
            people_label: compact(o.users.max(0) as u64),
            winner: winner == Some(o.id.as_str()),
            mine: picked.as_deref() == Some(o.id.as_str()),
        })
        .collect();
    let stake_words = p.stake.as_ref().map(|s| {
        let side = p.outcome(&s.outcome_id).map(|o| o.title.as_str()).unwrap_or("your pick");
        format!("{} on {side}", grouped(s.points))
    });
    let status_line = match phase {
        "open" => stake_words.clone(),
        "locked" => Some(match &stake_words {
            Some(s) => format!("{s} · waiting for the result"),
            None => "Waiting for the result".into(),
        }),
        "resolving" => Some("Working out the result".into()),
        _ => None,
    };
    let result = (phase == "ended").then(|| result_of(p, &word));
    let people: i64 = p.outcomes.iter().map(|o| o.users).sum();
    PredictionView {
        channel_id: p.channel_id.clone(),
        prediction_id: p.id.clone(),
        title: crate::services::poll_view::one_line(&p.title),
        phase,
        deadline_ms: if open { p.deadline_ms() } else { None },
        badge: match phase {
            "open" => None,
            "ended" => Some("Ended".into()),
            _ => Some("Closed".into()),
        },
        outcomes,
        can_predict: open,
        picked,
        status_line,
        result,
        totals_line: format!(
            "{} {} · {} {word}",
            grouped(people),
            if people == 1 { "person" } else { "people" },
            compact(pool.max(0) as u64)
        ),
        points_name: word,
        points_icon: points.and_then(|p| p.icon_url.clone()),
    }
}

/// How it ended for this viewer. Twitch's own result decides; without one (no
/// stake known here) the card just names the winner.
fn result_of(p: &Prediction, word: &str) -> ResultView {
    let winner = p.winning_outcome_id.as_deref().and_then(|id| p.outcome(id)).map(|o| o.title.clone());
    if p.status == "CANCELED" || winner.is_none() {
        let line = match &p.stake {
            Some(s) => format!("Canceled · your {} {word} came back", grouped(s.points)),
            None => "Canceled · points refunded".into(),
        };
        return ResultView { tone: "refunded", line };
    }
    let winner = winner.unwrap_or_default();
    match (&p.stake, &p.result) {
        (Some(_), Some(r)) if r.kind == "WIN" => ResultView {
            tone: "won",
            line: match r.points_won {
                Some(won) if won > 0 => format!("{winner} won · you won {} {word}", grouped(won)),
                _ => format!("{winner} won · you called it"),
            },
        },
        (Some(s), Some(r)) if r.kind == "LOSE" => {
            ResultView { tone: "lost", line: format!("{winner} won · you lost {}", grouped(s.points)) }
        }
        (Some(s), Some(r)) if r.kind == "REFUND" => ResultView {
            tone: "refunded",
            line: format!("{winner} won · your {} {word} came back", grouped(s.points)),
        },
        // A stake with no result yet: judged from the side it is on.
        (Some(s), None) if Some(s.outcome_id.as_str()) == p.winning_outcome_id.as_deref() => {
            ResultView { tone: "won", line: format!("{winner} won · you called it") }
        }
        (Some(s), None) => ResultView { tone: "lost", line: format!("{winner} won · you lost {}", grouped(s.points)) },
        _ => ResultView { tone: "ended", line: format!("{winner} won") },
    }
}

// ---- The bet: what it would say, and placing it ------------------------------------

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct BetPreset {
    pub label: String,
    pub amount: i64,
}

/// What a bet of `amount` on one side would do, in the words the card and its
/// confirm show. Nothing is spent by asking.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct BetView {
    /// The amount as clamped: at least 1, at most the balance and Twitch's cap.
    pub amount: i64,
    pub max: i64,
    pub presets: Vec<BetPreset>,
    /// Predicting is possible as asked.
    pub ok: bool,
    /// Why not, in a few words, when `ok` is false.
    pub problem: Option<String>,
    /// "Predict 100 on Yes" / "Add 100 to Yes".
    pub button: String,
    /// "Win up to 280".
    pub win_line: String,
    /// "Predict 100 Nooks on Yes?" / "Add 100 Nooks to Yes?".
    pub confirm_title: String,
    /// "Win up to 280 if Yes wins."
    pub confirm_line: String,
    /// "You can add more later, but you can't switch sides." on a first bet.
    pub confirm_note: Option<String>,
    /// "12,850 Nooks", when the balance is known.
    pub balance_line: Option<String>,
}

/// The total a stake of `stake` on `side` returns if that side wins, with
/// `added` of it new to the pool.
fn payout(pool: i64, side: i64, stake: i64, added: i64) -> i64 {
    let side_after = side + added;
    if side_after <= 0 {
        return stake;
    }
    ((stake as f64) * ((pool + added) as f64) / (side_after as f64)).floor() as i64
}

fn bet_of(p: &Prediction, outcome_id: &str, asked: Option<&str>, points: Option<&ChannelPoints>, now_ms: i64) -> BetView {
    let word = points_word(points);
    let balance = points.and_then(|c| c.balance);
    let max = balance.map_or(MAX_STAKE, |b| b.clamp(0, MAX_STAKE));
    let digits: String = asked.unwrap_or("").chars().filter(char::is_ascii_digit).collect();
    let wanted = digits.parse::<i64>().unwrap_or(10);
    let amount = wanted.clamp(1, max.max(1));
    let side = p.outcome(outcome_id);
    let side_title = side.map(|o| o.title.clone()).unwrap_or_else(|| "this side".into());
    let topping_up = p.stake.as_ref().is_some_and(|s| s.outcome_id == outcome_id);
    let problem = if !p.open_at(now_ms) {
        Some("Predictions are closed".to_string())
    } else if side.is_none() {
        Some("That side is gone".to_string())
    } else if p.stake.as_ref().is_some_and(|s| s.outcome_id != outcome_id) {
        let mine = p.stake.as_ref().and_then(|s| p.outcome(&s.outcome_id)).map(|o| o.title.as_str()).unwrap_or("the other side");
        Some(format!("You already picked {mine}"))
    } else if balance == Some(0) {
        Some(format!("You have no {word} yet"))
    } else {
        None
    };
    let held = p.stake.as_ref().filter(|_| topping_up).map_or(0, |s| s.points);
    let win = payout(p.pool(), side.map_or(0, |o| o.points), held + amount, amount);
    let amount_words = grouped(amount);
    let mut presets: Vec<BetPreset> = [10, 100, 1_000]
        .into_iter()
        .filter(|n| *n <= max)
        .map(|n| BetPreset { label: compact(n as u64), amount: n })
        .collect();
    if max > 0 && max != 1_000 {
        presets.push(BetPreset { label: "Max".into(), amount: max });
    }
    BetView {
        amount,
        max,
        presets,
        ok: problem.is_none(),
        problem,
        button: if topping_up { format!("Add {amount_words} to {side_title}") } else { format!("Predict {amount_words} on {side_title}") },
        win_line: format!("Win up to {}", grouped(win)),
        confirm_title: if topping_up {
            format!("Add {amount_words} {word} to {side_title}?")
        } else {
            format!("Predict {amount_words} {word} on {side_title}?")
        },
        confirm_line: format!("Win up to {} if {side_title} wins.", grouped(win)),
        confirm_note: (!topping_up).then(|| "You can add more later, but you can't switch sides.".to_string()),
        balance_line: balance.map(|b| format!("{} {word}", grouped(b))),
    }
}

/// What predicting answered.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PredictOutcome {
    pub success: bool,
    /// "Predicted 100 on Yes", "Added 100 to Yes", or why it failed.
    pub message: String,
    pub view: Option<PredictionView>,
}

// ---- Feeding it ----------------------------------------------------------------

fn outcomes_of(event: &Value) -> Vec<Outcome> {
    event["outcomes"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|o| {
                    Some(Outcome {
                        id: o["id"].as_str().filter(|s| !s.is_empty())?.to_string(),
                        title: o["title"].as_str().unwrap_or("").to_string(),
                        color: o["color"].as_str().unwrap_or("BLUE").to_string(),
                        points: o["total_points"].as_i64().unwrap_or(0),
                        users: o["total_users"].as_i64().unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Lay a channel-topic event (`event-created` / `event-updated`, the whole
/// prediction) over what is kept. True when the card changed.
fn apply_event(kept: &mut HashMap<String, Prediction>, channel_id: &str, event: &Value) -> bool {
    let Some(id) = event["id"].as_str().filter(|s| !s.is_empty()) else { return false };
    let before = kept.get(channel_id).cloned();
    let carried = before.as_ref().filter(|p| p.id == id);
    let mut next = Prediction {
        channel_id: channel_id.to_string(),
        id: id.to_string(),
        title: event["title"].as_str().map(String::from).or_else(|| carried.map(|p| p.title.clone())).unwrap_or_default(),
        status: event["status"].as_str().map(String::from).or_else(|| carried.map(|p| p.status.clone())).unwrap_or_else(|| "ACTIVE".into()),
        outcomes: outcomes_of(event),
        window_seconds: event["prediction_window_seconds"].as_i64().or_else(|| carried.map(|p| p.window_seconds)).unwrap_or(0),
        created_at_ms: event["created_at"].as_str().and_then(iso_ms).or_else(|| carried.and_then(|p| p.created_at_ms)),
        winning_outcome_id: event["winning_outcome_id"].as_str().filter(|s| !s.is_empty()).map(String::from),
        stake: carried.and_then(|p| p.stake.clone()),
        result: carried.and_then(|p| p.result.clone()),
    };
    if next.outcomes.is_empty() {
        if let Some(c) = carried {
            next.outcomes = c.outcomes.clone();
        }
    }
    if before.as_ref() == Some(&next) {
        return false;
    }
    kept.insert(channel_id.to_string(), next);
    true
}

/// Lay the viewer's own event over what is kept. True when the card changed.
fn apply_self(
    kept: &mut HashMap<String, Prediction>,
    channel_id: &str,
    event_id: &str,
    outcome_id: &str,
    points: i64,
    result_type: Option<&str>,
    points_won: Option<i64>,
) -> bool {
    let Some(p) = kept.get_mut(channel_id).filter(|p| p.id == event_id) else { return false };
    let before = (p.stake.clone(), p.result.clone());
    if !outcome_id.is_empty() && points > 0 {
        p.stake = Some(Stake { outcome_id: outcome_id.to_string(), points });
    }
    if let Some(kind) = result_type.filter(|k| !k.is_empty()) {
        p.result = Some(SelfResult { kind: kind.to_string(), points_won });
    }
    before != (p.stake.clone(), p.result.clone())
}

async fn points_for(channel_id: &str) -> Option<ChannelPoints> {
    let login = logins().get(channel_id).cloned()?;
    crate::services::channel_state::get(&login).await.and_then(|s| s.points)
}

/// The card for `channel_id` now, or None.
async fn view_now(channel_id: &str) -> Option<PredictionView> {
    let p = state().get(channel_id).cloned()?;
    let points = points_for(channel_id).await;
    Some(view_of(&p, points.as_ref(), now_ms()))
}

fn emit(push: &PredictionPush) {
    if let Some(app) = crate::services::providers::app_handle() {
        if let Err(e) = app.emit(EVENT, push) {
            debug!("[Prediction] emit failed: {e}");
        }
    }
}

/// Push the card, and once a prediction ends, let its result show a moment
/// and then take the card away (unless a newer prediction took its place).
fn publish(channel_id: &str) {
    let channel_id = channel_id.to_string();
    tauri::async_runtime::spawn(async move {
        let view = view_now(&channel_id).await;
        let ended = view.as_ref().filter(|v| v.phase == "ended").map(|v| v.prediction_id.clone());
        emit(&PredictionPush { channel_id: channel_id.clone(), view });
        if let Some(id) = ended {
            tokio::time::sleep(std::time::Duration::from_millis(RESULT_SHOWN_MS as u64)).await;
            let gone = {
                let mut kept = state();
                let same = kept.get(&channel_id).is_some_and(|p| p.id == id && p.ended());
                if same {
                    kept.remove(&channel_id);
                }
                same
            };
            if gone {
                emit(&PredictionPush { channel_id, view: None });
            }
        }
    });
}

/// A channel-topic message (`predictions-channel-v1`) for `channel_id`, whose
/// login is `login` when known.
pub fn note_channel_event(channel_id: &str, login: Option<&str>, message: &Value) {
    if channel_id.is_empty() {
        return;
    }
    if let Some(login) = login.filter(|l| !l.is_empty()) {
        logins().insert(channel_id.to_string(), login.to_lowercase());
    }
    let event = &message["data"]["event"];
    if apply_event(&mut state(), channel_id, event) {
        publish(channel_id);
    }
}

/// The viewer's own message (`predictions-user-v1`).
pub fn note_self(
    channel_id: &str,
    event_id: &str,
    outcome_id: &str,
    points: i64,
    result_type: Option<&str>,
    points_won: Option<i64>,
) {
    if apply_self(&mut state(), channel_id, event_id, outcome_id, points, result_type, points_won) {
        publish(channel_id);
    }
}

/// The card for a channel a shell just opened, keyed by id, with its login so
/// the points name and balance can be read.
pub async fn current(channel_id: &str, login: Option<&str>) -> PredictionPush {
    if let Some(login) = login.map(str::trim).filter(|l| !l.is_empty()) {
        logins().insert(channel_id.to_string(), login.to_lowercase());
    }
    PredictionPush { channel_id: channel_id.to_string(), view: view_now(channel_id).await }
}

/// What a bet of `amount` (digits as typed) on `outcome_id` would say.
pub async fn bet(channel_id: &str, outcome_id: &str, amount: Option<&str>) -> Option<BetView> {
    let p = state().get(channel_id).cloned()?;
    let points = points_for(channel_id).await;
    Some(bet_of(&p, outcome_id, amount, points.as_ref(), now_ms()))
}

/// The confirm's yes: put `amount` on `outcome_id`. Judged against the kept
/// prediction first, so a closed window or the other side is refused here.
pub async fn predict(channel_id: &str, outcome_id: &str, amount: i64) -> PredictOutcome {
    let Some(p) = state().get(channel_id).cloned() else {
        return PredictOutcome { success: false, message: "This prediction is over".into(), view: None };
    };
    let points = points_for(channel_id).await;
    let words = bet_of(&p, outcome_id, Some(&amount.to_string()), points.as_ref(), now_ms());
    if let Some(problem) = words.problem.clone() {
        return PredictOutcome { success: false, message: problem, view: view_now(channel_id).await };
    }
    let topping_up = p.stake.as_ref().is_some_and(|s| s.outcome_id == outcome_id);
    let side = p.outcome(outcome_id).map(|o| o.title.clone()).unwrap_or_default();
    let answer = crate::commands::drops::place_prediction(
        p.id.clone(),
        outcome_id.to_string(),
        words.amount as i32,
        channel_id.to_string(),
    )
    .await;
    match answer {
        Ok(body) => {
            // Twitch answers with the running total, which the viewer's own
            // topic also sends; both set it rather than add to it.
            let total = body["data"]["makePrediction"]["prediction"]["points"]
                .as_i64()
                .unwrap_or(words.amount + p.stake.as_ref().map_or(0, |s| s.points));
            apply_self(&mut state(), channel_id, &p.id, outcome_id, total, None, None);
            publish(channel_id);
            let amount = grouped(words.amount);
            PredictOutcome {
                success: true,
                message: if topping_up { format!("Added {amount} to {side}") } else { format!("Predicted {amount} on {side}") },
                view: view_now(channel_id).await,
            }
        }
        Err(e) => PredictOutcome { success: false, message: failure_words(&e), view: view_now(channel_id).await },
    }
}

/// Twitch's refusal in the viewer's words.
fn failure_words(error: &str) -> String {
    let code = error.rsplit(':').next().unwrap_or("").trim();
    match code {
        "NOT_ENOUGH_POINTS" => "You don't have enough points".into(),
        "EVENT_NOT_ACTIVE" | "PREDICTION_LOCKED" | "EVENT_LOCKED" => "Predictions are closed".into(),
        "MAX_POINTS_PER_EVENT" | "TOO_MANY_POINTS" => "That's more than one prediction can take".into(),
        "DUPLICATE_TRANSACTION" => "Already sent".into(),
        _ if error.contains("token") => "Sign in to Twitch drops to predict".into(),
        _ => "Couldn't place the prediction. Try again".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: i64 = 1_700_000_000_000;

    fn created(status: &str) -> Value {
        json!({
            "id": "p1", "title": "Beats it first try?", "status": status,
            "prediction_window_seconds": 120,
            "created_at": chrono::DateTime::from_timestamp_millis(NOW - 30_000).unwrap().to_rfc3339(),
            "outcomes": [
                { "id": "yes", "title": "Yes", "color": "BLUE", "total_points": 3600, "total_users": 412 },
                { "id": "no", "title": "No", "color": "PINK", "total_points": 6400, "total_users": 688 },
            ],
        })
    }

    fn kept_with(event: Value) -> HashMap<String, Prediction> {
        let mut kept = HashMap::new();
        assert!(apply_event(&mut kept, "c1", &event));
        kept
    }

    fn nooks(balance: i64) -> ChannelPoints {
        ChannelPoints { enabled: true, balance: Some(balance), name: Some("Nooks".into()), ..Default::default() }
    }

    #[test]
    fn an_open_prediction_has_a_deadline_shares_and_ratios() {
        let kept = kept_with(created("ACTIVE"));
        let v = view_of(&kept["c1"], Some(&nooks(12_850)), NOW);
        assert_eq!(v.phase, "open");
        assert!(v.can_predict);
        assert_eq!(v.deadline_ms, Some(NOW + 90_000), "the true time left, not a guess");
        assert_eq!(v.outcomes[0].percent, 36);
        assert_eq!(v.outcomes[1].percent, 64);
        assert_eq!(v.outcomes[0].ratio, "1:2.78");
        assert_eq!(v.outcomes[0].color, "blue");
        assert_eq!(v.totals_line, "1,100 people · 10K Nooks");
        assert_eq!(v.status_line, None);
    }

    #[test]
    fn nobody_predicting_yet_splits_evenly() {
        let mut e = created("ACTIVE");
        e["outcomes"][0]["total_points"] = json!(0);
        e["outcomes"][1]["total_points"] = json!(0);
        let kept = kept_with(e);
        let v = view_of(&kept["c1"], None, NOW);
        assert_eq!((v.outcomes[0].percent, v.outcomes[1].percent), (50, 50));
        assert_eq!(v.outcomes[0].ratio, "");
        assert!(v.totals_line.ends_with(" points"));
    }

    #[test]
    fn a_window_that_ran_out_is_closed_even_before_twitch_says_so() {
        let kept = kept_with(created("ACTIVE"));
        let v = view_of(&kept["c1"], None, NOW + 91_000);
        assert_eq!(v.phase, "locked");
        assert!(!v.can_predict);
        assert_eq!(v.badge.as_deref(), Some("Closed"));
    }

    #[test]
    fn a_stake_fixes_the_side_and_shows_on_the_card() {
        let mut kept = kept_with(created("ACTIVE"));
        assert!(apply_self(&mut kept, "c1", "p1", "yes", 100, None, None));
        let v = view_of(&kept["c1"], Some(&nooks(500)), NOW);
        assert_eq!(v.picked.as_deref(), Some("yes"));
        assert!(v.outcomes[0].mine);
        assert_eq!(v.status_line.as_deref(), Some("100 on Yes"));
        let other = bet_of(&kept["c1"], "no", Some("50"), Some(&nooks(500)), NOW);
        assert!(!other.ok);
        assert_eq!(other.problem.as_deref(), Some("You already picked Yes"));
        let more = bet_of(&kept["c1"], "yes", Some("50"), Some(&nooks(500)), NOW);
        assert!(more.ok);
        assert_eq!(more.button, "Add 50 to Yes");
        assert_eq!(more.confirm_title, "Add 50 Nooks to Yes?");
        assert_eq!(more.confirm_note, None);
    }

    #[test]
    fn a_later_update_keeps_the_stake_and_the_locked_line_says_so() {
        let mut kept = kept_with(created("ACTIVE"));
        apply_self(&mut kept, "c1", "p1", "yes", 100, None, None);
        assert!(apply_event(&mut kept, "c1", &created("LOCKED")));
        let v = view_of(&kept["c1"], None, NOW);
        assert_eq!(v.phase, "locked");
        assert_eq!(v.status_line.as_deref(), Some("100 on Yes · waiting for the result"));
        assert!(!apply_event(&mut kept, "c1", &created("LOCKED")), "the same frame twice is no change");
    }

    #[test]
    fn the_bet_is_clamped_and_worded_for_the_confirm() {
        let kept = kept_with(created("ACTIVE"));
        let b = bet_of(&kept["c1"], "yes", Some("100"), Some(&nooks(12_850)), NOW);
        assert!(b.ok);
        assert_eq!(b.amount, 100);
        assert_eq!(b.button, "Predict 100 on Yes");
        assert_eq!(b.confirm_title, "Predict 100 Nooks on Yes?");
        // 100 more on Yes: pool 10,100, Yes 3,700, so 100 returns 272.
        assert_eq!(b.win_line, "Win up to 272");
        assert_eq!(b.confirm_line, "Win up to 272 if Yes wins.");
        assert_eq!(b.confirm_note.as_deref(), Some("You can add more later, but you can't switch sides."));
        assert_eq!(b.balance_line.as_deref(), Some("12,850 Nooks"));
        assert_eq!(b.presets.iter().map(|p| p.label.as_str()).collect::<Vec<_>>(), vec!["10", "100", "1K", "Max"]);
        let too_much = bet_of(&kept["c1"], "yes", Some("99,999"), Some(&nooks(500)), NOW);
        assert_eq!(too_much.amount, 500);
        let none_typed = bet_of(&kept["c1"], "yes", Some(""), Some(&nooks(500)), NOW);
        assert_eq!(none_typed.amount, 10);
        let broke = bet_of(&kept["c1"], "yes", Some("10"), Some(&nooks(0)), NOW);
        assert_eq!(broke.problem.as_deref(), Some("You have no Nooks yet"));
        let closed = bet_of(&kept["c1"], "yes", Some("10"), None, NOW + 200_000);
        assert_eq!(closed.problem.as_deref(), Some("Predictions are closed"));
    }

    #[test]
    fn twitchs_result_decides_win_loss_and_refund() {
        let mut won = kept_with(created("ACTIVE"));
        apply_self(&mut won, "c1", "p1", "yes", 100, None, None);
        let mut resolved = created("RESOLVED");
        resolved["winning_outcome_id"] = json!("yes");
        apply_event(&mut won, "c1", &resolved);
        apply_self(&mut won, "c1", "p1", "yes", 100, Some("WIN"), Some(280));
        let v = view_of(&won["c1"], Some(&nooks(0)), NOW);
        assert_eq!(v.phase, "ended");
        assert_eq!(v.result, Some(ResultView { tone: "won", line: "Yes won · you won 280 Nooks".into() }));
        assert!(v.outcomes[0].winner);

        let mut lost = kept_with(created("ACTIVE"));
        apply_self(&mut lost, "c1", "p1", "no", 50, None, None);
        apply_event(&mut lost, "c1", &resolved);
        apply_self(&mut lost, "c1", "p1", "no", 50, Some("LOSE"), Some(0));
        assert_eq!(view_of(&lost["c1"], None, NOW).result.unwrap().line, "Yes won · you lost 50");

        let mut watched = kept_with(created("ACTIVE"));
        apply_event(&mut watched, "c1", &resolved);
        assert_eq!(view_of(&watched["c1"], None, NOW).result, Some(ResultView { tone: "ended", line: "Yes won".into() }));

        let mut canceled = kept_with(created("ACTIVE"));
        apply_self(&mut canceled, "c1", "p1", "no", 50, None, None);
        apply_event(&mut canceled, "c1", &created("CANCELED"));
        let r = view_of(&canceled["c1"], Some(&nooks(9)), NOW).result.unwrap();
        assert_eq!((r.tone, r.line.as_str()), ("refunded", "Canceled · your 50 Nooks came back"));
    }

    #[test]
    fn a_self_event_for_another_prediction_changes_nothing() {
        let mut kept = kept_with(created("ACTIVE"));
        assert!(!apply_self(&mut kept, "c1", "old", "yes", 100, None, None));
        assert!(!apply_self(&mut kept, "c2", "p1", "yes", 100, None, None));
    }

    #[test]
    fn a_new_prediction_replaces_the_old_one_and_its_stake() {
        let mut kept = kept_with(created("ACTIVE"));
        apply_self(&mut kept, "c1", "p1", "yes", 100, None, None);
        let mut next = created("ACTIVE");
        next["id"] = json!("p2");
        assert!(apply_event(&mut kept, "c1", &next));
        assert_eq!(kept["c1"].stake, None);
    }

    #[test]
    fn refusals_read_as_the_viewers_words() {
        assert_eq!(failure_words("Prediction failed: NOT_ENOUGH_POINTS"), "You don't have enough points");
        assert_eq!(failure_words("Prediction failed: EVENT_NOT_ACTIVE"), "Predictions are closed");
        assert_eq!(failure_words("Failed to get token: none"), "Sign in to Twitch drops to predict");
    }
}
