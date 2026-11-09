//! Slice 1.1: general jobs (phase1-spec §7.1, ADR 0009; tests P1-01 and
//! P1-02). A Phase 0 catalog upgrades to `jobs` without losing a paused
//! scan, and jobs of two kinds run at once through the one writer, each
//! paused without touching the other.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use rusqlite::Connection;
use rusqlite::types::Value;

use atlas_lib::domain::dto::{DocumentQuery, DocumentSort, InventoryOverview, JobKind, JobState};
use atlas_lib::error::ErrorCode;
use atlas_lib::infrastructure::native_path;
use atlas_lib::services::catalog::{SUPPORTED_SCHEMA_VERSION, migrations};
use atlas_lib::services::inventory;
use atlas_lib::services::jobs::{
    self, Flow, JobProcessor, JobRow, ProcessorContext, ProcessorFactory,
};
use atlas_lib::services::library::{self, fold::fold};
use atlas_lib::services::sources;
use atlas_lib::services::workspace::{OpenRoot, Workspace, manifest};

mod common;
use common::{Env, FaultySource, InventoryHarness, code, fast_config, wait_until};

/// Bounds a failure, not a performance claim: shared CI runners are slow.
const LONG: Duration = Duration::from_secs(120);

fn write(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn many(root: &Path, n: usize) {
    for i in 0..n {
        write(
            &root.join(format!("d{:02}/book-{i:05}.pdf", i % 20)),
            b"%PDF-1.4",
        );
    }
}

/// Every row `sql` returns, as raw values (so a comparison covers every
/// column, whatever its type).
fn rows(c: &Connection, sql: &str) -> rusqlite::Result<Vec<Vec<Value>>> {
    let mut stmt = c.prepare(sql)?;
    let n = stmt.column_count();
    stmt.query_map([], |r| (0..n).map(|i| r.get::<_, Value>(i)).collect())?
        .collect()
}

fn query(root: &OpenRoot, sql: impl Into<String>) -> Vec<Vec<Value>> {
    let sql = sql.into();
    root.catalog()
        .call(move |c| rows(c, &sql))
        .unwrap()
        .unwrap()
}

fn one_u32(root: &OpenRoot, sql: impl Into<String>) -> u32 {
    match query(root, sql).as_slice() {
        [row] => match row.as_slice() {
            [Value::Integer(n)] => u32::try_from(*n).unwrap(),
            other => panic!("expected one integer, got {other:?}"),
        },
        other => panic!("expected one row, got {other:?}"),
    }
}

fn state(root: &OpenRoot, job: &str) -> Option<JobState> {
    let id = job.to_owned();
    root.catalog()
        .call(move |c| {
            c.query_row("SELECT state FROM jobs WHERE id = ?1", [id], |r| {
                r.get::<_, String>(0)
            })
            .ok()
        })
        .unwrap()
        .and_then(|s| atlas_lib::services::catalog::codec::from_db(&s))
}

fn wait_state(root: &OpenRoot, job: &str, want: JobState) {
    assert!(
        wait_until(LONG, || state(root, job) == Some(want)),
        "job never reached {want:?}; now {:?}",
        state(root, job)
    );
}

// ------------------------------------------------------------------ P1-01

/// P1-01: a schema-6 catalog (Phase 0) with a scan the user paused mid-way
/// opens as schema 7. The scan is a `jobs` row of kind `scan`, unchanged in
/// every column, its frontier is intact, and resuming it lists only the
/// folders that were still pending.
#[test]
fn p1_01_upgrade_keeps_a_paused_scan_and_its_frontier() {
    let env = Env::new();
    let h = InventoryHarness::new(fast_config());
    let data_root = env.base.join("AtlasData");
    let books = env.base.join("Books");
    write(&books.join("a/x.pdf"), b"%PDF-1.4");
    write(&books.join("b/y.pdf"), b"%PDF-1.4");
    write(&books.join("c/z.epub"), b"PK");

    // A current library with one folder (no scan scheduled: no inventory).
    let source_id = {
        let mut ws = env.workspace();
        ws.choose(&data_root).unwrap();
        sources::add(ws.require_open().unwrap(), &books).unwrap().id
    };

    // Rebuild its catalog in place as schema 6. Rebuilding in place, rather
    // than swapping in another file, keeps the WAL consistent. Source roots
    // (schema 2) are kept; everything from schema 3 on is recreated by the
    // Phase 0 migrations.
    let catalog = data_root.join("library/catalog.sqlite");
    let rel = |p: &str| native_path::encode(Path::new(p));
    {
        let mut conn = Connection::open(&catalog).unwrap();
        let tables: Vec<String> = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT IN
                 ('schema_migrations', 'workspace', 'source_roots') AND name NOT LIKE 'sqlite_%'",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        conn.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
        for table in tables {
            conn.execute_batch(&format!("DROP TABLE \"{table}\""))
                .unwrap();
        }
        conn.execute("DELETE FROM schema_migrations WHERE version >= 3", [])
            .unwrap();
        // As the app's writer does: foreign keys on while migrating.
        conn.execute_batch("PRAGMA foreign_keys = ON").unwrap();
        assert_eq!(
            migrations::migrate_to(&mut conn, 6, "2026-10-01T00:00:00Z").unwrap(),
            6
        );
        assert!(
            rows(&conn, "SELECT name FROM sqlite_master WHERE name = 'jobs'")
                .unwrap()
                .is_empty()
        );

        // What a Phase 0 build leaves behind when the user pauses a scan
        // after it listed the root and `a/`, and failed `b/` once.
        conn.execute(
            "INSERT INTO scan_sessions (id, source_root_id, state, pause_reason,
                 throttle_reason, priority, created_at, started_at, finished_at, updated_at,
                 scanned_directories, discovered_files, skipped_symlinks, skipped_entries,
                 error_count, retry_count, elapsed_ms, last_error)
             VALUES ('job-6', ?1, 'paused_user', NULL, NULL, 0, '2026-10-01T10:00:00Z',
                 '2026-10-01T10:00:01Z', NULL, '2026-10-01T10:00:09Z', 0, 0, 1, 2, 1, 0,
                 8123, 'read_failed')",
            [&source_id],
        )
        .unwrap();
        for (path, dir_state, attempts, error) in [
            ("", "done", 0, None),
            ("a", "done", 0, None),
            ("b", "pending", 1, Some("read_failed")),
            ("c", "pending", 0, None),
        ] {
            conn.execute(
                "INSERT INTO scan_directories (session_id, rel_path, state, attempts, last_error)
                 VALUES ('job-6', ?1, ?2, ?3, ?4)",
                rusqlite::params![rel(path), dir_state, attempts, error],
            )
            .unwrap();
        }
        let display = native_path::display(&dunce::canonicalize(books.join("a/x.pdf")).unwrap());
        conn.execute(
            "INSERT INTO file_instances (id, source_root_id, rel_path, display_path,
                 display_name, extension, file_type, size_bytes, mtime_secs, modified_at,
                 availability, storage, last_seen_session_id, created_at, updated_at,
                 search_key, name_key)
             VALUES ('file-x', ?1, ?2, ?3, 'x.pdf', 'pdf', 'pdf', 8, NULL, NULL, 'available',
                 'local', 'job-6', '2026-10-01T10:00:05Z', '2026-10-01T10:00:05Z', ?4, ?5)",
            rusqlite::params![
                source_id,
                rel("a/x.pdf"),
                display,
                fold(&native_path::display(Path::new("a/x.pdf"))),
                fold("x.pdf")
            ],
        )
        .unwrap();
    }
    let mut m = manifest::read(&data_root).unwrap();
    m.catalog_schema_version = 6;
    manifest::write(&data_root, &m).unwrap();

    let (job_before, job_columns, frontier_before, files_before, file_columns) = {
        let conn = Connection::open(&catalog).unwrap();
        let columns = |table: &str| -> String {
            conn.prepare(&format!(
                "SELECT name FROM pragma_table_info('{table}') ORDER BY cid"
            ))
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .join(", ")
        };
        (
            rows(&conn, "SELECT * FROM scan_sessions").unwrap(),
            columns("scan_sessions"),
            rows(&conn, "SELECT * FROM scan_directories ORDER BY id").unwrap(),
            rows(&conn, "SELECT * FROM file_instances ORDER BY id").unwrap(),
            columns("file_instances"),
        )
    };

    // Open it with the current build, as the app does at launch.
    let mut ws = env.workspace_with_inventory(h.deps.clone());
    ws.startup();
    let root = ws.require_open().unwrap();

    // Schema 6 opens as the current schema (7 added jobs; later versions
    // add columns and tables beside them).
    const { assert!(SUPPORTED_SCHEMA_VERSION >= 7) };
    assert_eq!(
        manifest::read(&data_root).unwrap().catalog_schema_version,
        SUPPORTED_SCHEMA_VERSION
    );
    let versions: Vec<Vec<Value>> = query(
        root,
        "SELECT version FROM schema_migrations ORDER BY version",
    );
    assert_eq!(
        versions,
        (1..=i64::from(SUPPORTED_SCHEMA_VERSION))
            .map(|v| vec![Value::Integer(v)])
            .collect::<Vec<_>>()
    );
    // The same row, every column, now with a kind.
    assert_eq!(
        query(
            root,
            format!("SELECT {job_columns} FROM jobs WHERE kind = 'scan'")
        ),
        job_before
    );
    assert_eq!(
        query(root, "SELECT kind FROM jobs WHERE id = 'job-6'"),
        vec![vec![Value::Text("scan".into())]]
    );
    assert_eq!(
        query(root, "SELECT * FROM scan_directories ORDER BY id"),
        frontier_before
    );
    assert_eq!(
        query(
            root,
            format!("SELECT {file_columns} FROM file_instances ORDER BY id")
        ),
        files_before
    );
    // Its cataloged file waits for its text (phase1-spec §7.1).
    assert_eq!(
        one_u32(root, "SELECT COUNT(*) FROM jobs WHERE kind = 'extract'"),
        1
    );
    // The paused scan counts as this folder's scan: none was added.
    assert_eq!(
        one_u32(root, "SELECT COUNT(*) FROM jobs WHERE kind = 'scan'"),
        1
    );
    let active = inventory::control::overview(root).unwrap().active.unwrap();
    assert_eq!(active.job_id, "job-6");
    assert_eq!(active.state, JobState::PausedUser);
    assert_eq!(active.discovered_files, 1);
    assert_eq!(active.completed_directories, 2);
    assert!(query(root, "PRAGMA foreign_key_check").is_empty());

    jobs::control::resume(root, "job-6").unwrap();
    wait_state(root, "job-6", JobState::Completed);

    // Only the pending folders were listed, once each: the frontier was used.
    let calls = h.source.calls.lock().unwrap().clone();
    let pending: Vec<PathBuf> = ["b", "c"]
        .iter()
        .map(|d| dunce::canonicalize(books.join(d)).unwrap())
        .collect();
    assert_eq!(calls.len(), 2, "{calls:?}");
    for dir in &pending {
        assert_eq!(h.source.calls_for(dir), 1, "{calls:?}");
    }
    let files = query(
        root,
        "SELECT id, rel_path, availability FROM file_instances ORDER BY display_name",
    );
    assert_eq!(files.len(), 3);
    assert_eq!(
        files[0][0],
        Value::Text("file-x".into()),
        "x.pdf kept its ID"
    );
    assert!(
        files
            .iter()
            .all(|f| f[2] == Value::Text("available".into()))
    );
    let ov = inventory::control::overview(root).unwrap();
    assert_eq!(ov.missing_files, 0);
    assert_eq!(
        one_u32(root, "SELECT discovered_files FROM jobs WHERE id = 'job-6'"),
        3
    );
    assert_eq!(
        one_u32(
            root,
            "SELECT scanned_directories FROM jobs WHERE id = 'job-6'"
        ),
        4
    );

    // The renamed table still cascades from its source root.
    sources::remove(root, &source_id).unwrap();
    assert_eq!(one_u32(root, "SELECT COUNT(*) FROM jobs"), 0);
    assert_eq!(one_u32(root, "SELECT COUNT(*) FROM scan_directories"), 0);
    assert!(query(root, "PRAGMA foreign_key_check").is_empty());
}

// ------------------------------------------------------------------ P1-02

/// A stand-in for extraction (Slice 1.3): each unit of work is one guarded
/// write through the catalog writer, and it completes after `finish_after`
/// units. It re-checks its state and the stop signal between units, as a
/// real processor must.
struct FakeExtract {
    ctx: ProcessorContext,
    finish_after: Arc<AtomicU32>,
}

const UNIT: Duration = Duration::from_millis(2);

impl JobProcessor for FakeExtract {
    fn idle_wait(&self) -> Duration {
        Duration::from_millis(20)
    }

    fn run(&mut self, job: JobRow) -> Flow {
        if job.kind != JobKind::Extract {
            return Flow::Continue;
        }
        let id = job.id.clone();
        let started = self.ctx.catalog.call(move |c| {
            jobs::db::transition(
                c,
                &id,
                JobState::Running,
                &[JobState::Queued, JobState::PausedPolicy, JobState::Running],
                None,
                None,
                "t",
            )
        });
        if !matches!(started, Ok(Ok(true))) {
            return Flow::Continue;
        }
        loop {
            if self.ctx.signal.stopped() {
                return Flow::Stop;
            }
            let id = job.id.clone();
            let limit = self.finish_after.load(Ordering::SeqCst);
            let unit = self.ctx.catalog.call(move |c| -> rusqlite::Result<bool> {
                let wrote = c.execute(
                    "UPDATE jobs SET discovered_files = discovered_files + 1
                     WHERE id = ?1 AND state = 'running'",
                    [&id],
                )?;
                if wrote == 0 {
                    // Paused, cancelled, or removed: stop at this boundary.
                    return Ok(false);
                }
                let done: u32 = c.query_row(
                    "SELECT discovered_files FROM jobs WHERE id = ?1",
                    [&id],
                    |r| r.get(0),
                )?;
                if done >= limit {
                    jobs::db::transition(
                        c,
                        &id,
                        JobState::Completed,
                        &[JobState::Running],
                        None,
                        None,
                        "t",
                    )?;
                    return Ok(false);
                }
                Ok(true)
            });
            if !matches!(unit, Ok(Ok(true))) {
                return Flow::Continue;
            }
            if self.ctx.signal.wait(UNIT) {
                return Flow::Stop;
            }
        }
    }
}

fn fake_extract(finish_after: Arc<AtomicU32>) -> ProcessorFactory {
    Arc::new(move |ctx| {
        Box::new(FakeExtract {
            ctx,
            finish_after: Arc::clone(&finish_after),
        })
    })
}

/// Releases a held listing even when an assertion fails, so a failing test
/// fails instead of hanging in the runner's shutdown.
struct ReleaseOnDrop(Arc<FaultySource>);

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

struct Lib {
    ws: Workspace,
    h: InventoryHarness,
    finish_after: Arc<AtomicU32>,
    env: Env,
}

impl Lib {
    fn new() -> Self {
        let env = Env::new();
        let h = InventoryHarness::new(fast_config());
        let finish_after = Arc::new(AtomicU32::new(u32::MAX));
        let mut ws = env
            .workspace_with_inventory(h.deps.clone())
            .with_job_processor(JobKind::Extract, fake_extract(Arc::clone(&finish_after)));
        ws.choose(&env.base.join("AtlasData")).unwrap();
        Self {
            ws,
            h,
            finish_after,
            env,
        }
    }

    fn root(&self) -> &OpenRoot {
        self.ws.require_open().unwrap()
    }

    /// A folder with 300 books in 20 subfolders, added and with an extract
    /// job and a scan scheduled. The scan is held before one of its
    /// listings, so it is mid-way and cannot progress until released.
    fn start_both(&self) -> (String, String, String) {
        let books = self.env.base.join("Books");
        many(&books, 300);
        self.h.source.slow(Duration::from_millis(1));
        let source = sources::add(self.root(), &books).unwrap().id;
        let extract = jobs::control::enqueue(self.root(), &source, JobKind::Extract).unwrap();
        let scan = inventory::control::enqueue(self.root(), &source).unwrap();
        assert!(self.h.source.hold_next_listing(LONG));
        (source, scan, extract)
    }

    fn state(&self, job: &str) -> Option<JobState> {
        state(self.root(), job)
    }

    fn units(&self, job: &str) -> u32 {
        one_u32(
            self.root(),
            format!("SELECT discovered_files FROM jobs WHERE id = '{job}'"),
        )
    }

    fn file_count(&self) -> u32 {
        one_u32(self.root(), "SELECT COUNT(*) FROM file_instances")
    }

    /// Everything a scan writes: its job row, frontier, and files.
    fn scan_rows(&self, job: &str) -> Vec<Vec<Vec<Value>>> {
        vec![
            query(
                self.root(),
                format!("SELECT * FROM jobs WHERE id = '{job}'"),
            ),
            query(
                self.root(),
                format!("SELECT * FROM scan_directories WHERE session_id = '{job}' ORDER BY id"),
            ),
            query(self.root(), "SELECT * FROM file_instances ORDER BY id"),
        ]
    }

    fn events(&self) -> Vec<InventoryOverview> {
        self.h.events.lock().unwrap().clone()
    }

    fn scanning(&self) -> bool {
        let q = DocumentQuery {
            text: None,
            source_root_id: None,
            file_type: None,
            sort: DocumentSort::NameAsc,
            page: 0,
            page_size: 10,
            include_missing: false,
            text_status: None,
        };
        library::list(self.root().reader(), self.root().root_status(), &q)
            .unwrap()
            .scanning
    }
}

/// P1-02: a scan and an extract job run at the same time through the one
/// writer. Pausing either stops only that job, with no writes after the
/// pause; the scan's status never reports the extract job.
#[test]
fn p1_02_a_scan_and_an_extract_job_run_together_and_pause_independently() {
    let lib = Lib::new();
    let _release = ReleaseOnDrop(Arc::clone(&lib.h.source));
    let (_source, scan, extract) = lib.start_both();

    // Both are running at once: the scan is mid-way (held), and the
    // extract job keeps writing.
    assert!(wait_until(LONG, || {
        lib.state(&scan) == Some(JobState::Running)
            && lib.state(&extract) == Some(JobState::Running)
    }));
    let before = lib.units(&extract);
    assert!(wait_until(LONG, || lib.units(&extract) >= before + 10));
    assert_eq!(lib.state(&scan), Some(JobState::Running));

    // Pause the scan while its listing is in flight, then let the listing
    // finish. Its batch must not land, while the extract job carries on.
    jobs::control::pause(lib.root(), &scan).unwrap();
    let frozen = lib.scan_rows(&scan);
    lib.h.source.release();
    // The scan reports itself paused only after its in-flight batch was
    // refused, so this proves the late batch has been and gone.
    assert!(wait_until(LONG, || {
        lib.events().iter().any(|o| {
            o.active
                .as_ref()
                .is_some_and(|a| a.job_id == scan && a.state == JobState::PausedUser)
        })
    }));
    let units = lib.units(&extract);
    assert!(wait_until(LONG, || lib.units(&extract) >= units + 10));
    assert_eq!(
        lib.scan_rows(&scan),
        frozen,
        "no scan writes after its pause"
    );
    assert_eq!(lib.state(&extract), Some(JobState::Running));

    // The reverse: pause the extract job; the scan runs to completion and
    // the extract job's counter does not move.
    jobs::control::pause(lib.root(), &extract).unwrap();
    let frozen_units = lib.units(&extract);
    let files = lib.file_count();
    jobs::control::resume(lib.root(), &scan).unwrap();
    wait_state(lib.root(), &scan, JobState::Completed);
    assert_eq!(lib.file_count(), 300);
    assert!(lib.file_count() > files);
    assert_eq!(
        lib.units(&extract),
        frozen_units,
        "no extract writes after its pause"
    );

    // Kind isolation: with only the (paused) extract job left, cataloging
    // reads as idle everywhere the UI looks.
    assert_eq!(lib.state(&extract), Some(JobState::PausedUser));
    let ov = inventory::control::overview(lib.root()).unwrap();
    assert!(ov.active.is_none(), "{:?}", ov.active);
    assert_eq!(ov.queued_sessions, 0);
    assert!(ov.roots.iter().all(|r| r.active_job_id.is_none()));
    assert_eq!(ov.roots[0].last_state, Some(JobState::Completed));
    assert!(!lib.scanning());

    // Resume the extract job; it completes too.
    lib.finish_after.store(frozen_units + 5, Ordering::SeqCst);
    jobs::control::resume(lib.root(), &extract).unwrap();
    wait_state(lib.root(), &extract, JobState::Completed);
    assert_eq!(lib.units(&extract), frozen_units + 5);

    // No cataloging status the UI received ever carried the extract job.
    let events = lib.events();
    assert!(!events.is_empty());
    for o in &events {
        assert!(o.active.as_ref().is_none_or(|a| a.job_id != extract));
        assert!(
            o.roots
                .iter()
                .all(|r| r.active_job_id.as_deref() != Some(extract.as_str()))
        );
    }
}

/// P1-02, negative cases: job commands check the state of any kind, and
/// cancelling one kind leaves the other's work alone.
#[test]
fn p1_02_job_commands_check_state_for_every_kind() {
    let lib = Lib::new();
    let _release = ReleaseOnDrop(Arc::clone(&lib.h.source));
    let (source, scan, extract) = lib.start_both();
    let root = lib.root();
    wait_state(root, &extract, JobState::Running);

    // Scheduling is idempotent per folder and kind, and kinds don't mix.
    assert_eq!(
        jobs::control::enqueue(root, &source, JobKind::Extract).unwrap(),
        extract
    );
    assert_eq!(inventory::control::enqueue(root, &source).unwrap(), scan);
    assert_eq!(
        code(jobs::control::enqueue(root, "nope", JobKind::Extract)),
        ErrorCode::SourceNotFound
    );

    // Unknown jobs, and a job that is not paused.
    assert_eq!(
        code(jobs::control::pause(root, "nope")),
        ErrorCode::JobNotFound
    );
    assert_eq!(
        code(jobs::control::resume(root, "nope")),
        ErrorCode::JobNotFound
    );
    assert_eq!(
        code(jobs::control::cancel(root, "nope")),
        ErrorCode::JobNotFound
    );
    assert_eq!(
        code(jobs::control::resume(root, &extract)),
        ErrorCode::JobNotResumable
    );

    // Cancelling the extract job leaves the paused scan's frontier alone.
    jobs::control::pause(root, &scan).unwrap();
    let frontier = |job: &str| {
        query(
            root,
            format!("SELECT * FROM scan_directories WHERE session_id = '{job}' ORDER BY id"),
        )
    };
    let scan_frontier = frontier(&scan);
    assert!(!scan_frontier.is_empty());
    jobs::control::cancel(root, &extract).unwrap();
    assert_eq!(lib.state(&extract), Some(JobState::Cancelled));
    assert_eq!(frontier(&scan), scan_frontier);
    assert_eq!(lib.state(&scan), Some(JobState::PausedUser));
    assert_eq!(
        code(jobs::control::cancel(root, &extract)),
        ErrorCode::JobNotPausable
    );
    // After a cancel, scheduling the kind again starts a new job.
    let again = jobs::control::enqueue(root, &source, JobKind::Extract).unwrap();
    assert_ne!(again, extract);
    jobs::control::cancel(root, &again).unwrap();

    // A completed job can be neither paused nor resumed.
    lib.h.source.release();
    jobs::control::resume(root, &scan).unwrap();
    wait_state(root, &scan, JobState::Completed);
    assert_eq!(
        code(jobs::control::pause(root, &scan)),
        ErrorCode::JobNotPausable
    );
    assert_eq!(
        code(jobs::control::resume(root, &scan)),
        ErrorCode::JobNotResumable
    );
}
