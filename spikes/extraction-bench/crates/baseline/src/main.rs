//! Baseline "extractor": does nothing, but links what every candidate
//! links (the app's libraries via xbench). The reference for binary size
//! and build time.

use std::path::Path;

use serde_json::json;
use xbench::{Extracted, Failure};

fn extract(path: &Path) -> Result<Extracted, Failure> {
    let bytes = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    Ok(Extracted { pages: Vec::new(), meta: json!({ "bytes": bytes }) })
}

fn main() {
    xbench::main_with("baseline", "none", extract);
}
