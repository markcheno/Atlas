//! Where an export may write (Slice 3.8, phase3-spec §7.7, ADR 0019).
//!
//! The one place Atlas writes outside its data root:
//! - only inside the `Atlas` folder it makes in a folder the user chose in
//!   a native dialog;
//! - never in, or above, a library folder, nor inside the data root other
//!   than its `exports` folder;
//! - an existing `Atlas` (or subfolder) that is a file or a link is
//!   refused;
//! - a file is replaced only when its front matter carries the same
//!   `atlas_id`; anything else there is left alone and the note goes beside
//!   it with a number. Nothing is ever deleted.

use std::collections::HashSet;
use std::fs;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};

use crate::domain::dto::{ExportFormat, ExportReport};
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::fsutil::sync_dir;
use crate::infrastructure::native_path;
use crate::services::sources::{Relation, relation};

/// The folder Atlas makes in the chosen one.
pub const ATLAS: &str = "Atlas";
/// The data root's own export folder, the one place inside it allowed.
pub const EXPORTS: &str = "exports";
/// A name is cut to this many characters before its number and extension.
pub const MAX_NAME_CHARS: usize = 100;
/// How far into a file Atlas looks for its front matter.
const FRONT_MATTER_BYTES: u64 = 4096;
/// Numbered names tried before giving up.
const MAX_TRIES: u32 = 10_000;

const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// A file name safe on macOS, Windows and Linux, and inside an Obsidian
/// link: forbidden and link characters become spaces, a leading dot and
/// trailing dots and spaces go, reserved Windows names get `_`, and the
/// name is cut to [`MAX_NAME_CHARS`].
pub fn safe_name(text: &str) -> String {
    let mapped: String = text
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' | '#' | '[' | ']' | '^' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let collapsed = mapped.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut name: String = collapsed
        .trim_start_matches(['.', ' '])
        .chars()
        .take(MAX_NAME_CHARS)
        .collect();
    while name.ends_with(['.', ' ']) {
        name.pop();
    }
    if name.is_empty() {
        return "Untitled".to_owned();
    }
    let stem = name.split('.').next().unwrap_or_default();
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(stem)) {
        name.insert(stem.len(), '_');
    }
    name
}

/// The `atlas_id` in a file's front matter, if it starts with one: the
/// only proof a file is Atlas's own note from an earlier export.
pub fn front_matter_id(path: &Path) -> Option<String> {
    let mut head = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(FRONT_MATTER_BYTES)
        .read_to_end(&mut head)
        .ok()?;
    let text = String::from_utf8_lossy(&head);
    let mut lines = text.lines();
    if lines.next()?.trim_end() != "---" {
        return None;
    }
    for line in lines {
        let line = line.trim_end();
        if line == "---" {
            return None;
        }
        if let Some(value) = line.strip_prefix("atlas_id:") {
            return unquote(value.trim());
        }
    }
    None
}

fn unquote(value: &str) -> Option<String> {
    let inner = value.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                other => out.push(other),
            }
        } else {
            out.push(c);
        }
    }
    Some(out)
}

/// A name taken for one file of this export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub path: PathBuf,
    /// The subfolder (`Trails`, …, or empty), for links.
    pub folder: &'static str,
    /// The name without its extension, for links.
    pub name: String,
    /// It replaces Atlas's own note from an earlier export.
    pub replaces: bool,
    /// A file not Atlas's held the first name: this one is numbered.
    pub beside: bool,
}

/// The checked `Atlas` folder of one export, and what it has written.
pub struct ExportDest {
    atlas: PathBuf,
    used: HashSet<(&'static str, String)>,
    dirs: HashSet<PathBuf>,
    written: u32,
    replaced: u32,
    besides: u32,
}

fn is_inside_or_same(r: Relation) -> bool {
    matches!(r, Relation::Same | Relation::Inside)
}

/// A folder that must be a real folder, made if missing. A file or a link
/// in its place is refused.
fn real_folder(path: &Path) -> AtlasResult<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
            Err(AtlasError::ExportFolderUnusable)
        }
        Ok(_) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => match fs::create_dir(path) {
            Ok(()) => Ok(()),
            // Made meanwhile: check what it is.
            Err(e) if e.kind() == ErrorKind::AlreadyExists => real_folder_exists(path),
            Err(e) => Err(AtlasError::io("make the export folder")(e)),
        },
        Err(e) => Err(AtlasError::io("check the export folder")(e)),
    }
}

fn real_folder_exists(path: &Path) -> AtlasResult<()> {
    let meta = fs::symlink_metadata(path).map_err(AtlasError::io("check the export folder"))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(AtlasError::ExportFolderUnusable);
    }
    Ok(())
}

impl ExportDest {
    /// Checks `chosen` (from the native dialog) against the data root and
    /// the library folders, then makes or checks `chosen/Atlas`.
    pub fn prepare(chosen: &Path, data_root: &Path, sources: &[PathBuf]) -> AtlasResult<Self> {
        let chosen = dunce::canonicalize(chosen).map_err(|_| AtlasError::InvalidPath)?;
        if !chosen.is_dir() {
            return Err(AtlasError::InvalidPath);
        }
        let atlas = chosen.join(ATLAS);
        for source in sources {
            if is_inside_or_same(relation(&chosen, source))
                || !matches!(relation(&atlas, source), Relation::Unrelated)
            {
                return Err(AtlasError::ExportInsideLibrary);
            }
        }
        let data_root = dunce::canonicalize(data_root).unwrap_or_else(|_| data_root.to_path_buf());
        if is_inside_or_same(relation(&chosen, &data_root))
            && !is_inside_or_same(relation(&chosen, &data_root.join(EXPORTS)))
        {
            return Err(AtlasError::ExportInsideDataRoot);
        }
        if matches!(
            relation(&atlas, &data_root),
            Relation::Same | Relation::Contains
        ) {
            return Err(AtlasError::ExportInsideDataRoot);
        }
        real_folder(&atlas)?;
        Ok(Self {
            atlas,
            used: HashSet::new(),
            dirs: HashSet::new(),
            written: 0,
            replaced: 0,
            besides: 0,
        })
    }

    pub fn atlas(&self) -> &Path {
        &self.atlas
    }

    /// Takes a name for a file in `folder` (`""` for `Atlas` itself):
    /// `base.ext`, or Atlas's own earlier note with the same `atlas_id`, or
    /// else `base 2.ext`, `base 3.ext`… beside whatever holds the name.
    /// Names are compared case-insensitively within the export.
    pub fn claim(
        &mut self,
        folder: &'static str,
        base: &str,
        ext: &str,
        atlas_id: Option<&str>,
    ) -> AtlasResult<Claim> {
        let dir = if folder.is_empty() {
            self.atlas.clone()
        } else {
            let dir = self.atlas.join(folder);
            real_folder(&dir)?;
            dir
        };
        let mut beside = false;
        for n in 1..=MAX_TRIES {
            let name = if n == 1 {
                base.to_owned()
            } else {
                format!("{base} {n}")
            };
            let key = (folder, format!("{name}.{ext}").to_lowercase());
            if self.used.contains(&key) {
                continue;
            }
            let path = dir.join(format!("{name}.{ext}"));
            let replaces = match fs::symlink_metadata(&path) {
                Err(e) if e.kind() == ErrorKind::NotFound => false,
                Ok(meta)
                    if meta.is_file()
                        && !meta.file_type().is_symlink()
                        && atlas_id.is_some()
                        && front_matter_id(&path).as_deref() == atlas_id =>
                {
                    true
                }
                // Someone else's file, a link, a folder: left alone.
                Ok(_) => {
                    beside = true;
                    continue;
                }
                Err(e) => return Err(AtlasError::io("check the export folder")(e)),
            };
            self.used.insert(key);
            return Ok(Claim {
                path,
                folder,
                name,
                replaces,
                beside,
            });
        }
        Err(AtlasError::Internal("no free name for an exported note"))
    }

    /// Writes a claimed file: staged beside it under a temporary name, then
    /// renamed into place, so a reader never sees half a note. A rename
    /// replaces a directory entry; it never writes through a link. Notes
    /// are copies that can be written again, so each isn't flushed to disk
    /// on its own (1,000 of them took 8 s that way on macOS); [`finish`]
    /// flushes the folders once.
    ///
    /// [`finish`]: Self::finish
    pub fn write(&mut self, claim: &Claim, bytes: &[u8]) -> AtlasResult<()> {
        let dir = claim.path.parent().unwrap_or(&self.atlas);
        let name = claim
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let staged = dir.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple()));
        let written = (|| -> std::io::Result<()> {
            let mut file = fs::File::create_new(&staged)?;
            std::io::Write::write_all(&mut file, bytes)?;
            drop(file);
            fs::rename(&staged, &claim.path)
        })();
        if let Err(e) = written {
            let _ = fs::remove_file(&staged);
            return Err(AtlasError::io("write the export")(e));
        }
        self.dirs.insert(dir.to_path_buf());
        if claim.replaces {
            self.replaced += 1;
        } else {
            self.written += 1;
        }
        if claim.beside {
            self.besides += 1;
        }
        Ok(())
    }

    /// Flushes the folders written to, once each.
    pub fn finish(&self) -> AtlasResult<()> {
        for dir in &self.dirs {
            sync_dir(dir).map_err(AtlasError::io("write the export"))?;
        }
        Ok(())
    }

    pub fn report(&self, format: ExportFormat) -> ExportReport {
        ExportReport {
            format,
            folder: native_path::display(&self.atlas),
            written: self.written,
            replaced: self.replaced,
            besides: self.besides,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_safe_everywhere() {
        assert_eq!(safe_name("CON"), "CON_");
        assert_eq!(safe_name("con.txt"), "con_.txt");
        assert_eq!(safe_name("Lpt9"), "Lpt9_");
        assert_eq!(safe_name("CONSOLE"), "CONSOLE");
        assert_eq!(safe_name("a/b:c?"), "a b c");
        assert_eq!(safe_name("  ..hidden.  "), "hidden");
        assert_eq!(safe_name("x\u{7}y\nz"), "x y z");
        assert_eq!(safe_name("[[link]] #tag | ^b"), "link tag b");
        assert_eq!(safe_name("..."), "Untitled");
        assert_eq!(safe_name(""), "Untitled");
        let long = safe_name(&"é".repeat(300));
        assert_eq!(long.chars().count(), MAX_NAME_CHARS);
    }

    #[test]
    fn only_atlas_front_matter_names_a_note() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("n.md");
        fs::write(
            &p,
            "---\natlas_id: \"abc\"\natlas_kind: \"trail\"\n---\n# x\n",
        )
        .unwrap();
        assert_eq!(front_matter_id(&p).as_deref(), Some("abc"));
        fs::write(&p, "# x\natlas_id: \"abc\"\n").unwrap();
        assert_eq!(front_matter_id(&p), None);
        fs::write(&p, "---\ntitle: x\n---\natlas_id: \"abc\"\n").unwrap();
        assert_eq!(front_matter_id(&p), None);
        fs::write(&p, "---\natlas_id: \"a\\\"b\"\n---\n").unwrap();
        assert_eq!(front_matter_id(&p).as_deref(), Some("a\"b"));
        assert_eq!(front_matter_id(&dir.path().join("none.md")), None);
    }

    #[test]
    fn a_name_is_reused_only_for_the_same_note() {
        let dir = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let mut dest = ExportDest::prepare(dir.path(), data.path(), &[]).unwrap();
        let first = dest.claim("Trails", "Same", "md", Some("one")).unwrap();
        assert!(!first.replaces && !first.beside);
        dest.write(&first, b"---\natlas_id: \"one\"\n---\n")
            .unwrap();
        // Within one export, a second note of that name is numbered.
        let second = dest.claim("Trails", "same", "md", Some("two")).unwrap();
        assert_eq!(second.name, "same 2");
        // A later export: the same note replaces its file; another doesn't.
        let mut again = ExportDest::prepare(dir.path(), data.path(), &[]).unwrap();
        let mine = again.claim("Trails", "Same", "md", Some("one")).unwrap();
        assert!(mine.replaces && mine.name == "Same");
        let mut third = ExportDest::prepare(dir.path(), data.path(), &[]).unwrap();
        let other = third.claim("Trails", "Same", "md", Some("three")).unwrap();
        assert!(other.beside && !other.replaces && other.name == "Same 2");
    }
}
