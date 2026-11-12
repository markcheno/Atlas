//! The diagnostics report (Slice 4.1, phase4-spec §7.1): one JSON file the
//! user saves and may choose to share. Versions, readings, sizes and
//! counts, and the log's last events.
//!
//! It never holds a path, a file or folder name, a title, an author, a
//! search, a question, an answer, a note, an excerpt, a model's or
//! service's name, a URL or a key. Every field below is a number, a
//! version, or a value of a closed enum; models are counted by position.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use super::{LOGS_DIR, LogLine, log, tail};
use crate::domain::dto::{
    DeviceCheck, ExtractionCounts, KnowledgeState, KnowledgeStatus, Settings, StorageUsage,
};
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::fsutil::atomic_write;
use crate::services::catalog::{self, reader};
use crate::services::extraction::engine::garble::is_empty_page;
use crate::services::extraction::engine::{READER_VERSION_EPUB, READER_VERSION_PDF};
use crate::services::extraction::{self, PDFIUM_VERSION, PROTOCOL_VERSION};
use crate::services::knowledge::migrations::SUPPORTED_KNOWLEDGE_VERSION;
use crate::services::workspace::layout::CATALOG;

/// Where reports go in a library.
pub const REPORTS_DIR: &str = "exports/diagnostics";
pub const REPORT_VERSION: u32 = 1;
/// How many of the log's events a report carries.
pub const EVENTS: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Report {
    pub report_version: u32,
    pub saved_at: String,
    pub versions: Versions,
    pub device: Option<DeviceCheck>,
    pub settings: SettingsSummary,
    pub storage: Option<StorageUsage>,
    pub library: LibraryCounts,
    pub knowledge: KnowledgeSummary,
    pub log: LogSummary,
    pub events: Vec<LogLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Versions {
    pub atlas: String,
    pub os: Option<String>,
    pub os_version: Option<String>,
    pub arch: String,
    pub catalog_schema: u32,
    pub knowledge_schema: u32,
    pub pdf_reader: u32,
    pub epub_reader: u32,
    pub chunker: i64,
    pub worker_protocol: u32,
    pub pdfium: String,
}

/// The settings that are modes and switches. Acknowledged services are
/// counted, never named.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsSummary {
    pub theme: crate::domain::dto::ThemePreference,
    pub allow_remote_embedding: bool,
    pub disable_external_ai: bool,
    pub acknowledged_services: u32,
    pub record_trails: bool,
    pub expire_trails: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LibraryCounts {
    pub folders: u32,
    /// Files by `available`/`missing`, and by `local`/`cloud_only`.
    pub files_by_availability: BTreeMap<String, u64>,
    pub files_by_storage: BTreeMap<String, u64>,
    /// Files and their bytes by format (`pdf`, `epub`).
    pub files_by_format: BTreeMap<String, FormatCount>,
    /// Text status of files, as the status line counts them.
    pub text: ExtractionCounts,
    /// Documents by state (`done`, `needs_ocr`, …).
    pub documents_by_state: BTreeMap<String, u64>,
    pub documents_to_reread: u64,
    pub ocr: OcrCounts,
    pub passages: u64,
    pub chunks: u64,
    /// Vectors of each embedding model, by its position and status, never
    /// its name.
    pub models: Vec<ModelCount>,
    /// Jobs by kind and state, as `extract.completed`.
    pub jobs: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FormatCount {
    pub files: u64,
    pub bytes: u64,
}

/// What OCR would have to read (phase4-spec §7.2).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OcrCounts {
    pub pdfs_needing_ocr: u64,
    /// Their pages, where known (counted from Slice 4.1).
    pub pages: u64,
    pub pages_with_text: u64,
    /// Needs OCR PDFs whose pages aren't counted yet.
    pub uncounted_pdfs: u64,
    pub epubs_needing_ocr: u64,
    /// Read PDFs with pages under the reader's empty-page threshold
    /// (scanned pages, but also blank ones), and those pages.
    pub pdfs_with_empty_pages: u64,
    pub empty_pages: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelCount {
    pub position: u32,
    pub status: String,
    pub dimensions: u32,
    pub vectors: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KnowledgeSummary {
    pub state: KnowledgeState,
    /// The counts, when the store can be read.
    pub status: Option<KnowledgeStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LogSummary {
    pub files: u32,
    pub bytes: u64,
    pub dropped: u64,
}

/// What the report is made from, gathered by the caller.
pub struct Inputs {
    pub root: PathBuf,
    pub device: Option<DeviceCheck>,
    pub settings: Settings,
    pub storage: Option<StorageUsage>,
    pub knowledge: KnowledgeSummary,
    pub dropped: u64,
    pub saved_at: String,
}

/// A saved report: its file and size.
pub struct Saved {
    pub path: PathBuf,
    pub bytes: u64,
}

/// Builds the report for the library at `inputs.root`.
pub fn build(inputs: &Inputs) -> AtlasResult<Report> {
    let conn = reader::open_single(&inputs.root.join(CATALOG))?;
    let library = library(&conn).map_err(AtlasError::db("count the library"))?;
    let catalog_schema = catalog::migrations::current_version(&conn)?;
    drop(conn);
    let logs = inputs.root.join(LOGS_DIR);
    let files = log::files(&logs);
    let s = &inputs.settings;
    Ok(Report {
        report_version: REPORT_VERSION,
        saved_at: inputs.saved_at.clone(),
        versions: Versions {
            atlas: env!("CARGO_PKG_VERSION").to_owned(),
            os: sysinfo::System::name().ok(),
            os_version: sysinfo::System::os_version().ok(),
            arch: std::env::consts::ARCH.to_owned(),
            catalog_schema,
            knowledge_schema: SUPPORTED_KNOWLEDGE_VERSION,
            pdf_reader: READER_VERSION_PDF,
            epub_reader: READER_VERSION_EPUB,
            chunker: crate::services::embedding::chunk::CHUNKER_VERSION,
            worker_protocol: PROTOCOL_VERSION,
            pdfium: PDFIUM_VERSION.to_owned(),
        },
        device: inputs.device.clone(),
        settings: SettingsSummary {
            theme: s.appearance.theme,
            allow_remote_embedding: s.privacy.allow_remote_embedding,
            disable_external_ai: s.privacy.disable_external_ai,
            acknowledged_services: s.privacy.acknowledged.len() as u32,
            record_trails: s.privacy.record_trails,
            expire_trails: s.privacy.expire_trails,
        },
        storage: inputs.storage.clone(),
        library,
        knowledge: inputs.knowledge.clone(),
        log: LogSummary {
            files: files.len() as u32,
            bytes: files.iter().map(|f| f.2).sum(),
            dropped: inputs.dropped,
        },
        events: tail(&logs, EVENTS),
    })
}

/// Builds the report and writes it to `exports/diagnostics/`.
pub fn save(inputs: &Inputs) -> AtlasResult<Saved> {
    let report = build(inputs)?;
    let dir = inputs.root.join(REPORTS_DIR);
    fs::create_dir_all(&dir).map_err(|_| AtlasError::DiagnosticsUnavailable)?;
    let stamp: String = inputs
        .saved_at
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    let mut path = dir.join(format!("atlas-diagnostics-{stamp}.json"));
    let mut n = 1;
    while path.exists() {
        path = dir.join(format!("atlas-diagnostics-{stamp}-{n}.json"));
        n += 1;
    }
    let bytes =
        serde_json::to_vec_pretty(&report).map_err(|_| AtlasError::DiagnosticsUnavailable)?;
    atomic_write(&path, &dir, &bytes).map_err(|_| AtlasError::DiagnosticsUnavailable)?;
    Ok(Saved {
        path,
        bytes: bytes.len() as u64,
    })
}

/// The saved reports in a library, oldest first.
pub fn saved(root: &Path) -> Vec<(PathBuf, u64)> {
    let mut out: Vec<_> = fs::read_dir(root.join(REPORTS_DIR))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.starts_with("atlas-diagnostics-") && n.ends_with(".json"))
        })
        .filter_map(|e| {
            Some((
                e.path(),
                e.metadata().ok().filter(fs::Metadata::is_file)?.len(),
            ))
        })
        .collect();
    out.sort();
    out
}

fn grouped(conn: &Connection, sql: &str) -> rusqlite::Result<BTreeMap<String, u64>> {
    conn.prepare(sql)?
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?.max(0) as u64))
        })?
        .collect()
}

fn one(conn: &Connection, sql: &str) -> rusqlite::Result<u64> {
    conn.query_row(sql, [], |r| r.get::<_, i64>(0))
        .map(|n| n.max(0) as u64)
}

fn library(conn: &Connection) -> rusqlite::Result<LibraryCounts> {
    let files_by_format = conn
        .prepare(
            "SELECT file_type, COUNT(*), COALESCE(SUM(size_bytes), 0) FROM file_instances
              WHERE availability = 'available' GROUP BY file_type",
        )?
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                FormatCount {
                    files: r.get::<_, i64>(1)?.max(0) as u64,
                    bytes: r.get::<_, i64>(2)?.max(0) as u64,
                },
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let models = conn
        .prepare(
            "SELECT m.status, m.dims, (SELECT COUNT(*) FROM embeddings e WHERE e.model_id = m.id)
               FROM embedding_models m ORDER BY m.created_at, m.rowid",
        )?
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .enumerate()
        .map(|(i, (status, dims, vectors))| ModelCount {
            position: i as u32 + 1,
            // A closed set in the schema; anything else is reported as such.
            status: match status.as_str() {
                "building" | "ready" | "retired" | "failed" => status,
                _ => "other".to_owned(),
            },
            dimensions: dims.max(0) as u32,
            vectors: vectors.max(0) as u64,
        })
        .collect();
    let pdf_needs = "d.state = 'needs_ocr' AND EXISTS (SELECT 1 FROM file_instances f
                         WHERE f.document_id = d.id AND f.file_type = 'pdf')";
    let mut ocr = OcrCounts {
        pdfs_needing_ocr: one(conn, &format!("SELECT COUNT(*) FROM documents d WHERE {pdf_needs}"))?,
        pages: one(conn, &format!(
            "SELECT COALESCE(SUM(page_count), 0) FROM documents d WHERE {pdf_needs}"
        ))?,
        pages_with_text: one(conn, &format!(
            "SELECT COALESCE(SUM(text_pages), 0) FROM documents d WHERE {pdf_needs}"
        ))?,
        uncounted_pdfs: one(conn, &format!(
            "SELECT COUNT(*) FROM documents d WHERE {pdf_needs} AND d.page_count IS NULL"
        ))?,
        epubs_needing_ocr: one(
            conn,
            "SELECT COUNT(*) FROM documents d WHERE d.state = 'needs_ocr' AND NOT EXISTS
                 (SELECT 1 FROM file_instances f WHERE f.document_id = d.id AND f.file_type = 'pdf')",
        )?,
        ..OcrCounts::default()
    };
    // Pages of read PDFs under the reader's empty-page rule. Only short
    // pages can be empty, so only they are read.
    let mut stmt = conn.prepare(
        "SELECT p.document_id, p.text FROM passages p
          WHERE p.page_no IS NOT NULL AND length(p.text) < 2000 ORDER BY p.document_id",
    )?;
    let mut rows = stmt.query([])?;
    let mut last: Option<String> = None;
    let mut counted = false;
    while let Some(r) = rows.next()? {
        let doc: String = r.get(0)?;
        let text: String = r.get(1)?;
        if last.as_deref() != Some(doc.as_str()) {
            last = Some(doc);
            counted = false;
        }
        if is_empty_page(&text) {
            ocr.empty_pages += 1;
            if !counted {
                ocr.pdfs_with_empty_pages += 1;
                counted = true;
            }
        }
    }
    drop(rows);
    drop(stmt);
    Ok(LibraryCounts {
        folders: one(conn, "SELECT COUNT(*) FROM source_roots")? as u32,
        files_by_availability: grouped(
            conn,
            "SELECT availability, COUNT(*) FROM file_instances GROUP BY availability",
        )?,
        files_by_storage: grouped(
            conn,
            "SELECT storage, COUNT(*) FROM file_instances
              WHERE availability = 'available' GROUP BY storage",
        )?,
        files_by_format,
        text: extraction::db::overview(conn, None)?.counts,
        documents_by_state: grouped(conn, "SELECT state, COUNT(*) FROM documents GROUP BY state")?,
        documents_to_reread: one(conn, "SELECT COUNT(*) FROM documents WHERE refresh = 1")?,
        ocr,
        passages: one(conn, "SELECT COUNT(*) FROM passages")?,
        chunks: one(conn, "SELECT COUNT(*) FROM chunks")?,
        models,
        jobs: grouped(
            conn,
            "SELECT kind || '.' || state, COUNT(*) FROM jobs GROUP BY kind, state",
        )?,
    })
}
