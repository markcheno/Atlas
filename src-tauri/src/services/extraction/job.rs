//! The extraction job processor (phase1-spec §7.1–7.3, ADR 0009): reads the
//! text of a source root's pending files through the worker, one file at a
//! time, under the resource policy's duty cycle.
//!
//! Per file: a cloud-only file is skipped without being opened; otherwise
//! the worker hashes it, a document already read at this extractor version
//! is linked (copies and moves are read once), and anything else is
//! extracted and committed in one transaction. Policy is re-checked between
//! files; every write is guarded by the job still running.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::db::{self, DocumentResult, QueuedFile};
use super::engine::hash::HashError;
use super::supervisor::{
    DocumentProcessor, Extraction, StopCheck, Supervisor, SupervisorConfig, WorkerCommand,
    WorkerError, WorkerStats,
};
use crate::domain::dto::{
    ExtractionOverview, JobState, PauseReason, PolicyMode, SourceRootStatus, ThrottleReason,
};
use crate::infrastructure::clock::now_rfc3339;
use crate::infrastructure::native_path;
use crate::services::catalog::writer::CatalogHandle;
use crate::services::embedding;
use crate::services::jobs::{
    self, Flow, JobProcessor, JobRow, JobWaker, ProcessorContext, ProcessorFactory, Signal,
};
use crate::services::sources;
use crate::services::system::ResourceGovernor;
use crate::services::workspace::settings;

/// Debug builds only: no duty-cycle sleep between files (benchmarks).
pub const UNTHROTTLED_ENV: &str = "ATLAS_EXTRACT_UNTHROTTLED";

pub use crate::services::scheduler::duty::REST_QUANTUM;
use crate::services::scheduler::duty::rest_due;

#[derive(Debug, Clone)]
pub struct ExtractionConfig {
    /// Files read from the queue per query.
    pub batch_size: usize,
    /// No duty-cycle sleep.
    pub unthrottled: bool,
    /// Busy fraction when the policy says Run, and when it says Slow.
    pub run_duty: f64,
    pub slow_duty: f64,
    pub recheck_interval: Duration,
    pub policy_wait: Duration,
    pub idle_wait: Duration,
    /// Minimum time between progress events.
    pub event_interval: Duration,
    /// Worker restarts within one job before it backs off.
    pub max_restarts: u32,
    /// Crashes on one file before it is `worker_crashed`.
    pub max_attempts: u32,
    pub supervisor: SupervisorConfig,
}

impl Default for ExtractionConfig {
    fn default() -> Self {
        Self {
            batch_size: 20,
            unthrottled: false,
            run_duty: 0.5,
            slow_duty: 0.2,
            recheck_interval: Duration::from_secs(2),
            policy_wait: Duration::from_secs(5),
            idle_wait: Duration::from_secs(30),
            event_interval: Duration::from_millis(250),
            max_restarts: 10,
            max_attempts: 2,
            supervisor: SupervisorConfig::default(),
        }
    }
}

impl ExtractionConfig {
    pub fn from_env() -> Self {
        let unthrottled = cfg!(debug_assertions)
            && std::env::var(UNTHROTTLED_ENV).is_ok_and(|v| v == "1" || v == "true");
        Self {
            unthrottled,
            ..Self::default()
        }
    }
}

pub type ExtractionEvents = Arc<dyn Fn(&ExtractionOverview) + Send + Sync>;

/// Builds the document processor for one open data root; the stop check is
/// the runner's.
pub type DocumentProcessorFactory =
    Arc<dyn Fn(StopCheck) -> Box<dyn DocumentProcessor> + Send + Sync>;

#[derive(Clone)]
pub struct ExtractionDeps {
    pub governor: Arc<Mutex<ResourceGovernor>>,
    pub events: ExtractionEvents,
    pub config: ExtractionConfig,
    pub processor: DocumentProcessorFactory,
    pub stats: Arc<WorkerStats>,
}

impl ExtractionDeps {
    /// The real worker, started with `command`.
    pub fn new(
        governor: Arc<Mutex<ResourceGovernor>>,
        events: ExtractionEvents,
        config: ExtractionConfig,
        command: WorkerCommand,
    ) -> Self {
        let stats = Arc::new(WorkerStats::default());
        let supervisor_config = config.supervisor.clone();
        let counters = Arc::clone(&stats);
        let processor: DocumentProcessorFactory = Arc::new(move |stop| {
            Box::new(Supervisor::new(
                command.clone(),
                supervisor_config.clone(),
                stop,
                Arc::clone(&counters),
            ))
        });
        Self {
            governor,
            events,
            config,
            processor,
            stats,
        }
    }
}

/// Builds an [`ExtractProcessor`] for each data root the workspace opens.
pub fn factory(deps: ExtractionDeps) -> ProcessorFactory {
    Arc::new(move |ctx| Box::new(ExtractProcessor::new(ctx, deps.clone())))
}

enum Gate {
    Pause(PauseReason),
    Go {
        duty: f64,
        throttle: Option<ThrottleReason>,
    },
}

/// What happened to one file, for the job loop.
enum Step {
    Next,
    /// The job left `running` (paused, cancelled, removed): stop here.
    Interrupted,
    /// The library is closing.
    Stop,
    /// PDFium can't be used; the job has been failed.
    Unavailable,
    /// The worker crashed or hung; it has been restarted.
    Crashed,
}

pub struct ExtractProcessor {
    catalog: CatalogHandle,
    data_root: PathBuf,
    deps: ExtractionDeps,
    signal: Arc<Signal>,
    /// Wakes the other kinds' runners: the embed job, once a folder's text
    /// is read.
    waker: JobWaker,
    processor: Box<dyn DocumentProcessor>,
    last_emit: Option<Instant>,
    current: Option<String>,
}

impl JobProcessor for ExtractProcessor {
    fn idle_wait(&self) -> Duration {
        self.deps.config.idle_wait
    }

    fn started(&mut self) {
        self.emit(true);
    }

    fn run(&mut self, job: JobRow) -> Flow {
        let flow = self.run_job(&job);
        self.current = None;
        self.emit(true);
        flow
    }
}

impl ExtractProcessor {
    pub fn new(ctx: ProcessorContext, deps: ExtractionDeps) -> Self {
        let stop_signal = Arc::clone(&ctx.signal);
        let stop: StopCheck = Arc::new(move || stop_signal.stopped());
        Self {
            processor: (deps.processor)(stop),
            catalog: ctx.catalog,
            data_root: dunce::canonicalize(&ctx.data_root).unwrap_or(ctx.data_root),
            deps,
            signal: ctx.signal,
            waker: ctx.waker,
            last_emit: None,
            current: None,
        }
    }

    fn emit(&mut self, force: bool) {
        let due = self
            .last_emit
            .is_none_or(|t| t.elapsed() >= self.deps.config.event_interval);
        if !(force || due) {
            return;
        }
        let current = self.current.clone();
        if let Ok(Ok(overview)) = self
            .catalog
            .call(move |c| db::overview(c, current.as_deref()))
        {
            (self.deps.events)(&overview);
            self.last_emit = Some(Instant::now());
        }
    }

    fn transition(
        &self,
        id: &str,
        to: JobState,
        from: &'static [JobState],
        reason: Option<PauseReason>,
        error: Option<&'static str>,
    ) -> bool {
        let (id, now) = (id.to_owned(), now_rfc3339());
        matches!(
            self.catalog
                .call(move |c| jobs::db::transition(c, &id, to, from, reason, error, &now)),
            Ok(Ok(true))
        )
    }

    fn state_of(&self, id: &str) -> Option<JobState> {
        let id = id.to_owned();
        match self.catalog.call(move |c| jobs::db::job(c, &id)) {
            Ok(Ok(Some(row))) => Some(row.state),
            _ => None,
        }
    }

    fn gate(&self, root: &Path) -> Gate {
        let reachable = sources::probe::probe_all(&[Some(root.to_path_buf())]);
        if reachable[0] != SourceRootStatus::Available {
            return Gate::Pause(PauseReason::SourceRootMissing);
        }
        let check = {
            let mut governor = self.deps.governor.lock().unwrap_or_else(|p| p.into_inner());
            governor.check(Some(&self.data_root))
        };
        let config = &self.deps.config;
        match check.policy.mode {
            PolicyMode::Paused => Gate::Pause(
                check
                    .policy
                    .pause_reason
                    .unwrap_or(PauseReason::LowDiskSpace),
            ),
            mode => Gate::Go {
                duty: if config.unthrottled {
                    1.0
                } else if mode == PolicyMode::Slow {
                    config.slow_duty
                } else {
                    config.run_duty
                },
                throttle: check.policy.throttle_reason,
            },
        }
    }

    /// Applies the policy. `Some(flow)` means don't read a file now.
    fn apply_gate(
        &mut self,
        job: &str,
        root: &Path,
        paused: bool,
        throttle: &mut Option<ThrottleReason>,
        duty: &mut f64,
    ) -> Option<Flow> {
        match self.gate(root) {
            Gate::Pause(reason) => {
                let changed = self.transition(
                    job,
                    JobState::PausedPolicy,
                    &[JobState::Running, JobState::PausedPolicy],
                    Some(reason),
                    None,
                );
                self.emit(changed);
                if self.signal.wait(self.deps.config.policy_wait) {
                    return Some(Flow::Stop);
                }
                Some(Flow::Continue)
            }
            Gate::Go {
                duty: d,
                throttle: t,
            } => {
                if paused {
                    if !self.transition(
                        job,
                        JobState::Running,
                        &[JobState::PausedPolicy],
                        None,
                        None,
                    ) {
                        return Some(Flow::Continue);
                    }
                    self.emit(true);
                }
                if t != *throttle {
                    *throttle = t;
                    let id = job.to_owned();
                    let _ = self
                        .catalog
                        .call(move |c| jobs::db::set_throttle(c, &id, t));
                    self.emit(true);
                }
                *duty = d;
                None
            }
        }
    }

    fn run_job(&mut self, job: &JobRow) -> Flow {
        const STARTABLE: &[JobState] =
            &[JobState::Queued, JobState::PausedPolicy, JobState::Running];
        let Some(root) = native_path::decode(&job.root_path_bytes) else {
            self.transition(
                &job.id,
                JobState::FailedTerminal,
                STARTABLE,
                None,
                Some("path_undecodable"),
            );
            return Flow::Continue;
        };
        if !self.transition(&job.id, JobState::Running, STARTABLE, None, None) {
            return Flow::Continue;
        }
        self.emit(true);

        let mut cursor = 0i64;
        // Files waiting to be read first, then documents due to be read
        // again by a newer reader (which stay searchable meanwhile).
        let mut refreshing = false;
        let mut restarts = 0u32;
        let mut throttle = job.throttle_reason;
        let mut duty = self.deps.config.run_duty;
        let mut last_gate: Option<Instant> = None;
        // Rest owed by the duty cycle, paid in sleeps of at least
        // `REST_QUANTUM` (see below).
        let mut owed = Duration::ZERO;
        loop {
            if self.signal.stopped() {
                return Flow::Stop;
            }
            let state = self.state_of(&job.id);
            if !matches!(state, Some(JobState::Running | JobState::PausedPolicy)) {
                return Flow::Continue;
            }
            let due = last_gate.is_none_or(|t| t.elapsed() >= self.deps.config.recheck_interval);
            if due || state == Some(JobState::PausedPolicy) {
                last_gate = Some(Instant::now());
                let paused = state == Some(JobState::PausedPolicy);
                if let Some(flow) =
                    self.apply_gate(&job.id, &root, paused, &mut throttle, &mut duty)
                {
                    match flow {
                        Flow::Stop => return Flow::Stop,
                        Flow::Continue => continue,
                    }
                }
            }

            let (root_id, limit) = (job.source_root_id.clone(), self.deps.config.batch_size);
            let files = match self.catalog.call(move |c| {
                if refreshing {
                    db::next_refresh(c, &root_id, cursor, limit)
                } else {
                    db::next_files(c, &root_id, cursor, limit)
                }
            }) {
                Ok(Ok(files)) => files,
                _ => return Flow::Continue,
            };
            if files.is_empty() && !refreshing {
                refreshing = true;
                cursor = 0;
                continue;
            }
            if files.is_empty() {
                // Tidy the text index for search before reporting done.
                let _ = self.catalog.call(|c| db::merge_index(c, 20));
                if self.transition(
                    &job.id,
                    JobState::Completed,
                    &[JobState::Running],
                    None,
                    None,
                ) {
                    // The folder's text is read: its meaning can be indexed.
                    let allow = settings::read(&self.data_root)
                        .privacy
                        .allow_remote_embedding;
                    let root_id = job.source_root_id.clone();
                    let _ = self.catalog.call(move |c| {
                        embedding::triggers::queue_root(c, &root_id, allow, &now_rfc3339())
                    });
                    self.waker.wake();
                }
                return Flow::Continue;
            }

            for file in files {
                if self.signal.stopped() {
                    return Flow::Stop;
                }
                if self.state_of(&job.id) != Some(JobState::Running) {
                    break;
                }
                cursor = file.rowid;
                let started = Instant::now();
                self.current = Some(file.id.clone());
                self.emit(false);
                let step = self.read_file(&job.id, &root, &file);
                self.current = None;
                match step {
                    Step::Next => {}
                    Step::Interrupted => break,
                    Step::Stop => return Flow::Stop,
                    Step::Unavailable => return Flow::Continue,
                    Step::Crashed => {
                        restarts += 1;
                        if restarts >= self.deps.config.max_restarts {
                            return self.too_many_restarts(&job.id);
                        }
                        // Try the same file again with the new worker; after
                        // `max_attempts` crashes it is no longer pending.
                        cursor = file.rowid - 1;
                        break;
                    }
                }
                self.emit(false);
                // The duty cycle: rest so the busy fraction stays at `duty`.
                // Most files take milliseconds, and thousands of
                // millisecond sleeps cost more than they rest (timer
                // rounding, cold caches, cores clocking down), so rests add
                // up and are taken once they reach `REST_QUANTUM` (Slice
                // 1.7, docs/benchmarks.md).
                if let Some(rest) = rest_due(&mut owed, started.elapsed(), duty)
                    && self.signal.wait(rest)
                {
                    return Flow::Stop;
                }
                if last_gate.is_none_or(|t| t.elapsed() >= self.deps.config.recheck_interval) {
                    // Re-check policy before the next file.
                    break;
                }
            }
        }
    }

    /// Ten restarts within one job: back off, then queue again (like a scan's
    /// database failures).
    fn too_many_restarts(&mut self, id: &str) -> Flow {
        const FROM: &[JobState] = &[JobState::Running, JobState::PausedPolicy];
        let retry_id = id.to_owned();
        let retries = self
            .catalog
            .call(move |c| jobs::db::bump_retry(c, &retry_id))
            .ok()
            .and_then(Result::ok)
            .unwrap_or(u32::MAX);
        self.transition(
            id,
            JobState::FailedRetryable,
            FROM,
            None,
            Some("worker_restarts"),
        );
        self.emit(true);
        if self.signal.wait(Duration::from_secs(1 << retries.min(6))) {
            return Flow::Stop;
        }
        self.transition(
            id,
            JobState::Queued,
            &[JobState::FailedRetryable],
            None,
            None,
        );
        Flow::Continue
    }

    fn blocked(
        &self,
        job: &str,
        file: &str,
        state: &'static str,
        error: Option<&'static str>,
    ) -> Step {
        let (job, file) = (job.to_owned(), file.to_owned());
        match self
            .catalog
            .call(move |c| db::set_blocked(c, &job, &file, state, error))
        {
            Ok(Ok(true)) => Step::Next,
            _ => Step::Interrupted,
        }
    }

    fn unavailable(&mut self, job: &str) -> Step {
        const FROM: &[JobState] = &[JobState::Running, JobState::PausedPolicy];
        self.transition(
            job,
            JobState::FailedTerminal,
            FROM,
            None,
            Some("extractor_unavailable"),
        );
        Step::Unavailable
    }

    fn read_file(&mut self, job: &str, root: &Path, file: &QueuedFile) -> Step {
        if file.cloud_only {
            return self.blocked(job, &file.id, "skipped_cloud_only", None);
        }
        let Some(rel) = native_path::decode(&file.rel_path) else {
            return self.blocked(job, &file.id, "hash_failed", Some("path_undecodable"));
        };
        let path = root.join(rel);

        let hashed = match self.processor.hash(&path) {
            Ok(Ok(h)) => h,
            Ok(Err(HashError::CloudOnly)) => {
                return self.blocked(job, &file.id, "skipped_cloud_only", None);
            }
            Ok(Err(HashError::Unreadable)) => {
                return self.blocked(job, &file.id, "unreadable", Some("permission_denied"));
            }
            Ok(Err(HashError::Failed)) => {
                return self.blocked(job, &file.id, "hash_failed", Some("read_failed"));
            }
            Err(WorkerError::Stopped) => return Step::Stop,
            Err(WorkerError::Unavailable(_)) => return self.unavailable(job),
            Err(WorkerError::Crashed(_)) => {
                if self.crash_attempts(&file.id) >= self.deps.config.max_attempts {
                    let step = match &file.refresh_document {
                        Some(doc) => self.keep_old_text(job, doc),
                        None => self.blocked(job, &file.id, "hash_failed", Some("worker_crashed")),
                    };
                    if !matches!(step, Step::Next) {
                        return step;
                    }
                }
                return Step::Crashed;
            }
        };
        // Changed since the scan: leave it pending; the next scan sees it.
        if Some(hashed.size) != file.size_bytes || hashed.mtime_secs != file.mtime_secs {
            return Step::Next;
        }
        let Some(hash) = hex_bytes(&hashed.hash) else {
            return self.blocked(job, &file.id, "hash_failed", Some("bad_hash"));
        };

        let lookup = hash.clone();
        let file_type = file.file_type;
        if let Ok(Ok(Some(document))) = self
            .catalog
            .call(move |c| db::reusable_document(c, &lookup, file_type))
        {
            let (job, id) = (job.to_owned(), file.id.clone());
            let (size, mtime) = (hashed.size, hashed.mtime_secs);
            return match self
                .catalog
                .call(move |c| db::link(c, &job, &id, &document, size, mtime))
            {
                Ok(Ok(true)) => Step::Next,
                _ => Step::Interrupted,
            };
        }

        let extraction = match self.processor.extract(&path, file.file_type) {
            Ok(e) => e,
            Err(WorkerError::Stopped) => return Step::Stop,
            Err(WorkerError::Unavailable(_)) => return self.unavailable(job),
            Err(WorkerError::Crashed(_)) => {
                if self.crash_attempts(&file.id) >= self.deps.config.max_attempts {
                    // A refresh that keeps crashing keeps the old text.
                    if let Some(doc) = &file.refresh_document {
                        let step = self.keep_old_text(job, doc);
                        if !matches!(step, Step::Next) {
                            return step;
                        }
                        return Step::Crashed;
                    }
                    let step = self.commit(
                        job,
                        file,
                        hash,
                        hashed.size,
                        hashed.mtime_secs,
                        Extraction::Failed {
                            outcome: super::engine::Outcome::Malformed,
                            reason: String::new(),
                        },
                        Some("worker_crashed"),
                    );
                    if !matches!(step, Step::Next) {
                        return step;
                    }
                }
                return Step::Crashed;
            }
        };
        self.commit(
            job,
            file,
            hash,
            hashed.size,
            hashed.mtime_secs,
            extraction,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn commit(
        &mut self,
        job: &str,
        file: &QueuedFile,
        hash: Vec<u8>,
        size: u64,
        mtime: Option<i64>,
        extraction: Extraction,
        crashed: Option<&'static str>,
    ) -> Step {
        let (job, id, now) = (job.to_owned(), file.id.clone(), now_rfc3339());
        let file_type = file.file_type;
        let committed = self.catalog.call(move |c| {
            let result = match (&extraction, crashed) {
                (_, Some(reason)) => DocumentResult::NoText {
                    state: "worker_crashed",
                    reason,
                },
                (Extraction::Done { summary, pieces }, None) => {
                    DocumentResult::Text { summary, pieces }
                }
                (Extraction::Failed { outcome, reason }, None) => DocumentResult::NoText {
                    state: db::outcome_state(*outcome),
                    reason,
                },
            };
            db::commit_document(c, &job, &id, &hash, size, mtime, file_type, result, &now)
        });
        match committed {
            Ok(Ok(true)) => Step::Next,
            _ => Step::Interrupted,
        }
    }

    /// Gives up re-reading a document, keeping its old text.
    fn keep_old_text(&self, job: &str, document: &str) -> Step {
        let (job, document) = (job.to_owned(), document.to_owned());
        match self
            .catalog
            .call(move |c| db::clear_refresh(c, &job, &document))
        {
            Ok(Ok(true)) => Step::Next,
            _ => Step::Interrupted,
        }
    }

    /// Counts a crash on this file; returns the attempts so far.
    fn crash_attempts(&self, file: &str) -> u32 {
        let id = file.to_owned();
        self.catalog
            .call(move |c| db::record_crash(c, &id))
            .ok()
            .and_then(Result::ok)
            .unwrap_or(u32::MAX)
    }
}

fn hex_bytes(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect()
}
