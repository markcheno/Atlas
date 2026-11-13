//! One way to read a file row and to mark files in disconnected folders,
//! shared by the Library, the detail panel and the duplicates view.

use std::collections::HashMap;

use rusqlite::Row;

use crate::domain::dto::{
    Availability, ExtractionState, FileExtraction, FileInstance, FileStorage, FileType,
    SourceRootStatus,
};
use crate::services::catalog::codec::from_db;
use crate::services::extraction::db::{EFFECTIVE_REASON_SQL, EFFECTIVE_STATE_SQL};

use super::RootStatusCache;

/// The columns [`file_from_row`] reads, in order. Needs `f`
/// (`file_instances`), `r` (`source_roots`) and `d` (`documents`, left
/// joined).
pub fn file_columns() -> String {
    format!(
        "f.id, f.source_root_id, f.display_name, f.display_path, f.extension, f.file_type,
         f.size_bytes, f.modified_at, f.availability, f.storage, f.document_id,
         {EFFECTIVE_STATE_SQL}, {EFFECTIVE_REASON_SQL}, r.path_bytes"
    )
}

/// The `FROM` clause [`file_columns`] needs.
pub const FILE_FROM: &str = "FROM file_instances f JOIN source_roots r ON r.id = f.source_root_id
     LEFT JOIN documents d ON d.id = f.document_id";

/// Number of columns [`file_columns`] selects; extra columns follow it.
pub const FILE_COLUMN_COUNT: usize = 14;

/// A file instance and its folder's path bytes (for reachability).
pub fn file_from_row(r: &Row) -> rusqlite::Result<(FileInstance, Vec<u8>)> {
    let file_type: String = r.get(5)?;
    let availability: String = r.get(8)?;
    let storage: String = r.get(9)?;
    let state: String = r.get(11)?;
    Ok((
        FileInstance {
            id: r.get(0)?,
            source_root_id: r.get(1)?,
            display_name: r.get(2)?,
            display_path: r.get(3)?,
            extension: r.get(4)?,
            file_type: from_db(&file_type).unwrap_or(FileType::Pdf),
            size_bytes: r.get::<_, Option<i64>>(6)?.map(|s| s.max(0) as u64),
            modified_at: r.get(7)?,
            availability: if availability == "missing" {
                Availability::Missing
            } else {
                Availability::Available
            },
            storage: if storage == "cloud_only" {
                FileStorage::CloudOnly
            } else {
                FileStorage::Local
            },
            document_id: r.get(10)?,
            other_format: None,
            extraction: FileExtraction {
                state: from_db(&state).unwrap_or(ExtractionState::Pending),
                reason: r.get(12)?,
            },
        },
        r.get(13)?,
    ))
}

/// Collects each folder once, for [`RootStatusCache::statuses`].
#[derive(Default)]
pub struct Roots(Vec<(String, Vec<u8>)>);

impl Roots {
    pub fn add(&mut self, file: &FileInstance, path: Vec<u8>) {
        if !self.0.iter().any(|(id, _)| *id == file.source_root_id) {
            self.0.push((file.source_root_id.clone(), path));
        }
    }

    /// Probes the folders (briefly cached) and returns a marker that turns
    /// files in disconnected folders `unreachable`.
    pub fn probe(&self, cache: &RootStatusCache) -> Reachability {
        Reachability(cache.statuses(&self.0))
    }
}

pub struct Reachability(HashMap<String, SourceRootStatus>);

impl Reachability {
    pub fn mark(&self, f: &mut FileInstance) {
        let reachable = self
            .0
            .get(&f.source_root_id)
            .is_none_or(|s| *s == SourceRootStatus::Available);
        if !reachable && f.availability == Availability::Available {
            f.availability = Availability::Unreachable;
        }
    }
}
