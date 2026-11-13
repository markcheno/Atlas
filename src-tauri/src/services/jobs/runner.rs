//! One background thread per job kind and open data root (ADR 0009).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::db;
use super::{Flow, ProcessorContext, ProcessorFactory};
use crate::domain::dto::{JobKind, JobState};
use crate::error::{AtlasError, AtlasResult};
use crate::services::catalog::codec::to_db;
use crate::services::catalog::writer::CatalogHandle;
use crate::services::diagnostics::{Diagnostics, Event, FailureReason};
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
    #[allow(clippy::too_many_arguments)]
    fn start(
        kind: JobKind,
        catalog: CatalogHandle,
        data_root: &Path,
        factory: ProcessorFactory,
        signal: Arc<Signal>,
        waker: JobWaker,
        vectors: Arc<VectorRegistry>,
        diagnostics: Diagnostics,
    ) -> AtlasResult<Self> {
        let ctx = ProcessorContext {
            catalog: catalog.clone(),
            data_root: data_root.to_path_buf(),
            signal: Arc::clone(&signal),
            waker,
            vectors,
            diagnostics: diagnostics.clone(),
        };
        let loop_signal = Arc::clone(&signal);
        let thread = thread::Builder::new()
            .name(format!("atlas-job-{}", to_db(&kind)))
            .spawn(move || run(kind, catalog, &loop_signal, factory(ctx), &diagnostics))
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
    diagnostics: &Diagnostics,
) {
    processor.started();
    let mut db_failures = 0u32;
    while !signal.stopped() {
        match catalog.call(move |c| db::next_runnable(c, kind)) {
            Ok(Ok(Some(job))) => {
                db_failures = 0;
                let (id, before) = (job.id.clone(), items(&catalog, &job.id));
                // A job resumed after a policy pause is the same job going
                // on; only a fresh start is logged as one.
                let fresh = job.state == JobState::Queued;
                if fresh {
                    diagnostics.record(Event::JobStarted { kind });
                }
                let started = Instant::now();
                let flow = processor.run(job);
                let after = catalog
                    .call({
                        let id = id.clone();
                        move |c| db::job(c, &id)
                    })
                    .ok()
                    .and_then(Result::ok)
                    .flatten();
                // Only a failure has a reason: `last_error` outlives the
                // problem on a job that went on to finish.
                let reason = after
                    .as_ref()
                    .filter(|j| {
                        matches!(
                            j.state,
                            JobState::FailedTerminal | JobState::FailedRetryable
                        )
                    })
                    .and_then(|j| j.last_error.as_deref())
                    .map(FailureReason::of_code);
                diagnostics.record(Event::JobEnded {
                    kind,
                    state: after.as_ref().map(|j| j.state),
                    ms: started.elapsed().as_millis() as u64,
                    items: items(&catalog, &id).saturating_sub(before),
                    errors: after.map_or(0, |j| u64::from(j.error_count)),
                    reason,
                });
                if let Flow::Stop = flow {
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

/// What a job has done so far: files found (scan), files read (read),
/// chunks embedded (embed).
fn items(catalog: &CatalogHandle, id: &str) -> u64 {
    let id = id.to_owned();
    catalog
        .call(move |c| {
            c.query_row(
                "SELECT CASE kind WHEN 'scan' THEN discovered_files
                                  WHEN 'embed' THEN embedded_chunks
                                  ELSE (SELECT COUNT(*) FROM file_instances f
                                         WHERE f.source_root_id = jobs.source_root_id
                                           AND f.extract_state = 'linked') END
                   FROM jobs WHERE id = ?1",
                [&id],
                |r| r.get::<_, i64>(0),
            )
        })
        .ok()
        .and_then(Result::ok)
        .map_or(0, |n| n.max(0) as u64)
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
        diagnostics: &Diagnostics,
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
                    diagnostics.clone(),
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
