//! Storage accounting and cache cleaning (Settings › Storage, phase1-spec
//! §5.5, §8). Sizes come from the files and from SQLite's `dbstat`, on a
//! connection of their own so measuring never holds the workspace or a
//! pooled reader. Clean caches removes only `cache/` and `tmp/` contents.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::domain::dto::{IndexingEstimate, StorageUsage};
use crate::domain::traits::VolumeInspector;
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::clock::now_rfc3339;
use crate::services::catalog::reader;
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
}

fn measure_catalog(conn: &Connection) -> rusqlite::Result<Measured> {
    let mut text = 0u64;
    let mut index = 0u64;
    let mut stmt = conn.prepare("SELECT name, pgsize FROM dbstat WHERE aggregate = TRUE")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
        let (name, size) = row?;
        let size = size.max(0) as u64;
        if name == "passages" || name.starts_with("sqlite_autoindex_passages") {
            text += size;
        } else if name.starts_with("passages_fts") {
            index += size;
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
    Ok(Measured {
        text,
        index,
        done,
        pending,
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
        other_catalog_bytes: catalog.saturating_sub(m.text + m.index),
        cache_bytes: CACHE_DIRS.iter().map(|d| tree_bytes(&root.join(d))).sum(),
        data_volume_free_bytes: volumes.inspect(root).ok().and_then(|v| v.free_bytes),
        estimate: estimate(m.text + m.index, m.done, m.pending),
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
