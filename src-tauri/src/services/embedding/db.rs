//! Catalog rows for embeddings (migration 0012, ADR 0013): models, chunks,
//! content-addressed vectors, per-document state and tombstones.
//!
//! Writes take the writer's connection (inside the caller's transaction
//! where noted); reads take any connection.

use std::collections::HashSet;

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::chunk::{self, CHUNKER_VERSION, Span};
use super::vector::from_f16_blob;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingModel {
    pub id: String,
    pub provider_profile_id: Option<String>,
    pub model_name: String,
    pub dims: usize,
    pub chunker_version: i64,
    pub query_prefix: Option<String>,
    pub status: String,
    pub index_generation: i64,
    pub index_blake3: Option<[u8; 32]>,
    pub indexed_through_seq: i64,
    pub vector_count: u64,
    pub tombstones_through: i64,
}

/// Adds a model row in `building`. It starts after every existing
/// tombstone: its index is built from the chunks as they are now.
pub fn create_model(
    conn: &Connection,
    provider_profile_id: Option<&str>,
    model_name: &str,
    dims: usize,
    query_prefix: Option<&str>,
    now: &str,
) -> rusqlite::Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO embedding_models (id, provider_profile_id, model_name, dims, quantization,
             chunker_version, query_prefix, status, tombstones_through, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, 'f16', ?5, ?6, 'building',
                 (SELECT COALESCE(MAX(seq), 0) FROM vector_tombstones), ?7, ?7)",
        params![
            id,
            provider_profile_id,
            model_name,
            dims as i64,
            CHUNKER_VERSION,
            query_prefix,
            now
        ],
    )?;
    Ok(id)
}

pub fn get_model(conn: &Connection, id: &str) -> rusqlite::Result<Option<EmbeddingModel>> {
    conn.query_row(
        "SELECT id, provider_profile_id, model_name, dims, chunker_version, query_prefix, status,
                index_generation, index_blake3, indexed_through_seq, vector_count,
                tombstones_through
         FROM embedding_models WHERE id = ?1",
        [id],
        |r| {
            let hash: Option<Vec<u8>> = r.get(8)?;
            Ok(EmbeddingModel {
                id: r.get(0)?,
                provider_profile_id: r.get(1)?,
                model_name: r.get(2)?,
                dims: r.get::<_, i64>(3)? as usize,
                chunker_version: r.get(4)?,
                query_prefix: r.get(5)?,
                status: r.get(6)?,
                index_generation: r.get(7)?,
                index_blake3: hash.and_then(|h| h.try_into().ok()),
                indexed_through_seq: r.get(9)?,
                vector_count: r.get::<_, i64>(10)? as u64,
                tombstones_through: r.get(11)?,
            })
        },
    )
    .optional()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassageText {
    pub id: i64,
    pub text: String,
}

/// A document's passages in reading order.
pub fn passages(conn: &Connection, document_id: &str) -> rusqlite::Result<Vec<PassageText>> {
    let mut stmt = conn
        .prepare_cached("SELECT id, text FROM passages WHERE document_id = ?1 ORDER BY ordinal")?;
    stmt.query_map([document_id], |r| {
        Ok(PassageText {
            id: r.get(0)?,
            text: r.get(1)?,
        })
    })?
    .collect()
}

/// One chunk to store: its passage, its place there and its text's hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedChunk {
    pub passage_id: i64,
    pub span: Span,
    pub hash: [u8; 32],
}

/// A document's chunks, in order, with their texts (for the misses that
/// need embedding).
pub fn plan(passages: &[PassageText]) -> Vec<(PlannedChunk, &str)> {
    passages
        .iter()
        .flat_map(|p| {
            chunk::chunk(&p.text).into_iter().map(move |c| {
                (
                    PlannedChunk {
                        passage_id: p.id,
                        span: c.span,
                        hash: c.hash,
                    },
                    c.text,
                )
            })
        })
        .collect()
}

/// Which of `hashes` already have a vector for `model_id`.
pub fn known_hashes(
    conn: &Connection,
    model_id: &str,
    hashes: &[[u8; 32]],
) -> rusqlite::Result<HashSet<[u8; 32]>> {
    let mut stmt =
        conn.prepare_cached("SELECT 1 FROM embeddings WHERE model_id = ?1 AND text_hash = ?2")?;
    let mut known = HashSet::new();
    for h in hashes {
        if stmt.exists(params![model_id, &h[..]])? {
            known.insert(*h);
        }
    }
    Ok(known)
}

/// A chunk as stored: its id (the index key) and its text's hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoredChunk {
    pub id: u64,
    pub hash: [u8; 32],
}

/// Stores one document's chunks and its new vectors and marks it embedded
/// for `model_id`, inside the caller's transaction (which checks that its
/// job may still write). Chunks the document already has from the same
/// plan are kept, so another model's index keeps its keys; a different
/// plan replaces them, leaving tombstones. Fails, writing nothing, if a
/// chunk would have no vector.
pub fn write_document(
    tx: &Transaction<'_>,
    document_id: &str,
    model_id: &str,
    plan: &[PlannedChunk],
    new_vectors: &[([u8; 32], Vec<u8>)],
    now: &str,
) -> rusqlite::Result<Vec<StoredChunk>> {
    let existing = document_chunks(tx, document_id)?;
    let same = existing.len() == plan.len()
        && existing
            .iter()
            .zip(plan)
            .all(|((_, have), want)| have == want);
    if !same {
        tx.execute("DELETE FROM chunks WHERE document_id = ?1", [document_id])?;
        let mut insert = tx.prepare_cached(
            "INSERT INTO chunks (document_id, passage_id, ordinal, char_start, char_end, text_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for (i, c) in plan.iter().enumerate() {
            insert.execute(params![
                document_id,
                c.passage_id,
                i as i64,
                c.span.start as i64,
                c.span.end as i64,
                &c.hash[..]
            ])?;
        }
    }
    {
        let mut insert = tx.prepare_cached(
            "INSERT OR IGNORE INTO embeddings (model_id, text_hash, vector) VALUES (?1, ?2, ?3)",
        )?;
        for (hash, blob) in new_vectors {
            insert.execute(params![model_id, &hash[..], blob])?;
        }
    }
    let missing: i64 = tx.query_row(
        "SELECT COUNT(*) FROM chunks c
         WHERE c.document_id = ?1
           AND NOT EXISTS (SELECT 1 FROM embeddings e
                           WHERE e.model_id = ?2 AND e.text_hash = c.text_hash)",
        params![document_id, model_id],
        |r| r.get(0),
    )?;
    if missing > 0 {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
            Some(format!("{missing} chunks have no vector")),
        ));
    }
    tx.execute(
        "INSERT INTO document_embeddings (document_id, model_id, seq, status, reason, chunk_count,
             attempts, updated_at)
         VALUES (?1, ?2, (SELECT COALESCE(MAX(seq), 0) + 1 FROM document_embeddings
                          WHERE model_id = ?2),
                 ?3, NULL, ?4, 0, ?5)
         ON CONFLICT (document_id, model_id) DO UPDATE SET
             seq = excluded.seq, status = excluded.status, reason = NULL,
             chunk_count = excluded.chunk_count, updated_at = excluded.updated_at",
        params![
            document_id,
            model_id,
            if plan.is_empty() { "empty" } else { "done" },
            plan.len() as i64,
            now
        ],
    )?;
    Ok(document_chunks(tx, document_id)?
        .into_iter()
        .map(|(id, c)| StoredChunk { id, hash: c.hash })
        .collect())
}

fn document_chunks(
    conn: &Connection,
    document_id: &str,
) -> rusqlite::Result<Vec<(u64, PlannedChunk)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, passage_id, char_start, char_end, text_hash
         FROM chunks WHERE document_id = ?1 ORDER BY ordinal",
    )?;
    stmt.query_map([document_id], |r| {
        let hash: Vec<u8> = r.get(4)?;
        Ok((
            r.get::<_, i64>(0)? as u64,
            PlannedChunk {
                passage_id: r.get(1)?,
                span: Span {
                    start: r.get::<_, i64>(2)? as usize,
                    end: r.get::<_, i64>(3)? as usize,
                },
                hash: hash.try_into().unwrap_or([0; 32]),
            },
        ))
    })?
    .collect()
}

/// Up to `limit` (chunk id, vector) pairs of the model's done documents
/// with chunk ids above `after`, in id order: a rebuild's input. A vector of
/// the wrong length is skipped.
pub fn vectors_after(
    conn: &Connection,
    model_id: &str,
    dims: usize,
    after: u64,
    limit: usize,
) -> rusqlite::Result<Vec<(u64, Vec<f32>)>> {
    let mut stmt = conn.prepare_cached(
        // CROSS JOIN keeps chunks as the outer loop, so each batch walks the
        // chunk ids in order instead of sorting the whole join.
        "SELECT c.id, e.vector FROM chunks c
         CROSS JOIN document_embeddings d
           ON d.document_id = c.document_id AND d.model_id = ?1 AND d.status = 'done'
         CROSS JOIN embeddings e ON e.model_id = ?1 AND e.text_hash = c.text_hash
         WHERE c.id > ?2 ORDER BY c.id LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![model_id, after as i64, limit as i64], |r| {
        Ok((r.get::<_, i64>(0)? as u64, r.get::<_, Vec<u8>>(1)?))
    })?;
    decode(rows, dims)
}

/// The model's documents written after `seq`, as (seq, document id), in
/// order: what a saved index file lacks.
pub fn documents_after(
    conn: &Connection,
    model_id: &str,
    seq: i64,
    limit: usize,
) -> rusqlite::Result<Vec<(i64, String)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT seq, document_id FROM document_embeddings
         WHERE model_id = ?1 AND seq > ?2 AND status = 'done' ORDER BY seq LIMIT ?3",
    )?;
    stmt.query_map(params![model_id, seq, limit as i64], |r| {
        Ok((r.get(0)?, r.get(1)?))
    })?
    .collect()
}

/// One document's (chunk id, vector) pairs for the model.
pub fn document_vectors(
    conn: &Connection,
    model_id: &str,
    dims: usize,
    document_id: &str,
) -> rusqlite::Result<Vec<(u64, Vec<f32>)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT c.id, e.vector FROM chunks c
         JOIN embeddings e ON e.model_id = ?1 AND e.text_hash = c.text_hash
         WHERE c.document_id = ?2 ORDER BY c.id",
    )?;
    let rows = stmt.query_map(params![model_id, document_id], |r| {
        Ok((r.get::<_, i64>(0)? as u64, r.get::<_, Vec<u8>>(1)?))
    })?;
    decode(rows, dims)
}

fn decode(
    rows: impl Iterator<Item = rusqlite::Result<(u64, Vec<u8>)>>,
    dims: usize,
) -> rusqlite::Result<Vec<(u64, Vec<f32>)>> {
    let mut out = Vec::new();
    for row in rows {
        let (id, blob) = row?;
        if let Some(v) = from_f16_blob(&blob, dims) {
            out.push((id, v));
        }
    }
    Ok(out)
}

/// The model's latest document seq.
pub fn last_seq(conn: &Connection, model_id: &str) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COALESCE(MAX(seq), 0) FROM document_embeddings WHERE model_id = ?1",
        [model_id],
        |r| r.get(0),
    )
}

/// How many chunks the model's documents written after `seq` hold.
pub fn chunks_after_seq(conn: &Connection, model_id: &str, seq: i64) -> rusqlite::Result<u64> {
    conn.query_row(
        "SELECT COALESCE(SUM(chunk_count), 0) FROM document_embeddings
         WHERE model_id = ?1 AND seq > ?2 AND status = 'done'",
        params![model_id, seq],
        |r| r.get::<_, i64>(0).map(|n| n as u64),
    )
}

/// Tombstones after `seq`, as (seq, chunk id), in order.
pub fn tombstones_after(conn: &Connection, seq: i64) -> rusqlite::Result<Vec<(i64, u64)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT seq, chunk_id FROM vector_tombstones WHERE seq > ?1 ORDER BY seq",
    )?;
    stmt.query_map([seq], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? as u64)))?
        .collect()
}

pub fn last_tombstone(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COALESCE(MAX(seq), 0) FROM vector_tombstones",
        [],
        |r| r.get(0),
    )
}

/// What a saved index file holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedIndex {
    pub generation: i64,
    pub blake3: [u8; 32],
    pub indexed_through_seq: i64,
    pub vector_count: u64,
    pub tombstones_through: i64,
}

/// Points the model at its newly saved file, then drains the tombstones
/// that every live model's saved file has applied.
pub fn record_save(
    conn: &mut Connection,
    model_id: &str,
    saved: &SavedIndex,
    now: &str,
) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE embedding_models SET index_generation = ?2, index_blake3 = ?3,
             indexed_through_seq = ?4, vector_count = ?5, tombstones_through = ?6,
             updated_at = ?7
         WHERE id = ?1",
        params![
            model_id,
            saved.generation,
            &saved.blake3[..],
            saved.indexed_through_seq,
            saved.vector_count as i64,
            saved.tombstones_through,
            now
        ],
    )?;
    tx.execute(
        "DELETE FROM vector_tombstones
         WHERE seq <= (SELECT MIN(tombstones_through) FROM embedding_models
                       WHERE status IN ('building', 'ready'))",
        [],
    )?;
    tx.commit()
}

// ------------------------------------------------------------ the job (2.3)

/// Attempts after which a failed document waits for a new model or a
/// re-read instead of being tried again.
pub const MAX_ATTEMPTS: i64 = 3;

/// A document waiting to be embedded for a model in one library folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitingDocument {
    pub rowid: i64,
    pub id: String,
}

/// SQL: a document (alias `d`) the model still needs: read, with no
/// finished row, or a failed one under [`MAX_ATTEMPTS`].
const NEEDS_EMBEDDING: &str = "d.state = 'done'
     AND NOT EXISTS (SELECT 1 FROM document_embeddings de
                     WHERE de.document_id = d.id AND de.model_id = :model
                       AND (de.status IN ('done', 'empty') OR de.attempts >= 3))";

/// Up to `limit` documents with an available file in `root_id` that the
/// model still needs, after `after` (document rowid), in rowid order.
pub fn next_documents(
    conn: &Connection,
    root_id: &str,
    model_id: &str,
    after: i64,
    limit: usize,
) -> rusqlite::Result<Vec<WaitingDocument>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT d.rowid, d.id FROM documents d
         WHERE d.rowid > :after AND {NEEDS_EMBEDDING}
           AND EXISTS (SELECT 1 FROM file_instances f
                       WHERE f.document_id = d.id AND f.source_root_id = :root
                         AND f.availability = 'available')
         ORDER BY d.rowid LIMIT :limit"
    ))?;
    stmt.query_map(
        rusqlite::named_params! {
            ":after": after, ":model": model_id, ":root": root_id, ":limit": limit as i64,
        },
        |r| {
            Ok(WaitingDocument {
                rowid: r.get(0)?,
                id: r.get(1)?,
            })
        },
    )?
    .collect()
}

/// Whether `root_id` has a document the model still needs.
pub fn has_waiting(conn: &Connection, root_id: &str, model_id: &str) -> rusqlite::Result<bool> {
    Ok(!next_documents(conn, root_id, model_id, 0, 1)?.is_empty())
}

/// What a document's text looked like when the job read it: its update
/// time and its passages' ids. A re-read in between changes it (ADR 0014).
pub fn fingerprint(conn: &Connection, document_id: &str) -> rusqlite::Result<String> {
    conn.query_row(
        "SELECT d.updated_at || '|' ||
                COALESCE((SELECT GROUP_CONCAT(p.id) FROM
                            (SELECT id FROM passages WHERE document_id = d.id ORDER BY ordinal) p),
                         '')
         FROM documents d WHERE d.id = ?1",
        [document_id],
        |r| r.get(0),
    )
    .optional()
    .map(Option::unwrap_or_default)
}

/// Records that a document couldn't be embedded for the model, inside the
/// caller's transaction: one more attempt, a new seq.
pub fn mark_failed(
    tx: &Transaction<'_>,
    document_id: &str,
    model_id: &str,
    reason: &str,
    now: &str,
) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO document_embeddings (document_id, model_id, seq, status, reason, chunk_count,
             attempts, updated_at)
         VALUES (?1, ?2, (SELECT COALESCE(MAX(seq), 0) + 1 FROM document_embeddings
                          WHERE model_id = ?2),
                 'failed', ?3, 0, 1, ?4)
         ON CONFLICT (document_id, model_id) DO UPDATE SET
             seq = excluded.seq, status = 'failed', reason = excluded.reason,
             chunk_count = 0, attempts = document_embeddings.attempts + 1,
             updated_at = excluded.updated_at",
        params![document_id, model_id, reason, now],
    )?;
    Ok(())
}

/// Queues an embed job for `root_id` if the model has documents waiting
/// there and the folder has no active embed job. Returns whether it did.
pub fn enqueue_if_waiting(
    conn: &Connection,
    root_id: &str,
    model_id: &str,
    now: &str,
) -> rusqlite::Result<bool> {
    use crate::domain::dto::JobKind;
    use crate::services::jobs::db::{active_for_root, insert_job};
    if !has_waiting(conn, root_id, model_id)?
        || active_for_root(conn, root_id, JobKind::Embed)?.is_some()
    {
        return Ok(false);
    }
    insert_job(
        conn,
        &uuid::Uuid::new_v4().to_string(),
        root_id,
        JobKind::Embed,
        now,
    )?;
    Ok(true)
}

/// Adds to an embed job's counters.
pub fn add_counters(
    conn: &Connection,
    job_id: &str,
    chunks: u64,
    chars: u64,
    service_ms: u64,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE jobs SET embedded_chunks = embedded_chunks + ?2, sent_chars = sent_chars + ?3,
             service_ms = service_ms + ?4 WHERE id = ?1",
        params![job_id, chunks as i64, chars as i64, service_ms as i64],
    )?;
    Ok(())
}

/// The documents the model has finished with (embedded, empty or failed
/// for good): a model's coverage, for comparing an old and a new one.
pub fn covered_documents(conn: &Connection, model_id: &str) -> rusqlite::Result<u64> {
    conn.query_row(
        "SELECT COUNT(*) FROM document_embeddings
         WHERE model_id = ?1 AND (status IN ('done', 'empty') OR attempts >= 3)",
        [model_id],
        |r| r.get::<_, i64>(0).map(|n| n as u64),
    )
}

/// Up to `limit` random (chunk id, vector) pairs of the model's done
/// documents: a validation sample.
pub fn sample_vectors(
    conn: &Connection,
    model_id: &str,
    dims: usize,
    limit: usize,
) -> rusqlite::Result<Vec<(u64, Vec<f32>)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT c.id, e.vector FROM document_embeddings d
         JOIN chunks c ON c.document_id = d.document_id
         JOIN embeddings e ON e.model_id = d.model_id AND e.text_hash = c.text_hash
         WHERE d.model_id = ?1 AND d.status = 'done'
         ORDER BY random() LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![model_id, limit as i64], |r| {
        Ok((r.get::<_, i64>(0)? as u64, r.get::<_, Vec<u8>>(1)?))
    })?;
    decode(rows, dims)
}

/// Deletes up to `limit` of a retired or failed model's rows (embeddings,
/// then document states). Returns how many went; 0 means none are left.
pub fn delete_model_rows(
    conn: &Connection,
    model_id: &str,
    limit: usize,
) -> rusqlite::Result<usize> {
    let n = conn.execute(
        "DELETE FROM embeddings WHERE id IN
             (SELECT id FROM embeddings WHERE model_id = ?1 LIMIT ?2)",
        params![model_id, limit as i64],
    )?;
    if n > 0 {
        return Ok(n);
    }
    conn.execute(
        "DELETE FROM document_embeddings WHERE rowid IN
             (SELECT rowid FROM document_embeddings WHERE model_id = ?1 LIMIT ?2)",
        params![model_id, limit as i64],
    )
}
