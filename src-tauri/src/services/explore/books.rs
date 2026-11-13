//! One vector per book, read from the catalog: the unit mean of the
//! book's chunk vectors for one embedding model.
//!
//! By default the same sample Related documents averages
//! (`documents::related::sample_indices`), so Atlas has one definition of
//! a book's vector. Reads are one short statement per document, never one
//! long one: beside a running Atlas a long read would hold the catalog's
//! write-ahead log open for its whole length.

use rusqlite::{Connection, OptionalExtension, params};

use super::{Points, dot};
use crate::services::documents::metadata::display_title;
use crate::services::documents::related::sample_indices;
use crate::services::embedding::chunk::{Span, slice};
use crate::services::embedding::db::EmbeddingModel;
use crate::services::embedding::vector::{from_f16_blob, normalize};

/// Which chunks make a book's vector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sampling {
    /// Up to 64, as Related documents: the opening and an even spread.
    Sample,
    /// Every chunk (the spike's first fallback; about ten times the read).
    All,
}

/// How much of the library the vectors cover. Counts only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Coverage {
    /// Documents with text and a file.
    pub with_text: u64,
    /// Of those, by what the model made of them.
    pub done: u64,
    pub empty: u64,
    pub failed: u64,
    pub waiting: u64,
    /// Done documents that gave a vector, and those that didn't.
    pub used: u64,
    pub skipped: u64,
}

/// An embedding model row, as the harness lists it: never its name.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRow {
    #[serde(skip)]
    pub id: String,
    pub status: String,
    pub dims: usize,
    pub done: u64,
}

/// Every model row, oldest first, with how many documents it has done.
pub fn models(conn: &Connection) -> rusqlite::Result<Vec<ModelRow>> {
    conn.prepare(
        "SELECT m.id, m.status, m.dims,
                (SELECT COUNT(*) FROM document_embeddings e
                  WHERE e.model_id = m.id AND e.status = 'done')
         FROM embedding_models m ORDER BY m.created_at, m.id",
    )?
    .query_map([], |r| {
        Ok(ModelRow {
            id: r.get(0)?,
            status: r.get(1)?,
            dims: r.get::<_, i64>(2)? as usize,
            done: r.get::<_, i64>(3)? as u64,
        })
    })?
    .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct Books {
    /// Document ids, ascending: the order of the rows.
    pub ids: Vec<String>,
    /// Each book's chunk count.
    pub chunks: Vec<u32>,
    pub vectors: Points,
}

/// A vector for every document the model has done, with the coverage.
pub fn book_vectors(
    conn: &Connection,
    model: &EmbeddingModel,
    sampling: Sampling,
) -> rusqlite::Result<(Books, Coverage)> {
    let mut coverage = Coverage::default();
    // Documents with text and a file, by the model's status for each.
    let mut stmt = conn.prepare(
        "SELECT d.id, e.status, e.chunk_count FROM documents d
         LEFT JOIN document_embeddings e ON e.document_id = d.id AND e.model_id = ?1
         WHERE d.state = 'done'
           AND EXISTS (SELECT 1 FROM file_instances f WHERE f.document_id = d.id)
         ORDER BY d.id",
    )?;
    let rows: Vec<(String, Option<String>, Option<i64>)> = stmt
        .query_map([&model.id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut books = Books {
        ids: Vec::new(),
        chunks: Vec::new(),
        vectors: Points::new(model.dims),
    };
    for (id, status, chunk_count) in rows {
        coverage.with_text += 1;
        match status.as_deref() {
            Some("done") => coverage.done += 1,
            Some("empty") => {
                coverage.empty += 1;
                continue;
            }
            Some("failed") => {
                coverage.failed += 1;
                continue;
            }
            _ => {
                coverage.waiting += 1;
                continue;
            }
        }
        let count = chunk_count.unwrap_or(0).max(0) as usize;
        match mean(conn, model, &id, count, sampling)? {
            Some(vector) => {
                coverage.used += 1;
                books.ids.push(id);
                books.chunks.push(count as u32);
                books.vectors.push(&vector);
            }
            None => coverage.skipped += 1,
        }
    }
    Ok((books, coverage))
}

/// The unit mean of the document's chunk vectors, or `None` when it has
/// none or they cancel out. A chunk without a vector is passed over.
fn mean(
    conn: &Connection,
    model: &EmbeddingModel,
    document: &str,
    count: usize,
    sampling: Sampling,
) -> rusqlite::Result<Option<Vec<f32>>> {
    let mut sum = vec![0f32; model.dims];
    let mut found = false;
    let mut add = |blob: Vec<u8>| {
        if let Some(v) = from_f16_blob(&blob, model.dims) {
            for (s, x) in sum.iter_mut().zip(&v) {
                *s += x;
            }
            found = true;
        }
    };
    match sampling {
        Sampling::Sample => {
            let mut stmt = conn.prepare_cached(
                "SELECT e.vector FROM chunks c
                 JOIN embeddings e ON e.model_id = ?1 AND e.text_hash = c.text_hash
                 WHERE c.document_id = ?2 AND c.ordinal = ?3",
            )?;
            for ordinal in sample_indices(count) {
                let blob: Option<Vec<u8>> = stmt
                    .query_row(params![model.id, document, ordinal as i64], |r| r.get(0))
                    .optional()?;
                if let Some(blob) = blob {
                    add(blob);
                }
            }
        }
        Sampling::All => {
            let mut stmt = conn.prepare_cached(
                "SELECT e.vector FROM chunks c
                 JOIN embeddings e ON e.model_id = ?1 AND e.text_hash = c.text_hash
                 WHERE c.document_id = ?2 ORDER BY c.ordinal",
            )?;
            let blobs = stmt.query_map(params![model.id, document], |r| r.get::<_, Vec<u8>>(0))?;
            for blob in blobs {
                add(blob?);
            }
        }
    }
    Ok((found && normalize(&mut sum)).then_some(sum))
}

/// What Atlas calls each document, in the order of `ids`.
pub fn titles(conn: &Connection, ids: &[String]) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare_cached(
        "SELECT d.title, d.content_title,
                (SELECT f.display_name FROM file_instances f WHERE f.document_id = d.id
                  ORDER BY f.availability = 'available' DESC, f.display_path LIMIT 1)
         FROM documents d WHERE d.id = ?1",
    )?;
    ids.iter()
        .map(|id| {
            let row: Option<(Option<String>, Option<String>, Option<String>)> = stmt
                .query_row([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .optional()?;
            let (title, content, name) = row.unwrap_or_default();
            Ok(display_title(
                title.as_deref(),
                content.as_deref(),
                name.as_deref().unwrap_or(""),
            )
            .0)
        })
        .collect()
}

/// The text of the `take` sampled chunks of `document` nearest `centre`:
/// what the book says where it is most like its area.
pub fn central_chunks(
    conn: &Connection,
    model: &EmbeddingModel,
    document: &str,
    count: usize,
    centre: &[f32],
    take: usize,
) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare_cached(
        "SELECT e.vector, p.text, c.char_start, c.char_end FROM chunks c
         JOIN embeddings e ON e.model_id = ?1 AND e.text_hash = c.text_hash
         JOIN passages p ON p.id = c.passage_id
         WHERE c.document_id = ?2 AND c.ordinal = ?3",
    )?;
    let mut scored: Vec<(f32, usize, String)> = Vec::new();
    for ordinal in sample_indices(count) {
        let row: Option<(Vec<u8>, String, i64, i64)> = stmt
            .query_row(params![model.id, document, ordinal as i64], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .optional()?;
        let Some((blob, text, start, end)) = row else {
            continue;
        };
        let (Some(vector), Some(text)) = (
            from_f16_blob(&blob, model.dims),
            slice(
                &text,
                Span {
                    start: start.max(0) as usize,
                    end: end.max(0) as usize,
                },
            ),
        ) else {
            continue;
        };
        scored.push((dot(&vector, centre), ordinal, text.to_owned()));
    }
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    Ok(scored.into_iter().take(take).map(|(_, _, t)| t).collect())
}
