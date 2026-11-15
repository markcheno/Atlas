//! Resource policy for background work (phase0-spec §7.2, ADR 0004).
//!
//! A concrete type, not a trait: Phase 0 has one job kind (inventory), so
//! the abstraction budget keeps it concrete. The first matching rule wins:
//!
//! | Condition | Decision |
//! | --- | --- |
//! | Data drive unreachable | paused, `data_root_missing` |
//! | Free space below the low-space threshold | paused, `low_disk_space` |
//! | Critical memory pressure / thermal state | paused |
//! | On battery at ≤20%, or still below 25% after such a pause | paused, `battery_low` |
//! | On battery | slow, `on_battery` |
//! | Power unknown | slow, `power_unknown` (never assumes AC) |
//! | The owner's schedule holds (Slice 4.6) | paused, `paused_by_user`, `not_charging` or `outside_hours` |
//! | Full speed on, on mains power (Slice 4.6) | run at full speed |
//! | Elevated memory pressure or thermal state | slow, `system_busy` |
//! | Other applications keep the processor busy (Slice 4.6) | slow, `other_apps` |
//! | AC power, or a desktop with no battery | run |
//!
//! The schedule sits after the hard pauses and before the slowing rules;
//! full speed sits after the schedule's Paused and Only while charging
//! but overrides its hours (the owner asked now), and overrides every
//! slowing rule. On battery it doesn't apply: it is offered on mains power
//! only, and the governor ends it on unplugging.
//!
//! The battery rule has hysteresis: once paused at 20%, work resumes only
//! at 25% or on AC, so a reading that wobbles around 20% cannot flap.
//! Rates are engineering starting values to be tuned with Slice 0.8
//! benchmarks, not measured guarantees. A user's own Pause is handled by
//! the job state machine (`paused_user`), never here.

use crate::domain::dto::{
    DeviceSnapshot, PauseReason, PolicyDecision, PolicyMode, PowerSource, PressureLevel,
    ThermalLevel, ThrottleReason,
};
use crate::services::scheduler::schedule::on_mains;
use crate::services::system::low_space_threshold;

pub const BATTERY_PAUSE_AT_PERCENT: f32 = 20.0;
pub const BATTERY_RESUME_AT_PERCENT: f32 = 25.0;
/// Metadata operations per second on AC and when slowed. Chosen from the
/// Slice 0.8 measurements (docs/benchmarks.md, ADR 0004): about 7% of one
/// core on an M5 Pro at 30,000 files, leaving headroom for slower machines.
pub const AC_OPS_PER_SECOND: u32 = 500;
pub const SLOW_OPS_PER_SECOND: u32 = AC_OPS_PER_SECOND / 5;
/// An entry a resumed scan lists again, already written and unchanged,
/// costs this fraction of an operation (1/N): it is stat'ed but writes
/// nothing. Resuming 9,000 entries into a folder then waits about 0.9 s at
/// the AC rate instead of 18 s (P1-20, ADR 0004).
pub const REPLAY_COST_DIVISOR: u32 = 20;

#[derive(Debug, Default)]
pub struct ResourcePolicy {
    paused_for_battery: bool,
}

/// What the governor adds to the device's reading (Slice 4.6).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Layers {
    /// Why the owner's schedule holds work now, if it does.
    pub hold: Option<PauseReason>,
    /// Run at full speed now.
    pub full_speed: bool,
    /// Other applications keep the processor busy.
    pub other_apps_busy: bool,
}

impl ResourcePolicy {
    pub fn new() -> Self {
        Self::default()
    }

    /// The device's rules alone (ADR 0004).
    pub fn evaluate(&mut self, s: &DeviceSnapshot) -> PolicyDecision {
        self.evaluate_with(s, Layers::default())
    }

    /// Whether a hard pause holds every job now, whatever the owner chose.
    pub fn hard_pause(&mut self, s: &DeviceSnapshot) -> Option<PauseReason> {
        let d = self.evaluate(s);
        (d.mode == PolicyMode::Paused)
            .then_some(d.pause_reason)
            .flatten()
    }

    /// The device's rules with the schedule, full speed and the busy rule
    /// in the order the table above gives (phase4-spec §7.5).
    pub fn evaluate_with(&mut self, s: &DeviceSnapshot, layers: Layers) -> PolicyDecision {
        // Track battery hysteresis on every sample, whatever rule wins, so
        // the state is right when higher-priority conditions clear.
        let on_battery = s.power == PowerSource::Battery;
        let percent = s.battery_percent;
        self.paused_for_battery = match (on_battery, percent) {
            (false, _) => false,
            (true, Some(p)) if p <= BATTERY_PAUSE_AT_PERCENT => true,
            (true, Some(p)) if p >= BATTERY_RESUME_AT_PERCENT => false,
            (true, Some(_)) => self.paused_for_battery,
            // On battery with no level: keep whatever was decided before.
            (true, None) => self.paused_for_battery,
        };

        if s.data_volume_reachable == Some(false) {
            return paused(PauseReason::DataRootMissing);
        }
        if let (Some(free), Some(total)) = (s.data_volume_free_bytes, s.data_volume_total_bytes)
            && free < low_space_threshold(total)
        {
            return paused(PauseReason::LowDiskSpace);
        }
        if s.memory_pressure == PressureLevel::Critical {
            return paused(PauseReason::MemoryPressure);
        }
        if s.thermal == ThermalLevel::Critical {
            return paused(PauseReason::ThermalPressure);
        }
        if self.paused_for_battery {
            return paused(PauseReason::BatteryLow);
        }
        match layers.hold {
            // Full speed overrides the hours: the owner asked now.
            Some(PauseReason::OutsideHours) if layers.full_speed && on_mains(s) => {}
            Some(reason) => return paused(reason),
            None => {}
        }
        if layers.full_speed && on_mains(s) && !on_battery {
            return PolicyDecision {
                mode: PolicyMode::Run,
                pause_reason: None,
                throttle_reason: None,
                max_metadata_ops_per_second: None,
                full_speed: true,
            };
        }
        if on_battery {
            return slow(ThrottleReason::OnBattery);
        }
        if s.power == PowerSource::Unknown {
            return slow(ThrottleReason::PowerUnknown);
        }
        if s.memory_pressure == PressureLevel::Elevated || s.thermal == ThermalLevel::Elevated {
            return slow(ThrottleReason::SystemBusy);
        }
        if layers.other_apps_busy {
            return slow(ThrottleReason::OtherApps);
        }
        PolicyDecision {
            mode: PolicyMode::Run,
            pause_reason: None,
            throttle_reason: None,
            max_metadata_ops_per_second: Some(AC_OPS_PER_SECOND),
            full_speed: false,
        }
    }
}

fn paused(reason: PauseReason) -> PolicyDecision {
    PolicyDecision {
        mode: PolicyMode::Paused,
        pause_reason: Some(reason),
        throttle_reason: None,
        max_metadata_ops_per_second: None,
        full_speed: false,
    }
}

fn slow(reason: ThrottleReason) -> PolicyDecision {
    PolicyDecision {
        mode: PolicyMode::Slow,
        pause_reason: None,
        throttle_reason: Some(reason),
        max_metadata_ops_per_second: Some(SLOW_OPS_PER_SECOND),
        full_speed: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::dto::OsKind;

    const GIB: u64 = 1024 * 1024 * 1024;

    fn ac() -> DeviceSnapshot {
        DeviceSnapshot {
            schema_version: 1,
            os: OsKind::Macos,
            arch: "aarch64".into(),
            logical_cores: Some(8),
            total_memory_bytes: Some(16 * GIB),
            available_memory_bytes: Some(8 * GIB),
            data_volume_reachable: Some(true),
            data_volume_free_bytes: Some(200 * GIB),
            data_volume_total_bytes: Some(1000 * GIB),
            data_volume_kind: None,
            required_free_bytes: None,
            power: PowerSource::Ac,
            battery_present: Some(true),
            battery_percent: Some(100.0),
            charging: Some(false),
            memory_pressure: PressureLevel::Normal,
            thermal: ThermalLevel::Nominal,
            sampled_at: "t".into(),
        }
    }

    fn battery(percent: f32) -> DeviceSnapshot {
        DeviceSnapshot {
            power: PowerSource::Battery,
            battery_percent: Some(percent),
            charging: Some(false),
            ..ac()
        }
    }

    fn mode(d: PolicyDecision) -> (PolicyMode, Option<PauseReason>, Option<ThrottleReason>) {
        (d.mode, d.pause_reason, d.throttle_reason)
    }

    #[test]
    fn ac_and_desktop_run_at_full_rate() {
        let mut p = ResourcePolicy::new();
        let d = p.evaluate(&ac());
        assert_eq!(d.mode, PolicyMode::Run);
        assert_eq!(d.max_metadata_ops_per_second, Some(AC_OPS_PER_SECOND));

        let desktop = DeviceSnapshot {
            battery_present: Some(false),
            battery_percent: None,
            charging: None,
            ..ac()
        };
        assert_eq!(p.evaluate(&desktop).mode, PolicyMode::Run);
    }

    /// P0-10: pause at 20%, no flapping, resume at 25% or on AC.
    #[test]
    fn p0_10_battery_hysteresis() {
        let mut p = ResourcePolicy::new();
        let slow_battery = (PolicyMode::Slow, None, Some(ThrottleReason::OnBattery));
        let paused_battery = (PolicyMode::Paused, Some(PauseReason::BatteryLow), None);

        let d = p.evaluate(&battery(50.0));
        assert_eq!(mode(d), slow_battery);
        assert_eq!(d.max_metadata_ops_per_second, Some(SLOW_OPS_PER_SECOND));
        assert_eq!(mode(p.evaluate(&battery(21.0))), slow_battery);
        assert_eq!(mode(p.evaluate(&battery(20.0))), paused_battery);
        for wobble in [19.0, 20.0, 21.0, 24.0, 24.9] {
            assert_eq!(
                mode(p.evaluate(&battery(wobble))),
                paused_battery,
                "at {wobble}%"
            );
        }
        assert_eq!(mode(p.evaluate(&battery(25.0))), slow_battery);

        // Plugging in clears the pause immediately, even at a low level.
        assert_eq!(mode(p.evaluate(&battery(18.0))), paused_battery);
        let plugged = DeviceSnapshot {
            power: PowerSource::Ac,
            battery_percent: Some(18.0),
            charging: Some(true),
            ..ac()
        };
        assert_eq!(p.evaluate(&plugged).mode, PolicyMode::Run);
        // Unplugging at 22% starts fresh: above the pause threshold.
        assert_eq!(mode(p.evaluate(&battery(22.0))), slow_battery);
    }

    /// P0-11: unknown power is conservative and never treated as AC.
    #[test]
    fn p0_11_unknown_power_is_conservative() {
        let mut p = ResourcePolicy::new();
        let unknown = DeviceSnapshot {
            power: PowerSource::Unknown,
            battery_present: None,
            battery_percent: None,
            charging: None,
            ..ac()
        };
        let d = p.evaluate(&unknown);
        assert_eq!(
            mode(d),
            (PolicyMode::Slow, None, Some(ThrottleReason::PowerUnknown))
        );
        assert_eq!(d.max_metadata_ops_per_second, Some(SLOW_OPS_PER_SECOND));
    }

    /// P0-12: below the low-space threshold, new work pauses.
    #[test]
    fn p0_12_low_space_pauses_and_outranks_battery() {
        let mut p = ResourcePolicy::new();
        let total = 1000 * GIB;
        let threshold = low_space_threshold(total);
        let at = |free| DeviceSnapshot {
            data_volume_free_bytes: Some(free),
            data_volume_total_bytes: Some(total),
            ..ac()
        };
        assert_eq!(
            mode(p.evaluate(&at(threshold - 1))),
            (PolicyMode::Paused, Some(PauseReason::LowDiskSpace), None)
        );
        assert_eq!(p.evaluate(&at(threshold)).mode, PolicyMode::Run);

        let low_and_on_battery = DeviceSnapshot {
            data_volume_free_bytes: Some(threshold - 1),
            data_volume_total_bytes: Some(total),
            ..battery(10.0)
        };
        assert_eq!(
            p.evaluate(&low_and_on_battery).pause_reason,
            Some(PauseReason::LowDiskSpace)
        );
        // The battery pause was still recorded underneath.
        let space_freed = DeviceSnapshot {
            data_volume_free_bytes: Some(threshold * 2),
            data_volume_total_bytes: Some(total),
            ..battery(22.0)
        };
        assert_eq!(
            p.evaluate(&space_freed).pause_reason,
            Some(PauseReason::BatteryLow)
        );
    }

    #[test]
    fn missing_data_drive_pauses_but_no_root_does_not() {
        let mut p = ResourcePolicy::new();
        let unplugged = DeviceSnapshot {
            data_volume_reachable: Some(false),
            data_volume_free_bytes: None,
            data_volume_total_bytes: None,
            ..ac()
        };
        assert_eq!(
            p.evaluate(&unplugged).pause_reason,
            Some(PauseReason::DataRootMissing)
        );

        let no_root = DeviceSnapshot {
            data_volume_reachable: None,
            data_volume_free_bytes: None,
            data_volume_total_bytes: None,
            ..ac()
        };
        assert_eq!(p.evaluate(&no_root).mode, PolicyMode::Run);
    }

    #[test]
    fn pressure_and_thermal() {
        let mut p = ResourcePolicy::new();
        let critical_memory = DeviceSnapshot {
            memory_pressure: PressureLevel::Critical,
            ..ac()
        };
        assert_eq!(
            p.evaluate(&critical_memory).pause_reason,
            Some(PauseReason::MemoryPressure)
        );
        let hot = DeviceSnapshot {
            thermal: ThermalLevel::Critical,
            ..ac()
        };
        assert_eq!(
            p.evaluate(&hot).pause_reason,
            Some(PauseReason::ThermalPressure)
        );
        let busy = DeviceSnapshot {
            memory_pressure: PressureLevel::Elevated,
            ..ac()
        };
        assert_eq!(
            mode(p.evaluate(&busy)),
            (PolicyMode::Slow, None, Some(ThrottleReason::SystemBusy))
        );
        let unknown_readings = DeviceSnapshot {
            memory_pressure: PressureLevel::Unknown,
            thermal: ThermalLevel::Unknown,
            ..ac()
        };
        assert_eq!(p.evaluate(&unknown_readings).mode, PolicyMode::Run);
    }

    /// P4-13/P4-14: the schedule, full speed and the busy rule, in order.
    #[test]
    fn p4_14_the_layers_in_order() {
        let mut p = ResourcePolicy::new();
        let layers = |hold, full_speed, other_apps_busy| Layers {
            hold,
            full_speed,
            other_apps_busy,
        };
        let full = |d: PolicyDecision| (d.mode, d.full_speed, d.max_metadata_ops_per_second);

        // Full speed: run, unthrottled, over every slowing rule.
        let busy_ac = DeviceSnapshot {
            memory_pressure: PressureLevel::Elevated,
            ..ac()
        };
        assert_eq!(
            full(p.evaluate_with(&busy_ac, layers(None, true, true))),
            (PolicyMode::Run, true, None)
        );
        // ...and over the hours, but not over Paused or a hard pause.
        assert!(
            p.evaluate_with(&ac(), layers(Some(PauseReason::OutsideHours), true, false))
                .full_speed
        );
        assert_eq!(
            p.evaluate_with(&ac(), layers(Some(PauseReason::PausedByUser), true, false))
                .pause_reason,
            Some(PauseReason::PausedByUser)
        );
        let low = DeviceSnapshot {
            data_volume_free_bytes: Some(GIB),
            data_volume_total_bytes: Some(1000 * GIB),
            ..ac()
        };
        assert_eq!(
            p.evaluate_with(&low, layers(None, true, false))
                .pause_reason,
            Some(PauseReason::LowDiskSpace)
        );
        // On battery or unknown power it doesn't apply.
        let d = p.evaluate_with(&battery(80.0), layers(None, true, false));
        assert_eq!((d.mode, d.full_speed), (PolicyMode::Slow, false));
        let unknown = DeviceSnapshot {
            power: PowerSource::Unknown,
            battery_present: None,
            ..ac()
        };
        assert!(
            !p.evaluate_with(&unknown, layers(None, true, false))
                .full_speed
        );
        // A desktop with no battery counts as mains power.
        let desktop = DeviceSnapshot {
            power: PowerSource::Unknown,
            battery_present: Some(false),
            battery_percent: None,
            ..ac()
        };
        assert!(
            p.evaluate_with(&desktop, layers(None, true, false))
                .full_speed
        );

        // The schedule holds after the hard pauses...
        assert_eq!(
            mode(p.evaluate_with(&ac(), layers(Some(PauseReason::NotCharging), false, false))),
            (PolicyMode::Paused, Some(PauseReason::NotCharging), None)
        );
        assert_eq!(
            p.evaluate_with(&low, layers(Some(PauseReason::OutsideHours), false, false))
                .pause_reason,
            Some(PauseReason::LowDiskSpace)
        );
        // ...and other applications slow work last.
        assert_eq!(
            mode(p.evaluate_with(&ac(), layers(None, false, true))),
            (PolicyMode::Slow, None, Some(ThrottleReason::OtherApps))
        );
        assert_eq!(
            p.evaluate_with(&battery(80.0), layers(None, false, true))
                .throttle_reason,
            Some(ThrottleReason::OnBattery)
        );
        assert_eq!(p.evaluate_with(&ac(), Layers::default()), p.evaluate(&ac()));
    }
}
