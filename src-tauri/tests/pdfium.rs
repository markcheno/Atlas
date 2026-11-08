//! Slice 1.2: PDFium reads the generated text fixtures (phase1-spec §11).
//! The generator's own tests check the PDFs' structure; this checks that the
//! library the app ships extracts their text: every page is there and every
//! file's unique marker is found where the generator put it.
//!
//! PDFium runs in this test process, which is not the app (the app only
//! runs it in the worker). `thread_safe` is off, so this is one test.

use std::path::{Path, PathBuf};
use std::process::Command;

use atlas_lib::services::extraction::locate::Locator;
use pdfium_render::prelude::Pdfium;

fn pdfs(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let kind = entry.file_type().unwrap();
        if kind.is_symlink() || entry.file_name() == "locked" {
            continue;
        }
        if kind.is_dir() {
            pdfs(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
        {
            out.push(path);
        }
    }
}

#[test]
fn pdfium_reads_every_page_and_marker_of_the_text_fixture() {
    let tmp = tempfile::tempdir().unwrap();
    let fixture = tmp.path().join("fx");
    let generator = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/generate-fixtures.mjs");
    let out = Command::new("node")
        .arg(&generator)
        .arg(&fixture)
        .args(["--files", "24", "--flat", "4", "--text-kb", "8"])
        .output()
        .expect("node runs the fixture generator");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let summary = String::from_utf8_lossy(&out.stdout);
    let summary: serde_json::Value =
        serde_json::from_str(summary.split("\nTo delete later").next().unwrap()).unwrap();

    let library = Locator::from_process().find().expect("PDFium is fetched");
    let pdfium = Pdfium::new(Pdfium::bind_to_library(library).unwrap());

    let mut files = Vec::new();
    pdfs(&fixture, &mut files);
    assert!(files.len() >= 20, "{} PDFs", files.len());
    let mut pages = 0u64;
    let mut duplicates_seen = false;
    for file in &files {
        let doc = pdfium.load_pdf_from_file(file, None).unwrap();
        let count = doc.pages().len();
        assert!(count >= 2, "{}: {count} pages", file.display());
        let first = doc.pages().get(0).unwrap().text().unwrap().all();
        assert!(
            first.contains(" marker "),
            "{}: no marker on page 1: {first:.200}",
            file.display()
        );
        for page in doc.pages().iter() {
            let text = page.text().unwrap().all();
            assert!(text.len() > 1000, "{}: a short page", file.display());
        }
        // The six duplicates share one document; count its pages once.
        let duplicate = file.components().any(|c| c.as_os_str() == "duplicates");
        if !duplicate || !duplicates_seen {
            pages += u64::try_from(count).unwrap();
        }
        duplicates_seen |= duplicate;
    }
    assert_eq!(Some(pages), summary["pdfPages"].as_u64());
}
