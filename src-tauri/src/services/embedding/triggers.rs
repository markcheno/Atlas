//! When embed jobs are queued (phase2-spec §7.2): when a folder's text has
//! been read, when the library opens, when the embedding role changes or
//! embedding through an internet service is allowed, and on "Index
//! meaning now".

use rusqlite::Connection;

use super::{db, models, status};
use crate::domain::dto::{JobKind, JobState};
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::clock::now_rfc3339;
use crate::services::jobs;
use crate::services::workspace::OpenRoot;

/// Queues an embed job for every folder with documents the active model
/// still needs, if a role is set and its service may be used. Returns how
/// many it queued.
pub fn queue_waiting(conn: &Connection, allow_remote: bool, now: &str) -> rusqlite::Result<usize> {
    let Some(model) = runnable(conn, allow_remote)? else {
        return Ok(0);
    };
    let roots: Vec<String> = conn
        .prepare("SELECT id FROM source_roots")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut queued = 0;
    for root in roots {
        if db::enqueue_if_waiting(conn, &root, &model, now)? {
            queued += 1;
        }
    }
    Ok(queued)
}

/// As [`queue_waiting`], for one folder (its text was just read).
pub fn queue_root(
    conn: &Connection,
    root_id: &str,
    allow_remote: bool,
    now: &str,
) -> rusqlite::Result<bool> {
    match runnable(conn, allow_remote)? {
        Some(model) => db::enqueue_if_waiting(conn, root_id, &model, now),
        None => Ok(false),
    }
}

/// The active model, when the job could run for it now.
fn runnable(conn: &Connection, allow_remote: bool) -> rusqlite::Result<Option<String>> {
    if models::role(conn)?.is_none() || !status::permitted(conn, allow_remote)? {
        return Ok(None);
    }
    Ok(models::active(conn)?.map(|m| m.id))
}

/// After the embedding role or its service changed, or the privacy
/// setting: bring the model rows in line and queue what's waiting.
pub fn changed(root: &OpenRoot) -> AtlasResult<()> {
    let allow = crate::services::providers::privacy::remote_embedding(&root.settings().privacy);
    root.catalog()
        .call(move |c| {
            let now = now_rfc3339();
            models::sync(c, &now)?;
            queue_waiting(c, allow, &now)
        })?
        .map_err(AtlasError::db("queue meaning indexing"))?;
    root.wake_jobs();
    Ok(())
}

/// "Index meaning now" for a folder: queues its embed job, or retries one
/// that failed for now. Idempotent. Returns the job's id.
pub fn start(root: &OpenRoot, source_root_id: &str) -> AtlasResult<String> {
    changed(root)?;
    let id = source_root_id.to_owned();
    root.catalog()
        .call(move |c| {
            if let Some(job) = jobs::db::active_for_root(c, &id, JobKind::Embed)?
                && job.state == JobState::FailedRetryable
            {
                jobs::db::transition(
                    c,
                    &job.id,
                    JobState::Queued,
                    &[JobState::FailedRetryable],
                    None,
                    None,
                    &now_rfc3339(),
                )?;
            }
            Ok::<_, rusqlite::Error>(())
        })?
        .map_err(AtlasError::db("queue meaning indexing"))?;
    jobs::control::enqueue(root, source_root_id, JobKind::Embed)
}
