//! The scan job processor: Phase 0's inventory worker (phase0-spec §7.1–7.3,
//! ADR 0005), run by the generic job runner (ADR 0009).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};

use same_file::Handle;

use super::db::{self, Batch, DirRow, FileRecord};
use super::rate::TokenBucket;
use super::{InventoryDeps, classify};
use crate::domain::dto::{JobState, PauseReason, PolicyMode, SourceRootStatus, ThrottleReason};
use crate::domain::traits::{CatalogRepository, DirEntryInfo, EntryKind};
use crate::infrastructure::clock::now_rfc3339;
use crate::infrastructure::native_path;
use crate::services::catalog::codec::to_db;
use crate::services::catalog::repository::SqliteCatalog;
use crate::services::catalog::writer::CatalogHandle;
use crate::services::jobs::{
    self, Flow, JobProcessor, JobRow, ProcessorContext, ProcessorFactory, Signal,
};
use crate::services::library::fold::fold;
use crate::services::sources::{self, Relation};

/// Builds a [`ScanProcessor`] for each data root the workspace opens.
pub fn factory(deps: InventoryDeps) -> ProcessorFactory {
    Arc::new(move |ctx| Box::new(ScanProcessor::new(ctx, deps.clone())))
}

enum Gate {
    Pause(PauseReason),
    Go {
        rate: Option<u32>,
        throttle: Option<ThrottleReason>,
    },
}

enum DirOutcome {
    Done,
    Failed,
    /// Paused or interrupted mid-directory; the directory is replayed later.
    Interrupted,
    RootVanished,
    Stop,
}

struct Session {
    id: String,
    root_id: String,
    root: PathBuf,
    contains_data_root: bool,
    bucket: TokenBucket,
    last_sample: Option<Instant>,
    throttle: Option<ThrottleReason>,
    /// Start of the running interval not yet added to `elapsed_ms`.
    clock: Instant,
    /// Sub-millisecond remainder carried between batches.
    carry: Duration,
}

/// Walks one source root per job, in bounded batches, under the resource
/// policy and rate limit. It never opens file contents.
pub struct ScanProcessor {
    catalog: CatalogHandle,
    data_root: PathBuf,
    data_root_handle: Option<Handle>,
    deps: InventoryDeps,
    signal: Arc<Signal>,
    last_emit: Option<Instant>,
}

impl JobProcessor for ScanProcessor {
    fn idle_wait(&self) -> Duration {
        self.deps.config.idle_wait
    }

    fn started(&mut self) {
        self.emit(true);
    }

    fn run(&mut self, job: JobRow) -> Flow {
        self.run_session(job)
    }
}

impl ScanProcessor {
    pub fn new(ctx: ProcessorContext, deps: InventoryDeps) -> Self {
        let data_root =
            dunce::canonicalize(&ctx.data_root).unwrap_or_else(|_| ctx.data_root.clone());
        Self {
            catalog: ctx.catalog,
            data_root_handle: Handle::from_path(&data_root).ok(),
            data_root,
            deps,
            signal: ctx.signal,
            last_emit: None,
        }
    }

    fn now(&self) -> String {
        now_rfc3339()
    }

    /// Sends an overview to the UI, at most every `event_interval` unless
    /// `force` (state changes).
    fn emit(&mut self, force: bool) {
        let due = self
            .last_emit
            .is_none_or(|t| t.elapsed() >= self.deps.config.event_interval);
        if !(force || due) {
            return;
        }
        if let Ok(Ok(overview)) = self.catalog.call(|c| db::overview(c)) {
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
        let id = id.to_owned();
        let now = self.now();
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

    /// Whether this root lies inside another root (ADR 0003): if so, the
    /// outer root owns its files and it is never scanned on its own.
    fn is_nested(&self, root_id: &str) -> bool {
        let Ok(records) = SqliteCatalog::new(&self.catalog).list_source_root_records() else {
            return false;
        };
        let paths: Vec<Option<PathBuf>> = records
            .iter()
            .map(|r| native_path::decode(&r.path_bytes))
            .collect();
        records
            .iter()
            .zip(&paths)
            .find(|(r, _)| r.id == root_id)
            .and_then(|(_, p)| p.as_ref())
            .is_some_and(|p| sources::outermost_container(p, &records, &paths).is_some())
    }

    fn run_session(&mut self, row: JobRow) -> Flow {
        const STARTABLE: &[JobState] =
            &[JobState::Queued, JobState::PausedPolicy, JobState::Running];

        if self.is_nested(&row.source_root_id) {
            self.transition(
                &row.id,
                JobState::FailedTerminal,
                STARTABLE,
                None,
                Some("nested_root"),
            );
            self.emit(true);
            return Flow::Continue;
        }
        let Some(root) = native_path::decode(&row.root_path_bytes) else {
            self.transition(
                &row.id,
                JobState::FailedTerminal,
                STARTABLE,
                None,
                Some("path_undecodable"),
            );
            self.emit(true);
            return Flow::Continue;
        };
        if !self.transition(&row.id, JobState::Running, STARTABLE, None, None) {
            return Flow::Continue;
        }
        let seed_id = row.id.clone();
        let root_rel = native_path::encode(Path::new(""));
        if !matches!(
            self.catalog
                .call(move |c| db::seed_frontier(c, &seed_id, &root_rel)),
            Ok(Ok(()))
        ) {
            return self.database_failure(&row.id);
        }
        self.emit(true);

        let mut s = Session {
            contains_data_root: sources::relation_by_components(&self.data_root, &root)
                == Relation::Inside,
            id: row.id,
            root_id: row.source_root_id,
            root,
            bucket: TokenBucket::new(None),
            last_sample: None,
            throttle: row.throttle_reason,
            clock: Instant::now(),
            carry: Duration::ZERO,
        };

        loop {
            if self.signal.stopped() {
                return Flow::Stop;
            }
            let state = self.state_of(&s.id);
            match state {
                Some(JobState::Running | JobState::PausedPolicy) => {}
                // Paused or cancelled by the user, removed, or finished.
                _ => {
                    self.emit(true);
                    return Flow::Continue;
                }
            }

            let due = s
                .last_sample
                .is_none_or(|t| t.elapsed() >= self.deps.config.recheck_interval);
            if (due || state == Some(JobState::PausedPolicy))
                && let Some(flow) = self.apply_gate(&mut s, state)
            {
                match flow {
                    Flow::Stop => return Flow::Stop,
                    Flow::Continue => continue,
                }
            }

            let id = s.id.clone();
            let max = self.deps.config.max_dir_attempts;
            let next = match self.catalog.call(move |c| db::next_dir(c, &id, max)) {
                Ok(Ok(next)) => next,
                _ => return self.database_failure(&s.id),
            };
            let Some(dir) = next else {
                return self.finish(&s);
            };
            match self.process_dir(&mut s, dir) {
                DirOutcome::Done | DirOutcome::Failed | DirOutcome::Interrupted => {}
                DirOutcome::Stop => return Flow::Stop,
                DirOutcome::RootVanished => {
                    self.pause_for(&s, PauseReason::SourceRootMissing);
                    s.last_sample = Some(Instant::now());
                    if self.signal.wait(self.deps.config.policy_wait) {
                        return Flow::Stop;
                    }
                }
            }
        }
    }

    fn pause_for(&mut self, s: &Session, reason: PauseReason) {
        let changed = self.transition(
            &s.id,
            JobState::PausedPolicy,
            &[JobState::Running, JobState::PausedPolicy],
            Some(reason),
            None,
        );
        self.emit(changed);
    }

    /// Samples the device and the source root. `Some` means the caller
    /// should not process a directory now.
    fn apply_gate(&mut self, s: &mut Session, state: Option<JobState>) -> Option<Flow> {
        s.last_sample = Some(Instant::now());
        match self.gate(&s.root) {
            Gate::Pause(reason) => {
                self.pause_for(s, reason);
                if self.signal.wait(self.deps.config.policy_wait) {
                    return Some(Flow::Stop);
                }
                s.clock = Instant::now();
                Some(Flow::Continue)
            }
            Gate::Go { rate, throttle } => {
                if state == Some(JobState::PausedPolicy) {
                    let resumed = self.transition(
                        &s.id,
                        JobState::Running,
                        &[JobState::PausedPolicy],
                        None,
                        None,
                    );
                    if !resumed {
                        return Some(Flow::Continue);
                    }
                    s.clock = Instant::now();
                    self.emit(true);
                }
                if throttle != s.throttle {
                    s.throttle = throttle;
                    let id = s.id.clone();
                    let _ = self
                        .catalog
                        .call(move |c| jobs::db::set_throttle(c, &id, throttle));
                    self.emit(true);
                }
                s.bucket.set_rate(rate);
                None
            }
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
        match check.policy.mode {
            PolicyMode::Paused => Gate::Pause(
                check
                    .policy
                    .pause_reason
                    .unwrap_or(PauseReason::LowDiskSpace),
            ),
            PolicyMode::Run | PolicyMode::Slow => Gate::Go {
                rate: if self.deps.config.unthrottled {
                    None
                } else {
                    self.deps
                        .config
                        .rate_override
                        .or(check.policy.max_metadata_ops_per_second)
                },
                throttle: check.policy.throttle_reason,
            },
        }
    }

    fn is_data_root(&self, path: &Path) -> bool {
        path == self.data_root
            || self
                .data_root_handle
                .as_ref()
                .is_some_and(|h| Handle::from_path(path).is_ok_and(|p| &p == h))
    }

    fn process_dir(&mut self, s: &mut Session, dir: DirRow) -> DirOutcome {
        let Some(rel) = native_path::decode(&dir.rel_path) else {
            self.record_failure(s, &dir, "path_undecodable");
            return DirOutcome::Failed;
        };
        let abs = s.root.join(&rel);
        let source = Arc::clone(&self.deps.source);
        let entries = match source.read_dir(&abs) {
            Ok(entries) => entries,
            Err(err) => {
                if !s.root.is_dir() {
                    return DirOutcome::RootVanished;
                }
                self.record_failure(s, &dir, error_code(&err));
                return DirOutcome::Failed;
            }
        };

        let batch_size = self.deps.config.batch_size.max(1);
        let mut batch = Batch::default();
        for entry in entries {
            match entry {
                Ok(info) => self.classify(s, &rel, info, &mut batch),
                Err(_) => batch.entry_errors += 1,
            }
            if batch.len() >= batch_size
                && let Some(outcome) = self.flush(s, &mut batch, None)
            {
                return outcome;
            }
        }

        if !s.root.is_dir() {
            return DirOutcome::RootVanished;
        }
        match self.flush(s, &mut batch, Some(dir.id)) {
            Some(outcome) => outcome,
            None => DirOutcome::Done,
        }
    }

    fn classify(&self, s: &Session, rel: &Path, info: DirEntryInfo, batch: &mut Batch) {
        let rel_child = rel.join(&info.name);
        match info.kind {
            EntryKind::Symlink => batch.skipped_symlinks += 1,
            EntryKind::Directory => {
                if s.contains_data_root && self.is_data_root(&s.root.join(&rel_child)) {
                    batch.skipped_entries += 1;
                } else {
                    batch.dirs.push(native_path::encode(&rel_child));
                }
            }
            EntryKind::File => match classify::file_type(&info.name) {
                Some((kind, extension)) => {
                    let mtime_secs = info
                        .modified
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .and_then(|d| i64::try_from(d.as_secs()).ok());
                    let display_name = info.name.to_string_lossy().into_owned();
                    batch.files.push(FileRecord {
                        display_path: native_path::display(&s.root.join(&rel_child)),
                        search_key: fold(&native_path::display(&rel_child)),
                        name_key: fold(&display_name),
                        display_name,
                        rel_path: native_path::encode(&rel_child),
                        extension,
                        file_type: to_db(&kind),
                        size_bytes: info.size_bytes,
                        modified_at: mtime_secs.and_then(rfc3339_from_secs),
                        mtime_secs,
                        cloud_only: info.cloud_only,
                    });
                }
                None => batch.skipped_entries += 1,
            },
            EntryKind::Other => batch.skipped_entries += 1,
        }
    }

    /// Commits the batch, rate-limits, and checks whether to keep going.
    /// `None` means continue with the directory.
    fn flush(
        &mut self,
        s: &mut Session,
        batch: &mut Batch,
        done_dir: Option<i64>,
    ) -> Option<DirOutcome> {
        let ops = batch.len() as u32;
        batch.done_dir = done_dir;
        let now = Instant::now();
        // Running time includes rate-limit waits; policy and user pauses
        // reset `clock` and are excluded.
        let span = s.carry + now.duration_since(s.clock);
        batch.elapsed_ms = span.as_millis() as u64;
        s.carry = span - Duration::from_millis(batch.elapsed_ms);
        s.clock = now;

        let committed = std::mem::take(batch);
        let (id, root_id, ts) = (s.id.clone(), s.root_id.clone(), self.now());
        match self
            .catalog
            .call(move |c| db::commit_batch(c, &id, &root_id, &committed, &ts))
        {
            Ok(Ok(db::Committed::Yes)) => {}
            // Paused or cancelled while this batch was listed: drop it. The
            // directory stays pending and is listed again on resume.
            Ok(Ok(db::Committed::NotRunning)) => return Some(DirOutcome::Interrupted),
            // Most likely the folder was removed while scanning (cascade).
            _ => {
                if self.state_of(&s.id).is_some()
                    && let Flow::Stop = self.database_failure(&s.id)
                {
                    return Some(DirOutcome::Stop);
                }
                return Some(DirOutcome::Interrupted);
            }
        }
        self.emit(false);

        let wait = s.bucket.reserve(ops);
        if self.signal.wait(wait) {
            return Some(DirOutcome::Stop);
        }
        if done_dir.is_some() {
            return None;
        }
        if self.signal.stopped() {
            return Some(DirOutcome::Stop);
        }
        if self.state_of(&s.id) != Some(JobState::Running) {
            return Some(DirOutcome::Interrupted);
        }
        let due = s
            .last_sample
            .is_none_or(|t| t.elapsed() >= self.deps.config.recheck_interval);
        if due {
            s.last_sample = Some(Instant::now());
            match self.gate(&s.root) {
                Gate::Pause(PauseReason::SourceRootMissing) => {
                    return Some(DirOutcome::RootVanished);
                }
                Gate::Pause(reason) => {
                    self.pause_for(s, reason);
                    return Some(DirOutcome::Interrupted);
                }
                Gate::Go { rate, throttle } => {
                    s.bucket.set_rate(rate);
                    if throttle != s.throttle {
                        s.throttle = throttle;
                        let id = s.id.clone();
                        let _ = self
                            .catalog
                            .call(move |c| jobs::db::set_throttle(c, &id, throttle));
                        self.emit(true);
                    }
                }
            }
        }
        None
    }

    fn record_failure(&mut self, s: &Session, dir: &DirRow, code: &'static str) {
        let (id, dir_id, now) = (s.id.clone(), dir.id, self.now());
        let max = self.deps.config.max_dir_attempts;
        let _ = self
            .catalog
            .call(move |c| db::record_dir_failure(c, dir_id, &id, code, max, &now));
        self.emit(false);
    }

    fn finish(&mut self, s: &Session) -> Flow {
        let reachable = s.root.is_dir();
        if !reachable {
            self.pause_for(s, PauseReason::SourceRootMissing);
            return if self.signal.wait(self.deps.config.policy_wait) {
                Flow::Stop
            } else {
                Flow::Continue
            };
        }
        let (id, root_id, now) = (s.id.clone(), s.root_id.clone(), self.now());
        match self
            .catalog
            .call(move |c| db::finish(c, &id, &root_id, reachable, &now))
        {
            Ok(Ok(())) => {
                self.emit(true);
                Flow::Continue
            }
            _ => self.database_failure(&s.id),
        }
    }

    /// Bounded retries with exponential backoff, then a terminal failure.
    fn database_failure(&mut self, id: &str) -> Flow {
        let retry_id = id.to_owned();
        let retries = self
            .catalog
            .call(move |c| jobs::db::bump_retry(c, &retry_id))
            .ok()
            .and_then(Result::ok)
            .unwrap_or(u32::MAX);
        const FROM: &[JobState] = &[JobState::Running, JobState::PausedPolicy, JobState::Queued];
        if retries >= 3 {
            self.transition(id, JobState::FailedTerminal, FROM, None, Some("database"));
            self.emit(true);
            return Flow::Continue;
        }
        self.transition(id, JobState::FailedRetryable, FROM, None, Some("database"));
        self.emit(true);
        if self.signal.wait(Duration::from_secs(1 << retries)) {
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
}

fn error_code(err: &std::io::Error) -> &'static str {
    use std::io::ErrorKind;
    match err.kind() {
        ErrorKind::PermissionDenied => "permission_denied",
        ErrorKind::NotFound => "not_found",
        ErrorKind::TimedOut => "cloud_listing_timed_out",
        ErrorKind::NotADirectory => "not_a_folder",
        _ => "read_failed",
    }
}

fn rfc3339_from_secs(secs: i64) -> Option<String> {
    time::OffsetDateTime::from_unix_timestamp(secs)
        .ok()?
        .format(&time::format_description::well_known::Rfc3339)
        .ok()
}
