//! Which files are one book's two formats by name (Slice 4.5.1, ADR 0023):
//! a PDF and an EPUB in the same folder whose names, without the extension,
//! fold to the same text (case, accents and macOS's decomposed names don't
//! matter, as in the Library's own search keys).

use std::collections::HashMap;
use std::path::Path;

use crate::infrastructure::native_path;
use crate::services::library::fold::fold;

/// A file as the pairing pass sees it.
#[derive(Debug, Clone)]
pub struct NamedFile {
    pub id: String,
    /// The path within its folder, as stored (`native_path::encode`).
    pub rel_path: Vec<u8>,
    /// `pdf` or `epub`.
    pub file_type: String,
}

/// The folder (stored encoding) and the folded name without its extension.
/// None for a path that can't be decoded or has no name.
pub fn key(rel_path: &[u8]) -> Option<(Vec<u8>, String)> {
    let path = native_path::decode(rel_path)?;
    let stem = path.file_stem()?.to_string_lossy();
    let stem = fold(&stem).trim().to_owned();
    if stem.is_empty() {
        return None;
    }
    let parent = path.parent().unwrap_or(Path::new(""));
    Some((native_path::encode(parent), stem))
}

/// The (PDF, EPUB) file pairs among one folder's files. A name that holds
/// anything but exactly one PDF and one EPUB (two PDFs that differ only in
/// case, say) pairs nothing: Atlas doesn't guess.
pub fn pairs(files: &[NamedFile]) -> Vec<(String, String)> {
    // (folder, name) → (PDFs, EPUBs)
    type Group<'a> = (Vec<&'a str>, Vec<&'a str>);
    let mut groups: HashMap<(Vec<u8>, String), Group<'_>> = HashMap::new();
    for f in files {
        let Some(k) = key(&f.rel_path) else {
            continue;
        };
        let group = groups.entry(k).or_default();
        match f.file_type.as_str() {
            "pdf" => group.0.push(&f.id),
            "epub" => group.1.push(&f.id),
            _ => {}
        }
    }
    let mut out: Vec<(String, String)> = groups
        .into_values()
        .filter_map(|(pdfs, epubs)| match (pdfs.as_slice(), epubs.as_slice()) {
            ([pdf], [epub]) => Some(((*pdf).to_owned(), (*epub).to_owned())),
            _ => None,
        })
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn file(id: &str, rel: &str, kind: &str) -> NamedFile {
        let path: PathBuf = rel.split('/').collect();
        NamedFile {
            id: id.into(),
            rel_path: native_path::encode(&path),
            file_type: kind.into(),
        }
    }

    #[test]
    fn a_pdf_and_an_epub_with_one_name_in_one_folder_pair() {
        let files = [
            file("p", "Sci-Fi/Dune.pdf", "pdf"),
            file("e", "Sci-Fi/Dune.epub", "epub"),
            file("q", "Sci-Fi/Emma.pdf", "pdf"),
        ];
        assert_eq!(pairs(&files), vec![("p".into(), "e".into())]);
    }

    #[test]
    fn case_accents_and_decomposed_names_fold_to_one_name() {
        let files = [
            file("p", "Café.PDF", "pdf"),
            // "cafe" with the accent as a separate mark, as macOS stores it.
            file("e", "cafe\u{301}.epub", "epub"),
        ];
        assert_eq!(pairs(&files), vec![("p".into(), "e".into())]);
        let files = [file("p", "Café.pdf", "pdf"), file("e", "cafe.epub", "epub")];
        assert_eq!(pairs(&files).len(), 1, "accents fold away");
    }

    #[test]
    fn different_folders_or_an_ambiguous_name_pair_nothing() {
        let files = [file("p", "A/x.pdf", "pdf"), file("e", "B/x.epub", "epub")];
        assert!(pairs(&files).is_empty());
        let files = [
            file("p1", "Dune.pdf", "pdf"),
            file("p2", "DUNE.pdf", "pdf"),
            file("e", "Dune.epub", "epub"),
        ];
        assert!(pairs(&files).is_empty(), "two PDFs for one name: no guess");
    }

    #[test]
    fn the_folder_is_part_of_the_key() {
        assert_eq!(
            key(&native_path::encode(Path::new("Dune.pdf"))).unwrap().1,
            "dune"
        );
        let (a, _) = key(&native_path::encode(
            &["A", "x.pdf"].iter().collect::<PathBuf>(),
        ))
        .unwrap();
        let (b, _) = key(&native_path::encode(
            &["B", "x.pdf"].iter().collect::<PathBuf>(),
        ))
        .unwrap();
        assert_ne!(a, b);
    }
}
