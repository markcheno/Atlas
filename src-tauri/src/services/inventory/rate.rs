//! Token bucket limiting filesystem metadata operations per second.

use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct TokenBucket {
    /// Operations per second; `None` means unlimited.
    rate: Option<u32>,
    available: f64,
    last: Instant,
}

impl TokenBucket {
    pub fn new(rate: Option<u32>) -> Self {
        Self {
            rate,
            available: rate.map_or(0.0, f64::from),
            last: Instant::now(),
        }
    }

    pub fn rate(&self) -> Option<u32> {
        self.rate
    }

    pub fn set_rate(&mut self, rate: Option<u32>) {
        if rate != self.rate {
            self.refill();
            self.rate = rate;
            if let Some(r) = rate {
                self.available = self.available.min(f64::from(r));
            }
        }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        if let Some(r) = self.rate {
            let r = f64::from(r);
            self.available =
                (self.available + now.duration_since(self.last).as_secs_f64() * r).min(r);
        }
        self.last = now;
    }

    /// Takes `n` operations and returns how long to wait before doing more.
    /// Bursts of up to one second's worth are allowed.
    pub fn reserve(&mut self, n: u32) -> Duration {
        let Some(r) = self.rate.filter(|&r| r > 0) else {
            return Duration::ZERO;
        };
        self.refill();
        self.available -= f64::from(n);
        if self.available >= 0.0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(-self.available / f64::from(r))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlimited_never_waits() {
        let mut b = TokenBucket::new(None);
        assert_eq!(b.reserve(1_000_000), Duration::ZERO);
    }

    #[test]
    fn waits_in_proportion_to_the_deficit() {
        let mut b = TokenBucket::new(Some(100));
        assert_eq!(b.reserve(100), Duration::ZERO, "one second of burst");
        let wait = b.reserve(50);
        assert!(
            wait > Duration::from_millis(450) && wait <= Duration::from_millis(500),
            "{wait:?}"
        );
    }

    #[test]
    fn lowering_the_rate_caps_the_burst() {
        let mut b = TokenBucket::new(Some(100));
        b.set_rate(Some(20));
        assert_eq!(b.reserve(20), Duration::ZERO);
        let wait = b.reserve(20);
        assert!(wait > Duration::from_millis(900), "{wait:?}");
    }
}
