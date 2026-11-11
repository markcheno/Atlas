//! A PDF page for the page view (Slice 3.7, phase3-spec §7.5, ADR 0018):
//! the page as a PNG at a chosen width, and the rectangles, in the image's
//! pixels, where the passage is on it. Runs in the worker only.

use std::fs::File;

use pdfium_render::prelude::{
    PdfPage, PdfRenderConfig, PdfSearchOptions, Pdfium, PdfiumError, PdfiumInternalError,
};

use crate::services::extraction::protocol::RenderRect;

/// The widest image asked for.
pub const MAX_WIDTH: u32 = 2_000;
/// The image's long edge: a page taller than this at the width asked is
/// scaled down to it.
pub const MAX_EDGE: u32 = 3_000;
/// A larger PNG is refused as `too_large`.
pub const MAX_PNG_BYTES: usize = 8 * 1024 * 1024;
/// The passage is looked for by its first this many characters, then by
/// [`FALLBACK_CHARS`].
pub const FIND_CHARS: usize = 200;
pub const FALLBACK_CHARS: usize = 60;

/// A rendered page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub page: u32,
    pub page_count: u32,
    pub width: u32,
    pub height: u32,
    pub png: Vec<u8>,
    pub rects: Vec<RenderRect>,
}

/// `text` with runs of whitespace as one space, trimmed.
pub fn normalized(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn prefix(text: &str, n: usize) -> &str {
    match text.char_indices().nth(n) {
        Some((at, _)) => &text[..at],
        None => text,
    }
}

fn reason(err: PdfiumError) -> &'static str {
    match err {
        PdfiumError::PdfiumLibraryInternalError(
            PdfiumInternalError::PasswordError | PdfiumInternalError::SecurityError,
        ) => "encrypted",
        _ => "malformed",
    }
}

fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, &'static str> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().map_err(|_| "malformed")?;
        writer.write_image_data(rgba).map_err(|_| "malformed")?;
    }
    Ok(out)
}

/// Where `find` is on the page, in pixels: its first [`FIND_CHARS`]
/// characters, else its first [`FALLBACK_CHARS`]. Empty when it isn't
/// there, or the page has no text (a scan).
fn locate(page: &PdfPage<'_>, config: &PdfRenderConfig, find: &str) -> Vec<RenderRect> {
    let Ok(text) = page.text() else {
        return Vec::new();
    };
    let needle = normalized(find);
    for n in [FIND_CHARS, FALLBACK_CHARS] {
        let query = prefix(&needle, n);
        if query.is_empty() {
            continue;
        }
        let Ok(search) = text.search(query, &PdfSearchOptions::new()) else {
            continue;
        };
        let Some(segments) = search.find_next() else {
            continue;
        };
        let rects: Vec<RenderRect> = segments
            .iter()
            .filter_map(|segment| {
                let b = segment.bounds();
                let (x1, y1) = page.points_to_pixels(b.left(), b.top(), config).ok()?;
                let (x2, y2) = page.points_to_pixels(b.right(), b.bottom(), config).ok()?;
                let (x, y) = (x1.min(x2).max(0), y1.min(y2).max(0));
                Some(RenderRect {
                    x: x as u32,
                    y: y as u32,
                    width: (x1 - x2).unsigned_abs().max(1),
                    height: (y1 - y2).unsigned_abs().max(1),
                })
            })
            .collect();
        if !rects.is_empty() {
            return rects;
        }
    }
    Vec::new()
}

/// Renders page `page` (1-based) of `file` `width` pixels wide, and finds
/// `find` on it. The reason is a fixed code.
pub fn render(
    pdfium: &Pdfium,
    file: File,
    page: u32,
    width: u32,
    find: Option<&str>,
) -> Result<Page, &'static str> {
    let doc = pdfium.load_pdf_from_reader(file, None).map_err(reason)?;
    let pages = doc.pages();
    let page_count = u32::try_from(pages.len()).unwrap_or(0);
    if page == 0 || page > page_count {
        return Err("page_out_of_range");
    }
    let index = i32::try_from(page - 1).map_err(|_| "page_out_of_range")?;
    let p = pages.get(index).map_err(reason)?;
    let width = width.clamp(1, MAX_WIDTH);
    let config = PdfRenderConfig::new()
        .set_target_width(width as i32)
        .set_maximum_height(MAX_EDGE as i32);
    let bitmap = p.render_with_config(&config).map_err(reason)?;
    let (w, h) = (bitmap.width().max(0) as u32, bitmap.height().max(0) as u32);
    let png = encode_png(w, h, &bitmap.as_rgba_bytes())?;
    if png.len() > MAX_PNG_BYTES {
        return Err("too_large");
    }
    let rects = find.map(|f| locate(&p, &config, f)).unwrap_or_default();
    Ok(Page {
        page,
        page_count,
        width: w,
        height: h,
        png,
        rects,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_passage_is_looked_for_by_its_start_with_whitespace_collapsed() {
        assert_eq!(normalized("  a\n\tb   c "), "a b c");
        assert_eq!(prefix("ééé", 2), "éé");
        assert_eq!(prefix("ab", 60), "ab");
    }
}
