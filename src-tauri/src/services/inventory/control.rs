//! Inventory operations requested by the user (commands side). Scans are
//! jobs of kind `scan`, so pause, resume and cancel are the generic job
//! operations ([`crate::services::jobs::control`]); only scheduling has a
//! scan-specific rule (nested folders are never scanned on their own).

use std::path::PathBuf;

use super::db;
use crate::domain::dto::{
    InventoryOverview, JobKind, JobState, ScanOrigin, ScanProblem, ScanRecord, SourceRootStatus,
};
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
    enqueue_from(root, source_root_id, ScanOrigin::User)
}

/// Rescan (Slice 4.5): as [`enqueue`], except that a scan the user paused
/// is resumed, so the button always does what it says.
pub fn rescan(root: &OpenRoot, source_root_id: &str) -> AtlasResult<String> {
    let id = enqueue_from(root, source_root_id, ScanOrigin::User)?;
    let job_id = id.clone();
    let paused = root
        .catalog()
        .call(move |c| jobs::db::job(c, &job_id))?
        .map_err(AtlasError::db("read cataloging"))?
        .is_some_and(|j| j.state == JobState::PausedUser);
    if paused {
        jobs::control::resume(root, &id)?;
    }
    Ok(id)
}

/// Check all folders now (Slice 4.5): a scan for every folder that is
/// reachable and not inside another, unless one is already scheduled.
/// Returns how many folders have a scan scheduled.
pub fn check_all(root: &OpenRoot) -> AtlasResult<u32> {
    let records = SqliteCatalog::new(root.catalog()).list_source_root_records()?;
    let paths: Vec<Option<PathBuf>> = records
        .iter()
        .map(|r| native_path::decode(&r.path_bytes))
        .collect();
    let reachable = sources::probe::probe_all(&paths);
    let mut scheduled = 0;
    for ((record, path), status) in records.iter().zip(&paths).zip(reachable) {
        let nested = path
            .as_ref()
            .is_some_and(|p| sources::outermost_container(p, &records, &paths).is_some());
        if nested || status != SourceRootStatus::Available {
            continue;
        }
        rescan(root, &record.id)?;
        scheduled += 1;
    }
    Ok(scheduled)
}

/// As [`enqueue`], recording why the scan was made.
pub fn enqueue_from(
    root: &OpenRoot,
    source_root_id: &str,
    origin: ScanOrigin,
) -> AtlasResult<String> {
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
    jobs::control::enqueue_from(root, source_root_id, JobKind::Scan, origin)
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

/// A folder's recent scans, newest first (Slice 4.5).
pub fn scans(root: &OpenRoot, source_root_id: &str, limit: u32) -> AtlasResult<Vec<ScanRecord>> {
    let id = source_root_id.to_owned();
    root.reader()
        .read(move |c| db::scans(c, &id, limit.clamp(1, db::SCANS_WITH_PROBLEMS * 5)))?
        .map_err(AtlasError::db("read recent checks"))
}

/// The folders a scan couldn't read (Slice 4.5).
pub fn problems(root: &OpenRoot, job_id: &str) -> AtlasResult<Vec<ScanProblem>> {
    let id = job_id.to_owned();
    root.reader()
        .read(move |c| db::problems(c, &id))?
        .map_err(AtlasError::db("read what a check couldn't read"))
}

/// The indexing status for the UI. Read on a pooled read-only connection,
/// so it never queues behind catalog writes (on Windows, waiting for the
/// writer made it take 0.4–0.9 s at p95 during scans; docs/benchmarks.md).
pub fn overview(root: &OpenRoot) -> AtlasResult<InventoryOverview> {
    root.reader()
        .read(db::overview)?
        .map_err(AtlasError::db("read cataloging status"))
}
