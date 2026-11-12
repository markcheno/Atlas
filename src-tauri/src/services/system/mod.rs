//! Device probes behind `ResourceMonitor`: CPU, memory, data-volume
//! space, power source and battery level, with unknown values reported as
//! unknown rather than guessed.
//!
//! The OS probe lives in `infrastructure::system_probe`; this module holds
//! the shared low-space threshold and the device assessment.

pub mod assessment;

use std::path::Path;

use crate::domain::dto::{DeviceCheck, PolicyDecision};
use crate::domain::traits::ResourceMonitor;
use crate::services::diagnostics::{Diagnostics, Event};
use crate::services::scheduler::policy::ResourcePolicy;

const GIB: u64 = 1024 * 1024 * 1024;

/// The device monitor plus the policy's hysteresis state. One instance is
/// shared (behind a mutex) by the inventory worker and the device-check
/// command, so the UI and the worker always agree.
pub struct ResourceGovernor {
    pub monitor: Box<dyn ResourceMonitor>,
    pub policy: ResourcePolicy,
    /// Where the policy's changes are recorded (Slice 4.1).
    pub diagnostics: Option<Diagnostics>,
    last: Option<PolicyDecision>,
}

impl ResourceGovernor {
    pub fn new(monitor: Box<dyn ResourceMonitor>) -> Self {
        Self {
            monitor,
            policy: ResourcePolicy::new(),
            diagnostics: None,
            last: None,
        }
    }

    pub fn check(&mut self, data_root: Option<&Path>) -> DeviceCheck {
        let check = device_check(self.monitor.as_ref(), &mut self.policy, data_root);
        let decision = PolicyDecision {
            max_metadata_ops_per_second: None,
            ..check.policy
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
        check
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
