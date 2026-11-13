//! ADR 0020's measures: normalisation, word and character error rates,
//! the order-free word error, and the uncertainty of a difference.

use unicode_normalization::UnicodeNormalization;

/// Both texts, engine and reference, pass through this before they are
/// compared:
/// - words broken across lines with a hyphen joined (before anything
///   folds the line ends away);
/// - NFKC (ligatures, the long s, full-width forms);
/// - curly quotes and primes to ASCII; en and em dashes and the minus sign
///   to `-`;
/// - soft hyphens and zero-width characters removed;
/// - lower case;
/// - runs of whitespace to one space, none at the ends.
pub fn normalise(text: &str) -> String {
    let joined = xbench::dehyphenate(text);
    let mut out = String::with_capacity(joined.len());
    let mut space = false;
    for c in joined.nfkc().flat_map(char::to_lowercase) {
        let c = match c {
            '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' | '\u{2032}' | '\u{00B4}' => '\'',
            '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' | '\u{2033}' => '"',
            '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}'
            | '\u{2212}' => '-',
            '\u{00AD}' | '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}' => {
                continue;
            }
            c => c,
        };
        if c.is_whitespace() {
            space = !out.is_empty();
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        out.push(c);
    }
    out
}

/// The words compared for WER: Slice 0.9's alphanumeric runs, over the
/// normalised text.
pub fn words(text: &str) -> Vec<String> {
    xbench::tokens(&normalise(text))
}

/// The fewest substitutions, insertions and deletions that turn `a` into
/// `b`. Two rows, so a page of 10,000 characters needs no matrix.
pub fn levenshtein<T: PartialEq>(a: &[T], b: &[T]) -> usize {
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, x) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            let substitute = prev[j] + usize::from(x != y);
            cur[j + 1] = substitute.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// One page's measures against its reference.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageScore {
    pub ref_words: usize,
    pub word_edits: usize,
    /// Word error rate: word edits ÷ reference words. Above 1 when the
    /// engine adds more than the reference holds.
    pub wer: f64,
    pub ref_chars: usize,
    pub char_edits: usize,
    pub cer: f64,
    /// 1 − word F1: what was misread or missed, whatever the order.
    pub unordered_word_error: f64,
    /// Slice 0.9's reading order: of the words both share, the share in
    /// the same order.
    pub reading_order: f64,
}

/// `text`'s measures against `reference`.
pub fn score(text: &str, reference: &str) -> PageScore {
    let (got, want) = (words(text), words(reference));
    let word_edits = levenshtein(&got, &want);
    let got_chars: Vec<char> = normalise(text).chars().collect();
    let want_chars: Vec<char> = normalise(reference).chars().collect();
    let char_edits = levenshtein(&got_chars, &want_chars);
    let (_, _, f1, common) = xbench::word_f1(&got, &want);
    let rate = |edits: usize, of: usize| {
        if of == 0 {
            if edits == 0 { 0.0 } else { 1.0 }
        } else {
            edits as f64 / of as f64
        }
    };
    PageScore {
        ref_words: want.len(),
        word_edits,
        wer: rate(word_edits, want.len()),
        ref_chars: want_chars.len(),
        char_edits,
        cer: rate(char_edits, want_chars.len()),
        unordered_word_error: if want.is_empty() && got.is_empty() {
            0.0
        } else {
            1.0 - f1
        },
        reading_order: xbench::reading_order(&got, &want, common),
    }
}

/// The corpus figure: the mean of the pages' rates, each page weighing
/// the same.
pub fn page_mean(rates: &[f64]) -> f64 {
    if rates.is_empty() {
        return 0.0;
    }
    rates.iter().sum::<f64>() / rates.len() as f64
}

/// A difference between two variants' corpus figures, with its 95%
/// interval.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Difference {
    /// Mean over pages of (a − b): positive when `b` does better.
    pub mean: f64,
    pub low: f64,
    pub high: f64,
}

/// A paired bootstrap of `a − b` over pages, resampled within each
/// category so every resample keeps the corpus's mix. `pages` holds
/// (category, a's rate, b's rate) for the pages both variants read.
pub fn bootstrap(pages: &[(String, f64, f64)], resamples: usize, seed: u64) -> Difference {
    let mean = page_mean(&pages.iter().map(|p| p.1 - p.2).collect::<Vec<_>>());
    if pages.len() < 2 || resamples == 0 {
        return Difference {
            mean,
            low: mean,
            high: mean,
        };
    }
    let mut by: std::collections::BTreeMap<&str, Vec<f64>> = Default::default();
    for (category, a, b) in pages {
        by.entry(category.as_str()).or_default().push(a - b);
    }
    let mut state = seed;
    let mut next = move || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let mut means: Vec<f64> = (0..resamples)
        .map(|_| {
            let mut sum = 0.0;
            for diffs in by.values() {
                for _ in 0..diffs.len() {
                    sum += diffs[(next() % diffs.len() as u64) as usize];
                }
            }
            sum / pages.len() as f64
        })
        .collect();
    means.sort_by(f64::total_cmp);
    let at = |q: f64| means[((q * (resamples - 1) as f64).round() as usize).min(resamples - 1)];
    Difference {
        mean,
        low: at(0.025),
        high: at(0.975),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalising_folds_what_shouldnt_count() {
        assert_eq!(
            normalise("“Curiouser  and\ncurious-\ner!” — said ’Alice’ − ﬁne\u{00AD}ly\u{200B}"),
            "\"curiouser and curiouser!\" - said 'alice' - finely"
        );
        // A hyphen before a capital is kept: it's a compound, not a break.
        assert_eq!(normalise("Anglo-\nSaxon"), "anglo- saxon");
        assert_eq!(normalise("  \n\t "), "");
    }

    #[test]
    fn levenshtein_counts_substitutions_once() {
        assert_eq!(levenshtein(&['a', 'b', 'c'], &['a', 'x', 'c']), 1);
        assert_eq!(levenshtein::<char>(&[], &['a', 'b']), 2);
        assert_eq!(levenshtein(&['a', 'b'], &[]), 2);
        let k: Vec<char> = "kitten".chars().collect();
        let s: Vec<char> = "sitting".chars().collect();
        assert_eq!(levenshtein(&k, &s), 3);
        assert_eq!(levenshtein(&s, &k), 3);
    }

    #[test]
    fn word_and_character_error_rates() {
        let page = score("The quick brown fox jumpd", "The quick brown fox jumped");
        assert_eq!((page.ref_words, page.word_edits), (5, 1));
        assert!((page.wer - 0.2).abs() < 1e-12);
        assert_eq!(page.char_edits, 1);
        assert_eq!(page.ref_chars, "the quick brown fox jumped".len());
        // The same words in another order: every word is right, the order
        // isn't, and only the ordered measures say so.
        let swapped = score("fox brown quick the", "the quick brown fox");
        assert!(swapped.wer > 0.5);
        assert!(swapped.unordered_word_error.abs() < 1e-12);
        assert!(swapped.reading_order < 0.5);
        // Nothing read from a page with text: every word missed.
        let empty = score("", "two words");
        assert_eq!((empty.wer, empty.cer), (1.0, 1.0));
        // An empty page read as empty is perfect; text added to it is not.
        assert_eq!(score("", "").wer, 0.0);
        assert_eq!(score("noise", "").wer, 1.0);
    }

    #[test]
    fn layout_differences_that_are_not_errors_cost_nothing() {
        let reference = "It was the best of times, it was the worst of times.";
        let page = score(
            "It  was the best of\ntimes, it was the wor-\nst of times.",
            reference,
        );
        assert_eq!((page.word_edits, page.char_edits), (0, 0));
    }

    #[test]
    fn the_bootstrap_brackets_the_mean_and_shrinks_with_agreement() {
        let mut pages = Vec::new();
        for i in 0..40 {
            let category = if i < 20 { "book" } else { "two-column" };
            let noise = (i as f64 * 0.37).sin() * 0.02;
            pages.push((category.to_owned(), 0.10 + noise, 0.06));
        }
        let d = bootstrap(&pages, 2_000, 1);
        assert!(d.low <= d.mean && d.mean <= d.high);
        assert!((d.mean - 0.04).abs() < 0.01);
        assert!(d.low > 0.0, "{d:?}");
        // The same seed gives the same interval.
        assert_eq!(d, bootstrap(&pages, 2_000, 1));
        // Pages that disagree widely give a wide interval around 0.
        let mixed: Vec<_> = (0..40)
            .map(|i| ("book".to_owned(), if i % 2 == 0 { 0.3 } else { 0.0 }, 0.15))
            .collect();
        let wide = bootstrap(&mixed, 2_000, 1);
        assert!(wide.low < 0.0 && wide.high > 0.0, "{wide:?}");
    }
}
