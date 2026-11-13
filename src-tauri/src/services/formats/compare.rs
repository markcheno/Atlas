//! Whether two texts are one book (Slice 4.5.1, ADR 0023): deterministic,
//! cheap, and with no AI.
//!
//! Each text is normalised (PDF hyphenation joined, ligatures and accents
//! folded, lowercased) and cut into words; every run of [`SHINGLE`] words is
//! hashed, and a fixed eighth of the hashes is kept (those divisible by
//! [`SAMPLE`]), so a book's sketch is about one integer per eight words and
//! the same text always keeps the same ones. Two books are the same when the
//! smaller one's sketch is mostly in the larger one's (a PDF adds running
//! heads and page numbers, so containment, not overlap) and their lengths are
//! close (so one chapter isn't the whole book).

use std::collections::{HashSet, VecDeque};

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// Changes with any change below, so pairs are checked again.
pub const CHECK_VERSION: i64 = 1;
/// Words per shingle.
pub const SHINGLE: usize = 5;
/// One hash in this many is kept.
pub const SAMPLE: u64 = 8;
/// Sketch entries below which a text is too short to judge (about 500
/// words): the pair is then made on the name alone.
pub const MIN_SKETCH: usize = 64;
/// The verdict's thresholds (provisional; measured in the benchmarks).
pub const SAME_SIMILARITY: f64 = 0.5;
pub const SAME_LENGTH: f64 = 0.5;

/// A text's sample of shingle hashes, built a passage at a time.
#[derive(Debug, Default, Clone)]
pub struct Sketch {
    hashes: HashSet<u64>,
    window: VecDeque<String>,
    words: u64,
}

impl Sketch {
    /// Adds a passage. Shingles run on across passages, as the text does.
    pub fn add(&mut self, text: &str) {
        for word in words(&normalise(text)) {
            self.words += 1;
            if self.window.len() == SHINGLE {
                self.window.pop_front();
            }
            self.window.push_back(word);
            if self.window.len() == SHINGLE {
                let mut h = blake3::Hasher::new();
                for (i, w) in self.window.iter().enumerate() {
                    if i > 0 {
                        h.update(&[0x1f]);
                    }
                    h.update(w.as_bytes());
                }
                let bytes = h.finalize();
                let n = u64::from_le_bytes(bytes.as_bytes()[..8].try_into().unwrap());
                if n % SAMPLE == 0 {
                    self.hashes.insert(n);
                }
            }
        }
    }

    pub fn words(&self) -> u64 {
        self.words
    }

    pub fn len(&self) -> usize {
        self.hashes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hashes.is_empty()
    }

    /// Enough text to judge by.
    pub fn usable(&self) -> bool {
        self.hashes.len() >= MIN_SKETCH
    }
}

/// The text check's result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Comparison {
    /// The share of the smaller sketch found in the larger.
    pub similarity: f64,
    /// Words in the shorter text over words in the longer.
    pub length_ratio: f64,
    pub same: bool,
}

pub fn compare(a: &Sketch, b: &Sketch) -> Comparison {
    let (small, large) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    let shared = small.hashes.iter().filter(|h| large.hashes.contains(h)).count();
    let similarity = if small.is_empty() {
        0.0
    } else {
        shared as f64 / small.len() as f64
    };
    let (lo, hi) = (a.words.min(b.words), a.words.max(b.words));
    let length_ratio = if hi == 0 { 0.0 } else { lo as f64 / hi as f64 };
    Comparison {
        similarity,
        length_ratio,
        same: similarity >= SAME_SIMILARITY && length_ratio >= SAME_LENGTH,
    }
}

/// Joins words a PDF broke across lines: PDFium's hyphenation mark (U+0002)
/// or a soft hyphen, with or without the line break after it, and a hyphen
/// at a line's end before a lowercase letter.
fn normalise(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let marker = c == '\u{2}' || c == '\u{AD}';
        let hyphen = c == '-' && i > 0 && chars[i - 1].is_alphabetic();
        if marker || hyphen {
            let mut j = i + 1;
            while j < chars.len() && matches!(chars[j], ' ' | '\t') {
                j += 1;
            }
            let mut broke = false;
            while j < chars.len() && matches!(chars[j], '\r' | '\n') {
                j += 1;
                broke = true;
            }
            if marker {
                i = if broke { j } else { i + 1 };
                continue;
            }
            if broke && chars.get(j).is_some_and(|c| c.is_lowercase()) {
                i = j;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// CJK ideographs, kana and hangul: a word each, as they aren't spaced.
fn ideographic(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF
        | 0xF900..=0xFAFF | 0x20000..=0x2FA1F)
}

/// Lowercased words with ligatures and accents folded away.
fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    for c in text.nfkd().filter(|c| !is_combining_mark(*c)) {
        if ideographic(c) {
            if !word.is_empty() {
                out.push(std::mem::take(&mut word));
            }
            out.push(c.to_string());
        } else if c.is_alphanumeric() {
            word.extend(c.to_lowercase());
        } else if !word.is_empty() {
            out.push(std::mem::take(&mut word));
        }
    }
    if !word.is_empty() {
        out.push(word);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// About `n` words of varied, deterministic prose.
    fn prose(seed: u64, n: usize) -> String {
        const WORDS: &[&str] = &[
            "river", "lantern", "harbour", "quiet", "meadow", "stone", "letter", "winter",
            "candle", "orchard", "distant", "garden", "silver", "thunder", "valley", "market",
            "window", "journey", "evening", "forest", "promise", "shadow", "copper", "village",
        ];
        let mut x = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        (0..n)
            .map(|_| {
                x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                WORDS[(x >> 33) as usize % WORDS.len()]
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn sketch(parts: &[&str]) -> Sketch {
        let mut s = Sketch::default();
        for p in parts {
            s.add(p);
        }
        s
    }

    /// `text` cut into pages of `per` words with a running head and a page
    /// number, as a PDF reads.
    fn as_pdf(text: &str, per: usize) -> Vec<String> {
        let words: Vec<&str> = text.split(' ').collect();
        words
            .chunks(per)
            .enumerate()
            .map(|(i, page)| format!("THE BOOK OF THINGS\n{}\n{}", page.join(" "), i + 1))
            .collect()
    }

    #[test]
    fn one_text_cut_into_pages_and_into_chapters_is_the_same_book() {
        let text = prose(1, 20_000);
        let pdf = as_pdf(&text, 300);
        let words: Vec<&str> = text.split(' ').collect();
        let epub: Vec<String> = words.chunks(2_000).map(|c| c.join(" ")).collect();
        let a = sketch(&pdf.iter().map(String::as_str).collect::<Vec<_>>());
        let b = sketch(&epub.iter().map(String::as_str).collect::<Vec<_>>());
        let c = compare(&a, &b);
        assert!(c.same, "{c:?}");
        assert!(c.similarity > 0.85, "{c:?}");
    }

    #[test]
    fn different_books_are_not() {
        let a = sketch(&[&prose(1, 20_000)]);
        let b = sketch(&[&prose(2, 20_000)]);
        let c = compare(&a, &b);
        assert!(!c.same && c.similarity < 0.05, "{c:?}");
    }

    #[test]
    fn one_chapter_is_not_the_whole_book() {
        let text = prose(3, 20_000);
        let chapter: String = text.split(' ').take(2_000).collect::<Vec<_>>().join(" ");
        let c = compare(&sketch(&[&text]), &sketch(&[&chapter]));
        assert!(c.similarity > 0.9, "the chapter is in the book: {c:?}");
        assert!(!c.same, "but it isn't the book: {c:?}");
    }

    #[test]
    fn pdf_hyphenation_soft_hyphens_and_ligatures_fold_away() {
        assert_eq!(words(&normalise("alterna\u{2}\r\ntive")), ["alternative"]);
        assert_eq!(words(&normalise("alterna\u{2}tive")), ["alternative"]);
        assert_eq!(words(&normalise("soft\u{AD}ware")), ["software"]);
        assert_eq!(words(&normalise("exam-\nple")), ["example"]);
        // A hyphen before a capital is a real one.
        assert_eq!(words(&normalise("Anglo-\nSaxon")), ["anglo", "saxon"]);
        assert_eq!(words("ﬁnal Café"), ["final", "cafe"]);
    }

    #[test]
    fn unspaced_scripts_count_each_character() {
        assert_eq!(words("東京タワー"), ["東", "京", "タ", "ワ", "ー"].map(String::from));
        let text: String = "春夏秋冬山川草木".repeat(400);
        assert!(sketch(&[&text]).words() > 3_000);
    }

    #[test]
    fn a_tiny_text_is_too_short_to_judge_and_the_sketch_is_repeatable() {
        assert!(!sketch(&["just a few words here"]).usable());
        assert!(!sketch(&[]).usable());
        let a = sketch(&[&prose(5, 5_000)]);
        let b = sketch(&[&prose(5, 5_000)]);
        assert!(a.usable());
        assert_eq!(a.hashes, b.hashes);
        assert_eq!(compare(&a, &b).similarity, 1.0);
    }
}
