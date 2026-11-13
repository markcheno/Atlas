//! Full-text search over the read text (phase1-spec §7.4, ADR 0010).
//!
//! 1. All words: the best [`WINDOW`] passages by bm25, filtered to documents
//!    with a file inside the filters, grouped by document (best passage
//!    kept).
//! 2. Any words: when fewer than one page of documents match all words, the
//!    any-words form fills in after them.
//! 3. Names: Phase 0's folded file-name match. A document matching by name
//!    and text ranks above text-only documents of equal score; name-only
//!    results (including files not read yet) come after text matches.
//! 4. Snippets are made only for the page shown.
//!
//! Runs on the read-only connections, so it never waits for writes.

pub mod hybrid;
pub mod query;
pub mod snippet;

use std::collections::{HashMap, HashSet};

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, params_from_iter};

use std::sync::Arc;

use crate::domain::dto::{
    Availability, FileStorage, FileType, MatchReason, SearchHit, SearchPage, SearchQuery,
    SemanticStatus, SemanticUnavailable, SourceRootStatus,
};
use crate::error::{AtlasError, AtlasResult};
use crate::services::catalog::codec::{from_db, to_db};
use crate::services::catalog::reader::CatalogReader;
use crate::services::catalog::writer::CatalogHandle;
use crate::services::documents::metadata;
use crate::services::formats::{self, db::BookMap};
use crate::services::embedding::query::{NoTarget, QueryEmbedder};
use crate::services::embedding::registry::VectorRegistry;
use crate::services::jobs::db::{ACTIVE_STATES, state_list};
use crate::services::library::RootStatusCache;
use crate::services::library::fold::{fold, like_escape};
use crate::services::providers::secrets::SecretVault;

pub const MAX_PAGE_SIZE: u32 = 50;
/// Best passages considered per query, before grouping (phase1-spec §7.4).
pub const WINDOW: usize = 500;
/// How much a name match strengthens a text match's (negative) bm25 score.
const BOTH_BOOST: f64 = 1.25;

struct Filters {
    /// Conditions on file instance `f`, each starting with " AND ".
    sql: String,
    params: Vec<Value>,
    /// A folder or file type is chosen (not only "available").
    narrowed: bool,
    /// Books in two formats (Slice 4.5.1): their matches are grouped as
    /// one, and the copy kept out has no meaning matches.
    books: BookMap,
}

fn filters(q: &SearchQuery) -> Filters {
    let mut sql = String::new();
    let mut params = Vec::new();
    if !q.include_missing {
        sql.push_str(" AND f.availability = 'available'");
    }
    if let Some(root) = &q.source_root_id {
        sql.push_str(" AND f.source_root_id = ?");
        params.push(Value::Text(root.clone()));
    }
    if let Some(kind) = q.file_type {
        sql.push_str(" AND f.file_type = ?");
        params.push(Value::Text(to_db(&kind)));
    }
    let narrowed = q.source_root_id.is_some() || q.file_type.is_some();
    Filters {
        sql,
        params,
        narrowed,
        books: BookMap::default(),
    }
}

/// [`filters`], with the library's books in two formats.
fn filters_in(conn: &Connection, q: &SearchQuery) -> rusqlite::Result<Filters> {
    let mut f = filters(q);
    f.books = BookMap::load(conn)?;
    Ok(f)
}

#[derive(Debug, Clone)]
struct Entry {
    document: Option<String>,
    /// The book the document belongs to: itself, or for a copy kept out
    /// of a book in two formats, the copy shown (Slice 4.5.1). Matches are
    /// grouped by it.
    book: Option<String>,
    instance: Option<String>,
    passage: Option<i64>,
    /// bm25: lower is better.
    score: f64,
    why: MatchReason,
    all_words: bool,
    /// The FTS expression that matched, for the snippet.
    expr: Option<String>,
    /// A meaning match's chunk, in characters of its passage (Slice 2.4).
    span: Option<(usize, usize)>,
}

/// A document's best passage: (document id, passage id, bm25).
type Passage = (String, i64, f64);

/// The best passages for `expr`, grouped by document in rank order.
fn text_matches(
    conn: &Connection,
    expr: &str,
    f: &Filters,
) -> rusqlite::Result<(Vec<Passage>, bool)> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT p.document_id, p.id, bm25(passages_fts)
         FROM passages_fts JOIN passages p ON p.id = passages_fts.rowid
         WHERE passages_fts MATCH ?
           AND EXISTS (SELECT 1 FROM file_instances f WHERE f.document_id = p.document_id{})
         ORDER BY rank LIMIT {WINDOW}",
        f.sql
    ))?;
    let mut params = vec![Value::Text(expr.to_owned())];
    params.extend(f.params.iter().cloned());
    let rows: Vec<Passage> = stmt
        .query_map(params_from_iter(params.iter()), |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let capped = rows.len() >= WINDOW;
    let mut seen = HashSet::new();
    let grouped = rows
        .into_iter()
        .filter(|(doc, _, _)| seen.insert(f.books.book(doc).to_owned()))
        .collect();
    Ok((grouped, capped))
}

/// Files whose folded path contains every word (Phase 0's name search).
fn name_matches(
    conn: &Connection,
    terms: &[String],
    f: &Filters,
) -> rusqlite::Result<Vec<(String, Option<String>)>> {
    let folded: Vec<String> = terms
        .iter()
        .flat_map(|t| {
            fold(t)
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect();
    if folded.is_empty() {
        return Ok(Vec::new());
    }
    let mut sql = format!(
        "SELECT f.id, f.document_id FROM file_instances f WHERE 1 = 1{}",
        f.sql
    );
    let mut params = f.params.clone();
    for term in &folded {
        sql.push_str(" AND f.search_key LIKE ? ESCAPE '\\'");
        params.push(Value::Text(format!("%{}%", like_escape(term))));
    }
    sql.push_str(&format!(" ORDER BY f.name_key, f.rowid LIMIT {WINDOW}"));
    let mut stmt = conn.prepare(&sql)?;
    stmt.query_map(params_from_iter(params.iter()), |r| {
        Ok((r.get(0)?, r.get(1)?))
    })?
    .collect()
}

fn status(conn: &Connection) -> rusqlite::Result<(bool, u32, u32)> {
    let reading = conn.query_row(
        &format!(
            "SELECT EXISTS (SELECT 1 FROM jobs WHERE kind = 'extract' AND state IN ({}))",
            state_list(ACTIVE_STATES)
        ),
        [],
        |r| r.get(0),
    )?;
    let (read, total): (u32, u32) = conn.query_row(
        "SELECT COALESCE(SUM(extract_state <> 'pending'), 0), COUNT(*)
         FROM file_instances WHERE availability = 'available'",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok((reading, read, total))
}

fn hit(
    conn: &Connection,
    e: &Entry,
    f: &Filters,
    terms: &[String],
) -> rusqlite::Result<Option<SearchHit>> {
    // The file to show: an available copy inside the filters first.
    let (where_sql, key) = match (&e.document, &e.instance) {
        (Some(doc), _) => ("f.document_id = ?", doc.clone()),
        (None, Some(instance)) => ("f.id = ?", instance.clone()),
        (None, None) => return Ok(None),
    };
    let mut params = vec![Value::Text(key)];
    params.extend(f.params.iter().cloned());
    let file = conn
        .query_row(
            &format!(
                "SELECT f.id, f.display_name, f.display_path, f.file_type, f.source_root_id,
                        f.availability, f.storage
                 FROM file_instances f WHERE {where_sql}{}
                 ORDER BY f.availability = 'available' DESC, f.name_key LIMIT 1",
                f.sql
            ),
            params_from_iter(params.iter()),
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                ))
            },
        )
        .optional()?;
    let Some((id, name, path, file_type, root, availability, storage)) = file else {
        return Ok(None);
    };
    let other_format = match &e.document {
        Some(doc) => formats::db::other_format(conn, &f.books, doc, Some(&id))?,
        None => None,
    };
    let (title, content_title, copies) = match &e.document {
        Some(doc) => conn.query_row(
            "SELECT d.title, d.content_title,
                    (SELECT COUNT(*) FROM file_instances f
                     WHERE f.document_id = d.id AND f.availability = 'available')
             FROM documents d WHERE d.id = ?1",
            [doc],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, u32>(2)?,
                ))
            },
        )?,
        None => (None, None, 1),
    };
    let (title, title_source) =
        metadata::display_title(title.as_deref(), content_title.as_deref(), &name);
    let (location, page_no, snippet) = match (e.passage, &e.expr) {
        (Some(passage), Some(expr)) => {
            let (location, page_no): (Option<String>, Option<u32>) = conn.query_row(
                "SELECT location, page_no FROM passages WHERE id = ?1",
                [passage],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let marked: Option<String> = conn
                .query_row(
                    "SELECT snippet(passages_fts, 0, char(57344), char(57345), '…', 24)
                     FROM passages_fts WHERE passages_fts MATCH ?1 AND rowid = ?2",
                    rusqlite::params![expr, passage],
                    |r| r.get(0),
                )
                .optional()?;
            (
                location,
                page_no,
                marked.map(|m| snippet::spans(&m)).unwrap_or_default(),
            )
        }
        (Some(passage), None) if e.span.is_some() => {
            let (location, page_no, text): (Option<String>, Option<u32>, String) = conn.query_row(
                "SELECT location, page_no, text FROM passages WHERE id = ?1",
                [passage],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            let (start, end) = e.span.unwrap_or_default();
            let chunk = crate::services::embedding::chunk::slice(
                &text,
                crate::services::embedding::chunk::Span { start, end },
            )
            .unwrap_or("");
            (location, page_no, snippet::meaning(chunk, terms))
        }
        _ => (None, None, Vec::new()),
    };
    Ok(Some(SearchHit {
        document_id: e.document.clone(),
        file_instance_id: id,
        title,
        title_source,
        display_name: name,
        display_path: path,
        file_type: from_db(&file_type).unwrap_or(FileType::Pdf),
        source_root_id: root,
        availability: if availability == "missing" {
            Availability::Missing
        } else {
            Availability::Available
        },
        storage: if storage == "cloud_only" {
            FileStorage::CloudOnly
        } else {
            FileStorage::Local
        },
        location,
        page_no,
        snippet,
        why: e.why,
        all_words: e.all_words,
        other_locations: copies.saturating_sub(1),
        other_format,
        // The passage the snippet came from, if it came from one.
        passage: e.passage.filter(|_| e.expr.is_some() || e.span.is_some()),
    }))
}

/// Phase 1's ranking: text matches (all words, then any words) by bm25,
/// those that also match by name marked and boosted; and, apart, the
/// documents or files that match by name only.
struct TextRank {
    entries: Vec<Entry>,
    names_only: Vec<Entry>,
    capped: bool,
}

fn rank_text(
    conn: &Connection,
    built: &query::Query,
    f: &Filters,
    page_size: u32,
) -> rusqlite::Result<TextRank> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut by_document: HashMap<String, usize> = HashMap::new();
    let (all, mut capped) = text_matches(conn, &built.all, f)?;
    for (doc, passage, score) in all {
        let book = f.books.book(&doc).to_owned();
        by_document.insert(book.clone(), entries.len());
        entries.push(Entry {
            document: Some(doc),
            book: Some(book),
            instance: None,
            passage: Some(passage),
            score,
            why: MatchReason::Text,
            all_words: true,
            expr: Some(built.all.clone()),
            span: None,
        });
    }
    if entries.len() < page_size as usize
        && let Some(any) = &built.any
    {
        let (more, more_capped) = text_matches(conn, any, f)?;
        capped |= more_capped;
        for (doc, passage, score) in more {
            let book = f.books.book(&doc).to_owned();
            if by_document.contains_key(&book) {
                continue;
            }
            by_document.insert(book.clone(), entries.len());
            entries.push(Entry {
                document: Some(doc),
                book: Some(book),
                instance: None,
                passage: Some(passage),
                score,
                why: MatchReason::Text,
                all_words: false,
                expr: Some(any.clone()),
                span: None,
            });
        }
    }

    let mut names_only: Vec<Entry> = Vec::new();
    let mut seen_names: HashSet<String> = HashSet::new();
    for (instance, document) in name_matches(conn, &query::name_terms(built), f)? {
        let book = document.as_deref().map(|d| f.books.book(d).to_owned());
        match book.as_ref().and_then(|d| by_document.get(d)) {
            Some(&i) => {
                if entries[i].why != MatchReason::Both {
                    entries[i].why = MatchReason::Both;
                    entries[i].score *= BOTH_BOOST;
                }
            }
            None => {
                let key = book.clone().unwrap_or_else(|| instance.clone());
                if seen_names.insert(key) {
                    names_only.push(Entry {
                        document,
                        book,
                        instance: Some(instance),
                        passage: None,
                        score: 0.0,
                        why: MatchReason::Name,
                        all_words: true,
                        expr: None,
                        span: None,
                    });
                }
            }
        }
    }
    // All-words matches, then any-words, by score; names come after.
    entries.sort_by(|a, b| {
        b.all_words
            .cmp(&a.all_words)
            .then(a.score.total_cmp(&b.score))
    });
    Ok(TextRank {
        entries,
        names_only,
        capped,
    })
}

/// The query's words, folded, for marking a meaning match's snippet.
fn snippet_terms(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && !query::is_common_word(w))
        .map(fold)
        .collect()
}

fn empty_page(conn: &Connection, q: &SearchQuery) -> rusqlite::Result<SearchPage> {
    let (reading_text, text_read_files, total_files) = status(conn)?;
    Ok(SearchPage {
        hits: Vec::new(),
        page: q.page,
        page_size: q.page_size.clamp(1, MAX_PAGE_SIZE),
        matched_documents: 0,
        capped: false,
        reading_text,
        text_read_files,
        total_files,
        semantic: Default::default(),
    })
}

/// One page of results. Empty or punctuation-only input gives an empty
/// page, not an error. Text only (Phase 1).
pub fn search(conn: &Connection, q: &SearchQuery) -> rusqlite::Result<SearchPage> {
    search_with(conn, q, None)
}

/// One page of results, with meaning search when `semantic` is given
/// (Slice 2.4, [`hybrid`]). Without it, or when it finds nothing, the page
/// is exactly Phase 1's.
pub fn search_with(
    conn: &Connection,
    q: &SearchQuery,
    semantic: Option<hybrid::Semantic<'_>>,
) -> rusqlite::Result<SearchPage> {
    let mut page = empty_page(conn, q)?;
    let page_size = page.page_size;
    let Some(built) = query::build(&q.text) else {
        return Ok(page);
    };
    let f = filters_in(conn, q)?;
    let ranked = rank_text(conn, &built, &f, page_size)?;
    page.capped = ranked.capped;
    let near = match semantic {
        Some(s) => hybrid::nearest(conn, s, &f)?,
        None => Vec::new(),
    };
    let entries = hybrid::fuse(ranked.entries, ranked.names_only, near);

    page.matched_documents = u32::try_from(entries.len()).unwrap_or(u32::MAX);
    let terms = snippet_terms(&q.text);
    let start = (q.page as usize).saturating_mul(page_size as usize);
    for e in entries.iter().skip(start).take(page_size as usize) {
        if let Some(hit) = hit(conn, e, &f, &terms)? {
            page.hits.push(hit);
        }
    }
    Ok(page)
}

/// The documents meaning search alone finds for `q`, best first (the
/// evaluation's "meaning only" column; Slice 2.4).
pub fn meaning_ranking(
    conn: &Connection,
    q: &SearchQuery,
    semantic: hybrid::Semantic<'_>,
) -> rusqlite::Result<Vec<String>> {
    let f = filters_in(conn, q)?;
    Ok(hybrid::nearest(conn, semantic, &f)?
        .into_iter()
        .map(|n| n.document)
        .collect())
}

/// A document found by meaning, as a result: an available file of it
/// (available copies first), its title and the chunk's location. `None`
/// when no file of it is available (related documents, Slice 2.5).
pub(crate) fn meaning_hit(
    conn: &Connection,
    document: &str,
    passage: i64,
    span: (usize, usize),
) -> rusqlite::Result<Option<SearchHit>> {
    let f = Filters {
        sql: " AND f.availability = 'available'".to_owned(),
        params: Vec::new(),
        narrowed: false,
        books: BookMap::load(conn)?,
    };
    let e = Entry {
        document: Some(document.to_owned()),
        book: Some(document.to_owned()),
        instance: None,
        passage: Some(passage),
        score: 0.0,
        why: MatchReason::Semantic,
        all_words: true,
        expr: None,
        span: Some(span),
    };
    hit(conn, &e, &f, &[])
}

/// The source folders of hits, for marking unreachable ones.
pub(crate) fn hit_roots(
    c: &Connection,
    hits: &[SearchHit],
) -> rusqlite::Result<Vec<(String, Vec<u8>)>> {
    let mut paths: Vec<(String, Vec<u8>)> = Vec::new();
    for hit in hits {
        if paths.iter().any(|(id, _)| *id == hit.source_root_id) {
            continue;
        }
        let bytes: Vec<u8> = c.query_row(
            "SELECT path_bytes FROM source_roots WHERE id = ?1",
            [&hit.source_root_id],
            |r| r.get(0),
        )?;
        paths.push((hit.source_root_id.clone(), bytes));
    }
    Ok(paths)
}

pub(crate) fn mark_unreachable(
    hits: &mut [SearchHit],
    roots: &RootStatusCache,
    paths: &[(String, Vec<u8>)],
) {
    let statuses = roots.statuses(paths);
    for hit in hits {
        let reachable = statuses
            .get(&hit.source_root_id)
            .is_none_or(|s| *s == SourceRootStatus::Available);
        if !reachable && hit.availability == Availability::Available {
            hit.availability = Availability::Unreachable;
        }
    }
}

/// Searches on the read-only pool and marks results whose folder is not
/// connected `unreachable`, as the Library does. Text only (Phase 1).
pub fn run(
    reader: &CatalogReader,
    roots: &RootStatusCache,
    q: &SearchQuery,
) -> AtlasResult<SearchPage> {
    let (mut page, root_paths) = reader
        .read(|c| {
            let page = search(c, q)?;
            let paths = hit_roots(c, &page.hits)?;
            Ok::<_, rusqlite::Error>((page, paths))
        })?
        .map_err(AtlasError::db("search"))?;
    mark_unreachable(&mut page.hits, roots, &root_paths);
    Ok(page)
}

/// What hybrid search needs from the open library and the app.
pub struct MeaningContext<'a> {
    pub vectors: &'a Arc<VectorRegistry>,
    pub catalog: &'a CatalogHandle,
    pub embedder: &'a Arc<QueryEmbedder>,
    pub vault: &'a SecretVault,
    /// The library's privacy setting.
    pub allow_remote: bool,
}

/// Threads for opening an index that search found closed.
const BACKGROUND_OPEN_THREADS: usize = 2;

/// Search with meaning (Slice 2.4): as [`run`], plus meaning search when
/// an embedding model, its index and its service are there, and the
/// page's `semantic` status. Never fails for want of meaning search.
pub fn run_hybrid(
    reader: &CatalogReader,
    roots: &RootStatusCache,
    m: &MeaningContext<'_>,
    q: &SearchQuery,
) -> AtlasResult<SearchPage> {
    let (mut page, root_paths) = reader
        .read(|c| {
            let (page, status) = hybrid_page(c, m, q)?;
            let mut page = page;
            page.semantic = status;
            let paths = hit_roots(c, &page.hits)?;
            Ok::<_, rusqlite::Error>((page, paths))
        })?
        .map_err(AtlasError::db("search"))?;
    mark_unreachable(&mut page.hits, roots, &root_paths);
    Ok(page)
}

fn hybrid_page(
    c: &Connection,
    m: &MeaningContext<'_>,
    q: &SearchQuery,
) -> rusqlite::Result<(SearchPage, SemanticStatus)> {
    with_meaning(c, m, &q.text, |semantic| search_with(c, q, semantic))
}

/// Runs `run` with meaning search when an embedding model, its index and
/// its service are there, else without; and says which.
fn with_meaning<T>(
    c: &Connection,
    m: &MeaningContext<'_>,
    text: &str,
    run: impl FnOnce(Option<hybrid::Semantic<'_>>) -> rusqlite::Result<T>,
) -> rusqlite::Result<(T, SemanticStatus)> {
    let target = match m.embedder.target(c, m.vault, m.allow_remote) {
        Ok(t) => t,
        Err(NoTarget::Off) => return Ok((run(None)?, SemanticStatus::Off)),
        Err(NoTarget::Unavailable(reason)) => {
            return Ok((run(None)?, SemanticStatus::Unavailable { reason }));
        }
    };
    let Some(index) = m.vectors.loaded(&target.model.id) else {
        m.vectors
            .open_in_background(m.catalog, &target.model.id, BACKGROUND_OPEN_THREADS);
        return Ok((
            run(None)?,
            SemanticStatus::Unavailable {
                reason: SemanticUnavailable::Loading,
            },
        ));
    };
    let ready = SemanticStatus::Ready {
        coverage_percent: coverage(c, &target.model.id)?,
    };
    if query::build(text).is_none() {
        return Ok((run(None)?, ready));
    }
    let pending = m.embedder.start(&target, text);
    let failure: std::cell::Cell<Option<SemanticUnavailable>> = std::cell::Cell::new(None);
    let mut map = m.vectors.chunk_map();
    let semantic = hybrid::Semantic {
        vector: Box::new(|| match pending.wait() {
            Ok(v) => Some(v),
            Err(reason) => {
                failure.set(Some(reason));
                None
            }
        }),
        index: &index,
        chunk_map: &mut map,
        min_similarity: target.profile.min_similarity,
        weak_margin: target.profile.weak_margin,
    };
    let out = run(Some(semantic))?;
    let status = match failure.get() {
        Some(reason) => SemanticStatus::Unavailable { reason },
        None => ready,
    };
    Ok((out, status))
}

/// A chunk's place: its passage and character span there.
pub type ChunkAt = (i64, (usize, usize));

/// A document found for Ask Atlas (Slice 2.6): where it matched, by words
/// and by meaning, and the file to show for it.
#[derive(Debug, Clone)]
pub struct Found {
    pub hit: SearchHit,
    /// The best passage containing the words, and the FTS expression.
    pub text: Option<(i64, String)>,
    /// The best chunks by meaning, best first: passage and character span.
    pub meaning: Vec<ChunkAt>,
    /// The file the meaning chunks are in, when it isn't `hit`'s: a book in
    /// two formats whose words matched in the copy kept out (Slice 4.5.1).
    pub meaning_hit: Option<SearchHit>,
}

/// What Ask Atlas retrieval found, best first (public for `just eval-ask`).
pub struct Retrieval {
    pub found: Vec<Found>,
    pub semantic: SemanticStatus,
    /// The best meaning similarity among all documents, if any.
    pub best_similarity: Option<f32>,
    /// The serving model's similarity floor, when meaning search ran.
    pub min_similarity: Option<f32>,
    /// The serving model's weak margin, when meaning search ran.
    pub weak_margin: Option<f32>,
}

/// The `limit` best documents for a question, as hybrid search ranks them,
/// with both their text and meaning matches. Name-only matches have no
/// passage to quote and are left out.
pub fn retrieve(
    c: &Connection,
    m: &MeaningContext<'_>,
    q: &SearchQuery,
    limit: usize,
) -> rusqlite::Result<Retrieval> {
    let floor = std::cell::Cell::new(None);
    let ((found, best), semantic) = with_meaning(c, m, &q.text, |semantic| {
        let Some(built) = query::build(&q.text) else {
            return Ok((Vec::new(), None));
        };
        let f = filters_in(c, q)?;
        let ranked = rank_text(c, &built, &f, limit as u32)?;
        let near = match semantic {
            Some(s) => {
                floor.set(Some((s.min_similarity, s.weak_margin)));
                hybrid::nearest(c, s, &f)?
            }
            None => Vec::new(),
        };
        let best = near.iter().map(|n| n.similarity).reduce(f32::max);
        let meaning: HashMap<String, Vec<ChunkAt>> = near
            .iter()
            .map(|n| {
                let mut chunks = vec![(n.passage, n.span)];
                chunks.extend(n.also.iter().copied());
                (n.document.clone(), chunks)
            })
            .collect();
        let mut found = Vec::new();
        for e in hybrid::fuse(ranked.entries, ranked.names_only, near) {
            if found.len() >= limit {
                break;
            }
            let Some(document) = e.document.clone() else {
                continue;
            };
            let text = match (e.passage, &e.expr) {
                (Some(p), Some(expr)) => Some((p, expr.clone())),
                _ => None,
            };
            let book = e.book.clone().unwrap_or_else(|| document.clone());
            let meaning = meaning.get(&book).cloned().unwrap_or_default();
            if text.is_none() && meaning.is_empty() {
                continue;
            }
            // The meaning chunks are the book's indexed copy's; cite them
            // as that file, not the one the words matched in.
            let meaning_hit = match (book != document, meaning.first()) {
                (true, Some(&(passage, span))) => meaning_hit(c, &book, passage, span)?,
                _ => None,
            };
            if let Some(hit) = hit(c, &e, &f, &[])? {
                found.push(Found {
                    hit,
                    text,
                    meaning,
                    meaning_hit,
                });
            }
        }
        Ok((found, best))
    })?;
    Ok(Retrieval {
        found,
        semantic,
        best_similarity: best,
        min_similarity: floor.get().map(|(f, _)| f),
        weak_margin: floor.get().map(|(_, w)| w),
    })
}

/// The share of documents with text that the model has finished with; a
/// copy kept out of a book in two formats (Slice 4.5.1) counts on neither
/// side.
fn coverage(c: &Connection, model_id: &str) -> rusqlite::Result<u8> {
    let (done, total): (i64, i64) = c.query_row(
        "SELECT (SELECT COUNT(*) FROM document_embeddings de
                 WHERE de.model_id = ?1 AND de.status IN ('done', 'empty')
                   AND NOT EXISTS (SELECT 1 FROM book_pairs bp
                                   WHERE bp.hidden_document_id = de.document_id)),
                (SELECT COUNT(*) FROM documents d WHERE d.state = 'done'
                   AND NOT EXISTS (SELECT 1 FROM book_pairs bp
                                   WHERE bp.hidden_document_id = d.id))",
        [model_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok(if total == 0 {
        100
    } else {
        ((done * 100) / total).clamp(0, 100) as u8
    })
}
