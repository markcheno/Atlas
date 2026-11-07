//! OS implementation of [`ResourceMonitor`] (phase0-spec §8).
//!
//! Each reading comes from its own sub-probe, and a failing sub-probe only
//! makes its own fields unknown. Nothing here waits or sleeps, so one
//! sample is cheap enough to take on every request.

use std::path::Path;
use std::sync::Arc;

use crate::domain::dto::{
    DeviceSnapshot, OsKind, PowerSource, PressureLevel, ThermalLevel, VolumeKind,
};
use crate::domain::traits::{ResourceMonitor, VolumeInspector};
use crate::infrastructure::clock::now_rfc3339;
use crate::services::system::low_space_threshold;

pub const SNAPSHOT_SCHEMA_VERSION: u32 = 1;

pub struct OsResourceMonitor {
    volumes: Arc<dyn VolumeInspector>,
}

impl OsResourceMonitor {
    pub fn new(volumes: Arc<dyn VolumeInspector>) -> Self {
        Self { volumes }
    }
}

impl ResourceMonitor for OsResourceMonitor {
    fn snapshot(&self, data_root: Option<&Path>) -> DeviceSnapshot {
        let (logical_cores, total_memory_bytes, available_memory_bytes) = memory_and_cores();
        let power = power();
        let mut snapshot = DeviceSnapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            os: OsKind::current(),
            arch: std::env::consts::ARCH.to_owned(),
            logical_cores,
            total_memory_bytes,
            available_memory_bytes,
            data_volume_reachable: None,
            data_volume_free_bytes: None,
            data_volume_total_bytes: None,
            data_volume_kind: None,
            required_free_bytes: None,
            power: power.source,
            battery_present: power.present,
            battery_percent: power.percent,
            charging: power.charging,
            memory_pressure: platform::memory_pressure(),
            thermal: platform::thermal(),
            sampled_at: now_rfc3339(),
        };
        if let Some(root) = data_root {
            fill_data_volume(&mut snapshot, root, self.volumes.as_ref());
        }
        snapshot
    }
}

/// Fills the data-volume fields. An unreachable root (unplugged drive) is
/// reported as such, never as an empty volume.
pub fn fill_data_volume(snapshot: &mut DeviceSnapshot, root: &Path, volumes: &dyn VolumeInspector) {
    let reachable = root.is_dir();
    snapshot.data_volume_reachable = Some(reachable);
    if !reachable {
        return;
    }
    if let Ok(info) = volumes.inspect(root) {
        snapshot.data_volume_free_bytes = info.free_bytes;
        snapshot.data_volume_total_bytes = info.total_bytes;
        snapshot.data_volume_kind = Some(info.kind);
        snapshot.required_free_bytes = info.total_bytes.map(low_space_threshold);
    } else {
        snapshot.data_volume_kind = Some(VolumeKind::Unknown);
    }
}

fn memory_and_cores() -> (Option<u32>, Option<u64>, Option<u64>) {
    use sysinfo::{MemoryRefreshKind, RefreshKind, System};
    let cores = std::thread::available_parallelism()
        .ok()
        .and_then(|n| u32::try_from(n.get()).ok());
    let Ok(system) = System::new_with_specifics(
        RefreshKind::nothing().with_memory(MemoryRefreshKind::nothing().with_ram()),
    ) else {
        return (cores, None, None);
    };
    let total = Some(system.total_memory()).filter(|&b| b > 0);
    let available = total.map(|_| system.available_memory());
    (cores, total, available)
}

struct PowerReading {
    source: PowerSource,
    present: Option<bool>,
    percent: Option<f32>,
    charging: Option<bool>,
}

const UNKNOWN_POWER: PowerReading = PowerReading {
    source: PowerSource::Unknown,
    present: None,
    percent: None,
    charging: None,
};

/// No battery at all means a desktop on AC. A battery whose state cannot be
/// read makes the power source unknown, never AC.
fn power() -> PowerReading {
    use starship_battery::{Manager, State};
    let Ok(manager) = Manager::new() else {
        return UNKNOWN_POWER;
    };
    let Ok(batteries) = manager.batteries() else {
        return UNKNOWN_POWER;
    };
    let mut states = Vec::new();
    let mut charges = Vec::new();
    for battery in batteries {
        let Ok(battery) = battery else {
            return UNKNOWN_POWER;
        };
        states.push(battery.state());
        charges.push(battery.state_of_charge().value);
    }
    if states.is_empty() {
        return PowerReading {
            source: PowerSource::Ac,
            present: Some(false),
            percent: None,
            charging: None,
        };
    }
    let percent =
        Some((charges.iter().sum::<f32>() / charges.len() as f32 * 100.0).clamp(0.0, 100.0));
    let any = |s: State| states.contains(&s);
    let (source, charging) = if any(State::Discharging) || any(State::Empty) {
        (PowerSource::Battery, Some(false))
    } else if any(State::Charging) {
        (PowerSource::Ac, Some(true))
    } else if states.iter().all(|s| *s == State::Full) {
        (PowerSource::Ac, Some(false))
    } else {
        // `Unknown` is also what some Macs report when plugged in but
        // holding charge; Atlas reports it truthfully as unknown.
        (PowerSource::Unknown, None)
    };
    PowerReading {
        source,
        present: Some(true),
        percent,
        charging,
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;

    /// `kern.memorystatus_vm_pressure_level`: 1 normal, 2 warning, 4 critical.
    pub fn memory_pressure() -> PressureLevel {
        let mut level: libc::c_int = 0;
        let mut size = std::mem::size_of::<libc::c_int>();
        // SAFETY: the name is NUL-terminated; `level` and `size` describe a
        // valid writable buffer of the size sysctl expects for this key.
        let rc = unsafe {
            libc::sysctlbyname(
                c"kern.memorystatus_vm_pressure_level".as_ptr(),
                (&mut level as *mut libc::c_int).cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        };
        match (rc, level) {
            (0, 1) => PressureLevel::Normal,
            (0, 2) => PressureLevel::Elevated,
            (0, 4) => PressureLevel::Critical,
            _ => PressureLevel::Unknown,
        }
    }

    pub fn thermal() -> ThermalLevel {
        use objc2_foundation::{NSProcessInfo, NSProcessInfoThermalState};
        let state = NSProcessInfo::processInfo().thermalState();
        match state {
            NSProcessInfoThermalState::Nominal => ThermalLevel::Nominal,
            NSProcessInfoThermalState::Fair => ThermalLevel::Elevated,
            NSProcessInfoThermalState::Serious | NSProcessInfoThermalState::Critical => {
                ThermalLevel::Critical
            }
            _ => ThermalLevel::Unknown,
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;

    /// Pressure Stall Information. Thresholds are provisional.
    pub fn memory_pressure() -> PressureLevel {
        let Ok(text) = std::fs::read_to_string("/proc/pressure/memory") else {
            return PressureLevel::Unknown;
        };
        parse_psi(&text)
    }

    pub fn parse_psi(text: &str) -> PressureLevel {
        let avg10 = |kind: &str| -> Option<f32> {
            text.lines()
                .find(|l| l.starts_with(kind))?
                .split_whitespace()
                .find_map(|field| field.strip_prefix("avg10="))?
                .parse()
                .ok()
        };
        match (avg10("some"), avg10("full")) {
            (_, Some(full)) if full >= 10.0 => PressureLevel::Critical,
            (Some(some), _) if some >= 10.0 => PressureLevel::Elevated,
            (Some(_), _) => PressureLevel::Normal,
            _ => PressureLevel::Unknown,
        }
    }

    pub fn thermal() -> ThermalLevel {
        ThermalLevel::Unknown
    }

    #[cfg(test)]
    #[test]
    fn psi_parsing() {
        let calm = "some avg10=0.00 avg60=0.00 avg300=0.00 total=1\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=1";
        assert_eq!(parse_psi(calm), PressureLevel::Normal);
        let busy =
            "some avg10=12.50 avg60=3 avg300=1 total=1\nfull avg10=2.00 avg60=0 avg300=0 total=1";
        assert_eq!(parse_psi(busy), PressureLevel::Elevated);
        let stalled =
            "some avg10=40 avg60=3 avg300=1 total=1\nfull avg10=15 avg60=0 avg300=0 total=1";
        assert_eq!(parse_psi(stalled), PressureLevel::Critical);
        assert_eq!(parse_psi(""), PressureLevel::Unknown);
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod platform {
    use super::*;
    pub fn memory_pressure() -> PressureLevel {
        PressureLevel::Unknown
    }
    pub fn thermal() -> ThermalLevel {
        ThermalLevel::Unknown
    }
}

/// Debug-build overrides for demonstrating policy states on real hardware
/// (`ATLAS_SIMULATE_POWER`, `ATLAS_SIMULATE_FREE_BYTES`). Release builds
/// ignore them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Simulation {
    pub power: Option<SimulatedPower>,
    pub free_bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SimulatedPower {
    Ac,
    Battery(f32),
    Unknown,
    Desktop,
}

pub const SIMULATE_POWER_ENV: &str = "ATLAS_SIMULATE_POWER";
pub const SIMULATE_FREE_BYTES_ENV: &str = "ATLAS_SIMULATE_FREE_BYTES";

impl Simulation {
    pub fn from_env() -> Self {
        if !cfg!(debug_assertions) {
            return Self::default();
        }
        Self {
            power: std::env::var(SIMULATE_POWER_ENV)
                .ok()
                .and_then(|v| Self::parse_power(&v)),
            free_bytes: std::env::var(SIMULATE_FREE_BYTES_ENV)
                .ok()
                .and_then(|v| v.trim().parse().ok()),
        }
    }

    pub fn parse_power(value: &str) -> Option<SimulatedPower> {
        match value.trim() {
            "ac" => Some(SimulatedPower::Ac),
            "unknown" => Some(SimulatedPower::Unknown),
            "desktop" => Some(SimulatedPower::Desktop),
            other => other
                .strip_prefix("battery:")
                .and_then(|p| p.parse::<f32>().ok())
                .filter(|p| (0.0..=100.0).contains(p))
                .map(SimulatedPower::Battery),
        }
    }

    pub fn is_active(&self) -> bool {
        self.power.is_some() || self.free_bytes.is_some()
    }

    pub fn apply(&self, snapshot: &mut DeviceSnapshot) {
        match self.power {
            Some(SimulatedPower::Ac) => {
                snapshot.power = PowerSource::Ac;
                snapshot.battery_present = Some(true);
                snapshot.battery_percent = Some(100.0);
                snapshot.charging = Some(false);
            }
            Some(SimulatedPower::Battery(pct)) => {
                snapshot.power = PowerSource::Battery;
                snapshot.battery_present = Some(true);
                snapshot.battery_percent = Some(pct);
                snapshot.charging = Some(false);
            }
            Some(SimulatedPower::Unknown) => {
                snapshot.power = PowerSource::Unknown;
                snapshot.battery_present = None;
                snapshot.battery_percent = None;
                snapshot.charging = None;
            }
            Some(SimulatedPower::Desktop) => {
                snapshot.power = PowerSource::Ac;
                snapshot.battery_present = Some(false);
                snapshot.battery_percent = None;
                snapshot.charging = None;
            }
            None => {}
        }
        if let Some(free) = self.free_bytes
            && snapshot.data_volume_reachable == Some(true)
        {
            snapshot.data_volume_free_bytes = Some(free);
        }
    }
}

/// Wraps any monitor with a [`Simulation`].
pub struct SimulatedMonitor<M> {
    pub inner: M,
    pub simulation: Simulation,
}

impl<M: ResourceMonitor> ResourceMonitor for SimulatedMonitor<M> {
    fn snapshot(&self, data_root: Option<&Path>) -> DeviceSnapshot {
        let mut snapshot = self.inner.snapshot(data_root);
        self.simulation.apply(&mut snapshot);
        snapshot
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::volume::OsVolumeInspector;
    use std::time::{Duration, Instant};

    fn monitor() -> OsResourceMonitor {
        OsResourceMonitor::new(Arc::new(OsVolumeInspector))
    }

    #[test]
    fn real_probe_reports_plausible_values_quickly() {
        let dir = tempfile::tempdir().unwrap();
        let started = Instant::now();
        let s = monitor().snapshot(Some(dir.path()));
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "{:?}",
            started.elapsed()
        );

        assert_eq!(s.os, OsKind::current());
        assert!(s.logical_cores.unwrap() > 0);
        let total = s.total_memory_bytes.unwrap();
        assert!(total > 0);
        assert!(s.available_memory_bytes.unwrap() <= total);
        assert_eq!(s.data_volume_reachable, Some(true));
        assert!(s.data_volume_total_bytes.unwrap() > 0);
        assert!(s.required_free_bytes.unwrap() >= 2 * 1024 * 1024 * 1024);
        assert!(
            time::OffsetDateTime::parse(
                &s.sampled_at,
                &time::format_description::well_known::Rfc3339
            )
            .is_ok()
        );

        // Power fields are mutually consistent.
        match s.battery_present {
            Some(false) => assert_eq!(s.power, PowerSource::Ac),
            Some(true) => assert!(s.battery_percent.is_some()),
            None => assert_eq!(s.power, PowerSource::Unknown),
        }
    }

    #[test]
    fn no_data_root_leaves_volume_fields_empty() {
        let s = monitor().snapshot(None);
        assert_eq!(s.data_volume_reachable, None);
        assert_eq!(s.data_volume_free_bytes, None);
    }

    #[test]
    fn unreachable_root_is_reported_not_zeroed() {
        let dir = tempfile::tempdir().unwrap();
        let s = monitor().snapshot(Some(&dir.path().join("unplugged")));
        assert_eq!(s.data_volume_reachable, Some(false));
        assert_eq!(s.data_volume_free_bytes, None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_reports_pressure_and_thermal() {
        let s = monitor().snapshot(None);
        assert_ne!(s.memory_pressure, PressureLevel::Unknown);
        assert_ne!(s.thermal, ThermalLevel::Unknown);
    }

    #[test]
    fn simulation_overrides_only_requested_fields() {
        let dir = tempfile::tempdir().unwrap();
        let real = monitor().snapshot(Some(dir.path()));
        let simulated = SimulatedMonitor {
            inner: monitor(),
            simulation: Simulation {
                power: Some(SimulatedPower::Battery(18.0)),
                free_bytes: Some(1_000),
            },
        }
        .snapshot(Some(dir.path()));
        assert_eq!(simulated.power, PowerSource::Battery);
        assert_eq!(simulated.battery_percent, Some(18.0));
        assert_eq!(simulated.data_volume_free_bytes, Some(1_000));
        assert_eq!(simulated.total_memory_bytes, real.total_memory_bytes);
        assert_eq!(simulated.logical_cores, real.logical_cores);
    }

    #[test]
    fn simulation_values_parse() {
        assert_eq!(Simulation::parse_power("ac"), Some(SimulatedPower::Ac));
        assert_eq!(
            Simulation::parse_power("battery:18"),
            Some(SimulatedPower::Battery(18.0))
        );
        assert_eq!(Simulation::parse_power("battery:180"), None);
        assert_eq!(
            Simulation::parse_power("desktop"),
            Some(SimulatedPower::Desktop)
        );
        assert_eq!(Simulation::parse_power("nonsense"), None);
    }
}
