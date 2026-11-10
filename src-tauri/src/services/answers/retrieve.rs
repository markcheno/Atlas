//! The excerpts an answer is made from (phase2-spec §7.6, ADR 0015).
//!
//! Hybrid search ranks the documents for the question; the best
//! [`DOCUMENTS`] give excerpts:
//!
//! - **by meaning**, the best chunk with the chunks either side of it in
//!   the same passage (so a page or chapter part stays one location);
//! - **by words**, the text around the first match in the best passage.
//!
//! Each document's stronger match comes first (every word, else meaning).
//! The excerpts share a budget of about 6,000 tokens, filled round-robin:
//! every document's first excerpt, then second ones, so one long book
//! can't crowd out the rest. Each excerpt is one numbered source, sent and
//! shown exactly as cut.

use rusqlite::{Connection, OptionalExtension};

use crate::domain::dto::SourceUsed;
use crate::services::embedding::chunk::{self, Span};
use crate::services::search::Found;

/// Documents an answer draws on.
pub const DOCUMENTS: usize = 8;
/// About 6,000 tokens of excerpts, at about four characters a token.
pub const BUDGET_CHARS: usize = 24_000;
/// The longest single excerpt.
pub const EXCERPT_CHARS: usize = 3_000;
/// Around a word match, this many characters.
pub const TEXT_WINDOW_CHARS: usize = 1_500;
/// An excerpt cut shorter than this to fit the budget isn't worth sending.
const SHORTEST: usize = 300;
/// How far above the model's similarity floor the best meaning match must
/// be for retrieval not to count as weak, when no document has every word.
pub const WEAK_MARGIN: f32 = 0.10;
/// Fewer sources (excerpts) than this and retrieval is weak.
pub const FEW_SOURCES: usize = 3;

/// A place to quote: a passage and a character range in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Place {
    passage: i64,
    span: Span,
}

/// The excerpts for `found`, numbered from 1, within the budget.
pub(crate) fn excerpts(conn: &Connection, found: &[Found]) -> rusqlite::Result<Vec<SourceUsed>> {
    let mut places: Vec<Vec<Place>> = Vec::new();
    for f in found {
        let mut mine = Vec::new();
        let text = match &f.text {
            Some((passage, expr)) => word_place(conn, *passage, expr)?,
            None => None,
        };
        let meaning = match f.meaning {
            Some((passage, (start, end))) => {
                Some(meaning_place(conn, passage, Span { start, end })?)
            }
            None => None,
        };
        let words_first = f.hit.all_words && text.is_some();
        let (first, second) = if words_first {
            (text, meaning)
        } else {
            (meaning, text)
        };
        mine.extend(first);
        if let Some(s) = second
            && !mine.iter().any(|p| overlaps(p, &s))
        {
            mine.push(s);
        }
        places.push(mine);
    }

    let mut out: Vec<SourceUsed> = Vec::new();
    let mut left = BUDGET_CHARS;
    let rounds = places.iter().map(Vec::len).max().unwrap_or(0);
    'fill: for round in 0..rounds {
        for (f, mine) in found.iter().zip(&places) {
            let Some(place) = mine.get(round) else {
                continue;
            };
            if left < SHORTEST {
                break 'fill;
            }
            let Some((text, location, page_no)) = quote(conn, place, left.min(EXCERPT_CHARS))?
            else {
                continue;
            };
            let document_id = f.hit.document_id.clone().unwrap_or_default();
            if out
                .iter()
                .any(|s| s.document_id == document_id && s.excerpt == text)
            {
                continue; // The same words twice (a repeated page).
            }
            left = left.saturating_sub(text.chars().count());
            out.push(SourceUsed {
                n: 0,
                document_id,
                file_instance_id: f.hit.file_instance_id.clone(),
                title: f.hit.title.clone(),
                display_name: f.hit.display_name.clone(),
                file_type: f.hit.file_type,
                location,
                page_no,
                excerpt: text,
            });
        }
    }
    // Reading order within a document isn't the point; rank order is.
    // Number them as they will be cited.
    for (i, s) in out.iter_mut().enumerate() {
        s.n = i as u32 + 1;
    }
    Ok(out)
}

/// Whether Atlas found little (phase2-spec §7.6): fewer than
/// [`FEW_SOURCES`] excerpts, or no document with every word and no meaning
/// match well above the model's floor.
pub(crate) fn weak(
    found: &[Found],
    sources: usize,
    best_similarity: Option<f32>,
    min_similarity: Option<f32>,
) -> bool {
    if sources < FEW_SOURCES {
        return true;
    }
    let every_word = found.iter().any(|f| f.hit.all_words && f.text.is_some());
    let close = match (best_similarity, min_similarity) {
        (Some(best), Some(floor)) => best >= floor + WEAK_MARGIN,
        _ => false,
    };
    !(every_word || close)
}

fn overlaps(a: &Place, b: &Place) -> bool {
    a.passage == b.passage && a.span.start < b.span.end && b.span.start < a.span.end
}

/// The best chunk and its neighbours in the same passage, if the document
/// was chunked that way (it was: meaning found it).
fn meaning_place(conn: &Connection, passage: i64, span: Span) -> rusqlite::Result<Place> {
    let around: Option<(i64, i64)> = conn
        .query_row(
            "SELECT MIN(n.char_start), MAX(n.char_end)
             FROM chunks c JOIN chunks n
               ON n.document_id = c.document_id AND n.passage_id = c.passage_id
              AND n.ordinal BETWEEN c.ordinal - 1 AND c.ordinal + 1
             WHERE c.passage_id = ?1 AND c.char_start = ?2 AND c.char_end = ?3",
            rusqlite::params![passage, span.start as i64, span.end as i64],
            |r| Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, Option<i64>>(1)?)),
        )
        .optional()?
        .and_then(|(a, b)| Some((a?, b?)));
    let span = match around {
        Some((start, end)) => Span {
            start: start.max(0) as usize,
            end: end.max(0) as usize,
        },
        None => span,
    };
    Ok(Place { passage, span })
}

/// The text around the first word match in `passage`.
fn word_place(conn: &Connection, passage: i64, expr: &str) -> rusqlite::Result<Option<Place>> {
    let marked: Option<String> = conn
        .query_row(
            "SELECT highlight(passages_fts, 0, char(57344), char(57345))
             FROM passages_fts WHERE passages_fts MATCH ?1 AND rowid = ?2",
            rusqlite::params![expr, passage],
            |r| r.get(0),
        )
        .optional()?;
    let Some(marked) = marked else {
        return Ok(None);
    };
    // The match's place in the unmarked text, in characters.
    let mut at = None;
    let mut chars = 0usize;
    for c in marked.chars() {
        match c {
            '\u{E000}' => {
                at.get_or_insert(chars);
            }
            '\u{E001}' => {}
            _ => chars += 1,
        }
    }
    let centre = at.unwrap_or(0);
    let half = TEXT_WINDOW_CHARS / 2;
    let start = centre.saturating_sub(half);
    let end = (start + TEXT_WINDOW_CHARS).min(chars);
    let start = end.saturating_sub(TEXT_WINDOW_CHARS);
    Ok(Some(Place {
        passage,
        span: Span { start, end },
    }))
}

/// An excerpt's text, its passage's location and page.
type Quoted = (String, Option<String>, Option<u32>);

/// The text of `place`, at most `max` characters around its middle, cut at
/// word boundaries; with its passage's location.
fn quote(conn: &Connection, place: &Place, max: usize) -> rusqlite::Result<Option<Quoted>> {
    let row: Option<(String, Option<String>, Option<u32>)> = conn
        .query_row(
            "SELECT text, location, page_no FROM passages WHERE id = ?1",
            [place.passage],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((text, location, page_no)) = row else {
        return Ok(None);
    };
    let total = text.chars().count();
    let (mut start, mut end) = (place.span.start.min(total), place.span.end.min(total));
    if end - start > max {
        let middle = start + (end - start) / 2;
        start = middle - max / 2;
        end = start + max;
    }
    let Some(cut) = chunk::slice(&text, Span { start, end }) else {
        return Ok(None);
    };
    let cut = at_words(cut, start > 0, end < total);
    if cut.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some((cut.to_owned(), location, page_no)))
}

/// Drops a partial word at a cut edge (not at the passage's own edges).
fn at_words(text: &str, cut_start: bool, cut_end: bool) -> &str {
    let mut t = text;
    if cut_start && let Some(i) = t.find(char::is_whitespace) {
        t = &t[i..];
    }
    if cut_end && let Some(i) = t.rfind(char::is_whitespace) {
        t = &t[..i];
    }
    t.trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuts_fall_between_words() {
        assert_eq!(at_words("lo world again", true, true), "world");
        assert_eq!(at_words("hello world", false, false), "hello world");
        assert_eq!(at_words("hello world", false, true), "hello");
    }

    #[test]
    fn places_overlap_only_in_the_same_passage() {
        let p = |passage, start, end| Place {
            passage,
            span: Span { start, end },
        };
        assert!(overlaps(&p(1, 0, 10), &p(1, 5, 20)));
        assert!(!overlaps(&p(1, 0, 10), &p(1, 10, 20)));
        assert!(!overlaps(&p(1, 0, 10), &p(2, 0, 10)));
    }
}
