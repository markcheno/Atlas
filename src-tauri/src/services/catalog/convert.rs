//! Converting a catalog created before Slice 1.7 (4 KiB pages) to 16 KiB
//! pages before its first embedding (Slice 2.3, ADR 0014): a 4 KiB page
//! holds one 2 KB vector, so such a catalog would take 1.7 times the space
//! for the meaning index.
//!
//! Runs while a library opens, after its lock is taken and before any
//! other connection exists: a copy is written with `VACUUM INTO` at the new
//! page size, checked, and renamed over the catalog. A kill before the
//! rename leaves the old catalog untouched; the rename is atomic.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use super::writer::NEW_CATALOG_PAGE_SIZE;
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::fsutil::sync_dir;

const STAGED: &str = "catalog-16k.sqlite";

/// Whether the catalog at `path` should be converted now: smaller pages,
/// an embedding model chosen, and no vectors yet. Anything unreadable
/// means no (opening reports problems itself).
pub fn wanted(path: &Path) -> bool {
    let Ok(conn) = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        return false;
    };
    let page_size: u32 = conn
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .unwrap_or(NEW_CATALOG_PAGE_SIZE);
    if page_size >= NEW_CATALOG_PAGE_SIZE {
        return false;
    }
    let role: bool = conn
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM model_roles WHERE role = 'embedding')",
            [],
            |r| r.get(0),
        )
        .unwrap_or(false);
    let vectors: bool = conn
        .query_row("SELECT EXISTS (SELECT 1 FROM embeddings)", [], |r| r.get(0))
        .unwrap_or(true);
    role && !vectors
}

/// The catalog's size, with its write-ahead log.
pub fn size(path: &Path) -> u64 {
    let wal = PathBuf::from(format!("{}-wal", path.display()));
    [path, wal.as_path()]
        .iter()
        .filter_map(|p| fs::metadata(p).ok())
        .map(|m| m.len())
        .sum()
}

/// Rewrites the catalog at `path` with 16 KiB pages, staging the copy in
/// `staging`. Must run with no other connection to the catalog open.
pub fn to_large_pages(path: &Path, staging: &Path) -> AtlasResult<()> {
    fs::create_dir_all(staging).map_err(AtlasError::io("prepare the library"))?;
    let staged = staging.join(STAGED);
    let _ = fs::remove_file(&staged);
    let staged_text = staged.to_str().ok_or(AtlasError::InvalidPath)?.to_owned();
    {
        let conn = Connection::open(path).map_err(AtlasError::db("be opened"))?;
        // Everything in the log goes into the file first.
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .map_err(AtlasError::db("be prepared for meaning search"))?;
        conn.pragma_update(None, "page_size", NEW_CATALOG_PAGE_SIZE)
            .map_err(AtlasError::db("be prepared for meaning search"))?;
        conn.execute("VACUUM INTO ?1", [&staged_text])
            .map_err(AtlasError::db("be prepared for meaning search"))?;
    }
    let checked = (|| {
        let copy = Connection::open_with_flags(&staged, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let page: u32 = copy.query_row("PRAGMA page_size", [], |r| r.get(0))?;
        let ok: String = copy.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        Ok::<_, rusqlite::Error>(page == NEW_CATALOG_PAGE_SIZE && ok == "ok")
    })();
    if !matches!(checked, Ok(true)) {
        let _ = fs::remove_file(&staged);
        return Err(AtlasError::DataRootInvalid {
            reason: "its catalog could not be prepared for meaning search",
        });
    }
    // The old log was checkpointed and closed; it must not be applied to
    // the new file.
    for suffix in ["-wal", "-shm"] {
        let side = PathBuf::from(format!("{}{suffix}", path.display()));
        match fs::metadata(&side) {
            Ok(m) if suffix == "-wal" && m.len() > 0 => {
                let _ = fs::remove_file(&staged);
                return Err(AtlasError::DataRootInvalid {
                    reason: "its catalog could not be prepared for meaning search",
                });
            }
            Ok(_) => fs::remove_file(&side).map_err(AtlasError::io("prepare the library"))?,
            Err(_) => {}
        }
    }
    fs::rename(&staged, path).map_err(AtlasError::io("prepare the library"))?;
    if let Some(parent) = path.parent() {
        sync_dir(parent).map_err(AtlasError::io("prepare the library"))?;
    }
    Ok(())
}
