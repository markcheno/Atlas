//! Keeps the map current (Slice 4.6.1, ADR 0024): one thread per open
//! library, like the folder check and full speed's watch, since a map is
//! of the whole library and every job row belongs to one folder.
//!
//! Every minute, and when asked, it looks at how much has changed since
//! the map was drawn ([`build::staleness`]). It draws when the map is due
//! and the computer is running normally (or at full speed); a redraw the
//! owner asks for also runs when work is slowed, never while paused.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::app;
use super::build::{self, Outcome};
use crate::domain::dto::{ExploreStatus, PauseReason, PolicyMode};
use crate::infrastructure::clock::Clock;
use crate::services::catalog::reader::CatalogReader;
use crate::services::catalog::writer::CatalogHandle;
use crate::services::diagnostics::{Diagnostics, Event};
use crate::services::jobs::runner::Signal;
use crate::services::system::ResourceGovernor;

/// How often the watch looks.
pub const WATCH_EVERY: Duration = Duration::from_secs(60);

/// Where the map's status goes when it changes (`explore-status`).
pub type ExploreEvents = Arc<dyn Fn(&ExploreStatus) + Send + Sync>;

/// What the commands and the watch share.
pub struct ExploreShared {
    building: AtomicBool,
    /// A redraw asked for: `Some(fresh)`.
    request: Mutex<Option<bool>>,
    held: Mutex<Option<PauseReason>>,
    signal: Arc<Signal>,
}

impl Default for ExploreShared {
    fn default() -> Self {
        Self {
            building: AtomicBool::new(false),
            request: Mutex::new(None),
            held: Mutex::new(None),
            signal: Arc::new(Signal::new()),
        }
    }
}

impl ExploreShared {
    pub fn building(&self) -> bool {
        self.building.load(Ordering::SeqCst)
    }

    pub fn held(&self) -> Option<PauseReason> {
        *self.held.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Asks for a redraw now; `fresh` draws from scratch rather than from
    /// the old map.
    pub fn request(&self, fresh: bool) {
        let mut r = self.request.lock().unwrap_or_else(|p| p.into_inner());
        *r = Some(fresh || r.unwrap_or(false));
        drop(r);
        self.signal.notify();
    }

    fn take_request(&self) -> Option<bool> {
        self.request
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
    }

    fn put_back(&self, fresh: bool) {
        let mut r = self.request.lock().unwrap_or_else(|p| p.into_inner());
        *r = Some(fresh || r.unwrap_or(false));
    }

    fn set_held(&self, reason: Option<PauseReason>) {
        *self.held.lock().unwrap_or_else(|p| p.into_inner()) = reason;
    }
}

pub struct ExploreDeps {
    pub catalog: CatalogHandle,
    pub reader: Arc<CatalogReader>,
    pub governor: Arc<Mutex<ResourceGovernor>>,
    pub data_root: PathBuf,
    pub shared: Arc<ExploreShared>,
    pub events: Option<ExploreEvents>,
    pub diagnostics: Diagnostics,
    pub clock: Arc<dyn Clock>,
    pub every: Duration,
}

/// The watch's thread. Dropping it stops the thread (a build in progress
/// ends at its next step) and waits.
pub struct ExploreWatch {
    signal: Arc<Signal>,
    thread: Option<JoinHandle<()>>,
}

impl ExploreWatch {
    pub fn start(deps: ExploreDeps) -> Option<Self> {
        let signal = Arc::clone(&deps.shared.signal);
        let thread = thread::Builder::new()
            .name("atlas-explore".into())
            .spawn(move || run(&deps))
            .ok()?;
        Some(Self {
            signal,
            thread: Some(thread),
        })
    }
}

impl Drop for ExploreWatch {
    fn drop(&mut self) {
        self.signal.request_stop();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Background work gets half the cores at most.
fn threads() -> usize {
    std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).max(1))
}

fn emit(deps: &ExploreDeps) {
    if let Some(events) = &deps.events
        && let Ok(Ok(status)) = deps.reader.read(|c| app::status(c, &deps.shared))
    {
        events(&status);
    }
}

fn run(deps: &ExploreDeps) {
    let stop = &deps.shared.signal;
    loop {
        if stop.stopped() {
            break;
        }
        let request = deps.shared.take_request();
        let due = matches!(deps.reader.read(build::staleness), Ok(Ok(s)) if s.due());
        if request.is_some() || due {
            let policy = deps
                .governor
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .check(Some(&deps.data_root))
                .policy;
            let allowed = match request {
                Some(_) => policy.mode != PolicyMode::Paused,
                None => policy.mode == PolicyMode::Run,
            };
            if allowed {
                deps.shared.set_held(None);
                deps.shared.building.store(true, Ordering::SeqCst);
                emit(deps);
                let outcome = build::build(
                    &deps.catalog,
                    &deps.reader,
                    &deps.clock.now_rfc3339(),
                    request.unwrap_or(false),
                    threads(),
                    &|| stop.stopped(),
                );
                deps.shared.building.store(false, Ordering::SeqCst);
                if let Ok(Outcome::Built(b)) = &outcome {
                    deps.diagnostics.record(Event::ExploreBuilt {
                        books: b.books as u64,
                        areas: b.areas as u32,
                        kept: b.kept as u32,
                        ms: b.ms,
                    });
                }
                emit(deps);
            } else if let Some(fresh) = request {
                // Asked for, but held: kept until the computer allows.
                deps.shared.put_back(fresh);
                if deps.shared.held() != policy.pause_reason {
                    deps.shared.set_held(policy.pause_reason);
                    emit(deps);
                }
            }
        }
        let wait = if deps.shared.held().is_some() {
            // A held request looks again soon.
            deps.every.min(Duration::from_secs(5))
        } else {
            deps.every
        };
        if stop.wait(wait) {
            break;
        }
    }
}
