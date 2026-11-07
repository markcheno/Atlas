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
    /// The Phase 0 slice this build implements, e.g. "0.1".
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
    #[ts(type = "number | null")]
    pub data_volume_free_bytes: Option<u64>,
    #[ts(type = "number | null")]
    pub data_volume_total_bytes: Option<u64>,
    pub power: PowerSource,
    pub battery_percent: Option<f32>,
    pub sampled_at: String,
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
    #[ts(type = "number")]
    pub size_bytes: u64,
    pub modified_at: String,
    pub availability: Availability,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct InventoryStatus {
    pub job_id: Option<RecordId>,
    pub source_root_id: Option<RecordId>,
    pub state: Option<JobState>,
    pub pause_reason: Option<PauseReason>,
    pub discovered_files: u32,
    pub completed_directories: u32,
    pub failures: u32,
    pub skipped_symlinks: u32,
    #[ts(type = "number")]
    pub elapsed_ms: u64,
}

// --------------------------------------------------------- model providers

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ProviderKind {
    Ollama,
    OpenaiCompatible,
}

/// A saved provider profile as the UI sees it. The credential itself never
/// crosses IPC after it is saved; only whether one exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProviderProfile {
    pub id: RecordId,
    pub kind: ProviderKind,
    pub label: String,
    pub base_url: String,
    pub has_credential: bool,
    /// Per-profile acknowledgement for plain HTTP to a non-loopback host.
    pub insecure_http_acknowledged: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// Input for creating or updating a profile. `api_key` is written straight
/// to OS credential storage and is never persisted in the catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProviderProfileInput {
    pub id: Option<RecordId>,
    pub kind: ProviderKind,
    pub label: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub insecure_http_acknowledged: bool,
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
    pub model_ids: Vec<String>,
    pub message: String,
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
