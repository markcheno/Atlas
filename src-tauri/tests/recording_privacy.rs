//! Slice 3.3, P3-16 (phase3-spec §8, §12): after a recorded session, no
//! words searched, question, answer, note, title or excerpt is anywhere
//! but the knowledge store: not in the process's output, not in any other
//! file of the library or its config, not in the catalog's job tables.
//!
//! The session runs in a child (this binary again, `--ignored`) so its
//! stdout and stderr can be read whole; it leaves its library behind for
//! the parent to search, byte by byte.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use atlas_lib::domain::dto::{
    AskQuery, AskReply, OpenAction, OpenPlace, OpenVia, ProviderKind, ProviderProfileInput,
    SearchIntent,
};
use atlas_lib::services::answers::stream::ChatLimits;
use atlas_lib::services::answers::{self, AnswerEvents, AskDeps};
use atlas_lib::services::providers::{privacy, profiles, url_policy};
use atlas_lib::services::search::MeaningContext;
use rusqlite::Connection;
use rusqlite::types::Value;
use serde_json::json;

mod common;
use common::http::{MockServer, stream};
use common::meaning::{BREAD, LONG, Lib, SKY};
use common::wait_until;

const SEARCH_MARK: &str = "quokkasearchword";
const QUESTION_MARK: &str = "quokkaquestionword";
const ANSWER_MARK: &str = "zebracornanswerword";
const NOTE_MARK: &str = "quokkanoteword";
const TITLE_MARK: &str = "quokkatitleword";
/// Words of the cited passage: an excerpt the store keeps.
const EXCERPT: &str = "distant comets and glowing nebulae";
const ROOT_LINE: &str = "ATLAS_P3_16_BASE=";

/// Every word the session typed or received, and what the model wrote.
const MARKS: [&str; 5] = [
    SEARCH_MARK,
    QUESTION_MARK,
    ANSWER_MARK,
    NOTE_MARK,
    TITLE_MARK,
];

fn contains(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

/// Every file under `dir`, recursively.
fn files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() {
            out.extend(files(&path));
        } else {
            out.push(path);
        }
    }
    out
}

#[test]
fn p3_16_nothing_recorded_is_anywhere_but_the_knowledge_store() {
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["p3_16_child_process", "--exact", "--ignored", "--nocapture"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "the session failed:\n{stdout}\n{stderr}"
    );
    let base = PathBuf::from(
        stdout
            .lines()
            .find_map(|l| l.strip_prefix(ROOT_LINE))
            .expect("the child names its library"),
    );

    // Nothing in what the process printed.
    for mark in MARKS.iter().chain([&EXCERPT]) {
        assert!(!contains(stdout.as_bytes(), mark), "stdout has {mark}");
        assert!(!contains(stderr.as_bytes(), mark), "stderr has {mark}");
    }

    let data = base.join("AtlasData");
    let library = data.join("library");
    let snapshots = data.join("backups").join("knowledge");
    // The store with its log and shared memory, and its snapshots: by
    // path components, since Windows paths mix separators.
    let is_store = |path: &Path| {
        let in_library = path.parent() == Some(library.as_path());
        let named = path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("knowledge.sqlite"));
        (in_library && named) || path.starts_with(&snapshots)
    };
    let mut in_store = Vec::new();
    let mut checked = 0;
    for path in files(&data).into_iter().chain(files(&base.join("config"))) {
        let bytes = fs::read(&path).unwrap();
        let name = path.to_string_lossy().into_owned();
        if is_store(&path) {
            in_store.extend(bytes);
            continue;
        }
        checked += 1;
        for mark in MARKS {
            assert!(!contains(&bytes, mark), "{name} holds {mark}");
        }
        // The catalog holds the documents' own text; nothing else may.
        if !name.contains("catalog.sqlite") && !name.contains("vectors") {
            assert!(!contains(&bytes, EXCERPT), "{name} holds an excerpt");
        }
    }
    assert!(checked > 3, "the library's files were looked at");
    // The store holds them all: the scan would have seen them.
    for mark in MARKS.iter().chain([&EXCERPT]) {
        assert!(contains(&in_store, mark), "the store keeps {mark}");
    }

    // The catalog's job tables, and every table that isn't the documents'
    // own text, hold none of it either.
    let catalog = Connection::open(data.join("library/catalog.sqlite")).unwrap();
    let content = [
        "passages",
        "chunks",
        "documents",
        "file_instances",
        "embeddings",
    ];
    let tables: Vec<String> = catalog
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .filter(|t: &String| !content.contains(&t.as_str()) && !t.starts_with("passages_fts"))
        .collect();
    assert!(tables.iter().any(|t| t == "jobs"));
    for table in tables {
        let mut stmt = catalog
            .prepare(&format!("SELECT * FROM \"{table}\""))
            .unwrap();
        let columns = stmt.column_count();
        let rows: Vec<String> = stmt
            .query_map([], |r| {
                let mut row = String::new();
                for i in 0..columns {
                    match r.get::<_, Value>(i)? {
                        Value::Text(t) => row.push_str(&t),
                        Value::Blob(b) => row.push_str(&String::from_utf8_lossy(&b)),
                        _ => {}
                    }
                    row.push('\u{1f}');
                }
                Ok(row)
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for row in rows {
            for mark in MARKS.iter().chain([&EXCERPT]) {
                assert!(!contains(row.as_bytes(), mark), "{table} holds {mark}");
            }
        }
    }
    drop(catalog);
    let _ = fs::remove_dir_all(&base);
}

/// Child half: a recorded session, then the library is closed and left.
#[test]
#[ignore = "run only as the child of p3_16_nothing_recorded_is_anywhere_but_the_knowledge_store"]
fn p3_16_child_process() {
    const NEAR_SKY: &str =
        "Distant comets and glowing nebulae drifted past the starship on its long voyage.";
    let mut lib = Lib::with_texts(&[SKY, NEAR_SKY, BREAD], false);
    let answer = format!("The starship passed comets {ANSWER_MARK} [1].");
    let pieces = [
        format!(
            "data: {}\n\n",
            json!({"choices":[{"delta":{"content": answer}}]})
        ),
        "data: [DONE]\n\n".to_owned(),
    ];
    let refs: Vec<&str> = pieces.iter().map(String::as_str).collect();
    let server = MockServer::start(vec![(
        "POST /v1/chat/completions",
        stream("text/event-stream", &refs, Duration::ZERO),
    )]);
    let saved = profiles::save(
        lib.root(),
        &lib.vault,
        ProviderProfileInput {
            id: None,
            kind: ProviderKind::OpenaiCompatible,
            label: "Chat service".into(),
            base_url: server.url("/v1"),
            api_key: None,
            clear_api_key: false,
            insecure_http_acknowledged: false,
        },
    )
    .unwrap();
    lib.root()
        .catalog()
        .call(move |c| {
            c.execute(
                "INSERT OR REPLACE INTO model_roles (role, provider_profile_id, model_id,
                     dimensions, tested_at, updated_at)
                 VALUES ('chat', ?1, 'test-chat', NULL, 't', 't')",
                [&saved.id],
            )
            .unwrap();
        })
        .unwrap();
    let root = lib.root();
    let recorder = root.recorder();

    // A search, as `search_documents` runs it.
    let q = Lib::query(&format!("glowing comets {SEARCH_MARK}"));
    let page = lib.hybrid(&q);
    assert!(!page.hits.is_empty());
    recorder.search(&q, SearchIntent::Submit, &page);

    // A question, answered.
    let events: AnswerEvents = std::sync::Arc::new(|_| {});
    let settings = root.settings().privacy.clone();
    let reply = answers::ask(
        &AskDeps {
            reader: root.reader(),
            meaning: MeaningContext {
                vectors: root.vectors(),
                catalog: &root.catalog().handle(),
                embedder: &lib.embedder,
                vault: &lib.vault,
                allow_remote: privacy::remote_embedding(&settings),
            },
            vault: &lib.vault,
            privacy: &settings,
            locate: url_policy::location_of,
            limits: ChatLimits::default(),
            events,
            answers: root.answers(),
            recorder: Some(recorder),
        },
        &AskQuery {
            question: format!("comets and glowing nebulae {QUESTION_MARK}"),
            source_root_id: None,
            file_type: None,
        },
    )
    .unwrap();
    let AskReply::Started(started) = reply else {
        panic!("{reply:?}");
    };
    assert!(wait_until(LONG, || recorder.status().step_count == 2));

    // Its citation opened, a note, a title.
    recorder.open(
        &started.sources[0].file_instance_id,
        &OpenVia::Citation {
            answer_id: started.answer_id.clone(),
            n: 1,
        },
        OpenAction::Details,
        OpenPlace::default(),
    );
    let trail = recorder.status().current_trail_id.unwrap();
    recorder
        .note(&trail, None, &format!("remember {NOTE_MARK}"))
        .unwrap();
    recorder
        .rename(&trail, &format!("Comets {TITLE_MARK}"))
        .unwrap();
    assert_eq!(recorder.status().step_count, 4);

    let base = lib.env.base.clone();
    lib.ws.shutdown();
    println!("{ROOT_LINE}{}", base.display());
    // The parent reads the library, then removes it.
    std::mem::forget(lib);
}
