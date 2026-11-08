//! The extraction worker process: `atlas --extract-worker` (phase1-spec
//! §7.2). Slice 1.2: protocol hygiene, PDFium, the self-test, `hello`, and
//! a request loop that answers every request with `unknown_request` until
//! Slice 1.3 adds them.

use std::fs::File;
use std::io::{BufRead, Write};
use std::sync::{Mutex, OnceLock};

use pdfium_render::prelude::Pdfium;

use super::locate::{Locator, Unavailable};
use super::protocol::{Frame, SelfTest};
use super::{PDFIUM_VERSION, PROTOCOL_VERSION};

/// The built-in self-test PDF: two pages, "Selftest marker 0" on page 1
/// (written by `scripts/lib/fixture-text.mjs`, `selfTestPdf`).
const SELF_TEST_PDF: &[u8] = include_bytes!("selftest.pdf");
const SELF_TEST_MARKER: &str = "Selftest marker 0";

/// Debug builds only: write junk to file descriptor 1 before PDFium loads,
/// as a misbehaving library might, so tests can prove it never reaches the
/// protocol stream.
pub const TEST_STDOUT_NOISE_ENV: &str = "ATLAS_WORKER_TEST_STDOUT_NOISE";

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
    let Ok(protocol) = take_stdout() else {
        return EXIT_NO_PROTOCOL;
    };
    let _ = PROTOCOL.set(Mutex::new(protocol));
    // The protocol gets a fixed code; the message (which may name a file)
    // goes to stderr only.
    std::panic::set_hook(Box::new(|info| {
        eprintln!("atlas worker: {info}");
        send(&Frame::Error {
            reason: "panic".into(),
        });
    }));
    match std::panic::catch_unwind(serve) {
        Ok(code) => code,
        Err(_) => EXIT_PANIC,
    }
}

fn serve() -> i32 {
    if cfg!(debug_assertions) && std::env::var_os(TEST_STDOUT_NOISE_ENV).is_some() {
        write_noise();
    }
    let self_test = match load_and_test() {
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
        let reason = match serde_json::from_str::<serde_json::Value>(&line) {
            Ok(_) => "unknown_request",
            Err(_) => "invalid_request",
        };
        if !send(&Frame::Error {
            reason: reason.into(),
        }) {
            break;
        }
    }
    EXIT_OK
}

fn load_and_test() -> Result<SelfTest, Unavailable> {
    let path = Locator::from_process().find()?;
    // The loader's message goes to stderr only; the protocol carries the
    // fixed reason code.
    let bindings = Pdfium::bind_to_library(&path).map_err(|err| {
        eprintln!("atlas worker: PDFium did not load: {err}");
        Unavailable::LoadFailed
    })?;
    let pdfium = Pdfium::new(bindings);
    self_test(&pdfium).ok_or(Unavailable::SelfTestFailed)
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
