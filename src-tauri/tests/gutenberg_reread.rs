//! Slice 3.9, phase3-spec §13 ("the carry-over"): what re-reading the
//! corpus's Project Gutenberg books by reader version 3 changes, and how
//! much is embedded again.
//!
//! The books are first read as reader version 2 did, header and licence
//! included (the worker's debug-only `ATLAS_WORKER_TEST_KEEP_BOILERPLATE`),
//! and embedded with the scripted backend. They are then set back to
//! version 2, the library is opened with an ordinary worker, and they are
//! read again and embedded again (ADR 0011: only chunks whose text changed
//! are sent). The numbers are printed as one JSON line for
//! docs/benchmarks.md.

use std::fs;
use std::path::{Path, PathBuf};

use atlas_lib::services::embedding::triggers;
use atlas_lib::services::extraction::db;
use atlas_lib::services::extraction::engine::gutenberg::TEST_KEEP_ENV;
use atlas_lib::services::workspace::Workspace;
use atlas_lib::services::{inventory, sources};
use rusqlite::Connection;

mod common;
use common::embedding::{EmbedHarness, fast_embedding};
use common::{Env, ExtractionHarness, fast_extraction, wait_until, worker};

const LONG: std::time::Duration = std::time::Duration::from_secs(300);

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../spikes/extraction-bench/corpus/epub")
}

fn catalog(env: &Env) -> Connection {
    Connection::open(env.base.join("AtlasData/library/catalog.sqlite")).unwrap()
}

fn count(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn has_boilerplate(text: &str) -> bool {
    let t = text.to_lowercase();
    [
        "start of the project gutenberg",
        "end of the project gutenberg",
        "project gutenberg license",
        "literary archive foundation",
    ]
    .iter()
    .any(|m| t.contains(m))
}

fn all_text(c: &Connection) -> String {
    c.prepare("SELECT text FROM passages ORDER BY document_id, ordinal")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Reading done, and every read document embedded.
fn settled(ws: &Workspace) -> bool {
    let root = ws.require_open().unwrap();
    let inv = inventory::control::overview(root).unwrap();
    let ov = root
        .reader()
        .read(|c| db::overview(c, None))
        .unwrap()
        .unwrap();
    if inv.active.is_some() || ov.active.is_some() || ov.queued_jobs > 0 || ov.counts.pending > 0 {
        return false;
    }
    root.reader()
        .read(|c| {
            c.query_row(
                "SELECT COUNT(*) FROM documents d WHERE d.state = 'done' AND (d.refresh = 1 OR NOT EXISTS
                     (SELECT 1 FROM document_embeddings de WHERE de.document_id = d.id))",
                [],
                |r| r.get::<_, i64>(0),
            )
        })
        .unwrap()
        .unwrap()
        == 0
}

fn harness(keep_boilerplate: bool) -> EmbedHarness {
    let mut h = EmbedHarness::new(fast_embedding());
    let command = if keep_boilerplate {
        worker().env(TEST_KEEP_ENV, "1")
    } else {
        worker()
    };
    h.ex = ExtractionHarness::new(fast_extraction(), command);
    h
}

fn set_role(ws: &Workspace) {
    let root = ws.require_open().unwrap();
    root.catalog()
        .call(|c| {
            c.execute_batch(
                "INSERT OR IGNORE INTO provider_profiles (id, kind, label, base_url,
                     insecure_http_acknowledged, created_at, updated_at)
                 VALUES ('svc', 'openai_compatible', 'Test', 'http://localhost:9/v1', 1, 't', 't');
                 INSERT OR IGNORE INTO model_roles (role, provider_profile_id, model_id,
                     dimensions, tested_at, updated_at)
                 VALUES ('embedding', 'svc', 'test-embed', 64, 't', 't');",
            )
        })
        .unwrap()
        .unwrap();
    triggers::changed(root).unwrap();
}

#[test]
fn the_gutenberg_re_read_changes_only_what_it_cut() {
    let env = Env::new();
    let books = env.base.join("Books");
    fs::create_dir_all(&books).unwrap();
    let mut names = Vec::new();
    for e in fs::read_dir(corpus()).unwrap().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with("pg") && name.ends_with(".epub") {
            fs::copy(e.path(), books.join(&name)).unwrap();
            names.push(name);
        }
    }
    names.sort();
    assert_eq!(
        names.len(),
        5,
        "the corpus's Project Gutenberg books: {names:?}"
    );

    // Read as reader version 2 did, then embedded.
    let first = harness(true);
    let mut ws = env.workspace_with_embedding(&first);
    ws.choose(&env.base.join("AtlasData")).unwrap();
    let added = sources::add(ws.require_open().unwrap(), &books).unwrap();
    inventory::control::enqueue(ws.require_open().unwrap(), &added.id).unwrap();
    set_role(&ws);
    assert!(
        wait_until(LONG, || settled(&ws)),
        "the first read never settled"
    );
    drop(ws);
    let c = catalog(&env);
    assert!(
        has_boilerplate(&all_text(&c)),
        "read with the header and licence"
    );
    let passages_before = count(&c, "SELECT COUNT(*) FROM passages");
    let chars_before = count(&c, "SELECT COALESCE(SUM(length(text)), 0) FROM passages");
    let chunks_before = count(&c, "SELECT COUNT(*) FROM chunks");
    let embedded_first = first.script.answered().len();

    // As reader version 2 left them; then opened with an ordinary worker.
    c.execute(
        "UPDATE documents SET extractor_version = 2, boilerplate_chars = 0",
        [],
    )
    .unwrap();
    drop(c);
    let second = harness(false);
    let mut ws = env.workspace_with_embedding(&second);
    ws.startup();
    assert!(
        wait_until(LONG, || settled(&ws)),
        "the re-read never settled"
    );
    let c = catalog(&env);
    let text = all_text(&c);
    assert!(!has_boilerplate(&text), "the header and licence are gone");
    let cut = count(
        &c,
        "SELECT COALESCE(SUM(boilerplate_chars), 0) FROM documents",
    );
    let cut_books = count(
        &c,
        "SELECT COUNT(*) FROM documents WHERE boilerplate_chars > 0",
    );
    let passages_after = count(&c, "SELECT COUNT(*) FROM passages");
    let chars_after = count(&c, "SELECT COALESCE(SUM(length(text)), 0) FROM passages");
    let chunks_after = count(&c, "SELECT COUNT(*) FROM chunks");
    let embedded_again = second.script.answered().len() as i64;
    assert_eq!(cut_books, 5, "every book had something cut");
    assert!(passages_after <= passages_before);
    assert!(chars_after < chars_before);
    assert!(
        embedded_again < chunks_after,
        "unchanged chunks keep their vectors: {embedded_again} of {chunks_after} sent again"
    );
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM documents d WHERE NOT EXISTS
                 (SELECT 1 FROM document_embeddings de WHERE de.document_id = d.id)"
        ),
        0,
        "every book is embedded again"
    );
    drop(ws);

    println!(
        "gutenberg-reread {}",
        serde_json::json!({
            "books": names.len(),
            "passagesBefore": passages_before,
            "passagesAfter": passages_after,
            "charsBefore": chars_before,
            "charsAfter": chars_after,
            "charsCut": cut,
            "chunksBefore": chunks_before,
            "chunksAfter": chunks_after,
            "embeddedFirst": embedded_first,
            "embeddedAgain": embedded_again,
            "chunksKept": chunks_after - embedded_again,
        })
    );
}
