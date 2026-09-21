//! Injectable clock. Modules and services never call `Utc::now()` (§12 rule 4).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};

#[derive(Clone, Debug)]
pub struct Clock(Inner);

#[derive(Clone, Debug)]
enum Inner {
    System,
    Fake(Arc<Mutex<DateTime<Utc>>>),
}

impl Clock {
    pub fn system() -> Self {
        Self(Inner::System)
    }

    /// A clock that only moves when told to. Clones share the same time.
    pub fn fake(start: DateTime<Utc>) -> Self {
        Self(Inner::Fake(Arc::new(Mutex::new(start))))
    }

    pub fn now(&self) -> DateTime<Utc> {
        match &self.0 {
            Inner::System => Utc::now(),
            Inner::Fake(t) => *t.lock().unwrap_or_else(|e| e.into_inner()),
        }
    }

    /// Advance a fake clock. No-op on the system clock.
    pub fn advance(&self, by: Duration) {
        if let Inner::Fake(t) = &self.0 {
            let mut t = t.lock().unwrap_or_else(|e| e.into_inner());
            *t += chrono::Duration::from_std(by).unwrap_or(chrono::Duration::MAX);
        }
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::system()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_clock_advances_and_is_shared() {
        let start = DateTime::parse_from_rfc3339("2026-09-21T00:00:00Z").unwrap().to_utc();
        let a = Clock::fake(start);
        let b = a.clone();
        a.advance(Duration::from_secs(3 * 86_400));
        assert_eq!(b.now(), start + chrono::Duration::days(3));
    }
}
