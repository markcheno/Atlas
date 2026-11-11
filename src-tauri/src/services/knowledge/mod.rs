//! The knowledge store (Phase 3, Slice 3.2; phase3-spec §6, ADR 0016):
//! everything the user creates (trails, bookmarks, findings, interests and
//! projects) in a database of its own, `library/knowledge.sqlite`.
//!
//! - It refers to the catalog by value: anchors (`anchor.rs`) hold a
//!   document id, its content hash and the place as it was, and are
//!   resolved against the catalog each time they're shown.
//! - It has its own writer thread (the catalog's code, named apart), its
//!   own migrations (`PRAGMA user_version`) and a small read pool.
//! - It is snapshotted without being asked (`snapshot.rs`).
//! - A store that can't be opened never blocks the library: it reports
//!   its state, and Atlas leaves the file exactly as it is.

pub mod anchor;
pub mod migrations;
pub mod snapshot;
pub mod store;

pub use store::{KnowledgeHandle, KnowledgeStore};

/// Where snapshots go, under the data root.
pub const SNAPSHOTS_DIR: &str = "backups/knowledge";
/// Where a snapshot is written and checked before it is renamed into
/// [`SNAPSHOTS_DIR`]; emptied when the library opens.
pub const SNAPSHOT_STAGING_DIR: &str = "backups/knowledge/staging";
