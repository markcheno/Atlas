//! Storage accounting and cache cleaning (Settings › Storage, phase1-spec
//! §5.5, §8; the meaning index, phase2-spec §5.6). Sizes come from the
//! files and from SQLite's `dbstat`, on a connection of their own so
//! measuring never holds the workspace or a pooled reader. Clean caches
//! removes only `cache/` and `tmp/` contents, never the meaning index.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::domain::dto::{IndexingEstimate, MeaningEstimate, StorageUsage};
use crate::domain::traits::VolumeInspector;
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::clock::now_rfc3339;
use crate::services::catalog::reader;
use crate::services::embedding::index::VECTORS_DIR;
use crate::services::embedding::models;
use crate::services::extraction::db::EFFECTIVE_STATE_SQL;
use crate::services::workspace::layout::{CATALOG, DIRECTORIES, TMP};

/// Folders whose contents are rebuildable and safe to remove.
pub const CACHE_DIRS: &[&str] = &["cache", TMP];
/// Documents read before an estimate is shown (phase1-spec §8).
pub const ESTIMATE_SAMPLES: u32 = 50;
/// The estimate's range: ±40% (phase1-spec §8).
const SPREAD: f64 = 0.4;

/// Bytes under `dir`, never following symbolic links.
fn tree_bytes(dir: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let Ok(meta) = fs::symlink_metadata(e.path()) else {
                continue;
            };
            if meta.is_dir() {
                stack.push(e.path());
            } else {
                total += meta.len();
            }
        }
    }
    total
}

fn file_bytes(path: &Path) -> u64 {
    fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// The catalog file with its write-ahead log and shared memory.
fn catalog_bytes(root: &Path) -> u64 {
    let db = root.join(CATALOG);
    let mut total = file_bytes(&db);
    for suffix in ["-wal", "-shm"] {
        let mut side = db.clone().into_os_string();
        side.push(suffix);
        total += file_bytes(Path::new(&side));
    }
    total
}

struct Measured {
    text: u64,
    index: u64,
    done: u32,
    pending: u32,
    meaning: u64,
    /// Characters and files still waiting for the active model's meaning,
    /// and its dimensions; `None` without a model.
    waiting: Option<(u64, u32, usize)>,
}

/// The catalog tables and indexes that make up the meaning index.
fn is_meaning(name: &str) -> bool {
    const TABLES: &[&str] = &[
        "chunks",
        "embeddings",
        "document_embeddings",
        "vector_tombstones",
        "embedding_models",
    ];
    TABLES.contains(&name)
        || name.starts_with("chunks_")
        || name.starts_with("document_embeddings_")
        || TABLES
            .iter()
            .any(|t| name.starts_with(&format!("sqlite_autoindex_{t}_")))
}

/// Characters of text per chunk: the 1,200-character chunk less its
/// 150-character overlap (Slice 2.2).
const CHARS_PER_CHUNK: f64 = 1_050.0;
/// The meaning estimate's range: ±25%.
const MEANING_SPREAD: f64 = 0.25;

/// What one chunk adds at `dims` dimensions: the f16 vector and its rows
/// in the catalog (2,529 bytes at 1,024 with 16 KiB pages, Slice 2.2), and
/// its share of the index file (0.536 of the raw f32 vector).
fn bytes_per_chunk(dims: usize) -> f64 {
    let catalog = dims as f64 * 2.0 + 481.0;
    let file = dims as f64 * 4.0 * 0.536;
    catalog + file
}

/// What indexing `chars` characters of text in `files` files will add.
pub fn meaning_estimate(chars: u64, files: u32, dims: usize) -> MeaningEstimate {
    let expected = (chars as f64 / CHARS_PER_CHUNK).ceil() * bytes_per_chunk(dims);
    MeaningEstimate {
        remaining_files: files,
        low_bytes: (expected * (1.0 - MEANING_SPREAD)) as u64,
        high_bytes: (expected * (1.0 + MEANING_SPREAD)) as u64,
    }
}

fn measure_catalog(conn: &Connection) -> rusqlite::Result<Measured> {
    let mut text = 0u64;
    let mut index = 0u64;
    let mut meaning = 0u64;
    let mut stmt = conn.prepare("SELECT name, pgsize FROM dbstat WHERE aggregate = TRUE")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
        let (name, size) = row?;
        let size = size.max(0) as u64;
        if name == "passages" || name.starts_with("sqlite_autoindex_passages") {
            text += size;
        } else if name.starts_with("passages_fts") {
            index += size;
        } else if is_meaning(&name) {
            meaning += size;
        }
    }
    let done: u32 = conn.query_row(
        "SELECT COUNT(*) FROM documents WHERE state = 'done'",
        [],
        |r| r.get(0),
    )?;
    let pending: u32 = conn.query_row(
        &format!(
            "SELECT COUNT(*) FROM file_instances f LEFT JOIN documents d ON d.id = f.document_id
             WHERE f.availability = 'available' AND ({EFFECTIVE_STATE_SQL}) = 'pending'"
        ),
        [],
        |r| r.get(0),
    )?;
    let waiting = match (models::role(conn)?, models::active(conn)?) {
        (Some(_), Some(model)) => {
            let (chars, files): (i64, i64) = conn.query_row(
                "SELECT COALESCE(SUM(d.char_count), 0),
                        (SELECT COUNT(*) FROM file_instances f
                         WHERE f.availability = 'available' AND f.document_id IN
                             (SELECT d2.id FROM documents d2 WHERE d2.state = 'done'
                              AND NOT EXISTS (SELECT 1 FROM document_embeddings de
                                  WHERE de.document_id = d2.id AND de.model_id = ?1)))
                 FROM documents d
                 WHERE d.state = 'done'
                   AND NOT EXISTS (SELECT 1 FROM document_embeddings de
                                   WHERE de.document_id = d.id AND de.model_id = ?1)",
                [&model.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            Some((chars.max(0) as u64, files.max(0) as u32, model.dims))
        }
        _ => None,
    };
    Ok(Measured {
        text,
        index,
        done,
        pending,
        meaning,
        waiting,
    })
}

/// What the files still to be read will add, from what reading has added
/// so far per document. Pending files are counted before copies are known,
/// so the range leans high.
pub fn estimate(text_and_index: u64, done: u32, pending: u32) -> IndexingEstimate {
    if pending == 0 {
        return IndexingEstimate::AllRead;
    }
    if done < ESTIMATE_SAMPLES {
        return IndexingEstimate::NotEnoughRead {
            done,
            needed: ESTIMATE_SAMPLES,
        };
    }
    let expected = text_and_index as f64 / f64::from(done) * f64::from(pending);
    IndexingEstimate::Range {
        remaining_files: pending,
        low_bytes: (expected * (1.0 - SPREAD)) as u64,
        high_bytes: (expected * (1.0 + SPREAD)) as u64,
    }
}

/// Storage use of the library at `root`.
pub fn usage(root: &Path, volumes: &dyn VolumeInspector) -> AtlasResult<StorageUsage> {
    let conn = reader::open_single(&root.join(CATALOG))?;
    let m = measure_catalog(&conn).map_err(AtlasError::db("measure storage"))?;
    drop(conn);
    let catalog = catalog_bytes(root);
    Ok(StorageUsage {
        catalog_bytes: catalog,
        text_bytes: m.text,
        index_bytes: m.index,
        other_catalog_bytes: catalog.saturating_sub(m.text + m.index + m.meaning),
        cache_bytes: CACHE_DIRS.iter().map(|d| tree_bytes(&root.join(d))).sum(),
        data_volume_free_bytes: volumes.inspect(root).ok().and_then(|v| v.free_bytes),
        estimate: estimate(m.text + m.index, m.done, m.pending),
        meaning_catalog_bytes: m.meaning,
        meaning_file_bytes: tree_bytes(&root.join(VECTORS_DIR)),
        meaning_estimate: m
            .waiting
            .map(|(chars, files, dims)| meaning_estimate(chars, files, dims)),
        measured_at: now_rfc3339(),
    })
}

/// Empties `cache/` and `tmp/`, removing symbolic links themselves and
/// never what they point to. The library's own folders (`cache/previews`)
/// stay. Returns the bytes freed. Call with the workspace held, so nothing
/// is staging in `tmp/`.
pub fn clean_caches(root: &Path) -> AtlasResult<u64> {
    let keep: Vec<PathBuf> = DIRECTORIES.iter().map(|d| root.join(d)).collect();
    let mut freed = 0;
    for dir in CACHE_DIRS {
        freed += clear_counted(&root.join(dir), &keep).map_err(AtlasError::io("clean caches"))?;
    }
    Ok(freed)
}

fn clear_counted(dir: &Path, keep: &[PathBuf]) -> io::Result<u64> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err),
    };
    let mut freed = 0;
    for entry in entries {
        let path = entry?.path();
        let meta = fs::symlink_metadata(&path)?;
        if meta.is_dir() {
            freed += clear_counted(&path, keep)?;
            if !keep.contains(&path) {
                fs::remove_dir(&path)?;
            }
        } else {
            freed += meta.len();
            fs::remove_file(&path)?;
        }
    }
    Ok(freed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_meaning_estimate_scales_with_text_and_dimensions() {
        // 30,000 files of 20 KiB of text at 1,024 dimensions: about 0.6
        // million chunks, about 2.8 GB with the index file.
        let e = meaning_estimate(30_000 * 20_480, 30_000, 1024);
        assert_eq!(e.remaining_files, 30_000);
        let mid = (e.low_bytes + e.high_bytes) / 2;
        assert!((2_500_000_000..3_200_000_000).contains(&mid), "{mid}");
        assert!(e.high_bytes > e.low_bytes);
        let half = meaning_estimate(30_000 * 20_480, 30_000, 512);
        assert!(
            half.high_bytes < e.high_bytes * 6 / 10,
            "512 dimensions take about half"
        );
        assert_eq!(meaning_estimate(0, 0, 1024).high_bytes, 0);
        assert!(is_meaning("embeddings") && is_meaning("sqlite_autoindex_embeddings_1"));
        assert!(is_meaning("document_embeddings_by_seq") && is_meaning("chunks_by_passage"));
        assert!(!is_meaning("passages") && !is_meaning("documents"));
    }

    #[test]
    fn the_estimate_is_a_range_only_after_enough_samples() {
        assert_eq!(estimate(0, 0, 0), IndexingEstimate::AllRead);
        assert_eq!(
            estimate(1_000, 49, 10),
            IndexingEstimate::NotEnoughRead {
                done: 49,
                needed: 50
            }
        );
        // 50 documents took 5 MB: 100 more need about 10 MB, ±40%.
        assert_eq!(
            estimate(5_000_000, 50, 100),
            IndexingEstimate::Range {
                remaining_files: 100,
                low_bytes: 6_000_000,
                high_bytes: 14_000_000
            }
        );
    }
}
