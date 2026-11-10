//! Phase 2 vector index benchmark (phase2-spec §13, ADR 0013): the index
//! alone, on synthetic vectors, through Atlas's own catalog rows, save,
//! load and rebuild.
//!
//!   cargo bench --bench vectors -- [--size 100000] [--dims 1024]
//!       [--threads N] [--queries 1000] [--recall-queries 200] [--seed 9]
//!       [--page-size 16384] [--out results.json]
//!
//! `--page-size 4096` measures a catalog created before Slice 1.7, whose
//! pages hold one 2 KB vector each.
//!
//! Vectors are unit-length points around random cluster centres (a Gaussian
//! mixture, as the Slice 0.9 spike's `datagen`): a rough stand-in for text
//! embeddings, which cluster by topic. Each document of 100 chunks belongs
//! to one of 8 "folders" of skewed sizes, so filtered searches run at about
//! 40%, 5% and 0.5% selectivity. Recall@10 is against brute force over the
//! same stored (f16) vectors. Measurements only; nothing fails on numbers.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Instant;

use atlas_lib::services::catalog::migrations::migrate;
use atlas_lib::services::catalog::writer::{CatalogWriter, enable_wal};
use atlas_lib::services::embedding::db as edb;
use atlas_lib::services::embedding::index::{
    Opened, VECTORS_DIR, VectorIndex, VectorPaths, open_index,
};
use atlas_lib::services::embedding::vector::{from_f16_blob, normalize, to_f16_blob};
use atlas_lib::services::workspace::layout::{CATALOG, TMP};
use rusqlite::params;
use serde_json::{Value, json};

const K: usize = 10;
const CHUNKS_PER_DOC: usize = 100;
const ROOT_WEIGHTS: [f64; 8] = [0.40, 0.25, 0.15, 0.10, 0.05, 0.03, 0.015, 0.005];
/// The folders filtered on: about 40%, 5% and 0.5% of the vectors.
const FILTER_ROOTS: [usize; 3] = [0, 4, 7];
const SIGMA: f32 = 0.6;
const BATCH: usize = 10_000;

fn fail(msg: &str) -> ! {
    eprintln!("vectors bench: {msg}");
    std::process::exit(2)
}

struct Args {
    size: usize,
    dims: usize,
    threads: usize,
    queries: usize,
    recall_queries: usize,
    seed: u64,
    page_size: u32,
    out: Option<PathBuf>,
}

fn parse_args() -> Args {
    let mut a = Args {
        size: 100_000,
        dims: 1024,
        threads: thread::available_parallelism().map_or(4, |n| n.get()),
        queries: 1_000,
        recall_queries: 200,
        seed: 9,
        page_size: 16_384,
        out: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || {
            it.next()
                .unwrap_or_else(|| fail(&format!("{arg} needs a value")))
        };
        let mut num = |name: &str| -> usize {
            value()
                .replace('_', "")
                .parse()
                .unwrap_or_else(|_| fail(name))
        };
        match arg.as_str() {
            "--bench" => {}
            "--size" => a.size = num("--size"),
            "--dims" => a.dims = num("--dims"),
            "--threads" => a.threads = num("--threads").max(1),
            "--queries" => a.queries = num("--queries").max(1),
            "--recall-queries" => a.recall_queries = num("--recall-queries").max(1),
            "--seed" => a.seed = num("--seed") as u64,
            "--page-size" => a.page_size = num("--page-size") as u32,
            "--out" => a.out = Some(PathBuf::from(value())),
            other => fail(&format!("unknown argument {other}")),
        }
    }
    if a.size < CHUNKS_PER_DOC {
        fail("--size must be at least 100");
    }
    a
}

/// splitmix64: small, seeded, and good enough for test data.
struct Rng(u64);

impl Rng {
    fn new(seed: u64, stream: u64) -> Self {
        Self(seed ^ stream.wrapping_mul(0x9E37_79B9_7F4A_7C15))
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn uniform(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    /// Standard normal (Box–Muller).
    fn normal(&mut self) -> f32 {
        let u1 = self.uniform().max(f64::MIN_POSITIVE);
        let u2 = self.uniform();
        ((-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()) as f32
    }
}

fn centres(n: usize, dims: usize, seed: u64) -> Vec<Vec<f32>> {
    let mut rng = Rng::new(seed, 1);
    (0..(n / 1000).clamp(64, 2000))
        .map(|_| {
            let mut c: Vec<f32> = (0..dims).map(|_| rng.normal()).collect();
            normalize(&mut c);
            c
        })
        .collect()
}

/// Point `i` of stream `stream`: deterministic whatever the thread count.
fn point(centres: &[Vec<f32>], dims: usize, seed: u64, stream: u64, i: usize) -> Vec<f32> {
    let mut rng = Rng::new(
        seed ^ (i as u64).wrapping_mul(0xD1B5_4A32_D192_ED03),
        stream,
    );
    let c = &centres[(rng.next() % centres.len() as u64) as usize];
    let scale = SIGMA / (dims as f32).sqrt();
    let mut v: Vec<f32> = c.iter().map(|x| x + scale * rng.normal()).collect();
    normalize(&mut v);
    v
}

/// The stored vectors as f16 bytes (half the memory of f32), in parallel.
fn generate(a: &Args, centres: &[Vec<f32>]) -> Vec<Vec<u8>> {
    let per = a.size.div_ceil(a.threads);
    let mut out: Vec<Vec<u8>> = vec![Vec::new(); a.size];
    thread::scope(|s| {
        for (t, part) in out.chunks_mut(per).enumerate() {
            s.spawn(move || {
                for (j, slot) in part.iter_mut().enumerate() {
                    *slot = to_f16_blob(&point(centres, a.dims, a.seed, 2, t * per + j));
                }
            });
        }
    });
    out
}

/// The folder of document `d`, by the skewed weights.
fn root_of(d: usize, seed: u64) -> usize {
    let u = Rng::new(seed ^ d as u64, 3).uniform();
    let mut acc = 0.0;
    for (r, w) in ROOT_WEIGHTS.iter().enumerate() {
        acc += w;
        if u < acc {
            return r;
        }
    }
    ROOT_WEIGHTS.len() - 1
}

fn ms(t: Instant) -> f64 {
    (t.elapsed().as_secs_f64() * 1e6).round() / 1e3
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn latency(samples: &mut [f64]) -> Value {
    samples.sort_by(f64::total_cmp);
    json!({
        "p50Ms": percentile(samples, 0.50),
        "p95Ms": percentile(samples, 0.95),
        "maxMs": samples[samples.len() - 1],
    })
}

/// Exact top-K keys among the vectors `keep` accepts, by dot product with
/// unit vectors (cosine), for each query, in parallel.
fn brute_force(
    data: &[Vec<u8>],
    dims: usize,
    queries: &[Vec<f32>],
    threads: usize,
    keep: impl Fn(usize) -> bool + Sync,
) -> Vec<Vec<u64>> {
    let per = queries.len().div_ceil(threads).max(1);
    let keep = &keep;
    thread::scope(|s| {
        let handles: Vec<_> = queries
            .chunks(per)
            .map(|qs| {
                s.spawn(move || {
                    qs.iter()
                        .map(|q| {
                            let mut best: Vec<(f32, u64)> = Vec::with_capacity(K + 1);
                            for (i, blob) in data.iter().enumerate() {
                                if !keep(i) {
                                    continue;
                                }
                                let v = from_f16_blob(blob, dims).expect("length");
                                let d: f32 = v.iter().zip(q).map(|(a, b)| a * b).sum();
                                if best.len() < K || d > best[K - 1].0 {
                                    let at = best.partition_point(|(x, _)| *x >= d);
                                    best.insert(at, (d, i as u64 + 1));
                                    best.truncate(K);
                                }
                            }
                            best.into_iter().map(|(_, k)| k).collect::<Vec<u64>>()
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect()
    })
}

fn recall(found: &[Vec<u64>], truth: &[Vec<u64>]) -> f64 {
    let hits: usize = found
        .iter()
        .zip(truth)
        .map(|(f, t)| f.iter().filter(|k| t.contains(k)).count())
        .sum();
    let total: usize = truth.iter().map(Vec::len).sum();
    (hits as f64 / total.max(1) as f64 * 1000.0).round() / 1000.0
}

/// A library folder with a migrated catalog holding `data` as chunks of
/// done documents. Returns (paths, writer, model id, insert ms).
fn library(
    dir: &Path,
    data: &Arc<Vec<Vec<u8>>>,
    dims: usize,
    page_size: u32,
) -> (VectorPaths, CatalogWriter, String, f64) {
    for d in ["library", VECTORS_DIR, TMP] {
        fs::create_dir_all(dir.join(d)).unwrap();
    }
    // As `catalog::open` makes a new catalog, at the chosen page size.
    let writer = CatalogWriter::open(&dir.join(CATALOG)).unwrap();
    writer
        .call(move |c| c.pragma_update(None, "page_size", page_size))
        .unwrap()
        .unwrap();
    writer.call(enable_wal).unwrap().unwrap();
    let started = Instant::now();
    let data = Arc::clone(data);
    let model = writer
        .call(move |c| {
            migrate(c, "t").unwrap();
            let model = edb::create_model(c, None, "synthetic", dims, None, "t").unwrap();
            for (d, docs) in data.chunks(CHUNKS_PER_DOC * 50).enumerate() {
                let tx = c.transaction().unwrap();
                for (j, chunk_rows) in docs.chunks(CHUNKS_PER_DOC).enumerate() {
                    let doc = format!("doc-{d}-{j}");
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
                    for (i, blob) in chunk_rows.iter().enumerate() {
                        let hash = blake3::hash(format!("{doc}/{i}").as_bytes());
                        tx.execute(
                            "INSERT INTO chunks (document_id, passage_id, ordinal, char_start,
                                 char_end, text_hash) VALUES (?1, ?2, ?3, 0, 1200, ?4)",
                            params![doc, passage, i as i64, &hash.as_bytes()[..]],
                        )
                        .unwrap();
                        tx.execute(
                            "INSERT INTO embeddings (model_id, text_hash, vector) VALUES (?1, ?2, ?3)",
                            params![model, &hash.as_bytes()[..], blob],
                        )
                        .unwrap();
                    }
                    tx.execute(
                        "INSERT INTO document_embeddings (document_id, model_id, seq, status,
                             chunk_count, updated_at)
                         VALUES (?1, ?2, (SELECT COALESCE(MAX(seq), 0) + 1 FROM document_embeddings
                                          WHERE model_id = ?2), 'done', ?3, 't')",
                        params![doc, model, chunk_rows.len() as i64],
                    )
                    .unwrap();
                }
                tx.commit().unwrap();
            }
            c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
            model
        })
        .unwrap();
    (VectorPaths::for_root(dir), writer, model, ms(started))
}

fn machine() -> Value {
    json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "logicalCores": thread::available_parallelism().map(|n| n.get()).ok(),
        "ci": std::env::var("CI").is_ok(),
    })
}

fn main() {
    let a = parse_args();
    let tmp = tempfile::tempdir().unwrap();
    eprintln!(
        "vectors bench: {} vectors × {} dimensions, {} threads",
        a.size, a.dims, a.threads
    );

    let t = Instant::now();
    let centres = centres(a.size, a.dims, a.seed);
    let data = Arc::new(generate(&a, &centres));
    let queries: Vec<Vec<f32>> = (0..a.queries)
        .map(|i| point(&centres, a.dims, a.seed, 4, i))
        .collect();
    let generate_ms = ms(t);
    let roots: Vec<usize> = (0..a.size)
        .map(|i| root_of(i / CHUNKS_PER_DOC, a.seed))
        .collect();

    // The catalog rows, as the embedding job will leave them.
    let (paths, writer, model, insert_ms) = library(tmp.path(), &data, a.dims, a.page_size);
    let catalog_bytes = fs::metadata(&paths.catalog).unwrap().len();

    // Build in memory, as the job will (incremental adds).
    let t = Instant::now();
    let index = VectorIndex::new(&model, a.dims).unwrap();
    for (b, part) in data.chunks(BATCH).enumerate() {
        let items: Vec<(u64, Vec<f32>)> = part
            .iter()
            .enumerate()
            .map(|(i, blob)| {
                (
                    (b * BATCH + i) as u64 + 1,
                    from_f16_blob(blob, a.dims).unwrap(),
                )
            })
            .collect();
        index.add_batch(&items, a.threads).unwrap();
    }
    let build_ms = ms(t);
    // As the embedding job does after each document: the index holds every
    // document written so far.
    let conn = rusqlite::Connection::open(&paths.catalog).unwrap();
    index.advance_seq(edb::last_seq(&conn, &model).unwrap());
    drop(conn);
    let memory = index.memory_usage();
    let saved = index.save(&paths, &writer).unwrap();
    let file_bytes = saved.bytes;
    drop(index);

    // Reload from the saved file, then the first answer.
    let t = Instant::now();
    let (index, report) = open_index(&paths, &writer, &model, a.threads).unwrap();
    assert!(matches!(report.opened, Opened::Loaded { .. }), "{report:?}");
    let load_ms = ms(t);
    index.search(&queries[0], K).unwrap();
    let first_answer_ms = ms(t);

    // Unfiltered: latency over every query, recall over the first ones.
    let mut lat = Vec::with_capacity(queries.len());
    let mut found = Vec::new();
    for (i, q) in queries.iter().enumerate() {
        let t = Instant::now();
        let hits = index.search(q, K).unwrap();
        lat.push(ms(t));
        if i < a.recall_queries {
            found.push(hits.into_iter().map(|(k, _)| k).collect::<Vec<_>>());
        }
    }
    let mut wide = Vec::new();
    for q in &queries {
        let t = Instant::now();
        index.search(q, 200).unwrap();
        wide.push(ms(t));
    }
    let unfiltered_at_200 = latency(&mut wide);
    let rq = &queries[..a.recall_queries.min(queries.len())];
    let t = Instant::now();
    let truth = brute_force(&data, a.dims, rq, a.threads, |_| true);
    let brute_ms = ms(t) / rq.len() as f64;
    let unfiltered = json!({
        "recallAt10": recall(&found, &truth),
        "latency": latency(&mut lat),
        "latencyAt200": unfiltered_at_200,
        "bruteForceMsPerQuery": (brute_ms * 1000.0).round() / 1000.0,
    });

    let mut filtered = Vec::new();
    for root in FILTER_ROOTS {
        let share = roots.iter().filter(|r| **r == root).count() as f64 / a.size as f64;
        let roots = &roots;
        let keep = |k: u64| roots[(k - 1) as usize] == root;
        let mut lat = Vec::new();
        let mut found = Vec::new();
        for (i, q) in queries.iter().enumerate() {
            let t = Instant::now();
            let hits = index.filtered_search(q, K, keep).unwrap();
            lat.push(ms(t));
            if i < a.recall_queries {
                found.push(hits.into_iter().map(|(k, _)| k).collect::<Vec<_>>());
            }
        }
        // As hybrid search asks (Slice 2.4): the 200 nearest in the folder.
        let mut wide = Vec::new();
        for q in &queries {
            let t = Instant::now();
            index.filtered_search(q, 200, keep).unwrap();
            wide.push(ms(t));
        }
        let truth = brute_force(&data, a.dims, rq, a.threads, |i| roots[i] == root);
        filtered.push(json!({
            "latencyAt200": latency(&mut wide),
            "share": (share * 1000.0).round() / 1000.0,
            // No vectors in that folder at this size: nothing to recall.
            "recallAt10": (share > 0.0).then(|| recall(&found, &truth)),
            "latency": latency(&mut lat),
        }));
    }
    drop(index);

    // Rebuild from the catalog: the file is gone.
    fs::remove_file(paths.file(&model, saved.generation)).unwrap();
    let t = Instant::now();
    let (index, report) = open_index(&paths, &writer, &model, a.threads).unwrap();
    let rebuild_ms = ms(t);
    assert!(
        matches!(report.opened, Opened::Rebuilt { .. }),
        "{report:?}"
    );
    assert_eq!(index.len(), a.size);

    let raw = (a.size * a.dims * 4) as f64;
    let result = json!({
        "bench": "vectors",
        "machine": machine(),
        "size": a.size,
        "dims": a.dims,
        "threads": a.threads,
        "pageSize": a.page_size,
        "queries": a.queries,
        "recallQueries": rq.len(),
        "index": {
            "connectivity": 16, "expansionAdd": 128, "expansionSearch": 128,
            "metric": "cos", "quantization": "f16",
        },
        "generateMs": generate_ms,
        "catalogInsertMs": insert_ms,
        "buildMs": build_ms,
        "saveMs": saved.ms,
        "loadMs": load_ms,
        "openToFirstAnswerMs": first_answer_ms,
        "rebuildFromCatalogMs": rebuild_ms,
        "fileBytes": file_bytes,
        "fileToRawF32": (file_bytes as f64 / raw * 1000.0).round() / 1000.0,
        "indexMemoryBytes": memory,
        "catalogBytesPerVector": (catalog_bytes as f64 / a.size as f64).round(),
        "unfiltered": unfiltered,
        "filtered": filtered,
    });
    let text = serde_json::to_string_pretty(&result).unwrap();
    match &a.out {
        Some(out) => fs::write(out, &text).unwrap(),
        None => println!("{text}"),
    }
}
