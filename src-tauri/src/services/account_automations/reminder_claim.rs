//! A reminder's claim on streamnook.app: the keys it claims under, the one
//! decision it comes to, and the door it asks through.
//!
//! Reminders keep their own timers, texts and send path (`reminder_service`).
//! What P5 adds is that two devices running the same reminder post once per
//! fire. Right before `post_text`, the fire builds [`reminder_elements`] for
//! its slot, verifies the account it posts as and claims; [`claim_step`] is the
//! whole policy over a [`ReminderDoor`], so a test fakes the claim transport and
//! the token check with no network and no real wait.
//!
//! A rule made from a reminder claims under the same keys (`ClaimAs`), so both
//! share one builder: [`converted_elements`] calls [`reminder_elements`].

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::time::Instant;

use super::activity::{EntryKind, NewEntry, REMINDER_ID_PREFIX};
use super::claim_client::{CallKind, ClaimRequest, ClaimResult, Unknown};
use super::claims::{
    self, hash16, rail_family, reminder_kw_key, Element, KeyMode, BUILD_FAILED, MAX_WINDOW_S,
};
use super::executor::{read_answer, Reading};
use super::verify::{TokenRefusal, VerifiedToken};
use super::Engine;
use crate::models::automations::Rule;

/// Activity words when a reminder posted without the check.
pub const POSTED_ANYWAY: &str = "Couldn't check with StreamNook; posted anyway";
/// One deadline for a reminder's verify and its claim together.
pub const CLAIM_DEADLINE: Duration = Duration::from_secs(15);
/// The window of the `reminder_run` key an interval under 30 s claims through.
pub const RUN_WINDOW_S: u32 = 30;
/// The holder's longest time to send, answered to a device that loses.
pub const SETTLE_MS: u32 = 60_000;
const MIN_WINDOW_S: u64 = 30;
const DELAY_FLOOR_S: u64 = 600;

/// Which of the five triggers a reminder (or a converted rule's `claim_as`) is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReminderKind {
    Interval,
    Delay,
    Clock,
    Uptime,
    Keyword,
}

impl ReminderKind {
    pub fn word(self) -> &'static str {
        match self {
            ReminderKind::Interval => "interval",
            ReminderKind::Delay => "delay",
            ReminderKind::Clock => "clock",
            ReminderKind::Uptime => "uptime",
            ReminderKind::Keyword => "keyword",
        }
    }

    pub fn from_word(word: &str) -> Option<ReminderKind> {
        Some(match word {
            "interval" => ReminderKind::Interval,
            "delay" => ReminderKind::Delay,
            "clock" => ReminderKind::Clock,
            "uptime" => ReminderKind::Uptime,
            "keyword" => ReminderKind::Keyword,
            _ => return None,
        })
    }
}

/// What one fire is for: the part of its claim key that tells one fire from
/// the next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReminderSlot {
    Interval {
        secs: u64,
    },
    Delay {
        secs: u64,
    },
    /// The scheduled local date (`YYYY-MM-DD`), time (`HH:MM`) and the UTC
    /// offset in force at that time, so two devices across a DST change agree.
    Clock {
        date: String,
        time: String,
        offset_s: i32,
    },
    /// Twitch's `started_at` in epoch seconds, and the threshold in seconds.
    Uptime {
        started_s: i64,
        secs: u64,
    },
    /// The triggering message's claim id (the part after `chat:`).
    Keyword {
        msg_id: String,
    },
}

impl ReminderSlot {
    pub fn kind(&self) -> ReminderKind {
        match self {
            ReminderSlot::Interval { .. } => ReminderKind::Interval,
            ReminderSlot::Delay { .. } => ReminderKind::Delay,
            ReminderSlot::Clock { .. } => ReminderKind::Clock,
            ReminderSlot::Uptime { .. } => ReminderKind::Uptime,
            ReminderSlot::Keyword { .. } => ReminderKind::Keyword,
        }
    }
}

/// What a reminder's claim is built from besides the slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReminderSpec {
    /// 16 hex over the reminder's message and target ([`family_hash`]).
    pub hash: String,
    /// A keyword reminder's cooldown in seconds; 0 = no rail.
    pub cooldown_s: u32,
}

/// The first 16 hex of the SHA-256 of the reminder's message and target
/// (`current`, or the pinned login in lowercase). Two devices with the same
/// reminder agree on it; wording elsewhere never enters.
pub fn family_hash(message: &str, target: &str) -> String {
    hash16(&format!("{message}|{target}"))
}

/// The event family of one reminder in one chat: `reminder:<hash>|post:<room id>`.
pub fn event_family(hash: &str, room_id: &str) -> String {
    format!("reminder:{hash}|post:{room_id}")
}

/// `ceil(0.9 x secs)` clamped to 30 s through 30 days.
pub fn interval_window_s(secs: u64) -> u32 {
    let window = secs.saturating_mul(9).saturating_add(9) / 10;
    window.clamp(MIN_WINDOW_S, u64::from(MAX_WINDOW_S)) as u32
}

/// `max(secs, 600)`, at most 30 days.
pub fn delay_window_s(secs: u64) -> u32 {
    secs.max(DELAY_FLOOR_S).min(u64::from(MAX_WINDOW_S)) as u32
}

/// `+HH:MM` or `-HH:MM`.
pub fn offset_text(offset_s: i32) -> String {
    let sign = if offset_s < 0 { '-' } else { '+' };
    let a = offset_s.unsigned_abs();
    format!("{sign}{:02}:{:02}", a / 3600, (a % 3600) / 60)
}

/// The elements of one reminder fire's claim call (§ 4.8, Reminder fire row):
/// the event element for the slot, and for a keyword reminder with a cooldown
/// its own windowed rail. No other rail ever joins a reminder's call. Err when
/// an element would not have the plain shape (nothing is sent then).
pub fn reminder_elements(
    spec: &ReminderSpec,
    slot: &ReminderSlot,
    chat_key: &str,
    room_id: &str,
) -> Result<Vec<Element>, &'static str> {
    let family = event_family(&spec.hash, room_id);
    let (key, mode, window_s) = match slot {
        ReminderSlot::Interval { secs } if *secs < MIN_WINDOW_S => (
            format!("reminder_run:{chat_key}"),
            KeyMode::Windowed,
            Some(RUN_WINDOW_S),
        ),
        ReminderSlot::Interval { secs } => (
            format!("reminder:{chat_key}"),
            KeyMode::Windowed,
            Some(interval_window_s(*secs)),
        ),
        ReminderSlot::Delay { secs } => (
            format!("reminder:{chat_key}"),
            KeyMode::Windowed,
            Some(delay_window_s(*secs)),
        ),
        ReminderSlot::Clock {
            date,
            time,
            offset_s,
        } => (
            format!(
                "reminder:{chat_key}:{date}T{time}{}",
                offset_text(*offset_s)
            ),
            KeyMode::Exact,
            None,
        ),
        ReminderSlot::Uptime { started_s, secs } => (
            format!("reminder:{chat_key}:{started_s}:{secs}"),
            KeyMode::Exact,
            None,
        ),
        ReminderSlot::Keyword { msg_id } => (claims::chat_key(msg_id), KeyMode::Exact, None),
    };
    let mut out = vec![Element::Event {
        key,
        family: family.clone(),
        mode,
        window_s,
        sends: 1,
        to_person: false,
        settle_ms: SETTLE_MS,
    }];
    if matches!(slot, ReminderSlot::Keyword { .. }) && spec.cooldown_s > 0 {
        out.push(Element::Windowed {
            key: reminder_kw_key(&family, chat_key),
            family: rail_family("reminder_kw"),
            window_s: spec.cooldown_s.min(MAX_WINDOW_S),
        });
    }
    if out.iter().all(Element::plain_ok) {
        Ok(out)
    } else {
        Err(BUILD_FAILED)
    }
}

/// The elements a rule made from a reminder claims with, when its `claim_as`
/// still applies (same trigger kind, chat scope, Send target and owner) and
/// `slot` is of the kind it was converted for; None means "claim under the
/// rule's own keys". Interval and Delay take their seconds from `claim_as`,
/// so the window is the reminder's, whatever the rule's Timer says now.
pub fn converted_elements(
    rule: &Rule,
    room_id: &str,
    chat_key: &str,
    slot: &ReminderSlot,
) -> Option<Result<Vec<Element>, &'static str>> {
    let claim = rule.claim_as.as_ref().filter(|c| c.applies_to(rule))?;
    if ReminderKind::from_word(&claim.kind) != Some(slot.kind()) {
        return None;
    }
    let slot = match slot {
        ReminderSlot::Interval { .. } => ReminderSlot::Interval {
            secs: claim.seconds,
        },
        ReminderSlot::Delay { .. } => ReminderSlot::Delay {
            secs: claim.seconds,
        },
        other => other.clone(),
    };
    let spec = ReminderSpec {
        hash: claim.family_hash.clone(),
        cooldown_s: claim.cooldown_s,
    };
    Some(reminder_elements(&spec, &slot, chat_key, room_id))
}

/// A claim that won: what a release names.
#[derive(Clone, Debug)]
pub struct WonCall {
    pub call_id: String,
    pub claimed_at: std::time::Instant,
}

/// What one claim call came to, in a reminder's terms.
#[derive(Clone, Debug)]
pub enum Reply {
    /// The event won (and every rail). `None` for a win the cache answered:
    /// there is no call to release.
    Won(Option<WonCall>),
    /// A valid answer that says another device has it, in Activity words.
    Lost { words: String },
    /// A 429, with the wait its body named when it did.
    Busy { retry_after: Option<Duration> },
    /// The site's own "bad request" (400 or 413).
    BadRequest(u16),
    /// Anything that is not a valid answer: a network error, the bound, a
    /// 5xx, a non-JSON body, a 401, a pool wait that ran out.
    Unanswered,
}

fn lost_words(holder: &str, same_device: bool) -> String {
    if same_device {
        "Already posted from this device".into()
    } else if holder.is_empty() {
        "Another device posted this reminder".into()
    } else {
        format!("Another device posted this reminder ({holder})")
    }
}

fn cooldown_words(holder: &str) -> String {
    if holder.is_empty() {
        "Cooldown from another device".into()
    } else {
        format!("Cooldown from {holder}")
    }
}

/// One claim result, read as a reminder reads it. `retry_after` is what the
/// claim client kept from a 429's body.
pub fn map_result(
    result: ClaimResult,
    elements: &[Element],
    retry_after: Option<Duration>,
) -> Reply {
    match result {
        ClaimResult::Cached { won: true, .. } => Reply::Won(None),
        ClaimResult::Cached {
            won: false,
            holder_name,
            ..
        } => Reply::Lost {
            words: lost_words(&holder_name, false),
        },
        ClaimResult::Answered {
            call_id,
            claimed_at,
            answer,
        } => match read_answer(elements, &answer.elements) {
            Reading::Won(_) => Reply::Won(Some(WonCall {
                call_id,
                claimed_at,
            })),
            Reading::Cooldown { holder, .. } => Reply::Lost {
                words: cooldown_words(&holder),
            },
            Reading::Held {
                holder,
                same_device,
                ..
            } => Reply::Lost {
                words: lost_words(&holder, same_device),
            },
            _ => Reply::Lost {
                words: lost_words("", false),
            },
        },
        ClaimResult::Unknown {
            why: Unknown::Busy, ..
        } => Reply::Busy { retry_after },
        ClaimResult::Unknown {
            why: Unknown::NotReached(code),
            ..
        } if code == 400 || code == 413 => Reply::BadRequest(code),
        ClaimResult::Unknown { .. } => Reply::Unanswered,
    }
}

/// What a reminder asks through: the token check, the claim and the release.
/// The real one is [`EngineDoor`]; tests use fakes.
#[async_trait]
pub trait ReminderDoor: Send + Sync {
    async fn verify(&self, owner_id: &str, budget: Duration)
        -> Result<VerifiedToken, TokenRefusal>;
    async fn claim(&self, request: ClaimRequest) -> Reply;
    /// Give a won call back (a release sent from the claim client's pool).
    fn release(&self, owner_id: &str, call: &WonCall);
}

/// Everything one fire's claim needs.
#[derive(Clone, Debug)]
pub struct ClaimInput {
    /// `ChatRules::own_identity()`'s id: the account `post_text` posts as.
    pub owner_id: String,
    pub owner_login: String,
    pub spec: ReminderSpec,
    pub slot: ReminderSlot,
    /// `twitch:<login>`.
    pub chat_key: String,
    /// The chat's room id, resolved before the claim; None when it could not be.
    pub room_id: Option<String>,
}

/// What the claim decided.
#[derive(Clone, Debug)]
pub enum Decision {
    /// The claim won: post. `call` is None for a cached win.
    Post { call: Option<WonCall> },
    /// No valid answer (or nothing to claim with): post anyway, never release.
    PostAnyway,
    /// Skip, in Activity words.
    Skip { words: String },
}

/// The whole reminder claim policy, under one deadline shared by the token
/// check and the claim (D280). Posts anyway on every result that is not a
/// valid claim answer; skips only on a valid Lost, a 400 or a 413, or a
/// sign-in that is gone (`NoToken` or `Refused`). A 429 whose wait fits the
/// time left is waited once and the claim asked again.
pub async fn claim_step(
    door: &dyn ReminderDoor,
    input: &ClaimInput,
    deadline: Instant,
) -> Decision {
    let Some(room_id) = input.room_id.as_deref().filter(|r| !r.is_empty()) else {
        return Decision::PostAnyway;
    };
    let Ok(elements) = reminder_elements(&input.spec, &input.slot, &input.chat_key, room_id) else {
        return Decision::PostAnyway;
    };
    let left = || deadline.saturating_duration_since(Instant::now());
    let verified =
        match tokio::time::timeout_at(deadline, door.verify(&input.owner_id, left())).await {
            Ok(Ok(verified)) => verified,
            Ok(Err(TokenRefusal::NoToken(_))) | Ok(Err(TokenRefusal::Refused(_))) => {
                return Decision::Skip {
                    words: format!("Sign in as @{} again", input.owner_login),
                };
            }
            // WrongAccount, Unreachable or the deadline: the check could not be answered.
            Ok(Err(_)) | Err(_) => return Decision::PostAnyway,
        };
    let mut waited = false;
    loop {
        let budget = left();
        if budget.is_zero() {
            return Decision::PostAnyway;
        }
        let request = ClaimRequest {
            owner_id: input.owner_id.clone(),
            bearer: verified.token.clone(),
            elements: elements.clone(),
            kind: CallKind::Reminder,
            budget,
            settle_ms: u64::from(SETTLE_MS),
            next_fire_in: match &input.slot {
                ReminderSlot::Interval { secs } => Some(Duration::from_secs(*secs)),
                _ => None,
            },
        };
        let reply = match tokio::time::timeout_at(deadline, door.claim(request)).await {
            Ok(reply) => reply,
            Err(_) => return Decision::PostAnyway,
        };
        match reply {
            Reply::Won(call) => return Decision::Post { call },
            Reply::Lost { words } => return Decision::Skip { words },
            Reply::BadRequest(code) => {
                log::warn!("[Reminders] the claim was refused as a bad request ({code})");
                return Decision::Skip {
                    words: format!(
                        "StreamNook couldn't read this reminder's check ({code}); not posted"
                    ),
                };
            }
            Reply::Busy {
                retry_after: Some(wait),
            } if !waited && wait < left() => {
                waited = true;
                tokio::time::sleep(wait).await;
            }
            Reply::Busy { .. } | Reply::Unanswered => return Decision::PostAnyway,
        }
    }
}

/// The real door: the engine's token check and claim client.
pub struct EngineDoor {
    engine: Arc<Engine>,
}

impl EngineDoor {
    /// A door over one engine (the tests build engines on fakes).
    pub fn over(engine: Arc<Engine>) -> EngineDoor {
        EngineDoor { engine }
    }
}

/// The door over the running engine; None only before `Engine::init` sets the
/// singleton (never because run is Off or the phone is in the background).
pub fn engine_door() -> Option<Arc<dyn ReminderDoor>> {
    super::engine()
        .map(|engine| Arc::new(EngineDoor::over(engine.clone())) as Arc<dyn ReminderDoor>)
}

#[async_trait]
impl ReminderDoor for EngineDoor {
    async fn verify(
        &self,
        owner_id: &str,
        budget: Duration,
    ) -> Result<VerifiedToken, TokenRefusal> {
        self.engine.outward.verified_token(owner_id, budget).await
    }

    async fn claim(&self, request: ClaimRequest) -> Reply {
        let elements = request.elements.clone();
        let result = self.engine.claims.claim(request).await;
        let retry_after = match &result {
            ClaimResult::Unknown {
                call_id: Some(id),
                why: Unknown::Busy,
            } => self.engine.claims.take_retry_hint(id),
            _ => None,
        };
        map_result(result, &elements, retry_after)
    }

    fn release(&self, owner_id: &str, call: &WonCall) {
        // The first attempt's receiver is not awaited: a release is retried by its own pool.
        let _ = self.engine.claims.release(
            owner_id,
            &call.call_id,
            call.claimed_at,
            u64::from(SETTLE_MS),
        );
    }
}

/// One Activity line about a reminder, through the engine's log (a no-op
/// before the engine starts). Fires fold into one `acted` row per reminder.
pub fn log_reminder(kind: EntryKind, reminder_id: &str, name: &str, login: &str, text: &str) {
    let Some(engine) = super::engine() else {
        return;
    };
    engine.log(NewEntry {
        kind,
        rule_id: format!("{REMINDER_ID_PREFIX}{reminder_id}"),
        rule_name: name.to_string(),
        channel: format!("twitch:{login}"),
        text: text.to_string(),
    });
    engine.announce_status();
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use super::*;
    use crate::models::automations::{
        AccountRef, Action, ChannelRef, ChannelScope, ClaimAs, Line, Pick, Rule, Trigger,
    };

    const CHAT: &str = "twitch:streamer";
    const ROOM: &str = "4242";

    fn spec(cooldown_s: u32) -> ReminderSpec {
        ReminderSpec {
            hash: family_hash("Plug the Discord", "current"),
            cooldown_s,
        }
    }

    fn only(elements: Vec<Element>) -> Element {
        assert_eq!(elements.len(), 1, "{elements:?}");
        elements.into_iter().next().unwrap()
    }

    #[test]
    fn interval_windows_are_ninety_percent_clamped_to_30_seconds_through_30_days() {
        assert_eq!(interval_window_s(600), 540);
        assert_eq!(interval_window_s(601), 541);
        assert_eq!(interval_window_s(30), 30, "ceil(27) is lifted to 30");
        assert_eq!(
            interval_window_s(172_800),
            155_520,
            "a 48 h reminder's window is accepted"
        );
        assert_eq!(interval_window_s(u64::MAX / 2), MAX_WINDOW_S);
        assert_eq!(delay_window_s(5), 600);
        assert_eq!(delay_window_s(3600), 3600);
    }

    #[test]
    fn each_slot_builds_its_own_key_mode_and_window() {
        let key_of = |slot: ReminderSlot| match only(
            reminder_elements(&spec(0), &slot, CHAT, ROOM).unwrap(),
        ) {
            Element::Event {
                key,
                family,
                mode,
                window_s,
                sends,
                to_person,
                ..
            } => (key, family, mode, window_s, sends, to_person),
            other => panic!("{other:?}"),
        };
        let family = event_family(&spec(0).hash, ROOM);
        assert_eq!(
            key_of(ReminderSlot::Interval { secs: 600 }),
            (
                "reminder:twitch:streamer".to_string(),
                family.clone(),
                KeyMode::Windowed,
                Some(540),
                1,
                false
            )
        );
        assert_eq!(
            key_of(ReminderSlot::Interval { secs: 1 }).0,
            "reminder_run:twitch:streamer"
        );
        assert_eq!(key_of(ReminderSlot::Interval { secs: 20 }).3, Some(30));
        assert_eq!(key_of(ReminderSlot::Delay { secs: 60 }).3, Some(600));
        assert_eq!(
            key_of(ReminderSlot::Clock {
                date: "2026-11-01".into(),
                time: "21:00".into(),
                offset_s: -18_000
            }),
            (
                "reminder:twitch:streamer:2026-11-01T21:00-05:00".to_string(),
                family.clone(),
                KeyMode::Exact,
                None,
                1,
                false
            )
        );
        assert_eq!(
            key_of(ReminderSlot::Uptime {
                started_s: 1_790_000_000,
                secs: 5430
            })
            .0,
            "reminder:twitch:streamer:1790000000:5430"
        );
        assert_eq!(
            key_of(ReminderSlot::Keyword {
                msg_id: "m0".into()
            })
            .0,
            "chat:m0"
        );
        assert_eq!(offset_text(19_800), "+05:30");
        assert_eq!(offset_text(0), "+00:00");
    }

    #[test]
    fn a_keyword_reminder_carries_its_own_windowed_rail_and_nothing_else() {
        let slot = ReminderSlot::Keyword {
            msg_id: "m1".into(),
        };
        let elements = reminder_elements(&spec(180), &slot, CHAT, ROOM).unwrap();
        let family = event_family(&spec(180).hash, ROOM);
        assert_eq!(elements.len(), 2);
        match &elements[1] {
            Element::Windowed {
                key,
                family: rail,
                window_s,
            } => {
                assert_eq!(key, &reminder_kw_key(&family, CHAT));
                assert_eq!(rail, "rail:reminder_kw");
                assert_eq!(*window_s, 180);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            reminder_elements(&spec(0), &slot, CHAT, ROOM)
                .unwrap()
                .len(),
            1,
            "no cooldown, no rail"
        );
        // Two different keyword reminders in one chat have two rails and both post.
        let other = ReminderSpec {
            hash: family_hash("Other text", "current"),
            cooldown_s: 180,
        };
        let rail =
            |s: &ReminderSpec| match reminder_elements(s, &slot, CHAT, ROOM).unwrap().remove(1) {
                Element::Windowed { key, .. } => key,
                other => panic!("{other:?}"),
            };
        assert_ne!(rail(&spec(180)), rail(&other));
        // The same reminder in another chat is another rail.
        let elsewhere = match reminder_elements(&spec(180), &slot, "twitch:other", ROOM)
            .unwrap()
            .remove(1)
        {
            Element::Windowed { key, .. } => key,
            other => panic!("{other:?}"),
        };
        assert_ne!(rail(&spec(180)), elsewhere);
    }

    #[test]
    fn every_element_has_the_plain_shape_and_a_bad_chat_key_is_never_sent() {
        for slot in [
            ReminderSlot::Interval { secs: 600 },
            ReminderSlot::Delay { secs: 600 },
            ReminderSlot::Clock {
                date: "2026-11-01".into(),
                time: "21:00".into(),
                offset_s: 3600,
            },
            ReminderSlot::Uptime {
                started_s: 1,
                secs: 2,
            },
            ReminderSlot::Keyword {
                msg_id: "m0".into(),
            },
        ] {
            assert!(
                reminder_elements(&spec(60), &slot, CHAT, ROOM)
                    .unwrap()
                    .iter()
                    .all(Element::plain_ok),
                "{slot:?}"
            );
        }
        let long = format!("twitch:{}", "x".repeat(700));
        assert_eq!(
            reminder_elements(&spec(0), &ReminderSlot::Interval { secs: 600 }, &long, ROOM),
            Err(BUILD_FAILED)
        );
    }

    /// A rule in the shape "Make it a rule" leaves: the owner, one chat, one Send
    /// with no `to`, and a `claim_as` describing exactly that. (Its trigger is
    /// whatever `Rule::default()` has; `claim_as` names that kind, which is all
    /// `applies_to` compares.)
    fn converted_rule(kind: &str, seconds: u64, cooldown_s: u32) -> Rule {
        let mut rule = Rule::default();
        rule.owner = AccountRef {
            provider: "twitch".into(),
            id: "100".into(),
            login: "me".into(),
            name: "me".into(),
        };
        rule.send_as = rule.owner.clone();
        rule.only_if.channels = ChannelScope::These {
            list: vec![ChannelRef {
                id: "c-1".into(),
                key: CHAT.into(),
                room_id: ROOM.into(),
                name: "streamer".into(),
            }],
        };
        rule.actions = vec![Action::Send {
            id: "a-1".into(),
            to: None,
            texts: vec![Line {
                id: "l-1".into(),
                text: "Plug the Discord".into(),
            }],
            pick: Pick::Random,
        }];
        let (trigger_kind, scope_kind, scope_key, send_to) = ClaimAs::shape_of(&rule);
        rule.claim_as = Some(ClaimAs {
            account_id: "100".into(),
            kind: kind.into(),
            family_hash: spec(0).hash,
            seconds,
            cooldown_s,
            trigger_kind,
            scope_kind,
            scope_key,
            send_to,
        });
        rule
    }

    #[test]
    fn a_converted_rule_claims_exactly_as_the_reminder_while_claim_as_applies() {
        let mut rule = converted_rule("keyword", 0, 180);
        let slot = ReminderSlot::Keyword {
            msg_id: "m0".into(),
        };
        let want = reminder_elements(&spec(180), &slot, CHAT, ROOM);
        assert_eq!(converted_elements(&rule, ROOM, CHAT, &slot), Some(want));

        // A text edit keeps it.
        if let Some(Action::Send { texts, .. }) = rule.actions.first_mut() {
            texts[0].text = "Something else entirely".into();
        }
        assert!(converted_elements(&rule, ROOM, CHAT, &slot).is_some());

        // A change of the rule's owner, chat scope, trigger kind or the Send's chat clears it.
        let mut owner = rule.clone();
        owner.owner.id = "200".into();
        assert_eq!(converted_elements(&owner, ROOM, CHAT, &slot), None);
        let mut scope = rule.clone();
        scope.only_if.channels = ChannelScope::Own {};
        assert_eq!(converted_elements(&scope, ROOM, CHAT, &slot), None);
        let mut trigger = rule.clone();
        trigger.when = Trigger::FirstMessage {};
        assert_eq!(converted_elements(&trigger, ROOM, CHAT, &slot), None);
        let mut send_to = rule.clone();
        if let Some(Action::Send { to, .. }) = send_to.actions.first_mut() {
            *to = Some(ChannelRef {
                id: "c-2".into(),
                key: "twitch:other".into(),
                room_id: "9".into(),
                name: "other".into(),
            });
        }
        assert_eq!(converted_elements(&send_to, ROOM, CHAT, &slot), None);
        // A slot of another kind is not the reminder's.
        assert_eq!(
            converted_elements(&rule, ROOM, CHAT, &ReminderSlot::Delay { secs: 60 }),
            None
        );
        // A rule that was never converted claims under its own keys.
        rule.claim_as = None;
        assert_eq!(converted_elements(&rule, ROOM, CHAT, &slot), None);
    }

    #[test]
    fn a_converted_interval_takes_its_window_from_claim_as_not_the_timer() {
        let rule = converted_rule("interval", 600, 0);
        let elements = converted_elements(&rule, ROOM, CHAT, &ReminderSlot::Interval { secs: 900 })
            .unwrap()
            .unwrap();
        match only(elements) {
            Element::Event { window_s, .. } => assert_eq!(
                window_s,
                Some(540),
                "the reminder's 600 s, not the timer's 900 s"
            ),
            other => panic!("{other:?}"),
        }
    }

    // ---- the policy, over a fake door on a paused clock -------------------------------

    struct FakeDoor {
        verifies: Mutex<VecDeque<(Duration, Result<VerifiedToken, TokenRefusal>)>>,
        claims: Mutex<VecDeque<(Duration, Reply)>>,
        budgets: Mutex<Vec<Duration>>,
        claim_calls: AtomicUsize,
        released: Mutex<Vec<String>>,
    }

    fn token() -> VerifiedToken {
        VerifiedToken {
            token: "tok".into(),
            user_id: "100".into(),
            login: "me".into(),
        }
    }

    fn won() -> Reply {
        Reply::Won(Some(WonCall {
            call_id: "c1".into(),
            claimed_at: std::time::Instant::now(),
        }))
    }

    impl FakeDoor {
        fn new() -> FakeDoor {
            FakeDoor {
                verifies: Mutex::new(VecDeque::new()),
                claims: Mutex::new(VecDeque::new()),
                budgets: Mutex::new(Vec::new()),
                claim_calls: AtomicUsize::new(0),
                released: Mutex::new(Vec::new()),
            }
        }
        fn verify_gives(
            self,
            after: Duration,
            result: Result<VerifiedToken, TokenRefusal>,
        ) -> FakeDoor {
            self.verifies.lock().unwrap().push_back((after, result));
            self
        }
        fn claim_gives(self, after: Duration, reply: Reply) -> FakeDoor {
            self.claims.lock().unwrap().push_back((after, reply));
            self
        }
        fn calls(&self) -> usize {
            self.claim_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl ReminderDoor for FakeDoor {
        async fn verify(
            &self,
            _owner_id: &str,
            _budget: Duration,
        ) -> Result<VerifiedToken, TokenRefusal> {
            let next = self.verifies.lock().unwrap().pop_front();
            let (after, result) = next.unwrap_or((Duration::ZERO, Ok(token())));
            tokio::time::sleep(after).await;
            result
        }
        async fn claim(&self, request: ClaimRequest) -> Reply {
            self.claim_calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(
                request.kind,
                CallKind::Reminder,
                "a reminder claim spends the reminder budget"
            );
            self.budgets.lock().unwrap().push(request.budget);
            let next = self.claims.lock().unwrap().pop_front();
            let (after, reply) = next.unwrap_or((Duration::ZERO, Reply::Unanswered));
            tokio::time::sleep(after).await;
            reply
        }
        fn release(&self, _owner_id: &str, call: &WonCall) {
            self.released.lock().unwrap().push(call.call_id.clone());
        }
    }

    fn input(slot: ReminderSlot) -> ClaimInput {
        ClaimInput {
            owner_id: "100".into(),
            owner_login: "me".into(),
            spec: spec(0),
            slot,
            chat_key: CHAT.into(),
            room_id: Some(ROOM.into()),
        }
    }

    fn within(secs: u64) -> Instant {
        Instant::now() + Duration::from_secs(secs)
    }

    fn is_anyway(d: &Decision) -> bool {
        matches!(d, Decision::PostAnyway)
    }

    #[tokio::test(start_paused = true)]
    async fn a_won_claim_posts_and_a_lost_one_skips_in_the_reminders_words() {
        let door = FakeDoor::new().claim_gives(Duration::ZERO, won());
        assert!(matches!(
            claim_step(
                &door,
                &input(ReminderSlot::Interval { secs: 600 }),
                within(15)
            )
            .await,
            Decision::Post { call: Some(_) }
        ));
        let door = FakeDoor::new().claim_gives(
            Duration::ZERO,
            Reply::Lost {
                words: lost_words("Windows PC", false),
            },
        );
        match claim_step(
            &door,
            &input(ReminderSlot::Interval { secs: 600 }),
            within(15),
        )
        .await
        {
            Decision::Skip { words } => {
                assert_eq!(words, "Another device posted this reminder (Windows PC)")
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn every_unanswered_result_posts_anyway() {
        // network error, 5xx, non-JSON, a 401 and the pool wait all read as Unanswered
        let door = FakeDoor::new().claim_gives(Duration::ZERO, Reply::Unanswered);
        assert!(is_anyway(
            &claim_step(&door, &input(ReminderSlot::Delay { secs: 60 }), within(15)).await
        ));
        assert_eq!(door.calls(), 1);
    }

    #[test]
    fn the_result_table_reads_each_answer_as_the_plan_says() {
        let elements = reminder_elements(
            &spec(180),
            &ReminderSlot::Keyword {
                msg_id: "m1".into(),
            },
            CHAT,
            ROOM,
        )
        .unwrap();
        let unknown = |why: Unknown| ClaimResult::Unknown {
            call_id: Some("c".into()),
            why,
        };
        for why in [
            Unknown::Network,
            Unknown::Server(500),
            Unknown::Server(502),
            Unknown::Server(503),
            Unknown::Server(504),
            Unknown::Unreadable,
            Unknown::Deadline,
            Unknown::NotReached(401),
            Unknown::NotReached(404),
        ] {
            assert!(
                matches!(
                    map_result(unknown(why.clone()), &elements, None),
                    Reply::Unanswered
                ),
                "{why:?}"
            );
        }
        assert!(matches!(
            map_result(unknown(Unknown::NotReached(400)), &elements, None),
            Reply::BadRequest(400)
        ));
        assert!(matches!(
            map_result(unknown(Unknown::NotReached(413)), &elements, None),
            Reply::BadRequest(413)
        ));
        assert!(matches!(
            map_result(unknown(Unknown::Busy), &elements, Some(Duration::from_secs(3))),
            Reply::Busy { retry_after: Some(d) } if d == Duration::from_secs(3)
        ));
        assert!(matches!(
            map_result(
                ClaimResult::Cached {
                    won: true,
                    holder_name: String::new(),
                    free_at_ms: None
                },
                &elements,
                None
            ),
            Reply::Won(None)
        ));
        let answer = |json: serde_json::Value| ClaimResult::Answered {
            call_id: "c9".into(),
            claimed_at: std::time::Instant::now(),
            answer: serde_json::from_value(json).unwrap(),
        };
        // The event won and the rail lost: the keyword cooldown reads "Cooldown from <device>".
        let rail_lost = answer(serde_json::json!({
            "elements": [
                { "index": 0, "kind": "event", "won": false, "reason": "skipped" },
                { "index": 1, "kind": "windowed", "won": false, "reason": "reminder_kw", "holder_name": "Windows PC", "free_at_ms": 5 }
            ],
            "server_now_ms": 1
        }));
        match map_result(rail_lost, &elements, None) {
            Reply::Lost { words } => assert_eq!(words, "Cooldown from Windows PC"),
            other => panic!("{other:?}"),
        }
        let both_won = answer(serde_json::json!({
            "elements": [{ "index": 0, "kind": "event", "won": true }, { "index": 1, "kind": "windowed", "won": true }],
            "server_now_ms": 1
        }));
        assert!(
            matches!(map_result(both_won, &elements, None), Reply::Won(Some(c)) if c.call_id == "c9")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn silence_past_the_deadline_posts_anyway_at_15_seconds() {
        let door = FakeDoor::new().claim_gives(Duration::from_secs(60), won());
        let start = Instant::now();
        let decision = claim_step(
            &door,
            &input(ReminderSlot::Interval { secs: 600 }),
            start + CLAIM_DEADLINE,
        )
        .await;
        assert!(is_anyway(&decision));
        assert_eq!(
            start.elapsed(),
            CLAIM_DEADLINE,
            "no real wait, and it stops exactly at the bound"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_verify_that_takes_14_seconds_leaves_the_claim_one_second() {
        let door = FakeDoor::new()
            .verify_gives(Duration::from_secs(14), Ok(token()))
            .claim_gives(Duration::ZERO, won());
        let decision = claim_step(
            &door,
            &input(ReminderSlot::Interval { secs: 600 }),
            within(15),
        )
        .await;
        assert!(matches!(decision, Decision::Post { .. }));
        assert_eq!(
            door.budgets.lock().unwrap().as_slice(),
            &[Duration::from_secs(1)]
        );
        // A verify that uses the whole 15 s leaves no claim to make.
        let door = FakeDoor::new().verify_gives(Duration::from_secs(15), Ok(token()));
        assert!(is_anyway(
            &claim_step(
                &door,
                &input(ReminderSlot::Interval { secs: 600 }),
                within(15)
            )
            .await
        ));
        assert_eq!(door.calls(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn the_token_check_decides_between_posting_anyway_and_skipping() {
        let cases: Vec<(Result<VerifiedToken, TokenRefusal>, bool)> = vec![
            (Err(TokenRefusal::Unreachable("down".into())), true),
            (
                Err(TokenRefusal::WrongAccount {
                    expected: "100".into(),
                    actual: "200".into(),
                }),
                true,
            ),
            (Err(TokenRefusal::NoToken("none".into())), false),
            (Err(TokenRefusal::Refused("refused".into())), false),
        ];
        for (result, anyway) in cases {
            let door = FakeDoor::new().verify_gives(Duration::ZERO, result);
            let decision = claim_step(
                &door,
                &input(ReminderSlot::Interval { secs: 600 }),
                within(15),
            )
            .await;
            if anyway {
                assert!(is_anyway(&decision), "{decision:?}");
            } else {
                assert!(
                    matches!(&decision, Decision::Skip { words } if words == "Sign in as @me again"),
                    "{decision:?}"
                );
            }
            assert_eq!(door.calls(), 0, "no claim without a verified owner");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_429_that_fits_is_waited_once_and_asked_again() {
        let busy = |s: u64| Reply::Busy {
            retry_after: Some(Duration::from_secs(s)),
        };
        // Then a valid Lost skips.
        let door = FakeDoor::new()
            .claim_gives(Duration::ZERO, busy(2))
            .claim_gives(Duration::ZERO, Reply::Lost { words: "x".into() });
        let start = Instant::now();
        assert!(matches!(
            claim_step(
                &door,
                &input(ReminderSlot::Interval { secs: 600 }),
                within(15)
            )
            .await,
            Decision::Skip { .. }
        ));
        assert_eq!((door.calls(), start.elapsed()), (2, Duration::from_secs(2)));
        // A second 429 posts anyway.
        let door = FakeDoor::new()
            .claim_gives(Duration::ZERO, busy(2))
            .claim_gives(Duration::ZERO, busy(2));
        assert!(is_anyway(
            &claim_step(
                &door,
                &input(ReminderSlot::Interval { secs: 600 }),
                within(15)
            )
            .await
        ));
        assert_eq!(door.calls(), 2);
        // A Retry-After past the time left posts anyway at once, with one call.
        let door = FakeDoor::new().claim_gives(Duration::ZERO, busy(20));
        assert!(is_anyway(
            &claim_step(
                &door,
                &input(ReminderSlot::Interval { secs: 600 }),
                within(15)
            )
            .await
        ));
        assert_eq!(door.calls(), 1);
        // A 429 with no wait named reads as past the time left.
        let door = FakeDoor::new().claim_gives(Duration::ZERO, Reply::Busy { retry_after: None });
        assert!(is_anyway(
            &claim_step(
                &door,
                &input(ReminderSlot::Interval { secs: 600 }),
                within(15)
            )
            .await
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn a_bad_request_skips_and_a_slow_lost_posts_nothing() {
        let door = FakeDoor::new().claim_gives(Duration::ZERO, Reply::BadRequest(400));
        assert!(
            matches!(&claim_step(&door, &input(ReminderSlot::Delay { secs: 60 }), within(15)).await, Decision::Skip { words } if words.contains("400"))
        );
        let door = FakeDoor::new().claim_gives(Duration::ZERO, Reply::BadRequest(413));
        assert!(
            matches!(&claim_step(&door, &input(ReminderSlot::Delay { secs: 60 }), within(15)).await, Decision::Skip { words } if words.contains("413"))
        );
        // An answer 5 s late that says Lost still skips.
        let door = FakeDoor::new().claim_gives(
            Duration::from_secs(5),
            Reply::Lost {
                words: "late".into(),
            },
        );
        assert!(matches!(
            claim_step(&door, &input(ReminderSlot::Delay { secs: 60 }), within(15)).await,
            Decision::Skip { .. }
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn no_room_id_posts_anyway_without_touching_the_door() {
        let mut no_room = input(ReminderSlot::Interval { secs: 600 });
        no_room.room_id = None;
        let door = FakeDoor::new();
        assert!(is_anyway(&claim_step(&door, &no_room, within(15)).await));
        no_room.room_id = Some(String::new());
        assert!(is_anyway(&claim_step(&door, &no_room, within(15)).await));
        assert_eq!(door.calls(), 0);
    }

    // ---- a rule made from a reminder: the open-chat rule and the wiring ----------------------

    use super::super::compile::{compile, CompileInput, Compiled};
    use super::super::local::Local;
    use super::super::{reconcile, testkit};
    use crate::models::automations::{AutomationPrefs, Automations, RunState};

    fn compiled_with(rule: Rule) -> Compiled {
        let group = Automations {
            run: RunState::On,
            rules: vec![rule.to_value()],
            ..Automations::default()
        };
        let prefs = AutomationPrefs {
            other_channels: true,
            ..AutomationPrefs::default()
        };
        let own = [("100".to_string(), "me".to_string())];
        compile(&CompileInput {
            automations: &group,
            prefs: &prefs,
            own_accounts: &own,
            profile_user_id: Some("100"),
            primary_id: None,
        })
    }

    #[test]
    fn a_rule_that_posts_only_where_a_window_holds_the_chat_makes_reconcile_join_nothing() {
        let mut rule = converted_rule("keyword", 0, 180);
        rule.name = "Reminder: Plug the Discord".into();
        rule.live = true;
        let joins = |rule: &Rule| compiled_with(rule.clone()).needed_chats;
        assert_eq!(
            joins(&rule),
            vec!["streamer".to_string()],
            "an ordinary These rule is joined under the engine's label"
        );
        rule.only_if.only_while_open = true;
        assert!(
            joins(&rule).is_empty(),
            "no chat is joined for a rule that posts only in chats a window holds"
        );
        // Watching: reconcile keeps the watched chat joined only for a rule without it (§ 4.2's exception).
        let mut watching = converted_rule("keyword", 0, 180);
        watching.name = "Reminder: Plug the Discord".into();
        watching.live = true;
        watching.only_if.channels = ChannelScope::Watching {};
        assert!(reconcile::watching(&compiled_with(watching.clone())));
        watching.only_if.only_while_open = true;
        assert!(
            !reconcile::watching(&compiled_with(watching)),
            "a converted current-channel reminder never joins the watched chat"
        );
    }

    #[tokio::test]
    async fn only_a_label_other_than_the_engines_opens_a_chat_for_a_reminder() {
        let local = testkit::FakeLocal::new();
        local.hold(super::super::AUTOMATIONS_LABEL, "somechat");
        assert!(
            !local.open_for_reminders("somechat").await,
            "the engine's own claim never opens a chat"
        );
        local.hold("main", "somechat");
        assert!(local.open_for_reminders("somechat").await);
        assert!(!local.open_for_reminders("elsewhere").await);
    }

    #[test]
    fn the_status_carries_the_reminders_list_from_the_local_seam() {
        let status = include_str!("mod.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap_or_default();
        assert!(
            status.contains("let reminders = self.local.reminders();")
                && status.contains("            reminders,\n")
        );
        assert!(include_str!("local.rs")
            .contains("crate::services::reminder_service::convert::summaries(&settings)"));
    }

    #[test]
    fn the_executor_checks_open_chats_before_it_claims_and_claims_as_the_reminder() {
        let code = include_str!("executor.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap_or_default();
        let from = code
            .find("async fn final_checks(")
            .expect("final_checks exists");
        let checks = &code[from..];
        let checks = &checks[..checks.find("\n}\n").unwrap_or(checks.len())];
        let at = |needle: &str| {
            checks
                .find(needle)
                .unwrap_or_else(|| panic!("{needle} missing:\n{checks}"))
        };
        assert!(
            at("only.only_while_open") < at("match &job.rule().rule.only_if.channels"),
            "the new checks come first"
        );
        assert!(
            checks.contains("open_for_reminders(login)") && !checks.contains("adopted"),
            "no adopted set, unlike AnyOpen"
        );
        assert!(at("only.only_while_watching") < at("match &job.rule().rule.only_if.channels"));
        assert!(
            code.contains("reminder_claim::converted_elements("),
            "claim_class asks the reminder's keys first"
        );
    }
}
