//! Slice 1.3: extraction survives misbehaving workers and a killed app
//! (phase1-spec §7.2, §12; tests P1-11 and P1-12).
//!
//! P1-11 drives the real worker into crashes with its debug-only fault
//! hook. P1-12 re-runs this test binary as a child that reads a library,
//! kills it with SIGKILL (TerminateProcess on Windows) mid-way, then
//! reopens the library here and lets reading finish.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use atlas_lib::domain::dto::{ExtractionState, JobKind};
use atlas_lib::services::extraction::db;
use atlas_lib::services::extraction::job::{self as extraction_job, ExtractionConfig};
use atlas_lib::services::extraction::supervisor::{SupervisorConfig, WorkerStats};
use atlas_lib::services::extraction::worker_main::TEST_FAULT_ENV;
use atlas_lib::services::workspace::{Workspace, WorkspaceConfig};
use atlas_lib::services::{inventory, library, sources};

mod common;
use common::{Env, ExtractionHarness, fast_extraction, local, originals, wait_until, worker};

const LONG: Duration = Duration::from_secs(180);
const CHILD_ENV: &str = "ATLAS_TEST_EXTRACT_CRASH_DIR";

fn corpus(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../spikes/extraction-bench/corpus")
        .join(rel)
}

fn state_of(ws: &Workspace, path: &Path) -> (ExtractionState, Option<String>) {
    let root = ws.require_open().unwrap();
    let q = atlas_lib::domain::dto::DocumentQuery {
        text: None,
        source_root_id: None,
        file_type: None,
        sort: atlas_lib::domain::dto::DocumentSort::NameAsc,
        page: 0,
        page_size: 200,
        include_missing: false,
        text_status: None,
    };
    let canonical = dunce::canonicalize(path).unwrap();
    let page = library::list(root.reader(), root.root_status(), &q).unwrap();
    let f = page
        .items
        .into_iter()
        .find(|f| Path::new(&f.display_path) == canonical)
        .unwrap();
    (f.extraction.state, f.extraction.reason)
}

fn settled(ws: &Workspace) -> bool {
    let root = ws.require_open().unwrap();
    let inv = inventory::control::overview(root).unwrap();
    let ov = root
        .reader()
        .read(|c| db::overview(c, None))
        .unwrap()
        .unwrap();
    inv.active.is_none() && ov.active.is_none() && ov.queued_jobs == 0 && ov.counts.pending == 0
}

/// P1-11: a worker that exits mid-document, or hangs, is killed and
/// replaced; after two tries the file is `worker_crashed`, and the job
/// carries on with the next file.
#[test]
fn p1_11_crashing_and_hanging_files_do_not_stop_the_job() {
    let config = ExtractionConfig {
        supervisor: SupervisorConfig {
            inactivity: Duration::from_secs(1),
            ..SupervisorConfig::default()
        },
        ..fast_extraction()
    };
    let h = ExtractionHarness::new(
        config,
        worker().env(TEST_FAULT_ENV, "crash-exit:exit,crash-hang:hang"),
    );
    let env = Env::new();
    let mut ws = env.workspace_with_extraction(&h);
    ws.choose(&env.base.join("AtlasData")).unwrap();
    let books = env.base.join("Books");
    fs::create_dir_all(&books).unwrap();
    // Different contents, so no file is linked to another's document.
    fs::copy(
        corpus("pdf/generated/one-column.pdf"),
        books.join("crash-exit.pdf"),
    )
    .unwrap();
    fs::copy(
        corpus("pdf/generated/two-column.pdf"),
        books.join("crash-hang.pdf"),
    )
    .unwrap();
    fs::copy(corpus("pdf/generated/book.pdf"), books.join("good.pdf")).unwrap();
    let before = originals(&books);

    let root = ws.require_open().unwrap();
    let added = sources::add(root, &books).unwrap();
    inventory::control::enqueue(root, &added.id).unwrap();
    assert!(wait_until(LONG, || settled(&ws)), "never settled");

    let crashed = (
        ExtractionState::Unreadable,
        Some("worker_crashed".to_owned()),
    );
    assert_eq!(state_of(&ws, &books.join("crash-exit.pdf")), crashed);
    assert_eq!(state_of(&ws, &books.join("crash-hang.pdf")), crashed);
    assert_eq!(
        state_of(&ws, &books.join("good.pdf")).0,
        ExtractionState::Done
    );
    // Two tries each, a fresh worker after every crash.
    assert!(WorkerStats::get(&h.deps.stats.kills) >= 4);
    assert_eq!(originals(&books), before);
}

fn child_workspace(dir: &Path, config: ExtractionConfig) -> (Workspace, ExtractionHarness) {
    let h = ExtractionHarness::new(config, worker());
    let ws = Workspace::new(
        WorkspaceConfig {
            config_dir: dir.join("config"),
            default_root: dir.join("AtlasData"),
        },
        local(),
    )
    .with_inventory(h.inv.deps.clone())
    .with_job_processor(JobKind::Extract, extraction_job::factory(h.deps.clone()));
    (ws, h)
}

fn read_only(dir: &Path) -> Option<rusqlite::Connection> {
    rusqlite::Connection::open_with_flags(
        dir.join("AtlasData/library/catalog.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .ok()
}

fn count(conn: &rusqlite::Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap_or(-1)
}

/// Child half of P1-12: read slowly until killed.
#[test]
#[ignore = "run only as the child of p1_12_kill_mid_extraction_then_resume"]
fn p1_12_child_process() {
    let Ok(dir) = std::env::var(CHILD_ENV) else {
        return;
    };
    let dir = PathBuf::from(dir);
    let (mut ws, _h) = child_workspace(
        &dir,
        ExtractionConfig {
            unthrottled: false,
            ..fast_extraction()
        },
    );
    ws.choose(&dir.join("AtlasData")).unwrap();
    let root = ws.require_open().unwrap();
    let added = sources::add(root, &dir.join("Books")).unwrap();
    inventory::control::enqueue(root, &added.id).unwrap();
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// P1-12: a kill mid-extraction leaves no partial document; on relaunch the
/// rest is read with no duplicates.
#[test]
fn p1_12_kill_mid_extraction_then_resume() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let out = Command::new("node")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/generate-fixtures.mjs"))
        .arg(dir.join("Books"))
        .args(["--files", "300", "--text-kb", "16"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let before = originals(&dir.join("Books"));

    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["p1_12_child_process", "--exact", "--ignored", "--nocapture"])
        .env(CHILD_ENV, &dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let started = wait_until(Duration::from_secs(60), || {
        read_only(&dir).is_some_and(|c| count(&c, "SELECT COUNT(*) FROM documents") >= 20)
    });
    child.kill().unwrap(); // SIGKILL: no destructors, no checkpoint
    child.wait().unwrap();
    assert!(started, "the child never started reading");

    // What the kill left: whole documents only.
    {
        let c = read_only(&dir).unwrap();
        let total = count(&c, "SELECT COUNT(*) FROM file_instances");
        let pending = count(
            &c,
            "SELECT COUNT(*) FROM file_instances WHERE extract_state = 'pending'",
        );
        assert!(pending > 0, "the kill landed after reading finished");
        assert!(pending < total);
        assert_eq!(
            count(
                &c,
                // PDFs have one passage per page; EPUBs at least one piece.
                "SELECT COUNT(*) FROM documents d
                 JOIN file_instances f ON f.document_id = d.id
                 WHERE d.state = 'done' AND CASE f.file_type
                     WHEN 'pdf' THEN d.page_count IS NOT
                         (SELECT COUNT(*) FROM passages p WHERE p.document_id = d.id)
                     ELSE NOT EXISTS (SELECT 1 FROM passages p WHERE p.document_id = d.id) END"
            ),
            0,
            "every stored document has all its pages"
        );
        assert_eq!(
            count(
                &c,
                "SELECT COUNT(*) FROM documents d WHERE NOT EXISTS
                     (SELECT 1 FROM file_instances f WHERE f.document_id = d.id)"
            ),
            0,
            "no document without its file link"
        );
    }

    // Relaunch: the remembered library reopens and reading resumes.
    let (mut ws, h) = child_workspace(&dir, fast_extraction());
    ws.startup();
    assert!(
        wait_until(LONG, || settled(&ws)),
        "did not finish after restart"
    );
    let c = read_only(&dir).unwrap();
    let odd: Vec<(String, String, Option<String>, String)> = c
        .prepare(
            "SELECT display_name, extract_state, extract_error, availability FROM file_instances
             WHERE extract_state <> 'linked'",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(odd.is_empty(), "{odd:?}");
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM (SELECT content_hash FROM documents GROUP BY content_hash
                 HAVING COUNT(*) > 1)"
        ),
        0,
        "no duplicate documents"
    );
    let ok: String = c
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(ok, "ok");
    // The full-text index matches the passages (FTS5's own check; an error
    // means corruption).
    // FTS5 runs its check as an INSERT command, so through the writer.
    ws.require_open()
        .unwrap()
        .catalog()
        .call(|w| {
            w.execute_batch(
                "INSERT INTO passages_fts (passages_fts, rank) VALUES ('integrity-check', 1)",
            )
        })
        .unwrap()
        .expect("the full-text index is consistent after the kill");
    assert!(WorkerStats::get(&h.deps.stats.extracts) > 0);
    drop(ws);
    assert_eq!(originals(&dir.join("Books")), before);
}
