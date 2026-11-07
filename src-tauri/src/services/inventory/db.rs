//! SQL for scan sessions, the directory frontier, and file instances.
//! Every function runs on the catalog writer thread.
//!
//! State changes are conditional (`… WHERE state IN (…)`) so the worker and
//! user commands can never overwrite each other's decisions, for example a
//! policy pause can never replace a user pause.

use std::path::PathBuf;

use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::domain::dto::{
    InventoryOverview, InventoryStatus, JobState, PauseReason, RootInventorySummary, ThrottleReason,
};
use crate::infrastructure::native_path;

/// Enum ⇄ TEXT using the same snake_case names as the IPC contract.
pub fn to_db<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => s,
        _ => String::new(),
    }
}

pub fn from_db<T: DeserializeOwned>(text: &str) -> Option<T> {
    serde_json::from_value(serde_json::Value::String(text.to_owned())).ok()
}

fn opt_from_db<T: DeserializeOwned>(text: Option<String>) -> Option<T> {
    text.as_deref().and_then(from_db)
}

/// States that still have work to do or could resume.
pub const ACTIVE_STATES: &[JobState] = &[
    JobState::Queued,
    JobState::Running,
    JobState::PausedPolicy,
    JobState::PausedUser,
    JobState::FailedRetryable,
];

fn state_list(states: &[JobState]) -> String {
    states
        .iter()
        .map(|s| format!("'{}'", to_db(s)))
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Debug, Clone)]
pub struct SessionRow {
    pub id: String,
    pub source_root_id: String,
    pub root_path_bytes: Vec<u8>,
    pub root_display: String,
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
}

const SESSION_COLUMNS: &str = "s.id, s.source_root_id, r.path_bytes, r.display_path, s.state, \
     s.pause_reason, s.throttle_reason, s.started_at, s.finished_at, s.scanned_directories, \
     s.discovered_files, s.skipped_symlinks, s.skipped_entries, s.error_count, s.retry_count, \
     s.elapsed_ms, s.last_error";

fn session_from_row(row: &Row) -> rusqlite::Result<SessionRow> {
    let state: String = row.get(4)?;
    Ok(SessionRow {
        id: row.get(0)?,
        source_root_id: row.get(1)?,
        root_path_bytes: row.get(2)?,
        root_display: row.get(3)?,
        state: from_db(&state).unwrap_or(JobState::FailedTerminal),
        pause_reason: opt_from_db(row.get(5)?),
        throttle_reason: opt_from_db(row.get(6)?),
        started_at: row.get(7)?,
        finished_at: row.get(8)?,
        scanned_directories: row.get(9)?,
        discovered_files: row.get(10)?,
        skipped_symlinks: row.get(11)?,
        skipped_entries: row.get(12)?,
        error_count: row.get(13)?,
        retry_count: row.get(14)?,
        elapsed_ms: row.get::<_, i64>(15)?.max(0) as u64,
        last_error: row.get(16)?,
    })
}

pub fn session(conn: &Connection, id: &str) -> rusqlite::Result<Option<SessionRow>> {
    conn.query_row(
        &format!(
            "SELECT {SESSION_COLUMNS} FROM scan_sessions s
             JOIN source_roots r ON r.id = s.source_root_id WHERE s.id = ?1"
        ),
        [id],
        session_from_row,
    )
    .optional()
}

/// The next session the worker should run: an interrupted running one
/// first, then policy-paused, then queued, by priority and age.
pub fn next_runnable(conn: &Connection) -> rusqlite::Result<Option<SessionRow>> {
    conn.query_row(
        &format!(
            "SELECT {SESSION_COLUMNS} FROM scan_sessions s
             JOIN source_roots r ON r.id = s.source_root_id
             WHERE s.state IN ('running', 'paused_policy', 'queued')
             ORDER BY s.priority DESC,
                      CASE s.state WHEN 'running' THEN 0 WHEN 'paused_policy' THEN 1 ELSE 2 END,
                      s.created_at, s.rowid
             LIMIT 1"
        ),
        [],
        session_from_row,
    )
    .optional()
}

/// The newest non-terminal session for a root, if any.
pub fn active_for_root(conn: &Connection, root_id: &str) -> rusqlite::Result<Option<SessionRow>> {
    conn.query_row(
        &format!(
            "SELECT {SESSION_COLUMNS} FROM scan_sessions s
             JOIN source_roots r ON r.id = s.source_root_id
             WHERE s.source_root_id = ?1 AND s.state IN ({})
             ORDER BY s.rowid DESC LIMIT 1",
            state_list(ACTIVE_STATES)
        ),
        [root_id],
        session_from_row,
    )
    .optional()
}

pub fn insert_session(
    conn: &Connection,
    id: &str,
    root_id: &str,
    now: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO scan_sessions (id, source_root_id, state, created_at, updated_at)
         VALUES (?1, ?2, 'queued', ?3, ?3)",
        params![id, root_id, now],
    )
    .map(|_| ())
}

/// Moves a session to `to` only if it is currently in one of `from`.
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
            "UPDATE scan_sessions SET
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
        "UPDATE scan_sessions SET throttle_reason = ?2
         WHERE id = ?1 AND throttle_reason IS NOT ?2",
        params![id, throttle.map(|t| to_db(&t))],
    )
    .map(|_| ())
}

pub fn bump_retry(conn: &Connection, id: &str) -> rusqlite::Result<u32> {
    conn.execute(
        "UPDATE scan_sessions SET retry_count = retry_count + 1 WHERE id = ?1",
        [id],
    )?;
    conn.query_row(
        "SELECT retry_count FROM scan_sessions WHERE id = ?1",
        [id],
        |r| r.get(0),
    )
}

/// On open: interrupted work becomes queued again (a user's pause is kept,
/// P0-09), and folders that have never been scanned, such as ones added
/// before cataloging existed, are queued once.
pub fn recover_on_open(conn: &Connection, now: &str) -> rusqlite::Result<usize> {
    let requeued = conn.execute(
        "UPDATE scan_sessions SET state = 'queued', pause_reason = NULL, throttle_reason = NULL,
             updated_at = ?1
         WHERE state IN ('running', 'paused_policy', 'failed_retryable')",
        [now],
    )?;
    let never_scanned: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT r.id FROM source_roots r
             WHERE r.last_scan_at IS NULL
               AND NOT EXISTS (SELECT 1 FROM scan_sessions s WHERE s.source_root_id = r.id)",
        )?;
        stmt.query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };
    for root_id in &never_scanned {
        insert_session(conn, &uuid::Uuid::new_v4().to_string(), root_id, now)?;
    }
    Ok(requeued + never_scanned.len())
}

pub fn seed_frontier(conn: &Connection, session_id: &str, root_rel: &[u8]) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO scan_directories (session_id, rel_path, state)
         SELECT ?1, ?2, 'pending'
         WHERE NOT EXISTS (SELECT 1 FROM scan_directories WHERE session_id = ?1)",
        params![session_id, root_rel],
    )
    .map(|_| ())
}

#[derive(Debug, Clone)]
pub struct DirRow {
    pub id: i64,
    pub rel_path: Vec<u8>,
    pub attempts: u32,
}

pub fn next_dir(
    conn: &Connection,
    session_id: &str,
    max_attempts: u32,
) -> rusqlite::Result<Option<DirRow>> {
    conn.query_row(
        "SELECT id, rel_path, attempts FROM scan_directories
         WHERE session_id = ?1 AND state = 'pending' AND attempts < ?2
         ORDER BY attempts, id LIMIT 1",
        params![session_id, max_attempts],
        |r| {
            Ok(DirRow {
                id: r.get(0)?,
                rel_path: r.get(1)?,
                attempts: r.get(2)?,
            })
        },
    )
    .optional()
}

/// Counts one failed listing; after `max_attempts` the directory is given
/// up for this session (and nothing under it will be marked missing).
pub fn record_dir_failure(
    conn: &Connection,
    dir_id: i64,
    session_id: &str,
    code: &str,
    max_attempts: u32,
    now: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE scan_directories SET attempts = attempts + 1, last_error = ?2,
             state = CASE WHEN attempts + 1 >= ?3 THEN 'failed' ELSE 'pending' END
         WHERE id = ?1",
        params![dir_id, code, max_attempts],
    )?;
    conn.execute(
        "UPDATE scan_sessions SET error_count = error_count + 1, last_error = ?2, updated_at = ?3
         WHERE id = ?1",
        params![session_id, code, now],
    )
    .map(|_| ())
}

#[derive(Debug, Clone)]
pub struct FileRecord {
    pub rel_path: Vec<u8>,
    pub display_path: String,
    pub display_name: String,
    pub extension: String,
    pub file_type: String,
    pub size_bytes: Option<u64>,
    pub mtime_secs: Option<i64>,
    pub modified_at: Option<String>,
    pub cloud_only: bool,
}

#[derive(Debug, Default)]
pub struct Batch {
    pub dirs: Vec<Vec<u8>>,
    pub files: Vec<FileRecord>,
    pub skipped_symlinks: u32,
    pub skipped_entries: u32,
    pub entry_errors: u32,
    pub elapsed_ms: u64,
    /// Marks the directory complete in the same transaction.
    pub done_dir: Option<i64>,
}

impl Batch {
    pub fn len(&self) -> usize {
        self.dirs.len()
            + self.files.len()
            + (self.skipped_symlinks + self.skipped_entries + self.entry_errors) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0 && self.done_dir.is_none() && self.elapsed_ms == 0
    }
}

/// Writes one batch atomically: new frontier rows, file upserts, counters,
/// and (for the last batch of a directory) its completion.
pub fn commit_batch(
    conn: &mut Connection,
    session_id: &str,
    root_id: &str,
    batch: &Batch,
    now: &str,
) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    {
        let mut add_dir = tx.prepare_cached(
            "INSERT OR IGNORE INTO scan_directories (session_id, rel_path, state)
             VALUES (?1, ?2, 'pending')",
        )?;
        for rel in &batch.dirs {
            add_dir.execute(params![session_id, rel])?;
        }

        let mut upsert = tx.prepare_cached(
            "INSERT INTO file_instances (id, source_root_id, rel_path, display_path, display_name,
                 extension, file_type, size_bytes, mtime_secs, modified_at, availability, storage,
                 last_seen_session_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'available', ?11, ?12, ?13, ?13)
             ON CONFLICT (source_root_id, rel_path) DO UPDATE SET
                 updated_at = CASE
                     WHEN size_bytes IS NOT excluded.size_bytes
                       OR mtime_secs IS NOT excluded.mtime_secs
                       OR availability <> 'available'
                       OR storage <> excluded.storage
                     THEN excluded.updated_at ELSE updated_at END,
                 display_path = excluded.display_path,
                 display_name = excluded.display_name,
                 size_bytes = excluded.size_bytes,
                 mtime_secs = excluded.mtime_secs,
                 modified_at = excluded.modified_at,
                 availability = 'available',
                 storage = excluded.storage,
                 last_seen_session_id = excluded.last_seen_session_id",
        )?;
        for f in &batch.files {
            upsert.execute(params![
                uuid::Uuid::new_v4().to_string(),
                root_id,
                f.rel_path,
                f.display_path,
                f.display_name,
                f.extension,
                f.file_type,
                f.size_bytes.map(|s| s as i64),
                f.mtime_secs,
                f.modified_at,
                if f.cloud_only { "cloud_only" } else { "local" },
                session_id,
                now,
            ])?;
        }

        tx.execute(
            "UPDATE scan_sessions SET
                 skipped_symlinks = skipped_symlinks + ?2,
                 skipped_entries = skipped_entries + ?3,
                 error_count = error_count + ?4,
                 elapsed_ms = elapsed_ms + ?5,
                 updated_at = ?6
             WHERE id = ?1",
            params![
                session_id,
                batch.skipped_symlinks,
                batch.skipped_entries,
                batch.entry_errors,
                batch.elapsed_ms as i64,
                now
            ],
        )?;
        if let Some(dir_id) = batch.done_dir {
            tx.execute(
                "UPDATE scan_directories SET state = 'done' WHERE id = ?1",
                [dir_id],
            )?;
        }
    }
    tx.commit()
}

/// Completes a session whose frontier is empty.
///
/// A file not seen in this session is marked missing only when it lies in
/// a part of the tree that was listed successfully: never under a folder
/// that could not be read (or anything below it, which was not reached),
/// and never when the root itself is unreachable. So a permission error or
/// an offline cloud folder cannot make files look deleted (ADR 0005).
pub fn finish(
    conn: &mut Connection,
    session_id: &str,
    root_id: &str,
    root_reachable: bool,
    now: &str,
) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    if root_reachable {
        let failed: Vec<PathBuf> = {
            let mut stmt = tx.prepare(
                "SELECT rel_path FROM scan_directories WHERE session_id = ?1 AND state = 'failed'",
            )?;
            stmt.query_map([session_id], |r| r.get::<_, Vec<u8>>(0))?
                .filter_map(|bytes| bytes.ok().and_then(|b| native_path::decode(&b)))
                .collect()
        };
        let unseen: Vec<(String, Vec<u8>)> = {
            let mut stmt = tx.prepare(
                "SELECT id, rel_path FROM file_instances
                 WHERE source_root_id = ?1 AND availability = 'available'
                   AND last_seen_session_id IS NOT ?2",
            )?;
            stmt.query_map(params![root_id, session_id], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?
        };
        let mut mark = tx.prepare(
            "UPDATE file_instances SET availability = 'missing', updated_at = ?2 WHERE id = ?1",
        )?;
        for (id, rel) in unseen {
            let protected = match native_path::decode(&rel) {
                Some(rel) => failed.iter().any(|dir| rel.starts_with(dir)),
                // Undecodable: leave it alone rather than guess.
                None => true,
            };
            if !protected {
                mark.execute(params![id, now])?;
            }
        }
    }
    tx.execute(
        "UPDATE source_roots SET last_scan_at = ?2 WHERE id = ?1",
        params![root_id, now],
    )?;
    tx.execute(
        "UPDATE scan_sessions SET
             state = 'completed', pause_reason = NULL, throttle_reason = NULL,
             finished_at = ?2, updated_at = ?2,
             discovered_files = (SELECT COUNT(*) FROM file_instances WHERE last_seen_session_id = ?1),
             scanned_directories = (SELECT COUNT(*) FROM scan_directories
                                    WHERE session_id = ?1 AND state = 'done')
         WHERE id = ?1 AND state = 'running'",
        params![session_id, now],
    )?;
    tx.execute(
        "DELETE FROM scan_directories WHERE session_id = ?1",
        [session_id],
    )?;
    tx.commit()
}

/// Frontier rows are only useful while a session can resume.
pub fn drop_frontier(conn: &Connection, session_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM scan_directories WHERE session_id = ?1",
        [session_id],
    )
    .map(|_| ())
}

fn live_counts(conn: &Connection, s: &SessionRow) -> rusqlite::Result<(u32, u32)> {
    let terminal = matches!(
        s.state,
        JobState::Completed | JobState::Cancelled | JobState::FailedTerminal
    );
    if terminal {
        return Ok((s.discovered_files, s.scanned_directories));
    }
    let discovered = conn.query_row(
        "SELECT COUNT(*) FROM file_instances WHERE last_seen_session_id = ?1",
        [&s.id],
        |r| r.get(0),
    )?;
    let done = conn.query_row(
        "SELECT COUNT(*) FROM scan_directories WHERE session_id = ?1 AND state = 'done'",
        [&s.id],
        |r| r.get(0),
    )?;
    Ok((discovered, done))
}

pub fn status(conn: &Connection, s: &SessionRow) -> rusqlite::Result<InventoryStatus> {
    let (discovered_files, completed_directories) = live_counts(conn, s)?;
    Ok(InventoryStatus {
        job_id: s.id.clone(),
        source_root_id: s.source_root_id.clone(),
        source_root_display_path: s.root_display.clone(),
        state: s.state,
        pause_reason: s.pause_reason,
        throttle_reason: s.throttle_reason,
        discovered_files,
        completed_directories,
        failures: s.error_count,
        skipped_symlinks: s.skipped_symlinks,
        skipped_entries: s.skipped_entries,
        elapsed_ms: s.elapsed_ms,
        started_at: s.started_at.clone(),
        finished_at: s.finished_at.clone(),
        last_error: s.last_error.clone(),
    })
}

pub fn overview(conn: &Connection) -> rusqlite::Result<InventoryOverview> {
    let active = conn
        .query_row(
            &format!(
                "SELECT {SESSION_COLUMNS} FROM scan_sessions s
                 JOIN source_roots r ON r.id = s.source_root_id
                 WHERE s.state IN ({})
                 ORDER BY CASE s.state WHEN 'running' THEN 0 WHEN 'paused_policy' THEN 1
                              WHEN 'paused_user' THEN 2 WHEN 'failed_retryable' THEN 3 ELSE 4 END,
                          s.created_at, s.rowid
                 LIMIT 1",
                state_list(ACTIVE_STATES)
            ),
            [],
            session_from_row,
        )
        .optional()?;
    let active = match active {
        Some(s) => Some(status(conn, &s)?),
        None => None,
    };

    let queued_sessions = conn.query_row(
        "SELECT COUNT(*) FROM scan_sessions WHERE state = 'queued'",
        [],
        |r| r.get(0),
    )?;
    let (cataloged_files, missing_files, cloud_only_files): (u32, u32, u32) = conn.query_row(
        "SELECT COALESCE(SUM(availability = 'available'), 0),
                COALESCE(SUM(availability = 'missing'), 0),
                COALESCE(SUM(availability = 'available' AND storage = 'cloud_only'), 0)
         FROM file_instances",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;

    let mut roots_stmt = conn.prepare_cached(
        "SELECT r.id, r.last_scan_at,
                COALESCE(SUM(f.availability = 'available'), 0),
                COALESCE(SUM(f.availability = 'missing'), 0),
                COALESCE(SUM(f.availability = 'available' AND f.storage = 'cloud_only'), 0),
                (SELECT state FROM scan_sessions WHERE source_root_id = r.id ORDER BY rowid DESC LIMIT 1),
                (SELECT id FROM scan_sessions WHERE source_root_id = r.id AND state IN ({active})
                   ORDER BY rowid DESC LIMIT 1)
         FROM source_roots r LEFT JOIN file_instances f ON f.source_root_id = r.id
         GROUP BY r.id ORDER BY r.display_path, r.id"
            .replace("{active}", &state_list(ACTIVE_STATES))
            .as_str(),
    )?;
    let roots = roots_stmt
        .query_map([], |r| {
            Ok(RootInventorySummary {
                source_root_id: r.get(0)?,
                last_scan_at: r.get(1)?,
                file_count: r.get(2)?,
                missing_count: r.get(3)?,
                cloud_only_count: r.get(4)?,
                last_state: opt_from_db(r.get(5)?),
                active_job_id: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(InventoryOverview {
        active,
        queued_sessions,
        cataloged_files,
        missing_files,
        cloud_only_files,
        roots,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_text_matches_the_ipc_names() {
        assert_eq!(to_db(&JobState::PausedPolicy), "paused_policy");
        assert_eq!(
            from_db::<JobState>("paused_user"),
            Some(JobState::PausedUser)
        );
        assert_eq!(to_db(&PauseReason::BatteryLow), "battery_low");
        assert_eq!(from_db::<JobState>("nonsense"), None);
    }
}
