//! Which embedding model the job fills and search uses (phase2-spec §7.3,
//! ADR 0014).
//!
//! A model row is the embedding role's service and model at its
//! dimensions and the chunker's version. Changing any of them makes a new
//! row, `building`, which the job fills while the newest `ready` row keeps
//! serving. When it is complete and valid it becomes `ready` and the old
//! one `retired`; retired and failed rows are deleted in batches.

use rusqlite::{Connection, OptionalExtension, params};

use super::chunk::CHUNKER_VERSION;
use super::db::{self, EmbeddingModel};
use crate::services::providers::recommend::query_profile;

/// The embedding role, as a model row would describe it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleModel {
    pub profile_id: String,
    pub model_name: String,
    pub dims: usize,
}

/// The embedding role, if one is set with its dimensions.
pub fn role(conn: &Connection) -> rusqlite::Result<Option<RoleModel>> {
    conn.query_row(
        "SELECT provider_profile_id, model_id, dimensions FROM model_roles WHERE role = 'embedding'",
        [],
        |r| {
            Ok(r.get::<_, Option<i64>>(2)?.map(|dims| RoleModel {
                profile_id: r.get(0).unwrap_or_default(),
                model_name: r.get(1).unwrap_or_default(),
                dims: dims as usize,
            }))
        },
    )
    .optional()
    .map(Option::flatten)
}

fn newest(conn: &Connection, status: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT id FROM embedding_models WHERE status = ?1
         ORDER BY created_at DESC, rowid DESC LIMIT 1",
        [status],
        |r| r.get(0),
    )
    .optional()
}

/// The model the job fills: the newest `building` row, else the newest
/// `ready` one.
pub fn active(conn: &Connection) -> rusqlite::Result<Option<EmbeddingModel>> {
    let id = match newest(conn, "building")? {
        Some(id) => Some(id),
        None => newest(conn, "ready")?,
    };
    id.map_or(Ok(None), |id| db::get_model(conn, &id))
}

/// The model search uses: the newest `ready` row, else the `building` one
/// (a first index serves while it fills).
pub fn serving(conn: &Connection) -> rusqlite::Result<Option<EmbeddingModel>> {
    let id = match newest(conn, "ready")? {
        Some(id) => Some(id),
        None => newest(conn, "building")?,
    };
    id.map_or(Ok(None), |id| db::get_model(conn, &id))
}

/// The newest `ready` model other than `id`: what a building model
/// replaces.
pub fn predecessor(conn: &Connection, id: &str) -> rusqlite::Result<Option<EmbeddingModel>> {
    let found: Option<String> = conn
        .query_row(
            "SELECT id FROM embedding_models WHERE status = 'ready' AND id <> ?1
             ORDER BY created_at DESC, rowid DESC LIMIT 1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    found.map_or(Ok(None), |id| db::get_model(conn, &id))
}

/// Brings the model rows in line with the embedding role: the row that
/// matches it (building or ready) stays; with none, a new `building` row is
/// made, and a building row the role no longer names is abandoned
/// (`failed`). With no role, nothing changes: existing vectors stay.
/// Returns the active model.
pub fn sync(conn: &Connection, now: &str) -> rusqlite::Result<Option<EmbeddingModel>> {
    let Some(mut role) = role(conn)? else {
        return active(conn);
    };
    // A model the catalog says may be cut short is stored at fewer
    // dimensions (Slice 2.4); the row and its matching use that length.
    if let Some(keep) = query_profile(&role.model_name).stored_dimensions {
        role.dims = role.dims.min(keep);
    }
    let matching: Option<String> = conn
        .query_row(
            "SELECT id FROM embedding_models
             WHERE provider_profile_id = ?1 AND model_name = ?2 AND dims = ?3
               AND chunker_version = ?4 AND status IN ('building', 'ready')
             ORDER BY created_at DESC, rowid DESC LIMIT 1",
            params![
                role.profile_id,
                role.model_name,
                role.dims as i64,
                CHUNKER_VERSION
            ],
            |r| r.get(0),
        )
        .optional()?;
    let keep = match matching {
        Some(id) => id,
        None => db::create_model(
            conn,
            Some(&role.profile_id),
            &role.model_name,
            role.dims,
            None,
            now,
        )?,
    };
    conn.execute(
        "UPDATE embedding_models SET status = 'failed', updated_at = ?2
         WHERE status = 'building' AND id <> ?1",
        params![keep, now],
    )?;
    active(conn)
}

pub fn set_status(conn: &Connection, id: &str, status: &str, now: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE embedding_models SET status = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, status, now],
    )?;
    Ok(())
}

/// Models whose rows and files are to be removed.
pub fn finished_with(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    conn.prepare("SELECT id FROM embedding_models WHERE status IN ('retired', 'failed')")?
        .query_map([], |r| r.get(0))?
        .collect()
}

/// Every model the catalog still has, for sweeping stray index files.
pub fn all_ids(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    conn.prepare("SELECT id FROM embedding_models")?
        .query_map([], |r| r.get(0))?
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::catalog::migrations::migrate;
    use crate::services::catalog::writer::CatalogWriter;

    fn with_catalog(f: impl FnOnce(&Connection) + Send + 'static) {
        let dir = tempfile::tempdir().unwrap();
        let writer = CatalogWriter::open(&dir.path().join("catalog.sqlite")).unwrap();
        writer
            .call(move |c| {
                migrate(c, "t").unwrap();
                c.execute_batch(
                    "INSERT INTO provider_profiles (id, kind, label, base_url,
                         insecure_http_acknowledged, created_at, updated_at)
                     VALUES ('p', 'openai_compatible', 'oMLX', 'http://localhost:8000/v1', 0, 't', 't'),
                            ('q', 'ollama', 'Ollama', 'http://localhost:11434', 0, 't', 't');",
                )
                .unwrap();
                f(c);
            })
            .unwrap();
    }

    fn set_role(c: &Connection, profile: &str, model: &str, dims: i64) {
        c.execute(
            "INSERT INTO model_roles (role, provider_profile_id, model_id, dimensions, tested_at, updated_at)
             VALUES ('embedding', ?1, ?2, ?3, 't', 't')
             ON CONFLICT (role) DO UPDATE SET provider_profile_id = excluded.provider_profile_id,
                 model_id = excluded.model_id, dimensions = excluded.dimensions",
            params![profile, model, dims],
        )
        .unwrap();
    }

    #[test]
    fn the_role_makes_one_row_and_a_change_makes_another() {
        with_catalog(|c| {
            assert_eq!(sync(c, "t1").unwrap(), None, "no role, no model");
            set_role(c, "p", "qwen3-embedding", 1024);
            let first = sync(c, "t1").unwrap().unwrap();
            assert_eq!((first.status.as_str(), first.dims), ("building", 1024));
            assert_eq!(sync(c, "t1").unwrap().unwrap().id, first.id, "idempotent");
            assert_eq!(
                serving(c).unwrap().unwrap().id,
                first.id,
                "a first index serves"
            );

            set_status(c, &first.id, "ready", "t2").unwrap();
            // Another service for the same model is another index.
            set_role(c, "q", "qwen3-embedding", 1024);
            let second = sync(c, "t3").unwrap().unwrap();
            assert_ne!(second.id, first.id);
            assert_eq!(active(c).unwrap().unwrap().id, second.id);
            assert_eq!(
                serving(c).unwrap().unwrap().id,
                first.id,
                "the old one serves"
            );
            assert_eq!(predecessor(c, &second.id).unwrap().unwrap().id, first.id);

            // Changing again before it's ready abandons it.
            set_role(c, "q", "qwen3-embedding", 512);
            let third = sync(c, "t4").unwrap().unwrap();
            let second_now = db::get_model(c, &second.id).unwrap().unwrap();
            assert_eq!(second_now.status, "failed");
            assert_eq!(finished_with(c).unwrap(), std::slice::from_ref(&second.id));
            assert_eq!(third.dims, 512);

            // Back to the ready model's role: it's active again, nothing new.
            set_role(c, "p", "qwen3-embedding", 1024);
            assert_eq!(sync(c, "t5").unwrap().unwrap().id, first.id);
            assert_eq!(
                db::get_model(c, &third.id).unwrap().unwrap().status,
                "failed"
            );

            // Clearing the role keeps what exists.
            c.execute_batch("DELETE FROM model_roles").unwrap();
            assert_eq!(sync(c, "t6").unwrap().unwrap().id, first.id);
        });
    }

    #[test]
    fn a_model_the_catalog_cuts_short_is_stored_at_fewer_dimensions() {
        with_catalog(|c| {
            set_role(c, "p", "Qwen3-Embedding-0.6B-8bit", 1024);
            let m = sync(c, "t").unwrap().unwrap();
            assert_eq!(m.dims, 512, "Qwen3-Embedding is kept at 512 (Slice 2.4)");
            assert_eq!(sync(c, "t").unwrap().unwrap().id, m.id, "and still matched");
            set_role(c, "p", "some-other-embedder", 768);
            assert_eq!(sync(c, "t2").unwrap().unwrap().dims, 768, "others keep their length");
        });
    }
}

