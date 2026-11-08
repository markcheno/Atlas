//! EPUB with rbook 0.7.12 (reading order = the spine) and html2text 0.17.4
//! for XHTML → plain text. Each spine document becomes one "page".

use std::path::Path;

use rbook::Epub;
use serde_json::json;
use xbench::{Extracted, Failure};

fn extract(path: &Path) -> Result<Extracted, Failure> {
    let epub = Epub::open(path).map_err(|e| {
        let msg = format!("{e}");
        if msg.to_lowercase().contains("encrypt") {
            Failure::Encrypted(msg)
        } else {
            Failure::Malformed(msg)
        }
    })?;
    // DRM: an encryption.xml that covers content documents.
    let drm = epub.read_resource_bytes("/META-INF/encryption.xml").is_ok();
    let mut pages = Vec::new();
    for item in epub.reader() {
        let data = item.map_err(|e| Failure::Malformed(format!("{e}")))?;
        let html = data.content();
        let text = html2text::config::plain_no_decorate()
            .string_from_read(html.as_bytes(), 100_000)
            .map_err(|e| Failure::Malformed(format!("html2text: {e}")))?;
        pages.push(text);
    }
    if drm {
        return Err(Failure::Encrypted(
            "META-INF/encryption.xml present (DRM)".into(),
        ));
    }
    let title = epub.metadata().title().map(|t| t.value().to_string());
    Ok(Extracted {
        meta: json!({ "spineDocuments": pages.len(), "title": title }),
        pages,
    })
}

fn main() {
    xbench::main_with("rbook", "rbook 0.7.12 + html2text 0.17.4", extract);
}
