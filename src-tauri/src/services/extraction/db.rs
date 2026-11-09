//! SQL for extraction (phase1-spec §6.1, §7.1): the per-root queue, the
//! one-transaction document commit, per-file outcomes, and the status
//! overview. Writes run on the catalog writer; the overview also runs on
//! the reader pool for `get_extraction_status`.
//!
//! Every write that records work is guarded by the job still being
//! `running` (as scans are, ADR 0005), so a pause or cancel takes effect at
//! the next file with no late writes.

use rusqlite::{Connection, OptionalExtension, params};

use super::engine::{EXTRACTOR_VERSION, Outcome, Piece, Summary};
use crate::domain::dto::{
    ExtractionCounts, ExtractionOverview, ExtractionState, ExtractionStatus, FileType, JobKind,
    JobState, RootExtractionSummary,
};
use crate::services::catalog::codec::from_db;
use crate::services::jobs::db::{
    ACTIVE_STATES, JOB_COLUMNS, JobRow, active_for_root, insert_job, job_from_row, state_list,
};

/// The effective status of a file instance `f` with its document `d`
/// (phase1-spec §6.1): what blocks the instance, else its document's state.
pub const EFFECTIVE_STATE_SQL: &str = "CASE
    WHEN f.extract_state = 'skipped_cloud_only' THEN 'cloud_only'
    WHEN f.extract_state IN ('unreadable', 'hash_failed') THEN 'unreadable'
    WHEN f.extract_state = 'linked' AND d.state = 'done' THEN 'done'
    WHEN f.extract_state = 'linked' AND d.state = 'needs_ocr' THEN 'needs_ocr'
    WHEN f.extract_state = 'linked' AND d.state = 'encrypted' THEN 'encrypted'
    WHEN f.extract_state = 'linked' AND d.state IN ('malformed', 'unsupported', 'too_large',
         'worker_crashed', 'failed') THEN 'unreadable'
    ELSE 'pending' END";

/// The reason code behind [`EFFECTIVE_STATE_SQL`], if any.
pub const EFFECTIVE_REASON_SQL: &str = "CASE
    WHEN f.extract_state IN ('unreadable', 'hash_failed') THEN f.extract_error
    WHEN f.extract_state = 'linked' AND d.state <> 'done' THEN d.reason
    ELSE NULL END";

/// One file waiting to be read.
#[derive(Debug, Clone)]
pub struct QueuedFile {
    pub rowid: i64,
    pub id: String,
    pub rel_path: Vec<u8>,
    pub file_type: FileType,
    pub cloud_only: bool,
    pub size_bytes: Option<u64>,
    pub mtime_secs: Option<i64>,
}

/// Pending files of a root after `after_rowid`, in catalog order. The job
/// walks the queue with this cursor, so a file left pending (it changed
/// since the scan) is not read again until the next pass.
pub fn next_files(
    conn: &Connection,
    root_id: &str,
    after_rowid: i64,
    limit: usize,
) -> rusqlite::Result<Vec<QueuedFile>> {
    let mut stmt = conn.prepare_cached(
        "SELECT rowid, id, rel_path, file_type, storage, size_bytes, mtime_secs
         FROM file_instances
         WHERE source_root_id = ?1 AND availability = 'available'
           AND extract_state = 'pending' AND rowid > ?2
         ORDER BY rowid LIMIT ?3",
    )?;
    stmt.query_map(params![root_id, after_rowid, limit as i64], |r| {
        let file_type: String = r.get(3)?;
        let storage: String = r.get(4)?;
        Ok(QueuedFile {
            rowid: r.get(0)?,
            id: r.get(1)?,
            rel_path: r.get(2)?,
            file_type: from_db(&file_type).unwrap_or(FileType::Pdf),
            cloud_only: storage == "cloud_only",
            size_bytes: r.get::<_, Option<i64>>(5)?.map(|s| s.max(0) as u64),
            mtime_secs: r.get(6)?,
        })
    })?
    .collect()
}

fn job_running(conn: &Connection, job_id: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM jobs WHERE id = ?1 AND state = 'running')",
        [job_id],
        |r| r.get(0),
    )
}

/// Records why an instance was not hashed (`skipped_cloud_only`,
/// `unreadable`, `hash_failed`). Returns false if the job left `running`.
pub fn set_blocked(
    conn: &Connection,
    job_id: &str,
    file_id: &str,
    state: &str,
    error: Option<&str>,
) -> rusqlite::Result<bool> {
    if !job_running(conn, job_id)? {
        return Ok(false);
    }
    conn.execute(
        "UPDATE file_instances SET extract_state = ?2, extract_error = ?3, document_id = NULL
         WHERE id = ?1 AND extract_state = 'pending'",
        params![file_id, state, error],
    )?;
    Ok(true)
}

/// A document already read at the current extractor version, if any.
pub fn reusable_document(conn: &Connection, hash: &[u8]) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT id FROM documents
         WHERE content_hash = ?1 AND extractor_version = ?2
           AND state NOT IN ('pending', 'running')",
        params![hash, EXTRACTOR_VERSION],
        |r| r.get(0),
    )
    .optional()
}

/// Links an instance to an existing document (copies, moves, rescans).
pub fn link(
    conn: &Connection,
    job_id: &str,
    file_id: &str,
    document_id: &str,
    hashed_size: u64,
    hashed_mtime: Option<i64>,
) -> rusqlite::Result<bool> {
    if !job_running(conn, job_id)? {
        return Ok(false);
    }
    conn.execute(
        "UPDATE file_instances SET extract_state = 'linked', document_id = ?2,
             hashed_size = ?3, hashed_mtime_secs = ?4, extract_error = NULL
         WHERE id = ?1",
        params![file_id, document_id, hashed_size as i64, hashed_mtime],
    )?;
    Ok(true)
}

/// What to store for a document.
pub enum DocumentResult<'a> {
    Text {
        summary: &'a Summary,
        pieces: &'a [Piece],
    },
    NoText {
        state: &'a str,
        reason: &'a str,
    },
}

pub fn outcome_state(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Encrypted => "encrypted",
        Outcome::Malformed => "malformed",
        Outcome::Unsupported => "unsupported",
        Outcome::TooLarge => "too_large",
        Outcome::NeedsOcr => "needs_ocr",
    }
}

/// Stores a document, its passages and the instance link in **one**
/// transaction (phase1-spec §6.1, P1-12): a kill leaves all or nothing.
/// Returns false, writing nothing, if the job left `running`.
#[allow(clippy::too_many_arguments)]
pub fn commit_document(
    conn: &mut Connection,
    job_id: &str,
    file_id: &str,
    hash: &[u8],
    hashed_size: u64,
    hashed_mtime: Option<i64>,
    result: DocumentResult<'_>,
    now: &str,
) -> rusqlite::Result<bool> {
    let tx = conn.transaction()?;
    if !job_running(&tx, job_id)? {
        return Ok(false);
    }
    let (state, reason, summary) = match &result {
        DocumentResult::Text { summary, .. } => (
            "done",
            (summary.truncated > 0).then_some("passage_truncated"),
            Some(*summary),
        ),
        DocumentResult::NoText { state, reason } => (*state, Some(*reason), None),
    };
    let authors = summary
        .filter(|s| !s.authors.is_empty())
        .map(|s| serde_json::to_string(&s.authors).unwrap_or_default());
    let new_id = uuid::Uuid::new_v4().to_string();
    let document_id: String = tx.query_row(
        "INSERT INTO documents (id, content_hash, state, reason, extractor_version, page_count,
             char_count, title, authors, language, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)
         ON CONFLICT (content_hash) DO UPDATE SET
             state = excluded.state, reason = excluded.reason,
             extractor_version = excluded.extractor_version, page_count = excluded.page_count,
             char_count = excluded.char_count, title = excluded.title,
             authors = excluded.authors, language = excluded.language,
             updated_at = excluded.updated_at
         RETURNING id",
        params![
            new_id,
            hash,
            state,
            reason,
            EXTRACTOR_VERSION,
            summary.map(|s| s.page_count),
            summary.map(|s| s.char_count as i64),
            summary.and_then(|s| s.title.as_deref()),
            authors,
            summary.and_then(|s| s.language.as_deref()),
            now
        ],
        |r| r.get(0),
    )?;
    tx.execute(
        "DELETE FROM passages WHERE document_id = ?1",
        [&document_id],
    )?;
    if let DocumentResult::Text { pieces, .. } = result {
        let mut insert = tx.prepare_cached(
            "INSERT INTO passages (document_id, ordinal, page_no, location, text)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for p in pieces {
            insert.execute(params![
                document_id,
                p.ordinal,
                p.page_no,
                p.location,
                p.text
            ])?;
        }
    }
    tx.execute(
        "UPDATE file_instances SET extract_state = 'linked', document_id = ?2,
             hashed_size = ?3, hashed_mtime_secs = ?4, extract_error = NULL
         WHERE id = ?1",
        params![file_id, document_id, hashed_size as i64, hashed_mtime],
    )?;
    tx.commit()?;
    Ok(true)
}

/// Counts a worker crash on this file; returns the attempts so far.
pub fn record_crash(conn: &Connection, file_id: &str) -> rusqlite::Result<u32> {
    conn.execute(
        "UPDATE file_instances SET extract_attempts = extract_attempts + 1 WHERE id = ?1",
        [file_id],
    )?;
    conn.query_row(
        "SELECT extract_attempts FROM file_instances WHERE id = ?1",
        [file_id],
        |r| r.get(0),
    )
}

/// Whether a root has files waiting to be read.
pub fn has_pending(conn: &Connection, root_id: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM file_instances
             WHERE source_root_id = ?1 AND availability = 'available'
               AND extract_state = 'pending')",
        [root_id],
        |r| r.get(0),
    )
}

/// Schedules an extract job for a root with pending files, unless one is
/// already scheduled. Returns whether one was added.
pub fn enqueue_if_pending(conn: &Connection, root_id: &str, now: &str) -> rusqlite::Result<bool> {
    if !has_pending(conn, root_id)? || active_for_root(conn, root_id, JobKind::Extract)?.is_some() {
        return Ok(false);
    }
    insert_job(
        conn,
        &uuid::Uuid::new_v4().to_string(),
        root_id,
        JobKind::Extract,
        now,
    )?;
    Ok(true)
}

/// On open: documents read by an older extractor are read again, and every
/// root with pending files gets an extract job.
pub fn queue_pending(conn: &Connection, now: &str) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE file_instances SET extract_state = 'pending', document_id = NULL
         WHERE document_id IN (SELECT id FROM documents WHERE extractor_version IS NOT ?1)",
        [EXTRACTOR_VERSION],
    )?;
    let roots: Vec<String> = conn
        .prepare("SELECT id FROM source_roots")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut added = 0;
    for root in roots {
        if enqueue_if_pending(conn, &root, now)? {
            added += 1;
        }
    }
    Ok(added)
}

fn add(counts: &mut ExtractionCounts, state: ExtractionState, n: u32) {
    match state {
        ExtractionState::Done => counts.done += n,
        ExtractionState::Pending | ExtractionState::Reading => counts.pending += n,
        ExtractionState::NeedsOcr => counts.needs_ocr += n,
        ExtractionState::Encrypted => counts.encrypted += n,
        ExtractionState::CloudOnly => counts.cloud_only += n,
        ExtractionState::Unreadable => counts.unreadable += n,
    }
    counts.total += n;
}

fn status(
    conn: &Connection,
    job: &JobRow,
    current: Option<&str>,
) -> rusqlite::Result<ExtractionStatus> {
    let counts = root_counts(conn, Some(&job.source_root_id))?
        .into_iter()
        .next()
        .map(|(_, c)| c)
        .unwrap_or_default();
    Ok(ExtractionStatus {
        job_id: job.id.clone(),
        source_root_id: job.source_root_id.clone(),
        source_root_display_path: job.root_display.clone(),
        state: job.state,
        pause_reason: job.pause_reason,
        throttle_reason: job.throttle_reason,
        processed_files: counts.total - counts.pending,
        pending_files: counts.pending,
        current_file_id: current.map(str::to_owned),
        last_error: job.last_error.clone(),
    })
}

/// Counts by effective status, per root (all roots when `root` is `None`).
fn root_counts(
    conn: &Connection,
    root: Option<&str>,
) -> rusqlite::Result<Vec<(String, ExtractionCounts)>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT f.source_root_id, {EFFECTIVE_STATE_SQL} AS effective, COUNT(*)
         FROM file_instances f LEFT JOIN documents d ON d.id = f.document_id
         WHERE f.availability = 'available' AND (?1 IS NULL OR f.source_root_id = ?1)
         GROUP BY f.source_root_id, effective ORDER BY f.source_root_id"
    ))?;
    let rows = stmt.query_map([root], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, u32>(2)?,
        ))
    })?;
    let mut out: Vec<(String, ExtractionCounts)> = Vec::new();
    for row in rows {
        let (root, state, n) = row?;
        if out.last().is_none_or(|(r, _)| *r != root) {
            out.push((root.clone(), ExtractionCounts::default()));
        }
        let state = from_db(&state).unwrap_or(ExtractionState::Pending);
        add(&mut out.last_mut().expect("pushed").1, state, n);
    }
    Ok(out)
}

/// The extraction overview for the status bar and folder lines. `current`
/// is the file the active job is reading, when the caller knows it.
pub fn overview(conn: &Connection, current: Option<&str>) -> rusqlite::Result<ExtractionOverview> {
    let active = conn
        .query_row(
            &format!(
                "SELECT {JOB_COLUMNS} FROM jobs s
                 JOIN source_roots r ON r.id = s.source_root_id
                 WHERE s.kind = 'extract' AND s.state IN ({})
                 ORDER BY CASE s.state WHEN 'running' THEN 0 WHEN 'paused_policy' THEN 1
                              WHEN 'paused_user' THEN 2 WHEN 'failed_retryable' THEN 3 ELSE 4 END,
                          s.created_at, s.rowid
                 LIMIT 1",
                state_list(ACTIVE_STATES)
            ),
            [],
            job_from_row,
        )
        .optional()?;
    let active = match active {
        Some(job) => {
            let current = (job.state == JobState::Running)
                .then_some(current)
                .flatten();
            Some(status(conn, &job, current)?)
        }
        None => None,
    };
    let queued_jobs = conn.query_row(
        "SELECT COUNT(*) FROM jobs WHERE kind = 'extract' AND state = 'queued'",
        [],
        |r| r.get(0),
    )?;
    let scanning = conn.query_row(
        &format!(
            "SELECT EXISTS (SELECT 1 FROM jobs WHERE kind = 'scan' AND state IN ({}))",
            state_list(ACTIVE_STATES)
        ),
        [],
        |r| r.get(0),
    )?;
    let per_root = root_counts(conn, None)?;
    let mut counts = ExtractionCounts::default();
    for (_, c) in &per_root {
        counts.done += c.done;
        counts.pending += c.pending;
        counts.needs_ocr += c.needs_ocr;
        counts.encrypted += c.encrypted;
        counts.cloud_only += c.cloud_only;
        counts.unreadable += c.unreadable;
        counts.total += c.total;
    }
    let mut roots_stmt = conn.prepare_cached(&format!(
        "SELECT r.id,
                (SELECT state FROM jobs WHERE source_root_id = r.id AND kind = 'extract'
                   ORDER BY rowid DESC LIMIT 1),
                (SELECT id FROM jobs WHERE source_root_id = r.id AND kind = 'extract'
                   AND state IN ({}) ORDER BY rowid DESC LIMIT 1)
         FROM source_roots r ORDER BY r.display_path, r.id",
        state_list(ACTIVE_STATES)
    ))?;
    let roots = roots_stmt
        .query_map([], |r| {
            let id: String = r.get(0)?;
            let last: Option<String> = r.get(1)?;
            Ok((id, last, r.get::<_, Option<String>>(2)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|(id, last, active_job_id)| RootExtractionSummary {
            counts: per_root
                .iter()
                .find(|(r, _)| *r == id)
                .map(|(_, c)| c.clone())
                .unwrap_or_default(),
            source_root_id: id,
            last_state: last.as_deref().and_then(from_db),
            active_job_id,
        })
        .collect();
    Ok(ExtractionOverview {
        active,
        queued_jobs,
        counts,
        roots,
        scanning,
    })
}
