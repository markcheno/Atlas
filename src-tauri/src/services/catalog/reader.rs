//! Read-only connections for UI queries.
//!
//! Queries from the UI must not wait behind inventory batches on the writer
//! thread. In WAL mode readers proceed while the writer commits, so a small
//! pool of read-only connections serves `list_documents` and friends.

use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex};

use rusqlite::{Connection, OpenFlags};

use super::writer::BUSY_TIMEOUT;
use crate::error::{AtlasError, AtlasResult};

pub const POOL_SIZE: usize = 4;

pub struct CatalogReader {
    idle: Mutex<Vec<Connection>>,
    available: Condvar,
    path: PathBuf,
}

impl CatalogReader {
    /// Opens the pool. Call after the writer has applied migrations.
    pub fn open(path: &Path) -> AtlasResult<Self> {
        Self::open_sized(path, POOL_SIZE)
    }

    /// A pool of `size` connections (the knowledge store keeps two).
    pub fn open_sized(path: &Path, size: usize) -> AtlasResult<Self> {
        let mut conns = Vec::with_capacity(size);
        for _ in 0..size {
            conns.push(open_reader(path)?);
        }
        Ok(Self {
            idle: Mutex::new(conns),
            available: Condvar::new(),
            path: path.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Runs `f` with a pooled read-only connection, waiting for one if all
    /// are busy.
    pub fn read<R>(&self, f: impl FnOnce(&Connection) -> R) -> AtlasResult<R> {
        let conn = {
            let mut idle = self
                .idle
                .lock()
                .map_err(|_| AtlasError::Internal("catalog readers are unavailable"))?;
            loop {
                if let Some(conn) = idle.pop() {
                    break conn;
                }
                idle = self
                    .available
                    .wait(idle)
                    .map_err(|_| AtlasError::Internal("catalog readers are unavailable"))?;
            }
        };
        // Return the connection even if `f` panics.
        struct Return<'a>(&'a CatalogReader, Option<Connection>);
        impl Drop for Return<'_> {
            fn drop(&mut self) {
                if let (Some(conn), Ok(mut idle)) = (self.1.take(), self.0.idle.lock()) {
                    idle.push(conn);
                    self.0.available.notify_one();
                }
            }
        }
        let guard = Return(self, Some(conn));
        Ok(f(guard.1.as_ref().expect("connection present")))
    }
}

/// One read-only, query-only connection outside the pool, for slow reads
/// that must not hold a pooled connection (storage accounting).
pub fn open_single(path: &Path) -> AtlasResult<Connection> {
    open_reader(path)
}

fn open_reader(path: &Path) -> AtlasResult<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(AtlasError::db("be opened for reading"))?;
    conn.busy_timeout(BUSY_TIMEOUT)
        .map_err(AtlasError::db("be configured"))?;
    conn.pragma_update(None, "query_only", true)
        .map_err(AtlasError::db("be configured"))?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::catalog::writer::{CatalogWriter, enable_wal};

    #[test]
    fn readers_see_committed_writes_and_cannot_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite");
        let writer = CatalogWriter::open(&path).unwrap();
        writer.call(enable_wal).unwrap().unwrap();
        writer
            .call(|c| c.execute_batch("CREATE TABLE t(n INTEGER); INSERT INTO t VALUES (1)"))
            .unwrap()
            .unwrap();

        let reader = CatalogReader::open(&path).unwrap();
        let n: i64 = reader
            .read(|c| c.query_row("SELECT n FROM t", [], |r| r.get(0)).unwrap())
            .unwrap();
        assert_eq!(n, 1);
        let refused = reader
            .read(|c| c.execute("INSERT INTO t VALUES (2)", []).is_err())
            .unwrap();
        assert!(refused, "readers are read-only");

        writer
            .call(|c| c.execute("INSERT INTO t VALUES (3)", []))
            .unwrap()
            .unwrap();
        let count: i64 = reader
            .read(|c| {
                c.query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0))
                    .unwrap()
            })
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn pool_serves_concurrent_readers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite");
        let writer = CatalogWriter::open(&path).unwrap();
        writer.call(enable_wal).unwrap().unwrap();
        writer
            .call(|c| c.execute_batch("CREATE TABLE t(n INTEGER)"))
            .unwrap()
            .unwrap();
        let reader = std::sync::Arc::new(CatalogReader::open(&path).unwrap());
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let r = reader.clone();
                std::thread::spawn(move || {
                    r.read(|c| c.query_row("SELECT COUNT(*) FROM t", [], |r| r.get::<_, i64>(0)))
                        .unwrap()
                        .unwrap()
                })
            })
            .collect();
        for h in handles {
            assert_eq!(h.join().unwrap(), 0);
        }
    }
}
