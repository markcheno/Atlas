//! SQL for extraction (phase1-spec §6.1, §7.1): the per-root queue, the
//! one-transaction document commit, per-file outcomes, and the status
//! overview. Writes run on the catalog writer; the overview also runs on
//! the reader pool for `get_extraction_status`.
//!
//! Every write that records work is guarded by the job still being
//! `running` (as scans are, ADR 0005), so a pause or cancel takes effect at
//! the next file with no late writes.

use rusqlite::{Connection, OptionalExtension, params};

use super::engine::{Outcome, Piece, Summary, reader_version};
use super::supervisor::PageCounts;
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
    /// Set when the file is read again for a newer reader: its document's
    /// text stays searchable until the new text replaces it.
    pub refresh_document: Option<String>,
}

fn queued_from_row(r: &rusqlite::Row, refresh: bool) -> rusqlite::Result<QueuedFile> {
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
        refresh_document: if refresh { r.get(7)? } else { None },
    })
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
        queued_from_row(r, false)
    })?
    .collect()
}

/// Driven by the few marked documents (`documents_refresh`), each
/// file found through `file_instances_by_document`. `CROSS JOIN`
/// fixes that order, and `+` keeps the planner off the extract-queue
/// index, which made this quadratic in the root's files (45 s at
/// 30,000 on the M5 Pro, Slice 1.7).
const NEXT_REFRESH_SQL: &str =
    "SELECT f.rowid, f.id, f.rel_path, f.file_type, f.storage, f.size_bytes, f.mtime_secs,
                f.document_id
         FROM documents d CROSS JOIN file_instances f
         WHERE d.refresh = 1
           AND f.rowid = (SELECT MIN(g.rowid) FROM file_instances g
                          WHERE g.document_id = d.id AND g.availability = 'available'
                            AND +g.source_root_id = ?1 AND +g.storage = 'local'
                            AND +g.extract_state = 'linked')
           AND f.rowid > ?2
         ORDER BY f.rowid LIMIT ?3";

/// Files of a root whose document is due to be read again
/// (`documents.refresh`), one local, available file per document, after
/// `after_rowid`. They stay linked, so their text stays searchable.
pub fn next_refresh(
    conn: &Connection,
    root_id: &str,
    after_rowid: i64,
    limit: usize,
) -> rusqlite::Result<Vec<QueuedFile>> {
    let mut stmt = conn.prepare_cached(NEXT_REFRESH_SQL)?;
    stmt.query_map(params![root_id, after_rowid, limit as i64], |r| {
        queued_from_row(r, true)
    })?
    .collect()
}

/// A refresh that could not be done (the worker crashed on the file twice):
/// keep the old text and stop trying.
pub fn clear_refresh(conn: &Connection, job_id: &str, document_id: &str) -> rusqlite::Result<bool> {
    if !job_running(conn, job_id)? {
        return Ok(false);
    }
    conn.execute(
        "UPDATE documents SET refresh = 0 WHERE id = ?1",
        [document_id],
    )?;
    Ok(true)
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

/// A document already read by the current reader for its type (and not
/// due to be read again), if any.
pub fn reusable_document(
    conn: &Connection,
    hash: &[u8],
    file_type: FileType,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT id FROM documents
         WHERE content_hash = ?1 AND extractor_version >= ?2 AND refresh = 0
           AND state NOT IN ('pending', 'running')",
        params![hash, reader_version(file_type)],
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
        /// A PDF that needs OCR: what it is known to hold (Slice 4.1).
        pages: Option<PageCounts>,
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
    file_type: FileType,
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
        DocumentResult::NoText { state, reason, .. } => (*state, Some(*reason), None),
    };
    let counts = match &result {
        DocumentResult::NoText { pages, .. } => *pages,
        DocumentResult::Text { .. } => None,
    };
    let page_count = summary
        .map(|s| s.page_count)
        .or(counts.map(|p| p.page_count));
    let text_pages = counts.map(|p| p.text_pages);
    let authors = summary
        .filter(|s| !s.authors.is_empty())
        .map(|s| serde_json::to_string(&s.authors).unwrap_or_default());
    let new_id = uuid::Uuid::new_v4().to_string();
    let document_id: String = tx.query_row(
        "INSERT INTO documents (id, content_hash, state, reason, extractor_version, page_count,
             char_count, title, authors, language, content_title, refresh, boilerplate_chars,
             text_pages, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 0, ?13, ?14, ?12, ?12)
         ON CONFLICT (content_hash) DO UPDATE SET
             state = excluded.state, reason = excluded.reason,
             extractor_version = excluded.extractor_version, page_count = excluded.page_count,
             char_count = excluded.char_count, title = excluded.title,
             authors = excluded.authors, language = excluded.language,
             content_title = excluded.content_title, refresh = 0,
             boilerplate_chars = excluded.boilerplate_chars, text_pages = excluded.text_pages,
             updated_at = excluded.updated_at
         RETURNING id",
        params![
            new_id,
            hash,
            state,
            reason,
            reader_version(file_type),
            page_count,
            summary.map(|s| s.char_count as i64),
            summary.and_then(|s| s.title.as_deref()),
            authors,
            summary.and_then(|s| s.language.as_deref()),
            summary.and_then(|s| s.content_title.as_deref()),
            now,
            summary.map_or(0, |s| s.boilerplate_chars as i64),
            text_pages
        ],
        |r| r.get(0),
    )?;
    // The old passages take their chunks with them (leaving tombstones for
    // the vector index), and the document waits to be embedded again; an
    // unchanged text re-chunks to the same hashes, so nothing is re-sent
    // (ADR 0013).
    tx.execute(
        "DELETE FROM passages WHERE document_id = ?1",
        [&document_id],
    )?;
    tx.execute(
        "DELETE FROM document_embeddings WHERE document_id = ?1",
        [&document_id],
    )?;
    if let DocumentResult::Text { pieces, .. } = result {
        let mut insert = tx.prepare_cached(
            "INSERT INTO passages (document_id, ordinal, page_no, location, text)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for p in pieces {
            // Search marks hits with two private-use characters (ADR 0010);
            // stored text never contains them.
            let text = crate::services::search::snippet::strip_markers(&p.text);
            insert.execute(params![document_id, p.ordinal, p.page_no, p.location, text])?;
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

/// Tidies the full-text index after a pass (phase1-spec §6.1): FTS5
/// `merge` in bounded steps, so the writer is never held for long. Stops
/// when a step changes nothing or after `max_steps`.
pub fn merge_index(conn: &Connection, max_steps: u32) -> rusqlite::Result<u32> {
    let mut steps = 0;
    while steps < max_steps {
        let before = conn.total_changes();
        conn.execute(
            "INSERT INTO passages_fts (passages_fts, rank) VALUES ('merge', 500)",
            [],
        )?;
        steps += 1;
        // The FTS5 documentation: a step that does no work changes at most
        // one row.
        if conn.total_changes() - before < 2 {
            break;
        }
    }
    Ok(steps)
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

/// Whether a root has files waiting, or a marked document with a local
/// copy in it (driven by `documents_refresh`, as in [`NEXT_REFRESH_SQL`]).
const HAS_PENDING_SQL: &str = "SELECT EXISTS (SELECT 1 FROM file_instances
             WHERE source_root_id = ?1 AND availability = 'available'
               AND extract_state = 'pending')
         OR EXISTS (SELECT 1 FROM documents d CROSS JOIN file_instances f
             WHERE d.refresh = 1 AND f.document_id = d.id AND f.availability = 'available'
               AND +f.source_root_id = ?1 AND +f.storage = 'local'
               AND +f.extract_state = 'linked')";

/// Whether a root has files waiting to be read, or read again.
pub fn has_pending(conn: &Connection, root_id: &str) -> rusqlite::Result<bool> {
    conn.query_row(HAS_PENDING_SQL, [root_id], |r| r.get(0))
}

/// A document read by an older reader: id, state, metadata title, file
/// type of one of its files, reader version.
type Stale = (String, String, Option<String>, Option<String>, Option<u32>);

/// Marks documents read by an older reader to be read again, keeping them
/// linked and searchable (phase1-spec §6.1, Slice 1.6). A document whose
/// output would not change is moved to the current version instead:
/// documents without text (the reason stays the same), PDFs whose own
/// metadata already gives a usable title, and EPUBs already at reader
/// version 2 whose text never mentions Project Gutenberg (version 3 only
/// cuts that publisher's header and licence, Slice 3.1). Returns
/// (refreshing, bumped).
pub fn mark_refresh(conn: &Connection) -> rusqlite::Result<(usize, usize)> {
    let stale: Vec<Stale> = conn
        .prepare(
            "SELECT d.id, d.state, d.title,
                    (SELECT f.file_type FROM file_instances f
                     WHERE f.document_id = d.id LIMIT 1),
                    d.extractor_version
             FROM documents d
             WHERE d.refresh = 0 AND d.state NOT IN ('pending', 'running')
               AND COALESCE(d.extractor_version, 0) < MAX(?1, ?2)",
        )?
        .query_map(
            params![
                reader_version(FileType::Pdf),
                reader_version(FileType::Epub)
            ],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )?
        .collect::<rusqlite::Result<_>>()?;
    if stale.is_empty() {
        return Ok((0, 0));
    }
    // Documents whose text mentions Project Gutenberg, from the full-text
    // index: the only EPUBs reader version 3 reads differently.
    let gutenberg: std::collections::HashSet<String> = conn
        .prepare(
            "SELECT DISTINCT p.document_id FROM passages_fts
             JOIN passages p ON p.id = passages_fts.rowid
             WHERE passages_fts MATCH '\"project gutenberg\"'",
        )?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let (mut refreshing, mut bumped) = (0, 0);
    for (id, state, title, file_type, version) in stale {
        // An orphan (no files left) is never read again.
        let Some(file_type) = file_type.and_then(|t| from_db::<FileType>(&t)) else {
            continue;
        };
        let current = reader_version(file_type);
        if version.unwrap_or(0) >= current {
            continue;
        }
        let unchanged = state != "done"
            || (file_type == FileType::Pdf
                && crate::services::documents::has_usable_title(title.as_deref()))
            || (file_type == FileType::Epub
                && version.unwrap_or(0) >= 2
                && !gutenberg.contains(&id));
        if unchanged {
            conn.execute(
                "UPDATE documents SET extractor_version = ?2 WHERE id = ?1",
                params![id, current],
            )?;
            bumped += 1;
        } else {
            conn.execute("UPDATE documents SET refresh = 1 WHERE id = ?1", [&id])?;
            refreshing += 1;
        }
    }
    Ok((refreshing, bumped))
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

/// On open: documents read by an older reader are marked to be read again
/// (staying searchable), and every root with work gets an extract job.
pub fn queue_pending(conn: &Connection, now: &str) -> rusqlite::Result<usize> {
    mark_refresh(conn)?;
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

/// Files of each root whose document is being read again, counted from
/// the marked documents (this runs on every status read).
const UPDATING_SQL: &str = "SELECT f.source_root_id, COUNT(*)
         FROM documents d CROSS JOIN file_instances f
         WHERE d.refresh = 1 AND f.document_id = d.id AND f.availability = 'available'
           AND (?1 IS NULL OR +f.source_root_id = ?1)
         GROUP BY f.source_root_id";

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
    let mut updating = conn.prepare_cached(UPDATING_SQL)?;
    for row in updating.query_map([root], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?))
    })? {
        let (root, n) = row?;
        if let Some((_, counts)) = out.iter_mut().find(|(r, _)| *r == root) {
            counts.updating = n;
        }
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
        counts.updating += c.updating;
    }
    let mut roots_stmt = conn.prepare_cached(&format!(
        "SELECT r.id,
                (SELECT state FROM jobs WHERE source_root_id = r.id AND kind = 'extract'
                   ORDER BY rowid DESC LIMIT 1),
                (SELECT id FROM jobs WHERE source_root_id = r.id AND kind = 'extract'
                   AND state IN ({}) ORDER BY rowid DESC LIMIT 1),
                (SELECT last_error FROM jobs WHERE source_root_id = r.id AND kind = 'extract'
                   AND state = 'failed_terminal' ORDER BY rowid DESC LIMIT 1)
         FROM source_roots r ORDER BY r.display_path, r.id",
        state_list(ACTIVE_STATES)
    ))?;
    let roots = roots_stmt
        .query_map([], |r| {
            let id: String = r.get(0)?;
            let last: Option<String> = r.get(1)?;
            Ok((
                id,
                last,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|(id, last, active_job_id, failed)| RootExtractionSummary {
            counts: per_root
                .iter()
                .find(|(r, _)| *r == id)
                .map(|(_, c)| c.clone())
                .unwrap_or_default(),
            source_root_id: id,
            // Only while that failure is the folder's latest job.
            failed_because: failed.filter(|_| last.as_deref() == Some("failed_terminal")),
            last_state: last.as_deref().and_then(from_db),
            // A job of this folder that is active is its latest one.
            active_state: active_job_id
                .as_ref()
                .and(last.as_deref())
                .and_then(from_db),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The refresh queries start from the marked documents and find their
    /// files by document; the extract-queue index made `next_refresh`
    /// quadratic in a root's files.
    #[test]
    fn refresh_queries_are_driven_by_the_marked_documents() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::services::catalog::migrations::migrate(&mut conn, "t").unwrap();
        let plan = |sql: &str| -> String {
            let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            let unbound = std::iter::repeat_n(rusqlite::types::Null, stmt.parameter_count());
            stmt.query_map(rusqlite::params_from_iter(unbound), |r| {
                r.get::<_, String>(3)
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
            .join("\n")
        };
        for sql in [NEXT_REFRESH_SQL, HAS_PENDING_SQL, UPDATING_SQL] {
            let plan = plan(sql);
            assert!(plan.contains("documents_refresh"), "{sql}\n{plan}");
            assert!(plan.contains("file_instances_by_document"), "{sql}\n{plan}");
        }
        let plan = plan(NEXT_REFRESH_SQL);
        assert!(!plan.contains("extract_queue"), "{plan}");
    }
}
