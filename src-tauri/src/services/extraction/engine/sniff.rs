//! What a file really is, from its first bytes (phase1-spec §7.2). A
//! signature that contradicts the extension is `unsupported`, so a text file
//! renamed `.pdf` never reaches PDFium.

use std::io::{Read, Seek, SeekFrom};

use super::{Failure, Outcome};
use crate::domain::dto::FileType;

const PDF_MAGIC: &[u8] = b"%PDF-";
/// PDF readers accept the header anywhere in the first kilobyte.
const PDF_WINDOW: usize = 1024;
const ZIP_MAGIC: &[u8] = b"PK\x03\x04";

/// Checks the signature against the cataloged type, then rewinds.
pub fn check<R: Read + Seek>(file: &mut R, expected: FileType) -> Result<(), Failure> {
    let mut head = Vec::with_capacity(PDF_WINDOW);
    file.by_ref()
        .take(PDF_WINDOW as u64)
        .read_to_end(&mut head)
        .map_err(|_| Failure::new(Outcome::Malformed, "read_failed"))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|_| Failure::new(Outcome::Malformed, "read_failed"))?;
    match expected {
        FileType::Pdf if head.windows(PDF_MAGIC.len()).any(|w| w == PDF_MAGIC) => Ok(()),
        FileType::Pdf => Err(Failure::new(Outcome::Unsupported, "not_a_pdf")),
        // The EPUB pre-scan checks the `mimetype` entry; here, only that it
        // is a zip.
        FileType::Epub if head.starts_with(ZIP_MAGIC) => Ok(()),
        FileType::Epub => Err(Failure::new(Outcome::Unsupported, "not_an_epub")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn sniff(bytes: &[u8], t: FileType) -> Result<(), Failure> {
        let mut c = Cursor::new(bytes.to_vec());
        let r = check(&mut c, t);
        assert_eq!(c.position(), 0, "rewound");
        r
    }

    #[test]
    fn signatures_must_match_the_extension() {
        assert!(sniff(b"%PDF-1.7\n...", FileType::Pdf).is_ok());
        let mut late = vec![b' '; 500];
        late.extend_from_slice(b"%PDF-1.4");
        assert!(sniff(&late, FileType::Pdf).is_ok());
        assert_eq!(
            sniff(b"just text", FileType::Pdf).unwrap_err().reason,
            "not_a_pdf"
        );
        assert_eq!(sniff(b"", FileType::Pdf).unwrap_err().reason, "not_a_pdf");
        assert!(sniff(b"PK\x03\x04rest", FileType::Epub).is_ok());
        assert_eq!(
            sniff(b"%PDF-1.4", FileType::Epub).unwrap_err().reason,
            "not_an_epub"
        );
    }
}
