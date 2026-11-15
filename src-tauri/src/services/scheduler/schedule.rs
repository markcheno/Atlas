//! The owner's schedule for background work (phase4-spec §7.5, Slice 4.6):
//! Automatic, Only while charging, Only between two local times, Paused.
//!
//! Pure functions of the settings, the local time and the device reading;
//! the governor applies them after ADR 0004's hard pauses and before its
//! slowing rules.

use time::{OffsetDateTime, UtcOffset};

use crate::domain::dto::{
    BackgroundMode, BackgroundSettings, DEFAULT_FROM, DEFAULT_TO, DeviceSnapshot, PauseReason,
    PowerSource, minute_of_day,
};

/// The schedule in force, from Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Schedule {
    pub mode: BackgroundMode,
    /// The window's start and end, in minutes after local midnight.
    pub from: u16,
    pub to: u16,
}

impl Default for Schedule {
    fn default() -> Self {
        Self::from_settings(&BackgroundSettings::default())
    }
}

impl Schedule {
    pub fn from_settings(b: &BackgroundSettings) -> Self {
        let minute = |t: &str, default: &str| {
            minute_of_day(t)
                .or_else(|| minute_of_day(default))
                .unwrap_or(0)
        };
        Self {
            mode: b.mode,
            from: minute(&b.from, DEFAULT_FROM),
            to: minute(&b.to, DEFAULT_TO),
        }
    }

    /// Why the schedule holds background work now, if it does.
    pub fn hold(&self, minute: u16, s: &DeviceSnapshot) -> Option<PauseReason> {
        match self.mode {
            BackgroundMode::Automatic => None,
            BackgroundMode::Paused => Some(PauseReason::PausedByUser),
            BackgroundMode::Charging => (!on_mains(s)).then_some(PauseReason::NotCharging),
            BackgroundMode::Hours => (!self.in_window(minute)).then_some(PauseReason::OutsideHours),
        }
    }

    /// Whether `minute` is inside the window. A window from 22:00 to 07:00
    /// crosses midnight. One whose ends are equal can't be saved; read from
    /// a hand-edited file it holds nothing back.
    pub fn in_window(&self, minute: u16) -> bool {
        use std::cmp::Ordering::*;
        match self.from.cmp(&self.to) {
            Less => (self.from..self.to).contains(&minute),
            Greater => minute >= self.from || minute < self.to,
            Equal => true,
        }
    }

    /// The window's start as "HH:MM", for "Waiting until 22:00".
    pub fn opens_at(&self) -> String {
        format!("{:02}:{:02}", self.from / 60, self.from % 60)
    }
}

/// On mains power, or a computer with no battery at all. An unknown power
/// source is not mains power: Atlas never assumes it (ADR 0004).
pub fn on_mains(s: &DeviceSnapshot) -> bool {
    s.battery_present == Some(false) || s.power == PowerSource::Ac
}

/// Minutes after local midnight at `now`.
pub fn local_minute(now: OffsetDateTime, offset: UtcOffset) -> u16 {
    let t = now.to_offset(offset);
    u16::from(t.hour()) * 60 + u16::from(t.minute())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::dto::{OsKind, PressureLevel, ThermalLevel};

    fn snapshot(power: PowerSource, battery: Option<bool>) -> DeviceSnapshot {
        DeviceSnapshot {
            schema_version: 1,
            os: OsKind::Macos,
            arch: "aarch64".into(),
            logical_cores: Some(8),
            total_memory_bytes: None,
            available_memory_bytes: None,
            data_volume_reachable: Some(true),
            data_volume_free_bytes: None,
            data_volume_total_bytes: None,
            data_volume_kind: None,
            required_free_bytes: None,
            power,
            battery_present: battery,
            battery_percent: battery.and_then(|b| b.then_some(80.0)),
            charging: None,
            memory_pressure: PressureLevel::Normal,
            thermal: ThermalLevel::Nominal,
            sampled_at: "t".into(),
        }
    }

    fn schedule(mode: BackgroundMode, from: &str, to: &str) -> Schedule {
        Schedule::from_settings(&BackgroundSettings {
            mode,
            from: from.into(),
            to: to.into(),
            ..BackgroundSettings::default()
        })
    }

    const H: u16 = 60;

    /// P4-13: each mode against each power source, and the window.
    #[test]
    fn p4_13_each_mode_holds_for_its_reason() {
        let laptop_ac = snapshot(PowerSource::Ac, Some(true));
        let battery = snapshot(PowerSource::Battery, Some(true));
        let unknown = snapshot(PowerSource::Unknown, None);
        let desktop = snapshot(PowerSource::Unknown, Some(false));
        let all = [&laptop_ac, &battery, &unknown, &desktop];

        let automatic = schedule(BackgroundMode::Automatic, "22:00", "07:00");
        let paused = schedule(BackgroundMode::Paused, "22:00", "07:00");
        for s in all {
            for minute in [0, 12 * H, 23 * H + 59] {
                assert_eq!(automatic.hold(minute, s), None);
                assert_eq!(paused.hold(minute, s), Some(PauseReason::PausedByUser));
            }
        }

        let charging = schedule(BackgroundMode::Charging, "22:00", "07:00");
        assert_eq!(charging.hold(0, &laptop_ac), None);
        assert_eq!(charging.hold(0, &battery), Some(PauseReason::NotCharging));
        assert_eq!(charging.hold(0, &unknown), Some(PauseReason::NotCharging));
        // A desktop is never "waiting for power", whatever the probe says.
        assert_eq!(charging.hold(0, &desktop), None);
        assert_eq!(
            charging.hold(0, &snapshot(PowerSource::Ac, Some(false))),
            None
        );

        // A window across midnight, at its edges.
        let night = schedule(BackgroundMode::Hours, "22:00", "07:00");
        for (minute, inside) in [
            (21 * H + 59, false),
            (22 * H, true),
            (23 * H + 59, true),
            (0, true),
            (6 * H + 59, true),
            (7 * H, false),
            (12 * H, false),
        ] {
            let want = (!inside).then_some(PauseReason::OutsideHours);
            for s in all {
                assert_eq!(night.hold(minute, s), want, "{minute}");
            }
        }
        // A window within one day.
        let day = schedule(BackgroundMode::Hours, "09:30", "17:00");
        assert_eq!(
            day.hold(9 * H + 29, &laptop_ac),
            Some(PauseReason::OutsideHours)
        );
        assert_eq!(day.hold(9 * H + 30, &laptop_ac), None);
        assert_eq!(day.hold(16 * H + 59, &laptop_ac), None);
        assert_eq!(
            day.hold(17 * H, &laptop_ac),
            Some(PauseReason::OutsideHours)
        );
        // Equal ends (only from a hand-edited file) hold nothing back.
        assert_eq!(
            schedule(BackgroundMode::Hours, "08:00", "08:00").hold(0, &battery),
            None
        );
    }

    #[test]
    fn times_are_read_leniently_and_shown_as_written() {
        let s = schedule(BackgroundMode::Hours, "6:05", "nonsense");
        assert_eq!((s.from, s.to), (6 * H + 5, 7 * H));
        assert_eq!(s.opens_at(), "06:05");
        assert_eq!(Schedule::default().opens_at(), "22:00");
        for bad in ["24:00", "12:60", "12", "12:5", ":30", "ab:cd", ""] {
            assert_eq!(minute_of_day(bad), None, "{bad}");
        }
    }

    #[test]
    fn local_minute_follows_the_offset() {
        let t = time::macros::datetime!(2026-11-15 23:30 UTC);
        assert_eq!(local_minute(t, UtcOffset::UTC), 23 * H + 30);
        let minus_six = UtcOffset::from_hms(-6, 0, 0).unwrap();
        assert_eq!(local_minute(t, minus_six), 17 * H + 30);
        let plus_five_thirty = UtcOffset::from_hms(5, 30, 0).unwrap();
        assert_eq!(local_minute(t, plus_five_thirty), 5 * H);
    }
}
