//! Diagnostics (Slice 4.1, phase4-spec §7.1, ADR 0021): a local log of
//! what Atlas did, in counts and timings, and a report the user can save.
//! Nothing is sent anywhere.
//!
//! [`Diagnostics`] is a cheap handle. While a library is open it writes
//! [`Event`]s to that library's `logs/` on a thread of its own, behind a
//! bounded queue: a full queue drops events and counts them, and a log
//! that can't be written is switched off, so recording never fails or
//! slows the work it describes. With no library open, nothing is written.

pub mod event;
pub mod log;
pub mod report;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Instant;

use serde::{Deserialize, Serialize};

pub use event::{Event, FailureReason};
pub use log::{LOGS_DIR, Limits};

use log::LogWriter;

use crate::infrastructure::clock::now_rfc3339;

/// One line of the log: when, and what.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogLine {
    pub at: String,
    #[serde(flatten)]
    pub event: Event,
}

enum Msg {
    Line { date: String, text: String },
    Clear(mpsc::Sender<()>),
    Flush(mpsc::Sender<()>),
}

struct Sink {
    root: PathBuf,
    tx: SyncSender<Msg>,
    writer: Arc<Mutex<LogWriter>>,
    thread: Option<JoinHandle<()>>,
}

struct Inner {
    limits: Limits,
    sink: Mutex<Option<Sink>>,
    dropped: Arc<AtomicU64>,
    started: AtomicBool,
}

#[derive(Clone)]
pub struct Diagnostics {
    inner: Arc<Inner>,
}

impl Default for Diagnostics {
    fn default() -> Self {
        Self::with_limits(Limits::default())
    }
}

impl std::fmt::Debug for Diagnostics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Diagnostics")
    }
}

impl Diagnostics {
    pub fn new() -> Self {
        Self::default()
    }

    /// Smaller files and folder, for tests.
    pub fn with_limits(limits: Limits) -> Self {
        Self {
            inner: Arc::new(Inner {
                limits,
                sink: Mutex::new(None),
                dropped: Arc::new(AtomicU64::new(0)),
                started: AtomicBool::new(false),
            }),
        }
    }

    /// Records `event` in the open library's log, if one is open.
    pub fn record(&self, event: Event) {
        let sink = self.inner.sink.lock().unwrap_or_else(|p| p.into_inner());
        let Some(sink) = sink.as_ref() else {
            return;
        };
        let (date, text) = line(event);
        match sink.tx.try_send(Msg::Line { date, text }) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => {
                self.inner.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Writes `event` at once on this thread, past the queue: for a panic,
    /// which may end the process before the writer runs. Gives up rather
    /// than wait on a lock the panicking thread may hold.
    pub fn record_now(&self, event: Event) {
        let Ok(sink) = self.inner.sink.try_lock() else {
            return;
        };
        let Some(sink) = sink.as_ref() else {
            return;
        };
        let (date, text) = line(event);
        if let Ok(mut w) = sink.writer.try_lock() {
            w.write(&date, &text);
        }
    }

    /// Starts writing to the library at `root`, until the guard is dropped.
    /// Call with the library's lock held.
    pub fn attach(&self, root: &Path) -> LogGuard {
        self.detach();
        let writer = Arc::new(Mutex::new(LogWriter::new(
            root.join(LOGS_DIR),
            self.inner.limits,
        )));
        let (tx, rx) = mpsc::sync_channel(self.inner.limits.queue);
        let thread = {
            let writer = Arc::clone(&writer);
            let dropped = Arc::clone(&self.inner.dropped);
            std::thread::Builder::new()
                .name("atlas-log".into())
                .spawn(move || write_loop(&rx, &writer, &dropped))
                .ok()
        };
        *self.inner.sink.lock().unwrap_or_else(|p| p.into_inner()) = Some(Sink {
            root: root.to_path_buf(),
            tx,
            writer,
            thread,
        });
        if !self.inner.started.swap(true, Ordering::SeqCst) {
            let v = |i: usize| {
                env!("CARGO_PKG_VERSION")
                    .split('.')
                    .nth(i)
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(0)
            };
            self.record(Event::AppStarted {
                major: v(0),
                minor: v(1),
                patch: v(2),
            });
        }
        LogGuard {
            diagnostics: self.clone(),
            root: root.to_path_buf(),
            opened: Instant::now(),
        }
    }

    /// Stops writing: what is queued is written first.
    fn detach(&self) {
        let sink = self
            .inner
            .sink
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        if let Some(mut sink) = sink {
            drop(sink.tx);
            if let Some(t) = sink.thread.take() {
                let _ = t.join();
            }
        }
    }

    fn detach_if(&self, root: &Path) {
        let ours = self
            .inner
            .sink
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .is_some_and(|s| s.root == root);
        if ours {
            self.detach();
        }
    }

    /// Waits until everything recorded so far is written.
    pub fn flush(&self) {
        self.ask(Msg::Flush);
    }

    /// Removes the log's files; the next event starts a new one.
    pub fn clear(&self) {
        self.ask(Msg::Clear);
    }

    fn ask(&self, msg: impl FnOnce(mpsc::Sender<()>) -> Msg) {
        let (done, wait) = mpsc::channel();
        let sent = {
            let sink = self.inner.sink.lock().unwrap_or_else(|p| p.into_inner());
            sink.as_ref().is_some_and(|s| s.tx.send(msg(done)).is_ok())
        };
        if sent {
            let _ = wait.recv();
        }
    }

    /// Events dropped since the log's writer last caught up.
    pub fn dropped(&self) -> u64 {
        self.inner.dropped.load(Ordering::Relaxed)
    }

    /// Whether the open library's log has stopped after a failed write.
    pub fn is_broken(&self) -> bool {
        let sink = self.inner.sink.lock().unwrap_or_else(|p| p.into_inner());
        sink.as_ref()
            .is_some_and(|s| s.writer.lock().map_or(true, |w| w.is_broken()))
    }
}

fn line(event: Event) -> (String, String) {
    let at = now_rfc3339();
    let date = at.get(..10).unwrap_or("1970-01-01").to_owned();
    let text = serde_json::to_string(&LogLine { at, event }).unwrap_or_default();
    (date, text)
}

fn write_loop(rx: &Receiver<Msg>, writer: &Mutex<LogWriter>, dropped: &AtomicU64) {
    for msg in rx {
        let mut w = writer.lock().unwrap_or_else(|p| p.into_inner());
        match msg {
            Msg::Line { date, text } => {
                let lost = dropped.swap(0, Ordering::Relaxed);
                if lost > 0 {
                    let (d, t) = line(Event::EventsDropped { count: lost });
                    w.write(&d, &t);
                }
                w.write(&date, &text);
            }
            Msg::Clear(done) => {
                w.clear();
                let (d, t) = line(Event::LogCleared);
                w.write(&d, &t);
                let _ = done.send(());
            }
            Msg::Flush(done) => {
                let _ = done.send(());
            }
        }
    }
}

/// Writing to one library's log; dropping it records how long the library
/// was open, writes what is queued, and stops.
pub struct LogGuard {
    diagnostics: Diagnostics,
    root: PathBuf,
    opened: Instant,
}

impl LogGuard {
    pub fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }
}

impl Drop for LogGuard {
    fn drop(&mut self) {
        self.diagnostics.record(Event::LibraryClosed {
            open_secs: self.opened.elapsed().as_secs(),
        });
        self.diagnostics.detach_if(&self.root);
    }
}

/// The app's handle, for what has no library handle at hand: a panic, and
/// errors returned to the window. Set once, in `run()`; tests leave it
/// unset.
static APP: OnceLock<Diagnostics> = OnceLock::new();

/// Makes `d` the app's handle and records panics in it. The default hook
/// still runs after, so a developer still sees the message on stderr.
pub fn install(d: Diagnostics) {
    if APP.set(d).is_err() {
        return;
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let (Some(d), Some(at)) = (APP.get(), info.location()) {
            d.record_now(Event::Panic {
                file: event::SourceFile::of_location(at),
                line: at.line(),
                column: at.column(),
            });
        }
        previous(info);
    }));
}

/// Records in the app's handle, if there is one.
pub fn record_app(event: Event) {
    if let Some(d) = APP.get() {
        d.record(event);
    }
}

/// Reads the last `limit` lines of the log in `logs`, oldest first. Lines
/// that aren't events (a torn last line after a kill) are skipped.
pub fn tail(logs: &Path, limit: usize) -> Vec<LogLine> {
    let mut out = std::collections::VecDeque::with_capacity(limit.min(4096));
    for (_, path, _) in log::files(logs) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for l in text.lines() {
            if let Ok(line) = serde_json::from_str::<LogLine>(l) {
                if out.len() == limit {
                    out.pop_front();
                }
                out.push_back(line);
            }
        }
    }
    out.into()
}

/// The log and the reports of the library at `root`.
pub fn status(root: &Path, diagnostics: &Diagnostics) -> crate::domain::dto::DiagnosticsStatus {
    let files = log::files(&root.join(LOGS_DIR));
    let reports = report::saved(root);
    let last_report_at = reports
        .last()
        .and_then(|(path, _)| std::fs::metadata(path).ok()?.modified().ok())
        .map(|t| crate::infrastructure::clock::rfc3339(t.into()));
    crate::domain::dto::DiagnosticsStatus {
        log_bytes: files.iter().map(|f| f.2).sum(),
        log_files: files.len() as u32,
        dropped_events: diagnostics.dropped(),
        log_stopped: diagnostics.is_broken(),
        reports: reports.len() as u32,
        last_report_at,
    }
}
