//! The vector index (phase2-spec §7.3, ADR 0013): a usearch HNSW graph
//! (cosine, f16) over chunk ids, derived from the catalog and disposable.
//!
//! - **Saves are atomic and named by generation**: the index is written to
//!   `tmp/`, flushed, renamed to `indexes/vectors/<model>-<generation>.usearch`,
//!   hashed, and only then recorded in the catalog; the previous file is
//!   removed after that. A kill at any point leaves the catalog naming a
//!   whole file that exists.
//! - **Opening verifies before loading**: a file whose BLAKE3 differs from
//!   the catalog's never reaches usearch's loader. A missing, mismatched or
//!   unloadable file, or one that lacks more vectors than it holds, is
//!   rebuilt from the catalog.
//! - **Reconciling**: a loaded file gains the chunks of documents written
//!   after it was saved and loses the tombstoned chunks.
//!
//! The in-memory index is authoritative; the file is a cache.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::thread;
use std::time::Instant;

use rusqlite::Connection;
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

use super::db::{self, EmbeddingModel, SavedIndex};
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::clock::now_rfc3339;
use crate::infrastructure::fsutil::atomic_replace_with;
use crate::services::catalog::reader;
use crate::services::catalog::writer::CatalogHandle;
use crate::services::workspace::layout::CATALOG;

pub const CONNECTIVITY: usize = 16;
pub const EXPANSION_ADD: usize = 128;
pub const EXPANSION_SEARCH: usize = 128;
/// Rows per batch when rebuilding or reconciling.
pub const BATCH: usize = 10_000;
const RESERVE_STEP: usize = 10_000;
pub const VECTORS_DIR: &str = "indexes/vectors";
/// Where saves are staged before their rename, inside [`VECTORS_DIR`].
pub const STAGING_DIR: &str = "staging";

/// Where a library keeps the catalog, the index files and staged writes.
#[derive(Debug, Clone)]
pub struct VectorPaths {
    pub catalog: PathBuf,
    pub vectors: PathBuf,
    pub tmp: PathBuf,
}

impl VectorPaths {
    pub fn for_root(root: &Path) -> Self {
        Self {
            catalog: root.join(CATALOG),
            vectors: root.join(VECTORS_DIR),
            // Not the library's tmp/, which Clean caches empties while a
            // save may be staging (Slice 2.3). Emptied when the library opens.
            tmp: root.join(VECTORS_DIR).join(STAGING_DIR),
        }
    }

    pub fn file(&self, model_id: &str, generation: i64) -> PathBuf {
        self.vectors
            .join(format!("{model_id}-{generation}.usearch"))
    }
}

/// Why an index was rebuilt instead of loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RebuildReason {
    NeverSaved,
    FileMissing,
    HashMismatch,
    LoadFailed,
    WrongDimensions,
    TooFarBehind,
    /// More than [`COMPACT_ABOVE`] of the file's slots hold removed vectors.
    Compaction,
}

/// The share of removed vectors in a saved file above which opening it
/// rebuilds into a fresh one (phase2-spec §7.3).
pub const COMPACT_ABOVE: f64 = 0.2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opened {
    /// The saved file, plus `added` vectors and `removed` tombstones.
    Loaded {
        added: usize,
        removed: usize,
    },
    Rebuilt {
        reason: RebuildReason,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenReport {
    pub opened: Opened,
    pub vectors: usize,
    pub ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveReport {
    pub generation: i64,
    pub bytes: u64,
    pub ms: u64,
}

struct Inner {
    index: Index,
    /// The last document seq whose chunks are in the index.
    through_seq: i64,
    /// The last tombstone applied.
    tombstones_through: i64,
    /// The generation of the file last saved (0: none).
    generation: i64,
    changes_since_save: usize,
}

/// One embedding model's index. Searches and saves share a read lock;
/// adds and removes take the write lock.
pub struct VectorIndex {
    model_id: String,
    dims: usize,
    threads: usize,
    inner: RwLock<Inner>,
    save_lock: Mutex<()>,
}

fn options(dims: usize) -> IndexOptions {
    IndexOptions {
        dimensions: dims,
        metric: MetricKind::Cos,
        quantization: ScalarKind::F16,
        connectivity: CONNECTIVITY,
        expansion_add: EXPANSION_ADD,
        expansion_search: EXPANSION_SEARCH,
        multi: false,
    }
}

fn native<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::other(e.to_string())
}

/// Concurrent searches the index makes room for.
fn search_threads() -> usize {
    thread::available_parallelism().map_or(4, |n| n.get().clamp(2, 64))
}

static FORCE_BUFFER_IO: AtomicBool = AtomicBool::new(false);

/// Tests: save and load through memory buffers even where the index could
/// use the path itself.
#[doc(hidden)]
pub fn force_buffer_io(on: bool) {
    FORCE_BUFFER_IO.store(on, Ordering::Relaxed);
}

/// usearch's own save and load take a narrow path, which Windows reads in
/// the ANSI code page, so a path outside ASCII goes through a buffer and
/// Rust's file I/O instead (ADR 0013).
fn path_for_native_io(path: &Path) -> Option<&str> {
    if FORCE_BUFFER_IO.load(Ordering::Relaxed) {
        return None;
    }
    let s = path.to_str()?;
    (!cfg!(windows) || s.is_ascii()).then_some(s)
}

fn write_index(index: &Index, path: &Path) -> io::Result<()> {
    match path_for_native_io(path) {
        Some(p) => index.save(p).map_err(native),
        None => {
            let mut buffer = vec![0u8; index.serialized_length()];
            index.save_to_buffer(&mut buffer).map_err(native)?;
            File::create_new(path)?.write_all(&buffer)
        }
    }
}

fn read_index(path: &Path, dims: usize) -> io::Result<Index> {
    let index = Index::new(&options(dims)).map_err(native)?;
    match path_for_native_io(path) {
        Some(p) => index.load(p).map_err(native)?,
        None => index.load_from_buffer(&fs::read(path)?).map_err(native)?,
    }
    Ok(index)
}

pub fn hash_file(path: &Path) -> io::Result<[u8; 32]> {
    let mut hasher = blake3::Hasher::new();
    hasher.update_reader(File::open(path)?)?;
    Ok(*hasher.finalize().as_bytes())
}

impl VectorIndex {
    pub fn new(model_id: &str, dims: usize) -> io::Result<Self> {
        let index = Index::new(&options(dims)).map_err(native)?;
        Ok(Self::wrap(model_id, dims, index, 0, 0, 0))
    }

    fn wrap(
        model_id: &str,
        dims: usize,
        index: Index,
        through_seq: i64,
        tombstones_through: i64,
        generation: i64,
    ) -> Self {
        Self {
            model_id: model_id.to_owned(),
            dims,
            threads: search_threads(),
            inner: RwLock::new(Inner {
                index,
                through_seq,
                tombstones_through,
                generation,
                changes_since_save: 0,
            }),
            save_lock: Mutex::new(()),
        }
    }

    fn read(&self) -> RwLockReadGuard<'_, Inner> {
        self.inner.read().unwrap_or_else(|p| p.into_inner())
    }

    fn write(&self) -> RwLockWriteGuard<'_, Inner> {
        self.inner.write().unwrap_or_else(|p| p.into_inner())
    }

    pub fn model_id(&self) -> &str {
        &self.model_id
    }
    pub fn dims(&self) -> usize {
        self.dims
    }
    pub fn len(&self) -> usize {
        self.read().index.size()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn contains(&self, key: u64) -> bool {
        self.read().index.contains(key)
    }
    pub fn changes_since_save(&self) -> usize {
        self.read().changes_since_save
    }
    pub fn generation(&self) -> i64 {
        self.read().generation
    }
    pub fn through_seq(&self) -> i64 {
        self.read().through_seq
    }
    pub fn memory_usage(&self) -> usize {
        self.read().index.memory_usage()
    }

    /// Removes the chunks deleted since the last tombstone this index
    /// applied (a re-read on another thread deletes chunks). Returns how
    /// many it removed.
    pub fn apply_tombstones(&self, conn: &Connection) -> AtlasResult<usize> {
        let through = self.read().tombstones_through;
        let tombstones = db::tombstones_after(conn, through)
            .map_err(AtlasError::db("read the meaning index rows"))?;
        let Some((last, _)) = tombstones.last() else {
            return Ok(0);
        };
        let keys: Vec<u64> = tombstones.iter().map(|(_, k)| *k).collect();
        let removed = self
            .remove(&keys)
            .map_err(AtlasError::io("update the meaning index"))?;
        let mut inner = self.write();
        inner.tombstones_through = inner.tombstones_through.max(*last);
        Ok(removed)
    }

    /// Records that the index now holds every document up to `seq`.
    pub fn advance_seq(&self, seq: i64) {
        let mut inner = self.write();
        inner.through_seq = inner.through_seq.max(seq);
    }

    /// Adds vectors, skipping keys already present, on up to `threads`
    /// threads. Returns how many were added.
    pub fn add_batch(&self, items: &[(u64, Vec<f32>)], threads: usize) -> io::Result<usize> {
        if let Some((_, v)) = items.iter().find(|(_, v)| v.len() != self.dims) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("a vector has {} numbers, not {}", v.len(), self.dims),
            ));
        }
        let mut inner = self.write();
        let fresh: Vec<&(u64, Vec<f32>)> = items
            .iter()
            .filter(|(k, _)| !inner.index.contains(*k))
            .collect();
        if fresh.is_empty() {
            return Ok(0);
        }
        let index = &inner.index;
        let need = index.size() + fresh.len();
        let threads = threads.max(1);
        if index.capacity() < need {
            let grow = RESERVE_STEP.max(index.size() / 2);
            index
                .reserve_capacity_and_threads(
                    need.max(index.size() + grow),
                    threads.max(self.threads),
                )
                .map_err(native)?;
        }
        if threads == 1 || fresh.len() < 1_000 {
            for (k, v) in &fresh {
                index.add(*k, v).map_err(native)?;
            }
        } else {
            let per = fresh.len().div_ceil(threads);
            thread::scope(|s| {
                let handles: Vec<_> = fresh
                    .chunks(per)
                    .map(|mine| {
                        s.spawn(move || -> io::Result<()> {
                            for (k, v) in mine {
                                index.add(*k, v).map_err(native)?;
                            }
                            Ok(())
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .try_for_each(|h| h.join().unwrap_or_else(|_| Err(native("an add panicked"))))
            })?;
        }
        inner.changes_since_save += fresh.len();
        Ok(fresh.len())
    }

    /// Removes keys; absent keys are ignored. Returns how many were removed.
    pub fn remove(&self, keys: &[u64]) -> io::Result<usize> {
        let mut inner = self.write();
        let mut removed = 0;
        for k in keys {
            if inner.index.contains(*k) {
                removed += inner.index.remove(*k).map_err(native)?;
            }
        }
        inner.changes_since_save += removed;
        Ok(removed)
    }

    /// The `k` nearest keys with their cosine distances, nearest first.
    pub fn search(&self, query: &[f32], k: usize) -> io::Result<Vec<(u64, f32)>> {
        let inner = self.read();
        if inner.index.size() == 0 || k == 0 {
            return Ok(Vec::new());
        }
        pairs(inner.index.search(query, k).map_err(native)?)
    }

    /// As [`search`](Self::search), among keys `keep` accepts.
    pub fn filtered_search(
        &self,
        query: &[f32],
        k: usize,
        keep: impl Fn(u64) -> bool,
    ) -> io::Result<Vec<(u64, f32)>> {
        let inner = self.read();
        if inner.index.size() == 0 || k == 0 {
            return Ok(Vec::new());
        }
        pairs(
            inner
                .index
                .filtered_search(query, k, keep)
                .map_err(native)?,
        )
    }

    /// Brute force over every vector: the reference for recall.
    pub fn exact_search(&self, query: &[f32], k: usize) -> io::Result<Vec<(u64, f32)>> {
        let inner = self.read();
        if inner.index.size() == 0 || k == 0 {
            return Ok(Vec::new());
        }
        pairs(inner.index.exact_search(query, k).map_err(native)?)
    }

    /// Saves atomically as the next generation, records it in the catalog,
    /// drains applied tombstones and removes the previous file. Searches
    /// continue meanwhile (a save only reads the graph); adds and removes
    /// wait. Saves are one at a time.
    pub fn save(&self, paths: &VectorPaths, catalog: &CatalogHandle) -> AtlasResult<SaveReport> {
        let _one_save = self.save_lock.lock().unwrap_or_else(|p| p.into_inner());
        let started = Instant::now();
        let inner = self.read();
        let generation = inner.generation + 1;
        let target = paths.file(&self.model_id, generation);
        fs::create_dir_all(&paths.vectors).map_err(AtlasError::io("create the index folder"))?;
        atomic_replace_with(&target, &paths.tmp, |tmp| write_index(&inner.index, tmp))
            .map_err(AtlasError::io("save the meaning index"))?;
        let hashed = hash_file(&target).and_then(|h| Ok((h, fs::metadata(&target)?.len())));
        let (blake3, bytes) = match hashed {
            Ok(v) => v,
            Err(e) => {
                let _ = fs::remove_file(&target);
                return Err(AtlasError::io("check the saved meaning index")(e));
            }
        };
        let saved = SavedIndex {
            generation,
            blake3,
            indexed_through_seq: inner.through_seq,
            vector_count: inner.index.size() as u64,
            tombstones_through: inner.tombstones_through,
        };
        let saved_changes = inner.changes_since_save;
        // The file holds exactly this state; changes may resume.
        drop(inner);
        let model = self.model_id.clone();
        let now = now_rfc3339();
        let recorded = catalog
            .call(move |c| db::record_save(c, &model, &saved, &now))
            .and_then(|r| r.map_err(AtlasError::db("record the meaning index")));
        if let Err(e) = recorded {
            let _ = fs::remove_file(&target);
            return Err(e);
        }
        {
            let mut inner = self.write();
            inner.generation = generation;
            inner.changes_since_save = inner.changes_since_save.saturating_sub(saved_changes);
        }
        remove_other_generations(paths, &self.model_id, generation);
        Ok(SaveReport {
            generation,
            bytes,
            ms: started.elapsed().as_millis() as u64,
        })
    }
}

fn pairs(m: usearch::ffi::Matches) -> io::Result<Vec<(u64, f32)>> {
    Ok(m.keys.into_iter().zip(m.distances).collect())
}

/// Removes this model's index files other than `keep`'s generation
/// (left behind by a kill between a save's rename and its catalog record).
pub fn remove_other_generations(paths: &VectorPaths, model_id: &str, keep: i64) {
    let prefix = format!("{model_id}-");
    let keep_name = format!("{model_id}-{keep}.usearch");
    let Ok(entries) = fs::read_dir(&paths.vectors) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(&prefix) && name.ends_with(".usearch") && name != keep_name {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Opens a model's index: loads and reconciles the saved file, or rebuilds
/// from the catalog (and saves). `threads` bounds the rebuild's workers.
pub fn open_index(
    paths: &VectorPaths,
    catalog: &CatalogHandle,
    model_id: &str,
    threads: usize,
) -> AtlasResult<(VectorIndex, OpenReport)> {
    let started = Instant::now();
    let conn = reader::open_single(&paths.catalog)?;
    let model = db::get_model(&conn, model_id)
        .map_err(AtlasError::db("read the embedding model"))?
        .ok_or_else(|| {
            AtlasError::io("open the meaning index")(io::Error::new(
                io::ErrorKind::NotFound,
                "no such embedding model",
            ))
        })?;
    remove_other_generations(paths, model_id, model.index_generation);

    let loaded = load_saved(paths, &model);
    let (index, opened) = match loaded {
        Ok(index) => match reconcile(&conn, &model, index, threads)? {
            Ok((index, added, removed)) => (index, Opened::Loaded { added, removed }),
            Err(reason) => (rebuild(&conn, &model, threads)?, Opened::Rebuilt { reason }),
        },
        Err(reason) => (rebuild(&conn, &model, threads)?, Opened::Rebuilt { reason }),
    };
    if matches!(opened, Opened::Rebuilt { .. }) {
        index.save(paths, catalog)?;
    }
    let vectors = index.len();
    Ok((
        index,
        OpenReport {
            opened,
            vectors,
            ms: started.elapsed().as_millis() as u64,
        },
    ))
}

fn load_saved(paths: &VectorPaths, model: &EmbeddingModel) -> Result<VectorIndex, RebuildReason> {
    let Some(expected) = model.index_blake3.filter(|_| model.index_generation > 0) else {
        return Err(RebuildReason::NeverSaved);
    };
    let file = paths.file(&model.id, model.index_generation);
    match hash_file(&file) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(RebuildReason::FileMissing),
        Err(_) => return Err(RebuildReason::LoadFailed),
        Ok(h) if h != expected => return Err(RebuildReason::HashMismatch),
        Ok(_) => {}
    }
    // The file's header says how many of its slots hold removed vectors,
    // whichever way the file is read.
    let head = match path_for_native_io(&file) {
        Some(p) => Index::metadata(p).ok(),
        None => fs::read(&file)
            .ok()
            .and_then(|b| Index::metadata_from_buffer(&b).ok()),
    };
    if let Some(head) = head
        && head.count_deleted as f64 > COMPACT_ABOVE * head.count_present.max(1) as f64
    {
        return Err(RebuildReason::Compaction);
    }
    let index = read_index(&file, model.dims).map_err(|_| RebuildReason::LoadFailed)?;
    if index.dimensions() != model.dims {
        return Err(RebuildReason::WrongDimensions);
    }
    Ok(VectorIndex::wrap(
        &model.id,
        model.dims,
        index,
        model.indexed_through_seq,
        model.tombstones_through,
        model.index_generation,
    ))
}

type Reconciled = Result<(VectorIndex, usize, usize), RebuildReason>;

/// Adds what the saved file lacks and removes what was deleted since, all
/// from one read snapshot.
fn reconcile(
    conn: &Connection,
    model: &EmbeddingModel,
    index: VectorIndex,
    threads: usize,
) -> AtlasResult<Reconciled> {
    let snapshot = conn
        .unchecked_transaction()
        .map_err(AtlasError::db("read the meaning index rows"))?;
    let behind = db::chunks_after_seq(&snapshot, &model.id, model.indexed_through_seq)
        .map_err(AtlasError::db("read the meaning index rows"))?;
    if behind > model.vector_count.max(1) {
        return Ok(Err(RebuildReason::TooFarBehind));
    }
    let mut added = 0;
    let mut seq = model.indexed_through_seq;
    loop {
        let docs = db::documents_after(&snapshot, &model.id, seq, 500)
            .map_err(AtlasError::db("read the meaning index rows"))?;
        let Some(last) = docs.last().map(|(s, _)| *s) else {
            break;
        };
        let mut batch = Vec::new();
        for (_, doc) in &docs {
            batch.extend(
                db::document_vectors(&snapshot, &model.id, model.dims, doc)
                    .map_err(AtlasError::db("read the meaning index rows"))?,
            );
        }
        added += index
            .add_batch(&batch, threads)
            .map_err(AtlasError::io("update the meaning index"))?;
        seq = last;
    }
    index.advance_seq(
        db::last_seq(&snapshot, &model.id)
            .map_err(AtlasError::db("read the meaning index rows"))?,
    );

    let tombstones = db::tombstones_after(&snapshot, model.tombstones_through)
        .map_err(AtlasError::db("read the meaning index rows"))?;
    let keys: Vec<u64> = tombstones.iter().map(|(_, k)| *k).collect();
    let removed = index
        .remove(&keys)
        .map_err(AtlasError::io("update the meaning index"))?;
    if let Some((last, _)) = tombstones.last() {
        index.write().tombstones_through = *last;
    }
    Ok(Ok((index, added, removed)))
}

/// A fresh index from every vector of the model's done documents, from one
/// read snapshot. It keeps the old file's generation so its save is the
/// next one.
fn rebuild(conn: &Connection, model: &EmbeddingModel, threads: usize) -> AtlasResult<VectorIndex> {
    let read = |e| AtlasError::db("read the meaning index rows")(e);
    let snapshot = conn.unchecked_transaction().map_err(read)?;
    let tombstones_through = db::last_tombstone(&snapshot).map_err(read)?;
    let through_seq = db::last_seq(&snapshot, &model.id).map_err(read)?;
    let index = VectorIndex::new(&model.id, model.dims)
        .map_err(AtlasError::io("create the meaning index"))?;
    let mut after = 0;
    loop {
        let batch =
            db::vectors_after(&snapshot, &model.id, model.dims, after, BATCH).map_err(read)?;
        let Some((last, _)) = batch.last() else {
            break;
        };
        after = *last;
        index
            .add_batch(&batch, threads)
            .map_err(AtlasError::io("build the meaning index"))?;
    }
    {
        let mut inner = index.write();
        inner.through_seq = through_seq;
        inner.tombstones_through = tombstones_through;
        inner.generation = model.index_generation;
    }
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(seed: u64, dims: usize) -> Vec<f32> {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        let mut v: Vec<f32> = (0..dims)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x % 2001) as f32 - 1000.0
            })
            .collect();
        assert!(crate::services::embedding::vector::normalize(&mut v));
        v
    }

    #[test]
    fn adds_searches_removes_and_filters() {
        let index = VectorIndex::new("m", 32).unwrap();
        let items: Vec<(u64, Vec<f32>)> = (1..=2_000).map(|k| (k, unit(k, 32))).collect();
        assert_eq!(index.add_batch(&items, 4).unwrap(), 2_000);
        assert_eq!(
            index.add_batch(&items[..10], 1).unwrap(),
            0,
            "present keys are skipped"
        );
        assert_eq!(index.len(), 2_000);

        let hits = index.search(&unit(42, 32), 5).unwrap();
        assert_eq!(hits[0].0, 42, "a vector is its own nearest neighbour");
        assert!(hits[0].1 < 1e-2);

        let even = index
            .filtered_search(&unit(42, 32), 5, |k| k % 2 == 1)
            .unwrap();
        assert!(even.iter().all(|(k, _)| k % 2 == 1));

        assert_eq!(index.remove(&[42, 999_999]).unwrap(), 1);
        assert!(!index.contains(42));
        assert!(
            index
                .search(&unit(42, 32), 5)
                .unwrap()
                .iter()
                .all(|(k, _)| *k != 42)
        );

        let wrong = index.add_batch(&[(5_000, vec![0.5; 31])], 1);
        assert!(wrong.is_err(), "dimensions are checked");
    }

    #[test]
    fn saves_and_loads_through_both_routes() {
        let dir = tempfile::tempdir().unwrap();
        let index = VectorIndex::new("m", 16).unwrap();
        let items: Vec<(u64, Vec<f32>)> = (1..=300).map(|k| (k, unit(k, 16))).collect();
        index.add_batch(&items, 1).unwrap();
        for (i, buffered) in [false, true].into_iter().enumerate() {
            let path = dir.path().join(format!("i{i}.usearch"));
            let inner = index.read();
            if buffered {
                let mut buffer = vec![0u8; inner.index.serialized_length()];
                inner.index.save_to_buffer(&mut buffer).unwrap();
                fs::write(&path, buffer).unwrap();
            } else {
                write_index(&inner.index, &path).unwrap();
            }
            drop(inner);
            let back = read_index(&path, 16).unwrap();
            assert_eq!(back.size(), 300);
            assert!(back.contains(7));
        }
    }

    #[test]
    fn a_truncated_file_fails_to_load_rather_than_crashing() {
        let dir = tempfile::tempdir().unwrap();
        let index = VectorIndex::new("m", 16).unwrap();
        let items: Vec<(u64, Vec<f32>)> = (1..=300).map(|k| (k, unit(k, 16))).collect();
        index.add_batch(&items, 1).unwrap();
        let path = dir.path().join("i.usearch");
        write_index(&index.read().index, &path).unwrap();
        let bytes = fs::read(&path).unwrap();
        fs::write(&path, &bytes[..bytes.len() / 3]).unwrap();
        assert!(read_index(&path, 16).is_err());
    }
}
