//! Opening a library file read-only and hashing it (phase1-spec §7.2).
//!
//! A cloud-only file is never opened for reading: on Windows the file is
//! opened with `FILE_FLAG_OPEN_NO_RECALL`, and on every OS a file whose
//! metadata says its contents are not local (iCloud dataless, Windows
//! placeholders) is refused before any byte is read.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::infrastructure::local_source::is_cloud_only;

/// What the worker reports for a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hashed {
    /// BLAKE3-256, lowercase hex.
    pub hash: String,
    pub size: u64,
    pub mtime_secs: Option<i64>,
}

/// Why a file could not be hashed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HashError {
    /// Its contents are only in the cloud; nothing was read.
    CloudOnly,
    /// Permission denied.
    Unreadable,
    /// Missing, not a regular file, or a read failed.
    Failed,
}

/// Opens a library file for reading without recalling cloud content.
pub fn open(path: &Path) -> Result<File, HashError> {
    let meta = std::fs::metadata(path).map_err(classify)?;
    if !meta.is_file() {
        return Err(HashError::Failed);
    }
    if is_cloud_only(&meta) {
        return Err(HashError::CloudOnly);
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        /// Never recall an offloaded file's contents (winbase.h).
        const FILE_FLAG_OPEN_NO_RECALL: u32 = 0x0010_0000;
        options.custom_flags(FILE_FLAG_OPEN_NO_RECALL);
    }
    options.open(path).map_err(classify)
}

fn classify(err: io::Error) -> HashError {
    match err.kind() {
        io::ErrorKind::PermissionDenied => HashError::Unreadable,
        _ => HashError::Failed,
    }
}

/// Streams the file through BLAKE3. The caller seeks back before parsing.
pub fn hash(file: &mut File) -> Result<Hashed, HashError> {
    let meta = file.metadata().map_err(classify)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut size = 0u64;
    loop {
        let n = file.read(&mut buf).map_err(classify)?;
        if n == 0 {
            break;
        }
        size += n as u64;
        hasher.update(&buf[..n]);
    }
    let mtime_secs = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_secs()).ok());
    Ok(Hashed {
        hash: hasher.finalize().to_hex().to_string(),
        size,
        mtime_secs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_match_blake3_and_report_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.pdf");
        std::fs::write(&path, b"%PDF-1.4 hello").unwrap();
        let mut f = open(&path).unwrap();
        let h = hash(&mut f).unwrap();
        assert_eq!(h.size, 14);
        assert_eq!(h.hash, blake3::hash(b"%PDF-1.4 hello").to_hex().to_string());
        assert!(h.mtime_secs.is_some());
    }

    #[test]
    fn missing_files_and_folders_are_not_hashed() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            open(&dir.path().join("nope.pdf")).unwrap_err(),
            HashError::Failed
        );
        assert_eq!(open(dir.path()).unwrap_err(), HashError::Failed);
    }
}
