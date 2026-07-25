//! Per-sender sliding-window rate limiting for `coordination/send`.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use batman_protocol::WorkerId;

/// The sliding window width: one minute.
const WINDOW: Duration = Duration::from_secs(60);

/// Returned when a sender exceeds the allowed rate within the window.
#[derive(Debug, thiserror::Error)]
#[error("rate limited: sender exceeded {limit} messages per minute")]
pub struct RateLimitError {
    pub limit: u32,
}

/// A per-sender sliding-window limiter. One instance per runtime process,
/// shared by the coordination broker.
pub struct RateLimiter {
    limit: u32,
    sent_at: Mutex<HashMap<WorkerId, Vec<Instant>>>,
}

impl RateLimiter {
    #[must_use]
    pub fn new(limit: u32) -> Self {
        Self {
            limit,
            sent_at: Mutex::new(HashMap::new()),
        }
    }

    /// Records one message from `sender` at `now`, evicting timestamps
    /// outside the window first. Returns [`RateLimitError`] if this would
    /// be the `limit + 1`-th message within the trailing minute.
    ///
    /// # Errors
    /// Returns [`RateLimitError`] when the sender has already sent `limit`
    /// messages within the trailing one-minute window.
    pub fn check(&self, sender: WorkerId, now: Instant) -> Result<(), RateLimitError> {
        let mut sent_at = self
            .sent_at
            .lock()
            .expect("rate limiter mutex is never poisoned");
        let timestamps = sent_at.entry(sender).or_default();
        timestamps.retain(|t| now.duration_since(*t) < WINDOW);

        if timestamps.len() >= self.limit as usize {
            return Err(RateLimitError { limit: self.limit });
        }
        timestamps.push(now);
        Ok(())
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new(batman_protocol::COORDINATION_RATE_LIMIT_PER_MINUTE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_up_to_the_limit_then_rejects() {
        let limiter = RateLimiter::new(3);
        let sender = WorkerId::new();
        let now = Instant::now();

        assert!(limiter.check(sender, now).is_ok());
        assert!(limiter.check(sender, now).is_ok());
        assert!(limiter.check(sender, now).is_ok());
        assert!(limiter.check(sender, now).is_err());
    }

    #[test]
    fn window_slides_and_frees_capacity() {
        let limiter = RateLimiter::new(1);
        let sender = WorkerId::new();
        let t0 = Instant::now();

        assert!(limiter.check(sender, t0).is_ok());
        assert!(limiter.check(sender, t0).is_err());

        let later = t0 + Duration::from_secs(61);
        assert!(limiter.check(sender, later).is_ok());
    }

    #[test]
    fn tracks_each_sender_independently() {
        let limiter = RateLimiter::new(1);
        let a = WorkerId::new();
        let b = WorkerId::new();
        let now = Instant::now();

        assert!(limiter.check(a, now).is_ok());
        assert!(limiter.check(a, now).is_err());
        assert!(limiter.check(b, now).is_ok());
    }
}
