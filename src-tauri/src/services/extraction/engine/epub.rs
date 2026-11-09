//! EPUB text with rbook and html2text (phase1-spec §7.2), ported from the
//! Slice 0.9 spike's `epub-rbook` candidate.
//!
//! A pre-scan of the zip runs **before rbook reads anything**: DRM, entries
//! that escape the archive, and oversized entries are refused there. (The
//! spike checked for DRM only after reading every chapter, and let a zip
//! bomb through.)

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use zip::ZipArchive;

use super::garble::Readability;
use super::{Budget, EPUB_PIECE_CHARS, Failure, Outcome, Piece, Sink, Summary};

/// Any single entry, uncompressed.
pub const MAX_ENTRY_BYTES: u64 = 32 * 1024 * 1024;
/// The whole archive, uncompressed.
pub const MAX_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;
/// `encryption.xml` larger than this is treated as DRM without parsing.
const MAX_ENCRYPTION_XML: u64 = 1024 * 1024;

/// Font obfuscation (IDPF and Adobe) also uses `encryption.xml`, but the
/// text stays readable, so it is not DRM.
const FONT_OBFUSCATION: &[&str] = &[
    "http://www.idpf.org/2008/embedding",
    "http://ns.adobe.com/pdf/enc#RC",
];

fn malformed(reason: &'static str) -> Failure {
    Failure::new(Outcome::Malformed, reason)
}

/// Checks the archive without decompressing anything but `mimetype` and
/// `META-INF/encryption.xml`.
pub fn prescan<R: Read + Seek>(reader: &mut R) -> Result<(), Failure> {
    let mut zip = ZipArchive::new(reader).map_err(|_| malformed("not_a_zip"))?;
    let mut total = 0u64;
    let mut mimetype = None;
    let mut encryption = None;
    for i in 0..zip.len() {
        let entry = zip.by_index_raw(i).map_err(|_| malformed("zip_entry"))?;
        if entry.enclosed_name().is_none() {
            return Err(Failure::new(Outcome::Unsupported, "zip_slip"));
        }
        let size = entry.size();
        if size > MAX_ENTRY_BYTES {
            return Err(Failure::new(Outcome::TooLarge, "entry_too_large"));
        }
        total = total.saturating_add(size);
        if total > MAX_ARCHIVE_BYTES {
            return Err(Failure::new(Outcome::TooLarge, "archive_too_large"));
        }
        match entry.name().as_deref() {
            Ok("mimetype") => mimetype = Some(i),
            Ok("META-INF/encryption.xml") => encryption = Some((i, size)),
            _ => {}
        }
    }

    let Some(i) = mimetype else {
        return Err(Failure::new(Outcome::Unsupported, "not_an_epub"));
    };
    let mut kind = String::new();
    zip.by_index(i)
        .map_err(|_| malformed("zip_entry"))?
        .take(256)
        .read_to_string(&mut kind)
        .map_err(|_| malformed("zip_entry"))?;
    if kind.trim() != "application/epub+zip" {
        return Err(Failure::new(Outcome::Unsupported, "not_an_epub"));
    }

    if let Some((i, size)) = encryption {
        if size > MAX_ENCRYPTION_XML {
            return Err(Failure::new(Outcome::Encrypted, "drm"));
        }
        let mut xml = String::new();
        zip.by_index(i)
            .map_err(|_| malformed("zip_entry"))?
            .read_to_string(&mut xml)
            .map_err(|_| Failure::new(Outcome::Encrypted, "drm"))?;
        if is_drm(&xml) {
            return Err(Failure::new(Outcome::Encrypted, "drm"));
        }
    }
    Ok(())
}

/// Whether `encryption.xml` encrypts anything with a real cipher, rather
/// than only obfuscating fonts.
fn is_drm(xml: &str) -> bool {
    let mut rest = xml;
    let mut any = false;
    while let Some(at) = rest.find("Algorithm=") {
        rest = &rest[at + "Algorithm=".len()..];
        let Some(quote) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') else {
            return true;
        };
        let body = &rest[1..];
        let Some(end) = body.find(quote) else {
            return true;
        };
        any = true;
        if !FONT_OBFUSCATION.contains(&body[..end].trim()) {
            return true;
        }
        rest = &body[end..];
    }
    // An encryption.xml we can't understand is treated as DRM.
    !any
}

/// Splits chapter text into pieces of at most `max` characters, at
/// paragraph boundaries where possible, then at whitespace.
pub fn split_pieces(text: &str, max: usize) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;
    let flush = |current: &mut String, current_len: &mut usize, pieces: &mut Vec<String>| {
        let piece = current.trim();
        if !piece.is_empty() {
            pieces.push(piece.to_owned());
        }
        current.clear();
        *current_len = 0;
    };
    for paragraph in text.split("\n\n").map(str::trim).filter(|p| !p.is_empty()) {
        let len = paragraph.chars().count();
        if len > max {
            flush(&mut current, &mut current_len, &mut pieces);
            let mut part = String::new();
            let mut part_len = 0usize;
            for word in paragraph.split_whitespace() {
                let wlen = word.chars().count();
                if part_len > 0 && part_len + 1 + wlen > max {
                    pieces.push(std::mem::take(&mut part));
                    part_len = 0;
                }
                if wlen > max {
                    // One unbroken run longer than a piece: cut it.
                    let chars: Vec<char> = word.chars().collect();
                    for chunk in chars.chunks(max) {
                        pieces.push(chunk.iter().collect());
                    }
                    continue;
                }
                if part_len > 0 {
                    part.push(' ');
                    part_len += 1;
                }
                part.push_str(word);
                part_len += wlen;
            }
            if !part.is_empty() {
                pieces.push(part);
            }
            continue;
        }
        if current_len > 0 && current_len + 2 + len > max {
            flush(&mut current, &mut current_len, &mut pieces);
        }
        if current_len > 0 {
            current.push_str("\n\n");
            current_len += 2;
        }
        current.push_str(paragraph);
        current_len += len;
    }
    flush(&mut current, &mut current_len, &mut pieces);
    pieces
}

/// A chapter's text with no Markdown marks: headings, links, list bullets
/// and quotes come out as plain text (`TrivialDecorator`), so snippets
/// never start with `# ` (reader version 2).
pub fn chapter_text(html: &[u8]) -> Result<String, Failure> {
    html2text::config::with_decorator(html2text::render::TrivialDecorator::new())
        .string_from_read(html, 100_000)
        .map_err(|_| malformed("chapter_html"))
}

/// Sends each chapter piece to `sink`, then reports the summary, or why the
/// EPUB has no usable text.
pub fn extract(mut file: File, sink: &mut Sink<'_>) -> Result<Summary, Failure> {
    prescan(&mut file)?;
    file.seek(SeekFrom::Start(0))
        .map_err(|_| malformed("read_failed"))?;
    let epub = rbook::Epub::read(file).map_err(|_| malformed("epub_structure"))?;
    let meta = epub.metadata();
    let clean = |s: &str| Some(s.trim().to_owned()).filter(|s| !s.is_empty());
    let title = meta.title().and_then(|t| clean(t.value()));
    let authors = meta.creators().filter_map(|c| clean(c.value())).collect();
    let language = meta.language().and_then(|l| clean(l.value()));

    let mut budget = Budget::default();
    let mut readability = Readability::default();
    let mut ordinal = 0u32;
    let mut chapters = 0u32;
    for (n, item) in epub.reader().enumerate() {
        let item = item.map_err(|_| malformed("chapter_unreadable"))?;
        chapters += 1;
        let text = chapter_text(item.content().as_bytes())?;
        readability.add_page(&text);
        budget.add(&text)?;
        for (m, text) in split_pieces(&text, EPUB_PIECE_CHARS)
            .into_iter()
            .enumerate()
        {
            let piece = Piece {
                ordinal,
                page_no: None,
                location: format!("Chapter {}, part {}", n + 1, m + 1),
                text,
            };
            if !sink(piece) {
                return Err(malformed("output_closed"));
            }
            ordinal += 1;
        }
    }
    if readability.needs_ocr(true) {
        return Err(Failure::new(Outcome::NeedsOcr, "unreadable_text"));
    }
    Ok(Summary {
        page_count: chapters,
        char_count: budget.chars,
        title,
        content_title: None,
        authors,
        language,
        truncated: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    #[test]
    fn chapter_text_carries_no_markdown_marks() {
        let html = br#"<html><body><h1>Chapter 1</h1>
            <p>Read <a href="n.xhtml">the note</a> and <em>this</em>.</p>
            <ul><li>first</li><li>second</li></ul>
            <blockquote>Quoted</blockquote><h2>Part two</h2></body></html>"#;
        let text = chapter_text(html).unwrap();
        assert!(text.starts_with("Chapter 1"), "{text:?}");
        for mark in ["# ", "[", "]", "* ", "> ", "*this*"] {
            assert!(!text.contains(mark), "{mark:?} in {text:?}");
        }
        for words in ["the note", "this", "first", "second", "Quoted", "Part two"] {
            assert!(text.contains(words), "{words:?} missing from {text:?}");
        }
    }
    use zip::write::SimpleFileOptions;

    fn epub(entries: &[(&str, &[u8])]) -> Cursor<Vec<u8>> {
        let mut out = Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut out);
            let stored =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            for (name, data) in entries {
                z.start_file(*name, stored).unwrap();
                z.write_all(data).unwrap();
            }
            z.finish().unwrap();
        }
        out.set_position(0);
        out
    }

    #[test]
    fn the_prescan_refuses_drm_and_unsafe_archives_before_reading_chapters() {
        let mime: (&str, &[u8]) = ("mimetype", b"application/epub+zip");
        assert!(prescan(&mut epub(&[mime])).is_ok());
        let drm = br#"<encryption><EncryptedData><EncryptionMethod Algorithm="http://www.w3.org/2001/04/xmlenc#aes128-cbc"/></EncryptedData></encryption>"#;
        assert_eq!(
            prescan(&mut epub(&[mime, ("META-INF/encryption.xml", drm)])),
            Err(Failure::new(Outcome::Encrypted, "drm"))
        );
        let fonts = br#"<encryption><EncryptionMethod Algorithm="http://www.idpf.org/2008/embedding"/></encryption>"#;
        assert!(prescan(&mut epub(&[mime, ("META-INF/encryption.xml", fonts)])).is_ok());
        assert_eq!(
            prescan(&mut epub(&[mime, ("../../escape.xhtml", b"x")])),
            Err(Failure::new(Outcome::Unsupported, "zip_slip"))
        );
        assert_eq!(
            prescan(&mut epub(&[("mimetype", b"application/zip")])),
            Err(Failure::new(Outcome::Unsupported, "not_an_epub"))
        );
        assert_eq!(
            prescan(&mut epub(&[("a.txt", b"hi")])),
            Err(Failure::new(Outcome::Unsupported, "not_an_epub"))
        );
        assert_eq!(
            prescan(&mut Cursor::new(b"PK\x03\x04 broken".to_vec())),
            Err(Failure::new(Outcome::Malformed, "not_a_zip"))
        );
    }

    #[test]
    fn oversized_entries_are_refused_from_their_declared_size() {
        // A deflated entry that expands past the cap, like the corpus's
        // zip bomb: the declared size is checked, nothing is inflated.
        let mut out = Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut out);
            z.start_file(
                "mimetype",
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
            z.write_all(b"application/epub+zip").unwrap();
            z.start_file("OEBPS/big.xhtml", SimpleFileOptions::default())
                .unwrap();
            let zeros = vec![0u8; 1024 * 1024];
            for _ in 0..33 {
                z.write_all(&zeros).unwrap();
            }
            z.finish().unwrap();
        }
        out.set_position(0);
        assert_eq!(
            prescan(&mut out),
            Err(Failure::new(Outcome::TooLarge, "entry_too_large"))
        );
    }

    #[test]
    fn pieces_split_at_paragraphs_then_words() {
        let text = format!(
            "{}\n\n{}\n\n{}",
            "a ".repeat(10),
            "b ".repeat(10),
            "c".repeat(25)
        );
        let pieces = split_pieces(&text, 30);
        assert!(pieces.iter().all(|p| p.chars().count() <= 30), "{pieces:?}");
        assert_eq!(pieces.concat().matches('a').count(), 10);
        assert_eq!(pieces.concat().matches('c').count(), 25);
        let long = "word ".repeat(100);
        let pieces = split_pieces(&long, 40);
        assert!(pieces.iter().all(|p| p.chars().count() <= 40));
        assert_eq!(pieces.join(" ").split_whitespace().count(), 100);
        assert!(split_pieces("\n\n  \n\n", 10).is_empty());
    }
}
