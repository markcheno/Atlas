//! The knowledge store's migrations (`migrations/knowledge/`, phase3-spec
//! §6.1). Its version is `PRAGMA user_version`, read before anything is
//! written; a store from a newer Atlas is refused untouched. Each
//! migration runs in one transaction with the version it sets.

use rusqlite::Connection;

pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

pub const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    name: "knowledge",
    sql: include_str!("../../../migrations/knowledge/0001_knowledge.sql"),
}];

/// Highest knowledge store version this build can read and write.
pub const SUPPORTED_KNOWLEDGE_VERSION: u32 = MIGRATIONS[MIGRATIONS.len() - 1].version;

/// The store's version, without writing anything: 0 for a new file.
pub fn current_version(conn: &Connection) -> rusqlite::Result<u32> {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
}

/// Whether the file has any tables, indexes or triggers: a version 0 file
/// that has some is not a store Atlas made, and is left alone.
pub fn has_objects(conn: &Connection) -> rusqlite::Result<bool> {
    conn.query_row("SELECT EXISTS (SELECT 1 FROM sqlite_master)", [], |r| {
        r.get(0)
    })
}

/// Applies every pending migration. Returns the resulting version. The
/// caller has already refused a newer store.
pub fn migrate(conn: &mut Connection, now: &str) -> rusqlite::Result<u32> {
    let current = current_version(conn)?;
    for m in MIGRATIONS.iter().filter(|m| m.version > current) {
        let tx = conn.transaction()?;
        tx.execute_batch(m.sql)?;
        if m.version == 1 {
            tx.execute(
                "INSERT INTO meta (key, value) VALUES ('created_at', ?1)",
                [now],
            )?;
        }
        tx.pragma_update(None, "user_version", m.version)?;
        tx.commit()?;
    }
    current_version(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLES: &[&str] = &[
        "meta",
        "trails",
        "trail_steps",
        "step_documents",
        "bookmarks",
        "findings",
        "collections",
        "collection_items",
        "dismissed_suggestions",
    ];

    #[test]
    fn versions_are_contiguous_from_one() {
        for (i, m) in MIGRATIONS.iter().enumerate() {
            assert_eq!(m.version as usize, i + 1, "{}", m.name);
        }
    }

    #[test]
    fn a_new_store_gets_every_table_and_migrating_again_changes_nothing() {
        let mut conn = Connection::open_in_memory().unwrap();
        assert_eq!(current_version(&conn).unwrap(), 0);
        assert!(!has_objects(&conn).unwrap());
        assert_eq!(
            migrate(&mut conn, "2026-11-11T00:00:00Z").unwrap(),
            SUPPORTED_KNOWLEDGE_VERSION
        );
        for t in TABLES {
            let n: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [t],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 1, "{t}");
        }
        let created: String = conn
            .query_row("SELECT value FROM meta WHERE key = 'created_at'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(created, "2026-11-11T00:00:00Z");
        assert_eq!(
            migrate(&mut conn, "later").unwrap(),
            SUPPORTED_KNOWLEDGE_VERSION
        );
        let created_rows: i64 = conn
            .query_row(
                "SELECT count(*) FROM meta WHERE key = 'created_at'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(created_rows, 1);
    }

    #[test]
    fn every_write_to_a_user_table_counts_as_a_change_and_meta_does_not() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        migrate(&mut conn, "t").unwrap();
        let changes = |c: &Connection| -> i64 {
            c.query_row(
                "SELECT CAST(value AS INTEGER) FROM meta WHERE key = 'changes'",
                [],
                |r| r.get(0),
            )
            .unwrap()
        };
        assert_eq!(changes(&conn), 0);
        conn.execute_batch(
            "INSERT INTO trails (id, started_at, last_step_at) VALUES ('t', 'a', 'a');
             INSERT INTO trail_steps (id, trail_id, seq, kind, at, payload)
                 VALUES ('s', 't', 1, 'note', 'a', '{}');
             UPDATE trails SET pinned = 1;
             INSERT INTO meta (key, value) VALUES ('last_snapshot_at', 'x');",
        )
        .unwrap();
        assert_eq!(changes(&conn), 3);
        // A cascade is a write too.
        conn.execute("DELETE FROM trails", []).unwrap();
        assert_eq!(changes(&conn), 5);
    }
}
