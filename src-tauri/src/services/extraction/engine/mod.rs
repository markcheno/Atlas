//! Extraction engines. They run only in the worker process (PDFium never
//! runs in the app), on files the worker opens itself (phase1-spec §7.2,
//! ADR 0008).
//!
//! Every outcome carries a fixed reason code, never a path or document text.

pub mod epub;
pub mod garble;
pub mod gutenberg;
pub mod hash;
pub mod pdf;
pub mod render;
pub mod sniff;
pub mod title;

use serde::{Deserialize, Serialize};

use crate::domain::dto::FileType;

/// Bump when a reader's output changes: documents read by an older version
/// are read again (phase1-spec §6.1), staying searchable meanwhile.
/// 2: a title from the first page (Slice 1.6).
pub const READER_VERSION_PDF: u32 = 2;
/// 2: plain text without Markdown marks such as `# ` (Slice 1.6).
/// 3: Project Gutenberg's header and licence left out (Slice 3.1).
pub const READER_VERSION_EPUB: u32 = 3;

/// The current reader version for a file type.
pub const fn reader_version(file_type: FileType) -> u32 {
    match file_type {
        FileType::Pdf => READER_VERSION_PDF,
        FileType::Epub => READER_VERSION_EPUB,
    }
}

/// Characters per document; past it the document is `too_large`.
pub const MAX_CHARS: u64 = 20_000_000;
/// Characters per page or passage; longer ones are cut, with a reason.
pub const MAX_PASSAGE_CHARS: usize = 1_000_000;
/// EPUB passages are split at paragraph boundaries into pieces of at most
/// this many characters.
pub const EPUB_PIECE_CHARS: usize = 4_000;

/// Why a document has no text, as stored in `documents.state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Encrypted,
    Malformed,
    Unsupported,
    TooLarge,
    NeedsOcr,
}

/// A failed extraction: the outcome and a fixed reason code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub outcome: Outcome,
    pub reason: &'static str,
}

impl Failure {
    pub const fn new(outcome: Outcome, reason: &'static str) -> Self {
        Self { outcome, reason }
    }
}

/// One stored passage: a PDF page, or a piece of an EPUB chapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Piece {
    pub ordinal: u32,
    /// 1-based PDF page; `None` for EPUB pieces.
    pub page_no: Option<u32>,
    /// What the UI shows: "Page 12", "Chapter 3, part 2".
    pub location: String,
    pub text: String,
}

/// What a successful extraction reports besides its pieces.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    /// PDF pages or EPUB spine documents.
    pub page_count: u32,
    pub char_count: u64,
    pub title: Option<String>,
    /// The largest text on a PDF's first page, when it stands out.
    #[serde(default)]
    pub content_title: Option<String>,
    pub authors: Vec<String>,
    pub language: Option<String>,
    /// Passages cut at [`MAX_PASSAGE_CHARS`].
    pub truncated: u32,
    /// Characters left out as Project Gutenberg's header and licence
    /// (EPUB reader version 3).
    #[serde(default)]
    pub boilerplate_chars: u64,
}

/// Receives pieces as they are extracted. Returns false to stop (the
/// worker's output is gone).
pub type Sink<'a> = dyn FnMut(Piece) -> bool + 'a;

/// Cuts `text` to at most `max` characters at a character boundary.
pub fn truncate_chars(text: &mut String, max: usize) -> bool {
    match text.char_indices().nth(max) {
        Some((at, _)) => {
            text.truncate(at);
            true
        }
        None => false,
    }
}

/// Counts characters, enforcing the per-document cap.
#[derive(Debug, Default)]
pub struct Budget {
    pub chars: u64,
}

impl Budget {
    pub fn add(&mut self, text: &str) -> Result<(), Failure> {
        self.chars += text.chars().count() as u64;
        if self.chars > MAX_CHARS {
            return Err(Failure::new(Outcome::TooLarge, "max_chars"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_respects_character_boundaries() {
        let mut s = "caféé".to_owned();
        assert!(truncate_chars(&mut s, 4));
        assert_eq!(s, "café");
        assert!(!truncate_chars(&mut s, 10));
    }

    #[test]
    fn the_budget_refuses_huge_documents() {
        let mut b = Budget {
            chars: MAX_CHARS - 1,
        };
        assert!(b.add("a").is_ok());
        assert_eq!(
            b.add("b"),
            Err(Failure::new(Outcome::TooLarge, "max_chars"))
        );
    }
}
