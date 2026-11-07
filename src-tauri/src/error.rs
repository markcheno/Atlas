//! The single error type crossing the IPC boundary.
//!
//! Every command returns `Result<T, AtlasError>`. On the wire an error is an
//! [`ErrorPayload`]: a stable machine-readable `code`, a safe human-readable
//! `message`, and an optional `recovery` hint the UI can show as an action.
//! Messages must never contain secrets, extracted document text, or
//! unredacted user file paths.

use serde::{Serialize, Serializer};
use ts_rs::TS;

/// Stable error codes. Renaming a variant is a breaking IPC change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ErrorCode {
    /// The command exists in the contract but its slice has not shipped yet.
    NotImplemented,
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

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AtlasError {
    #[error("{command} is not available yet; planned for Phase 0 slice {planned_slice}")]
    NotImplemented {
        command: &'static str,
        planned_slice: &'static str,
    },
}

impl AtlasError {
    pub fn not_implemented(command: &'static str, planned_slice: &'static str) -> Self {
        Self::NotImplemented {
            command,
            planned_slice,
        }
    }

    pub fn code(&self) -> ErrorCode {
        match self {
            Self::NotImplemented { .. } => ErrorCode::NotImplemented,
        }
    }

    pub fn payload(&self) -> ErrorPayload {
        match self {
            Self::NotImplemented {
                command,
                planned_slice,
            } => ErrorPayload {
                code: self.code(),
                message: self.to_string(),
                recovery: None,
                command: Some((*command).to_owned()),
                planned_slice: Some((*planned_slice).to_owned()),
            },
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
}
