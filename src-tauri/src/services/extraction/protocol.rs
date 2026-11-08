//! The worker protocol: JSON lines, one request at a time (phase1-spec
//! §7.2). The app sends [`Request`]s on the worker's stdin; the worker
//! answers with [`Frame`]s on its stdout. Reasons are fixed codes; frames
//! never carry paths. Paths in requests are the catalog's platform-tagged
//! bytes (`native_path::encode`), hex-encoded, so any path survives JSON.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::engine::hash::HashError;
use super::engine::{Outcome, Piece, Summary};
use super::locate::Unavailable;
use crate::domain::dto::FileType;
use crate::infrastructure::native_path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Request {
    /// Answered by `hashed` or `hashFailed`.
    Hash { path: String },
    /// Answered by `page` frames, then `done` or `failed`.
    Extract { path: String, kind: FileType },
}

/// Encodes a path for a request.
pub fn encode_path(path: &Path) -> String {
    native_path::encode(path)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Decodes a request's path; `None` if it is not one this platform wrote.
pub fn decode_path(hex: &str) -> Option<PathBuf> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    native_path::decode(&bytes)
}

impl Request {
    pub fn to_line(&self) -> String {
        let mut line = serde_json::to_string(self).unwrap_or_default();
        line.push('\n');
        line
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Frame {
    /// The worker's first line once PDFium is loaded and has read the
    /// built-in self-test PDF.
    Hello {
        protocol: u32,
        extractor_version: String,
        pdfium_version: String,
        self_test: SelfTest,
    },
    /// The first and last line when PDFium can't be used.
    Unavailable {
        reason: Unavailable,
    },
    /// A request the worker can't answer, or a panic outside a request.
    Error {
        reason: String,
    },
    Hashed {
        /// BLAKE3-256, lowercase hex.
        hash: String,
        size: u64,
        mtime_secs: Option<i64>,
    },
    HashFailed {
        reason: HashError,
    },
    /// One passage. Pages arrive in order, before `done`.
    Page {
        ordinal: u32,
        page_no: Option<u32>,
        location: String,
        text: String,
    },
    Done {
        summary: Summary,
    },
    /// No usable text. Any pages already sent are discarded by the app.
    Failed {
        outcome: Outcome,
        reason: String,
    },
}

impl Frame {
    pub fn page(piece: Piece) -> Self {
        Frame::Page {
            ordinal: piece.ordinal,
            page_no: piece.page_no,
            location: piece.location,
            text: piece.text,
        }
    }
}

/// What PDFium read from the built-in PDF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelfTest {
    pub pages: u32,
    /// Non-whitespace characters across all pages.
    pub chars: u32,
}

impl Frame {
    pub fn to_line(&self) -> String {
        let mut line = serde_json::to_string(self)
            .unwrap_or_else(|_| r#"{"type":"error","reason":"encode"}"#.to_owned());
        line.push('\n');
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_use_the_documented_json_names() {
        let hello = Frame::Hello {
            protocol: 1,
            extractor_version: "0.1.0".into(),
            pdfium_version: "chromium/8125".into(),
            self_test: SelfTest { pages: 2, chars: 3 },
        };
        assert_eq!(
            hello.to_line(),
            "{\"type\":\"hello\",\"protocol\":1,\"extractorVersion\":\"0.1.0\",\
             \"pdfiumVersion\":\"chromium/8125\",\"selfTest\":{\"pages\":2,\"chars\":3}}\n"
        );
        assert_eq!(
            Frame::Unavailable {
                reason: Unavailable::LoadFailed
            }
            .to_line(),
            "{\"type\":\"unavailable\",\"reason\":\"load_failed\"}\n"
        );
        let back: Frame = serde_json::from_str(hello.to_line().trim()).unwrap();
        assert_eq!(back, hello);
    }

    #[test]
    fn requests_and_paths_round_trip() {
        let path = Path::new("/Books/Café crème.pdf");
        let req = Request::Extract {
            path: encode_path(path),
            kind: FileType::Pdf,
        };
        let line = req.to_line();
        assert!(
            line.starts_with("{\"type\":\"extract\",\"path\":\""),
            "{line}"
        );
        let back: Request = serde_json::from_str(line.trim()).unwrap();
        let Request::Extract { path: hex, .. } = back else {
            panic!("{back:?}")
        };
        assert_eq!(decode_path(&hex).as_deref(), Some(path));
        assert_eq!(decode_path("zz"), None);
        assert_eq!(decode_path("abc"), None);
        let done = Frame::Done {
            summary: Summary {
                page_count: 2,
                char_count: 10,
                ..Summary::default()
            },
        };
        assert!(done.to_line().contains("\"pageCount\":2"));
        assert_eq!(
            Frame::Failed {
                outcome: Outcome::NeedsOcr,
                reason: "no_text_layer".into()
            }
            .to_line(),
            "{\"type\":\"failed\",\"outcome\":\"needs_ocr\",\"reason\":\"no_text_layer\"}\n"
        );
    }

    #[test]
    fn the_pinned_pdfium_matches_tools_lock() {
        let lock: serde_json::Value =
            serde_json::from_str(include_str!("../../../../tools.lock.json")).unwrap();
        assert_eq!(
            lock["tools"]["pdfium"]["version"],
            super::super::PDFIUM_VERSION
        );
    }
}
