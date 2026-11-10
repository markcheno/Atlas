//! P2-08 (ADR 0013): a kill during an index save never loses the index.
//! usearch writes its file in place, and in the Slice 0.9 spike a kill
//! during a save left an unloadable file once in four. Atlas saves to
//! `tmp/`, renames, and records the generation afterwards, so whatever a
//! kill interrupts, the next open loads the recorded file or rebuilds.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use atlas_lib::services::embedding::db as edb;
use atlas_lib::services::embedding::index::{Opened, open_index};
use atlas_lib::services::embedding::vector::to_f16_blob;
use rusqlite::params;

mod common;
use common::vectors::{Lib, assert_matches_catalog, count};
use common::wait_until;

const CHILD_ENV: &str = "ATLAS_TEST_VECTORS_CRASH_ROOT";
const MODEL_ENV: &str = "ATLAS_TEST_VECTORS_CRASH_MODEL";
const DIMS: usize = 128;
const DOCS: usize = 100;
const CHUNKS_PER_DOC: usize = 300;
const KILLS: u64 = 8;

/// 30,000 stored chunks with vectors, written directly (the test is about
/// the index, not chunking).
fn bulk(lib: &Lib, model: &str) {
    let model = model.to_owned();
    lib.writer
        .call(move |c| {
            let tx = c.transaction().unwrap();
            let mut x: u64 = 0x2545_F491_4F6C_DD1D;
            for d in 0..DOCS {
                let doc = format!("doc-{d}");
                tx.execute(
                    "INSERT INTO documents (id, content_hash, state, created_at, updated_at)
                     VALUES (?1, ?2, 'done', 't', 't')",
                    params![doc, blake3::hash(doc.as_bytes()).as_bytes().to_vec()],
                )
                .unwrap();
                tx.execute(
                    "INSERT INTO passages (document_id, ordinal, page_no, location, text)
                     VALUES (?1, 0, 1, 'Page 1', 'text')",
                    [&doc],
                )
                .unwrap();
                let passage = tx.last_insert_rowid();
                for i in 0..CHUNKS_PER_DOC {
                    let hash = blake3::hash(format!("{d}/{i}").as_bytes());
                    let mut v: Vec<f32> = (0..DIMS)
                        .map(|_| {
                            x ^= x << 13;
                            x ^= x >> 7;
                            x ^= x << 17;
                            (x % 2001) as f32 - 1000.0
                        })
                        .collect();
                    let n = v.iter().map(|a| a * a).sum::<f32>().sqrt();
                    v.iter_mut().for_each(|a| *a /= n);
                    tx.execute(
                        "INSERT INTO chunks (document_id, passage_id, ordinal, char_start, char_end, text_hash)
                         VALUES (?1, ?2, ?3, 0, 4, ?4)",
                        params![doc, passage, i as i64, &hash.as_bytes()[..]],
                    )
                    .unwrap();
                    tx.execute(
                        "INSERT INTO embeddings (model_id, text_hash, vector) VALUES (?1, ?2, ?3)",
                        params![model, &hash.as_bytes()[..], to_f16_blob(&v)],
                    )
                    .unwrap();
                }
                tx.execute(
                    "INSERT INTO document_embeddings (document_id, model_id, seq, status, chunk_count, updated_at)
                     VALUES (?1, ?2, ?3, 'done', ?4, 't')",
                    params![doc, model, d as i64 + 1, CHUNKS_PER_DOC as i64],
                )
                .unwrap();
            }
            tx.commit().unwrap();
        })
        .unwrap();
}

fn generation(lib: &Lib, model: &str) -> i64 {
    edb::get_model(&lib.conn(), model)
        .unwrap()
        .unwrap()
        .index_generation
}

/// SQLite's and FTS5's own consistency checks pass.
fn assert_catalog_consistent(lib: &Lib) {
    lib.writer
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

#[test]
fn p2_08_kill_during_save_never_loses_the_index() {
    let lib = Lib::new();
    let model = lib.create_model(DIMS);
    bulk(&lib, &model);
    let (index, _) = open_index(&lib.paths, &lib.writer, &model, 4).unwrap();
    assert_eq!(index.len(), DOCS * CHUNKS_PER_DOC);
    drop(index);
    let root: PathBuf = lib.root.clone();
    let mut lib = lib;
    // The folder outlives each reopened handle.
    let _keep = lib.dir.take();

    let mut outcomes = Vec::new();
    for kill in 0..KILLS {
        let before = generation(&lib, &model);
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["p2_08_child_process", "--exact", "--ignored", "--nocapture"])
            .env(CHILD_ENV, &root)
            .env(MODEL_ENV, &model)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        // Once the child is saving in a loop, kill it at a varying moment.
        assert!(
            wait_until(Duration::from_secs(120), || generation(&lib, &model)
                > before),
            "the child never saved"
        );
        std::thread::sleep(Duration::from_millis(7 + kill * 23));
        child.kill().unwrap();
        child.wait().unwrap();

        // What the kill left, then what the next open makes of it.
        let staged = fs::read_dir(&lib.paths.tmp).unwrap().count();
        let files = fs::read_dir(&lib.paths.vectors).unwrap().count();
        let (index, report) = open_index(&lib.paths, &lib.writer, &model, 4).unwrap();
        assert_matches_catalog(&lib, &index, &model, DIMS);
        assert_eq!(
            fs::read_dir(&lib.paths.vectors).unwrap().count(),
            1,
            "only the recorded generation is kept"
        );
        assert_catalog_consistent(&lib);
        outcomes.push((
            staged,
            files,
            matches!(report.opened, Opened::Loaded { .. }),
        ));
        drop(index);
        // Opening a library empties tmp/ (workspace open).
        for e in fs::read_dir(&lib.paths.tmp).unwrap() {
            fs::remove_file(e.unwrap().path()).unwrap();
        }
        lib = Lib::open(&root);
    }
    let c = lib.conn();
    assert_eq!(count(&c, "SELECT COUNT(*) FROM vector_tombstones"), 0);
    println!("(staged files, index files, loaded) after each kill: {outcomes:?}");
}

/// Child half: save the index over and over until killed.
#[test]
#[ignore = "run only as the child of p2_08_kill_during_save_never_loses_the_index"]
fn p2_08_child_process() {
    let root = PathBuf::from(std::env::var(CHILD_ENV).expect("the parent sets the root"));
    let model = std::env::var(MODEL_ENV).unwrap();
    let lib = Lib::open(&root);
    let (index, _) = open_index(&lib.paths, &lib.writer, &model, 2).unwrap();
    let until = Instant::now() + Duration::from_secs(300);
    while Instant::now() < until {
        index.save(&lib.paths, &lib.writer).unwrap();
    }
}
