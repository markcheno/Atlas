//! Chunking (phase2-spec §6.1, ADR 0013): a passage is cut into chunks of
//! at most [`MAX_CHARS`] characters with about [`OVERLAP`] characters of
//! overlap, preferring a paragraph break, then a sentence end, then a
//! space. A chunk never crosses passages, so it keeps its page or chapter.
//!
//! Offsets are character (Unicode scalar value) offsets into the passage's
//! text. A chunk's identity is the BLAKE3 of its exact text: unchanged text
//! gives the same chunks and hashes, so its vectors are reused.
//!
//! Changing anything here that changes the output is a new
//! [`CHUNKER_VERSION`].

pub const CHUNKER_VERSION: i64 = 1;
/// About 300 tokens for an embedding model.
pub const MAX_CHARS: usize = 1200;
pub const OVERLAP: usize = 150;
/// A cut is looked for in the last half of a full window, so every chunk
/// but the last has at least this many characters.
const MIN_CUT: usize = MAX_CHARS / 2;

/// A chunk's place in its passage, in characters, end exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

/// A chunk of one passage, ready to store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk<'a> {
    pub span: Span,
    pub text: &'a str,
    pub hash: [u8; 32],
}

pub fn hash(text: &str) -> [u8; 32] {
    *blake3::hash(text.as_bytes()).as_bytes()
}

/// The chunks of one passage's text, in order. Whitespace-only text has
/// none.
pub fn chunk(text: &str) -> Vec<Chunk<'_>> {
    // Byte offset of every character, plus the end.
    let bytes: Vec<usize> = text
        .char_indices()
        .map(|(b, _)| b)
        .chain(std::iter::once(text.len()))
        .collect();
    let chars: Vec<char> = text.chars().collect();
    spans(&chars)
        .into_iter()
        .map(|span| {
            let t = &text[bytes[span.start]..bytes[span.end]];
            Chunk {
                span,
                text: t,
                hash: hash(t),
            }
        })
        .collect()
}

/// The text of `span` in `text`, or `None` if the span doesn't fit.
pub fn slice(text: &str, span: Span) -> Option<&str> {
    let mut indices = text
        .char_indices()
        .map(|(b, _)| b)
        .chain(std::iter::once(text.len()));
    let start = indices.nth(span.start)?;
    let end = if span.end == span.start {
        start
    } else {
        indices.nth(span.end - span.start - 1)?
    };
    text.get(start..end)
}

fn spans(chars: &[char]) -> Vec<Span> {
    let mut out = Vec::new();
    let end = trim_end(chars, chars.len(), 0);
    let mut pos = skip_space(chars, 0, end);
    while pos < end {
        if end - pos <= MAX_CHARS {
            out.push(Span { start: pos, end });
            break;
        }
        let cut = cut_point(chars, pos);
        let chunk_end = trim_end(chars, cut, pos);
        out.push(Span {
            start: pos,
            end: chunk_end,
        });
        let next = overlap_start(chars, pos, cut);
        pos = skip_space(chars, next, end);
    }
    out
}

/// Where to end a full window that starts at `pos` (exclusive).
fn cut_point(chars: &[char], pos: usize) -> usize {
    let lo = pos + MIN_CUT;
    let hi = pos + MAX_CHARS;
    // A paragraph break: cut before the blank line.
    if let Some(i) = (lo..hi)
        .rev()
        .find(|&i| chars[i] == '\n' && chars.get(i + 1) == Some(&'\n'))
    {
        return i;
    }
    // A sentence end: after the mark, before the space (or right after a
    // CJK full stop, which needs none).
    if let Some(i) = (lo..hi).rev().find(|&i| {
        let c = chars[i - 1];
        (matches!(c, '.' | '!' | '?') && chars[i].is_whitespace())
            || matches!(c, '。' | '！' | '？')
    }) {
        return i;
    }
    if let Some(i) = (lo..hi).rev().find(|&i| chars[i].is_whitespace()) {
        return i;
    }
    // An unbroken run: a hard cut.
    hi
}

/// The next chunk starts about [`OVERLAP`] characters before `cut`, at the
/// start of a word when there is one in that stretch.
fn overlap_start(chars: &[char], pos: usize, cut: usize) -> usize {
    let from = cut.saturating_sub(OVERLAP).max(pos + 1);
    (from..cut)
        .find(|&i| chars[i - 1].is_whitespace() && !chars[i].is_whitespace())
        .unwrap_or(from)
}

fn skip_space(chars: &[char], mut i: usize, end: usize) -> usize {
    while i < end && chars[i].is_whitespace() {
        i += 1;
    }
    i
}

fn trim_end(chars: &[char], mut end: usize, floor: usize) -> usize {
    while end > floor && chars[end - 1].is_whitespace() {
        end -= 1;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(n: usize) -> String {
        (0..n)
            .map(|i| format!("word{i}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Every chunk is within the limit, matches its span, and together they
    /// cover every non-space character of the text.
    fn check(text: &str) -> Vec<Chunk<'_>> {
        let chunks = chunk(text);
        let chars: Vec<char> = text.chars().collect();
        let mut covered = vec![false; chars.len()];
        for c in &chunks {
            let n = c.span.end - c.span.start;
            assert!(n > 0 && n <= MAX_CHARS, "length {n}");
            assert_eq!(slice(text, c.span), Some(c.text));
            assert_eq!(c.hash, hash(c.text));
            assert_eq!(c.text, c.text.trim(), "no space at either end");
            covered[c.span.start..c.span.end].fill(true);
        }
        for (i, ch) in chars.iter().enumerate() {
            assert!(covered[i] || ch.is_whitespace(), "char {i} not covered");
        }
        for w in chunks.windows(2) {
            assert!(w[1].span.start > w[0].span.start, "progress");
            assert!(w[1].span.start <= w[0].span.end, "overlap or touch");
        }
        chunks
    }

    #[test]
    fn short_text_is_one_chunk_and_space_is_none() {
        let c = check("  A short page.\n");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].text, "A short page.");
        assert!(chunk("").is_empty());
        assert!(chunk(" \n\t ").is_empty());
        assert_eq!(check(&"x".repeat(MAX_CHARS)).len(), 1);
    }

    #[test]
    fn long_text_is_cut_at_words_with_overlap() {
        let text = words(1000);
        let c = check(&text);
        assert!(c.len() >= 7, "{} chunks", c.len());
        for w in c.windows(2) {
            let overlap = w[0].span.end.saturating_sub(w[1].span.start);
            assert!((100..=OVERLAP).contains(&overlap), "overlap {overlap}");
            assert!(w[1].text.starts_with("word"), "starts at a word");
        }
    }

    #[test]
    fn prefers_paragraphs_then_sentences() {
        let para = format!("{}.\n\n{}", "a".repeat(800), words(200));
        let c = check(&para);
        assert_eq!(c[0].text, format!("{}.", "a".repeat(800)));

        let sentences = format!("{}. {}. {}", words(120), words(60), words(200));
        let c = check(&sentences);
        assert!(
            c[0].text.ends_with('.'),
            "{:?}",
            &c[0].text[c[0].text.len() - 20..]
        );
    }

    #[test]
    fn cjk_and_unbroken_runs() {
        let cjk = "天文学は宇宙を研究する学問である。".repeat(120);
        let c = check(&cjk);
        assert!(c.len() > 1);
        assert!(c[0].text.ends_with('。'));

        let run = "z".repeat(5000);
        let c = check(&run);
        assert_eq!(
            c[0].span,
            Span {
                start: 0,
                end: MAX_CHARS
            }
        );
        assert_eq!(c[1].span.start, MAX_CHARS - OVERLAP);
    }

    #[test]
    fn offsets_are_characters_and_hashes_are_stable() {
        let text = format!("é{}", "ü ".repeat(1000));
        let a = check(&text);
        let b = chunk(&text);
        assert_eq!(a, b);
        assert!(a[0].text.starts_with('é'));
        assert_eq!(slice(&text, Span { start: 0, end: 1 }), Some("é"));
        assert_eq!(
            slice(
                &text,
                Span {
                    start: 0,
                    end: 99_999
                }
            ),
            None
        );
    }
}
