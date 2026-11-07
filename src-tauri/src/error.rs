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
