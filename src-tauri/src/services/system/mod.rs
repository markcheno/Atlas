//! Device probes behind `ResourceMonitor`: CPU, memory, data-volume
//! space, power source and battery level, with unknown values reported as
//! unknown rather than guessed.
//!
//! The OS probe lives in `infrastructure::system_probe`; this module holds
//! the shared low-space threshold and the device assessment.

pub mod assessment;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use crate::domain::dto::{
    BackgroundMode, DeviceCheck, FullSpeedBlock, FullSpeedStatus, PauseReason, PolicyDecision,
    PolicyMode, PowerSource, ResourceStatus,
};
use crate::domain::traits::ResourceMonitor;
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::clock::{Clock, SystemClock};
use crate::services::diagnostics::event::FullSpeedEnd;
use crate::services::diagnostics::{Diagnostics, Event};
use crate::services::scheduler::busy::{BusyRule, CpuProbe, NoCpuProbe};
use crate::services::scheduler::policy::{Layers, ResourcePolicy};
use crate::services::scheduler::schedule::{Schedule, local_minute, on_mains};

const GIB: u64 = 1024 * 1024 * 1024;

/// Where the resource status goes when it changes (`resource-status`).
pub type StatusEvents = Arc<dyn Fn(&ResourceStatus) + Send + Sync>;

/// The device monitor, the policy's hysteresis, the owner's schedule, full
/// speed and the busy rule (Slice 4.6). One instance is shared (behind a
/// mutex) by every job runner and the commands, so the UI and the jobs
/// always agree.
pub struct ResourceGovernor {
    pub monitor: Box<dyn ResourceMonitor>,
    pub policy: ResourcePolicy,
    /// Where the policy's changes are recorded (Slice 4.1).
    pub diagnostics: Option<Diagnostics>,
    /// Where "now" and the local time come from, for the hours.
    pub clock: Arc<dyn Clock>,
    /// How busy other applications keep the processor.
    pub cpu: Box<dyn CpuProbe>,
    pub busy: BusyRule,
    pub events: Option<StatusEvents>,
    schedule: Schedule,
    /// Since when full speed has been on. Held in memory only: it ends
    /// when Atlas quits.
    full_speed: Option<Instant>,
    last_root: Option<PathBuf>,
    last: Option<PolicyDecision>,
    last_status: Option<ResourceStatus>,
}

impl ResourceGovernor {
    pub fn new(monitor: Box<dyn ResourceMonitor>) -> Self {
        Self {
            monitor,
            policy: ResourcePolicy::new(),
            diagnostics: None,
            clock: Arc::new(SystemClock),
            cpu: Box::new(NoCpuProbe),
            busy: BusyRule::default(),
            events: None,
            schedule: Schedule::default(),
            full_speed: None,
            last_root: None,
            last: None,
            last_status: None,
        }
    }

    /// Samples the device and decides what background work may do now.
    pub fn check(&mut self, data_root: Option<&Path>) -> DeviceCheck {
        self.check_with_status(data_root).0
    }

    /// The resource status now, for the window.
    pub fn status(&mut self, data_root: Option<&Path>) -> ResourceStatus {
        self.check_with_status(data_root).1
    }

    fn check_with_status(&mut self, data_root: Option<&Path>) -> (DeviceCheck, ResourceStatus) {
        self.last_root = data_root.map(Path::to_path_buf);
        let snapshot = self.monitor.snapshot(data_root);
        let minute = local_minute(self.clock.now(), self.clock.local_offset());
        let hold = self.schedule.hold(minute, &snapshot);
        if self.full_speed.is_some() && snapshot.power == PowerSource::Battery {
            self.end_full_speed(FullSpeedEnd::Unplugged);
        }
        let other_apps_busy = self.busy.update(Instant::now(), self.cpu.as_mut());
        let policy = self.policy.evaluate_with(
            &snapshot,
            Layers {
                hold,
                full_speed: self.full_speed.is_some(),
                other_apps_busy,
            },
        );
        let check = DeviceCheck {
            assessment: assessment::assess(&snapshot),
            policy,
            snapshot,
        };
        self.record(&check.policy);
        let status = self.resource_status(&check, hold);
        if self.last_status.as_ref() != Some(&status) {
            self.last_status = Some(status.clone());
            if let Some(events) = &self.events {
                events(&status);
            }
        }
        (check, status)
    }

    fn record(&mut self, policy: &PolicyDecision) {
        let decision = PolicyDecision {
            max_metadata_ops_per_second: None,
            ..*policy
        };
        if self.last != Some(decision) {
            self.last = Some(decision);
            if let Some(d) = &self.diagnostics {
                d.record(Event::PolicyChanged {
                    mode: decision.mode,
                    pause: decision.pause_reason,
                    throttle: decision.throttle_reason,
                });
            }
        }
    }

    fn resource_status(
        &mut self,
        check: &DeviceCheck,
        hold: Option<PauseReason>,
    ) -> ResourceStatus {
        let s = &check.snapshot;
        let hard = self.policy.hard_pause(s).is_some();
        let blocked = if hard {
            Some(FullSpeedBlock::Held)
        } else if self.schedule.mode == BackgroundMode::Paused {
            Some(FullSpeedBlock::Paused)
        } else if s.power == PowerSource::Battery && s.battery_present != Some(false) {
            Some(FullSpeedBlock::OnBattery)
        } else if !on_mains(s) {
            Some(FullSpeedBlock::PowerUnknown)
        } else {
            None
        };
        ResourceStatus {
            policy: check.policy,
            full_speed: FullSpeedStatus {
                on: self.full_speed.is_some(),
                offered: blocked.is_none(),
                blocked,
            },
            mode: self.schedule.mode,
            waiting_until: (hold == Some(PauseReason::OutsideHours)
                && check.policy.mode == PolicyMode::Paused)
                .then(|| self.schedule.opens_at()),
        }
    }

    /// The schedule from Settings. Paused ends full speed.
    pub fn set_schedule(&mut self, schedule: Schedule) {
        self.schedule = schedule;
        if schedule.mode == BackgroundMode::Paused {
            self.end_full_speed(FullSpeedEnd::Paused);
        }
    }

    pub fn schedule(&self) -> Schedule {
        self.schedule
    }

    pub fn full_speed(&self) -> bool {
        self.full_speed.is_some()
    }

    /// Switches full speed on, if it's offered now. The caller has checked
    /// that work is waiting.
    pub fn start_full_speed(&mut self, data_root: Option<&Path>) -> AtlasResult<ResourceStatus> {
        let status = self.status(data_root);
        if status.full_speed.on {
            return Ok(status);
        }
        match status.full_speed.blocked {
            Some(FullSpeedBlock::OnBattery | FullSpeedBlock::PowerUnknown) => {
                return Err(AtlasError::FullSpeedOnBattery);
            }
            Some(FullSpeedBlock::Paused) => return Err(AtlasError::BackgroundPaused),
            Some(FullSpeedBlock::Held) => return Err(AtlasError::FullSpeedHeld),
            None => {}
        }
        self.full_speed = Some(Instant::now());
        if let Some(d) = &self.diagnostics {
            d.record(Event::FullSpeed {
                on: true,
                ended: None,
                secs: None,
            });
        }
        Ok(self.status(data_root))
    }

    /// Ends full speed, if it's on, and says why in the log. The next check
    /// tells the window.
    pub fn end_full_speed(&mut self, why: FullSpeedEnd) {
        if let Some(since) = self.full_speed.take()
            && let Some(d) = &self.diagnostics
        {
            d.record(Event::FullSpeed {
                on: false,
                ended: Some(why),
                secs: Some(since.elapsed().as_secs()),
            });
        }
    }

    /// Ends full speed and tells the window at once.
    pub fn stop_full_speed(&mut self, why: FullSpeedEnd) -> ResourceStatus {
        self.end_full_speed(why);
        let root = self.last_root.clone();
        self.status(root.as_deref())
    }

    /// Leaves the AI service listening on `ports` out of the busy rule.
    pub fn exclude_ports(&mut self, ports: &[u16]) {
        self.cpu.exclude_ports(ports);
    }
}

/// Samples the device, updates the policy's hysteresis, and assesses.
pub fn device_check(
    monitor: &dyn ResourceMonitor,
    policy: &mut ResourcePolicy,
    data_root: Option<&Path>,
) -> DeviceCheck {
    let snapshot = monitor.snapshot(data_root);
    DeviceCheck {
        assessment: assessment::assess(&snapshot),
        policy: policy.evaluate(&snapshot),
        snapshot,
    }
}

/// Free space below which Atlas starts no new heavy work on a volume:
/// `max(2 GiB, min(10 GiB, 2% of capacity))` (phase0-spec §7.2).
pub fn low_space_threshold(total_bytes: u64) -> u64 {
    (total_bytes / 50).clamp(2 * GIB, 10 * GIB)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold_follows_the_spec_formula() {
        assert_eq!(low_space_threshold(0), 2 * GIB);
        assert_eq!(low_space_threshold(50 * GIB), 2 * GIB);
        assert_eq!(low_space_threshold(250 * GIB), 5 * GIB);
        assert_eq!(low_space_threshold(4096 * GIB), 10 * GIB);
    }
}
