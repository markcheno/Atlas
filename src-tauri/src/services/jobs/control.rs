//! Job operations requested by the user (commands side), for any kind. Each
//! one is a conditional state change on the writer thread followed by a
//! wake-up, so the runners see it before their next unit of work.

use super::db;
use crate::domain::dto::{JobKind, JobState};
use crate::domain::traits::CatalogRepository;
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::clock::now_rfc3339;
use crate::services::catalog::repository::SqliteCatalog;
use crate::services::diagnostics::Event;
use crate::services::diagnostics::event::PauseBy;
use crate::services::workspace::OpenRoot;

/// Schedules a job of `kind` for a source root, or returns the one already
/// scheduled (including one the user paused, which stays paused). Callers
/// apply their kind's own rules first (scans refuse nested roots).
pub fn enqueue(root: &OpenRoot, source_root_id: &str, kind: JobKind) -> AtlasResult<String> {
    let exists = SqliteCatalog::new(root.catalog())
        .list_source_root_records()?
        .iter()
        .any(|r| r.id == source_root_id);
    if !exists {
        return Err(AtlasError::SourceNotFound);
    }
    let root_id = source_root_id.to_owned();
    let id = root.catalog().call(move |c| {
        if let Some(active) = db::active_for_root(c, &root_id, kind)? {
            return Ok(active.id);
        }
        let id = uuid::Uuid::new_v4().to_string();
        db::insert_job(c, &id, &root_id, kind, &now_rfc3339())?;
        Ok::<_, rusqlite::Error>(id)
    })?;
    let id = id.map_err(AtlasError::db("schedule cataloging"))?;
    root.wake_jobs();
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
        let Some(job) = db::job(c, &id)? else {
            return Ok(None);
        };
        let changed = db::transition(c, &id, to, from, None, None, &now_rfc3339())?;
        if changed && to == JobState::Cancelled {
            db::drop_frontier(c, &id)?;
        }
        Ok::<_, rusqlite::Error>(Some((changed, job.kind, job.state)))
    })?;
    match changed.map_err(AtlasError::db("update cataloging"))? {
        None => Err(AtlasError::JobNotFound),
        Some((false, _, _)) => Err(refused),
        Some((true, kind, was)) => {
            let by = PauseBy::User;
            match to {
                JobState::PausedUser => root.diagnostics().record(Event::JobPaused {
                    kind,
                    by,
                    reason: None,
                }),
                JobState::Queued if was == JobState::PausedUser => {
                    root.diagnostics().record(Event::JobResumed { kind, by });
                }
                _ => {}
            }
            root.wake_jobs();
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

/// Resumes a job the user paused, or retries one that failed in a way
/// worth retrying ("Retry" on a status line, Slice 2.3).
pub fn resume(root: &OpenRoot, job_id: &str) -> AtlasResult<()> {
    change(
        root,
        job_id,
        JobState::Queued,
        &[JobState::PausedUser, JobState::FailedRetryable],
        AtlasError::JobNotResumable,
    )
}

/// Stops the job. Its finished work (cataloged files) stays.
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
