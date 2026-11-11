//! One background thread per job kind and open data root (ADR 0009).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::db;
use super::{Flow, ProcessorContext, ProcessorFactory};
use crate::domain::dto::JobKind;
use crate::error::{AtlasError, AtlasResult};
use crate::services::catalog::codec::to_db;
use crate::services::catalog::writer::CatalogHandle;
use crate::services::embedding::registry::VectorRegistry;

/// Wakes a runner early (user commands) and tells it to stop (shutdown).
pub struct Signal {
    stop: AtomicBool,
    seq: Mutex<u64>,
    cv: Condvar,
}

impl Signal {
    pub(crate) fn new() -> Self {
        Self {
            stop: AtomicBool::new(false),
            seq: Mutex::new(0),
            cv: Condvar::new(),
        }
    }

    pub fn stopped(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// Sleeps up to `d`, returning early on a wake or stop. Returns whether
    /// a stop was requested.
    pub fn wait(&self, d: Duration) -> bool {
        if d.is_zero() {
            return self.stopped();
        }
        let guard = self.seq.lock().unwrap_or_else(|p| p.into_inner());
        let start = *guard;
        let _ = self
            .cv
            .wait_timeout_while(guard, d, |seq| *seq == start && !self.stopped());
        self.stopped()
    }

    pub(crate) fn notify(&self) {
        *self.seq.lock().unwrap_or_else(|p| p.into_inner()) += 1;
        self.cv.notify_all();
    }

    pub(crate) fn request_stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.notify();
    }
}

/// Owns one runner thread. Dropping it stops the thread and waits for it, so
/// it never outlives the catalog it writes.
pub struct JobRunner {
    signal: Arc<Signal>,
    thread: Option<JoinHandle<()>>,
}

/// Wakes every runner of one open data root, for example the extraction
/// runner when a scan finishes with files to read.
#[derive(Clone, Default)]
pub struct JobWaker(Arc<Vec<Arc<Signal>>>);

impl JobWaker {
    pub fn wake(&self) {
        for signal in self.0.iter() {
            signal.notify();
        }
    }
}

impl JobRunner {
    /// Starts the runner for `kind` with its own `signal`. The processor is
    /// built on the runner's own thread, so it is also dropped there before
    /// the thread ends.
    fn start(
        kind: JobKind,
        catalog: CatalogHandle,
        data_root: &Path,
        factory: ProcessorFactory,
        signal: Arc<Signal>,
        waker: JobWaker,
        vectors: Arc<VectorRegistry>,
    ) -> AtlasResult<Self> {
        let ctx = ProcessorContext {
            catalog: catalog.clone(),
            data_root: data_root.to_path_buf(),
            signal: Arc::clone(&signal),
            waker,
            vectors,
        };
        let loop_signal = Arc::clone(&signal);
        let thread = thread::Builder::new()
            .name(format!("atlas-job-{}", to_db(&kind)))
            .spawn(move || run(kind, catalog, &loop_signal, factory(ctx)))
            .map_err(AtlasError::io("start background work"))?;
        Ok(Self {
            signal,
            thread: Some(thread),
        })
    }

    /// Asks the runner to re-read job states now instead of at its next
    /// check, for example after a user pauses or adds a folder.
    pub fn wake(&self) {
        self.signal.notify();
    }
}

impl Drop for JobRunner {
    fn drop(&mut self) {
        self.signal.request_stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Phase 0's worker loop, for any kind: run the next runnable job, sleep
/// when there is none, back off on database errors, and stop when the
/// catalog closes.
fn run(
    kind: JobKind,
    catalog: CatalogHandle,
    signal: &Signal,
    mut processor: Box<dyn super::JobProcessor>,
) {
    processor.started();
    let mut db_failures = 0u32;
    while !signal.stopped() {
        match catalog.call(move |c| db::next_runnable(c, kind)) {
            Ok(Ok(Some(job))) => {
                db_failures = 0;
                if let Flow::Stop = processor.run(job) {
                    break;
                }
            }
            Ok(Ok(None)) => {
                if signal.wait(processor.idle_wait()) {
                    break;
                }
            }
            Ok(Err(_)) => {
                db_failures += 1;
                let backoff = Duration::from_secs(1 << db_failures.min(5));
                if signal.wait(backoff) {
                    break;
                }
            }
            // The writer is gone: the root is closing.
            Err(_) => break,
        }
    }
}

/// The runners of one open data root, at most one per kind (two runners of
/// one kind would both run the same job).
#[derive(Default)]
pub struct JobRunners {
    runners: BTreeMap<JobKind, JobRunner>,
}

impl JobRunners {
    pub fn start(
        catalog: &CatalogHandle,
        data_root: &Path,
        factories: &BTreeMap<JobKind, ProcessorFactory>,
        vectors: &Arc<VectorRegistry>,
    ) -> AtlasResult<Self> {
        // Every signal exists before any runner starts, so each processor's
        // waker reaches all of them.
        let signals: Vec<(JobKind, Arc<Signal>)> = factories
            .keys()
            .map(|kind| (*kind, Arc::new(Signal::new())))
            .collect();
        let waker = JobWaker(Arc::new(
            signals.iter().map(|(_, s)| Arc::clone(s)).collect(),
        ));
        let mut runners = BTreeMap::new();
        for (kind, signal) in signals {
            let factory = Arc::clone(&factories[&kind]);
            runners.insert(
                kind,
                JobRunner::start(
                    kind,
                    catalog.clone(),
                    data_root,
                    factory,
                    signal,
                    waker.clone(),
                    Arc::clone(vectors),
                )?,
            );
        }
        Ok(Self { runners })
    }

    pub fn wake_all(&self) {
        for runner in self.runners.values() {
            runner.wake();
        }
    }
}

impl Drop for JobRunners {
    /// Tells every runner to stop before waiting for any, so closing a root
    /// takes as long as the slowest runner, not the sum.
    fn drop(&mut self) {
        for runner in self.runners.values() {
            runner.signal.request_stop();
        }
        self.runners.clear();
    }
}
