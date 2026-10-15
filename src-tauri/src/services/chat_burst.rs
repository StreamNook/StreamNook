//! `/spam` and `/pyramid`: a run of chat messages the viewer asked for, sent
//! from Rust at the pace the viewer chose.
//!
//! The viewer owns the speed (Settings > Chat > Spam & pyramid, or a
//! `--delay` on the command). Nothing here second-guesses it. The rails that
//! stay are the ones that protect the viewer from the command itself:
//!
//! - one run per channel, stoppable with `/spam stop` or `/pyramid stop`;
//! - a line may never start with `/` or `.`, so a run can never fire a chat
//!   command;
//! - every line goes out over Helix only, so Twitch confirms or refuses each
//!   one; a refused request is never retried over IRC, where a refusal is
//!   invisible and the line would leave as whoever owns the IRC connection;
//! - the run ends on its own when Twitch says the viewer is timed out or
//!   banned, or drops [`DROPS_IN_A_ROW_TO_STOP`] lines in a row, because
//!   sending into a wall helps nobody;
//! - an account switch or sign-out stops every run ([`stop_all`]);
//! - a typo bound ([`MAX_SPAM_COUNT`], [`MAX_PYRAMID_HEIGHT`]) and Twitch's
//!   500-character line limit.
//!
//! Progress is pushed as [`EVENT`] so the composer can show "Pyramid 5/9".

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;

/// Tauri event carrying a [`BurstStatus`] whenever a run moves or ends.
pub const EVENT: &str = "chat-burst-progress";

/// Twitch's limit for one chat message, in characters.
pub const MAX_LINE_CHARS: usize = 500;
/// Upper bound on `/spam` that only exists to catch typos like `/spam 9999999`.
pub const MAX_SPAM_COUNT: u32 = 1000;
/// Upper bound on `/pyramid` height, for the same reason.
pub const MAX_PYRAMID_HEIGHT: u32 = 100;
/// The pause between two lines, in milliseconds.
pub const DEFAULT_DELAY_MS: u32 = 1000;
pub const MIN_DELAY_MS: u32 = 10;
pub const MAX_DELAY_MS: u32 = 5000;
/// Dropped lines in a row that end a run.
pub const DROPS_IN_A_ROW_TO_STOP: u32 = 3;
/// Characters kept free on every line while "Vary repeated messages" is on:
/// the variation is a space and U+E0000, three UTF-16 units, so four covers
/// either way Twitch counts with one to spare.
pub const VARY_RESERVE_CHARS: usize = 4;
/// What "Vary repeats" appends to every other copy of a repeated line. Twitch
/// keeps U+E0000 in chat text; it strips the other tag characters
/// (U+E0020..U+E007E), so a variation built from those arrives identical and
/// the duplicate filter drops it.
pub const VARY_SUFFIX: &str = " \u{E0000}";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BurstKind {
    Spam,
    Pyramid,
}

impl BurstKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "spam" => Some(Self::Spam),
            "pyramid" => Some(Self::Pyramid),
            _ => None,
        }
    }

    fn command(self) -> &'static str {
        match self {
            Self::Spam => "/spam",
            Self::Pyramid => "/pyramid",
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Self::Spam => "Spam",
            Self::Pyramid => "Pyramid",
        }
    }

    fn usage(self) -> &'static str {
        match self {
            Self::Spam => "Usage: /spam <count> <message> [--delay <ms>], or /spam stop",
            Self::Pyramid => "Usage: /pyramid <height> <word> [--delay <ms>], or /pyramid stop",
        }
    }
}

/// What the text after `/spam` or `/pyramid` asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum BurstArgs {
    Start {
        amount: u32,
        text: String,
        /// `--delay <ms>` at the end of the command, overriding the setting.
        delay_ms: Option<u32>,
    },
    Stop,
}

/// Reads the arguments of `/spam` or `/pyramid` (everything after the command
/// name). The message keeps its own spacing; only the count and an optional
/// trailing `--delay <ms>` are taken off it.
pub fn parse_args(kind: BurstKind, raw: &str) -> Result<BurstArgs, String> {
    let raw = raw.trim();
    if raw.eq_ignore_ascii_case("stop") {
        return Ok(BurstArgs::Stop);
    }
    let (body, delay_ms) = split_delay(raw)?;
    let (amount_text, text) = match body.split_once(char::is_whitespace) {
        Some((a, t)) => (a, t.trim()),
        None => (body, ""),
    };
    if amount_text.is_empty() || text.is_empty() {
        return Err(kind.usage().to_string());
    }
    let amount: u32 = amount_text
        .parse()
        .map_err(|_| format!("\"{amount_text}\" is not a number. {}", kind.usage()))?;
    Ok(BurstArgs::Start { amount, text: text.to_string(), delay_ms })
}

/// Splits a trailing `--delay <ms>` (or `--delay <ms>ms`) off the command.
fn split_delay(raw: &str) -> Result<(&str, Option<u32>), String> {
    let Some(at) = raw.rfind("--delay") else {
        return Ok((raw, None));
    };
    // Only a whole word at the end counts; "--delayed" or a flag in the
    // middle of the message is part of the message.
    let before_ok = at == 0 || raw[..at].ends_with(char::is_whitespace);
    let rest = &raw[at + "--delay".len()..];
    if !before_ok || !rest.starts_with(char::is_whitespace) {
        return Ok((raw, None));
    }
    let value = rest.trim();
    if value.is_empty() || value.contains(char::is_whitespace) {
        return Ok((raw, None));
    }
    let number = value.strip_suffix("ms").unwrap_or(value);
    let ms: u32 = number
        .parse()
        .map_err(|_| format!("--delay takes milliseconds, like --delay 250 (got \"{value}\")"))?;
    Ok((raw[..at].trim_end(), Some(check_delay(ms)?)))
}

/// A delay the viewer typed, held to the same range as the setting.
pub fn check_delay(ms: u32) -> Result<u32, String> {
    if (MIN_DELAY_MS..=MAX_DELAY_MS).contains(&ms) {
        Ok(ms)
    } else {
        Err(format!("--delay must be between {MIN_DELAY_MS} and {MAX_DELAY_MS} ms"))
    }
}

/// The delay setting as stored, clamped into range (an edited settings file
/// can hold anything).
pub fn clamp_delay(ms: u32) -> u32 {
    ms.clamp(MIN_DELAY_MS, MAX_DELAY_MS)
}

/// A line may never start a chat command, and may not carry a line break
/// (which would end the IRC line early on the fallback path).
fn check_text(text: &str) -> Result<(), String> {
    if text.starts_with('/') || text.starts_with('.') {
        return Err("The message can't start with / or . (that would run a chat command)".into());
    }
    if text.contains(['\r', '\n', '\0']) {
        return Err("The message can't contain line breaks".into());
    }
    Ok(())
}

/// `count` copies of `text`.
pub fn plan_spam(count: u32, text: &str, vary: bool) -> Result<Vec<String>, String> {
    if count == 0 || count > MAX_SPAM_COUNT {
        return Err(format!("/spam sends 1 to {MAX_SPAM_COUNT} messages"));
    }
    check_text(text)?;
    let room = if vary && count > 1 { MAX_LINE_CHARS - VARY_RESERVE_CHARS } else { MAX_LINE_CHARS };
    let len = text.chars().count();
    if len > room {
        return Err(if room == MAX_LINE_CHARS {
            format!("That message is {len} characters; Twitch allows {MAX_LINE_CHARS}")
        } else {
            format!(
                "That message is {len} characters; Twitch allows {MAX_LINE_CHARS}, and \
                 \"Vary repeated messages\" needs {VARY_RESERVE_CHARS} of them, so {room} is the most here"
            )
        });
    }
    let lines = vec![text.to_string(); count as usize];
    Ok(if vary { vary_repeats(lines) } else { lines })
}

/// Rows of 1, 2, ... `height` ... 2, 1 copies of `word`, space separated:
/// `2 * height - 1` lines.
pub fn plan_pyramid(height: u32, word: &str, vary: bool) -> Result<Vec<String>, String> {
    if !(2..=MAX_PYRAMID_HEIGHT).contains(&height) {
        return Err(format!("/pyramid height goes from 2 to {MAX_PYRAMID_HEIGHT}"));
    }
    if word.contains(char::is_whitespace) {
        return Err("/pyramid takes one word or emote, like /pyramid 5 Kappa".into());
    }
    check_text(word)?;
    let w = word.chars().count();
    let room = if vary { MAX_LINE_CHARS - VARY_RESERVE_CHARS } else { MAX_LINE_CHARS };
    let widest = height as usize * w + (height as usize - 1);
    if widest > room {
        // Widest row: h words and h-1 spaces, so the tallest that fits is
        // (room + 1) / (w + 1).
        let tallest = (room + 1) / (w + 1);
        return Err(if tallest >= 2 {
            format!(
                "At height {height} the widest row is {widest} characters and Twitch allows {room}. \
                 The tallest that fits is {tallest}"
            )
        } else {
            format!("\"{word}\" is too long to stack: two of it is over {room} characters")
        });
    }
    let row = |n: u32| vec![word; n as usize].join(" ");
    let lines: Vec<String> = (1..=height).chain((1..height).rev()).map(row).collect();
    Ok(if vary { vary_repeats(lines) } else { lines })
}

/// Makes every line differ from the one sent before it, so Twitch's
/// "identical to your previous message" filter does not drop a repeat. A
/// line equal to the one before it alternates between as typed and with
/// [`VARY_SUFFIX`] (the space keeps a trailing emote its own word), so each
/// copy differs from the copy before it even after Twitch strips what it
/// strips. Lines that already differ are left exactly as typed.
pub fn vary_repeats(lines: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    for (i, line) in lines.iter().enumerate() {
        let repeat = i > 0 && lines[i - 1] == *line;
        let previous_varied = out.last().is_some_and(|p| p.ends_with(VARY_SUFFIX));
        if repeat && !previous_varied {
            out.push(format!("{line}{VARY_SUFFIX}"));
        } else {
            out.push(line.clone());
        }
    }
    out
}

/// How one line went.
#[derive(Debug, Clone, PartialEq)]
pub enum LineOutcome {
    /// Twitch confirmed the line (Helix `is_sent`). Only this resets the
    /// count of drops in a row.
    Sent,
    /// Twitch took the request and dropped the line, with its reason, or
    /// refused it for sending too fast.
    Dropped(String),
    /// The send itself failed (network, auth).
    Failed(String),
}

/// Reads a Helix send: `Ok((is_sent, drop_reason))`, or the HTTP status (when
/// Twitch answered at all) and the error text. HTTP 429 is Twitch saying
/// "too fast", so it counts as a dropped line, never as a reason to retry
/// some other way.
pub fn line_outcome(result: Result<(bool, Option<String>), (Option<u16>, String)>) -> LineOutcome {
    match result {
        Ok((true, _)) => LineOutcome::Sent,
        Ok((false, reason)) => LineOutcome::Dropped(reason.unwrap_or_else(|| "dropped by Twitch".to_string())),
        Err((Some(429), _)) => LineOutcome::Dropped("rate limited".to_string()),
        Err((_, e)) => LineOutcome::Failed(e),
    }
}

/// One run as the UI shows it.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct BurstStatus {
    pub channel: String,
    pub kind: BurstKind,
    pub sent: usize,
    pub total: usize,
    /// "running", "done", "stopped" (by the viewer) or "failed" (ended itself).
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub delay_ms: u32,
    /// The line the page shows for this state ([`BurstStatus::label`]).
    pub label: String,
}

impl BurstStatus {
    /// "Pyramid 5/9", "Spam done, 20/20", or why a run ended itself: the
    /// progress line, and on a phone (which has no progress bar) the chat
    /// line saying how the run ended.
    pub fn label(&self) -> String {
        let noun = match self.kind {
            BurstKind::Spam => "Spam",
            BurstKind::Pyramid => "Pyramid",
        };
        let count = format!("{}/{}", self.sent, self.total);
        match self.state {
            "running" => format!("{noun} {count}"),
            "done" => format!("{noun} done, {count}"),
            "stopped" => format!("{noun} stopped at {count}"),
            _ => self.reason.clone().unwrap_or_else(|| format!("{noun} stopped at {count}")),
        }
    }
}

/// A run in progress. Shared between the runner, the stop command and the IRC
/// notice hook.
pub struct Job {
    pub id: u64,
    pub channel: String,
    pub kind: BurstKind,
    pub total: usize,
    pub delay_ms: u32,
    sent: AtomicUsize,
    cancel: AtomicBool,
    drops_in_row: AtomicU32,
    /// Set when Twitch says the viewer cannot post here at all.
    forced_stop: Mutex<Option<String>>,
}

impl Job {
    pub fn new(channel: &str, kind: BurstKind, total: usize, delay_ms: u32) -> Arc<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Arc::new(Self {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            channel: channel.to_lowercase(),
            kind,
            total,
            delay_ms,
            sent: AtomicUsize::new(0),
            cancel: AtomicBool::new(false),
            drops_in_row: AtomicU32::new(0),
            forced_stop: Mutex::new(None),
        })
    }

    pub fn status(&self, state: &'static str, reason: Option<String>) -> BurstStatus {
        let mut status = BurstStatus {
            channel: self.channel.clone(),
            kind: self.kind,
            sent: self.sent.load(Ordering::SeqCst),
            total: self.total,
            state,
            reason,
            delay_ms: self.delay_ms,
            label: String::new(),
        };
        status.label = status.label();
        status
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    fn force_stop(&self, reason: String) {
        let mut slot = self.forced_stop.lock().unwrap_or_else(|e| e.into_inner());
        slot.get_or_insert(reason);
    }

    fn forced(&self) -> Option<String> {
        self.forced_stop.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// One more dropped line; the stop reason once too many came in a row.
    fn note_drop(&self, reason: &str) -> Option<String> {
        if is_cannot_post(reason) {
            return Some(format!("Stopped: Twitch says you can't post here ({reason})"));
        }
        let n = self.drops_in_row.fetch_add(1, Ordering::SeqCst) + 1;
        (n >= DROPS_IN_A_ROW_TO_STOP).then(|| {
            format!(
                "Stopped: Twitch is dropping your messages (slow mode / rate limit?). Last reason: {reason}"
            )
        })
    }
}

/// A drop that means no further line can land: timed out, banned, suspended.
fn is_cannot_post(reason: &str) -> bool {
    let r = reason.to_ascii_lowercase();
    ["banned", "timed out", "timedout", "timeout", "suspended"]
        .iter()
        .any(|k| r.contains(k))
}

/// Runs the lines through `send`, one at a time, `job.delay_ms` apart, until
/// they are all sent, the viewer stops it, or Twitch says no often enough.
/// `emit` sees every change (rate-limited to one "running" update per 100 ms,
/// the final state always). Returns the final status.
pub async fn run<S, F, E>(job: Arc<Job>, lines: Vec<String>, mut send: S, emit: E) -> BurstStatus
where
    S: FnMut(String) -> F,
    F: Future<Output = LineOutcome>,
    E: Fn(&BurstStatus),
{
    let delay = Duration::from_millis(job.delay_ms as u64);
    let mut last_start: Option<Instant> = None;
    let mut last_emit: Option<Instant> = None;
    let end = 'run: {
        for line in lines {
            // Wait out the gap since the previous line started, watching for
            // a stop the whole time.
            if let Some(prev) = last_start {
                let due = prev + delay;
                loop {
                    if let Some(end) = stop_check(&job) {
                        break 'run end;
                    }
                    let now = Instant::now();
                    if now >= due {
                        break;
                    }
                    tokio::time::sleep((due - now).min(Duration::from_millis(50))).await;
                }
            }
            if let Some(end) = stop_check(&job) {
                break 'run end;
            }
            last_start = Some(Instant::now());
            match send(line).await {
                LineOutcome::Sent => {
                    job.sent.fetch_add(1, Ordering::SeqCst);
                    job.drops_in_row.store(0, Ordering::SeqCst);
                }
                LineOutcome::Dropped(reason) | LineOutcome::Failed(reason) => {
                    if let Some(stop) = job.note_drop(&reason) {
                        break 'run job.status("failed", Some(stop));
                    }
                }
            }
            if last_emit.map_or(true, |t| t.elapsed() >= Duration::from_millis(100)) {
                emit(&job.status("running", None));
                last_emit = Some(Instant::now());
            }
        }
        // The last line may have been the third drop in a row via an IRC
        // notice that landed while it was in flight.
        stop_check(&job).unwrap_or_else(|| job.status("done", None))
    };
    emit(&end);
    end
}

fn stop_check(job: &Job) -> Option<BurstStatus> {
    if let Some(reason) = job.forced() {
        return Some(job.status("failed", Some(reason)));
    }
    if job.cancel.load(Ordering::SeqCst) {
        return Some(job.status("stopped", None));
    }
    None
}

// ---- the runs in flight, one per channel ----------------------------------

static JOBS: OnceLock<Mutex<HashMap<String, Arc<Job>>>> = OnceLock::new();

fn jobs() -> std::sync::MutexGuard<'static, HashMap<String, Arc<Job>>> {
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Registers a run, unless one is already going in that channel.
pub fn claim(job: &Arc<Job>) -> Result<(), String> {
    let mut jobs = jobs();
    if let Some(running) = jobs.get(&job.channel) {
        return Err(format!(
            "A {} is already running here ({}/{}). {} stop ends it",
            running.kind.noun().to_lowercase(),
            running.sent.load(Ordering::SeqCst),
            running.total,
            running.kind.command(),
        ));
    }
    jobs.insert(job.channel.clone(), job.clone());
    Ok(())
}

/// Forgets a finished run (only that run: a newer one in the same channel
/// stays).
pub fn release(job: &Job) {
    let mut jobs = jobs();
    if jobs.get(&job.channel).map(|j| j.id) == Some(job.id) {
        jobs.remove(&job.channel);
    }
}

/// The run going in `channel`, if any.
pub fn current(channel: &str) -> Option<Arc<Job>> {
    jobs().get(&channel.to_lowercase()).cloned()
}

/// Asks the run in `channel` to stop. Returns it, or None when nothing runs.
pub fn stop(channel: &str) -> Option<Arc<Job>> {
    let job = current(channel)?;
    job.cancel();
    Some(job)
}

/// Stops every run in every channel: the signed-in account changed, so the
/// lines left belong to an account that is no longer sending. Returns how
/// many runs were asked to stop.
pub fn stop_all() -> usize {
    let running: Vec<Arc<Job>> = jobs().values().cloned().collect();
    stop_for_account_change(&running)
}

fn stop_for_account_change(running: &[Arc<Job>]) -> usize {
    for job in running {
        job.force_stop("Stopped: you switched or signed out of the Twitch account it was sending as".to_string());
    }
    running.len()
}

/// A NOTICE from Twitch IRC for `channel`. While a run is going there, a
/// timeout or ban ends it and a refusal (slow mode, rate limit, duplicate,
/// room mode) counts as a dropped line. Covers lines that went out over IRC,
/// where the send itself cannot report a drop.
pub fn note_notice(channel: &str, msg_id: &str) {
    let Some(job) = current(channel) else {
        return;
    };
    match msg_id {
        "msg_banned" | "msg_timedout" | "msg_suspended" | "msg_channel_suspended"
        | "msg_requires_verified_phone_number" | "msg_verified_email" => {
            job.force_stop(format!("Stopped: Twitch says you can't post here ({msg_id})"));
        }
        "msg_ratelimit" | "msg_duplicate" | "msg_slowmode" | "msg_followersonly"
        | "msg_followersonly_zero" | "msg_followersonly_followed" | "msg_subsonly"
        | "msg_emoteonly" | "msg_r9k" | "msg_rejected" | "msg_rejected_mandatory" => {
            if let Some(stop) = job.note_drop(msg_id) {
                job.force_stop(stop);
            }
        }
        _ => {}
    }
}

/// The confirmation shown when a run starts.
pub fn started_message(job: &Job) -> String {
    format!(
        "{} started: {} {} at {} ms apart. {} stop ends it",
        job.kind.noun(),
        job.total,
        if job.total == 1 { "message" } else { "messages" },
        job.delay_ms,
        job.kind.command(),
    )
}

/// What the stop command says.
pub fn stopped_message(job: &Job) -> String {
    format!(
        "Stopping the {} at {}/{}",
        job.kind.noun().to_lowercase(),
        job.sent.load(Ordering::SeqCst),
        job.total
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    #[test]
    fn labels_read_like_the_progress_line() {
        let status = |state: &'static str, sent: usize, reason: Option<&str>| BurstStatus {
            channel: "chan".into(),
            kind: BurstKind::Pyramid,
            sent,
            total: 9,
            state,
            reason: reason.map(str::to_string),
            delay_ms: 1000,
            label: String::new(),
        };
        assert_eq!(status("running", 5, None).label(), "Pyramid 5/9");
        assert_eq!(status("done", 9, None).label(), "Pyramid done, 9/9");
        assert_eq!(status("stopped", 5, None).label(), "Pyramid stopped at 5/9");
        assert_eq!(status("failed", 5, Some("Stopped: rate limited")).label(), "Stopped: rate limited");
        assert_eq!(status("failed", 5, None).label(), "Pyramid stopped at 5/9");
    }

    fn start(kind: BurstKind, raw: &str) -> (u32, String, Option<u32>) {
        match parse_args(kind, raw).expect("parses") {
            BurstArgs::Start { amount, text, delay_ms } => (amount, text, delay_ms),
            BurstArgs::Stop => panic!("expected a start"),
        }
    }

    #[test]
    fn parses_count_message_and_stop() {
        assert_eq!(start(BurstKind::Spam, "20 hi there"), (20, "hi there".into(), None));
        assert_eq!(start(BurstKind::Spam, "  3   two  spaces kept "), (3, "two  spaces kept".into(), None));
        assert_eq!(parse_args(BurstKind::Spam, "stop"), Ok(BurstArgs::Stop));
        assert_eq!(parse_args(BurstKind::Pyramid, " STOP "), Ok(BurstArgs::Stop));
        assert!(parse_args(BurstKind::Spam, "").is_err());
        assert!(parse_args(BurstKind::Spam, "20").is_err());
        assert!(parse_args(BurstKind::Spam, "lots hi").unwrap_err().contains("not a number"));
        assert!(parse_args(BurstKind::Spam, "-1 hi").is_err());
    }

    #[test]
    fn delay_override_is_a_trailing_flag_in_milliseconds() {
        assert_eq!(start(BurstKind::Spam, "20 hi --delay 200"), (20, "hi".into(), Some(200)));
        assert_eq!(start(BurstKind::Pyramid, "5 Kappa --delay 250ms"), (5, "Kappa".into(), Some(250)));
        // Out of range, or not a number, is refused rather than guessed.
        assert!(parse_args(BurstKind::Spam, "2 hi --delay 5").unwrap_err().contains("between"));
        assert!(parse_args(BurstKind::Spam, "2 hi --delay 6000").is_err());
        assert!(parse_args(BurstKind::Spam, "2 hi --delay fast").unwrap_err().contains("milliseconds"));
        // Not at the end, or not a whole word: part of the message.
        assert_eq!(start(BurstKind::Spam, "2 use --delay 5 then go"), (2, "use --delay 5 then go".into(), None));
        assert_eq!(start(BurstKind::Spam, "2 hi--delay 200"), (2, "hi--delay 200".into(), None));
        assert_eq!(start(BurstKind::Spam, "2 hi --delayed 200"), (2, "hi --delayed 200".into(), None));
    }

    #[test]
    fn settings_delay_is_clamped() {
        assert_eq!(clamp_delay(0), MIN_DELAY_MS);
        assert_eq!(clamp_delay(99_999), MAX_DELAY_MS);
        assert_eq!(clamp_delay(1000), 1000);
    }

    #[test]
    fn pyramid_rows_go_up_and_back_down() {
        let rows = plan_pyramid(3, "Kappa", false).unwrap();
        assert_eq!(rows, vec!["Kappa", "Kappa Kappa", "Kappa Kappa Kappa", "Kappa Kappa", "Kappa"]);
        assert_eq!(plan_pyramid(50, "x", false).unwrap().len(), 99);
        // Consecutive rows always differ, so varying changes nothing.
        assert_eq!(plan_pyramid(4, "LUL", true).unwrap(), plan_pyramid(4, "LUL", false).unwrap());
    }

    #[test]
    fn pyramid_respects_the_500_character_line() {
        // 5-letter word: h rows of 5 + (h-1) spaces. h=83 is 497, h=84 is 503.
        let ok = plan_pyramid(83, "Kappa", false).unwrap();
        assert_eq!(ok.iter().map(|r| r.chars().count()).max(), Some(497));
        let err = plan_pyramid(84, "Kappa", false).unwrap_err();
        assert!(err.contains("503") && err.contains("tallest that fits is 83"), "{err}");
        // Varying keeps four characters free: 496, so 82 is the tallest.
        assert!(plan_pyramid(83, "Kappa", true).unwrap_err().contains("tallest that fits is 82"));
        // A word too long to stack at all.
        let long = "a".repeat(300);
        assert!(plan_pyramid(2, &long, false).unwrap_err().contains("too long to stack"));
    }

    #[test]
    fn caps_and_command_guard() {
        assert!(plan_spam(0, "hi", false).is_err());
        assert!(plan_spam(MAX_SPAM_COUNT + 1, "hi", false).is_err());
        assert_eq!(plan_spam(MAX_SPAM_COUNT, "hi", false).unwrap().len(), 1000);
        assert!(plan_pyramid(1, "hi", false).is_err());
        assert!(plan_pyramid(MAX_PYRAMID_HEIGHT + 1, "a", false).is_err());
        assert!(plan_pyramid(MAX_PYRAMID_HEIGHT, "a", false).is_ok());
        for bad in ["/ban someone", ".timeout someone", "a\nb"] {
            assert!(plan_spam(2, bad, false).is_err(), "{bad}");
        }
        assert!(plan_pyramid(3, "/me", false).is_err());
        assert!(plan_pyramid(3, "two words", false).is_err());
        // 500 exactly is fine; with varying, 496 is the most.
        assert!(plan_spam(2, &"a".repeat(500), false).is_ok());
        assert!(plan_spam(2, &"a".repeat(501), false).is_err());
        assert!(plan_spam(2, &"a".repeat(497), true).unwrap_err().contains("496"));
        assert!(plan_spam(2, &"a".repeat(496), true).is_ok());
        // A single message is never varied, so it gets the full 500.
        assert!(plan_spam(1, &"a".repeat(500), true).is_ok());
    }

    #[test]
    fn vary_makes_consecutive_repeats_differ_and_can_be_off() {
        let plain = plan_spam(4, "Kappa", false).unwrap();
        assert!(plain.iter().all(|l| l == "Kappa"));

        let varied = plan_spam(200, "Kappa", true).unwrap();
        assert_eq!(varied[0], "Kappa", "the first line is sent as typed");
        // Every other copy carries a space and U+E0000; the emote stays its
        // own word.
        assert_eq!(varied[1], "Kappa \u{E0000}");
        assert_eq!(varied[2], "Kappa");
        assert_eq!(varied[3], "Kappa \u{E0000}");
        for line in &varied {
            assert!(line.chars().count() <= 5 + VARY_RESERVE_CHARS);
            assert!(line.encode_utf16().count() <= 5 + VARY_RESERVE_CHARS);
        }
    }

    /// What Twitch does to chat text, as seen live: U+E0000 survives, the tag
    /// characters U+E0020..U+E007E are removed.
    fn as_twitch_keeps_it(line: &str) -> String {
        line.chars().filter(|c| !(0xE0020..=0xE007E).contains(&(*c as u32))).collect()
    }

    #[test]
    fn varied_repeats_still_differ_after_twitch_strips_tag_characters() {
        let varied = plan_spam(50, "sn spam", true).unwrap();
        for pair in varied.windows(2) {
            assert_ne!(as_twitch_keeps_it(&pair[0]), as_twitch_keeps_it(&pair[1]), "{pair:?}");
        }
        // Nothing but U+E0000 is ever added.
        for line in &varied {
            assert!(line.chars().all(|c| c == '\u{E0000}' || !(0xE0000..=0xE007F).contains(&(c as u32))));
            assert!(!line.contains('\u{034F}'));
        }
        // A run of repeats inside other text alternates from its own start.
        let mixed = vary_repeats(vec!["a".into(), "b".into(), "b".into(), "b".into(), "c".into()]);
        assert_eq!(mixed, vec!["a", "b", "b \u{E0000}", "b", "c"]);
    }

    fn lines(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("line {i}")).collect()
    }

    async fn run_with(job: Arc<Job>, n: usize, outcomes: Vec<LineOutcome>) -> (BurstStatus, Vec<String>) {
        let sent = Arc::new(StdMutex::new(Vec::new()));
        let seen = sent.clone();
        let mut queue = outcomes.into_iter();
        let end = run(
            job,
            lines(n),
            move |line| {
                seen.lock().unwrap().push(line);
                let outcome = queue.next().unwrap_or(LineOutcome::Sent);
                async move { outcome }
            },
            |_| {},
        )
        .await;
        let sent = sent.lock().unwrap().clone();
        (end, sent)
    }

    #[tokio::test]
    async fn sends_everything_at_the_chosen_pace() {
        let job = Job::new("Chan", BurstKind::Spam, 4, 20);
        let t0 = Instant::now();
        let (end, sent) = run_with(job, 4, vec![]).await;
        assert_eq!(end.state, "done");
        assert_eq!((end.sent, end.total), (4, 4));
        assert_eq!(sent.len(), 4);
        assert!(t0.elapsed() >= Duration::from_millis(60), "three 20 ms gaps");
    }

    #[tokio::test]
    async fn stops_after_three_drops_in_a_row_but_not_scattered_ones() {
        let d = || LineOutcome::Dropped("msg_duplicate".into());
        let (end, sent) =
            run_with(Job::new("c", BurstKind::Spam, 10, 10), 10, vec![LineOutcome::Sent, d(), d(), d()]).await;
        assert_eq!(end.state, "failed");
        assert_eq!(sent.len(), 4, "nothing goes out after the third drop");
        assert!(end.reason.unwrap().contains("dropping your messages"));

        let (end, sent) = run_with(
            Job::new("c", BurstKind::Spam, 6, 10),
            6,
            vec![d(), d(), LineOutcome::Sent, d(), LineOutcome::Failed("network".into()), LineOutcome::Sent],
        )
        .await;
        assert_eq!(end.state, "done");
        assert_eq!((sent.len(), end.sent), (6, 2));
    }

    #[tokio::test]
    async fn a_timeout_or_ban_stops_at_once() {
        let (end, sent) = run_with(
            Job::new("c", BurstKind::Pyramid, 9, 10),
            9,
            vec![LineOutcome::Sent, LineOutcome::Dropped("You are timed out for 600 more seconds.".into())],
        )
        .await;
        assert_eq!(end.state, "failed");
        assert_eq!(sent.len(), 2);
        assert!(end.reason.unwrap().contains("can't post here"));
    }

    #[tokio::test]
    async fn an_irc_notice_ends_a_run_and_stop_cancels_one() {
        // A ban notice while the run is waiting between lines.
        let job = Job::new("noticechan", BurstKind::Spam, 50, 30);
        claim(&job).unwrap();
        let runner = tokio::spawn(run(job.clone(), lines(50), |_| async { LineOutcome::Sent }, |_| {}));
        tokio::time::sleep(Duration::from_millis(70)).await;
        note_notice("NoticeChan", "msg_timedout");
        let end = runner.await.unwrap();
        release(&job);
        assert_eq!(end.state, "failed");
        assert!(end.sent < 50);

        // Three refusal notices in a row end it as well.
        let job = Job::new("noticechan2", BurstKind::Spam, 50, 30);
        claim(&job).unwrap();
        let runner = tokio::spawn(run(job.clone(), lines(50), |_| async { LineOutcome::Sent }, |_| {}));
        tokio::time::sleep(Duration::from_millis(20)).await;
        for _ in 0..3 {
            note_notice("noticechan2", "msg_ratelimit");
        }
        let end = runner.await.unwrap();
        release(&job);
        assert_eq!(end.state, "failed");
        assert!(end.reason.unwrap().contains("dropping your messages"));

        // The viewer's stop.
        let job = Job::new("stopchan", BurstKind::Pyramid, 50, 30);
        claim(&job).unwrap();
        let runner = tokio::spawn(run(job.clone(), lines(50), |_| async { LineOutcome::Sent }, |_| {}));
        tokio::time::sleep(Duration::from_millis(70)).await;
        assert!(stop("StopChan").is_some());
        let end = runner.await.unwrap();
        release(&job);
        assert_eq!(end.state, "stopped");
        assert!(end.sent >= 1 && end.sent < 50);
        assert!(stop("stopchan").is_none(), "released");
    }

    #[test]
    fn helix_answers_map_to_line_outcomes() {
        assert_eq!(line_outcome(Ok((true, None))), LineOutcome::Sent);
        assert_eq!(
            line_outcome(Ok((false, Some("msg_duplicate".into())))),
            LineOutcome::Dropped("msg_duplicate".into())
        );
        assert_eq!(line_outcome(Ok((false, None))), LineOutcome::Dropped("dropped by Twitch".into()));
        // Too fast is a dropped line, so three in a row end the run.
        assert_eq!(
            line_outcome(Err((Some(429), "Helix send returned HTTP 429".into()))),
            LineOutcome::Dropped("rate limited".into())
        );
        assert_eq!(line_outcome(Err((Some(401), "auth".into()))), LineOutcome::Failed("auth".into()));
        assert_eq!(line_outcome(Err((None, "offline".into()))), LineOutcome::Failed("offline".into()));
    }

    #[tokio::test]
    async fn rate_limit_answers_end_a_run() {
        let job = Job::new("ratechan", BurstKind::Spam, 20, 10);
        let (end, sent) = run_with(
            job,
            20,
            (0..20).map(|_| line_outcome(Err((Some(429), "HTTP 429".into())))).collect(),
        )
        .await;
        assert_eq!(end.state, "failed");
        assert_eq!(sent.len(), DROPS_IN_A_ROW_TO_STOP as usize);
    }

    #[tokio::test]
    async fn an_account_change_stops_a_run_mid_way() {
        // Not registered in JOBS, so other tests' runs are never touched.
        let job = Job::new("acctchan", BurstKind::Spam, 50, 20);
        let runner = tokio::spawn(run(job.clone(), lines(50), |_| async { LineOutcome::Sent }, |_| {}));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(stop_for_account_change(&[job.clone()]), 1);
        let end = runner.await.unwrap();
        assert_eq!(end.state, "failed");
        assert!(end.reason.unwrap().contains("switched or signed out"));
        assert!(end.sent < 50);
    }

    #[test]
    fn one_run_per_channel() {
        let a = Job::new("OneChan", BurstKind::Spam, 5, 1000);
        let b = Job::new("onechan", BurstKind::Pyramid, 5, 1000);
        claim(&a).unwrap();
        let err = claim(&b).unwrap_err();
        assert!(err.contains("spam is already running") && err.contains("/spam stop"), "{err}");
        // Releasing a stale run never frees a newer one.
        release(&b);
        assert!(current("onechan").is_some());
        release(&a);
        assert!(current("onechan").is_none());
        claim(&b).unwrap();
        release(&b);
    }
}
