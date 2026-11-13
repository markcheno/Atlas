//! SQL for job rows (every kind). Every function runs on the catalog writer
//! thread, except reads that callers pass to the reader pool.
//!
//! State changes are conditional (`… WHERE state IN (…)`) so a runner and
//! user commands can never overwrite each other's decisions, for example a
//! policy pause can never replace a user pause (ADR 0005, ADR 0009).

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::domain::dto::{JobKind, JobState, PauseReason, ScanOrigin, ThrottleReason};
use crate::services::catalog::codec::{from_db, opt_from_db, to_db};

/// States that still have work to do or could resume.
pub const ACTIVE_STATES: &[JobState] = &[
    JobState::Queued,
    JobState::Running,
    JobState::PausedPolicy,
    JobState::PausedUser,
    JobState::FailedRetryable,
];

pub fn state_list(states: &[JobState]) -> String {
    states
        .iter()
        .map(|s| format!("'{}'", to_db(s)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// One job with its source root. The counters are the scan's; other kinds
/// leave them at zero.
#[derive(Debug, Clone)]
pub struct JobRow {
    pub id: String,
    pub source_root_id: String,
    pub root_path_bytes: Vec<u8>,
    pub root_display: String,
    pub kind: JobKind,
    pub state: JobState,
    pub pause_reason: Option<PauseReason>,
    pub throttle_reason: Option<ThrottleReason>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub scanned_directories: u32,
    pub discovered_files: u32,
    pub skipped_symlinks: u32,
    pub skipped_entries: u32,
    pub error_count: u32,
    pub retry_count: u32,
    pub elapsed_ms: u64,
    pub last_error: Option<String>,
    /// Why it was made; `None` for a job made before Slice 4.5, which a
    /// scan finishes by Phase 0's rule.
    pub origin: Option<ScanOrigin>,
    pub new_files: u32,
    pub changed_files: u32,
    pub missing_files: u32,
}

/// Columns for [`job_from_row`]; the job table is aliased `s`, its source
/// root `r`.
pub const JOB_COLUMNS: &str = "s.id, s.source_root_id, r.path_bytes, r.display_path, s.kind, \
     s.state, s.pause_reason, s.throttle_reason, s.started_at, s.finished_at, \
     s.scanned_directories, s.discovered_files, s.skipped_symlinks, s.skipped_entries, \
     s.error_count, s.retry_count, s.elapsed_ms, s.last_error, s.origin, s.new_files, \
     s.changed_files, s.missing_files";

/// Decodes a row selected with [`JOB_COLUMNS`]. An unknown kind is an
/// error, never mistaken for a scan (there is no CHECK constraint on it).
pub fn job_from_row(row: &Row) -> rusqlite::Result<JobRow> {
    let kind: String = row.get(4)?;
    let kind = from_db::<JobKind>(&kind).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            4,
            rusqlite::types::Type::Text,
            format!("unknown job kind {kind:?}").into(),
        )
    })?;
    let state: String = row.get(5)?;
    Ok(JobRow {
        id: row.get(0)?,
        source_root_id: row.get(1)?,
        root_path_bytes: row.get(2)?,
        root_display: row.get(3)?,
        kind,
        state: from_db(&state).unwrap_or(JobState::FailedTerminal),
        pause_reason: opt_from_db(row.get(6)?),
        throttle_reason: opt_from_db(row.get(7)?),
        started_at: row.get(8)?,
        finished_at: row.get(9)?,
        scanned_directories: row.get(10)?,
        discovered_files: row.get(11)?,
        skipped_symlinks: row.get(12)?,
        skipped_entries: row.get(13)?,
        error_count: row.get(14)?,
        retry_count: row.get(15)?,
        elapsed_ms: row.get::<_, i64>(16)?.max(0) as u64,
        last_error: row.get(17)?,
        origin: opt_from_db(row.get(18)?),
        new_files: row.get(19)?,
        changed_files: row.get(20)?,
        missing_files: row.get(21)?,
    })
}

pub fn job(conn: &Connection, id: &str) -> rusqlite::Result<Option<JobRow>> {
    conn.query_row(
        &format!(
            "SELECT {JOB_COLUMNS} FROM jobs s
             JOIN source_roots r ON r.id = s.source_root_id WHERE s.id = ?1"
        ),
        [id],
        job_from_row,
    )
    .optional()
}

/// The next job of `kind` its runner should run: an interrupted running one
/// first, then policy-paused, then queued, by priority and age.
///
/// A scan paused because its folder is unreachable waits outside the
/// runner (Slice 4.5): the folder check queues it again when the folder
/// comes back, so one unplugged drive never holds up the other folders.
pub fn next_runnable(conn: &Connection, kind: JobKind) -> rusqlite::Result<Option<JobRow>> {
    conn.query_row(
        &format!(
            "SELECT {JOB_COLUMNS} FROM jobs s
             JOIN source_roots r ON r.id = s.source_root_id
             WHERE s.kind = ?1 AND s.state IN ('running', 'paused_policy', 'queued')
               AND NOT (s.kind = 'scan' AND s.state = 'paused_policy'
                        AND s.pause_reason = 'source_root_missing')
             ORDER BY s.priority DESC,
                      CASE s.state WHEN 'running' THEN 0 WHEN 'paused_policy' THEN 1 ELSE 2 END,
                      s.created_at, s.rowid
             LIMIT 1"
        ),
        [to_db(&kind)],
        job_from_row,
    )
    .optional()
}

/// The newest non-terminal job of `kind` for a root, if any.
pub fn active_for_root(
    conn: &Connection,
    root_id: &str,
    kind: JobKind,
) -> rusqlite::Result<Option<JobRow>> {
    conn.query_row(
        &format!(
            "SELECT {JOB_COLUMNS} FROM jobs s
             JOIN source_roots r ON r.id = s.source_root_id
             WHERE s.source_root_id = ?1 AND s.kind = ?2 AND s.state IN ({})
             ORDER BY s.rowid DESC LIMIT 1",
            state_list(ACTIVE_STATES)
        ),
        params![root_id, to_db(&kind)],
        job_from_row,
    )
    .optional()
}

pub fn insert_job(
    conn: &Connection,
    id: &str,
    root_id: &str,
    kind: JobKind,
    now: &str,
) -> rusqlite::Result<()> {
    insert_job_from(conn, id, root_id, kind, ScanOrigin::User, now)
}

/// A queued job, recording why it was made.
pub fn insert_job_from(
    conn: &Connection,
    id: &str,
    root_id: &str,
    kind: JobKind,
    origin: ScanOrigin,
    now: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO jobs (id, source_root_id, kind, state, origin, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'queued', ?4, ?5, ?5)",
        params![id, root_id, to_db(&kind), to_db(&origin), now],
    )
    .map(|_| ())
}

/// Moves a job to `to` only if it is currently in one of `from`.
/// Returns whether the transition happened.
pub fn transition(
    conn: &Connection,
    id: &str,
    to: JobState,
    from: &[JobState],
    pause_reason: Option<PauseReason>,
    last_error: Option<&str>,
    now: &str,
) -> rusqlite::Result<bool> {
    let terminal = matches!(
        to,
        JobState::Completed | JobState::Cancelled | JobState::FailedTerminal
    );
    let changed = conn.execute(
        &format!(
            "UPDATE jobs SET
                 state = ?2,
                 pause_reason = ?3,
                 throttle_reason = CASE WHEN ?2 = 'running' THEN throttle_reason ELSE NULL END,
                 last_error = COALESCE(?4, last_error),
                 started_at = CASE WHEN ?2 = 'running' THEN COALESCE(started_at, ?5) ELSE started_at END,
                 finished_at = CASE WHEN ?6 THEN ?5 ELSE finished_at END,
                 updated_at = ?5
             WHERE id = ?1 AND state IN ({})",
            state_list(from)
        ),
        params![
            id,
            to_db(&to),
            pause_reason.map(|r| to_db(&r)),
            last_error,
            now,
            terminal
        ],
    )?;
    Ok(changed > 0)
}

pub fn set_throttle(
    conn: &Connection,
    id: &str,
    throttle: Option<ThrottleReason>,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE jobs SET throttle_reason = ?2
         WHERE id = ?1 AND throttle_reason IS NOT ?2",
        params![id, throttle.map(|t| to_db(&t))],
    )
    .map(|_| ())
}

pub fn bump_retry(conn: &Connection, id: &str) -> rusqlite::Result<u32> {
    conn.execute(
        "UPDATE jobs SET retry_count = retry_count + 1 WHERE id = ?1",
        [id],
    )?;
    conn.query_row("SELECT retry_count FROM jobs WHERE id = ?1", [id], |r| {
        r.get(0)
    })
}

/// On open: interrupted work of every kind becomes queued again; a user's
/// pause is kept (P0-09). The rows are the checkpoint, so nothing else is
/// needed to resume.
pub fn requeue_interrupted(conn: &Connection, now: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE jobs SET state = 'queued', pause_reason = NULL, throttle_reason = NULL,
             updated_at = ?1
         WHERE state IN ('running', 'paused_policy', 'failed_retryable')",
        [now],
    )
}

/// Frontier rows are only useful while a scan can resume. Other kinds have
/// none, so this deletes nothing for them.
pub fn drop_frontier(conn: &Connection, job_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM scan_directories WHERE session_id = ?1",
        [job_id],
    )?;
    // A cancelled scan marks nothing missing (Slice 4.5).
    conn.execute("DELETE FROM scan_missing WHERE session_id = ?1", [job_id])
        .map(|_| ())
}
