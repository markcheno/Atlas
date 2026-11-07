//! Source roots: the folders the user authorized Atlas to read.
//!
//! Read-only by construction. This module only calls `metadata`,
//! `read_dir` (dropped after the first call), `canonicalize`, and opens
//! directories read-only to compare their identity. It never writes to,
//! renames, or changes timestamps in a source folder (phase0-spec §6).
//!
//! Rules (phase0-spec §5.3, ADR 0003):
//! - A source must be an existing, readable folder; its canonical path is
//!   stored as native bytes.
//! - The Atlas data root, or any folder inside it, is refused. A source
//!   that contains the data root is allowed and flagged.
//! - A folder already present, inside a present root, or containing one is
//!   refused. Identity uses filesystem handles (`same-file`), so symlinks,
//!   case-insensitive spellings, and Unicode-normalization variants of the
//!   same folder are caught.
//! - Overlaps that already exist in a catalog are resolved
//!   deterministically when listing: the outermost root owns the subtree.

pub mod probe;

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use same_file::Handle;

use crate::domain::dto::{AddSourceRootsResult, SourceRoot, SourceRootRejection};
use crate::domain::traits::{CatalogRepository, SourceRootRecord};
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::clock::now_rfc3339;
use crate::infrastructure::native_path;
use crate::services::catalog::repository::SqliteCatalog;
use crate::services::workspace::OpenRoot;

/// How a candidate folder relates to an existing one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    Same,
    /// The candidate is inside the existing folder.
    Inside,
    /// The candidate contains the existing folder.
    Contains,
    Unrelated,
}

/// Compares by filesystem identity when both folders are reachable, and
/// by path components otherwise (for example when a drive is unplugged).
pub fn relation(candidate: &Path, existing: &Path) -> Relation {
    let (Ok(candidate_handle), Ok(existing_handle)) =
        (Handle::from_path(candidate), Handle::from_path(existing))
    else {
        return relation_by_components(candidate, existing);
    };
    if candidate_handle == existing_handle {
        return Relation::Same;
    }
    let has_ancestor = |path: &Path, target: &Handle| {
        path.ancestors()
            .skip(1)
            .any(|ancestor| Handle::from_path(ancestor).is_ok_and(|h| &h == target))
    };
    if has_ancestor(candidate, &existing_handle) {
        Relation::Inside
    } else if has_ancestor(existing, &candidate_handle) {
        Relation::Contains
    } else {
        Relation::Unrelated
    }
}

/// Purely lexical comparison of canonical paths. Deterministic and
/// independent of what is currently mounted.
pub fn relation_by_components(candidate: &Path, existing: &Path) -> Relation {
    if candidate == existing {
        Relation::Same
    } else if candidate.starts_with(existing) {
        Relation::Inside
    } else if existing.starts_with(candidate) {
        Relation::Contains
    } else {
        Relation::Unrelated
    }
}

fn map_access(err: std::io::Error) -> AtlasError {
    match err.kind() {
        ErrorKind::NotFound => AtlasError::SourceMissing,
        ErrorKind::PermissionDenied => AtlasError::SourceUnreadable,
        ErrorKind::NotADirectory => AtlasError::SourceNotAFolder,
        _ => AtlasError::io("read that folder")(err),
    }
}

fn canonical_data_root(root: &OpenRoot) -> PathBuf {
    fs::canonicalize(root.path()).unwrap_or_else(|_| root.path().to_path_buf())
}

/// Lists library folders with live status, ordered by display path.
pub fn list(root: &OpenRoot) -> AtlasResult<Vec<SourceRoot>> {
    let records = SqliteCatalog::new(root.catalog()).list_source_root_records()?;
    let paths: Vec<Option<PathBuf>> = records
        .iter()
        .map(|r| native_path::decode(&r.path_bytes))
        .collect();
    let statuses = probe::probe_all(&paths);
    let data_root = canonical_data_root(root);

    Ok(records
        .iter()
        .zip(&paths)
        .zip(statuses)
        .map(|((record, path), status)| {
            let nested_in_root_id = path
                .as_ref()
                .and_then(|p| outermost_container(p, &records, &paths));
            SourceRoot {
                id: record.id.clone(),
                display_path: record.display_path.clone(),
                status,
                contains_data_root: path
                    .as_ref()
                    .is_some_and(|p| relation_by_components(&data_root, p) == Relation::Inside),
                nested_in_root_id,
                created_at: record.created_at.clone(),
                last_scan_at: record.last_scan_at.clone(),
            }
        })
        .collect())
}

/// The ID of the outermost other root containing `path`, if any.
fn outermost_container(
    path: &Path,
    records: &[SourceRootRecord],
    paths: &[Option<PathBuf>],
) -> Option<String> {
    records
        .iter()
        .zip(paths)
        .filter_map(|(record, other)| {
            let other = other.as_ref()?;
            (relation_by_components(path, other) == Relation::Inside)
                .then_some((other.components().count(), &record.id))
        })
        .min_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)))
        .map(|(_, id)| id.clone())
}

/// Adds one folder after applying every rule above.
pub fn add(root: &OpenRoot, path: &Path) -> AtlasResult<SourceRoot> {
    if !path.is_absolute() {
        return Err(AtlasError::InvalidPath);
    }
    let meta = fs::metadata(path).map_err(map_access)?;
    if !meta.is_dir() {
        return Err(AtlasError::SourceNotAFolder);
    }
    drop(fs::read_dir(path).map_err(map_access)?);
    let canonical = fs::canonicalize(path).map_err(map_access)?;

    match relation(&canonical, &canonical_data_root(root)) {
        Relation::Same | Relation::Inside => return Err(AtlasError::SourceIsDataRoot),
        Relation::Contains | Relation::Unrelated => {}
    }

    let catalog = SqliteCatalog::new(root.catalog());
    for record in catalog.list_source_root_records()? {
        let Some(existing) = native_path::decode(&record.path_bytes) else {
            continue;
        };
        match relation(&canonical, &existing) {
            Relation::Same => {
                return Err(AtlasError::SourceAlreadyAdded {
                    existing_root_id: Some(record.id),
                });
            }
            Relation::Inside | Relation::Contains => {
                return Err(AtlasError::SourceOverlaps {
                    conflicting_root_id: record.id,
                });
            }
            Relation::Unrelated => {}
        }
    }

    let record = SourceRootRecord {
        id: uuid::Uuid::new_v4().to_string(),
        workspace_id: root.manifest().root_id.clone(),
        path_bytes: native_path::encode(&canonical),
        display_path: native_path::display(&canonical),
        created_at: now_rfc3339(),
        last_scan_at: None,
    };
    catalog.insert_source_root(record.clone())?;

    let data_root = canonical_data_root(root);
    Ok(SourceRoot {
        id: record.id,
        display_path: record.display_path,
        status: probe::probe(&canonical),
        contains_data_root: relation_by_components(&data_root, &canonical) == Relation::Inside,
        nested_in_root_id: None,
        created_at: record.created_at,
        last_scan_at: None,
    })
}

/// Adds several folders, collecting a reason for each one refused.
pub fn add_many(root: &OpenRoot, paths: &[PathBuf]) -> AddSourceRootsResult {
    let mut result = AddSourceRootsResult::default();
    for path in paths {
        match add(root, path) {
            Ok(added) => result.added.push(added),
            Err(err) => result.rejected.push(SourceRootRejection {
                display_path: native_path::display(path),
                conflicting_root_id: err.conflicting_root_id(),
                error: err.payload(),
            }),
        }
    }
    result
}

/// Removes a folder from Atlas. Files on disk are never touched.
pub fn remove(root: &OpenRoot, id: &str) -> AtlasResult<()> {
    if SqliteCatalog::new(root.catalog()).delete_source_root(id)? {
        Ok(())
    } else {
        Err(AtlasError::SourceNotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_relations() {
        let books = Path::new("/data/Books");
        assert_eq!(relation_by_components(books, books), Relation::Same);
        assert_eq!(
            relation_by_components(Path::new("/data/Books/Astronomy"), books),
            Relation::Inside
        );
        assert_eq!(
            relation_by_components(Path::new("/data"), books),
            Relation::Contains
        );
        // Component-wise, not string prefix.
        assert_eq!(
            relation_by_components(Path::new("/data/Books2"), books),
            Relation::Unrelated
        );
    }

    #[test]
    fn identity_relations_see_through_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        fs::create_dir_all(real.join("inner")).unwrap();
        #[cfg(unix)]
        {
            let link = dir.path().join("link");
            std::os::unix::fs::symlink(&real, &link).unwrap();
            assert_eq!(relation(&link, &real), Relation::Same);
        }
        assert_eq!(relation(&real.join("inner"), &real), Relation::Inside);
        assert_eq!(relation(&real, &real.join("inner")), Relation::Contains);
        assert_eq!(
            relation(&real, &dir.path().join("unplugged")),
            Relation::Unrelated
        );
    }
}
