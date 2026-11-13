//! SQL for scans: the directory frontier, file instances, and the overview.
//! Every function runs on the catalog writer thread, except reads that
//! callers pass to the reader pool. Job rows of every kind, and their state
//! changes, are in [`crate::services::jobs::db`].

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use crate::domain::dto::{
    InventoryOverview, InventoryStatus, JobKind, JobState, RootInventorySummary, ScanOrigin,
    ScanProblem, ScanRecord,
};
use crate::infrastructure::native_path;
use crate::services::catalog::codec::opt_from_db;
use crate::services::jobs::db::{
    ACTIVE_STATES, JOB_COLUMNS, JobRow, insert_job_from, job_from_row, state_list,
};

/// On open, folders that have never been scanned, such as ones added before
/// cataloging existed, are queued once. Returns how many were queued.
pub fn queue_never_scanned(conn: &Connection, now: &str) -> rusqlite::Result<usize> {
    let never_scanned: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT r.id FROM source_roots r
             WHERE r.last_scan_at IS NULL
               AND NOT EXISTS (SELECT 1 FROM jobs s WHERE s.source_root_id = r.id AND s.kind = 'scan')",
        )?;
        stmt.query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };
    for root_id in &never_scanned {
        insert_job_from(
            conn,
            &uuid::Uuid::new_v4().to_string(),
            root_id,
            JobKind::Scan,
            ScanOrigin::Launch,
            now,
        )?;
    }
    Ok(never_scanned.len())
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
    listed: &[i64],
) -> rusqlite::Result<Option<DirRow>> {
    // Folders listed whose completion is held (Slice 4.5) are skipped.
    let skip = if listed.is_empty() {
        String::new()
    } else {
        let ids: Vec<String> = listed.iter().map(i64::to_string).collect();
        format!(" AND id NOT IN ({})", ids.join(","))
    };
    conn.query_row(
        &format!(
            "SELECT id, rel_path, attempts FROM scan_directories
             WHERE session_id = ?1 AND state = 'pending' AND attempts < ?2{skip}
             ORDER BY attempts, id LIMIT 1"
        ),
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
        "UPDATE jobs SET error_count = error_count + 1, last_error = ?2, updated_at = ?3
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
    /// Folded root-relative path and file name (`library::fold`).
    pub search_key: String,
    pub name_key: String,
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
    /// With `done_dir`: the directory, every name its listing held, and how
    /// many catalogable files it had. Rows under it whose name wasn't listed
    /// become missing candidates; the files are added to the scan's count
    /// (Slice 4.5).
    pub listing: Option<Listing>,
}

#[derive(Debug, Default)]
pub struct Listing {
    pub dir_rel: Vec<u8>,
    pub names: HashSet<OsString>,
    pub files: u32,
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

/// Whether a batch was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Committed {
    /// Written. `replayed` counts entries that changed nothing: already in
    /// the catalog exactly as listed (Slice 4.5), or, for a scan begun
    /// before 4.5, already written by this session (a folder listed again
    /// after a pause). The scan charges them less (P1-20, ADR 0005).
    Yes { replayed: usize },
    /// No new files: not written, for the caller to hold (Slice 4.5),
    /// with the batch's files to write (by index) and its missing
    /// candidates.
    Deferred {
        replayed: usize,
        write: Vec<usize>,
        missing: Vec<String>,
    },
    /// The session left `running` (paused, cancelled, or removed) while the
    /// batch was being listed. Nothing was written; the directory stays
    /// pending and is listed again on resume.
    NotRunning,
}

/// What the catalog holds for a listed file: size, mtime, availability,
/// storage and extract state.
type Stored = (Option<i64>, Option<i64>, String, String, String);

/// Work a rescan holds back from the catalog (Slice 4.5): folders listed
/// with nothing new in them, so only the frontier, the counters, changed
/// files and missing candidates to write. Held work goes in with the next
/// commit that adds a file, or when enough is held, so a rescan of a
/// little-changed library commits a few times instead of once a batch. If
/// Atlas stops first it is lost, which costs only listing those folders
/// again: they are still pending in the catalog.
#[derive(Debug, Default)]
pub struct Deferred {
    /// Files listed that differ from their rows, to write.
    pub files: Vec<FileRecord>,
    /// Rows found missing under the held folders.
    pub missing: Vec<String>,
    /// Subfolders found, to add to the frontier.
    pub dirs: Vec<Vec<u8>>,
    /// Folders listed in full, to mark done.
    pub done_dirs: Vec<i64>,
    pub skipped_symlinks: u32,
    pub skipped_entries: u32,
    pub entry_errors: u32,
    pub elapsed_ms: u64,
    /// Catalogable files the held folders listed.
    pub discovered: u32,
}

impl Deferred {
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
            && self.missing.is_empty()
            && self.dirs.is_empty()
            && self.done_dirs.is_empty()
            && self.skipped_symlinks + self.skipped_entries + self.entry_errors == 0
            && self.elapsed_ms == 0
            && self.discovered == 0
    }

    /// Takes on a batch [`commit_batch`] returned as
    /// [`Committed::Deferred`]: the files it named to write, the missing
    /// candidates it found, and the rest of the batch.
    pub fn absorb(&mut self, mut batch: Batch, write: &[usize], missing: Vec<String>) {
        for &i in write.iter().rev() {
            self.files.push(batch.files.swap_remove(i));
        }
        self.missing.extend(missing);
        self.dirs.extend(batch.dirs);
        self.done_dirs.extend(batch.done_dir);
        self.skipped_symlinks += batch.skipped_symlinks;
        self.skipped_entries += batch.skipped_entries;
        self.entry_errors += batch.entry_errors;
        self.elapsed_ms += batch.elapsed_ms;
        self.discovered += batch.listing.map_or(0, |l| l.files);
    }
}

/// Writes one batch atomically, with any held work: new frontier rows, file
/// upserts, counters, and (for the last batch of a directory) its
/// completion.
///
/// With `may_defer` (a scan begun in Slice 4.5 or later), a batch with no
/// new files writes nothing and returns [`Committed::Deferred`]: the
/// caller holds it in its [`Deferred`]. New files are written at once, so a
/// first scan shows its rows as it goes.
///
/// The batch is written only while the session is still `running`, so a
/// pause or cancel takes effect at the batch boundary with no late writes.
/// State changes and commits share the single writer connection, so the
/// check and the writes cannot interleave with a transition.
#[allow(clippy::too_many_arguments)]
pub fn commit_batch(
    conn: &mut Connection,
    session_id: &str,
    root_id: &str,
    batch: &Batch,
    held: &Deferred,
    legacy: bool,
    may_defer: bool,
    now: &str,
) -> rusqlite::Result<Committed> {
    let tx = conn.transaction()?;
    let running: bool = tx
        .query_row(
            "SELECT state = 'running' FROM jobs WHERE id = ?1",
            [session_id],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(false);
    if !running {
        return Ok(Committed::NotRunning);
    }
    let mut replayed = 0;
    let (mut new_files, mut changed_files) = (0u32, 0u32);
    let mut writes: Vec<&FileRecord> = Vec::with_capacity(batch.files.len());
    // Which of the batch's own files are to be written.
    let mut write = Vec::new();
    if legacy {
        writes.extend(held.files.iter().chain(&batch.files));
    } else {
        // Slice 4.5: a row that already says what was listed is left
        // alone, so a rescan of an unchanged folder writes nothing.
        let mut lookup = tx.prepare_cached(
            "SELECT size_bytes, mtime_secs, availability, storage, extract_state
             FROM file_instances WHERE source_root_id = ?1 AND rel_path = ?2",
        )?;
        let held_files = held.files.iter().map(|f| (None, f));
        let batch_files = batch.files.iter().enumerate().map(|(i, f)| (Some(i), f));
        for (index, f) in held_files.chain(batch_files) {
            let storage = if f.cloud_only { "cloud_only" } else { "local" };
            let row: Option<Stored> = lookup
                .query_row(params![root_id, f.rel_path], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                })
                .optional()?;
            match &row {
                Some((size, mtime, availability, stored, extract))
                    if *size == f.size_bytes.map(|s| s as i64)
                        && *mtime == f.mtime_secs
                        && availability == "available"
                        && stored == storage
                        // Back from the cloud: still to be read (P3-01).
                        && !(extract == "skipped_cloud_only" && storage == "local") =>
                {
                    // A held file written meanwhile isn't a replay.
                    if index.is_some() {
                        replayed += 1;
                    }
                    continue;
                }
                Some(_) => changed_files += 1,
                None => new_files += 1,
            }
            write.extend(index);
            writes.push(f);
        }
    }
    let missing = match (&batch.listing, legacy) {
        (Some(listing), false) => missing_under(&tx, root_id, listing)?,
        _ => Vec::new(),
    };
    if may_defer && !legacy && new_files == 0 {
        // The transaction only read; the caller holds the rest.
        return Ok(Committed::Deferred {
            replayed,
            write,
            missing,
        });
    }

    tx.execute(
        "UPDATE jobs SET
             skipped_symlinks = skipped_symlinks + ?2,
             skipped_entries = skipped_entries + ?3,
             error_count = error_count + ?4,
             elapsed_ms = elapsed_ms + ?5,
             updated_at = ?6
         WHERE id = ?1",
        params![
            session_id,
            held.skipped_symlinks + batch.skipped_symlinks,
            held.skipped_entries + batch.skipped_entries,
            held.entry_errors + batch.entry_errors,
            (held.elapsed_ms + batch.elapsed_ms) as i64,
            now
        ],
    )?;
    {
        let mut add_dir = tx.prepare_cached(
            "INSERT OR IGNORE INTO scan_directories (session_id, rel_path, state)
             VALUES (?1, ?2, 'pending')",
        )?;
        for rel in &held.dirs {
            add_dir.execute(params![session_id, rel])?;
        }
        for rel in &batch.dirs {
            if add_dir.execute(params![session_id, rel])? == 0 {
                replayed += 1;
            }
        }

        let mut upsert = tx.prepare_cached(
            "INSERT INTO file_instances (id, source_root_id, rel_path, display_path, display_name,
                 extension, file_type, size_bytes, mtime_secs, modified_at, availability, storage,
                 last_seen_session_id, created_at, updated_at, search_key, name_key)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'available', ?11, ?12, ?13, ?13, ?14, ?15)
             ON CONFLICT (source_root_id, rel_path) DO UPDATE SET
                 -- Changed content needs reading again (phase1-spec §6.1).
                 -- SET expressions see the old row, so order doesn't matter.
                 -- A file skipped while only in the cloud and now stored
                 -- here is read: a download keeps its size and time
                 -- (Slice 3.1).
                 extract_state = CASE
                     WHEN size_bytes IS NOT excluded.size_bytes
                       OR mtime_secs IS NOT excluded.mtime_secs
                     THEN 'pending'
                     WHEN extract_state = 'skipped_cloud_only' AND excluded.storage = 'local'
                     THEN 'pending' ELSE extract_state END,
                 document_id = CASE
                     WHEN size_bytes IS NOT excluded.size_bytes
                       OR mtime_secs IS NOT excluded.mtime_secs
                     THEN NULL ELSE document_id END,
                 extract_attempts = CASE
                     WHEN size_bytes IS NOT excluded.size_bytes
                       OR mtime_secs IS NOT excluded.mtime_secs
                       OR (extract_state = 'skipped_cloud_only' AND excluded.storage = 'local')
                     THEN 0 ELSE extract_attempts END,
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
                 last_seen_session_id = excluded.last_seen_session_id,
                 search_key = excluded.search_key,
                 name_key = excluded.name_key
             -- A row this session already wrote, unchanged, is left alone,
             -- so the statement reports no change and the entry counts as
             -- replayed.
             WHERE last_seen_session_id IS NOT excluded.last_seen_session_id
                OR size_bytes IS NOT excluded.size_bytes
                OR mtime_secs IS NOT excluded.mtime_secs
                OR availability <> 'available'
                OR storage <> excluded.storage",
        )?;
        for f in writes {
            let storage = if f.cloud_only { "cloud_only" } else { "local" };
            let changed = upsert.execute(params![
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
                storage,
                session_id,
                now,
                f.search_key,
                f.name_key,
            ])?;
            if changed == 0 {
                replayed += 1;
            }
        }
        if !legacy {
            let listed = batch.listing.as_ref().map_or(0, |l| l.files);
            tx.execute(
                "UPDATE jobs SET new_files = new_files + ?2, changed_files = changed_files + ?3,
                     discovered_files = discovered_files + ?4
                 WHERE id = ?1",
                params![
                    session_id,
                    new_files,
                    changed_files,
                    held.discovered + listed
                ],
            )?;
            let mut note = tx.prepare_cached(
                "INSERT OR IGNORE INTO scan_missing (session_id, file_id) VALUES (?1, ?2)",
            )?;
            for id in held.missing.iter().chain(&missing) {
                note.execute(params![session_id, id])?;
            }
        }

        let mut done =
            tx.prepare_cached("UPDATE scan_directories SET state = 'done' WHERE id = ?1")?;
        for dir_id in held.done_dirs.iter().chain(batch.done_dir.as_ref()) {
            done.execute([dir_id])?;
        }
    }
    tx.commit()?;
    Ok(Committed::Yes { replayed })
}

/// The stored prefix every row under `dir` starts with, and the first key
/// past them, for a range scan of the `(source_root_id, rel_path)` index.
fn child_range(dir: &Path) -> (Vec<u8>, Option<Vec<u8>>) {
    // The encoding of `dir/x` less that of `x`: the tag, `dir` and its
    // separator (only the tag for the root itself).
    let probe = native_path::encode(&dir.join("x"));
    let x = native_path::encode(Path::new("x"));
    let start = probe[..probe.len() - (x.len() - 1)].to_vec();
    let mut end = start.clone();
    while let Some(last) = end.pop() {
        if last < u8::MAX {
            end.push(last + 1);
            return (start, Some(end));
        }
    }
    (start, None)
}

/// Rows under a directory just listed whose first name below it wasn't in
/// the listing: deleted, moved away, or under a folder that is gone. They
/// become missing when the scan completes (Slice 4.5).
fn missing_under(
    tx: &Connection,
    root_id: &str,
    listing: &Listing,
) -> rusqlite::Result<Vec<String>> {
    let Some(dir) = native_path::decode(&listing.dir_rel) else {
        return Ok(Vec::new());
    };
    let (start, end) = child_range(&dir);
    let rows: Vec<(String, Vec<u8>)> = {
        let mut stmt = tx.prepare_cached(
            "SELECT id, rel_path FROM file_instances
             WHERE source_root_id = ?1 AND availability = 'available'
               AND rel_path >= ?2 AND (?3 IS NULL OR rel_path < ?3)",
        )?;
        stmt.query_map(params![root_id, start, end], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?
    };
    let mut missing = Vec::new();
    for (id, rel) in rows {
        // Undecodable: leave it alone rather than guess.
        let Some(rel) = native_path::decode(&rel) else {
            continue;
        };
        let Ok(below) = rel.strip_prefix(&dir) else {
            continue;
        };
        let Some(first) = below.components().next() else {
            continue;
        };
        if !listing.names.contains(first.as_os_str()) {
            missing.push(id);
        }
    }
    Ok(missing)
}

/// Problems kept a scan, and how many of a folder's scans keep theirs.
pub const PROBLEMS_PER_SCAN: u32 = 200;
pub const SCANS_WITH_PROBLEMS: u32 = 10;

/// Completes a session whose frontier is empty.
///
/// A file not seen in this session is marked missing only when it lies in
/// a part of the tree that was listed successfully: never under a folder
/// that could not be read (or anything below it, which was not reached),
/// and never when the root itself is unreachable. So a permission error or
/// an offline cloud folder cannot make files look deleted (ADR 0005).
///
/// From Slice 4.5 the files not seen are the session's missing candidates,
/// noted folder by folder as each was listed; a scan begun before 4.5
/// (`legacy`) finds them by the marker on the rows it wrote, as Phase 0
/// did. The folders it couldn't read are kept as the scan's problems.
#[allow(clippy::too_many_arguments)]
pub fn finish(
    conn: &mut Connection,
    session_id: &str,
    root_id: &str,
    root_display: &str,
    root_reachable: bool,
    legacy: bool,
    allow_remote: bool,
    now: &str,
) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    let mut missing = 0usize;
    if root_reachable && legacy {
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
                missing += mark.execute(params![id, now])?;
            }
        }
    } else if root_reachable {
        missing = tx.execute(
            "UPDATE file_instances SET availability = 'missing', updated_at = ?2
             WHERE availability = 'available'
               AND id IN (SELECT file_id FROM scan_missing WHERE session_id = ?1)",
            params![session_id, now],
        )?;
    }
    record_problems(&tx, session_id, root_id, root_display)?;
    tx.execute(
        "UPDATE source_roots SET last_scan_at = ?2 WHERE id = ?1",
        params![root_id, now],
    )?;
    let discovered = if legacy {
        "(SELECT COUNT(*) FROM file_instances WHERE last_seen_session_id = ?1)"
    } else {
        "discovered_files"
    };
    tx.execute(
        &format!(
            "UPDATE jobs SET
                 state = 'completed', pause_reason = NULL, throttle_reason = NULL,
                 finished_at = ?2, updated_at = ?2,
                 discovered_files = {discovered},
                 missing_files = ?3,
                 scanned_directories = (SELECT COUNT(*) FROM scan_directories
                                        WHERE session_id = ?1 AND state = 'done')
             WHERE id = ?1 AND state = 'running'"
        ),
        params![session_id, now, missing as i64],
    )?;
    tx.execute(
        "DELETE FROM scan_directories WHERE session_id = ?1",
        [session_id],
    )?;
    tx.execute(
        "DELETE FROM scan_missing WHERE session_id = ?1",
        [session_id],
    )?;
    // New or changed files are read next (phase1-spec §7.1).
    crate::services::extraction::db::enqueue_if_pending(&tx, root_id, now)?;
    // Books in two formats (Slice 4.5.1), in the same transaction, so the
    // scan is never seen finished with its pairs out of date: files gone or
    // moved end their pairs, and a copy kept out is shown again and queued
    // for its meaning. New files are paired once read.
    crate::services::formats::db::reconcile_root(&tx, root_id, now)?;
    let changes = crate::services::formats::db::apply(&tx, now)?;
    if !changes.unhidden.is_empty() {
        crate::services::embedding::triggers::queue_root(&tx, root_id, allow_remote, now)?;
    }
    tx.commit()
}

/// Keeps the folders this scan couldn't read, with why, and drops the
/// problems of the folder's older scans past [`SCANS_WITH_PROBLEMS`].
fn record_problems(
    tx: &Connection,
    session_id: &str,
    root_id: &str,
    root_display: &str,
) -> rusqlite::Result<()> {
    let failed: Vec<(Vec<u8>, String)> = {
        let mut stmt = tx.prepare(
            "SELECT rel_path, COALESCE(last_error, 'read_failed') FROM scan_directories
             WHERE session_id = ?1 AND state = 'failed' ORDER BY id LIMIT ?2",
        )?;
        stmt.query_map(params![session_id, PROBLEMS_PER_SCAN], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?
        .collect::<rusqlite::Result<_>>()?
    };
    let mut add = tx.prepare_cached(
        "INSERT INTO scan_problems (job_id, rel_path, display_path, code) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (rel, code) in failed {
        let display = match native_path::decode(&rel) {
            Some(p) if p.as_os_str().is_empty() => root_display.to_owned(),
            Some(p) => native_path::display(&Path::new(root_display).join(p)),
            None => format!("{root_display} (a folder whose name can't be shown)"),
        };
        add.execute(params![session_id, rel, display, code])?;
    }
    tx.execute(
        "DELETE FROM scan_problems WHERE job_id IN (
             SELECT id FROM jobs WHERE source_root_id = ?1 AND kind = 'scan'
             ORDER BY rowid DESC LIMIT -1 OFFSET ?2)",
        params![root_id, SCANS_WITH_PROBLEMS],
    )
    .map(|_| ())
}

fn live_counts(conn: &Connection, s: &JobRow) -> rusqlite::Result<(u32, u32)> {
    let terminal = matches!(
        s.state,
        JobState::Completed | JobState::Cancelled | JobState::FailedTerminal
    );
    if terminal {
        return Ok((s.discovered_files, s.scanned_directories));
    }
    // From Slice 4.5 a scan counts what it found as it goes; one begun
    // before counts the rows it marked.
    let discovered = if s.origin.is_some() {
        s.discovered_files
    } else {
        conn.query_row(
            "SELECT COUNT(*) FROM file_instances WHERE last_seen_session_id = ?1",
            [&s.id],
            |r| r.get(0),
        )?
    };
    let done = conn.query_row(
        "SELECT COUNT(*) FROM scan_directories WHERE session_id = ?1 AND state = 'done'",
        [&s.id],
        |r| r.get(0),
    )?;
    Ok((discovered, done))
}

pub fn status(conn: &Connection, s: &JobRow) -> rusqlite::Result<InventoryStatus> {
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
        origin: s.origin,
        last_error: s.last_error.clone(),
    })
}

pub fn overview(conn: &Connection) -> rusqlite::Result<InventoryOverview> {
    let active = conn
        .query_row(
            &format!(
                "SELECT {JOB_COLUMNS} FROM jobs s
                 JOIN source_roots r ON r.id = s.source_root_id
                 WHERE s.kind = 'scan' AND s.state IN ({})
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
        Some(s) => Some(status(conn, &s)?),
        None => None,
    };

    let queued_sessions = conn.query_row(
        "SELECT COUNT(*) FROM jobs WHERE kind = 'scan' AND state = 'queued'",
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
                (SELECT state FROM jobs WHERE source_root_id = r.id AND kind = 'scan'
                   ORDER BY rowid DESC LIMIT 1),
                (SELECT id FROM jobs WHERE source_root_id = r.id AND kind = 'scan' AND state IN ({active})
                   ORDER BY rowid DESC LIMIT 1)
         FROM source_roots r LEFT JOIN file_instances f ON f.source_root_id = r.id
         GROUP BY r.id ORDER BY r.display_path, r.id"
            .replace("{active}", &state_list(ACTIVE_STATES))
            .as_str(),
    )?;
    let mut roots = roots_stmt
        .query_map([], |r| {
            Ok(RootInventorySummary {
                source_root_id: r.get(0)?,
                last_scan_at: r.get(1)?,
                file_count: r.get(2)?,
                missing_count: r.get(3)?,
                cloud_only_count: r.get(4)?,
                last_state: opt_from_db(r.get(5)?),
                active_job_id: r.get(6)?,
                active_state: None,
                last_scan: None,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    // Every scan scheduled (Slice 4.5), in the status bar's order.
    let active_rows: Vec<JobRow> = conn
        .prepare_cached(&format!(
            "SELECT {JOB_COLUMNS} FROM jobs s
             JOIN source_roots r ON r.id = s.source_root_id
             WHERE s.kind = 'scan' AND s.state IN ({})
             ORDER BY CASE s.state WHEN 'running' THEN 0 WHEN 'paused_policy' THEN 1
                          WHEN 'paused_user' THEN 2 WHEN 'failed_retryable' THEN 3 ELSE 4 END,
                      s.created_at, s.rowid",
            state_list(ACTIVE_STATES)
        ))?
        .query_map([], job_from_row)?
        .collect::<rusqlite::Result<_>>()?;
    let mut active_jobs = Vec::with_capacity(active_rows.len());
    for row in &active_rows {
        active_jobs.push(status(conn, row)?);
    }
    for root in &mut roots {
        root.active_state = active_rows
            .iter()
            .find(|j| Some(&j.id) == root.active_job_id.as_ref())
            .map(|j| j.state);
        root.last_scan = scans(conn, &root.source_root_id, 1)?.into_iter().next();
    }

    Ok(InventoryOverview {
        active,
        queued_sessions,
        cataloged_files,
        missing_files,
        cloud_only_files,
        roots,
        active_jobs,
    })
}

/// A folder's scans that ended, newest first (Slice 4.5).
pub fn scans(conn: &Connection, root_id: &str, limit: u32) -> rusqlite::Result<Vec<ScanRecord>> {
    conn.prepare_cached(
        "SELECT j.id, j.origin, j.state, j.started_at, j.finished_at, j.elapsed_ms,
                j.discovered_files, j.new_files, j.changed_files, j.missing_files,
                (SELECT COUNT(*) FROM scan_problems p WHERE p.job_id = j.id), j.last_error
         FROM jobs j
         WHERE j.source_root_id = ?1 AND j.kind = 'scan'
           AND j.state IN ('completed', 'cancelled', 'failed_terminal')
         ORDER BY j.rowid DESC LIMIT ?2",
    )?
    .query_map(params![root_id, limit], |r| {
        let state: String = r.get(2)?;
        Ok(ScanRecord {
            job_id: r.get(0)?,
            origin: opt_from_db(r.get(1)?),
            state: crate::services::catalog::codec::from_db(&state)
                .unwrap_or(JobState::FailedTerminal),
            started_at: r.get(3)?,
            finished_at: r.get(4)?,
            elapsed_ms: r.get::<_, i64>(5)?.max(0) as u64,
            discovered_files: r.get(6)?,
            new_files: r.get(7)?,
            changed_files: r.get(8)?,
            missing_files: r.get(9)?,
            problems: r.get(10)?,
            last_error: r.get(11)?,
        })
    })?
    .collect()
}

/// The folders a scan couldn't read (Slice 4.5).
pub fn problems(conn: &Connection, job_id: &str) -> rusqlite::Result<Vec<ScanProblem>> {
    conn.prepare_cached(
        "SELECT display_path, code FROM scan_problems WHERE job_id = ?1 ORDER BY id",
    )?
    .query_map([job_id], |r| {
        Ok(ScanProblem {
            display_path: r.get(0)?,
            code: r.get(1)?,
        })
    })?
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::jobs::db::{insert_job, transition};

    #[test]
    fn a_batch_for_a_session_that_left_running_writes_nothing() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::services::catalog::migrations::migrate(&mut conn, "t").unwrap();
        conn.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
        insert_job(&conn, "s1", "r1", JobKind::Scan, "t").unwrap();
        assert!(
            transition(
                &conn,
                "s1",
                JobState::Running,
                &[JobState::Queued],
                None,
                None,
                "t"
            )
            .unwrap()
        );
        let batch = |dir: &[u8]| Batch {
            dirs: vec![dir.to_vec()],
            skipped_entries: 1,
            ..Batch::default()
        };
        let dirs = |c: &Connection| -> i64 {
            c.query_row("SELECT COUNT(*) FROM scan_directories", [], |r| r.get(0))
                .unwrap()
        };

        assert_eq!(
            commit_batch(
                &mut conn,
                "s1",
                "r1",
                &batch(b"a"),
                &Deferred::default(),
                false,
                false,
                "t"
            )
            .unwrap(),
            Committed::Yes { replayed: 0 }
        );
        assert_eq!(dirs(&conn), 1);

        transition(
            &conn,
            "s1",
            JobState::PausedUser,
            &[JobState::Running],
            None,
            None,
            "t",
        )
        .unwrap();
        assert_eq!(
            commit_batch(
                &mut conn,
                "s1",
                "r1",
                &batch(b"b"),
                &Deferred::default(),
                false,
                false,
                "t"
            )
            .unwrap(),
            Committed::NotRunning
        );
        assert_eq!(dirs(&conn), 1, "the late batch must not land");
        let skipped: u32 = conn
            .query_row(
                "SELECT skipped_entries FROM jobs WHERE id = 's1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(skipped, 1);
    }

    fn running_session(conn: &Connection, id: &str) {
        insert_job(conn, id, "r1", JobKind::Scan, "t").unwrap();
        assert!(
            transition(
                conn,
                id,
                JobState::Running,
                &[JobState::Queued],
                None,
                None,
                "t"
            )
            .unwrap()
        );
    }

    fn file(name: &str, size: u64) -> FileRecord {
        FileRecord {
            rel_path: name.as_bytes().to_vec(),
            display_path: name.into(),
            display_name: name.into(),
            extension: "pdf".into(),
            file_type: "pdf".into(),
            size_bytes: Some(size),
            mtime_secs: Some(1),
            modified_at: None,
            cloud_only: false,
            search_key: name.into(),
            name_key: name.into(),
        }
    }

    #[test]
    fn entries_a_session_already_wrote_count_as_replayed() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::services::catalog::migrations::migrate(&mut conn, "t").unwrap();
        conn.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
        running_session(&conn, "s1");
        let batch = |files: Vec<FileRecord>| Batch {
            dirs: vec![b"sub".to_vec()],
            files,
            ..Batch::default()
        };
        let updated = |c: &Connection| -> String {
            c.query_row(
                "SELECT updated_at FROM file_instances WHERE rel_path = CAST('a.pdf' AS BLOB)",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };

        let first = commit_batch(
            &mut conn,
            "s1",
            "r1",
            &batch(vec![file("a.pdf", 1), file("b.pdf", 2)]),
            &Deferred::default(),
            false,
            false,
            "t1",
        );
        assert_eq!(first.unwrap(), Committed::Yes { replayed: 0 });

        // The same folder listed again after a pause: nothing changes.
        let again = commit_batch(
            &mut conn,
            "s1",
            "r1",
            &batch(vec![file("a.pdf", 1), file("b.pdf", 2)]),
            &Deferred::default(),
            false,
            false,
            "t2",
        );
        assert_eq!(again.unwrap(), Committed::Yes { replayed: 3 });
        assert_eq!(updated(&conn), "t1");

        // A file that changed meanwhile is written, not replayed.
        let changed = commit_batch(
            &mut conn,
            "s1",
            "r1",
            &batch(vec![file("a.pdf", 9), file("b.pdf", 2)]),
            &Deferred::default(),
            false,
            false,
            "t3",
        );
        assert_eq!(changed.unwrap(), Committed::Yes { replayed: 2 });
        assert_eq!(updated(&conn), "t3");

        // A rescan is a new session (Slice 4.5): what is unchanged isn't
        // written; only the session's new frontier row is.
        running_session(&conn, "s2");
        let rescan = commit_batch(
            &mut conn,
            "s2",
            "r1",
            &batch(vec![file("a.pdf", 9), file("b.pdf", 2)]),
            &Deferred::default(),
            false,
            false,
            "t4",
        );
        assert_eq!(rescan.unwrap(), Committed::Yes { replayed: 2 });
        assert_eq!(updated(&conn), "t3");
        let (new, changed): (i64, i64) = conn
            .query_row(
                "SELECT new_files, changed_files FROM jobs WHERE id = 's2'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((new, changed), (0, 0));
        // The first session counted what it wrote: two new, then one
        // changed.
        let (new, changed): (i64, i64) = conn
            .query_row(
                "SELECT new_files, changed_files FROM jobs WHERE id = 's1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((new, changed), (2, 1));

        // A scan begun before Slice 4.5 writes every entry again, as Phase 0
        // did, so it can find unseen rows by its marker.
        running_session(&conn, "s3");
        let legacy = commit_batch(
            &mut conn,
            "s3",
            "r1",
            &batch(vec![file("a.pdf", 9), file("b.pdf", 2)]),
            &Deferred::default(),
            true,
            false,
            "t5",
        );
        assert_eq!(legacy.unwrap(), Committed::Yes { replayed: 0 });
        let seen: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM file_instances WHERE last_seen_session_id = 's3'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(seen, 2);
    }

    #[test]
    fn a_rescan_holds_unchanged_folders_and_writes_them_with_later_work() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::services::catalog::migrations::migrate(&mut conn, "t").unwrap();
        conn.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
        running_session(&conn, "s1");
        seed_frontier(&conn, "s1", b"").unwrap();
        let root: i64 = conn
            .query_row("SELECT id FROM scan_directories", [], |r| r.get(0))
            .unwrap();
        let listed = |files: Vec<FileRecord>, done: Option<i64>| Batch {
            dirs: vec![b"sub".to_vec()],
            files,
            done_dir: done,
            listing: Some(Listing {
                dir_rel: native_path::encode(Path::new("")),
                names: ["a.pdf", "sub"].map(OsString::from).into_iter().collect(),
                files: 1,
            }),
            skipped_entries: 1,
            ..Batch::default()
        };
        // Encoded as the scan encodes paths, so the folder's range holds it.
        let stored = |name: &str| FileRecord {
            rel_path: native_path::encode(Path::new(name)),
            ..file(name, 1)
        };
        let commit = |conn: &mut Connection, id: &str, b: &Batch, held: &Deferred| {
            commit_batch(conn, id, "r1", b, held, false, true, "t").unwrap()
        };
        let count =
            |c: &Connection, sql: &str| -> i64 { c.query_row(sql, [], |r| r.get(0)).unwrap() };

        // A first scan has something to write, so it is never held.
        let first = listed(vec![stored("a.pdf")], Some(root));
        assert_eq!(
            commit(&mut conn, "s1", &first, &Deferred::default()),
            Committed::Yes { replayed: 0 }
        );

        // A rescan of the same folder: nothing to write, so nothing written.
        running_session(&conn, "s2");
        seed_frontier(&conn, "s2", b"").unwrap();
        let root2: i64 = conn
            .query_row(
                "SELECT id FROM scan_directories WHERE session_id = 's2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let quiet = listed(vec![stored("a.pdf")], Some(root2));
        let before = count(&conn, "SELECT total_changes()");
        assert_eq!(
            commit(&mut conn, "s2", &quiet, &Deferred::default()),
            Committed::Deferred {
                replayed: 1,
                write: vec![],
                missing: vec![]
            }
        );
        assert_eq!(
            count(&conn, "SELECT total_changes()"),
            before,
            "nothing written"
        );
        // The held folder is skipped when choosing the next one.
        assert!(next_dir(&conn, "s2", 3, &[root2]).unwrap().is_none());

        // Held work goes in with the next commit, which marks the folder
        // done, adds its subfolder and counts what it listed.
        let mut held = Deferred::default();
        held.absorb(quiet, &[], vec![]);
        let flushed = commit_batch(
            &mut conn,
            "s2",
            "r1",
            &Batch::default(),
            &held,
            false,
            false,
            "t",
        );
        assert_eq!(flushed.unwrap(), Committed::Yes { replayed: 0 });
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM scan_directories WHERE session_id = 's2' AND state = 'done'"
            ),
            1
        );
        assert_eq!(
            next_dir(&conn, "s2", 3, &[]).unwrap().unwrap().rel_path,
            b"sub".to_vec()
        );
        let (found, skipped): (i64, i64) = conn
            .query_row(
                "SELECT discovered_files, skipped_entries FROM jobs WHERE id = 's2'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((found, skipped), (1, 1));

        // A file changed and a file gone are held too, and written with
        // the held work; a new file is written at once, with what is held.
        running_session(&conn, "s3");
        let mut gone = listed(vec![], None);
        gone.listing
            .as_mut()
            .unwrap()
            .names
            .remove(std::ffi::OsStr::new("a.pdf"));
        let missing_id: String = conn
            .query_row("SELECT id FROM file_instances", [], |r| r.get(0))
            .unwrap();
        let before = count(&conn, "SELECT total_changes()");
        let Committed::Deferred { missing, .. } =
            commit(&mut conn, "s3", &gone, &Deferred::default())
        else {
            panic!("a missing file alone is held");
        };
        assert_eq!(missing, vec![missing_id]);
        let changed = Batch {
            files: vec![FileRecord {
                size_bytes: Some(7),
                ..stored("a.pdf")
            }],
            ..Batch::default()
        };
        let Committed::Deferred { write, .. } =
            commit(&mut conn, "s3", &changed, &Deferred::default())
        else {
            panic!("a changed file is held");
        };
        assert_eq!(write, vec![0]);
        assert_eq!(
            count(&conn, "SELECT total_changes()"),
            before,
            "nothing written"
        );

        let mut held = Deferred::default();
        held.absorb(gone, &[], missing);
        held.absorb(changed, &write, vec![]);
        let new = Batch {
            files: vec![stored("b.pdf")],
            ..Batch::default()
        };
        assert_eq!(
            commit(&mut conn, "s3", &new, &held),
            Committed::Yes { replayed: 0 }
        );
        assert_eq!(
            count(
                &conn,
                "SELECT COUNT(*) FROM scan_missing WHERE session_id = 's3'"
            ),
            1
        );
        assert_eq!(
            count(
                &conn,
                "SELECT size_bytes FROM file_instances WHERE name_key = 'a.pdf'"
            ),
            7
        );
        let (new, changed): (i64, i64) = conn
            .query_row(
                "SELECT new_files, changed_files FROM jobs WHERE id = 's3'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((new, changed), (1, 1));
    }

    #[test]
    fn the_range_under_a_folder_holds_its_subtree_only() {
        // Built with `join`, as the scan builds them, so the separator is
        // the platform's.
        let rel = |p: &str| {
            native_path::encode(
                &p.split('/')
                    .fold(PathBuf::new(), |acc, part| acc.join(part)),
            )
        };
        let (start, end) = child_range(Path::new("a"));
        let end = end.unwrap();
        let inside = |p: &str| rel(p) >= start && rel(p) < end;
        assert!(inside("a/x.pdf") && inside("a/b/y.pdf"));
        assert!(!inside("a") && !inside("ab/x.pdf") && !inside("b/x.pdf"));
        // The folder itself: every row of the root.
        let (start, end) = child_range(Path::new(""));
        assert!(rel("x.pdf") >= start && end.is_none_or(|e| rel("x.pdf") < e));
    }
}
