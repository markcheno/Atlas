//! Anchors: a place in a document by value (phase3-spec §6.2).
//!
//! An anchor holds the document's id and content hash, a title to show
//! if it is gone, the place as it was (page, location, passage ordinal)
//! and an excerpt. It never holds a passage, chunk or file row id: those
//! don't survive a re-read or a move.
//!
//! [`resolve`] reads the catalog as it is now and never writes. The
//! document is looked up by id, else by content hash (a rebuilt catalog
//! gives documents new ids). The excerpt is searched for at the saved
//! ordinal first, then in the document's other passages, folded and with
//! whitespace collapsed; if it isn't found, the saved place is used.

use rusqlite::{Connection, OptionalExtension, params};

use crate::domain::dto::{Anchor, AnchorState, FileType, ResolvedAnchor};
use crate::error::{AtlasError, AtlasResult};
use crate::services::library::fold::fold;

/// An excerpt's length limit, in characters.
pub const ANCHOR_EXCERPT_CHARS: usize = 2000;
/// When the whole excerpt isn't found, its first this many characters
/// (normalized) are looked for: a re-split passage may hold only part of
/// the old one.
const HEAD_CHARS: usize = 160;

/// Where in a document an anchor points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// The whole document: no place, no excerpt.
    Document,
    /// A passage, by its ordinal in the catalog now.
    Ordinal(u32),
    /// A PDF page: its first passage.
    Page(u32),
}

pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn from_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

/// Folded, with runs of whitespace as one space.
fn normalize(text: &str) -> String {
    fold(text).split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => text[..at].to_owned(),
        None => text.to_owned(),
    }
}

fn read_err(e: rusqlite::Error) -> AtlasError {
    AtlasError::db("be read")(e)
}

/// Builds the anchor for `place` in the document of file `file_id`, from
/// the catalog. `None` when the file, its document or the place doesn't
/// exist (a file not read yet has no document).
pub fn build(conn: &Connection, file_id: &str, place: Place) -> AtlasResult<Option<Anchor>> {
    let doc: Option<(String, Vec<u8>, String, String)> = conn
        .query_row(
            "SELECT d.id, d.content_hash, COALESCE(d.title, fi.display_name), fi.file_type
             FROM file_instances fi JOIN documents d ON d.id = fi.document_id
             WHERE fi.id = ?1",
            [file_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(read_err)?;
    let Some((document_id, hash, title, file_type)) = doc else {
        return Ok(None);
    };
    let file_type = if file_type == "epub" {
        FileType::Epub
    } else {
        FileType::Pdf
    };
    let mut anchor = Anchor {
        document_id,
        content_hash: to_hex(&hash),
        title,
        file_type,
        page_no: None,
        location: None,
        ordinal: None,
        excerpt: None,
    };
    let (sql, key) = match place {
        Place::Document => return Ok(Some(anchor)),
        Place::Ordinal(n) => (
            "SELECT ordinal, page_no, location, text FROM passages
             WHERE document_id = ?1 AND ordinal = ?2",
            n,
        ),
        Place::Page(p) => (
            "SELECT ordinal, page_no, location, text FROM passages
             WHERE document_id = ?1 AND page_no = ?2 ORDER BY ordinal LIMIT 1",
            p,
        ),
    };
    let passage: Option<(u32, Option<u32>, Option<String>, String)> = conn
        .query_row(sql, params![anchor.document_id, key], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .optional()
        .map_err(read_err)?;
    let Some((ordinal, page_no, location, text)) = passage else {
        return Ok(None);
    };
    anchor.ordinal = Some(ordinal);
    anchor.page_no = page_no;
    anchor.location = location;
    anchor.excerpt = Some(truncate_chars(&text, ANCHOR_EXCERPT_CHARS));
    Ok(Some(anchor))
}

/// The document the anchor names, as the catalog has it now: by id when
/// its content matches, else by content hash.
fn find_document(conn: &Connection, anchor: &Anchor) -> AtlasResult<Option<String>> {
    let hash = from_hex(&anchor.content_hash);
    let by_id: Option<Vec<u8>> = conn
        .query_row(
            "SELECT content_hash FROM documents WHERE id = ?1",
            [&anchor.document_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(read_err)?;
    if let Some(found) = by_id
        && hash.as_ref().is_none_or(|h| *h == found)
    {
        return Ok(Some(anchor.document_id.clone()));
    }
    let Some(hash) = hash else {
        return Ok(None);
    };
    conn.query_row(
        "SELECT id FROM documents WHERE content_hash = ?1",
        [hash],
        |r| r.get(0),
    )
    .optional()
    .map_err(read_err)
}

/// The file to open: an available, local file of the document, then one
/// only in the cloud.
fn find_file(conn: &Connection, document_id: &str) -> AtlasResult<Option<String>> {
    conn.query_row(
        "SELECT id FROM file_instances
         WHERE document_id = ?1 AND availability = 'available'
         ORDER BY storage = 'local' DESC, display_path, id LIMIT 1",
        [document_id],
        |r| r.get(0),
    )
    .optional()
    .map_err(read_err)
}

struct Hit {
    ordinal: u32,
    page_no: Option<u32>,
    location: Option<String>,
}

/// Where in the document `needle` (normalized) is: the saved ordinal
/// first, then the others in order.
fn find_text(
    conn: &Connection,
    document_id: &str,
    saved: Option<u32>,
    needle: &str,
) -> AtlasResult<Option<Hit>> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT ordinal, page_no, location, text FROM passages
             WHERE document_id = ?1
             ORDER BY ordinal = ?2 DESC, ordinal",
        )
        .map_err(read_err)?;
    let mut rows = stmt
        .query(params![document_id, saved.map(i64::from).unwrap_or(-1)])
        .map_err(read_err)?;
    while let Some(r) = rows.next().map_err(read_err)? {
        let text: String = r.get(3).map_err(read_err)?;
        if normalize(&text).contains(needle) {
            return Ok(Some(Hit {
                ordinal: r.get(0).map_err(read_err)?,
                page_no: r.get(1).map_err(read_err)?,
                location: r.get(2).map_err(read_err)?,
            }));
        }
    }
    Ok(None)
}

/// Resolves `anchor` against the catalog now. Never writes.
pub fn resolve(conn: &Connection, anchor: &Anchor) -> AtlasResult<ResolvedAnchor> {
    let saved = |state, document_id, file_instance_id, excerpt_found| ResolvedAnchor {
        state,
        document_id,
        file_instance_id,
        page_no: anchor.page_no,
        location: anchor.location.clone(),
        ordinal: anchor.ordinal,
        excerpt_found,
    };
    let Some(document_id) = find_document(conn, anchor)? else {
        return Ok(saved(AnchorState::Unknown, None, None, false));
    };
    let Some(file_id) = find_file(conn, &document_id)? else {
        return Ok(saved(AnchorState::NoFile, Some(document_id), None, false));
    };
    let needle = anchor
        .excerpt
        .as_deref()
        .map(normalize)
        .filter(|n| !n.is_empty());
    let Some(needle) = needle else {
        // A whole document: there is nothing to look for.
        return Ok(saved(
            AnchorState::Found,
            Some(document_id),
            Some(file_id),
            true,
        ));
    };
    let mut tries = vec![needle.clone()];
    if needle.chars().count() > HEAD_CHARS {
        tries.push(truncate_chars(&needle, HEAD_CHARS));
    }
    for t in &tries {
        if let Some(hit) = find_text(conn, &document_id, anchor.ordinal, t)? {
            let same = Some(hit.ordinal) == anchor.ordinal && hit.page_no == anchor.page_no;
            return Ok(ResolvedAnchor {
                state: if same {
                    AnchorState::Found
                } else {
                    AnchorState::FoundElsewhere
                },
                document_id: Some(document_id),
                file_instance_id: Some(file_id),
                page_no: hit.page_no,
                location: hit.location,
                ordinal: Some(hit.ordinal),
                excerpt_found: true,
            });
        }
    }
    Ok(saved(
        AnchorState::Found,
        Some(document_id),
        Some(file_id),
        false,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips_and_rejects_nonsense() {
        let bytes = [0u8, 15, 16, 255, 171];
        assert_eq!(to_hex(&bytes), "000f10ffab");
        assert_eq!(from_hex("000f10ffab").unwrap(), bytes);
        assert_eq!(from_hex("abc"), None);
        assert_eq!(from_hex("zz"), None);
        assert_eq!(from_hex("é1"), None);
    }

    #[test]
    fn excerpts_compare_folded_with_whitespace_collapsed() {
        assert_eq!(normalize("  Café\n\tAU   lait "), "cafe au lait");
        assert_eq!(truncate_chars("ééé", 2), "éé");
        assert_eq!(truncate_chars("ab", 5), "ab");
    }
}
