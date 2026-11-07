//! Device probes behind `ResourceMonitor`: CPU, memory, data-volume
//! space, power source and battery level, with unknown values reported as
//! unknown rather than guessed.
//!
//! Slice 0.2 adds only the shared low-space threshold. The probe itself is
//! delivered by Slice 0.4.

const GIB: u64 = 1024 * 1024 * 1024;

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
