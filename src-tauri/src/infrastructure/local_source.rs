//! Local filesystem implementation of [`InventorySource`].
//!
//! Lightweight metadata only: entries come from `read_dir`, kinds from
//! `DirEntry::file_type` (which does not follow symlinks), and size and
//! modification time from `DirEntry::metadata` (also not following links).
//! File contents are never opened.
//!
//! Cloud-offloaded files (ADR 0005):
//! - **macOS dataless files** (iCloud Drive with "Optimize Mac Storage",
//!   and other File Provider clients) keep their real name and size; the
//!   `SF_DATALESS` flag marks them cloud-only. Reading metadata does not
//!   download them.
//! - **Legacy iCloud placeholders** named `.Name.pdf.icloud` are reported as
//!   `Name.pdf`, with the size from the placeholder's property list, so the
//!   catalog entry survives the file being downloaded or evicted again.
//! - **Dataless folders** may need iCloud to supply their listing. That
//!   listing is opened with a timeout so an offline or slow service cannot
//!   stall the scan; a timeout is an ordinary, retried directory failure.
//! - **Windows cloud placeholders** (OneDrive Files On-Demand, iCloud for
//!   Windows) carry offline or recall-on-access attributes.

use std::ffi::OsString;
use std::fs::{self, DirEntry};
use std::io;
use std::path::Path;
use std::time::Duration;

use crate::domain::traits::{DirEntries, DirEntryInfo, EntryKind, InventorySource};

/// Upper bound on waiting for a cloud service to list an offloaded folder.
pub const CLOUD_LISTING_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Default, Clone, Copy)]
pub struct LocalFsSource;

impl InventorySource for LocalFsSource {
    fn read_dir(&self, dir: &Path) -> io::Result<DirEntries<'_>> {
        let entries = platform::open_dir(dir)?;
        Ok(Box::new(entries.filter_map(|entry| match entry {
            Ok(entry) => describe(&entry),
            Err(err) => Some(Err(err)),
        })))
    }
}

/// `None` for entries that should be invisible to the scanner (legacy
/// placeholders for files that are not themselves placeholders are kept).
fn describe(entry: &DirEntry) -> Option<io::Result<DirEntryInfo>> {
    let file_type = match entry.file_type() {
        Ok(t) => t,
        Err(err) => return Some(Err(err)),
    };
    let kind = if file_type.is_symlink() {
        EntryKind::Symlink
    } else if file_type.is_dir() {
        EntryKind::Directory
    } else if file_type.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    let mut info = DirEntryInfo {
        name: entry.file_name(),
        kind,
        size_bytes: None,
        modified: None,
        cloud_only: false,
    };
    if kind != EntryKind::File {
        return Some(Ok(info));
    }
    if let Some(real_name) = legacy_icloud_target(&info.name) {
        info.name = real_name;
        info.cloud_only = true;
        info.size_bytes = platform::placeholder_size(&entry.path());
        return Some(Ok(info));
    }
    match entry.metadata() {
        Ok(meta) => {
            info.size_bytes = Some(meta.len());
            info.modified = meta.modified().ok();
            info.cloud_only = platform::is_cloud_only(&meta);
        }
        Err(err) => return Some(Err(err)),
    }
    Some(Ok(info))
}

/// `.Name.pdf.icloud` → `Name.pdf`; anything else → `None`.
pub fn legacy_icloud_target(name: &OsString) -> Option<OsString> {
    let name = name.to_str()?;
    let inner = name.strip_prefix('.')?.strip_suffix(".icloud")?;
    (!inner.is_empty()).then(|| OsString::from(inner))
}

#[cfg(target_os = "macos")]
mod platform {
    use super::*;
    use std::os::macos::fs::MetadataExt;
    use std::sync::mpsc;

    /// `SF_DATALESS` from `<sys/stat.h>`: contents are not present locally.
    const SF_DATALESS: u32 = 0x4000_0000;

    pub fn is_cloud_only(meta: &fs::Metadata) -> bool {
        meta.st_flags() & SF_DATALESS != 0
    }

    pub fn placeholder_size(path: &Path) -> Option<u64> {
        // The placeholder is a small local property list, not the document.
        let value = plist::Value::from_file(path).ok()?;
        value
            .as_dictionary()?
            .get("NSURLFileSizeKey")?
            .as_unsigned_integer()
    }

    pub fn open_dir(dir: &Path) -> io::Result<fs::ReadDir> {
        let dataless = fs::symlink_metadata(dir).is_ok_and(|m| is_cloud_only(&m));
        if !dataless {
            return fs::read_dir(dir);
        }
        // Listing a dataless folder asks the cloud service for its contents.
        // Bound the wait; a stuck helper thread holds no Atlas lock.
        let (tx, rx) = mpsc::sync_channel(1);
        let path = dir.to_path_buf();
        std::thread::Builder::new()
            .name("atlas-cloud-listing".into())
            .spawn(move || {
                let _ = tx.send(fs::read_dir(&path));
            })?;
        rx.recv_timeout(CLOUD_LISTING_TIMEOUT).unwrap_or_else(|_| {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "cloud folder listing timed out",
            ))
        })
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_OFFLINE: u32 = 0x0000_1000;
    const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x0004_0000;
    const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;

    pub fn is_cloud_only(meta: &fs::Metadata) -> bool {
        meta.file_attributes()
            & (FILE_ATTRIBUTE_OFFLINE
                | FILE_ATTRIBUTE_RECALL_ON_OPEN
                | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS)
            != 0
    }

    pub fn placeholder_size(_path: &Path) -> Option<u64> {
        None
    }

    pub fn open_dir(dir: &Path) -> io::Result<fs::ReadDir> {
        fs::read_dir(dir)
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    use super::*;

    pub fn is_cloud_only(_meta: &fs::Metadata) -> bool {
        false
    }

    pub fn placeholder_size(_path: &Path) -> Option<u64> {
        None
    }

    pub fn open_dir(dir: &Path) -> io::Result<fs::ReadDir> {
        fs::read_dir(dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(dir: &Path) -> Vec<DirEntryInfo> {
        let mut v: Vec<_> = LocalFsSource
            .read_dir(dir)
            .unwrap()
            .map(Result::unwrap)
            .collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }

    #[test]
    fn reports_kinds_sizes_and_times_without_following_links() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("book.pdf"), b"%PDF-1.7").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.path(), dir.path().join("loop")).unwrap();

        let entries = list(dir.path());
        let book = entries.iter().find(|e| e.name == "book.pdf").unwrap();
        assert_eq!(book.kind, EntryKind::File);
        assert_eq!(book.size_bytes, Some(8));
        assert!(book.modified.is_some());
        assert!(!book.cloud_only);
        assert_eq!(
            entries.iter().find(|e| e.name == "sub").unwrap().kind,
            EntryKind::Directory
        );
        #[cfg(unix)]
        assert_eq!(
            entries.iter().find(|e| e.name == "loop").unwrap().kind,
            EntryKind::Symlink
        );
    }

    #[test]
    fn legacy_placeholder_names() {
        let t = |s: &str| legacy_icloud_target(&OsString::from(s));
        assert_eq!(t(".Cosmos.pdf.icloud"), Some(OsString::from("Cosmos.pdf")));
        assert_eq!(
            t(".a b – c.epub.icloud"),
            Some(OsString::from("a b – c.epub"))
        );
        assert_eq!(t("Cosmos.pdf.icloud"), None);
        assert_eq!(t(".icloud"), None);
        assert_eq!(t("..icloud"), None);
        assert_eq!(t(".hidden.pdf"), None);
    }

    /// A legacy iCloud placeholder is reported as the file it stands for.
    #[cfg(target_os = "macos")]
    #[test]
    fn legacy_placeholder_is_reported_as_the_real_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut dict = plist::Dictionary::new();
        dict.insert("NSURLNameKey".into(), "Cosmos.pdf".into());
        dict.insert(
            "NSURLFileSizeKey".into(),
            plist::Value::Integer(48_213_004u64.into()),
        );
        dict.insert(
            "NSURLFileResourceTypeKey".into(),
            "NSURLFileResourceTypeRegular".into(),
        );
        plist::Value::Dictionary(dict)
            .to_file_binary(dir.path().join(".Cosmos.pdf.icloud"))
            .unwrap();

        let entries = list(dir.path());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "Cosmos.pdf");
        assert_eq!(entries[0].kind, EntryKind::File);
        assert_eq!(entries[0].size_bytes, Some(48_213_004));
        assert!(entries[0].cloud_only);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn unreadable_placeholder_has_unknown_size() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".Broken.epub.icloud"), b"not a plist").unwrap();
        let entries = list(dir.path());
        assert_eq!(entries[0].name, "Broken.epub");
        assert_eq!(entries[0].size_bytes, None);
        assert!(entries[0].cloud_only);
    }
}
