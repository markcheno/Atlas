//! PDF text with PDFium: one passage per page (phase1-spec §7.2), ported
//! from the Slice 0.9 spike's `pdf-pdfium` candidate.

use std::fs::File;

use pdfium_render::prelude::{
    PdfDocumentMetadataTagType, Pdfium, PdfiumError, PdfiumInternalError,
};

use super::garble::Readability;
use super::{Budget, Failure, MAX_PASSAGE_CHARS, Outcome, Piece, Sink, Summary, truncate_chars};

fn classify(err: PdfiumError) -> Failure {
    match err {
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::PasswordError) => {
            Failure::new(Outcome::Encrypted, "password")
        }
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::SecurityError) => {
            Failure::new(Outcome::Encrypted, "security_handler")
        }
        PdfiumError::PdfiumLibraryInternalError(PdfiumInternalError::FormatError) => {
            Failure::new(Outcome::Malformed, "pdf_format")
        }
        _ => Failure::new(Outcome::Malformed, "pdf_error"),
    }
}

fn tag(
    doc: &pdfium_render::prelude::PdfDocument<'_>,
    t: PdfDocumentMetadataTagType,
) -> Option<String> {
    doc.metadata()
        .get(t)
        .map(|v| v.value().trim().to_owned())
        .filter(|v| !v.is_empty())
}

/// Sends each page to `sink`, then reports the summary, or why the PDF has
/// no usable text.
pub fn extract(pdfium: &Pdfium, file: File, sink: &mut Sink<'_>) -> Result<Summary, Failure> {
    let doc = pdfium.load_pdf_from_reader(file, None).map_err(classify)?;
    let title = tag(&doc, PdfDocumentMetadataTagType::Title);
    let authors = tag(&doc, PdfDocumentMetadataTagType::Author)
        .into_iter()
        .collect();
    let mut budget = Budget::default();
    let mut readability = Readability::default();
    let mut truncated = 0u32;
    let mut pages = 0u32;
    for (n, page) in doc.pages().iter().enumerate() {
        let mut text = page.text().map_err(classify)?.all();
        if truncate_chars(&mut text, MAX_PASSAGE_CHARS) {
            truncated += 1;
        }
        readability.add_page(&text);
        budget.add(&text)?;
        let number = u32::try_from(n + 1).unwrap_or(u32::MAX);
        pages = number;
        let piece = Piece {
            ordinal: number - 1,
            page_no: Some(number),
            location: format!("Page {number}"),
            text,
        };
        if !sink(piece) {
            return Err(Failure::new(Outcome::Malformed, "output_closed"));
        }
    }
    if readability.needs_ocr(false) {
        let reason = if u64::from(readability.empty_pages) * 2 >= u64::from(readability.pages) {
            "no_text_layer"
        } else {
            "unreadable_text"
        };
        return Err(Failure::new(Outcome::NeedsOcr, reason));
    }
    Ok(Summary {
        page_count: pages,
        char_count: budget.chars,
        title,
        authors,
        language: None,
        truncated,
    })
}
