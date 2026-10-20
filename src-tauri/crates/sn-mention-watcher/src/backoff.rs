//! Doubling waits between retries: 1 s, 2 s, 4 s ... up to 60 s.

use std::time::Duration;

pub const FIRST: Duration = Duration::from_secs(1);
pub const MAX: Duration = Duration::from_secs(60);

#[derive(Debug)]
pub struct Backoff {
    next: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Backoff { next: FIRST }
    }
}

impl Backoff {
    /// The wait before the next try, doubling the one after it.
    pub fn next(&mut self) -> Duration {
        let wait = self.next;
        self.next = (self.next * 2).min(MAX);
        wait
    }

    pub fn reset(&mut self) {
        self.next = FIRST;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doubles_to_a_minute_and_resets() {
        let mut b = Backoff::default();
        let waits: Vec<u64> = (0..8).map(|_| b.next().as_secs()).collect();
        assert_eq!(waits, vec![1, 2, 4, 8, 16, 32, 60, 60]);
        b.reset();
        assert_eq!(b.next(), FIRST);
    }
}
