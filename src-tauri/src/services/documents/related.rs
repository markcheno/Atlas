//! Related documents (phase2-spec §7.5, Slice 2.5): what else in the
//! library is close in meaning to one document.
//!
//! - **The document's vector:** the mean of up to [`SAMPLE`] of its chunk
//!   vectors (all of them, or the first [`HEAD`] and then evenly spaced
//!   ones), re-normalised.
//! - **Search:** the [`NEIGHBOURS`] nearest chunks of other documents (its
//!   own are filtered out inside the index, so they can't take the places),
//!   under the model's similarity floor dropped, grouped by document (best
//!   chunk kept). When that leaves fewer than [`DEFAULT_LIMIT`] documents
//!   (one book's chunks crowding the neighbours), once more at [`WIDER`].
//! - **Never related:** the document itself and its copies (copies are the
//!   same document), and documents with the same text in different bytes
//!   (a re-saved PDF, a repackaged EPUB): [`SAME_TEXT`] of their chunks'
//!   text shared. A document with no available file is left out.
//! - **Nothing is sent anywhere:** only stored vectors are compared, so
//!   related documents work while the AI service is down.
//! - **Cache:** [`CACHE_SIZE`] documents' candidates, keyed by the model,
//!   the index's generation (a save) and its size (an add). Files are
//!   looked up afresh every time, so a file gone missing drops out at once.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension, params};

use crate::domain::dto::{RelatedHit, RelatedPage, RelatedStatus, SearchHit};
use crate::error::{AtlasError, AtlasResult};
use crate::services::catalog::reader::CatalogReader;
use crate::services::catalog::writer::CatalogHandle;
use crate::services::embedding::db as edb;
use crate::services::embedding::index::VectorIndex;
use crate::services::embedding::models;
use crate::services::embedding::registry::VectorRegistry;
use crate::services::embedding::vector::normalize;
use crate::services::library::RootStatusCache;
use crate::services::providers::recommend::query_profile;
use crate::services::search;

/// What the panel asks for, and the most a caller may.
pub const DEFAULT_LIMIT: u32 = 10;
pub const MAX_LIMIT: u32 = 20;
/// Chunk vectors averaged into the document's vector.
pub const SAMPLE: usize = 64;
/// Leading chunks always in the sample (a book's opening says most about
/// it); the rest are spread evenly over what follows.
pub const HEAD: usize = 32;
/// Nearest chunks asked of the index, and the wider second ask.
pub const NEIGHBOURS: usize = 60;
pub const WIDER: usize = 200;
/// Share of chunk text two documents must share to count as one text.
pub const SAME_TEXT: f64 = 0.95;
/// Documents whose candidates are kept.
pub const CACHE_SIZE: usize = 64;
/// Threads for opening an index that related documents found closed.
const BACKGROUND_OPEN_THREADS: usize = 2;

/// Which of `n` chunks go into the document's vector, in order.
pub fn sample_indices(n: usize) -> Vec<usize> {
    if n <= SAMPLE {
        return (0..n).collect();
    }
    let rest = n - HEAD;
    let spread = SAMPLE - HEAD;
    let mut out: Vec<usize> = (0..HEAD).collect();
    out.extend((0..spread).map(|i| HEAD + i * rest / spread));
    out
}

/// The re-normalised mean of the sampled vectors; `None` when there are
/// none or they cancel out.
pub fn mean_vector(vectors: &[Vec<f32>]) -> Option<Vec<f32>> {
    let first = vectors.first()?;
    let mut mean = vec![0.0f32; first.len()];
    for i in sample_indices(vectors.len()) {
        let v = &vectors[i];
        if v.len() != mean.len() {
            return None;
        }
        for (m, x) in mean.iter_mut().zip(v) {
            *m += x;
        }
    }
    normalize(&mut mean).then_some(mean)
}

/// Whether two documents are one text: `shared` distinct chunk texts in
/// both, against the larger document's count of distinct chunk texts.
pub fn same_text(shared: usize, own: usize, theirs: usize) -> bool {
    let larger = own.max(theirs);
    larger > 0 && shared as f64 / larger as f64 >= SAME_TEXT
}

/// A related document before its file is looked up: its best chunk.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub document: String,
    pub passage: i64,
    pub span: (usize, usize),
    pub similarity: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Key {
    document: String,
    model: String,
    generation: i64,
    len: usize,
}

/// The candidates of the documents asked about most recently.
#[derive(Default)]
pub struct RelatedCache {
    entries: VecDeque<(Key, Arc<Vec<Candidate>>)>,
}

impl RelatedCache {
    fn get(&mut self, key: &Key) -> Option<Arc<Vec<Candidate>>> {
        let at = self.entries.iter().position(|(k, _)| k == key)?;
        let entry = self.entries.remove(at)?;
        let found = Arc::clone(&entry.1);
        self.entries.push_back(entry);
        Some(found)
    }

    fn put(&mut self, key: Key, candidates: Arc<Vec<Candidate>>) {
        self.entries
            .retain(|(k, _)| k.document != key.document || k.model != key.model);
        if self.entries.len() >= CACHE_SIZE {
            self.entries.pop_front();
        }
        self.entries.push_back((key, candidates));
    }

    /// Drops a model's entries (retired or failed).
    pub fn forget(&mut self, model_id: &str) {
        self.entries.retain(|(k, _)| k.model != model_id);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Why a document can't be compared, before any vector work.
fn status_of(
    conn: &Connection,
    document_id: &str,
) -> rusqlite::Result<Result<edb::EmbeddingModel, RelatedStatus>> {
    if models::role(conn)?.is_none() {
        return Ok(Err(RelatedStatus::Off));
    }
    let Some(model) = models::serving(conn)? else {
        return Ok(Err(RelatedStatus::NotIndexed));
    };
    let state: Option<String> = conn
        .query_row(
            "SELECT status FROM document_embeddings WHERE document_id = ?1 AND model_id = ?2",
            params![document_id, model.id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(match state.as_deref() {
        Some("done") => Ok(model),
        Some("empty") => Err(RelatedStatus::NoText),
        _ => Err(RelatedStatus::NotIndexed),
    })
}

/// How many distinct chunk texts of `candidate` `document` has too, and
/// how many distinct chunk texts `candidate` has. Distinct, so a passage
/// repeated (a running header) or split differently doesn't count twice.
fn shared_chunks(
    conn: &Connection,
    document: &str,
    candidate: &str,
) -> rusqlite::Result<(usize, usize)> {
    conn.query_row(
        "SELECT COUNT(DISTINCT CASE WHEN c.text_hash IN
                    (SELECT text_hash FROM chunks WHERE document_id = ?1)
                 THEN c.text_hash END),
                COUNT(DISTINCT c.text_hash)
         FROM chunks c WHERE c.document_id = ?2",
        params![document, candidate],
        |r| Ok((r.get::<_, i64>(0)? as usize, r.get::<_, i64>(1)? as usize)),
    )
}

/// How many distinct chunk texts a document has.
fn distinct_chunks(conn: &Connection, document: &str) -> rusqlite::Result<usize> {
    conn.query_row(
        "SELECT COUNT(DISTINCT text_hash) FROM chunks WHERE document_id = ?1",
        [document],
        |r| Ok(r.get::<_, i64>(0)? as usize),
    )
}

/// One document's comparison.
struct Ask<'a> {
    document_id: &'a str,
    vector: &'a [f32],
    own_chunks: usize,
    min_similarity: f32,
}

/// The nearest other documents to the document's vector, best first, with
/// same-text documents left out: the same whatever limit the caller asks,
/// so the cache serves every limit. `None` when the index can't be
/// searched.
fn candidates(
    conn: &Connection,
    vectors: &VectorRegistry,
    index: &VectorIndex,
    ask: &Ask<'_>,
) -> rusqlite::Result<Option<Vec<Candidate>>> {
    let mut map = vectors.chunk_map();
    map.top_up(conn)?;
    let own = map.number_of(ask.document_id);
    let keep = |chunk: u64| {
        let doc = map.document_of(chunk);
        doc.is_some() && doc != own
    };
    let mut k = NEIGHBOURS;
    loop {
        let Ok(found) = index.filtered_search(ask.vector, k, keep) else {
            return Ok(None);
        };
        let ids: Vec<u64> = found
            .iter()
            .filter(|(_, d)| 1.0 - d >= ask.min_similarity)
            .map(|(id, _)| *id)
            .collect();
        let places = edb::chunk_places(conn, &ids)?;
        let mut out: Vec<Candidate> = Vec::new();
        let mut seen: HashSet<&str> = HashSet::new();
        for (chunk, distance) in &found {
            let Some((document, passage, start, end)) = places.get(chunk) else {
                continue;
            };
            if document == ask.document_id || !seen.insert(document.as_str()) {
                continue;
            }
            let (shared, theirs) = shared_chunks(conn, ask.document_id, document)?;
            if same_text(shared, ask.own_chunks, theirs) {
                continue;
            }
            out.push(Candidate {
                document: document.clone(),
                passage: *passage,
                span: (*start, *end),
                similarity: 1.0 - distance,
            });
        }
        if out.len() >= DEFAULT_LIMIT as usize || k >= WIDER || index.len() <= k {
            return Ok(Some(out));
        }
        k = WIDER;
    }
}

/// The status, and the related documents as search hits (best first).
fn find(
    conn: &Connection,
    vectors: &Arc<VectorRegistry>,
    catalog: &CatalogHandle,
    document_id: &str,
    limit: u32,
) -> rusqlite::Result<(RelatedStatus, Vec<SearchHit>)> {
    let limit = limit.clamp(1, MAX_LIMIT) as usize;
    let model = match status_of(conn, document_id)? {
        Ok(model) => model,
        Err(status) => return Ok((status, Vec::new())),
    };
    let Some(index) = vectors.loaded(&model.id) else {
        vectors.open_in_background(catalog, &model.id, BACKGROUND_OPEN_THREADS);
        return Ok((RelatedStatus::Loading, Vec::new()));
    };
    let key = Key {
        document: document_id.to_owned(),
        model: model.id.clone(),
        generation: index.generation(),
        len: index.len(),
    };
    let cached = vectors.related_cache().get(&key);
    let found = match cached {
        Some(found) => found,
        None => {
            let own = edb::document_vectors(conn, &model.id, model.dims, document_id)?;
            let own: Vec<Vec<f32>> = own.into_iter().map(|(_, v)| v).collect();
            let Some(vector) = mean_vector(&own) else {
                return Ok((RelatedStatus::NotIndexed, Vec::new()));
            };
            let ask = Ask {
                document_id,
                vector: &vector,
                own_chunks: distinct_chunks(conn, document_id)?,
                min_similarity: query_profile(&model.model_name).min_similarity,
            };
            let Some(found) = candidates(conn, vectors, &index, &ask)? else {
                return Ok((RelatedStatus::Failed, Vec::new()));
            };
            let found = Arc::new(found);
            vectors.related_cache().put(key, Arc::clone(&found));
            found
        }
    };
    let mut hits = Vec::new();
    for c in found.iter() {
        if hits.len() >= limit {
            break;
        }
        if let Some(h) = search::meaning_hit(conn, &c.document, c.passage, c.span)? {
            hits.push(h);
        }
    }
    Ok((RelatedStatus::Ready, hits))
}

fn page(status: RelatedStatus, hits: Vec<SearchHit>) -> RelatedPage {
    RelatedPage {
        status,
        hits: hits
            .into_iter()
            .filter_map(|h| {
                Some(RelatedHit {
                    document_id: h.document_id?,
                    file_instance_id: h.file_instance_id,
                    title: h.title,
                    title_source: h.title_source,
                    display_name: h.display_name,
                    display_path: h.display_path,
                    file_type: h.file_type,
                    source_root_id: h.source_root_id,
                    availability: h.availability,
                    storage: h.storage,
                    location: h.location,
                    page_no: h.page_no,
                })
            })
            .collect(),
    }
}

/// The documents related to `document_id`, best first, at most `limit`
/// (clamped to 1..=[`MAX_LIMIT`]), without folder checks.
pub fn related(
    conn: &Connection,
    vectors: &Arc<VectorRegistry>,
    catalog: &CatalogHandle,
    document_id: &str,
    limit: u32,
) -> rusqlite::Result<RelatedPage> {
    let (status, hits) = find(conn, vectors, catalog, document_id, limit)?;
    Ok(page(status, hits))
}

/// On the read-only pool; files whose folder isn't connected are marked
/// `unreachable`, as everywhere else. A catalog that can't be read is an
/// error; an index that can't be searched is [`RelatedStatus::Failed`].
pub fn run(
    reader: &CatalogReader,
    roots: &RootStatusCache,
    vectors: &Arc<VectorRegistry>,
    catalog: &CatalogHandle,
    document_id: &str,
    limit: u32,
) -> AtlasResult<RelatedPage> {
    let (status, mut hits, paths) = reader
        .read(|c| {
            let (status, hits) = find(c, vectors, catalog, document_id, limit)?;
            let paths = search::hit_roots(c, &hits)?;
            Ok::<_, rusqlite::Error>((status, hits, paths))
        })?
        .map_err(AtlasError::db("related documents"))?;
    search::mark_unreachable(&mut hits, roots, &paths);
    Ok(page(status, hits))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_document_uses_every_chunk() {
        assert_eq!(sample_indices(0), Vec::<usize>::new());
        assert_eq!(sample_indices(5), vec![0, 1, 2, 3, 4]);
        assert_eq!(sample_indices(SAMPLE), (0..SAMPLE).collect::<Vec<_>>());
    }

    #[test]
    fn a_long_document_uses_its_opening_and_evenly_spaced_chunks() {
        let n = 1000;
        let s = sample_indices(n);
        assert_eq!(s.len(), SAMPLE);
        assert_eq!(&s[..HEAD], &(0..HEAD).collect::<Vec<_>>()[..]);
        assert!(s.windows(2).all(|w| w[0] < w[1]), "in order, no repeats");
        assert!(*s.last().unwrap() < n);
        assert!(*s.last().unwrap() > n * 9 / 10, "reaches the end: {s:?}");
        // Just over the sample: still distinct.
        let s = sample_indices(SAMPLE + 1);
        assert_eq!(s.len(), SAMPLE);
        assert!(s.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn the_mean_is_unit_length() {
        let v = mean_vector(&[vec![1.0, 0.0], vec![0.0, 1.0]]).unwrap();
        let len = (v[0] * v[0] + v[1] * v[1]).sqrt();
        assert!((len - 1.0).abs() < 1e-6);
        assert!((v[0] - v[1]).abs() < 1e-6);
        assert_eq!(mean_vector(&[]), None);
        assert_eq!(mean_vector(&[vec![1.0, 0.0], vec![-1.0, 0.0]]), None);
        assert_eq!(mean_vector(&[vec![1.0, 0.0], vec![1.0]]), None);
    }

    #[test]
    fn same_text_needs_nearly_all_chunks_of_the_larger_document() {
        assert!(same_text(100, 100, 100));
        assert!(same_text(95, 100, 96), "an extra copyright page");
        assert!(!same_text(94, 100, 94), "under the share");
        assert!(
            !same_text(10, 100, 10),
            "one chapter of the book stays related"
        );
        assert!(
            !same_text(10, 10, 100),
            "a book containing this chapter stays related"
        );
        assert!(!same_text(0, 0, 0));
    }

    fn key(doc: &str, generation: i64) -> Key {
        Key {
            document: doc.into(),
            model: "m".into(),
            generation,
            len: 10,
        }
    }

    #[test]
    fn the_cache_keeps_the_most_recent_and_follows_the_index() {
        let mut cache = RelatedCache::default();
        let found = Arc::new(Vec::new());
        for i in 0..CACHE_SIZE {
            cache.put(key(&format!("d{i}"), 1), Arc::clone(&found));
        }
        assert!(
            cache.get(&key("d0", 1)).is_some(),
            "d0 is now the most recent"
        );
        cache.put(key("new", 1), Arc::clone(&found));
        assert_eq!(cache.len(), CACHE_SIZE);
        assert!(cache.get(&key("d1", 1)).is_none(), "the least recent went");
        assert!(cache.get(&key("d0", 1)).is_some());
        assert!(cache.get(&key("d0", 2)).is_none(), "a save makes it stale");
        cache.put(key("d0", 2), Arc::clone(&found));
        assert_eq!(cache.len(), CACHE_SIZE, "replaced, not added");
        cache.forget("m");
        assert!(cache.is_empty());
    }
}
