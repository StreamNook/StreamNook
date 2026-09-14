//! Chat logging to plain text files: one folder per channel, one file per
//! day. Driven by the IRC service at the points where messages are parsed,
//! so it sees exactly what chat sees (plus the user's own sent lines, which
//! Twitch IRC never echoes back). Reads the live settings on every message,
//! so toggling or retargeting the folder takes effect immediately.
//!
//! File layout: `<base>\<channel>\YYYY-MM-DD.log`, where base is the custom
//! folder from settings or `ChatLogs` under the app data dir. Each file gets
//! a "# Logging started ..." line when opened (session start or day rollover).

use crate::models::chat_layout::ChatMessage;
use crate::models::settings::{ChatLoggingSettings, Settings};
use chrono::Local;
use log::warn;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

static SETTINGS: OnceLock<Arc<Mutex<Settings>>> = OnceLock::new();
static STATE: OnceLock<Mutex<LoggerState>> = OnceLock::new();

struct OpenLog {
    date: String,
    file: std::io::BufWriter<File>,
    last_flush: std::time::Instant,
}

#[derive(Default)]
struct LoggerState {
    /// Base folder the open handles were created under; a settings change
    /// that moves the base drops them so files reopen at the new location.
    base: PathBuf,
    open: HashMap<String, OpenLog>,
}

fn state() -> &'static Mutex<LoggerState> {
    STATE.get_or_init(|| Mutex::new(LoggerState::default()))
}

pub struct ChatLoggerService;

impl ChatLoggerService {
    /// Hands the service the live settings handle. Called whenever chat
    /// starts; only the first call wires it.
    pub fn init(settings: Arc<Mutex<Settings>>) {
        let _ = SETTINGS.set(settings);
    }

    fn config() -> Option<ChatLoggingSettings> {
        let settings = SETTINGS.get()?;
        let guard = settings.lock().ok()?;
        Some(guard.chat_logging.clone())
    }

    /// Enabled probe without the full config clone (channel filter Vec and
    /// all): log_message runs per chat message and logging is off by default.
    fn logging_enabled() -> bool {
        SETTINGS
            .get()
            .and_then(|s| s.lock().ok().map(|g| g.chat_logging.enabled))
            .unwrap_or(false)
    }

    pub fn default_dir() -> Option<PathBuf> {
        crate::services::cache_service::get_app_data_dir()
            .ok()
            .map(|d| d.join("ChatLogs"))
    }

    /// The folder a given settings value writes to: the custom folder when
    /// set, else the default. Also used by the settings UI to show the path.
    pub fn resolve_dir(folder: &str) -> Option<PathBuf> {
        if folder.trim().is_empty() {
            Self::default_dir()
        } else {
            Some(PathBuf::from(folder.trim()))
        }
    }

    fn passes_filter(cfg: &ChatLoggingSettings, channel: &str) -> bool {
        // A provider channel arrives as the composite key ("kick:xqc") while the
        // allowlist holds bare logins, so match on the channel part. Naming a
        // channel therefore logs it on every platform you watch it, which is what
        // "log this channel" is understood to mean.
        let bare = crate::services::providers::key::parse_key(channel).channel;
        cfg.channels.is_empty()
            || cfg
                .channels
                .iter()
                .any(|c| {
                    c.channel_login.eq_ignore_ascii_case(channel)
                        || c.channel_login.eq_ignore_ascii_case(&bare)
                })
    }

    /// `[HH:MM:SS] ` from an epoch-milliseconds string (the ChatMessage
    /// timestamp format), in local time; falls back to now.
    fn prefix_from_epoch_ms(cfg: &ChatLoggingSettings, epoch_ms: &str) -> String {
        if !cfg.timestamps {
            return String::new();
        }
        // Twitch stamps epoch milliseconds; Kick and YouTube stamp ISO-8601. Try
        // both before falling back to now, or every provider line would be logged
        // with its WRITE time instead of its send time.
        let local = epoch_ms
            .parse::<i64>()
            .ok()
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|t| t.with_timezone(&Local))
            .or_else(|| {
                chrono::DateTime::parse_from_rfc3339(epoch_ms)
                    .ok()
                    .map(|t| t.with_timezone(&Local))
            })
            .unwrap_or_else(Local::now);
        format!("[{}] ", local.format("%H:%M:%S"))
    }

    fn prefix_now(cfg: &ChatLoggingSettings) -> String {
        if !cfg.timestamps {
            return String::new();
        }
        format!("[{}] ", Local::now().format("%H:%M:%S"))
    }

    /// Logs a parsed incoming message (PRIVMSG or USERNOTICE). Event notices
    /// write their readable system line; an attached user message (e.g. a
    /// resub message) gets its own normal line after it.
    pub fn log_message(msg: &ChatMessage) {
        if !Self::logging_enabled() {
            Self::release_handles();
            return;
        }
        let Some(cfg) = Self::config() else { return };
        if !cfg.enabled {
            Self::release_handles();
            return;
        }
        let channel = msg.channel.to_lowercase();
        if channel.is_empty() || !Self::passes_filter(&cfg, &channel) {
            return;
        }
        let is_event = msg.metadata.msg_type.is_some();
        if is_event && !cfg.include_events {
            return;
        }

        let prefix = Self::prefix_from_epoch_ms(&cfg, &msg.timestamp);
        let display = if msg.display_name.is_empty() {
            &msg.username
        } else {
            &msg.display_name
        };

        let mut lines: Vec<String> = Vec::with_capacity(2);
        if let Some(system) = msg.metadata.system_message.as_deref() {
            if !system.is_empty() {
                lines.push(format!("{prefix}{system}"));
            }
        }
        if !msg.content.is_empty() {
            if msg.metadata.is_action {
                lines.push(format!("{prefix}* {display} {}", msg.content));
            } else {
                lines.push(format!("{prefix}{display}: {}", msg.content));
            }
        }
        if !lines.is_empty() {
            Self::write(&cfg, &channel, &lines);
        }
    }

    /// Logs a message the user sent from the app (no IRC echo exists for it).
    pub fn log_own_message(channel: &str, login: &str, text: &str) {
        let Some(cfg) = Self::config() else { return };
        let channel = channel.to_lowercase();
        if !cfg.enabled || !Self::passes_filter(&cfg, &channel) {
            return;
        }
        let prefix = Self::prefix_now(&cfg);
        let line = match text.strip_prefix("/me ") {
            Some(rest) => format!("{prefix}* {login} {rest}"),
            None => format!("{prefix}{login}: {text}"),
        };
        Self::write(&cfg, &channel, &[line]);
    }

    pub fn log_timeout(channel: &str, user: &str, duration_secs: Option<u64>) {
        let line = match duration_secs {
            Some(secs) => format!("{user} has been timed out for {}", human_duration(secs)),
            None => format!("{user} has been banned"),
        };
        Self::log_event_line(channel, &line);
    }

    pub fn log_chat_cleared(channel: &str) {
        Self::log_event_line(channel, "chat was cleared");
    }

    pub fn log_deleted_message(channel: &str, login: &str, text: Option<&str>) {
        let who = if login.is_empty() { "someone" } else { login };
        let line = match text {
            Some(t) if !t.is_empty() => format!("a message from {who} was deleted: {t}"),
            _ => format!("a message from {who} was deleted"),
        };
        Self::log_event_line(channel, &line);
    }

    /// A standalone event line (moderation actions), gated like other events.
    /// Honest gap marker for a saturated side-effect lane: the log must never
    /// silently omit messages, so a drop burst leaves one line saying how many.
    pub fn log_dropped_marker(channel: &str, count: u64) {
        if count == 0 {
            return;
        }
        Self::log_event_line(
            channel,
            &format!("# {count} messages not logged (side-effect lane saturated)"),
        );
    }

    fn log_event_line(channel: &str, text: &str) {
        let Some(cfg) = Self::config() else { return };
        let channel = channel.to_lowercase();
        if !cfg.enabled || !cfg.include_events || !Self::passes_filter(&cfg, &channel) {
            return;
        }
        let line = format!("{}{}", Self::prefix_now(&cfg), text);
        Self::write(&cfg, &channel, &[line]);
    }

    /// Closes every open file. Called lazily when logging is found disabled
    /// so handles don't linger on files the user turned off.
    fn release_handles() {
        if let Ok(mut st) = state().lock() {
            for entry in st.open.values_mut() {
                let _ = entry.file.flush();
            }
            st.open.clear();
        }
    }

    /// Flush every buffered writer without closing it. Wired into both exit
    /// paths so the buffering below can never cost lines on quit.
    pub fn flush_all() {
        if let Ok(mut st) = state().lock() {
            for entry in st.open.values_mut() {
                let _ = entry.file.flush();
                entry.last_flush = std::time::Instant::now();
            }
        }
    }

    fn write(cfg: &ChatLoggingSettings, channel: &str, lines: &[String]) {
        let Some(base) = Self::resolve_dir(&cfg.folder) else {
            return;
        };
        let Ok(mut st) = state().lock() else { return };
        if st.base != base {
            st.open.clear();
            st.base = base.clone();
        }
        let date = Local::now().format("%Y-%m-%d").to_string();
        let stale = st.open.get(channel).is_none_or(|e| e.date != date);
        if stale {
            match Self::open_file(&base, channel, &date) {
                Ok(file) => {
                    st.open.insert(
                        channel.to_string(),
                        OpenLog {
                            date,
                            file,
                            last_flush: std::time::Instant::now(),
                        },
                    );
                }
                Err(e) => {
                    warn!("[ChatLogger] could not open a log file for {channel}: {e}");
                    return;
                }
            }
        }
        let entry = st.open.get_mut(channel).expect("opened above");
        // Buffered: one syscall per ~8KB or per 2s instead of a write+flush
        // syscall pair per line. Crash exposure is <=2s of a channel's lines;
        // exit paths call flush_all.
        let mut result = lines
            .iter()
            .try_for_each(|line| writeln!(entry.file, "{line}"));
        if result.is_ok() && entry.last_flush.elapsed() >= std::time::Duration::from_secs(2) {
            result = entry.file.flush();
            entry.last_flush = std::time::Instant::now();
        }
        if let Err(e) = result {
            // Drop the handle so the next message retries with a fresh open.
            warn!("[ChatLogger] write failed for {channel}: {e}");
            st.open.remove(channel);
        }
    }

    fn open_file(
        base: &PathBuf,
        channel: &str,
        date: &str,
    ) -> std::io::Result<std::io::BufWriter<File>> {
        let dir = base.join(safe_dir_name(channel));
        fs::create_dir_all(&dir)?;
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join(format!("{date}.log")))?;
        let mut file = std::io::BufWriter::new(file);
        writeln!(
            file,
            "# Logging started {}",
            Local::now().format("%Y-%m-%d %H:%M:%S")
        )?;
        Ok(file)
    }
}

/// Channel logins are already filesystem-safe ([a-z0-9_]), but sanitize
/// defensively since the name becomes a folder.
fn safe_dir_name(channel: &str) -> String {
    channel
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Readable timeout durations: 600 -> "10m", 90 -> "1m 30s", 7200 -> "2h".
fn human_duration(secs: u64) -> String {
    if secs >= 3600 && secs.is_multiple_of(3600) {
        format!("{}h", secs / 3600)
    } else if secs >= 60 {
        if secs.is_multiple_of(60) {
            format!("{}m", secs / 60)
        } else {
            format!("{}m {}s", secs / 60, secs % 60)
        }
    } else {
        format!("{secs}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_naturally() {
        assert_eq!(human_duration(45), "45s");
        assert_eq!(human_duration(60), "1m");
        assert_eq!(human_duration(90), "1m 30s");
        assert_eq!(human_duration(600), "10m");
        assert_eq!(human_duration(7200), "2h");
        assert_eq!(human_duration(5400), "90m");
    }

    #[test]
    fn channel_names_become_safe_folders() {
        assert_eq!(safe_dir_name("somechannel"), "somechannel");
        assert_eq!(safe_dir_name("a/b\\c:d"), "a_b_c_d");
    }
}
