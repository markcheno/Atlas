//! Slice 1.3: extraction survives misbehaving workers and a killed app
//! (phase1-spec §7.2, §12; tests P1-11 and P1-12).
//!
//! P1-11 drives the real worker into crashes with its debug-only fault
//! hook. P1-12 re-runs this test binary as a child that reads a library,
//! kills it with SIGKILL (TerminateProcess on Windows) mid-way, then
//! reopens the library here and lets reading finish. P1-22 kills the same
//! kind of child while its worker is reading (P1-12) or hung, and checks
//! that the worker does not outlive it.

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
/// A worker must be gone this soon after its app is killed. Its watchdog
/// polls every 250 ms; the rest is margin for slow runners.
const WORKER_GONE: Duration = Duration::from_secs(5);

/// The extraction workers `parent` started.
fn workers_of(parent: u32) -> Vec<sysinfo::Pid> {
    let mut sys = sysinfo::System::new().unwrap();
    sys.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        sysinfo::ProcessRefreshKind::nothing(),
    );
    sys.processes()
        .iter()
        // On Linux sysinfo lists threads too, with their process as parent.
        .filter(|(_, p)| {
            p.parent() == Some(sysinfo::Pid::from_u32(parent)) && p.thread_kind().is_none()
        })
        .map(|(pid, _)| *pid)
        .collect()
}

/// Whether `pid` is still running (a zombie waiting to be reaped is not).
fn running(pid: sysinfo::Pid) -> bool {
    let mut sys = sysinfo::System::new().unwrap();
    sys.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&[pid]),
        true,
        sysinfo::ProcessRefreshKind::nothing(),
    );
    sys.process(pid)
        .is_some_and(|p| p.status() != sysinfo::ProcessStatus::Zombie)
}

/// Kills the child the way a crash or force quit does, and checks its
/// workers end too (P1-22).
fn kill_and_check_workers(child: &mut std::process::Child, workers: &[sysinfo::Pid]) {
    child.kill().unwrap(); // SIGKILL / TerminateProcess: no destructors
    child.wait().unwrap();
    let gone = wait_until(WORKER_GONE, || workers.iter().all(|pid| !running(*pid)));
    let left: Vec<_> = workers.iter().filter(|pid| running(**pid)).collect();
    // Never leave a stray process behind, even when the check fails.
    for pid in &left {
        let mut sys = sysinfo::System::new().unwrap();
        sys.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&[**pid]),
            true,
            sysinfo::ProcessRefreshKind::nothing(),
        );
        if let Some(p) = sys.process(**pid) {
            p.kill();
        }
    }
    assert!(gone, "workers outlived the app: {left:?}");
}

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
    assert_catalog_consistent(&ws);
    assert_eq!(originals(&books), before);
}

fn child_workspace(dir: &Path, config: ExtractionConfig) -> (Workspace, ExtractionHarness) {
    child_workspace_with(dir, config, worker())
}

fn child_workspace_with(
    dir: &Path,
    config: ExtractionConfig,
    command: atlas_lib::services::extraction::supervisor::WorkerCommand,
) -> (Workspace, ExtractionHarness) {
    let h = ExtractionHarness::new(config, command);
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

/// SQLite's and FTS5's own consistency checks pass (phase1-spec §12: after
/// every crash test).
fn assert_catalog_consistent(ws: &Workspace) {
    // Both run through the writer: FTS5 runs its check as an INSERT command.
    ws.require_open()
        .unwrap()
        .catalog()
        .call(|w| {
            let ok: String = w.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
            assert_eq!(ok, "ok");
            w.execute_batch(
                "INSERT INTO passages_fts (passages_fts, rank) VALUES ('integrity-check', 1)",
            )
        })
        .unwrap()
        .expect("the full-text index matches the passages");
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
    let workers = workers_of(child.id());
    assert!(started, "the child never started reading");
    assert!(!workers.is_empty(), "the child's worker is running");
    // SIGKILL: no destructors, no checkpoint. P1-22: the worker, busy
    // reading, ends with the app.
    kill_and_check_workers(&mut child, &workers);

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
    assert_catalog_consistent(&ws);
    assert!(WorkerStats::get(&h.deps.stats.extracts) > 0);
    drop(ws);
    assert_eq!(originals(&dir.join("Books")), before);
}

/// Child half of P1-22: read a file the worker hangs on, until killed.
#[test]
#[ignore = "run only as the child of p1_22_a_hung_worker_does_not_outlive_a_killed_app"]
fn p1_22_child_process() {
    let Ok(dir) = std::env::var(CHILD_ENV) else {
        return;
    };
    let dir = PathBuf::from(dir);
    let (mut ws, _h) = child_workspace_with(
        &dir,
        ExtractionConfig {
            // Long enough that the app never kills the hung worker itself.
            supervisor: SupervisorConfig {
                inactivity: Duration::from_secs(600),
                ..SupervisorConfig::default()
            },
            ..fast_extraction()
        },
        worker().env(TEST_FAULT_ENV, "hang-here:hang"),
    );
    ws.choose(&dir.join("AtlasData")).unwrap();
    let root = ws.require_open().unwrap();
    let added = sources::add(root, &dir.join("Books")).unwrap();
    inventory::control::enqueue(root, &added.id).unwrap();
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// P1-22: a worker stuck inside a request, which would never read its
/// closed pipe, still ends when the app is killed; relaunching reads the
/// file again.
#[test]
fn p1_22_a_hung_worker_does_not_outlive_a_killed_app() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let books = dir.join("Books");
    fs::create_dir_all(&books).unwrap();
    fs::copy(
        corpus("pdf/generated/book.pdf"),
        books.join("hang-here.pdf"),
    )
    .unwrap();

    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["p1_22_child_process", "--exact", "--ignored", "--nocapture"])
        .env(CHILD_ENV, &dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // The worker appears, hashes the file, then hangs extracting it.
    let mut workers = Vec::new();
    let seen = wait_until(Duration::from_secs(60), || {
        workers = workers_of(child.id());
        !workers.is_empty()
    });
    if !seen {
        let _ = child.kill();
        let _ = child.wait();
        panic!("the child never started a worker");
    }
    std::thread::sleep(Duration::from_secs(2));
    assert!(
        workers.iter().all(|pid| running(*pid)),
        "still hung, not crashed"
    );
    kill_and_check_workers(&mut child, &workers);

    // Relaunch without the fault: the file is read.
    let (mut ws, _h) = child_workspace(&dir, fast_extraction());
    ws.startup();
    assert!(
        wait_until(LONG, || settled(&ws)),
        "did not finish after restart"
    );
    assert_eq!(
        state_of(&ws, &books.join("hang-here.pdf")).0,
        ExtractionState::Done
    );
    assert_catalog_consistent(&ws);
}
