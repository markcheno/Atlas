//! Interests and projects (phase3-spec §5.4, §6.1; Slice 3.6): groups of
//! trails, bookmarks, findings and documents. Nothing requires one, and a
//! collection holds links, never the items: deleting it, or taking an item
//! out, never deletes the item.
//!
//! - An interest is a name and an optional description; a project adds an
//!   optional goal and can be finished. Either can be archived.
//! - A document is kept by its id and content hash, and resolved when
//!   shown, like an anchor.
//! - Every write is one transaction.

use rusqlite::{Connection, OptionalExtension, params};

use super::anchor;
use super::record::Recorder;
use super::saved;
use super::trails::{self, MAX_NOTE_CHARS};
use crate::domain::dto::{
    Anchor, Collection, CollectionCounts, CollectionDetail, CollectionDocument, CollectionInput,
    CollectionItemRef, CollectionKind, CollectionStatus, FileType, Membership, SavedItem,
    SavedKind,
};
use crate::error::{AtlasError, AtlasResult};

pub const MAX_NAME_CHARS: usize = 200;

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

fn read_err(e: rusqlite::Error) -> AtlasError {
    AtlasError::db("be read")(e)
}

fn checked(text: Option<&str>, what: &'static str, max: usize) -> AtlasResult<Option<String>> {
    let Some(text) = text.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(None);
    };
    if text.chars().count() > max {
        return Err(AtlasError::TextTooLong { what, max });
    }
    Ok(Some(text.to_owned()))
}

const SELECT: &str = "SELECT c.id, c.kind, c.name, c.description, c.goal, c.status,
        c.created_at, c.updated_at,
        (SELECT count(*) FROM collection_items i WHERE i.collection_id = c.id AND i.item_kind = 'trail'),
        (SELECT count(*) FROM collection_items i WHERE i.collection_id = c.id AND i.item_kind = 'bookmark'),
        (SELECT count(*) FROM collection_items i WHERE i.collection_id = c.id AND i.item_kind = 'finding'),
        (SELECT count(*) FROM collection_items i WHERE i.collection_id = c.id AND i.item_kind = 'document')
    FROM collections c";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Collection> {
    let kind: String = r.get(1)?;
    let status: String = r.get(5)?;
    Ok(Collection {
        id: r.get(0)?,
        kind: if kind == "project" {
            CollectionKind::Project
        } else {
            CollectionKind::Interest
        },
        name: r.get(2)?,
        description: r.get(3)?,
        goal: r.get(4)?,
        status: match status.as_str() {
            "finished" => CollectionStatus::Finished,
            "archived" => CollectionStatus::Archived,
            _ => CollectionStatus::Active,
        },
        created_at: r.get(6)?,
        updated_at: r.get(7)?,
        counts: CollectionCounts {
            trails: r.get(8)?,
            bookmarks: r.get(9)?,
            findings: r.get(10)?,
            documents: r.get(11)?,
        },
    })
}

fn one(conn: &Connection, id: &str) -> rusqlite::Result<Option<Collection>> {
    conn.query_row(&format!("{SELECT} WHERE c.id = ?1"), [id], row)
        .optional()
}

/// Collections (all, or of one status), most recently changed first.
pub fn list(rec: &Recorder, status: Option<CollectionStatus>) -> AtlasResult<Vec<Collection>> {
    let wanted = status.map(status_text);
    rec.store()
        .read(|c| -> rusqlite::Result<Vec<Collection>> {
            let mut stmt = c.prepare_cached(&format!(
                "{SELECT} WHERE (?1 IS NULL OR c.status = ?1) ORDER BY c.updated_at DESC, c.name"
            ))?;
            stmt.query_map([wanted], row)?.collect()
        })?
        .map_err(read_err)
}

/// Creates (no id) or changes a collection. A goal and Finished are a
/// project's only.
pub fn save(rec: &Recorder, input: &CollectionInput) -> AtlasResult<Collection> {
    let name = checked(Some(&input.name), "name", MAX_NAME_CHARS)?
        .ok_or(AtlasError::InvalidCollection("Give it a name."))?;
    let description = checked(input.description.as_deref(), "description", MAX_NOTE_CHARS)?;
    let goal = checked(input.goal.as_deref(), "goal", MAX_NOTE_CHARS)?;
    let existing = match &input.id {
        Some(id) => Some(
            rec.store()
                .read(|c| one(c, id))?
                .map_err(read_err)?
                .ok_or(AtlasError::CollectionNotFound)?,
        ),
        None => None,
    };
    // A collection keeps its kind.
    let kind = existing.as_ref().map_or(input.kind, |c| c.kind);
    let status = input
        .status
        .or(existing.as_ref().map(|c| c.status))
        .unwrap_or(CollectionStatus::Active);
    if kind == CollectionKind::Interest {
        if goal.is_some() {
            return Err(AtlasError::InvalidCollection("Only a project has a goal."));
        }
        if status == CollectionStatus::Finished {
            return Err(AtlasError::InvalidCollection(
                "Only a project can be finished.",
            ));
        }
    }
    let now = rec.store().clock().now_rfc3339();
    let id = input
        .id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let (row_id, is_new) = (id.clone(), existing.is_none());
    rec.store()
        .call(move |c| -> rusqlite::Result<()> {
            if is_new {
                c.execute(
                    "INSERT INTO collections (id, kind, name, description, goal, status, created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                    params![row_id, kind_text(kind), name, description, goal, status_text(status), now],
                )?;
            } else {
                c.execute(
                    "UPDATE collections SET name = ?2, description = ?3, goal = ?4, status = ?5,
                         updated_at = ?6
                     WHERE id = ?1",
                    params![row_id, name, description, goal, status_text(status), now],
                )?;
            }
            Ok(())
        })?
        .map_err(AtlasError::db("save it"))?;
    rec.store()
        .read(|c| one(c, &id))?
        .map_err(read_err)?
        .ok_or(AtlasError::CollectionNotFound)
}

/// Deletes a collection and its links and dismissals. Its trails,
/// bookmarks and findings stay.
pub fn delete(rec: &Recorder, id: &str) -> AtlasResult<()> {
    let row = id.to_owned();
    let n = rec
        .store()
        .call(move |c| -> rusqlite::Result<usize> {
            let tx = c.transaction()?;
            tx.execute(
                "DELETE FROM dismissed_suggestions WHERE collection_id = ?1",
                [&row],
            )?;
            let n = tx.execute("DELETE FROM collections WHERE id = ?1", [&row])?;
            tx.commit()?;
            Ok(n)
        })?
        .map_err(AtlasError::db("delete it"))?;
    if n == 0 {
        return Err(AtlasError::CollectionNotFound);
    }
    Ok(())
}

/// What a link names in `collection_items`: its kind, id and (for a
/// document) content hash.
fn link(
    rec: &Recorder,
    item: &CollectionItemRef,
) -> AtlasResult<(&'static str, String, Option<Vec<u8>>)> {
    match item {
        CollectionItemRef::Trail { id } => Ok(("trail", id.clone(), None)),
        CollectionItemRef::Bookmark { id } => Ok(("bookmark", id.clone(), None)),
        CollectionItemRef::Finding { id } => Ok(("finding", id.clone(), None)),
        CollectionItemRef::Document {
            file_id,
            document_id,
        } => {
            let found: Option<(String, Vec<u8>)> = rec
                .catalog()
                .read(|c| match (file_id, document_id) {
                    (Some(file), _) => c
                        .query_row(
                            "SELECT d.id, d.content_hash FROM file_instances f
                             JOIN documents d ON d.id = f.document_id WHERE f.id = ?1",
                            [file],
                            |r| Ok((r.get(0)?, r.get(1)?)),
                        )
                        .optional(),
                    (None, Some(doc)) => c
                        .query_row(
                            "SELECT id, content_hash FROM documents WHERE id = ?1",
                            [doc],
                            |r| Ok((r.get(0)?, r.get(1)?)),
                        )
                        .optional(),
                    (None, None) => Ok(None),
                })?
                .map_err(read_err)?;
            match (found, document_id) {
                (Some((doc, hash)), _) => Ok(("document", doc, Some(hash))),
                // Removing a document the catalog no longer has.
                (None, Some(doc)) => Ok(("document", doc.clone(), None)),
                (None, None) => Err(AtlasError::SaveSourceGone),
            }
        }
    }
}

/// Whether the item a link names is there to be added.
fn exists(conn: &Connection, kind: &str, id: &str) -> rusqlite::Result<bool> {
    let table = match kind {
        "trail" => "trails",
        "bookmark" => "bookmarks",
        "finding" => "findings",
        _ => return Ok(true),
    };
    conn.query_row(
        &format!("SELECT 1 FROM {table} WHERE id = ?1"),
        [id],
        |_| Ok(()),
    )
    .optional()
    .map(|r| r.is_some())
}

/// Adds an item to a collection; adding it again changes nothing.
pub fn add(
    rec: &Recorder,
    collection_id: &str,
    item: &CollectionItemRef,
) -> AtlasResult<Collection> {
    let (kind, item_id, hash) = link(rec, item)?;
    let now = rec.store().clock().now_rfc3339();
    let collection = collection_id.to_owned();
    let added = rec
        .store()
        .call(move |c| -> rusqlite::Result<Result<(), AtlasError>> {
            let tx = c.transaction()?;
            if one(&tx, &collection)?.is_none() {
                return Ok(Err(AtlasError::CollectionNotFound));
            }
            if !exists(&tx, kind, &item_id)? {
                return Ok(Err(match kind {
                    "trail" => AtlasError::TrailNotFound,
                    _ => AtlasError::SaveSourceGone,
                }));
            }
            let n = tx.execute(
                "INSERT OR IGNORE INTO collection_items (collection_id, item_kind, item_id, content_hash, added_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![collection, kind, item_id, hash, now],
            )?;
            if n > 0 {
                tx.execute(
                    "UPDATE collections SET updated_at = ?2 WHERE id = ?1",
                    params![collection, now],
                )?;
            }
            tx.commit()?;
            Ok(Ok(()))
        })?
        .map_err(AtlasError::db("add it"))?;
    added?;
    rec.store()
        .read(|c| one(c, collection_id))?
        .map_err(read_err)?
        .ok_or(AtlasError::CollectionNotFound)
}

/// Takes an item out of a collection; the item stays.
pub fn remove(
    rec: &Recorder,
    collection_id: &str,
    item: &CollectionItemRef,
) -> AtlasResult<Collection> {
    let (kind, item_id, _) = link(rec, item)?;
    let collection = collection_id.to_owned();
    rec.store()
        .call(move |c| {
            c.execute(
                "DELETE FROM collection_items WHERE collection_id = ?1 AND item_kind = ?2 AND item_id = ?3",
                params![collection, kind, item_id],
            )
        })?
        .map_err(AtlasError::db("take it out"))?;
    rec.store()
        .read(|c| one(c, collection_id))?
        .map_err(read_err)?
        .ok_or(AtlasError::CollectionNotFound)
}

/// The collections an item is in (Add to… shows them).
pub fn membership(rec: &Recorder, item: &CollectionItemRef) -> AtlasResult<Membership> {
    let (kind, item_id, _) = link(rec, item)?;
    let collection_ids = rec
        .store()
        .read(|c| -> rusqlite::Result<Vec<String>> {
            let mut stmt = c.prepare_cached(
                "SELECT collection_id FROM collection_items WHERE item_kind = ?1 AND item_id = ?2
                 ORDER BY collection_id",
            )?;
            stmt.query_map([kind, item_id.as_str()], |r| r.get(0))?
                .collect()
        })?
        .map_err(read_err)?;
    Ok(Membership { collection_ids })
}

/// A stored link: its kind, item id and (documents) content hash.
type Link = (String, String, Option<Vec<u8>>);

/// A collection's page: its trails, bookmarks, findings and documents,
/// each as it is now.
pub fn get(rec: &Recorder, id: &str) -> AtlasResult<CollectionDetail> {
    let (collection, items) = rec
        .store()
        .read(|c| -> rusqlite::Result<Option<(Collection, Vec<Link>)>> {
            let Some(collection) = one(c, id)? else {
                return Ok(None);
            };
            let mut stmt = c.prepare_cached(
                "SELECT item_kind, item_id, content_hash FROM collection_items
                 WHERE collection_id = ?1 ORDER BY added_at DESC, item_id",
            )?;
            let items = stmt
                .query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(Some((collection, items)))
        })?
        .map_err(read_err)?
        .ok_or(AtlasError::CollectionNotFound)?;
    let of = |kind: &str| -> Vec<String> {
        items
            .iter()
            .filter(|(k, _, _)| k == kind)
            .map(|(_, id, _)| id.clone())
            .collect()
    };
    let trails = rec
        .store()
        .read(|c| {
            of("trail")
                .iter()
                .filter_map(|t| trails::summary(c, t).transpose())
                .collect::<rusqlite::Result<Vec<_>>>()
        })?
        .map_err(read_err)?;
    let bookmarks = saved::by_ids(rec, SavedKind::Bookmark, &of("bookmark"))?
        .into_iter()
        .filter_map(|i| match i {
            SavedItem::Bookmark(b) => Some(b),
            SavedItem::Finding(_) => None,
        })
        .collect();
    let findings = saved::by_ids(rec, SavedKind::Finding, &of("finding"))?
        .into_iter()
        .filter_map(|i| match i {
            SavedItem::Finding(f) => Some(f),
            SavedItem::Bookmark(_) => None,
        })
        .collect();
    let documents = rec.catalog().read(|c| {
        items
            .iter()
            .filter(|(k, _, _)| k == "document")
            .map(|(_, doc, hash)| {
                let a = Anchor {
                    document_id: doc.clone(),
                    content_hash: hash.as_deref().map(anchor::to_hex).unwrap_or_default(),
                    title: String::new(),
                    file_type: FileType::Pdf,
                    page_no: None,
                    location: None,
                    ordinal: None,
                    excerpt: None,
                };
                let resolved = anchor::resolve(c, &a)?;
                let title = match &resolved.document_id {
                    Some(d) => c
                        .query_row(
                            "SELECT COALESCE(d.title,
                                    (SELECT display_name FROM file_instances WHERE document_id = d.id LIMIT 1))
                             FROM documents d WHERE d.id = ?1",
                            [d],
                            |r| r.get::<_, Option<String>>(0),
                        )
                        .optional()
                        .map_err(read_err)?
                        .flatten(),
                    None => None,
                };
                Ok(CollectionDocument {
                    document_id: doc.clone(),
                    title,
                    resolved,
                })
            })
            .collect::<AtlasResult<Vec<_>>>()
    })??;
    Ok(CollectionDetail {
        collection,
        trails,
        bookmarks,
        findings,
        documents,
    })
}
