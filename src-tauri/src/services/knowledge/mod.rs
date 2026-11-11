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
//! - Trails are recorded into it by the [`Recorder`] (Slice 3.3,
//!   ADR 0017) and read, changed and expired through [`trails`].

pub mod anchor;
pub mod meta;
pub mod migrations;
pub mod record;
pub mod saved;
pub mod snapshot;
pub mod store;
pub mod trails;

use std::sync::atomic::{AtomicBool, Ordering};

use crate::domain::dto::PrivacySettings;

pub use record::{AskRecord, Recorder, TrailEvents};
pub use store::{KnowledgeHandle, KnowledgeStore};

/// Settings › Privacy's trail choices, shared by the recorder and the
/// store's hourly thread, and updated when the user changes them.
#[derive(Debug)]
pub struct TrailSettings {
    record: AtomicBool,
    expire: AtomicBool,
}

impl TrailSettings {
    pub fn new(privacy: &PrivacySettings) -> Self {
        Self {
            record: AtomicBool::new(privacy.record_trails),
            expire: AtomicBool::new(privacy.expire_trails),
        }
    }

    pub fn set(&self, privacy: &PrivacySettings) {
        self.record.store(privacy.record_trails, Ordering::SeqCst);
        self.expire.store(privacy.expire_trails, Ordering::SeqCst);
    }

    /// "Record my trails".
    pub fn recording(&self) -> bool {
        self.record.load(Ordering::SeqCst)
    }

    /// "Remove unpinned trails after 90 days".
    pub fn expiring(&self) -> bool {
        self.expire.load(Ordering::SeqCst)
    }
}

impl Default for TrailSettings {
    fn default() -> Self {
        Self::new(&PrivacySettings::default())
    }
}

/// Where snapshots go, under the data root.
pub const SNAPSHOTS_DIR: &str = "backups/knowledge";
/// Where a snapshot is written and checked before it is renamed into
/// [`SNAPSHOTS_DIR`]; emptied when the library opens.
pub const SNAPSHOT_STAGING_DIR: &str = "backups/knowledge/staging";
