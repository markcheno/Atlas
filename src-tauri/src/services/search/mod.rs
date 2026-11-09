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

pub mod query;
pub mod snippet;

use std::collections::{HashMap, HashSet};

use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, params_from_iter};

use crate::domain::dto::{
    Availability, FileStorage, FileType, MatchReason, SearchHit, SearchPage, SearchQuery,
    SourceRootStatus,
};
use crate::error::{AtlasError, AtlasResult};
use crate::services::catalog::codec::{from_db, to_db};
use crate::services::catalog::reader::CatalogReader;
use crate::services::jobs::db::{ACTIVE_STATES, state_list};
use crate::services::library::RootStatusCache;
use crate::services::library::fold::{fold, like_escape};

pub const MAX_PAGE_SIZE: u32 = 50;
/// Best passages considered per query, before grouping (phase1-spec §7.4).
pub const WINDOW: usize = 500;
/// How much a name match strengthens a text match's (negative) bm25 score.
const BOTH_BOOST: f64 = 1.25;

struct Filters {
    /// Conditions on file instance `f`, each starting with " AND ".
    sql: String,
    params: Vec<Value>,
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
    Filters { sql, params }
}

#[derive(Debug, Clone)]
struct Entry {
    document: Option<String>,
    instance: Option<String>,
    passage: Option<i64>,
    /// bm25: lower is better.
    score: f64,
    why: MatchReason,
    all_words: bool,
    /// The FTS expression that matched, for the snippet.
    expr: Option<String>,
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
        .filter(|(doc, _, _)| seen.insert(doc.clone()))
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

fn hit(conn: &Connection, e: &Entry, f: &Filters) -> rusqlite::Result<Option<SearchHit>> {
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
    let (title, copies) = match &e.document {
        Some(doc) => conn.query_row(
            "SELECT d.title,
                    (SELECT COUNT(*) FROM file_instances f
                     WHERE f.document_id = d.id AND f.availability = 'available')
             FROM documents d WHERE d.id = ?1",
            [doc],
            |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, u32>(1)?)),
        )?,
        None => (None, 1),
    };
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
        _ => (None, None, Vec::new()),
    };
    Ok(Some(SearchHit {
        document_id: e.document.clone(),
        file_instance_id: id,
        title,
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
    }))
}

/// One page of results. Empty or punctuation-only input gives an empty
/// page, not an error.
pub fn search(conn: &Connection, q: &SearchQuery) -> rusqlite::Result<SearchPage> {
    let page_size = q.page_size.clamp(1, MAX_PAGE_SIZE);
    let (reading_text, text_read_files, total_files) = status(conn)?;
    let mut page = SearchPage {
        hits: Vec::new(),
        page: q.page,
        page_size,
        matched_documents: 0,
        capped: false,
        reading_text,
        text_read_files,
        total_files,
    };
    let Some(built) = query::build(&q.text) else {
        return Ok(page);
    };
    let f = filters(q);

    let mut entries: Vec<Entry> = Vec::new();
    let mut by_document: HashMap<String, usize> = HashMap::new();
    let (all, capped) = text_matches(conn, &built.all, &f)?;
    page.capped = capped;
    for (doc, passage, score) in all {
        by_document.insert(doc.clone(), entries.len());
        entries.push(Entry {
            document: Some(doc),
            instance: None,
            passage: Some(passage),
            score,
            why: MatchReason::Text,
            all_words: true,
            expr: Some(built.all.clone()),
        });
    }
    if entries.len() < page_size as usize
        && let Some(any) = &built.any
    {
        let (more, capped) = text_matches(conn, any, &f)?;
        page.capped |= capped;
        for (doc, passage, score) in more {
            if by_document.contains_key(&doc) {
                continue;
            }
            by_document.insert(doc.clone(), entries.len());
            entries.push(Entry {
                document: Some(doc),
                instance: None,
                passage: Some(passage),
                score,
                why: MatchReason::Text,
                all_words: false,
                expr: Some(any.clone()),
            });
        }
    }

    let mut name_only: Vec<Entry> = Vec::new();
    let mut seen_names: HashSet<String> = HashSet::new();
    for (instance, document) in name_matches(conn, &query::name_terms(&built), &f)? {
        match document.as_ref().and_then(|d| by_document.get(d)) {
            Some(&i) => {
                if entries[i].why != MatchReason::Both {
                    entries[i].why = MatchReason::Both;
                    entries[i].score *= BOTH_BOOST;
                }
            }
            None => {
                let key = document.clone().unwrap_or_else(|| instance.clone());
                if seen_names.insert(key) {
                    name_only.push(Entry {
                        document,
                        instance: Some(instance),
                        passage: None,
                        score: 0.0,
                        why: MatchReason::Name,
                        all_words: true,
                        expr: None,
                    });
                }
            }
        }
    }
    // All-words matches, then any-words, by score; then names in name order.
    entries.sort_by(|a, b| {
        b.all_words
            .cmp(&a.all_words)
            .then(a.score.total_cmp(&b.score))
    });
    entries.extend(name_only);

    page.matched_documents = u32::try_from(entries.len()).unwrap_or(u32::MAX);
    let start = (q.page as usize).saturating_mul(page_size as usize);
    for e in entries.iter().skip(start).take(page_size as usize) {
        if let Some(hit) = hit(conn, e, &f)? {
            page.hits.push(hit);
        }
    }
    Ok(page)
}

/// Searches on the read-only pool and marks results whose folder is not
/// connected `unreachable`, as the Library does.
pub fn run(
    reader: &CatalogReader,
    roots: &RootStatusCache,
    q: &SearchQuery,
) -> AtlasResult<SearchPage> {
    let (mut page, root_paths) = reader
        .read(|c| {
            let page = search(c, q)?;
            let mut paths: Vec<(String, Vec<u8>)> = Vec::new();
            for hit in &page.hits {
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
            Ok::<_, rusqlite::Error>((page, paths))
        })?
        .map_err(AtlasError::db("search"))?;
    let statuses = roots.statuses(&root_paths);
    for hit in &mut page.hits {
        let reachable = statuses
            .get(&hit.source_root_id)
            .is_none_or(|s| *s == SourceRootStatus::Available);
        if !reachable && hit.availability == Availability::Available {
            hit.availability = Availability::Unreachable;
        }
    }
    Ok(page)
}
