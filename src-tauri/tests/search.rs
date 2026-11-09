//! Slice 1.4: full-text search (phase1-spec §7.4; tests P1-15, P1-16, plus
//! snippets, filters, paging, grouping, name matches and index
//! consistency). Libraries are read by the real worker.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use atlas_lib::domain::dto::{FileType, MatchReason, SearchPage, SearchQuery};
use atlas_lib::services::extraction::db;
use atlas_lib::services::workspace::{OpenRoot, Workspace};
use atlas_lib::services::{inventory, search, sources};

mod common;
use common::{Env, ExtractionHarness, fast_extraction, wait_until, worker};

const LONG: Duration = Duration::from_secs(180);

fn corpus(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../spikes/extraction-bench/corpus")
        .join(rel)
}

struct Lib {
    ws: Workspace,
    _h: ExtractionHarness,
    env: Env,
}

impl Lib {
    fn new() -> Self {
        let h = ExtractionHarness::new(fast_extraction(), worker());
        let env = Env::new();
        let mut ws = env.workspace_with_extraction(&h);
        ws.choose(&env.base.join("AtlasData")).unwrap();
        Self { ws, _h: h, env }
    }

    fn root(&self) -> &OpenRoot {
        self.ws.require_open().unwrap()
    }

    fn folder(&self, name: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = self.env.base.join(name);
        for (to, from) in files {
            let dest = dir.join(to);
            fs::create_dir_all(dest.parent().unwrap()).unwrap();
            fs::copy(corpus(from), dest).unwrap();
        }
        dir
    }

    fn add(&self, dir: &Path) -> String {
        let added = sources::add(self.root(), dir).unwrap();
        inventory::control::enqueue(self.root(), &added.id).unwrap();
        added.id
    }

    fn settle(&self) {
        assert!(wait_until(LONG, || {
            let inv = inventory::control::overview(self.root()).unwrap();
            let ov = self
                .root()
                .reader()
                .read(|c| db::overview(c, None))
                .unwrap()
                .unwrap();
            inv.active.is_none() && ov.active.is_none() && ov.counts.pending == 0
        }));
    }

    fn search(&self, q: SearchQuery) -> SearchPage {
        search::run(self.root().reader(), self.root().root_status(), &q).unwrap()
    }

    fn find(&self, text: &str) -> SearchPage {
        self.search(query(text))
    }

    fn names(&self, text: &str) -> Vec<String> {
        self.find(text)
            .hits
            .into_iter()
            .map(|h| h.display_name)
            .collect()
    }

    fn integrity(&self) {
        self.root()
            .catalog()
            .call(|c| {
                c.execute_batch(
                    "INSERT INTO passages_fts (passages_fts, rank) VALUES ('integrity-check', 1)",
                )
            })
            .unwrap()
            .unwrap();
    }
}

fn query(text: &str) -> SearchQuery {
    SearchQuery {
        text: text.into(),
        source_root_id: None,
        file_type: None,
        include_missing: false,
        page: 0,
        page_size: 10,
    }
}

/// The small library most tests share.
fn library() -> Lib {
    let lib = Lib::new();
    let books = lib.folder(
        "Books",
        &[
            ("features.pdf", "pdf/generated/features.pdf"),
            ("raven.epub", "epub/pg1065.epub"),
            ("prince.epub", "epub/pg1232.epub"),
            ("alice.pdf", "pdf/generated/one-column.pdf"),
            ("copies/alice again.pdf", "pdf/generated/one-column.pdf"),
            ("boeing.pdf", "pdf/real/fr-2026-22817.pdf"),
        ],
    );
    lib.add(&books);
    lib.settle();
    lib
}

/// P1-15: whatever is typed, there is no FTS syntax error; operators are
/// searched as words; only a trailing `*` is a prefix.
#[test]
fn p1_15_typed_operators_are_words_and_never_errors() {
    let lib = library();
    for text in [
        "\"",
        "\"\"\"",
        "*",
        "NEAR(raven",
        "raven NEAR",
        "(raven",
        "raven)",
        "raven AND",
        "OR raven",
        "NOT raven",
        "raven -lenore",
        "title:raven",
        "^raven",
        "raven \"unclosed",
        "{raven}",
        "raven' OR 1=1 --",
        "?",
        "",
        "   ",
    ] {
        let page = lib.find(text);
        assert!(page.hits.len() <= 10, "{text:?}");
    }
    // Punctuation only: an empty page, not an error.
    assert!(lib.find("?! -- ()").hits.is_empty());
    // A trailing star is a prefix; elsewhere it is part of a word.
    assert!(lib.names("Neverm*").contains(&"raven.epub".to_owned()));
    assert!(!lib.names("Neverm").contains(&"raven.epub".to_owned()));
    assert!(lib.names("*Nevermore").contains(&"raven.epub".to_owned()));
}

/// P1-16: phrases are respected; case and accents are ignored.
#[test]
fn p1_16_phrases_accents_and_case() {
    let lib = library();
    for text in ["\"café crème\"", "\"CAFE CREME\"", "cafe creme", "CAFÉ"] {
        assert_eq!(
            lib.names(text).first().map(String::as_str),
            Some("features.pdf"),
            "{text}"
        );
    }
    // The phrase in the wrong order is not the phrase.
    let reversed = lib.find("\"crème café\"");
    assert!(
        reversed
            .hits
            .iter()
            .all(|h| !h.all_words || h.display_name != "features.pdf"),
        "{:?}",
        reversed.hits
    );
    assert_eq!(
        lib.names("\"bleak December\"").first().map(String::as_str),
        Some("raven.epub")
    );
}

#[test]
fn snippets_are_spans_with_the_hits_marked() {
    let lib = library();
    let page = lib.find("Nevermore");
    let hit = &page.hits[0];
    assert_eq!(hit.display_name, "raven.epub");
    assert!(hit.location.as_deref().unwrap().starts_with("Chapter "));
    assert!(
        hit.snippet
            .iter()
            .any(|s| s.hit && s.text.eq_ignore_ascii_case("Nevermore"))
    );
    for span in &hit.snippet {
        assert!(!span.text.contains('<') && !span.text.contains('\u{E000}'));
    }
    let pdf = lib.find("coaxial cables");
    assert_eq!(pdf.hits[0].display_name, "boeing.pdf");
    assert!(pdf.hits[0].page_no.is_some());
    assert!(
        pdf.hits[0]
            .location
            .as_deref()
            .unwrap()
            .starts_with("Page ")
    );
}

#[test]
fn copies_are_one_result_with_their_other_locations() {
    let lib = library();
    let page = lib.find("\"Down the Rabbit-Hole\"");
    let alice: Vec<_> = page
        .hits
        .iter()
        .filter(|h| h.display_name.contains("alice"))
        .collect();
    assert_eq!(alice.len(), 1, "{:?}", page.hits);
    assert_eq!(alice[0].other_locations, 1);
}

#[test]
fn filters_and_paging() {
    let lib = library();
    let other = lib.folder("Other", &[("raven copy.epub", "epub/pg1065.epub")]);
    let other_id = lib.add(&other);
    lib.settle();

    let in_other = lib.search(SearchQuery {
        source_root_id: Some(other_id),
        ..query("Nevermore")
    });
    assert_eq!(in_other.hits.len(), 1);
    assert_eq!(in_other.hits[0].display_name, "raven copy.epub");

    let pdfs = lib.search(SearchQuery {
        file_type: Some(FileType::Pdf),
        ..query("the")
    });
    assert!(!pdfs.hits.is_empty());
    assert!(pdfs.hits.iter().all(|h| h.file_type == FileType::Pdf));

    // Pages don't overlap and add up to the documents matched.
    let first = lib.search(SearchQuery {
        page_size: 2,
        ..query("the")
    });
    let second = lib.search(SearchQuery {
        page_size: 2,
        page: 1,
        ..query("the")
    });
    assert_eq!(first.hits.len(), 2);
    assert!(first.matched_documents >= 3);
    for h in &second.hits {
        assert!(first.hits.iter().all(|f| f.document_id != h.document_id));
    }
}

#[test]
fn file_names_are_found_too() {
    let lib = library();
    // In the name and the text.
    let page = lib.find("raven");
    let raven = page
        .hits
        .iter()
        .find(|h| h.display_name == "raven.epub")
        .unwrap();
    assert_eq!(raven.why, MatchReason::Both);
    // Only in the name: a file that has no text.
    let notes = lib.env.base.join("Books/quarterly notes.pdf");
    fs::write(&notes, b"not really a pdf").unwrap();
    let root_id = lib
        .root()
        .reader()
        .read(|c| c.query_row("SELECT id FROM source_roots", [], |r| r.get::<_, String>(0)));
    inventory::control::enqueue(lib.root(), &root_id.unwrap().unwrap()).unwrap();
    lib.settle();
    let page = lib.find("quarterly");
    assert_eq!(page.hits.len(), 1);
    assert_eq!(page.hits[0].why, MatchReason::Name);
    assert!(page.hits[0].snippet.is_empty());
}

/// The index stays consistent through rescans with changed content and the
/// removal of a folder.
#[test]
fn the_index_stays_consistent() {
    let lib = library();
    lib.integrity();
    let books = lib.env.base.join("Books");
    // New content under an old name.
    fs::copy(
        corpus("pdf/real/fr-2026-22772.pdf"),
        books.join("boeing.pdf"),
    )
    .unwrap();
    let f = fs::OpenOptions::new()
        .write(true)
        .open(books.join("boeing.pdf"))
        .unwrap();
    f.set_modified(std::time::SystemTime::now() + Duration::from_secs(60))
        .unwrap();
    let root_id: String = lib
        .root()
        .reader()
        .read(|c| c.query_row("SELECT id FROM source_roots", [], |r| r.get(0)))
        .unwrap()
        .unwrap();
    inventory::control::enqueue(lib.root(), &root_id).unwrap();
    lib.settle();
    lib.integrity();
    assert_eq!(
        lib.names("\"Single Family Housing\"")
            .first()
            .map(String::as_str),
        Some("boeing.pdf")
    );
    sources::remove(lib.root(), &root_id).unwrap();
    lib.integrity();
    assert!(lib.find("Nevermore").hits.is_empty());
}
