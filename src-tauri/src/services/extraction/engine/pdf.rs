//! PDF text with PDFium: one passage per page (phase1-spec §7.2), ported
//! from the Slice 0.9 spike's `pdf-pdfium` candidate.

use std::fs::File;

use pdfium_render::prelude::{
    PdfDocumentMetadataTagType, PdfPageText, Pdfium, PdfiumError, PdfiumInternalError,
};

use super::garble::Readability;
use super::title::{PageChar, first_page_title};
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

/// The first page's characters, for [`first_page_title`]. Characters PDFium
/// can't place are skipped.
fn page_chars(text: &PdfPageText<'_>) -> Vec<PageChar> {
    text.chars()
        .iter()
        .filter_map(|c| {
            let ch = c.unicode_char()?;
            let (x, y) = c.origin().ok()?;
            Some(PageChar {
                ch,
                size: c.scaled_font_size().value,
                x: x.value,
                y: y.value,
                angle: c.angle_degrees().unwrap_or(0.0),
            })
        })
        .collect()
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
    let mut content_title = None;
    for (n, page) in doc.pages().iter().enumerate() {
        let page_text = page.text().map_err(classify)?;
        if n == 0 {
            content_title = first_page_title(&page_chars(&page_text));
        }
        let mut text = page_text.all();
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
        return Err(Failure::needs_ocr(
            reason,
            readability.pages,
            readability.text_pages,
        ));
    }
    Ok(Summary {
        page_count: pages,
        char_count: budget.chars,
        title,
        content_title,
        authors,
        language: None,
        truncated,
        boilerplate_chars: 0,
    })
}
