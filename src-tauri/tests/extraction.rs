//! Slice 1.3: reading text (phase1-spec §7, §12; tests P1-03 … P1-10,
//! P1-13, P1-14). Every test runs the real worker (`atlas
//! --extract-worker`) on copies of the spike corpus or generated fixtures,
//! and checks that every original is unchanged afterwards.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use atlas_lib::domain::dto::{
    DocumentQuery, DocumentSort, ExtractionOverview, ExtractionState, FileInstance, JobState,
    PauseReason, PowerSource, ThrottleReason,
};
use atlas_lib::services::extraction::db;
use atlas_lib::services::extraction::job::ExtractionConfig;
use atlas_lib::services::extraction::supervisor::WorkerStats;
use atlas_lib::services::workspace::{OpenRoot, Workspace};
use atlas_lib::services::{inventory, library, sources};

mod common;
use common::{Env, ExtractionHarness, fast_extraction, originals, wait_until, worker};

/// Bounds a failure, not a performance claim.
const LONG: Duration = Duration::from_secs(180);

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../spikes/extraction-bench/corpus")
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_tree(&e.path(), &dest);
        } else {
            fs::copy(e.path(), dest).unwrap();
        }
    }
}

/// Writes a generated text fixture; returns its catalogable file count.
fn generate(dir: &Path, files: u32, kb: u32) -> u32 {
    let out = Command::new("node")
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/generate-fixtures.mjs"))
        .arg(dir)
        .args(["--files", &files.to_string(), "--text-kb", &kb.to_string()])
        .output()
        .expect("node runs the fixture generator");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let summary = String::from_utf8_lossy(&out.stdout);
    let summary: serde_json::Value =
        serde_json::from_str(summary.split("\nTo delete later").next().unwrap()).unwrap();
    summary["catalogableFiles"].as_u64().unwrap() as u32
}

struct Lib {
    ws: Workspace,
    h: ExtractionHarness,
    env: Env,
}

impl Lib {
    fn new() -> Self {
        Self::with(ExtractionHarness::new(fast_extraction(), worker()))
    }

    fn with(h: ExtractionHarness) -> Self {
        let env = Env::new();
        let mut ws = env.workspace_with_extraction(&h);
        ws.choose(&env.base.join("AtlasData")).unwrap();
        Self { ws, h, env }
    }

    fn root(&self) -> &OpenRoot {
        self.ws.require_open().unwrap()
    }

    fn books(&self) -> PathBuf {
        let p = self.env.base.join("Books");
        fs::create_dir_all(&p).unwrap();
        p
    }

    /// Adds a folder and catalogs it; reading starts when the scan ends.
    fn add(&self, path: &Path) -> String {
        let added = sources::add(self.root(), path).unwrap();
        inventory::control::enqueue(self.root(), &added.id).unwrap();
        added.id
    }

    fn rescan(&self, id: &str) {
        inventory::control::enqueue(self.root(), id).unwrap();
    }

    fn overview(&self) -> ExtractionOverview {
        self.root()
            .reader()
            .read(|c| db::overview(c, None))
            .unwrap()
            .unwrap()
    }

    fn settled(&self) -> bool {
        let inv = inventory::control::overview(self.root()).unwrap();
        let ov = self.overview();
        inv.active.is_none() && ov.active.is_none() && ov.queued_jobs == 0 && ov.counts.pending == 0
    }

    fn wait_settled(&self) {
        assert!(
            wait_until(LONG, || self.settled()),
            "never settled: {:?}",
            self.overview()
        );
    }

    /// Every available file, by absolute path.
    fn files(&self) -> BTreeMap<PathBuf, FileInstance> {
        let mut out = BTreeMap::new();
        for page in 0.. {
            let q = DocumentQuery {
                text: None,
                source_root_id: None,
                file_type: None,
                sort: DocumentSort::NameAsc,
                page,
                page_size: 200,
                include_missing: false,
                text_status: None,
            };
            let got = library::list(self.root().reader(), self.root().root_status(), &q).unwrap();
            if got.items.is_empty() {
                break;
            }
            for f in got.items {
                out.insert(PathBuf::from(&f.display_path), f);
            }
        }
        out
    }

    fn file(&self, path: &Path) -> FileInstance {
        let canonical = dunce::canonicalize(path).unwrap();
        self.files()
            .into_iter()
            .find(|(p, _)| *p == canonical)
            .unwrap_or_else(|| panic!("{} not cataloged", path.display()))
            .1
    }

    fn one(&self, sql: &str) -> i64 {
        let sql = sql.to_owned();
        self.root()
            .catalog()
            .call(move |c| c.query_row(&sql, [], |r| r.get(0)))
            .unwrap()
            .unwrap()
    }

    fn stats(&self) -> (u64, u64) {
        let s = &self.h.deps.stats;
        (WorkerStats::get(&s.hashes), WorkerStats::get(&s.extracts))
    }
}

fn set_mtime(path: &Path, secs_later: u64) {
    let f = fs::OpenOptions::new().write(true).open(path).unwrap();
    let now = fs::metadata(path).unwrap().modified().unwrap();
    f.set_modified(now + Duration::from_secs(secs_later))
        .unwrap();
}

/// P1-04, P1-08, P1-09: every corpus document ends in the state its
/// manifest expects; no text document is mistaken for a scan.
#[test]
fn p1_04_the_corpus_reaches_its_expected_statuses() {
    let lib = Lib::new();
    let books = lib.books();
    for dir in ["epub", "pdf", "malformed"] {
        copy_tree(&corpus().join(dir), &books.join(dir));
    }
    let before = originals(&books);
    lib.add(&books);
    lib.wait_settled();

    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(corpus().join("manifest.json")).unwrap()).unwrap();
    for doc in manifest["documents"].as_array().unwrap() {
        let rel = doc["path"].as_str().unwrap();
        let f = lib.file(&books.join(rel));
        let got = f.extraction.state;
        let id = doc["id"].as_str().unwrap();
        match doc["expect"].as_str().unwrap() {
            // Also P1-08: no text document is mistaken for a scan.
            "text" => assert_eq!(got, ExtractionState::Done, "{id}: {:?}", f.extraction),
            "needs-ocr" => assert_eq!(got, ExtractionState::NeedsOcr, "{id}"),
            "encrypted" => assert_eq!(got, ExtractionState::Encrypted, "{id}"),
            "no-crash" => assert!(
                !matches!(got, ExtractionState::Pending | ExtractionState::Reading),
                "{id}: {got:?}"
            ),
            other => panic!("unknown expectation {other}"),
        }
        if got != ExtractionState::Done {
            assert!(f.extraction.reason.is_some(), "{id}: no reason");
        }
    }
    // Text is stored only for readable documents.
    assert_eq!(
        lib.one(
            "SELECT COUNT(*) FROM passages p JOIN documents d ON d.id = p.document_id
             WHERE d.state <> 'done'"
        ),
        0
    );
    // 484 since Slice 3.1 left out the five Project Gutenberg licences
    // (about five 4,000-character pieces each).
    assert!(lib.one("SELECT COUNT(*) FROM passages") > 450);
    // PDFs: one passage per page.
    assert_eq!(
        lib.one(
            "SELECT COUNT(*) FROM documents d WHERE d.state = 'done' AND d.page_count IS NOT
                 (SELECT COUNT(*) FROM passages p WHERE p.document_id = d.id)
               AND EXISTS (SELECT 1 FROM passages p WHERE p.document_id = d.id AND p.page_no IS NOT NULL)"
        ),
        0
    );
    assert_eq!(originals(&books), before, "originals unchanged");
}

fn zip(path: &Path, entries: &[(&str, &[u8])]) {
    let file = fs::File::create(path).unwrap();
    let mut z = zip::ZipWriter::new(file);
    for (name, data) in entries {
        let method = if *name == "mimetype" {
            zip::CompressionMethod::Stored
        } else {
            zip::CompressionMethod::Deflated
        };
        z.start_file(
            *name,
            zip::write::SimpleFileOptions::default().compression_method(method),
        )
        .unwrap();
        z.write_all(data).unwrap();
    }
    z.finish().unwrap();
}

const CONTAINER: &[u8] = br#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
const OPF: &[u8] = br#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">urn:t</dc:identifier><dc:title>T</dc:title><dc:language>en</dc:language></metadata><manifest><item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/></spine></package>"#;
const CHAPTER: &[u8] = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>c</title></head><body><p>It was a bright cold day in April, and the clocks were striking thirteen.</p></body></html>";

/// P1-09: DRM is recognised before any chapter is read. The encrypted
/// chapter here is not valid UTF-8, so reading it first would have made the
/// book `malformed`, not `encrypted`. Font obfuscation alone is not DRM.
#[test]
fn p1_09_drm_is_found_before_any_chapter_is_read() {
    let lib = Lib::new();
    let books = lib.books();
    let cipher: Vec<u8> = (0..4096u32)
        .map(|i| (i * 7919 % 251) as u8 | 0x80)
        .collect();
    let drm = br#"<encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><EncryptedData xmlns="http://www.w3.org/2001/04/xmlenc#"><EncryptionMethod Algorithm="http://www.w3.org/2001/04/xmlenc#aes256-cbc"/><CipherData><CipherReference URI="OEBPS/c1.xhtml"/></CipherData></EncryptedData></encryption>"#;
    zip(
        &books.join("locked.epub"),
        &[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER),
            ("META-INF/encryption.xml", drm),
            ("OEBPS/content.opf", OPF),
            ("OEBPS/c1.xhtml", &cipher),
        ],
    );
    let fonts = br#"<encryption xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><EncryptedData xmlns="http://www.w3.org/2001/04/xmlenc#"><EncryptionMethod Algorithm="http://www.idpf.org/2008/embedding"/><CipherData><CipherReference URI="OEBPS/font.otf"/></CipherData></EncryptedData></encryption>"#;
    zip(
        &books.join("fonts.epub"),
        &[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER),
            ("META-INF/encryption.xml", fonts),
            ("OEBPS/content.opf", OPF),
            ("OEBPS/c1.xhtml", CHAPTER),
        ],
    );
    let before = originals(&books);
    lib.add(&books);
    lib.wait_settled();

    let locked = lib.file(&books.join("locked.epub"));
    assert_eq!(locked.extraction.state, ExtractionState::Encrypted);
    assert_eq!(locked.extraction.reason.as_deref(), Some("drm"));
    assert_eq!(
        lib.file(&books.join("fonts.epub")).extraction.state,
        ExtractionState::Done
    );
    assert_eq!(originals(&books), before);
}

/// P1-10: a 64 MiB chapter, a zip-slip entry, and plain text named `.pdf`
/// get a status and a reason; no worker is killed for memory.
#[test]
fn p1_10_oversized_unsafe_and_mislabelled_files() {
    let lib = Lib::new();
    let books = lib.books();
    let zeros = vec![b' '; 64 * 1024 * 1024];
    zip(
        &books.join("huge.epub"),
        &[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER),
            ("OEBPS/content.opf", OPF),
            ("OEBPS/c1.xhtml", &zeros),
        ],
    );
    fs::copy(
        corpus().join("malformed/zip-slip.epub"),
        books.join("slip.epub"),
    )
    .unwrap();
    fs::write(books.join("notes.pdf"), b"just some notes, not a PDF").unwrap();
    let before = originals(&books);
    lib.add(&books);
    lib.wait_settled();

    let reason = |name: &str| {
        let f = lib.file(&books.join(name));
        (f.extraction.state, f.extraction.reason)
    };
    assert_eq!(
        reason("huge.epub"),
        (ExtractionState::Unreadable, Some("entry_too_large".into()))
    );
    assert_eq!(
        reason("slip.epub"),
        (ExtractionState::Unreadable, Some("zip_slip".into()))
    );
    assert_eq!(
        reason("notes.pdf"),
        (ExtractionState::Unreadable, Some("not_a_pdf".into()))
    );
    assert_eq!(WorkerStats::get(&lib.h.deps.stats.kills), 0);
    assert_eq!(originals(&books), before);
}

/// P1-05, P1-06: a generated text library is read completely; the six
/// identical `Introduction.pdf` files are one document, read once.
#[test]
fn p1_05_p1_06_a_text_fixture_is_fully_read_and_copies_once() {
    let lib = Lib::new();
    let books = lib.env.base.join("Books");
    let catalogable = generate(&books, 120, 4);
    let before = originals(&books);
    lib.add(&books);
    lib.wait_settled();

    let ov = lib.overview();
    assert_eq!(ov.counts.done, catalogable, "{:?}", ov.counts);
    assert_eq!(ov.counts.total, catalogable);
    let documents = lib.one("SELECT COUNT(DISTINCT document_id) FROM file_instances");
    assert_eq!(
        documents,
        i64::from(catalogable) - 5,
        "six copies, one document"
    );
    assert_eq!(
        lib.one(
            "SELECT COUNT(DISTINCT document_id) FROM file_instances
             WHERE display_name = 'Introduction.pdf'"
        ),
        1
    );
    let (hashes, extracts) = lib.stats();
    assert_eq!(hashes, u64::from(catalogable));
    assert_eq!(extracts as i64, documents, "each content read once");
    assert_eq!(originals(&books), before);
}

/// P1-03: a rescan reads only what changed. A new mtime with the same bytes
/// is hashed again and linked; new bytes are read again.
#[test]
fn p1_03_only_changed_files_are_read_again() {
    let lib = Lib::new();
    let books = lib.books();
    for name in ["one-column.pdf", "two-column.pdf", "book.pdf"] {
        fs::copy(corpus().join("pdf/generated").join(name), books.join(name)).unwrap();
    }
    let root = lib.add(&books);
    lib.wait_settled();
    let (h0, e0) = lib.stats();
    assert_eq!((h0, e0), (3, 3));

    set_mtime(&books.join("one-column.pdf"), 100);
    lib.rescan(&root);
    lib.wait_settled();
    assert_eq!(lib.stats(), (4, 3), "re-hashed, not re-read");
    assert_eq!(
        lib.file(&books.join("one-column.pdf")).extraction.state,
        ExtractionState::Done
    );

    fs::copy(
        corpus().join("pdf/generated/features.pdf"),
        books.join("two-column.pdf"),
    )
    .unwrap();
    set_mtime(&books.join("two-column.pdf"), 200);
    lib.rescan(&root);
    lib.wait_settled();
    assert_eq!(lib.stats(), (5, 4), "new bytes are read again");
    assert_eq!(
        lib.one("SELECT COUNT(*) FROM documents"),
        4,
        "the old content's document stays until its last file is purged"
    );
}

/// P1-07: a moved file is linked to its document by content, not read again.
#[test]
fn p1_07_a_moved_file_links_by_hash() {
    let lib = Lib::new();
    let books = lib.books();
    fs::copy(
        corpus().join("pdf/generated/book.pdf"),
        books.join("book.pdf"),
    )
    .unwrap();
    let root = lib.add(&books);
    lib.wait_settled();
    let document = lib.file(&books.join("book.pdf")).document_id.unwrap();
    let (_, extracts) = lib.stats();

    fs::create_dir_all(books.join("moved")).unwrap();
    fs::rename(books.join("book.pdf"), books.join("moved/book.pdf")).unwrap();
    lib.rescan(&root);
    lib.wait_settled();
    let moved = lib.file(&books.join("moved/book.pdf"));
    assert_eq!(moved.document_id.as_deref(), Some(document.as_str()));
    assert_eq!(moved.extraction.state, ExtractionState::Done);
    assert_eq!(lib.stats().1, extracts, "not read again");
}

/// P1-13: a cloud-only file is skipped and never opened: the worker gets
/// no request for it.
#[test]
fn p1_13_cloud_only_files_are_never_opened() {
    let lib = Lib::new();
    let books = lib.books();
    for name in ["one-column.pdf", "two-column.pdf"] {
        fs::copy(corpus().join("pdf/generated").join(name), books.join(name)).unwrap();
    }
    let canonical = dunce::canonicalize(&books).unwrap();
    lib.h
        .inv
        .source
        .mark_cloud_only(&canonical.join("two-column.pdf"));
    lib.add(&books);
    lib.wait_settled();

    assert_eq!(
        lib.file(&books.join("two-column.pdf")).extraction.state,
        ExtractionState::CloudOnly
    );
    assert_eq!(lib.overview().counts.cloud_only, 1);
    assert_eq!(
        lib.stats(),
        (1, 1),
        "only the local file reached the worker"
    );
}

/// P3-01: a file skipped while only in the cloud is read once a rescan
/// finds it stored here, though a download keeps its size and time; one
/// still in the cloud is never opened. A file that was evicted after the
/// scan (skipped by the hash step, its storage still local) is read on the
/// next rescan too.
#[test]
fn p3_01_files_back_from_the_cloud_are_read() {
    let lib = Lib::new();
    let books = lib.books();
    for name in ["one-column.pdf", "two-column.pdf", "features.pdf"] {
        fs::copy(corpus().join("pdf/generated").join(name), books.join(name)).unwrap();
    }
    let canonical = dunce::canonicalize(&books).unwrap();
    let source = &lib.h.inv.source;
    source.mark_cloud_only(&canonical.join("two-column.pdf"));
    source.mark_cloud_only(&canonical.join("features.pdf"));
    let root = lib.add(&books);
    lib.wait_settled();
    assert_eq!(lib.stats(), (1, 1));
    assert_eq!(
        lib.file(&books.join("two-column.pdf")).extraction.state,
        ExtractionState::CloudOnly
    );

    // Downloaded: the same bytes, size and time, only stored here now.
    source.mark_local(&canonical.join("two-column.pdf"));
    lib.rescan(&root);
    lib.wait_settled();
    assert_eq!(
        lib.file(&books.join("two-column.pdf")).extraction.state,
        ExtractionState::Done
    );
    assert_eq!(
        lib.file(&books.join("features.pdf")).extraction.state,
        ExtractionState::CloudOnly,
        "still in the cloud"
    );
    assert_eq!(lib.stats(), (2, 2), "the cloud file was never opened");

    // Evicted between the scan and the read: the hash step skipped it while
    // its storage stayed local.
    let evicted = lib.file(&books.join("one-column.pdf")).id;
    lib.root()
        .catalog()
        .call(move |c| {
            c.execute(
                "UPDATE file_instances SET extract_state = 'skipped_cloud_only', document_id = NULL
                 WHERE id = ?1",
                [&evicted],
            )
        })
        .unwrap()
        .unwrap();
    assert_eq!(
        lib.file(&books.join("one-column.pdf")).extraction.state,
        ExtractionState::CloudOnly
    );
    lib.rescan(&root);
    lib.wait_settled();
    assert_eq!(
        lib.file(&books.join("one-column.pdf")).extraction.state,
        ExtractionState::Done,
        "read again once a rescan finds it here"
    );
}

/// P1-14: on battery reading slows; at 20% it pauses; it resumes only at
/// 25% (the same hysteresis as cataloging, P0-10).
#[test]
fn p1_14_battery_slows_then_pauses_then_resumes() {
    let config = ExtractionConfig {
        unthrottled: false,
        ..fast_extraction()
    };
    let lib = Lib::with(ExtractionHarness::new(config, worker()));
    let books = lib.env.base.join("Books");
    let catalogable = generate(&books, 160, 8);
    lib.h.inv.monitor.set(|s| {
        s.power = PowerSource::Battery;
        s.battery_present = Some(true);
        s.battery_percent = Some(50.0);
    });
    lib.add(&books);

    let active = |want: &dyn Fn(&atlas_lib::domain::dto::ExtractionStatus) -> bool| {
        wait_until(LONG, || lib.overview().active.as_ref().is_some_and(want))
    };
    assert!(
        active(&|a| a.state == JobState::Running
            && a.throttle_reason == Some(ThrottleReason::OnBattery)),
        "slowed on battery: {:?}",
        lib.overview().active
    );
    lib.h.inv.monitor.set(|s| s.battery_percent = Some(20.0));
    assert!(active(
        &|a| a.state == JobState::PausedPolicy && a.pause_reason == Some(PauseReason::BatteryLow)
    ));
    let paused_at = lib.overview().counts.done;
    lib.h.inv.monitor.set(|s| s.battery_percent = Some(22.0));
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(
        lib.overview().counts.done,
        paused_at,
        "no reading while paused"
    );
    assert_eq!(
        lib.overview().active.unwrap().state,
        JobState::PausedPolicy,
        "still paused below 25%"
    );
    assert!(paused_at < catalogable, "paused before the end");

    lib.h.inv.monitor.set(|s| s.battery_percent = Some(25.0));
    lib.wait_settled();
    assert_eq!(lib.overview().counts.done, catalogable);
}

/// P1-21: a path over 300 characters (each folder name well under the
/// 255-character limit) is cataloged, read by the worker, searchable and
/// resolvable for Open. On Windows this is past the old 260-character
/// `MAX_PATH`.
#[test]
fn p1_21_a_300_character_path_is_cataloged_and_read() {
    let lib = Lib::new();
    let books = lib.books();
    let mut dir = books.clone();
    let mut depth = 0;
    while dir.as_os_str().len() < 300 {
        depth += 1;
        dir = dir.join(format!(
            "{depth:02}-{}",
            "long folder name ".repeat(3).trim()
        ));
    }
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("a book in a very deep folder.pdf");
    fs::copy(corpus().join("pdf/generated/book.pdf"), &file).unwrap();
    assert!(file.as_os_str().len() > 300, "{}", file.display());
    let before = originals(&books);

    let root = lib.add(&books);
    lib.wait_settled();
    let found = lib
        .files()
        .into_values()
        .find(|f| f.display_name == "a book in a very deep folder.pdf")
        .expect("the long path is cataloged");
    assert!(found.display_path.len() > 300);
    assert_eq!(found.extraction.state, ExtractionState::Done);
    let document = found.document_id.clone().unwrap();

    // Searchable by a word from its own text.
    let word: String = {
        let document = document.clone();
        lib.root()
            .catalog()
            .call(move |c| {
                c.query_row(
                    "SELECT text FROM passages WHERE document_id = ?1 ORDER BY ordinal LIMIT 1",
                    [document],
                    |r| r.get::<_, String>(0),
                )
            })
            .unwrap()
            .unwrap()
            .split(|c: char| !c.is_alphabetic())
            .find(|w| w.chars().count() >= 6)
            .unwrap()
            .to_owned()
    };
    let hits = atlas_lib::services::search::run(
        lib.root().reader(),
        lib.root().root_status(),
        &atlas_lib::domain::dto::SearchQuery {
            text: word.clone(),
            source_root_id: Some(root),
            file_type: None,
            include_missing: false,
            page: 0,
            page_size: 10,
        },
    )
    .unwrap();
    assert!(
        hits.hits
            .iter()
            .any(|h| h.document_id.as_deref() == Some(document.as_str())),
        "searching {word:?} finds the deep file"
    );

    // Open resolves it (the OS call itself is not made in tests).
    let rel = Path::new(&found.display_path)
        .strip_prefix(dunce::canonicalize(&books).unwrap())
        .unwrap()
        .to_path_buf();
    assert!(matches!(
        atlas_lib::services::source_open::resolve(&dunce::canonicalize(&books).unwrap(), &rel),
        Ok(atlas_lib::services::source_open::Resolved::File { .. })
    ));
    assert_eq!(originals(&books), before);
}
