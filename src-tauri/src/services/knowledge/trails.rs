//! Trails in the knowledge store (phase3-spec §7.1, §7.2; ADR 0017): the
//! session rule, appending and merging steps, reading them back with their
//! anchors resolved, deleting, and expiry.
//!
//! Every write here is one transaction, which the caller opens and
//! commits. Nothing here reads the catalog except [`get`], which resolves
//! anchors against it and never writes it.

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use time::{Duration, OffsetDateTime};

use super::anchor::{self, from_hex};
use super::meta;
use super::store::Inner;
use crate::domain::dto::{
    Anchor, KnowledgeState, StepPayload, TrailDetail, TrailPage, TrailQuery, TrailStep,
    TrailSummary,
};
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::clock::{parse_rfc3339, rfc3339};
use crate::services::library::fold::fold;

/// A step within this long of the last one joins its trail.
pub const SESSION: Duration = Duration::minutes(30);
/// Unpinned trails in no collection go this long after their last step.
pub const EXPIRE_AFTER: Duration = Duration::days(90);
/// Expiry runs at most this often.
pub const EXPIRY_EVERY: Duration = Duration::hours(24);
pub const MAX_TITLE_CHARS: usize = 200;
/// Notes are plain text (phase3-spec §2).
pub const MAX_NOTE_CHARS: usize = 4_000;
pub const MAX_PAGE_SIZE: u32 = 100;
/// The version every payload is written at.
pub const PAYLOAD_VERSION: u32 = 1;
/// A label longer than this is cut.
const LABEL_CHARS: usize = 200;

fn json_err(e: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(e))
}

fn read_err(e: rusqlite::Error) -> AtlasError {
    AtlasError::db("be read")(e)
}

/// Which trail a new step joins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// The current trail, or a new one (the session rule).
    Current,
    /// This trail, which must exist (a note the user adds to it).
    Trail(String),
}

/// What [`append`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Appended {
    pub trail_id: String,
    pub step_id: String,
    /// The step updated the trail's last one instead of adding one.
    pub merged: bool,
}

fn kind(payload: &StepPayload) -> &'static str {
    match payload {
        StepPayload::Search(_) => "search",
        StepPayload::Ask(_) => "ask",
        StepPayload::Open(_) => "open",
        StepPayload::Note(_) => "note",
    }
}

/// The anchors of a step, in the order `TrailStep::anchors` resolves them.
pub fn anchors(payload: &StepPayload) -> Vec<&Anchor> {
    match payload {
        StepPayload::Search(s) => s.results.iter().map(|r| &r.anchor).collect(),
        StepPayload::Ask(a) => a.sources.iter().map(|s| &s.anchor).collect(),
        StepPayload::Open(o) => vec![&o.anchor],
        StepPayload::Note(_) => Vec::new(),
    }
}

/// The documents a step touched: a search's results, a question's cited
/// sources, the opened document.
fn documents(payload: &StepPayload) -> Vec<(&str, Vec<u8>)> {
    let anchors: Vec<&Anchor> = match payload {
        StepPayload::Ask(a) => a
            .sources
            .iter()
            .filter(|s| s.cited)
            .map(|s| &s.anchor)
            .collect(),
        other => anchors(other),
    };
    anchors
        .into_iter()
        .filter_map(|a| Some((a.document_id.as_str(), from_hex(&a.content_hash)?)))
        .collect()
}

/// The step a new one replaces: the same search again, or the same
/// document opened again the same way (its actions are added).
fn merge(last: &StepPayload, new: &StepPayload) -> Option<StepPayload> {
    match (last, new) {
        (StepPayload::Search(a), StepPayload::Search(b))
            if a.words == b.words
                && a.filters.source_root_id == b.filters.source_root_id
                && a.filters.file_type == b.filters.file_type =>
        {
            Some(new.clone())
        }
        (StepPayload::Open(a), StepPayload::Open(b))
            if a.anchor.document_id == b.anchor.document_id && a.via == b.via =>
        {
            let mut merged = a.clone();
            for action in &b.actions {
                if !merged.actions.contains(action) {
                    merged.actions.push(*action);
                }
            }
            Some(StepPayload::Open(merged))
        }
        _ => None,
    }
}

fn exists(conn: &Connection, trail_id: &str) -> rusqlite::Result<bool> {
    conn.query_row("SELECT 1 FROM trails WHERE id = ?1", [trail_id], |_| Ok(()))
        .optional()
        .map(|r| r.is_some())
}

/// The trail a step recorded now joins, if any: the one whose last step
/// (or Continue) was under [`SESSION`] ago. It survives a relaunch.
pub fn current(conn: &Connection, now: OffsetDateTime) -> rusqlite::Result<Option<String>> {
    let Some(id) = meta::get(conn, "current_trail")? else {
        return Ok(None);
    };
    let until = meta::get(conn, "current_trail_until")?.and_then(|t| parse_rfc3339(&t));
    if until.is_some_and(|u| now < u) && exists(conn, &id)? {
        Ok(Some(id))
    } else {
        Ok(None)
    }
}

fn make_current(conn: &Connection, trail_id: &str, now: OffsetDateTime) -> rusqlite::Result<()> {
    meta::set(conn, "current_trail", trail_id)?;
    meta::set(conn, "current_trail_until", &rfc3339(now + SESSION))
}

fn set_documents(tx: &Transaction, step_id: &str, payload: &StepPayload) -> rusqlite::Result<()> {
    tx.execute("DELETE FROM step_documents WHERE step_id = ?1", [step_id])?;
    let mut insert = tx.prepare_cached(
        "INSERT OR IGNORE INTO step_documents (step_id, document_id, content_hash)
         VALUES (?1, ?2, ?3)",
    )?;
    for (document_id, hash) in documents(payload) {
        insert.execute(params![step_id, document_id, hash])?;
    }
    Ok(())
}

/// Rewrites a trail's filter text: its title, questions and searches,
/// folded.
fn refresh_search_text(tx: &Transaction, trail_id: &str) -> rusqlite::Result<()> {
    let title: Option<String> =
        tx.query_row("SELECT title FROM trails WHERE id = ?1", [trail_id], |r| {
            r.get(0)
        })?;
    let mut stmt = tx.prepare_cached(
        "SELECT COALESCE(json_extract(payload, '$.question'), json_extract(payload, '$.words'))
         FROM trail_steps WHERE trail_id = ?1 AND kind IN ('search', 'ask') ORDER BY seq",
    )?;
    let mut parts: Vec<String> = title.into_iter().collect();
    for text in stmt.query_map([trail_id], |r| r.get::<_, Option<String>>(0))? {
        parts.extend(text?);
    }
    tx.execute(
        "UPDATE trails SET search_text = ?2 WHERE id = ?1",
        params![trail_id, fold(&parts.join("\n"))],
    )?;
    Ok(())
}

/// Recounts a trail's steps and its last activity. A trail left with no
/// steps is deleted.
fn refresh_trail(tx: &Transaction, trail_id: &str) -> rusqlite::Result<()> {
    let (count, last): (u32, Option<String>) = tx.query_row(
        "SELECT count(*), max(at) FROM trail_steps WHERE trail_id = ?1",
        [trail_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let Some(last) = last.filter(|_| count > 0) else {
        delete(tx, trail_id)?;
        return Ok(());
    };
    tx.execute(
        "UPDATE trails SET step_count = ?2, last_step_at = ?3 WHERE id = ?1",
        params![trail_id, count, last],
    )?;
    refresh_search_text(tx, trail_id)
}

/// Adds `payload` as a step at `now`, or merges it into the trail's last
/// step (the same search again; the same source opened again the same
/// way). A step joining the current trail keeps the session going.
pub fn append(
    tx: &Transaction,
    now: OffsetDateTime,
    target: &Target,
    payload: &StepPayload,
) -> rusqlite::Result<Appended> {
    let at = rfc3339(now);
    let existing = match target {
        Target::Current => current(tx, now)?,
        Target::Trail(id) if exists(tx, id)? => Some(id.clone()),
        Target::Trail(_) => return Err(rusqlite::Error::QueryReturnedNoRows),
    };
    let trail_id = match existing {
        Some(id) => id,
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO trails (id, started_at, last_step_at) VALUES (?1, ?2, ?2)",
                params![id, at],
            )?;
            id
        }
    };

    let last: Option<(String, String)> = tx
        .query_row(
            "SELECT id, payload FROM trail_steps WHERE trail_id = ?1 ORDER BY seq DESC LIMIT 1",
            [&trail_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let merged = last.and_then(|(id, text)| {
        let old: StepPayload = serde_json::from_str(&text).ok()?;
        Some((id, merge(&old, payload)?))
    });
    let (step_id, merged) = match merged {
        Some((step_id, payload)) => {
            let text = serde_json::to_string(&payload).map_err(json_err)?;
            tx.execute(
                "UPDATE trail_steps SET at = ?2, payload = ?3 WHERE id = ?1",
                params![step_id, at, text],
            )?;
            set_documents(tx, &step_id, &payload)?;
            (step_id, true)
        }
        None => {
            let step_id = uuid::Uuid::new_v4().to_string();
            let text = serde_json::to_string(payload).map_err(json_err)?;
            tx.execute(
                "INSERT INTO trail_steps (id, trail_id, seq, kind, at, payload)
                 VALUES (?1, ?2,
                         (SELECT COALESCE(max(seq), 0) + 1 FROM trail_steps WHERE trail_id = ?2),
                         ?3, ?4, ?5)",
                params![step_id, trail_id, kind(payload), at, text],
            )?;
            set_documents(tx, &step_id, payload)?;
            (step_id, false)
        }
    };
    refresh_trail(tx, &trail_id)?;
    if *target == Target::Current {
        make_current(tx, &trail_id, now)?;
    }
    if meta::get(tx, "first_step_noted")?.is_none() {
        meta::set(tx, "first_step_noted", "0")?;
    }
    Ok(Appended {
        trail_id,
        step_id,
        merged,
    })
}

/// The steps of the trail the next step joins, if it is still current.
pub fn current_steps(
    conn: &Connection,
    now: OffsetDateTime,
) -> rusqlite::Result<Option<(String, u32)>> {
    let Some(id) = current(conn, now)? else {
        return Ok(None);
    };
    let count = conn.query_row("SELECT step_count FROM trails WHERE id = ?1", [&id], |r| {
        r.get(0)
    })?;
    Ok(Some((id, count)))
}

/// Whether Home's one-time note is waiting to be read.
pub fn first_step_note(conn: &Connection) -> rusqlite::Result<bool> {
    Ok(meta::get(conn, "first_step_noted")?.as_deref() == Some("0"))
}

pub fn dismiss_first_step_note(conn: &Connection) -> rusqlite::Result<()> {
    meta::set(conn, "first_step_noted", "1")
}

// ------------------------------------------------------------- reading

const SUMMARY: &str = "SELECT t.id, t.title, t.pinned, t.step_count, t.started_at, t.last_step_at,
        (SELECT COALESCE(json_extract(s.payload, '$.question'), json_extract(s.payload, '$.words'),
                         json_extract(s.payload, '$.anchor.title'), json_extract(s.payload, '$.text'))
         FROM trail_steps s WHERE s.trail_id = t.id ORDER BY s.seq LIMIT 1)
    FROM trails t";

fn summary_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<TrailSummary> {
    let title: Option<String> = r.get(1)?;
    let first: Option<String> = r.get(6)?;
    let label = title.clone().or(first).unwrap_or_default();
    Ok(TrailSummary {
        id: r.get(0)?,
        label: match label.char_indices().nth(LABEL_CHARS) {
            Some((at, _)) => format!("{}…", &label[..at]),
            None => label,
        },
        title,
        pinned: r.get::<_, i64>(2)? != 0,
        step_count: r.get(3)?,
        started_at: r.get(4)?,
        last_step_at: r.get(5)?,
    })
}

pub fn summary(conn: &Connection, trail_id: &str) -> rusqlite::Result<Option<TrailSummary>> {
    conn.query_row(
        &format!("{SUMMARY} WHERE t.id = ?1"),
        [trail_id],
        summary_row,
    )
    .optional()
}

/// `text` for a LIKE pattern, with `\` as the escape.
fn like(text: &str) -> String {
    let mut out = String::from("%");
    for c in text.chars() {
        if matches!(c, '\\' | '%' | '_') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

/// A page of trails, newest activity first, filtered by their titles,
/// questions and searches.
pub fn list(conn: &Connection, query: &TrailQuery) -> rusqlite::Result<TrailPage> {
    let page_size = query.page_size.clamp(1, MAX_PAGE_SIZE);
    let filter = fold(query.filter.trim());
    let pattern = like(&filter);
    let filtered = "(?1 = '' OR t.search_text LIKE ?2 ESCAPE '\\')";
    let total: u32 = conn.query_row(
        &format!("SELECT count(*) FROM trails t WHERE {filtered}"),
        params![filter, pattern],
        |r| r.get(0),
    )?;
    let mut stmt = conn.prepare_cached(&format!(
        "{SUMMARY} WHERE {filtered} ORDER BY t.last_step_at DESC, t.id LIMIT ?3 OFFSET ?4"
    ))?;
    let trails = stmt
        .query_map(
            params![
                filter,
                pattern,
                page_size,
                i64::from(query.page) * i64::from(page_size)
            ],
            summary_row,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(TrailPage {
        trails,
        page: query.page,
        page_size,
        total,
    })
}

/// A trail with its steps, oldest first, each anchor resolved against
/// `catalog` as it is now. A step this build can't read is left out.
pub fn get(
    knowledge: &Connection,
    catalog: &Connection,
    trail_id: &str,
) -> AtlasResult<Option<TrailDetail>> {
    let Some(trail) = summary(knowledge, trail_id).map_err(read_err)? else {
        return Ok(None);
    };
    let mut stmt = knowledge
        .prepare_cached(
            "SELECT id, seq, at, note, payload FROM trail_steps WHERE trail_id = ?1 ORDER BY seq",
        )
        .map_err(read_err)?;
    let rows = stmt
        .query_map([trail_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, String>(4)?,
            ))
        })
        .map_err(read_err)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(read_err)?;
    let mut steps = Vec::with_capacity(rows.len());
    for (id, seq, at, note, text) in rows {
        let Ok(payload) = serde_json::from_str::<StepPayload>(&text) else {
            continue;
        };
        let anchors = anchors(&payload)
            .into_iter()
            .map(|a| anchor::resolve(catalog, a))
            .collect::<AtlasResult<Vec<_>>>()?;
        steps.push(TrailStep {
            id,
            seq,
            at,
            note,
            payload,
            anchors,
        });
    }
    Ok(Some(TrailDetail { trail, steps }))
}

// ------------------------------------------------------------- changes

/// "New trail": the next step starts one. No empty trail is made.
pub fn start(conn: &Connection) -> rusqlite::Result<()> {
    meta::remove(conn, "current_trail")?;
    meta::remove(conn, "current_trail_until")
}

/// "Continue this trail": new steps join it for the next [`SESSION`].
pub fn continue_trail(
    conn: &Connection,
    trail_id: &str,
    now: OffsetDateTime,
) -> rusqlite::Result<Option<TrailSummary>> {
    if !exists(conn, trail_id)? {
        return Ok(None);
    }
    make_current(conn, trail_id, now)?;
    summary(conn, trail_id)
}

/// A title of at most [`MAX_TITLE_CHARS`], trimmed; empty is none.
pub fn checked_title(text: &str) -> AtlasResult<Option<String>> {
    checked(text, "title", MAX_TITLE_CHARS)
}

/// A note of at most [`MAX_NOTE_CHARS`], trimmed; empty is none.
pub fn checked_note(text: &str) -> AtlasResult<Option<String>> {
    checked(text, "note", MAX_NOTE_CHARS)
}

fn checked(text: &str, what: &'static str, max: usize) -> AtlasResult<Option<String>> {
    let text = text.trim();
    if text.chars().count() > max {
        return Err(AtlasError::TextTooLong { what, max });
    }
    Ok((!text.is_empty()).then(|| text.to_owned()))
}

pub fn rename(tx: &Transaction, trail_id: &str, title: Option<&str>) -> rusqlite::Result<bool> {
    let n = tx.execute(
        "UPDATE trails SET title = ?2 WHERE id = ?1",
        params![trail_id, title],
    )?;
    if n > 0 {
        refresh_search_text(tx, trail_id)?;
    }
    Ok(n > 0)
}

pub fn pin(conn: &Connection, trail_id: &str, pinned: bool) -> rusqlite::Result<bool> {
    conn.execute(
        "UPDATE trails SET pinned = ?2 WHERE id = ?1",
        params![trail_id, i64::from(pinned)],
    )
    .map(|n| n > 0)
}

/// Sets (or, with `None`, clears) the note on a step.
pub fn set_step_note(
    tx: &Transaction,
    step_id: &str,
    note: Option<&str>,
) -> rusqlite::Result<bool> {
    tx.execute(
        "UPDATE trail_steps SET note = ?2 WHERE id = ?1",
        params![step_id, note],
    )
    .map(|n| n > 0)
}

/// Deletes a trail, its steps and its place in any collection. Findings
/// keep their own copies and are untouched.
pub fn delete(tx: &Transaction, trail_id: &str) -> rusqlite::Result<bool> {
    tx.execute(
        "DELETE FROM collection_items WHERE item_kind = 'trail' AND item_id = ?1",
        [trail_id],
    )?;
    let n = tx.execute("DELETE FROM trails WHERE id = ?1", [trail_id])?;
    if meta::get(tx, "current_trail")?.as_deref() == Some(trail_id) {
        start(tx)?;
    }
    Ok(n > 0)
}

/// Deletes one step; a trail left without steps goes with it.
pub fn delete_step(tx: &Transaction, step_id: &str) -> rusqlite::Result<bool> {
    let trail_id: Option<String> = tx
        .query_row(
            "SELECT trail_id FROM trail_steps WHERE id = ?1",
            [step_id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(trail_id) = trail_id else {
        return Ok(false);
    };
    tx.execute("DELETE FROM trail_steps WHERE id = ?1", [step_id])?;
    refresh_trail(tx, &trail_id)?;
    Ok(true)
}

/// Deletes every trail; bookmarks and findings stay. The number deleted.
pub fn delete_all(tx: &Transaction) -> rusqlite::Result<u32> {
    tx.execute("DELETE FROM collection_items WHERE item_kind = 'trail'", [])?;
    let n = tx.execute("DELETE FROM trails", [])?;
    start(tx)?;
    Ok(n as u32)
}

/// Deletes the trails that are unpinned, in no collection, and whose last
/// step is over [`EXPIRE_AFTER`] before `now`. The number deleted.
pub fn expire(tx: &Transaction, now: OffsetDateTime) -> rusqlite::Result<u32> {
    let before = rfc3339(now - EXPIRE_AFTER);
    tx.execute(
        "DELETE FROM trails WHERE pinned = 0 AND last_step_at < ?1
           AND id NOT IN (SELECT item_id FROM collection_items WHERE item_kind = 'trail')",
        [before],
    )
    .map(|n| n as u32)
}

/// Runs [`expire`] if it is switched on and a day has passed since it
/// last ran. `None` when it didn't run.
pub fn expire_if_due(inner: &Inner) -> Option<u32> {
    if inner.state() != KnowledgeState::Ready || !inner.trail_settings().expiring() {
        return None;
    }
    let now = inner.clock().now();
    inner
        .call(move |c| -> rusqlite::Result<Option<u32>> {
            let last = meta::get(c, "last_expiry_at")?.and_then(|t| parse_rfc3339(&t));
            if last.is_some_and(|l| now - l < EXPIRY_EVERY) {
                return Ok(None);
            }
            let tx = c.transaction()?;
            let n = expire(&tx, now)?;
            meta::set(&tx, "last_expiry_at", &rfc3339(now))?;
            tx.commit()?;
            Ok(Some(n))
        })
        .ok()
        .and_then(Result::ok)
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_escape_like_wildcards() {
        assert_eq!(like("50%_off\\"), "%50\\%\\_off\\\\%");
        assert_eq!(like(""), "%%");
    }

    #[test]
    fn titles_and_notes_are_trimmed_and_capped() {
        assert_eq!(
            checked_title("  Stoics  ").unwrap().as_deref(),
            Some("Stoics")
        );
        assert_eq!(checked_title("   ").unwrap(), None);
        assert!(checked_title(&"é".repeat(MAX_TITLE_CHARS)).is_ok());
        assert!(matches!(
            checked_title(&"é".repeat(MAX_TITLE_CHARS + 1)),
            Err(AtlasError::TextTooLong { what: "title", .. })
        ));
        assert!(checked_note(&"a".repeat(MAX_NOTE_CHARS)).is_ok());
        assert!(checked_note(&"a".repeat(MAX_NOTE_CHARS + 1)).is_err());
    }
}
