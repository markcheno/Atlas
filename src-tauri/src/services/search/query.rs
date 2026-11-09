//! Turns what the user typed into FTS5 queries (phase1-spec §7.4, ADR 0010).
//!
//! Input is data, never FTS syntax: every word and phrase is quoted, so
//! `*`, `NEAR(`, parentheses, `AND`/`OR`/`NOT`, `^`, `:` and quotes are
//! searched as words. The only operator a user can reach is a trailing `*`
//! on a word, which means "words starting with". Words are ANDed; the
//! any-words form (for the fallback when few documents match all words)
//! drops common words and ORs the rest.

/// One unit of the query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    Word(String),
    /// A trailing `*`: words starting with this.
    Prefix(String),
    /// `"two words"`: the words in this order.
    Phrase(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    pub tokens: Vec<Token>,
    /// Every token must match.
    pub all: String,
    /// Any non-common token may match; `None` when it would be the same
    /// query as `all` or empty.
    pub any: Option<String>,
}

/// Common words left out of the any-words form (English, plus the French
/// of the corpus). They stay in the all-words form, where they cost
/// nothing: bm25 weighs them low.
const STOPWORDS: &[&str] = &[
    "a", "about", "an", "and", "are", "as", "at", "be", "been", "but", "by", "can", "could", "did",
    "do", "does", "for", "from", "had", "has", "have", "he", "her", "his", "how", "i", "if", "in",
    "into", "is", "it", "its", "of", "on", "or", "our", "she", "should", "so", "than", "that",
    "the", "their", "them", "then", "there", "they", "this", "to", "was", "we", "were", "what",
    "when", "where", "which", "who", "why", "will", "with", "would", "you", "your", "au", "aux",
    "ce", "dans", "de", "des", "du", "en", "est", "et", "il", "la", "le", "les", "un", "une",
    "pour", "que", "qui", "sur",
];

fn has_word_chars(s: &str) -> bool {
    s.chars().any(char::is_alphanumeric)
}

/// A string literal for FTS5: quotes doubled inside.
fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

fn render(token: &Token) -> String {
    match token {
        Token::Word(w) => quote(w),
        Token::Prefix(w) => format!("{}*", quote(w)),
        Token::Phrase(words) => quote(&words.join(" ")),
    }
}

fn is_stopword(token: &Token) -> bool {
    match token {
        Token::Word(w) => STOPWORDS.contains(&w.to_lowercase().as_str()),
        Token::Prefix(_) | Token::Phrase(_) => false,
    }
}

/// Splits input into words and quoted phrases.
pub fn tokens(input: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        if c == '"' {
            chars.next();
            let mut phrase = String::new();
            for c in chars.by_ref() {
                if c == '"' {
                    break;
                }
                phrase.push(c);
            }
            let words: Vec<String> = phrase
                .split_whitespace()
                .filter(|w| has_word_chars(w))
                .map(str::to_owned)
                .collect();
            match words.len() {
                0 => {}
                1 => out.push(Token::Word(words.into_iter().next().expect("one"))),
                _ => out.push(Token::Phrase(words)),
            }
            continue;
        }
        let mut word = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() || c == '"' {
                break;
            }
            word.push(c);
            chars.next();
        }
        if !has_word_chars(&word) {
            continue;
        }
        let stem = word.trim_end_matches('*');
        if stem.len() < word.len() && has_word_chars(stem) {
            out.push(Token::Prefix(stem.to_owned()));
        } else {
            out.push(Token::Word(word));
        }
    }
    out
}

/// `None` when nothing searchable was typed (empty or punctuation only).
pub fn build(input: &str) -> Option<Query> {
    let tokens = tokens(input);
    if tokens.is_empty() {
        return None;
    }
    let all = tokens.iter().map(render).collect::<Vec<_>>().join(" ");
    let kept: Vec<String> = tokens
        .iter()
        .filter(|t| !is_stopword(t))
        .map(render)
        .collect();
    // With fewer than two words left, "any" would just repeat "all".
    let any = (kept.len() >= 2)
        .then(|| kept.join(" OR "))
        .filter(|any| *any != all);
    Some(Query { tokens, all, any })
}

/// The words for the file-name match (Phase 0's folded `search_key`).
pub fn name_terms(query: &Query) -> Vec<String> {
    query
        .tokens
        .iter()
        .flat_map(|t| match t {
            Token::Word(w) | Token::Prefix(w) => vec![w.clone()],
            Token::Phrase(words) => words.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(input: &str) -> Option<String> {
        build(input).map(|q| q.all)
    }

    #[test]
    fn words_are_quoted_so_nothing_typed_is_an_operator() {
        assert_eq!(all("harmonic drive"), Some(r#""harmonic" "drive""#.into()));
        assert_eq!(all("NEAR(a b)"), Some(r#""NEAR(a" "b)""#.into()));
        assert_eq!(all("cats AND dogs"), Some(r#""cats" "AND" "dogs""#.into()));
        assert_eq!(all("NOT this"), Some(r#""NOT" "this""#.into()));
        assert_eq!(all("title:raven"), Some(r#""title:raven""#.into()));
        assert_eq!(all("^start"), Some(r#""^start""#.into()));
        assert_eq!(all("(x)"), Some(r#""(x)""#.into()));
        assert_eq!(all(r#"it's "#), Some(r#""it's""#.into()));
    }

    #[test]
    fn only_a_trailing_star_makes_a_prefix() {
        assert_eq!(all("meteor*"), Some(r#""meteor"*"#.into()));
        assert_eq!(all("met*eor"), Some(r#""met*eor""#.into()));
        assert_eq!(all("*meteor"), Some(r#""*meteor""#.into()));
        assert_eq!(all("***"), None);
    }

    #[test]
    fn phrases_keep_their_words_together() {
        assert_eq!(
            all(r#""café crème" prix"#),
            Some(r#""café crème" "prix""#.into())
        );
        assert_eq!(
            all(r#""unclosed phrase"#),
            Some(r#""unclosed phrase""#.into())
        );
        assert_eq!(all(r#""" "  ""#), None);
        assert_eq!(all(r#""one""#), Some(r#""one""#.into()));
    }

    #[test]
    fn empty_and_punctuation_only_input_is_no_query() {
        assert_eq!(build(""), None);
        assert_eq!(build("   "), None);
        assert_eq!(build("?! -- … ()"), None);
    }

    #[test]
    fn the_any_words_form_drops_common_words() {
        let q = build("should a ruler be loved or feared").unwrap();
        assert_eq!(q.any.as_deref(), Some(r#""ruler" OR "loved" OR "feared""#));
        assert_eq!(build("raven").unwrap().any, None, "one word: same query");
        assert_eq!(build("the raven").unwrap().any, None, "one word left");
        let q = build(r#""bleak December" raven"#).unwrap();
        assert_eq!(q.any.as_deref(), Some(r#""bleak December" OR "raven""#));
    }

    #[test]
    fn non_latin_runs_are_passed_through() {
        assert_eq!(all("道德經"), Some("\"道德經\"".into()));
    }
}
