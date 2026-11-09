//! Slice 0.6: browsing and searching the catalog (phase0-spec §5.5, §13;
//! tests P0-15, P0-24).

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use atlas_lib::domain::dto::{
    Availability, DocumentPage, DocumentQuery, DocumentSort, FileType, JobState,
};
use atlas_lib::services::inventory::{InventoryConfig, control};
use atlas_lib::services::library;
use atlas_lib::services::sources;
use atlas_lib::services::workspace::Workspace;

mod common;
use common::{Env, InventoryHarness, fast_config, wait_until};

/// Bounds a failure, not a performance claim: shared CI runners are slow.
const LONG: Duration = Duration::from_secs(120);

struct Lib {
    env: Env,
    h: InventoryHarness,
    ws: Workspace,
}

impl Lib {
    fn new() -> Self {
        Self::with(fast_config())
    }

    fn with(config: InventoryConfig) -> Self {
        let env = Env::new();
        let h = InventoryHarness::new(config);
        let mut ws = env.workspace_with_inventory(h.deps.clone());
        ws.choose(&env.base.join("AtlasData")).unwrap();
        Self { env, h, ws }
    }

    fn add(&self, path: &Path) -> String {
        let root = self.ws.require_open().unwrap();
        let added = sources::add(root, path).unwrap();
        control::enqueue(root, &added.id).unwrap();
        added.id
    }

    fn settle(&self) {
        assert!(wait_until(LONG, || {
            control::overview(self.ws.require_open().unwrap())
                .is_ok_and(|o| o.active.is_none() && o.queued_sessions == 0)
        }));
    }

    fn rescan(&self, root_id: &str) {
        control::enqueue(self.ws.require_open().unwrap(), root_id).unwrap();
        self.settle();
    }

    fn list(&self, q: &DocumentQuery) -> DocumentPage {
        let root = self.ws.require_open().unwrap();
        library::list(root.reader(), root.root_status(), q).unwrap()
    }

    fn names(&self, q: &DocumentQuery) -> Vec<String> {
        self.list(q)
            .items
            .into_iter()
            .map(|i| i.display_name)
            .collect()
    }
}

fn query() -> DocumentQuery {
    DocumentQuery {
        text: None,
        source_root_id: None,
        file_type: None,
        sort: DocumentSort::NameAsc,
        page: 0,
        page_size: 200,
        include_missing: false,
        text_status: None,
    }
}

fn search(text: &str) -> DocumentQuery {
    DocumentQuery {
        text: Some(text.into()),
        ..query()
    }
}

fn write(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, b"%PDF").unwrap();
}

fn set_mtime(path: &Path, secs_ago: u64) {
    let file = fs::File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::now() - Duration::from_secs(secs_ago))
        .unwrap();
}

fn tree(root: &Path) {
    for f in [
        "Astronomy/Meteors/detection in noisy video.pdf",
        "Astronomy/Café crème.pdf",
        "Astronomy/Cafe\u{301} au lait.epub",
        "Engineering/harmonic drive.EPUB",
        "Math/100% proof.pdf",
        "Math/a_b notes.pdf",
        "Math/aXb notes.pdf",
        "日本語/本の題名.pdf",
    ] {
        write(&root.join(f));
    }
}

#[test]
fn search_is_case_accent_and_normalization_insensitive() {
    let lib = Lib::new();
    let books = lib.env.base.join("Books");
    tree(&books);
    lib.add(&books);
    lib.settle();

    assert_eq!(
        lib.names(&search("meteor noisy")),
        ["detection in noisy video.pdf"]
    );
    assert_eq!(lib.list(&search("cafe")).matched_so_far, 2);
    assert_eq!(lib.list(&search("CAFÉ")).matched_so_far, 2);
    assert_eq!(lib.list(&search("Cafe\u{301}")).matched_so_far, 2);
    assert_eq!(lib.names(&search("題名")), ["本の題名.pdf"]);
    // Folder names are searchable too.
    assert_eq!(lib.list(&search("astronomy")).matched_so_far, 3);
    // Wildcards in user input are literal.
    assert_eq!(lib.names(&search("100%")), ["100% proof.pdf"]);
    assert_eq!(lib.names(&search("a_b")), ["a_b notes.pdf"]);
    // Every term must match.
    assert_eq!(lib.list(&search("cafe meteor")).matched_so_far, 0);
    // The data root's own path is not part of the search text.
    assert_eq!(lib.list(&search("atlasdata")).matched_so_far, 0);
}

#[test]
fn filters_sorting_and_pagination() {
    let lib = Lib::new();
    let books = lib.env.base.join("Books");
    let papers = lib.env.base.join("Papers");
    tree(&books);
    for i in 0..120 {
        let p = papers.join(format!("paper {i:03}.pdf"));
        write(&p);
        set_mtime(&p, 10_000 - i * 10);
    }
    let books_id = lib.add(&books);
    let papers_id = lib.add(&papers);
    lib.settle();

    let epubs = DocumentQuery {
        file_type: Some(FileType::Epub),
        ..query()
    };
    assert_eq!(lib.list(&epubs).matched_so_far, 2);
    let only_books = DocumentQuery {
        source_root_id: Some(books_id),
        ..query()
    };
    assert_eq!(lib.list(&only_books).matched_so_far, 8);

    let papers_q = |sort, page, page_size| DocumentQuery {
        source_root_id: Some(papers_id.clone()),
        sort,
        page,
        page_size,
        ..query()
    };
    let first = lib.list(&papers_q(DocumentSort::NameAsc, 0, 50));
    assert_eq!(first.items.len(), 50);
    assert_eq!(first.matched_so_far, 120);
    assert_eq!(first.items[0].display_name, "paper 000.pdf");
    assert_eq!(
        lib.list(&papers_q(DocumentSort::NameAsc, 2, 50))
            .items
            .len(),
        20
    );
    assert!(
        lib.list(&papers_q(DocumentSort::NameAsc, 99, 50))
            .items
            .is_empty()
    );
    assert_eq!(
        lib.list(&papers_q(DocumentSort::NameDesc, 0, 1)).items[0].display_name,
        "paper 119.pdf"
    );
    // Newest first: paper 119 was given the most recent mtime.
    assert_eq!(
        lib.list(&papers_q(DocumentSort::ModifiedDesc, 0, 1)).items[0].display_name,
        "paper 119.pdf"
    );
    assert_eq!(
        lib.list(&papers_q(DocumentSort::ModifiedAsc, 0, 1)).items[0].display_name,
        "paper 000.pdf"
    );
    let clamped = lib.list(&papers_q(DocumentSort::NameAsc, 0, 10_000));
    assert_eq!(clamped.page_size, library::MAX_PAGE_SIZE);
    assert_eq!(
        lib.list(&papers_q(DocumentSort::NameAsc, 0, 0)).items.len(),
        1
    );
    assert!(!clamped.scanning);
}

#[test]
fn missing_files_are_hidden_unless_asked_for() {
    let lib = Lib::new();
    let books = lib.env.base.join("Books");
    tree(&books);
    let id = lib.add(&books);
    lib.settle();
    fs::remove_file(books.join("Math/100% proof.pdf")).unwrap();
    lib.rescan(&id);

    assert_eq!(lib.list(&query()).matched_so_far, 7);
    let all = lib.list(&DocumentQuery {
        include_missing: true,
        text_status: None,
        ..search("proof")
    });
    assert_eq!(all.items.len(), 1);
    assert_eq!(all.items[0].availability, Availability::Missing);
}

#[test]
fn files_in_an_unplugged_folder_are_unreachable() {
    let lib = Lib::new();
    let drive = lib.env.base.join("Drive");
    tree(&drive.join("Books"));
    lib.add(&drive.join("Books"));
    lib.settle();
    fs::rename(&drive, lib.env.base.join("unplugged")).unwrap();

    let started = Instant::now();
    let page = lib.list(&query());
    assert!(started.elapsed() < Duration::from_millis(2500));
    assert_eq!(page.items.len(), 8);
    assert!(
        page.items
            .iter()
            .all(|i| i.availability == Availability::Unreachable)
    );
}

#[test]
fn search_keys_are_backfilled_for_older_libraries() {
    let mut lib = Lib::new();
    let books = lib.env.base.join("Books");
    tree(&books);
    lib.add(&books);
    lib.settle();
    lib.ws
        .require_open()
        .unwrap()
        .catalog()
        .call(|c| {
            c.execute(
                "UPDATE file_instances SET search_key = NULL, name_key = NULL",
                [],
            )
        })
        .unwrap()
        .unwrap();
    assert_eq!(lib.list(&search("cafe")).matched_so_far, 0);

    lib.ws = lib.env.other_workspace();
    let mut ws = lib.env.workspace_with_inventory(lib.h.deps.clone());
    ws.startup();
    lib.ws = ws;
    assert_eq!(lib.list(&search("cafe")).matched_so_far, 2);
}

/// P0-15: browsing works while the scan is still running.
#[test]
fn p0_15_results_grow_while_scanning() {
    let lib = Lib::new();
    let books = lib.env.base.join("Books");
    for i in 0..400 {
        write(&books.join(format!("shelf {}/book {i:04}.pdf", i % 8)));
    }
    lib.h.source.slow(Duration::from_millis(2));
    lib.add(&books);

    let mut seen_scanning = false;
    let mut last = 0;
    let started = Instant::now();
    while started.elapsed() < LONG {
        let page = lib.list(&search("book"));
        assert!(page.matched_so_far >= last, "results never shrink");
        last = page.matched_so_far;
        seen_scanning |= page.scanning && page.matched_so_far > 0 && page.matched_so_far < 400;
        if !page.scanning && page.matched_so_far == 400 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(seen_scanning, "saw partial results while scanning");
    assert_eq!(last, 400);
    let overview = control::overview(lib.ws.require_open().unwrap()).unwrap();
    assert_eq!(overview.roots[0].last_state, Some(JobState::Completed));
}

/// P0-24: names that are not valid UTF-8 list with replacement characters.
#[cfg(target_os = "linux")]
#[test]
fn p0_24_non_utf8_names_are_listed() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let lib = Lib::new();
    let books = lib.env.base.join("Books");
    fs::create_dir_all(&books).unwrap();
    fs::write(books.join(OsStr::from_bytes(b"caf\xe9.pdf")), b"%PDF").unwrap();
    lib.add(&books);
    lib.settle();
    let page = lib.list(&query());
    assert!(page.items[0].display_name.contains('\u{FFFD}'));
}

/// P0-24 on Windows: a file name with an unpaired surrogate lists with a
/// replacement character.
#[cfg(windows)]
#[test]
fn p0_24_unpaired_surrogate_names_are_listed() {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    let lib = Lib::new();
    let books = lib.env.base.join("Books");
    fs::create_dir_all(&books).unwrap();
    let mut wide: Vec<u16> = "caf".encode_utf16().collect();
    wide.push(0xD800);
    wide.extend(".pdf".encode_utf16());
    fs::write(books.join(OsString::from_wide(&wide)), b"%PDF").unwrap();
    lib.add(&books);
    lib.settle();
    let page = lib.list(&query());
    assert!(page.items[0].display_name.contains('\u{FFFD}'));
}

/// Paging and filters stay correct on a larger library. Latency is measured
/// by the benchmark harness (`benches/inventory.rs`), not asserted here: CI
/// runners are too noisy for timing gates.
#[test]
fn paging_and_filters_on_two_thousand_files() {
    let lib = Lib::with(InventoryConfig {
        batch_size: 500,
        ..fast_config()
    });
    let books = lib.env.base.join("Books");
    let subjects = ["astronomy", "engineering", "history", "math", "software"];
    for i in 0..2_000 {
        let subject = subjects[i % subjects.len()];
        let dir = books.join(subject).join(format!("shelf {:03}", i % 40));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("{subject} volume {i:05}.pdf")), b"").unwrap();
    }
    let root_id = lib.add(&books);
    lib.settle();
    assert_eq!(lib.list(&query()).matched_so_far, 2_000);

    // Terms match anywhere in the path, ANDed: "012" also hits "shelf 012".
    assert_eq!(lib.list(&search("volume 012")).matched_so_far, 157);

    let last = lib.list(&DocumentQuery {
        sort: DocumentSort::ModifiedDesc,
        page: 39,
        page_size: 50,
        ..query()
    });
    assert_eq!(last.items.len(), 50);
    let beyond = lib.list(&DocumentQuery {
        page: 40,
        page_size: 50,
        ..query()
    });
    assert!(beyond.items.is_empty());

    let filtered = lib.list(&DocumentQuery {
        source_root_id: Some(root_id),
        file_type: Some(FileType::Pdf),
        ..search("history shelf 002")
    });
    assert_eq!(filtered.matched_so_far, 69);
    assert!(
        filtered
            .items
            .iter()
            .all(|i| i.display_name.starts_with("history volume"))
    );

    assert_eq!(lib.list(&search("no such file anywhere")).matched_so_far, 0);
}
