//! The knowledge store's `meta` table: one text value per key. Writing it
//! counts as no change (only the user tables' triggers bump `changes`).
//!
//! Keys: `changes`, `created_at`, `last_snapshot_at`,
//! `last_snapshot_change` (Slice 3.2); `current_trail`,
//! `current_trail_until`, `first_step_noted`, `last_expiry_at` (Slice 3.3).

use rusqlite::{Connection, OptionalExtension};

pub fn get(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .optional()
}

pub fn get_int(conn: &Connection, key: &str) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT CAST(value AS INTEGER) FROM meta WHERE key = ?1",
        [key],
        |r| r.get(0),
    )
    .optional()
}

pub fn set(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        [key, value],
    )
    .map(drop)
}

pub fn remove(conn: &Connection, key: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM meta WHERE key = ?1", [key])
        .map(drop)
}
