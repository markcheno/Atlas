//! Slice 1.6: duplicate groups (Library › Duplicates). Files with the same
//! bytes are grouped once they are hashed; a group needs two available
//! copies inside the filters; missing copies never count; copies in a
//! disconnected folder are marked. Runs the real worker.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use atlas_lib::domain::dto::{Availability, DuplicatePage, DuplicateQuery, FileType, TitleSource};
use atlas_lib::services::documents::duplicates;
use atlas_lib::services::extraction::db;
use atlas_lib::services::workspace::{OpenRoot, Workspace};
use atlas_lib::services::{inventory, sources};

mod common;
use common::{Env, ExtractionHarness, fast_extraction, originals, wait_until, worker};

const LONG: Duration = Duration::from_secs(180);

fn generate(dir: &Path) {
    let out = Command::new("node")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/generate-fixtures.mjs"))
        .arg(dir)
        .args(["--files", "12", "--text-kb", "4"])
        .output()
        .expect("node runs the fixture generator");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn find(dir: &Path, name: &str) -> PathBuf {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.file_name().is_some_and(|n| n == name) {
                return p;
            }
            if e.file_type().unwrap().is_dir() && p.file_name().unwrap() != "locked" {
                stack.push(p);
            }
        }
    }
    panic!("{name} not generated")
}

struct Lib {
    ws: Workspace,
    _h: ExtractionHarness,
    env: Env,
    a: String,
    b: String,
}

impl Lib {
    fn root(&self) -> &OpenRoot {
        self.ws.require_open().unwrap()
    }

    fn settled(&self) -> bool {
        let inv = inventory::control::overview(self.root()).unwrap();
        let ov = self
            .root()
            .reader()
            .read(|c| db::overview(c, None))
            .unwrap()
            .unwrap();
        inv.active.is_none() && ov.active.is_none() && ov.queued_jobs == 0 && ov.counts.pending == 0
    }

    fn wait(&self) {
        assert!(wait_until(LONG, || self.settled()), "never settled");
    }

    fn list(&self, f: impl FnOnce(&mut DuplicateQuery)) -> DuplicatePage {
        let mut q = DuplicateQuery {
            source_root_id: None,
            file_type: None,
            include_missing: false,
            page: 0,
            page_size: 20,
        };
        f(&mut q);
        duplicates::list(self.root().reader(), self.root().root_status(), &q).unwrap()
    }

    fn rescan(&self, root: &str) {
        inventory::control::enqueue(self.root(), root).unwrap();
        self.wait();
    }
}

/// Folder A: x.pdf, x copy.pdf, y.epub, z.pdf. Folder B: x.pdf, y.epub.
fn library() -> (Lib, PathBuf, PathBuf) {
    let env = Env::new();
    let generated = env.base.join("generated");
    generate(&generated);
    let a = env.base.join("A");
    let b = env.base.join("B");
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    let x = find(&generated, "book 000001.pdf");
    let y = find(&generated, "book 000006.epub");
    fs::copy(&x, a.join("x.pdf")).unwrap();
    fs::copy(&x, a.join("x copy.pdf")).unwrap();
    fs::copy(&y, a.join("y.epub")).unwrap();
    fs::copy(find(&generated, "book 000002.pdf"), a.join("z.pdf")).unwrap();
    fs::copy(&x, b.join("x.pdf")).unwrap();
    fs::copy(&y, b.join("y.epub")).unwrap();

    let h = ExtractionHarness::new(fast_extraction(), worker());
    let mut ws = env.workspace_with_extraction(&h);
    ws.choose(&env.base.join("AtlasData")).unwrap();
    let root = ws.require_open().unwrap();
    let ra = sources::add(root, &a).unwrap().id;
    let rb = sources::add(root, &b).unwrap().id;
    inventory::control::enqueue(root, &ra).unwrap();
    inventory::control::enqueue(root, &rb).unwrap();
    let lib = Lib {
        ws,
        _h: h,
        env,
        a: ra,
        b: rb,
    };
    lib.wait();
    (lib, a, b)
}

#[test]
fn copies_are_grouped_with_their_extra_space_most_first() {
    let (lib, a, b) = library();
    let before = (originals(&a), originals(&b));
    let x_size = fs::metadata(a.join("x.pdf")).unwrap().len();
    let y_size = fs::metadata(a.join("y.epub")).unwrap().len();

    let page = lib.list(|_| {});
    assert_eq!(page.total_groups, 2, "{page:#?}");
    assert_eq!(page.extra_bytes, 2 * x_size + y_size);
    let mut expected = [
        (2 * x_size, "Engineering book 000001"),
        (y_size, "Astronomy book 000006"),
    ];
    expected.sort_by(|p, q| q.0.cmp(&p.0));
    let got: Vec<(u64, &str)> = page
        .groups
        .iter()
        .map(|g| (g.extra_bytes, g.title.as_str()))
        .collect();
    assert_eq!(got, expected);
    let x = page.groups.iter().find(|g| g.copies.len() == 3).unwrap();
    assert_eq!(x.size_bytes, Some(x_size));
    assert_eq!(x.title_source, TitleSource::Metadata);
    let names: Vec<&str> = x.copies.iter().map(|c| c.display_name.as_str()).collect();
    assert_eq!(names.len(), 3);
    assert!(
        x.copies
            .iter()
            .all(|c| c.availability == Availability::Available)
    );
    // z.pdf has one copy: not a group.
    assert!(
        page.groups
            .iter()
            .all(|g| g.copies.iter().all(|c| c.display_name != "z.pdf"))
    );

    // Filters: inside folder A only x has two copies; EPUBs only y.
    let only_a = lib.list(|q| q.source_root_id = Some(lib.a.clone()));
    assert_eq!(only_a.total_groups, 1);
    assert_eq!(only_a.groups[0].copies.len(), 2);
    assert_eq!(only_a.extra_bytes, x_size);
    let epubs = lib.list(|q| q.file_type = Some(FileType::Epub));
    assert_eq!(epubs.total_groups, 1);
    assert_eq!(epubs.groups[0].title, "Astronomy book 000006");

    // Paging over groups.
    let first = lib.list(|q| q.page_size = 1);
    let second = lib.list(|q| {
        q.page_size = 1;
        q.page = 1;
    });
    assert_eq!((first.groups.len(), second.groups.len()), (1, 1));
    assert_eq!(first.total_groups, 2);
    assert_ne!(first.groups[0].document_id, second.groups[0].document_id);
    let past = lib.list(|q| q.page = 5);
    assert!(past.groups.is_empty());
    assert_eq!(past.total_groups, 2);
    assert_eq!(
        (originals(&a), originals(&b)),
        before,
        "nothing is changed or removed"
    );
}

#[test]
fn missing_copies_never_count_and_disconnected_folders_are_marked() {
    let (lib, _a, b) = library();
    // y's second copy goes missing: one available copy is no group, even
    // when missing files are shown.
    fs::remove_file(b.join("y.epub")).unwrap();
    lib.rescan(&lib.b);
    assert_eq!(lib.list(|_| {}).total_groups, 1);
    let with_missing = lib.list(|q| q.include_missing = true);
    assert_eq!(with_missing.total_groups, 1);
    let x = &with_missing.groups[0];
    assert_eq!(x.copies.len(), 3, "x has no missing copy");

    // x's copy in B goes missing: still a group of two, its extra space
    // shrinks, and the missing copy is listed last only on request.
    fs::remove_file(b.join("x.pdf")).unwrap();
    lib.rescan(&lib.b);
    let page = lib.list(|_| {});
    assert_eq!(page.total_groups, 1);
    let x_size = page.groups[0].size_bytes.unwrap();
    assert_eq!(page.extra_bytes, x_size);
    assert_eq!(page.groups[0].copies.len(), 2);
    let shown = lib.list(|q| q.include_missing = true);
    let copies = &shown.groups[0].copies;
    assert_eq!(copies.len(), 3);
    assert_eq!(copies[2].availability, Availability::Missing);
    assert_eq!(shown.extra_bytes, x_size, "missing copies never count");

    // Folder A disconnects: its copies are marked, not dropped.
    fs::copy(lib.env.base.join("A/x.pdf"), b.join("x.pdf")).unwrap();
    lib.rescan(&lib.b);
    let moved = lib.env.base.join("A-away");
    fs::rename(lib.env.base.join("A"), &moved).unwrap();
    std::thread::sleep(Duration::from_secs(6)); // past the 5 s folder-status cache
    let page = lib.list(|_| {});
    assert_eq!(page.total_groups, 1);
    let states: Vec<Availability> = page.groups[0]
        .copies
        .iter()
        .map(|c| c.availability)
        .collect();
    assert_eq!(
        states
            .iter()
            .filter(|s| **s == Availability::Unreachable)
            .count(),
        2,
        "{states:?}"
    );
    fs::rename(&moved, lib.env.base.join("A")).unwrap();
}
