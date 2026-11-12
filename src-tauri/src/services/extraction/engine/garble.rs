//! The unreadable-text rule (ADR 0008, phase1-spec §7.2), ported from the
//! Slice 0.9 spike's scorer (`spikes/extraction-bench/crates/score`).
//!
//! A PDF needs OCR when at least half its pages have fewer than 20 visible
//! characters (scans), or when at least 10% of its visible characters are
//! control, private-use or replacement characters (fonts without ToUnicode
//! maps). EPUBs are exempt from the half-pages rule. Then no passages are
//! stored: unreadable text would only pollute search.

/// Pages with fewer visible characters count as empty.
pub const EMPTY_PAGE_CHARS: usize = 20;

#[derive(Debug, Default, Clone, Copy)]
pub struct Readability {
    pub pages: u32,
    pub empty_pages: u32,
    /// Non-whitespace characters.
    pub visible: u64,
    pub bad: u64,
    /// Pages with at least `EMPTY_PAGE_CHARS` visible characters, under a
    /// tenth of them garbled: pages whose text layer is usable (Slice 4.1).
    pub text_pages: u32,
}

/// Whether a page's text counts as empty: what a scanned page leaves.
pub fn is_empty_page(text: &str) -> bool {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .take(EMPTY_PAGE_CHARS)
        .count()
        < EMPTY_PAGE_CHARS
}

fn is_bad(c: char) -> bool {
    (c.is_control() && !c.is_whitespace())
        || matches!(c as u32, 0xE000..=0xF8FF | 0xF0000..=0x10FFFF | 0xFFFD)
}

impl Readability {
    /// Adds one PDF page or EPUB spine document.
    pub fn add_page(&mut self, text: &str) {
        let mut visible = 0usize;
        let mut bad = 0usize;
        for c in text.chars().filter(|c| !c.is_whitespace()) {
            visible += 1;
            if is_bad(c) {
                bad += 1;
            }
        }
        self.pages += 1;
        self.visible += visible as u64;
        self.bad += bad as u64;
        if visible < EMPTY_PAGE_CHARS {
            self.empty_pages += 1;
        } else if bad * 10 < visible {
            self.text_pages += 1;
        }
    }

    pub fn needs_ocr(&self, epub: bool) -> bool {
        if self.pages == 0 {
            return false;
        }
        let mostly_empty = !epub && u64::from(self.empty_pages) * 2 >= u64::from(self.pages);
        let garbled = self.visible > 0 && self.bad * 10 >= self.visible;
        mostly_empty || garbled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(pages: &[&str], epub: bool) -> bool {
        let mut r = Readability::default();
        for p in pages {
            r.add_page(p);
        }
        r.needs_ocr(epub)
    }

    const PROSE: &str = "Alice was beginning to get very tired of sitting by her sister";

    #[test]
    fn usable_pages_are_counted() {
        let mut r = Readability::default();
        let garbled: String = "\u{E000}".repeat(30);
        for p in ["", "  short ", PROSE, &garbled, PROSE] {
            r.add_page(p);
        }
        assert_eq!((r.pages, r.empty_pages, r.text_pages), (5, 2, 2));
        assert!(is_empty_page(" short words "));
        assert!(!is_empty_page(PROSE));
    }

    #[test]
    fn scans_and_garbled_fonts_need_ocr() {
        assert!(rule(&["", "", PROSE], false), "two of three pages empty");
        assert!(rule(&["", PROSE], false), "exactly half");
        assert!(!rule(&["", PROSE, PROSE], false));
        let garbled: String = "\u{1}\u{2}\u{3}abcdefgh".repeat(10);
        assert!(rule(&[&garbled], false), "30% control characters");
        let private: String = format!("{}{}", "\u{E001}".repeat(10), "a".repeat(80));
        assert!(rule(&[&private], false), "11% private use");
        assert!(!rule(
            &[&format!("{}{}", "\u{FFFD}", "a".repeat(80))],
            false
        ));
    }

    #[test]
    fn epubs_are_exempt_from_the_empty_pages_rule() {
        assert!(!rule(&["", "", PROSE], true));
        assert!(
            rule(&[&"\u{FFFD}".repeat(50)], true),
            "but not from garbage"
        );
        assert!(!rule(&[], false), "no pages: nothing to judge");
    }
}
