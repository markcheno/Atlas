//! Lossless storage of native filesystem paths (phase0-spec §6).
//!
//! Unix paths are arbitrary bytes and Windows paths are arbitrary UTF-16
//! (unpaired surrogates included), so neither may pass through a Rust
//! `String`. Stored form: one platform tag byte, then the payload.
//!
//! | Tag | Payload |
//! | --- | --- |
//! | `0x01` | Unix `OsStr` bytes |
//! | `0x02` | Windows UTF-16 code units, little-endian |
//!
//! A path tagged for another platform decodes to `None`, so a catalog
//! carried to a different OS reports its folders as unavailable instead of
//! misreading them.

use std::path::{Path, PathBuf};

const TAG_UNIX: u8 = 0x01;
const TAG_WINDOWS: u8 = 0x02;

#[cfg(unix)]
pub fn encode(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    let bytes = path.as_os_str().as_bytes();
    let mut out = Vec::with_capacity(bytes.len() + 1);
    out.push(TAG_UNIX);
    out.extend_from_slice(bytes);
    out
}

#[cfg(windows)]
pub fn encode(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    let mut out = vec![TAG_WINDOWS];
    for unit in path.as_os_str().encode_wide() {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    out
}

#[cfg(unix)]
pub fn decode(stored: &[u8]) -> Option<PathBuf> {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    match stored.split_first() {
        // An empty payload is the empty relative path (a source root itself).
        Some((&TAG_UNIX, payload)) => Some(PathBuf::from(OsStr::from_bytes(payload))),
        _ => None,
    }
}

#[cfg(windows)]
pub fn decode(stored: &[u8]) -> Option<PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    match stored.split_first() {
        Some((&TAG_WINDOWS, payload)) if payload.len() % 2 == 0 => {
            let units: Vec<u16> = payload
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            Some(PathBuf::from(OsString::from_wide(&units)))
        }
        _ => None,
    }
}

/// Human-readable rendering; invalid sequences become U+FFFD.
pub fn display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// The same path in a form that compares equal however it was produced.
///
/// On Windows a canonical path comes back plain (`C:\x`) or verbatim
/// (`\\?\C:\x`): `dunce::canonicalize` keeps the verbatim form when a name
/// isn't valid Unicode (an unpaired surrogate). Comparing the two forms
/// fails, so this rewrites the verbatim disk and UNC prefixes to their plain
/// forms. Lexical only: use it to compare paths, never to open them.
#[cfg(windows)]
pub fn comparable(path: &Path) -> PathBuf {
    use std::ffi::OsString;
    use std::path::{Component, Prefix};
    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return path.to_path_buf();
    };
    let head = match prefix.kind() {
        Prefix::VerbatimDisk(drive) => PathBuf::from(format!("{}:\\", drive as char)),
        Prefix::VerbatimUNC(server, share) => {
            let mut s = OsString::from(r"\\");
            s.push(server);
            s.push(r"\");
            s.push(share);
            s.push(r"\");
            PathBuf::from(s)
        }
        _ => return path.to_path_buf(),
    };
    let rest: PathBuf = components
        .filter(|c| !matches!(c, Component::RootDir))
        .collect();
    head.join(rest)
}

#[cfg(not(windows))]
pub fn comparable(path: &Path) -> PathBuf {
    path.to_path_buf()
}

/// Exposed for tests that simulate a catalog from another platform.
pub fn foreign_tag() -> u8 {
    if cfg!(windows) { TAG_UNIX } else { TAG_WINDOWS }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn verbatim_and_plain_paths_compare_equal() {
        let plain = Path::new(r"C:\Books\Astronomy");
        assert_eq!(comparable(Path::new(r"\\?\C:\Books\Astronomy")), plain);
        assert_eq!(comparable(plain), plain);
        assert!(comparable(Path::new(r"\\?\C:\Books\Astronomy\x.pdf")).starts_with(plain));
        assert_eq!(
            comparable(Path::new(r"\\?\UNC\server\share\Books")),
            Path::new(r"\\server\share\Books")
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn comparable_is_identity_off_windows() {
        let p = Path::new("/Users/someone/Books");
        assert_eq!(comparable(p), p);
    }

    #[test]
    fn round_trips_ordinary_paths() {
        let path = Path::new("/Users/someone/Books/Ünïcode – long name.pdf");
        assert_eq!(decode(&encode(path)).unwrap(), path);
    }

    #[cfg(unix)]
    #[test]
    fn round_trips_non_utf8_bytes() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let raw = b"/data/caf\xe9/\xff\xfe-scan";
        let path = Path::new(OsStr::from_bytes(raw));
        let stored = encode(path);
        assert_eq!(stored[0], TAG_UNIX);
        assert_eq!(&stored[1..], raw);
        assert_eq!(decode(&stored).unwrap(), path);
        assert!(display(path).contains('\u{FFFD}'));
    }

    #[cfg(windows)]
    #[test]
    fn round_trips_unpaired_surrogates() {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt;
        let units = [b'C' as u16, b':' as u16, b'\\' as u16, 0xD800, b'x' as u16];
        let path = PathBuf::from(OsString::from_wide(&units));
        assert_eq!(decode(&encode(&path)).unwrap(), path);
    }

    #[test]
    fn empty_relative_path_round_trips() {
        assert_eq!(decode(&encode(Path::new(""))).unwrap(), PathBuf::new());
    }

    #[test]
    fn foreign_or_empty_payloads_do_not_decode() {
        assert_eq!(decode(&[]), None);
        assert_eq!(decode(&[foreign_tag(), b'/', 0, b'x', 0]), None);
        assert_eq!(decode(&[0x7f, b'/']), None);
    }
}
