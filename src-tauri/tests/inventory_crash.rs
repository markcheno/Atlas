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
