//! Shared helpers for integration tests: temporary data roots and config
//! folders, and a fake volume inspector.
#![allow(dead_code)] // Each test crate uses a different subset.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use atlas_lib::domain::dto::VolumeKind;
use atlas_lib::domain::traits::{VolumeInfo, VolumeInspector};
use atlas_lib::error::{AtlasResult, ErrorCode};
use atlas_lib::services::workspace::{Workspace, WorkspaceConfig};

pub const GIB: u64 = 1024 * 1024 * 1024;

pub struct FakeVolumes {
    pub kind: VolumeKind,
    pub free: u64,
    pub total: u64,
}

impl VolumeInspector for FakeVolumes {
    fn inspect(&self, _path: &Path) -> AtlasResult<VolumeInfo> {
        Ok(VolumeInfo {
            free_bytes: Some(self.free),
            total_bytes: Some(self.total),
            fs_type: Some("fake".into()),
            kind: self.kind,
        })
    }
}

pub fn local() -> Arc<FakeVolumes> {
    Arc::new(FakeVolumes {
        kind: VolumeKind::Local,
        free: 100 * GIB,
        total: 500 * GIB,
    })
}

pub struct Env {
    _tmp: tempfile::TempDir,
    pub config: PathBuf,
    pub base: PathBuf,
}

impl Env {
    pub fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path().join("config");
        let base = tmp.path().join("data");
        fs::create_dir_all(&base).unwrap();
        Self {
            config,
            base,
            _tmp: tmp,
        }
    }

    pub fn workspace(&self) -> Workspace {
        self.workspace_with(local())
    }

    pub fn workspace_with(&self, volumes: Arc<dyn VolumeInspector>) -> Workspace {
        Workspace::new(
            WorkspaceConfig {
                config_dir: self.config.clone(),
                default_root: self.config.join("Library"),
            },
            volumes,
        )
    }

    /// A second, unrelated Atlas install (its own locator).
    pub fn other_workspace(&self) -> Workspace {
        Workspace::new(
            WorkspaceConfig {
                config_dir: self.base.join("other-config"),
                default_root: self.base.join("other-default"),
            },
            local(),
        )
    }
}

pub fn code(result: AtlasResult<impl std::fmt::Debug>) -> ErrorCode {
    result.expect_err("expected an error").code()
}

pub fn dir_listing(path: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(path)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

// ------------------------------------------------------------- inventory

use std::collections::HashMap;
use std::io;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use atlas_lib::domain::dto::{
    DeviceSnapshot, InventoryOverview, OsKind, PowerSource, PressureLevel, ThermalLevel,
};
use atlas_lib::domain::traits::{DirEntries, InventorySource, ResourceMonitor};
use atlas_lib::infrastructure::local_source::LocalFsSource;
use atlas_lib::services::inventory::{InventoryConfig, InventoryDeps};
use atlas_lib::services::system::ResourceGovernor;

/// A device whose readings the test changes at will.
#[derive(Clone)]
pub struct FakeMonitor(pub Arc<Mutex<DeviceSnapshot>>);

impl FakeMonitor {
    pub fn plugged_in() -> Self {
        Self(Arc::new(Mutex::new(ac_snapshot())))
    }
    pub fn set(&self, f: impl FnOnce(&mut DeviceSnapshot)) {
        f(&mut self.0.lock().unwrap());
    }
}

impl ResourceMonitor for FakeMonitor {
    fn snapshot(&self, _data_root: Option<&Path>) -> DeviceSnapshot {
        self.0.lock().unwrap().clone()
    }
}

pub fn ac_snapshot() -> DeviceSnapshot {
    DeviceSnapshot {
        schema_version: 1,
        os: OsKind::current(),
        arch: "test".into(),
        logical_cores: Some(8),
        total_memory_bytes: Some(16 * GIB),
        available_memory_bytes: Some(8 * GIB),
        data_volume_reachable: None,
        data_volume_free_bytes: None,
        data_volume_total_bytes: None,
        data_volume_kind: None,
        required_free_bytes: None,
        power: PowerSource::Ac,
        battery_present: Some(false),
        battery_percent: None,
        charging: None,
        memory_pressure: PressureLevel::Normal,
        thermal: ThermalLevel::Nominal,
        sampled_at: "t".into(),
    }
}

/// The real filesystem source with injectable failures and slowness.
#[derive(Default)]
pub struct FaultySource {
    pub fail: Mutex<HashMap<PathBuf, io::ErrorKind>>,
    pub delay_per_entry: Mutex<Option<Duration>>,
    pub calls: Mutex<HashMap<PathBuf, u32>>,
}

impl FaultySource {
    pub fn fail_dir(&self, path: &Path, kind: io::ErrorKind) {
        self.fail.lock().unwrap().insert(path.to_path_buf(), kind);
    }
    pub fn slow(&self, per_entry: Duration) {
        *self.delay_per_entry.lock().unwrap() = Some(per_entry);
    }
    pub fn calls_for(&self, path: &Path) -> u32 {
        self.calls.lock().unwrap().get(path).copied().unwrap_or(0)
    }
}

impl InventorySource for FaultySource {
    fn read_dir(&self, dir: &Path) -> io::Result<DirEntries<'_>> {
        *self
            .calls
            .lock()
            .unwrap()
            .entry(dir.to_path_buf())
            .or_default() += 1;
        if let Some(kind) = self.fail.lock().unwrap().get(dir) {
            return Err(io::Error::new(*kind, "injected"));
        }
        let delay = *self.delay_per_entry.lock().unwrap();
        let inner = LocalFsSource.read_dir(dir)?;
        Ok(Box::new(inner.inspect(move |_| {
            if let Some(d) = delay {
                std::thread::sleep(d);
            }
        })))
    }
}

pub fn fast_config() -> InventoryConfig {
    InventoryConfig {
        batch_size: 25,
        unthrottled: true,
        rate_override: None,
        recheck_interval: Duration::from_millis(5),
        policy_wait: Duration::from_millis(20),
        idle_wait: Duration::from_millis(50),
        max_dir_attempts: 3,
        event_interval: Duration::ZERO,
    }
}

pub struct InventoryHarness {
    pub monitor: FakeMonitor,
    pub source: Arc<FaultySource>,
    pub events: Arc<Mutex<Vec<InventoryOverview>>>,
    pub deps: InventoryDeps,
}

impl InventoryHarness {
    pub fn new(config: InventoryConfig) -> Self {
        let monitor = FakeMonitor::plugged_in();
        let source = Arc::new(FaultySource::default());
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let deps = InventoryDeps {
            governor: Arc::new(Mutex::new(ResourceGovernor::new(Box::new(monitor.clone())))),
            source: source.clone(),
            events: Arc::new(move |o: &InventoryOverview| sink.lock().unwrap().push(o.clone())),
            config,
        };
        Self {
            monitor,
            source,
            events,
            deps,
        }
    }
}

impl Env {
    pub fn workspace_with_inventory(&self, deps: InventoryDeps) -> Workspace {
        self.workspace().with_inventory(deps)
    }
}

/// Polls `f` every 10 ms until it holds or `timeout` passes.
pub fn wait_until(timeout: Duration, mut f: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    f()
}
