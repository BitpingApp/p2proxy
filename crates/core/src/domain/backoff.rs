use std::time::Duration;

/// Exponential delay schedule, capped. The actor sleeps for `next_delay()` via
/// the `Clock` port; this struct holds no clock so the schedule is pure and
/// asserted directly in tests.
#[derive(Debug, Clone)]
pub struct Backoff {
    initial: Duration,
    current: Duration,
    factor: u32,
    max: Duration,
}

impl Backoff {
    pub fn new(initial: Duration, factor: u32, max: Duration) -> Self {
        Self {
            initial,
            current: initial,
            factor: factor.max(1),
            max,
        }
    }

    pub fn next_delay(&mut self) -> Duration {
        let delay = self.current.min(self.max);
        self.current = (self.current.saturating_mul(self.factor)).min(self.max);
        delay
    }

    pub fn reset(&mut self) {
        self.current = self.initial;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grows_exponentially_and_caps() {
        let mut b = Backoff::new(Duration::from_secs(1), 2, Duration::from_secs(8));
        let seq: Vec<u64> = (0..6).map(|_| b.next_delay().as_secs()).collect();
        assert_eq!(seq, vec![1, 2, 4, 8, 8, 8]);
    }

    #[test]
    fn reset_returns_to_initial() {
        let mut b = Backoff::new(Duration::from_secs(1), 2, Duration::from_secs(8));
        b.next_delay();
        b.next_delay();
        b.reset();
        assert_eq!(b.next_delay(), Duration::from_secs(1));
    }

    #[test]
    fn factor_zero_is_treated_as_one() {
        let mut b = Backoff::new(Duration::from_millis(500), 0, Duration::from_secs(8));
        assert_eq!(b.next_delay(), Duration::from_millis(500));
        assert_eq!(b.next_delay(), Duration::from_millis(500));
    }
}
