//! How busy other applications keep the processor (Slice 4.6), for the
//! busy-computer rule in `services::scheduler::busy`.
//!
//! One `sysinfo::System` is kept between readings, because a process's
//! CPU use is the difference between two of them. A reading refreshes
//! every process's CPU time only (and its executable once), which takes a
//! few milliseconds; the rule asks at most every 5 s, and only while a job
//! runs or Settings is open.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::services::scheduler::busy::{CpuProbe, Proc, other_share};

/// How often the AI service's process is looked up again by its port.
const LOOK_UP_EVERY: Duration = Duration::from_secs(60);

pub struct OsCpuProbe {
    system: Option<System>,
    own: u32,
    own_exe: Option<PathBuf>,
    cores: usize,
    primed: bool,
    ports: Vec<u16>,
    excluded: Vec<u32>,
    looked_up: Option<Instant>,
}

impl Default for OsCpuProbe {
    fn default() -> Self {
        Self::new()
    }
}

impl OsCpuProbe {
    pub fn new() -> Self {
        Self {
            system: System::new().ok(),
            own: std::process::id(),
            own_exe: std::env::current_exe()
                .ok()
                .and_then(|p| dunce::canonicalize(p).ok()),
            cores: std::thread::available_parallelism().map_or(0, |n| n.get()),
            primed: false,
            ports: Vec::new(),
            excluded: Vec::new(),
            looked_up: None,
        }
    }

    /// The processes listening on the AI service's ports, looked up again
    /// every minute: the service may restart with a new process id.
    fn refresh_excluded(&mut self) {
        let due = self.looked_up.is_none_or(|t| t.elapsed() >= LOOK_UP_EVERY);
        if !due {
            return;
        }
        self.looked_up = Some(Instant::now());
        self.excluded = self
            .ports
            .iter()
            .filter_map(|&port| listeners::get_process_by_port(port, listeners::Protocol::TCP).ok())
            .map(|p| p.pid)
            .collect();
    }

    /// The processes since the last reading, as the rule sees them.
    fn table(&self, system: &System) -> Vec<Proc> {
        system
            .processes()
            .values()
            .map(|p| {
                let parent = p.parent().map(Pid::as_u32);
                // Only Atlas's children can be its workers.
                let atlas_exe = parent == Some(self.own)
                    && match (&self.own_exe, p.exe()) {
                        (Some(own), Some(exe)) => {
                            exe == own.as_path()
                                || dunce::canonicalize(exe).is_ok_and(|e| &e == own)
                        }
                        _ => false,
                    };
                Proc {
                    pid: p.pid().as_u32(),
                    parent,
                    atlas_exe,
                    cpu_percent: p.cpu_usage(),
                }
            })
            .collect()
    }
}

impl CpuProbe for OsCpuProbe {
    fn sample(&mut self) -> Option<f32> {
        if self.cores == 0 {
            return None;
        }
        self.refresh_excluded();
        let system = self.system.as_mut()?;
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_exe(UpdateKind::OnlyIfNotSet),
        );
        if !std::mem::replace(&mut self.primed, true) {
            // CPU use is a difference: the first refresh only starts it.
            return None;
        }
        let system = self.system.as_ref()?;
        Some(other_share(
            &self.table(system),
            self.own,
            &self.excluded,
            self.cores,
        ))
    }

    fn exclude_ports(&mut self, ports: &[u16]) {
        if self.ports != ports {
            self.ports = ports.to_vec();
            self.looked_up = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_reading_starts_measuring_then_shares_are_plausible() {
        let mut probe = OsCpuProbe::new();
        assert_eq!(probe.sample(), None);
        std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
        let started = Instant::now();
        let share = probe.sample().expect("a second reading measures");
        assert!((0.0..=1.0).contains(&share), "{share}");
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
