//! The JSON export format (Slice 3.8, phase3-spec §7.7, ADR 0019): one
//! file holding every row in the export's scope, anchors included, read
//! back by import. Typed throughout, so an import is checked as it is
//! parsed. It holds no paths: documents are named by id and content hash.
//!
//! What can be worked out again isn't kept: a trail's step count, last
//! step and filter text, and which documents a step touched, all follow
//! from its steps.

use serde::{Deserialize, Serialize};

use super::{ExportSet, Exporter};
use crate::domain::dto::{
    Anchor, CollectionKind, CollectionStatus, ExportFormat, ExportReport, FindingKind, StepPayload,
};
use crate::error::{AtlasError, AtlasResult};
use crate::services::export::dest::ExportDest;

pub const FORMAT: &str = "atlas-knowledge";
pub const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KnowledgeFile {
    pub format: String,
    pub version: u32,
    pub exported: String,
    /// `everything`, `trail`, `finding` or `collection`: what was asked
    /// for. Import treats every file alike.
    pub scope: String,
    #[serde(default)]
    pub trails: Vec<TrailRow>,
    #[serde(default)]
    pub bookmarks: Vec<BookmarkRow>,
    #[serde(default)]
    pub findings: Vec<FindingRow>,
    #[serde(default)]
    pub collections: Vec<CollectionRow>,
    #[serde(default)]
    pub dismissed_suggestions: Vec<DismissalRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrailRow {
    pub id: String,
    pub title: Option<String>,
    pub pinned: bool,
    pub started_at: String,
    pub updated_at: String,
    pub steps: Vec<StepRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StepRow {
    pub id: String,
    pub seq: u32,
    pub at: String,
    pub updated_at: String,
    pub note: Option<String>,
    pub payload: StepPayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BookmarkRow {
    pub id: String,
    pub anchor: Anchor,
    pub note: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// A finding's source, as `findings.sources` keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRow {
    pub n: u32,
    pub anchor: Anchor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FindingRow {
    pub id: String,
    pub kind: FindingKind,
    pub question: Option<String>,
    pub text: String,
    pub sources: Vec<SourceRow>,
    pub note: Option<String>,
    /// The step it was saved from; that step may be gone since.
    pub from_step_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    Trail,
    Bookmark,
    Finding,
    Document,
}

impl ItemKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trail => "trail",
            Self::Bookmark => "bookmark",
            Self::Finding => "finding",
            Self::Document => "document",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "trail" => Self::Trail,
            "bookmark" => Self::Bookmark,
            "finding" => Self::Finding,
            "document" => Self::Document,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ItemRow {
    pub kind: ItemKind,
    /// A trail, bookmark or finding id, or a document id.
    pub id: String,
    /// A document's content hash, hex; none for the other kinds.
    pub content_hash: Option<String>,
    pub added_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CollectionRow {
    pub id: String,
    pub kind: CollectionKind,
    pub name: String,
    pub description: Option<String>,
    pub goal: Option<String>,
    pub status: CollectionStatus,
    pub created_at: String,
    pub updated_at: String,
    pub items: Vec<ItemRow>,
}

/// A "Not for this": `collectionId` is empty for the recent trails' seed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DismissalRow {
    pub collection_id: String,
    pub document_id: String,
    pub content_hash: String,
    pub at: String,
}

/// Writes the set as `Atlas/atlas-knowledge-<date>.json`. An earlier file
/// of that name is never replaced: a second export the same day goes
/// beside it with a number.
pub struct JsonExporter;

impl Exporter for JsonExporter {
    fn id(&self) -> &'static str {
        "json"
    }

    fn export(&self, set: &ExportSet, dest: &mut ExportDest) -> AtlasResult<ExportReport> {
        let bytes = serde_json::to_vec(&set.file)
            .map_err(|_| AtlasError::Internal("the export could not be encoded"))?;
        let date = set.file.exported.get(..10).unwrap_or("export");
        let claim = dest.claim("", &format!("atlas-knowledge-{date}"), "json", None)?;
        dest.write(&claim, &bytes)?;
        let mut report = dest.report(ExportFormat::Json);
        report.besides = u32::from(claim.beside);
        report.written = 1;
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_fields_are_refused() {
        let text = r#"{"format":"atlas-knowledge","version":1,"exported":"t","scope":"everything","extra":1}"#;
        assert!(serde_json::from_str::<KnowledgeFile>(text).is_err());
        let ok = r#"{"format":"atlas-knowledge","version":1,"exported":"t","scope":"everything"}"#;
        let file: KnowledgeFile = serde_json::from_str(ok).unwrap();
        assert!(file.trails.is_empty() && file.dismissed_suggestions.is_empty());
    }

    #[test]
    fn a_finding_source_is_stored_as_the_store_keeps_it() {
        let source = SourceRow {
            n: 2,
            anchor: Anchor {
                document_id: "d".into(),
                content_hash: "ab".into(),
                title: "T".into(),
                file_type: crate::domain::dto::FileType::Pdf,
                page_no: Some(3),
                location: None,
                ordinal: Some(1),
                excerpt: None,
            },
        };
        let text = serde_json::to_string(&source).unwrap();
        assert!(
            text.starts_with(r#"{"n":2,"anchor":{"documentId":"d""#),
            "{text}"
        );
    }
}
