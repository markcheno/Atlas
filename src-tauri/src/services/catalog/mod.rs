//! SQLite catalog: the single writer thread, versioned migrations, and
//! (from Slice 0.3) the built-in `CatalogRepository` adapter.
//!
//! Slice 0.2 ships the writer, migrations, and the `workspace` table.
//! Source roots arrive in 0.3, scan sessions and the directory frontier in
//! 0.5, read-only query connections in 0.6.

pub mod migrations;
pub mod repository;
pub mod writer;

use std::path::Path;

pub use migrations::SUPPORTED_SCHEMA_VERSION;
pub use writer::CatalogWriter;

use crate::error::{AtlasError, AtlasResult};

/// Opens the catalog at `path`, refusing a newer schema before anything is
/// written, then enables WAL and applies pending migrations.
/// Returns the writer and the resulting schema version.
pub fn open(path: &Path, now: String) -> AtlasResult<(CatalogWriter, u32)> {
    let writer = CatalogWriter::open(path)?;
    let found = writer.call(|conn| migrations::current_version(conn))??;
    if found > SUPPORTED_SCHEMA_VERSION {
        return Err(AtlasError::DataRootIncompatible {
            found,
            supported: SUPPORTED_SCHEMA_VERSION,
        });
    }
    writer.call(writer::enable_wal)??;
    let version = writer.call(move |conn| migrations::migrate(conn, &now))??;
    Ok((writer, version))
}

/// Inserts the local workspace row if the catalog has none yet, and returns
/// the workspace ID, which is also the data root's ID. Reusing an existing
/// row keeps the ID stable when an interrupted creation is resumed.
pub fn ensure_workspace(
    writer: &CatalogWriter,
    candidate_id: String,
    name: String,
    now: String,
) -> AtlasResult<String> {
    writer.call(move |conn| {
        conn.execute(
            "INSERT INTO workspace (id, name, created_at)
             SELECT ?1, ?2, ?3 WHERE NOT EXISTS (SELECT 1 FROM workspace)",
            rusqlite::params![candidate_id, name, now],
        )
        .map_err(AtlasError::db("record the workspace"))?;
        conn.query_row(
            "SELECT id FROM workspace ORDER BY created_at LIMIT 1",
            [],
            |row| row.get(0),
        )
        .map_err(AtlasError::db("read the workspace"))
    })?
}
