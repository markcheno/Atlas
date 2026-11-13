//! One book in two formats (Slice 4.5.1, ADR 0023): a PDF and an EPUB side
//! by side with one name whose texts match are one book. One of the two is
//! indexed for meaning and shown; the other is kept out of what Atlas shows
//! and counts, and its text, passages and vectors stay as they are.

pub mod compare;
pub mod db;
pub mod names;

use crate::services::catalog::writer::CatalogHandle;

/// Pairs per catalog call: each reads two books' text on the writer.
const CHECKS_PER_CALL: usize = 4;

/// What a folder's catch-up did.
#[derive(Debug, Default)]
pub struct CaughtUp {
    pub checked: u32,
    pub same: u32,
    pub changes: db::Changes,
    /// False when stopped part-way; the folder is then still behind.
    pub finished: bool,
}

/// Pairs a folder's books: its names, the text check for pairs not yet
/// checked, then the active books. A few pairs at a time, so other writes
/// go on between; `stop` ends it early (the rest is done next time).
pub fn catch_up(
    catalog: &CatalogHandle,
    root_id: &str,
    now: &str,
    stop: impl Fn() -> bool,
) -> CaughtUp {
    let mut out = CaughtUp::default();
    let (root, ts) = (root_id.to_owned(), now.to_owned());
    if !matches!(
        catalog.call(move |c| db::reconcile_root(c, &root, &ts)),
        Ok(Ok(_))
    ) {
        return out;
    }
    loop {
        if stop() {
            return out;
        }
        let (root, ts) = (root_id.to_owned(), now.to_owned());
        let batch = catalog.call(move |c| -> rusqlite::Result<(u32, u32)> {
            let pending = db::pending_checks(c, &root, CHECKS_PER_CALL)?;
            let mut same = 0;
            for (pdf, epub) in &pending {
                if db::check_pair(c, pdf, epub, &ts)? {
                    same += 1;
                }
            }
            Ok((pending.len() as u32, same))
        });
        match batch {
            Ok(Ok((0, _))) => break,
            Ok(Ok((checked, same))) => {
                out.checked += checked;
                out.same += same;
            }
            _ => return out,
        }
    }
    let (root, ts) = (root_id.to_owned(), now.to_owned());
    match catalog.call(move |c| -> rusqlite::Result<db::Changes> {
        let changes = db::apply(c, &ts)?;
        db::mark_current(c, &root)?;
        Ok(changes)
    }) {
        Ok(Ok(changes)) => {
            out.changes = changes;
            out.finished = true;
        }
        _ => {}
    }
    out
}

/// The owner's choice (Slice 4.5.1): whether a file and the other format
/// beside it are one book. Kept over every later check; a copy shown again
/// is queued for its meaning at once.
pub fn set_same_book(
    root: &crate::services::workspace::OpenRoot,
    file_id: &str,
    same: bool,
) -> crate::error::AtlasResult<()> {
    use crate::error::AtlasError;
    use rusqlite::OptionalExtension;
    let allow = crate::services::providers::privacy::remote_embedding(&root.settings().privacy);
    let (file, now) = (file_id.to_owned(), root.clock().now_rfc3339());
    root.catalog()
        .call(move |c| -> rusqlite::Result<Result<(), AtlasError>> {
            type Pair = (Option<String>, Option<String>, String);
            let pair: Option<Pair> = c
                .query_row(
                    "SELECT fpdf.document_id, fe.document_id, fpdf.source_root_id
                     FROM format_pairs fp
                     JOIN file_instances fpdf ON fpdf.id = fp.pdf_file_id
                     JOIN file_instances fe ON fe.id = fp.epub_file_id
                     WHERE fp.pdf_file_id = ?1 OR fp.epub_file_id = ?1",
                    [&file],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let Some((pdf, epub, root_id)) = pair else {
                return Ok(Err(AtlasError::NoOtherFormat));
            };
            let (Some(pdf), Some(epub)) = (pdf, epub) else {
                return Ok(Err(AtlasError::NotReadYet));
            };
            let tx = c.transaction()?;
            db::set_owner_verdict(&tx, &pdf, &epub, same, &now)?;
            let changes = db::apply(&tx, &now)?;
            if !changes.unhidden.is_empty() {
                crate::services::embedding::triggers::queue_root(&tx, &root_id, allow, &now)?;
            }
            tx.commit()?;
            Ok(Ok(()))
        })?
        .map_err(AtlasError::db("save whether these are one book"))??;
    root.wake_jobs();
    Ok(())
}
