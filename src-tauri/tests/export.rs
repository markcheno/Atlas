//! Slice 3.8 (phase3-spec §7.7, §12): export and import.
//!
//! - P3-22: Markdown export of each kind in both link styles; names with
//!   reserved and long forms; a second export.
//! - P3-23: export destinations: inside a library folder or the data root;
//!   an `Atlas` that is a file or a link; a link planted at a note's name.
//!   Nothing outside `<chosen>/Atlas/` is ever created, changed or deleted.
//! - P3-24: JSON export, then import into an empty store and into the same
//!   one; a newer item; bad files refused whole.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use atlas_lib::domain::dto::{
    CollectionInput, CollectionItemRef, CollectionKind, ExportFormat, ExportReport, ExportScope,
    FilePlace, FindingSource, ImportCounts, LinkStyle, OpenAction, OpenPlace, OpenVia, SaveTarget,
    SavedKind, SearchIntent,
};
use atlas_lib::error::{AtlasResult, ErrorCode};
use atlas_lib::infrastructure::clock::{Clock, TestClock};
use atlas_lib::services::export::{self, Handles, import};
use atlas_lib::services::inventory::control;
use atlas_lib::services::knowledge::{Recorder, collections, rediscovery, saved};
use rusqlite::Connection;
use rusqlite::types::Value;
use time::macros::datetime;

mod common;
use common::chat::{Chat, sse, streamed};
use common::meaning::{LONG, Lib};
use common::{code, wait_until};

const TABLES: &[&str] = &[
    "trails",
    "trail_steps",
    "step_documents",
    "bookmarks",
    "findings",
    "collections",
    "collection_items",
    "dismissed_suggestions",
];

struct X {
    lib: Lib,
    clock: Arc<TestClock>,
    chat: Chat,
}

/// What [`X::populate`] made.
struct Made {
    trail: String,
    second: String,
    finding: String,
    project: String,
}

impl X {
    fn new() -> Self {
        let clock = Arc::new(TestClock::at(datetime!(2026-11-11 09:00 UTC)));
        let c = clock.clone();
        let lib = Lib::custom(&[], false, 0, &[], move |ws| {
            ws.with_clock(c as Arc<dyn Clock>)
        });
        let chat = Chat::new(&lib);
        Self { lib, clock, chat }
    }

    fn rec(&self) -> &Arc<Recorder> {
        self.lib.root().recorder()
    }

    fn later(&self) {
        self.clock.advance(time::Duration::minutes(1));
    }

    fn handles(&self) -> Handles {
        Handles::of(self.lib.root()).unwrap()
    }

    fn file_id(&self, name: &str) -> String {
        self.lib
            .catalog()
            .query_row(
                "SELECT id FROM file_instances WHERE display_name = ?1",
                [name],
                |r| r.get(0),
            )
            .unwrap_or_else(|e| {
                let names: Vec<String> = self
                    .lib
                    .catalog()
                    .prepare("SELECT display_name FROM file_instances")
                    .unwrap()
                    .query_map([], |r| r.get(0))
                    .unwrap()
                    .map(Result::unwrap)
                    .collect();
                panic!("{name}: {e}; the catalog has {names:?}")
            })
    }

    fn search(&self, words: &str) {
        let q = Lib::query(words);
        let page = self.lib.hybrid(&q);
        self.rec().search(&q, SearchIntent::Submit, &page);
    }

    fn ask(&self, question: &str, parts: &[&str]) -> String {
        self.chat
            .answer_with(&self.lib, streamed(&sse(parts), Duration::ZERO));
        let started = self.chat.started(&self.lib, question);
        let id = started.answer_id.clone();
        assert!(wait_until(LONG, || self.chat.ended(&id)));
        id
    }

    fn store(&self) -> Connection {
        Connection::open(self.lib.env.base.join("AtlasData/library/knowledge.sqlite")).unwrap()
    }

    fn changes(&self) -> i64 {
        self.store()
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM meta WHERE key = 'changes'",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn export(
        &self,
        scope: ExportScope,
        format: ExportFormat,
        style: LinkStyle,
        dir: &Path,
    ) -> AtlasResult<ExportReport> {
        export::export(&self.handles(), &scope, format, style, dir)
    }

    fn everything(&self, style: LinkStyle, dir: &Path) -> ExportReport {
        self.export(ExportScope::Everything, ExportFormat::Markdown, style, dir)
            .unwrap()
    }

    fn json(&self, dir: &Path) -> PathBuf {
        self.export(
            ExportScope::Everything,
            ExportFormat::Json,
            LinkStyle::Obsidian,
            dir,
        )
        .unwrap();
        newest_json(&dir.join("Atlas"))
    }

    /// One of everything: a trail with a search, a question, an open, a
    /// note and a step's note; a second trail of the same name; bookmarks;
    /// an answer and a passage finding; an interest and a project named
    /// `CON`; a "Not for this".
    fn populate(&self) -> Made {
        self.search("Engineering marker 1");
        let trail = self.rec().status().current_trail_id.unwrap();
        self.later();
        let answer = self.ask("Biology marker 5", &["Cells divide [1]."]);
        assert!(wait_until(LONG, || self.rec().status().step_count == 2));
        self.later();
        let book = self.file_id("book 000001.pdf");
        self.rec().open(
            &book,
            &OpenVia::Result,
            OpenAction::Details,
            OpenPlace::default(),
        );
        self.later();
        self.rec()
            .note(&trail, None, "A thought of my own")
            .unwrap();
        let first = self.rec().get(&trail).unwrap().steps[0].id.clone();
        self.rec()
            .note(&trail, Some(&first), "On the search")
            .unwrap();
        self.rec().rename(&trail, "Stoic gears: a/b?").unwrap();

        // A second trail with the same name.
        self.rec().start().unwrap();
        self.later();
        self.search("Mathematics marker 2");
        let second = self.rec().status().current_trail_id.unwrap();
        assert_ne!(second, trail);
        self.rec().rename(&second, "Stoic gears: a/b?").unwrap();

        self.later();
        let bookmark = saved::save_bookmark(
            self.rec(),
            &SaveTarget::File {
                file_id: book.clone(),
                place: FilePlace::Result,
            },
        )
        .unwrap();
        saved::set_note(self.rec(), SavedKind::Bookmark, &bookmark.id, "Read again").unwrap();
        let finding =
            saved::save_finding(self.rec(), &FindingSource::Answer { answer_id: answer }).unwrap();
        saved::save_finding(
            self.rec(),
            &FindingSource::Passage {
                target: SaveTarget::File {
                    file_id: self.file_id("book 000002.pdf"),
                    place: FilePlace::Result,
                },
            },
        )
        .unwrap();

        let input = |kind, name: &str| CollectionInput {
            id: None,
            kind,
            name: name.into(),
            description: Some("Gears\nand drives".into()),
            goal: None,
            status: None,
        };
        let interest =
            collections::save(self.rec(), &input(CollectionKind::Interest, "Mechanisms")).unwrap();
        let project = collections::save(
            self.rec(),
            &CollectionInput {
                goal: Some("A chapter".into()),
                ..input(CollectionKind::Project, "CON")
            },
        )
        .unwrap();
        let other = self.file_id("book 000004.pdf");
        for item in [
            CollectionItemRef::Trail { id: trail.clone() },
            CollectionItemRef::Bookmark {
                id: bookmark.id.clone(),
            },
            CollectionItemRef::Finding {
                id: finding.id.clone(),
            },
            CollectionItemRef::Document {
                file_id: Some(other.clone()),
                document_id: None,
            },
        ] {
            self.later();
            collections::add(self.rec(), &interest.id, &item).unwrap();
            collections::add(self.rec(), &project.id, &item).unwrap();
        }
        let doc: String = self
            .lib
            .catalog()
            .query_row(
                "SELECT document_id FROM file_instances WHERE id = ?1",
                [&other],
                |r| r.get(0),
            )
            .unwrap();
        rediscovery::dismiss(self.rec(), &interest.id, &doc).unwrap();
        Made {
            trail,
            second,
            finding: finding.id,
            project: project.id,
        }
    }

    fn rescan(&self) {
        let root = self.lib.root();
        let id: String = self
            .lib
            .catalog()
            .query_row("SELECT id FROM source_roots", [], |r| r.get(0))
            .unwrap();
        control::enqueue(root, &id).unwrap();
        assert!(wait_until(LONG, || {
            control::overview(root).is_ok_and(|o| o.active.is_none() && o.queued_sessions == 0)
        }));
    }
}

fn newest_json(atlas: &Path) -> PathBuf {
    let mut files: Vec<PathBuf> = fs::read_dir(atlas)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    files.pop().unwrap()
}

/// Every table but `meta`, each row as text, sorted.
fn dump(conn: &Connection) -> BTreeMap<&'static str, Vec<String>> {
    let mut out = BTreeMap::new();
    for table in TABLES {
        let mut stmt = conn.prepare(&format!("SELECT * FROM {table}")).unwrap();
        let n = stmt.column_count();
        let mut rows: Vec<String> = stmt
            .query_map([], |r| {
                Ok((0..n)
                    .map(|i| format!("{:?}", r.get::<_, Value>(i).unwrap()))
                    .collect::<Vec<_>>()
                    .join("|"))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        rows.sort();
        out.insert(*table, rows);
    }
    out
}

/// Every file under `root` with its bytes, but not under `skip`.
fn tree(root: &Path, skip: &[&Path]) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        // The fixture's own unreadable folder is listed, not walked.
        let Ok(entries) = fs::read_dir(&dir) else {
            out.insert(dir.clone(), b"<unreadable>".to_vec());
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if skip.iter().any(|s| p.starts_with(s)) {
                continue;
            }
            let meta = fs::symlink_metadata(&p).unwrap();
            if meta.file_type().is_symlink() {
                out.insert(
                    p.clone(),
                    fs::read_link(&p)
                        .unwrap()
                        .into_os_string()
                        .into_encoded_bytes(),
                );
            } else if meta.is_dir() {
                out.insert(p.clone(), b"<dir>".to_vec());
                stack.push(p);
            } else {
                out.insert(p.clone(), fs::read(&p).unwrap());
            }
        }
    }
    out
}

fn find(dir: &Path, name: &str) -> Option<PathBuf> {
    for e in fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.file_name().is_some_and(|n| n == name) {
            return Some(p);
        }
        if p.is_dir()
            && let Some(found) = find(&p, name)
        {
            return Some(found);
        }
    }
    None
}

fn notes(atlas: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for sub in ["Trails", "Findings", "Bookmarks", "Interests", "Projects"] {
        if let Ok(entries) = fs::read_dir(atlas.join(sub)) {
            out.extend(
                entries
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|e| e == "md")),
            );
        }
    }
    out.sort();
    out
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() + 1 {
            out.push(u8::from_str_radix(&text[i + 1..i + 3], 16).unwrap());
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

/// Every link in a note, as the path it points at.
fn links(note: &Path, text: &str, style: LinkStyle) -> Vec<PathBuf> {
    let atlas = note.parent().unwrap().parent().unwrap();
    let mut out = Vec::new();
    match style {
        LinkStyle::Obsidian => {
            for part in text.split("[[").skip(1) {
                let target = part.split("]]").next().unwrap();
                out.push(atlas.join(format!("{target}.md")));
            }
        }
        LinkStyle::Markdown => {
            for part in text.split("](../").skip(1) {
                let target = part.split(')').next().unwrap();
                out.push(atlas.join(decode(target)));
            }
        }
    }
    out
}

fn symlink(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
    #[cfg(windows)]
    {
        let made = if target.is_dir() {
            std::os::windows::fs::symlink_dir(target, link)
        } else {
            std::os::windows::fs::symlink_file(target, link)
        };
        if made.is_err() {
            eprintln!("this Windows account can't make symbolic links: that case is skipped");
        }
        made.is_ok()
    }
}

#[test]
fn p3_22_markdown_notes_with_front_matter_and_working_links() {
    let x = X::new();
    let made = x.populate();
    for style in [LinkStyle::Obsidian, LinkStyle::Markdown] {
        let dir = tempfile::tempdir().unwrap();
        let report = x.everything(style, dir.path());
        let atlas = dir.path().join("Atlas");
        let written = notes(&atlas);
        // 2 trails, 2 findings, 1 bookmarked document, 2 collections.
        assert_eq!(written.len(), 7, "{written:?}");
        assert_eq!((report.written, report.replaced, report.besides), (7, 0, 0));
        let mut linked = 0;
        for note in &written {
            let text = fs::read_to_string(note).unwrap();
            assert!(text.starts_with("---\natlas_id: \""), "{note:?}");
            for key in ["atlas_kind: ", "created: ", "updated: ", "exported: "] {
                assert!(text.contains(&format!("\n{key}\"")), "{key} in {note:?}");
            }
            for target in links(note, &text, style) {
                assert!(target.is_file(), "{note:?} links to {target:?}");
                linked += 1;
            }
        }
        // Each collection links its trail, finding and bookmarks.
        assert!(linked >= 6, "{linked} links");

        // The names: forbidden characters, a reserved one, the same twice.
        let trails = atlas.join("Trails");
        assert!(trails.join("2026-11-11 Stoic gears a b.md").is_file());
        assert!(trails.join("2026-11-11 Stoic gears a b 2.md").is_file());
        assert!(atlas.join("Projects/CON_.md").is_file());
        assert!(atlas.join("Interests/Mechanisms.md").is_file());

        // A trail as recorded, its sources with their files as text.
        let trail = fs::read_to_string(trails.join("2026-11-11 Stoic gears a b.md")).unwrap();
        for part in [
            &format!("atlas_id: \"{}\"", made.trail),
            "# Stoic gears: a/b?",
            "· Search: Engineering marker 1",
            "· Question: Biology marker 5",
            "Cells divide [1].",
            "· Opened ",
            "> A thought of my own",
            "> **Note:**\n> On the search",
            "book 000001.pdf`",
        ] {
            assert!(trail.contains(part), "{part:?} in\n{trail}");
        }
        let finding = fs::read_dir(atlas.join("Findings"))
            .unwrap()
            .flatten()
            .map(|e| fs::read_to_string(e.path()).unwrap())
            .find(|t| t.contains(&made.finding))
            .unwrap();
        assert!(finding.contains("**Question:** Biology marker 5"));
        assert!(finding.contains("From the trail "), "{finding}");
        let project = fs::read_to_string(atlas.join("Projects/CON_.md")).unwrap();
        assert!(project.contains(&made.project) && project.contains("**Goal:** A chapter"));
        assert!(project.contains("## Documents") && project.contains("book 000004.pdf"));
    }

    // A second export into a folder where the user keeps their own note
    // under the trail's name; then again after a rename.
    let dir = tempfile::tempdir().unwrap();
    let trails = dir.path().join("Atlas/Trails");
    fs::create_dir_all(&trails).unwrap();
    let mine = trails.join("2026-11-11 Stoic gears a b.md");
    fs::write(&mine, "# My own notes\n").unwrap();
    let first = x.everything(LinkStyle::Obsidian, dir.path());
    assert_eq!(first.besides, 2, "both trails moved past my note");
    assert_eq!(fs::read(&mine).unwrap(), b"# My own notes\n");
    let ours = trails.join("2026-11-11 Stoic gears a b 2.md");
    let before = fs::read_to_string(&ours).unwrap();
    assert!(before.contains(&made.trail) || before.contains(&made.second));
    let again = x.everything(LinkStyle::Obsidian, dir.path());
    assert_eq!((again.written, again.replaced), (0, 7), "{again:?}");
    assert_eq!(fs::read(&mine).unwrap(), b"# My own notes\n");
    x.rec().rename(&made.second, "Renamed").unwrap();
    let renamed = x.everything(LinkStyle::Obsidian, dir.path());
    assert_eq!(renamed.written, 1, "{renamed:?}");
    assert!(trails.join("2026-11-11 Renamed.md").is_file());
    assert_eq!(notes(&dir.path().join("Atlas")).len(), 9, "nothing deleted");

    // A long title stays within the name limit.
    x.rec().rename(&made.trail, &"é".repeat(200)).unwrap();
    let long = tempfile::tempdir().unwrap();
    x.export(
        ExportScope::Trail {
            id: made.trail.clone(),
        },
        ExportFormat::Markdown,
        LinkStyle::Markdown,
        long.path(),
    )
    .unwrap();
    let names = notes(&long.path().join("Atlas"));
    assert_eq!(names.len(), 1);
    let stem = names[0].file_stem().unwrap().to_string_lossy().into_owned();
    assert_eq!(stem.chars().count(), 100);

    // A source whose file is gone says so.
    let books = x.lib.env.base.join("Books");
    let path = find(&books, "book 000001.pdf").unwrap();
    fs::remove_file(path).unwrap();
    x.rescan();
    let gone = tempfile::tempdir().unwrap();
    x.everything(LinkStyle::Obsidian, gone.path());
    let bookmarks = notes(&gone.path().join("Atlas"))
        .into_iter()
        .find(|p| p.parent().unwrap().ends_with("Bookmarks"))
        .unwrap();
    let text = fs::read_to_string(bookmarks).unwrap();
    assert!(text.contains("No file has this document now."), "{text}");
    assert!(text.contains("Note: Read again"));
}

#[test]
fn p3_23_atlas_writes_only_inside_its_own_atlas_folder() {
    let x = X::new();
    let made = x.populate();
    let scope = || ExportScope::Trail {
        id: made.trail.clone(),
    };
    let md = |dir: &Path| x.export(scope(), ExportFormat::Markdown, LinkStyle::Obsidian, dir);
    let base = &x.lib.env.base;
    let books = base.join("Books");
    let data = base.join("AtlasData");

    // Every refusal leaves the scratch tree as it was. The data root is
    // left out of the comparison (its databases are written by the
    // library's own jobs); no Atlas folder appears in it.
    let refused = |dir: &Path, expected: ErrorCode| {
        let before = tree(base, &[&data]);
        let atlas = || fs::symlink_metadata(dir.join("Atlas")).is_ok();
        let had = atlas();
        assert_eq!(code(md(dir)), expected, "{dir:?}");
        assert_eq!(tree(base, &[&data]), before, "{dir:?}");
        assert_eq!(atlas(), had, "{dir:?}");
    };
    refused(&books, ErrorCode::ExportInsideLibrary);
    fs::create_dir_all(books.join("inside")).unwrap();
    refused(&books.join("inside"), ErrorCode::ExportInsideLibrary);
    refused(&data, ErrorCode::ExportInsideDataRoot);
    refused(&data.join("library"), ErrorCode::ExportInsideDataRoot);
    // The data root's own exports folder is allowed.
    md(&data.join("exports")).unwrap();
    assert!(data.join("exports/Atlas/Trails").is_dir());

    // An Atlas that is a file, or a link, is refused.
    let out = base.join("Out");
    fs::create_dir_all(&out).unwrap();
    fs::write(out.join("Atlas"), "a file").unwrap();
    refused(&out, ErrorCode::ExportFolderUnusable);
    fs::remove_file(out.join("Atlas")).unwrap();
    let elsewhere = base.join("Elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    if symlink(&elsewhere, &out.join("Atlas")) {
        refused(&out, ErrorCode::ExportFolderUnusable);
        assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
        fs::remove_file(out.join("Atlas"))
            .or_else(|_| fs::remove_dir(out.join("Atlas")))
            .unwrap();
    }
    // A subfolder that is a link: refused before anything is written.
    fs::create_dir_all(out.join("Atlas")).unwrap();
    if symlink(&elsewhere, &out.join("Atlas/Trails")) {
        refused(&out, ErrorCode::ExportFolderUnusable);
        assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
        fs::remove_file(out.join("Atlas/Trails"))
            .or_else(|_| fs::remove_dir(out.join("Atlas/Trails")))
            .unwrap();
    }

    // A link planted at a note's name: the note goes beside it, and what
    // the link points at is untouched.
    let target = elsewhere.join("target.md");
    fs::write(&target, "not Atlas's").unwrap();
    let trails = out.join("Atlas/Trails");
    fs::create_dir_all(&trails).unwrap();
    let planted = trails.join("2026-11-11 Stoic gears a b.md");
    if symlink(&target, &planted) {
        let before = tree(base, &[&data, &out.join("Atlas")]);
        let report = md(&out).unwrap();
        assert_eq!((report.written, report.besides), (1, 1));
        assert_eq!(fs::read_to_string(&target).unwrap(), "not Atlas's");
        assert!(
            fs::symlink_metadata(&planted)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(trails.join("2026-11-11 Stoic gears a b 2.md").is_file());
        assert_eq!(
            tree(base, &[&data, &out.join("Atlas")]),
            before,
            "only Atlas/ changed"
        );
    }

    // A folder that contains the library is fine: only its Atlas is used.
    let before = tree(base, &[&data, &base.join("Atlas")]);
    md(base).unwrap();
    assert_eq!(tree(base, &[&data, &base.join("Atlas")]), before);
    assert!(base.join("Atlas/Trails").is_dir());
}

#[test]
fn p3_24_json_round_trip_newer_wins_and_bad_files_are_refused_whole() {
    let one = X::new();
    let made = one.populate();
    let dir = tempfile::tempdir().unwrap();
    let file = one.json(dir.path());
    assert!(
        file.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("atlas-knowledge-2026-11-11")
    );
    let text = fs::read_to_string(&file).unwrap();
    assert!(
        !text.contains(&*one.lib.env.base.to_string_lossy()),
        "no paths"
    );

    // Into an empty store: everything is new, and the stores are equal.
    let two = X::new();
    let preview = export::preview(&two.handles(), &file).unwrap();
    assert_eq!(
        preview.summary.trails,
        ImportCounts {
            new: 2,
            newer: 0,
            here: 0
        }
    );
    assert_eq!(preview.summary.findings.new, 2);
    assert_eq!(preview.summary.bookmarks.new, 1);
    assert_eq!(preview.summary.collections.new, 2);
    assert_eq!(preview.expiring, 0);
    assert!(dump(&two.store()).values().all(Vec::is_empty), "a dry run");
    let snapshots = |x: &X| {
        fs::read_dir(x.lib.env.base.join("AtlasData/backups/knowledge"))
            .map(|d| {
                d.flatten()
                    .filter(|e| e.path().extension().is_some_and(|x| x == "sqlite"))
                    .count()
            })
            .unwrap_or(0)
    };
    let before = snapshots(&two);
    export::apply(&two.handles(), &preview.token).unwrap();
    assert_eq!(snapshots(&two), before + 1, "a snapshot first");
    assert_eq!(dump(&two.store()), dump(&one.store()));
    // A token applies once.
    assert_eq!(
        code(export::apply(&two.handles(), &preview.token)),
        ErrorCode::ImportExpired
    );
    assert_eq!(
        code(export::apply(&two.handles(), "nope")),
        ErrorCode::ImportExpired
    );

    // Into the same store: all here, nothing written.
    let changes = one.changes();
    let same = export::preview(&one.handles(), &file).unwrap();
    for counts in [
        same.summary.trails,
        same.summary.bookmarks,
        same.summary.findings,
        same.summary.collections,
    ] {
        assert_eq!((counts.new, counts.newer), (0, 0), "{:?}", same.summary);
    }
    export::apply(&one.handles(), &same.token).unwrap();
    assert_eq!(one.changes(), changes, "nothing changed");

    // Newer copies win, both ways; older ones don't.
    two.clock.advance(time::Duration::hours(1));
    two.rec().rename(&made.trail, "Renamed elsewhere").unwrap();
    saved::set_note(two.rec(), SavedKind::Finding, &made.finding, "A newer note").unwrap();
    let step = two.rec().get(&made.trail).unwrap().steps[1].id.clone();
    two.rec()
        .note(&made.trail, Some(&step), "A newer step note")
        .unwrap();
    let newer_dir = tempfile::tempdir().unwrap();
    let newer = two.json(newer_dir.path());
    let p = export::preview(&one.handles(), &newer).unwrap();
    assert_eq!(
        (p.summary.trails.newer, p.summary.findings.newer),
        (1, 1),
        "{:?}",
        p.summary
    );
    export::apply(&one.handles(), &p.token).unwrap();
    let trail = one.rec().get(&made.trail).unwrap();
    assert_eq!(trail.trail.title.as_deref(), Some("Renamed elsewhere"));
    assert_eq!(trail.steps[1].note.as_deref(), Some("A newer step note"));
    assert_eq!(trail.steps.len(), 4, "no step duplicated");
    assert_eq!(dump(&one.store()), dump(&two.store()));
    // The first file is older now: nothing in it wins.
    let old = export::preview(&two.handles(), &file).unwrap();
    assert_eq!(
        (old.summary.trails.newer, old.summary.findings.newer),
        (0, 0)
    );

    // A bookmark at the same place under another id is the same bookmark.
    let bookmark: String = one
        .store()
        .query_row("SELECT id FROM bookmarks", [], |r| r.get(0))
        .unwrap();
    let other = text.replace(&bookmark, "0f0f0f0f-0f0f-4f0f-8f0f-0f0f0f0f0f0f");
    let other_file = dir.path().join("other.json");
    fs::write(&other_file, &other).unwrap();
    let p = export::preview(&one.handles(), &other_file).unwrap();
    assert_eq!(p.summary.bookmarks.new, 0, "{:?}", p.summary);
    export::apply(&one.handles(), &p.token).unwrap();
    let count: i64 = one
        .store()
        .query_row("SELECT count(*) FROM bookmarks", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);

    // Bad files: each refused whole, the store as it was.
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    let changed = |f: &dyn Fn(&mut serde_json::Value)| {
        let mut v = value.clone();
        f(&mut v);
        serde_json::to_string(&v).unwrap()
    };
    let cases: Vec<(&str, Vec<u8>, ErrorCode)> = vec![
        (
            "truncated",
            text.as_bytes()[..text.len() / 2].to_vec(),
            ErrorCode::ImportUnreadable,
        ),
        (
            "version 99",
            changed(&|v| v["version"] = 99.into()).into_bytes(),
            ErrorCode::ImportUnknownVersion,
        ),
        (
            "a bad id",
            changed(&|v| v["trails"][0]["id"] = "not-a-uuid".into()).into_bytes(),
            ErrorCode::ImportInvalid,
        ),
        (
            "a long note",
            changed(&|v| v["findings"][0]["note"] = "a".repeat(4_001).into()).into_bytes(),
            ErrorCode::ImportInvalid,
        ),
        (
            "a payload of another version",
            changed(&|v| v["trails"][0]["steps"][0]["payload"]["v"] = 2.into()).into_bytes(),
            ErrorCode::ImportInvalid,
        ),
        (
            "a step kind that doesn't exist",
            changed(&|v| v["trails"][0]["steps"][0]["payload"]["kind"] = "dance".into())
                .into_bytes(),
            ErrorCode::ImportInvalid,
        ),
        (
            "a link to nothing",
            changed(&|v| {
                v["collections"][0]["items"]
                    .as_array_mut()
                    .unwrap()
                    .push(serde_json::json!({
                        "kind": "trail",
                        "id": "1f1f1f1f-1f1f-4f1f-8f1f-1f1f1f1f1f1f",
                        "contentHash": null,
                        "addedAt": "2026-11-11T09:00:00Z"
                    }))
            })
            .into_bytes(),
            ErrorCode::ImportInvalid,
        ),
        (
            "an unknown field",
            changed(&|v| v["extra"] = 1.into()).into_bytes(),
            ErrorCode::ImportInvalid,
        ),
    ];
    let three = X::new();
    for (name, bytes, expected) in cases {
        let path = dir.path().join("bad.json");
        fs::write(&path, bytes).unwrap();
        for x in [&one, &three] {
            let before = dump(&x.store());
            assert_eq!(
                code(export::preview(&x.handles(), &path)),
                expected,
                "{name}"
            );
            assert_eq!(dump(&x.store()), before, "{name}");
        }
    }
    // Over 256 MiB, without writing that much.
    let big = dir.path().join("big.json");
    fs::File::create(&big)
        .unwrap()
        .set_len(import::MAX_BYTES + 1)
        .unwrap();
    assert_eq!(
        code(export::preview(&three.handles(), &big)),
        ErrorCode::ImportTooLarge
    );
    assert!(dump(&three.store()).values().all(Vec::is_empty));
}

#[test]
fn a_generated_store_imports_cleanly() {
    let x = X::new();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("generated.json");
    let out = std::process::Command::new("node")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/generate-knowledge.mjs"))
        .arg(&file)
        .args(["--trails", "950", "--steps", "1000"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let preview = export::preview(&x.handles(), &file).unwrap();
    assert_eq!(preview.summary.trails.new, 950);
    assert_eq!(preview.summary.bookmarks.new, 950);
    assert_eq!(preview.summary.findings.new, 475);
    assert_eq!(preview.summary.collections.new, 9);
    // Every trail is from early 2026: past the 90 days. The first project
    // holds 900; of the other 50, the unpinned ones in no collection
    // would expire, and the preview says so.
    assert!((1..=50).contains(&preview.expiring), "{}", preview.expiring);
    export::apply(&x.handles(), &preview.token).unwrap();
    let steps: i64 = x
        .store()
        .query_row("SELECT count(*) FROM trail_steps", [], |r| r.get(0))
        .unwrap();
    assert_eq!(steps, 1000);
}
