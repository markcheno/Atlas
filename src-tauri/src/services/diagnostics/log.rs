//! The log's files (Slice 4.1, phase4-spec §7.1): JSON lines in
//! `logs/atlas-<UTC date>.jsonl`, a new file at each UTC day and when one
//! passes its size, and the folder kept under its cap by removing the
//! oldest. Only files whose names follow that pattern are ever read,
//! changed or removed.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Where the log lives in a library.
pub const LOGS_DIR: &str = "logs";

/// Sizes that bound the log.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// A file rolls to the next one past this size.
    pub file_bytes: u64,
    /// The folder's files together stay under this.
    pub folder_bytes: u64,
    /// Events waiting for the writer; more are dropped and counted.
    pub queue: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            file_bytes: 5 * 1024 * 1024,
            folder_bytes: 50 * 1024 * 1024,
            queue: 1024,
        }
    }
}

/// A log file's place in the order: its date, then its number that day.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LogName {
    pub date: String,
    pub n: u32,
}

impl LogName {
    /// `atlas-2026-11-12.jsonl` or `atlas-2026-11-12-3.jsonl`; anything else
    /// isn't Atlas's.
    pub fn parse(name: &str) -> Option<Self> {
        let rest = name.strip_prefix("atlas-")?.strip_suffix(".jsonl")?;
        let (date, n) = match rest.get(10..) {
            Some("") => (rest, 0),
            Some(tail) => (rest.get(..10)?, tail.strip_prefix('-')?.parse().ok()?),
            None => return None,
        };
        let b = date.as_bytes();
        let shape = b.len() == 10
            && b[4] == b'-'
            && b[7] == b'-'
            && b.iter()
                .enumerate()
                .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit());
        (shape && (n > 0 || rest.len() == 10)).then(|| Self {
            date: date.to_owned(),
            n,
        })
    }

    pub fn file_name(&self) -> String {
        if self.n == 0 {
            format!("atlas-{}.jsonl", self.date)
        } else {
            format!("atlas-{}-{}.jsonl", self.date, self.n)
        }
    }
}

/// The log's files in `dir`, oldest first, with their sizes.
pub fn files(dir: &Path) -> Vec<(LogName, PathBuf, u64)> {
    let mut out: Vec<_> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = LogName::parse(e.file_name().to_str()?)?;
            let meta = e.metadata().ok().filter(fs::Metadata::is_file)?;
            Some((name, e.path(), meta.len()))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Writes lines into the log folder. A write that fails turns it off
/// until the library is opened again: the log never fails the work it
/// describes.
pub struct LogWriter {
    dir: PathBuf,
    limits: Limits,
    current: Option<(LogName, File, u64)>,
    broken: bool,
}

impl LogWriter {
    pub fn new(dir: PathBuf, limits: Limits) -> Self {
        let broken = fs::create_dir_all(&dir).is_err();
        Self {
            dir,
            limits,
            current: None,
            broken,
        }
    }

    pub fn is_broken(&self) -> bool {
        self.broken
    }

    /// Appends `line` (one JSON object, no newline) dated `date`.
    pub fn write(&mut self, date: &str, line: &str) {
        if self.broken {
            return;
        }
        let len = line.len() as u64 + 1;
        let fits = self.current.as_ref().is_some_and(|(name, _, size)| {
            name.date == date && size + len <= self.limits.file_bytes
        });
        if !fits && self.roll(date, len).is_err() {
            self.broken = true;
            return;
        }
        let Some((_, file, size)) = self.current.as_mut() else {
            return;
        };
        let mut bytes = Vec::with_capacity(line.len() + 1);
        bytes.extend_from_slice(line.as_bytes());
        bytes.push(b'\n');
        if file.write_all(&bytes).and_then(|()| file.flush()).is_err() {
            self.broken = true;
            self.current = None;
            return;
        }
        *size += len;
    }

    /// Opens the file the next line goes to: today's newest, if it has
    /// room, else a new one. Then keeps the folder under its cap.
    fn roll(&mut self, date: &str, len: u64) -> std::io::Result<()> {
        self.current = None;
        let today: Vec<_> = files(&self.dir)
            .into_iter()
            .filter(|(name, _, _)| name.date == date)
            .collect();
        let name = match today.last() {
            Some((name, _, size)) if size + len <= self.limits.file_bytes => name.clone(),
            Some((name, _, _)) => LogName {
                date: date.to_owned(),
                n: name.n + 1,
            },
            None => LogName {
                date: date.to_owned(),
                n: 0,
            },
        };
        let path = self.dir.join(name.file_name());
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let size = file.metadata()?.len();
        self.prune(&name);
        self.current = Some((name, file, size));
        Ok(())
    }

    /// Removes the oldest files until the folder is under its cap, never
    /// the one being written.
    fn prune(&self, keep: &LogName) {
        let all = files(&self.dir);
        let mut total: u64 = all.iter().map(|(_, _, size)| size).sum();
        for (name, path, size) in all {
            if total <= self.limits.folder_bytes {
                break;
            }
            if &name != keep && fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(size);
            }
        }
    }

    /// Removes every log file. The next line starts a new one.
    pub fn clear(&mut self) {
        self.current = None;
        for (_, path, _) in files(&self.dir) {
            let _ = fs::remove_file(path);
        }
        self.broken = fs::create_dir_all(&self.dir).is_err();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_atlas_names_are_the_log() {
        assert_eq!(
            LogName::parse("atlas-2026-11-12.jsonl"),
            Some(LogName {
                date: "2026-11-12".into(),
                n: 0
            })
        );
        assert_eq!(
            LogName::parse("atlas-2026-11-12-3.jsonl").map(|n| n.n),
            Some(3)
        );
        for other in [
            "atlas-2026-11-12-0.jsonl",
            "atlas-2026-1-12.jsonl",
            "atlas-2026-11-12.json",
            "notes.jsonl",
            "atlas-2026-11-12-x.jsonl",
        ] {
            assert_eq!(LogName::parse(other), None, "{other}");
        }
        let name = LogName::parse("atlas-2026-11-12-3.jsonl").unwrap();
        assert_eq!(name.file_name(), "atlas-2026-11-12-3.jsonl");
    }

    #[test]
    fn files_roll_by_size_and_day_and_the_oldest_go() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("logs");
        fs::create_dir_all(&logs).unwrap();
        fs::write(logs.join("mine.txt"), "not Atlas's").unwrap();
        let limits = Limits {
            file_bytes: 100,
            folder_bytes: 350,
            queue: 8,
        };
        let mut w = LogWriter::new(logs.clone(), limits);
        let line = "x".repeat(39); // 40 bytes with its newline
        for _ in 0..5 {
            w.write("2026-11-11", &line);
        }
        let names: Vec<_> = files(&logs).iter().map(|f| f.0.file_name()).collect();
        assert_eq!(
            names,
            [
                "atlas-2026-11-11.jsonl",
                "atlas-2026-11-11-1.jsonl",
                "atlas-2026-11-11-2.jsonl"
            ]
        );
        for _ in 0..8 {
            w.write("2026-11-12", &line);
        }
        let all = files(&logs);
        let total: u64 = all.iter().map(|f| f.2).sum();
        assert!(total <= 350 + 40, "{total}");
        assert_eq!(
            all.last().unwrap().0.file_name(),
            "atlas-2026-11-12-3.jsonl"
        );
        assert_eq!(
            all.first().unwrap().0.date,
            "2026-11-11",
            "pruned oldest first"
        );
        assert!(logs.join("mine.txt").exists(), "never someone else's file");

        w.clear();
        assert!(files(&logs).is_empty());
        assert!(logs.join("mine.txt").exists());
        w.write("2026-11-12", "after");
        assert_eq!(files(&logs).len(), 1);
    }
}
