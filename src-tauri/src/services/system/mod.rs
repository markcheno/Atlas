//! Device probes behind `ResourceMonitor`: CPU, memory, data-volume
//! space, power source and battery level, with unknown values reported as
//! unknown rather than guessed.
//!
//! The OS probe lives in `infrastructure::system_probe`; this module holds
//! the shared low-space threshold and the device assessment.

pub mod assessment;

use std::path::Path;

use crate::domain::dto::DeviceCheck;
use crate::domain::traits::ResourceMonitor;
use crate::services::scheduler::policy::ResourcePolicy;

const GIB: u64 = 1024 * 1024 * 1024;

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
