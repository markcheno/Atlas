//! Document details for the panel (phase1-spec §5.3): a file, its document
//! (title, authors, language, pages) and the other copies of its content.
//! Read on the read-only pool.

pub mod duplicates;
pub mod metadata;
pub mod rediscover;
pub mod related;

use rusqlite::{Connection, OptionalExtension};

use crate::domain::dto::{DocumentDetail, FileType, FormatPair, FormatPairBasis, FormatPairState};
use crate::error::{AtlasError, AtlasResult};
use crate::services::catalog::reader::CatalogReader;
use crate::services::library::RootStatusCache;
use crate::services::library::rows::{self, Roots};

struct Meta {
    title: Option<String>,
    content_title: Option<String>,
    authors: Option<String>,
    language: Option<String>,
    pages: Option<u32>,
    chars: Option<i64>,
}

fn load(conn: &Connection, file_id: &str) -> rusqlite::Result<Option<(DocumentDetail, Roots)>> {
    let from = rows::FILE_FROM;
    let Some((file, path)) = conn
        .query_row(
            &format!("SELECT {} {from} WHERE f.id = ?1", rows::file_columns()),
            [file_id],
            rows::file_from_row,
        )
        .optional()?
    else {
        return Ok(None);
    };
    let mut roots = Roots::default();
    roots.add(&file, path);
    let (title, title_source) = metadata::display_title(None, None, &file.display_name);
    let mut detail = DocumentDetail {
        document_id: file.document_id.clone(),
        title,
        title_source,
        authors: Vec::new(),
        language: None,
        page_count: None,
        char_count: None,
        also_at: Vec::new(),
        formats: None,
        file,
    };
    if let Some(doc) = detail.document_id.clone() {
        let meta = conn.query_row(
            "SELECT title, content_title, authors, language, page_count, char_count
             FROM documents WHERE id = ?1",
            [&doc],
            |r| {
                Ok(Meta {
                    title: r.get(0)?,
                    content_title: r.get(1)?,
                    authors: r.get(2)?,
                    language: r.get(3)?,
                    pages: r.get(4)?,
                    chars: r.get(5)?,
                })
            },
        )?;
        (detail.title, detail.title_source) = metadata::display_title(
            meta.title.as_deref(),
            meta.content_title.as_deref(),
            &detail.file.display_name,
        );
        detail.authors = metadata::authors_from_json(meta.authors.as_deref());
        detail.language = meta.language.filter(|l| !l.trim().is_empty());
        detail.page_count = meta.pages;
        detail.char_count = meta.chars.map(|c| c.max(0) as u64);
        let mut stmt = conn.prepare(&format!(
            "SELECT {} {from} WHERE f.document_id = ?1 AND f.id <> ?2
             ORDER BY f.availability = 'available' DESC, f.display_path",
            rows::file_columns()
        ))?;
        for row in stmt.query_map([&doc, file_id], rows::file_from_row)? {
            let (copy, path) = row?;
            roots.add(&copy, path);
            detail.also_at.push(copy);
        }
    }
    detail.formats = formats(conn, &detail.file, &mut roots)?;
    Ok(Some((detail, roots)))
}

/// The other format beside a file, and whether the two are one book
/// (Slice 4.5.1).
fn formats(
    conn: &Connection,
    file: &crate::domain::dto::FileInstance,
    roots: &mut Roots,
) -> rusqlite::Result<Option<FormatPair>> {
    let partner_id: Option<String> = conn
        .query_row(
            "SELECT epub_file_id FROM format_pairs WHERE pdf_file_id = ?1
             UNION ALL
             SELECT pdf_file_id FROM format_pairs WHERE epub_file_id = ?1",
            [&file.id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(partner_id) = partner_id else {
        return Ok(None);
    };
    let from = rows::FILE_FROM;
    let Some((partner, path)) = conn
        .query_row(
            &format!("SELECT {} {from} WHERE f.id = ?1", rows::file_columns()),
            [&partner_id],
            rows::file_from_row,
        )
        .optional()?
    else {
        return Ok(None);
    };
    roots.add(&partner, path);
    let (pdf, epub) = if file.file_type == FileType::Pdf {
        (file, &partner)
    } else {
        (&partner, file)
    };
    let verdict: Option<(String, String, Option<String>)> =
        match (&pdf.document_id, &epub.document_id) {
            (Some(p), Some(e)) => conn
                .query_row(
                    "SELECT verdict, basis, index_format FROM book_pairs
                     WHERE pdf_document_id = ?1 AND epub_document_id = ?2",
                    [p, e],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?,
            _ => None,
        };
    let (state, basis, indexed_as) = match verdict {
        None => (FormatPairState::Waiting, None, None),
        Some((verdict, basis, index)) => {
            let same = verdict == "same";
            (
                if same {
                    FormatPairState::Same
                } else {
                    FormatPairState::Different
                },
                Some(match basis.as_str() {
                    "text" => FormatPairBasis::Text,
                    "name" => FormatPairBasis::Name,
                    _ => FormatPairBasis::Owner,
                }),
                index.filter(|_| same).map(|i| {
                    if i == "pdf" {
                        FileType::Pdf
                    } else {
                        FileType::Epub
                    }
                }),
            )
        }
    };
    Ok(Some(FormatPair {
        partner,
        state,
        basis,
        indexed_as,
    }))
}

/// The detail panel's data for one file. Files whose folder is not
/// connected are reported `unreachable`, as in the Library.
pub fn detail(
    reader: &CatalogReader,
    roots: &RootStatusCache,
    file_id: &str,
) -> AtlasResult<DocumentDetail> {
    let (mut detail, folders) = reader
        .read(|c| load(c, file_id))?
        .map_err(AtlasError::db("read document details"))?
        .ok_or(AtlasError::FileNotFound)?;
    let reach = folders.probe(roots);
    reach.mark(&mut detail.file);
    detail.also_at.iter_mut().for_each(|f| reach.mark(f));
    if let Some(pair) = &mut detail.formats {
        reach.mark(&mut pair.partner);
    }
    Ok(detail)
}

/// Whether a stored metadata title is usable (no first-page title or
/// file-name fallback needed).
pub fn has_usable_title(title: Option<&str>) -> bool {
    title.and_then(metadata::clean_title).is_some()
}
