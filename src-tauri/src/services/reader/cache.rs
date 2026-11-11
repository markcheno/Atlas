//! The page cache (phase3-spec §7.5, Slice 3.7):
//! `cache/previews/<content hash>/<page>-<width>-<find key>.png`, with a
//! `.json` beside it holding the page count, the size and where the
//! passage was. Keyed by content, so a moved or copied file's pages are
//! reused. At most a cap's worth of files, the least recently used first
//! to go. Clean caches empties it.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::services::extraction::engine::render::normalized;
use crate::services::extraction::protocol::RenderRect;

/// What is kept beside a page's PNG.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    pub page_count: u32,
    pub width: u32,
    pub height: u32,
    pub rects: Vec<RenderRect>,
}

/// Which outline a cached page has: 8 hex characters of the normalised
/// text looked for, or `none`.
pub fn find_key(find: Option<&str>) -> String {
    match find.map(normalized).filter(|f| !f.is_empty()) {
        Some(f) => blake3::hash(f.as_bytes()).to_hex()[..8].to_owned(),
        None => "none".to_owned(),
    }
}

fn paths(dir: &Path, hash: &str, page: u32, width: u32, key: &str) -> (PathBuf, PathBuf) {
    let base = dir.join(hash);
    (
        base.join(format!("{page}-{width}-{key}.png")),
        base.join(format!("{page}-{width}-{key}.json")),
    )
}

fn touch(path: &Path) {
    if let Ok(f) = fs::File::options().write(true).open(path) {
        let _ = f.set_modified(SystemTime::now());
    }
}

/// A cached page, marked as just used.
pub fn get(dir: &Path, hash: &str, page: u32, width: u32, key: &str) -> Option<(Vec<u8>, Meta)> {
    let (png, json) = paths(dir, hash, page, width, key);
    let meta: Meta = serde_json::from_slice(&fs::read(&json).ok()?).ok()?;
    let bytes = fs::read(&png).ok()?;
    touch(&png);
    Some((bytes, meta))
}

/// Keeps a page: written beside, then renamed into place.
pub fn put(
    dir: &Path,
    hash: &str,
    page: u32,
    width: u32,
    key: &str,
    png: &[u8],
    meta: &Meta,
) -> io::Result<()> {
    let (png_path, json_path) = paths(dir, hash, page, width, key);
    let folder = png_path.parent().unwrap_or(dir);
    fs::create_dir_all(folder)?;
    let write = |path: &Path, bytes: &[u8]| -> io::Result<()> {
        let staged = path.with_extension("part");
        fs::write(&staged, bytes)?;
        fs::rename(&staged, path)
    };
    write(
        &json_path,
        &serde_json::to_vec(meta).map_err(io::Error::other)?,
    )?;
    write(&png_path, png)
}

/// Removes the least recently used pages until the cache holds at most
/// `cap` bytes. Returns how many pages went.
pub fn prune(dir: &Path, cap: u64) -> usize {
    let mut pages: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
    let Ok(folders) = fs::read_dir(dir) else {
        return 0;
    };
    for folder in folders.flatten().filter(|e| e.path().is_dir()) {
        let Ok(files) = fs::read_dir(folder.path()) else {
            continue;
        };
        for f in files.flatten() {
            let path = f.path();
            if path.extension().is_some_and(|e| e == "png")
                && let Ok(meta) = f.metadata()
            {
                let json = fs::metadata(path.with_extension("json")).map_or(0, |m| m.len());
                let used = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                pages.push((used, meta.len() + json, path));
            }
        }
    }
    let mut total: u64 = pages.iter().map(|p| p.1).sum();
    if total <= cap {
        return 0;
    }
    pages.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.2.cmp(&b.2)));
    let mut gone = 0;
    for (_, bytes, path) in pages {
        if total <= cap {
            break;
        }
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("json"));
        if let Some(folder) = path.parent() {
            // An empty folder goes too; one still holding pages stays.
            let _ = fs::remove_dir(folder);
        }
        total = total.saturating_sub(bytes);
        gone += 1;
    }
    gone
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> Meta {
        Meta {
            page_count: 2,
            width: 10,
            height: 20,
            rects: vec![RenderRect {
                x: 1,
                y: 2,
                width: 3,
                height: 4,
            }],
        }
    }

    #[test]
    fn pages_are_kept_by_content_page_width_and_what_was_looked_for() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(find_key(None), "none");
        assert_eq!(find_key(Some("  ")), "none");
        assert_eq!(find_key(Some("a  b")), find_key(Some("a b")));
        assert_ne!(find_key(Some("a b")), find_key(Some("a c")));
        let key = find_key(Some("marker"));
        put(dir.path(), "ab12", 1, 800, &key, b"png", &meta()).unwrap();
        assert_eq!(
            get(dir.path(), "ab12", 1, 800, &key),
            Some((b"png".to_vec(), meta()))
        );
        assert!(get(dir.path(), "ab12", 1, 1200, &key).is_none());
        assert!(get(dir.path(), "ab12", 1, 800, "none").is_none());
    }

    #[test]
    fn the_least_recently_used_pages_go_first() {
        let dir = tempfile::tempdir().unwrap();
        let big = vec![0u8; 1000];
        for page in 1..=4 {
            put(dir.path(), "h", page, 800, "none", &big, &meta()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // Page 1 used again: it is now the most recent.
        assert!(get(dir.path(), "h", 1, 800, "none").is_some());
        let gone = prune(dir.path(), 2500);
        assert_eq!(gone, 2);
        assert!(get(dir.path(), "h", 1, 800, "none").is_some());
        assert!(get(dir.path(), "h", 4, 800, "none").is_some());
        assert!(get(dir.path(), "h", 2, 800, "none").is_none());
        assert_eq!(prune(dir.path(), 1_000_000), 0);
    }
}
