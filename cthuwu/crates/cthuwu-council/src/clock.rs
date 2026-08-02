use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// Whole Unix seconds. Clocks are injected into state machines and tests.
pub trait Clock: Send + Sync {
    fn now(&self) -> u64;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}

#[derive(Clone, Debug)]
pub struct ManualClock(Arc<Mutex<u64>>);

impl ManualClock {
    pub fn new(now: u64) -> Self {
        Self(Arc::new(Mutex::new(now)))
    }

    pub fn set(&self, now: u64) {
        *self.0.lock().expect("manual clock mutex poisoned") = now;
    }

    pub fn advance(&self, seconds: u64) {
        let mut now = self.0.lock().expect("manual clock mutex poisoned");
        *now = now.saturating_add(seconds);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> u64 {
        *self.0.lock().expect("manual clock mutex poisoned")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_is_deterministic() {
        let clock = ManualClock::new(100);
        clock.advance(25);
        assert_eq!(clock.now(), 125);
        clock.set(7);
        assert_eq!(clock.now(), 7);
    }
}
