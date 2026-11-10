//! Slice 2.3: a catalog created before Slice 1.7 (4 KiB pages) gets 16 KiB
//! pages before its first embedding (ADR 0014), keeping everything in it;
//! without room it stays as it is; a kill mid-conversion loses nothing.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use atlas_lib::domain::dto::{SearchQuery, VolumeKind};
use atlas_lib::services::workspace::{Workspace, WorkspaceConfig};
use atlas_lib::services::{inventory, search, sources};
use rusqlite::Connection;

mod common;
use common::{
    Env, ExtractionHarness, FakeVolumes, GIB, fast_extraction, local, wait_until, worker,
};

const CHILD_ENV: &str = "ATLAS_TEST_CONVERT_DIR";
const LONG: Duration = Duration::from_secs(120);

fn catalog_path(dir: &Path) -> PathBuf {
    dir.join("AtlasData/library/catalog.sqlite")
}

fn page_size(dir: &Path) -> u32 {
    Connection::open(catalog_path(dir))
        .unwrap()
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .unwrap()
}

fn workspace(dir: &Path, h: &ExtractionHarness, volumes: Arc<FakeVolumes>) -> Workspace {
    Workspace::new(
        WorkspaceConfig {
            config_dir: dir.join("config"),
            default_root: dir.join("AtlasData"),
        },
        volumes,
    )
    .with_inventory(h.inv.deps.clone())
    .with_job_processor(
        atlas_lib::domain::dto::JobKind::Extract,
        atlas_lib::services::extraction::job::factory(h.deps.clone()),
    )
}

/// A read library whose catalog is then rewritten with 4 KiB pages, as one
/// created before Slice 1.7, with an embedding model chosen. `filler_mb`
/// makes the conversion take a while.
fn old_library(dir: &Path, filler_mb: usize) {
    let books = dir.join("Books");
    let out = Command::new("node")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/generate-fixtures.mjs"))
        .arg(&books)
        .args(["--files", "8", "--text-kb", "3"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let h = ExtractionHarness::new(fast_extraction(), worker());
    let mut ws = workspace(dir, &h, local());
    ws.choose(&dir.join("AtlasData")).unwrap();
    let root = ws.require_open().unwrap();
    let added = sources::add(root, &books).unwrap();
    inventory::control::enqueue(root, &added.id).unwrap();
    assert!(wait_until(LONG, || {
        let c = Connection::open(catalog_path(dir)).unwrap();
        let pending: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM file_instances WHERE extract_state = 'pending'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let jobs: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM jobs WHERE state NOT IN ('completed', 'cancelled')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        pending == 0 && jobs == 0
    }));
    drop(ws);

    let c = Connection::open(catalog_path(dir)).unwrap();
    c.execute_batch(&format!(
        "PRAGMA journal_mode = DELETE;
         PRAGMA page_size = 4096;
         VACUUM;
         PRAGMA journal_mode = WAL;
         INSERT INTO provider_profiles (id, kind, label, base_url, insecure_http_acknowledged,
             created_at, updated_at)
         VALUES ('svc', 'openai_compatible', 'Test', 'http://localhost:9/v1', 1, 't', 't');
         INSERT INTO model_roles (role, provider_profile_id, model_id, dimensions, tested_at,
             updated_at)
         VALUES ('embedding', 'svc', 'test-embed', 64, 't', 't');
         INSERT OR REPLACE INTO sqlite_sequence (name, seq) VALUES ('chunks', 1234);
         CREATE TABLE filler (b BLOB);
         WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {filler_mb})
         INSERT INTO filler SELECT randomblob(1048576) FROM n;"
    ))
    .unwrap();
    drop(c);
    assert_eq!(page_size(dir), 4096);
}

fn snapshot(dir: &Path) -> (Vec<(String, String)>, i64, i64) {
    let c = Connection::open(catalog_path(dir)).unwrap();
    let docs: Vec<(String, String)> = c
        .prepare("SELECT id, state FROM documents ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let passages: i64 = c
        .query_row("SELECT COUNT(*) FROM passages", [], |r| r.get(0))
        .unwrap();
    let seq: i64 = c
        .query_row(
            "SELECT seq FROM sqlite_sequence WHERE name = 'chunks'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    (docs, passages, seq)
}

fn hits(ws: &Workspace) -> Vec<String> {
    let root = ws.require_open().unwrap();
    search::run(
        root.reader(),
        root.root_status(),
        &SearchQuery {
            text: "the".into(),
            source_root_id: None,
            file_type: None,
            include_missing: false,
            page: 0,
            page_size: 50,
        },
    )
    .unwrap()
    .hits
    .into_iter()
    .filter_map(|h| h.document_id)
    .collect()
}

#[test]
fn an_old_catalog_gets_16_kib_pages_and_keeps_everything() {
    let env = Env::new();
    let dir = env.base.clone();
    old_library(&dir, 0);
    let before = snapshot(&dir);

    let h = ExtractionHarness::new(fast_extraction(), worker());
    let mut ws = workspace(&dir, &h, local());
    ws.startup();
    assert!(ws.require_open().is_ok());
    assert_eq!(page_size(&dir), 16_384);
    assert_eq!(
        snapshot(&dir),
        before,
        "documents, text and chunk ids survive"
    );
    let found = hits(&ws);
    assert!(!found.is_empty(), "search still works");
    let c = Connection::open(catalog_path(&dir)).unwrap();
    let mode: String = c
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    let ok: String = c
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(ok, "ok");
    assert!(!dir.join("AtlasData/tmp/catalog-16k.sqlite").exists());

    // Opening again changes nothing more.
    ws.reopen();
    assert_eq!(page_size(&dir), 16_384);
    assert_eq!(hits(&ws), found);
}

#[test]
fn without_room_for_the_copy_the_catalog_stays_as_it_is() {
    let env = Env::new();
    let dir = env.base.clone();
    old_library(&dir, 0);
    let h = ExtractionHarness::new(fast_extraction(), worker());
    let full = Arc::new(FakeVolumes {
        kind: VolumeKind::Local,
        free: 1024,
        total: 500 * GIB,
    });
    let mut ws = workspace(&dir, &h, full);
    ws.startup();
    assert!(ws.require_open().is_ok(), "the library still opens");
    assert_eq!(page_size(&dir), 4096);
}

#[test]
fn a_kill_mid_conversion_loses_nothing() {
    let env = Env::new();
    let dir = env.base.clone();
    old_library(&dir, 150);
    let before = snapshot(&dir);
    let mut outcomes = Vec::new();
    for delay_ms in [30, 120, 400] {
        if page_size(&dir) == 16_384 {
            break;
        }
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "convert_child_process",
                "--exact",
                "--ignored",
                "--nocapture",
            ])
            .env(CHILD_ENV, &dir)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(delay_ms));
        child.kill().unwrap();
        child.wait().unwrap();
        outcomes.push((delay_ms, page_size(&dir)));
    }
    // Whatever the kill interrupted, the next open finds a whole catalog.
    let h = ExtractionHarness::new(fast_extraction(), worker());
    let mut ws = workspace(&dir, &h, local());
    ws.startup();
    assert!(ws.require_open().is_ok());
    assert_eq!(page_size(&dir), 16_384);
    assert_eq!(snapshot(&dir), before);
    let c = Connection::open(catalog_path(&dir)).unwrap();
    let ok: String = c
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(ok, "ok");
    c.execute_batch("INSERT INTO passages_fts (passages_fts, rank) VALUES ('integrity-check', 1)")
        .expect("the full-text index matches the passages");
    println!("(kill after ms, page size left) {outcomes:?}");
}

/// Child half: open the library (which converts it) and wait to be killed.
#[test]
#[ignore = "run only as the child of a_kill_mid_conversion_loses_nothing"]
fn convert_child_process() {
    let Ok(dir) = std::env::var(CHILD_ENV) else {
        return;
    };
    let dir = PathBuf::from(dir);
    let h = ExtractionHarness::new(fast_extraction(), worker());
    let mut ws = workspace(&dir, &h, local());
    ws.startup();
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}
