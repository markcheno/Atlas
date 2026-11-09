//! Snippets as typed spans (phase1-spec §7.4): FTS5 `snippet()` marks hits
//! with private-use characters, turned here into `SnippetSpan`s, so the UI
//! never receives or renders markup. Passages are stored without these two
//! characters (extraction strips them), so a document can't forge a hit.

use crate::domain::dto::SnippetSpan;

pub const HIT_START: char = '\u{E000}';
pub const HIT_END: char = '\u{E001}';

/// Removes the hit markers from stored text.
pub fn strip_markers(text: &str) -> String {
    text.replace([HIT_START, HIT_END], "")
}

/// Splits marked snippet text into spans, merging neighbours of one kind.
pub fn spans(marked: &str) -> Vec<SnippetSpan> {
    let mut out: Vec<SnippetSpan> = Vec::new();
    let mut hit = false;
    let mut current = String::new();
    let push = |text: &mut String, hit: bool, out: &mut Vec<SnippetSpan>| {
        if text.is_empty() {
            return;
        }
        match out.last_mut() {
            Some(last) if last.hit == hit => last.text.push_str(text),
            _ => out.push(SnippetSpan {
                text: std::mem::take(text),
                hit,
            }),
        }
        text.clear();
    };
    for c in marked.chars() {
        match c {
            HIT_START => {
                push(&mut current, hit, &mut out);
                hit = true;
            }
            HIT_END => {
                push(&mut current, hit, &mut out);
                hit = false;
            }
            // Whitespace runs (page layout) collapse to one space.
            c if c.is_whitespace() => {
                if !current.ends_with(' ') {
                    current.push(' ');
                }
            }
            c => current.push(c),
        }
    }
    push(&mut current, hit, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(text: &str, hit: bool) -> SnippetSpan {
        SnippetSpan {
            text: text.into(),
            hit,
        }
    }

    #[test]
    fn markers_become_spans() {
        assert_eq!(
            spans("…Quoth the \u{E000}Raven\u{E001}, \u{E000}Nevermore\u{E001}."),
            vec![
                s("…Quoth the ", false),
                s("Raven", true),
                s(", ", false),
                s("Nevermore", true),
                s(".", false)
            ]
        );
        assert_eq!(spans("no hits\n\n  here"), vec![s("no hits here", false)]);
        assert_eq!(spans(""), vec![]);
        // Adjacent hits merge.
        assert_eq!(
            spans("\u{E000}café\u{E001}\u{E000} crème\u{E001}"),
            vec![s("café crème", true)]
        );
    }

    #[test]
    fn stored_text_cannot_carry_markers() {
        assert_eq!(strip_markers("a\u{E000}b\u{E001}c"), "abc");
    }
}
