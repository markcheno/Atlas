//! Shared helpers for integration tests: temporary data roots and config
//! folders, and a fake volume inspector.
#![allow(dead_code)] // Each test crate uses a different subset.

pub mod embedding;
pub mod http;
pub mod vectors;

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
    /// While set, the next listing waits *before* opening its folder, so
    /// the scan is mid-way with no handle open (Windows can't rename a
    /// folder that has open handles inside it).
    hold: Mutex<bool>,
    held: Mutex<u32>,
    released: std::sync::Condvar,
    /// Files reported as cloud-only (contents offloaded), as iCloud or
    /// OneDrive placeholders are.
    cloud: Mutex<std::collections::HashSet<PathBuf>>,
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
    /// Stops the scan at its next folder; true once it is waiting there.
    pub fn hold_next_listing(&self, timeout: Duration) -> bool {
        *self.hold.lock().unwrap() = true;
        wait_until(timeout, || *self.held.lock().unwrap() > 0)
    }
    pub fn release(&self) {
        *self.hold.lock().unwrap() = false;
        self.released.notify_all();
    }
    /// Reports `path` as cloud-only from the next listing on.
    pub fn mark_cloud_only(&self, path: &Path) {
        self.cloud.lock().unwrap().insert(path.to_path_buf());
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
        {
            let mut hold = self.hold.lock().unwrap();
            if *hold {
                *self.held.lock().unwrap() += 1;
                while *hold {
                    hold = self.released.wait(hold).unwrap();
                }
                *self.held.lock().unwrap() -= 1;
            }
        }
        if let Some(kind) = self.fail.lock().unwrap().get(dir) {
            return Err(io::Error::new(*kind, "injected"));
        }
        let delay = *self.delay_per_entry.lock().unwrap();
        let cloud = self.cloud.lock().unwrap().clone();
        let dir = dir.to_path_buf();
        let inner = LocalFsSource.read_dir(&dir)?;
        Ok(Box::new(inner.map(move |entry| {
            if let Some(d) = delay {
                std::thread::sleep(d);
            }
            entry.map(|mut info| {
                if cloud.contains(&dir.join(&info.name)) {
                    info.cloud_only = true;
                }
                info
            })
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

// ------------------------------------------------------------ extraction

use atlas_lib::domain::dto::{ExtractionOverview, JobKind};
use atlas_lib::services::extraction::job::{
    self as extraction_job, ExtractionConfig, ExtractionDeps,
};
use atlas_lib::services::extraction::supervisor::WorkerCommand;

/// The real extraction worker: the `atlas` binary Cargo built.
pub fn worker() -> WorkerCommand {
    WorkerCommand::new(env!("CARGO_BIN_EXE_atlas"))
}

pub fn fast_extraction() -> ExtractionConfig {
    ExtractionConfig {
        batch_size: 8,
        unthrottled: true,
        recheck_interval: Duration::from_millis(5),
        policy_wait: Duration::from_millis(20),
        idle_wait: Duration::from_millis(50),
        event_interval: Duration::ZERO,
        ..ExtractionConfig::default()
    }
}

/// Cataloging and extraction sharing one governor and fake device, as in
/// the app.
pub struct ExtractionHarness {
    pub inv: InventoryHarness,
    pub deps: ExtractionDeps,
    pub events: Arc<Mutex<Vec<ExtractionOverview>>>,
}

impl ExtractionHarness {
    pub fn new(config: ExtractionConfig, worker: WorkerCommand) -> Self {
        let inv = InventoryHarness::new(fast_config());
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let deps = ExtractionDeps::new(
            inv.deps.governor.clone(),
            Arc::new(move |o: &ExtractionOverview| sink.lock().unwrap().push(o.clone())),
            config,
            worker,
        );
        Self { inv, deps, events }
    }
}

impl Env {
    pub fn workspace_with_extraction(&self, h: &ExtractionHarness) -> Workspace {
        self.workspace_with_inventory(h.inv.deps.clone())
            .with_job_processor(JobKind::Extract, extraction_job::factory(h.deps.clone()))
    }
}

/// Every file under `dir`: (BLAKE3, mtime, length). Extraction must leave
/// all of it unchanged (phase1-spec §12).
pub fn originals(
    dir: &Path,
) -> std::collections::BTreeMap<PathBuf, (String, std::time::SystemTime, u64)> {
    let mut out = std::collections::BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                let Ok(bytes) = fs::read(&path) else {
                    continue;
                };
                out.insert(
                    path,
                    (
                        blake3::hash(&bytes).to_hex().to_string(),
                        meta.modified().unwrap(),
                        meta.len(),
                    ),
                );
            }
        }
    }
    out
}
