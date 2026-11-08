//! Frames the worker writes, one JSON object per line (phase1-spec §7.2).
//! Reasons are fixed codes; they never carry paths or document text.

use serde::{Deserialize, Serialize};

use super::locate::Unavailable;

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
    Unavailable { reason: Unavailable },
    /// A request the worker can't answer, or a panic.
    Error { reason: String },
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
    fn the_pinned_pdfium_matches_tools_lock() {
        let lock: serde_json::Value =
            serde_json::from_str(include_str!("../../../../tools.lock.json")).unwrap();
        assert_eq!(
            lock["tools"]["pdfium"]["version"],
            super::super::PDFIUM_VERSION
        );
    }
}
