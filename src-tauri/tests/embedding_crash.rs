//! P2-05 (crash half, ADR 0014): a kill while indexing meaning leaves
//! whole documents only, and relaunching finishes without sending any text
//! twice, with the index equal to the catalog.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use atlas_lib::domain::dto::JobKind;
use atlas_lib::services::embedding::chunk;
use atlas_lib::services::embedding::job as embedding_job;
use atlas_lib::services::extraction::db as extraction_db;
use atlas_lib::services::extraction::job as extraction_job;
use atlas_lib::services::workspace::{Workspace, WorkspaceConfig};
use atlas_lib::services::{inventory, sources};
use rusqlite::Connection;

mod common;
use common::embedding::{EmbedHarness, fast_embedding};
use common::vectors::{count, expected_keys};
use common::{local, wait_until};

const CHILD_ENV: &str = "ATLAS_TEST_EMBED_CRASH_DIR";
const LONG: Duration = Duration::from_secs(180);

fn workspace(dir: &Path, h: &EmbedHarness, embed: bool) -> Workspace {
    let ws = Workspace::new(
        WorkspaceConfig {
            config_dir: dir.join("config"),
            default_root: dir.join("AtlasData"),
        },
        local(),
    )
    .with_inventory(h.ex.inv.deps.clone())
    .with_job_processor(JobKind::Extract, extraction_job::factory(h.ex.deps.clone()));
    if embed {
        ws.with_job_processor(JobKind::Embed, embedding_job::factory(h.deps.clone()))
    } else {
        ws
    }
}

fn catalog(dir: &Path) -> Connection {
    Connection::open(dir.join("AtlasData/library/catalog.sqlite")).unwrap()
}

/// Reads a folder of books with no embedding job, then chooses an
/// embedding model, so the child does all the embedding.
fn prepare(dir: &Path) {
    let books = dir.join("Books");
    let out = Command::new("node")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/generate-fixtures.mjs"))
        .arg(&books)
        .args(["--files", "24", "--text-kb", "3"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let h = EmbedHarness::new(fast_embedding());
    let mut ws = workspace(dir, &h, false);
    ws.choose(&dir.join("AtlasData")).unwrap();
    let root = ws.require_open().unwrap();
    let added = sources::add(root, &books).unwrap();
    inventory::control::enqueue(root, &added.id).unwrap();
    assert!(wait_until(LONG, || {
        let inv = inventory::control::overview(root).unwrap();
        let ov = root
            .reader()
            .read(|c| extraction_db::overview(c, None))
            .unwrap()
            .unwrap();
        inv.active.is_none() && ov.active.is_none() && ov.counts.pending == 0
    }));
    root.catalog()
        .call(|c| {
            c.execute_batch(
                "INSERT INTO provider_profiles (id, kind, label, base_url,
                     insecure_http_acknowledged, created_at, updated_at)
                 VALUES ('svc', 'openai_compatible', 'Test', 'http://localhost:9/v1', 1, 't', 't');
                 INSERT INTO model_roles (role, provider_profile_id, model_id, dimensions,
                     tested_at, updated_at)
                 VALUES ('embedding', 'svc', 'test-embed', 64, 't', 't');",
            )
        })
        .unwrap()
        .unwrap();
}

/// SQLite's and FTS5's own consistency checks pass.
fn assert_consistent(dir: &Path) {
    let c = catalog(dir);
    let ok: String = c
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(ok, "ok");
    c.execute_batch("INSERT INTO passages_fts (passages_fts, rank) VALUES ('integrity-check', 1)")
        .expect("the full-text index matches the passages");
}

/// Every document marked embedded has all its chunks, each with a vector.
fn assert_whole_documents(c: &Connection) {
    assert_eq!(
        count(
            c,
            "SELECT COUNT(*) FROM document_embeddings de
             WHERE de.status = 'done' AND de.chunk_count <>
                 (SELECT COUNT(*) FROM chunks ch WHERE ch.document_id = de.document_id)"
        ),
        0,
        "every embedded document has all its chunks"
    );
    assert_eq!(
        count(
            c,
            "SELECT COUNT(*) FROM chunks ch WHERE NOT EXISTS
                 (SELECT 1 FROM embeddings e WHERE e.text_hash = ch.text_hash)"
        ),
        0,
        "no chunk without its vector"
    );
}

#[test]
fn p2_05_kill_mid_embedding_then_finish_without_sending_anything_twice() {
    let tmp = tempfile::tempdir().unwrap();
    let dir: PathBuf = tmp.path().to_path_buf();
    prepare(&dir);

    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["p2_05_child_process", "--exact", "--ignored", "--nocapture"])
        .env(CHILD_ENV, &dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let embedded = |dir: &Path| {
        count(
            &catalog(dir),
            "SELECT COUNT(*) FROM document_embeddings WHERE status = 'done'",
        )
    };
    assert!(
        wait_until(LONG, || embedded(&dir) >= 4),
        "the child never embedded"
    );
    child.kill().unwrap();
    child.wait().unwrap();

    let c = catalog(&dir);
    let total = count(&c, "SELECT COUNT(*) FROM documents WHERE state = 'done'");
    let at_kill = embedded(&dir);
    assert!(at_kill < total, "the kill landed after embedding finished");
    assert_whole_documents(&c);
    let had: BTreeSet<Vec<u8>> = c
        .prepare("SELECT text_hash FROM embeddings")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    drop(c);
    assert_consistent(&dir);

    // Relaunch: the job finishes, sending only what the kill left unsent.
    let h = EmbedHarness::new(fast_embedding());
    let mut ws = workspace(&dir, &h, true);
    ws.startup();
    assert!(
        wait_until(LONG, || embedded(&dir) == total),
        "did not finish after the relaunch"
    );
    let sent_again: Vec<String> = h
        .script
        .answered()
        .into_iter()
        .filter(|t| had.contains(&chunk::hash(t).to_vec()))
        .collect();
    assert!(
        sent_again.is_empty(),
        "{} chunks were sent twice",
        sent_again.len()
    );
    let c = catalog(&dir);
    assert_whole_documents(&c);
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM embeddings"),
        count(&c, "SELECT COUNT(DISTINCT text_hash) FROM chunks"),
        "no duplicate vectors"
    );
    let model: String = c
        .query_row("SELECT id FROM embedding_models", [], |r| r.get(0))
        .unwrap();
    let root = ws.require_open().unwrap();
    assert!(wait_until(LONG, || root.vectors().loaded(&model).is_some()));
    let expected = expected_keys(&c, &model);
    assert!(
        wait_until(LONG, || root.vectors().loaded(&model).unwrap().len()
            == expected.len()),
        "the reconciled index equals the catalog"
    );
    let index = root.vectors().loaded(&model).unwrap();
    for k in &expected {
        assert!(index.contains(*k));
    }
    drop(ws);
    assert_consistent(&dir);
}

/// Child half: embed slowly until killed.
#[test]
#[ignore = "run only as the child of p2_05_kill_mid_embedding_then_finish_without_sending_anything_twice"]
fn p2_05_child_process() {
    let Ok(dir) = std::env::var(CHILD_ENV) else {
        return;
    };
    let dir = PathBuf::from(dir);
    let mut config = fast_embedding();
    config.batch_size = 1;
    let h = EmbedHarness::new(config);
    h.script.with(|s| s.delay = Duration::from_millis(40));
    let mut ws = workspace(&dir, &h, true);
    ws.startup();
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}
