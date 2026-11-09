//! Duplicate groups (Library › Duplicates, Slice 1.6): documents with at
//! least two available copies inside the filters, most extra space first.
//! Copies are found by content (BLAKE3), so a file joins a group once it
//! has been hashed. Atlas never deletes or moves a copy.

use rusqlite::types::Value;
use rusqlite::{Connection, params_from_iter};

use crate::domain::dto::{DuplicateGroup, DuplicatePage, DuplicateQuery, FileInstance};
use crate::error::{AtlasError, AtlasResult};
use crate::services::catalog::codec::to_db;
use crate::services::catalog::reader::CatalogReader;
use crate::services::library::RootStatusCache;
use crate::services::library::rows::{self, Roots};

use super::metadata;

pub const MAX_PAGE_SIZE: u32 = 100;

/// The folder and type conditions on `f`, with their parameters.
fn scope(q: &DuplicateQuery) -> (String, Vec<Value>) {
    let mut clause = String::new();
    let mut params = Vec::new();
    if let Some(root) = &q.source_root_id {
        clause.push_str(" AND f.source_root_id = ?");
        params.push(Value::Text(root.clone()));
    }
    if let Some(kind) = q.file_type {
        clause.push_str(" AND f.file_type = ?");
        params.push(Value::Text(to_db(&kind)));
    }
    (clause, params)
}

struct Group {
    document: String,
    size: Option<i64>,
    extra: i64,
    title: Option<String>,
    content_title: Option<String>,
}

fn load(conn: &Connection, q: &DuplicateQuery) -> rusqlite::Result<(DuplicatePage, Roots)> {
    let (clause, params) = scope(q);
    let groups_sql = format!(
        "WITH g AS (
             SELECT f.document_id AS doc, COUNT(*) AS n, MAX(f.size_bytes) AS size
             FROM file_instances f
             WHERE f.document_id IS NOT NULL AND f.availability = 'available'{clause}
             GROUP BY f.document_id HAVING COUNT(*) > 1)"
    );
    let (total, extra): (u32, i64) = conn.query_row(
        &format!(
            "{groups_sql} SELECT COUNT(*), COALESCE(SUM((n - 1) * COALESCE(size, 0)), 0) FROM g"
        ),
        params_from_iter(params.iter()),
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;

    let page_size = q.page_size.clamp(1, MAX_PAGE_SIZE);
    let mut page_params = params.clone();
    page_params.push(Value::Integer(i64::from(page_size)));
    page_params.push(Value::Integer(i64::from(q.page) * i64::from(page_size)));
    let groups: Vec<Group> = conn
        .prepare(&format!(
            "{groups_sql}
             SELECT g.doc, g.size, (g.n - 1) * COALESCE(g.size, 0) AS extra,
                    d.title, d.content_title
             FROM g JOIN documents d ON d.id = g.doc
             ORDER BY extra DESC, d.title COLLATE NOCASE, g.doc
             LIMIT ? OFFSET ?"
        ))?
        .query_map(params_from_iter(page_params.iter()), |r| {
            Ok(Group {
                document: r.get(0)?,
                size: r.get(1)?,
                extra: r.get(2)?,
                title: r.get(3)?,
                content_title: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;

    let shown = if q.include_missing {
        "f.availability IN ('available', 'missing')"
    } else {
        "f.availability = 'available'"
    };
    let mut copies_stmt = conn.prepare(&format!(
        "SELECT {} {} WHERE f.document_id = ? AND {shown}{clause}
         ORDER BY f.availability = 'available' DESC, f.display_path",
        rows::file_columns(),
        rows::FILE_FROM,
    ))?;
    let mut roots = Roots::default();
    let mut out = Vec::with_capacity(groups.len());
    for g in groups {
        let mut copy_params = vec![Value::Text(g.document.clone())];
        copy_params.extend(params.iter().cloned());
        let mut copies: Vec<FileInstance> = Vec::new();
        for row in
            copies_stmt.query_map(params_from_iter(copy_params.iter()), rows::file_from_row)?
        {
            let (file, path) = row?;
            roots.add(&file, path);
            copies.push(file);
        }
        let name = copies.first().map_or("", |f| f.display_name.as_str());
        let (title, title_source) =
            metadata::display_title(g.title.as_deref(), g.content_title.as_deref(), name);
        out.push(DuplicateGroup {
            document_id: g.document,
            title,
            title_source,
            size_bytes: g.size.map(|s| s.max(0) as u64),
            extra_bytes: g.extra.max(0) as u64,
            copies,
        });
    }
    Ok((
        DuplicatePage {
            groups: out,
            page: q.page,
            page_size,
            total_groups: total,
            extra_bytes: extra.max(0) as u64,
        },
        roots,
    ))
}

/// One page of duplicate groups. Copies whose folder is not connected are
/// reported `unreachable`, as in the Library.
pub fn list(
    reader: &CatalogReader,
    roots: &RootStatusCache,
    q: &DuplicateQuery,
) -> AtlasResult<DuplicatePage> {
    let (mut page, folders) = reader
        .read(|c| load(c, q))?
        .map_err(AtlasError::db("list duplicates"))?;
    let reach = folders.probe(roots);
    for group in &mut page.groups {
        group.copies.iter_mut().for_each(|f| reach.mark(f));
    }
    Ok(page)
}
