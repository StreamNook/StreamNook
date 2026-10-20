//! The JOIN pace: one bucket for the whole process, so every connection
//! together stays under Twitch's join limit. Anonymous limits are not
//! documented; 15 per 10 s sits under the 20 documented for accounts.

use std::time::{Duration, Instant};

pub const CAPACITY: u32 = 15;
pub const WINDOW: Duration = Duration::from_secs(10);

/// A token bucket refilled evenly: `CAPACITY` tokens per `WINDOW`, kept in
/// whole nanoseconds (the generic cell rate algorithm): `tat` is when the
/// bucket would be full again. With no floating point, a refusal always names
/// a wait of at least a nanosecond, so a caller that sleeps it always gets a
/// token next time instead of spinning on a wait rounded down to zero.
#[derive(Debug)]
pub struct Bucket {
    /// The time one token takes to come back.
    interval: Duration,
    /// How far ahead of now `tat` may run: a full bucket's worth, less one token.
    slack: Duration,
    tat: Instant,
}

impl Bucket {
    pub fn new(capacity: u32, window: Duration, now: Instant) -> Self {
        let capacity = capacity.max(1);
        let interval = window / capacity;
        Bucket {
            interval,
            slack: interval * (capacity - 1),
            tat: now,
        }
    }

    /// Take a token now, or say how long until one is ready (never zero).
    pub fn take(&mut self, now: Instant) -> Result<(), Duration> {
        let tat = self.tat.max(now);
        let ahead = tat - now;
        if ahead <= self.slack {
            self.tat = tat + self.interval;
            Ok(())
        } else {
            Err(ahead - self.slack)
        }
    }

    /// Empty the bucket, for when Twitch pushed back on a join: the next
    /// token comes one interval from now.
    pub fn drain(&mut self, now: Instant) {
        self.tat = now + self.slack + self.interval;
    }
}

/// The process-wide bucket, awaited by every connection before a JOIN.
#[derive(Clone)]
pub struct Shared(std::sync::Arc<tokio::sync::Mutex<Bucket>>);

impl Shared {
    pub fn new() -> Self {
        Shared(std::sync::Arc::new(tokio::sync::Mutex::new(Bucket::new(CAPACITY, WINDOW, Instant::now()))))
    }

    /// Wait for a join token.
    pub async fn acquire(&self) {
        loop {
            let wait = {
                let mut b = self.0.lock().await;
                match b.take(Instant::now()) {
                    Ok(()) => return,
                    Err(wait) => wait,
                }
            };
            tokio::time::sleep(wait).await;
        }
    }

    pub async fn drain(&self) {
        self.0.lock().await.drain(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fifteen_at_once_then_one_every_two_thirds_of_a_second() {
        let t0 = Instant::now();
        let mut b = Bucket::new(CAPACITY, WINDOW, t0);
        for _ in 0..15 {
            assert!(b.take(t0).is_ok());
        }
        let wait = b.take(t0).unwrap_err();
        assert!((wait.as_secs_f64() - 10.0 / 15.0).abs() < 1e-6);
        assert!(b.take(t0 + Duration::from_millis(700)).is_ok());
        assert!(b.take(t0 + Duration::from_millis(700)).is_err());
    }

    #[test]
    fn no_more_than_capacity_after_a_long_idle() {
        let t0 = Instant::now();
        let mut b = Bucket::new(CAPACITY, WINDOW, t0);
        let later = t0 + Duration::from_secs(3600);
        for _ in 0..15 {
            assert!(b.take(later).is_ok());
        }
        assert!(b.take(later).is_err());
    }

    #[test]
    fn a_pushback_empties_the_bucket() {
        let t0 = Instant::now();
        let mut b = Bucket::new(CAPACITY, WINDOW, t0);
        b.drain(t0);
        assert!(b.take(t0).is_err());
        assert!(b.take(t0 + Duration::from_secs(1)).is_ok());
    }

    #[test]
    fn a_refusal_never_names_a_zero_wait() {
        let t0 = Instant::now();
        let mut b = Bucket::new(CAPACITY, WINDOW, t0);
        let mut now = t0;
        for _ in 0..1_000 {
            match b.take(now) {
                Ok(()) => {}
                Err(wait) => {
                    assert!(wait > Duration::ZERO);
                    now += wait;
                }
            }
        }
    }

    #[test]
    fn sixty_joins_take_about_thirty_seconds() {
        // 15 at once, then 45 more at 1.5 per second.
        let t0 = Instant::now();
        let mut b = Bucket::new(CAPACITY, WINDOW, t0);
        let mut now = t0;
        let mut joined = 0;
        while joined < 60 {
            match b.take(now) {
                Ok(()) => joined += 1,
                Err(wait) => now += wait,
            }
        }
        let secs = now.duration_since(t0).as_secs_f64();
        assert!((secs - 30.0).abs() < 0.01, "took {secs}");
    }
}
