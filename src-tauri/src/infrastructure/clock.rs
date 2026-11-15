//! Wall-clock time: RFC 3339 UTC timestamps for the catalog, and a [`Clock`]
//! the knowledge services take as a dependency, so tests can move time on
//! by weeks (Phase 3: sessions, expiry, snapshots).

use std::sync::Mutex;

use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime, UtcOffset};

pub fn now_rfc3339() -> String {
    rfc3339(OffsetDateTime::now_utc())
}

/// `t` to the second, as the catalog writes times.
pub fn rfc3339(t: OffsetDateTime) -> String {
    t.replace_nanosecond(0)
        .ok()
        .and_then(|t| t.format(&Rfc3339).ok())
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_owned())
}

/// Parses an RFC 3339 time.
pub fn parse_rfc3339(text: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(text, &Rfc3339).ok()
}

/// Where "now" comes from.
pub trait Clock: Send + Sync {
    fn now(&self) -> OffsetDateTime;
    fn now_rfc3339(&self) -> String {
        rfc3339(self.now())
    }
    /// The local time zone's offset from UTC now, for the hours in
    /// Settings (Slice 4.6).
    fn local_offset(&self) -> UtcOffset {
        UtcOffset::UTC
    }
}

/// The system clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }

    /// Asked of the OS each time, so a change of time zone or daylight
    /// saving takes effect at once. chrono reads the zone without the
    /// environment races `time`'s local offset guards against.
    fn local_offset(&self) -> UtcOffset {
        let seconds = chrono::Local::now().offset().local_minus_utc();
        UtcOffset::from_whole_seconds(seconds).unwrap_or(UtcOffset::UTC)
    }
}

/// A clock tests set and move on. Public so integration tests can use it;
/// the app never does.
#[derive(Debug)]
pub struct TestClock {
    now: Mutex<OffsetDateTime>,
    offset: Mutex<UtcOffset>,
}

impl TestClock {
    pub fn at(t: OffsetDateTime) -> Self {
        Self {
            now: Mutex::new(t),
            offset: Mutex::new(UtcOffset::UTC),
        }
    }

    /// Sets the local offset the clock reports.
    pub fn set_offset(&self, offset: UtcOffset) {
        *self.offset.lock().unwrap_or_else(|e| e.into_inner()) = offset;
    }

    /// Moves the clock to `t`.
    pub fn set(&self, t: OffsetDateTime) {
        *self.now.lock().unwrap_or_else(|e| e.into_inner()) = t;
    }

    /// Starting at the system's time now.
    pub fn starting_now() -> Self {
        Self::at(OffsetDateTime::now_utc().replace_nanosecond(0).unwrap())
    }

    pub fn advance(&self, by: Duration) {
        let mut now = self.now.lock().unwrap_or_else(|e| e.into_inner());
        *now += by;
    }
}

impl Clock for TestClock {
    fn now(&self) -> OffsetDateTime {
        *self.now.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn local_offset(&self) -> UtcOffset {
        *self.offset.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_test_clock_moves_on_and_formats_to_the_second() {
        let c = TestClock::at(time::macros::datetime!(2026-11-10 12:00:00.5 UTC));
        assert_eq!(c.now_rfc3339(), "2026-11-10T12:00:00Z");
        c.advance(Duration::weeks(12));
        assert_eq!(c.now_rfc3339(), "2027-02-02T12:00:00Z");
        assert_eq!(
            parse_rfc3339("2027-02-02T12:00:00Z"),
            Some(c.now().replace_nanosecond(0).unwrap())
        );
    }

    #[test]
    fn the_system_clock_reports_a_plausible_local_offset() {
        let offset = SystemClock.local_offset();
        assert!(offset.whole_hours().abs() <= 14, "{offset}");
    }
}
