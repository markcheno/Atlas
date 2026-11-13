//! Names for areas, from the books' own words: the words that many of an
//! area's books use and other areas' books mostly don't. Computed here,
//! on this computer; nothing is sent to a model.
//!
//! A word counts once per book, however often the book repeats it, so one
//! long book can't name an area by itself.

use std::collections::{BTreeMap, BTreeSet};

use super::cluster::Area;
use super::{Points, dot};
use crate::services::search::query::is_common_word;

const MIN_LEN: usize = 3;
const MAX_LEN: usize = 24;

/// English words that say nothing about a subject, beyond the short list
/// the search form drops. Sorted, for the binary search.
const PLAIN_WORDS: &[&str] = &[
    "about", "above", "after", "again", "against", "all", "almost", "also", "although", "always",
    "among", "and", "another", "any", "are", "around", "because", "been", "before", "being",
    "below", "between", "both", "but", "can", "cannot", "chapter", "could", "did", "does", "doing",
    "done", "down", "during", "each", "either", "else", "even", "ever", "every", "example",
    "figure", "first", "for", "from", "further", "get", "given", "had", "has", "have", "having",
    "her", "here", "hers", "him", "his", "how", "however", "into", "its", "itself", "just", "last",
    "less", "like", "made", "make", "many", "may", "might", "more", "most", "much", "must", "near",
    "need", "new", "next", "nor", "not", "now", "off", "often", "once", "one", "only", "other",
    "our", "out", "over", "own", "page", "part", "per", "rather", "same", "section", "see", "seen",
    "shall", "she", "should", "since", "some", "still", "such", "table", "than", "that", "the",
    "their", "them", "then", "there", "these", "they", "this", "those", "though", "three",
    "through", "thus", "too", "two", "under", "until", "upon", "use", "used", "using", "very",
    "was", "way", "were", "what", "when", "where", "whether", "which", "while", "who", "whom",
    "whose", "why", "will", "with", "within", "without", "would", "yet", "you", "your",
];

/// The words of `text` that could name a subject: lowercase, letters
/// only, of a sensible length, and not plain.
pub fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphabetic())
        .filter(|w| {
            let len = w.chars().count();
            (MIN_LEN..=MAX_LEN).contains(&len)
        })
        .map(str::to_lowercase)
        .filter(|w| PLAIN_WORDS.binary_search(&w.as_str()).is_err() && !is_common_word(w))
        .collect()
}

/// For each area, its `take` most distinctive words. `areas[a]` holds one
/// set of words per book of area `a`. A word is scored by the share of
/// the area's books that use it, weighted by how much of the word's use
/// across all areas is here; it must be in at least `min_books` of the
/// area's books. A word and its plain plural don't both appear.
pub fn distinctive(
    areas: &[Vec<BTreeSet<String>>],
    take: usize,
    min_books: usize,
) -> Vec<Vec<String>> {
    // Per area: word → share of the area's books using it.
    let shares: Vec<BTreeMap<&str, (usize, f64)>> = areas
        .iter()
        .map(|books| {
            let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
            for book in books {
                for word in book {
                    *counts.entry(word.as_str()).or_default() += 1;
                }
            }
            counts
                .into_iter()
                .map(|(w, c)| (w, (c, c as f64 / books.len().max(1) as f64)))
                .collect()
        })
        .collect();
    let mut everywhere: BTreeMap<&str, f64> = BTreeMap::new();
    for area in &shares {
        for (word, (_, share)) in area {
            *everywhere.entry(word).or_default() += share;
        }
    }
    let count = areas.len() as f64;
    shares
        .iter()
        .map(|area| {
            let mut scored: Vec<(f64, &str)> = area
                .iter()
                .filter(|(_, (books, _))| *books >= min_books)
                .map(|(word, (_, share))| {
                    let here = share / everywhere[word];
                    (share * (1.0 + count * here).ln(), *word)
                })
                .collect();
            scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(b.1)));
            let mut picked: Vec<String> = Vec::new();
            for (_, word) in scored {
                if picked.len() == take {
                    break;
                }
                let twin = |p: &String| {
                    p.strip_suffix('s') == Some(word) || word.strip_suffix('s') == Some(p.as_str())
                };
                if !picked.iter().any(twin) {
                    picked.push(word.to_owned());
                }
            }
            picked
        })
        .collect()
}

/// The `take` members of `area` nearest its centre, nearest first.
pub fn central(points: &Points, area: &Area, take: usize) -> Vec<u32> {
    let mut by: Vec<(f32, u32)> = area
        .members
        .iter()
        .map(|&m| (dot(points.row(m as usize), &area.centre), m))
        .collect();
    by.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    by.into_iter().take(take).map(|(_, m)| m).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(text: &str) -> BTreeSet<String> {
        words(text).into_iter().collect()
    }

    #[test]
    fn the_plain_words_are_sorted() {
        assert!(PLAIN_WORDS.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn common_words_and_numbers_are_left_out() {
        assert_eq!(
            words("The 3 Gears of 1999: a study, with Figure 12 and H2O; über-long café"),
            ["gears", "study", "über", "long", "café"]
        );
        assert!(words("of it by 42 a").is_empty());
        let long = "a".repeat(30);
        assert!(words(&long).is_empty());
    }

    #[test]
    fn distinctive_words_are_the_planted_ones() {
        let areas = vec![
            vec![
                book("orbit and telescope notes on science"),
                book("the telescope and the orbit of a comet in science"),
                book("an orbit, a telescope: science"),
            ],
            vec![
                book("sourdough bread and yeast for science"),
                book("bread, yeast and ovens as science"),
                book("yeast and bread at home: kitchen science"),
            ],
        ];
        let names = distinctive(&areas, 2, 2);
        assert_eq!(names[0], ["orbit", "telescope"]);
        assert_eq!(names[1], ["bread", "yeast"]);
        // A word every area uses comes after the area's own.
        let more = distinctive(&areas, 4, 2);
        assert_eq!(more[0], ["orbit", "telescope", "science"]);
    }

    #[test]
    fn a_word_from_one_book_is_not_a_name() {
        let areas = vec![
            vec![
                book("gearing gearing gearing gearing harmonic"),
                book("harmonic drives"),
                book("harmonic motion"),
            ],
            vec![book("gardens"), book("gardens and soil")],
        ];
        let names = distinctive(&areas, 3, 2);
        assert_eq!(names[0], ["harmonic"]);
        assert_eq!(names[1], ["gardens"]);
    }

    #[test]
    fn a_word_and_its_plural_do_not_both_appear() {
        let areas = vec![
            vec![book("gear gears ratio"), book("gear gears ratio")],
            vec![book("soil"), book("soil")],
        ];
        assert_eq!(distinctive(&areas, 3, 2)[0], ["gear", "ratio"]);
    }

    #[test]
    fn the_central_members_are_nearest_the_centre() {
        let points = Points::from_rows(
            2,
            &[
                vec![1.0, 0.0],
                vec![0.6, 0.8],
                vec![0.0, 1.0],
                vec![0.8, 0.6],
            ],
        );
        let area = Area {
            members: vec![0, 1, 2, 3],
            centre: vec![1.0, 0.0],
            cohesion: 0.0,
            children: Vec::new(),
        };
        assert_eq!(central(&points, &area, 3), [0, 3, 1]);
    }
}
