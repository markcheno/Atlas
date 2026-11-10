//! The embedding job (phase2-spec §7.2, ADR 0014): one job per library
//! folder fills the active embedding model from the text extraction left
//! in the catalog.
//!
//! - **Per document, one transaction.** Its passages are chunked, only the
//!   chunks whose text has no vector for the model are sent, and the chunks,
//!   new vectors and the document's state are written together, guarded by
//!   the job still running and by the document's text being unchanged since
//!   it was read. The in-memory index gains the chunks after the commit.
//! - **A chunk is never requested again after an answer**: vectors already
//!   received for a document are kept across retries.
//! - **Policy:** the shared resource policy, except that on battery this job
//!   pauses where extraction slows (product spec §9).
//! - **Failures:** the service being down, slow or busy backs off; six in a
//!   row fail the job for a later retry; a refused key or a model that
//!   changed fail it for good; a chunk the service refuses fails only its
//!   document.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rusqlite::{Connection, ErrorCode};

use super::client::{ProviderEmbeddingBackend, StopCheck};
use super::index::VectorIndex;
use super::registry::VectorRegistry;
use super::vector::to_f16_blob;
use super::{db, models, status};
use crate::domain::dto::{
    EmbeddingOverview, JobState, PauseReason, PolicyMode, ProviderKind, ThrottleReason,
};
use crate::domain::traits::{EmbedError, EmbeddingBackend};
use crate::error::AtlasError;
use crate::infrastructure::clock::now_rfc3339;
use crate::infrastructure::http_probe::{MAX_RETRY_AFTER, ProbeConfig};
use crate::services::catalog::reader;
use crate::services::catalog::writer::CatalogHandle;
use crate::services::jobs::ProcessorFactory;
use crate::services::jobs::{self, Flow, JobProcessor, JobRow, JobWaker, ProcessorContext, Signal};
use crate::services::providers::profiles::{self, Prepared};
use crate::services::providers::secrets::{Secret, SecretVault};
use crate::services::scheduler::duty::rest_due;
use crate::services::system::ResourceGovernor;
use crate::services::workspace::layout::CATALOG;
use crate::services::workspace::settings;

/// Debug builds only: no duty-cycle rest between documents (benchmarks).
pub const UNTHROTTLED_ENV: &str = "ATLAS_EMBED_UNTHROTTLED";

#[derive(Debug, Clone)]
pub struct EmbedConfig {
    pub unthrottled: bool,
    pub run_duty: f64,
    pub slow_duty: f64,
    /// Documents fetched at a time.
    pub batch_size: usize,
    /// The most chunks, and characters, in one request.
    pub max_request_chunks: usize,
    pub max_request_chars: usize,
    pub recheck_interval: Duration,
    pub policy_wait: Duration,
    pub idle_wait: Duration,
    pub event_interval: Duration,
    /// Save the index after this many changes, or this long.
    pub save_every_changes: usize,
    pub save_every: Duration,
    /// Backoff after a failed request: `backoff_base × 2ⁿ`, at most
    /// `backoff_max`, ±20%.
    pub backoff_base: Duration,
    pub backoff_max: Duration,
    /// Failed requests in a row before the job fails for a later retry.
    pub max_consecutive_failures: u32,
    /// The wait before such a job is queued again: `retry_base × 2ⁿ`
    /// (n = earlier retries, at most 4). Retry cuts it short.
    pub retry_base: Duration,
    /// Threads for building an index when it must be rebuilt.
    pub rebuild_threads: usize,
}

impl Default for EmbedConfig {
    fn default() -> Self {
        Self {
            unthrottled: false,
            run_duty: 0.5,
            slow_duty: 0.2,
            batch_size: 10,
            max_request_chunks: 16,
            max_request_chars: 16_000,
            recheck_interval: Duration::from_secs(2),
            policy_wait: Duration::from_secs(5),
            idle_wait: Duration::from_secs(30),
            event_interval: Duration::from_millis(250),
            save_every_changes: 10_000,
            save_every: Duration::from_secs(300),
            backoff_base: Duration::from_secs(1),
            backoff_max: Duration::from_secs(64),
            max_consecutive_failures: 6,
            retry_base: Duration::from_secs(60),
            rebuild_threads: std::thread::available_parallelism()
                .map_or(2, |n| n.get())
                .clamp(1, 8),
        }
    }
}

impl EmbedConfig {
    pub fn from_env() -> Self {
        let unthrottled = cfg!(debug_assertions)
            && std::env::var(UNTHROTTLED_ENV).is_ok_and(|v| v == "1" || v == "true");
        Self {
            unthrottled,
            ..Self::default()
        }
    }
}

/// The embedding role's service, resolved for one job.
pub struct ResolvedService {
    pub kind: ProviderKind,
    pub base: String,
    pub key: Option<Secret>,
    pub model: String,
    /// The vector length Atlas stores (the model row's dimensions).
    pub dims: usize,
    /// The vector length the service returns: more than `dims` when Atlas
    /// keeps only the first `dims` (Slice 2.4).
    pub native_dims: usize,
}

/// Builds the backend a job sends its chunks to.
pub type BackendFactory =
    Arc<dyn Fn(&ResolvedService, StopCheck) -> Box<dyn EmbeddingBackend> + Send + Sync>;

/// The service itself, over HTTP.
pub fn http_backends(config: ProbeConfig) -> BackendFactory {
    Arc::new(move |s, stop| {
        Box::new(ProviderEmbeddingBackend {
            kind: s.kind,
            base: s.base.clone(),
            key: s.key.clone(),
            model: s.model.clone(),
            dims: s.native_dims.max(s.dims),
            keep: s.dims,
            config,
            stop,
        })
    })
}

pub type EmbeddingEvents = Arc<dyn Fn(&EmbeddingOverview) + Send + Sync>;

#[derive(Clone)]
pub struct EmbedDeps {
    pub governor: Arc<Mutex<ResourceGovernor>>,
    pub vault: Arc<SecretVault>,
    pub events: EmbeddingEvents,
    pub config: EmbedConfig,
    pub backends: BackendFactory,
}

/// Builds an [`EmbedProcessor`] for each library the workspace opens.
pub fn factory(deps: EmbedDeps) -> ProcessorFactory {
    Arc::new(move |ctx| Box::new(EmbedProcessor::new(ctx, deps.clone())))
}

enum Gate {
    Pause(PauseReason),
    Go {
        duty: f64,
        throttle: Option<ThrottleReason>,
    },
}

/// What happened to one document.
enum Step {
    /// Embedded (or nothing to send), or skipped because its text changed.
    Next,
    /// The service refused one of its chunks every time: marked failed.
    Failed,
    /// The service is down, slow or busy: back off and try it again.
    Transient(EmbedError),
    /// The job can't go on: fail it with this code.
    Terminal(&'static str),
    /// The job left `running`.
    Interrupted,
    /// The library is closing.
    Stop,
}

/// The pause after the `n`th failure in a row (n ≥ 1): doubling from
/// `base`, at most `max`, ±20%, and never shorter than the service asked.
pub fn backoff(
    base: Duration,
    max: Duration,
    n: u32,
    asked: Option<Duration>,
    seed: u64,
) -> Duration {
    let doubled = base
        .saturating_mul(1u32 << (n.saturating_sub(1)).min(16))
        .min(max);
    // ±20%, from a cheap hash of the seed.
    let spread = (seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 40) as f64 / (1u64 << 24) as f64;
    let jittered = doubled.mul_f64(0.8 + 0.4 * spread);
    match asked {
        Some(a) => jittered.max(a.min(MAX_RETRY_AFTER)),
        None => jittered,
    }
}

/// Splits `texts` into requests of at most `max_chunks` items and
/// `max_chars` characters (a single longer text is a request of its own).
pub fn requests(
    texts: &[&str],
    max_chunks: usize,
    max_chars: usize,
) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let (mut start, mut chars) = (0, 0);
    for (i, t) in texts.iter().enumerate() {
        let n = t.chars().count();
        if i > start && (i - start >= max_chunks || chars + n > max_chars) {
            out.push(start..i);
            start = i;
            chars = 0;
        }
        chars += n;
    }
    if start < texts.len() {
        out.push(start..texts.len());
    }
    out
}

pub struct EmbedProcessor {
    catalog: CatalogHandle,
    data_root: PathBuf,
    deps: EmbedDeps,
    signal: Arc<Signal>,
    waker: JobWaker,
    vectors: Arc<VectorRegistry>,
    reader: Option<Connection>,
    /// Vectors received for the document in hand, kept across retries.
    received: HashMap<[u8; 32], Vec<u8>>,
    last_emit: Option<Instant>,
    last_save: Instant,
}

impl JobProcessor for EmbedProcessor {
    fn idle_wait(&self) -> Duration {
        self.deps.config.idle_wait
    }

    fn started(&mut self) {
        self.sweep();
        self.emit(true);
    }

    fn run(&mut self, job: JobRow) -> Flow {
        let flow = self.run_job(&job);
        self.emit(true);
        flow
    }
}

impl EmbedProcessor {
    pub fn new(ctx: ProcessorContext, deps: EmbedDeps) -> Self {
        Self {
            catalog: ctx.catalog,
            data_root: ctx.data_root,
            deps,
            signal: ctx.signal,
            waker: ctx.waker,
            vectors: ctx.vectors,
            reader: None,
            received: HashMap::new(),
            last_emit: None,
            last_save: Instant::now(),
        }
    }

    fn allow_remote(&self) -> bool {
        crate::services::providers::privacy::remote_embedding(
            &settings::read(&self.data_root).privacy,
        )
    }

    /// A read-only connection of the job's own, for passages and vectors.
    fn reader(&mut self) -> Option<&Connection> {
        if self.reader.is_none() {
            self.reader = reader::open_single(&self.data_root.join(CATALOG)).ok();
        }
        self.reader.as_ref()
    }

    fn emit(&mut self, force: bool) {
        let due = self
            .last_emit
            .is_none_or(|t| t.elapsed() >= self.deps.config.event_interval);
        if !(force || due) {
            return;
        }
        let allow = self.allow_remote();
        if let Ok(Ok(overview)) = self.catalog.call(move |c| status::overview(c, allow)) {
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

    /// The resource policy for this job. Text comes from the catalog, so a
    /// library folder being unplugged doesn't matter here.
    fn gate(&self) -> Gate {
        let check = {
            let mut governor = self.deps.governor.lock().unwrap_or_else(|p| p.into_inner());
            governor.check(Some(&self.data_root))
        };
        let config = &self.deps.config;
        let policy = check.policy;
        match policy.mode {
            PolicyMode::Paused => {
                Gate::Pause(policy.pause_reason.unwrap_or(PauseReason::LowDiskSpace))
            }
            PolicyMode::Slow if policy.throttle_reason == Some(ThrottleReason::OnBattery) => {
                Gate::Pause(PauseReason::OnBattery)
            }
            mode => Gate::Go {
                duty: if config.unthrottled {
                    1.0
                } else if mode == PolicyMode::Slow {
                    config.slow_duty
                } else {
                    config.run_duty
                },
                throttle: policy.throttle_reason,
            },
        }
    }

    /// Applies the policy. `Some(flow)` means don't embed now.
    fn apply_gate(
        &mut self,
        job: &str,
        index: &VectorIndex,
        paused: bool,
        throttle: &mut Option<ThrottleReason>,
        duty: &mut f64,
    ) -> Option<Flow> {
        match self.gate() {
            Gate::Pause(reason) => {
                let changed = self.transition(
                    job,
                    JobState::PausedPolicy,
                    &[JobState::Running, JobState::PausedPolicy],
                    Some(reason),
                    None,
                );
                if changed {
                    self.save(index, true);
                }
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

    /// Saves the index when it is due (or `force`d and changed), after
    /// applying the tombstones other threads left.
    fn save(&mut self, index: &VectorIndex, force: bool) {
        if let Some(conn) = self.reader() {
            let _ = index.apply_tombstones(conn);
        }
        let changes = index.changes_since_save();
        let due = changes >= self.deps.config.save_every_changes
            || (changes > 0 && self.last_save.elapsed() >= self.deps.config.save_every);
        if ((force && changes > 0) || due)
            && index.save(self.vectors.paths(), &self.catalog).is_ok()
        {
            self.last_save = Instant::now();
        }
    }

    fn run_job(&mut self, job: &JobRow) -> Flow {
        const STARTABLE: &[JobState] =
            &[JobState::Queued, JobState::PausedPolicy, JobState::Running];
        if !self.transition(&job.id, JobState::Running, STARTABLE, None, None) {
            return Flow::Continue;
        }
        self.emit(true);

        // The model, its role's service, and whether it may be used.
        let allow = self.allow_remote();
        let vault = Arc::clone(&self.deps.vault);
        let now = now_rfc3339();
        let setup = self.catalog.call(move |c| {
            let model = models::sync(c, &now)?;
            let role = models::role(c)?;
            let permitted = status::permitted(c, allow)?;
            let prepared = role
                .as_ref()
                .map(|r| profiles::resolve_saved(c, &vault, &r.profile_id));
            Ok::<_, rusqlite::Error>((model, role, permitted, prepared))
        });
        let (model, role, permitted, prepared) = match setup {
            Ok(Ok(found)) => found,
            _ => return Flow::Continue,
        };
        let (Some(model), Some(role), true) = (model, role, permitted) else {
            // Nothing to do until a model is chosen or allowed; the status
            // says which. Setting either queues the folders again.
            self.transition(
                &job.id,
                JobState::Completed,
                &[JobState::Running],
                None,
                None,
            );
            return Flow::Continue;
        };
        let service = match prepared {
            Some(Ok(Prepared::Ready {
                kind,
                endpoint,
                key,
            })) => ResolvedService {
                kind,
                base: endpoint.base,
                key,
                model: model.model_name.clone(),
                dims: model.dims,
                native_dims: role.dims,
            },
            Some(Ok(Prepared::Blocked(_))) => {
                self.fail(&job.id, "address_rejected");
                return Flow::Continue;
            }
            _ => {
                self.transition(
                    &job.id,
                    JobState::Completed,
                    &[JobState::Running],
                    None,
                    None,
                );
                return Flow::Continue;
            }
        };
        let stop_signal = Arc::clone(&self.signal);
        let stop: StopCheck = Arc::new(move || stop_signal.stopped());
        let backend = (self.deps.backends)(&service, stop);
        let index = match self.vectors.get_or_open(
            &self.catalog,
            &model.id,
            self.deps.config.rebuild_threads,
        ) {
            Ok((index, _)) => index,
            Err(_) => {
                self.fail(&job.id, "index_unavailable");
                return Flow::Continue;
            }
        };

        let flow = self.embed_root(job, &model, backend.as_ref(), &index);
        self.save(&index, true);
        if flow == Flow::Continue && self.state_of(&job.id) == Some(JobState::Completed) {
            self.finish_if_complete(&model.id, &index);
            self.sweep();
        }
        flow
    }

    fn fail(&mut self, id: &str, code: &'static str) {
        const FROM: &[JobState] = &[JobState::Running, JobState::PausedPolicy];
        self.transition(id, JobState::FailedTerminal, FROM, None, Some(code));
    }

    fn embed_root(
        &mut self,
        job: &JobRow,
        model: &db::EmbeddingModel,
        backend: &dyn EmbeddingBackend,
        index: &VectorIndex,
    ) -> Flow {
        let mut cursor = 0i64;
        let mut throttle = job.throttle_reason;
        let mut duty = self.deps.config.run_duty;
        let mut last_gate: Option<Instant> = None;
        let mut owed = Duration::ZERO;
        let mut failures = 0u32;
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
                    self.apply_gate(&job.id, index, paused, &mut throttle, &mut duty)
                {
                    match flow {
                        Flow::Stop => return Flow::Stop,
                        Flow::Continue => continue,
                    }
                }
            }

            let (root_id, model_id, limit) = (
                job.source_root_id.clone(),
                model.id.clone(),
                self.deps.config.batch_size,
            );
            let docs = match self
                .catalog
                .call(move |c| db::next_documents(c, &root_id, &model_id, cursor, limit))
            {
                Ok(Ok(docs)) => docs,
                _ => return Flow::Continue,
            };
            if docs.is_empty() {
                self.transition(
                    &job.id,
                    JobState::Completed,
                    &[JobState::Running],
                    None,
                    None,
                );
                return Flow::Continue;
            }

            for doc in docs {
                if self.signal.stopped() {
                    return Flow::Stop;
                }
                if self.state_of(&job.id) != Some(JobState::Running) {
                    break;
                }
                let started = Instant::now();
                match self.embed_document(&job.id, model, &doc, backend, index) {
                    Step::Next | Step::Failed => {
                        failures = 0;
                        cursor = doc.rowid;
                    }
                    Step::Transient(err) => {
                        failures += 1;
                        if failures >= self.deps.config.max_consecutive_failures {
                            return self.unreachable(&job.id, index);
                        }
                        let wait = backoff(
                            self.deps.config.backoff_base,
                            self.deps.config.backoff_max,
                            failures,
                            err.retry_after(),
                            started.elapsed().as_nanos() as u64 ^ doc.rowid as u64,
                        );
                        if self.signal.wait(wait) {
                            return Flow::Stop;
                        }
                        // The same document again; what it already got is kept.
                        break;
                    }
                    Step::Terminal(code) => {
                        self.fail(&job.id, code);
                        return Flow::Continue;
                    }
                    Step::Interrupted => break,
                    Step::Stop => return Flow::Stop,
                }
                self.emit(false);
                self.save(index, false);
                if let Some(rest) = rest_due(&mut owed, started.elapsed(), duty)
                    && self.signal.wait(rest)
                {
                    return Flow::Stop;
                }
                if last_gate.is_none_or(|t| t.elapsed() >= self.deps.config.recheck_interval) {
                    break;
                }
            }
            if let Some(conn) = self.reader() {
                let _ = index.apply_tombstones(conn);
            }
        }
    }

    /// Too many failed requests in a row: fail the job for now, wait, and
    /// queue it again (Retry, i.e. Resume, cuts the wait short).
    fn unreachable(&mut self, id: &str, index: &VectorIndex) -> Flow {
        const FROM: &[JobState] = &[JobState::Running, JobState::PausedPolicy];
        self.save(index, true);
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
            Some("provider_unreachable"),
        );
        self.emit(true);
        let wait = self
            .deps
            .config
            .retry_base
            .saturating_mul(1 << retries.saturating_sub(1).min(4));
        if self.signal.wait(wait) {
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

    fn embed_document(
        &mut self,
        job_id: &str,
        model: &db::EmbeddingModel,
        doc: &db::WaitingDocument,
        backend: &dyn EmbeddingBackend,
        index: &VectorIndex,
    ) -> Step {
        let Some(conn) = self.reader() else {
            return Step::Transient(EmbedError::Unavailable);
        };
        let read = (|| {
            let fp = db::fingerprint(conn, &doc.id)?;
            let passages = db::passages(conn, &doc.id)?;
            Ok::<_, rusqlite::Error>((fp, passages))
        })();
        let Ok((fingerprint, passages)) = read else {
            return Step::Transient(EmbedError::Unavailable);
        };
        let planned = db::plan(&passages);
        let hashes: Vec<[u8; 32]> = planned.iter().map(|(c, _)| c.hash).collect();
        let Ok(known) = db::known_hashes(conn, &model.id, &hashes) else {
            return Step::Transient(EmbedError::Unavailable);
        };
        // The texts still needed, once each.
        let mut misses: Vec<([u8; 32], &str)> = Vec::new();
        for (c, text) in &planned {
            if !known.contains(&c.hash)
                && !self.received.contains_key(&c.hash)
                && !misses.iter().any(|(h, _)| *h == c.hash)
            {
                misses.push((c.hash, text));
            }
        }

        let (mut chunks, mut chars, mut waited) = (0u64, 0u64, Duration::ZERO);
        let texts: Vec<&str> = misses.iter().map(|(_, t)| *t).collect();
        let config = &self.deps.config;
        let mut refused = false;
        for range in requests(&texts, config.max_request_chunks, config.max_request_chars) {
            let started = Instant::now();
            let answer = backend.embed(&texts[range.clone()]);
            waited += started.elapsed();
            let answer = match answer {
                Err(EmbedError::BadRequest { .. } | EmbedError::WrongCount { .. })
                    if range.len() > 1 =>
                {
                    // One chunk may be to blame: send them one at a time.
                    let mut singles = Vec::with_capacity(range.len());
                    for i in range.clone() {
                        let t = Instant::now();
                        let one = backend.embed(&texts[i..=i]);
                        waited += t.elapsed();
                        match one {
                            Ok(v) if v.iter().any(|x| x.len() != model.dims) => {
                                return Step::Terminal("model_changed");
                            }
                            Ok(mut v) if v.len() == 1 => singles.push(Some(v.remove(0))),
                            Ok(_)
                            | Err(EmbedError::BadRequest { .. } | EmbedError::WrongCount { .. }) => {
                                singles.push(None)
                            }
                            Err(e) => return self.classify(e),
                        }
                    }
                    if singles.iter().any(Option::is_none) {
                        refused = true;
                    }
                    for (i, v) in range.clone().zip(singles) {
                        if let Some(v) = v {
                            chars += texts[i].chars().count() as u64;
                            chunks += 1;
                            self.received.insert(misses[i].0, to_f16_blob(&v));
                        }
                    }
                    continue;
                }
                Err(EmbedError::BadRequest { .. } | EmbedError::WrongCount { .. }) => {
                    refused = true;
                    continue;
                }
                Err(e) => return self.classify(e),
                // Whatever the backend, every vector must have the model's
                // length: a different one means the model changed.
                Ok(v) if v.iter().any(|x| x.len() != model.dims) => {
                    return Step::Terminal("model_changed");
                }
                Ok(v) => v,
            };
            for (i, v) in range.zip(answer) {
                chars += texts[i].chars().count() as u64;
                chunks += 1;
                self.received.insert(misses[i].0, to_f16_blob(&v));
            }
        }

        let (job, document, model_id, now) = (
            job_id.to_owned(),
            doc.id.clone(),
            model.id.clone(),
            now_rfc3339(),
        );
        let service_ms = waited.as_millis() as u64;
        let written = if refused {
            self.catalog.call(move |c| {
                let tx = c.transaction()?;
                if !job_running(&tx, &job)? {
                    return Ok(Written::Interrupted);
                }
                db::mark_failed(&tx, &document, &model_id, "chunk_rejected", &now)?;
                db::add_counters(&tx, &job, chunks, chars, service_ms)?;
                tx.commit()?;
                Ok::<_, rusqlite::Error>(Written::Failed)
            })
        } else {
            let plan: Vec<db::PlannedChunk> = planned.iter().map(|(c, _)| c.clone()).collect();
            let new: Vec<([u8; 32], Vec<u8>)> = plan
                .iter()
                .filter_map(|c| self.received.get(&c.hash).map(|v| (c.hash, v.clone())))
                .collect();
            self.catalog.call(move |c| {
                let tx = c.transaction()?;
                if !job_running(&tx, &job)? {
                    return Ok(Written::Interrupted);
                }
                if db::fingerprint(&tx, &document)? != fingerprint {
                    return Ok(Written::Changed);
                }
                match db::write_document(&tx, &document, &model_id, &plan, &new, &now) {
                    Ok(_) => {}
                    // Its text went away meanwhile, or lost a vector.
                    Err(rusqlite::Error::SqliteFailure(e, _))
                        if e.code == ErrorCode::ConstraintViolation =>
                    {
                        return Ok(Written::Changed);
                    }
                    Err(e) => return Err(e),
                }
                db::add_counters(&tx, &job, chunks, chars, service_ms)?;
                tx.commit()?;
                Ok(Written::Done)
            })
        };
        match written {
            Ok(Ok(Written::Done)) => {
                self.received.clear();
                // The chunks it now has, including those whose text other
                // documents had already embedded.
                if let Some(conn) = self.reader()
                    && let Ok(vectors) = db::document_vectors(conn, &model.id, model.dims, &doc.id)
                {
                    let _ = index.add_batch(&vectors, 1);
                    if let Ok(seq) = db::last_seq(conn, &model.id) {
                        index.advance_seq(seq);
                    }
                }
                Step::Next
            }
            Ok(Ok(Written::Failed)) => {
                self.received.clear();
                Step::Failed
            }
            Ok(Ok(Written::Changed)) => {
                self.received.clear();
                Step::Next
            }
            Ok(Ok(Written::Interrupted)) => Step::Interrupted,
            _ => Step::Transient(EmbedError::Unavailable),
        }
    }

    fn classify(&mut self, e: EmbedError) -> Step {
        match e {
            EmbedError::Cancelled => Step::Stop,
            EmbedError::Auth => Step::Terminal("provider_auth"),
            EmbedError::ModelNotFound => Step::Terminal("model_not_found"),
            EmbedError::WrongDimensions { .. } => Step::Terminal("model_changed"),
            other => Step::Transient(other),
        }
    }

    /// A building model whose every folder is done: validate it against
    /// the model it replaces, then make it the one search uses.
    fn finish_if_complete(&mut self, model_id: &str, index: &VectorIndex) {
        let id = model_id.to_owned();
        let pending = self.catalog.call(move |c| {
            let model = db::get_model(c, &id)?;
            if model.as_ref().is_none_or(|m| m.status != "building") {
                return Ok(None);
            }
            let active: bool = c.query_row(
                "SELECT EXISTS (SELECT 1 FROM jobs WHERE kind = 'embed'
                     AND state IN ('running', 'paused_policy', 'paused_user',
                                   'failed_retryable', 'queued'))",
                [],
                |r| r.get(0),
            )?;
            let roots: Vec<String> = c
                .prepare("SELECT id FROM source_roots")?
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            let mut waiting = false;
            for root in roots {
                waiting |= db::has_waiting(c, &root, &id)?;
            }
            Ok::<_, rusqlite::Error>((!active && !waiting).then(|| models::predecessor(c, &id)))
        });
        let Ok(Ok(Some(predecessor))) = pending else {
            return;
        };
        let now = now_rfc3339();
        let id = model_id.to_owned();
        let Ok(old) = predecessor else {
            return;
        };
        let valid = match &old {
            None => true,
            Some(old) => self.validate(model_id, &old.id, index),
        };
        let old_id = old.map(|o| o.id);
        let _ = self.catalog.call(move |c| {
            let tx = c.transaction()?;
            if valid {
                models::set_status(&tx, &id, "ready", &now)?;
                if let Some(old) = &old_id {
                    models::set_status(&tx, old, "retired", &now)?;
                }
            } else {
                models::set_status(&tx, &id, "failed", &now)?;
            }
            tx.commit()
        });
        self.waker.wake();
    }

    /// Coverage at least the old model's; 50 random chunks find themselves
    /// (or an identical text) nearest; the file saves.
    fn validate(&mut self, new: &str, old: &str, index: &VectorIndex) -> bool {
        let (new_id, old_id) = (new.to_owned(), old.to_owned());
        let dims = index.dims();
        let checked = self.catalog.call(move |c| {
            let covered = db::covered_documents(c, &new_id)? >= db::covered_documents(c, &old_id)?;
            let sample = db::sample_vectors(c, &new_id, dims, 50)?;
            Ok::<_, rusqlite::Error>((covered, sample))
        });
        let Ok(Ok((covered, sample))) = checked else {
            return false;
        };
        let found = sample.iter().all(|(_, v)| {
            index
                .search(v, 1)
                .ok()
                .and_then(|hits| hits.first().copied())
                .is_some_and(|(_, distance)| distance <= 1e-3)
        });
        covered && found && index.save(self.vectors.paths(), &self.catalog).is_ok()
    }

    /// Deletes retired and failed models (rows in batches, then the row,
    /// then their files) and index files of models the catalog no longer
    /// has.
    fn sweep(&mut self) {
        const BATCH: usize = 5_000;
        let Ok(Ok(done)) = self.catalog.call(|c| models::finished_with(c)) else {
            return;
        };
        for id in done {
            self.vectors.forget(&id);
            loop {
                let model = id.clone();
                match self
                    .catalog
                    .call(move |c| db::delete_model_rows(c, &model, BATCH))
                {
                    Ok(Ok(0)) => break,
                    Ok(Ok(_)) => continue,
                    _ => return,
                }
            }
            let model = id.clone();
            let _ = self
                .catalog
                .call(move |c| c.execute("DELETE FROM embedding_models WHERE id = ?1", [model]));
        }
        let Ok(Ok(known)) = self.catalog.call(|c| models::all_ids(c)) else {
            return;
        };
        let Ok(entries) = std::fs::read_dir(&self.vectors.paths().vectors) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(stem) = name.strip_suffix(".usearch") else {
                continue;
            };
            let owner = stem.rsplit_once('-').map_or(stem, |(m, _)| m);
            if !known.iter().any(|k| k == owner) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

enum Written {
    Done,
    Failed,
    Changed,
    Interrupted,
}

fn job_running(conn: &Connection, job_id: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM jobs WHERE id = ?1 AND state = 'running')",
        [job_id],
        |r| r.get(0),
    )
}

impl From<AtlasError> for Step {
    fn from(_: AtlasError) -> Self {
        Step::Transient(EmbedError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_with_jitter_and_honours_the_service() {
        let (b, m) = (Duration::from_secs(1), Duration::from_secs(64));
        for seed in [0, 1, 99, u64::MAX] {
            for n in 1..=10 {
                let want = Duration::from_secs((1u64 << (n - 1)).min(64));
                let got = backoff(b, m, n, None, seed);
                assert!(
                    got >= want.mul_f64(0.8) && got <= want.mul_f64(1.2),
                    "n {n}: {got:?}"
                );
            }
        }
        let asked = Some(Duration::from_secs(30));
        assert!(backoff(b, m, 1, asked, 7) >= Duration::from_secs(30));
        assert!(backoff(b, m, 1, Some(Duration::from_secs(9_999)), 7) <= MAX_RETRY_AFTER);
    }

    #[test]
    fn requests_hold_at_most_16_chunks_or_16000_characters() {
        let small = vec!["a"; 40];
        let r = requests(&small, 16, 16_000);
        assert_eq!(r, [0..16, 16..32, 32..40]);
        let big = "x".repeat(1_200);
        let texts: Vec<&str> = vec![big.as_str(); 30];
        let r = requests(&texts, 16, 16_000);
        assert_eq!(r[0], 0..13, "13 × 1,200 characters fit");
        assert!(r.iter().all(|x| x.len() <= 16));
        assert_eq!(r.iter().map(|x| x.len()).sum::<usize>(), 30);
        assert!(requests(&[], 16, 16_000).is_empty());
    }
}
