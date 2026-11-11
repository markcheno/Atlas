//! Suggestions' knowledge side (phase3-spec §7.4, Slice 3.6): the seeds
//! (each active interest or project, and the three most recent trails),
//! what the user opened, and what they dismissed. The vectors are the
//! catalog side's (`services/documents/rediscover.rs`).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use rusqlite::{Connection, params};

use super::record::Recorder;
use crate::domain::dto::{Suggestion, SuggestionReason};
use crate::error::{AtlasError, AtlasResult};
use crate::services::catalog::writer::CatalogHandle;
use crate::services::documents::rediscover::{self, SeedDocuments};
use crate::services::embedding::registry::VectorRegistry;

/// The documents a set of collection items reaches: its documents, its
/// bookmarks' and findings' sources, and its trails' steps.
fn reached(conn: &Connection, collection: &str) -> rusqlite::Result<Vec<String>> {
    let mut docs: BTreeSet<String> = BTreeSet::new();
    let mut add = |sql: &str| -> rusqlite::Result<()> {
        let mut stmt = conn.prepare_cached(sql)?;
        for d in stmt.query_map([collection], |r| r.get::<_, Option<String>>(0))? {
            docs.extend(d?);
        }
        Ok(())
    };
    add(
        "SELECT item_id FROM collection_items WHERE collection_id = ?1 AND item_kind = 'document'",
    )?;
    add(
        "SELECT b.document_id FROM bookmarks b JOIN collection_items i
           ON i.item_kind = 'bookmark' AND i.item_id = b.id WHERE i.collection_id = ?1",
    )?;
    add("SELECT json_extract(s.value, '$.anchor.documentId')
         FROM findings f JOIN collection_items i ON i.item_kind = 'finding' AND i.item_id = f.id,
              json_each(f.sources) s
         WHERE i.collection_id = ?1")?;
    add("SELECT d.document_id FROM step_documents d
         JOIN trail_steps s ON s.id = d.step_id
         JOIN collection_items i ON i.item_kind = 'trail' AND i.item_id = s.trail_id
         WHERE i.collection_id = ?1")?;
    Ok(docs.into_iter().collect())
}

/// Every seed with its documents: active collections, then recent trails.
pub fn seeds(conn: &Connection) -> rusqlite::Result<Vec<SeedDocuments>> {
    let mut out = Vec::new();
    let mut stmt = conn.prepare_cached(
        "SELECT id, kind, name FROM collections WHERE status = 'active' ORDER BY id",
    )?;
    let active = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, kind, name) in active {
        let reason = if kind == "project" {
            SuggestionReason::Project { name }
        } else {
            SuggestionReason::Interest { name }
        };
        let documents = reached(conn, &id)?;
        out.push(SeedDocuments {
            id,
            reason,
            documents,
        });
    }
    let mut stmt = conn.prepare_cached(
        "SELECT DISTINCT d.document_id FROM step_documents d
         JOIN trail_steps s ON s.id = d.step_id
         WHERE s.trail_id IN (SELECT id FROM trails ORDER BY last_step_at DESC, id LIMIT 3)
         ORDER BY d.document_id",
    )?;
    let recent = stmt
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    out.push(SeedDocuments {
        id: String::new(),
        reason: SuggestionReason::Recent,
        documents: recent,
    });
    Ok(out)
}

/// Documents the user opened from anywhere in a trail.
pub fn opened(conn: &Connection) -> rusqlite::Result<HashSet<String>> {
    let mut stmt = conn.prepare_cached(
        "SELECT DISTINCT d.document_id FROM step_documents d
         JOIN trail_steps s ON s.id = d.step_id WHERE s.kind = 'open'",
    )?;
    stmt.query_map([], |r| r.get(0))?.collect()
}

/// Documents dismissed, by seed.
pub fn dismissed(conn: &Connection) -> rusqlite::Result<HashMap<String, HashSet<String>>> {
    let mut out: HashMap<String, HashSet<String>> = HashMap::new();
    let mut stmt =
        conn.prepare_cached("SELECT collection_id, document_id FROM dismissed_suggestions")?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (seed, doc) = row?;
        out.entry(seed).or_default().insert(doc);
    }
    Ok(out)
}

/// Up to `limit` suggestions, in the day's order. Nothing is sent
/// anywhere: only vectors already stored are read.
pub fn suggestions(
    rec: &Recorder,
    vectors: &Arc<VectorRegistry>,
    catalog: &CatalogHandle,
    limit: usize,
) -> AtlasResult<Vec<Suggestion>> {
    let (seeds, opened, dismissed) = rec
        .store()
        .read(|c| -> rusqlite::Result<_> { Ok((seeds(c)?, opened(c)?, dismissed(c)?)) })?
        .map_err(AtlasError::db("be read"))?;
    let day = rec.store().clock().now_rfc3339()[..10].to_owned();
    rec.catalog()
        .read(|c| {
            rediscover::suggest(
                c, vectors, catalog, &seeds, &opened, &dismissed, limit, &day,
            )
        })?
        .map_err(AtlasError::db("be read"))
}

/// "Not for this": the document is no longer suggested for that seed (a
/// collection, or `''` for recent trails).
pub fn dismiss(rec: &Recorder, seed_id: &str, document_id: &str) -> AtlasResult<()> {
    let hash: Vec<u8> = rec
        .catalog()
        .read(|c| {
            c.query_row(
                "SELECT content_hash FROM documents WHERE id = ?1",
                [document_id],
                |r| r.get(0),
            )
        })?
        .map_err(|_| AtlasError::SaveSourceGone)?;
    let now = rec.store().clock().now_rfc3339();
    let (seed, doc) = (seed_id.to_owned(), document_id.to_owned());
    rec.store()
        .call(move |c| {
            c.execute(
                "INSERT OR REPLACE INTO dismissed_suggestions (collection_id, document_id, content_hash, at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![seed, doc, hash, now],
            )
        })?
        .map_err(AtlasError::db("save that"))?;
    Ok(())
}
