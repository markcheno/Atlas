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

use crate::services::diagnostics::{Diagnostics, Event, LogGuard};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::domain::dto::{
    CandidateKind, CandidateProblem, DataRootCandidate, DataRootState, DataRootStatus, JobKind,
    Settings, VolumeKind,
};
use crate::domain::traits::VolumeInspector;
use crate::error::{AtlasError, AtlasResult, ErrorPayload};
use crate::infrastructure::clock::{Clock, SystemClock, now_rfc3339};
use crate::infrastructure::fsutil::{clear_dir, sync_dir};
use crate::infrastructure::native_path;
use crate::services::answers::AnswerRegistry;
use crate::services::catalog::reader::CatalogReader;
use crate::services::catalog::{self, CatalogWriter};
use crate::services::embedding::{self, registry::VectorRegistry};
use crate::services::export::import::Pending;
use crate::services::extraction;
use crate::services::inventory::{self, InventoryDeps};
use crate::services::jobs::{self, JobRunners, ProcessorFactory};
use crate::services::knowledge::{KnowledgeStore, Recorder, TrailEvents, TrailSettings};
use crate::services::library::{self, RootStatusCache};
use crate::services::reader::{ReaderConfig, ReaderService};
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
/// declaration order: the job runners stop first, then the readers and the
/// writer close, then the lock is released.
pub struct OpenRoot {
    /// The folder check (Slice 4.5), stopped first: it queues scans.
    _checker: Option<inventory::auto::Checker>,
    check_setting: inventory::auto::CheckSetting,
    /// The answer in progress (Slice 2.6), ended first, so an answer
    /// stopped by closing is still recorded.
    answers: Arc<AnswerRegistry>,
    /// Records trails into the knowledge store (Slice 3.3).
    recorder: Arc<Recorder>,
    /// The page view's renderer and its worker (Slice 3.7).
    pages: Arc<ReaderService>,
    /// An import's dry run, waiting for Import (Slice 3.8).
    imports: Arc<Pending>,
    jobs: JobRunners,
    /// The vector indexes, after the runners (which save them as they stop).
    vectors: Arc<VectorRegistry>,
    /// Shared so a search can run without the workspace lock (Slice 2.4).
    /// A search still running when the library closes keeps its reader
    /// connections for the rest of it (at most a couple of seconds).
    reader: Arc<CatalogReader>,
    root_status: Arc<RootStatusCache>,
    /// What the user creates (Slice 3.2), closed before the catalog.
    knowledge: KnowledgeStore,
    catalog: CatalogWriter,
    _lock: RootLock,
    path: PathBuf,
    display: String,
    manifest: Manifest,
    settings: Settings,
    /// The library's log (Slice 4.1), last: everything above records into
    /// it as it closes.
    log: LogGuard,
}

impl OpenRoot {
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// Where this library's events are recorded (Slice 4.1).
    pub fn diagnostics(&self) -> &Diagnostics {
        self.log.diagnostics()
    }
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub fn catalog(&self) -> &CatalogWriter {
        &self.catalog
    }
    /// The library's knowledge store (Slice 3.2, ADR 0016).
    pub fn knowledge(&self) -> &KnowledgeStore {
        &self.knowledge
    }
    /// The library's answer in progress (Slice 2.6).
    pub fn answers(&self) -> &Arc<AnswerRegistry> {
        &self.answers
    }
    /// Records and changes trails (Slice 3.3, ADR 0017).
    pub fn recorder(&self) -> &Arc<Recorder> {
        &self.recorder
    }
    /// Renders PDF pages for the page view (Slice 3.7, ADR 0018).
    pub fn pages(&self) -> &Arc<ReaderService> {
        &self.pages
    }
    /// An import waiting for its Import button (Slice 3.8).
    pub fn imports(&self) -> &Arc<Pending> {
        &self.imports
    }
    /// The library's vector indexes (Slice 2.3).
    pub fn vectors(&self) -> &Arc<VectorRegistry> {
        &self.vectors
    }
    /// The library's preferences as last saved.
    pub fn settings(&self) -> &Settings {
        &self.settings
    }
    /// Read-only connections for UI queries.
    pub fn reader(&self) -> &CatalogReader {
        &self.reader
    }
    /// Briefly cached reachability of library folders.
    pub fn root_status(&self) -> &RootStatusCache {
        &self.root_status
    }
    /// The read-only pool, to use without the workspace lock.
    pub fn reader_handle(&self) -> Arc<CatalogReader> {
        Arc::clone(&self.reader)
    }
    pub fn root_status_handle(&self) -> Arc<RootStatusCache> {
        Arc::clone(&self.root_status)
    }
    /// Asks every job runner to re-read job states now, for example after
    /// a user pauses a job or adds a folder.
    pub fn wake_jobs(&self) {
        self.jobs.wake_all();
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
    /// Where "now" comes from for the knowledge store (Slice 3.2).
    clock: Arc<dyn Clock>,
    /// Where the recording status goes (`trail-changed`, Slice 3.3).
    trail_events: Option<TrailEvents>,
    /// The page view's worker (Slice 3.7); `None` renders nothing.
    reader: Option<ReaderConfig>,
    /// One processor per job kind, started for every root this workspace
    /// opens (ADR 0009).
    processors: BTreeMap<JobKind, ProcessorFactory>,
    /// Scans' dependencies, for the folder check (Slice 4.5).
    inventory: Option<InventoryDeps>,
    /// Records into the open library's log (Slice 4.1).
    diagnostics: Diagnostics,
    state: State,
}

impl Workspace {
    pub fn new(config: WorkspaceConfig, volumes: Arc<dyn VolumeInspector>) -> Self {
        Self {
            config,
            volumes,
            clock: Arc::new(SystemClock),
            trail_events: None,
            reader: None,
            processors: BTreeMap::new(),
            inventory: None,
            diagnostics: Diagnostics::new(),
            state: State::NotConfigured,
        }
    }

    /// Records into `diagnostics` for every root this workspace opens
    /// (Slice 4.1; the app shares one with the resource governor).
    pub fn with_diagnostics(mut self, diagnostics: Diagnostics) -> Self {
        self.diagnostics = diagnostics;
        self
    }

    pub fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }

    /// The volume inspector, for free-space readings outside the workspace
    /// lock.
    pub fn volumes(&self) -> Arc<dyn VolumeInspector> {
        Arc::clone(&self.volumes)
    }

    /// Uses `clock` for every root this workspace opens (tests).
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Sends the recording status of every root this workspace opens to
    /// `events` (Slice 3.3).
    pub fn with_trail_events(mut self, events: TrailEvents) -> Self {
        self.trail_events = Some(events);
        self
    }

    /// Renders pages for the page view with `config`'s worker (Slice 3.7).
    pub fn with_reader(mut self, config: ReaderConfig) -> Self {
        self.reader = Some(config);
        self
    }

    /// Runs scans (cataloging) for every root this workspace opens, and
    /// checks its folders for changes (Slice 4.5).
    pub fn with_inventory(mut self, deps: InventoryDeps) -> Self {
        self.inventory = Some(deps.clone());
        self.with_job_processor(JobKind::Scan, inventory::scan::factory(deps))
    }

    /// Runs jobs of `kind` with `factory`'s processor for every root this
    /// workspace opens, replacing any earlier processor for that kind.
    pub fn with_job_processor(mut self, kind: JobKind, factory: ProcessorFactory) -> Self {
        self.processors.insert(kind, factory);
        self
    }

    /// Closes the open root (stopping its worker) without forgetting it.
    /// Used when the app exits.
    pub fn shutdown(&mut self) {
        self.state = State::NotConfigured;
    }

    /// Closes the open library and opens it again, for example to convert
    /// its catalog's pages (Slice 2.3). Problems become the reported status,
    /// as at startup.
    pub fn reopen(&mut self) -> DataRootStatus {
        let State::Open(root) = &self.state else {
            return self.status();
        };
        let (path, display, root_id) = (
            root.path.clone(),
            root.display.clone(),
            root.manifest.root_id.clone(),
        );
        // Dropping the root stops its jobs and closes the catalog.
        self.state = State::NotConfigured;
        self.state = match self.open_existing(&path) {
            Ok(root) => State::Open(Box::new(root)),
            Err(err) => State::Unavailable {
                state: state_for(&err),
                display,
                root_id: Some(root_id),
                problem: err.payload(),
            },
        };
        self.status()
    }

    /// Whether the open library's catalog would be converted on reopening.
    pub fn wants_page_conversion(&self) -> bool {
        self.open_root()
            .is_some_and(|root| catalog::convert::wanted(&root.path.join(CATALOG)))
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
        let privacy_changed = root.settings.privacy != new.privacy;
        // The folder check reads its interval at every tick (Slice 4.5).
        *root.check_setting.lock().unwrap_or_else(|p| p.into_inner()) =
            new.background.check_folders;
        root.settings = new.clone();
        if privacy_changed {
            // Recording and expiry take effect at once (Slice 3.3).
            root.knowledge.trail_settings().set(&new.privacy);
            root.recorder.emit();
            // Allowing an internet service lets embedding start (Slice 2.3).
            embedding::triggers::changed(root)?;
        }
        Ok(new)
    }

    // ----------------------------------------------------------- internals

    fn is_open_at(&self, path: &Path) -> bool {
        let Some(root) = self.open_root() else {
            return false;
        };
        match (dunce::canonicalize(path), dunce::canonicalize(&root.path)) {
            (Ok(a), Ok(b)) => native_path::comparable(&a) == native_path::comparable(&b),
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
        let log = self.diagnostics.attach(path);
        let catalog_path = path.join(CATALOG);
        if !catalog_path.is_file() {
            return Err(AtlasError::DataRootInvalid {
                reason: "its catalog is missing",
            });
        }
        // A catalog with 4 KiB pages gets 16 KiB ones before its first
        // embedding (Slice 2.3), while nothing else has it open. Without
        // room for the copy it stays as it is.
        if catalog::convert::wanted(&catalog_path) {
            let needed = catalog::convert::size(&catalog_path) / 5 * 6;
            let room = self.volumes.inspect(path).map(|v| v.free_bytes);
            if room.is_ok_and(|free| free.is_some_and(|f| f >= needed)) {
                catalog::convert::to_large_pages(&catalog_path, &path.join(TMP))?;
            }
        }
        let (catalog, found, version) = catalog::open_from(&catalog_path, now_rfc3339())?;

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

        // Interrupted jobs become queued; a user's pause is kept. Folders
        // never scanned are queued once.
        let now = now_rfc3339();
        let allow_remote =
            crate::services::providers::privacy::remote_embedding(&settings::read(path).privacy);
        catalog
            .call(move |c| {
                jobs::db::requeue_interrupted(c, &now)?;
                inventory::db::queue_never_scanned(c, &now)?;
                // Files still waiting for their text, including ones read
                // by an older extractor (phase1-spec §7.1).
                extraction::db::queue_pending(c, &now)?;
                // Text waiting for its meaning (Slice 2.3).
                embedding::models::sync(c, &now)?;
                embedding::triggers::queue_waiting(c, allow_remote, &now)
            })?
            .map_err(AtlasError::db("recover interrupted cataloging"))?;
        // Rows cataloged before search keys existed (Slice 0.5).
        library::backfill_keys(&catalog)?;

        let settings = settings::read(path);
        let knowledge = self.open_knowledge(path, &settings);
        let vectors =
            Arc::new(VectorRegistry::new(path).with_diagnostics(self.diagnostics.clone()));
        let reader = Arc::new(CatalogReader::open(&catalog_path)?);
        self.diagnostics.record(Event::LibraryOpened {
            catalog_schema: version,
            migrated_from: (found < version).then_some(found),
            knowledge: knowledge.state(),
            created: false,
        });
        let jobs = self.start_jobs(&catalog, path, &vectors)?;
        let check_setting = Arc::new(std::sync::Mutex::new(settings.background.check_folders));
        Ok(OpenRoot {
            _checker: self.start_checker(&catalog, &jobs, &check_setting),
            check_setting,
            answers: Arc::default(),
            recorder: self.recorder(&knowledge, &reader),
            pages: self.pages(path),
            imports: Arc::default(),
            jobs,
            vectors,
            reader,
            root_status: Arc::default(),
            knowledge,
            catalog,
            _lock: lock,
            path: path.to_path_buf(),
            display,
            settings,
            manifest,
            log,
        })
    }

    /// Opens the knowledge store, takes a snapshot if one is due, then
    /// removes expired trails. A store that can't be used reports why; the
    /// library opens anyway.
    fn open_knowledge(&self, path: &Path, settings: &Settings) -> KnowledgeStore {
        let store = KnowledgeStore::open(
            path,
            Arc::clone(&self.clock),
            Arc::clone(&self.volumes),
            Arc::new(TrailSettings::new(&settings.privacy)),
        );
        store.set_diagnostics(self.diagnostics.clone());
        store.snapshot_if_due();
        store.expire_if_due();
        store
    }

    fn pages(&self, path: &Path) -> Arc<ReaderService> {
        let config = self.reader.clone().map(|mut c| {
            c.diagnostics = Some(self.diagnostics.clone());
            c
        });
        Arc::new(ReaderService::new(path, config))
    }

    fn recorder(&self, knowledge: &KnowledgeStore, reader: &Arc<CatalogReader>) -> Arc<Recorder> {
        Arc::new(Recorder::new(
            knowledge.shared(),
            Arc::clone(reader),
            self.trail_events.clone(),
        ))
    }

    /// The folder check (Slice 4.5), when this workspace runs scans.
    fn start_checker(
        &self,
        catalog: &CatalogWriter,
        jobs: &JobRunners,
        setting: &inventory::auto::CheckSetting,
    ) -> Option<inventory::auto::Checker> {
        let deps = self.inventory.as_ref()?;
        inventory::auto::Checker::start(inventory::auto::CheckerDeps {
            catalog: catalog.handle(),
            waker: jobs.waker(),
            clock: Arc::clone(&self.clock),
            setting: Arc::clone(setting),
            events: Arc::clone(&deps.events),
            every: deps.config.check_every,
        })
    }

    fn start_jobs(
        &self,
        catalog: &CatalogWriter,
        path: &Path,
        vectors: &Arc<VectorRegistry>,
    ) -> AtlasResult<JobRunners> {
        JobRunners::start(
            &catalog.handle(),
            path,
            &self.processors,
            vectors,
            &self.diagnostics,
        )
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
        let log = self.diagnostics.attach(path);
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

        let settings = settings::read(path);
        let knowledge = self.open_knowledge(path, &settings);
        let vectors =
            Arc::new(VectorRegistry::new(path).with_diagnostics(self.diagnostics.clone()));
        let reader = Arc::new(CatalogReader::open(&path.join(CATALOG))?);
        self.diagnostics.record(Event::LibraryOpened {
            catalog_schema: version,
            migrated_from: None,
            knowledge: knowledge.state(),
            created: true,
        });
        let jobs = self.start_jobs(&catalog, path, &vectors)?;
        let check_setting = Arc::new(std::sync::Mutex::new(settings.background.check_folders));
        Ok(OpenRoot {
            _checker: self.start_checker(&catalog, &jobs, &check_setting),
            check_setting,
            answers: Arc::default(),
            recorder: self.recorder(&knowledge, &reader),
            pages: self.pages(path),
            imports: Arc::default(),
            jobs,
            vectors,
            reader,
            root_status: Arc::default(),
            knowledge,
            catalog,
            _lock: lock,
            path: path.to_path_buf(),
            display,
            settings,
            manifest,
            log,
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
