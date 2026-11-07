//! The single error type crossing the IPC boundary.
//!
//! Every command returns `Result<T, AtlasError>`. On the wire an error is an
//! [`ErrorPayload`]: a stable machine-readable `code`, a safe human-readable
//! `message`, and an optional `recovery` hint the UI can show as an action.
//! Messages must never contain secrets, extracted document text, or full
//! user file paths; the UI already knows which path it asked about.

use serde::{Serialize, Serializer};
use ts_rs::TS;

/// Stable error codes. Renaming a variant is a breaking IPC change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ErrorCode {
    /// The command exists in the contract but its slice has not shipped yet.
    NotImplemented,
    /// A path argument was relative, empty, or not valid Unicode.
    InvalidPath,
    DataRootNotWritable,
    /// The data root's folder or drive is not present.
    DataRootMissing,
    /// Another running Atlas holds this data root's lock.
    DataRootInUse,
    /// Created by a newer Atlas; refused to avoid unsafe writes.
    DataRootIncompatible,
    /// Manifest or catalog is unreadable, malformed, or incomplete.
    DataRootInvalid,
    DataRootNetworkVolume,
    DataRootCloudSynced,
    DataRootLowSpace,
    DataRootNotAFolder,
    /// The folder is inside another Atlas library.
    DataRootNested,
    /// The folder holds other files; Atlas will not mix its data into it.
    FolderNotEmpty,
    /// The operation needs an open data root and none is open.
    NoDataRoot,
    /// The chosen library folder does not exist.
    SourceMissing,
    SourceNotAFolder,
    /// Atlas lacks permission to read the folder.
    SourceUnreadable,
    /// The folder is the Atlas data root or inside it.
    SourceIsDataRoot,
    SourceAlreadyAdded,
    /// The folder is inside, or contains, a folder already in the library.
    SourceOverlaps,
    /// No library folder has that ID.
    SourceNotFound,
    /// No cataloging job has that ID.
    JobNotFound,
    /// The job has already finished, so it cannot be paused or cancelled.
    JobNotPausable,
    /// Only a job paused by the user can be resumed.
    JobNotResumable,
    /// No cataloged file has that ID.
    FileNotFound,
    /// The file is no longer where it was cataloged.
    FileMissing,
    /// The cataloged path is now a link or something other than a file.
    FileNotRegular,
    /// The path now leads outside its library folder.
    FileOutsideFolder,
    /// Opening it will download it from a cloud service; ask first.
    FileCloudOnly,
    /// Only a legacy iCloud placeholder is present; download it in Finder.
    FileCloudPlaceholder,
    /// The library folder holding the file is not connected.
    SourceUnavailable,
    /// The operating system could not open or show the file.
    OpenFailed,
    IoError,
    DatabaseError,
    Internal,
}

/// Serialized form of [`AtlasError`] as received by the frontend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ErrorPayload {
    pub code: ErrorCode,
    pub message: String,
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery: Option<String>,
    /// For `not_implemented`: the IPC command that was called.
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// For `not_implemented`: the Phase 0 slice planned to deliver it.
    #[ts(optional)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub planned_slice: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum AtlasError {
    #[error("{command} is not available yet; planned for Phase 0 slice {planned_slice}")]
    NotImplemented {
        command: &'static str,
        planned_slice: &'static str,
    },
    #[error("That location is not a usable folder path.")]
    InvalidPath,
    #[error("Atlas cannot write to that location.")]
    DataRootNotWritable,
    #[error("Atlas cannot find its library at that location.")]
    DataRootMissing,
    #[error("This Atlas library is already open in another Atlas window or process.")]
    DataRootInUse,
    #[error(
        "This library was created by a newer version of Atlas (format {found}; this version supports up to {supported})."
    )]
    DataRootIncompatible { found: u32, supported: u32 },
    #[error("This Atlas library is damaged or incomplete: {reason}.")]
    DataRootInvalid { reason: &'static str },
    #[error("Atlas libraries cannot be stored on a network drive.")]
    DataRootNetworkVolume,
    #[error("Atlas libraries cannot be stored in a cloud-synced folder.")]
    DataRootCloudSynced,
    #[error(
        "Not enough free space: {free_mib} MiB free, at least {required_mib} MiB needed to start safely."
    )]
    DataRootLowSpace { free_mib: u64, required_mib: u64 },
    #[error("That location is a file, not a folder.")]
    DataRootNotAFolder,
    #[error("That folder is inside another Atlas library.")]
    DataRootNested,
    #[error("That folder already contains other files.")]
    FolderNotEmpty,
    #[error("No Atlas library is open.")]
    NoDataRoot,
    #[error("That folder could not be found.")]
    SourceMissing,
    #[error("That is a file, not a folder.")]
    SourceNotAFolder,
    #[error("Atlas does not have permission to read that folder.")]
    SourceUnreadable,
    #[error("That folder is where Atlas keeps its own data, not your documents.")]
    SourceIsDataRoot,
    #[error("That folder is already in your library.")]
    SourceAlreadyAdded { existing_root_id: Option<String> },
    #[error("That folder overlaps a folder already in your library.")]
    SourceOverlaps { conflicting_root_id: String },
    #[error("That library folder no longer exists in Atlas.")]
    SourceNotFound,
    #[error("That cataloging job no longer exists.")]
    JobNotFound,
    #[error("That cataloging job has already finished.")]
    JobNotPausable,
    #[error("That cataloging job is not paused.")]
    JobNotResumable,
    #[error("Atlas can't find that file in its catalog.")]
    FileNotFound,
    #[error("That file is no longer where Atlas found it.")]
    FileMissing,
    #[error("That item is no longer an ordinary file, so Atlas won't open it.")]
    FileNotRegular,
    #[error("That file's location now leads outside its library folder, so Atlas won't open it.")]
    FileOutsideFolder,
    #[error("This file is stored only in the cloud. Opening it will download it.")]
    FileCloudOnly,
    #[error("This file is stored only in iCloud and must be downloaded before it can be opened.")]
    FileCloudPlaceholder,
    #[error("The folder that holds this file isn't connected.")]
    SourceUnavailable,
    #[error("The system couldn't open that file.")]
    OpenFailed,
    #[error("Atlas could not {context} ({kind}).", kind = source.kind())]
    Io {
        context: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("The Atlas catalog could not {context}.")]
    Database {
        context: &'static str,
        #[source]
        source: rusqlite::Error,
    },
    #[error("Atlas hit an internal error: {0}.")]
    Internal(&'static str),
}

impl AtlasError {
    pub fn not_implemented(command: &'static str, planned_slice: &'static str) -> Self {
        Self::NotImplemented {
            command,
            planned_slice,
        }
    }

    pub fn io(context: &'static str) -> impl FnOnce(std::io::Error) -> Self {
        move |source| Self::Io { context, source }
    }

    pub fn db(context: &'static str) -> impl FnOnce(rusqlite::Error) -> Self {
        move |source| Self::Database { context, source }
    }

    /// The existing library folder an add conflicted with, if any.
    pub fn conflicting_root_id(&self) -> Option<String> {
        match self {
            Self::SourceOverlaps {
                conflicting_root_id,
            } => Some(conflicting_root_id.clone()),
            Self::SourceAlreadyAdded { existing_root_id } => existing_root_id.clone(),
            _ => None,
        }
    }

    pub fn code(&self) -> ErrorCode {
        match self {
            Self::NotImplemented { .. } => ErrorCode::NotImplemented,
            Self::InvalidPath => ErrorCode::InvalidPath,
            Self::DataRootNotWritable => ErrorCode::DataRootNotWritable,
            Self::DataRootMissing => ErrorCode::DataRootMissing,
            Self::DataRootInUse => ErrorCode::DataRootInUse,
            Self::DataRootIncompatible { .. } => ErrorCode::DataRootIncompatible,
            Self::DataRootInvalid { .. } => ErrorCode::DataRootInvalid,
            Self::DataRootNetworkVolume => ErrorCode::DataRootNetworkVolume,
            Self::DataRootCloudSynced => ErrorCode::DataRootCloudSynced,
            Self::DataRootLowSpace { .. } => ErrorCode::DataRootLowSpace,
            Self::DataRootNotAFolder => ErrorCode::DataRootNotAFolder,
            Self::DataRootNested => ErrorCode::DataRootNested,
            Self::FolderNotEmpty => ErrorCode::FolderNotEmpty,
            Self::NoDataRoot => ErrorCode::NoDataRoot,
            Self::SourceMissing => ErrorCode::SourceMissing,
            Self::SourceNotAFolder => ErrorCode::SourceNotAFolder,
            Self::SourceUnreadable => ErrorCode::SourceUnreadable,
            Self::SourceIsDataRoot => ErrorCode::SourceIsDataRoot,
            Self::SourceAlreadyAdded { .. } => ErrorCode::SourceAlreadyAdded,
            Self::SourceOverlaps { .. } => ErrorCode::SourceOverlaps,
            Self::SourceNotFound => ErrorCode::SourceNotFound,
            Self::JobNotFound => ErrorCode::JobNotFound,
            Self::JobNotPausable => ErrorCode::JobNotPausable,
            Self::JobNotResumable => ErrorCode::JobNotResumable,
            Self::FileNotFound => ErrorCode::FileNotFound,
            Self::FileMissing => ErrorCode::FileMissing,
            Self::FileNotRegular => ErrorCode::FileNotRegular,
            Self::FileOutsideFolder => ErrorCode::FileOutsideFolder,
            Self::FileCloudOnly => ErrorCode::FileCloudOnly,
            Self::FileCloudPlaceholder => ErrorCode::FileCloudPlaceholder,
            Self::SourceUnavailable => ErrorCode::SourceUnavailable,
            Self::OpenFailed => ErrorCode::OpenFailed,
            Self::Io { .. } => ErrorCode::IoError,
            Self::Database { .. } => ErrorCode::DatabaseError,
            Self::Internal(_) => ErrorCode::Internal,
        }
    }

    fn recovery(&self) -> Option<&'static str> {
        Some(match self {
            Self::NotImplemented { .. } | Self::Internal(_) => return None,
            Self::InvalidPath | Self::DataRootNotAFolder => "Choose a folder.",
            Self::DataRootNotWritable => {
                "Choose a folder you can write to, or check its permissions."
            }
            Self::DataRootMissing => {
                "Reconnect the drive that holds your Atlas library, then try again."
            }
            Self::DataRootInUse => "Close the other Atlas window, then try again.",
            Self::DataRootIncompatible { .. } => {
                "Update Atlas to the latest version to open this library."
            }
            Self::DataRootInvalid { .. } => {
                "Choose another location. Your original documents are not affected."
            }
            Self::DataRootNetworkVolume | Self::DataRootCloudSynced => {
                "Choose a folder on this computer or on a directly connected drive."
            }
            Self::DataRootLowSpace { .. } => {
                "Free up space on that drive, or choose a drive with more room."
            }
            Self::DataRootNested => "Choose a folder outside your existing Atlas library.",
            Self::FolderNotEmpty => "Choose an empty folder, or let Atlas create one inside it.",
            Self::NoDataRoot => "Choose where Atlas should keep its library.",
            Self::SourceMissing => {
                "Check that the drive is connected, then choose the folder again."
            }
            Self::SourceNotAFolder => "Choose a folder that contains your documents.",
            Self::SourceUnreadable => {
                "Allow Atlas to read this folder in your system privacy settings, or choose another folder."
            }
            Self::SourceIsDataRoot => "Choose the folders where you keep your books and papers.",
            Self::SourceAlreadyAdded { .. } => "Nothing to do; it is already listed.",
            Self::SourceOverlaps { .. } => {
                "Keep the outer folder, or remove it first and add the folders inside it separately."
            }
            Self::SourceNotFound => "Refresh the list of library folders.",
            Self::JobNotFound | Self::JobNotPausable | Self::JobNotResumable => {
                "Refresh to see the current cataloging status."
            }
            Self::FileNotFound => "Refresh the library list.",
            Self::FileMissing => {
                "It may have been moved, renamed, or deleted. Rescan its folder to update the library."
            }
            Self::FileNotRegular | Self::FileOutsideFolder => {
                "Rescan its folder to update the library."
            }
            Self::FileCloudOnly => "Choose Download and open to continue.",
            Self::FileCloudPlaceholder => {
                "Show it in Finder and download it there, then open it again."
            }
            Self::SourceUnavailable => "Reconnect the drive or folder, then try again.",
            Self::OpenFailed => "Check that an app that opens this kind of file is installed.",
            Self::Io { .. } | Self::Database { .. } => {
                "Try again. If this keeps happening, check the drive for problems."
            }
        })
    }

    pub fn payload(&self) -> ErrorPayload {
        let (command, planned_slice) = match self {
            Self::NotImplemented {
                command,
                planned_slice,
            } => (
                Some((*command).to_owned()),
                Some((*planned_slice).to_owned()),
            ),
            _ => (None, None),
        };
        ErrorPayload {
            code: self.code(),
            message: self.to_string(),
            recovery: self.recovery().map(str::to_owned),
            command,
            planned_slice,
        }
    }
}

impl Serialize for AtlasError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.payload().serialize(serializer)
    }
}

pub type AtlasResult<T> = Result<T, AtlasError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_implemented_serializes_to_stable_shape() {
        let err = AtlasError::not_implemented("list_documents", "0.6");
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "code": "not_implemented",
                "message": "list_documents is not available yet; planned for Phase 0 slice 0.6",
                "command": "list_documents",
                "plannedSlice": "0.6"
            })
        );
    }

    #[test]
    fn io_errors_never_include_paths() {
        let source = std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "/Users/someone/private/file",
        );
        let payload = AtlasError::io("write the settings file")(source).payload();
        assert_eq!(payload.code, ErrorCode::IoError);
        assert!(!payload.message.contains("/Users/"), "{}", payload.message);
        assert!(payload.recovery.is_some());
    }
}
