//! The deadline heap for Timer, Clock, Uptime and Delay: a pure step machine
//! fed injected steady and wall times and an injected time zone. It decides
//! what is due; `timed.rs` turns a due entry into a job.
//!
//! Phase is kept by (rule id, chat key) across compiles, so an edit does not
//! re-arm. A Timer arms at its first sight (a chat's `only_live` gate is the
//! caller's) and never fires on arming; each chat's offset is a hash of its
//! key mod the interval, so a rule in many chats spreads its posts. A skipped
//! entry (offline, held, claim lost) moves to its next deadline. A wall-clock
//! step back never re-fires a stamped deadline; a resume re-arms Timers.

use std::collections::HashMap;
use std::time::Duration;

use crate::models::automations::Weekday;
use crate::services::account_automations::counts::TimedFired;

/// The window after a deadline a Clock, Uptime or Delay may still fire in.
pub const ALLOWANCE_MS: i64 = 120_000;
/// A queued job's grace beyond its rule's `max_wait` (S7.3a `CHAT_ALLOWANCE`).
pub const QUEUE_GRACE_S: u64 = 5;

/// "15 minutes", "2 hours", "90 seconds" for summaries and Activity.
pub fn every_words(seconds: u32) -> String {
    let (n, unit) = if seconds % 3600 == 0 && seconds >= 3600 {
        (seconds / 3600, "hour")
    } else if seconds % 60 == 0 && seconds >= 60 {
        (seconds / 60, "minute")
    } else {
        (seconds, "second")
    };
    format!("{n} {unit}{}", if n == 1 { "" } else { "s" })
}

/// What kind of timed trigger a spec is, with the numbers it needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpecKind {
    Timer {
        every_s: u32,
    },
    /// `at_min`: minutes past local midnight; `days` empty = every day.
    Clock {
        at_min: u32,
        days: Vec<Weekday>,
    },
    /// `started_ms`: the stream's `started_at` (Twitch time, epoch ms), None
    /// while offline or unknown.
    Uptime {
        seconds: u32,
        started_ms: Option<i64>,
        late_ok: bool,
    },
    /// `joined_steady_ms`: when a window joined the chat (the engine's steady clock);
    /// `joined_id`: that join's wall ms, the stamp `fired_for` keeps (a new join is a new id).
    Delay {
        seconds: u32,
        joined_steady_ms: Option<i64>,
        joined_id: Option<i64>,
    },
}

/// One (rule, chat) the machine watches this tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spec {
    pub rule_id: String,
    pub chat_key: String,
    pub kind: SpecKind,
    /// What it last fired for (from `Counts`).
    pub fired: TimedFired,
}

/// A local wall-clock reading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalPoint {
    /// `YYYY-MM-DD`.
    pub date: String,
    /// Minutes past local midnight.
    pub minute: u32,
    /// Milliseconds past local midnight (a Clock's lateness is measured in ms, D283).
    pub ms_of_day: i64,
    pub weekday: Weekday,
}

/// The time zone, injected so DST is testable.
pub trait Zone {
    fn local_of(&self, wall_ms: i64) -> LocalPoint;
    /// The first local minute at or after `minute` on `date` that exists (a
    /// missing DST minute maps to the next valid one).
    fn next_valid_minute(&self, date: &str, minute: u32) -> u32;
}

/// The device's zone.
pub struct SystemZone;

impl Zone for SystemZone {
    fn local_of(&self, wall_ms: i64) -> LocalPoint {
        use chrono::{Datelike, TimeZone, Timelike};
        let dt = chrono::Local
            .timestamp_millis_opt(wall_ms)
            .single()
            .unwrap_or_else(chrono::Local::now);
        LocalPoint {
            date: dt.format("%Y-%m-%d").to_string(),
            minute: dt.hour() * 60 + dt.minute(),
            ms_of_day: i64::from(dt.num_seconds_from_midnight()) * 1000
                + i64::from(dt.timestamp_subsec_millis()),
            weekday: match dt.weekday() {
                chrono::Weekday::Mon => Weekday::Mon,
                chrono::Weekday::Tue => Weekday::Tue,
                chrono::Weekday::Wed => Weekday::Wed,
                chrono::Weekday::Thu => Weekday::Thu,
                chrono::Weekday::Fri => Weekday::Fri,
                chrono::Weekday::Sat => Weekday::Sat,
                chrono::Weekday::Sun => Weekday::Sun,
            },
        }
    }

    fn next_valid_minute(&self, date: &str, minute: u32) -> u32 {
        use chrono::{NaiveDate, TimeZone};
        let Ok(day) = NaiveDate::parse_from_str(date, "%Y-%m-%d") else {
            return minute;
        };
        for m in minute..1440 {
            let naive = day.and_hms_opt(m / 60, m % 60, 0).unwrap_or_default();
            if chrono::Local
                .from_local_datetime(&naive)
                .earliest()
                .is_some()
            {
                return m;
            }
        }
        minute
    }
}

/// Why an entry fired (or, with `missed`, why it will not).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fire {
    /// Index into the specs of this tick.
    pub spec: usize,
    /// How long after its deadline the tick found it, in ms.
    pub late_ms: i64,
    /// Past its allowance: stamp it, run nothing.
    pub missed: bool,
    /// What to store once the fire is handled (or missed).
    pub stamp: TimedFired,
    /// The local `YYYY-MM-DD HH:MM` of a Clock fire, for its claim key.
    pub local: Option<(String, String)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Entry {
    /// Steady ms of a Timer's next deadline.
    next_ms: i64,
}

/// The machine's carried phase.
#[derive(Default)]
pub struct Machine {
    entries: HashMap<(String, String), Entry>,
}

pub(crate) fn offset_ms(chat_key: &str, every_s: u32) -> i64 {
    let hash = chat_key
        .bytes()
        .fold(14_695_981_039_346_656_037u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(1_099_511_628_211)
        });
    ((hash % u64::from(every_s.max(1))) * 1000) as i64
}

impl Machine {
    /// A wall-clock gap or the presence resume: Timers re-arm from now.
    pub fn resume(&mut self) {
        self.entries.clear();
    }

    /// A rule's phase in one chat is dropped (an offline chat of an only-live
    /// Timer): it arms again at go-live and never fires on arming.
    pub fn forget(&mut self, rule_id: &str, chat_key: &str) {
        self.entries
            .remove(&(rule_id.to_string(), chat_key.to_string()));
    }

    /// Forget entries no spec names (a deleted rule, a chat no longer listened in).
    pub fn retain(&mut self, specs: &[Spec]) {
        self.entries
            .retain(|k, _| specs.iter().any(|s| s.rule_id == k.0 && s.chat_key == k.1));
    }

    /// A Timer fired, was skipped or lost its claim: its next deadline is a
    /// full interval on.
    pub fn moved(&mut self, rule_id: &str, chat_key: &str, steady_ms: i64, every_s: u32) {
        self.entries.insert(
            (rule_id.to_string(), chat_key.to_string()),
            Entry {
                next_ms: steady_ms + i64::from(every_s) * 1000,
            },
        );
    }

    /// One tick: what is due. `steady_ms` is a monotonic clock, `wall_ms` the
    /// device's wall clock, `twitch_now_ms` Twitch's time (for Uptime).
    pub fn tick(
        &mut self,
        steady_ms: i64,
        wall_ms: i64,
        twitch_now_ms: i64,
        specs: &[Spec],
        zone: &dyn Zone,
    ) -> Vec<Fire> {
        let local = zone.local_of(wall_ms);
        let mut out = Vec::new();
        for (index, spec) in specs.iter().enumerate() {
            match &spec.kind {
                SpecKind::Timer { every_s } => {
                    let key = (spec.rule_id.clone(), spec.chat_key.clone());
                    let every_ms = i64::from(*every_s) * 1000;
                    let entry = self.entries.entry(key).or_insert_with(|| Entry {
                        next_ms: steady_ms + offset_ms(&spec.chat_key, *every_s).max(1),
                    });
                    if steady_ms < entry.next_ms {
                        continue;
                    }
                    // A wall clock that stepped back never re-fires a stamped deadline.
                    if spec.fired.last_fire_ms > 0
                        && wall_ms - spec.fired.last_fire_ms < every_ms - 1000
                        && wall_ms >= spec.fired.last_fire_ms
                    {
                        entry.next_ms =
                            steady_ms + (every_ms - (wall_ms - spec.fired.last_fire_ms)).max(1000);
                        continue;
                    }
                    if wall_ms < spec.fired.last_fire_ms {
                        entry.next_ms = steady_ms + every_ms;
                        continue;
                    }
                    let late_ms = steady_ms - entry.next_ms;
                    entry.next_ms = steady_ms + every_ms;
                    out.push(Fire {
                        spec: index,
                        late_ms,
                        missed: false,
                        stamp: TimedFired {
                            last_fire_ms: wall_ms,
                            ..spec.fired.clone()
                        },
                        local: None,
                    });
                }
                SpecKind::Clock { at_min, days } => {
                    if spec.fired.last_date == local.date {
                        continue;
                    }
                    if !days.is_empty() && !days.contains(&local.weekday) {
                        continue;
                    }
                    let effective = zone.next_valid_minute(&local.date, *at_min);
                    if local.minute < effective {
                        continue;
                    }
                    let late_ms = (local.ms_of_day - i64::from(effective) * 60_000).max(0);
                    let missed = late_ms > ALLOWANCE_MS;
                    let hhmm = format!("{:02}:{:02}", at_min / 60, at_min % 60);
                    out.push(Fire {
                        spec: index,
                        late_ms,
                        missed,
                        stamp: TimedFired {
                            last_date: local.date.clone(),
                            ..spec.fired.clone()
                        },
                        local: Some((local.date.clone(), hhmm)),
                    });
                }
                SpecKind::Uptime {
                    seconds,
                    started_ms,
                    late_ok,
                } => {
                    let Some(started) = started_ms else { continue };
                    let started_s = started / 1000;
                    if spec.fired.fired_for == started_s {
                        continue;
                    }
                    let deadline = started + i64::from(*seconds) * 1000;
                    if twitch_now_ms < deadline {
                        continue;
                    }
                    let late_ms = twitch_now_ms - deadline;
                    let missed = late_ms > ALLOWANCE_MS && !late_ok;
                    out.push(Fire {
                        spec: index,
                        late_ms,
                        missed,
                        stamp: TimedFired {
                            fired_for: started_s,
                            ..spec.fired.clone()
                        },
                        local: None,
                    });
                }
                SpecKind::Delay {
                    seconds,
                    joined_steady_ms,
                    joined_id,
                } => {
                    let (Some(joined), Some(id)) = (joined_steady_ms, joined_id) else {
                        continue;
                    };
                    if spec.fired.fired_for == *id {
                        continue;
                    }
                    let deadline = joined + i64::from(*seconds) * 1000;
                    if steady_ms < deadline {
                        continue;
                    }
                    let late_ms = steady_ms - deadline;
                    out.push(Fire {
                        spec: index,
                        late_ms,
                        missed: late_ms > ALLOWANCE_MS,
                        stamp: TimedFired {
                            fired_for: *id,
                            ..spec.fired.clone()
                        },
                        local: None,
                    });
                }
            }
        }
        out
    }
}

/// How long a timed job may wait before it acts, counted from its deadline
/// (D273, D283): a Timer `max_wait + 5 s`; a Clock, Uptime and Delay
/// `120 s + max_wait + 5 s`; a `late_ok` Uptime counts from admission like a
/// chat job (`late_ms` is then ignored). None: already past it.
pub fn job_age_bound(kind: &SpecKind, max_wait_s: u32, late_ms: i64) -> Option<Duration> {
    let base = u64::from(max_wait_s) + QUEUE_GRACE_S;
    let (total_ms, late) = match kind {
        SpecKind::Timer { .. } => (base * 1000, late_ms),
        SpecKind::Uptime { late_ok: true, .. } => (base * 1000, 0),
        _ => (base * 1000 + ALLOWANCE_MS as u64, late_ms),
    };
    let left = total_ms as i64 - late.max(0);
    (left > 0).then(|| Duration::from_millis(left as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed zone: UTC, with an optional missing minute.
    struct Utc {
        gap: Option<(u32, u32)>,
    }

    impl Zone for Utc {
        fn local_of(&self, wall_ms: i64) -> LocalPoint {
            let secs = wall_ms.div_euclid(1000);
            let days = secs.div_euclid(86_400);
            let minute = (secs.rem_euclid(86_400) / 60) as u32;
            let date = chrono::NaiveDate::from_num_days_from_ce_opt(719_163 + days as i32).unwrap();
            let weekday = [
                Weekday::Mon,
                Weekday::Tue,
                Weekday::Wed,
                Weekday::Thu,
                Weekday::Fri,
                Weekday::Sat,
                Weekday::Sun,
            ][chrono::Datelike::weekday(&date).num_days_from_monday() as usize];
            LocalPoint {
                date: date.format("%Y-%m-%d").to_string(),
                minute,
                ms_of_day: secs.rem_euclid(86_400) * 1000 + wall_ms.rem_euclid(1000),
                weekday,
            }
        }
        fn next_valid_minute(&self, _date: &str, minute: u32) -> u32 {
            match self.gap {
                Some((from, to)) if (from..to).contains(&minute) => to,
                _ => minute,
            }
        }
    }

    fn timer(rule: &str, chat: &str, every: u32) -> Spec {
        Spec {
            rule_id: rule.into(),
            chat_key: chat.into(),
            kind: SpecKind::Timer { every_s: every },
            fired: TimedFired::default(),
        }
    }

    fn clock(at_min: u32) -> Spec {
        Spec {
            rule_id: "c".into(),
            chat_key: "twitch:a".into(),
            kind: SpecKind::Clock {
                at_min,
                days: vec![],
            },
            fired: TimedFired::default(),
        }
    }

    const MIN: i64 = 60_000;
    const DAY0: i64 = 1_700_000_000_000 - (1_700_000_000_000 % 86_400_000);

    #[test]
    fn a_timer_never_fires_on_arming_and_then_every_interval() {
        let mut m = Machine::default();
        let specs = [timer("t", "twitch:a", 900)];
        assert!(
            m.tick(0, DAY0, DAY0, &specs, &Utc { gap: None }).is_empty(),
            "arming never fires"
        );
        let offset = offset_ms("twitch:a", 900).max(1);
        assert!(m
            .tick(
                offset - 1,
                DAY0 + offset - 1,
                DAY0,
                &specs,
                &Utc { gap: None }
            )
            .is_empty());
        let fire = m.tick(offset, DAY0 + offset, DAY0, &specs, &Utc { gap: None });
        assert_eq!(fire.len(), 1);
        assert_eq!(fire[0].stamp.last_fire_ms, DAY0 + offset);
        assert!(m
            .tick(
                offset + 899_000,
                DAY0 + offset + 899_000,
                DAY0,
                &specs,
                &Utc { gap: None }
            )
            .is_empty());
        assert_eq!(
            m.tick(
                offset + 900_000,
                DAY0 + offset + 900_000,
                DAY0,
                &specs,
                &Utc { gap: None }
            )
            .len(),
            1
        );
    }

    #[test]
    fn offsets_spread_a_25_chat_rule() {
        let offsets: std::collections::HashSet<i64> = (0..25)
            .map(|i| offset_ms(&format!("twitch:chat{i}"), 900))
            .collect();
        assert!(offsets.len() >= 20, "{} distinct", offsets.len());
    }

    #[test]
    fn a_wall_clock_step_back_never_refires_a_stamped_deadline() {
        let mut m = Machine::default();
        let mut s = timer("t", "twitch:a", 900);
        s.fired.last_fire_ms = DAY0 + 10 * MIN;
        // Armed long ago: due by the steady clock, but the wall clock stepped back.
        m.entries
            .insert(("t".into(), "twitch:a".into()), Entry { next_ms: 0 });
        assert!(m
            .tick(1000, DAY0 + 5 * MIN, DAY0, &[s], &Utc { gap: None })
            .is_empty());
    }

    #[test]
    fn a_resume_rearms_and_a_restart_never_double_fires() {
        let mut m = Machine::default();
        let specs = [timer("t", "twitch:a", 900)];
        m.tick(0, DAY0, DAY0, &specs, &Utc { gap: None });
        m.resume();
        assert!(
            m.tick(10 * MIN, DAY0 + 10 * MIN, DAY0, &specs, &Utc { gap: None })
                .is_empty(),
            "armed again from now"
        );
        let mut stamped = specs[0].clone();
        stamped.fired.last_fire_ms = DAY0 + 10 * MIN;
        let mut fresh = Machine::default();
        fresh.tick(
            0,
            DAY0 + 10 * MIN,
            DAY0,
            &[stamped.clone()],
            &Utc { gap: None },
        );
        let at = offset_ms("twitch:a", 900).max(1);
        assert!(
            fresh
                .tick(
                    at,
                    DAY0 + 10 * MIN + 1000,
                    DAY0,
                    &[stamped],
                    &Utc { gap: None }
                )
                .is_empty(),
            "a fresh process honors the stamp"
        );
    }

    #[test]
    fn a_clock_fires_once_per_date_and_skips_when_over_two_minutes_late() {
        let mut m = Machine::default();
        let mut s = clock(19 * 60 + 30);
        let at = DAY0 + (19 * 60 + 30) * MIN;
        assert!(m
            .tick(0, at - MIN, 0, &[s.clone()], &Utc { gap: None })
            .is_empty());
        let f = m.tick(0, at, 0, &[s.clone()], &Utc { gap: None });
        assert_eq!((f.len(), f[0].missed), (1, false));
        assert_eq!(f[0].local, Some(("2023-11-14".into(), "19:30".into())));
        s.fired = f[0].stamp.clone();
        assert!(
            m.tick(0, at + MIN, 0, &[s.clone()], &Utc { gap: None })
                .is_empty(),
            "once per local date"
        );
        let mut late = clock(19 * 60 + 30);
        late.fired = TimedFired::default();
        let f = m.tick(0, at + 3 * MIN, 0, &[late], &Utc { gap: None });
        assert!(f[0].missed, "found 3 minutes late");
    }

    #[test]
    fn a_clock_late_is_measured_in_milliseconds_not_whole_minutes() {
        let mut m = Machine::default();
        let s = clock(19 * 60 + 30);
        let at = DAY0 + (19 * 60 + 30) * MIN;
        let f = m.tick(0, at + 119_999, 0, &[s.clone()], &Utc { gap: None });
        assert!(
            !f[0].missed && f[0].late_ms == 119_999,
            "inside the 2 minute allowance by 1 ms"
        );
        let f = m.tick(0, at + 120_001, 0, &[s], &Utc { gap: None });
        assert!(f[0].missed, "1 ms past it");
    }

    #[test]
    fn a_missing_dst_minute_fires_at_the_next_valid_one() {
        let mut m = Machine::default();
        let s = clock(2 * 60 + 30);
        let zone = Utc {
            gap: Some((2 * 60, 3 * 60)),
        };
        assert!(m
            .tick(0, DAY0 + (2 * 60 + 45) * MIN, 0, &[s.clone()], &zone)
            .is_empty());
        let f = m.tick(0, DAY0 + 3 * 60 * MIN, 0, &[s], &zone);
        assert_eq!((f.len(), f[0].missed), (1, false));
    }

    #[test]
    fn uptime_fires_once_per_broadcast_and_late_ok_after_the_window() {
        let mut m = Machine::default();
        let started = 1_700_000_000_000;
        let spec = |late_ok| Spec {
            rule_id: "u".into(),
            chat_key: "twitch:a".into(),
            kind: SpecKind::Uptime {
                seconds: 3600,
                started_ms: Some(started),
                late_ok,
            },
            fired: TimedFired::default(),
        };
        let deadline = started + 3_600_000;
        assert!(m
            .tick(0, 0, deadline - 1, &[spec(false)], &Utc { gap: None })
            .is_empty());
        assert_eq!(
            m.tick(0, 0, deadline + 1000, &[spec(false)], &Utc { gap: None })[0].missed,
            false
        );
        assert!(m.tick(0, 0, deadline + 3 * MIN, &[spec(false)], &Utc { gap: None })[0].missed);
        let late = m.tick(
            0,
            0,
            deadline + 3 * 3_600_000,
            &[spec(true)],
            &Utc { gap: None },
        );
        assert!(!late[0].missed, "late_ok fires any time after, once");
        let mut done = spec(true);
        done.fired = late[0].stamp.clone();
        assert!(m
            .tick(0, 0, deadline + 4 * 3_600_000, &[done], &Utc { gap: None })
            .is_empty());
    }

    #[test]
    fn delay_fires_once_per_join_within_the_allowance() {
        let mut m = Machine::default();
        let s = Spec {
            rule_id: "d".into(),
            chat_key: "twitch:a".into(),
            kind: SpecKind::Delay {
                seconds: 60,
                joined_steady_ms: Some(1000),
                joined_id: Some(77),
            },
            fired: TimedFired::default(),
        };
        assert!(m
            .tick(60_999, 0, 0, &[s.clone()], &Utc { gap: None })
            .is_empty());
        assert_eq!(
            m.tick(61_000, 0, 0, &[s.clone()], &Utc { gap: None }).len(),
            1
        );
        assert!(m.tick(61_000 + 3 * MIN, 0, 0, &[s], &Utc { gap: None })[0].missed);
    }

    #[test]
    fn the_job_age_bound_counts_from_the_deadline_with_the_rules_allowance() {
        let timer = SpecKind::Timer { every_s: 900 };
        // Default max_wait 10 s: a Timer job 14 s old runs, 15 s old is dropped.
        assert!(job_age_bound(&timer, 10, 14_000).is_some());
        assert!(job_age_bound(&timer, 10, 15_000).is_none());
        let clock = SpecKind::Clock {
            at_min: 0,
            days: vec![],
        };
        // Bound 120 + 10 + 5 = 135 s: admitted at 118 s there are 17 s left (runs when
        // started at 134 s, dropped at 136 s).
        let left = job_age_bound(&clock, 10, 118_000).unwrap();
        assert_eq!(left, Duration::from_secs(17));
        let late_ok = SpecKind::Uptime {
            seconds: 1,
            started_ms: None,
            late_ok: true,
        };
        assert_eq!(
            job_age_bound(&late_ok, 10, 3 * 3_600_000),
            Some(Duration::from_secs(15)),
            "counts from admission"
        );
        assert_eq!(
            job_age_bound(
                &SpecKind::Delay {
                    seconds: 1,
                    joined_steady_ms: None,
                    joined_id: None
                },
                10,
                136_000
            ),
            None
        );
    }

    #[test]
    fn every_words_reads_plainly() {
        assert_eq!(every_words(900), "15 minutes");
        assert_eq!(every_words(3600), "1 hour");
        assert_eq!(every_words(7200), "2 hours");
        assert_eq!(every_words(90), "90 seconds");
        assert_eq!(every_words(60), "1 minute");
    }
}
