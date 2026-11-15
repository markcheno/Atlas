//! Ends full speed when the work runs out (Slice 4.6): one thread per open
//! library, checking every few seconds while full speed is on whether any
//! scan, reading or indexing job still has work. Two empty checks in a row
//! end it, so the moment between a scan finishing and its reading being
//! queued doesn't. Closing the library ends it too.

use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::domain::dto::ResourceStatus;
use crate::error::{AtlasError, AtlasResult};
use crate::services::catalog::writer::CatalogHandle;
use crate::services::diagnostics::event::FullSpeedEnd;
use crate::services::jobs::db::any_runnable;
use crate::services::jobs::runner::Signal;
use crate::services::system::ResourceGovernor;
use crate::services::workspace::OpenRoot;

/// How often the watch looks while full speed is on.
pub const WATCH_EVERY: Duration = Duration::from_secs(5);

pub struct FullSpeedWatch {
    signal: Arc<Signal>,
    governor: Arc<Mutex<ResourceGovernor>>,
    thread: Option<JoinHandle<()>>,
}

impl FullSpeedWatch {
    pub fn start(
        catalog: CatalogHandle,
        governor: Arc<Mutex<ResourceGovernor>>,
        every: Duration,
    ) -> Option<Self> {
        let signal = Arc::new(Signal::new());
        let stop = Arc::clone(&signal);
        let watched = Arc::clone(&governor);
        let thread = thread::Builder::new()
            .name("atlas-full-speed".into())
            .spawn(move || {
                let mut empty = 0;
                loop {
                    let on = lock(&watched).full_speed();
                    if on {
                        match catalog.call(|c| any_runnable(c)) {
                            Ok(Ok(false)) => empty += 1,
                            _ => empty = 0,
                        }
                        if empty >= 2 {
                            lock(&watched).stop_full_speed(FullSpeedEnd::Done);
                            empty = 0;
                        }
                    } else {
                        empty = 0;
                    }
                    if stop.wait(every) {
                        break;
                    }
                }
            })
            .ok()?;
        Some(Self {
            signal,
            governor,
            thread: Some(thread),
        })
    }
}

/// "Run at full speed now" and Stop (Slice 4.6). Full speed starts only
/// when work is waiting and it's offered; either way the jobs are woken,
/// so a job resting between files or documents sees the change at once.
pub fn set_full_speed(
    root: &OpenRoot,
    governor: &Mutex<ResourceGovernor>,
    on: bool,
) -> AtlasResult<ResourceStatus> {
    let status = if on {
        let waiting = root
            .catalog()
            .call(|c| any_runnable(c))
            .map_err(|_| AtlasError::Internal("the catalog is unavailable"))?
            .map_err(AtlasError::db("look for waiting work"))?;
        if !waiting {
            return Err(AtlasError::NothingWaiting);
        }
        lock(governor).start_full_speed(Some(root.path()))?
    } else {
        lock(governor).stop_full_speed(FullSpeedEnd::Stopped)
    };
    root.wake_jobs();
    Ok(status)
}

fn lock(g: &Mutex<ResourceGovernor>) -> std::sync::MutexGuard<'_, ResourceGovernor> {
    g.lock().unwrap_or_else(|p| p.into_inner())
}

impl Drop for FullSpeedWatch {
    fn drop(&mut self) {
        self.signal.request_stop();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        if lock(&self.governor).full_speed() {
            lock(&self.governor).stop_full_speed(FullSpeedEnd::Closed);
        }
    }
}
