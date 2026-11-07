//! Versioned, append-only catalog migrations (see `migrations/README.md`).

use rusqlite::{Connection, OptionalExtension};

use crate::error::{AtlasError, AtlasResult};

pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

pub const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "initial",
    sql: include_str!("../../../migrations/0001_initial.sql"),
}];

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
    let current = current_version(conn)?;
    if current > SUPPORTED_SCHEMA_VERSION {
        return Err(AtlasError::DataRootIncompatible {
            found: current,
            supported: SUPPORTED_SCHEMA_VERSION,
        });
    }
    conn.execute_batch(CREATE_TRACKING_TABLE)
        .map_err(AtlasError::db("be prepared for updates"))?;

    for migration in MIGRATIONS.iter().filter(|m| m.version > current) {
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
