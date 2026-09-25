use std::time::Duration;

/// Exponential backoff without jitter, so tests are deterministic. The queue never retries
/// (§11.2); modules opt in through `Ctx::retry_with_backoff`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Total attempts including the first. Values below 1 behave as 1.
    pub max_attempts: u32,
    pub initial_delay: Duration,
    pub max_delay: Duration,
}

impl RetryPolicy {
    /// 3 attempts, 1s then 2s: what a fetcher wants.
    pub fn standard() -> Self {
        Self { max_attempts: 3, initial_delay: Duration::from_secs(1), max_delay: Duration::from_secs(30) }
    }

    /// Delay to wait after failed attempt number `attempt` (1-based).
    pub fn delay_after(&self, attempt: u32) -> Duration {
        let shift = attempt.saturating_sub(1).min(31);
        self.initial_delay.checked_mul(1u32 << shift).unwrap_or(self.max_delay).min(self.max_delay)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delays_double_and_cap() {
        let p =
            RetryPolicy { max_attempts: 10, initial_delay: Duration::from_secs(1), max_delay: Duration::from_secs(5) };
        let d: Vec<_> = (1..=5).map(|a| p.delay_after(a).as_secs()).collect();
        assert_eq!(d, [1, 2, 4, 5, 5]);
        assert_eq!(p.delay_after(u32::MAX), Duration::from_secs(5));
    }
}
