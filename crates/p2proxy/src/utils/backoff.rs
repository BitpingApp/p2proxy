//! Exponential backoff utility with jitter
//!
//! Provides configurable exponential backoff for retry logic.

use std::time::Duration;
use rand::{Rng, SeedableRng};
use rand::rngs::StdRng;

/// Exponential backoff calculator with jitter
///
/// Uses `StdRng` for async compatibility (Send-safe).
#[derive(Debug)]
pub struct ExponentialBackoff {
    current: Duration,
    initial: Duration,
    max: Duration,
    multiplier: u32,
    jitter_pct: f64,
    rng: StdRng,  // Send-safe RNG for async contexts
}

impl ExponentialBackoff {
    /// Create a new backoff calculator
    ///
    /// # Arguments
    /// * `initial` - Initial backoff duration
    /// * `max` - Maximum backoff duration (cap)
    /// * `jitter_pct` - Jitter percentage (0.0-1.0)
    ///
    /// # Example
    /// ```
    /// use std::time::Duration;
    /// use p2proxy::utils::backoff::ExponentialBackoff;
    ///
    /// let mut backoff = ExponentialBackoff::new(
    ///     Duration::from_millis(100),
    ///     Duration::from_secs(30),
    ///     0.25  // ±25% jitter
    /// );
    /// ```
    pub fn new(initial: Duration, max: Duration, jitter_pct: f64) -> Self {
        Self {
            current: initial,
            initial,
            max,
            multiplier: 2,
            jitter_pct: jitter_pct.clamp(0.0, 1.0),
            rng: SeedableRng::from_entropy(),  // Seed from system entropy
        }
    }

    /// Create backoff with explicit seed (for deterministic testing)
    pub fn with_seed(initial: Duration, max: Duration, jitter_pct: f64, seed: u64) -> Self {
        Self {
            current: initial,
            initial,
            max,
            multiplier: 2,
            jitter_pct: jitter_pct.clamp(0.0, 1.0),
            rng: StdRng::seed_from_u64(seed),
        }
    }

    /// Get the next backoff duration and advance the internal state
    pub fn next(&mut self) -> Duration {
        let backoff = self.current;

        // Double for next time (capped at max)
        self.current = (self.current * self.multiplier).min(self.max);

        // Add jitter
        self.add_jitter(backoff)
    }

    /// Get the next backoff duration without advancing state
    /// Returns the base duration without jitter for preview purposes
    pub fn peek(&self) -> Duration {
        self.current
    }

    /// Reset to initial backoff
    pub fn reset(&mut self) {
        self.current = self.initial;
    }

    #[allow(deprecated)]  // gen_range/gen_bool deprecated in rand 0.9+, but current version needs them
    fn add_jitter(&mut self, base: Duration) -> Duration {
        if self.jitter_pct == 0.0 {
            return base;
        }

        let jitter_range = (base.as_millis() as f64 * self.jitter_pct) as u64;
        if jitter_range == 0 {
            return base;
        }

        let jitter = self.rng.gen_range(0..=jitter_range);

        // Jitter can be positive or negative (50% chance)
        if self.rng.gen_bool(0.5) {
            base + Duration::from_millis(jitter)
        } else {
            base.saturating_sub(Duration::from_millis(jitter))
        }
    }
}

impl Default for ExponentialBackoff {
    fn default() -> Self {
        Self::new(
            Duration::from_millis(100),
            Duration::from_secs(30),
            0.25,
        )
    }
}

// Note: StdRng implements Send, so ExponentialBackoff automatically implements Send

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exponential_growth() {
        let mut backoff = ExponentialBackoff::new(
            Duration::from_millis(100),
            Duration::from_secs(10),
            0.0,  // No jitter for testing
        );

        assert_eq!(backoff.next(), Duration::from_millis(100));
        assert_eq!(backoff.next(), Duration::from_millis(200));
        assert_eq!(backoff.next(), Duration::from_millis(400));
        assert_eq!(backoff.next(), Duration::from_millis(800));
    }

    #[test]
    fn test_max_cap() {
        let mut backoff = ExponentialBackoff::new(
            Duration::from_secs(5),
            Duration::from_secs(10),
            0.0,
        );

        assert_eq!(backoff.next(), Duration::from_secs(5));
        assert_eq!(backoff.next(), Duration::from_secs(10));  // Capped
        assert_eq!(backoff.next(), Duration::from_secs(10));  // Stays capped
    }

    #[test]
    fn test_reset() {
        let mut backoff = ExponentialBackoff::new(
            Duration::from_millis(100),
            Duration::from_secs(10),
            0.0,
        );

        backoff.next(); // 100ms
        backoff.next(); // 200ms
        backoff.reset();
        assert_eq!(backoff.next(), Duration::from_millis(100));
    }

    #[test]
    fn test_jitter_variance() {
        let mut backoff = ExponentialBackoff::with_seed(
            Duration::from_secs(1),
            Duration::from_secs(30),
            0.25,
            12345,  // Deterministic seed
        );

        let base = Duration::from_secs(1);
        let jittered = backoff.next();

        // With 25% jitter, result should be within ±250ms
        let diff_ms = jittered.as_millis().abs_diff(base.as_millis());
        assert!(diff_ms <= 250, "Jitter too large: {}ms", diff_ms);
    }

    #[test]
    fn test_peek_doesnt_advance() {
        let mut backoff = ExponentialBackoff::new(
            Duration::from_millis(100),
            Duration::from_secs(10),
            0.0,
        );

        // Peek should return same value multiple times
        let peeked1 = backoff.peek();
        let peeked2 = backoff.peek();
        assert_eq!(peeked1, peeked2);

        // Next should advance
        let next1 = backoff.next();
        assert_eq!(next1, Duration::from_millis(100));

        let next2 = backoff.next();
        assert_eq!(next2, Duration::from_millis(200));
    }
}
