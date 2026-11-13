//! P0-08: a forced process kill mid-scan loses nothing and duplicates
//! nothing. The test re-runs this binary as a child process that scans a
//! throttled tree, kills it with SIGKILL (TerminateProcess on Windows), then
//! reopens the library in-process and lets the scan finish.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use atlas_lib::domain::dto::JobState;
use atlas_lib::services::inventory::{InventoryConfig, control};
use atlas_lib::services::sources;
use atlas_lib::services::workspace::{Workspace, WorkspaceConfig};

mod common;
use common::{InventoryHarness, fast_config, local, wait_until};

const CHILD_ENV: &str = "ATLAS_TEST_CRASH_DIR";
const FILES: usize = 3_000;

fn workspace(dir: &Path, config: InventoryConfig) -> (Workspace, InventoryHarness) {
    let h = InventoryHarness::new(config);
    let ws = Workspace::new(
        WorkspaceConfig {
            config_dir: dir.join("config"),
            default_root: dir.join("AtlasData"),
        },
        local(),
    )
    .with_inventory(h.deps.clone());
    (ws, h)
}

fn catalog_path(dir: &Path) -> PathBuf {
    dir.join("AtlasData/library/catalog.sqlite")
}

/// (rel_path → id) read through a separate read-only connection.
fn rows(dir: &Path) -> BTreeMap<Vec<u8>, String> {
    let Ok(conn) = rusqlite::Connection::open_with_flags(
        catalog_path(dir),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    ) else {
        return BTreeMap::new();
    };
    let Ok(mut stmt) = conn.prepare("SELECT rel_path, id FROM file_instances") else {
        return BTreeMap::new();
    };
    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .map(|it| it.filter_map(Result::ok).collect())
        .unwrap_or_default()
}

/// Child half: scan slowly until killed. Ignored unless run by the parent.
#[test]
#[ignore = "run only as the child of p0_08_kill_mid_scan_then_resume"]
fn p0_08_child_process() {
    let Ok(dir) = std::env::var(CHILD_ENV) else {
        return;
    };
    let dir = PathBuf::from(dir);
    let (mut ws, _h) = workspace(
        &dir,
        InventoryConfig {
            unthrottled: false,
            rate_override: Some(1_500),
            ..fast_config()
        },
    );
    ws.choose(&dir.join("AtlasData")).unwrap();
    let root = ws.require_open().unwrap();
    let added = sources::add(root, &dir.join("Books")).unwrap();
    control::enqueue(root, &added.id).unwrap();
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[test]
fn p0_08_kill_mid_scan_then_resume() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    for i in 0..FILES {
        let path = dir.join(format!("Books/shelf-{:02}/book-{i:05}.pdf", i % 30));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"%PDF").unwrap();
    }

    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["p0_08_child_process", "--exact", "--ignored", "--nocapture"])
        .env(CHILD_ENV, &dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let started = wait_until(Duration::from_secs(30), || rows(&dir).len() >= 300);
    child.kill().unwrap(); // SIGKILL: no destructors, no checkpoint
    child.wait().unwrap();
    assert!(started, "child never started cataloging");

    let before_resume = rows(&dir);
    assert!(
        before_resume.len() < FILES,
        "kill landed after the scan finished; slow the child down"
    );

    // Relaunch: the remembered library reopens and the scan resumes.
    let (mut ws, _h) = workspace(&dir, fast_config());
    ws.startup();
    let done = wait_until(Duration::from_secs(60), || {
        control::overview(ws.require_open().unwrap())
            .is_ok_and(|o| o.active.is_none() && o.roots[0].last_state == Some(JobState::Completed))
    });
    assert!(done, "scan did not complete after restart");

    let after = rows(&dir);
    assert_eq!(after.len(), FILES, "every file exactly once");
    for (rel, id) in &before_resume {
        assert_eq!(
            after.get(rel),
            Some(id),
            "IDs from before the kill are kept"
        );
    }
    let ok: String = ws
        .require_open()
        .unwrap()
        .catalog()
        .call(|c| {
            c.query_row("PRAGMA integrity_check", [], |r| r.get(0))
                .unwrap()
        })
        .unwrap();
    assert_eq!(ok, "ok");
}

const RESCAN_ENV: &str = "ATLAS_TEST_RESCAN_DIR";

/// Child half of the rescan kill: reopen the library and rescan slowly
/// until killed. Ignored unless run by the parent.
#[test]
#[ignore = "run only as the child of p4_11_kill_mid_rescan_then_resume"]
fn p4_11_child_process() {
    let Ok(dir) = std::env::var(RESCAN_ENV) else {
        return;
    };
    let dir = PathBuf::from(dir);
    let (mut ws, _h) = workspace(
        &dir,
        InventoryConfig {
            unthrottled: false,
            rate_override: Some(40),
            ..fast_config()
        },
    );
    ws.startup();
    let root = ws.require_open().unwrap();
    let folder = sources::list(root).unwrap().remove(0);
    control::rescan(root, &folder.id).unwrap();
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

/// Slice 4.5: a rescan killed part-way, with its unchanged work held in
/// memory, resumes and finds exactly what changed, counted once, with
/// every row's ID kept.
#[test]
fn p4_11_kill_mid_rescan_then_resume() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let book = |i: usize| dir.join(format!("Books/shelf-{:02}/book-{i:05}.pdf", i % 30));
    for i in 0..FILES {
        fs::create_dir_all(book(i).parent().unwrap()).unwrap();
        fs::write(book(i), b"%PDF").unwrap();
    }
    {
        let (mut ws, _h) = workspace(&dir, fast_config());
        ws.choose(&dir.join("AtlasData")).unwrap();
        let root = ws.require_open().unwrap();
        let added = sources::add(root, &dir.join("Books")).unwrap();
        control::enqueue(root, &added.id).unwrap();
        assert!(wait_until(Duration::from_secs(60), || {
            control::overview(ws.require_open().unwrap()).is_ok_and(|o| {
                o.active.is_none() && o.roots[0].last_state == Some(JobState::Completed)
            })
        }));
    }
    let before = rows(&dir);
    assert_eq!(before.len(), FILES);

    // Ten deleted (shelf 0), five changed (shelf 15), five new (shelf 29).
    for i in (0..300).step_by(30) {
        fs::remove_file(book(i)).unwrap();
    }
    for i in (15..165).step_by(30) {
        fs::write(book(i), b"%PDF-1.7 edited").unwrap();
    }
    for n in 0..5 {
        fs::write(dir.join(format!("Books/shelf-29/new-{n}.pdf")), b"%PDF").unwrap();
    }

    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["p4_11_child_process", "--exact", "--ignored", "--nocapture"])
        .env(RESCAN_ENV, &dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let rescan_running = || {
        rusqlite::Connection::open_with_flags(
            catalog_path(&dir),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .and_then(|c| {
            c.query_row(
                "SELECT COUNT(*) FROM jobs WHERE kind = 'scan' AND origin = 'user'
                   AND state = 'running'",
                [],
                |r| r.get::<_, i64>(0),
            )
        })
        .is_ok_and(|n| n == 1)
    };
    let started = wait_until(Duration::from_secs(30), rescan_running);
    std::thread::sleep(Duration::from_millis(1_500));
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(started, "child never started the rescan");
    assert!(
        rescan_running(),
        "kill landed after the rescan finished; slow the child down"
    );

    let (mut ws, _h) = workspace(&dir, fast_config());
    ws.startup();
    let done = wait_until(Duration::from_secs(60), || {
        control::overview(ws.require_open().unwrap())
            .is_ok_and(|o| o.active.is_none() && o.roots[0].last_state == Some(JobState::Completed))
    });
    assert!(done, "rescan did not complete after restart");

    let after = rows(&dir);
    assert_eq!(after.len(), FILES + 5, "the new files, each once");
    for (rel, id) in &before {
        assert_eq!(after.get(rel), Some(id), "IDs from before are kept");
    }
    let (missing, counts): (Vec<String>, (i64, i64, i64)) = ws
        .require_open()
        .unwrap()
        .catalog()
        .call(|c| {
            let mut stmt = c
                .prepare(
                    "SELECT display_name FROM file_instances WHERE availability = 'missing'
                     ORDER BY display_name",
                )
                .unwrap();
            let missing = stmt
                .query_map([], |r| r.get(0))
                .unwrap()
                .collect::<Result<Vec<String>, _>>()
                .unwrap();
            let counts = c
                .query_row(
                    // The rescan: the latest scan.
                    "SELECT new_files, changed_files, missing_files FROM jobs
                     WHERE kind = 'scan' ORDER BY rowid DESC LIMIT 1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .unwrap();
            (missing, counts)
        })
        .unwrap();
    let expected: Vec<String> = (0..300)
        .step_by(30)
        .map(|i| format!("book-{i:05}.pdf"))
        .collect();
    assert_eq!(missing, expected, "exactly the deleted files are missing");
    assert_eq!(
        counts,
        (5, 5, 10),
        "each change counted once across the kill"
    );
}
