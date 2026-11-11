//! The extraction worker process: `atlas --extract-worker` (phase1-spec
//! §7.2): protocol hygiene, PDFium and its self-test, `hello`, then one
//! request at a time (`hash`, `extract`) until stdin closes.

use std::fs::File;
use std::io::{BufRead, Seek, SeekFrom, Write};
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use pdfium_render::prelude::Pdfium;

use super::engine::{self, Failure, Outcome, hash, sniff};
use super::locate::{Locator, Unavailable};
use super::protocol::{Frame, Request, SelfTest, decode_path};
use super::{PDFIUM_VERSION, PROTOCOL_VERSION};
use crate::domain::dto::FileType;

/// The built-in self-test PDF: two pages, "Selftest marker 0" on page 1
/// (written by `scripts/lib/fixture-text.mjs`, `selfTestPdf`).
const SELF_TEST_PDF: &[u8] = include_bytes!("selftest.pdf");
const SELF_TEST_MARKER: &str = "Selftest marker 0";

/// Debug builds only: write junk to file descriptor 1 before PDFium loads,
/// as a misbehaving library might, so tests can prove it never reaches the
/// protocol stream.
pub const TEST_STDOUT_NOISE_ENV: &str = "ATLAS_WORKER_TEST_STDOUT_NOISE";

/// Debug builds only: misbehave on chosen files, so supervisor tests can
/// drive a real worker into a hang, a memory blow-up or a crash. A list of
/// `<file-name substring>:<hang|exit|bloat>`, separated by commas.
pub const TEST_FAULT_ENV: &str = "ATLAS_WORKER_TEST_FAULT";

pub const EXIT_OK: i32 = 0;
pub const EXIT_UNAVAILABLE: i32 = 3;
pub const EXIT_NO_PROTOCOL: i32 = 4;
pub const EXIT_PANIC: i32 = 101;

static PROTOCOL: OnceLock<Mutex<File>> = OnceLock::new();

fn send(frame: &Frame) -> bool {
    let Some(out) = PROTOCOL.get() else {
        return false;
    };
    let mut out = out.lock().unwrap_or_else(|p| p.into_inner());
    out.write_all(frame.to_line().as_bytes()).is_ok() && out.flush().is_ok()
}

pub fn run() -> i32 {
    // Before anything slow: a worker whose app has died ends (P1-22).
    super::lifetime::watch_parent();
    let Ok(protocol) = take_stdout() else {
        return EXIT_NO_PROTOCOL;
    };
    let _ = PROTOCOL.set(Mutex::new(protocol));
    // The message (which may name a file) goes to stderr only; a panic in a
    // request is reported as a fixed code (see `answer`).
    std::panic::set_hook(Box::new(|info| {
        eprintln!("atlas worker: {info}");
    }));
    match std::panic::catch_unwind(serve) {
        Ok(code) => code,
        Err(_) => {
            send(&Frame::Error {
                reason: "panic".into(),
            });
            EXIT_PANIC
        }
    }
}

fn serve() -> i32 {
    if cfg!(debug_assertions) && std::env::var_os(TEST_STDOUT_NOISE_ENV).is_some() {
        write_noise();
    }
    let (pdfium, self_test) = match load_and_test() {
        Ok(t) => t,
        Err(reason) => {
            send(&Frame::Unavailable { reason });
            return EXIT_UNAVAILABLE;
        }
    };
    send(&Frame::Hello {
        protocol: PROTOCOL_VERSION,
        extractor_version: env!("CARGO_PKG_VERSION").to_owned(),
        pdfium_version: PDFIUM_VERSION.to_owned(),
        self_test,
    });

    for line in std::io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let request = match serde_json::from_str::<Request>(&line) {
            Ok(r) => r,
            Err(_) => {
                let reason = if serde_json::from_str::<serde_json::Value>(&line).is_ok() {
                    "unknown_request"
                } else {
                    "invalid_request"
                };
                if send(&Frame::Error {
                    reason: reason.into(),
                }) {
                    continue;
                }
                break;
            }
        };
        if !answer(&pdfium, request) {
            break;
        }
    }
    EXIT_OK
}

/// Answers one request. Returns false when the protocol stream is gone.
fn answer(pdfium: &Pdfium, request: Request) -> bool {
    match request {
        Request::Hash { path } => {
            let Some(path) = decode_path(&path) else {
                return send(&Frame::HashFailed {
                    reason: hash::HashError::Failed,
                });
            };
            let frame = match hash::open(&path).and_then(|mut f| hash::hash(&mut f)) {
                Ok(h) => Frame::Hashed {
                    hash: h.hash,
                    size: h.size,
                    mtime_secs: h.mtime_secs,
                },
                Err(reason) => Frame::HashFailed { reason },
            };
            send(&frame)
        }
        Request::Extract { path, kind } => {
            let Some(path) = decode_path(&path) else {
                return send(&failed(Failure::new(Outcome::Malformed, "bad_path")));
            };
            let mut alive = true;
            let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                extract(pdfium, &path, kind, &mut |piece| {
                    alive = send(&Frame::page(piece));
                    alive
                })
            }));
            if !alive {
                return false;
            }
            let frame = match result {
                Ok(Ok(summary)) => Frame::Done { summary },
                Ok(Err(f)) => failed(f),
                Err(_) => failed(Failure::new(Outcome::Malformed, "panic")),
            };
            send(&frame)
        }
        Request::Render {
            path,
            page,
            width,
            find,
        } => {
            let Some(path) = decode_path(&path) else {
                return send(&render_failed("bad_path"));
            };
            let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                render(pdfium, &path, page, width, find.as_deref())
            }));
            let frame = match result {
                Ok(Ok(p)) => {
                    use base64::prelude::{BASE64_STANDARD, Engine as _};
                    Frame::Rendered {
                        page: p.page,
                        page_count: p.page_count,
                        width: p.width,
                        height: p.height,
                        png: BASE64_STANDARD.encode(&p.png),
                        found: !p.rects.is_empty(),
                        rects: p.rects,
                    }
                }
                Ok(Err(reason)) => render_failed(reason),
                Err(_) => render_failed("malformed"),
            };
            send(&frame)
        }
    }
}

fn render_failed(reason: &str) -> Frame {
    Frame::RenderFailed {
        reason: reason.to_owned(),
    }
}

/// A page of a PDF for the page view (Slice 3.7).
fn render(
    pdfium: &Pdfium,
    path: &Path,
    page: u32,
    width: u32,
    find: Option<&str>,
) -> Result<engine::render::Page, &'static str> {
    let mut file = hash::open(path).map_err(|e| match e {
        hash::HashError::CloudOnly => "cloud_only",
        hash::HashError::Unreadable | hash::HashError::Failed => "malformed",
    })?;
    sniff::check(&mut file, FileType::Pdf).map_err(|_| "malformed")?;
    file.seek(SeekFrom::Start(0)).map_err(|_| "malformed")?;
    fault(path, None);
    engine::render::render(pdfium, file, page, width, find)
}

fn failed(f: Failure) -> Frame {
    Frame::Failed {
        outcome: f.outcome,
        reason: f.reason.to_owned(),
    }
}

fn extract(
    pdfium: &Pdfium,
    path: &Path,
    kind: FileType,
    sink: &mut engine::Sink<'_>,
) -> Result<engine::Summary, Failure> {
    let mut file = hash::open(path).map_err(|e| match e {
        hash::HashError::CloudOnly => Failure::new(Outcome::Unsupported, "cloud_only"),
        hash::HashError::Unreadable => Failure::new(Outcome::Malformed, "unreadable"),
        hash::HashError::Failed => Failure::new(Outcome::Malformed, "open_failed"),
    })?;
    sniff::check(&mut file, kind)?;
    file.seek(SeekFrom::Start(0))
        .map_err(|_| Failure::new(Outcome::Malformed, "read_failed"))?;
    fault(path, Some(sink));
    match kind {
        FileType::Pdf => engine::pdf::extract(pdfium, file, sink),
        FileType::Epub => engine::epub::extract(file, sink),
    }
}

/// See [`TEST_FAULT_ENV`]: strikes mid-document, after one page has been
/// sent (an extraction), or before answering (a render). Does nothing in
/// release builds.
fn fault(path: &Path, sink: Option<&mut engine::Sink<'_>>) {
    if !cfg!(debug_assertions) {
        return;
    }
    let Some(spec) = std::env::var_os(TEST_FAULT_ENV) else {
        return;
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let spec = spec.to_string_lossy().into_owned();
    let Some(mode) = spec.split(',').find_map(|rule| {
        let (needle, mode) = rule.split_once(':')?;
        (!needle.is_empty() && name.contains(needle)).then_some(mode)
    }) else {
        return;
    };
    if let Some(sink) = sink {
        sink(engine::Piece {
            ordinal: 0,
            page_no: Some(1),
            location: "Page 1".into(),
            text: "partial".into(),
        });
    }
    match mode {
        "hang" => loop {
            std::thread::sleep(std::time::Duration::from_secs(60));
        },
        "exit" => std::process::exit(9),
        "bloat" => {
            let mut hoard: Vec<Vec<u8>> = Vec::new();
            loop {
                hoard.push(vec![1u8; 64 * 1024 * 1024]);
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
        _ => {}
    }
}

fn load_and_test() -> Result<(Pdfium, SelfTest), Unavailable> {
    let path = Locator::from_process().find()?;
    // The loader's message goes to stderr only; the protocol carries the
    // fixed reason code.
    let bindings = Pdfium::bind_to_library(&path).map_err(|err| {
        eprintln!("atlas worker: PDFium did not load: {err}");
        Unavailable::LoadFailed
    })?;
    let pdfium = Pdfium::new(bindings);
    let test = self_test(&pdfium).ok_or(Unavailable::SelfTestFailed)?;
    Ok((pdfium, test))
}

/// Opens the built-in PDF and checks what PDFium reads from it.
pub fn self_test(pdfium: &Pdfium) -> Option<SelfTest> {
    let doc = pdfium.load_pdf_from_byte_slice(SELF_TEST_PDF, None).ok()?;
    let mut chars = 0usize;
    let mut first = String::new();
    for (n, page) in doc.pages().iter().enumerate() {
        let text = page.text().ok()?.all();
        chars += text.chars().filter(|c| !c.is_whitespace()).count();
        if n == 0 {
            first = text;
        }
    }
    let pages = u32::try_from(doc.pages().len()).unwrap_or(0);
    (pages == 2 && first.contains(SELF_TEST_MARKER) && chars > 0).then(|| SelfTest {
        pages,
        chars: u32::try_from(chars).unwrap_or(u32::MAX),
    })
}

/// Keeps the real stdout for the protocol and points file descriptor 1 (and
/// on Windows the standard output handle) at stderr, so anything a library
/// prints can't corrupt the protocol stream.
#[cfg(unix)]
fn take_stdout() -> std::io::Result<File> {
    use std::os::fd::FromRawFd;
    // SAFETY: dup and dup2 on the process's own standard descriptors; the
    // duplicate is owned by the returned File from here on.
    unsafe {
        let fd = libc::dup(1);
        if fd < 0 || libc::dup2(2, 1) < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(File::from_raw_fd(fd))
    }
}

#[cfg(windows)]
fn take_stdout() -> std::io::Result<File> {
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::Foundation::{DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE};
    use windows_sys::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    // SAFETY: duplicates this process's own standard output handle, then
    // redirects the handle and the C runtime's descriptor 1 to stderr. The
    // duplicate is owned by the returned File from here on.
    unsafe {
        let out = GetStdHandle(STD_OUTPUT_HANDLE);
        let err = GetStdHandle(STD_ERROR_HANDLE);
        let process = GetCurrentProcess();
        let mut dup: HANDLE = std::ptr::null_mut();
        if DuplicateHandle(process, out, process, &mut dup, 0, 0, DUPLICATE_SAME_ACCESS) == 0 {
            return Err(std::io::Error::last_os_error());
        }
        SetStdHandle(STD_OUTPUT_HANDLE, err);
        libc::dup2(2, 1);
        Ok(File::from_raw_handle(dup as _))
    }
}

/// See [`TEST_STDOUT_NOISE_ENV`]: both a raw write to descriptor 1 and
/// Rust's own stdout must land on stderr.
fn write_noise() {
    let noise = b"noise on fd 1\n";
    // SAFETY: a plain write of a static buffer to descriptor 1.
    unsafe {
        libc::write(1, noise.as_ptr().cast(), noise.len() as _);
    }
    println!("noise from println");
}
