//! Search-key folding: case- and accent-insensitive, normalization-proof.
//!
//! Decompose (NFD), drop combining marks, lowercase, recompose (NFC). So
//! "Café", "CAFÉ", "cafe", and the decomposed form macOS filesystems may
//! return all fold to "cafe". Scripts without combining accents (CJK) pass
//! through unchanged apart from case.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

pub fn fold(text: &str) -> String {
    let stripped: String = text.nfd().filter(|c| !is_combining_mark(*c)).collect();
    stripped.to_lowercase().nfc().collect()
}

/// Escapes `%`, `_` and `\` for `LIKE … ESCAPE '\'`.
pub fn like_escape(term: &str) -> String {
    let mut out = String::with_capacity(term.len());
    for c in term.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn case_accents_and_normalization_fold_together() {
        assert_eq!(fold("Café Crème"), "cafe creme");
        assert_eq!(fold("CAFÉ"), "cafe");
        assert_eq!(fold("Cafe\u{301}"), "cafe", "NFD input");
        assert_eq!(fold("Ünïcode — naïve résumé"), "unicode — naive resume");
        assert_eq!(fold("日本語の本"), "日本語の本");
        assert_eq!(fold("ÅNGSTRÖM"), "angstrom");
    }

    #[test]
    fn like_wildcards_are_escaped() {
        assert_eq!(like_escape("100%"), "100\\%");
        assert_eq!(like_escape("a_b"), "a\\_b");
        assert_eq!(like_escape("c:\\x"), "c:\\\\x");
        assert_eq!(like_escape("plain"), "plain");
    }
}
