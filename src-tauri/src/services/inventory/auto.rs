//! Automatic folder checks (Slice 4.5, phase4-spec §7.4, ADR 0022).
//!
//! Each open library has a [`Checker`]: a thread that, once at open and
//! then every minute, probes every library folder and queues an ordinary
//! scan job (origin `automatic`) for a folder that
//! - is reachable, and its last complete scan is older than the interval
//!   in `background.checkFolders`; or
//! - was unreachable at the last check and is reachable now.
//!
//! It never queues a scan for a folder that already has one scheduled, for
//! a folder that is unreachable or inside another, or when the setting is
//! "Only when I ask". A scan paused because its folder went away waits
//! outside the runner; when the folder is reachable again the checker
//! queues it once more (whatever the setting: it is the user's scan
//! carrying on).
//!
//! The scan itself has no special path: the resource policy, Pause and
//! Cancel apply as to any scan.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::{InventoryEvents, db};
use crate::domain::dto::{
    CheckFolders, JobKind, JobState, PauseReason, ScanOrigin, SourceRootStatus,
};
use crate::domain::traits::CatalogRepository;
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::clock::{Clock, parse_rfc3339};
use crate::infrastructure::native_path;
use crate::services::catalog::repository::SqliteCatalog;
use crate::services::catalog::writer::CatalogHandle;
use crate::services::jobs::{self, JobWaker, Signal};
use crate::services::sources;

/// How often the checker looks. A folder is checked at most once an
/// interval; a returning drive within a minute.
pub const CHECK_EVERY: Duration = Duration::from_secs(60);

/// The setting the checker reads, changed by Settings while it runs.
pub type CheckSetting = Arc<Mutex<CheckFolders>>;

/// What one check did, for the tests and the log.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Checked {
    /// Folders a new automatic scan was queued for.
    pub queued: Vec<String>,
    /// Folders whose scan, paused while they were away, was queued again.
    pub resumed: Vec<String>,
    /// Folders whose reachability changed since the last check.
    pub changed: Vec<String>,
}

/// What the checker remembers between checks: each folder's last probed
/// reachability, to notice one coming back.
#[derive(Debug, Default)]
pub struct CheckState {
    last: HashMap<String, SourceRootStatus>,
}

impl CheckState {
    pub fn status(&self, root_id: &str) -> Option<SourceRootStatus> {
        self.last.get(root_id).copied()
    }
}

/// One check, at `clock`'s now. Pure but for the catalog and the probes,
/// so tests call it directly with a settable clock.
pub fn check_if_due(
    catalog: &CatalogHandle,
    clock: &dyn Clock,
    setting: CheckFolders,
    state: &mut CheckState,
) -> AtlasResult<Checked> {
    let records = SqliteCatalog::new(catalog).list_source_root_records()?;
    let paths: Vec<Option<PathBuf>> = records
        .iter()
        .map(|r| native_path::decode(&r.path_bytes))
        .collect();
    let statuses = sources::probe::probe_all(&paths);
    let now = clock.now();
    let mut out = Checked::default();
    for ((record, path), status) in records.iter().zip(&paths).zip(statuses) {
        let before = state.last.insert(record.id.clone(), status);
        if before.is_some_and(|b| b != status) {
            out.changed.push(record.id.clone());
        }
        let nested = path
            .as_ref()
            .is_some_and(|p| sources::outermost_container(p, &records, &paths).is_some());
        if nested || status != SourceRootStatus::Available {
            continue;
        }
        let came_back = before.is_some_and(|b| b != SourceRootStatus::Available);
        let interval = setting.interval();
        let (root_id, ts) = (record.id.clone(), clock.now_rfc3339());
        let acted = catalog
            .call(move |c| {
                if let Some(active) = jobs::db::active_for_root(c, &root_id, JobKind::Scan)? {
                    let away = active.state == JobState::PausedPolicy
                        && active.pause_reason == Some(PauseReason::SourceRootMissing);
                    if away {
                        let queued = jobs::db::transition(
                            c,
                            &active.id,
                            JobState::Queued,
                            &[JobState::PausedPolicy],
                            None,
                            None,
                            &ts,
                        )?;
                        return Ok(queued.then_some(Act::Resumed));
                    }
                    return Ok(None);
                }
                // Fresh means scanned, or a scan started (one the user
                // cancelled included), within the interval.
                let last: Option<String> = c.query_row(
                    "SELECT MAX(t) FROM (
                         SELECT last_scan_at AS t FROM source_roots WHERE id = ?1
                         UNION ALL
                         SELECT created_at FROM jobs WHERE source_root_id = ?1 AND kind = 'scan')",
                    [&root_id],
                    |r| r.get(0),
                )?;
                let stale = interval.is_some_and(|interval| {
                    last.as_deref().and_then(parse_rfc3339).is_none_or(|at| {
                        now - at
                            >= time::Duration::try_from(interval).unwrap_or(time::Duration::MAX)
                    })
                });
                let wanted = setting != CheckFolders::Never && (stale || came_back);
                if !wanted {
                    return Ok(None);
                }
                jobs::db::insert_job_from(
                    c,
                    &uuid::Uuid::new_v4().to_string(),
                    &root_id,
                    JobKind::Scan,
                    ScanOrigin::Automatic,
                    &ts,
                )?;
                Ok::<_, rusqlite::Error>(Some(Act::Queued))
            })?
            .map_err(AtlasError::db("check folders"))?;
        match acted {
            Some(Act::Queued) => out.queued.push(record.id.clone()),
            Some(Act::Resumed) => out.resumed.push(record.id.clone()),
            None => {}
        }
    }
    Ok(out)
}

enum Act {
    Queued,
    Resumed,
}

/// Everything the checker's thread needs.
#[derive(Clone)]
pub struct CheckerDeps {
    pub catalog: CatalogHandle,
    pub waker: Option<JobWaker>,
    pub clock: Arc<dyn Clock>,
    pub setting: CheckSetting,
    pub events: InventoryEvents,
    pub every: Duration,
}

/// The folder check's thread for one open library. Dropping it stops the
/// thread and waits.
pub struct Checker {
    signal: Arc<Signal>,
    thread: Option<JoinHandle<()>>,
}

impl Checker {
    /// Starts checking now, then every `deps.every`.
    pub fn start(deps: CheckerDeps) -> Option<Checker> {
        let signal = Arc::new(Signal::new());
        let stop = Arc::clone(&signal);
        let thread = thread::Builder::new()
            .name("atlas-folder-check".into())
            .spawn(move || {
                let mut state = CheckState::default();
                loop {
                    let setting = *deps.setting.lock().unwrap_or_else(|p| p.into_inner());
                    if let Ok(done) = check_if_due(&deps.catalog, &*deps.clock, setting, &mut state)
                    {
                        if !(done.queued.is_empty() && done.resumed.is_empty())
                            && let Some(waker) = &deps.waker
                        {
                            waker.wake();
                        }
                        // A folder coming or going shows in the folder list.
                        if !(done.changed.is_empty()
                            && done.queued.is_empty()
                            && done.resumed.is_empty())
                            && let Ok(Ok(overview)) = deps.catalog.call(|c| db::overview(c))
                        {
                            (deps.events)(&overview);
                        }
                    }
                    if stop.wait(deps.every) {
                        break;
                    }
                }
            })
            .ok()?;
        Some(Checker {
            signal,
            thread: Some(thread),
        })
    }
}

impl Drop for Checker {
    fn drop(&mut self) {
        self.signal.request_stop();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
