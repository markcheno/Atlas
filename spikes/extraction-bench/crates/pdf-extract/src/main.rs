//! pdf-extract 0.12.1 (pure Rust, on lopdf): the reference for what
//! shipping no native PDF library costs.

use std::path::Path;

use serde_json::json;
use xbench::{Extracted, Failure};

fn extract(path: &Path) -> Result<Extracted, Failure> {
    let pages = pdf_extract::extract_text_by_pages(path).map_err(|e| {
        let msg = format!("{e:?}");
        if msg.to_lowercase().contains("encrypt") || msg.to_lowercase().contains("password") {
            Failure::Encrypted(msg)
        } else {
            Failure::Malformed(msg)
        }
    })?;
    Ok(Extracted { meta: json!({ "pageCount": pages.len() }), pages })
}

fn main() {
    xbench::main_with("pdf-extract", "pdf-extract 0.12.1", extract);
}
