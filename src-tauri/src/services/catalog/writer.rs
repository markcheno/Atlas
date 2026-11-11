//! The catalog's single writer.
//!
//! Exactly one read-write SQLite connection exists per open data root, and
//! it lives on a dedicated OS thread. Every catalog mutation is sent to that
//! thread as a closure and runs there in arrival order, so writes never
//! contend with each other and `SQLITE_BUSY` can only come from readers.
//! Read-only connections for UI queries arrive in Slice 0.6.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rusqlite::Connection;

use crate::error::{AtlasError, AtlasResult};

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

/// Provisional; revisited with Slice 0.8 measurements.
pub const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// A cloneable way to submit jobs to the writer thread. The inventory
/// worker holds one; it must be dropped before the [`CatalogWriter`], whose
/// `Drop` waits for every handle to go away.
#[derive(Clone)]
pub struct CatalogHandle {
    tx: mpsc::Sender<Job>,
}

impl CatalogHandle {
    /// Runs `f` on the writer thread and waits for its result.
    /// Call from blocking contexts only (never on an async executor thread).
    pub fn call<R, F>(&self, f: F) -> AtlasResult<R>
    where
        R: Send + 'static,
        F: FnOnce(&mut Connection) -> R + Send + 'static,
    {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        let job: Job = Box::new(move |conn| {
            let _ = reply_tx.send(f(conn));
        });
        self.tx
            .send(job)
            .map_err(|_| AtlasError::Internal("catalog writer is not running"))?;
        reply_rx
            .recv()
            .map_err(|_| AtlasError::Internal("catalog writer stopped unexpectedly"))
    }
}

pub struct CatalogWriter {
    handle: Option<CatalogHandle>,
    thread: Option<JoinHandle<()>>,
    path: PathBuf,
}

impl std::ops::Deref for CatalogWriter {
    type Target = CatalogHandle;
    fn deref(&self) -> &CatalogHandle {
        self.handle
            .as_ref()
            .expect("catalog handle is present until drop")
    }
}

impl CatalogWriter {
    /// Opens (creating if needed) the database at `path` on a new writer
    /// thread. Only connection-level settings are applied here; nothing is
    /// written to the database file until a job asks for it, so a catalog
    /// from a newer Atlas can be inspected and refused untouched.
    pub fn open(path: &Path) -> AtlasResult<Self> {
        Self::open_named(path, "atlas-catalog-writer")
    }

    /// As [`open`](Self::open), with the writer thread named `thread`: the
    /// knowledge store (Phase 3) runs its own writer on this same code.
    pub fn open_named(path: &Path, thread: &str) -> AtlasResult<Self> {
        let (tx, rx) = mpsc::channel::<Job>();
        let (ready_tx, ready_rx) = mpsc::sync_channel::<AtlasResult<()>>(1);
        let db_path = path.to_path_buf();

        let thread = thread::Builder::new()
            .name(thread.into())
            .spawn(move || {
                let mut conn = match open_connection(&db_path) {
                    Ok(conn) => {
                        let _ = ready_tx.send(Ok(()));
                        conn
                    }
                    Err(err) => {
                        let _ = ready_tx.send(Err(err));
                        return;
                    }
                };
                for job in rx {
                    job(&mut conn);
                }
                // `conn` drops here, closing the database before the thread ends.
            })
            .map_err(AtlasError::io("start the catalog writer"))?;

        ready_rx
            .recv()
            .map_err(|_| AtlasError::Internal("catalog writer exited during startup"))??;

        Ok(Self {
            handle: Some(CatalogHandle { tx }),
            thread: Some(thread),
            path: path.to_path_buf(),
        })
    }

    /// Another handle to the same writer thread.
    pub fn handle(&self) -> CatalogHandle {
        (**self).clone()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for CatalogWriter {
    /// Closes the channel and waits for the thread, so the database is
    /// fully closed before anything that outlives the writer (such as the
    /// data-root lock) is released.
    fn drop(&mut self) {
        drop(self.handle.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn open_connection(path: &Path) -> AtlasResult<Connection> {
    let conn = Connection::open(path).map_err(AtlasError::db("be opened"))?;
    conn.busy_timeout(BUSY_TIMEOUT)
        .map_err(AtlasError::db("be configured"))?;
    conn.pragma_update(None, "foreign_keys", true)
        .map_err(AtlasError::db("be configured"))?;
    Ok(conn)
}

/// The page size of a new catalog. Passages run 2–4 KB, so with SQLite's
/// default 4 KiB pages almost every passage filled a page of its own, about
/// a third of it unused (ADR 0010). Set before the switch to WAL, which
/// fixes the size; an existing catalog keeps the size it was created with.
pub const NEW_CATALOG_PAGE_SIZE: u32 = 16_384;

/// Chooses the page size for a catalog with nothing in it yet. On a
/// database that already has pages, SQLite ignores it.
pub fn set_new_page_size(conn: &mut Connection) -> AtlasResult<()> {
    conn.pragma_update(None, "page_size", NEW_CATALOG_PAGE_SIZE)
        .map_err(AtlasError::db("be configured"))
}

/// Switches the database to WAL with `synchronous=NORMAL` (corruption-safe
/// in WAL mode; at worst the last transaction is lost on power failure).
/// Called only after the schema version has been accepted.
pub fn enable_wal(conn: &mut Connection) -> AtlasResult<()> {
    let mode: String = conn
        .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
        .map_err(AtlasError::db("switch to write-ahead logging"))?;
    if !mode.eq_ignore_ascii_case("wal") {
        // Happens on filesystems without shared-memory support.
        return Err(AtlasError::DataRootNetworkVolume);
    }
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(AtlasError::db("be configured"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jobs_run_in_order_on_one_connection() {
        let dir = tempfile::tempdir().unwrap();
        let writer = CatalogWriter::open(&dir.path().join("t.sqlite")).unwrap();
        writer
            .call(|c| c.execute_batch("CREATE TABLE t(n INTEGER)"))
            .unwrap()
            .unwrap();
        for n in 0..10 {
            writer
                .call(move |c| c.execute("INSERT INTO t VALUES (?1)", [n]))
                .unwrap()
                .unwrap();
        }
        let ns: Vec<i64> = writer
            .call(|c| {
                let mut stmt = c.prepare("SELECT n FROM t ORDER BY rowid").unwrap();
                stmt.query_map([], |r| r.get(0))
                    .unwrap()
                    .collect::<Result<_, _>>()
                    .unwrap()
            })
            .unwrap();
        assert_eq!(ns, (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn wal_and_foreign_keys_are_active() {
        let dir = tempfile::tempdir().unwrap();
        let writer = CatalogWriter::open(&dir.path().join("t.sqlite")).unwrap();
        writer.call(enable_wal).unwrap().unwrap();
        let (mode, fk, sync): (String, i64, i64) = writer
            .call(|c| {
                (
                    c.query_row("PRAGMA journal_mode", [], |r| r.get(0))
                        .unwrap(),
                    c.query_row("PRAGMA foreign_keys", [], |r| r.get(0))
                        .unwrap(),
                    c.query_row("PRAGMA synchronous", [], |r| r.get(0)).unwrap(),
                )
            })
            .unwrap();
        assert_eq!(mode, "wal");
        assert_eq!(fk, 1);
        assert_eq!(sync, 1, "NORMAL");
    }

    #[test]
    fn panicking_job_reports_an_error_instead_of_hanging() {
        let dir = tempfile::tempdir().unwrap();
        let writer = CatalogWriter::open(&dir.path().join("t.sqlite")).unwrap();
        let result: AtlasResult<()> = writer.call(|_| panic!("boom"));
        assert!(matches!(result, Err(AtlasError::Internal(_))));
    }
}
