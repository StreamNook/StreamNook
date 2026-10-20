//! Matches on their way to the Worker: batched (every second, or at 50),
//! signed, retried with backoff while the Worker is unreachable, and capped
//! at 5,000 waiting, the oldest dropped first.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use crate::backoff::Backoff;
use crate::health::Health;
use crate::matcher::Mention;
use crate::sign::{now_secs, signature, SIG_HEADER, TS_HEADER};

pub const PATH: &str = "/watcher/mentions";
pub const BATCH: usize = 50;
pub const CAP: usize = 5_000;
pub const EVERY: Duration = Duration::from_secs(1);

/// The waiting matches, oldest first.
#[derive(Debug, Default)]
pub struct Queue {
    items: VecDeque<Mention>,
}

impl Queue {
    /// Add one; true when the oldest had to go to make room.
    pub fn push(&mut self, m: Mention) -> bool {
        let full = self.items.len() >= CAP;
        if full {
            self.items.pop_front();
        }
        self.items.push_back(m);
        full
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The next batch, left in place until it is accepted.
    pub fn peek(&self) -> Vec<Mention> {
        self.items.iter().take(BATCH).cloned().collect()
    }

    pub fn pop(&mut self, n: usize) {
        for _ in 0..n.min(self.items.len()) {
            self.items.pop_front();
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Accepted,
    /// Keep the batch and try again later.
    Retry,
    /// The Worker will never take this batch: drop it.
    Refused,
}

/// What an HTTP status means for the batch.
pub fn classify(status: u16) -> Outcome {
    match status {
        200..=299 => Outcome::Accepted,
        401 | 403 | 408 | 429 | 500..=599 => Outcome::Retry,
        _ => Outcome::Refused,
    }
}

pub struct Poster {
    pub client: reqwest::Client,
    pub base: String,
    pub secret: String,
}

impl Poster {
    async fn post(&self, batch: &[Mention]) -> Outcome {
        let body = match serde_json::to_vec(batch) {
            Ok(body) => body,
            Err(e) => {
                tracing::error!("outbox: could not serialize a batch: {e}");
                return Outcome::Refused;
            }
        };
        let ts = now_secs();
        let sig = signature(&self.secret, ts, "POST", PATH, &body);
        let sent = self
            .client
            .post(format!("{}{}", self.base, PATH))
            .header(TS_HEADER, ts.to_string())
            .header(SIG_HEADER, sig)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await;
        match sent {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let outcome = classify(status);
                if outcome != Outcome::Accepted {
                    tracing::warn!("outbox: the Worker answered {status} for {} mentions", batch.len());
                }
                outcome
            }
            Err(e) => {
                tracing::warn!("outbox: could not reach the Worker: {e}");
                Outcome::Retry
            }
        }
    }
}

/// Take matches from `rx` and hand them on until `rx` closes, then try once
/// more with what is left.
pub async fn run(mut rx: mpsc::Receiver<Mention>, poster: Poster, health: Arc<Health>) {
    let mut queue = Queue::default();
    let mut tick = tokio::time::interval(EVERY);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut backoff = Backoff::default();
    let mut not_before = Instant::now();
    let mut closing = false;
    loop {
        tokio::select! {
            got = rx.recv(), if !closing => match got {
                Some(m) => {
                    if queue.push(m) {
                        health.drop_mentions(1);
                    }
                    if queue.len() < BATCH {
                        continue;
                    }
                }
                None => closing = true,
            },
            _ = tick.tick() => {}
        }
        while !queue.is_empty() && Instant::now() >= not_before {
            let batch = queue.peek();
            match poster.post(&batch).await {
                Outcome::Accepted => {
                    queue.pop(batch.len());
                    backoff.reset();
                    health.post_ok();
                    tracing::debug!("outbox: handed on {} mentions", batch.len());
                }
                Outcome::Refused => {
                    queue.pop(batch.len());
                    health.drop_mentions(batch.len() as u64);
                }
                Outcome::Retry => {
                    not_before = Instant::now() + backoff.next();
                    break;
                }
            }
            if queue.len() < BATCH && !closing {
                break;
            }
        }
        if closing && (queue.is_empty() || Instant::now() < not_before) {
            if !queue.is_empty() {
                tracing::warn!("outbox: stopping with {} mentions not handed on", queue.len());
            }
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mention(n: usize) -> Mention {
        Mention {
            user_id: "1".into(),
            msg_id: format!("m{n}"),
            channel_id: "2".into(),
            channel_login: "c".into(),
            sender_id: "3".into(),
            sender_login: "s".into(),
            sender_name: "S".into(),
            color: String::new(),
            text: "hi".into(),
            kind: "mention",
            reply_parent_id: None,
            at_ms: 0,
        }
    }

    #[test]
    fn the_queue_drops_the_oldest_past_the_cap() {
        let mut q = Queue::default();
        for n in 0..CAP {
            assert!(!q.push(mention(n)));
        }
        assert!(q.push(mention(CAP)));
        assert_eq!(q.len(), CAP);
        assert_eq!(q.peek()[0].msg_id, "m1");
    }

    #[test]
    fn a_batch_is_at_most_fifty_and_stays_until_popped() {
        let mut q = Queue::default();
        for n in 0..120 {
            q.push(mention(n));
        }
        assert_eq!(q.peek().len(), BATCH);
        assert_eq!(q.len(), 120);
        q.pop(BATCH);
        assert_eq!(q.peek()[0].msg_id, "m50");
    }

    #[test]
    fn statuses_sort_into_accept_retry_and_refuse() {
        assert_eq!(classify(200), Outcome::Accepted);
        assert_eq!(classify(204), Outcome::Accepted);
        assert_eq!(classify(403), Outcome::Retry);
        assert_eq!(classify(429), Outcome::Retry);
        assert_eq!(classify(503), Outcome::Retry);
        assert_eq!(classify(400), Outcome::Refused);
        assert_eq!(classify(413), Outcome::Refused);
    }

    #[test]
    fn the_wire_shape_matches_the_worker_contract() {
        let json = serde_json::to_value(vec![mention(7)]).unwrap();
        let keys: Vec<&str> = json[0].as_object().unwrap().keys().map(String::as_str).collect();
        for key in [
            "user_id", "msg_id", "channel_id", "channel_login", "sender_id", "sender_login", "sender_name",
            "color", "text", "kind", "reply_parent_id", "at_ms",
        ] {
            assert!(keys.contains(&key), "missing {key}");
        }
        assert!(json[0]["reply_parent_id"].is_null());
    }
}
