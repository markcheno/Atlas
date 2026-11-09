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
    Both,
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
    /// The address is this computer (loopback), for the privacy indicator.
    pub on_this_computer: bool,
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
}
