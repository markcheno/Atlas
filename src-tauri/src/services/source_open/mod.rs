//! Opening and revealing original files (phase0-spec §9, ADR 0006).
//!
//! The renderer only ever sends a file ID. Rust looks up the cataloged
//! path, checks it is still a regular file inside its authorized folder,
//! and hands the native path to the OS opener. Paths are never put into a
//! shell command string.
//!
//! Cloud-offloaded files: opening a dataless file makes the OS download
//! it, so Atlas asks first (`allow_download`). A legacy iCloud placeholder
//! cannot be opened by its real name; Atlas offers to reveal it instead.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use rusqlite::OptionalExtension;

use crate::domain::dto::SourceRootStatus;
use crate::domain::traits::SourceOpener;
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::local_source;
use crate::infrastructure::native_path;
use crate::services::sources::probe;
use crate::services::workspace::OpenRoot;

/// What Atlas found when it went to open a cataloged file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// A regular local or cloud-dataless file at `path`.
    File { path: PathBuf, cloud_only: bool },
    /// Only a legacy iCloud placeholder (`.Name.icloud`) exists.
    Placeholder { placeholder: PathBuf },
}

fn lookup(root: &OpenRoot, file_id: &str) -> AtlasResult<(PathBuf, PathBuf)> {
    let id = file_id.to_owned();
    let row: Option<(Vec<u8>, Vec<u8>)> = root
        .reader()
        .read(move |c| {
            c.query_row(
                "SELECT r.path_bytes, f.rel_path FROM file_instances f
                 JOIN source_roots r ON r.id = f.source_root_id WHERE f.id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
        })?
        .map_err(AtlasError::db("find the file"))?;
    let (root_bytes, rel_bytes) = row.ok_or(AtlasError::FileNotFound)?;
    match (
        native_path::decode(&root_bytes),
        native_path::decode(&rel_bytes),
    ) {
        (Some(root), Some(rel)) => Ok((root, rel)),
        _ => Err(AtlasError::SourceUnavailable),
    }
}

/// Validates a cataloged file before handing it to the OS.
pub fn resolve(source_root: &Path, rel: &Path) -> AtlasResult<Resolved> {
    let reachable = probe::probe_all(&[Some(source_root.to_path_buf())]);
    if reachable[0] != SourceRootStatus::Available {
        return Err(AtlasError::SourceUnavailable);
    }
    let path = source_root.join(rel);
    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
                let mut placeholder_name = std::ffi::OsString::from(".");
                placeholder_name.push(name);
                placeholder_name.push(".icloud");
                let placeholder = parent.join(placeholder_name);
                if fs::symlink_metadata(&placeholder).is_ok_and(|m| m.is_file()) {
                    return Ok(Resolved::Placeholder { placeholder });
                }
            }
            return Err(AtlasError::FileMissing);
        }
        Err(_) => return Err(AtlasError::SourceUnavailable),
    };
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err(AtlasError::FileNotRegular);
    }
    let (Ok(canonical), Ok(canonical_root)) =
        (dunce::canonicalize(&path), dunce::canonicalize(source_root))
    else {
        return Err(AtlasError::FileMissing);
    };
    // Compare in one form: a name that isn't valid Unicode keeps Windows'
    // verbatim prefix while its folder doesn't (native_path::comparable).
    if !native_path::comparable(&canonical).starts_with(native_path::comparable(&canonical_root)) {
        return Err(AtlasError::FileOutsideFolder);
    }
    Ok(Resolved::File {
        path,
        cloud_only: local_source::is_cloud_only(&meta),
    })
}

/// Opens a cataloged file in its default application.
pub fn open(
    root: &OpenRoot,
    opener: &dyn SourceOpener,
    file_id: &str,
    allow_download: bool,
) -> AtlasResult<()> {
    let (source_root, rel) = lookup(root, file_id)?;
    match resolve(&source_root, &rel)? {
        Resolved::Placeholder { .. } => Err(AtlasError::FileCloudPlaceholder),
        Resolved::File {
            cloud_only: true, ..
        } if !allow_download => Err(AtlasError::FileCloudOnly),
        Resolved::File { path, .. } => opener.open_original(&path),
    }
}

/// The path to read a cataloged file from, after Open's checks: the folder
/// is reachable, the path is a regular file inside it, and a file only in
/// the cloud needs `allow_download` (the page view, Slice 3.7).
pub fn path_for(root: &OpenRoot, file_id: &str, allow_download: bool) -> AtlasResult<PathBuf> {
    let (source_root, rel) = lookup(root, file_id)?;
    match resolve(&source_root, &rel)? {
        Resolved::Placeholder { .. } => Err(AtlasError::FileCloudPlaceholder),
        Resolved::File {
            cloud_only: true, ..
        } if !allow_download => Err(AtlasError::FileCloudOnly),
        Resolved::File { path, .. } => Ok(path),
    }
}

/// Shows a cataloged file selected in the OS file manager. Never downloads.
pub fn reveal(root: &OpenRoot, opener: &dyn SourceOpener, file_id: &str) -> AtlasResult<()> {
    let (source_root, rel) = lookup(root, file_id)?;
    match resolve(&source_root, &rel)? {
        Resolved::Placeholder { placeholder } => opener.reveal_original(&placeholder),
        Resolved::File { path, .. } => opener.reveal_original(&path),
    }
}

/// The OS opener (`tauri-plugin-opener`): `NSWorkspace` / `open(1)` with a
/// separate argument on macOS, `ShellExecuteW` and
/// `SHOpenFolderAndSelectItems` on Windows, `xdg-open` and the D-Bus
/// FileManager1 interface on Linux.
#[derive(Debug, Default, Clone, Copy)]
pub struct OsOpener;

impl SourceOpener for OsOpener {
    fn open_original(&self, path: &Path) -> AtlasResult<()> {
        tauri_plugin_opener::open_path(path, None::<&str>).map_err(|_| AtlasError::OpenFailed)
    }

    fn reveal_original(&self, path: &Path) -> AtlasResult<()> {
        tauri_plugin_opener::reveal_item_in_dir(path).map_err(|_| AtlasError::OpenFailed)
    }
}
