//! Suggestions from the library (phase3-spec §7.4, Slice 3.6): documents
//! near what the user is working on, from vectors already stored. No model
//! is called, and nothing is stored: they are computed each time.
//!
//! - **A seed** is a set of documents (an active interest or project, or
//!   the three most recent trails). Its vector is the re-normalised mean of
//!   its documents' vectors, each the mean Related uses.
//! - **For each seed,** the nearest chunks of other documents in the index
//!   (the seed's own filtered inside it), at or above the model's
//!   similarity floor, grouped by document, best first.
//! - **Left out** (by the caller's sets): documents opened in a trail, and
//!   those dismissed for that seed; and here, documents with no available
//!   file.
//! - **Order:** seeds by a hash of the day and the seed, then round-robin,
//!   each document once: the same all day.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use rusqlite::Connection;

use super::related::{Candidate, mean_vector};
use crate::domain::dto::{Suggestion, SuggestionReason};
use crate::services::catalog::writer::CatalogHandle;
use crate::services::embedding::db as edb;
use crate::services::embedding::index::VectorIndex;
use crate::services::embedding::models;
use crate::services::embedding::registry::VectorRegistry;
use crate::services::providers::recommend::query_profile;
use crate::services::search;

/// How many chunks a seed's search looks at.
const NEIGHBOURS: usize = 200;
const OPEN_THREADS: usize = 2;
/// The most a caller may ask for.
pub const MAX_SUGGESTIONS: usize = 6;

/// What a seed brings: why, and its documents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedDocuments {
    pub id: String,
    pub reason: SuggestionReason,
    pub documents: Vec<String>,
}

/// The documents nearest `vector` other than `exclude`, best first, each
/// once with its best chunk.
fn nearest(
    conn: &Connection,
    vectors: &VectorRegistry,
    index: &VectorIndex,
    vector: &[f32],
    exclude: &HashSet<&str>,
    floor: f32,
) -> rusqlite::Result<Vec<Candidate>> {
    let mut map = vectors.chunk_map();
    map.top_up(conn)?;
    let out_numbers: HashSet<u32> = exclude.iter().filter_map(|d| map.number_of(d)).collect();
    let keep = |chunk: u64| {
        map.document_of(chunk)
            .is_some_and(|n| !out_numbers.contains(&n))
    };
    let Ok(found) = index.filtered_search(vector, NEIGHBOURS, keep) else {
        return Ok(Vec::new());
    };
    let ids: Vec<u64> = found
        .iter()
        .filter(|(_, d)| 1.0 - d >= floor)
        .map(|(id, _)| *id)
        .collect();
    let places = edb::chunk_places(conn, &ids)?;
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for (chunk, distance) in &found {
        let Some((document, passage, start, end)) = places.get(chunk) else {
            continue;
        };
        if exclude.contains(document.as_str()) || !seen.insert(document.clone()) {
            continue;
        }
        out.push(Candidate {
            document: document.clone(),
            passage: *passage,
            span: (*start, *end),
            similarity: 1.0 - distance,
        });
    }
    Ok(out)
}

/// The day's place of a seed: the same all day, another the next.
fn day_order(day: &str, seed: &str) -> [u8; 32] {
    *blake3::hash(format!("{day}\u{1f}{seed}").as_bytes()).as_bytes()
}

/// Up to `limit` suggestions for `seeds`. Empty with no meaning index, or
/// while it is still being opened (it is then opened in the background).
#[allow(clippy::too_many_arguments)]
pub fn suggest(
    conn: &Connection,
    vectors: &Arc<VectorRegistry>,
    catalog: &CatalogHandle,
    seeds: &[SeedDocuments],
    opened: &HashSet<String>,
    dismissed: &HashMap<String, HashSet<String>>,
    limit: usize,
    day: &str,
) -> rusqlite::Result<Vec<Suggestion>> {
    let limit = limit.min(MAX_SUGGESTIONS);
    if limit == 0 || models::role(conn)?.is_none() {
        return Ok(Vec::new());
    }
    let Some(model) = models::serving(conn)? else {
        return Ok(Vec::new());
    };
    let Some(index) = vectors.loaded(&model.id) else {
        vectors.open_in_background(catalog, &model.id, OPEN_THREADS);
        return Ok(Vec::new());
    };
    let floor = query_profile(&model.model_name).min_similarity;
    let mut order: Vec<&SeedDocuments> = seeds.iter().filter(|s| !s.documents.is_empty()).collect();
    order.sort_by_key(|s| day_order(day, &s.id));

    let none = HashSet::new();
    // Books in two formats (Slice 4.5.1): a copy kept out reads as the
    // book's copy shown, and is never suggested; a book opened in either
    // format counts as opened.
    let books = crate::services::formats::db::BookMap::load(conn)?;
    let opened: HashSet<String> = opened
        .iter()
        .flat_map(|d| [d.clone(), books.book(d).to_owned()])
        .collect();
    let opened = &opened;
    let mut queues: Vec<(&SeedDocuments, VecDeque<Candidate>)> = Vec::new();
    for seed in order {
        let seeded: Vec<String> = seed
            .documents
            .iter()
            .map(|d| books.book(d).to_owned())
            .collect();
        let means: Vec<Vec<f32>> = seeded
            .iter()
            .map(|d| {
                let own: Vec<Vec<f32>> = edb::document_vectors(conn, &model.id, model.dims, d)?
                    .into_iter()
                    .map(|(_, v)| v)
                    .collect();
                Ok(mean_vector(&own))
            })
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect();
        let Some(vector) = mean_vector(&means) else {
            continue;
        };
        let exclude: HashSet<&str> = seed
            .documents
            .iter()
            .chain(&seeded)
            .map(String::as_str)
            .chain(books.index_of.keys().map(String::as_str))
            .collect();
        let not_for_this = dismissed.get(&seed.id).unwrap_or(&none);
        let found: VecDeque<Candidate> = nearest(conn, vectors, &index, &vector, &exclude, floor)?
            .into_iter()
            .filter(|c| !opened.contains(&c.document) && !not_for_this.contains(&c.document))
            .collect();
        queues.push((seed, found));
    }

    let mut out: Vec<Suggestion> = Vec::new();
    let mut used: HashSet<String> = HashSet::new();
    while out.len() < limit && queues.iter().any(|(_, q)| !q.is_empty()) {
        for (seed, queue) in &mut queues {
            if out.len() >= limit {
                break;
            }
            while let Some(c) = queue.pop_front() {
                if used.contains(&c.document) {
                    continue;
                }
                // Only a document with a file there now.
                let Some(hit) = search::meaning_hit(conn, &c.document, c.passage, c.span)? else {
                    continue;
                };
                used.insert(c.document.clone());
                out.push(Suggestion {
                    hit,
                    reason: seed.reason.clone(),
                    seed_id: seed.id.clone(),
                });
                break;
            }
        }
    }
    Ok(out)
}
