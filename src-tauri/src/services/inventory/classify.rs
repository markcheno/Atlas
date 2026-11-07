//! File classification for Phase 0: by extension only, case-insensitively.
//!
//! The spec allows a lightweight signature check "where appropriate". It is
//! deferred to Phase 1 extraction: reading even a few bytes would open every
//! file, and for cloud-offloaded files it would trigger a download.

use std::ffi::OsStr;
use std::path::Path;

use crate::domain::dto::FileType;

pub fn file_type(name: &OsStr) -> Option<(FileType, String)> {
    let ext = Path::new(name).extension()?.to_str()?.to_ascii_lowercase();
    let kind = match ext.as_str() {
        "pdf" => FileType::Pdf,
        "epub" => FileType::Epub,
        _ => return None,
    };
    Some((kind, ext))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_are_case_insensitive_and_exact() {
        let t = |s: &str| file_type(OsStr::new(s)).map(|(k, _)| k);
        assert_eq!(t("a.pdf"), Some(FileType::Pdf));
        assert_eq!(t("B.PDF"), Some(FileType::Pdf));
        assert_eq!(t("c.Epub"), Some(FileType::Epub));
        assert_eq!(t("notes.pdf.txt"), None);
        assert_eq!(t("pdf"), None);
        assert_eq!(t(".pdf"), None);
        assert_eq!(t("archive.epub.zip"), None);
    }
}
