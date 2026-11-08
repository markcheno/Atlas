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

                assert_eq!(migrate(c, "t2").unwrap(), 7);

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
