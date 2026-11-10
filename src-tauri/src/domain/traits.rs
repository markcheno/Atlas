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
    AiRole, CapabilityResult, ConnectionResult, DeviceSnapshot, FileType, ProviderKind, VolumeKind,
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

/// Source-root persistence (SQLite adapter: `services::catalog::repository`).
/// All writes go through the one dedicated writer connection. Other catalog
/// access lives with the services that own it: scan sessions and the
/// directory frontier in `services::inventory::db` (writer), browse queries
/// in `services::library` and open/reveal lookups in `services::source_open`
/// (both on the read-only pool, so they never wait behind writes).
pub trait CatalogRepository {
    /// Ordered by display path.
    fn list_source_root_records(&self) -> AtlasResult<Vec<SourceRootRecord>>;
    /// Fails with `SourceAlreadyAdded` if the same native path is stored.
    fn insert_source_root(&self, record: SourceRootRecord) -> AtlasResult<()>;
    /// Removes the record only; never touches files on disk. Returns
    /// whether a record existed.
    fn delete_source_root(&self, id: &str) -> AtlasResult<bool>;
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
/// One immediate child of a directory, from lightweight metadata only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntryInfo {
    pub name: OsString,
    pub kind: EntryKind,
    pub size_bytes: Option<u64>,
    pub modified: Option<SystemTime>,
    /// The file's contents are offloaded to a cloud service. For legacy
    /// iCloud placeholders (`.Name.pdf.icloud`) the source reports the real
    /// name and size, so the entry looks like the file it stands for.
    pub cloud_only: bool,
}

/// Streamed directory entries; errors are per entry.
pub type DirEntries<'a> = Box<dyn Iterator<Item = std::io::Result<DirEntryInfo>> + Send + 'a>;

/// Read-only directory enumeration (local filesystem adapter:
/// `infrastructure::local_source`). Entries are streamed so a huge
/// directory is never buffered in memory. Implementations must not read
/// file contents, follow symlinks, trigger cloud downloads of files, or
/// modify anything.
pub trait InventorySource: Send + Sync {
    fn read_dir(&self, dir: &Path) -> std::io::Result<DirEntries<'_>>;
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

/// An AI endpoint (Ollama and OpenAI-compatible). A connection test lists
/// models (Slice 0.7); a capability test sends one fixed request to one
/// model for one role (Slice 2.1, ADR 0012). Neither sends library content.
/// Dropping a returned future cancels the request.
pub trait ModelProvider {
    fn kind(&self) -> ProviderKind;
    fn test_connection(
        &self,
        base_url: &str,
        credential: Option<&str>,
    ) -> impl Future<Output = ConnectionResult> + Send;
    fn test_capability(
        &self,
        base_url: &str,
        credential: Option<&str>,
        role: AiRole,
        model: &str,
    ) -> impl Future<Output = CapabilityResult> + Send;
}

/// What an embedding backend is: the model name and the length of the
/// vectors it returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendInfo {
    pub model: String,
    pub dims: usize,
}

/// Why an embedding request produced no vectors (Slice 2.3, ADR 0014).
/// Never carries the request or answer body.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EmbedError {
    #[error("the backend returned {got} vectors for {sent} inputs")]
    WrongCount { sent: usize, got: usize },
    #[error("the embedding backend is unavailable")]
    Unavailable,
    /// Nothing answered, or the connection dropped.
    #[error("the embedding service could not be reached")]
    Network,
    #[error("the embedding service took too long to answer")]
    Timeout,
    /// 429: try again after `retry_after` when the service says.
    #[error("the embedding service asked Atlas to slow down")]
    RateLimited {
        retry_after: Option<std::time::Duration>,
    },
    /// 5xx: the service failed; it may say when to try again.
    #[error("the embedding service failed (HTTP {status})")]
    Server {
        status: u16,
        retry_after: Option<std::time::Duration>,
    },
    /// 401 or 403.
    #[error("the embedding service refused the key")]
    Auth,
    #[error("the embedding service doesn't have this model")]
    ModelNotFound,
    /// 400, 413 or 422: the service refused this input.
    #[error("the embedding service refused the request (HTTP {status})")]
    BadRequest { status: u16 },
    /// An answer of the wrong shape, too large, or not JSON.
    #[error("the embedding service answered in an unexpected way")]
    Protocol,
    /// The model now returns vectors of another length.
    #[error("the model returned {got} numbers per vector, not {expected}")]
    WrongDimensions { expected: usize, got: usize },
    /// The request was abandoned because the job is stopping.
    #[error("the request was cancelled")]
    Cancelled,
}

impl EmbedError {
    /// Worth retrying after a pause: the service may recover.
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::Network
                | Self::Timeout
                | Self::RateLimited { .. }
                | Self::Server { .. }
                | Self::Unavailable
        )
    }

    /// The wait the service asked for, if any.
    pub fn retry_after(&self) -> Option<std::time::Duration> {
        match self {
            Self::RateLimited { retry_after } | Self::Server { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

/// Turns text into vectors (Slice 2.2, ADR 0013). Exists because tests and
/// the benchmarks inject `HashedBagOfWordsBackend`; the provider-backed
/// implementation arrives with the embedding job (Slice 2.3). Returns one
/// vector of `describe().dims` numbers per input, in order.
pub trait EmbeddingBackend: Send + Sync {
    fn embed(&self, inputs: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError>;
    fn describe(&self) -> BackendInfo;
}

/// Opens or reveals an original document with OS-native facilities
/// (Slice 0.6). Callers must pass only paths of cataloged files inside an
/// authorized root; implementations never build shell command strings.
pub trait SourceOpener: Send + Sync {
    fn open_original(&self, path: &Path) -> AtlasResult<()>;
    fn reveal_original(&self, path: &Path) -> AtlasResult<()>;
}

/// Reserved for Phase 1 text extraction. No implementation exists, and none
/// may claim to extract content before Phase 1.
pub trait DocumentProcessor {
    fn id(&self) -> &'static str;
    fn supports(&self, file_type: FileType) -> bool;
}
