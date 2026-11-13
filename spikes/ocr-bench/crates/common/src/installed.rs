//! The installed-layout check (ADR 0020's stand-in for §14's worker check):
//! an engine binary bundled into a real Atlas installer beside the app,
//! run from where the installer put it, finds PDFium, ONNX Runtime and its
//! model files by the worker's rules (`src-tauri/src/services/extraction/
//! locate.rs`) and reads one scanned page embedded in it.
//!
//! ```text
//! <engine> --self-check     # prints one JSON line; exit 0 when CER ≤ 5%
//! ```
//!
//! Where the installers put things (Tauri 2):
//! - macOS: the binary in `Contents/MacOS`, libraries in
//!   `Contents/Frameworks`, files in `Contents/Resources`;
//! - Windows: everything in the install folder;
//! - Linux: the binary in `usr/bin`, libraries and files in
//!   `usr/lib/Atlas` (a `.deb` at `/`, an AppImage under `$APPDIR`).
//!
//! The files an engine needs are under `ocr/` in the resources, with
//! `ocr/variant.txt` naming the variant the installer carries.

use std::path::{Path, PathBuf};
use std::time::Instant;

use pdfium_render::prelude::*;
use serde_json::json;

use crate::run::{Args, Engine, render};
use crate::score::score;

/// A scanned page of our own (Project Gutenberg text, typeset and run
/// through a print-and-scan look), and its text.
const PAGE: &[u8] = include_bytes!("../../../corpus/check/page.pdf");
const TRUTH: &str = include_str!("../../../corpus/check/page.txt");
const PRODUCT: &str = "Atlas";
const MAX_CER: f64 = 0.05;

/// The folders an installer may have put Atlas's libraries and files in,
/// most likely first.
fn library_dirs(exe: &Path) -> Vec<PathBuf> {
    let dir = exe.parent().unwrap_or(Path::new(""));
    if cfg!(target_os = "macos") {
        vec![dir.join("../Frameworks")]
    } else if cfg!(target_os = "windows") {
        vec![dir.to_path_buf()]
    } else {
        let mut out = vec![dir.join("../lib").join(PRODUCT)];
        if let Some(appdir) = std::env::var_os("APPDIR") {
            out.push(Path::new(&appdir).join("usr/lib").join(PRODUCT));
        }
        out.push(Path::new("/usr/lib").join(PRODUCT));
        out
    }
}

fn resource_dirs(exe: &Path) -> Vec<PathBuf> {
    if cfg!(target_os = "macos") {
        let dir = exe.parent().unwrap_or(Path::new(""));
        vec![dir.join("../Resources")]
    } else {
        library_dirs(exe)
    }
}

fn first(dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
    dirs.iter().map(|d| d.join(name)).find(|p| p.exists())
}

/// A library whose file name starts with `stem` (versioned names:
/// `libonnxruntime.1.28.0.dylib`, `libonnxruntime.so.1.28.0`).
fn first_library(dirs: &[PathBuf], stem: &str) -> Option<PathBuf> {
    dirs.iter().find_map(|d| {
        std::fs::read_dir(d)
            .ok()?
            .filter_map(Result::ok)
            .find_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.starts_with(stem).then(|| e.path())
            })
    })
}

/// Runs the check; returns the process's exit code.
pub fn self_check<E: Engine>(name: &str, init: impl FnOnce(&Args) -> Result<E, String>) -> i32 {
    let started = Instant::now();
    let fail = |stage: &str, why: String| {
        println!(
            "{}",
            json!({ "check": name, "ok": false, "stage": stage, "error": why })
        );
        1
    };
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => return fail("exe", e.to_string()),
    };
    let libs = library_dirs(&exe);
    let resources = resource_dirs(&exe);
    let pdfium_file = if cfg!(target_os = "macos") {
        "libpdfium.dylib"
    } else if cfg!(target_os = "windows") {
        "pdfium.dll"
    } else {
        "libpdfium.so"
    };
    let Some(pdfium_path) = first(&libs, pdfium_file) else {
        return fail("pdfium", format!("not found in {libs:?}"));
    };
    let ocr = first(&resources, "ocr");
    let variant = ocr
        .as_ref()
        .and_then(|d| std::fs::read_to_string(d.join("variant.txt")).ok())
        .map(|v| v.trim().to_owned())
        .unwrap_or_else(|| "none".into());
    // The engines read these; set them from the installed places.
    // SAFETY: the process has one thread at this point.
    unsafe {
        if let Some(dir) = &ocr {
            std::env::set_var("ATLAS_TESSDATA", dir.join("tessdata"));
            std::env::set_var("ATLAS_PPOCR_MODELS", dir.join("ppocr"));
        }
        if let Some(ort) =
            first_library(&libs, "libonnxruntime.").or_else(|| first(&libs, "onnxruntime.dll"))
        {
            std::env::set_var("ORT_DYLIB_PATH", ort);
        }
    }
    let pdfium = match Pdfium::bind_to_library(&pdfium_path) {
        Ok(bindings) => Pdfium::new(bindings),
        Err(e) => return fail("pdfium", format!("{e:?}")),
    };
    let page_file =
        std::env::temp_dir().join(format!("atlas-ocr-check-{}.pdf", std::process::id()));
    if let Err(e) = std::fs::write(&page_file, PAGE) {
        return fail("page", e.to_string());
    }
    let args = Args {
        dpi: 300,
        variant: variant.clone(),
        jobs: PathBuf::new(),
        dump: None,
    };
    let mut engine = match init(&args) {
        Ok(engine) => engine,
        Err(why) => return fail("init", why),
    };
    let init_ms = started.elapsed().as_secs_f64() * 1e3;
    let page = render(&pdfium, &page_file, 1, 300);
    let _ = std::fs::remove_file(&page_file);
    let page = match page {
        Ok(page) => page,
        Err(why) => return fail("render", why),
    };
    let t = Instant::now();
    let text = match engine.recognise(&page, 300) {
        Ok(text) => text,
        Err(why) => return fail("recognise", why),
    };
    let s = score(&text, TRUTH);
    // The baseline stand-in reads nothing: rendering the page is its check.
    let ok = name == "baseline" || s.cer <= MAX_CER;
    println!(
        "{}",
        json!({
            "check": name,
            "variant": variant,
            "ok": ok,
            "cer": s.cer,
            "wer": s.wer,
            "initMs": init_ms,
            "ocrMs": t.elapsed().as_secs_f64() * 1e3,
            "pdfium": pdfium_path,
            "onnxRuntime": std::env::var_os("ORT_DYLIB_PATH").map(PathBuf::from),
            "files": ocr,
            "engineInfo": engine.info(),
        })
    );
    if ok { 0 } else { 1 }
}
