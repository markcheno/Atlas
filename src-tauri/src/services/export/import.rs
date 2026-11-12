//! Import of a JSON export (Slice 3.8, phase3-spec §7.7, ADR 0019).
//!
//! The file is untrusted: at most [`MAX_BYTES`], a known format and
//! version, every row checked like user input. A bad file is refused
//! whole and nothing is written. A dry run ([`merge`] with `dry`) reports
//! what would change; applying runs the same code in one transaction,
//! after a snapshot, so an import can be undone.
//!
//! Merging adds what is new and, for what is already here, keeps the
//! newer copy by `updated_at`. It never deletes: a trail keeps steps the
//! file doesn't have, and links and "Not for this" are only added.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use time::OffsetDateTime;

use super::json::{
    BookmarkRow, CollectionRow, FORMAT, FindingRow, ItemKind, KnowledgeFile, StepRow, TrailRow,
    VERSION,
};
use crate::domain::dto::{
    Anchor, CollectionKind, CollectionStatus, FindingKind, ImportCounts, ImportSummary,
    SnapshotOutcome, StepPayload,
};
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::clock::{parse_rfc3339, rfc3339};
use crate::services::knowledge::anchor::{ANCHOR_EXCERPT_CHARS, from_hex};
use crate::services::knowledge::store::Inner;
use crate::services::knowledge::trails::{
    self, EXPIRE_AFTER, MAX_NOTE_CHARS, MAX_TITLE_CHARS, PAYLOAD_VERSION,
};
use crate::services::knowledge::{collections, snapshot};

/// The largest file Atlas imports.
pub const MAX_BYTES: u64 = 256 * 1024 * 1024;
/// Rows of every kind, together.
pub const MAX_ROWS: usize = 1_000_000;
/// How long a dry run's file is kept for `apply_import`.
pub const PENDING_FOR: Duration = Duration::from_secs(10 * 60);
/// Recorded text Atlas has no limit of its own for.
const MAX_TEXT_CHARS: usize = 100_000;
const MAX_SHORT_CHARS: usize = 4_000;
const MAX_ID_CHARS: usize = 64;
const MAX_HASH_CHARS: usize = 128;

// ------------------------------------------------------------- reading

#[derive(Deserialize)]
struct Header {
    format: Option<String>,
    version: Option<u32>,
}

/// Reads and checks `path`, chosen in the native dialog.
pub fn read(path: &Path) -> AtlasResult<KnowledgeFile> {
    let file = fs::File::open(path).map_err(|_| AtlasError::ImportUnreadable)?;
    let meta = file.metadata().map_err(|_| AtlasError::ImportUnreadable)?;
    if !meta.is_file() {
        return Err(AtlasError::ImportUnreadable);
    }
    if meta.len() > MAX_BYTES {
        return Err(AtlasError::ImportTooLarge);
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| AtlasError::ImportUnreadable)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(AtlasError::ImportTooLarge);
    }
    parse(&bytes)
}

/// Parses and checks an export's bytes.
pub fn parse(bytes: &[u8]) -> AtlasResult<KnowledgeFile> {
    let known = |h: &Header| h.format.as_deref() == Some(FORMAT) && h.version == Some(VERSION);
    let file = match serde_json::from_slice::<KnowledgeFile>(bytes) {
        Ok(file) => file,
        Err(_) => {
            // Whole JSON of another format or version, or a row Atlas
            // can't read, or not whole JSON at all.
            return Err(match serde_json::from_slice::<Header>(bytes) {
                Ok(h) if !known(&h) => AtlasError::ImportUnknownVersion,
                Ok(_) => AtlasError::ImportInvalid("row"),
                Err(_) => AtlasError::ImportUnreadable,
            });
        }
    };
    if file.format != FORMAT || file.version != VERSION {
        return Err(AtlasError::ImportUnknownVersion);
    }
    validate(&file).map_err(AtlasError::ImportInvalid)?;
    Ok(file)
}

// ------------------------------------------------------------- checks

type Check = Result<(), &'static str>;

fn uuid(id: &str) -> Check {
    uuid::Uuid::parse_str(id).map(|_| ()).map_err(|_| "id")
}

fn doc_id(id: &str) -> Check {
    if id.is_empty() || id.chars().count() > MAX_ID_CHARS || id.chars().any(char::is_control) {
        return Err("document");
    }
    Ok(())
}

fn hash(text: &str) -> Check {
    if text.len() > MAX_HASH_CHARS || from_hex(text).is_none() {
        return Err("hash");
    }
    Ok(())
}

/// An RFC 3339 time exactly as Atlas writes them (UTC, to the second), so
/// times compare as text.
fn at(text: &str) -> Check {
    match parse_rfc3339(text) {
        Some(t) if text.ends_with('Z') && rfc3339(t) == text => Ok(()),
        _ => Err("time"),
    }
}

fn chars(text: &str, max: usize, what: &'static str) -> Check {
    if text.chars().count() > max {
        return Err(what);
    }
    Ok(())
}

fn opt_chars(text: &Option<String>, max: usize, what: &'static str) -> Check {
    text.as_deref().map_or(Ok(()), |t| chars(t, max, what))
}

fn anchor(a: &Anchor) -> Check {
    doc_id(&a.document_id)?;
    hash(&a.content_hash)?;
    chars(&a.title, MAX_SHORT_CHARS, "title")?;
    opt_chars(&a.location, MAX_SHORT_CHARS, "location")?;
    opt_chars(&a.excerpt, ANCHOR_EXCERPT_CHARS, "excerpt")
}

fn payload(p: &StepPayload) -> Check {
    match p {
        StepPayload::Search(s) => {
            if s.v != PAYLOAD_VERSION {
                return Err("payload");
            }
            chars(&s.words, MAX_SHORT_CHARS, "words")?;
            opt_chars(&s.filters.folder_name, MAX_SHORT_CHARS, "folder")?;
            s.results.iter().try_for_each(|r| anchor(&r.anchor))
        }
        StepPayload::Ask(a) => {
            if a.v != PAYLOAD_VERSION {
                return Err("payload");
            }
            chars(&a.question, MAX_SHORT_CHARS, "question")?;
            chars(&a.answer, MAX_TEXT_CHARS, "answer")?;
            opt_chars(&a.model, MAX_SHORT_CHARS, "model")?;
            opt_chars(&a.filters.folder_name, MAX_SHORT_CHARS, "folder")?;
            a.sources.iter().try_for_each(|s| anchor(&s.anchor))
        }
        StepPayload::Open(o) => {
            if o.v != PAYLOAD_VERSION {
                return Err("payload");
            }
            anchor(&o.anchor)
        }
        StepPayload::Note(n) => {
            if n.v != PAYLOAD_VERSION {
                return Err("payload");
            }
            if n.text.trim().is_empty() {
                return Err("note");
            }
            chars(&n.text, MAX_NOTE_CHARS, "note")
        }
    }
}

fn trail(t: &TrailRow, step_ids: &mut HashSet<String>) -> Check {
    uuid(&t.id)?;
    opt_chars(&t.title, MAX_TITLE_CHARS, "title")?;
    at(&t.started_at)?;
    at(&t.updated_at)?;
    if t.steps.is_empty() {
        return Err("steps");
    }
    let mut seqs = HashSet::new();
    for s in &t.steps {
        step(s)?;
        if s.seq == 0 || !seqs.insert(s.seq) {
            return Err("seq");
        }
        if !step_ids.insert(s.id.clone()) {
            return Err("duplicate");
        }
    }
    Ok(())
}

fn step(s: &StepRow) -> Check {
    uuid(&s.id)?;
    at(&s.at)?;
    at(&s.updated_at)?;
    opt_chars(&s.note, MAX_NOTE_CHARS, "note")?;
    payload(&s.payload)
}

fn bookmark(b: &BookmarkRow) -> Check {
    uuid(&b.id)?;
    anchor(&b.anchor)?;
    opt_chars(&b.note, MAX_NOTE_CHARS, "note")?;
    at(&b.created_at)?;
    at(&b.updated_at)
}

fn finding(f: &FindingRow) -> Check {
    uuid(&f.id)?;
    if let Some(s) = &f.from_step_id {
        uuid(s)?;
    }
    opt_chars(&f.question, MAX_SHORT_CHARS, "question")?;
    chars(&f.text, MAX_TEXT_CHARS, "text")?;
    opt_chars(&f.note, MAX_NOTE_CHARS, "note")?;
    if f.sources.is_empty() && f.kind == FindingKind::Passage {
        return Err("sources");
    }
    f.sources.iter().try_for_each(|s| anchor(&s.anchor))?;
    at(&f.created_at)?;
    at(&f.updated_at)
}

fn collection(c: &CollectionRow) -> Check {
    uuid(&c.id)?;
    if c.name.trim().is_empty() {
        return Err("name");
    }
    chars(&c.name, collections::MAX_NAME_CHARS, "name")?;
    opt_chars(&c.description, MAX_SHORT_CHARS, "description")?;
    opt_chars(&c.goal, MAX_SHORT_CHARS, "goal")?;
    if c.kind == CollectionKind::Interest
        && (c.goal.is_some() || c.status == CollectionStatus::Finished)
    {
        return Err("collection");
    }
    at(&c.created_at)?;
    at(&c.updated_at)?;
    let mut seen = HashSet::new();
    for i in &c.items {
        match i.kind {
            ItemKind::Document => {
                doc_id(&i.id)?;
                hash(i.content_hash.as_deref().ok_or("hash")?)?;
            }
            _ => {
                uuid(&i.id)?;
                if i.content_hash.is_some() {
                    return Err("hash");
                }
            }
        }
        at(&i.added_at)?;
        if !seen.insert((i.kind, i.id.clone())) {
            return Err("duplicate");
        }
    }
    Ok(())
}

/// Every row checked like user input; the file's own ids unique.
fn validate(file: &KnowledgeFile) -> Check {
    at(&file.exported)?;
    let rows = file.trails.len()
        + file.trails.iter().map(|t| t.steps.len()).sum::<usize>()
        + file.bookmarks.len()
        + file.findings.len()
        + file.collections.len()
        + file
            .collections
            .iter()
            .map(|c| c.items.len())
            .sum::<usize>()
        + file.dismissed_suggestions.len();
    if rows > MAX_ROWS {
        return Err("rows");
    }
    let mut ids = HashSet::new();
    let mut unique = |id: &str| -> Check {
        if ids.insert(id.to_owned()) {
            Ok(())
        } else {
            Err("duplicate")
        }
    };
    let mut steps = HashSet::new();
    for t in &file.trails {
        trail(t, &mut steps)?;
        unique(&t.id)?;
    }
    for b in &file.bookmarks {
        bookmark(b)?;
        unique(&b.id)?;
    }
    for f in &file.findings {
        finding(f)?;
        unique(&f.id)?;
    }
    for c in &file.collections {
        collection(c)?;
        unique(&c.id)?;
    }
    let mut dismissed = HashSet::new();
    for d in &file.dismissed_suggestions {
        if !d.collection_id.is_empty() {
            uuid(&d.collection_id)?;
        }
        doc_id(&d.document_id)?;
        hash(&d.content_hash)?;
        at(&d.at)?;
        if !dismissed.insert((&d.collection_id, &d.document_id)) {
            return Err("duplicate");
        }
    }
    Ok(())
}

// ------------------------------------------------------------- merging

fn tally(c: &mut ImportCounts, class: Class) {
    match class {
        Class::New => c.new += 1,
        Class::Newer => c.newer += 1,
        Class::Here => c.here += 1,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    New,
    Newer,
    Here,
}

/// What merging found, or a reason the file is refused.
pub struct Merged {
    pub summary: ImportSummary,
    /// New unpinned trails older than the expiry, in no collection.
    pub expiring: u32,
}

fn json_err(e: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(e))
}

fn updated(c: &Connection, table: &str, id: &str) -> rusqlite::Result<Option<String>> {
    c.query_row(
        &format!("SELECT updated_at FROM {table} WHERE id = ?1"),
        [id],
        |r| r.get(0),
    )
    .optional()
}

fn exists(c: &Connection, table: &str, id: &str) -> rusqlite::Result<bool> {
    c.query_row(
        &format!("SELECT 1 FROM {table} WHERE id = ?1"),
        [id],
        |_| Ok(()),
    )
    .optional()
    .map(|r| r.is_some())
}

fn insert_step(c: &Connection, trail_id: &str, s: &StepRow, seq: i64) -> rusqlite::Result<()> {
    let text = serde_json::to_string(&s.payload).map_err(json_err)?;
    let kind = match &s.payload {
        StepPayload::Search(_) => "search",
        StepPayload::Ask(_) => "ask",
        StepPayload::Open(_) => "open",
        StepPayload::Note(_) => "note",
    };
    c.execute(
        "INSERT INTO trail_steps (id, trail_id, seq, kind, at, payload, note, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![s.id, trail_id, seq, kind, s.at, text, s.note, s.updated_at],
    )?;
    Ok(())
}

/// One trail: new, or combined step by step with the one here.
fn merge_trail(
    c: &Connection,
    t: &TrailRow,
    dry: bool,
) -> rusqlite::Result<Result<Class, &'static str>> {
    let Some(local) = updated(c, "trails", &t.id)? else {
        if !dry {
            c.execute(
                "INSERT INTO trails (id, title, pinned, started_at, last_step_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?4, ?5)",
                params![
                    t.id,
                    t.title,
                    i64::from(t.pinned),
                    t.started_at,
                    t.updated_at
                ],
            )?;
        }
        for s in &t.steps {
            if exists(c, "trail_steps", &s.id)? {
                // A step of another trail here: the file contradicts it.
                return Ok(Err("step"));
            }
            if !dry {
                insert_step(c, &t.id, s, i64::from(s.seq))?;
                trails::set_documents(c, &s.id, &s.payload)?;
            }
        }
        if !dry {
            trails::refresh_trail(c, &t.id)?;
        }
        return Ok(Ok(Class::New));
    };
    let mut changed = false;
    if t.updated_at > local {
        changed = true;
        if !dry {
            c.execute(
                "UPDATE trails SET title = ?2, pinned = ?3, updated_at = ?4 WHERE id = ?1",
                params![t.id, t.title, i64::from(t.pinned), t.updated_at],
            )?;
        }
    }
    let mut added = false;
    for s in &t.steps {
        let here: Option<(String, String)> = c
            .query_row(
                "SELECT trail_id, updated_at FROM trail_steps WHERE id = ?1",
                [&s.id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match here {
            Some((trail, _)) if trail != t.id => return Ok(Err("step")),
            Some((_, local_step)) if s.updated_at > local_step => {
                changed = true;
                if !dry {
                    let text = serde_json::to_string(&s.payload).map_err(json_err)?;
                    c.execute(
                        "UPDATE trail_steps SET at = ?2, payload = ?3, note = ?4, updated_at = ?5
                         WHERE id = ?1",
                        params![s.id, s.at, text, s.note, s.updated_at],
                    )?;
                    trails::set_documents(c, &s.id, &s.payload)?;
                }
            }
            Some(_) => {}
            None => {
                changed = true;
                added = true;
                if !dry {
                    // Numbered after the others for now; renumbered below.
                    let next: i64 = c.query_row(
                        "SELECT COALESCE(max(seq), 0) + 1 FROM trail_steps WHERE trail_id = ?1",
                        [&t.id],
                        |r| r.get(0),
                    )?;
                    insert_step(c, &t.id, s, next)?;
                    trails::set_documents(c, &s.id, &s.payload)?;
                }
            }
        }
    }
    if !dry && changed {
        if added {
            renumber(c, &t.id)?;
        }
        trails::refresh_trail(c, &t.id)?;
        trails::touch(c, &t.id, &t.updated_at)?;
    }
    Ok(Ok(if changed { Class::Newer } else { Class::Here }))
}

/// Steps in order of time once two copies are combined.
fn renumber(c: &Connection, trail_id: &str) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE trail_steps SET seq = -seq WHERE trail_id = ?1",
        [trail_id],
    )?;
    let ids: Vec<String> = {
        let mut stmt =
            c.prepare("SELECT id FROM trail_steps WHERE trail_id = ?1 ORDER BY at, id")?;
        stmt.query_map([trail_id], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };
    for (i, id) in ids.iter().enumerate() {
        c.execute(
            "UPDATE trail_steps SET seq = ?2 WHERE id = ?1",
            params![id, i as i64 + 1],
        )?;
    }
    Ok(())
}

/// A bookmark: matched by id, or by its place, as saving matches. Returns
/// the id it has here.
fn merge_bookmark(c: &Connection, b: &BookmarkRow, dry: bool) -> rusqlite::Result<(Class, String)> {
    let by_place = || -> rusqlite::Result<Option<String>> {
        c.query_row(
            "SELECT id FROM bookmarks
             WHERE document_id = ?1 AND json_extract(anchor, '$.ordinal') IS ?2",
            params![b.anchor.document_id, b.anchor.ordinal],
            |r| r.get(0),
        )
        .optional()
    };
    let id = if exists(c, "bookmarks", &b.id)? {
        Some(b.id.clone())
    } else {
        by_place()?
    };
    let text = serde_json::to_string(&b.anchor).map_err(json_err)?;
    let hash = from_hex(&b.anchor.content_hash).unwrap_or_default();
    let Some(id) = id else {
        if !dry {
            c.execute(
                "INSERT INTO bookmarks (id, anchor, document_id, content_hash, note, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![b.id, text, b.anchor.document_id, hash, b.note, b.created_at, b.updated_at],
            )?;
        }
        return Ok((Class::New, b.id.clone()));
    };
    let local = updated(c, "bookmarks", &id)?.unwrap_or_default();
    if b.updated_at <= local {
        return Ok((Class::Here, id));
    }
    if !dry {
        c.execute(
            "UPDATE bookmarks SET anchor = ?2, document_id = ?3, content_hash = ?4, note = ?5,
                 updated_at = ?6 WHERE id = ?1",
            params![id, text, b.anchor.document_id, hash, b.note, b.updated_at],
        )?;
    }
    Ok((Class::Newer, id))
}

fn merge_finding(c: &Connection, f: &FindingRow, dry: bool) -> rusqlite::Result<Class> {
    let sources = serde_json::to_string(&f.sources).map_err(json_err)?;
    let kind = match f.kind {
        FindingKind::Answer => "answer",
        FindingKind::Passage => "passage",
    };
    let Some(local) = updated(c, "findings", &f.id)? else {
        if !dry {
            c.execute(
                "INSERT INTO findings (id, kind, question, text, sources, note, from_step_id,
                     created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    f.id,
                    kind,
                    f.question,
                    f.text,
                    sources,
                    f.note,
                    f.from_step_id,
                    f.created_at,
                    f.updated_at
                ],
            )?;
        }
        return Ok(Class::New);
    };
    if f.updated_at <= local {
        return Ok(Class::Here);
    }
    if !dry {
        c.execute(
            "UPDATE findings SET kind = ?2, question = ?3, text = ?4, sources = ?5, note = ?6,
                 from_step_id = ?7, updated_at = ?8 WHERE id = ?1",
            params![
                f.id,
                kind,
                f.question,
                f.text,
                sources,
                f.note,
                f.from_step_id,
                f.updated_at
            ],
        )?;
    }
    Ok(Class::Newer)
}

fn kind_text(k: CollectionKind) -> &'static str {
    match k {
        CollectionKind::Interest => "interest",
        CollectionKind::Project => "project",
    }
}

fn status_text(s: CollectionStatus) -> &'static str {
    match s {
        CollectionStatus::Active => "active",
        CollectionStatus::Finished => "finished",
        CollectionStatus::Archived => "archived",
    }
}

/// A collection and its links. A link names an item in the file (by the
/// id it has here) or already here; anything else refuses the file.
fn merge_collection(
    c: &Connection,
    col: &CollectionRow,
    bookmark_ids: &HashMap<String, String>,
    file_ids: &HashSet<(ItemKind, String)>,
    dry: bool,
) -> rusqlite::Result<Result<Class, &'static str>> {
    let local: Option<(String, String)> = c
        .query_row(
            "SELECT updated_at, kind FROM collections WHERE id = ?1",
            [&col.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let mut class = match &local {
        None => {
            if !dry {
                c.execute(
                    "INSERT INTO collections (id, kind, name, description, goal, status, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        col.id,
                        kind_text(col.kind),
                        col.name,
                        col.description,
                        col.goal,
                        status_text(col.status),
                        col.created_at,
                        col.updated_at
                    ],
                )?;
            }
            Class::New
        }
        Some((at, kind)) if col.updated_at > *at => {
            // A collection keeps its kind: a goal or Finished only for a
            // project here.
            let project = kind == "project";
            if !dry {
                c.execute(
                    "UPDATE collections SET name = ?2, description = ?3, goal = ?4, status = ?5,
                         updated_at = ?6 WHERE id = ?1",
                    params![
                        col.id,
                        col.name,
                        col.description,
                        if project { col.goal.as_deref() } else { None },
                        if !project && col.status == CollectionStatus::Finished {
                            "active"
                        } else {
                            status_text(col.status)
                        },
                        col.updated_at
                    ],
                )?;
            }
            Class::Newer
        }
        Some(_) => Class::Here,
    };
    for item in &col.items {
        let id = match item.kind {
            ItemKind::Bookmark => bookmark_ids
                .get(&item.id)
                .cloned()
                .unwrap_or_else(|| item.id.clone()),
            _ => item.id.clone(),
        };
        let table = match item.kind {
            ItemKind::Trail => Some("trails"),
            ItemKind::Bookmark => Some("bookmarks"),
            ItemKind::Finding => Some("findings"),
            ItemKind::Document => None,
        };
        if let Some(table) = table
            && !file_ids.contains(&(item.kind, item.id.clone()))
            && !exists(c, table, &id)?
        {
            return Ok(Err("collection_item"));
        }
        let linked = c
            .query_row(
                "SELECT 1 FROM collection_items
                 WHERE collection_id = ?1 AND item_kind = ?2 AND item_id = ?3",
                params![col.id, item.kind.as_str(), id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if linked {
            continue;
        }
        if class == Class::Here {
            class = Class::Newer;
        }
        if !dry {
            let hash = item.content_hash.as_deref().and_then(from_hex);
            c.execute(
                "INSERT OR IGNORE INTO collection_items (collection_id, item_kind, item_id, content_hash, added_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![col.id, item.kind.as_str(), id, hash, item.added_at],
            )?;
        }
    }
    Ok(Ok(class))
}

/// Compares `file` with the store (`dry`) or merges it into the store.
/// The same code both ways, so the dry run says what applying does.
pub fn merge(
    c: &Connection,
    file: &KnowledgeFile,
    dry: bool,
    now: OffsetDateTime,
) -> rusqlite::Result<Result<Merged, &'static str>> {
    let mut summary = ImportSummary::default();
    let mut expiring = 0;
    let in_collections: HashSet<&str> = file
        .collections
        .iter()
        .flat_map(|col| col.items.iter())
        .filter(|i| i.kind == ItemKind::Trail)
        .map(|i| i.id.as_str())
        .collect();
    let old = rfc3339(now - EXPIRE_AFTER);
    for t in &file.trails {
        let class = match merge_trail(c, t, dry)? {
            Ok(class) => class,
            Err(reason) => return Ok(Err(reason)),
        };
        let last = t
            .steps
            .iter()
            .map(|s| s.at.as_str())
            .max()
            .unwrap_or_default();
        if class == Class::New
            && !t.pinned
            && last < old.as_str()
            && !in_collections.contains(t.id.as_str())
        {
            expiring += 1;
        }
        tally(&mut summary.trails, class);
    }
    let mut bookmark_ids = HashMap::new();
    for b in &file.bookmarks {
        let (class, id) = merge_bookmark(c, b, dry)?;
        bookmark_ids.insert(b.id.clone(), id);
        tally(&mut summary.bookmarks, class);
    }
    for f in &file.findings {
        tally(&mut summary.findings, merge_finding(c, f, dry)?);
    }
    let file_ids: HashSet<(ItemKind, String)> = file
        .trails
        .iter()
        .map(|t| (ItemKind::Trail, t.id.clone()))
        .chain(
            file.bookmarks
                .iter()
                .map(|b| (ItemKind::Bookmark, b.id.clone())),
        )
        .chain(
            file.findings
                .iter()
                .map(|f| (ItemKind::Finding, f.id.clone())),
        )
        .collect();
    let collection_ids: HashSet<&str> = file.collections.iter().map(|c| c.id.as_str()).collect();
    for col in &file.collections {
        match merge_collection(c, col, &bookmark_ids, &file_ids, dry)? {
            Ok(class) => tally(&mut summary.collections, class),
            Err(reason) => return Ok(Err(reason)),
        }
    }
    for d in &file.dismissed_suggestions {
        if !d.collection_id.is_empty()
            && !collection_ids.contains(d.collection_id.as_str())
            && !exists(c, "collections", &d.collection_id)?
        {
            return Ok(Err("dismissal"));
        }
        if !dry {
            c.execute(
                "INSERT OR IGNORE INTO dismissed_suggestions (collection_id, document_id, content_hash, at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    d.collection_id,
                    d.document_id,
                    from_hex(&d.content_hash).unwrap_or_default(),
                    d.at
                ],
            )?;
        }
    }
    Ok(Ok(Merged { summary, expiring }))
}

/// The dry run: what importing `file` would do to the store now.
pub fn preview(inner: &Inner, file: &Arc<KnowledgeFile>) -> AtlasResult<Merged> {
    let now = inner.clock().now();
    let file = Arc::clone(file);
    let expiry = inner.trail_settings().expiring();
    let mut merged = inner
        .read(move |c| merge(c, &file, true, now))?
        .map_err(|_| AtlasError::Knowledge("read"))?
        .map_err(AtlasError::ImportInvalid)?;
    if !expiry {
        merged.expiring = 0;
    }
    Ok(merged)
}

/// Applies `file`, after a snapshot, in one transaction.
pub fn apply(inner: &Inner, file: &Arc<KnowledgeFile>) -> AtlasResult<ImportSummary> {
    if snapshot::take(inner) != Some(SnapshotOutcome::Taken) {
        return Err(AtlasError::ImportNoSnapshot);
    }
    let now = inner.clock().now();
    let file = Arc::clone(file);
    inner
        .call(
            move |c| -> rusqlite::Result<Result<ImportSummary, &'static str>> {
                let tx = c.transaction()?;
                match merge(&tx, &file, false, now)? {
                    Ok(merged) => {
                        tx.commit()?;
                        Ok(Ok(merged.summary))
                    }
                    Err(reason) => Ok(Err(reason)),
                }
            },
        )?
        .map_err(|_| AtlasError::Knowledge("save"))?
        .map_err(AtlasError::ImportInvalid)
}

// ------------------------------------------------------------- pending

struct Waiting {
    token: String,
    at: Instant,
    file: Arc<KnowledgeFile>,
}

/// The one import waiting for its Import button: the parsed file, kept
/// for [`PENDING_FOR`]. A new dry run replaces it; closing the library
/// drops it.
#[derive(Default)]
pub struct Pending {
    slot: Mutex<Option<Waiting>>,
}

impl Pending {
    pub fn put(&self, file: KnowledgeFile) -> (String, Arc<KnowledgeFile>) {
        let token = uuid::Uuid::new_v4().to_string();
        let file = Arc::new(file);
        *self.slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(Waiting {
            token: token.clone(),
            at: Instant::now(),
            file: Arc::clone(&file),
        });
        (token, file)
    }

    /// The file for `token`, taken: each dry run applies at most once.
    pub fn take(&self, token: &str) -> AtlasResult<Arc<KnowledgeFile>> {
        let mut slot = self.slot.lock().unwrap_or_else(|p| p.into_inner());
        match slot.take() {
            Some(w) if w.token == token && w.at.elapsed() < PENDING_FOR => Ok(w.file),
            Some(w) if w.token != token => {
                *slot = Some(w);
                Err(AtlasError::ImportExpired)
            }
            _ => Err(AtlasError::ImportExpired),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal() -> String {
        r#"{"format":"atlas-knowledge","version":1,"exported":"2026-11-11T00:00:00Z","scope":"everything"}"#
            .to_owned()
    }

    #[test]
    fn bad_files_are_told_apart() {
        assert!(parse(minimal().as_bytes()).is_ok());
        let truncated = &minimal()[..30];
        assert!(matches!(
            parse(truncated.as_bytes()),
            Err(AtlasError::ImportUnreadable)
        ));
        assert!(matches!(
            parse(
                minimal()
                    .replace("\"version\":1", "\"version\":99")
                    .as_bytes()
            ),
            Err(AtlasError::ImportUnknownVersion)
        ));
        assert!(matches!(
            parse(minimal().replace("atlas-knowledge", "other").as_bytes()),
            Err(AtlasError::ImportUnknownVersion)
        ));
        assert!(matches!(
            parse(minimal().replace("}", ",\"extra\":[]}").as_bytes()),
            Err(AtlasError::ImportInvalid("row"))
        ));
        assert!(matches!(
            parse(
                minimal()
                    .replace("2026-11-11T00:00:00Z", "yesterday")
                    .as_bytes()
            ),
            Err(AtlasError::ImportInvalid("time"))
        ));
        assert!(matches!(
            parse(b"[]"),
            Err(AtlasError::ImportInvalid(_)
                | AtlasError::ImportUnknownVersion
                | AtlasError::ImportUnreadable)
        ));
    }

    #[test]
    fn times_must_be_written_as_atlas_writes_them() {
        assert!(at("2026-11-11T09:30:00Z").is_ok());
        assert!(at("2026-11-11T09:30:00.5Z").is_err());
        assert!(at("2026-11-11T09:30:00+01:00").is_err());
    }

    #[test]
    fn a_dry_run_applies_once_and_only_by_its_token() {
        let pending = Pending::default();
        let file = parse(minimal().as_bytes()).unwrap();
        let (token, _) = pending.put(file);
        assert!(matches!(
            pending.take("other"),
            Err(AtlasError::ImportExpired)
        ));
        assert!(pending.take(&token).is_ok());
        assert!(matches!(
            pending.take(&token),
            Err(AtlasError::ImportExpired)
        ));
    }
}
