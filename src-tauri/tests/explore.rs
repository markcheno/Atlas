//! Slice 4.1.2, the Explore spike (docs/spike-explore.md): can the
//! library be laid out by meaning, as named areas on a map?
//!
//! The harness here reads a library's catalog, read-only, and writes one
//! page to look at: the map, the areas with their names, a tick for each,
//! and the numbers to share. The owner runs it on the real library
//! (`just spike-explore <data root>`); the tests run it on a small one
//! made with the fake embedder.
//!
//! What it prints holds counts, measures and timings only. Titles and
//! area names (words from books) go to the page and nowhere else.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

use atlas_lib::error::AtlasError;
use atlas_lib::services::catalog::migrations::{self, SUPPORTED_SCHEMA_VERSION};
use atlas_lib::services::catalog::reader::open_offline;
use atlas_lib::services::documents::related;
use atlas_lib::services::embedding::db as edb;
use atlas_lib::services::embedding::fake::HashedBagOfWordsBackend;
use atlas_lib::services::explore::books::{self, Sampling};
use atlas_lib::services::explore::survey::{self, Options, Survey};
use atlas_lib::services::explore::synthetic::planted;
use atlas_lib::services::workspace::layout::CATALOG;
use rusqlite::Connection;
use serde_json::{Value, json};

mod common;
use common::vectors::Lib;

const DIMS: usize = 64;
const PAGE: &str = include_str!("explore/page.html");
const PLACEHOLDER: &str = "\"__ATLAS_EXPLORE_DATA__\"";
const PAGE_NAME: &str = "atlas-explore.html";

// ------------------------------------------------------------ the harness

/// What a run leaves: the page, and what was printed.
#[derive(Debug)]
struct Outcome {
    page: PathBuf,
    /// Numbers only: safe to paste anywhere.
    printed: Value,
    survey: Survey,
}

/// `path` with `..` and symlinks resolved as far as it exists.
fn resolved(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut at = path.to_path_buf();
    loop {
        // Without Windows' `\\?\` prefix, so `starts_with` compares like
        // with like.
        if let Ok(real) = dunce::canonicalize(&at) {
            return rest.iter().rev().fold(real, |p, c| p.join(c));
        }
        match (at.file_name(), at.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(name.to_owned());
                at = parent.to_path_buf();
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// The page holds titles, so it may not land in the library (where a
/// backup or an export would carry it) or among the repository's files.
fn check_output(out: &Path, data_root: &Path) -> Result<PathBuf, String> {
    if out.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("the output folder may not contain `..`".into());
    }
    let out = resolved(out);
    if out.starts_with(resolved(data_root)) {
        return Err("the output folder is inside the library: choose one outside it".into());
    }
    let crate_dir = resolved(Path::new(env!("CARGO_MANIFEST_DIR")));
    let repo = crate_dir.parent().unwrap_or(&crate_dir).to_path_buf();
    if out.starts_with(&repo) && !out.starts_with(crate_dir.join("target")) {
        return Err(
            "the output folder is inside the repository: the page holds titles, so choose \
             src-tauri/target or a folder outside the repository"
                .into(),
        );
    }
    Ok(out)
}

/// JSON for a `<script>` block: nothing in it can end the block or start
/// a tag, whatever a title holds.
fn for_script(value: &Value) -> String {
    serde_json::to_string(value)
        .expect("JSON")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

fn round(x: f32) -> f64 {
    (f64::from(x) * 1000.0).round() / 1000.0
}

/// Surveys the library at `data_root` and writes the page into `out`.
/// `model` is a row's position (1 is the oldest); without it, the row
/// with the most documents done.
fn explore(
    data_root: &Path,
    out: &Path,
    opts: &Options,
    model: Option<usize>,
) -> Result<Outcome, String> {
    let out = check_output(out, data_root)?;
    let conn = open_offline(&data_root.join(CATALOG)).map_err(|e| match e {
        AtlasError::DataRootIncompatible { found, supported } => format!(
            "this library's catalog is at format {found} and this build reads format \
             {supported}: run the spike from the commit the installed Atlas was built from"
        ),
        other => other.to_string(),
    })?;
    let db = |e: rusqlite::Error| format!("the catalog couldn't be read: {e}");
    let rows = books::models(&conn).map_err(db)?;
    let position = match model {
        Some(p) if (1..=rows.len()).contains(&p) => p,
        Some(p) => return Err(format!("there is no model {p}: {} listed", rows.len())),
        None => rows
            .iter()
            .enumerate()
            .max_by_key(|(i, r)| (r.done, *i))
            .map(|(i, _)| i + 1)
            .ok_or("this library has no embedding model yet")?,
    };
    let model = edb::get_model(&conn, &rows[position - 1].id)
        .map_err(db)?
        .ok_or("the model row went away")?;
    let survey = survey::run(&conn, &model, opts)
        .map_err(db)?
        .ok_or("fewer than three books are indexed for meaning: nothing to lay out")?;

    let area_of: BTreeMap<u32, (usize, usize)> = survey
        .areas
        .iter()
        .enumerate()
        .flat_map(|(a, area)| {
            // The sub-area of each member; 0 when the area has none.
            let mut sub = BTreeMap::new();
            for (s, child) in area.children.iter().enumerate() {
                for &m in &child.members {
                    sub.insert(m, s);
                }
            }
            area.members
                .iter()
                .map(move |&m| (m, (a, sub.get(&m).copied().unwrap_or(0))))
                .collect::<Vec<_>>()
        })
        .collect();
    let numbers = serde_json::to_value(&survey.numbers).expect("numbers");
    let printed = json!({ "models": rows, "model": position, "numbers": numbers });
    let data = json!({
        "run": format!("{}-{}", survey.numbers.books, survey.numbers.timings.load_ms),
        "models": rows,
        "model": position,
        "numbers": numbers,
        "books": survey.xy.iter().enumerate().map(|(i, p)| {
            let (area, sub) = area_of[&(i as u32)];
            json!([round(p[0]), round(p[1]), area, sub, survey.titles[i]])
        }).collect::<Vec<_>>(),
        "areas": survey.areas.iter().zip(&survey.names).map(|(area, name)| json!({
            "words": name.words,
            "titles": name.titles,
            "size": area.members.len(),
            "cohesion": round(area.cohesion),
            "subs": area.children.iter().zip(&name.sub_areas).map(|(c, words)| json!({
                "words": words,
                "size": c.members.len(),
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    });
    assert_eq!(
        PAGE.matches(PLACEHOLDER).count(),
        1,
        "the page has one place for its data"
    );
    let page = out.join(PAGE_NAME);
    fs::create_dir_all(&out).map_err(|e| format!("the output folder couldn't be made: {e}"))?;
    fs::write(&page, PAGE.replace(PLACEHOLDER, &for_script(&data)))
        .map_err(|e| format!("the page couldn't be written: {e}"))?;
    Ok(Outcome {
        page,
        printed,
        survey,
    })
}

// ------------------------------------------------------------- a library

const TOPICS: [[&str; 6]; 3] = [
    ["orbit", "telescope", "comet", "galaxy", "nebula", "planet"],
    ["bread", "yeast", "flour", "oven", "crust", "dough"],
    ["violin", "sonata", "tempo", "chord", "melody", "rhythm"],
];
const FILLER: [&str; 6] = ["notes", "study", "history", "method", "theory", "practice"];

/// A page of a book on `topic`: mostly the topic's words, some that
/// every book uses, and one word of the book's own.
fn page(topic: usize, book: usize, page: usize) -> String {
    let mut x = (topic * 1_000 + book * 10 + page) as u64 * 2_654_435_761 + 1;
    let mut out = String::new();
    for i in 0..160 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let word = match x % 10 {
            0..=6 => TOPICS[topic][(x >> 8) as usize % 6],
            7..=8 => FILLER[(x >> 8) as usize % 6],
            _ => &format!("own{topic}x{book}"),
        };
        out.push_str(word);
        out.push_str(if i % 12 == 11 { ". " } else { " " });
    }
    out
}

/// A library of `per_topic` books on each of the three topics, embedded
/// with the fake embedder. Returns it with its model and document ids
/// (topic after topic).
fn library(per_topic: usize) -> (Lib, String, Vec<String>) {
    let lib = Lib::new();
    let backend = HashedBagOfWordsBackend::new(DIMS);
    let model = lib.create_model(DIMS);
    let mut docs = Vec::new();
    for (topic, words) in TOPICS.iter().enumerate() {
        for book in 0..per_topic {
            let pages: Vec<String> = (0..3).map(|p| page(topic, book, p)).collect();
            let pages: Vec<&str> = pages.iter().map(String::as_str).collect();
            let name = format!("{} volume {book}.pdf", words[0]);
            let doc = lib.read(&name, format!("{topic}-{book}").as_bytes(), &pages);
            lib.embed(&backend, &model, &doc);
            docs.push(doc);
        }
    }
    (lib, model, docs)
}

fn three_areas() -> Options {
    Options {
        areas: 3,
        sub_areas: 2,
        neighbours: 8,
        threads: 2,
        ..Options::default()
    }
}

/// Every file under `root` with its contents' hash.
fn files(root: &Path) -> BTreeMap<PathBuf, String> {
    fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<PathBuf, String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                out.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    blake3::hash(&fs::read(&path).unwrap()).to_hex().to_string(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn is_side_file(path: &Path, suffix: &str) -> bool {
    path.to_string_lossy().ends_with(suffix)
}

// ----------------------------------------------------------------- tests

#[test]
fn book_vectors_match_related_documents() {
    let lib = Lib::new();
    let backend = HashedBagOfWordsBackend::new(DIMS);
    let model_id = lib.create_model(DIMS);
    // One book of more chunks than the sample, one of fewer.
    let long: Vec<String> = (0..40).map(|p| common::vectors::prose(p, 500)).collect();
    let long: Vec<&str> = long.iter().map(String::as_str).collect();
    let short = [common::vectors::prose(99, 200)];
    let short: Vec<&str> = short.iter().map(String::as_str).collect();
    let a = lib.read("long.pdf", b"long", &long);
    let b = lib.read("short.pdf", b"short", &short);
    for d in [&a, &b] {
        lib.embed(&backend, &model_id, d);
    }
    let conn = lib.conn();
    let model = edb::get_model(&conn, &model_id).unwrap().unwrap();
    let (found, coverage) = books::book_vectors(&conn, &model, Sampling::Sample).unwrap();
    assert_eq!((coverage.with_text, coverage.used), (2, 2));
    assert_eq!(found.ids.len(), 2);
    assert!(
        found.chunks.iter().any(|&c| c as usize > related::SAMPLE),
        "{:?}",
        found.chunks
    );
    for (i, id) in found.ids.iter().enumerate() {
        let vectors: Vec<Vec<f32>> = edb::document_vectors(&conn, &model.id, DIMS, id)
            .unwrap()
            .into_iter()
            .map(|(_, v)| v)
            .collect();
        assert_eq!(vectors.len(), found.chunks[i] as usize);
        let expected = related::mean_vector(&vectors).unwrap();
        for (x, y) in found.vectors.row(i).iter().zip(&expected) {
            assert!((x - y).abs() < 1e-6, "book {i}: {x} against {y}");
        }
    }
    // Every chunk gives another vector for the long book, still of unit
    // length.
    let (all, _) = books::book_vectors(&conn, &model, Sampling::All).unwrap();
    let long_at = found.ids.iter().position(|id| *id == a).unwrap();
    assert_ne!(all.vectors.row(long_at), found.vectors.row(long_at));
    let len: f32 = all.vectors.row(long_at).iter().map(|x| x * x).sum();
    assert!((len - 1.0).abs() < 1e-4);
}

#[test]
fn documents_without_vectors_are_counted_not_used() {
    let (lib, model_id, docs) = library(2);
    let waiting = lib.read("unindexed.pdf", b"unindexed", &["bread yeast flour oven"]);
    let conn = lib.conn();
    let model = edb::get_model(&conn, &model_id).unwrap().unwrap();
    let (found, coverage) = books::book_vectors(&conn, &model, Sampling::Sample).unwrap();
    assert_eq!(coverage.with_text, 7);
    assert_eq!((coverage.done, coverage.waiting), (6, 1));
    assert_eq!((coverage.used, coverage.skipped), (6, 0));
    assert!(!found.ids.contains(&waiting));
    let mut expected = docs.clone();
    expected.sort();
    assert_eq!(found.ids, expected, "in document order");
    let rows = books::models(&conn).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].dims, rows[0].done), (DIMS, 6));
}

#[test]
fn planted_topics_are_found_end_to_end() {
    let (lib, _, docs) = library(15);
    let out = tempfile::tempdir().unwrap();
    let outcome = explore(&lib.root, out.path(), &three_areas(), None).unwrap();
    let s = &outcome.survey;
    assert_eq!(s.areas.len(), 3);
    // Each area is one topic's books.
    let topic_of = |id: &String| docs.iter().position(|d| d == id).unwrap() / 15;
    let mut seen = Vec::new();
    for (area, name) in s.areas.iter().zip(&s.names) {
        assert_eq!(area.members.len(), 15);
        let topics: Vec<usize> = area
            .members
            .iter()
            .map(|&m| topic_of(&s.books.ids[m as usize]))
            .collect();
        assert!(topics.iter().all(|t| *t == topics[0]), "{topics:?}");
        seen.push(topics[0]);
        // Named by the topic's own words, never by what every book uses
        // or what one book alone uses.
        assert!(name.words.len() >= 3, "{:?}", name.words);
        for word in &name.words {
            assert!(TOPICS[topics[0]].contains(&word.as_str()), "{word}");
        }
        assert_eq!(name.titles.len(), 3);
        for title in &name.titles {
            assert!(
                title.to_lowercase().contains(TOPICS[topics[0]][0]),
                "{title}"
            );
        }
    }
    seen.sort_unstable();
    assert_eq!(seen, [0, 1, 2]);
    let n = &s.numbers;
    assert_eq!((n.books, n.coverage.used), (45, 45));
    assert!((n.covered - 1.0).abs() < 1e-6);
    assert!(n.map.area_agreement >= 0.9, "{:?}", n.map);
    assert!(n.map.trustworthiness >= 0.8, "{:?}", n.map);
    assert!(n.silhouette > 0.2, "{}", n.silhouette);
    assert!(outcome.page.is_file());
}

#[test]
fn the_printed_numbers_hold_no_titles() {
    let (lib, _, _) = library(5);
    let out = tempfile::tempdir().unwrap();
    let outcome = explore(&lib.root, out.path(), &three_areas(), None).unwrap();
    let printed = outcome.printed.to_string().to_lowercase();
    for word in TOPICS
        .iter()
        .flatten()
        .chain(&FILLER)
        .chain(&["volume", "fake", "own"])
    {
        assert!(!printed.contains(word), "{word} in {printed}");
    }
    // Neither the model's name nor its id is printed: its position is.
    assert_eq!(outcome.printed["model"], 1);
    assert_eq!(outcome.printed["models"][0]["status"], "building");
    assert!(outcome.printed["models"][0].get("id").is_none());
    assert_eq!(outcome.printed["numbers"]["spikeVersion"], 1);
    // The page has what the terminal doesn't.
    let page = fs::read_to_string(&outcome.page).unwrap();
    assert!(
        page.to_lowercase().contains("orbit volume"),
        "titles are on the page"
    );
}

#[test]
fn the_page_is_self_contained_and_escapes_titles() {
    let (lib, _, docs) = library(5);
    let title = "Gears <script>alert(1)<script> & more";
    let doc = docs[0].clone();
    lib.writer
        .call(move |c| {
            c.execute(
                "UPDATE documents SET title = ?1 WHERE id = ?2",
                rusqlite::params![title, doc],
            )
            .unwrap();
        })
        .unwrap();
    let out = tempfile::tempdir().unwrap();
    let outcome = explore(&lib.root, out.path(), &three_areas(), None).unwrap();
    assert!(outcome.survey.titles.iter().any(|t| t == title));
    let page = fs::read_to_string(&outcome.page).unwrap();
    assert!(!page.contains(PLACEHOLDER));
    assert!(!page.contains("<script>alert"), "a title became markup");
    assert!(page.contains("\\u003cscript\\u003ealert(1)"));
    assert_eq!(
        page.matches("</script>").count(),
        PAGE.matches("</script>").count(),
        "no title can end the data block"
    );
    // Nothing is fetched: no address, and a policy that allows none.
    assert!(!page.contains("http://") && !page.contains("https://"));
    assert!(page.contains("default-src 'none'"));
    // Whatever a title holds, the data block still ends where it should.
    let hostile = for_script(&json!(["</script><img src=x>", "a\u{2028}b"]));
    assert!(!hostile.contains('<') && !hostile.contains('>'));
    assert!(!hostile.contains('\u{2028}'));
    assert_eq!(
        serde_json::from_str::<Value>(&hostile).unwrap(),
        json!(["</script><img src=x>", "a\u{2028}b"])
    );
}

#[test]
fn the_spike_refuses_another_schema_version() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("data");
    fs::create_dir_all(root.join("library")).unwrap();
    let mut conn = Connection::open(root.join(CATALOG)).unwrap();
    migrations::migrate_to(&mut conn, SUPPORTED_SCHEMA_VERSION - 1, "t").unwrap();
    drop(conn);
    let before = files(&root);
    let out = dir.path().join("out");
    let err = explore(&root, &out, &three_areas(), None).unwrap_err();
    assert!(
        err.contains(&format!("format {}", SUPPORTED_SCHEMA_VERSION - 1)),
        "{err}"
    );
    assert_eq!(files(&root), before);
    assert!(!out.exists(), "nothing is written for a refused library");
    // No catalog at all is said plainly too.
    let empty = dir.path().join("empty");
    fs::create_dir_all(&empty).unwrap();
    assert!(explore(&empty, &out, &three_areas(), None).is_err());
    assert!(!out.exists());
}

#[test]
fn the_spike_writes_nothing_to_a_closed_library() {
    let (lib, _, _) = library(5);
    let Lib {
        dir, root, writer, ..
    } = lib;
    // As after quitting Atlas: the last connection closes and the
    // write-ahead log is folded in.
    drop(writer);
    let before = files(&root);
    assert!(
        !before
            .keys()
            .any(|p| is_side_file(p, "-wal") || is_side_file(p, "-shm")),
        "{:?}",
        before.keys()
    );
    let out = tempfile::tempdir().unwrap();
    explore(&root, out.path(), &three_areas(), None).unwrap();
    assert_eq!(files(&root), before, "names and contents unchanged");
    drop(dir);
}

#[test]
fn beside_an_open_library_only_the_shm_is_touched() {
    let (lib, _, _) = library(5);
    // The writer stays open, as a running Atlas's does.
    let before = files(&lib.root);
    assert!(before.keys().any(|p| is_side_file(p, "-wal")));
    let out = tempfile::tempdir().unwrap();
    let outcome = explore(&lib.root, out.path(), &three_areas(), None).unwrap();
    assert_eq!(outcome.survey.numbers.books, 15);
    let after = files(&lib.root);
    assert_eq!(
        after.keys().collect::<Vec<_>>(),
        before.keys().collect::<Vec<_>>(),
        "no file is added or removed"
    );
    for (path, hash) in &after {
        if !is_side_file(path, "-shm") {
            assert_eq!(hash, &before[path], "{}", path.display());
        }
    }
}

#[test]
fn the_spike_refuses_an_output_folder_in_the_library_or_the_repo() {
    let (lib, _, _) = library(5);
    let before = files(&lib.root);
    let inside = lib.root.join("exports").join("explore");
    let err = explore(&lib.root, &inside, &three_areas(), None).unwrap_err();
    assert!(err.contains("inside the library"), "{err}");
    assert_eq!(files(&lib.root), before);

    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    for bad in [repo.join("docs"), repo.join("src-tauri").join("tests")] {
        let err = explore(&lib.root, &bad.join("explore-out"), &three_areas(), None).unwrap_err();
        assert!(err.contains("inside the repository"), "{err}");
        assert!(!bad.join("explore-out").exists());
    }
    let err = explore(
        &lib.root,
        &lib.root.join("..").join("x"),
        &three_areas(),
        None,
    )
    .unwrap_err();
    assert!(err.contains(".."), "{err}");
    // The build's own folder is allowed: it is ignored by git.
    let allowed = Path::new(env!("CARGO_TARGET_TMPDIR")).join("explore-allowed");
    let outcome = explore(&lib.root, &allowed, &three_areas(), None).unwrap();
    assert!(outcome.page.is_file());
    fs::remove_dir_all(&allowed).unwrap();
}

#[test]
fn a_model_is_chosen_by_position_or_by_what_it_has_done() {
    let (lib, _, docs) = library(5);
    // A newer model with fewer books done, as after a change of model.
    let backend = HashedBagOfWordsBackend::new(DIMS);
    let newer = lib.create_model(DIMS);
    for d in &docs[..4] {
        lib.embed(&backend, &newer, d);
    }
    let out = tempfile::tempdir().unwrap();
    let by_done = explore(&lib.root, out.path(), &three_areas(), None).unwrap();
    assert_eq!(by_done.printed["models"].as_array().unwrap().len(), 2);
    let done: Vec<u64> = by_done.printed["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["done"].as_u64().unwrap())
        .collect();
    let most = if done[0] >= done[1] { 1 } else { 2 };
    assert_eq!(by_done.printed["model"], most);
    assert_eq!(by_done.survey.numbers.books, 15);
    let other = 3 - most;
    let by_position = explore(&lib.root, out.path(), &three_areas(), Some(other)).unwrap();
    assert_eq!(by_position.survey.numbers.books, 4);
    assert_eq!(by_position.survey.numbers.coverage.waiting, 11);
    assert!(explore(&lib.root, out.path(), &three_areas(), Some(3)).is_err());
}

// ------------------------------------------------------------ the owner's

fn env_number(name: &str) -> Option<usize> {
    std::env::var(name).ok().filter(|v| !v.is_empty()).map(|v| {
        v.parse()
            .unwrap_or_else(|_| panic!("{name} must be a number, not {v:?}"))
    })
}

/// The owner's run (`just spike-explore <data root> [out]`). Read-only on
/// the library; best with Atlas quit, when nothing beside the catalog is
/// touched at all.
#[test]
#[ignore = "run by the library's owner: just spike-explore <data root>"]
fn the_owners_library() {
    let root = std::env::var("ATLAS_EXPLORE_DATA_ROOT")
        .ok()
        .filter(|v| !v.is_empty())
        .expect("set ATLAS_EXPLORE_DATA_ROOT to the library's folder");
    let out = std::env::var("ATLAS_EXPLORE_OUT")
        .ok()
        .filter(|v| !v.is_empty())
        .map_or_else(
            || Path::new(env!("CARGO_TARGET_TMPDIR")).join("explore"),
            PathBuf::from,
        );
    let mut opts = Options::default();
    if std::env::var("ATLAS_EXPLORE_SAMPLING").as_deref() == Ok("all") {
        opts.sampling = Sampling::All;
    }
    if std::env::var("ATLAS_EXPLORE_CENTRED").as_deref() == Ok("1") {
        opts.centred = true;
    }
    if let Some(areas) = env_number("ATLAS_EXPLORE_AREAS") {
        opts.areas = areas;
    }
    match explore(
        Path::new(&root),
        &out,
        &opts,
        env_number("ATLAS_EXPLORE_MODEL"),
    ) {
        Ok(outcome) => {
            println!("explore-numbers {}", outcome.printed);
            let n = &outcome.survey.numbers;
            println!(
                "\n{} books in {} areas, {}% of the documents with text.",
                n.books,
                n.areas.len(),
                (n.covered * 100.0).round()
            );
            if n.covered < 0.9 {
                println!(
                    "Preliminary: under 90% is indexed for meaning. Run it again when \
                     Atlas says \"Meaning up to date\"."
                );
            }
            println!(
                "Loading took {:.1} s, the rest {:.1} s.",
                n.timings.load_ms as f64 / 1000.0,
                n.timings.after_loading_ms() as f64 / 1000.0
            );
            println!("\nOpen this page in a browser:\n{}", outcome.page.display());
        }
        Err(why) => panic!("the spike didn't run: {why}"),
    }
}

/// What the spike costs without a library (docs/benchmarks.md): books
/// drawn around planted centres, at the real collection's size and at the
/// 30,000-file fixture's.
#[test]
#[ignore = "a measurement: just bench-explore"]
fn measure_explore_at_5k_and_30k() {
    for books in [5_000usize, 30_000] {
        let points = planted(40, books / 40, 512, 1.4, 7);
        let chart = survey::chart(&points, &Options::default());
        let sizes: Vec<usize> = chart.areas.iter().map(|a| a.members.len()).collect();
        println!(
            "explore-cost {}",
            json!({
                "books": books,
                "dims": 512,
                "threads": Options::default().threads,
                "areas": sizes.len(),
                "smallestArea": sizes.iter().min(),
                "largestArea": sizes.iter().max(),
                "timings": chart.timings,
                "afterLoadingMs": chart.timings.after_loading_ms(),
                "map": chart.map,
                "silhouette": chart.silhouette,
                "vectorsMiB": (books * 512 * 4) as f64 / 1_048_576.0,
            })
        );
    }
}
