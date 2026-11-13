//! The engines' contract and the page render they share.
//!
//! Every engine binary takes `--dpi N --variant V <jobs.jsonl>`, where each
//! job line is `{"id", "pdf", "page"}` (pages from 1), and writes JSON lines
//! to the file named by `XBENCH_OUT` (else stdout):
//! - `{"ready": true, "engine", "variant", "initMs"}` once the engine is
//!   loaded;
//! - one `{"id", "outcome", "text", "renderMs", "ocrMs", "cpuMs", "w", "h"}`
//!   per page, `outcome` being `ok`, `failed` or `panic`;
//! - `{"done": true, …, "peakRssBytes", "rssAfterInitBytes", "engineInfo"}`.
//!
//! Each binary renders its pages itself, with PDFium, as the worker would:
//! greyscale, annotations off, at the DPI asked for (phase4-spec §3).
//! The grey pixels come from a BGRA render (see [`render`]).
//! A crash or a hang is recorded by the runner, which watches the process.

use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::time::Instant;

use pdfium_render::prelude::*;
use serde_json::{Value, json};

/// A page image: one byte a pixel, rows packed with no padding.
pub struct Grey {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// What an engine is to the harness.
pub trait Engine {
    /// The page's text, lines separated by `\n`, in reading order.
    fn recognise(&mut self, page: &Grey, dpi: u32) -> Result<String, String>;
    /// Versions and model hashes, for the `done` line.
    fn info(&self) -> Value;
}

/// Parsed from the command line.
pub struct Args {
    pub dpi: u32,
    pub variant: String,
    pub jobs: PathBuf,
    /// Where to write each rendered page as a PNG, when asked (`--dump`).
    pub dump: Option<PathBuf>,
}

pub fn args() -> Args {
    let mut dpi = 300;
    let mut variant = String::new();
    let mut jobs = None;
    let mut dump = None;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--dpi" => dpi = it.next().and_then(|v| v.parse().ok()).expect("--dpi N"),
            "--variant" => variant = it.next().expect("--variant V"),
            "--dump" => dump = it.next().map(PathBuf::from),
            _ => jobs = Some(PathBuf::from(a)),
        }
    }
    Args {
        dpi,
        variant,
        jobs: jobs.expect("a jobs file"),
        dump,
    }
}

#[derive(serde::Deserialize)]
struct Job {
    id: String,
    pdf: PathBuf,
    page: u16,
}

fn pdfium() -> Pdfium {
    let lib = std::env::var("ATLAS_PDFIUM").expect("set ATLAS_PDFIUM to the PDFium library");
    Pdfium::new(Pdfium::bind_to_library(lib).expect("load PDFium"))
}

/// Page `page` (from 1) of `pdf`, greyscale at `dpi`, annotations off.
pub fn render(pdfium: &Pdfium, pdf: &Path, page: u16, dpi: u32) -> Result<Grey, String> {
    let doc = pdfium
        .load_pdf_from_file(pdf, None)
        .map_err(|e| format!("open: {e:?}"))?;
    let p = doc
        .pages()
        .get(i32::from(page.checked_sub(1).ok_or("pages count from 1")?))
        .map_err(|e| format!("page: {e:?}"))?;
    // PDFium's own greyscale bitmap (`PdfBitmapFormat::Gray`) leaves image
    // objects undrawn, which is all a scan is; so the page is rendered as
    // BGRA with PDFium's greyscale flag and reduced to one byte a pixel
    // here (ITU-R BT.601 weights).
    let config = PdfRenderConfig::new()
        .scale_page_by_factor(dpi as f32 / 72.0)
        .set_format(PdfBitmapFormat::BGRA)
        .use_grayscale_rendering(true)
        .render_annotations(false)
        .render_form_data(false);
    let bitmap = p
        .render_with_config(&config)
        .map_err(|e| format!("render: {e:?}"))?;
    let (width, height) = (bitmap.width() as u32, bitmap.height() as u32);
    let raw = bitmap.as_raw_bytes();
    // PDFium may pad each row; keep the pixels only.
    let stride = raw.len() / height.max(1) as usize;
    if stride < width as usize * 4 {
        return Err(format!(
            "render: a row of {stride} bytes for {width} pixels"
        ));
    }
    let mut pixels = Vec::with_capacity((width * height) as usize);
    for row in raw.chunks_exact(stride).take(height as usize) {
        for bgra in row[..width as usize * 4].chunks_exact(4) {
            let (b, g, r) = (u32::from(bgra[0]), u32::from(bgra[1]), u32::from(bgra[2]));
            pixels.push(((299 * r + 587 * g + 114 * b + 500) / 1000) as u8);
        }
    }
    Ok(Grey {
        width,
        height,
        pixels,
    })
}

/// A page as a greyscale PNG.
pub fn png(page: &Grey) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, page.width, page.height);
        encoder.set_color(png::ColorType::Grayscale);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().expect("PNG header");
        writer.write_image_data(&page.pixels).expect("PNG data");
    }
    out
}

/// Runs the jobs through `engine`, made by `init` (which is timed).
pub fn main_with<E: Engine>(name: &str, init: impl FnOnce(&Args) -> Result<E, String>) {
    if std::env::args().nth(1).as_deref() == Some("--self-check") {
        std::process::exit(crate::installed::self_check(name, init));
    }
    if std::env::args().nth(1).as_deref() == Some("--pages") {
        list_pages(std::env::args().skip(2));
        return;
    }
    std::panic::set_hook(Box::new(|_| {}));
    let args = args();
    let mut out: Box<dyn Write> = match std::env::var_os("XBENCH_OUT") {
        Some(p) => Box::new(std::fs::File::create(p).expect("XBENCH_OUT")),
        None => Box::new(std::io::stdout()),
    };
    let jobs: Vec<Job> = std::fs::read_to_string(&args.jobs)
        .expect("read the jobs")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("a job line"))
        .collect();
    let pdfium = pdfium();
    let started = Instant::now();
    let mut engine = match init(&args) {
        Ok(engine) => engine,
        Err(why) => {
            let line =
                json!({ "ready": false, "engine": name, "variant": args.variant, "error": why });
            writeln!(out, "{line}").expect("write");
            std::process::exit(2);
        }
    };
    let init_ms = started.elapsed().as_secs_f64() * 1e3;
    let rss_after_init = crate::sys::peak_rss_bytes();
    let ready = json!({ "ready": true, "engine": name, "variant": args.variant, "dpi": args.dpi, "initMs": init_ms });
    writeln!(out, "{ready}").expect("write");
    out.flush().expect("flush");
    for job in &jobs {
        let t = Instant::now();
        let page = match render(&pdfium, &job.pdf, job.page, args.dpi) {
            Ok(page) => page,
            Err(why) => {
                let line = json!({ "id": job.id, "outcome": "failed", "error": why });
                writeln!(out, "{line}").expect("write");
                continue;
            }
        };
        let render_ms = t.elapsed().as_secs_f64() * 1e3;
        // The reference row: the PDF's own text, not OCR.
        if args.variant == "text-layer" {
            let line = match page_text(&pdfium, &job.pdf, job.page) {
                Ok(text) => json!({ "id": job.id, "outcome": "ok", "text": text }),
                Err(why) => json!({ "id": job.id, "outcome": "failed", "error": why }),
            };
            writeln!(out, "{line}").expect("write");
            continue;
        }
        if let Some(dir) = &args.dump {
            std::fs::create_dir_all(dir).expect("the dump folder");
            std::fs::write(dir.join(format!("{}.png", job.id)), png(&page)).expect("dump");
        }
        let cpu_before = crate::sys::cpu_ms();
        let t = Instant::now();
        let result = catch_unwind(AssertUnwindSafe(|| engine.recognise(&page, args.dpi)));
        let ocr_ms = t.elapsed().as_secs_f64() * 1e3;
        let cpu_ms = crate::sys::cpu_ms() - cpu_before;
        let (w, h) = (page.width, page.height);
        let line = match result {
            Ok(Ok(text)) => json!({ "id": job.id, "outcome": "ok", "text": text,
                "renderMs": render_ms, "ocrMs": ocr_ms, "cpuMs": cpu_ms, "w": w, "h": h }),
            Ok(Err(why)) => json!({ "id": job.id, "outcome": "failed", "error": why,
                "renderMs": render_ms, "ocrMs": ocr_ms, "cpuMs": cpu_ms, "w": w, "h": h }),
            Err(p) => {
                let why = p
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "panic".into());
                json!({ "id": job.id, "outcome": "panic", "error": why, "ocrMs": ocr_ms })
            }
        };
        writeln!(out, "{line}").expect("write");
        out.flush().expect("flush");
    }
    let done = json!({
        "done": true,
        "engine": name,
        "variant": args.variant,
        "peakRssBytes": crate::sys::peak_rss_bytes(),
        "rssAfterInitBytes": rss_after_init,
        "engineInfo": engine.info(),
        "appLibraries": xbench::app_libraries(),
    });
    writeln!(out, "{done}").expect("write");
}

/// The SHA-256 of a model file, for the `done` line.
pub fn file_sha256(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    match std::fs::read(path) {
        Ok(bytes) => format!("{:x}", Sha256::digest(bytes)),
        Err(e) => format!("unreadable: {e}"),
    }
}

/// The text layer of page `page` (from 1) of `pdf`.
pub fn page_text(pdfium: &Pdfium, pdf: &Path, page: u16) -> Result<String, String> {
    let doc = pdfium
        .load_pdf_from_file(pdf, None)
        .map_err(|e| format!("open: {e:?}"))?;
    let p = doc
        .pages()
        .get(i32::from(page.checked_sub(1).ok_or("pages count from 1")?))
        .map_err(|e| format!("page: {e:?}"))?;
    Ok(p.text().map_err(|e| format!("text: {e:?}"))?.all())
}

/// `--pages <pdf>…`: one JSON line per file with each page's count of
/// visible characters in its text layer (Atlas's rule: under 20 is an
/// image-only page), or why it couldn't be opened.
fn list_pages(files: impl Iterator<Item = String>) {
    let pdfium = pdfium();
    for file in files {
        let line = match pdfium.load_pdf_from_file(&file, None) {
            Ok(doc) => {
                let chars: Vec<usize> = doc
                    .pages()
                    .iter()
                    .map(|p| {
                        p.text()
                            .map(|t| t.all().chars().filter(|c| !c.is_whitespace()).count())
                            .unwrap_or(0)
                    })
                    .collect();
                json!({ "pdf": file, "visibleChars": chars })
            }
            Err(e) => json!({ "pdf": file, "error": format!("{e:?}") }),
        };
        println!("{line}");
    }
}
