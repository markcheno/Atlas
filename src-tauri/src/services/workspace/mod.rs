//! Atlas Data Root lifecycle: choosing, creating, opening, and remembering
//! the one folder that holds all Atlas-owned persistent data.
//!
//! Guarantees (phase0-spec §6, ADR 0002):
//! - A folder becomes a valid root only when `atlas.json` is written, which
//!   happens last, atomically. An interrupted creation is resumable.
//! - Opening never creates a root. A remembered root that has disappeared
//!   (for example an unplugged drive) is reported as missing, not recreated.
//! - One process at a time: an exclusive OS lock is held while open.
//! - A catalog from a newer Atlas is refused before anything is written.
//! - Network and cloud-synced locations are refused.
//!
//! This service has no Tauri dependency; commands wrap it.

pub mod inspect;
pub mod layout;
pub mod locator;
pub mod lock;
pub mod manifest;
pub mod settings;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::domain::dto::{
    CandidateKind, CandidateProblem, DataRootCandidate, DataRootState, DataRootStatus, Settings,
    VolumeKind,
};
use crate::domain::traits::VolumeInspector;
use crate::error::{AtlasError, AtlasResult, ErrorPayload};
use crate::infrastructure::clock::now_rfc3339;
use crate::infrastructure::fsutil::{clear_dir, sync_dir};
use crate::services::catalog::{self, CatalogWriter};
use crate::services::inventory::{self, InventoryDeps, InventoryRunner};
use crate::services::system::low_space_threshold;
use layout::{CATALOG, DIRECTORIES, INIT_MARKER, TMP};
use lock::RootLock;
use manifest::{APP_VERSION, Manifest};

pub struct WorkspaceConfig {
    /// Holds the locator file. Outside the data root by necessity.
    pub config_dir: PathBuf,
    /// Suggested location on first run.
    pub default_root: PathBuf,
}

/// An open data root. Field order matters, because Rust drops fields in
/// declaration order: the inventory worker stops first, then the catalog
/// closes, then the lock is released.
pub struct OpenRoot {
    inventory: Option<InventoryRunner>,
    catalog: CatalogWriter,
    _lock: RootLock,
    path: PathBuf,
    display: String,
    manifest: Manifest,
    settings: Settings,
}

impl OpenRoot {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub fn catalog(&self) -> &CatalogWriter {
        &self.catalog
    }
    /// The background inventory worker, when this workspace runs one.
    pub fn inventory(&self) -> Option<&InventoryRunner> {
        self.inventory.as_ref()
    }
}

enum State {
    NotConfigured,
    Open(Box<OpenRoot>),
    Unavailable {
        state: DataRootState,
        display: String,
        root_id: Option<String>,
        problem: ErrorPayload,
    },
}

pub struct Workspace {
    config: WorkspaceConfig,
    volumes: Arc<dyn VolumeInspector>,
    inventory_deps: Option<InventoryDeps>,
    state: State,
}

impl Workspace {
    pub fn new(config: WorkspaceConfig, volumes: Arc<dyn VolumeInspector>) -> Self {
        Self {
            config,
            volumes,
            inventory_deps: None,
            state: State::NotConfigured,
        }
    }

    /// Runs an inventory worker for every root this workspace opens.
    pub fn with_inventory(mut self, deps: InventoryDeps) -> Self {
        self.inventory_deps = Some(deps);
        self
    }

    /// Closes the open root (stopping its worker) without forgetting it.
    /// Used when the app exits.
    pub fn shutdown(&mut self) {
        self.state = State::NotConfigured;
    }

    /// Reopens the remembered root, if any. Never fails: problems become
    /// the reported status so the UI can offer recovery.
    pub fn startup(&mut self) {
        let Some(locator) = locator::read(&self.config.config_dir) else {
            self.state = State::NotConfigured;
            return;
        };
        let path = locator.path();
        self.state = match self.open_existing(&path) {
            Ok(root) => State::Open(Box::new(root)),
            Err(err) => State::Unavailable {
                state: state_for(&err),
                display: locator.data_root.clone(),
                root_id: Some(locator.root_id.clone()),
                problem: err.payload(),
            },
        };
    }

    pub fn status(&self) -> DataRootStatus {
        match &self.state {
            State::NotConfigured => DataRootStatus {
                state: DataRootState::NotConfigured,
                display_path: None,
                root_id: None,
                problem: None,
            },
            State::Open(root) => DataRootStatus {
                state: DataRootState::Open,
                display_path: Some(root.display.clone()),
                root_id: Some(root.manifest.root_id.clone()),
                problem: None,
            },
            State::Unavailable {
                state,
                display,
                root_id,
                problem,
            } => DataRootStatus {
                state: *state,
                display_path: Some(display.clone()),
                root_id: root_id.clone(),
                problem: Some(problem.clone()),
            },
        }
    }

    pub fn open_root(&self) -> Option<&OpenRoot> {
        match &self.state {
            State::Open(root) => Some(root.as_ref()),
            _ => None,
        }
    }

    /// The open root, or `NoDataRoot`.
    pub fn require_open(&self) -> AtlasResult<&OpenRoot> {
        self.open_root().ok_or(AtlasError::NoDataRoot)
    }

    pub fn default_root(&self) -> String {
        self.config.default_root.to_string_lossy().into_owned()
    }

    pub fn inspect(&self, path: &Path) -> AtlasResult<DataRootCandidate> {
        inspect::inspect(path, self.volumes.as_ref())
    }

    /// Uses `path` as the data root: opens an existing library there, or
    /// creates a new one in a new, empty, or partially created folder.
    pub fn choose(&mut self, path: &Path) -> AtlasResult<DataRootStatus> {
        let candidate = self.inspect(path)?;
        if self.is_open_at(path) {
            return Ok(self.status());
        }
        match candidate.kind {
            CandidateKind::ExistingLibrary => return self.open(path),
            CandidateKind::NotAFolder => return Err(AtlasError::DataRootNotAFolder),
            CandidateKind::NonEmptyFolder => {
                if let Some(problem) = candidate.problems.first() {
                    return Err(problem_error(*problem, &candidate));
                }
                return Err(AtlasError::FolderNotEmpty);
            }
            CandidateKind::NewFolder
            | CandidateKind::EmptyFolder
            | CandidateKind::IncompleteLibrary => {}
        }
        if let Some(problem) = candidate.problems.first() {
            return Err(problem_error(*problem, &candidate));
        }
        let root = self.create(path)?;
        self.activate(root)
    }

    /// Opens an existing library at `path`. Never creates anything.
    pub fn open(&mut self, path: &Path) -> AtlasResult<DataRootStatus> {
        inspect::display_path(path)?;
        if self.is_open_at(path) {
            return Ok(self.status());
        }
        let root = self.open_existing(path)?;
        self.activate(root)
    }

    pub fn settings(&self) -> AtlasResult<Settings> {
        self.open_root()
            .map(|root| root.settings.clone())
            .ok_or(AtlasError::NoDataRoot)
    }

    pub fn update_settings(&mut self, new: Settings) -> AtlasResult<Settings> {
        let State::Open(root) = &mut self.state else {
            return Err(AtlasError::NoDataRoot);
        };
        settings::write(&root.path, &new)?;
        root.settings = new.clone();
        Ok(new)
    }

    // ----------------------------------------------------------- internals

    fn is_open_at(&self, path: &Path) -> bool {
        let Some(root) = self.open_root() else {
            return false;
        };
        match (fs::canonicalize(path), fs::canonicalize(&root.path)) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    }

    /// Remembers `root` and makes it current. The previously open root (if
    /// any) is closed only after the new one is fully open.
    fn activate(&mut self, root: OpenRoot) -> AtlasResult<DataRootStatus> {
        locator::write(
            &self.config.config_dir,
            &root.display,
            &root.manifest.root_id,
        )?;
        self.state = State::Open(Box::new(root));
        Ok(self.status())
    }

    fn check_volume(&self, path: &Path) -> AtlasResult<()> {
        match self.volumes.inspect(path).map(|v| v.kind) {
            Ok(VolumeKind::Network) => Err(AtlasError::DataRootNetworkVolume),
            Ok(VolumeKind::CloudSynced) => Err(AtlasError::DataRootCloudSynced),
            _ => Ok(()),
        }
    }

    fn open_existing(&self, path: &Path) -> AtlasResult<OpenRoot> {
        let display = inspect::display_path(path)?;
        match fs::metadata(path) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => return Err(AtlasError::DataRootNotAFolder),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Err(AtlasError::DataRootMissing);
            }
            Err(err) => return Err(AtlasError::io("reach the library folder")(err)),
        }
        if !path.join(layout::MANIFEST).exists() && path.join(INIT_MARKER).exists() {
            return Err(AtlasError::DataRootInvalid {
                reason: "it was not fully created",
            });
        }
        let mut manifest = manifest::read(path)?;
        self.check_volume(path)?;

        let lock = lock::acquire(path)?;
        let catalog_path = path.join(CATALOG);
        if !catalog_path.is_file() {
            return Err(AtlasError::DataRootInvalid {
                reason: "its catalog is missing",
            });
        }
        let (catalog, version) = catalog::open(&catalog_path, now_rfc3339())?;

        // Layout repair and housekeeping, only after the lock is held.
        for dir in DIRECTORIES {
            fs::create_dir_all(path.join(dir)).map_err(AtlasError::io("prepare the library"))?;
        }
        clear_dir(&path.join(TMP)).map_err(AtlasError::io("clean temporary files"))?;
        let _ = fs::remove_file(path.join(INIT_MARKER));

        if manifest.catalog_schema_version != version
            || manifest.last_written_by_app_version != APP_VERSION
        {
            manifest.catalog_schema_version = version;
            manifest.last_written_by_app_version = APP_VERSION.to_owned();
            manifest::write(path, &manifest)?;
        }

        // Interrupted scans become queued; a user's pause is kept.
        let now = now_rfc3339();
        catalog
            .call(move |c| inventory::db::recover_on_open(c, &now))?
            .map_err(AtlasError::db("recover interrupted cataloging"))?;

        Ok(OpenRoot {
            inventory: self.start_inventory(&catalog, path)?,
            catalog,
            _lock: lock,
            path: path.to_path_buf(),
            display,
            settings: settings::read(path),
            manifest,
        })
    }

    fn start_inventory(
        &self,
        catalog: &CatalogWriter,
        path: &Path,
    ) -> AtlasResult<Option<InventoryRunner>> {
        self.inventory_deps
            .as_ref()
            .map(|deps| InventoryRunner::start(catalog.handle(), path, deps.clone()))
            .transpose()
    }

    /// Creates a root, or finishes a previously interrupted creation.
    /// Every step is idempotent; the manifest is written last.
    fn create(&self, path: &Path) -> AtlasResult<OpenRoot> {
        let display = inspect::display_path(path)?;
        fs::create_dir_all(path).map_err(AtlasError::io("create the library folder"))?;

        let marker = path.join(INIT_MARKER);
        if !marker.exists() {
            fs::write(&marker, APP_VERSION)
                .map_err(AtlasError::io("start creating the library"))?;
            let _ = sync_dir(path);
        }

        let lock = lock::acquire(path)?;
        for dir in DIRECTORIES {
            fs::create_dir_all(path.join(dir))
                .map_err(AtlasError::io("create the library folders"))?;
        }
        clear_dir(&path.join(TMP)).map_err(AtlasError::io("clean temporary files"))?;

        let now = now_rfc3339();
        let (catalog, version) = catalog::open(&path.join(CATALOG), now.clone())?;
        let root_id = catalog::ensure_workspace(
            &catalog,
            uuid::Uuid::new_v4().to_string(),
            "My Library".to_owned(),
            now.clone(),
        )?;

        if !settings::exists(path) {
            settings::write(path, &Settings::default())?;
        }
        let manifest = Manifest::new(root_id, version, now);
        manifest::write(path, &manifest)?;

        fs::remove_file(&marker).map_err(AtlasError::io("finish creating the library"))?;
        let _ = sync_dir(path);

        Ok(OpenRoot {
            inventory: self.start_inventory(&catalog, path)?,
            catalog,
            _lock: lock,
            path: path.to_path_buf(),
            display,
            settings: settings::read(path),
            manifest,
        })
    }
}

fn state_for(err: &AtlasError) -> DataRootState {
    match err {
        AtlasError::DataRootMissing => DataRootState::Missing,
        AtlasError::DataRootInUse => DataRootState::InUse,
        AtlasError::DataRootIncompatible { .. } => DataRootState::Incompatible,
        _ => DataRootState::Invalid,
    }
}

fn problem_error(problem: CandidateProblem, candidate: &DataRootCandidate) -> AtlasError {
    const MIB: u64 = 1024 * 1024;
    match problem {
        CandidateProblem::NotWritable => AtlasError::DataRootNotWritable,
        CandidateProblem::NetworkVolume => AtlasError::DataRootNetworkVolume,
        CandidateProblem::CloudSynced => AtlasError::DataRootCloudSynced,
        CandidateProblem::InsideLibrary => AtlasError::DataRootNested,
        CandidateProblem::LowSpace => AtlasError::DataRootLowSpace {
            free_mib: candidate.free_bytes.unwrap_or(0) / MIB,
            required_mib: candidate
                .required_free_bytes
                .unwrap_or_else(|| low_space_threshold(0))
                / MIB,
        },
        // Opening reports these precisely; choose() routes existing
        // libraries to open() before reaching here.
        CandidateProblem::NewerSchema | CandidateProblem::InvalidManifest => {
            AtlasError::DataRootInvalid {
                reason: "its manifest could not be used",
            }
        }
    }
}
