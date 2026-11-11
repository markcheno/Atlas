//! Opening the knowledge store, its writer and readers, its status, and
//! the thread that checks every hour for a snapshot and for trails to
//! expire.
//!
//! Opening never fails the library. A store that is newer than this build
//! (`incompatible`) or that isn't one Atlas can read (`damaged`) is left
//! byte for byte as it is, and every use of it is refused with
//! [`AtlasError::Knowledge`].

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rusqlite::Connection;

use super::{SNAPSHOT_STAGING_DIR, SNAPSHOTS_DIR, TrailSettings, migrations, snapshot, trails};
use crate::domain::dto::{KnowledgeState, KnowledgeStatus, SnapshotOutcome, SnapshotStatus};
use crate::domain::traits::VolumeInspector;
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::clock::Clock;
use crate::services::catalog::reader::CatalogReader;
use crate::services::catalog::writer::{self, CatalogHandle, CatalogWriter};
use crate::services::jobs::Signal;
use crate::services::workspace::layout::KNOWLEDGE;

/// Submits closures to the knowledge store's writer thread.
pub type KnowledgeHandle = CatalogHandle;

/// Read-only connections to the store: few, since its reads are small.
const READERS: usize = 2;
/// How often an open library checks whether a snapshot (and expiry) is
/// due.
pub const CHECK_EVERY: Duration = Duration::from_secs(60 * 60);

/// What the snapshot thread and the store share.
pub struct Inner {
    pub(super) state: KnowledgeState,
    // The reader closes before the writer, which closes last.
    reader: Option<CatalogReader>,
    writer: Option<CatalogWriter>,
    pub(super) path: PathBuf,
    pub(super) root: PathBuf,
    pub(super) clock: Arc<dyn Clock>,
    pub(super) volumes: Arc<dyn VolumeInspector>,
    /// Recording and expiry, as Settings › Privacy has them (Slice 3.3).
    pub(super) trails: Arc<TrailSettings>,
    /// The last snapshot check since the library opened.
    pub(super) last: Mutex<Option<SnapshotOutcome>>,
    /// One snapshot at a time (the open check and the hourly one).
    pub(super) snapshotting: Mutex<()>,
}

impl Inner {
    pub(super) fn writer(&self, context: &'static str) -> AtlasResult<&CatalogWriter> {
        self.writer.as_ref().ok_or(AtlasError::Knowledge(context))
    }

    pub fn snapshots_dir(&self) -> PathBuf {
        self.root.join(SNAPSHOTS_DIR)
    }

    pub fn staging_dir(&self) -> PathBuf {
        self.root.join(SNAPSHOT_STAGING_DIR)
    }

    pub fn state(&self) -> KnowledgeState {
        self.state
    }

    pub fn clock(&self) -> &Arc<dyn Clock> {
        &self.clock
    }

    pub fn trail_settings(&self) -> &Arc<TrailSettings> {
        &self.trails
    }

    /// Runs `f` on the writer thread, refused unless the store is ready.
    pub fn call<R, F>(&self, f: F) -> AtlasResult<R>
    where
        R: Send + 'static,
        F: FnOnce(&mut Connection) -> R + Send + 'static,
    {
        self.writer("save")?.call(f)
    }

    /// Runs `f` with a read-only connection, refused unless the store is
    /// ready.
    pub fn read<R>(&self, f: impl FnOnce(&Connection) -> R) -> AtlasResult<R> {
        self.reader
            .as_ref()
            .ok_or(AtlasError::Knowledge("read"))?
            .read(f)
    }
}

/// The open knowledge store of one library. Dropping it stops the
/// snapshot thread, then closes the readers and the writer.
pub struct KnowledgeStore {
    ticker: Option<Ticker>,
    inner: Arc<Inner>,
}

impl KnowledgeStore {
    /// Opens (creating if needed) the store of the library at `root`, and
    /// starts the hourly snapshot check. Never fails: problems become the
    /// store's state. Call with the library's lock held.
    pub fn open(
        root: &Path,
        clock: Arc<dyn Clock>,
        volumes: Arc<dyn VolumeInspector>,
        trails: Arc<TrailSettings>,
    ) -> KnowledgeStore {
        let path = root.join(KNOWLEDGE);
        let now = clock.now_rfc3339();
        let (state, writer, reader) = match open_files(&path, now) {
            Ok((writer, reader)) => (KnowledgeState::Ready, Some(writer), Some(reader)),
            Err(state) => (state, None, None),
        };
        let inner = Arc::new(Inner {
            state,
            reader,
            writer,
            path,
            root: root.to_path_buf(),
            clock,
            volumes,
            trails,
            last: Mutex::new(None),
            snapshotting: Mutex::new(()),
        });
        snapshot::clean_staging(&inner.staging_dir());
        let ticker = (state == KnowledgeState::Ready)
            .then(|| Ticker::start(Arc::clone(&inner)))
            .flatten();
        KnowledgeStore { ticker, inner }
    }

    pub fn state(&self) -> KnowledgeState {
        self.inner.state
    }

    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    /// What the snapshot functions work on.
    pub fn inner(&self) -> &Inner {
        &self.inner
    }

    /// The store, for what outlives a borrow of it: the recorder, and an
    /// answer's thread through it.
    pub fn shared(&self) -> Arc<Inner> {
        Arc::clone(&self.inner)
    }

    pub fn trail_settings(&self) -> &Arc<TrailSettings> {
        &self.inner.trails
    }

    /// A handle to the writer thread, refused unless the store is ready.
    pub fn handle(&self) -> AtlasResult<KnowledgeHandle> {
        Ok(self.inner.writer("save")?.handle())
    }

    /// Runs `f` on the writer thread, refused unless the store is ready.
    pub fn call<R, F>(&self, f: F) -> AtlasResult<R>
    where
        R: Send + 'static,
        F: FnOnce(&mut Connection) -> R + Send + 'static,
    {
        self.inner.call(f)
    }

    /// Runs `f` with a read-only connection, refused unless the store is
    /// ready.
    pub fn read<R>(&self, f: impl FnOnce(&Connection) -> R) -> AtlasResult<R> {
        self.inner.read(f)
    }

    /// Takes a snapshot if one is due (phase3-spec §6.3). `None` when the
    /// store isn't ready.
    pub fn snapshot_if_due(&self) -> Option<SnapshotOutcome> {
        snapshot::if_due(&self.inner)
    }

    /// Removes expired trails if a day has passed since the last time
    /// (phase3-spec §7.1). The number removed, or `None` when it wasn't
    /// due, is switched off, or the store isn't ready.
    pub fn expire_if_due(&self) -> Option<u32> {
        trails::expire_if_due(&self.inner)
    }

    /// Settings › Storage's view of the store.
    pub fn status(&self) -> KnowledgeStatus {
        let counts = self
            .read(|c| {
                c.query_row(
                    "SELECT (SELECT count(*) FROM trails), (SELECT count(*) FROM trail_steps),
                            (SELECT count(*) FROM bookmarks), (SELECT count(*) FROM findings),
                            (SELECT count(*) FROM collections)",
                    [],
                    |r| Ok([r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?]),
                )
            })
            .ok()
            .and_then(Result::ok)
            .unwrap_or([0u32; 5]);
        let snaps = snapshot::list(&self.inner.snapshots_dir());
        KnowledgeStatus {
            state: self.inner.state,
            bytes: store_bytes(&self.inner.path),
            trails: counts[0],
            steps: counts[1],
            bookmarks: counts[2],
            findings: counts[3],
            collections: counts[4],
            snapshots: SnapshotStatus {
                count: snaps.len() as u32,
                bytes: snaps.iter().map(|s| s.bytes).sum(),
                newest_at: snaps
                    .last()
                    .map(|s| crate::infrastructure::clock::rfc3339(s.at)),
                last_outcome: *self.inner.last.lock().unwrap_or_else(|p| p.into_inner()),
            },
        }
    }
}

impl Drop for KnowledgeStore {
    fn drop(&mut self) {
        // The thread holds the other reference to `inner`; once it has
        // stopped, `inner` (and with it the writer) closes here.
        drop(self.ticker.take());
    }
}

/// The store's file with its write-ahead log and shared memory.
pub fn store_bytes(path: &Path) -> u64 {
    let mut total = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    for suffix in ["-wal", "-shm"] {
        total += fs::metadata(format!("{}{suffix}", path.display()))
            .map(|m| m.len())
            .unwrap_or(0);
    }
    total
}

/// Opens the writer, checks the version, migrates and opens the readers.
/// The error is the state to report; nothing has been written then.
fn open_files(path: &Path, now: String) -> Result<(CatalogWriter, CatalogReader), KnowledgeState> {
    let damaged = |_| KnowledgeState::Damaged;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| KnowledgeState::Damaged)?;
    }
    let writer = CatalogWriter::open_named(path, "atlas-knowledge-writer").map_err(damaged)?;
    let (version, objects) = writer
        .call(|c| {
            Ok::<_, rusqlite::Error>((migrations::current_version(c)?, migrations::has_objects(c)?))
        })
        .map_err(damaged)?
        .map_err(|_| KnowledgeState::Damaged)?;
    if version > migrations::SUPPORTED_KNOWLEDGE_VERSION {
        return Err(KnowledgeState::Incompatible);
    }
    if version == 0 && objects {
        // Some other SQLite file: not Atlas's to change.
        return Err(KnowledgeState::Damaged);
    }
    writer
        .call(writer::enable_wal)
        .map_err(damaged)?
        .map_err(damaged)?;
    writer
        .call(move |c| migrations::migrate(c, &now))
        .map_err(damaged)?
        .map_err(|_| KnowledgeState::Damaged)?;
    let reader = CatalogReader::open_sized(path, READERS).map_err(damaged)?;
    Ok((writer, reader))
}

/// The hourly snapshot check. Dropping it stops the thread and waits.
struct Ticker {
    signal: Arc<Signal>,
    thread: Option<JoinHandle<()>>,
}

impl Ticker {
    fn start(inner: Arc<Inner>) -> Option<Ticker> {
        let signal = Arc::new(Signal::new());
        let stop = Arc::clone(&signal);
        let thread = thread::Builder::new()
            .name("atlas-knowledge-snapshots".into())
            .spawn(move || {
                while !stop.wait(CHECK_EVERY) {
                    // A snapshot first, so it holds what expiry removes.
                    snapshot::if_due(&inner);
                    trails::expire_if_due(&inner);
                }
            })
            .ok()?;
        Some(Ticker {
            signal,
            thread: Some(thread),
        })
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        self.signal.request_stop();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
