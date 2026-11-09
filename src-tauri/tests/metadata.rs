//! Slice 1.6, P1-18: titles and authors (phase1-spec §5.2, §5.3). Titles
//! come from the file's metadata when usable, then from a PDF's first page,
//! then from the file name; junk metadata is cleaned; EPUB text carries no
//! Markdown marks; snippets carry no markup. Runs the real worker.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use atlas_lib::domain::dto::{SearchQuery, TitleSource};
use atlas_lib::services::extraction::db;
use atlas_lib::services::workspace::{OpenRoot, Workspace};
use atlas_lib::services::{documents, inventory, search, sources};

mod common;
use common::{Env, ExtractionHarness, fast_extraction, wait_until, worker};

const LONG: Duration = Duration::from_secs(180);

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn node(args: &[&str]) -> String {
    let out = Command::new("node").args(args).output().expect("node runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A text PDF with the given Info title and author.
fn pdf_with(path: &Path, title: &str, author: &str) {
    let lib = repo().join("scripts/lib/fixture-text.mjs");
    // A file:// URL: Node's loader refuses a bare `D:\...` path on Windows.
    let script = "import fs from 'node:fs';
         import { pathToFileURL } from 'node:url';
         const [lib, out, title, author] = process.argv.slice(1);
         const { textPdf, bodyText } = await import(pathToFileURL(lib).href);
         fs.writeFileSync(out, textPdf({ title, author, paragraphs: bodyText('Notes', 99, 4) }).bytes);";
    node(&[
        "--input-type=module",
        "-e",
        script,
        &lib.to_string_lossy(),
        &path.to_string_lossy(),
        title,
        author,
    ]);
}

struct Lib {
    ws: Workspace,
    _h: ExtractionHarness,
    _env: Env,
}

impl Lib {
    fn read(books: impl FnOnce(&Path), env: Env) -> Self {
        let h = ExtractionHarness::new(fast_extraction(), worker());
        let mut ws = env.workspace_with_extraction(&h);
        ws.choose(&env.base.join("AtlasData")).unwrap();
        let dir = env.base.join("Books");
        fs::create_dir_all(&dir).unwrap();
        books(&dir);
        let root = ws.require_open().unwrap();
        let added = sources::add(root, &dir).unwrap();
        inventory::control::enqueue(root, &added.id).unwrap();
        let lib = Self {
            ws,
            _h: h,
            _env: env,
        };
        assert!(wait_until(LONG, || lib.settled()), "never settled");
        lib
    }

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
        inv.active.is_none() && ov.active.is_none() && ov.counts.pending == 0
    }

    fn file_id(&self, name: &str) -> String {
        let name = name.to_owned();
        self.root()
            .reader()
            .read(move |c| {
                c.query_row(
                    "SELECT id FROM file_instances WHERE display_name = ?1",
                    [name],
                    |r| r.get::<_, String>(0),
                )
            })
            .unwrap()
            .unwrap_or_else(|e| panic!("{e}"))
    }

    fn title(&self, name: &str) -> (String, TitleSource, Vec<String>) {
        let d = documents::detail(
            self.root().reader(),
            self.root().root_status(),
            &self.file_id(name),
        )
        .unwrap();
        (d.title, d.title_source, d.authors)
    }
}

/// For comparing titles: whitespace, case, a hyphen at a line break and a
/// final full stop don't matter.
fn words(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        .replace("- ", "-")
        .trim_end_matches('.')
        .to_owned()
}

#[test]
fn p1_18_titles_come_from_metadata_then_the_first_page_then_the_file_name() {
    let env = Env::new();
    let generated = env.base.join("generated");
    node(&[
        &repo()
            .join("scripts/generate-fixtures.mjs")
            .to_string_lossy(),
        &generated.to_string_lossy(),
        "--files",
        "12",
        "--text-kb",
        "4",
        "--untitled-every",
        "4",
    ]);
    let stub = env.base.join("stub");
    node(&[
        &repo()
            .join("scripts/generate-fixtures.mjs")
            .to_string_lossy(),
        &stub.to_string_lossy(),
        "--files",
        "1",
    ]);
    let find = |dir: &Path, name: &str| -> PathBuf {
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
    };
    let lib = Lib::read(
        |books| {
            for name in [
                "book 000006.epub",
                "book 000003.EPUB",
                "book 000007.pdf",
                "book 000001.pdf",
            ] {
                fs::copy(find(&generated, name), books.join(name)).unwrap();
            }
            // A one-line PDF with no Info dictionary and no title-sized text.
            fs::copy(
                find(&stub, "Introduction.pdf"),
                books.join("scan_notes-draft_v2.pdf"),
            )
            .unwrap();
            pdf_with(
                &books.join("word export.pdf"),
                "Microsoft Word - Notes on gears.docx",
                "Administrator; Lin, Ada",
            );
            pdf_with(&books.join("untitled.pdf"), "Untitled", "Bo Okafor");
        },
        env,
    );

    // Metadata titles.
    let (t, s, a) = lib.title("book 000001.pdf");
    assert_eq!(
        (t.as_str(), s),
        ("Engineering book 000001", TitleSource::Metadata)
    );
    assert_eq!(a, ["Bo Okafor"]);
    // Junk metadata is cleaned: the Word prefix and extension go, and the
    // account name is dropped from the authors.
    let (t, s, a) = lib.title("word export.pdf");
    assert_eq!((t.as_str(), s), ("Notes on gears", TitleSource::Metadata));
    assert_eq!(a, ["Lin, Ada"]);
    // No metadata title: the first page's large title.
    let (t, s, _) = lib.title("book 000007.pdf");
    assert_eq!(s, TitleSource::Contents);
    assert_eq!(t, "Engineering book 000007");
    // A placeholder title with no large first-page text: the file name.
    assert_eq!(
        lib.title("untitled.pdf"),
        (
            "untitled".into(),
            TitleSource::FileName,
            vec!["Bo Okafor".into()]
        )
    );
    let (t, s, _) = lib.title("scan_notes-draft_v2.pdf");
    assert_eq!(
        (t.as_str(), s),
        ("scan notes-draft v2", TitleSource::FileName)
    );
    // An EPUB without dc:title: the file name.
    let (t, s, _) = lib.title("book 000003.EPUB");
    assert_eq!((t.as_str(), s), ("book 000003", TitleSource::FileName));
    let (t, s, _) = lib.title("book 000006.epub");
    assert_eq!(
        (t.as_str(), s),
        ("Astronomy book 000006", TitleSource::Metadata)
    );

    // Search results carry the same title and source.
    let q = |text: &str| SearchQuery {
        text: text.into(),
        source_root_id: None,
        file_type: None,
        include_missing: false,
        page: 0,
        page_size: 20,
    };
    let root = lib.root();
    let page = search::run(
        root.reader(),
        root.root_status(),
        &q("\"Engineering marker 7\""),
    )
    .unwrap();
    assert_eq!(page.hits.len(), 1);
    assert_eq!(page.hits[0].title, "Engineering book 000007");
    assert_eq!(page.hits[0].title_source, TitleSource::Contents);

    // EPUB text has no Markdown marks; snippets are plain spans.
    let marked: i64 = root
        .reader()
        .read(|c| {
            c.query_row(
                "SELECT COUNT(*) FROM passages p JOIN file_instances f ON f.document_id = p.document_id
                 WHERE f.file_type = 'epub' AND (p.text LIKE '#%' OR p.text LIKE '%\n#%'
                       OR p.text LIKE '%[%]%')",
                [],
                |r| r.get(0),
            )
        })
        .unwrap()
        .unwrap();
    assert_eq!(marked, 0);
    let page = search::run(
        root.reader(),
        root.root_status(),
        &q("\"Astronomy marker 6\""),
    )
    .unwrap();
    assert_eq!(page.hits.len(), 1);
    let snippet: String = page.hits[0]
        .snippet
        .iter()
        .map(|s| s.text.as_str())
        .collect();
    assert!(
        snippet.starts_with("Chapter 1") || snippet.starts_with('…'),
        "{snippet:?}"
    );
    for mark in ['#', '<', '>', '\u{E000}', '\u{E001}'] {
        assert!(!snippet.contains(mark), "{mark:?} in {snippet:?}");
    }
}

#[test]
fn p1_18_papers_get_their_titles_from_the_first_page() {
    let corpus = repo().join("spikes/extraction-bench/corpus");
    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(corpus.join("manifest.json")).unwrap()).unwrap();
    let docs = manifest
        .as_array()
        .cloned()
        .or_else(|| manifest["documents"].as_array().cloned())
        .expect("manifest lists documents");
    let papers: Vec<(String, String)> = docs
        .iter()
        .filter(|d| {
            d["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("pdf-arxiv-"))
        })
        .map(|d| {
            (
                d["path"].as_str().unwrap().to_owned(),
                d["note"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(papers.len(), 6);
    let lib = Lib::read(
        |books| {
            for (path, _) in &papers {
                let src = corpus.join(path);
                fs::copy(&src, books.join(src.file_name().unwrap())).unwrap();
            }
        },
        Env::new(),
    );
    let mut report = Vec::new();
    for (path, expected) in &papers {
        let name = Path::new(path)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let (title, source, _) = lib.title(&name);
        report.push(format!(
            "{name}: {source:?} {title:?} (expected {expected:?})"
        ));
        let ok = matches!(source, TitleSource::Metadata | TitleSource::Contents)
            && words(&title) == words(expected);
        assert!(ok, "{}", report.join("\n"));
        // The first-page rule finds the same title on its own (these papers
        // also have it in their metadata).
        let id = lib.file_id(&name);
        let content: Option<String> = lib
            .root()
            .reader()
            .read(move |c| {
                c.query_row(
                    "SELECT d.content_title FROM documents d
                     JOIN file_instances f ON f.document_id = d.id WHERE f.id = ?1",
                    [id],
                    |r| r.get(0),
                )
            })
            .unwrap()
            .unwrap();
        report.push(format!("  first page: {content:?}"));
        let content = content.unwrap_or_default();
        assert_eq!(words(&content), words(expected), "{}", report.join("\n"));
    }
    println!("{}", report.join("\n"));
}
