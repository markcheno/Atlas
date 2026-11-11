//! Hybrid search (phase2-spec §7.4, ADR 0014): Phase 1's text ranking
//! fused with meaning search over the vector index.
//!
//! - **Meaning pass:** the query's vector against the index, the 200
//!   nearest chunks, kept only when their document has a file inside the
//!   filters and their similarity clears the model's floor, grouped by
//!   document (best chunk first, with up to [`ALSO`] more for Ask Atlas).
//! - **Fusion:** reciprocal rank fusion, k = 60, over the two document
//!   lists, in two tiers. Documents containing every word come first, fused
//!   with the meaning list among themselves; then documents with some of
//!   the words and those found by meaning alone, fused. A document with
//!   every word is the strongest evidence a search has: fused with partial
//!   matches, it fell behind them on Phase 1's question set (Slice 2.4). A
//!   name match still strengthens a result (×1.25). **The best meaning
//!   match counts twice** (Slice 3.1.1): as if the words agreed, so the
//!   document meaning ranks first leads the second tier even when its words
//!   don't match. Without it, any partial text match that was also near in
//!   meaning outranked it: v06, v14 and v38 of the vague set ranked 7th and
//!   8th by hybrid though meaning alone ranked them first. At most
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
/// More chunks kept per document, after its best, for Ask Atlas to choose
/// a second passage from (a neighbour of the best one won't do).
pub const ALSO: usize = 4;
/// How much a name match strengthens a fused score.
const NAME_BOOST: f64 = 1.25;

/// What meaning search needs for one query.
pub struct Semantic<'a> {
    /// The query's unit vector (with the model's query prefix), asked for
    /// after the text pass, so embedding the query overlaps it. `None`
    /// when there is none (the service failed or was too slow).
    pub vector: Box<dyn FnOnce() -> Option<Vec<f32>> + 'a>,
    pub index: &'a VectorIndex,
    pub chunk_map: &'a mut ChunkMap,
    /// Cosine similarity below which a chunk isn't a meaning match.
    pub min_similarity: f32,
    /// How far above the floor the best match must be for Ask Atlas's
    /// retrieval not to count as weak: the model's own (Slice 3.1).
    pub weak_margin: f32,
}

/// One document found by meaning: its best chunk.
#[derive(Debug, Clone)]
pub struct Near {
    pub document: String,
    pub passage: i64,
    pub span: (usize, usize),
    pub similarity: f32,
    /// Its next-best chunks among the nearest, best first: passage and span.
    pub also: Vec<super::ChunkAt>,
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
    let Some(vector) = (s.vector)() else {
        return Ok(Vec::new());
    };
    let map = &*s.chunk_map;
    let keep = |chunk: u64| map.document_of(chunk).is_some_and(|d| allowed.contains(&d));
    let found = if f.narrowed {
        s.index.filtered_search(&vector, NEIGHBOURS, keep)
    } else {
        s.index.search(&vector, NEIGHBOURS)
    };
    let found = match found {
        Ok(found) => found,
        Err(err) => {
            if crate::infrastructure::e2e::trace_meaning() {
                eprintln!("e2e meaning: the index search failed: {err}");
            }
            return Ok(Vec::new());
        }
    };
    if crate::infrastructure::e2e::trace_meaning() {
        let best = found.iter().map(|(_, d)| 1.0 - d).fold(f32::MIN, f32::max);
        let above = found
            .iter()
            .filter(|(_, d)| 1.0 - d >= s.min_similarity)
            .count();
        let kept = found
            .iter()
            .filter(|(k, d)| 1.0 - d >= s.min_similarity && keep(*k))
            .count();
        let exact: Vec<(u64, f32)> = s
            .index
            .exact_search(&vector, 3)
            .map(|v| v.into_iter().map(|(k, d)| (k, 1.0 - d)).collect())
            .unwrap_or_default();
        eprintln!("e2e meaning: an exact scan's best: {exact:?}");
        eprintln!(
            "e2e meaning: query norm {:.3}, dims {}, index size {}, allowed documents {}, found {}, best similarity {best:.3}, above the floor {above}, kept {kept}, first keys {:?}",
            vector.iter().map(|x| x * x).sum::<f32>().sqrt(),
            vector.len(),
            s.index.len(),
            allowed.len(),
            found.len(),
            found
                .iter()
                .take(3)
                .map(|(k, d)| (*k, 1.0 - d))
                .collect::<Vec<_>>(),
        );
    }
    let ids: Vec<u64> = found
        .iter()
        .filter(|(k, d)| 1.0 - d >= s.min_similarity && keep(*k))
        .map(|(k, _)| *k)
        .collect();
    let places = edb::chunk_places(conn, &ids)?;
    let mut at: HashMap<String, usize> = HashMap::new();
    let mut out: Vec<Near> = Vec::new();
    for (chunk, distance) in found {
        let Some((document, passage, start, end)) = places.get(&chunk) else {
            continue;
        };
        match at.get(document) {
            Some(&i) => {
                if out[i].also.len() < ALSO {
                    out[i].also.push((*passage, (*start, *end)));
                }
            }
            None => {
                at.insert(document.clone(), out.len());
                out.push(Near {
                    document: document.clone(),
                    passage: *passage,
                    span: (*start, *end),
                    similarity: 1.0 - distance,
                    also: Vec::new(),
                });
            }
        }
    }
    Ok(out)
}

fn rrf(rank: usize) -> f64 {
    1.0 / (RRF_K + rank as f64 + 1.0)
}

/// A document's share from the meaning list: its rank's, twice for the
/// best match (the module docs).
fn meaning_rrf(rank: usize) -> f64 {
    if rank == 0 { 2.0 * rrf(0) } else { rrf(rank) }
}

/// Fuses Phase 1's ranked text matches with the meaning list, in two
/// tiers: documents with every word, then the rest. With an empty meaning
/// list, Phase 1's order exactly: text matches, then name matches.
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

    struct Scored {
        /// 0: every word matched; 1: some words, or meaning alone.
        tier: u8,
        score: f64,
        text_rank: usize,
        meaning_rank: usize,
        entry: Entry,
    }
    let mut scored: Vec<Scored> = Vec::new();
    for (t, mut e) in text.into_iter().enumerate() {
        let m = e
            .document
            .as_deref()
            .and_then(|d| near_rank.get(d).copied());
        let mut score = rrf(t) + m.map_or(0.0, meaning_rrf);
        if e.why == MatchReason::Both {
            score *= NAME_BOOST;
        }
        let tier = u8::from(!e.all_words);
        if m.is_some() {
            e.why = MatchReason::TextSemantic;
            e.all_words = true;
        }
        scored.push(Scored {
            tier,
            score,
            text_rank: t,
            meaning_rank: m.unwrap_or(usize::MAX),
            entry: e,
        });
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
        let mut score = meaning_rrf(m);
        // A file that also matches by name: one result, strengthened.
        let named = names_only
            .iter()
            .position(|e| e.document.as_deref() == Some(n.document.as_str()));
        if let Some(i) = named {
            names_only.remove(i);
            score *= NAME_BOOST;
        }
        scored.push(Scored {
            tier: 1,
            score,
            text_rank: usize::MAX,
            meaning_rank: m,
            entry: Entry {
                document: Some(n.document.clone()),
                instance: None,
                passage: Some(n.passage),
                score: 0.0,
                why: MatchReason::Semantic,
                all_words: true,
                expr: None,
                span: Some(n.span),
            },
        });
    }
    scored.sort_by(|a, b| {
        a.tier
            .cmp(&b.tier)
            .then(b.score.total_cmp(&a.score))
            .then(a.text_rank.cmp(&b.text_rank))
            .then(a.meaning_rank.cmp(&b.meaning_rank))
    });
    let mut out: Vec<Entry> = scored.into_iter().map(|s| s.entry).collect();
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
            also: Vec::new(),
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
            Some("m0"),
            "the best meaning match counts twice"
        );
        assert_eq!(
            out[1].document.as_deref(),
            Some("m3"),
            "the name match is strengthened above its neighbours"
        );
    }

    /// Slice 3.1.1: the document meaning ranks first leads the second tier
    /// though its words don't match, ahead of partial text matches that are
    /// also near in meaning; documents with every word still come first.
    #[test]
    fn the_best_meaning_match_leads_the_partial_matches() {
        let partial = |d: &str| Entry {
            all_words: false,
            ..text(d, MatchReason::Text)
        };
        let out = fuse(
            vec![
                text("every", MatchReason::Text),
                partial("p0"),
                partial("p1"),
                partial("p2"),
            ],
            Vec::new(),
            vec![near("raven"), near("p0"), near("p1"), near("p2")],
        );
        assert_eq!(
            order(&out),
            [
                ("every".into(), MatchReason::Text),
                ("raven".into(), MatchReason::Semantic),
                ("p0".into(), MatchReason::TextSemantic),
                ("p1".into(), MatchReason::TextSemantic),
                ("p2".into(), MatchReason::TextSemantic),
            ]
        );
        // Found both ways, the best meaning match leads as before.
        let out = fuse(
            vec![partial("a"), partial("b")],
            Vec::new(),
            vec![near("b"), near("a")],
        );
        assert_eq!(out[0].document.as_deref(), Some("b"));
    }
}
