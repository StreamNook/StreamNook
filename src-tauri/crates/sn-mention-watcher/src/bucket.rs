//! The JOIN pace: one bucket for the whole process, so every connection
//! together stays under Twitch's join limit. Anonymous limits are not
//! documented; 15 per 10 s sits under the 20 documented for accounts.

use std::time::{Duration, Instant};

pub const CAPACITY: u32 = 15;
pub const WINDOW: Duration = Duration::from_secs(10);

/// A token bucket refilled evenly: `CAPACITY` tokens per `WINDOW`.
#[derive(Debug)]
pub struct Bucket {
    capacity: f64,
    per_sec: f64,
    tokens: f64,
    last: Instant,
}

impl Bucket {
    pub fn new(capacity: u32, window: Duration, now: Instant) -> Self {
        Bucket {
            capacity: capacity as f64,
            per_sec: capacity as f64 / window.as_secs_f64(),
            tokens: capacity as f64,
            last: now,
        }
    }

    fn refill(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.per_sec).min(self.capacity);
        self.last = now;
    }

    /// Take a token now, or say how long until one is ready.
    pub fn take(&mut self, now: Instant) -> Result<(), Duration> {
        self.refill(now);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            Ok(())
        } else {
            Err(Duration::from_secs_f64((1.0 - self.tokens) / self.per_sec))
        }
    }

    /// Empty the bucket, for when Twitch pushed back on a join.
    pub fn drain(&mut self, now: Instant) {
        self.refill(now);
        self.tokens = 0.0;
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
