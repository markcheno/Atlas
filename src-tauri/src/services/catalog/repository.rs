//! SQLite implementation of [`CatalogRepository`].

use rusqlite::{ErrorCode as SqliteCode, params};

use super::writer::CatalogHandle;
use crate::domain::dto::{DocumentPage, DocumentQuery, FileInstance};
use crate::domain::traits::{CatalogRepository, SourceRootRecord};
use crate::error::{AtlasError, AtlasResult};

pub struct SqliteCatalog<'a> {
    writer: &'a CatalogHandle,
}

impl<'a> SqliteCatalog<'a> {
    pub fn new(writer: &'a CatalogHandle) -> Self {
        Self { writer }
    }
}

impl CatalogRepository for SqliteCatalog<'_> {
    fn list_source_root_records(&self) -> AtlasResult<Vec<SourceRootRecord>> {
        self.writer.call(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, workspace_id, path_bytes, display_path, created_at, last_scan_at
                     FROM source_roots ORDER BY display_path, id",
                )
                .map_err(AtlasError::db("list library folders"))?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(SourceRootRecord {
                        id: row.get(0)?,
                        workspace_id: row.get(1)?,
                        path_bytes: row.get(2)?,
                        display_path: row.get(3)?,
                        created_at: row.get(4)?,
                        last_scan_at: row.get(5)?,
                    })
                })
                .map_err(AtlasError::db("list library folders"))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(AtlasError::db("list library folders"))
        })?
    }

    fn insert_source_root(&self, record: SourceRootRecord) -> AtlasResult<()> {
        self.writer.call(move |conn| {
            conn.execute(
                "INSERT INTO source_roots
                     (id, workspace_id, path_bytes, display_path, created_at, last_scan_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    record.id,
                    record.workspace_id,
                    record.path_bytes,
                    record.display_path,
                    record.created_at,
                    record.last_scan_at,
                ],
            )
            .map(|_| ())
            .map_err(|err| match err.sqlite_error_code() {
                Some(SqliteCode::ConstraintViolation) => AtlasError::SourceAlreadyAdded {
                    existing_root_id: None,
                },
                _ => AtlasError::db("save the library folder")(err),
            })
        })?
    }

    fn delete_source_root(&self, id: &str) -> AtlasResult<bool> {
        let id = id.to_owned();
        self.writer.call(move |conn| {
            conn.execute("DELETE FROM source_roots WHERE id = ?1", [id])
                .map(|n| n > 0)
                .map_err(AtlasError::db("remove the library folder"))
        })?
    }

    fn list_documents(&self, _query: &DocumentQuery) -> AtlasResult<DocumentPage> {
        Err(AtlasError::not_implemented("list_documents", "0.6"))
    }

    fn file_instance(&self, _id: &str) -> AtlasResult<Option<FileInstance>> {
        Err(AtlasError::not_implemented("open_original", "0.6"))
    }
}
