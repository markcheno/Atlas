//! Versioned, append-only catalog migrations (see `migrations/README.md`).

use rusqlite::{Connection, OptionalExtension};

use crate::error::{AtlasError, AtlasResult};

pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "initial",
        sql: include_str!("../../../migrations/0001_initial.sql"),
    },
    Migration {
        version: 2,
        name: "source_roots",
        sql: include_str!("../../../migrations/0002_source_roots.sql"),
    },
    Migration {
        version: 3,
        name: "inventory",
        sql: include_str!("../../../migrations/0003_inventory.sql"),
    },
    Migration {
        version: 4,
        name: "search_keys",
        sql: include_str!("../../../migrations/0004_search_keys.sql"),
    },
    Migration {
        version: 5,
        name: "provider_profiles",
        sql: include_str!("../../../migrations/0005_provider_profiles.sql"),
    },
    Migration {
        version: 6,
        name: "status_counts_index",
        sql: include_str!("../../../migrations/0006_status_counts_index.sql"),
    },
    Migration {
        version: 7,
        name: "jobs",
        sql: include_str!("../../../migrations/0007_jobs.sql"),
    },
    Migration {
        version: 8,
        name: "documents",
        sql: include_str!("../../../migrations/0008_documents.sql"),
    },
    Migration {
        version: 9,
        name: "passages_fts",
        sql: include_str!("../../../migrations/0009_passages_fts.sql"),
    },
    Migration {
        version: 10,
        name: "documents_quality",
        sql: include_str!("../../../migrations/0010_documents_quality.sql"),
    },
    Migration {
        version: 11,
        name: "model_roles",
        sql: include_str!("../../../migrations/0011_model_roles.sql"),
    },
    Migration {
        version: 12,
        name: "embeddings",
        sql: include_str!("../../../migrations/0012_embeddings.sql"),
    },
    Migration {
        version: 13,
        name: "embed_jobs",
        sql: include_str!("../../../migrations/0013_embed_jobs.sql"),
    },
    Migration {
        version: 14,
        name: "boilerplate",
        sql: include_str!("../../../migrations/0014_boilerplate.sql"),
    },
    Migration {
        version: 15,
        name: "ocr_pages",
        sql: include_str!("../../../migrations/0015_ocr_pages.sql"),
    },
    Migration {
        version: 16,
        name: "scan_history",
        sql: include_str!("../../../migrations/0016_scan_history.sql"),
    },
];

/// Highest catalog schema version this build can read and write.
pub const SUPPORTED_SCHEMA_VERSION: u32 = MIGRATIONS[MIGRATIONS.len() - 1].version;

const CREATE_TRACKING_TABLE: &str = "CREATE TABLE IF NOT EXISTS schema_migrations (
    version    INTEGER PRIMARY KEY NOT NULL,
    name       TEXT NOT NULL,
    applied_at TEXT NOT NULL
) STRICT";

/// Current schema version without writing anything: 0 for a new database.
pub fn current_version(conn: &Connection) -> AtlasResult<u32> {
    let has_table: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_migrations'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(AtlasError::db("be read"))?;
    if has_table.is_none() {
        return Ok(0);
    }
    let version: Option<i64> = conn
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .map_err(AtlasError::db("be read"))?;
    u32::try_from(version.unwrap_or(0)).map_err(|_| AtlasError::DataRootInvalid {
        reason: "the catalog's version record is unreadable",
    })
}

/// Applies every pending migration, each in its own transaction.
/// Returns the resulting schema version. Refuses newer databases.
pub fn migrate(conn: &mut Connection, now: &str) -> AtlasResult<u32> {
    migrate_to(conn, SUPPORTED_SCHEMA_VERSION, now)
}

/// Applies pending migrations up to and including `target`. The app always
/// migrates to [`SUPPORTED_SCHEMA_VERSION`]; tests use a lower target to
/// build the catalog an older release would have left behind.
pub fn migrate_to(conn: &mut Connection, target: u32, now: &str) -> AtlasResult<u32> {
    let current = current_version(conn)?;
    if current > SUPPORTED_SCHEMA_VERSION {
        return Err(AtlasError::DataRootIncompatible {
            found: current,
            supported: SUPPORTED_SCHEMA_VERSION,
        });
    }
    conn.execute_batch(CREATE_TRACKING_TABLE)
        .map_err(AtlasError::db("be prepared for updates"))?;

    for migration in MIGRATIONS
        .iter()
        .filter(|m| m.version > current && m.version <= target)
    {
        let tx = conn
            .transaction()
            .map_err(AtlasError::db("start an update"))?;
        tx.execute_batch(migration.sql)
            .map_err(AtlasError::db("apply an update"))?;
        tx.execute(
            "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![migration.version, migration.name, now],
        )
        .map_err(AtlasError::db("record an update"))?;
        tx.commit().map_err(AtlasError::db("finish an update"))?;
    }
    current_version(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_are_contiguous_from_one() {
        for (i, m) in MIGRATIONS.iter().enumerate() {
            assert_eq!(
                m.version as usize,
                i + 1,
                "migration {} out of order",
                m.name
            );
        }
    }

    #[test]
    fn migrate_is_idempotent() {
        let mut conn = Connection::open_in_memory().unwrap();
        assert_eq!(current_version(&conn).unwrap(), 0);
        let now = "2026-11-07T00:00:00Z";
        assert_eq!(migrate(&mut conn, now).unwrap(), SUPPORTED_SCHEMA_VERSION);
        assert_eq!(migrate(&mut conn, now).unwrap(), SUPPORTED_SCHEMA_VERSION);
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows as usize, MIGRATIONS.len());
    }

    /// Version 7 renames `scan_sessions` to `jobs` (Phase 1, ADR 0009). Run
    /// on the real writer connection, which has foreign keys on, as in the
    /// app: the frontier's foreign key must follow the rename.
    #[test]
    fn version_9_indexes_existing_passages_and_keeps_the_index_in_step() {
        use crate::services::catalog::writer::CatalogWriter;

        let dir = tempfile::tempdir().unwrap();
        let writer = CatalogWriter::open(&dir.path().join("catalog.sqlite")).unwrap();
        writer
            .call(|c| {
                assert_eq!(migrate_to(c, 8, "t").unwrap(), 8);
                c.execute_batch(
                    "INSERT INTO documents (id, content_hash, state, created_at, updated_at)
                         VALUES ('d', x'01', 'done', 't', 't');
                     INSERT INTO passages (document_id, ordinal, text)
                         VALUES ('d', 0, 'Quoth the Raven, Nevermore'),
                                ('d', 1, 'Un café crème, s''il vous plaît');",
                )
                .unwrap();

                assert_eq!(migrate_to(c, 9, "t2").unwrap(), 9);
                let hits = |c: &Connection, q: &str| -> i64 {
                    c.query_row(
                        "SELECT COUNT(*) FROM passages_fts WHERE passages_fts MATCH ?1",
                        [q],
                        |r| r.get(0),
                    )
                    .unwrap()
                };
                // Passages stored before the index existed are found, with
                // case and accents folded.
                assert_eq!(hits(c, "nevermore"), 1);
                assert_eq!(hits(c, "\"CAFE CREME\""), 1);

                // Inserts, updates and deletes keep the index in step.
                c.execute(
                    "INSERT INTO passages (document_id, ordinal, text) VALUES ('d', 2, 'harmonic drive')",
                    [],
                )
                .unwrap();
                assert_eq!(hits(c, "harmonic"), 1);
                c.execute(
                    "UPDATE passages SET text = 'gear train' WHERE ordinal = 2",
                    [],
                )
                .unwrap();
                assert_eq!(hits(c, "harmonic"), 0);
                assert_eq!(hits(c, "gear"), 1);
                c.execute("DELETE FROM passages WHERE ordinal = 0", []).unwrap();
                assert_eq!(hits(c, "nevermore"), 0);
                c.execute_batch(
                    "INSERT INTO passages_fts (passages_fts, rank) VALUES ('integrity-check', 1)",
                )
                .unwrap();
            })
            .unwrap();
    }

    #[test]
    fn version_10_adds_content_titles_refresh_and_the_duplicates_index() {
        use crate::services::catalog::writer::CatalogWriter;

        let dir = tempfile::tempdir().unwrap();
        let writer = CatalogWriter::open(&dir.path().join("catalog.sqlite")).unwrap();
        writer
            .call(|c| {
                assert_eq!(migrate_to(c, 9, "t").unwrap(), 9);
                c.execute_batch(
                    "INSERT INTO documents (id, content_hash, state, title, created_at, updated_at)
                         VALUES ('d', x'01', 'done', 'Kept', 't', 't');",
                )
                .unwrap();
                assert_eq!(migrate_to(c, 10, "t2").unwrap(), 10);
                let (title, content, refresh): (String, Option<String>, i64) = c
                    .query_row(
                        "SELECT title, content_title, refresh FROM documents WHERE id = 'd'",
                        [],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .unwrap();
                assert_eq!((title.as_str(), content, refresh), ("Kept", None, 0));
                let cols: Vec<String> = c
                    .prepare("SELECT name FROM pragma_index_info('file_instances_by_document')")
                    .unwrap()
                    .query_map([], |r| r.get(0))
                    .unwrap()
                    .collect::<Result<_, _>>()
                    .unwrap();
                assert_eq!(cols, ["document_id", "availability"]);
            })
            .unwrap();
    }

    #[test]
    fn version_11_adds_model_roles_that_follow_their_profile() {
        use crate::services::catalog::writer::CatalogWriter;

        let dir = tempfile::tempdir().unwrap();
        let writer = CatalogWriter::open(&dir.path().join("catalog.sqlite")).unwrap();
        writer
            .call(|c| {
                assert_eq!(migrate_to(c, 10, "t").unwrap(), 10);
                c.execute_batch(
                    "INSERT INTO provider_profiles (id, kind, label, base_url,
                         insecure_http_acknowledged, created_at, updated_at)
                         VALUES ('p', 'openai_compatible', 'oMLX', 'http://localhost:8000/v1', 0, 't', 't');",
                )
                .unwrap();
                assert_eq!(migrate_to(c, 11, "t2").unwrap(), 11);
                c.execute_batch(
                    "INSERT INTO model_roles (role, provider_profile_id, model_id, dimensions, tested_at, updated_at)
                         VALUES ('embedding', 'p', 'qwen3-embedding:0.6b', 1024, 't', 't'),
                                ('chat', 'p', 'qwen3.5:9b', NULL, 't', 't');",
                )
                .unwrap();
                assert!(
                    c.execute_batch(
                        "INSERT INTO model_roles (role, provider_profile_id, model_id, tested_at, updated_at)
                             VALUES ('reranker', 'p', 'm', 't', 't')"
                    )
                    .is_err(),
                    "only the two roles"
                );
                assert!(
                    c.execute_batch(
                        "INSERT OR REPLACE INTO model_roles (role, provider_profile_id, model_id, tested_at, updated_at)
                             VALUES ('chat', 'missing', 'm', 't', 't')"
                    )
                    .is_err(),
                    "a role names a saved service"
                );
                c.execute_batch("DELETE FROM provider_profiles WHERE id = 'p'")
                    .unwrap();
                let left: i64 = c
                    .query_row("SELECT COUNT(*) FROM model_roles", [], |r| r.get(0))
                    .unwrap();
                assert_eq!(left, 0, "removing the service clears its roles");
            })
            .unwrap();
    }

    #[test]
    fn version_12_adds_chunks_and_embeddings_that_follow_their_rows() {
        use crate::services::catalog::writer::CatalogWriter;

        let dir = tempfile::tempdir().unwrap();
        let writer = CatalogWriter::open(&dir.path().join("catalog.sqlite")).unwrap();
        writer
            .call(|c| {
                assert_eq!(migrate_to(c, 11, "t").unwrap(), 11);
                c.execute_batch(
                    "INSERT INTO documents (id, content_hash, state, created_at, updated_at)
                         VALUES ('d', x'01', 'done', 't', 't');
                     INSERT INTO passages (id, document_id, ordinal, page_no, location, text)
                         VALUES (1, 'd', 0, 1, 'Page 1', 'one'), (2, 'd', 1, 2, 'Page 2', 'two');",
                )
                .unwrap();
                assert_eq!(migrate_to(c, 12, "t2").unwrap(), 12);
                let passages: i64 = c
                    .query_row("SELECT COUNT(*) FROM passages", [], |r| r.get(0))
                    .unwrap();
                assert_eq!(passages, 2, "the upgrade leaves existing rows alone");

                c.execute_batch(
                    "INSERT INTO embedding_models (id, model_name, dims, quantization, chunker_version,
                         status, created_at, updated_at)
                         VALUES ('m', 'qwen3-embedding:0.6b', 4, 'f16', 1, 'building', 't', 't');
                     INSERT INTO chunks (document_id, passage_id, ordinal, char_start, char_end, text_hash)
                         VALUES ('d', 1, 0, 0, 3, x'aa'), ('d', 2, 1, 0, 3, x'bb');
                     INSERT INTO embeddings (model_id, text_hash, vector) VALUES ('m', x'aa', x'00'), ('m', x'bb', x'00');
                     INSERT INTO document_embeddings (document_id, model_id, seq, status, chunk_count, updated_at)
                         VALUES ('d', 'm', 1, 'done', 2, 't');",
                )
                .unwrap();
                assert!(
                    c.execute_batch(
                        "INSERT INTO embedding_models (id, model_name, dims, quantization, chunker_version,
                             status, created_at, updated_at)
                             VALUES ('x', 'm', 4, 'f32', 1, 'building', 't', 't')"
                    )
                    .is_err(),
                    "f16 only"
                );
                assert!(
                    c.execute_batch(
                        "INSERT INTO chunks (document_id, passage_id, ordinal, char_start, char_end, text_hash)
                             VALUES ('d', 1, 5, 3, 3, x'cc')"
                    )
                    .is_err(),
                    "a chunk is never empty"
                );

                // Deleting a passage cascades to its chunks, and the trigger
                // fires for cascaded rows: the index learns of the removal.
                c.execute_batch("DELETE FROM passages WHERE id = 1").unwrap();
                let tombs: Vec<i64> = c
                    .prepare("SELECT chunk_id FROM vector_tombstones")
                    .unwrap()
                    .query_map([], |r| r.get(0))
                    .unwrap()
                    .map(Result::unwrap)
                    .collect();
                assert_eq!(tombs, [1]);

                // A deleted id is never reused (AUTOINCREMENT).
                c.execute_batch("DELETE FROM chunks WHERE id = 2").unwrap();
                c.execute_batch(
                    "INSERT INTO chunks (document_id, passage_id, ordinal, char_start, char_end, text_hash)
                         VALUES ('d', 2, 0, 0, 3, x'bb')",
                )
                .unwrap();
                let id: i64 = c
                    .query_row("SELECT MAX(id) FROM chunks", [], |r| r.get(0))
                    .unwrap();
                assert_eq!(id, 3);

                c.execute_batch("DELETE FROM embedding_models WHERE id = 'm'")
                    .unwrap();
                let left: (i64, i64) = c
                    .query_row(
                        "SELECT (SELECT COUNT(*) FROM embeddings),
                                (SELECT COUNT(*) FROM document_embeddings)",
                        [],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .unwrap();
                assert_eq!(left, (0, 0), "a model's vectors go with it");
            })
            .unwrap();
    }

    #[test]
    fn version_13_adds_the_embed_job_counters() {
        use crate::services::catalog::writer::CatalogWriter;

        let dir = tempfile::tempdir().unwrap();
        let writer = CatalogWriter::open(&dir.path().join("catalog.sqlite")).unwrap();
        writer
            .call(|c| {
                assert_eq!(migrate_to(c, 12, "t").unwrap(), 12);
                c.execute_batch(
                    "INSERT INTO workspace (id, name, created_at) VALUES ('w', 'W', 't');
                     INSERT INTO source_roots (id, workspace_id, path_bytes, display_path, created_at)
                         VALUES ('r', 'w', x'00', '/b', 't');
                     INSERT INTO jobs (id, source_root_id, state, kind, created_at, updated_at)
                         VALUES ('j', 'r', 'completed', 'extract', 't', 't');",
                )
                .unwrap();
                assert_eq!(migrate_to(c, 13, "t2").unwrap(), 13);
                let counters: (i64, i64, i64) = c
                    .query_row(
                        "SELECT embedded_chunks, sent_chars, service_ms FROM jobs WHERE id = 'j'",
                        [],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .unwrap();
                assert_eq!(counters, (0, 0, 0), "existing jobs start at zero");
            })
            .unwrap();
    }

    #[test]
    fn version_14_counts_the_boilerplate_left_out() {
        use crate::services::catalog::writer::CatalogWriter;

        let dir = tempfile::tempdir().unwrap();
        let writer = CatalogWriter::open(&dir.path().join("catalog.sqlite")).unwrap();
        writer
            .call(|c| {
                assert_eq!(migrate_to(c, 13, "t").unwrap(), 13);
                c.execute_batch(
                    "INSERT INTO documents (id, content_hash, state, created_at, updated_at)
                         VALUES ('d', x'01', 'done', 't', 't');",
                )
                .unwrap();
                assert_eq!(migrate_to(c, 14, "t2").unwrap(), 14);
                let n: i64 = c
                    .query_row(
                        "SELECT boilerplate_chars FROM documents WHERE id = 'd'",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert_eq!(n, 0, "documents read before start at zero");
            })
            .unwrap();
    }

    #[test]
    fn version_15_sends_uncounted_scanned_pdfs_back_once() {
        use crate::services::catalog::writer::CatalogWriter;

        let dir = tempfile::tempdir().unwrap();
        let writer = CatalogWriter::open(&dir.path().join("catalog.sqlite")).unwrap();
        writer
            .call(|c| {
                assert_eq!(migrate_to(c, 14, "t").unwrap(), 14);
                c.execute_batch(
                    "INSERT INTO workspace (id, name, created_at) VALUES ('w', 'W', 't');
                     INSERT INTO source_roots (id, workspace_id, path_bytes, display_path, created_at)
                         VALUES ('r', 'w', x'00', '/b', 't');
                     INSERT INTO documents (id, content_hash, state, page_count, created_at, updated_at)
                         VALUES ('done', x'01', 'done', 3, 't', 't'),
                                ('scan', x'02', 'needs_ocr', NULL, 't', 't'),
                                ('book', x'03', 'needs_ocr', NULL, 't', 't');",
                )
                .unwrap();
                for (id, doc, kind) in [("f1", "done", "pdf"), ("f2", "scan", "pdf"), ("f3", "book", "epub")] {
                    c.execute(
                        "INSERT INTO file_instances (id, source_root_id, rel_path, display_path,
                             display_name, extension, file_type, availability, document_id,
                             created_at, updated_at)
                         VALUES (?1, 'r', CAST(?1 AS BLOB), ?1, ?1, ?3, ?3, 'available', ?2, 't', 't')",
                        rusqlite::params![id, doc, kind],
                    )
                    .unwrap();
                }
                assert_eq!(migrate_to(c, 15, "t2").unwrap(), 15);
                let rows: Vec<(String, i64, Option<i64>)> = c
                    .prepare("SELECT id, refresh, text_pages FROM documents ORDER BY id")
                    .unwrap()
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                    .unwrap()
                    .map(Result::unwrap)
                    .collect();
                assert_eq!(
                    rows,
                    vec![
                        ("book".into(), 0, None),
                        ("done".into(), 0, None),
                        ("scan".into(), 1, None),
                    ],
                    "only the scanned PDF is read again"
                );
            })
            .unwrap();
    }

    #[test]
    fn version_16_starts_scan_history_and_keeps_a_scan_in_progress_counting() {
        use crate::services::catalog::writer::CatalogWriter;

        let dir = tempfile::tempdir().unwrap();
        let writer = CatalogWriter::open(&dir.path().join("catalog.sqlite")).unwrap();
        writer
            .call(|c| {
                assert_eq!(migrate_to(c, 15, "t").unwrap(), 15);
                c.execute_batch(
                    "INSERT INTO workspace (id, name, created_at) VALUES ('w', 'W', 't');
                     INSERT INTO source_roots (id, workspace_id, path_bytes, display_path, created_at)
                         VALUES ('r', 'w', x'00', '/b', 't');
                     INSERT INTO jobs (id, source_root_id, state, kind, created_at, updated_at, discovered_files)
                         VALUES ('old', 'r', 'completed', 'scan', 't', 't', 7),
                                ('live', 'r', 'paused_user', 'scan', 't', 't', 0);",
                )
                .unwrap();
                for (id, session) in [("f1", "live"), ("f2", "live"), ("f3", "old")] {
                    c.execute(
                        "INSERT INTO file_instances (id, source_root_id, rel_path, display_path,
                             display_name, extension, file_type, availability,
                             last_seen_session_id, created_at, updated_at)
                         VALUES (?1, 'r', CAST(?1 AS BLOB), ?1, ?1, 'pdf', 'pdf', 'available', ?2, 't', 't')",
                        rusqlite::params![id, session],
                    )
                    .unwrap();
                }
                assert_eq!(migrate_to(c, 16, "t2").unwrap(), 16);
                let jobs: Vec<(String, Option<String>, i64, i64)> = c
                    .prepare("SELECT id, origin, discovered_files, new_files FROM jobs ORDER BY id")
                    .unwrap()
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
                    .unwrap()
                    .map(Result::unwrap)
                    .collect();
                // The finished scan keeps its count; the one in progress
                // counts what it had written; neither has an origin.
                assert_eq!(
                    jobs,
                    vec![("live".into(), None, 2, 0), ("old".into(), None, 7, 0)]
                );
                for table in ["scan_missing", "scan_problems"] {
                    let n: i64 = c
                        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                        .unwrap();
                    assert_eq!(n, 0);
                }
            })
            .unwrap();
    }

    #[test]
    fn version_8_adds_documents_passages_and_extraction_state() {
        use crate::services::catalog::writer::CatalogWriter;

        let dir = tempfile::tempdir().unwrap();
        let writer = CatalogWriter::open(&dir.path().join("catalog.sqlite")).unwrap();
        writer
            .call(|c| {
                assert_eq!(migrate_to(c, 7, "t").unwrap(), 7);
                c.execute_batch(
                    "INSERT INTO workspace (id, name, created_at) VALUES ('w', 'W', 't');
                     INSERT INTO source_roots (id, workspace_id, path_bytes, display_path, created_at)
                         VALUES ('r', 'w', x'01', '/books', 't');
                     INSERT INTO file_instances (id, source_root_id, rel_path, display_path,
                         display_name, extension, file_type, availability, created_at, updated_at)
                         VALUES ('f1', 'r', x'61', '/books/a.pdf', 'a.pdf', 'pdf', 'pdf',
                                 'available', 't', 't');",
                )
                .unwrap();

                assert_eq!(migrate_to(c, 8, "t2").unwrap(), 8);

                // Existing files wait for extraction; nothing links them yet.
                let (state, doc, attempts): (String, Option<String>, i64) = c
                    .query_row(
                        "SELECT extract_state, document_id, extract_attempts FROM file_instances",
                        [],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .unwrap();
                assert_eq!((state.as_str(), doc, attempts), ("pending", None, 0));

                // New files can still be cataloged (a REFERENCES to a table
                // created later would make every insert fail).
                c.execute_batch(
                    "INSERT INTO file_instances (id, source_root_id, rel_path, display_path,
                         display_name, extension, file_type, availability, created_at, updated_at)
                         VALUES ('f2', 'r', x'62', '/books/b.pdf', 'b.pdf', 'pdf', 'pdf',
                                 'available', 't', 't');
                     INSERT INTO documents (id, content_hash, state, created_at, updated_at)
                         VALUES ('d', x'00ff', 'done', 't', 't');
                     INSERT INTO passages (document_id, ordinal, page_no, location, text)
                         VALUES ('d', 0, 1, 'Page 1', 'hello'), ('d', 1, 2, 'Page 2', 'world');
                     UPDATE file_instances SET document_id = 'd', extract_state = 'linked';",
                )
                .unwrap();
                assert!(
                    c.execute(
                        "UPDATE file_instances SET extract_state = 'nonsense' WHERE id = 'f1'",
                        []
                    )
                    .is_err(),
                    "extract_state is checked"
                );
                assert!(
                    c.execute(
                        "INSERT INTO documents (id, content_hash, state, created_at, updated_at)
                         VALUES ('d2', x'00ff', 'done', 't', 't')",
                        []
                    )
                    .is_err(),
                    "one document per content hash"
                );
                let violations = c
                    .prepare("PRAGMA foreign_key_check")
                    .unwrap()
                    .query_map([], |_| Ok(()))
                    .unwrap()
                    .count();
                assert_eq!(violations, 0);

                // A document that files still point at can't be deleted; once
                // they're gone, its passages go with it.
                assert!(c.execute("DELETE FROM documents WHERE id = 'd'", []).is_err());
                c.execute("DELETE FROM source_roots WHERE id = 'r'", []).unwrap();
                c.execute("DELETE FROM documents WHERE id = 'd'", []).unwrap();
                let left: i64 = c
                    .query_row("SELECT COUNT(*) FROM passages", [], |r| r.get(0))
                    .unwrap();
                assert_eq!(left, 0);
            })
            .unwrap();
    }

    #[test]
    fn version_7_turns_scan_sessions_into_jobs() {
        use crate::services::catalog::writer::CatalogWriter;

        let dir = tempfile::tempdir().unwrap();
        let writer = CatalogWriter::open(&dir.path().join("catalog.sqlite")).unwrap();
        writer
            .call(|c| {
                assert_eq!(migrate_to(c, 6, "t").unwrap(), 6);
                c.execute_batch(
                    "INSERT INTO workspace (id, name, created_at) VALUES ('w', 'W', 't');
                     INSERT INTO source_roots (id, workspace_id, path_bytes, display_path, created_at)
                         VALUES ('r', 'w', x'01', '/books', 't');
                     INSERT INTO scan_sessions
                         (id, source_root_id, state, created_at, updated_at, discovered_files)
                         VALUES ('s', 'r', 'paused_user', 't', 't', 42);
                     INSERT INTO scan_directories (session_id, rel_path, state, attempts)
                         VALUES ('s', x'01', 'done', 0), ('s', x'0161', 'pending', 1);",
                )
                .unwrap();

                assert_eq!(migrate_to(c, 7, "t2").unwrap(), 7);

                let (kind, state, found): (String, String, i64) = c
                    .query_row(
                        "SELECT kind, state, discovered_files FROM jobs WHERE id = 's'",
                        [],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .unwrap();
                assert_eq!((kind.as_str(), state.as_str(), found), ("scan", "paused_user", 42));
                let frontier: Vec<(String, u32)> = c
                    .prepare("SELECT state, attempts FROM scan_directories ORDER BY id")
                    .unwrap()
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .unwrap()
                    .collect::<Result<_, _>>()
                    .unwrap();
                assert_eq!(frontier, [("done".into(), 0), ("pending".into(), 1)]);

                let parent: String = c
                    .query_row(
                        "SELECT \"table\" FROM pragma_foreign_key_list('scan_directories')",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert_eq!(parent, "jobs");
                let violations = c
                    .prepare("PRAGMA foreign_key_check")
                    .unwrap()
                    .query_map([], |_| Ok(()))
                    .unwrap()
                    .count();
                assert_eq!(violations, 0);
                let legacy: i64 = c
                    .query_row("PRAGMA legacy_alter_table", [], |r| r.get(0))
                    .unwrap();
                assert_eq!(legacy, 0);
                let old_table: i64 = c
                    .query_row(
                        "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE 'scan_sessions%'",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert_eq!(old_table, 0, "no table or index keeps the old name");

                // The frontier still cascades from its job.
                c.execute("DELETE FROM source_roots WHERE id = 'r'", []).unwrap();
                let left: i64 = c
                    .query_row(
                        "SELECT (SELECT COUNT(*) FROM jobs) + (SELECT COUNT(*) FROM scan_directories)",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert_eq!(left, 0);
            })
            .unwrap();
    }

    #[test]
    fn migrate_to_stops_at_the_target() {
        let mut conn = Connection::open_in_memory().unwrap();
        assert_eq!(migrate_to(&mut conn, 3, "t").unwrap(), 3);
        assert_eq!(migrate(&mut conn, "t").unwrap(), SUPPORTED_SCHEMA_VERSION);
    }

    #[test]
    fn newer_database_is_refused_without_changes() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn, "t").unwrap();
        conn.execute(
            "INSERT INTO schema_migrations VALUES (999, 'future', 't')",
            [],
        )
        .unwrap();
        let err = migrate(&mut conn, "t").unwrap_err();
        assert!(matches!(
            err,
            AtlasError::DataRootIncompatible { found: 999, .. }
        ));
    }
}
