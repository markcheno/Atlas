//! Bookmarks and findings (phase3-spec §5.3, §7.3; Slice 3.5): what the
//! user keeps on purpose.
//!
//! - **Rust builds what is kept.** The renderer names a file and a place,
//!   an answer or a trail step; the anchor comes from the catalog and what
//!   the screen showed (the recorder's passages), and an answer's text from
//!   the last ended answer, kept in memory until the next question.
//! - **One bookmark per place:** the same document and passage (or the
//!   whole document) updates the bookmark already there.
//! - **A finding keeps its own copy** of its text and sources, so it
//!   outlives the trail it came from.
//! - Every write is one transaction; deleting also takes the item out of
//!   any collection.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use super::anchor::{self, Place};
use super::record::Recorder;
use super::trails::{self, checked_note};
use crate::domain::dto::{
    Anchor, Bookmark, FilePlace, Finding, FindingKind, FindingSource, SaveTarget, SavedItem,
    SavedKind, SavedPage, SavedQuery, SavedSource, StepPayload,
};
use crate::error::{AtlasError, AtlasResult};
use crate::services::library::fold::fold;

pub const MAX_PAGE_SIZE: u32 = 100;

/// A finding's source as stored (`findings.sources`).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct KeptSource {
    n: u32,
    anchor: Anchor,
}

fn json_err(e: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(e))
}

fn table(kind: SavedKind) -> &'static str {
    match kind {
        SavedKind::Bookmark => "bookmarks",
        SavedKind::Finding => "findings",
    }
}

fn item_kind(kind: SavedKind) -> &'static str {
    match kind {
        SavedKind::Bookmark => "bookmark",
        SavedKind::Finding => "finding",
    }
}

/// The anchor `target` names, built now from the catalog (a file) or as it
/// was recorded (a trail step).
fn anchor_for(rec: &Recorder, target: &SaveTarget) -> AtlasResult<Anchor> {
    match target {
        SaveTarget::File { file_id, place } => {
            // A citation of the answer on screen: its source as it was sent.
            if let FilePlace::Citation { answer_id, n } = place
                && let Some(last) = rec.last_answer(answer_id)
                && let Some((_, a)) = last.record.sources.iter().find(|(m, _)| m == n)
            {
                return Ok(a.clone());
            }
            let at = rec.place_for(file_id, place);
            rec.catalog()
                .read(|c| -> AtlasResult<Option<Anchor>> {
                    match anchor::build(c, file_id, at)? {
                        Some(a) => Ok(Some(a)),
                        // A place gone since (a re-read): the whole document.
                        None if at != Place::Document => anchor::build(c, file_id, Place::Document),
                        None => Ok(None),
                    }
                })??
                .ok_or(AtlasError::SaveSourceGone)
        }
        SaveTarget::Step { step_id, index } => {
            let payload = step_payload(rec, step_id)?;
            trails::anchors(&payload)
                .get(*index as usize)
                .map(|a| (*a).clone())
                .ok_or(AtlasError::SaveSourceGone)
        }
    }
}

fn step_payload(rec: &Recorder, step_id: &str) -> AtlasResult<StepPayload> {
    let text: Option<String> = rec
        .store()
        .read(|c| {
            c.query_row(
                "SELECT payload FROM trail_steps WHERE id = ?1",
                [step_id],
                |r| r.get(0),
            )
            .optional()
        })?
        .map_err(AtlasError::db("be read"))?;
    let text = text.ok_or(AtlasError::SaveSourceGone)?;
    serde_json::from_str(&text).map_err(|_| AtlasError::SaveSourceGone)
}

// ------------------------------------------------------------- bookmarks

/// Saves a bookmark of `target`, or refreshes the one already at that
/// place.
pub fn save_bookmark(rec: &Recorder, target: &SaveTarget) -> AtlasResult<Bookmark> {
    let anchor = anchor_for(rec, target)?;
    let now = rec.store().clock().now_rfc3339();
    let text = serde_json::to_string(&anchor).map_err(|_| AtlasError::Internal("an anchor"))?;
    let hash = anchor::from_hex(&anchor.content_hash).unwrap_or_default();
    let (doc, ordinal) = (anchor.document_id.clone(), anchor.ordinal);
    let id = rec
        .store()
        .call(move |c| -> rusqlite::Result<String> {
            let tx = c.transaction()?;
            let existing: Option<String> = tx
                .query_row(
                    "SELECT id FROM bookmarks
                     WHERE document_id = ?1 AND json_extract(anchor, '$.ordinal') IS ?2",
                    params![doc, ordinal],
                    |r| r.get(0),
                )
                .optional()?;
            let id = match existing {
                Some(id) => {
                    tx.execute(
                        "UPDATE bookmarks SET anchor = ?2, content_hash = ?3, updated_at = ?4
                         WHERE id = ?1",
                        params![id, text, hash, now],
                    )?;
                    id
                }
                None => {
                    let id = uuid::Uuid::new_v4().to_string();
                    tx.execute(
                        "INSERT INTO bookmarks (id, anchor, document_id, content_hash, created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                        params![id, text, doc, hash, now],
                    )?;
                    id
                }
            };
            tx.commit()?;
            Ok(id)
        })?
        .map_err(AtlasError::db("save the bookmark"))?;
    match get(rec, SavedKind::Bookmark, &id)? {
        SavedItem::Bookmark(b) => Ok(b),
        SavedItem::Finding(_) => Err(AtlasError::Internal("a bookmark")),
    }
}

// -------------------------------------------------------------- findings

/// What a finding keeps: its kind, question, text and sources, and the
/// step it came from.
type Kept = (
    FindingKind,
    Option<String>,
    String,
    Vec<KeptSource>,
    Option<String>,
);

fn cited(sources: impl IntoIterator<Item = (u32, Anchor)>, cited: &[u32]) -> Vec<KeptSource> {
    sources
        .into_iter()
        .filter(|(n, _)| cited.contains(n))
        .map(|(n, anchor)| KeptSource { n, anchor })
        .collect()
}

fn kept(rec: &Recorder, source: &FindingSource) -> AtlasResult<Kept> {
    match source {
        FindingSource::Answer { answer_id } => {
            let last = rec
                .last_answer(answer_id)
                .filter(|l| !l.outcome.text.trim().is_empty())
                .ok_or(AtlasError::SaveSourceGone)?;
            Ok((
                FindingKind::Answer,
                Some(last.record.question.clone()),
                last.outcome.text.clone(),
                cited(last.record.sources.clone(), &last.outcome.cited),
                last.step_id.clone(),
            ))
        }
        FindingSource::Step { step_id } => match step_payload(rec, step_id)? {
            StepPayload::Ask(a) if !a.answer.trim().is_empty() => Ok((
                FindingKind::Answer,
                Some(a.question),
                a.answer,
                cited(a.sources.into_iter().map(|s| (s.n, s.anchor)), &a.cited),
                Some(step_id.clone()),
            )),
            _ => Err(AtlasError::SaveSourceGone),
        },
        FindingSource::Passage { target } => {
            let anchor = anchor_for(rec, target)?;
            let text = anchor.excerpt.clone().ok_or(AtlasError::SaveSourceGone)?;
            let from = match target {
                SaveTarget::Step { step_id, .. } => Some(step_id.clone()),
                SaveTarget::File { .. } => None,
            };
            Ok((
                FindingKind::Passage,
                None,
                text,
                vec![KeptSource { n: 1, anchor }],
                from,
            ))
        }
    }
}

/// Saves a finding: an answer with its question and cited sources, or a
/// passage.
pub fn save_finding(rec: &Recorder, source: &FindingSource) -> AtlasResult<Finding> {
    let (kind, question, text, sources, from) = kept(rec, source)?;
    let now = rec.store().clock().now_rfc3339();
    let sources = serde_json::to_string(&sources).map_err(|_| AtlasError::Internal("sources"))?;
    let kind_text = match kind {
        FindingKind::Answer => "answer",
        FindingKind::Passage => "passage",
    };
    let id = uuid::Uuid::new_v4().to_string();
    let row = id.clone();
    rec.store()
        .call(move |c| {
            c.execute(
                "INSERT INTO findings (id, kind, question, text, sources, from_step_id, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                params![row, kind_text, question, text, sources, from, now],
            )
        })?
        .map_err(AtlasError::db("save the finding"))?;
    match get(rec, SavedKind::Finding, &id)? {
        SavedItem::Finding(f) => Ok(f),
        SavedItem::Bookmark(_) => Err(AtlasError::Internal("a finding")),
    }
}

// ---------------------------------------------------------------- changes

/// Sets (or, empty, clears) an item's note: plain text, at most 4,000
/// characters.
pub fn set_note(rec: &Recorder, kind: SavedKind, id: &str, text: &str) -> AtlasResult<SavedItem> {
    let note = checked_note(text)?;
    let now = rec.store().clock().now_rfc3339();
    let (sql, row) = (
        format!(
            "UPDATE {} SET note = ?2, updated_at = ?3 WHERE id = ?1",
            table(kind)
        ),
        id.to_owned(),
    );
    let n = rec
        .store()
        .call(move |c| c.execute(&sql, params![row, note, now]))?
        .map_err(AtlasError::db("save the note"))?;
    if n == 0 {
        return Err(AtlasError::SaveSourceGone);
    }
    get(rec, kind, id)
}

/// Deletes a bookmark or a finding, and takes it out of every collection.
pub fn delete(rec: &Recorder, kind: SavedKind, id: &str) -> AtlasResult<()> {
    let (row, sql, item) = (
        id.to_owned(),
        format!("DELETE FROM {} WHERE id = ?1", table(kind)),
        item_kind(kind),
    );
    let n = rec
        .store()
        .call(move |c| -> rusqlite::Result<usize> {
            let tx = c.transaction()?;
            tx.execute(
                "DELETE FROM collection_items WHERE item_kind = ?1 AND item_id = ?2",
                [item, row.as_str()],
            )?;
            let n = tx.execute(&sql, [&row])?;
            tx.commit()?;
            Ok(n)
        })?
        .map_err(AtlasError::db("delete it"))?;
    if n == 0 {
        return Err(AtlasError::SaveSourceGone);
    }
    Ok(())
}

// ---------------------------------------------------------------- reading

/// A stored item before its anchors are resolved.
enum Row {
    Bookmark {
        id: String,
        anchor: Anchor,
        note: Option<String>,
        created_at: String,
        updated_at: String,
    },
    Finding {
        id: String,
        kind: FindingKind,
        question: Option<String>,
        text: String,
        sources: Vec<KeptSource>,
        note: Option<String>,
        from_step_id: Option<String>,
        created_at: String,
        updated_at: String,
    },
}

impl Row {
    fn updated_at(&self) -> &str {
        match self {
            Row::Bookmark { updated_at, .. } | Row::Finding { updated_at, .. } => updated_at,
        }
    }

    fn id(&self) -> &str {
        match self {
            Row::Bookmark { id, .. } | Row::Finding { id, .. } => id,
        }
    }

    fn kind(&self) -> SavedKind {
        match self {
            Row::Bookmark { .. } => SavedKind::Bookmark,
            Row::Finding { .. } => SavedKind::Finding,
        }
    }

    fn documents(&self) -> Vec<&str> {
        match self {
            Row::Bookmark { anchor, .. } => vec![anchor.document_id.as_str()],
            Row::Finding { sources, .. } => sources
                .iter()
                .map(|s| s.anchor.document_id.as_str())
                .collect(),
        }
    }

    /// Everything the filter looks at, folded.
    fn text(&self) -> String {
        fn anchor_text<'a>(a: &'a Anchor, parts: &mut Vec<&'a str>) {
            parts.push(&a.title);
            parts.extend(a.location.as_deref());
        }
        let mut parts: Vec<&str> = Vec::new();

        match self {
            Row::Bookmark { anchor, note, .. } => {
                anchor_text(anchor, &mut parts);
                parts.extend(anchor.excerpt.as_deref());
                parts.extend(note.as_deref());
            }
            Row::Finding {
                question,
                text,
                sources,
                note,
                ..
            } => {
                parts.extend(question.as_deref());
                parts.push(text);
                parts.extend(note.as_deref());
                for s in sources {
                    anchor_text(&s.anchor, &mut parts);
                }
            }
        }
        fold(&parts.join("\n"))
    }
}

fn rows(conn: &Connection) -> rusqlite::Result<Vec<Row>> {
    let mut out = Vec::new();
    let mut stmt =
        conn.prepare_cached("SELECT id, anchor, note, created_at, updated_at FROM bookmarks")?;
    for row in stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
        ))
    })? {
        let (id, anchor, note, created_at, updated_at) = row?;
        let Ok(anchor) = serde_json::from_str(&anchor) else {
            continue;
        };
        out.push(Row::Bookmark {
            id,
            anchor,
            note,
            created_at,
            updated_at,
        });
    }
    let mut stmt = conn.prepare_cached(
        "SELECT id, kind, question, text, sources, note, from_step_id, created_at, updated_at
         FROM findings",
    )?;
    for row in stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, Option<String>>(5)?,
            r.get::<_, Option<String>>(6)?,
            r.get::<_, String>(7)?,
            r.get::<_, String>(8)?,
        ))
    })? {
        let (id, kind, question, text, sources, note, from_step_id, created_at, updated_at) = row?;
        let sources: Vec<KeptSource> = serde_json::from_str(&sources).map_err(json_err)?;
        out.push(Row::Finding {
            id,
            kind: if kind == "passage" {
                FindingKind::Passage
            } else {
                FindingKind::Answer
            },
            question,
            text,
            sources,
            note,
            from_step_id,
            created_at,
            updated_at,
        });
    }
    Ok(out)
}

fn finish(conn: &Connection, row: Row) -> AtlasResult<SavedItem> {
    Ok(match row {
        Row::Bookmark {
            id,
            anchor,
            note,
            created_at,
            updated_at,
        } => SavedItem::Bookmark(Bookmark {
            resolved: anchor::resolve(conn, &anchor)?,
            id,
            anchor,
            note,
            created_at,
            updated_at,
        }),
        Row::Finding {
            id,
            kind,
            question,
            text,
            sources,
            note,
            from_step_id,
            created_at,
            updated_at,
        } => SavedItem::Finding(Finding {
            sources: sources
                .into_iter()
                .map(|s| {
                    Ok(SavedSource {
                        resolved: anchor::resolve(conn, &s.anchor)?,
                        n: s.n,
                        anchor: s.anchor,
                    })
                })
                .collect::<AtlasResult<Vec<_>>>()?,
            id,
            kind,
            question,
            text,
            note,
            from_step_id,
            created_at,
            updated_at,
        }),
    })
}

fn get(rec: &Recorder, kind: SavedKind, id: &str) -> AtlasResult<SavedItem> {
    let row = rec
        .store()
        .read(rows)?
        .map_err(AtlasError::db("be read"))?
        .into_iter()
        .find(|r| r.kind() == kind && r.id() == id)
        .ok_or(AtlasError::SaveSourceGone)?;
    rec.catalog().read(|c| finish(c, row))?
}

/// A page of bookmarks and findings, most recently saved or changed first,
/// each anchor resolved against the catalog now.
pub fn list(rec: &Recorder, query: &SavedQuery) -> AtlasResult<SavedPage> {
    let page_size = query.page_size.clamp(1, MAX_PAGE_SIZE);
    let filter = fold(query.filter.trim());
    let mut rows: Vec<Row> = rec
        .store()
        .read(rows)?
        .map_err(AtlasError::db("be read"))?
        .into_iter()
        .filter(|r| query.kind.is_none_or(|k| r.kind() == k))
        .filter(|r| {
            query
                .document_id
                .as_deref()
                .is_none_or(|d| r.documents().contains(&d))
        })
        .filter(|r| filter.is_empty() || r.text().contains(&filter))
        .collect();
    rows.sort_by(|a, b| {
        b.updated_at()
            .cmp(a.updated_at())
            .then_with(|| a.id().cmp(b.id()))
    });
    let total = rows.len() as u32;
    let start = (query.page as usize).saturating_mul(page_size as usize);
    let shown: Vec<Row> = rows
        .into_iter()
        .skip(start)
        .take(page_size as usize)
        .collect();
    let items = rec.catalog().read(|c| {
        shown
            .into_iter()
            .map(|r| finish(c, r))
            .collect::<AtlasResult<Vec<_>>>()
    })??;
    Ok(SavedPage {
        items,
        page: query.page,
        page_size,
        total,
    })
}
