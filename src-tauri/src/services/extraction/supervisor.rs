//! The app side of the worker (phase1-spec §7.2): spawns `atlas
//! --extract-worker`, speaks the protocol, and kills and restarts the worker
//! when it hangs, grows past the memory cap, or dies. PDFium never runs in
//! this process.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use super::engine::hash::{HashError, Hashed};
use super::engine::{Outcome, Piece, Summary};
use super::lifetime;
use super::locate::Unavailable;
use super::protocol::{Frame, RenderRect, Request, encode_path};
use super::{PROTOCOL_VERSION, WORKER_ARG};
use crate::domain::dto::FileType;
use crate::services::diagnostics::event::WorkerRole;
use crate::services::diagnostics::{Diagnostics, Event};

/// How to start the worker. The app uses its own executable; tests and
/// benches pass the `atlas` binary Cargo built (under `cargo test`,
/// `current_exe` is the test binary).
#[derive(Debug, Clone)]
pub struct WorkerCommand {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub env: Vec<(OsString, OsString)>,
}

impl WorkerCommand {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: vec![WORKER_ARG.into()],
            env: Vec::new(),
        }
    }

    pub fn current_exe() -> std::io::Result<Self> {
        Ok(Self::new(std::env::current_exe()?))
    }

    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }
}

#[derive(Debug, Clone)]
pub struct SupervisorConfig {
    /// A request is abandoned after this long without a frame (any frame
    /// resets it, so a long book is fine and a hung parser is not).
    pub inactivity: Duration,
    /// The worker is killed above this resident size.
    pub rss_cap_bytes: u64,
    /// How often stop requests and memory are checked while waiting.
    pub poll: Duration,
    /// How long a fresh worker may take to say hello.
    pub start_timeout: Duration,
}

impl Default for SupervisorConfig {
    fn default() -> Self {
        Self {
            inactivity: Duration::from_secs(30),
            rss_cap_bytes: default_rss_cap(),
            poll: Duration::from_millis(250),
            start_timeout: Duration::from_secs(30),
        }
    }
}

/// `min(1 GiB, 15% of physical memory)` (phase1-spec §7.2).
pub fn default_rss_cap() -> u64 {
    const GIB: u64 = 1024 * 1024 * 1024;
    let Ok(mut sys) = sysinfo::System::new() else {
        return GIB;
    };
    sys.refresh_memory();
    let total = sys.total_memory();
    if total == 0 {
        GIB
    } else {
        GIB.min(total / 100 * 15)
    }
}

/// Counters for tests, benchmarks and the status line.
#[derive(Debug, Default)]
pub struct WorkerStats {
    pub spawns: AtomicU64,
    pub hashes: AtomicU64,
    pub extracts: AtomicU64,
    pub kills: AtomicU64,
    /// Pages rendered for the page view (Slice 3.7).
    pub renders: AtomicU64,
    /// The worker's largest resident size seen while polling, in bytes.
    pub peak_rss_bytes: AtomicU64,
}

impl WorkerStats {
    pub fn get(c: &AtomicU64) -> u64 {
        c.load(Ordering::SeqCst)
    }
}

/// Why a request got no answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerError {
    /// PDFium is missing or broken; restarting won't help.
    Unavailable(Unavailable),
    /// The worker hung, outgrew the cap, or died. It has been killed; the
    /// next request starts a new one.
    Crashed(Crash),
    /// The caller asked to stop (the library is closing).
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Crash {
    Timeout,
    Memory,
    Exited,
    Protocol,
}

/// The result of an `extract` request that the worker answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Extraction {
    Done {
        summary: Summary,
        pieces: Vec<Piece>,
    },
    Failed {
        outcome: Outcome,
        reason: String,
        /// For a PDF that needs OCR: its pages, and those with usable text
        /// (Slice 4.1). `None` otherwise.
        pages: Option<PageCounts>,
    },
}

/// What a document that needs OCR is known to hold (Slice 4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageCounts {
    pub page_count: u32,
    pub text_pages: u32,
}

/// A page the worker rendered (Slice 3.7): the PNG's bytes, and where the
/// passage asked for is on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedPage {
    pub page: u32,
    pub page_count: u32,
    pub width: u32,
    pub height: u32,
    pub png: Vec<u8>,
    pub rects: Vec<RenderRect>,
    pub found: bool,
}

/// Text extraction as the extraction job sees it (phase1-spec §9). The
/// supervisor is the real implementation; tests can script one.
pub trait DocumentProcessor: Send {
    fn hash(&mut self, path: &Path) -> Result<Result<Hashed, HashError>, WorkerError>;
    fn extract(&mut self, path: &Path, kind: FileType) -> Result<Extraction, WorkerError>;
}

/// Asked between waits whether to give up (the library is closing).
pub type StopCheck = Arc<dyn Fn() -> bool + Send + Sync>;

struct Running {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
    pid: sysinfo::Pid,
}

pub struct Supervisor {
    command: WorkerCommand,
    config: SupervisorConfig,
    stop: StopCheck,
    stats: Arc<WorkerStats>,
    running: Option<Running>,
    /// `None` when the process probe is unavailable; the memory cap is then
    /// not enforced (the inactivity timeout still is).
    sys: Option<sysinfo::System>,
    /// The protocol a refused worker reported, for the log.
    refused: Option<u32>,
    /// Where starts, crashes and refusals are recorded (Slice 4.1).
    diagnostics: Option<(Diagnostics, WorkerRole)>,
}

impl Supervisor {
    pub fn new(
        command: WorkerCommand,
        config: SupervisorConfig,
        stop: StopCheck,
        stats: Arc<WorkerStats>,
    ) -> Self {
        Self {
            command,
            config,
            stop,
            stats,
            running: None,
            sys: sysinfo::System::new().ok(),
            refused: None,
            diagnostics: None,
        }
    }

    /// Records this worker's starts, crashes and refusals as `role`.
    pub fn with_diagnostics(mut self, diagnostics: Diagnostics, role: WorkerRole) -> Self {
        self.diagnostics = Some((diagnostics, role));
        self
    }

    fn record(&self, event: impl FnOnce(WorkerRole) -> Event) {
        if let Some((d, role)) = &self.diagnostics {
            d.record(event(*role));
        }
    }

    /// Stops the worker, if any. The next request starts a new one.
    pub fn kill(&mut self) {
        if let Some(mut r) = self.running.take() {
            let _ = r.child.kill();
            let _ = r.child.wait();
            self.stats.kills.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn spawn(&mut self) -> Result<(), WorkerError> {
        let mut cmd = Command::new(&self.command.program);
        cmd.args(&self.command.args)
            .envs(self.command.env.iter().map(|(k, v)| (k, v)))
            .env(lifetime::PARENT_ENV, std::process::id().to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd
            .spawn()
            .map_err(|_| WorkerError::Unavailable(Unavailable::NotFound))?;
        lifetime::contain(&child);
        self.stats.spawns.fetch_add(1, Ordering::SeqCst);
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let (tx, lines) = mpsc::channel();
        std::thread::Builder::new()
            .name("atlas-worker-reader".into())
            .spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if tx.send(line).is_err() {
                        break;
                    }
                }
            })
            .map_err(|_| WorkerError::Crashed(Crash::Exited))?;
        let pid = sysinfo::Pid::from_u32(child.id());
        self.running = Some(Running {
            child,
            stdin,
            lines,
            pid,
        });
        match self.next_frame(self.config.start_timeout) {
            Ok(Frame::Hello { protocol, .. }) if protocol == PROTOCOL_VERSION => {
                self.record(|role| Event::WorkerStarted { role });
                Ok(())
            }
            // Another protocol is a mismatched install, not a crash:
            // restarting would only meet it again (Slice 4.1).
            Ok(Frame::Hello { protocol, .. }) => {
                self.kill();
                self.refused = Some(protocol);
                self.record(|role| Event::WorkerRefused { role, protocol });
                Err(WorkerError::Unavailable(Unavailable::ProtocolMismatch))
            }
            Ok(Frame::Unavailable { reason }) => {
                self.kill();
                Err(WorkerError::Unavailable(reason))
            }
            Ok(_) => {
                self.kill();
                Err(WorkerError::Crashed(Crash::Protocol))
            }
            Err(e) => Err(e),
        }
    }

    fn rss(&mut self, pid: sysinfo::Pid) -> u64 {
        let Some(sys) = self.sys.as_mut() else {
            return 0;
        };
        sys.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::Some(&[pid]),
            true,
            sysinfo::ProcessRefreshKind::nothing().with_memory(),
        );
        sys.process(pid).map_or(0, sysinfo::Process::memory)
    }

    /// Waits for the next frame, killing the worker if it goes quiet for
    /// `quiet`, outgrows the cap, or dies, and giving up on a stop request.
    fn next_frame(&mut self, quiet: Duration) -> Result<Frame, WorkerError> {
        let deadline = Instant::now() + quiet;
        loop {
            let Some(running) = self.running.as_ref() else {
                return Err(WorkerError::Crashed(Crash::Exited));
            };
            let pid = running.pid;
            match running.lines.recv_timeout(self.config.poll) {
                Ok(line) => {
                    return match serde_json::from_str::<Frame>(&line) {
                        Ok(frame) => Ok(frame),
                        Err(_) => {
                            self.kill();
                            Err(WorkerError::Crashed(Crash::Protocol))
                        }
                    };
                }
                Err(RecvTimeoutError::Disconnected) => {
                    self.kill();
                    return Err(WorkerError::Crashed(Crash::Exited));
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            if (self.stop)() {
                self.kill();
                return Err(WorkerError::Stopped);
            }
            if Instant::now() >= deadline {
                self.kill();
                return Err(WorkerError::Crashed(Crash::Timeout));
            }
            let rss = self.rss(pid);
            self.stats.peak_rss_bytes.fetch_max(rss, Ordering::SeqCst);
            if rss > self.config.rss_cap_bytes {
                self.kill();
                return Err(WorkerError::Crashed(Crash::Memory));
            }
        }
    }

    fn send(&mut self, request: &Request) -> Result<(), WorkerError> {
        if self.running.is_none() {
            self.spawn()?;
        }
        let running = self.running.as_mut().expect("spawned");
        let line = request.to_line();
        if running.stdin.write_all(line.as_bytes()).is_err() || running.stdin.flush().is_err() {
            self.kill();
            return Err(WorkerError::Crashed(Crash::Exited));
        }
        Ok(())
    }
}

impl Supervisor {
    /// Renders page `page` (1-based) of the PDF at `path`, `width` pixels
    /// wide, finding `find` on it (Slice 3.7). The inner error is the
    /// worker's fixed reason (`page_out_of_range`, `encrypted`, …).
    pub fn render(
        &mut self,
        path: &Path,
        page: u32,
        width: u32,
        find: Option<&str>,
    ) -> Result<Result<RenderedPage, String>, WorkerError> {
        let r = self.render_inner(path, page, width, find);
        self.noted(r)
    }

    fn render_inner(
        &mut self,
        path: &Path,
        page: u32,
        width: u32,
        find: Option<&str>,
    ) -> Result<Result<RenderedPage, String>, WorkerError> {
        use base64::prelude::{BASE64_STANDARD, Engine as _};
        self.send(&Request::Render {
            path: encode_path(path),
            page,
            width,
            find: find.map(str::to_owned),
        })?;
        self.stats.renders.fetch_add(1, Ordering::SeqCst);
        let frame = self.next_frame(self.config.inactivity)?;
        // A render is often done before the first memory poll: one sample
        // after it, so the peak is still seen.
        if let Some(pid) = self.running.as_ref().map(|r| r.pid) {
            let rss = self.rss(pid);
            self.stats.peak_rss_bytes.fetch_max(rss, Ordering::SeqCst);
        }
        match frame {
            Frame::Rendered {
                page,
                page_count,
                width,
                height,
                png,
                rects,
                found,
            } => match BASE64_STANDARD.decode(png) {
                Ok(png) => Ok(Ok(RenderedPage {
                    page,
                    page_count,
                    width,
                    height,
                    png,
                    rects,
                    found,
                })),
                Err(_) => {
                    self.kill();
                    Err(WorkerError::Crashed(Crash::Protocol))
                }
            },
            Frame::RenderFailed { reason } => Ok(Err(reason)),
            _ => {
                self.kill();
                Err(WorkerError::Crashed(Crash::Protocol))
            }
        }
    }

    /// Whether a worker is running now.
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// The protocol the last refused worker reported (Slice 4.1).
    pub fn refused_protocol(&self) -> Option<u32> {
        self.refused
    }
}

impl DocumentProcessor for Supervisor {
    fn hash(&mut self, path: &Path) -> Result<Result<Hashed, HashError>, WorkerError> {
        let r = self.hash_inner(path);
        self.noted(r)
    }

    fn extract(&mut self, path: &Path, kind: FileType) -> Result<Extraction, WorkerError> {
        let r = self.extract_inner(path, kind);
        self.noted(r)
    }
}

impl Supervisor {
    /// Records a crash in the log on its way to the caller.
    fn noted<T>(&self, r: Result<T, WorkerError>) -> Result<T, WorkerError> {
        if let Err(WorkerError::Crashed(crash)) = &r {
            let crash = *crash;
            self.record(|role| Event::WorkerCrashed { role, crash });
        }
        r
    }

    fn hash_inner(&mut self, path: &Path) -> Result<Result<Hashed, HashError>, WorkerError> {
        self.send(&Request::Hash {
            path: encode_path(path),
        })?;
        self.stats.hashes.fetch_add(1, Ordering::SeqCst);
        match self.next_frame(self.config.inactivity)? {
            Frame::Hashed {
                hash,
                size,
                mtime_secs,
            } => Ok(Ok(Hashed {
                hash,
                size,
                mtime_secs,
            })),
            Frame::HashFailed { reason } => Ok(Err(reason)),
            _ => {
                self.kill();
                Err(WorkerError::Crashed(Crash::Protocol))
            }
        }
    }

    fn extract_inner(&mut self, path: &Path, kind: FileType) -> Result<Extraction, WorkerError> {
        self.send(&Request::Extract {
            path: encode_path(path),
            kind,
        })?;
        self.stats.extracts.fetch_add(1, Ordering::SeqCst);
        let mut pieces = Vec::new();
        loop {
            match self.next_frame(self.config.inactivity)? {
                Frame::Page {
                    ordinal,
                    page_no,
                    location,
                    text,
                } => pieces.push(Piece {
                    ordinal,
                    page_no,
                    location,
                    text,
                }),
                Frame::Done { summary } => return Ok(Extraction::Done { summary, pieces }),
                Frame::Failed {
                    outcome,
                    reason,
                    page_count,
                    text_pages,
                } => {
                    let pages = match (page_count, text_pages) {
                        (Some(page_count), Some(text_pages)) => Some(PageCounts {
                            page_count,
                            text_pages: text_pages.min(page_count),
                        }),
                        _ => None,
                    };
                    return Ok(Extraction::Failed {
                        outcome,
                        reason,
                        pages,
                    });
                }
                _ => {
                    self.kill();
                    return Err(WorkerError::Crashed(Crash::Protocol));
                }
            }
        }
    }
}

impl Drop for Supervisor {
    /// The worker never outlives the library that started it.
    fn drop(&mut self) {
        self.kill();
    }
}
