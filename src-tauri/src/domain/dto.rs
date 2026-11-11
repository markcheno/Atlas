//! IPC data-transfer objects.
//!
//! These types are the stable contract between the Rust core and the UI.
//! TypeScript definitions are generated from them by `ts-rs` during
//! `cargo test` into `src/lib/api/bindings/`; never edit those files by hand.
//!
//! Conventions: camelCase fields, snake_case enum values, RFC 3339 UTC
//! timestamps as strings, record IDs as opaque UUID strings, and byte counts
//! as JSON numbers (exact up to 2^53, ample for file and volume sizes).
//! Source paths appear here only as lossy *display* strings; the native
//! path bytes stay on the Rust side (see the Phase 0 spec, section 6).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub type RecordId = String;

// ---------------------------------------------------------------- app status

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum OsKind {
    Macos,
    Windows,
    Linux,
    /// A platform Atlas does not target; reported honestly rather than guessed.
    Other,
}

impl OsKind {
    pub fn current() -> Self {
        match std::env::consts::OS {
            "macos" => Self::Macos,
            "windows" => Self::Windows,
            "linux" => Self::Linux,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum BuildProfile {
    Debug,
    Release,
}

/// One user-visible capability and whether this build actually provides it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FeatureStatus {
    pub id: String,
    pub label: String,
    pub implemented: bool,
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planned_slice: Option<String>,
}

/// Facts about the running application. Contains no library data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppStatus {
    pub app_version: String,
    pub os: OsKind,
    pub arch: String,
    pub build_profile: BuildProfile,
    /// The slice this build implements, "<phase>.<slice>", e.g. "1.1".
    pub slice: String,
    pub features: Vec<FeatureStatus>,
}

// ------------------------------------------------------------ device snapshot

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PowerSource {
    Ac,
    Battery,
    /// No reliable reading. Never treated as AC.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PressureLevel {
    Normal,
    Elevated,
    Critical,
    /// This OS does not report it, or the reading failed.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ThermalLevel {
    Nominal,
    Elevated,
    Critical,
    Unknown,
}

/// Versioned, timestamped resource reading. Every measurement is optional:
/// a missing value means "not reported by this OS", never zero.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeviceSnapshot {
    pub schema_version: u32,
    pub os: OsKind,
    pub arch: String,
    pub logical_cores: Option<u32>,
    #[ts(type = "number | null")]
    pub total_memory_bytes: Option<u64>,
    #[ts(type = "number | null")]
    pub available_memory_bytes: Option<u64>,
    /// `None` when no data root is open; `Some(false)` when the open root's
    /// drive cannot be reached.
    pub data_volume_reachable: Option<bool>,
    #[ts(type = "number | null")]
    pub data_volume_free_bytes: Option<u64>,
    #[ts(type = "number | null")]
    pub data_volume_total_bytes: Option<u64>,
    pub data_volume_kind: Option<VolumeKind>,
    /// Free space below which Atlas starts no new heavy work on that drive.
    #[ts(type = "number | null")]
    pub required_free_bytes: Option<u64>,
    pub power: PowerSource,
    /// `Some(false)`: no battery at all (a desktop), which is not "on
    /// battery". `None`: the probe could not tell.
    pub battery_present: Option<bool>,
    pub battery_percent: Option<f32>,
    pub charging: Option<bool>,
    pub memory_pressure: PressureLevel,
    pub thermal: ThermalLevel,
    pub sampled_at: String,
}

// --------------------------------------------------------- resource policy

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PolicyMode {
    Run,
    Slow,
    Paused,
}

/// Why background work runs slower than normal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ThrottleReason {
    OnBattery,
    /// Power source unknown; Atlas never assumes AC.
    PowerUnknown,
    /// Elevated memory pressure or thermal state.
    SystemBusy,
}

/// What background work may do right now (phase0-spec §7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PolicyDecision {
    pub mode: PolicyMode,
    pub pause_reason: Option<PauseReason>,
    pub throttle_reason: Option<ThrottleReason>,
    /// Measured on one development machine (docs/benchmarks.md); not a guarantee.
    pub max_metadata_ops_per_second: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum LibraryFit {
    Comfortable,
    Usable,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum LocalAiFit {
    Likely,
    Possible,
    Unlikely,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AssessmentNote {
    OnBattery,
    PowerUnknown,
    Desktop,
    LowSpace,
    MemoryPressure,
    UnknownMemory,
}

/// Plain-language guidance derived from a snapshot. Provisional planning
/// targets (product spec §7), never a hard block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeviceAssessment {
    pub library: LibraryFit,
    pub local_ai: LocalAiFit,
    pub notes: Vec<AssessmentNote>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DeviceCheck {
    pub snapshot: DeviceSnapshot,
    pub assessment: DeviceAssessment,
    pub policy: PolicyDecision,
}

// ----------------------------------------------------------- library catalog

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SourceRootStatus {
    Available,
    /// Not present or not responding (for example an unplugged drive or a
    /// stale network mount). Its catalog entries are kept.
    Unavailable,
    /// Present, but Atlas is not allowed to read it.
    NoAccess,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SourceRoot {
    pub id: RecordId,
    pub display_path: String,
    /// Live availability, checked each time roots are listed.
    pub status: SourceRootStatus,
    /// The Atlas data root is inside this folder; inventory will skip it.
    pub contains_data_root: bool,
    /// Set when this root lies inside another root (possible only in
    /// imported catalogs). The outermost root owns the subtree, and this
    /// one is not scanned separately.
    pub nested_in_root_id: Option<RecordId>,
    pub created_at: String,
    pub last_scan_at: Option<String>,
}

/// A folder the picker returned that Atlas did not add, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SourceRootRejection {
    pub display_path: String,
    pub error: crate::error::ErrorPayload,
    /// For `source_overlaps` and `source_already_added`: the existing root.
    pub conflicting_root_id: Option<RecordId>,
}

/// Outcome of picking folders. Empty on both sides when the user cancels.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AddSourceRootsResult {
    pub added: Vec<SourceRoot>,
    pub rejected: Vec<SourceRootRejection>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum FileType {
    Pdf,
    Epub,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Availability {
    Available,
    /// Not seen by the last *successful* scan of its source root.
    Missing,
    /// Its source root is currently unavailable.
    Unreachable,
}

/// Where a file's contents are right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum FileStorage {
    Local,
    /// Offloaded to a cloud service (an iCloud placeholder or "dataless"
    /// file, or a Windows cloud-files placeholder). Opening it downloads it.
    CloudOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileInstance {
    pub id: RecordId,
    pub source_root_id: RecordId,
    pub display_name: String,
    pub display_path: String,
    pub extension: String,
    pub file_type: FileType,
    /// `None` when unknown, never a guessed zero.
    #[ts(type = "number | null")]
    pub size_bytes: Option<u64>,
    pub modified_at: Option<String>,
    pub availability: Availability,
    pub storage: FileStorage,
    /// The content's document, once its bytes have been hashed (several
    /// files with the same bytes share one).
    pub document_id: Option<RecordId>,
    /// Whether Atlas has read the file's text (phase1-spec §5.3).
    pub extraction: FileExtraction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DocumentSort {
    NameAsc,
    NameDesc,
    ModifiedDesc,
    ModifiedAsc,
}

/// Paginated filename/path query. Phase 0 never searches document bodies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocumentQuery {
    pub text: Option<String>,
    pub source_root_id: Option<RecordId>,
    pub file_type: Option<FileType>,
    pub sort: DocumentSort,
    pub page: u32,
    pub page_size: u32,
    /// Also list files a completed scan no longer found.
    #[serde(default)]
    pub include_missing: bool,
    /// Only files with this text status (`reading` counts as `pending`).
    #[serde(default)]
    pub text_status: Option<ExtractionState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocumentPage {
    pub items: Vec<FileInstance>,
    pub page: u32,
    pub page_size: u32,
    /// Matching rows known so far. While a scan runs this can grow; it is
    /// never presented as a final total.
    pub matched_so_far: u32,
    /// Cataloging is still in progress, so more matches may appear.
    pub scanning: bool,
}

// ------------------------------------------------------------ inventory jobs

/// Job states from the Phase 0 spec, section 7.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum JobState {
    Queued,
    Running,
    PausedUser,
    PausedPolicy,
    FailedRetryable,
    FailedTerminal,
    Cancelled,
    Completed,
}

/// What a job does (phase1-spec §7.1, ADR 0009). Stored as TEXT in
/// `jobs.kind`; not part of the IPC contract until a DTO carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    /// Cataloging a source root's files (Phase 0's inventory).
    Scan,
    /// Text extraction from cataloged files (Phase 1, Slice 1.3).
    Extract,
    /// Embedding extracted text for meaning search (Phase 2, Slice 2.3).
    Embed,
}

// ------------------------------------------------------------ extraction

/// A file's text status as the Library shows it (phase1-spec §5.3).
/// `Reading` is the file the active extraction job is on; the catalog
/// stores it as `pending`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ExtractionState {
    Pending,
    Reading,
    Done,
    NeedsOcr,
    Encrypted,
    CloudOnly,
    /// Malformed, unsupported, too large, crashed the reader, or not readable
    /// from disk; `reason` says which.
    Unreadable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileExtraction {
    pub state: ExtractionState,
    /// Fixed reason code when there is no text, e.g. `no_text_layer`,
    /// `not_a_pdf`, `drm`, `zip_slip`, `entry_too_large`, `worker_crashed`.
    pub reason: Option<String>,
}

/// Files by text status, over available files.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExtractionCounts {
    pub done: u32,
    pub pending: u32,
    pub needs_ocr: u32,
    pub encrypted: u32,
    pub cloud_only: u32,
    pub unreadable: u32,
    pub total: u32,
    /// Files with text (counted in `done`) being read again by a newer
    /// reader; their old text stays searchable meanwhile.
    #[serde(default)]
    pub updating: u32,
}

/// The extraction job shown in the status bar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExtractionStatus {
    pub job_id: RecordId,
    pub source_root_id: RecordId,
    pub source_root_display_path: String,
    pub state: JobState,
    pub pause_reason: Option<PauseReason>,
    pub throttle_reason: Option<ThrottleReason>,
    /// Files of this folder with a final text status.
    pub processed_files: u32,
    pub pending_files: u32,
    /// The file being read now, while running.
    pub current_file_id: Option<RecordId>,
    /// Stable code of the most recent problem, e.g. `extractor_unavailable`.
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RootExtractionSummary {
    pub source_root_id: RecordId,
    pub counts: ExtractionCounts,
    pub last_state: Option<JobState>,
    pub active_job_id: Option<RecordId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExtractionOverview {
    /// The job to show: running, then policy-paused, then paused by the
    /// user, then retrying, then queued.
    pub active: Option<ExtractionStatus>,
    pub queued_jobs: u32,
    pub counts: ExtractionCounts,
    pub roots: Vec<RootExtractionSummary>,
    /// Cataloging is still finding files, so totals may still grow; the UI
    /// then shows counts only, never "of N" (phase0-spec §5.6).
    pub scanning: bool,
}

// ------------------------------------------------------------- embedding

/// Files with text and where they stand for the active embedding model
/// (phase2-spec §5.2). Counts are files: copies count separately, as in
/// the reading counts. Files without text are never waiting for meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EmbeddingCounts {
    pub indexed: u32,
    pub waiting: u32,
    /// Files whose text the model refused, after every attempt.
    pub failed: u32,
    pub total: u32,
}

/// One embed job (one library folder).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EmbeddingStatus {
    pub job_id: RecordId,
    pub source_root_id: RecordId,
    pub source_root_display_path: String,
    pub state: JobState,
    pub pause_reason: Option<PauseReason>,
    pub throttle_reason: Option<ThrottleReason>,
    /// A fixed code: `provider_unreachable`, `provider_auth`,
    /// `model_not_found`, `model_changed`, `address_rejected`.
    pub last_error: Option<String>,
    /// Chunks sent to the service, their characters, and the time spent
    /// waiting for it, by this job.
    #[ts(type = "number")]
    pub embedded_chunks: u64,
    #[ts(type = "number")]
    pub sent_chars: u64,
    #[ts(type = "number")]
    pub service_ms: u64,
}

/// The model the job fills.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EmbeddingModelInfo {
    pub name: String,
    pub dims: u32,
    /// The service's label and place, when it still exists.
    pub profile_label: Option<String>,
    pub location: Option<ProviderLocation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RootEmbeddingSummary {
    pub source_root_id: RecordId,
    pub counts: EmbeddingCounts,
    pub active_job_id: Option<RecordId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct EmbeddingOverview {
    /// The job to show: running, then policy-paused, then paused by the
    /// user, then retrying, then queued.
    pub active: Option<EmbeddingStatus>,
    pub queued_jobs: u32,
    pub counts: EmbeddingCounts,
    /// `None` until an embedding model is chosen.
    pub model: Option<EmbeddingModelInfo>,
    /// A new model is being filled while the previous one serves search.
    pub rebuilding: bool,
    /// No embedding model is chosen.
    pub needs_model: bool,
    /// An AI service is saved. Without one, Atlas says nothing about
    /// meaning: it works without AI.
    pub has_ai_service: bool,
    /// The embedding service is on the internet and embedding the library
    /// through it isn't allowed in Settings › Privacy.
    pub needs_permission: bool,
    pub roots: Vec<RootEmbeddingSummary>,
    /// Text is still being read, so totals may grow.
    pub reading: bool,
}

// ------------------------------------------------------------- documents

/// Where a document's displayed title came from (phase1-spec P1-18).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TitleSource {
    /// The file's own metadata (PDF Info, EPUB `dc:title`), cleaned.
    Metadata,
    /// The largest line on a PDF's first page.
    Contents,
    /// The file name, tidied.
    FileName,
}

/// What the library's data takes on disk (Settings › Storage, phase1-spec
/// §5.5, §8). Catalog sizes are exact (SQLite's `dbstat`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StorageUsage {
    /// `catalog.sqlite` with its `-wal` and `-shm` files.
    #[ts(type = "number")]
    pub catalog_bytes: u64,
    /// The stored text (`passages`), part of the catalog.
    #[ts(type = "number")]
    pub text_bytes: u64,
    /// The full-text index (`passages_fts`), part of the catalog.
    #[ts(type = "number")]
    pub index_bytes: u64,
    /// The rest of the catalog: files, folders, documents, jobs.
    #[ts(type = "number")]
    pub other_catalog_bytes: u64,
    /// Rebuildable files in `cache/` and `tmp/`.
    #[ts(type = "number")]
    pub cache_bytes: u64,
    /// `None` when the drive can't be asked.
    #[ts(type = "number | null")]
    pub data_volume_free_bytes: Option<u64>,
    pub estimate: IndexingEstimate,
    /// The meaning index (Slice 2.3): its rows in the catalog (chunks,
    /// vectors, their state), part of `catalog_bytes`.
    #[ts(type = "number")]
    pub meaning_catalog_bytes: u64,
    /// The index files under `indexes/vectors/`.
    #[ts(type = "number")]
    pub meaning_file_bytes: u64,
    /// What indexing the meaning of the files still waiting will add;
    /// `None` until an embedding model is chosen.
    pub meaning_estimate: Option<MeaningEstimate>,
    /// `library/knowledge.sqlite` with its `-wal` and `-shm` files
    /// (Slice 3.2): what the user creates. Not part of the catalog, and
    /// never touched by Clean caches.
    #[ts(type = "number")]
    pub knowledge_bytes: u64,
    /// The snapshots in `backups/knowledge/`. Never touched by Clean caches.
    #[ts(type = "number")]
    pub snapshot_bytes: u64,
    pub snapshot_count: u32,
    pub measured_at: String,
}

/// A range, as the text estimate's (phase2-spec §5.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct MeaningEstimate {
    pub remaining_files: u32,
    #[ts(type = "number")]
    pub low_bytes: u64,
    #[ts(type = "number")]
    pub high_bytes: u64,
}

/// How much the text still to be read will need (phase1-spec §8): always a
/// range, and only after enough documents have been read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum IndexingEstimate {
    /// Fewer than `needed` documents read so far.
    NotEnoughRead { done: u32, needed: u32 },
    /// No file is waiting to be read.
    AllRead,
    Range {
        #[serde(rename = "remainingFiles")]
        remaining_files: u32,
        #[serde(rename = "lowBytes")]
        #[ts(type = "number")]
        low_bytes: u64,
        #[serde(rename = "highBytes")]
        #[ts(type = "number")]
        high_bytes: u64,
    },
}

/// What Clean caches removed, and the usage afterwards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CleanCachesResult {
    #[ts(type = "number")]
    pub freed_bytes: u64,
    pub usage: StorageUsage,
}

/// Groups of files with identical content (Library › Duplicates, Slice
/// 1.6). A group is a document with at least two available copies inside
/// the filters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DuplicateQuery {
    pub source_root_id: Option<RecordId>,
    pub file_type: Option<FileType>,
    /// Also list missing copies (they never count toward a group).
    #[serde(default)]
    pub include_missing: bool,
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DuplicateGroup {
    pub document_id: RecordId,
    pub title: String,
    pub title_source: TitleSource,
    /// The size of one copy.
    #[ts(type = "number | null")]
    pub size_bytes: Option<u64>,
    /// Space the extra available copies take: (copies − 1) × size.
    #[ts(type = "number")]
    pub extra_bytes: u64,
    /// Available copies first, then by path.
    pub copies: Vec<FileInstance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DuplicatePage {
    /// Most extra space first.
    pub groups: Vec<DuplicateGroup>,
    pub page: u32,
    pub page_size: u32,
    pub total_groups: u32,
    /// Extra space across every group, not just this page.
    #[ts(type = "number")]
    pub extra_bytes: u64,
}

/// Everything the detail panel shows for one file (phase1-spec §5.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocumentDetail {
    /// The file the panel was opened for, with its text status.
    pub file: FileInstance,
    /// Set once the file's bytes have been hashed.
    pub document_id: Option<RecordId>,
    /// Never empty: metadata, then the first page, then the file name.
    pub title: String,
    pub title_source: TitleSource,
    /// Cleaned: placeholders dropped, `;`-separated names split.
    pub authors: Vec<String>,
    pub language: Option<String>,
    /// PDF pages, or EPUB spine documents.
    pub page_count: Option<u32>,
    #[ts(type = "number | null")]
    pub char_count: Option<u64>,
    /// The other cataloged copies of the same content, missing ones
    /// included (and marked).
    pub also_at: Vec<FileInstance>,
}

// ---------------------------------------------------------------- search

/// A full-text search (phase1-spec §7.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchQuery {
    /// What the user typed: words, "quoted phrases", a trailing `*` for a
    /// prefix. Never FTS syntax.
    pub text: String,
    pub source_root_id: Option<RecordId>,
    pub file_type: Option<FileType>,
    #[serde(default)]
    pub include_missing: bool,
    pub page: u32,
    pub page_size: u32,
}

/// Part of a snippet; `hit` marks matched words. Plain text, never markup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SnippetSpan {
    pub text: String,
    pub hit: bool,
}

/// Why a result matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MatchReason {
    Text,
    Name,
    /// In the text and the name.
    Both,
    /// Similar meaning only (Slice 2.4).
    Semantic,
    /// In the text and similar meaning.
    TextSemantic,
}

/// Why meaning search did not take part in a search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SemanticUnavailable {
    /// The embedding service is on the internet and not allowed.
    NotPermitted,
    /// The embedding service couldn't be reached.
    Unreachable,
    /// It took longer than search waits for (1.5 s).
    Timeout,
    /// The meaning index is being opened; the next search can use it.
    Loading,
    /// Anything else (the service refused the query, a damaged index).
    Failed,
}

/// Whether meaning search took part (phase2-spec §5.3, §7.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
#[ts(export)]
pub enum SemanticStatus {
    /// No embedding model is chosen: text search only.
    #[default]
    Off,
    /// Meaning search ran over the files indexed so far.
    Ready {
        #[serde(rename = "coveragePercent")]
        coverage_percent: u8,
    },
    Unavailable {
        reason: SemanticUnavailable,
    },
}

/// One document (or, for a name match on a file not read yet, one file).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchHit {
    pub document_id: Option<RecordId>,
    /// The file to open: an available copy inside the filters when there is
    /// one.
    pub file_instance_id: RecordId,
    /// Never empty: metadata, then the first page, then the file name.
    pub title: String,
    pub title_source: TitleSource,
    pub display_name: String,
    pub display_path: String,
    pub file_type: FileType,
    pub source_root_id: RecordId,
    pub availability: Availability,
    pub storage: FileStorage,
    /// Where the best passage is: "Page 12", "Chapter 3, part 2".
    pub location: Option<String>,
    pub page_no: Option<u32>,
    pub snippet: Vec<SnippetSpan>,
    pub why: MatchReason,
    /// False for results found by the any-words fallback.
    pub all_words: bool,
    /// Other cataloged copies of the same content.
    pub other_locations: u32,
    /// The passage shown, for a trail step's anchor (Slice 3.3). A
    /// catalog row id: never sent, never stored.
    #[serde(skip)]
    #[ts(skip)]
    pub passage: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchPage {
    pub hits: Vec<SearchHit>,
    pub page: u32,
    pub page_size: u32,
    /// Documents found within the search window.
    pub matched_documents: u32,
    /// The window of best passages was full: there may be more matches.
    pub capped: bool,
    /// Text reading is still running, so more may be found later.
    pub reading_text: bool,
    /// Available files with a final text status, and all available files.
    pub text_read_files: u32,
    pub total_files: u32,
    /// Whether meaning search took part, and if not, why (Slice 2.4).
    pub semantic: SemanticStatus,
}

// ---------------------------------------------------------------- related

/// Whether a document's related documents could be found (Slice 2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RelatedStatus {
    /// No embedding model is set up: the panel shows nothing.
    #[default]
    Off,
    /// Found (the list may be empty: nothing is close enough).
    Ready,
    /// The document hasn't been indexed for meaning yet (or failed).
    NotIndexed,
    /// The document has no text to compare.
    NoText,
    /// The meaning index is being opened; ask again shortly.
    Loading,
    /// The index couldn't be searched.
    Failed,
}

/// A document related by meaning to the one in the panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RelatedHit {
    pub document_id: RecordId,
    /// An available file of it.
    pub file_instance_id: RecordId,
    pub title: String,
    pub title_source: TitleSource,
    pub display_name: String,
    pub display_path: String,
    pub file_type: FileType,
    pub source_root_id: RecordId,
    pub availability: Availability,
    pub storage: FileStorage,
    /// Where the best matching passage is: "Page 12", "Chapter 3, part 2".
    pub location: Option<String>,
    pub page_no: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RelatedPage {
    pub status: RelatedStatus,
    /// Best first; never the document itself, its copies or a document
    /// with the same text.
    pub hits: Vec<RelatedHit>,
}

// ------------------------------------------------------------ Ask Atlas

/// A question for Ask Atlas (phase2-spec §5.5, §7.6), with the search
/// filters. Data, never prompt structure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AskQuery {
    pub question: String,
    #[serde(default)]
    pub source_root_id: Option<RecordId>,
    #[serde(default)]
    pub file_type: Option<FileType>,
}

/// One passage given to the model, numbered as the model cites it. The
/// list is exactly what was sent: "Sources used".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SourceUsed {
    /// The citation number, from 1.
    pub n: u32,
    pub document_id: RecordId,
    pub file_instance_id: RecordId,
    pub title: String,
    pub display_name: String,
    pub file_type: FileType,
    /// "Page 12", "Chapter 3, part 2".
    pub location: Option<String>,
    /// 1-based, for opening a PDF at the page.
    pub page_no: Option<u32>,
    /// The excerpt exactly as sent, as plain text.
    pub excerpt: String,
    /// The passage shown, for a trail step's anchor (Slice 3.3). A
    /// catalog row id: never sent, never stored.
    #[serde(skip)]
    #[ts(skip)]
    pub passage: Option<i64>,
}

/// An answer that started streaming; `answer-stream` events follow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AskStarted {
    pub answer_id: String,
    pub sources: Vec<SourceUsed>,
    /// Atlas found little about this: the answer may be thin.
    pub weak: bool,
    pub semantic: SemanticStatus,
}

/// What an internet service would receive, asked before the first request
/// to it (phase2-spec §5.7). Nothing was sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DisclosureNeeded {
    pub profile_id: RecordId,
    pub profile_label: String,
    pub base_url: String,
    pub excerpt_count: u32,
    /// The question and excerpts, in UTF-8 bytes.
    #[ts(type = "number")]
    pub bytes: u64,
}

/// The reply to `ask_atlas`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum AskReply {
    Started(AskStarted),
    /// The library has nothing on it; the model was not asked.
    NothingFound {
        semantic: SemanticStatus,
    },
    NeedsDisclosure(DisclosureNeeded),
}

/// Why an answer stopped early, in fixed codes; the UI words them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AnswerErrorCode {
    NetworkError,
    Timeout,
    AuthenticationFailed,
    ModelNotFound,
    /// The service reported an error (overloaded, out of memory, …).
    ServiceError,
    /// The service answered in a way Atlas doesn't understand.
    ProtocolMismatch,
    /// The answer passed Atlas's size limit.
    TooLong,
    /// The service finished without writing any answer (a thinking model
    /// can spend its whole allowance reasoning).
    NoAnswer,
}

/// One step of a streamed answer (the `answer-stream` event).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum AnswerStep {
    /// The model is reasoning before it answers.
    Thinking,
    /// More answer text. Citations appear as `[n]`, each one checked
    /// against the sources; invented ones are already removed.
    Delta { text: String },
    /// Source `n` was cited for the first time.
    Citation { n: u32 },
    /// The answer is complete.
    Done { cited: Vec<u32>, dropped: u32 },
    /// Stop was pressed; nothing more follows.
    Stopped,
    /// The answer stopped early; `partial` when some text had arrived,
    /// which is kept.
    Error {
        code: AnswerErrorCode,
        message: String,
        partial: bool,
        cited: Vec<u32>,
        dropped: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AnswerEvent {
    pub answer_id: String,
    pub step: AnswerStep,
}

/// Machine-readable pause reasons; display text is chosen by the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum PauseReason {
    BatteryLow,
    LowDiskSpace,
    DataRootMissing,
    SourceRootMissing,
    MemoryPressure,
    ThermalPressure,
    /// Embedding only: bulk embedding pauses on battery, where text
    /// extraction slows (product spec §9, Slice 2.3).
    OnBattery,
}

/// Progress of one scan session. Counts only: Atlas never shows a total or
/// percentage before traversal finishes (phase0-spec §5.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InventoryStatus {
    pub job_id: RecordId,
    pub source_root_id: RecordId,
    pub source_root_display_path: String,
    pub state: JobState,
    pub pause_reason: Option<PauseReason>,
    pub throttle_reason: Option<ThrottleReason>,
    pub discovered_files: u32,
    pub completed_directories: u32,
    pub failures: u32,
    pub skipped_symlinks: u32,
    pub skipped_entries: u32,
    #[ts(type = "number")]
    pub elapsed_ms: u64,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    /// Stable code of the most recent problem, if any.
    pub last_error: Option<String>,
}

/// Per-folder catalog summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RootInventorySummary {
    pub source_root_id: RecordId,
    pub file_count: u32,
    pub missing_count: u32,
    /// Files whose contents live only in iCloud, OneDrive, or a similar
    /// service; Atlas lists them without downloading them.
    pub cloud_only_count: u32,
    pub last_scan_at: Option<String>,
    pub last_state: Option<JobState>,
    pub active_job_id: Option<RecordId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InventoryOverview {
    /// The session to show in the status bar: running, then paused, then
    /// queued. `None` when nothing is scheduled.
    pub active: Option<InventoryStatus>,
    pub queued_sessions: u32,
    pub cataloged_files: u32,
    pub missing_files: u32,
    pub cloud_only_files: u32,
    pub roots: Vec<RootInventorySummary>,
}

// --------------------------------------------------------- model providers

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ProviderKind {
    Ollama,
    OpenaiCompatible,
}

/// Where a profile's API key is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CredentialStorage {
    None,
    /// The OS keychain (macOS Keychain, Windows Credential Manager, Linux
    /// Secret Service).
    Keychain,
    /// In memory only; it must be entered again after Atlas restarts.
    SessionOnly,
}

/// Where an AI service runs, judged from its address alone (ADR 0012):
/// a label for the user, not a security boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ProviderLocation {
    ThisComputer,
    /// A private or local-network address, or a name only a local network
    /// resolves.
    AnotherComputer,
    Internet,
}

/// A saved provider profile as the UI sees it. The key itself never
/// crosses IPC after it is saved; only where it is kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProviderProfile {
    pub id: RecordId,
    pub kind: ProviderKind,
    pub label: String,
    pub base_url: String,
    pub has_credential: bool,
    pub credential_storage: CredentialStorage,
    /// Per-profile acknowledgement for plain HTTP to a non-loopback host.
    pub insecure_http_acknowledged: bool,
    /// Where the service runs, for the privacy label (from the address).
    pub location: ProviderLocation,
    pub created_at: String,
    pub updated_at: String,
}

/// Input for creating or updating a profile. `api_key` is written straight
/// to OS credential storage and is never persisted in the catalog. Omit it
/// to keep an existing key; set `clear_api_key` to remove it. `Debug`
/// redacts the key.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProviderProfileInput {
    pub id: Option<RecordId>,
    pub kind: ProviderKind,
    pub label: String,
    pub base_url: String,
    pub api_key: Option<String>,
    #[serde(default)]
    pub clear_api_key: bool,
    pub insecure_http_acknowledged: bool,
}

impl std::fmt::Debug for ProviderProfileInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderProfileInput")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("label", &self.label)
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("clear_api_key", &self.clear_api_key)
            .field(
                "insecure_http_acknowledged",
                &self.insecure_http_acknowledged,
            )
            .finish()
    }
}

/// An AI service found answering on this computer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct LocalProviderSuggestion {
    pub kind: ProviderKind,
    pub label: String,
    pub base_url: String,
    /// It answered but requires an API key.
    pub needs_key: bool,
    pub model_count: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "target", rename_all = "snake_case")]
#[ts(export)]
pub enum ConnectionTestTarget {
    SavedProfile {
        #[serde(rename = "profileId")]
        profile_id: RecordId,
    },
    Candidate {
        profile: ProviderProfileInput,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ConnectionOutcome {
    /// The endpoint answered with a valid model listing. This confirms
    /// reachability only, not chat or embedding compatibility.
    Reachable,
    NoModelsListed,
    NetworkError,
    Timeout,
    AuthenticationFailed,
    ProtocolMismatch,
    InsecureUrlRejected,
    /// "Disable all external AI" is on and this is an internet service;
    /// nothing was sent.
    ExternalAiDisabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConnectionResult {
    pub outcome: ConnectionOutcome,
    /// Listed model identifiers. Listing a model does not mean Atlas has
    /// checked it can chat or produce embeddings.
    pub model_ids: Vec<String>,
    /// A short technical detail (for example the HTTP status). Never
    /// contains the API key.
    pub message: String,
    pub server_version: Option<String>,
    pub tested_at: String,
}

// ----------------------------------------------------------- model roles

/// What a model is used for (phase2-spec §7.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AiRole {
    Chat,
    Embedding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CapabilityOutcome {
    /// The model answered the fixed test request for this role.
    Works,
    /// No model id was given; nothing was sent.
    NeedsModelId,
    ModelNotFound,
    /// The model exists but can't do this job (a chat model asked for
    /// embeddings, or the reverse).
    WrongRole,
    AuthenticationFailed,
    ProtocolMismatch,
    NetworkError,
    Timeout,
    InsecureUrlRejected,
    /// "Disable all external AI" is on and this is an internet service;
    /// nothing was sent.
    ExternalAiDisabled,
}

/// The result of one capability test: a single fixed request, never
/// anything from the library (ADR 0012).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CapabilityResult {
    pub role: AiRole,
    pub model_id: String,
    pub outcome: CapabilityOutcome,
    /// Embeddings: the length of the vector the model returned.
    pub dimensions: Option<u32>,
    /// A short technical detail (for example the HTTP status). Never the
    /// API key, and never the model's reply.
    pub message: String,
    pub elapsed_ms: u32,
    pub tested_at: String,
}

/// The service and model assigned to a role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModelRole {
    pub role: AiRole,
    pub profile_id: RecordId,
    pub profile_label: String,
    pub location: ProviderLocation,
    pub model_id: String,
    pub dimensions: Option<u32>,
    /// When the capability test that justified the assignment passed.
    pub tested_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModelRoles {
    pub chat: Option<ModelRole>,
    pub embedding: Option<ModelRole>,
}

/// A local AI runtime Atlas can give steps for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AiRuntime {
    Omlx,
    Ollama,
    Lmstudio,
}

/// How to get one recommended model in one runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RuntimeSteps {
    pub runtime: AiRuntime,
    /// The id to use in Atlas once the model is installed.
    pub model_id: String,
    /// A command to copy, when the runtime has one.
    pub command: Option<String>,
    pub steps: Vec<String>,
    /// Checked to work with this runtime; otherwise the card says so.
    pub verified: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RecommendationKind {
    Recommended,
    Faster,
    HigherQuality,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModelRecommendation {
    pub id: String,
    pub label: String,
    pub kind: RecommendationKind,
    #[ts(type = "number")]
    pub size_bytes: u64,
    pub min_memory_gb: u32,
    pub dimensions: Option<u32>,
    pub why: String,
    pub runtimes: Vec<RuntimeSteps>,
    /// A model id the user's services already list that matches this
    /// recommendation.
    pub installed_as: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RoleRecommendations {
    pub role: AiRole,
    /// Recommended first, then at most one faster and one higher-quality
    /// alternative that fits this computer.
    pub models: Vec<ModelRecommendation>,
}

/// From the catalog bundled with this release and the device check; no
/// network involved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Recommendations {
    pub catalog_version: u32,
    pub total_memory_gb: Option<u32>,
    pub embedding: RoleRecommendations,
    pub chat: RoleRecommendations,
}

// ------------------------------------------------------------- data root

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DataRootState {
    /// No data root has been chosen yet (first run).
    NotConfigured,
    Open,
    /// The remembered location is absent, e.g. its drive is disconnected.
    Missing,
    InUse,
    Incompatible,
    Invalid,
}

/// Which data root Atlas is using, or why it cannot use the remembered one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DataRootStatus {
    pub state: DataRootState,
    pub display_path: Option<String>,
    pub root_id: Option<String>,
    /// Why the remembered root could not be opened.
    pub problem: Option<crate::error::ErrorPayload>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum VolumeKind {
    Local,
    Network,
    CloudSynced,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CandidateKind {
    /// Does not exist yet; Atlas will create it.
    NewFolder,
    EmptyFolder,
    ExistingLibrary,
    /// A previous creation attempt stopped part-way; Atlas can finish it.
    IncompleteLibrary,
    /// Contains other files; Atlas will not write into it.
    NonEmptyFolder,
    /// A file, not a folder.
    NotAFolder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CandidateProblem {
    NotWritable,
    NetworkVolume,
    CloudSynced,
    LowSpace,
    NewerSchema,
    InvalidManifest,
    /// The folder is inside another Atlas library.
    InsideLibrary,
}

/// The result of checking a prospective data-root location. Read-only:
/// inspecting never creates the folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DataRootCandidate {
    pub display_path: String,
    pub kind: CandidateKind,
    pub problems: Vec<CandidateProblem>,
    /// True when `choose_data_root` on this path is expected to succeed.
    pub can_use: bool,
    #[ts(type = "number | null")]
    pub free_bytes: Option<u64>,
    #[ts(type = "number | null")]
    pub total_bytes: Option<u64>,
    /// Space below which Atlas will not create a new library here.
    #[ts(type = "number | null")]
    pub required_free_bytes: Option<u64>,
    pub volume_kind: VolumeKind,
    /// For `non_empty_folder`: an "Atlas Library" subfolder to offer instead.
    pub suggested_subfolder: Option<String>,
    /// For `existing_library`: the library's ID from its manifest.
    pub root_id: Option<String>,
}

// -------------------------------------------------------------- settings

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct AppearanceSettings {
    pub theme: ThemePreference,
}

/// Portable, non-secret preferences stored in `config/settings.json`.
/// Unknown or missing fields fall back to defaults when read.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct Settings {
    pub appearance: AppearanceSettings,
    pub privacy: PrivacySettings,
}

/// Settings › Privacy (phase2-spec §5.7, ADR 0015). Services on this
/// computer or another one are never affected by these choices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct PrivacySettings {
    /// Embedding the library through an internet service (off by default).
    pub allow_remote_embedding: bool,
    /// "Disable all external AI": no request at all to an internet service
    /// (Ask Atlas, embedding, search, tests).
    pub disable_external_ai: bool,
    /// Internet services the user agreed to send Ask Atlas excerpts to,
    /// after seeing what is sent. Per profile and address.
    pub acknowledged: Vec<Acknowledgement>,
    /// "Record my trails" (Slice 3.3; on by default). Off, nothing is
    /// recorded and nothing else changes.
    pub record_trails: bool,
    /// "Remove unpinned trails after 90 days" (Slice 3.3; on by default).
    pub expire_trails: bool,
}

impl Default for PrivacySettings {
    fn default() -> Self {
        Self {
            allow_remote_embedding: false,
            disable_external_ai: false,
            acknowledged: Vec::new(),
            record_trails: true,
            expire_trails: true,
        }
    }
}

/// One agreement to the Ask Atlas disclosure (phase2-spec §5.7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Acknowledgement {
    pub profile_id: String,
    /// The address agreed to; a changed address asks again.
    pub base_url: String,
    pub label: String,
    pub at: String,
}

// ------------------------------------------------- knowledge (Phase 3, 3.2)

/// Whether the knowledge store (`library/knowledge.sqlite`, ADR 0016) can
/// be used. The library opens whatever this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum KnowledgeState {
    Ready,
    /// Made by a newer Atlas: left exactly as it is.
    Incompatible,
    /// Not a store Atlas can read: left exactly as it is.
    Damaged,
}

/// What a snapshot check did (phase3-spec §6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SnapshotOutcome {
    Taken,
    /// Nothing changed since the last snapshot.
    NotChanged,
    /// The newest snapshot is less than a day old.
    NotDue,
    /// The drive is nearly full: no snapshot, nothing deleted.
    LowSpace,
    /// The copy failed its check and was removed.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SnapshotStatus {
    pub count: u32,
    #[ts(type = "number")]
    pub bytes: u64,
    /// RFC 3339; `None` before the first snapshot.
    pub newest_at: Option<String>,
    /// The last check since the library opened.
    pub last_outcome: Option<SnapshotOutcome>,
}

/// Settings › Storage's "Your trails and notes" (phase3-spec §10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct KnowledgeStatus {
    pub state: KnowledgeState,
    /// The store with its `-wal` and `-shm` files.
    #[ts(type = "number")]
    pub bytes: u64,
    pub trails: u32,
    pub steps: u32,
    pub bookmarks: u32,
    pub findings: u32,
    pub collections: u32,
    pub snapshots: SnapshotStatus,
}

/// A place in a document, by value (phase3-spec §6.2): never a passage,
/// chunk or file row id, which don't survive a re-read or a move.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Anchor {
    pub document_id: String,
    /// Hex.
    pub content_hash: String,
    /// Shown when the document is gone.
    pub title: String,
    pub file_type: FileType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_no: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ordinal: Option<u32>,
    /// At most `ANCHOR_EXCERPT_CHARS`; absent for a whole document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AnchorState {
    /// The document is known and the excerpt is at the saved place.
    Found,
    /// The same content, the excerpt now somewhere else in it.
    FoundElsewhere,
    /// The document is known, but no file in the library has it now.
    NoFile,
    /// The catalog has no such document (yet).
    Unknown,
}

/// An anchor resolved against the catalog as it is now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ResolvedAnchor {
    pub state: AnchorState,
    pub document_id: Option<String>,
    /// The file to open: available and local first, then cloud-only.
    pub file_instance_id: Option<String>,
    pub page_no: Option<u32>,
    pub location: Option<String>,
    pub ordinal: Option<u32>,
    /// Whether the saved excerpt was found; false means the saved place
    /// is used as it is.
    pub excerpt_found: bool,
}

// ---------------------------------------------------- trails (Phase 3, 3.3)

/// Why `search_documents` ran (phase3-spec §7.1). Only a submit is a trail
/// step; paging, a live refresh and a filter change are refreshes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SearchIntent {
    Submit,
    #[default]
    Refresh,
}

/// How the user reached a source they opened, as the renderer says it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum OpenVia {
    /// A search result.
    Result,
    /// Source `n` of the answer `answerId`.
    Citation {
        #[serde(rename = "answerId")]
        #[ts(rename = "answerId")]
        answer_id: String,
        n: u32,
    },
    Related,
    Suggestion,
    Bookmark,
    Trail,
    /// The Library view: never a step.
    Library,
}

/// How a recorded source was reached: [`OpenVia`] without the ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Reached {
    Result,
    Citation,
    Related,
    Suggestion,
    Bookmark,
    Trail,
}

/// What the user did with a source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum OpenAction {
    /// The details panel.
    Details,
    /// The page view (Slice 3.7).
    Page,
    Open,
    Reveal,
}

/// Where in the document a source was opened, when the renderer knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct OpenPlace {
    pub page_no: Option<u32>,
}

/// A step's filters as they were.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct StepFilters {
    pub source_root_id: Option<RecordId>,
    /// The folder's name then.
    pub folder_name: Option<String>,
    pub file_type: Option<FileType>,
}

/// One of a search step's top results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StepResult {
    pub anchor: Anchor,
    pub why: MatchReason,
}

/// A submitted search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchStep {
    /// The payload's version.
    pub v: u32,
    pub words: String,
    pub filters: StepFilters,
    /// How many documents matched then.
    pub matched: u32,
    /// The top five then.
    pub results: Vec<StepResult>,
    pub semantic: SemanticStatus,
}

/// How a recorded question ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum AskEnding {
    Done,
    Stopped,
    Failed {
        code: AnswerErrorCode,
    },
    /// Nothing in the library to answer from; the model wasn't asked.
    NothingFound,
}

/// A source sent with a question: cited ones keep their excerpt as sent,
/// the others are anchors without one (phase3-spec §7.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct StepSource {
    pub n: u32,
    pub cited: bool,
    pub anchor: Anchor,
}

/// A question and its answer as delivered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AskStep {
    pub v: u32,
    pub question: String,
    pub filters: StepFilters,
    /// The answer's text as the screen received it, with its checked
    /// `[n]` markers. Untrusted model output: shown as text.
    pub answer: String,
    pub ending: AskEnding,
    pub cited: Vec<u32>,
    pub dropped: u32,
    pub weak: bool,
    pub semantic: SemanticStatus,
    /// The chat model, and where its service was.
    pub model: Option<String>,
    pub location: Option<ProviderLocation>,
    pub sources: Vec<StepSource>,
}

/// A source the user opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct OpenStep {
    pub v: u32,
    pub anchor: Anchor,
    pub via: Reached,
    /// In order, each once: the details panel, then Open, is one step.
    pub actions: Vec<OpenAction>,
}

/// A note the user added to the trail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct NoteStep {
    pub v: u32,
    pub text: String,
}

/// A trail step's content, stored as JSON (`trail_steps.payload`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum StepPayload {
    Search(SearchStep),
    Ask(AskStep),
    Open(OpenStep),
    Note(NoteStep),
}

/// A trail in a list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TrailSummary {
    pub id: String,
    /// The user's title, if they gave one.
    pub title: Option<String>,
    /// What to show: the title, else the first question or search.
    pub label: String,
    pub pinned: bool,
    pub step_count: u32,
    pub started_at: String,
    pub last_step_at: String,
}

/// A page of trails, newest activity first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TrailQuery {
    /// Matched against titles, questions and searches, folded.
    #[serde(default)]
    pub filter: String,
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TrailPage {
    pub trails: Vec<TrailSummary>,
    pub page: u32,
    pub page_size: u32,
    pub total: u32,
}

/// A step with its anchors resolved against the catalog now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TrailStep {
    pub id: String,
    pub seq: u32,
    pub at: String,
    /// The user's note on this step.
    pub note: Option<String>,
    pub payload: StepPayload,
    /// One per anchor in the payload, in order: a search's results, a
    /// question's sources, an open's anchor.
    pub anchors: Vec<ResolvedAnchor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TrailDetail {
    pub trail: TrailSummary,
    /// Oldest first.
    pub steps: Vec<TrailStep>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RecordingState {
    Recording,
    Paused,
    /// The knowledge store can't be used (Settings › Storage says why).
    Unavailable,
}

/// The status bar's trail indicator; `trail-changed` carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RecordingStatus {
    pub state: RecordingState,
    /// The trail the next step joins, if it is still current.
    pub current_trail_id: Option<String>,
    pub step_count: u32,
    /// Home's one-time note, after the first step, until dismissed.
    pub first_step_note: bool,
    /// The last step couldn't be saved; cleared by the next one that is.
    pub failed: bool,
}

// ------------------------------------------- saved (Phase 3, Slice 3.5)

/// Where in a file the renderer asks to save (phase3-spec §7.3). Rust turns
/// it into an anchor from the catalog and what it showed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum FilePlace {
    /// The passage of that file's hit in the last results.
    Result,
    /// Source `n` of the answer `answerId`, as it was sent.
    Citation {
        #[serde(rename = "answerId")]
        #[ts(rename = "answerId")]
        answer_id: String,
        n: u32,
    },
    /// A PDF page (the page view, Slice 3.7).
    Page {
        #[serde(rename = "pageNo")]
        #[ts(rename = "pageNo")]
        page_no: u32,
    },
    /// The whole document.
    Document,
}

/// What to bookmark, or to save as a passage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum SaveTarget {
    File {
        #[serde(rename = "fileId")]
        #[ts(rename = "fileId")]
        file_id: RecordId,
        place: FilePlace,
    },
    /// Anchor `index` of a recorded trail step, as it was recorded.
    Step {
        #[serde(rename = "stepId")]
        #[ts(rename = "stepId")]
        step_id: String,
        index: u32,
    },
}

/// Where a finding comes from: never its text, which Rust has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum FindingSource {
    /// The answer on screen (the last one ended).
    Answer {
        #[serde(rename = "answerId")]
        #[ts(rename = "answerId")]
        answer_id: String,
    },
    /// A question recorded in a trail.
    Step {
        #[serde(rename = "stepId")]
        #[ts(rename = "stepId")]
        step_id: String,
    },
    /// A passage.
    Passage { target: SaveTarget },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SavedKind {
    Bookmark,
    Finding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum FindingKind {
    Answer,
    Passage,
}

/// A source of a finding, as it was kept, and where it is now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SavedSource {
    pub n: u32,
    pub anchor: Anchor,
    pub resolved: ResolvedAnchor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Bookmark {
    pub id: String,
    pub anchor: Anchor,
    /// Where it is now (§6.2).
    pub resolved: ResolvedAnchor,
    pub note: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Finding {
    pub id: String,
    pub kind: FindingKind,
    /// An answer's question.
    pub question: Option<String>,
    /// The answer with its `[n]` markers, or the passage. Shown as text.
    pub text: String,
    pub sources: Vec<SavedSource>,
    pub note: Option<String>,
    /// The trail step it came from; it may have expired since.
    pub from_step_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Tagged `item`, not `kind`: a finding has a `kind` of its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "item", rename_all = "snake_case")]
#[ts(export)]
pub enum SavedItem {
    Bookmark(Bookmark),
    Finding(Finding),
}

/// A page of saved items, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SavedQuery {
    #[serde(default)]
    pub kind: Option<SavedKind>,
    /// Matched folded against titles, excerpts, questions, text and notes.
    #[serde(default)]
    pub filter: String,
    /// Only bookmarks and findings of this document.
    #[serde(default)]
    pub document_id: Option<RecordId>,
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SavedPage {
    pub items: Vec<SavedItem>,
    pub page: u32,
    pub page_size: u32,
    pub total: u32,
}

// ---------------------------- interests and projects (Phase 3, Slice 3.6)

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CollectionKind {
    Interest,
    Project,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum CollectionStatus {
    Active,
    /// Projects only.
    Finished,
    Archived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CollectionCounts {
    pub trails: u32,
    pub bookmarks: u32,
    pub findings: u32,
    pub documents: u32,
}

/// An interest or a project (phase3-spec §5.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Collection {
    pub id: String,
    pub kind: CollectionKind,
    pub name: String,
    pub description: Option<String>,
    /// Projects only.
    pub goal: Option<String>,
    pub status: CollectionStatus,
    pub counts: CollectionCounts,
    pub created_at: String,
    pub updated_at: String,
}

/// A new collection (no id) or changes to one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CollectionInput {
    #[serde(default)]
    pub id: Option<String>,
    pub kind: CollectionKind,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub goal: Option<String>,
    /// Left as it is when absent; new collections are active.
    #[serde(default)]
    pub status: Option<CollectionStatus>,
}

/// What to add to a collection or take out of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum CollectionItemRef {
    Trail {
        id: String,
    },
    Bookmark {
        id: String,
    },
    Finding {
        id: String,
    },
    /// A document, named by one of its files when added, by its id when
    /// removed.
    Document {
        #[serde(default, rename = "fileId")]
        #[ts(rename = "fileId")]
        file_id: Option<RecordId>,
        #[serde(default, rename = "documentId")]
        #[ts(rename = "documentId")]
        document_id: Option<RecordId>,
    },
}

/// A document in a collection, resolved now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CollectionDocument {
    pub document_id: String,
    /// The catalog's title now, when it knows the document.
    pub title: Option<String>,
    pub resolved: ResolvedAnchor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CollectionDetail {
    pub collection: Collection,
    pub trails: Vec<TrailSummary>,
    pub bookmarks: Vec<Bookmark>,
    pub findings: Vec<Finding>,
    pub documents: Vec<CollectionDocument>,
}

/// Which collections an item is in, for Add to….
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Membership {
    pub collection_ids: Vec<String>,
}

/// Why a document is suggested.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "seed", rename_all = "snake_case")]
#[ts(export)]
pub enum SuggestionReason {
    Interest {
        name: String,
    },
    Project {
        name: String,
    },
    /// The three most recent trails.
    Recent,
}

/// A document from the library near what the user is working on
/// (phase3-spec §7.4), from stored vectors only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Suggestion {
    pub hit: SearchHit,
    pub reason: SuggestionReason,
    /// The collection it came from; empty for recent trails.
    pub seed_id: String,
}

// --------------------------------------------- the page view (Slice 3.7)

/// Where the passage is on a rendered page, in the image's pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PageRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// A PDF page for the page view (phase3-spec §5.5, §7.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PageImage {
    /// 1-based.
    pub page_no: u32,
    pub page_count: u32,
    pub width: u32,
    pub height: u32,
    /// The PNG, base64, shown through a `data:` URL.
    pub png: String,
    /// Where the passage looked for is; empty when it wasn't found.
    pub rects: Vec<PageRect>,
    pub found: bool,
    /// The page's text as Atlas read it, for reading aloud and copying.
    pub text: String,
    pub from_cache: bool,
}
