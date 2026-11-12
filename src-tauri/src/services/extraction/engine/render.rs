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
/// Then by pieces of about this many characters (Slice 3.9).
pub const PIECE_CHARS: usize = 60;
/// A piece shorter than this could match anywhere.
pub const PIECE_MIN_CHARS: usize = 20;
/// At most this many pieces are looked for.
pub const MAX_PIECES: usize = 16;

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

/// Characters PDFium's page search doesn't match as the page's text has
/// them: the mark it puts where a word is hyphenated across a line end
/// (U+0002) and other control characters, a soft hyphen, private-use
/// characters (search's highlight markers), the replacement character and
/// an ellipsis (a snippet's ends).
fn breaks_a_piece(c: char) -> bool {
    (c.is_control() && !c.is_whitespace())
        || matches!(c, '\u{AD}' | '\u{2026}' | '\u{FFFD}' | '\u{FFFE}')
        || ('\u{E000}'..='\u{F8FF}').contains(&c)
}

/// The pieces of `find` to look for when its start isn't found: its runs
/// between characters PDFium matches differently, cut at spaces into
/// pieces of about [`PIECE_CHARS`], in order, at most [`MAX_PIECES`].
pub fn pieces(find: &str) -> Vec<String> {
    let mut out = Vec::new();
    for run in find.split(breaks_a_piece) {
        let words: Vec<&str> = run.split_whitespace().collect();
        let mut piece = String::new();
        for w in words {
            if !piece.is_empty() && piece.chars().count() + 1 + w.chars().count() > PIECE_CHARS {
                if piece.chars().count() >= PIECE_MIN_CHARS {
                    out.push(std::mem::take(&mut piece));
                } else {
                    piece.clear();
                }
            }
            if !piece.is_empty() {
                piece.push(' ');
            }
            piece.push_str(w);
        }
        if piece.chars().count() >= PIECE_MIN_CHARS {
            out.push(piece);
        }
        if out.len() >= MAX_PIECES {
            out.truncate(MAX_PIECES);
            break;
        }
    }
    out
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

/// Where `find` is on the page, in pixels.
///
/// 1. Its first [`FIND_CHARS`] characters, else its first
///    [`FALLBACK_CHARS`] (Slice 3.7).
/// 2. Else every one of its [`pieces`] that is on the page (Slice 3.9): an
///    excerpt that crosses a hyphenated line end, or a search snippet with
///    its ellipses, is still outlined where its words are.
///
/// Empty when none of it is there, or the page has no text (a scan).
fn locate(page: &PdfPage<'_>, config: &PdfRenderConfig, find: &str) -> Vec<RenderRect> {
    let Ok(text) = page.text() else {
        return Vec::new();
    };
    let found = |query: &str| -> Vec<RenderRect> {
        if query.is_empty() {
            return Vec::new();
        }
        let Ok(search) = text.search(query, &PdfSearchOptions::new()) else {
            return Vec::new();
        };
        let Some(segments) = search.find_next() else {
            return Vec::new();
        };
        segments
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
            .collect()
    };
    let needle = normalized(find);
    for n in [FIND_CHARS, FALLBACK_CHARS] {
        let rects = found(prefix(&needle, n));
        if !rects.is_empty() {
            return rects;
        }
    }
    let mut rects: Vec<RenderRect> = Vec::new();
    for piece in pieces(find) {
        for r in found(&piece) {
            if !rects.contains(&r) {
                rects.push(r);
            }
        }
    }
    rects
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
    fn pieces_skip_what_pdfium_reads_differently() {
        // A hyphenated line end as PDFium writes it, and a snippet's ends.
        let find = "…the donor pools, alterna\u{2}\r\ntive preperiods, and leave one donor out refits of the synthetic control…";
        let got = pieces(find);
        assert!(got.iter().all(|p| !p.contains(['\u{2}', '…'])), "{got:?}");
        assert!(got.iter().all(|p| p.chars().count() >= PIECE_MIN_CHARS));
        assert!(got.iter().all(|p| p.chars().count() <= PIECE_CHARS));
        assert!(
            got.contains(&"the donor pools, alterna".to_owned()),
            "{got:?}"
        );
        assert!(
            got[1].starts_with("tive preperiods, and leave one donor"),
            "{got:?}"
        );
        // Short runs are left out; the count is capped.
        assert!(pieces("a\u{2}b").is_empty());
        assert!(pieces(&"word ".repeat(2_000)).len() == MAX_PIECES);
    }

    #[test]
    fn the_passage_is_looked_for_by_its_start_with_whitespace_collapsed() {
        assert_eq!(normalized("  a\n\tb   c "), "a b c");
        assert_eq!(prefix("ééé", 2), "éé");
        assert_eq!(prefix("ab", 60), "ab");
    }
}
