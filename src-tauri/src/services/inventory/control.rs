//! Inventory operations requested by the user (commands side). Scans are
//! jobs of kind `scan`, so pause, resume and cancel are the generic job
//! operations ([`crate::services::jobs::control`]); only scheduling has a
//! scan-specific rule (nested folders are never scanned on their own).

use std::path::PathBuf;

use super::db;
use crate::domain::dto::{InventoryOverview, JobKind};
use crate::domain::traits::CatalogRepository;
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::native_path;
use crate::services::catalog::repository::SqliteCatalog;
use crate::services::jobs;
use crate::services::sources;
use crate::services::workspace::OpenRoot;

/// Starts cataloging a folder, or returns the job already scheduled for it
/// (including one the user paused, which stays paused).
pub fn enqueue(root: &OpenRoot, source_root_id: &str) -> AtlasResult<String> {
    let records = SqliteCatalog::new(root.catalog()).list_source_root_records()?;
    let paths: Vec<Option<PathBuf>> = records
        .iter()
        .map(|r| native_path::decode(&r.path_bytes))
        .collect();
    let Some((_, path)) = records
        .iter()
        .zip(&paths)
        .find(|(r, _)| r.id == source_root_id)
    else {
        return Err(AtlasError::SourceNotFound);
    };
    if let Some(outer) = path
        .as_ref()
        .and_then(|p| sources::outermost_container(p, &records, &paths))
    {
        return Err(AtlasError::SourceOverlaps {
            conflicting_root_id: outer,
        });
    }
    jobs::control::enqueue(root, source_root_id, JobKind::Scan)
}

/// A user pause survives restarts and is never lifted automatically.
pub fn pause(root: &OpenRoot, job_id: &str) -> AtlasResult<()> {
    jobs::control::pause(root, job_id)
}

pub fn resume(root: &OpenRoot, job_id: &str) -> AtlasResult<()> {
    jobs::control::resume(root, job_id)
}

/// Stops the job. Files already cataloged stay in the library.
pub fn cancel(root: &OpenRoot, job_id: &str) -> AtlasResult<()> {
    jobs::control::cancel(root, job_id)
}

/// The indexing status for the UI. Read on a pooled read-only connection,
/// so it never queues behind catalog writes (on Windows, waiting for the
/// writer made it take 0.4–0.9 s at p95 during scans; docs/benchmarks.md).
pub fn overview(root: &OpenRoot) -> AtlasResult<InventoryOverview> {
    root.reader()
        .read(db::overview)?
        .map_err(AtlasError::db("read cataloging status"))
}
