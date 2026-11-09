//! Document details for the panel (phase1-spec §5.3): a file, its document
//! (title, authors, language, pages) and the other copies of its content.
//! Read on the read-only pool.

use rusqlite::{Connection, OptionalExtension, Row};

use crate::domain::dto::{
    Availability, DocumentDetail, ExtractionState, FileExtraction, FileInstance, FileStorage,
    FileType, SourceRootStatus,
};
use crate::error::{AtlasError, AtlasResult};
use crate::services::catalog::codec::from_db;
use crate::services::catalog::reader::CatalogReader;
use crate::services::extraction::db::{EFFECTIVE_REASON_SQL, EFFECTIVE_STATE_SQL};
use crate::services::library::RootStatusCache;

fn columns() -> String {
    format!(
        "f.id, f.source_root_id, f.display_name, f.display_path, f.extension, f.file_type,
         f.size_bytes, f.modified_at, f.availability, f.storage, f.document_id,
         {EFFECTIVE_STATE_SQL}, {EFFECTIVE_REASON_SQL}, r.path_bytes"
    )
}

/// A file instance and its folder's path bytes (for reachability).
fn file_from_row(r: &Row) -> rusqlite::Result<(FileInstance, Vec<u8>)> {
    let file_type: String = r.get(5)?;
    let availability: String = r.get(8)?;
    let storage: String = r.get(9)?;
    let state: String = r.get(11)?;
    Ok((
        FileInstance {
            id: r.get(0)?,
            source_root_id: r.get(1)?,
            display_name: r.get(2)?,
            display_path: r.get(3)?,
            extension: r.get(4)?,
            file_type: from_db(&file_type).unwrap_or(FileType::Pdf),
            size_bytes: r.get::<_, Option<i64>>(6)?.map(|s| s.max(0) as u64),
            modified_at: r.get(7)?,
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
            document_id: r.get(10)?,
            extraction: FileExtraction {
                state: from_db(&state).unwrap_or(ExtractionState::Pending),
                reason: r.get(12)?,
            },
        },
        r.get(13)?,
    ))
}

type Loaded = (DocumentDetail, Vec<(String, Vec<u8>)>);

struct Meta {
    title: Option<String>,
    authors: Option<String>,
    language: Option<String>,
    pages: Option<u32>,
    chars: Option<i64>,
}

fn load(conn: &Connection, file_id: &str) -> rusqlite::Result<Option<Loaded>> {
    let from = "FROM file_instances f JOIN source_roots r ON r.id = f.source_root_id
                LEFT JOIN documents d ON d.id = f.document_id";
    let Some((file, path)) = conn
        .query_row(
            &format!("SELECT {} {from} WHERE f.id = ?1", columns()),
            [file_id],
            file_from_row,
        )
        .optional()?
    else {
        return Ok(None);
    };
    let mut roots = vec![(file.source_root_id.clone(), path)];
    let mut detail = DocumentDetail {
        document_id: file.document_id.clone(),
        title: None,
        authors: Vec::new(),
        language: None,
        page_count: None,
        char_count: None,
        also_at: Vec::new(),
        file,
    };
    if let Some(doc) = detail.document_id.clone() {
        let meta = conn.query_row(
            "SELECT title, authors, language, page_count, char_count FROM documents WHERE id = ?1",
            [&doc],
            |r| {
                Ok(Meta {
                    title: r.get(0)?,
                    authors: r.get(1)?,
                    language: r.get(2)?,
                    pages: r.get(3)?,
                    chars: r.get(4)?,
                })
            },
        )?;
        let Meta {
            title,
            authors,
            language,
            pages,
            chars,
        } = meta;
        detail.title = title;
        detail.authors = authors
            .and_then(|a| serde_json::from_str(&a).ok())
            .unwrap_or_default();
        detail.language = language;
        detail.page_count = pages;
        detail.char_count = chars.map(|c| c.max(0) as u64);
        let mut stmt = conn.prepare(&format!(
            "SELECT {} {from} WHERE f.document_id = ?1 AND f.id <> ?2
             ORDER BY f.availability = 'available' DESC, f.display_path",
            columns()
        ))?;
        for row in stmt.query_map([&doc, file_id], file_from_row)? {
            let (copy, path) = row?;
            if !roots.iter().any(|(id, _)| *id == copy.source_root_id) {
                roots.push((copy.source_root_id.clone(), path));
            }
            detail.also_at.push(copy);
        }
    }
    Ok(Some((detail, roots)))
}

/// The detail panel's data for one file. Files whose folder is not
/// connected are reported `unreachable`, as in the Library.
pub fn detail(
    reader: &CatalogReader,
    roots: &RootStatusCache,
    file_id: &str,
) -> AtlasResult<DocumentDetail> {
    let (mut detail, root_paths) = reader
        .read(|c| load(c, file_id))?
        .map_err(AtlasError::db("read document details"))?
        .ok_or(AtlasError::FileNotFound)?;
    let statuses = roots.statuses(&root_paths);
    let mark = |f: &mut FileInstance| {
        let reachable = statuses
            .get(&f.source_root_id)
            .is_none_or(|s| *s == SourceRootStatus::Available);
        if !reachable && f.availability == Availability::Available {
            f.availability = Availability::Unreachable;
        }
    };
    mark(&mut detail.file);
    detail.also_at.iter_mut().for_each(mark);
    Ok(detail)
}
