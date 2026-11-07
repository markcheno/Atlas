//! Service interfaces.
//!
//! Abstraction budget (Phase 0 spec, section 9): a trait exists here only
//! when tests need to inject a fake for it, or, for `DocumentProcessor`, to
//! reserve a name for Phase 1. The scheduler's job processor and scheduling
//! policy stay concrete until a second job kind exists.
//!
//! No implementations ship in Slice 0.1. Each trait notes the slice that
//! delivers its first built-in adapter. Signatures may be refined when that
//! adapter lands, but changes must keep the IPC DTOs stable.

use std::ffi::OsString;
use std::future::Future;
use std::path::Path;
use std::time::SystemTime;

use super::dto::{
    ConnectionResult, DeviceSnapshot, DocumentPage, DocumentQuery, FileInstance, FileType,
    ProviderKind, VolumeKind,
};
use crate::error::AtlasResult;

/// A stored source root. `path_bytes` is the platform-tagged native path
/// (see `infrastructure::native_path`); it never passes through `String`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRootRecord {
    pub id: String,
    pub workspace_id: String,
    pub path_bytes: Vec<u8>,
    pub display_path: String,
    pub created_at: String,
    pub last_scan_at: Option<String>,
}

/// Catalog persistence (SQLite adapter: `services::catalog::repository`).
/// Source roots arrived in Slice 0.3; scan sessions and the directory
/// frontier are added in 0.5, and document queries are implemented in 0.6.
/// All writes go through the one dedicated writer connection.
pub trait CatalogRepository {
    /// Ordered by display path.
    fn list_source_root_records(&self) -> AtlasResult<Vec<SourceRootRecord>>;
    /// Fails with `SourceAlreadyAdded` if the same native path is stored.
    fn insert_source_root(&self, record: SourceRootRecord) -> AtlasResult<()>;
    /// Removes the record only; never touches files on disk. Returns
    /// whether a record existed.
    fn delete_source_root(&self, id: &str) -> AtlasResult<bool>;
    fn list_documents(&self, query: &DocumentQuery) -> AtlasResult<DocumentPage>;
    fn file_instance(&self, id: &str) -> AtlasResult<Option<FileInstance>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
    /// Skipped in Phase 0, but counted.
    Symlink,
    Other,
}

/// One immediate child of a directory, from lightweight metadata only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntryInfo {
    pub name: OsString,
    pub kind: EntryKind,
    pub size_bytes: Option<u64>,
    pub modified: Option<SystemTime>,
}

/// Read-only directory enumeration (local filesystem, Slice 0.5).
/// Entries are streamed so a huge directory is never buffered in memory.
/// Implementations must not read file contents or modify anything.
pub trait InventorySource {
    type Entries: Iterator<Item = AtlasResult<DirEntryInfo>>;
    fn read_dir(&self, dir: &Path) -> AtlasResult<Self::Entries>;
}

/// Facts about the filesystem volume holding a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeInfo {
    pub free_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    /// Filesystem name as the OS reports it (e.g. "apfs", "exfat"), if known.
    pub fs_type: Option<String>,
    pub kind: VolumeKind,
}

/// Inspects the volume that holds (or would hold) a path. For a path that
/// does not exist yet, implementations inspect its nearest existing
/// ancestor. A trait so tests can simulate network, cloud-synced, and
/// nearly full volumes (Slice 0.2; reused by the 0.4 storage guard).
pub trait VolumeInspector: Send + Sync {
    fn inspect(&self, path: &Path) -> AtlasResult<VolumeInfo>;
}

/// Device and resource readings (OS probe: `infrastructure::system_probe`).
/// Sampled on demand; the 0.5 scan loop re-samples periodically.
pub trait ResourceMonitor: Send + Sync {
    /// `data_root` selects the volume whose free space is reported.
    fn snapshot(&self, data_root: Option<&Path>) -> DeviceSnapshot;
}

/// Connection probe for an AI endpoint (Ollama and OpenAI-compatible,
/// Slice 0.7). Lists models only; never sends library content. Dropping the
/// returned future cancels the request.
pub trait ModelProvider {
    fn kind(&self) -> ProviderKind;
    fn test_connection(
        &self,
        base_url: &str,
        credential: Option<&str>,
    ) -> impl Future<Output = ConnectionResult> + Send;
}

/// Opens or reveals an original document with OS-native facilities
/// (Slice 0.6). Callers must pass only paths of cataloged files inside an
/// authorized root; implementations never build shell command strings.
pub trait SourceOpener {
    fn open_original(&self, path: &Path) -> AtlasResult<()>;
    fn reveal_original(&self, path: &Path) -> AtlasResult<()>;
}

/// Reserved for Phase 1 text extraction. No implementation exists, and none
/// may claim to extract content before Phase 1.
pub trait DocumentProcessor {
    fn id(&self) -> &'static str;
    fn supports(&self, file_type: FileType) -> bool;
}
