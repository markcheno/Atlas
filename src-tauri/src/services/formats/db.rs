//! The catalog side of one book in two formats (Slice 4.5.1, ADR 0023).
//!
//! `format_pairs` says which files sit side by side with one name; it is
//! rebuilt from the names. `book_pairs` says whether two documents are one
//! book (the text check, the name alone, or the owner), which format is
//! indexed and shown, and, while the book is active, which document is kept
//! out. [`apply`] is the only writer of the active columns.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, params};

use super::compare::{self, CHECK_VERSION, Comparison, Sketch};
use super::names::{self, NamedFile};

/// The pairing pass's version; a folder below it is paired again.
pub const FORMATS_VERSION: i64 = 1;

/// What [`apply`] changed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Changes {
    /// Documents shown again: no longer kept out of a book.
    pub unhidden: Vec<String>,
    /// Documents newly kept out.
    pub hidden: Vec<String>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.unhidden.is_empty() && self.hidden.is_empty()
    }
}

/// Rebuilds a folder's name pairs from its available files. Writes only
/// what changed; returns whether anything did.
pub fn reconcile_root(conn: &Connection, root_id: &str, now: &str) -> rusqlite::Result<bool> {
    let files: Vec<NamedFile> = conn
        .prepare(
            "SELECT id, rel_path, file_type FROM file_instances
             WHERE source_root_id = ?1 AND availability = 'available'",
        )?
        .query_map([root_id], |r| {
            Ok(NamedFile {
                id: r.get(0)?,
                rel_path: r.get(1)?,
                file_type: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let wanted: HashSet<(String, String)> = names::pairs(&files).into_iter().collect();
    let existing: HashSet<(String, String)> = conn
        .prepare(
            "SELECT fp.pdf_file_id, fp.epub_file_id FROM format_pairs fp
             JOIN file_instances f ON f.id = fp.pdf_file_id
             WHERE f.source_root_id = ?1",
        )?
        .query_map([root_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut changed = false;
    for (pdf, _) in existing.difference(&wanted) {
        conn.execute("DELETE FROM format_pairs WHERE pdf_file_id = ?1", [pdf])?;
        changed = true;
    }
    for (pdf, epub) in wanted.difference(&existing) {
        // A file in a stale pair of its own was removed just above.
        conn.execute(
            "DELETE FROM format_pairs WHERE pdf_file_id = ?1 OR epub_file_id = ?2",
            params![pdf, epub],
        )?;
        conn.execute(
            "INSERT INTO format_pairs (pdf_file_id, epub_file_id, created_at) VALUES (?1, ?2, ?3)",
            params![pdf, epub, now],
        )?;
        changed = true;
    }
    Ok(changed)
}

/// Document pairs in a folder that need the text check: both files read,
/// and never checked, checked by an older check, or re-read since. The
/// owner's choices are never checked again.
pub fn pending_checks(
    conn: &Connection,
    root_id: &str,
    limit: usize,
) -> rusqlite::Result<Vec<(String, String)>> {
    conn.prepare(
        "SELECT DISTINCT fpdf.document_id, fe.document_id
         FROM format_pairs fp
         JOIN file_instances fpdf ON fpdf.id = fp.pdf_file_id
         JOIN file_instances fe ON fe.id = fp.epub_file_id
         JOIN documents dp ON dp.id = fpdf.document_id
         JOIN documents de ON de.id = fe.document_id
         LEFT JOIN book_pairs bp
           ON bp.pdf_document_id = dp.id AND bp.epub_document_id = de.id
         WHERE fpdf.source_root_id = ?1
           AND dp.id <> de.id
           AND dp.state NOT IN ('pending', 'running')
           AND de.state NOT IN ('pending', 'running')
           AND (bp.pdf_document_id IS NULL
                OR (bp.basis <> 'owner'
                    AND (bp.check_version < ?2
                         OR bp.pdf_checked_at <> dp.updated_at
                         OR bp.epub_checked_at <> de.updated_at)))
         LIMIT ?3",
    )?
    .query_map(params![root_id, CHECK_VERSION, limit as i64], |r| {
        Ok((r.get(0)?, r.get(1)?))
    })?
    .collect()
}

/// A document's sketch, from its passages (empty unless its text is read).
fn sketch_of(conn: &Connection, document_id: &str) -> rusqlite::Result<(Sketch, String)> {
    let (state, updated): (String, String) = conn.query_row(
        "SELECT state, updated_at FROM documents WHERE id = ?1",
        [document_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let mut sketch = Sketch::default();
    if state == "done" {
        let mut stmt =
            conn.prepare_cached("SELECT text FROM passages WHERE document_id = ?1 ORDER BY ordinal")?;
        let mut rows = stmt.query([document_id])?;
        while let Some(row) = rows.next()? {
            sketch.add(row.get_ref(0)?.as_str()?);
        }
    }
    Ok((sketch, updated))
}

/// Whether a document's meaning is indexed for a model in use.
fn indexed(conn: &Connection, document_id: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM document_embeddings de
                        JOIN embedding_models m ON m.id = de.model_id
                        WHERE de.document_id = ?1 AND de.status = 'done'
                          AND m.status IN ('building', 'ready'))",
        [document_id],
        |r| r.get(0),
    )
}

/// Which format a book is indexed and shown in. A copy already indexed
/// stays the one used, so nothing indexed is done again; otherwise the
/// EPUB, whose text is cleaner, unless only the PDF has text.
fn choose_index(
    conn: &Connection,
    pdf: (&str, bool),
    epub: (&str, bool),
    kept: Option<&str>,
) -> rusqlite::Result<&'static str> {
    let has_text = |format: &str| if format == "pdf" { pdf.1 } else { epub.1 };
    if let Some(kept) = kept
        && has_text(kept)
    {
        return Ok(if kept == "pdf" { "pdf" } else { "epub" });
    }
    let (pdf_done, epub_done) = (indexed(conn, pdf.0)?, indexed(conn, epub.0)?);
    Ok(match (epub_done, pdf_done) {
        (true, _) => "epub",
        (false, true) => "pdf",
        _ if epub.1 => "epub",
        _ if pdf.1 => "pdf",
        _ => "epub",
    })
}

/// The text check for one pair, written to `book_pairs`. With text on both
/// sides the texts decide: their samples when both are long enough to
/// judge, else their lengths (a sample chapter beside its book is two
/// books). With no text on one side (a scan, a locked file), the name alone
/// pairs them, and the side with text is the one used. Returns the verdict.
pub fn check_pair(
    conn: &Connection,
    pdf_document_id: &str,
    epub_document_id: &str,
    now: &str,
) -> rusqlite::Result<bool> {
    let (pdf, pdf_at) = sketch_of(conn, pdf_document_id)?;
    let (epub, epub_at) = sketch_of(conn, epub_document_id)?;
    let (verdict, basis, measures) = if pdf.words() > 0 && epub.words() > 0 {
        let c: Comparison = compare::compare(&pdf, &epub);
        let same = if pdf.usable() && epub.usable() {
            c.same
        } else {
            // Too short to sample fairly: close in length and sharing what
            // little was sampled, or with nothing sampled to share.
            c.length_ratio >= compare::SAME_LENGTH
                && (c.similarity >= compare::SAME_SIMILARITY || pdf.is_empty() || epub.is_empty())
        };
        (same, "text", Some(c))
    } else {
        (true, "name", None)
    };
    let kept: Option<String> = conn
        .query_row(
            "SELECT index_format FROM book_pairs
             WHERE pdf_document_id = ?1 AND epub_document_id = ?2",
            params![pdf_document_id, epub_document_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let index_format = if verdict {
        Some(choose_index(
            conn,
            (pdf_document_id, pdf.usable()),
            (epub_document_id, epub.usable()),
            kept.as_deref(),
        )?)
    } else {
        None
    };
    conn.execute(
        "INSERT INTO book_pairs (pdf_document_id, epub_document_id, verdict, basis, similarity,
             length_ratio, index_format, check_version, pdf_checked_at, epub_checked_at,
             created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)
         ON CONFLICT (pdf_document_id, epub_document_id) DO UPDATE SET
             verdict = excluded.verdict, basis = excluded.basis,
             similarity = excluded.similarity, length_ratio = excluded.length_ratio,
             index_format = excluded.index_format, check_version = excluded.check_version,
             pdf_checked_at = excluded.pdf_checked_at, epub_checked_at = excluded.epub_checked_at,
             updated_at = excluded.updated_at",
        params![
            pdf_document_id,
            epub_document_id,
            if verdict { "same" } else { "different" },
            basis,
            measures.map(|c| c.similarity),
            measures.map(|c| c.length_ratio),
            index_format,
            CHECK_VERSION,
            pdf_at,
            epub_at,
            now,
        ],
    )?;
    Ok(verdict)
}

/// The owner's choice for a pair, kept over any later check.
pub fn set_owner_verdict(
    conn: &Connection,
    pdf_document_id: &str,
    epub_document_id: &str,
    same: bool,
    now: &str,
) -> rusqlite::Result<()> {
    let (pdf, pdf_at) = sketch_of(conn, pdf_document_id)?;
    let (epub, epub_at) = sketch_of(conn, epub_document_id)?;
    let kept: Option<String> = conn
        .query_row(
            "SELECT index_format FROM book_pairs
             WHERE pdf_document_id = ?1 AND epub_document_id = ?2",
            params![pdf_document_id, epub_document_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    let index_format = if same {
        Some(choose_index(
            conn,
            (pdf_document_id, pdf.usable() || indexed(conn, pdf_document_id)?),
            (epub_document_id, epub.usable() || indexed(conn, epub_document_id)?),
            kept.as_deref(),
        )?)
    } else {
        kept.as_deref().map(|k| if k == "pdf" { "pdf" } else { "epub" })
    };
    conn.execute(
        "INSERT INTO book_pairs (pdf_document_id, epub_document_id, verdict, basis,
             index_format, check_version, pdf_checked_at, epub_checked_at, created_at, updated_at)
         VALUES (?1, ?2, ?3, 'owner', ?4, ?5, ?6, ?7, ?8, ?8)
         ON CONFLICT (pdf_document_id, epub_document_id) DO UPDATE SET
             verdict = excluded.verdict, basis = 'owner',
             index_format = excluded.index_format, pdf_checked_at = excluded.pdf_checked_at,
             epub_checked_at = excluded.epub_checked_at, updated_at = excluded.updated_at",
        params![
            pdf_document_id,
            epub_document_id,
            if same { "same" } else { "different" },
            index_format,
            CHECK_VERSION,
            pdf_at,
            epub_at,
            now,
        ],
    )?;
    Ok(())
}

/// Brings the active books in line with the verdicts and the files side by
/// side: a book is active while its two documents are 'same' and some
/// folder holds a file of each, side by side and available. A document is
/// in one active book at most (the first, in a fixed order). Writes only
/// what changed.
pub fn apply(conn: &Connection, now: &str) -> rusqlite::Result<Changes> {
    struct Candidate {
        pdf_doc: String,
        epub_doc: String,
        index_format: String,
        pdf_file: String,
        epub_file: String,
    }
    let candidates: Vec<Candidate> = conn
        .prepare(
            "SELECT bp.pdf_document_id, bp.epub_document_id, COALESCE(bp.index_format, 'epub'),
                    fp.pdf_file_id, fp.epub_file_id
             FROM book_pairs bp
             JOIN file_instances fpdf ON fpdf.document_id = bp.pdf_document_id
                  AND fpdf.availability = 'available'
             JOIN format_pairs fp ON fp.pdf_file_id = fpdf.id
             JOIN file_instances fe ON fe.id = fp.epub_file_id
                  AND fe.document_id = bp.epub_document_id AND fe.availability = 'available'
             WHERE bp.verdict = 'same'
             ORDER BY bp.pdf_document_id, bp.epub_document_id, fp.pdf_file_id",
        )?
        .query_map([], |r| {
            Ok(Candidate {
                pdf_doc: r.get(0)?,
                epub_doc: r.get(1)?,
                index_format: r.get(2)?,
                pdf_file: r.get(3)?,
                epub_file: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;

    // The books wanted: (pdf doc, epub doc) → (index doc, hidden doc); and
    // each name pair's folded file.
    let mut used: HashSet<String> = HashSet::new();
    let mut books: HashMap<(String, String), (String, String)> = HashMap::new();
    let mut folded: HashMap<String, String> = HashMap::new();
    for c in &candidates {
        let key = (c.pdf_doc.clone(), c.epub_doc.clone());
        if !books.contains_key(&key) {
            if used.contains(&c.pdf_doc) || used.contains(&c.epub_doc) {
                continue;
            }
            used.insert(c.pdf_doc.clone());
            used.insert(c.epub_doc.clone());
            let pair = if c.index_format == "pdf" {
                (c.pdf_doc.clone(), c.epub_doc.clone())
            } else {
                (c.epub_doc.clone(), c.pdf_doc.clone())
            };
            books.insert(key.clone(), pair);
        }
        let (_, hidden) = &books[&key];
        let file = if *hidden == c.pdf_doc { &c.pdf_file } else { &c.epub_file };
        folded.insert(c.pdf_file.clone(), file.clone());
    }

    let current: HashMap<(String, String), (String, String)> = conn
        .prepare(
            "SELECT pdf_document_id, epub_document_id, index_document_id, hidden_document_id
             FROM book_pairs WHERE hidden_document_id IS NOT NULL",
        )?
        .query_map([], |r| Ok(((r.get(0)?, r.get(1)?), (r.get(2)?, r.get(3)?))))?
        .collect::<rusqlite::Result<_>>()?;

    let mut changes = Changes::default();
    // Clear first, so the unique indexes never see a document twice.
    for (key, (_, hidden)) in &current {
        if books.get(key).map(|(_, h)| h) != Some(hidden) {
            conn.execute(
                "UPDATE book_pairs SET index_document_id = NULL, hidden_document_id = NULL,
                     updated_at = ?3
                 WHERE pdf_document_id = ?1 AND epub_document_id = ?2",
                params![key.0, key.1, now],
            )?;
            changes.unhidden.push(hidden.clone());
        }
    }
    for (key, (index, hidden)) in &books {
        if current.get(key) != Some(&(index.clone(), hidden.clone())) {
            conn.execute(
                "UPDATE book_pairs SET index_document_id = ?3, hidden_document_id = ?4,
                     updated_at = ?5
                 WHERE pdf_document_id = ?1 AND epub_document_id = ?2",
                params![key.0, key.1, index, hidden, now],
            )?;
            changes.hidden.push(hidden.clone());
        }
    }
    // A document hidden again under another book isn't shown in between.
    changes.unhidden.retain(|d| !changes.hidden.contains(d));

    let folded_now: HashMap<String, Option<String>> = conn
        .prepare("SELECT pdf_file_id, folded_file_id FROM format_pairs")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    for (pdf_file, was) in &folded_now {
        let want = folded.get(pdf_file);
        if was.as_ref() != want {
            conn.execute(
                "UPDATE format_pairs SET folded_file_id = ?2 WHERE pdf_file_id = ?1",
                params![pdf_file, want],
            )?;
        }
    }
    changes.unhidden.sort();
    changes.hidden.sort();
    Ok(changes)
}

/// The active books, both ways: hidden → index and index → hidden.
#[derive(Debug, Default, Clone)]
pub struct BookMap {
    pub index_of: HashMap<String, String>,
    pub hidden_of: HashMap<String, String>,
}

impl BookMap {
    pub fn load(conn: &Connection) -> rusqlite::Result<Self> {
        let mut map = BookMap::default();
        let mut stmt = conn.prepare_cached(
            "SELECT index_document_id, hidden_document_id FROM book_pairs
             WHERE hidden_document_id IS NOT NULL",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (index, hidden) = row?;
            map.index_of.insert(hidden.clone(), index.clone());
            map.hidden_of.insert(index, hidden);
        }
        Ok(map)
    }

    pub fn is_empty(&self) -> bool {
        self.index_of.is_empty()
    }

    /// The document a book is indexed and shown as: itself, unless hidden.
    pub fn book<'a>(&'a self, document_id: &'a str) -> &'a str {
        self.index_of.get(document_id).map_or(document_id, String::as_str)
    }

    pub fn is_hidden(&self, document_id: &str) -> bool {
        self.index_of.contains_key(document_id)
    }

    /// The other document of the book, either way.
    pub fn partner(&self, document_id: &str) -> Option<&str> {
        self.index_of
            .get(document_id)
            .or_else(|| self.hidden_of.get(document_id))
            .map(String::as_str)
    }
}

/// The other format's file for a book shown as `document_id`: the one side
/// by side with `beside` (the file shown) first, then any available copy.
pub fn other_format(
    conn: &Connection,
    map: &BookMap,
    document_id: &str,
    beside: Option<&str>,
) -> rusqlite::Result<Option<crate::domain::dto::FormatFile>> {
    use crate::domain::dto::{Availability, FileStorage, FileType, FormatFile};
    let Some(partner) = map.partner(document_id) else {
        return Ok(None);
    };
    conn.query_row(
        "SELECT f.id, f.file_type, f.display_name, f.display_path, f.source_root_id,
                f.availability, f.storage
         FROM file_instances f WHERE f.document_id = ?1
         ORDER BY f.id IN (SELECT epub_file_id FROM format_pairs WHERE pdf_file_id = ?2
                           UNION ALL
                           SELECT pdf_file_id FROM format_pairs WHERE epub_file_id = ?2) DESC,
                  f.availability = 'available' DESC, f.name_key
         LIMIT 1",
        params![partner, beside.unwrap_or("")],
        |r| {
            Ok(FormatFile {
                file_instance_id: r.get(0)?,
                file_type: if r.get::<_, String>(1)? == "epub" {
                    FileType::Epub
                } else {
                    FileType::Pdf
                },
                display_name: r.get(2)?,
                display_path: r.get(3)?,
                source_root_id: r.get(4)?,
                availability: if r.get::<_, String>(5)? == "missing" {
                    Availability::Missing
                } else {
                    Availability::Available
                },
                storage: if r.get::<_, String>(6)? == "cloud_only" {
                    FileStorage::CloudOnly
                } else {
                    FileStorage::Local
                },
            })
        },
    )
    .optional()
}

/// For a Library row: the file folded into it, its book's other format
/// beside it, while the two are one book.
pub fn folded_partner(
    conn: &Connection,
    file_id: &str,
) -> rusqlite::Result<Option<crate::domain::dto::FormatFile>> {
    use crate::domain::dto::{Availability, FileStorage, FileType, FormatFile};
    conn.query_row(
        "SELECT f.id, f.file_type, f.display_name, f.display_path, f.source_root_id,
                f.availability, f.storage
         FROM format_pairs fp JOIN file_instances f ON f.id = fp.folded_file_id
         WHERE (fp.pdf_file_id = ?1 OR fp.epub_file_id = ?1) AND fp.folded_file_id <> ?1",
        [file_id],
        |r| {
            Ok(FormatFile {
                file_instance_id: r.get(0)?,
                file_type: if r.get::<_, String>(1)? == "epub" {
                    FileType::Epub
                } else {
                    FileType::Pdf
                },
                display_name: r.get(2)?,
                display_path: r.get(3)?,
                source_root_id: r.get(4)?,
                availability: if r.get::<_, String>(5)? == "missing" {
                    Availability::Missing
                } else {
                    Availability::Available
                },
                storage: if r.get::<_, String>(6)? == "cloud_only" {
                    FileStorage::CloudOnly
                } else {
                    FileStorage::Local
                },
            })
        },
    )
    .optional()
}

/// Changes whenever the active books do, for caches keyed on them.
pub fn stamp(conn: &Connection) -> rusqlite::Result<String> {
    conn.query_row(
        "SELECT COUNT(*) || ':' || COALESCE(MAX(updated_at), '') FROM book_pairs
         WHERE hidden_document_id IS NOT NULL",
        [],
        |r| r.get(0),
    )
}

/// Folders whose pairing pass is behind this build's.
pub fn behind(conn: &Connection, root_id: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT formats_version < ?2 FROM source_roots WHERE id = ?1",
        params![root_id, FORMATS_VERSION],
        |r| r.get(0),
    )
    .optional()
    .map(|v| v.unwrap_or(false))
}

pub fn mark_current(conn: &Connection, root_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE source_roots SET formats_version = ?2 WHERE id = ?1",
        params![root_id, FORMATS_VERSION],
    )?;
    Ok(())
}

/// At open: a reading job for each folder whose books haven't been paired
/// by this build, so an existing library pairs them once. The job finds
/// nothing to read and pairs as it ends.
pub fn queue_unchecked(conn: &Connection, now: &str) -> rusqlite::Result<usize> {
    use crate::domain::dto::JobKind;
    use crate::services::jobs;
    let roots: Vec<String> = conn
        .prepare("SELECT id FROM source_roots WHERE formats_version < ?1")?
        .query_map([FORMATS_VERSION], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut queued = 0;
    for root in roots {
        if jobs::db::active_for_root(conn, &root, JobKind::Extract)?.is_none() {
            let id = uuid::Uuid::new_v4().to_string();
            jobs::db::insert_job(conn, &id, &root, JobKind::Extract, now)?;
            queued += 1;
        }
    }
    Ok(queued)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::native_path;
    use std::path::Path;

    fn library() -> Connection {
        let mut c = Connection::open_in_memory().unwrap();
        crate::services::catalog::migrations::migrate(&mut c, "t").unwrap();
        c.execute_batch(
            "INSERT INTO workspace (id, name, created_at) VALUES ('w', 'W', 't');
             INSERT INTO source_roots (id, workspace_id, path_bytes, display_path, created_at)
                 VALUES ('r', 'w', x'00', '/b', 't');",
        )
        .unwrap();
        c
    }

    /// About 6,000 words; each seed a different text.
    fn book_text(seed: u64) -> String {
        let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
        (0..6_000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                format!("w{}", x % 5_000)
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// A file and its document, read with `text` (None: Needs OCR).
    fn add(c: &Connection, id: &str, name: &str, text: Option<&str>) {
        let kind = if name.ends_with(".pdf") { "pdf" } else { "epub" };
        let state = if text.is_some() { "done" } else { "needs_ocr" };
        c.execute(
            "INSERT INTO documents (id, content_hash, state, created_at, updated_at)
             VALUES (?1, CAST(?1 AS BLOB), ?2, 't', 't')",
            params![format!("d-{id}"), state],
        )
        .unwrap();
        if let Some(text) = text {
            c.execute(
                "INSERT INTO passages (document_id, ordinal, text) VALUES (?1, 0, ?2)",
                params![format!("d-{id}"), text],
            )
            .unwrap();
        }
        c.execute(
            "INSERT INTO file_instances (id, source_root_id, rel_path, display_path, display_name,
                 extension, file_type, availability, extract_state, document_id, created_at,
                 updated_at)
             VALUES (?1, 'r', ?2, ?3, ?3, ?4, ?4, 'available', 'linked', ?5, 't', 't')",
            params![id, native_path::encode(Path::new(name)), name, kind, format!("d-{id}")],
        )
        .unwrap();
    }

    fn pass(c: &Connection) -> Changes {
        reconcile_root(c, "r", "t").unwrap();
        for (p, e) in pending_checks(c, "r", 100).unwrap() {
            check_pair(c, &p, &e, "t").unwrap();
        }
        apply(c, "t").unwrap()
    }

    #[test]
    fn a_pair_with_one_text_is_one_book_shown_as_the_epub() {
        let c = library();
        let text = book_text(1);
        add(&c, "p", "Dune.pdf", Some(&text));
        add(&c, "e", "Dune.epub", Some(&text));
        let changes = pass(&c);
        assert_eq!(changes.hidden, vec!["d-p".to_owned()]);
        let map = BookMap::load(&c).unwrap();
        assert_eq!(map.book("d-p"), "d-e");
        assert_eq!(map.partner("d-e"), Some("d-p"));
        let folded: Option<String> = c
            .query_row("SELECT folded_file_id FROM format_pairs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(folded.as_deref(), Some("p"));
        // Nothing changed: nothing written.
        assert!(pass(&c).is_empty());
    }

    #[test]
    fn different_texts_with_one_name_stay_two_books() {
        let c = library();
        add(&c, "p", "notes.pdf", Some(&book_text(1)));
        add(&c, "e", "notes.epub", Some(&book_text(2)));
        assert!(pass(&c).is_empty());
        let (verdict, basis): (String, String) = c
            .query_row("SELECT verdict, basis FROM book_pairs", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!((verdict.as_str(), basis.as_str()), ("different", "text"));
    }

    #[test]
    fn a_scan_without_text_pairs_by_name_and_the_epub_is_used() {
        let c = library();
        add(&c, "p", "Alice.pdf", None);
        add(&c, "e", "Alice.epub", Some(&book_text(3)));
        pass(&c);
        let basis: String = c.query_row("SELECT basis FROM book_pairs", [], |r| r.get(0)).unwrap();
        assert_eq!(basis, "name");
        assert_eq!(BookMap::load(&c).unwrap().book("d-p"), "d-e");
    }

    #[test]
    fn a_copy_already_indexed_stays_the_one_used() {
        let c = library();
        let text = book_text(4);
        add(&c, "p", "Emma.pdf", Some(&text));
        add(&c, "e", "Emma.epub", Some(&text));
        // The PDF's meaning was indexed before the pair was known.
        c.execute_batch(
            "INSERT INTO embedding_models (id, model_name, dims, quantization, chunker_version,
                 status, created_at, updated_at)
                 VALUES ('m', 'x', 4, 'f16', 1, 'building', 't', 't');
             INSERT INTO document_embeddings (document_id, model_id, seq, status, updated_at)
                 VALUES ('d-p', 'm', 1, 'done', 't');",
        )
        .unwrap();
        pass(&c);
        let map = BookMap::load(&c).unwrap();
        assert_eq!(map.book("d-e"), "d-p", "the indexed PDF is used; the EPUB waits for nothing");
    }

    #[test]
    fn the_owners_choice_is_kept_and_a_copy_gone_brings_its_partner_back() {
        let c = library();
        let text = book_text(5);
        add(&c, "p", "Kim.pdf", Some(&text));
        add(&c, "e", "Kim.epub", Some(&text));
        pass(&c);
        set_owner_verdict(&c, "d-p", "d-e", false, "t2").unwrap();
        let changes = apply(&c, "t2").unwrap();
        assert_eq!(changes.unhidden, vec!["d-p".to_owned()]);
        // A re-read doesn't overturn it.
        c.execute("UPDATE documents SET updated_at = 't9'", []).unwrap();
        assert!(pending_checks(&c, "r", 10).unwrap().is_empty());
        set_owner_verdict(&c, "d-p", "d-e", true, "t3").unwrap();
        assert_eq!(apply(&c, "t3").unwrap().hidden, vec!["d-p".to_owned()]);
        // The EPUB deleted: the PDF is shown again.
        c.execute("UPDATE file_instances SET availability = 'missing' WHERE id = 'e'", [])
            .unwrap();
        reconcile_root(&c, "r", "t4").unwrap();
        assert_eq!(apply(&c, "t4").unwrap().unhidden, vec!["d-p".to_owned()]);
        assert!(BookMap::load(&c).unwrap().is_empty());
    }

    #[test]
    fn an_existing_library_is_paired_once_after_the_upgrade() {
        let c = library();
        assert!(behind(&c, "r").unwrap());
        assert_eq!(queue_unchecked(&c, "t").unwrap(), 1);
        assert_eq!(queue_unchecked(&c, "t").unwrap(), 0, "one job a folder");
        mark_current(&c, "r").unwrap();
        assert!(!behind(&c, "r").unwrap());
    }
}
