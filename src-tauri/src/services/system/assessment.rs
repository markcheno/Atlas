//! Plain-language guidance for the Device check screen.
//!
//! Thresholds are planning targets from the product spec (§7), not
//! measured requirements: Atlas never refuses to run because of them, and
//! the UI presents them as provisional.

use crate::domain::dto::{
    AssessmentNote, DeviceAssessment, DeviceSnapshot, LibraryFit, LocalAiFit, PowerSource,
    PressureLevel,
};
use crate::services::system::low_space_threshold;

const GIB: u64 = 1024 * 1024 * 1024;

pub fn assess(s: &DeviceSnapshot) -> DeviceAssessment {
    let (library, local_ai) = match s.total_memory_bytes {
        None => (LibraryFit::Unknown, LocalAiFit::Unknown),
        // OSes report slightly less than the installed amount.
        Some(bytes) => match bytes / GIB + u64::from(bytes % GIB > GIB / 2) {
            0..=7 => (LibraryFit::Usable, LocalAiFit::Unlikely),
            8..=15 => (LibraryFit::Comfortable, LocalAiFit::Unlikely),
            16..=31 => (LibraryFit::Comfortable, LocalAiFit::Possible),
            _ => (LibraryFit::Comfortable, LocalAiFit::Likely),
        },
    };

    let mut notes = Vec::new();
    if s.total_memory_bytes.is_none() {
        notes.push(AssessmentNote::UnknownMemory);
    }
    match (s.power, s.battery_present) {
        (_, Some(false)) => notes.push(AssessmentNote::Desktop),
        (PowerSource::Battery, _) => notes.push(AssessmentNote::OnBattery),
        (PowerSource::Unknown, _) => notes.push(AssessmentNote::PowerUnknown),
        (PowerSource::Ac, _) => {}
    }
    if let (Some(free), Some(total)) = (s.data_volume_free_bytes, s.data_volume_total_bytes)
        && free < low_space_threshold(total)
    {
        notes.push(AssessmentNote::LowSpace);
    }
    if matches!(
        s.memory_pressure,
        PressureLevel::Elevated | PressureLevel::Critical
    ) {
        notes.push(AssessmentNote::MemoryPressure);
    }

    DeviceAssessment {
        library,
        local_ai,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::dto::{OsKind, ThermalLevel};

    fn snapshot(memory_gib: Option<u64>) -> DeviceSnapshot {
        DeviceSnapshot {
            schema_version: 1,
            os: OsKind::Macos,
            arch: "aarch64".into(),
            logical_cores: Some(8),
            // Real machines report a little under the installed amount.
            total_memory_bytes: memory_gib.map(|g| g * GIB - 200 * 1024 * 1024),
            available_memory_bytes: None,
            data_volume_reachable: None,
            data_volume_free_bytes: None,
            data_volume_total_bytes: None,
            data_volume_kind: None,
            required_free_bytes: None,
            power: PowerSource::Ac,
            battery_present: Some(true),
            battery_percent: Some(90.0),
            charging: Some(false),
            memory_pressure: PressureLevel::Normal,
            thermal: ThermalLevel::Nominal,
            sampled_at: "t".into(),
        }
    }

    #[test]
    fn memory_boundaries() {
        let fit = |g| {
            let a = assess(&snapshot(Some(g)));
            (a.library, a.local_ai)
        };
        assert_eq!(fit(4), (LibraryFit::Usable, LocalAiFit::Unlikely));
        assert_eq!(fit(8), (LibraryFit::Comfortable, LocalAiFit::Unlikely));
        assert_eq!(fit(16), (LibraryFit::Comfortable, LocalAiFit::Possible));
        assert_eq!(fit(32), (LibraryFit::Comfortable, LocalAiFit::Likely));
        assert_eq!(fit(64), (LibraryFit::Comfortable, LocalAiFit::Likely));
        let unknown = assess(&snapshot(None));
        assert_eq!(unknown.library, LibraryFit::Unknown);
        assert!(unknown.notes.contains(&AssessmentNote::UnknownMemory));
    }

    #[test]
    fn notes_reflect_power_space_and_pressure() {
        let on_battery = DeviceSnapshot {
            power: PowerSource::Battery,
            ..snapshot(Some(16))
        };
        assert_eq!(assess(&on_battery).notes, [AssessmentNote::OnBattery]);

        let unknown_power = DeviceSnapshot {
            power: PowerSource::Unknown,
            battery_present: None,
            ..snapshot(Some(16))
        };
        assert_eq!(assess(&unknown_power).notes, [AssessmentNote::PowerUnknown]);

        let desktop = DeviceSnapshot {
            battery_present: Some(false),
            ..snapshot(Some(16))
        };
        assert_eq!(assess(&desktop).notes, [AssessmentNote::Desktop]);

        let cramped = DeviceSnapshot {
            data_volume_free_bytes: Some(GIB),
            data_volume_total_bytes: Some(500 * GIB),
            memory_pressure: PressureLevel::Elevated,
            ..snapshot(Some(16))
        };
        assert_eq!(
            assess(&cramped).notes,
            [AssessmentNote::LowSpace, AssessmentNote::MemoryPressure]
        );
    }
}
