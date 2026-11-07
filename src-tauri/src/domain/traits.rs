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
    ProviderKind, SourceRoot,
};
use crate::error::AtlasResult;

/// Persistence for source roots and file instances (SQLite, Slice 0.2/0.3).
/// Scan-session and frontier methods are added with the frontier schema in
/// Slice 0.5. All writes go through one dedicated writer connection.
pub trait CatalogRepository {
    fn list_source_roots(&self) -> AtlasResult<Vec<SourceRoot>>;
    /// `path` is the native path; it is stored as bytes, never via `String`.
    fn add_source_root(&self, path: &Path) -> AtlasResult<SourceRoot>;
    /// Removes the root from Atlas only. Never touches files on disk.
    fn remove_source_root(&self, id: &str) -> AtlasResult<()>;
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

/// Device and resource readings (OS probe, Slice 0.4). Change notification
/// for power and wake events is added with the first real probe.
pub trait ResourceMonitor {
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
