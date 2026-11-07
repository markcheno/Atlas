//! Read-only assessment of a prospective data-root location.
//!
//! Inspecting never creates the folder. The only write is a uniquely named
//! empty probe file, created and removed immediately, to test writability
//! in a folder Atlas would write to anyway.

use std::fs;
use std::io;
use std::path::Path;

use super::layout::{IGNORABLE_ENTRIES, INIT_MARKER, MANIFEST, SUGGESTED_SUBFOLDER};
use super::manifest;
use crate::domain::dto::{CandidateKind, CandidateProblem, DataRootCandidate, VolumeKind};
use crate::domain::traits::{VolumeInfo, VolumeInspector};
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::fsutil::{nearest_existing_ancestor, probe_writable};
use crate::services::catalog::SUPPORTED_SCHEMA_VERSION;
use crate::services::system::low_space_threshold;

/// Data-root paths must be absolute and valid Unicode so they can be shown,
/// stored in the locator, and passed over IPC without loss.
pub fn display_path(path: &Path) -> AtlasResult<String> {
    match path.to_str() {
        Some(s) if !s.is_empty() && path.is_absolute() => Ok(s.to_owned()),
        _ => Err(AtlasError::InvalidPath),
    }
}

pub fn classify(path: &Path) -> AtlasResult<CandidateKind> {
    let meta = match fs::metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(CandidateKind::NewFolder),
        Err(err) => return Err(AtlasError::io("look at that folder")(err)),
    };
    if !meta.is_dir() {
        return Ok(CandidateKind::NotAFolder);
    }
    if path.join(MANIFEST).exists() {
        return Ok(CandidateKind::ExistingLibrary);
    }
    if path.join(INIT_MARKER).exists() {
        return Ok(CandidateKind::IncompleteLibrary);
    }
    let mut entries = fs::read_dir(path).map_err(AtlasError::io("look inside that folder"))?;
    let has_content = entries.any(|entry| {
        entry.map_or(true, |e| {
            !IGNORABLE_ENTRIES
                .iter()
                .any(|ignored| e.file_name() == *ignored)
        })
    });
    Ok(if has_content {
        CandidateKind::NonEmptyFolder
    } else {
        CandidateKind::EmptyFolder
    })
}

/// True when some ancestor (not `path` itself) is an Atlas data root.
pub fn inside_existing_library(path: &Path) -> bool {
    path.ancestors().skip(1).any(|a| a.join(MANIFEST).is_file())
}

pub fn inspect(path: &Path, volumes: &dyn VolumeInspector) -> AtlasResult<DataRootCandidate> {
    let display = display_path(path)?;
    let kind = classify(path)?;
    let mut problems = Vec::new();
    let mut root_id = None;
    let mut suggested_subfolder = None;

    if inside_existing_library(path) {
        problems.push(CandidateProblem::InsideLibrary);
    }

    match kind {
        CandidateKind::ExistingLibrary => match manifest::read(path) {
            Ok(m) => {
                if m.catalog_schema_version > SUPPORTED_SCHEMA_VERSION {
                    problems.push(CandidateProblem::NewerSchema);
                }
                root_id = Some(m.root_id);
            }
            Err(AtlasError::DataRootIncompatible { .. }) => {
                problems.push(CandidateProblem::NewerSchema);
            }
            Err(_) => problems.push(CandidateProblem::InvalidManifest),
        },
        CandidateKind::NonEmptyFolder => {
            let sub = path.join(SUGGESTED_SUBFOLDER);
            let usable = matches!(
                classify(&sub),
                Ok(CandidateKind::NewFolder
                    | CandidateKind::EmptyFolder
                    | CandidateKind::ExistingLibrary
                    | CandidateKind::IncompleteLibrary)
            );
            if usable {
                suggested_subfolder = sub.to_str().map(str::to_owned);
            }
        }
        _ => {}
    }

    // A volume that cannot be inspected is reported as unknown, not fatal.
    let volume = volumes.inspect(path).unwrap_or(VolumeInfo {
        free_bytes: None,
        total_bytes: None,
        fs_type: None,
        kind: VolumeKind::Unknown,
    });
    match volume.kind {
        VolumeKind::Network => problems.push(CandidateProblem::NetworkVolume),
        VolumeKind::CloudSynced => problems.push(CandidateProblem::CloudSynced),
        VolumeKind::Local | VolumeKind::Unknown => {}
    }

    let creates = matches!(
        kind,
        CandidateKind::NewFolder | CandidateKind::EmptyFolder | CandidateKind::IncompleteLibrary
    );
    let required_free_bytes = volume.total_bytes.map(low_space_threshold);
    if creates
        && let (Some(free), Some(required)) = (volume.free_bytes, required_free_bytes)
        && free < required
    {
        problems.push(CandidateProblem::LowSpace);
    }

    let write_target = match kind {
        CandidateKind::NewFolder => nearest_existing_ancestor(path),
        CandidateKind::EmptyFolder
        | CandidateKind::IncompleteLibrary
        | CandidateKind::ExistingLibrary => Some(path.to_path_buf()),
        CandidateKind::NonEmptyFolder | CandidateKind::NotAFolder => None,
    };
    if let Some(dir) = write_target
        && !probe_writable(&dir)
    {
        problems.push(CandidateProblem::NotWritable);
    }

    let can_use = problems.is_empty()
        && !matches!(
            kind,
            CandidateKind::NonEmptyFolder | CandidateKind::NotAFolder
        );

    Ok(DataRootCandidate {
        display_path: display,
        kind,
        problems,
        can_use,
        free_bytes: volume.free_bytes,
        total_bytes: volume.total_bytes,
        required_free_bytes,
        volume_kind: volume.kind,
        suggested_subfolder,
        root_id,
    })
}
