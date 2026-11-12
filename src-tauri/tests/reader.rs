//! Slice 3.7: the page view (phase3-spec §7.5, §12), against the real
//! worker, in a process of its own.
//!
//! - P3-19: a page of a fixture PDF with its marker found; an excerpt that
//!   isn't on the page; a scanned page; the last page plus one.
//! - P3-20: malformed PDFs; a page over the limits; a worker that hangs or
//!   grows; the idle kill; an extraction job running at the same time is
//!   undisturbed; this process never loads PDFium.
//! - P3-21: the page cache: a second request; a moved file; the cap; a
//!   file only in the cloud; Clean caches.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use atlas_lib::error::{AtlasError, ErrorCode};
use atlas_lib::services::extraction::db as extraction_db;
use atlas_lib::services::extraction::supervisor::{SupervisorConfig, WorkerCommand, WorkerStats};
use atlas_lib::services::extraction::worker_main::TEST_FAULT_ENV;
use atlas_lib::services::reader::{self, PREVIEWS_DIR, Prepared, ReaderConfig, ReaderService};
use atlas_lib::services::workspace::{OpenRoot, Workspace};
use atlas_lib::services::{inventory, sources, storage};

mod common;
use common::{Env, ExtractionHarness, fast_extraction, wait_until, worker};

const LONG: Duration = Duration::from_secs(180);

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../spikes/extraction-bench/corpus")
}

fn generate(dir: &Path, files: u32) {
    let out = Command::new("node")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/generate-fixtures.mjs"))
        .arg(dir)
        .args(["--files", &files.to_string(), "--text-kb", "4"])
        .output()
        .expect("node runs the fixture generator");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn find(dir: &Path, name: &str) -> Option<PathBuf> {
    for e in fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Some(found) = find(&p, name) {
                return Some(found);
            }
        } else if p.file_name().is_some_and(|n| n == name) {
            return Some(p);
        }
    }
    None
}

/// The page view's worker: the `atlas` binary Cargo built, with the test
/// faults on for files named `crash-hang` and `crash-bloat`.
fn reader_config() -> ReaderConfig {
    let mut config = ReaderConfig::new(
        WorkerCommand::new(env!("CARGO_BIN_EXE_atlas"))
            .env(TEST_FAULT_ENV, "crash-hang:hang,crash-bloat:bloat"),
    );
    config.supervisor = SupervisorConfig {
        inactivity: Duration::from_secs(5),
        rss_cap_bytes: 600 * 1024 * 1024,
        poll: Duration::from_millis(50),
        ..SupervisorConfig::default()
    };
    config
}

struct Lib {
    ws: Workspace,
    env: Env,
    _h: ExtractionHarness,
    books: PathBuf,
    books_id: String,
}

impl Lib {
    /// A read library of generated two-page PDFs.
    fn new() -> Self {
        let env = Env::new();
        let books = env.base.join("Books");
        generate(&books, 4);
        let h = ExtractionHarness::new(fast_extraction(), worker());
        let mut ws = env
            .workspace_with_extraction(&h)
            .with_reader(reader_config());
        ws.choose(&env.base.join("AtlasData")).unwrap();
        let books_id = sources::add(ws.require_open().unwrap(), &books).unwrap().id;
        let lib = Self {
            ws,
            env,
            _h: h,
            books,
            books_id,
        };
        lib.rescan(&lib.books_id.clone());
        lib
    }

    fn root(&self) -> &OpenRoot {
        self.ws.require_open().unwrap()
    }

    fn pages(&self) -> &Arc<ReaderService> {
        self.root().pages()
    }

    fn stats(&self) -> &Arc<WorkerStats> {
        self.pages().stats()
    }

    fn settled(&self) -> bool {
        let inv = inventory::control::overview(self.root()).unwrap();
        let ov = self
            .root()
            .reader()
            .read(|c| extraction_db::overview(c, None))
            .unwrap()
            .unwrap();
        inv.active.is_none() && ov.active.is_none() && ov.queued_jobs == 0 && ov.counts.pending == 0
    }

    fn rescan(&self, id: &str) {
        inventory::control::enqueue(self.root(), id).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert!(wait_until(LONG, || self.settled()), "never settled");
    }

    fn file(&self, name: &str) -> String {
        let name = name.to_owned();
        self.root()
            .reader()
            .read(move |c| {
                c.query_row(
                    "SELECT id FROM file_instances WHERE display_name = ?1 AND availability = 'available'",
                    [name],
                    |r| r.get(0),
                )
            })
            .unwrap()
            .unwrap()
    }

    fn render(
        &self,
        file: &str,
        page: u32,
        width: u32,
        find: Option<&str>,
    ) -> Result<atlas_lib::domain::dto::PageImage, AtlasError> {
        let prepared = reader::prepare(self.root(), file, page, false)?;
        self.pages().render(prepared, page, width, find)
    }

    /// A file of the corpus rendered directly, as if it were cataloged.
    fn render_file(&self, path: &Path, page: u32, width: u32) -> Result<(), AtlasError> {
        let prepared = Prepared {
            hash: blake3::hash(path.to_string_lossy().as_bytes())
                .to_hex()
                .to_string(),
            text: String::new(),
            path: Ok(path.to_path_buf()),
        };
        self.pages().render(prepared, page, width, None).map(drop)
    }
}

fn count(c: &std::sync::atomic::AtomicU64) -> u64 {
    WorkerStats::get(c)
}

#[test]
fn p3_19_a_page_with_its_passage_outlined() {
    let lib = Lib::new();
    let file = lib.file("book 000001.pdf");

    // Page 1 with its marker: an image of the asked width, outlined near
    // where the generator writes the first line (72 pt in, near the top).
    let page = lib
        .render(&file, 1, 800, Some("Engineering marker 1"))
        .unwrap();
    assert_eq!((page.page_no, page.page_count, page.width), (1, 2, 800));
    assert!(page.height > 1000, "US Letter at 800 px: {}", page.height);
    use base64::prelude::{BASE64_STANDARD, Engine as _};
    let png = BASE64_STANDARD.decode(&page.png).unwrap();
    assert_eq!(&png[..4], b"\x89PNG");
    assert!(page.found && !page.rects.is_empty());
    let r = page.rects[0];
    assert!((60..250).contains(&r.x), "{r:?}");
    assert!(r.y < page.height / 5, "{r:?}");
    assert!(r.x + r.width <= page.width && r.y + r.height <= page.height);
    assert!(
        page.text.contains("Engineering marker 1"),
        "the page's text"
    );
    assert!(!page.from_cache);

    // Text that isn't on the page: the image without an outline.
    let page2 = lib
        .render(&file, 2, 800, Some("Engineering marker 1"))
        .unwrap();
    assert!(!page2.found && page2.rects.is_empty());
    let nowhere = lib
        .render(&file, 1, 800, Some("nothing like this is in any fixture"))
        .unwrap();
    assert!(!nowhere.found);

    // A scanned page: an image, nothing found.
    let scan = corpus().join("pdf/scanned/synthetic-scan.pdf");
    let prepared = Prepared {
        hash: "scan".into(),
        text: String::new(),
        path: Ok(scan),
    };
    let scanned = lib
        .pages()
        .render(prepared, 1, 600, Some("anything at all"))
        .unwrap();
    assert!(scanned.width == 600 && !scanned.found);

    // Slice 3.9 (the owner's DMG smoke): a search snippet, with its
    // ellipses, is outlined where its words are.
    let snippet = lib
        .render(&file, 1, 800, Some("…Engineering marker 1…"))
        .unwrap();
    assert!(snippet.found, "a snippet's ellipses don't hide it");
    // A real paper's passage that crosses a word hyphenated at a line end,
    // as PDFium writes it (U+0002 and a line break): its start can't be
    // matched, so its pieces are.
    let paper = corpus().join("pdf/real/arxiv-2609.04872.pdf");
    let cited = lib
        .pages()
        .render(
            Prepared {
                hash: "paper".into(),
                text: String::new(),
                path: Ok(paper),
            },
            2,
            800,
            Some(
                "\nyear pseudo holdouts, alternative donor pools, alterna\u{2}tive preperiods, \
                 and leave one donor out refits. The\r\nconceptual",
            ),
        )
        .unwrap();
    assert!(
        cited.found && cited.rects.len() > 1,
        "outlined where its words are: {:?}",
        cited.rects
    );

    // The last page plus one: a clear refusal, and the worker lives on.
    assert!(matches!(
        lib.render(&file, 3, 800, None),
        Err(AtlasError::RenderFailed("page_out_of_range"))
    ));
    assert_eq!(count(&lib.stats().spawns), 1, "one worker served them all");
    assert_eq!(count(&lib.stats().kills), 0);
}

/// Whether PDFium is loaded into this process, where the platform can say.
fn pdfium_loaded_here() -> Option<bool> {
    #[cfg(target_os = "linux")]
    {
        let maps = fs::read_to_string("/proc/self/maps").ok()?;
        return Some(maps.to_lowercase().contains("pdfium"));
    }
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn _dyld_image_count() -> u32;
            fn _dyld_get_image_name(index: u32) -> *const std::ffi::c_char;
        }
        // SAFETY: dyld's image list is read-only and stable for the call.
        unsafe {
            for i in 0.._dyld_image_count() {
                let name = _dyld_get_image_name(i);
                if !name.is_null()
                    && std::ffi::CStr::from_ptr(name)
                        .to_string_lossy()
                        .to_lowercase()
                        .contains("pdfium")
                {
                    return Some(true);
                }
            }
        }
        return Some(false);
    }
    #[allow(unreachable_code)]
    None
}

#[test]
fn p3_20_bad_files_and_stuck_workers_never_hold_the_page_view_up() {
    let lib = Lib::new();
    let file = lib.file("book 000001.pdf");
    let good = |lib: &Lib| lib.render(&file, 1, 400, None).unwrap();
    good(&lib);

    // Malformed PDFs: refused, and the next page still renders.
    let malformed = corpus().join("malformed");
    for name in [
        "bad-length.pdf",
        "broken-xref.pdf",
        "deep-nesting.pdf",
        "not-a-pdf.pdf",
        "truncated.pdf",
        "user-password.pdf",
        "zero-bytes.pdf",
    ] {
        let result = lib.render_file(&malformed.join(name), 1, 400);
        assert!(
            matches!(result, Err(AtlasError::RenderFailed(_)) | Ok(())),
            "{name}: {result:?}"
        );
        good(&lib);
    }

    // Over the limits: refused before any worker is asked.
    let before = count(&lib.stats().renders);
    assert!(matches!(
        lib.render(&file, 1, 2_001, None),
        Err(AtlasError::InvalidRender)
    ));
    assert!(matches!(
        lib.render(&file, 0, 400, None),
        Err(AtlasError::InvalidRender)
    ));
    assert_eq!(count(&lib.stats().renders), before);

    // A worker that hangs, and one that grows: killed, then a new one.
    let book = corpus().join("pdf/generated/book.pdf");
    let dir = lib.env.base.join("faults");
    fs::create_dir_all(&dir).unwrap();
    fs::copy(&book, dir.join("crash-hang.pdf")).unwrap();
    fs::copy(&book, dir.join("crash-bloat.pdf")).unwrap();

    // An extraction job runs meanwhile, on a folder of its own.
    let more = lib.env.base.join("More");
    generate(&more, 12);
    let more_id = sources::add(lib.root(), &more).unwrap().id;
    inventory::control::enqueue(lib.root(), &more_id).unwrap();

    let kills = count(&lib.stats().kills);
    let started = Instant::now();
    assert!(matches!(
        lib.render_file(&dir.join("crash-hang.pdf"), 1, 400),
        Err(AtlasError::RenderFailed("worker"))
    ));
    assert!(started.elapsed() < Duration::from_secs(30));
    assert_eq!(
        count(&lib.stats().kills),
        kills + 1,
        "the hung worker was killed"
    );
    good(&lib);
    assert!(matches!(
        lib.render_file(&dir.join("crash-bloat.pdf"), 1, 400),
        Err(AtlasError::RenderFailed("worker"))
    ));
    assert_eq!(
        count(&lib.stats().kills),
        kills + 2,
        "the growing worker was killed"
    );
    good(&lib);

    // The extraction job finished, every new file read.
    std::thread::sleep(Duration::from_millis(200));
    assert!(wait_until(LONG, || lib.settled()));
    let unread: i64 = lib
        .root()
        .reader()
        .read(move |c| {
            c.query_row(
                "SELECT count(*) FROM file_instances
                 WHERE source_root_id = ?1 AND file_type = 'pdf' AND extract_state != 'linked'",
                [more_id],
                |r| r.get(0),
            )
        })
        .unwrap()
        .unwrap();
    assert_eq!(unread, 0, "the extraction job was undisturbed");

    // This process never loaded PDFium; the workers did.
    if let Some(loaded) = pdfium_loaded_here() {
        assert!(!loaded, "PDFium is loaded in the main process");
    }

    // Idle: the worker goes after the configured time.
    let mut config = reader_config();
    config.idle = Duration::from_millis(300);
    let idle = ReaderService::new(&lib.env.base.join("idle"), Some(config));
    let prepared = reader::prepare(lib.root(), &file, 1, false).unwrap();
    idle.render(prepared, 1, 300, None).unwrap();
    assert!(idle.running());
    assert!(
        wait_until(Duration::from_secs(10), || !idle.running()),
        "killed when idle"
    );
}

#[test]
fn p3_21_the_page_cache_by_content() {
    let mut lib = Lib::new();
    let file = lib.file("book 000002.pdf");
    let first = lib
        .render(&file, 1, 640, Some("Mathematics marker 2"))
        .unwrap();
    assert!(!first.from_cache && first.found);

    // A second request: from the cache, no new render.
    let renders = count(&lib.stats().renders);
    let again = lib
        .render(&file, 1, 640, Some("Mathematics marker 2"))
        .unwrap();
    assert!(again.from_cache);
    assert_eq!(
        (again.png.as_str(), &again.rects),
        (first.png.as_str(), &first.rects)
    );
    assert_eq!(count(&lib.stats().renders), renders);

    // A moved file: the same content, so the same pages.
    let path = find(&lib.books, "book 000002.pdf").unwrap();
    fs::create_dir_all(lib.books.join("Moved")).unwrap();
    fs::rename(&path, lib.books.join("Moved/book 000002.pdf")).unwrap();
    lib.rescan(&lib.books_id.clone());
    let moved = lib.file("book 000002.pdf");
    assert_ne!(moved, file);
    let after_move = lib
        .render(&moved, 1, 640, Some("Mathematics marker 2"))
        .unwrap();
    assert!(after_move.from_cache, "reused by content");

    // A file only in the cloud: refused without agreement, unless the page
    // is already in the cache.
    let hash = reader::prepare(lib.root(), &moved, 1, false).unwrap().hash;
    let cloud = || Prepared {
        hash: hash.clone(),
        text: String::new(),
        path: Err(AtlasError::FileCloudOnly),
    };
    let refused = lib.pages().render(cloud(), 2, 640, None).unwrap_err();
    assert_eq!(refused.code(), ErrorCode::FileCloudOnly);
    assert!(
        lib.pages()
            .render(cloud(), 1, 640, Some("Mathematics marker 2"))
            .unwrap()
            .from_cache
    );

    // The cap: the least recently used pages go first.
    let previews = lib.env.base.join("AtlasData").join(PREVIEWS_DIR);
    let one_page = fs::read_dir(previews.join(&hash))
        .unwrap()
        .flatten()
        .map(|e| e.metadata().unwrap().len())
        .sum::<u64>();
    let mut config = reader_config();
    config.cache_cap_bytes = one_page * 2 + one_page / 2;
    let capped = ReaderService::new(&lib.env.base.join("AtlasData"), Some(config));
    let prepared = |page: u32| reader::prepare(lib.root(), &moved, page, false).unwrap();
    for width in [641, 642, 643] {
        std::thread::sleep(Duration::from_millis(20));
        capped.render(prepared(1), 1, width, None).unwrap();
    }
    let kept: Vec<String> = fs::read_dir(previews.join(&hash))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".png"))
        .collect();
    assert!(
        kept.iter().any(|n| n.starts_with("1-643-")),
        "the newest stays: {kept:?}"
    );
    assert!(
        !kept.iter().any(|n| n.starts_with("1-640-")),
        "the oldest went: {kept:?}"
    );

    // Clean caches empties it.
    storage::clean_caches(lib.root().path()).unwrap();
    assert!(
        !lib.render(&moved, 1, 640, Some("Mathematics marker 2"))
            .unwrap()
            .from_cache
    );
    drop(capped);
    lib.ws.shutdown();
}
