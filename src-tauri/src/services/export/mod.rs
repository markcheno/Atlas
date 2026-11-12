//! Export and import of what the user created (Slice 3.8, phase3-spec
//! §5.8, §7.7; ADR 0019).
//!
//! - [`gather`] reads the rows in an export's scope from the knowledge
//!   store, in one read, into the JSON format's types ([`json`]), and
//!   finds each source's file as text in the catalog.
//! - An [`Exporter`] writes them through an [`ExportDest`], the checked
//!   `Atlas` folder of a folder the user chose: [`markdown`] or [`json`].
//! - [`import`] reads a JSON export back, after a dry run.

pub mod dest;
pub mod import;
pub mod json;
pub mod markdown;

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use self::dest::ExportDest;
use self::json::{
    BookmarkRow, CollectionRow, DismissalRow, FindingRow, ItemKind, ItemRow, KnowledgeFile,
    SourceRow, StepRow, TrailRow,
};
use crate::domain::dto::{
    Anchor, CollectionKind, CollectionStatus, ExportFormat, ExportReport, ExportScope, FindingKind,
    ImportPreview, ImportReport, KnowledgeState, LinkStyle, StepPayload,
};
use crate::domain::traits::CatalogRepository;
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::native_path;
use crate::services::catalog::reader::CatalogReader;
use crate::services::catalog::repository::SqliteCatalog;
use crate::services::knowledge::anchor::{self, to_hex};
use crate::services::knowledge::record::Recorder;
use crate::services::knowledge::store::Inner;
use crate::services::knowledge::trails;
use crate::services::workspace::{OpenRoot, settings};

/// A document as the catalog has it now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Located {
    /// Its file, as text; `None` when no file in the library has it.
    pub path: Option<String>,
    /// Its title now, when the catalog knows the document.
    pub title: Option<String>,
}

/// What an export writes, gathered before anything is written.
pub struct ExportSet {
    pub file: KnowledgeFile,
    /// Each document as the catalog has it now, by document id.
    pub places: HashMap<String, Located>,
    pub link_style: LinkStyle,
}

/// A way of writing an [`ExportSet`] (product spec §10). Two are built
/// in; nothing loads exporters at run time.
pub trait Exporter {
    fn id(&self) -> &'static str;
    fn export(&self, set: &ExportSet, dest: &mut ExportDest) -> AtlasResult<ExportReport>;
}

pub fn exporter(format: ExportFormat) -> Box<dyn Exporter> {
    match format {
        ExportFormat::Markdown => Box::new(markdown::MarkdownExporter),
        ExportFormat::Json => Box::new(json::JsonExporter),
    }
}

fn hash_hex(bytes: Option<Vec<u8>>) -> Option<String> {
    bytes.map(|b| to_hex(&b))
}

fn ids(k: &Connection, sql: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = k.prepare(sql)?;
    stmt.query_map([], |r| r.get(0))?.collect()
}

pub(crate) fn trail_row(k: &Connection, id: &str) -> rusqlite::Result<Option<TrailRow>> {
    let head = k
        .query_row(
            "SELECT id, title, pinned, started_at, updated_at FROM trails WHERE id = ?1",
            [id],
            |r| {
                Ok(TrailRow {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    pinned: r.get::<_, i64>(2)? != 0,
                    started_at: r.get(3)?,
                    updated_at: r.get(4)?,
                    steps: Vec::new(),
                })
            },
        )
        .optional()?;
    let Some(mut trail) = head else {
        return Ok(None);
    };
    let mut stmt = k.prepare_cached(
        "SELECT id, seq, at, updated_at, note, payload FROM trail_steps
         WHERE trail_id = ?1 ORDER BY seq",
    )?;
    let rows = stmt.query_map([id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, u32>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, String>(5)?,
        ))
    })?;
    for row in rows {
        let (id, seq, at, updated_at, note, text) = row?;
        // A step this build can't read is left out, as the timeline does.
        let Ok(payload) = serde_json::from_str::<StepPayload>(&text) else {
            continue;
        };
        trail.steps.push(StepRow {
            id,
            seq,
            at,
            updated_at,
            note,
            payload,
        });
    }
    Ok(Some(trail))
}

pub(crate) fn bookmark_row(k: &Connection, id: &str) -> rusqlite::Result<Option<BookmarkRow>> {
    let row = k
        .query_row(
            "SELECT id, anchor, note, created_at, updated_at FROM bookmarks WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            },
        )
        .optional()?;
    Ok(row.and_then(|(id, anchor, note, created_at, updated_at)| {
        Some(BookmarkRow {
            id,
            anchor: serde_json::from_str::<Anchor>(&anchor).ok()?,
            note,
            created_at,
            updated_at,
        })
    }))
}

pub(crate) fn finding_row(k: &Connection, id: &str) -> rusqlite::Result<Option<FindingRow>> {
    let row = k
        .query_row(
            "SELECT id, kind, question, text, sources, note, from_step_id, created_at, updated_at
             FROM findings WHERE id = ?1",
            [id],
            |r| {
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
            },
        )
        .optional()?;
    Ok(row.and_then(
        |(id, kind, question, text, sources, note, from_step_id, created_at, updated_at)| {
            Some(FindingRow {
                id,
                kind: match kind.as_str() {
                    "answer" => FindingKind::Answer,
                    "passage" => FindingKind::Passage,
                    _ => return None,
                },
                question,
                text,
                sources: serde_json::from_str::<Vec<SourceRow>>(&sources).ok()?,
                note,
                from_step_id,
                created_at,
                updated_at,
            })
        },
    ))
}

pub(crate) fn collection_row(k: &Connection, id: &str) -> rusqlite::Result<Option<CollectionRow>> {
    let head = k
        .query_row(
            "SELECT id, kind, name, description, goal, status, created_at, updated_at
             FROM collections WHERE id = ?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                ))
            },
        )
        .optional()?;
    let Some((id, kind, name, description, goal, status, created_at, updated_at)) = head else {
        return Ok(None);
    };
    let kind = match kind.as_str() {
        "interest" => CollectionKind::Interest,
        _ => CollectionKind::Project,
    };
    let status = match status.as_str() {
        "finished" => CollectionStatus::Finished,
        "archived" => CollectionStatus::Archived,
        _ => CollectionStatus::Active,
    };
    let mut stmt = k.prepare_cached(
        "SELECT item_kind, item_id, content_hash, added_at FROM collection_items
         WHERE collection_id = ?1 ORDER BY added_at, item_kind, item_id",
    )?;
    let items = stmt
        .query_map([&id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<Vec<u8>>>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .filter_map(|(kind, item, hash, added_at)| {
            Some(ItemRow {
                kind: ItemKind::parse(&kind)?,
                id: item,
                content_hash: hash_hex(hash),
                added_at,
            })
        })
        .collect();
    Ok(Some(CollectionRow {
        id,
        kind,
        name,
        description,
        goal,
        status,
        created_at,
        updated_at,
        items,
    }))
}

fn dismissals(k: &Connection, collection: Option<&str>) -> rusqlite::Result<Vec<DismissalRow>> {
    let mut stmt = k.prepare(
        "SELECT collection_id, document_id, content_hash, at FROM dismissed_suggestions
         WHERE ?1 IS NULL OR collection_id = ?1 ORDER BY collection_id, document_id",
    )?;
    stmt.query_map(params![collection], |r| {
        Ok(DismissalRow {
            collection_id: r.get(0)?,
            document_id: r.get(1)?,
            content_hash: to_hex(&r.get::<_, Vec<u8>>(2)?),
            at: r.get(3)?,
        })
    })?
    .collect()
}

fn scope_name(scope: &ExportScope) -> &'static str {
    match scope {
        ExportScope::Trail { .. } => "trail",
        ExportScope::Finding { .. } => "finding",
        ExportScope::Collection { .. } => "collection",
        ExportScope::Everything => "everything",
    }
}

/// Reads every row in `scope` from the knowledge store. A collection
/// brings its trails, bookmarks and findings, and its "Not for this".
pub fn gather(k: &Connection, scope: &ExportScope, now: &str) -> AtlasResult<KnowledgeFile> {
    gather_rows(k, scope, now).map_err(|e| match e {
        Gathered::Missing(err) => err,
        Gathered::Db => AtlasError::Knowledge("read"),
    })
}

enum Gathered {
    Missing(AtlasError),
    Db,
}

impl From<rusqlite::Error> for Gathered {
    fn from(_: rusqlite::Error) -> Self {
        Self::Db
    }
}

fn gather_rows(k: &Connection, scope: &ExportScope, now: &str) -> Result<KnowledgeFile, Gathered> {
    let mut file = KnowledgeFile {
        format: json::FORMAT.to_owned(),
        version: json::VERSION,
        exported: now.to_owned(),
        scope: scope_name(scope).to_owned(),
        trails: Vec::new(),
        bookmarks: Vec::new(),
        findings: Vec::new(),
        collections: Vec::new(),
        dismissed_suggestions: Vec::new(),
    };
    let (trail_ids, bookmark_ids, finding_ids): (Vec<String>, Vec<String>, Vec<String>) =
        match scope {
            ExportScope::Trail { id } => {
                let row = trail_row(k, id)?.ok_or(Gathered::Missing(AtlasError::TrailNotFound))?;
                file.trails.push(row);
                (Vec::new(), Vec::new(), Vec::new())
            }
            ExportScope::Finding { id } => {
                let row =
                    finding_row(k, id)?.ok_or(Gathered::Missing(AtlasError::SaveSourceGone))?;
                file.findings.push(row);
                (Vec::new(), Vec::new(), Vec::new())
            }
            ExportScope::Collection { id } => {
                let row = collection_row(k, id)?
                    .ok_or(Gathered::Missing(AtlasError::CollectionNotFound))?;
                let pick = |kind: ItemKind| -> Vec<String> {
                    let set: BTreeSet<String> = row
                        .items
                        .iter()
                        .filter(|i| i.kind == kind)
                        .map(|i| i.id.clone())
                        .collect();
                    set.into_iter().collect()
                };
                let picked = (
                    pick(ItemKind::Trail),
                    pick(ItemKind::Bookmark),
                    pick(ItemKind::Finding),
                );
                file.dismissed_suggestions = dismissals(k, Some(id))?;
                file.collections.push(row);
                picked
            }
            ExportScope::Everything => {
                for id in ids(k, "SELECT id FROM collections ORDER BY created_at, id")? {
                    file.collections.extend(collection_row(k, &id)?);
                }
                file.dismissed_suggestions = dismissals(k, None)?;
                (
                    ids(k, "SELECT id FROM trails ORDER BY started_at, id")?,
                    ids(k, "SELECT id FROM bookmarks ORDER BY created_at, id")?,
                    ids(k, "SELECT id FROM findings ORDER BY created_at, id")?,
                )
            }
        };
    for id in &trail_ids {
        file.trails.extend(trail_row(k, id)?);
    }
    for id in &bookmark_ids {
        file.bookmarks.extend(bookmark_row(k, id)?);
    }
    for id in &finding_ids {
        file.findings.extend(finding_row(k, id)?);
    }
    Ok(file)
}

/// Every anchor in the set, in no particular order.
pub fn anchors(file: &KnowledgeFile) -> Vec<&Anchor> {
    let mut out: Vec<&Anchor> = Vec::new();
    for t in &file.trails {
        for s in &t.steps {
            out.extend(trails::anchors(&s.payload));
        }
    }
    out.extend(file.bookmarks.iter().map(|b| &b.anchor));
    for f in &file.findings {
        out.extend(f.sources.iter().map(|s| &s.anchor));
    }
    out
}

/// Each document's file now, as text (phase3-spec §7.7: "the file's path
/// as text"), and its title. Only the catalog is read.
pub fn places(catalog: &Connection, file: &KnowledgeFile) -> AtlasResult<HashMap<String, Located>> {
    let mut wanted: Vec<Anchor> = anchors(file).into_iter().cloned().collect();
    for c in &file.collections {
        for item in c.items.iter().filter(|i| i.kind == ItemKind::Document) {
            wanted.push(Anchor {
                document_id: item.id.clone(),
                content_hash: item.content_hash.clone().unwrap_or_default(),
                title: String::new(),
                file_type: crate::domain::dto::FileType::Pdf,
                page_no: None,
                location: None,
                ordinal: None,
                excerpt: None,
            });
        }
    }
    let mut out: HashMap<String, Located> = HashMap::new();
    for a in wanted {
        if out.contains_key(&a.document_id) {
            continue;
        }
        let whole = Anchor {
            excerpt: None,
            ..a.clone()
        };
        let resolved = anchor::resolve(catalog, &whole)?;
        let db = |e: rusqlite::Error| {
            let _ = e;
            AtlasError::Knowledge("read")
        };
        let path = match &resolved.file_instance_id {
            Some(file_id) => file_path(catalog, file_id).map_err(db)?,
            None => None,
        };
        let title = match &resolved.document_id {
            Some(d) => catalog
                .query_row(
                    "SELECT COALESCE(d.title,
                            (SELECT display_name FROM file_instances WHERE document_id = d.id LIMIT 1))
                     FROM documents d WHERE d.id = ?1",
                    [d],
                    |r| r.get::<_, Option<String>>(0),
                )
                .optional()
                .map_err(db)?
                .flatten(),
            None => None,
        };
        out.insert(a.document_id.clone(), Located { path, title });
    }
    Ok(out)
}

fn file_path(catalog: &Connection, file_id: &str) -> rusqlite::Result<Option<String>> {
    let row: Option<(Vec<u8>, Vec<u8>)> = catalog
        .query_row(
            "SELECT r.path_bytes, f.rel_path FROM file_instances f
             JOIN source_roots r ON r.id = f.source_root_id WHERE f.id = ?1",
            [file_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    Ok(row.and_then(|(root, rel)| {
        let path: PathBuf = native_path::decode(&root)?.join(native_path::decode(&rel)?);
        Some(native_path::display(&path))
    }))
}

/// Exports `set` into `chosen/Atlas`, after checking where that is.
pub fn write(
    set: &ExportSet,
    format: ExportFormat,
    chosen: &Path,
    data_root: &Path,
    sources: &[PathBuf],
) -> AtlasResult<ExportReport> {
    let mut dest = ExportDest::prepare(chosen, data_root, sources)?;
    let report = exporter(format).export(set, &mut dest)?;
    dest.finish()?;
    Ok(report)
}

// ------------------------------------------------------------- the app

/// What an export or import needs, copied out under the workspace lock so
/// the work runs without it.
pub struct Handles {
    pub knowledge: std::sync::Arc<Inner>,
    pub catalog: std::sync::Arc<CatalogReader>,
    pub data_root: PathBuf,
    pub sources: Vec<PathBuf>,
    pub imports: std::sync::Arc<import::Pending>,
    pub recorder: std::sync::Arc<Recorder>,
}

impl Handles {
    pub fn of(root: &OpenRoot) -> AtlasResult<Self> {
        if root.knowledge().state() != KnowledgeState::Ready {
            return Err(AtlasError::Knowledge("reach"));
        }
        let sources = SqliteCatalog::new(root.catalog())
            .list_source_root_records()?
            .iter()
            .filter_map(|r| native_path::decode(&r.path_bytes))
            .collect();
        Ok(Self {
            knowledge: root.knowledge().shared(),
            catalog: root.reader_handle(),
            data_root: root.path().to_path_buf(),
            sources,
            imports: std::sync::Arc::clone(root.imports()),
            recorder: std::sync::Arc::clone(root.recorder()),
        })
    }

    /// Where the export dialog starts: the last export's folder, or the
    /// data root's own `exports`.
    pub fn start_folder(&self) -> PathBuf {
        settings::last_export_folder(&self.data_root)
            .unwrap_or_else(|| self.data_root.join(dest::EXPORTS))
    }
}

/// Exports `scope` into `chosen/Atlas` (`chosen` from the native dialog),
/// and remembers `chosen` for the next dialog.
pub fn export(
    h: &Handles,
    scope: &ExportScope,
    format: ExportFormat,
    link_style: LinkStyle,
    chosen: &Path,
) -> AtlasResult<ExportReport> {
    let now = h.knowledge.clock().now_rfc3339();
    let scope = scope.clone();
    let file = h.knowledge.read(move |k| gather(k, &scope, &now))??;
    let places = match format {
        ExportFormat::Markdown => h.catalog.read(|c| places(c, &file))??,
        ExportFormat::Json => HashMap::new(),
    };
    let set = ExportSet {
        file,
        places,
        link_style,
    };
    let report = write(&set, format, chosen, &h.data_root, &h.sources)?;
    // Only where the next dialog starts: not worth failing the export for.
    let _ = settings::set_last_export_folder(&h.data_root, chosen);
    Ok(report)
}

/// Reads `path` (from the native dialog) and compares it with the store;
/// nothing is written. The parsed file waits for [`apply`].
pub fn preview(h: &Handles, path: &Path) -> AtlasResult<ImportPreview> {
    let file = import::read(path)?;
    let exported = file.exported.clone();
    let (token, file) = h.imports.put(file);
    let merged = import::preview(&h.knowledge, &file)?;
    Ok(ImportPreview {
        token,
        file_name: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        exported,
        summary: merged.summary,
        expiring: merged.expiring,
    })
}

/// Applies the dry run named by `token`.
pub fn apply(h: &Handles, token: &str) -> AtlasResult<ImportReport> {
    let file = h.imports.take(token)?;
    let summary = import::apply(&h.knowledge, &file)?;
    h.recorder.emit();
    Ok(ImportReport { summary })
}
