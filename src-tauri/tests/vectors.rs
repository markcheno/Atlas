//! Slice 2.2: chunks, durable embeddings and the disposable vector index
//! (phase2-spec §6.1, §7.3; ADR 0013).
//!
//! - P2-04: chunks map back to their passages; an unchanged re-read keeps
//!   the hashes and re-requests nothing; a changed one re-queues the
//!   document and tombstones its chunks.
//! - P2-08: a missing, corrupted, mismatched or stale index file is rebuilt
//!   or reconciled from the catalog, and searches equal a fresh rebuild's.

use std::fs;

use atlas_lib::services::embedding::chunk::{self, Span};
use atlas_lib::services::embedding::db as edb;
use atlas_lib::services::embedding::fake::HashedBagOfWordsBackend;
use atlas_lib::services::embedding::index::{Opened, RebuildReason, force_buffer_io, open_index};

mod common;
use common::vectors::{Lib, assert_matches_catalog, count, ids, prose};

const DIMS: usize = 64;

/// A library with a few embedded documents and a saved index.
fn embedded(lib: &Lib, backend: &HashedBagOfWordsBackend) -> (String, Vec<String>) {
    let model = lib.create_model(DIMS);
    let docs: Vec<String> = (0..4)
        .map(|i| {
            let pages = [
                prose(i * 10, 900),
                prose(i * 10 + 1, 40),
                prose(i * 10 + 2, 600),
            ];
            let pages: Vec<&str> = pages.iter().map(String::as_str).collect();
            lib.read(
                &format!("book {i}.pdf"),
                format!("file {i}").as_bytes(),
                &pages,
            )
        })
        .collect();
    for d in &docs {
        lib.embed(backend, &model, d);
    }
    (model, docs)
}

#[test]
fn p2_04_chunks_map_back_to_their_pages() {
    let lib = Lib::new();
    let backend = HashedBagOfWordsBackend::new(DIMS);
    let model = lib.create_model(DIMS);
    let long = prose(1, 1500);
    let doc = lib.read("a.pdf", b"a", &[&long, "A short page.", "   "]);
    let stored = lib.embed(&backend, &model, &doc);
    assert!(stored.len() >= 8, "{} chunks", stored.len());

    let c = lib.conn();
    let mut stmt = c
        .prepare(
            "SELECT c.char_start, c.char_end, c.text_hash, p.text, p.page_no, p.location
             FROM chunks c JOIN passages p ON p.id = c.passage_id
             WHERE c.document_id = ?1 ORDER BY c.ordinal",
        )
        .unwrap();
    let rows: Vec<(usize, usize, Vec<u8>, String, i64, String)> = stmt
        .query_map([&doc], |r| {
            Ok((
                r.get::<_, i64>(0)? as usize,
                r.get::<_, i64>(1)? as usize,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(rows.len(), stored.len());
    for (start, end, hash, text, page, location) in &rows {
        let t = chunk::slice(
            text,
            Span {
                start: *start,
                end: *end,
            },
        )
        .unwrap();
        assert_eq!(
            &chunk::hash(t)[..],
            &hash[..],
            "the hash is the chunk's text"
        );
        assert!(t.chars().count() <= chunk::MAX_CHARS);
        assert_eq!(location, &format!("Page {page}"), "a chunk keeps its page");
    }
    assert_eq!(rows.last().unwrap().4, 2, "the blank page has no chunk");
    assert_eq!(
        count(
            &c,
            "SELECT chunk_count FROM document_embeddings WHERE status = 'done'"
        ),
        stored.len() as i64
    );
}

#[test]
fn p2_04_unchanged_text_is_never_sent_again_and_changed_text_is_requeued() {
    let lib = Lib::new();
    let backend = HashedBagOfWordsBackend::new(DIMS);
    let model = lib.create_model(DIMS);
    let (p1, p2) = (prose(1, 700), prose(2, 700));
    let doc = lib.read("a.pdf", b"a", &[&p1, &p2]);
    let first = lib.embed(&backend, &model, &doc);
    let sent = backend.counts().1;
    assert_eq!(sent, first.len());

    // Read again with the same text (e.g. a newer reader): the document
    // waits again, its old chunks leave tombstones, and re-embedding
    // requests nothing.
    assert_eq!(lib.read("a.pdf", b"a", &[&p1, &p2]), doc, "same document");
    let c = lib.conn();
    assert_eq!(count(&c, "SELECT COUNT(*) FROM document_embeddings"), 0);
    let tombs = ids(&c, "SELECT chunk_id FROM vector_tombstones");
    assert_eq!(tombs, first.iter().map(|s| s.id).collect());
    let again = lib.embed(&backend, &model, &doc);
    assert_eq!(backend.counts().1, sent, "no chunk was sent twice");
    assert_eq!(
        again.iter().map(|s| s.hash).collect::<Vec<_>>(),
        first.iter().map(|s| s.hash).collect::<Vec<_>>(),
        "stable hashes"
    );
    assert!(
        again.iter().all(|s| s.id > first.last().unwrap().id),
        "new keys"
    );

    // Change the second page: only its chunks are sent.
    let changed = prose(3, 700);
    lib.read("a.pdf", b"a", &[&p1, &changed]);
    let third = lib.embed(&backend, &model, &doc);
    let new_hashes = chunk::chunk(&changed).len();
    assert_eq!(backend.counts().1, sent + new_hashes);
    assert_eq!(third.len(), chunk::chunk(&p1).len() + new_hashes);
}

#[test]
fn p2_04_identical_text_in_two_files_is_embedded_once() {
    let lib = Lib::new();
    let backend = HashedBagOfWordsBackend::new(DIMS);
    let model = lib.create_model(DIMS);
    let shared = prose(9, 500);
    let a = lib.read("a.pdf", b"a", &[&shared, &prose(10, 100)]);
    let b = lib.read("b.pdf", b"b", &[&shared, &prose(11, 100)]);
    lib.embed(&backend, &model, &a);
    let before = backend.counts().1;
    lib.embed(&backend, &model, &b);
    assert_eq!(backend.counts().1 - before, 1, "only b's own page");
    let c = lib.conn();
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM embeddings"),
        count(&c, "SELECT COUNT(DISTINCT text_hash) FROM chunks")
    );
}

#[test]
fn p2_08_an_index_is_built_saved_and_loaded() {
    let lib = Lib::new();
    let backend = HashedBagOfWordsBackend::new(DIMS);
    let (model, _) = embedded(&lib, &backend);

    let (index, report) = open_index(&lib.paths, &lib.writer, &model, 4).unwrap();
    assert_eq!(
        report.opened,
        Opened::Rebuilt {
            reason: RebuildReason::NeverSaved
        }
    );
    assert_matches_catalog(&lib, &index, &model, DIMS);
    assert!(lib.paths.file(&model, 1).exists(), "a rebuild is saved");
    drop(index);

    let (index, report) = open_index(&lib.paths, &lib.writer, &model, 4).unwrap();
    assert_eq!(
        report.opened,
        Opened::Loaded {
            added: 0,
            removed: 0
        }
    );
    assert_matches_catalog(&lib, &index, &model, DIMS);

    // A second save is the next generation; the first file goes.
    index.save(&lib.paths, &lib.writer).unwrap();
    assert!(lib.paths.file(&model, 2).exists());
    assert!(!lib.paths.file(&model, 1).exists());
    let m = edb::get_model(&lib.conn(), &model).unwrap().unwrap();
    assert_eq!(
        (m.index_generation, m.vector_count),
        (2, index.len() as u64)
    );
}

#[test]
fn p2_08_a_file_behind_the_catalog_is_reconciled() {
    let lib = Lib::new();
    let backend = HashedBagOfWordsBackend::new(DIMS);
    let (model, docs) = embedded(&lib, &backend);
    let (index, _) = open_index(&lib.paths, &lib.writer, &model, 4).unwrap();
    drop(index); // what a kill before the next save leaves

    // After the save: a new document, and a re-read of an old one.
    let pages = [prose(50, 800)];
    let new = lib.read("new.pdf", b"new", &[&pages[0]]);
    lib.embed(&backend, &model, &new);
    let p = [prose(60, 300)];
    lib.read("book 0.pdf", b"file 0", &[&p[0]]);
    lib.embed(&backend, &model, &docs[0]);

    let (index, report) = open_index(&lib.paths, &lib.writer, &model, 4).unwrap();
    match report.opened {
        Opened::Loaded { added, removed } => {
            assert!(added > 0 && removed > 0, "added {added}, removed {removed}");
        }
        other => panic!("expected a reconcile, got {other:?}"),
    }
    assert_matches_catalog(&lib, &index, &model, DIMS);

    // Saving drains the tombstones it applied.
    index.save(&lib.paths, &lib.writer).unwrap();
    assert_eq!(
        count(&lib.conn(), "SELECT COUNT(*) FROM vector_tombstones"),
        0
    );
    let (index, report) = open_index(&lib.paths, &lib.writer, &model, 4).unwrap();
    assert_eq!(
        report.opened,
        Opened::Loaded {
            added: 0,
            removed: 0
        }
    );
    assert_matches_catalog(&lib, &index, &model, DIMS);
}

/// Something done to an index file.
type Damage = fn(&std::path::Path);

#[test]
fn p2_08_a_damaged_or_missing_file_is_rebuilt() {
    let lib = Lib::new();
    let backend = HashedBagOfWordsBackend::new(DIMS);
    let (model, _) = embedded(&lib, &backend);
    let (index, _) = open_index(&lib.paths, &lib.writer, &model, 4).unwrap();
    drop(index);

    let damage: [(&str, Damage); 4] = [
        ("flipped", |p| {
            let mut b = fs::read(p).unwrap();
            let mid = b.len() / 2;
            b[mid] ^= 0xFF;
            fs::write(p, b).unwrap();
        }),
        ("truncated", |p| {
            let b = fs::read(p).unwrap();
            fs::write(p, &b[..b.len() / 2]).unwrap();
        }),
        ("garbage", |p| fs::write(p, b"not an index").unwrap()),
        ("deleted", |p| fs::remove_file(p).unwrap()),
    ];
    for (what, hurt) in damage {
        let m = edb::get_model(&lib.conn(), &model).unwrap().unwrap();
        hurt(&lib.paths.file(&model, m.index_generation));
        let (index, report) = open_index(&lib.paths, &lib.writer, &model, 4).unwrap();
        let reason = if what == "deleted" {
            RebuildReason::FileMissing
        } else {
            RebuildReason::HashMismatch
        };
        assert_eq!(report.opened, Opened::Rebuilt { reason }, "{what}");
        assert_matches_catalog(&lib, &index, &model, DIMS);
    }
}

#[test]
fn p2_08_wrong_dimensions_and_stray_generations() {
    let lib = Lib::new();
    let backend = HashedBagOfWordsBackend::new(DIMS);
    let (model, _) = embedded(&lib, &backend);
    let (index, _) = open_index(&lib.paths, &lib.writer, &model, 4).unwrap();
    let generation = index.generation();
    drop(index);

    // A kill between a save's rename and its catalog record leaves the next
    // generation unreferenced: the recorded one loads and the stray goes.
    let stray = lib.paths.file(&model, generation + 1);
    fs::copy(lib.paths.file(&model, generation), &stray).unwrap();
    let (_, report) = open_index(&lib.paths, &lib.writer, &model, 4).unwrap();
    assert_eq!(
        report.opened,
        Opened::Loaded {
            added: 0,
            removed: 0
        }
    );
    assert!(!stray.exists());

    // A file of another length of vector is never trusted.
    let other = lib.create_model(DIMS / 2);
    let m = edb::get_model(&lib.conn(), &model).unwrap().unwrap();
    let (gen_, hash) = (m.index_generation, m.index_blake3.unwrap());
    fs::copy(lib.paths.file(&model, gen_), lib.paths.file(&other, gen_)).unwrap();
    let o = other.clone();
    lib.writer
        .call(move |c| {
            c.execute(
                "UPDATE embedding_models SET index_generation = ?2, index_blake3 = ?3 WHERE id = ?1",
                rusqlite::params![o, gen_, &hash[..]],
            )
            .unwrap()
        })
        .unwrap();
    let (index, report) = open_index(&lib.paths, &lib.writer, &other, 2).unwrap();
    assert!(
        matches!(
            report.opened,
            Opened::Rebuilt {
                reason: RebuildReason::WrongDimensions | RebuildReason::LoadFailed
            }
        ),
        "{:?}",
        report.opened
    );
    assert_eq!(index.dims(), DIMS / 2);
    assert!(index.is_empty(), "the other model has no documents yet");
}

#[test]
fn p2_08_tombstones_wait_for_every_live_model() {
    let lib = Lib::new();
    let backend = HashedBagOfWordsBackend::new(DIMS);
    let (a, docs) = embedded(&lib, &backend);
    let b = lib.create_model(DIMS);
    for d in &docs {
        lib.embed(&backend, &b, d);
    }
    let (ia, _) = open_index(&lib.paths, &lib.writer, &a, 2).unwrap();
    let (ib, _) = open_index(&lib.paths, &lib.writer, &b, 2).unwrap();
    drop((ia, ib));

    let p = [prose(70, 200)];
    lib.read("book 1.pdf", b"file 1", &[&p[0]]);
    let tombs = count(&lib.conn(), "SELECT COUNT(*) FROM vector_tombstones");
    assert!(tombs > 0);

    let (ia, _) = open_index(&lib.paths, &lib.writer, &a, 2).unwrap();
    ia.save(&lib.paths, &lib.writer).unwrap();
    assert_eq!(
        count(&lib.conn(), "SELECT COUNT(*) FROM vector_tombstones"),
        tombs,
        "b hasn't applied them yet"
    );
    let (ib, report) = open_index(&lib.paths, &lib.writer, &b, 2).unwrap();
    assert!(matches!(report.opened, Opened::Loaded { removed, .. } if removed > 0));
    ib.save(&lib.paths, &lib.writer).unwrap();
    assert_eq!(
        count(&lib.conn(), "SELECT COUNT(*) FROM vector_tombstones"),
        0
    );
    assert_matches_catalog(&lib, &ia, &a, DIMS);
    assert_matches_catalog(&lib, &ib, &b, DIMS);
}

#[test]
fn p2_08_a_library_under_a_non_ascii_path() {
    for buffered in [false, true] {
        force_buffer_io(buffered);
        let lib = Lib::under("Bibliothèque José 図書館");
        let backend = HashedBagOfWordsBackend::new(DIMS);
        let (model, _) = embedded(&lib, &backend);
        let (index, _) = open_index(&lib.paths, &lib.writer, &model, 2).unwrap();
        index.save(&lib.paths, &lib.writer).unwrap();
        drop(index);
        let (index, report) = open_index(&lib.paths, &lib.writer, &model, 2).unwrap();
        assert_eq!(
            report.opened,
            Opened::Loaded {
                added: 0,
                removed: 0
            },
            "buffered {buffered}"
        );
        assert_matches_catalog(&lib, &index, &model, DIMS);
    }
    force_buffer_io(false);
}

#[test]
fn p2_08_searches_run_while_the_index_saves() {
    let lib = Lib::new();
    let backend = HashedBagOfWordsBackend::new(DIMS);
    let (model, _) = embedded(&lib, &backend);
    let (index, _) = open_index(&lib.paths, &lib.writer, &model, 2).unwrap();
    let q = backend.vector("orbit of the comet");
    let expected = index.search(&q, 5).unwrap();
    std::thread::scope(|s| {
        let saver = s.spawn(|| {
            for _ in 0..5 {
                index.save(&lib.paths, &lib.writer).unwrap();
            }
        });
        let searcher = s.spawn(|| {
            for _ in 0..500 {
                assert_eq!(index.search(&q, 5).unwrap(), expected);
            }
        });
        saver.join().unwrap();
        searcher.join().unwrap();
    });
    assert_eq!(index.generation(), 6);
    assert_eq!(index.changes_since_save(), 0);
}
