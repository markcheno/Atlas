//! Browsing the catalog: paginated filename/path search with filters and
//! sorting (phase0-spec §5.5, ADR 0006). Reads use the read-only pool, so
//! they never wait behind inventory batches.

pub mod fold;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rusqlite::types::Value;
use rusqlite::{Connection, params_from_iter};

use crate::domain::dto::{
    Availability, DocumentPage, DocumentQuery, DocumentSort, FileInstance, FileStorage,
    SourceRootStatus,
};
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::native_path;
use crate::services::catalog::reader::CatalogReader;
use crate::services::catalog::writer::CatalogHandle;
use crate::services::inventory::db::{ACTIVE_STATES, from_db, to_db};
use crate::services::sources::probe;
use fold::{fold, like_escape};

pub const MAX_PAGE_SIZE: u32 = 200;
const STATUS_TTL: Duration = Duration::from_secs(5);

/// Source-root reachability, cached briefly so listing many pages costs at
/// most one bounded probe per root every few seconds.
#[derive(Default)]
pub struct RootStatusCache {
    entries: Mutex<HashMap<String, (Instant, SourceRootStatus)>>,
}

impl RootStatusCache {
    pub fn statuses(&self, roots: &[(String, Vec<u8>)]) -> HashMap<String, SourceRootStatus> {
        let mut out = HashMap::new();
        let mut stale = Vec::new();
        {
            let entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
            for (id, path) in roots {
                match entries.get(id) {
                    Some((at, status)) if at.elapsed() < STATUS_TTL => {
                        out.insert(id.clone(), *status);
                    }
                    _ => stale.push((id.clone(), native_path::decode(path))),
                }
            }
        }
        if !stale.is_empty() {
            let paths: Vec<Option<PathBuf>> = stale.iter().map(|(_, p)| p.clone()).collect();
            let fresh = probe::probe_all(&paths);
            let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
            for ((id, _), status) in stale.into_iter().zip(fresh) {
                entries.insert(id.clone(), (Instant::now(), status));
                out.insert(id, status);
            }
        }
        out
    }
}

struct Filter {
    clause: String,
    params: Vec<Value>,
}

fn filter(q: &DocumentQuery) -> Filter {
    let mut conditions = Vec::new();
    let mut params = Vec::new();
    if !q.include_missing {
        conditions.push("f.availability = 'available'".to_owned());
    }
    if let Some(root) = &q.source_root_id {
        conditions.push("f.source_root_id = ?".to_owned());
        params.push(Value::Text(root.clone()));
    }
    if let Some(kind) = q.file_type {
        conditions.push("f.file_type = ?".to_owned());
        params.push(Value::Text(to_db(&kind)));
    }
    if let Some(text) = &q.text {
        for term in fold(text).split_whitespace() {
            conditions.push("f.search_key LIKE ? ESCAPE '\\'".to_owned());
            params.push(Value::Text(format!("%{}%", like_escape(term))));
        }
    }
    let clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };
    Filter { clause, params }
}

fn order_by(sort: DocumentSort) -> &'static str {
    match sort {
        DocumentSort::NameAsc => "f.name_key ASC, f.rowid ASC",
        DocumentSort::NameDesc => "f.name_key DESC, f.rowid DESC",
        DocumentSort::ModifiedDesc => "f.mtime_secs IS NULL, f.mtime_secs DESC, f.name_key ASC",
        DocumentSort::ModifiedAsc => "f.mtime_secs IS NULL, f.mtime_secs ASC, f.name_key ASC",
    }
}

struct Row {
    instance: FileInstance,
    root_path: Vec<u8>,
}

fn query_page(conn: &Connection, q: &DocumentQuery) -> rusqlite::Result<(Vec<Row>, u32, bool)> {
    let f = filter(q);
    let matched: u32 = conn.query_row(
        &format!("SELECT COUNT(*) FROM file_instances f {}", f.clause),
        params_from_iter(f.params.iter()),
        |r| r.get(0),
    )?;

    let page_size = q.page_size.clamp(1, MAX_PAGE_SIZE);
    let offset = u64::from(q.page) * u64::from(page_size);
    let mut params = f.params.clone();
    params.push(Value::Integer(i64::from(page_size)));
    params.push(Value::Integer(offset as i64));
    let mut stmt = conn.prepare(&format!(
        "SELECT f.id, f.source_root_id, r.path_bytes, f.display_name, f.display_path,
                f.extension, f.file_type, f.size_bytes, f.modified_at, f.availability, f.storage
         FROM file_instances f JOIN source_roots r ON r.id = f.source_root_id
         {} ORDER BY {} LIMIT ? OFFSET ?",
        f.clause,
        order_by(q.sort)
    ))?;
    let rows = stmt
        .query_map(params_from_iter(params.iter()), |r| {
            let file_type: String = r.get(6)?;
            let availability: String = r.get(9)?;
            let storage: String = r.get(10)?;
            Ok(Row {
                root_path: r.get(2)?,
                instance: FileInstance {
                    id: r.get(0)?,
                    source_root_id: r.get(1)?,
                    display_name: r.get(3)?,
                    display_path: r.get(4)?,
                    extension: r.get(5)?,
                    file_type: from_db(&file_type).unwrap_or(crate::domain::dto::FileType::Pdf),
                    size_bytes: r.get::<_, Option<i64>>(7)?.map(|s| s.max(0) as u64),
                    modified_at: r.get(8)?,
                    availability: if availability == "missing" {
                        Availability::Missing
                    } else {
                        Availability::Available
                    },
                    storage: if storage == "cloud_only" {
                        FileStorage::CloudOnly
                    } else {
                        FileStorage::Local
                    },
                },
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let active = ACTIVE_STATES
        .iter()
        .map(|s| format!("'{}'", to_db(s)))
        .collect::<Vec<_>>()
        .join(", ");
    let scanning: bool = conn.query_row(
        &format!("SELECT EXISTS (SELECT 1 FROM scan_sessions WHERE state IN ({active}))"),
        [],
        |r| r.get(0),
    )?;
    Ok((rows, matched, scanning))
}

/// One page of cataloged files. Files whose folder is currently not
/// reachable are reported `unreachable`.
pub fn list(
    reader: &CatalogReader,
    roots: &RootStatusCache,
    q: &DocumentQuery,
) -> AtlasResult<DocumentPage> {
    let (rows, matched, scanning) = reader
        .read(|c| query_page(c, q))?
        .map_err(AtlasError::db("list files"))?;

    let mut distinct: Vec<(String, Vec<u8>)> = Vec::new();
    for row in &rows {
        if !distinct
            .iter()
            .any(|(id, _)| id == &row.instance.source_root_id)
        {
            distinct.push((row.instance.source_root_id.clone(), row.root_path.clone()));
        }
    }
    let statuses = roots.statuses(&distinct);
    let items = rows
        .into_iter()
        .map(|row| {
            let mut item = row.instance;
            let reachable = statuses
                .get(&item.source_root_id)
                .is_none_or(|s| *s == SourceRootStatus::Available);
            if !reachable && item.availability == Availability::Available {
                item.availability = Availability::Unreachable;
            }
            item
        })
        .collect();

    Ok(DocumentPage {
        items,
        page: q.page,
        page_size: q.page_size.clamp(1, MAX_PAGE_SIZE),
        matched_so_far: matched,
        scanning,
    })
}

/// Fills search keys for rows cataloged before keys existed (Slice 0.5).
/// Idempotent; runs in batches on the writer.
pub fn backfill_keys(writer: &CatalogHandle) -> AtlasResult<usize> {
    let mut total = 0;
    loop {
        let updated = writer
            .call(|c| -> rusqlite::Result<usize> {
                let tx = c.transaction()?;
                let rows: Vec<(String, Vec<u8>, String)> = {
                    let mut stmt = tx.prepare(
                        "SELECT id, rel_path, display_name FROM file_instances
                         WHERE search_key IS NULL LIMIT 1000",
                    )?;
                    stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                        .collect::<rusqlite::Result<_>>()?
                };
                {
                    let mut update = tx.prepare(
                        "UPDATE file_instances SET search_key = ?2, name_key = ?3 WHERE id = ?1",
                    )?;
                    for (id, rel, name) in &rows {
                        let rel_display = native_path::decode(rel)
                            .map(|p| native_path::display(&p))
                            .unwrap_or_else(|| name.clone());
                        update.execute(rusqlite::params![id, fold(&rel_display), fold(name)])?;
                    }
                }
                tx.commit()?;
                Ok(rows.len())
            })?
            .map_err(AtlasError::db("prepare search"))?;
        total += updated;
        if updated == 0 {
            return Ok(total);
        }
    }
}
