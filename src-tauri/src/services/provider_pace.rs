//! The pace of automated posts on a platform that has no shared limiter.
//!
//! Twitch's limits live in `chat_send_limiter`. Kick has no such service, so
//! this module keeps Kick's: one message per chat every 2 seconds, 15 an
//! account in 60 seconds, and the same text not twice in a chat inside 30
//! seconds. An automated post books a slot here and waits for it; a typed
//! post is only recorded and is never refused (you are never stopped from
//! typing, the same counting rule as Twitch's). The keys are the caller's:
//! `kick:<account id>` and `kick:<channel>`, so an id on one platform never
//! shares a count with the same number on another.
//!
//! The booking logic is the pure [`Pace`] over injected `Instant`s; the free
//! functions use one process-wide instance.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// One message per chat in this span.
pub const CHAT_SPACING: Duration = Duration::from_secs(2);
/// The most messages one account sends inside [`ACCOUNT_WINDOW`].
pub const ACCOUNT_PER_WINDOW: usize = 15;
/// The account window.
pub const ACCOUNT_WINDOW: Duration = Duration::from_secs(60);
/// The same text in the same chat is a duplicate for this long.
pub const DUPLICATE_SPAN: Duration = Duration::from_secs(30);
/// What stays remembered.
const KEEP: Duration = Duration::from_secs(180);

/// Why a booking was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaceRefusal {
    /// The same text went out (or is booked) in this chat in the last 30 s.
    Duplicate,
    /// The next free slot is further away than the caller may wait.
    TooLong { wait_s: u64 },
}

impl PaceRefusal {
    /// The words Activity shows.
    pub fn text(&self) -> String {
        match self {
            PaceRefusal::Duplicate => {
                "The same message went out here in the last 30 seconds".to_string()
            }
            PaceRefusal::TooLong { wait_s } => {
                format!("Kick's pace: the next free moment is {wait_s} seconds away")
            }
        }
    }
}

/// A booked slot. Cancel it when nothing was sent; keep it (do nothing) when
/// the send was Sent or Unknown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaceTicket {
    id: u64,
    account: String,
    chat: String,
    at: Instant,
}

impl PaceTicket {
    /// When the slot opens.
    pub fn at(&self) -> Instant {
        self.at
    }
}

#[derive(Clone, Debug)]
struct ChatSlot {
    id: u64,
    at: Instant,
    text: String,
}

/// Slots by chat and by account, each list oldest first.
#[derive(Debug, Default)]
pub struct Pace {
    next_id: u64,
    chats: HashMap<String, Vec<ChatSlot>>,
    accounts: HashMap<String, Vec<(u64, Instant)>>,
}

fn insert_sorted<T>(list: &mut Vec<T>, item: T, at: impl Fn(&T) -> Instant) {
    let key = at(&item);
    let index = list.partition_point(|x| at(x) <= key);
    list.insert(index, item);
}

impl Pace {
    fn prune(&mut self, now: Instant) {
        if let Some(cutoff) = now.checked_sub(KEEP) {
            for slots in self.chats.values_mut() {
                slots.retain(|s| s.at >= cutoff);
            }
            for times in self.accounts.values_mut() {
                times.retain(|(_, t)| *t >= cutoff);
            }
        }
        self.chats.retain(|_, v| !v.is_empty());
        self.accounts.retain(|_, v| !v.is_empty());
    }

    /// Book the next free moment for `text` in `chat` as `account`: at least
    /// 2 s after the chat's last slot, and not inside a full 60 s account
    /// window. Answers how long to wait and the ticket, or why not.
    pub fn book(
        &mut self,
        account: &str,
        chat: &str,
        text: &str,
        max_wait: Duration,
        now: Instant,
    ) -> Result<(Duration, PaceTicket), PaceRefusal> {
        self.prune(now);
        let slots = self.chats.get(chat).map(Vec::as_slice).unwrap_or(&[]);
        if slots
            .iter()
            .any(|s| s.text == text && (s.at > now || now.duration_since(s.at) < DUPLICATE_SPAN))
        {
            return Err(PaceRefusal::Duplicate);
        }
        let mut at = now;
        if let Some(last) = slots.last() {
            at = at.max(last.at + CHAT_SPACING);
        }
        let times = self.accounts.get(account).map(Vec::as_slice).unwrap_or(&[]);
        loop {
            let live: Vec<Instant> = times
                .iter()
                .map(|(_, t)| *t)
                .filter(|t| *t + ACCOUNT_WINDOW > at)
                .collect();
            if live.len() < ACCOUNT_PER_WINDOW {
                break;
            }
            at = at.max(live[live.len() - ACCOUNT_PER_WINDOW] + ACCOUNT_WINDOW);
        }
        let wait = at.saturating_duration_since(now);
        if wait > max_wait {
            return Err(PaceRefusal::TooLong {
                wait_s: wait.as_secs(),
            });
        }
        self.next_id += 1;
        let id = self.next_id;
        insert_sorted(
            self.chats.entry(chat.to_string()).or_default(),
            ChatSlot {
                id,
                at,
                text: text.to_string(),
            },
            |s| s.at,
        );
        insert_sorted(
            self.accounts.entry(account.to_string()).or_default(),
            (id, at),
            |(_, t)| *t,
        );
        Ok((
            wait,
            PaceTicket {
                id,
                account: account.to_string(),
                chat: chat.to_string(),
                at,
            },
        ))
    }

    /// Give a slot back (nothing was sent).
    pub fn cancel(&mut self, ticket: &PaceTicket) {
        if let Some(slots) = self.chats.get_mut(&ticket.chat) {
            slots.retain(|s| s.id != ticket.id);
        }
        if let Some(times) = self.accounts.get_mut(&ticket.account) {
            times.retain(|(id, _)| *id != ticket.id);
        }
    }

    /// A message you typed: counted now, never refused.
    pub fn record_typed(&mut self, account: &str, chat: &str, text: &str, now: Instant) {
        self.prune(now);
        self.next_id += 1;
        let id = self.next_id;
        insert_sorted(
            self.chats.entry(chat.to_string()).or_default(),
            ChatSlot {
                id,
                at: now,
                text: text.to_string(),
            },
            |s| s.at,
        );
        insert_sorted(
            self.accounts.entry(account.to_string()).or_default(),
            (id, now),
            |(_, t)| *t,
        );
    }
}

static PACE: LazyLock<Mutex<Pace>> = LazyLock::new(|| Mutex::new(Pace::default()));

fn pace() -> MutexGuard<'static, Pace> {
    PACE.lock().unwrap_or_else(|e| e.into_inner())
}

/// The process-wide pace the app's own door hands the engine, so a typed send
/// and an automated one count in one place. A test door holds its own `Pace`.
pub fn global() -> &'static Mutex<Pace> {
    &PACE
}

/// Book a Kick slot (the engine's own budget for the Kick class).
pub fn provider_acquire_within(
    account: &str,
    chat: &str,
    text: &str,
    max_wait: Duration,
) -> Result<(Duration, PaceTicket), PaceRefusal> {
    pace().book(account, chat, text, max_wait, Instant::now())
}

/// Give a booked slot back.
pub fn provider_cancel(ticket: &PaceTicket) {
    pace().cancel(ticket);
}

/// Count a message typed by hand.
pub fn provider_record_typed(account: &str, chat: &str, text: &str) {
    pace().record_typed(account, chat, text, Instant::now());
}

/// How long a refusal holds that account in that chat: a rejected sign-in 60
/// s, a refusal to post here 5 minutes, a rate answer 10 s; any other answer
/// holds nothing.
pub fn hold_for_status(status: Option<u16>) -> Option<Duration> {
    match status {
        Some(401) => Some(Duration::from_secs(60)),
        Some(403) => Some(Duration::from_secs(300)),
        Some(429) => Some(Duration::from_secs(10)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX: Duration = Duration::from_secs(130);

    fn sec(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn one_message_per_chat_every_two_seconds() {
        let mut p = Pace::default();
        let now = Instant::now();
        assert_eq!(
            p.book("kick:1", "kick:a", "one", MAX, now).unwrap().0,
            Duration::ZERO
        );
        assert_eq!(
            p.book("kick:1", "kick:a", "two", MAX, now).unwrap().0,
            sec(2)
        );
        assert_eq!(
            p.book("kick:1", "kick:a", "three", MAX, now).unwrap().0,
            sec(4)
        );
        assert_eq!(
            p.book("kick:1", "kick:b", "four", MAX, now).unwrap().0,
            Duration::ZERO,
            "another chat is free"
        );
    }

    #[test]
    fn the_sixteenth_send_in_sixty_seconds_waits_or_is_refused() {
        let mut p = Pace::default();
        let now = Instant::now();
        let mut first = None;
        for n in 0..15 {
            let (wait, ticket) = p
                .book("kick:1", &format!("kick:chat{n}"), "hi", MAX, now)
                .unwrap();
            assert_eq!(wait, Duration::ZERO, "send {n}");
            first.get_or_insert(ticket);
        }
        let (wait, sixteenth) = p.book("kick:1", "kick:chat15", "hi", MAX, now).unwrap();
        assert_eq!(
            wait,
            sec(60),
            "the 16th waits until the first leaves the window"
        );
        let refused = p
            .book("kick:1", "kick:chat16", "hi", sec(10), now)
            .unwrap_err();
        assert_eq!(refused, PaceRefusal::TooLong { wait_s: 60 });
        // The booked 16th holds a slot too (at +60 s), so a freed slot opens the window only
        // once both it and the first are given back.
        p.cancel(&first.unwrap());
        p.cancel(&sixteenth);
        let (wait, _) = p.book("kick:1", "kick:chat17", "hi", sec(10), now).unwrap();
        assert!(wait < sec(60), "a freed slot opens the window again");
        // Another account is not counted.
        assert_eq!(
            p.book("kick:2", "kick:chat18", "hi", MAX, now).unwrap().0,
            Duration::ZERO
        );
    }

    #[test]
    fn typed_sends_are_recorded_and_never_refused() {
        let mut p = Pace::default();
        let now = Instant::now();
        for n in 0..20 {
            p.record_typed("kick:1", &format!("kick:chat{n}"), "typed", now);
        }
        // Twenty typed sends were all accepted; the next automated one waits for the window.
        let (wait, _) = p.book("kick:1", "kick:other", "auto", MAX, now).unwrap();
        assert!(wait > Duration::ZERO && wait <= sec(60));
        // A typed send pushes the chat's next booking behind its 2 s.
        let mut q = Pace::default();
        q.record_typed("kick:1", "kick:a", "typed", now);
        assert_eq!(
            q.book("kick:1", "kick:a", "auto", MAX, now).unwrap().0,
            sec(2)
        );
    }

    #[test]
    fn the_same_text_twice_in_thirty_seconds_is_a_duplicate() {
        let mut p = Pace::default();
        let now = Instant::now();
        p.book("kick:1", "kick:a", "same", MAX, now).unwrap();
        assert_eq!(
            p.book("kick:1", "kick:a", "same", MAX, now + sec(10))
                .unwrap_err(),
            PaceRefusal::Duplicate
        );
        assert!(p
            .book("kick:1", "kick:a", "other", MAX, now + sec(10))
            .is_ok());
        assert!(
            p.book("kick:1", "kick:b", "same", MAX, now + sec(10))
                .is_ok(),
            "another chat"
        );
        assert!(
            p.book("kick:1", "kick:a", "same", MAX, now + sec(31))
                .is_ok(),
            "after 30 s"
        );
    }

    #[test]
    fn a_canceled_booking_frees_its_slot_and_its_text() {
        let mut p = Pace::default();
        let now = Instant::now();
        let (_, ticket) = p.book("kick:1", "kick:a", "same", MAX, now).unwrap();
        p.cancel(&ticket);
        let (wait, _) = p.book("kick:1", "kick:a", "same", MAX, now).unwrap();
        assert_eq!(wait, Duration::ZERO);
    }

    #[test]
    fn a_kick_id_equal_to_another_platform_id_shares_nothing() {
        let mut p = Pace::default();
        let now = Instant::now();
        for n in 0..15 {
            p.book("kick:7", &format!("kick:c{n}"), "hi", MAX, now)
                .unwrap();
        }
        // The same number under another prefix is another account.
        assert_eq!(
            p.book("youtube:7", "youtube:c0", "hi", MAX, now).unwrap().0,
            Duration::ZERO
        );
        assert_eq!(p.book("7", "c0", "hi", MAX, now).unwrap().0, Duration::ZERO);
    }

    #[test]
    fn old_slots_are_forgotten() {
        let mut p = Pace::default();
        let now = Instant::now();
        p.book("kick:1", "kick:a", "old", MAX, now).unwrap();
        let later = now + sec(200);
        p.book("kick:1", "kick:b", "new", MAX, later).unwrap();
        assert!(p.chats.get("kick:a").is_none(), "pruned");
    }

    #[test]
    fn holds_follow_the_refusal_code() {
        assert_eq!(hold_for_status(Some(401)), Some(sec(60)));
        assert_eq!(hold_for_status(Some(403)), Some(sec(300)));
        assert_eq!(hold_for_status(Some(429)), Some(sec(10)));
        assert_eq!(hold_for_status(Some(400)), None);
        assert_eq!(hold_for_status(None), None);
    }

    #[test]
    fn the_free_functions_share_one_pace() {
        let (wait, ticket) =
            provider_acquire_within("kick:pace-test-acct", "kick:pace-test-chat", "hello", MAX)
                .unwrap();
        assert_eq!(wait, Duration::ZERO);
        let refused =
            provider_acquire_within("kick:pace-test-acct", "kick:pace-test-chat", "hello", MAX);
        assert_eq!(refused.unwrap_err(), PaceRefusal::Duplicate);
        provider_cancel(&ticket);
        assert!(provider_acquire_within(
            "kick:pace-test-acct",
            "kick:pace-test-chat",
            "hello",
            MAX
        )
        .is_ok());
        provider_record_typed("kick:pace-test-acct", "kick:pace-test-chat2", "typed");
    }
}
