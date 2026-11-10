//! A small library for the vector tests: a catalog with documents stored
//! through the real `commit_document`, embedded with the fake backend.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use atlas_lib::domain::dto::FileType;
use atlas_lib::domain::traits::EmbeddingBackend;
use atlas_lib::services::catalog;
use atlas_lib::services::catalog::writer::CatalogWriter;
use atlas_lib::services::embedding::db::{self, StoredChunk};
use atlas_lib::services::embedding::fake::HashedBagOfWordsBackend;
use atlas_lib::services::embedding::index::{VECTORS_DIR, VectorIndex, VectorPaths};
use atlas_lib::services::embedding::vector::to_f16_blob;
use atlas_lib::services::extraction::db::{DocumentResult, commit_document};
use atlas_lib::services::extraction::engine::{Piece, Summary};
use atlas_lib::services::workspace::layout::{CATALOG, TMP};
use rusqlite::Connection;

pub const JOB: &str = "job-extract";
pub const ROOT: &str = "root-1";

pub struct Lib {
    pub dir: Option<tempfile::TempDir>,
    pub root: PathBuf,
    pub writer: CatalogWriter,
    pub paths: VectorPaths,
}

impl Lib {
    pub fn new() -> Self {
        Self::under("")
    }

    /// A library in a subfolder of a temporary folder (e.g. a non-ASCII name).
    pub fn under(name: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(name);
        for d in ["library", VECTORS_DIR, TMP] {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        // As a library opens it: 16 KiB pages, WAL, every migration.
        let (writer, _) = catalog::open(&root.join(CATALOG), "t".into()).unwrap();
        writer
            .call(|c| {
                c.execute_batch(&format!(
                    "INSERT INTO workspace (id, name, created_at) VALUES ('w', 'Test', 't');
                     INSERT INTO source_roots (id, workspace_id, path_bytes, display_path, created_at)
                         VALUES ('{ROOT}', 'w', x'00', '/books', 't');
                     INSERT INTO jobs (id, source_root_id, state, kind, created_at, updated_at)
                         VALUES ('{JOB}', '{ROOT}', 'running', 'extract', 't', 't');"
                ))
                .unwrap();
            })
            .unwrap();
        let paths = VectorPaths::for_root(&root);
        Self {
            dir: Some(dir),
            root,
            writer,
            paths,
        }
    }

    /// An existing library (a crash test's child, or the parent after it).
    pub fn open(root: &Path) -> Self {
        let (writer, _) = catalog::open(&root.join(CATALOG), "t".into()).unwrap();
        Self {
            dir: None,
            root: root.to_path_buf(),
            writer,
            paths: VectorPaths::for_root(root),
        }
    }

    pub fn conn(&self) -> Connection {
        Connection::open(&self.paths.catalog).unwrap()
    }

    /// Stores (or re-reads) the file `name` with these page texts, as
    /// extraction does. Returns the document id.
    pub fn read(&self, name: &str, content: &[u8], pages: &[&str]) -> String {
        let name = name.to_owned();
        let content = content.to_vec();
        let pieces: Vec<Piece> = pages
            .iter()
            .enumerate()
            .map(|(i, t)| Piece {
                ordinal: i as u32,
                page_no: Some(i as u32 + 1),
                location: format!("Page {}", i + 1),
                text: (*t).to_owned(),
            })
            .collect();
        self.writer
            .call(move |c| {
                c.execute(
                    "INSERT OR IGNORE INTO file_instances (id, source_root_id, rel_path,
                         display_path, display_name, extension, file_type, availability,
                         created_at, updated_at)
                     VALUES (?1, ?2, CAST(?1 AS BLOB), ?1, ?1, 'pdf', 'pdf', 'available', 't', 't')",
                    rusqlite::params![name, ROOT],
                )
                .unwrap();
                let summary = Summary {
                    page_count: pieces.len() as u32,
                    char_count: pieces.iter().map(|p| p.text.len() as u64).sum(),
                    title: None,
                    content_title: None,
                    authors: Vec::new(),
                    language: None,
                    truncated: 0,
                };
                let hash = blake3::hash(&content);
                assert!(
                    commit_document(
                        c,
                        JOB,
                        &name,
                        hash.as_bytes(),
                        content.len() as u64,
                        None,
                        FileType::Pdf,
                        DocumentResult::Text {
                            summary: &summary,
                            pieces: &pieces,
                        },
                        "t",
                    )
                    .unwrap()
                );
                c.query_row(
                    "SELECT id FROM documents WHERE content_hash = ?1",
                    [&hash.as_bytes()[..]],
                    |r| r.get(0),
                )
                .unwrap()
            })
            .unwrap()
    }

    pub fn create_model(&self, dims: usize) -> String {
        self.writer
            .call(move |c| db::create_model(c, None, "fake", dims, None, "t").unwrap())
            .unwrap()
    }

    /// Embeds one document as the job will: chunk, embed only the misses,
    /// store in one transaction. Returns the stored chunks.
    pub fn embed(
        &self,
        backend: &HashedBagOfWordsBackend,
        model: &str,
        document_id: &str,
    ) -> Vec<StoredChunk> {
        let conn = self.conn();
        let passages = db::passages(&conn, document_id).unwrap();
        let planned = db::plan(&passages);
        let hashes: Vec<[u8; 32]> = planned.iter().map(|(c, _)| c.hash).collect();
        let known = db::known_hashes(&conn, model, &hashes).unwrap();
        let mut misses: Vec<([u8; 32], &str)> = Vec::new();
        for (c, text) in &planned {
            if !known.contains(&c.hash) && !misses.iter().any(|(h, _)| *h == c.hash) {
                misses.push((c.hash, text));
            }
        }
        let vectors = if misses.is_empty() {
            Vec::new()
        } else {
            let texts: Vec<&str> = misses.iter().map(|(_, t)| *t).collect();
            backend.embed(&texts).unwrap()
        };
        let new: Vec<([u8; 32], Vec<u8>)> = misses
            .iter()
            .zip(&vectors)
            .map(|((h, _), v)| (*h, to_f16_blob(v)))
            .collect();
        let plan: Vec<db::PlannedChunk> = planned.into_iter().map(|(c, _)| c).collect();
        let (model, doc) = (model.to_owned(), document_id.to_owned());
        self.writer
            .call(move |c| {
                let tx = c.transaction().unwrap();
                let stored = db::write_document(&tx, &doc, &model, &plan, &new, "t").unwrap();
                tx.commit().unwrap();
                stored
            })
            .unwrap()
    }
}

/// `n` words of text, varied by `seed`, with sentences.
pub fn prose(seed: usize, n: usize) -> String {
    const WORDS: &[&str] = &[
        "orbit",
        "planet",
        "comet",
        "telescope",
        "galaxy",
        "nebula",
        "spectrum",
        "lens",
        "harvest",
        "river",
        "bread",
        "garden",
        "violin",
        "harbor",
        "glacier",
        "lantern",
        "theorem",
        "matrix",
        "protein",
        "cell",
        "market",
        "ledger",
        "castle",
        "ballad",
    ];
    let mut x = seed as u64 * 2_654_435_761 + 1;
    let mut out = String::new();
    for i in 0..n {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.push_str(WORDS[(x % WORDS.len() as u64) as usize]);
        out.push_str(if i % 12 == 11 { ". " } else { " " });
    }
    out
}

pub fn count(c: &Connection, sql: &str) -> i64 {
    c.query_row(sql, [], |r| r.get(0)).unwrap()
}

pub fn ids(c: &Connection, sql: &str) -> BTreeSet<u64> {
    c.prepare(sql)
        .unwrap()
        .query_map([], |r| r.get::<_, i64>(0).map(|n| n as u64))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// The chunk ids a model's index must hold: the chunks of its done
/// documents.
pub fn expected_keys(c: &Connection, model: &str) -> BTreeSet<u64> {
    ids(
        c,
        &format!(
            "SELECT c.id FROM chunks c JOIN document_embeddings d
               ON d.document_id = c.document_id AND d.model_id = '{model}' AND d.status = 'done'"
        ),
    )
}

/// The index holds exactly `expected`, and exact searches agree with a
/// brand-new index built from the same catalog.
pub fn assert_matches_catalog(lib: &Lib, index: &VectorIndex, model: &str, dims: usize) {
    let c = lib.conn();
    let expected = expected_keys(&c, model);
    assert_eq!(index.len(), expected.len(), "vector count");
    for k in &expected {
        assert!(index.contains(*k), "chunk {k} missing");
    }
    let fresh = VectorIndex::new(model, dims).unwrap();
    let all = db::vectors_after(&c, model, dims, 0, usize::MAX >> 1).unwrap();
    fresh.add_batch(&all, 1).unwrap();
    let backend = HashedBagOfWordsBackend::new(dims);
    for q in [
        "orbit of the comet",
        "bread from the garden",
        "a ballad of castles",
    ] {
        let v = backend.vector(q);
        let a = index.exact_search(&v, 10).unwrap();
        let b = fresh.exact_search(&v, 10).unwrap();
        assert_eq!(
            a.iter().map(|(k, _)| *k).collect::<Vec<_>>(),
            b.iter().map(|(k, _)| *k).collect::<Vec<_>>(),
            "results for {q:?}"
        );
        // The graph finds a vector itself.
        let (k, v) = &all[all.len() / 2];
        assert_eq!(index.search(v, 1).unwrap()[0].0, *k);
    }
}
