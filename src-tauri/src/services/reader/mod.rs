//! The page view (phase3-spec §5.5, §7.5; Slice 3.7; ADR 0018): a PDF page
//! rendered inside Atlas, with the passage outlined when it is found.
//!
//! - **A second worker,** for rendering only: its own [`Supervisor`] over
//!   the extraction worker's executable, started at the first render and
//!   killed after [`ReaderConfig::idle`] without one, or when the library
//!   closes. Extraction never uses it, so neither waits for the other. The
//!   main process never loads PDFium.
//! - **Open's checks** before a file is read (`source_open::path_for`): the
//!   folder is reachable, the path is a regular file inside it, and a file
//!   only in the cloud needs the user's agreement.
//! - **The cache** (`cache.rs`) is read first, by content, so a page seen
//!   before is shown without touching the file at all.

pub mod cache;

use crate::services::diagnostics::Diagnostics;
use crate::services::diagnostics::event::WorkerRole;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use base64::prelude::{BASE64_STANDARD, Engine as _};
use rusqlite::OptionalExtension;

use crate::domain::dto::{PageImage, PageRect};
use crate::error::{AtlasError, AtlasResult};
use crate::services::extraction::engine::render::MAX_WIDTH;
use crate::services::extraction::supervisor::{
    Supervisor, SupervisorConfig, WorkerCommand, WorkerStats,
};
use crate::services::jobs::Signal;
use crate::services::knowledge::anchor::to_hex;
use crate::services::source_open;
use crate::services::workspace::OpenRoot;

/// The page cache's folder, under the data root.
pub const PREVIEWS_DIR: &str = "cache/previews";
/// The longest `find` text accepted.
pub const MAX_FIND_CHARS: usize = 2_000;

/// How the page view's worker and cache behave; tests use small values.
#[derive(Debug, Clone)]
pub struct ReaderConfig {
    pub command: WorkerCommand,
    pub supervisor: SupervisorConfig,
    /// Killed after this long without a render.
    pub idle: Duration,
    /// The cache's most, in bytes.
    pub cache_cap_bytes: u64,
    /// The library's log; the workspace sets it (Slice 4.1).
    pub diagnostics: Option<Diagnostics>,
}

impl ReaderConfig {
    pub fn new(command: WorkerCommand) -> Self {
        Self {
            command,
            supervisor: SupervisorConfig::default(),
            idle: Duration::from_secs(60),
            cache_cap_bytes: 200 * 1024 * 1024,
            diagnostics: None,
        }
    }
}

/// What a render needs from the library, looked up with the workspace
/// held; the render itself runs without it.
#[derive(Debug)]
pub struct Prepared {
    /// The document's content hash, hex: the cache key.
    pub hash: String,
    /// The page's passages, as Atlas read them.
    pub text: String,
    /// The file to read, or why it can't be.
    pub path: AtlasResult<PathBuf>,
}

/// Looks up `file_id`'s document, the page's text, and the path (with
/// Open's checks).
pub fn prepare(
    root: &OpenRoot,
    file_id: &str,
    page: u32,
    allow_download: bool,
) -> AtlasResult<Prepared> {
    let id = file_id.to_owned();
    let found: Option<(String, Vec<u8>, String)> = root
        .reader()
        .read(|c| -> rusqlite::Result<Option<(String, Vec<u8>, String)>> {
            let Some((doc, hash, kind)) = c
                .query_row(
                    "SELECT d.id, d.content_hash, f.file_type FROM file_instances f
                     JOIN documents d ON d.id = f.document_id WHERE f.id = ?1",
                    [&id],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, Vec<u8>>(1)?,
                            r.get::<_, String>(2)?,
                        ))
                    },
                )
                .optional()?
            else {
                return Ok(None);
            };
            if kind != "pdf" {
                return Ok(Some((doc, Vec::new(), kind)));
            }
            Ok(Some((doc, hash, kind)))
        })?
        .map_err(AtlasError::db("be read"))?;
    let Some((doc, hash, kind)) = found else {
        return Err(AtlasError::RenderFailed("not_read"));
    };
    if kind != "pdf" {
        return Err(AtlasError::RenderFailed("not_a_pdf"));
    }
    let text = root
        .reader()
        .read(|c| -> rusqlite::Result<String> {
            let mut stmt = c.prepare_cached(
                "SELECT text FROM passages WHERE document_id = ?1 AND page_no = ?2 ORDER BY ordinal",
            )?;
            let parts = stmt
                .query_map(rusqlite::params![doc, page], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(parts.join("\n\n"))
        })?
        .map_err(AtlasError::db("be read"))?;
    Ok(Prepared {
        hash: to_hex(&hash),
        text,
        path: source_open::path_for(root, file_id, allow_download),
    })
}

struct Shared {
    worker: Mutex<Option<Supervisor>>,
    last_used: Mutex<Instant>,
    stop: Arc<AtomicBool>,
    signal: Signal,
    stats: Arc<WorkerStats>,
    /// `None`: no worker was configured (some tests), so nothing renders.
    config: Option<ReaderConfig>,
}

impl Shared {
    fn kill_if_idle(&self) {
        let Some(config) = &self.config else {
            return;
        };
        let idle = self
            .last_used
            .lock()
            .map(|t| t.elapsed() >= config.idle)
            .unwrap_or(false);
        if idle && let Ok(mut worker) = self.worker.try_lock() {
            // Dropping the supervisor kills its worker.
            worker.take();
        }
    }
}

/// The library's page renderer: one render at a time, a worker of its own.
pub struct ReaderService {
    shared: Arc<Shared>,
    previews: PathBuf,
    idle_thread: Option<JoinHandle<()>>,
}

fn reason(worker: &str) -> &'static str {
    match worker {
        "page_out_of_range" => "page_out_of_range",
        "encrypted" => "encrypted",
        "too_large" => "too_large",
        "cloud_only" => "cloud_only",
        _ => "malformed",
    }
}

impl ReaderService {
    /// The renderer of the library at `root`. Nothing starts until the
    /// first render.
    pub fn new(root: &std::path::Path, config: Option<ReaderConfig>) -> Self {
        let shared = Arc::new(Shared {
            worker: Mutex::new(None),
            last_used: Mutex::new(Instant::now()),
            stop: Arc::new(AtomicBool::new(false)),
            signal: Signal::new(),
            stats: Arc::new(WorkerStats::default()),
            config,
        });
        let idle = shared
            .config
            .as_ref()
            .map_or(Duration::from_secs(60), |c| c.idle);
        let every = (idle / 4).clamp(Duration::from_millis(50), Duration::from_secs(5));
        let watch = Arc::clone(&shared);
        let idle_thread = std::thread::Builder::new()
            .name("atlas-reader-idle".into())
            .spawn(move || {
                while !watch.signal.wait(every) {
                    watch.kill_if_idle();
                }
            })
            .ok();
        Self {
            shared,
            previews: root.join(PREVIEWS_DIR),
            idle_thread,
        }
    }

    /// The worker's counters: spawns, renders, kills, peak memory.
    pub fn stats(&self) -> &Arc<WorkerStats> {
        &self.shared.stats
    }

    /// Whether the worker is running now.
    pub fn running(&self) -> bool {
        self.shared
            .worker
            .lock()
            .map(|w| w.as_ref().is_some_and(Supervisor::is_running))
            .unwrap_or(false)
    }

    /// Page `page` (1-based) `width` pixels wide, with `find` outlined
    /// when it is on the page: from the cache, else rendered and kept.
    pub fn render(
        &self,
        prepared: Prepared,
        page: u32,
        width: u32,
        find: Option<&str>,
    ) -> AtlasResult<PageImage> {
        if page == 0 || width == 0 || width > MAX_WIDTH {
            return Err(AtlasError::InvalidRender);
        }
        let Prepared { hash, text, path } = prepared;
        let find = find.map(str::trim).filter(|f| !f.is_empty()).map(|f| {
            match f.char_indices().nth(MAX_FIND_CHARS) {
                Some((at, _)) => &f[..at],
                None => f,
            }
        });
        let key = cache::find_key(find);
        let image = |png: &[u8], meta: cache::Meta, from_cache: bool| PageImage {
            page_no: page,
            page_count: meta.page_count,
            width: meta.width,
            height: meta.height,
            png: BASE64_STANDARD.encode(png),
            found: !meta.rects.is_empty(),
            rects: meta
                .rects
                .iter()
                .map(|r| PageRect {
                    x: r.x,
                    y: r.y,
                    width: r.width,
                    height: r.height,
                })
                .collect(),
            text: text.clone(),
            from_cache,
        };
        if let Some((png, meta)) = cache::get(&self.previews, &hash, page, width, &key) {
            return Ok(image(&png, meta, true));
        }
        let path = path?;
        let Some(config) = &self.shared.config else {
            return Err(AtlasError::RenderFailed("unavailable"));
        };
        let rendered = {
            let mut worker = self
                .shared
                .worker
                .lock()
                .map_err(|_| AtlasError::Internal("the page renderer"))?;
            let supervisor = worker.get_or_insert_with(|| {
                let stop = Arc::clone(&self.shared.stop);
                let s = Supervisor::new(
                    config.command.clone(),
                    config.supervisor.clone(),
                    Arc::new(move || stop.load(Ordering::SeqCst)),
                    Arc::clone(&self.shared.stats),
                );
                match &config.diagnostics {
                    Some(d) => s.with_diagnostics(d.clone(), WorkerRole::Render),
                    None => s,
                }
            });
            let result = supervisor.render(&path, page, width, find);
            if let Ok(mut t) = self.shared.last_used.lock() {
                *t = Instant::now();
            }
            result
        };
        match rendered {
            Ok(Ok(r)) => {
                let meta = cache::Meta {
                    page_count: r.page_count,
                    width: r.width,
                    height: r.height,
                    rects: r.rects,
                };
                // A page that can't be kept is still shown.
                let _ = cache::put(&self.previews, &hash, page, width, &key, &r.png, &meta);
                cache::prune(&self.previews, config.cache_cap_bytes);
                Ok(image(&r.png, meta, false))
            }
            Ok(Err(why)) => Err(AtlasError::RenderFailed(reason(&why))),
            Err(_) => Err(AtlasError::RenderFailed("worker")),
        }
    }
}

impl Drop for ReaderService {
    /// The worker never outlives the library.
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.shared.signal.request_stop();
        if let Some(t) = self.idle_thread.take() {
            let _ = t.join();
        }
        if let Ok(mut worker) = self.shared.worker.lock() {
            worker.take();
        }
    }
}
