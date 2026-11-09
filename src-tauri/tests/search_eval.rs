//! P1-17: the committed evaluation questions (`fixtures/evaluation/
//! questions.json`) against a library built from the spike corpus and read
//! by the real worker. A question passes when a relevant document is in the
//! top 5; at least 80% must pass. Every question's rank is printed, so a
//! regression names the question.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use atlas_lib::domain::dto::SearchQuery;
use atlas_lib::services::extraction::db;
use atlas_lib::services::{inventory, search, sources};

mod common;
use common::{Env, ExtractionHarness, fast_extraction, wait_until, worker};

const LONG: Duration = Duration::from_secs(180);
const PASS: f64 = 0.80;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
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

#[test]
fn p1_17_evaluation_questions_hit_at_5() {
    let corpus = repo().join("spikes/extraction-bench/corpus");
    let manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(corpus.join("manifest.json")).unwrap()).unwrap();
    let questions: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(repo().join("fixtures/evaluation/questions.json")).unwrap(),
    )
    .unwrap();

    let h = ExtractionHarness::new(fast_extraction(), worker());
    let env = Env::new();
    let mut ws = env.workspace_with_extraction(&h);
    ws.choose(&env.base.join("AtlasData")).unwrap();
    let books = env.base.join("Books");
    for dir in ["epub", "pdf"] {
        copy_tree(&corpus.join(dir), &books.join(dir));
    }
    let root = ws.require_open().unwrap();
    let added = sources::add(root, &books).unwrap();
    inventory::control::enqueue(root, &added.id).unwrap();
    assert!(
        wait_until(LONG, || {
            let inv = inventory::control::overview(root).unwrap();
            let ov = root
                .reader()
                .read(|c| db::overview(c, None))
                .unwrap()
                .unwrap();
            inv.active.is_none()
                && ov.active.is_none()
                && ov.counts.total > 0
                && ov.counts.pending == 0
        }),
        "the corpus was not read"
    );
    let books = dunce::canonicalize(&books).unwrap();
    let path_of = |id: &str| -> PathBuf {
        let doc = manifest["documents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["id"] == id)
            .unwrap_or_else(|| panic!("unknown document {id}"));
        books.join(doc["path"].as_str().unwrap())
    };

    let mut passed = 0usize;
    let mut report = Vec::new();
    let list = questions["questions"].as_array().unwrap();
    for q in list {
        let text = q["question"].as_str().unwrap();
        let relevant: Vec<PathBuf> = q["relevant"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| path_of(r.as_str().unwrap()))
            .collect();
        let page = root
            .reader()
            .read(|c| {
                search::search(
                    c,
                    &SearchQuery {
                        text: text.to_owned(),
                        source_root_id: None,
                        file_type: None,
                        include_missing: false,
                        page: 0,
                        page_size: 10,
                    },
                )
            })
            .unwrap()
            .unwrap();
        let rank = page.hits.iter().position(|hit| {
            // A hit stands for its document; any of its copies may be the
            // relevant file.
            let shown = PathBuf::from(&hit.display_path);
            relevant.contains(&shown)
                || hit.document_id.as_ref().is_some_and(|doc| {
                    relevant.iter().any(|r| {
                        root.reader()
                            .read(|c| {
                                c.query_row(
                                    "SELECT EXISTS (SELECT 1 FROM file_instances
                                         WHERE document_id = ?1 AND display_path = ?2)",
                                    rusqlite::params![doc, r.to_string_lossy()],
                                    |row| row.get::<_, bool>(0),
                                )
                                .unwrap()
                            })
                            .unwrap()
                    })
                })
        });
        let ok = rank.is_some_and(|r| r < 5);
        let via = rank.map_or("", |r| {
            if page.hits[r].all_words {
                "all words"
            } else {
                "any words"
            }
        });
        passed += usize::from(ok);
        report.push(format!(
            "{} {:<10} {:<6} {:<9} {}",
            q["id"].as_str().unwrap(),
            q["kind"].as_str().unwrap(),
            rank.map_or("-".to_owned(), |r| (r + 1).to_string()),
            via,
            text
        ));
    }
    // The bar is not trivially met: nonsense finds nothing.
    let nothing = root
        .reader()
        .read(|c| {
            search::search(
                c,
                &SearchQuery {
                    text: "zqxjv plorbix".into(),
                    source_root_id: None,
                    file_type: None,
                    include_missing: false,
                    page: 0,
                    page_size: 10,
                },
            )
        })
        .unwrap()
        .unwrap();
    assert!(nothing.hits.is_empty(), "{:?}", nothing.hits);
    let score = passed as f64 / list.len() as f64;
    eprintln!("{}", report.join("\n"));
    eprintln!("hit@5: {passed}/{} = {score:.3}", list.len());
    assert!(score >= PASS, "hit@5 {score:.3} is below {PASS}");
}
