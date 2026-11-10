//! Hybrid search (phase2-spec §7.4, ADR 0014): Phase 1's text ranking
//! fused with meaning search over the vector index.
//!
//! - **Meaning pass:** the query's vector against the index, the 200
//!   nearest chunks, kept only when their document has a file inside the
//!   filters and their similarity clears the model's floor, grouped by
//!   document (best chunk kept).
//! - **Fusion:** reciprocal rank fusion, k = 60, over the two document
//!   lists; a name match still strengthens a result (×1.25). At most
//!   [`MAX_MEANING_ONLY`] documents found by meaning alone take part, so a
//!   query that matches nothing still finds next to nothing.
//! - **Without a meaning list** (meaning search off, failed, or finding
//!   nothing) the order is exactly Phase 1's.

use std::collections::{HashMap, HashSet};

use rusqlite::types::Value;
use rusqlite::{Connection, params_from_iter};

use super::{Entry, Filters};
use crate::domain::dto::MatchReason;
use crate::services::embedding::chunk_map::ChunkMap;
use crate::services::embedding::db as edb;
use crate::services::embedding::index::VectorIndex;

/// Nearest chunks asked of the index.
pub const NEIGHBOURS: usize = 200;
/// Reciprocal rank fusion's constant.
pub const RRF_K: f64 = 60.0;
/// Documents found by meaning alone that take part in a result.
pub const MAX_MEANING_ONLY: usize = 20;
/// How much a name match strengthens a fused score.
const NAME_BOOST: f64 = 1.25;

/// What meaning search needs for one query.
pub struct Semantic<'a> {
    /// The query's unit vector (with the model's query prefix).
    pub vector: &'a [f32],
    pub index: &'a VectorIndex,
    pub chunk_map: &'a mut ChunkMap,
    /// Cosine similarity below which a chunk isn't a meaning match.
    pub min_similarity: f32,
}

/// One document found by meaning: its best chunk.
#[derive(Debug, Clone)]
pub struct Near {
    pub document: String,
    pub passage: i64,
    pub span: (usize, usize),
    pub similarity: f32,
}

/// The documents nearest the query, best first.
pub(super) fn nearest(
    conn: &Connection,
    s: Semantic<'_>,
    f: &Filters,
) -> rusqlite::Result<Vec<Near>> {
    // The documents the filters allow, as the chunk map numbers them.
    s.chunk_map.top_up(conn)?;
    let mut stmt = conn.prepare(&format!(
        "SELECT DISTINCT f.document_id FROM file_instances f
         WHERE f.document_id IS NOT NULL{}",
        f.sql
    ))?;
    let params: Vec<Value> = f.params.clone();
    let allowed: HashSet<u32> = stmt
        .query_map(params_from_iter(params.iter()), |r| r.get::<_, String>(0))?
        .filter_map(|d| d.ok().and_then(|d| s.chunk_map.number_of(&d)))
        .collect();
    if allowed.is_empty() {
        return Ok(Vec::new());
    }
    let map = &*s.chunk_map;
    let keep = |chunk: u64| map.document_of(chunk).is_some_and(|d| allowed.contains(&d));
    let found = if f.narrowed {
        s.index.filtered_search(s.vector, NEIGHBOURS, keep)
    } else {
        s.index.search(s.vector, NEIGHBOURS)
    };
    let Ok(found) = found else {
        return Ok(Vec::new());
    };
    let ids: Vec<u64> = found
        .iter()
        .filter(|(k, d)| 1.0 - d >= s.min_similarity && keep(*k))
        .map(|(k, _)| *k)
        .collect();
    let places = edb::chunk_places(conn, &ids)?;
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (chunk, distance) in found {
        let Some((document, passage, start, end)) = places.get(&chunk) else {
            continue;
        };
        if seen.insert(document.clone()) {
            out.push(Near {
                document: document.clone(),
                passage: *passage,
                span: (*start, *end),
                similarity: 1.0 - distance,
            });
        }
    }
    Ok(out)
}

fn rrf(rank: usize) -> f64 {
    1.0 / (RRF_K + rank as f64 + 1.0)
}

/// Fuses Phase 1's ranked text matches with the meaning list. With an
/// empty meaning list, Phase 1's order exactly: text matches, then name
/// matches.
pub(super) fn fuse(text: Vec<Entry>, mut names_only: Vec<Entry>, near: Vec<Near>) -> Vec<Entry> {
    if near.is_empty() {
        let mut out = text;
        out.extend(names_only);
        return out;
    }
    let near_rank: HashMap<&str, usize> = near
        .iter()
        .enumerate()
        .map(|(i, n)| (n.document.as_str(), i))
        .collect();
    let in_text: HashSet<String> = text.iter().filter_map(|e| e.document.clone()).collect();

    // (fused score, text rank, meaning rank, entry)
    let mut scored: Vec<(f64, usize, usize, Entry)> = Vec::new();
    for (t, mut e) in text.into_iter().enumerate() {
        let m = e
            .document
            .as_deref()
            .and_then(|d| near_rank.get(d).copied());
        let mut score = rrf(t) + m.map_or(0.0, rrf);
        if e.why == MatchReason::Both {
            score *= NAME_BOOST;
        }
        if m.is_some() {
            e.why = MatchReason::TextSemantic;
            e.all_words = true;
        }
        scored.push((score, t, m.unwrap_or(usize::MAX), e));
    }
    let mut taken = 0;
    for (m, n) in near.iter().enumerate() {
        if in_text.contains(&n.document) {
            continue;
        }
        if taken == MAX_MEANING_ONLY {
            break;
        }
        taken += 1;
        let mut score = rrf(m);
        // A file that also matches by name: one result, strengthened.
        let named = names_only
            .iter()
            .position(|e| e.document.as_deref() == Some(n.document.as_str()));
        if let Some(i) = named {
            names_only.remove(i);
            score *= NAME_BOOST;
        }
        scored.push((
            score,
            usize::MAX,
            m,
            Entry {
                document: Some(n.document.clone()),
                instance: None,
                passage: Some(n.passage),
                score: 0.0,
                why: MatchReason::Semantic,
                all_words: true,
                expr: None,
                span: Some(n.span),
            },
        ));
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let mut out: Vec<Entry> = scored.into_iter().map(|(_, _, _, e)| e).collect();
    out.extend(names_only);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(doc: &str, why: MatchReason) -> Entry {
        Entry {
            document: Some(doc.into()),
            instance: None,
            passage: Some(1),
            score: -1.0,
            why,
            all_words: true,
            expr: Some("x".into()),
            span: None,
        }
    }

    fn near(doc: &str) -> Near {
        Near {
            document: doc.into(),
            passage: 2,
            span: (0, 10),
            similarity: 0.9,
        }
    }

    fn order(entries: &[Entry]) -> Vec<(String, MatchReason)> {
        entries
            .iter()
            .map(|e| (e.document.clone().unwrap_or_default(), e.why))
            .collect()
    }

    #[test]
    fn without_meaning_the_order_is_phase_1s() {
        let names = vec![Entry {
            why: MatchReason::Name,
            ..text("n", MatchReason::Name)
        }];
        let out = fuse(
            vec![text("a", MatchReason::Text), text("b", MatchReason::Both)],
            names,
            Vec::new(),
        );
        assert_eq!(
            order(&out),
            [
                ("a".into(), MatchReason::Text),
                ("b".into(), MatchReason::Both),
                ("n".into(), MatchReason::Name)
            ]
        );
    }

    #[test]
    fn found_both_ways_ranks_first_and_meaning_only_joins_in() {
        let out = fuse(
            vec![text("a", MatchReason::Text), text("b", MatchReason::Text)],
            Vec::new(),
            vec![near("b"), near("c")],
        );
        assert_eq!(
            order(&out),
            [
                ("b".into(), MatchReason::TextSemantic),
                ("a".into(), MatchReason::Text),
                ("c".into(), MatchReason::Semantic)
            ]
        );
        assert_eq!(out[2].span, Some((0, 10)));
        assert!(
            out[2].all_words,
            "a meaning match isn't 'some of your words'"
        );
    }

    #[test]
    fn meaning_only_is_capped_and_a_name_match_merges() {
        let many: Vec<Near> = (0..40).map(|i| near(&format!("m{i}"))).collect();
        let names = vec![Entry {
            why: MatchReason::Name,
            ..text("m3", MatchReason::Name)
        }];
        let out = fuse(Vec::new(), names, many);
        assert_eq!(out.len(), MAX_MEANING_ONLY);
        assert_eq!(
            out.iter()
                .filter(|e| e.document.as_deref() == Some("m3"))
                .count(),
            1
        );
        assert_eq!(
            out[0].document.as_deref(),
            Some("m3"),
            "the name match is strengthened"
        );
    }
}
