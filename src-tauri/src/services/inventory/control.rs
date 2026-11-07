//! Inventory operations requested by the user (commands side). Each one is
//! a conditional state change on the writer thread followed by a wake-up,
//! so the worker sees it before its next batch.

use std::path::PathBuf;

use super::db;
use crate::domain::dto::{InventoryOverview, JobState};
use crate::domain::traits::CatalogRepository;
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::clock::now_rfc3339;
use crate::infrastructure::native_path;
use crate::services::catalog::repository::SqliteCatalog;
use crate::services::sources;
use crate::services::workspace::OpenRoot;

fn wake(root: &OpenRoot) {
    if let Some(runner) = root.inventory() {
        runner.wake();
    }
}

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

    let root_id = source_root_id.to_owned();
    let id = root.catalog().call(move |c| {
        if let Some(active) = db::active_for_root(c, &root_id)? {
            return Ok(active.id);
        }
        let id = uuid::Uuid::new_v4().to_string();
        db::insert_session(c, &id, &root_id, &now_rfc3339())?;
        Ok::<_, rusqlite::Error>(id)
    })?;
    let id = id.map_err(AtlasError::db("schedule cataloging"))?;
    wake(root);
    Ok(id)
}

fn change(
    root: &OpenRoot,
    job_id: &str,
    to: JobState,
    from: &'static [JobState],
    refused: AtlasError,
) -> AtlasResult<()> {
    let id = job_id.to_owned();
    let changed = root.catalog().call(move |c| {
        if db::session(c, &id)?.is_none() {
            return Ok(None);
        }
        let changed = db::transition(c, &id, to, from, None, None, &now_rfc3339())?;
        if changed && to == JobState::Cancelled {
            db::drop_frontier(c, &id)?;
        }
        Ok::<_, rusqlite::Error>(Some(changed))
    })?;
    match changed.map_err(AtlasError::db("update cataloging"))? {
        None => Err(AtlasError::JobNotFound),
        Some(false) => Err(refused),
        Some(true) => {
            wake(root);
            Ok(())
        }
    }
}

const PAUSABLE: &[JobState] = &[
    JobState::Queued,
    JobState::Running,
    JobState::PausedPolicy,
    JobState::FailedRetryable,
];

/// A user pause survives restarts and is never lifted automatically.
pub fn pause(root: &OpenRoot, job_id: &str) -> AtlasResult<()> {
    change(
        root,
        job_id,
        JobState::PausedUser,
        PAUSABLE,
        AtlasError::JobNotPausable,
    )
}

pub fn resume(root: &OpenRoot, job_id: &str) -> AtlasResult<()> {
    change(
        root,
        job_id,
        JobState::Queued,
        &[JobState::PausedUser],
        AtlasError::JobNotResumable,
    )
}

/// Stops the job. Files already cataloged stay in the library.
pub fn cancel(root: &OpenRoot, job_id: &str) -> AtlasResult<()> {
    const CANCELLABLE: &[JobState] = &[
        JobState::Queued,
        JobState::Running,
        JobState::PausedPolicy,
        JobState::PausedUser,
        JobState::FailedRetryable,
    ];
    change(
        root,
        job_id,
        JobState::Cancelled,
        CANCELLABLE,
        AtlasError::JobNotPausable,
    )
}

pub fn overview(root: &OpenRoot) -> AtlasResult<InventoryOverview> {
    root.catalog()
        .call(|c| db::overview(c))?
        .map_err(AtlasError::db("read cataloging status"))
}
