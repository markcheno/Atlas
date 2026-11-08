//! PDFium through pdfium-render 0.9.5 (dynamically loaded shared library,
//! as an app would ship it). Text per page with `PdfPageText::all()`.
//!
//! The library path comes from `ATLAS_PDFIUM` (the .dylib/.so/.dll).

use std::path::Path;

use pdfium_render::prelude::*;
use serde_json::json;
use xbench::{Extracted, Failure};

thread_local! {
    static PDFIUM: Pdfium = {
        let lib = std::env::var("ATLAS_PDFIUM").expect("set ATLAS_PDFIUM to the PDFium library");
        Pdfium::new(Pdfium::bind_to_library(lib).expect("load PDFium"))
    };
}

fn classify(e: PdfiumError) -> Failure {
    match e {
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError) => Failure::Encrypted(format!("{e:?}")),
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::SecurityError) => Failure::Encrypted(format!("{e:?}")),
        other => Failure::Malformed(format!("{other:?}")),
    }
}

fn extract(path: &Path) -> Result<Extracted, Failure> {
    PDFIUM.with(|pdfium| {
        let doc = pdfium.load_pdf_from_file(path, None).map_err(classify)?;
        let mut pages = Vec::new();
        for page in doc.pages().iter() {
            pages.push(page.text().map_err(classify)?.all());
        }
        Ok(Extracted { meta: json!({ "pageCount": pages.len() }), pages })
    })
}

fn main() {
    xbench::main_with("pdfium", "pdfium-render 0.9.5, PDFium chromium/8125", extract);
}
