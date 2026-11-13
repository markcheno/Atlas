//! Slice 2.3: the embedding job (phase2-spec §7.2, ADR 0014).
//!
//! - P2-05: every readable document is embedded once, identical text once;
//!   counts and status are right; cancel keeps what's done; the index
//!   equals the catalog.
//! - P2-06: on battery the job pauses (extraction only slows), and resumes
//!   on AC.
//! - P2-07: the service down, slow, busy or refusing: bounded backoff, no
//!   chunk sent again after an answer, a retry, honest failures, text
//!   search unaffected.
//! - P2-09: a model change builds a new index while the old one serves,
//!   then retires the old one.
//! - The remote opt-in and a missing embedding model.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use atlas_lib::domain::dto::{
    EmbeddingOverview, JobKind, JobState, PauseReason, PowerSource, SearchQuery,
};
use atlas_lib::domain::traits::EmbedError;
use atlas_lib::services::embedding::{models, status, triggers};
use atlas_lib::services::extraction::db as extraction_db;
use atlas_lib::services::workspace::{OpenRoot, Workspace};
use atlas_lib::services::{inventory, jobs, search, sources};
use rusqlite::Connection;

mod common;
use common::embedding::{EmbedHarness, fast_embedding};
use common::vectors::{count, expected_keys};
use common::{Env, wait_until};

const LONG: Duration = Duration::from_secs(180);
const DIMS: i64 = 64;

struct Lib {
    ws: Workspace,
    h: EmbedHarness,
    env: Env,
}

impl Lib {
    fn new() -> Self {
        Self::with(EmbedHarness::new(fast_embedding()))
    }

    fn with(h: EmbedHarness) -> Self {
        let env = Env::new();
        let mut ws = env.workspace_with_embedding(&h);
        ws.choose(&env.base.join("AtlasData")).unwrap();
        Self { ws, h, env }
    }

    fn root(&self) -> &OpenRoot {
        self.ws.require_open().unwrap()
    }

    fn catalog(&self) -> Connection {
        Connection::open(self.env.base.join("AtlasData/library/catalog.sqlite")).unwrap()
    }

    /// Generates `files` books (with `copies` identical copies) and reads
    /// them; returns the folder's id.
    fn read_books(&self, files: u32, copies: u32) -> String {
        let books = self.env.base.join("Books");
        let out = Command::new("node")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/generate-fixtures.mjs"))
            .arg(&books)
            .args(["--files", &files.to_string(), "--text-kb", "3"])
            .args(["--copies", &copies.to_string()])
            .output()
            .expect("node runs the fixture generator");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let added = sources::add(self.root(), &books).unwrap();
        inventory::control::enqueue(self.root(), &added.id).unwrap();
        assert!(
            wait_until(LONG, || self.text_read()),
            "reading never settled"
        );
        added.id
    }

    fn text_read(&self) -> bool {
        let inv = inventory::control::overview(self.root()).unwrap();
        let ov = self
            .root()
            .reader()
            .read(|c| extraction_db::overview(c, None))
            .unwrap()
            .unwrap();
        inv.active.is_none() && ov.active.is_none() && ov.queued_jobs == 0 && ov.counts.pending == 0
    }

    /// An embedding role on a service at `base_url`, then the trigger the
    /// role commands call.
    fn set_role(&self, base_url: &str, dims: i64) {
        let base = base_url.to_owned();
        self.root()
            .catalog()
            .call(move |c| {
                c.execute(
                    "INSERT INTO provider_profiles (id, kind, label, base_url,
                         insecure_http_acknowledged, created_at, updated_at)
                     VALUES ('svc', 'openai_compatible', 'Test service', ?1, 1, 't', 't')
                     ON CONFLICT (id) DO UPDATE SET base_url = excluded.base_url",
                    [&base],
                )
                .unwrap();
                c.execute(
                    "INSERT INTO model_roles (role, provider_profile_id, model_id, dimensions,
                         tested_at, updated_at)
                     VALUES ('embedding', 'svc', 'test-embed', ?1, 't', 't')
                     ON CONFLICT (role) DO UPDATE SET dimensions = excluded.dimensions",
                    [dims],
                )
                .unwrap();
            })
            .unwrap();
        triggers::changed(self.root()).unwrap();
    }

    fn overview(&self) -> EmbeddingOverview {
        let allow = atlas_lib::services::providers::privacy::remote_embedding(
            &self.root().settings().privacy,
        );
        self.root()
            .reader()
            .read(move |c| status::overview(c, allow))
            .unwrap()
            .unwrap()
    }

    fn idle(&self) -> bool {
        let ov = self.overview();
        ov.active.is_none() && ov.queued_jobs == 0
    }

    fn wait_idle(&self) {
        assert!(
            wait_until(LONG, || self.idle()),
            "never idle: {:?}",
            self.overview()
        );
    }

    /// The job marks a finished model ready just after its last job ends.
    fn wait_ready(&self, model: &str) {
        let model = model.to_owned();
        assert!(
            wait_until(LONG, || {
                self.catalog()
                    .query_row(
                        "SELECT status FROM embedding_models WHERE id = ?1",
                        [&model],
                        |r| r.get::<_, String>(0),
                    )
                    .is_ok_and(|s| s == "ready")
            }),
            "model {model} never became ready"
        );
    }

    fn active_model(&self) -> String {
        models::active(&self.catalog()).unwrap().unwrap().id
    }

    fn job_state(&self) -> Option<(JobState, Option<PauseReason>, Option<String>)> {
        self.overview()
            .active
            .map(|a| (a.state, a.pause_reason, a.last_error))
    }

    /// The job's last state, active or not.
    fn last_job(&self) -> (String, String, Option<String>) {
        self.catalog()
            .query_row(
                "SELECT id, state, last_error FROM jobs WHERE kind = 'embed'
                 ORDER BY rowid DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
    }

    /// The model's index in memory holds exactly the catalog's chunks.
    fn assert_index_matches(&self, model: &str) {
        let c = self.catalog();
        let expected = expected_keys(&c, model);
        let index = self.root().vectors().loaded(model).expect("index open");
        assert_eq!(index.len(), expected.len(), "vector count");
        for k in &expected {
            assert!(index.contains(*k), "chunk {k} missing from the index");
        }
    }
}

fn distinct_hashes(c: &Connection) -> i64 {
    count(c, "SELECT COUNT(DISTINCT text_hash) FROM chunks")
}

#[test]
fn p2_05_every_document_is_embedded_once_and_identical_text_once() {
    let lib = Lib::new();
    let root_id = lib.read_books(12, 4);
    // Nothing happens without a model.
    let before = lib.overview();
    assert!(before.needs_model);
    assert!(before.active.is_none() && before.model.is_none());

    lib.set_role("http://localhost:9/v1", DIMS);
    lib.wait_idle();
    let model = lib.active_model();
    let c = lib.catalog();

    let readable = count(&c, "SELECT COUNT(*) FROM documents WHERE state = 'done'");
    assert!(readable >= 12);
    assert_eq!(
        count(
            &c,
            &format!(
                "SELECT COUNT(*) FROM document_embeddings
                 WHERE model_id = '{model}' AND status IN ('done', 'empty')"
            )
        ),
        readable,
        "every readable document, once"
    );
    // Identical copies are one document; identical text is one vector.
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM embeddings"),
        distinct_hashes(&c)
    );
    let answered = lib.h.script.answered();
    let unique: BTreeSet<&String> = answered.iter().collect();
    assert_eq!(answered.len(), unique.len(), "no text was sent twice");
    assert_eq!(answered.len() as i64, distinct_hashes(&c));

    let ov = lib.overview();
    assert!(!ov.needs_model);
    assert_eq!(ov.model.as_ref().unwrap().dims, DIMS as u32);
    assert_eq!(ov.counts.waiting, 0);
    assert_eq!(ov.counts.indexed, ov.counts.total);
    let files_with_text = count(
        &c,
        "SELECT COUNT(*) FROM file_instances f JOIN documents d ON d.id = f.document_id
         WHERE f.availability = 'available' AND d.state = 'done'",
    );
    assert_eq!(
        ov.counts.total as i64, files_with_text,
        "copies count as files"
    );
    assert_eq!(ov.roots[0].source_root_id, root_id);
    let (_, state, _) = lib.last_job();
    assert_eq!(state, "completed");
    let chunks: i64 = c
        .query_row(
            "SELECT embedded_chunks FROM jobs WHERE kind = 'embed' ORDER BY rowid DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(chunks, distinct_hashes(&c), "the job counted what it sent");

    // The first model serves as soon as it's complete, and its index is
    // the catalog.
    lib.wait_ready(&model);
    lib.assert_index_matches(&model);
    assert!(
        !lib.h.events.lock().unwrap().is_empty(),
        "status events were sent"
    );

    // Index meaning now with nothing waiting: no new requests.
    triggers::start(lib.root(), &root_id).unwrap();
    lib.wait_idle();
    assert_eq!(lib.h.script.answered().len(), answered.len());
}

#[test]
fn p2_05_cancel_keeps_what_is_embedded_and_index_meaning_now_finishes() {
    let mut config = fast_embedding();
    config.batch_size = 1;
    let lib = Lib::with(EmbedHarness::new(config));
    let root_id = lib.read_books(12, 0);
    lib.h.script.with(|s| s.delay = Duration::from_millis(40));
    lib.set_role("http://localhost:9/v1", DIMS);
    let model = lib.active_model();
    let done = |lib: &Lib| {
        count(
            &lib.catalog(),
            &format!("SELECT COUNT(*) FROM document_embeddings WHERE model_id = '{model}'"),
        )
    };
    assert!(wait_until(LONG, || done(&lib) >= 2));
    let job = lib.overview().active.unwrap().job_id;
    jobs::control::cancel(lib.root(), &job).unwrap();
    lib.wait_idle();
    let kept = done(&lib);
    assert!(kept >= 2, "embedded documents stay");
    let ov = lib.overview();
    assert!(ov.counts.waiting > 0, "the rest waits: {:?}", ov.counts);

    lib.h.script.with(|s| s.delay = Duration::ZERO);
    let again = triggers::start(lib.root(), &root_id).unwrap();
    assert_ne!(again, job);
    lib.wait_idle();
    assert_eq!(lib.overview().counts.waiting, 0);
    let answered = lib.h.script.answered();
    let unique: BTreeSet<&String> = answered.iter().collect();
    assert_eq!(
        answered.len(),
        unique.len(),
        "nothing sent twice across the cancel"
    );
    lib.assert_index_matches(&model);
}

#[test]
fn p2_06_on_battery_the_job_pauses_while_reading_only_slows() {
    let lib = Lib::new();
    lib.h.ex.inv.monitor.set(|s| {
        s.power = PowerSource::Battery;
        s.battery_percent = Some(50.0);
    });
    // Reading still finishes on battery (it slows, and the test config
    // reads unthrottled).
    lib.read_books(6, 0);
    lib.set_role("http://localhost:9/v1", DIMS);
    assert!(
        wait_until(LONG, || matches!(
            lib.job_state(),
            Some((JobState::PausedPolicy, Some(PauseReason::OnBattery), _))
        )),
        "{:?}",
        lib.overview().active
    );
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(lib.h.script.calls(), 0, "nothing sent on battery");
    assert_eq!(lib.overview().counts.indexed, 0);

    lib.h.ex.inv.monitor.set(|s| {
        s.power = PowerSource::Ac;
        s.battery_percent = Some(55.0);
    });
    lib.wait_idle();
    assert_eq!(lib.overview().counts.waiting, 0, "finished on AC");
}

#[test]
fn p2_07_a_failing_service_backs_off_and_nothing_is_sent_twice() {
    let lib = Lib::new();
    lib.read_books(8, 0);
    lib.h.script.with(|s| {
        s.fail_next.extend([
            EmbedError::Network,
            EmbedError::Timeout,
            EmbedError::Server {
                status: 503,
                retry_after: None,
            },
        ]);
    });
    lib.set_role("http://localhost:9/v1", DIMS);
    lib.wait_idle();
    assert_eq!(lib.overview().counts.waiting, 0);
    let answered = lib.h.script.answered();
    let unique: BTreeSet<&String> = answered.iter().collect();
    assert_eq!(
        answered.len(),
        unique.len(),
        "no chunk sent again after an answer"
    );
    assert_eq!(answered.len() as i64, distinct_hashes(&lib.catalog()));
}

#[test]
fn p2_07_retry_after_is_honoured() {
    let lib = Lib::new();
    lib.read_books(3, 0);
    lib.h.script.with(|s| {
        s.fail_next.push_back(EmbedError::RateLimited {
            retry_after: Some(Duration::from_millis(600)),
        });
    });
    let started = Instant::now();
    lib.set_role("http://localhost:9/v1", DIMS);
    lib.wait_idle();
    assert!(
        started.elapsed() >= Duration::from_millis(600),
        "waited as asked: {:?}",
        started.elapsed()
    );
    assert_eq!(lib.overview().counts.waiting, 0);
}

#[test]
fn p2_07_an_unreachable_service_fails_for_a_retry_and_search_still_works() {
    let mut config = fast_embedding();
    config.retry_base = Duration::from_secs(60);
    let lib = Lib::with(EmbedHarness::new(config));
    lib.read_books(4, 0);
    lib.h
        .script
        .with(|s| s.fail_always = Some(EmbedError::Network));
    lib.set_role("http://localhost:9/v1", DIMS);
    assert!(
        wait_until(LONG, || matches!(
            lib.job_state(),
            Some((JobState::FailedRetryable, _, Some(ref e))) if e == "provider_unreachable"
        )),
        "{:?}",
        lib.overview().active
    );
    let calls = lib.h.script.calls();
    assert!(calls >= 6, "six tries before giving up: {calls}");

    // Full-text search is unaffected.
    let page = search::run(
        lib.root().reader(),
        lib.root().root_status(),
        &SearchQuery {
            text: "the".into(),
            source_root_id: None,
            file_type: None,
            include_missing: false,
            page: 0,
            page_size: 5,
        },
    )
    .unwrap();
    assert!(!page.hits.is_empty(), "text search still answers");

    // The service is back; Retry (resume) runs it at once.
    lib.h.script.with(|s| s.fail_always = None);
    let job = lib.overview().active.unwrap().job_id;
    jobs::control::resume(lib.root(), &job).unwrap();
    lib.wait_idle();
    assert_eq!(lib.overview().counts.waiting, 0);
}

#[test]
fn p2_07_a_refused_chunk_fails_only_its_document() {
    let lib = Lib::new();
    lib.read_books(6, 0);
    // One book's text has a word no other book has.
    let c = lib.catalog();
    let (doc, word): (String, String) = c
        .query_row(
            "SELECT d.id, f.display_name FROM documents d
             JOIN file_instances f ON f.document_id = d.id
             WHERE d.state = 'done' ORDER BY f.display_name LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    drop(word);
    let unique = "zyzzogetonquux";
    let target = doc.clone();
    lib.root()
        .catalog()
        .call(move |c| {
            c.execute(
                "UPDATE passages SET text = text || ' ' || ?2 WHERE document_id = ?1 AND ordinal = 0",
                rusqlite::params![target, unique],
            )
        })
        .unwrap()
        .unwrap();
    lib.h.script.with(|s| s.reject_word = Some(unique.into()));
    lib.set_role("http://localhost:9/v1", DIMS);
    lib.wait_idle();
    let c = lib.catalog();
    let (status, attempts): (String, i64) = c
        .query_row(
            "SELECT status, attempts FROM document_embeddings WHERE document_id = ?1",
            [&doc],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((status.as_str(), attempts), ("failed", 1));
    assert_eq!(
        count(
            &c,
            "SELECT COUNT(*) FROM document_embeddings WHERE status = 'done'"
        ),
        count(&c, "SELECT COUNT(*) FROM documents WHERE state = 'done'") - 1,
        "every other document is embedded"
    );
    let (_, state, _) = lib.last_job();
    assert_eq!(state, "completed");
}

#[test]
fn p2_07_a_refused_key_or_a_changed_model_fails_for_good() {
    for (error, code) in [
        (EmbedError::Auth, "provider_auth"),
        (EmbedError::ModelNotFound, "model_not_found"),
    ] {
        let lib = Lib::new();
        lib.read_books(2, 0);
        lib.h.script.with(|s| s.fail_always = Some(error.clone()));
        lib.set_role("http://localhost:9/v1", DIMS);
        lib.wait_idle();
        let (_, state, last) = lib.last_job();
        assert_eq!(
            (state.as_str(), last.as_deref()),
            ("failed_terminal", Some(code))
        );
    }
    let lib = Lib::new();
    lib.read_books(2, 0);
    lib.h.script.with(|s| s.wrong_dims = Some(32));
    lib.set_role("http://localhost:9/v1", DIMS);
    lib.wait_idle();
    let (_, state, last) = lib.last_job();
    assert_eq!(
        (state.as_str(), last.as_deref()),
        ("failed_terminal", Some("model_changed"))
    );
}

#[test]
fn p2_09_a_new_model_builds_while_the_old_one_serves_then_replaces_it() {
    let mut config = fast_embedding();
    config.batch_size = 1;
    let lib = Lib::with(EmbedHarness::new(config));
    lib.read_books(8, 0);
    lib.set_role("http://localhost:9/v1", DIMS);
    lib.wait_idle();
    let old = lib.active_model();
    lib.wait_ready(&old);
    lib.assert_index_matches(&old);
    let old_file = file_of(&lib, &old);
    assert!(old_file.exists());

    // A different dimension count is a new model: it builds while the
    // old one serves.
    lib.h.script.with(|s| s.delay = Duration::from_millis(30));
    lib.set_role("http://localhost:9/v1", 32);
    let new = lib.active_model();
    assert_ne!(new, old);
    assert!(wait_until(LONG, || lib.overview().rebuilding));
    assert_eq!(models::serving(&lib.catalog()).unwrap().unwrap().id, old);

    lib.h.script.with(|s| s.delay = Duration::ZERO);
    lib.wait_idle();
    assert!(
        wait_until(LONG, || models::serving(&lib.catalog())
            .unwrap()
            .unwrap()
            .id
            == new),
        "the new model serves"
    );
    let c = lib.catalog();
    assert!(
        wait_until(LONG, || count(
            &lib.catalog(),
            "SELECT COUNT(*) FROM embedding_models"
        ) == 1),
        "the old model's row is gone"
    );
    assert_eq!(
        count(
            &c,
            &format!("SELECT COUNT(*) FROM embeddings WHERE model_id = '{old}'")
        ),
        0
    );
    assert!(!old_file.exists(), "the old index file is gone");
    assert!(!lib.overview().rebuilding);
    lib.assert_index_matches(&new);
}

fn file_of(lib: &Lib, model: &str) -> PathBuf {
    let generation: i64 = lib
        .catalog()
        .query_row(
            "SELECT index_generation FROM embedding_models WHERE id = ?1",
            [model],
            |r| r.get(0),
        )
        .unwrap();
    lib.root().vectors().paths().file(model, generation)
}

#[test]
fn an_internet_service_needs_permission_first() {
    let lib = Lib::new();
    lib.read_books(3, 0);
    lib.set_role("https://embeddings.invalid/v1", DIMS);
    std::thread::sleep(Duration::from_millis(300));
    let ov = lib.overview();
    assert!(ov.needs_permission);
    assert!(ov.active.is_none(), "no job runs without permission");
    assert_eq!(lib.h.script.calls(), 0);

    let mut settings = lib.ws.settings().unwrap();
    settings.privacy.allow_remote_embedding = true;
    let mut ws = lib.ws;
    ws.update_settings(settings).unwrap();
    let lib = Lib { ws, ..lib };
    lib.wait_idle();
    let ov = lib.overview();
    assert!(!ov.needs_permission);
    assert_eq!(ov.counts.waiting, 0);
}

#[test]
fn queued_jobs_are_never_made_without_a_model() {
    let lib = Lib::new();
    lib.read_books(2, 0);
    let c = lib.catalog();
    assert_eq!(
        count(&c, "SELECT COUNT(*) FROM jobs WHERE kind = 'embed'"),
        0,
        "reading a folder queues nothing without a model"
    );
    // A stray job (e.g. from an older session) completes doing nothing.
    let root = c
        .query_row("SELECT id FROM source_roots", [], |r| r.get::<_, String>(0))
        .unwrap();
    jobs::control::enqueue(lib.root(), &root, JobKind::Embed).unwrap();
    lib.wait_idle();
    assert_eq!(lib.h.script.calls(), 0);
    assert!(lib.overview().needs_model);
    let _ = fs::metadata(lib.env.base.join("AtlasData"));
}

#[test]
fn p2_19_the_meaning_index_is_counted_estimated_and_kept_by_clean_caches() {
    use atlas_lib::services::storage;
    let lib = Lib::new();
    lib.read_books(6, 0);
    let path = lib.env.base.join("AtlasData");
    let volumes = common::local();
    let before = storage::usage(&path, volumes.as_ref()).unwrap();
    assert_eq!(before.meaning_estimate, None, "no estimate without a model");

    // On battery the job waits, so the estimate covers everything.
    lib.h.ex.inv.monitor.set(|s| {
        s.power = PowerSource::Battery;
        s.battery_percent = Some(60.0);
    });
    lib.set_role("http://localhost:9/v1", DIMS);
    let waiting = storage::usage(&path, volumes.as_ref()).unwrap();
    let e = waiting
        .meaning_estimate
        .expect("an estimate once a model is chosen");
    assert!(e.remaining_files > 0 && e.low_bytes > 0 && e.high_bytes > e.low_bytes);

    lib.h.ex.inv.monitor.set(|s| s.power = PowerSource::Ac);
    lib.wait_idle();
    let model = lib.active_model();
    lib.wait_ready(&model);
    let after = storage::usage(&path, volumes.as_ref()).unwrap();
    assert_eq!(after.meaning_estimate.unwrap().remaining_files, 0);
    let c = lib.catalog();
    let rows: i64 = c
        .query_row(
            "SELECT SUM(pgsize) FROM dbstat WHERE aggregate = TRUE AND
                 (name IN ('chunks', 'embeddings', 'document_embeddings', 'vector_tombstones',
                           'embedding_models')
                  OR name LIKE 'chunks\\_%' ESCAPE '\\'
                  OR name LIKE 'document\\_embeddings\\_%' ESCAPE '\\'
                  OR name LIKE 'sqlite\\_autoindex\\_embeddings\\_%' ESCAPE '\\'
                  OR name LIKE 'sqlite\\_autoindex\\_document\\_embeddings\\_%' ESCAPE '\\'
                  OR name LIKE 'sqlite\\_autoindex\\_chunks\\_%' ESCAPE '\\'
                  OR name LIKE 'sqlite\\_autoindex\\_embedding\\_models\\_%' ESCAPE '\\')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        after.meaning_catalog_bytes as i64, rows,
        "the category is the dbstat sum"
    );
    let file = file_of(&lib, &model);
    assert!(file.exists());
    assert_eq!(after.meaning_file_bytes, fs::metadata(&file).unwrap().len());
    assert_eq!(
        after.catalog_bytes,
        after.text_bytes
            + after.index_bytes
            + after.meaning_catalog_bytes
            + after.other_catalog_bytes
    );

    let vectors = count(&c, "SELECT COUNT(*) FROM embeddings");
    storage::clean_caches(&path).unwrap();
    assert!(file.exists(), "Clean caches keeps the index");
    assert_eq!(
        count(&lib.catalog(), "SELECT COUNT(*) FROM embeddings"),
        vectors
    );
}

/// Slice 4.1.1 (found on the owner's library): a job that stopped for good
/// isn't active, so the status line said "Meaning waiting" and nothing
/// else. The folder's summary now carries the reason, until another job
/// for the folder takes its place.
#[test]
fn a_job_that_stopped_for_good_says_why_until_another_runs() {
    let lib = Lib::new();
    let root_id = lib.read_books(2, 0);
    lib.h
        .script
        .with(|s| s.fail_always = Some(EmbedError::ModelNotFound));
    lib.set_role("http://localhost:9/v1", DIMS);
    lib.wait_idle();
    let ov = lib.overview();
    assert!(ov.active.is_none(), "{:?}", ov.active);
    assert!(ov.counts.waiting > 0);
    assert_eq!(
        ov.roots[0].failed_because.as_deref(),
        Some("model_not_found")
    );

    lib.h.script.with(|s| s.fail_always = None);
    triggers::start(lib.root(), &root_id).unwrap();
    lib.wait_idle();
    let ov = lib.overview();
    assert_eq!(ov.counts.waiting, 0);
    assert_eq!(ov.roots[0].failed_because, None);
}

/// Slice 4.1 (found by P3-13 on CI): a document read again after the job
/// has passed it waits again, and the reading job's trigger sees this job
/// still running. The job looks once more from the start before finishing,
/// so the document isn't left waiting until the next launch.
#[test]
fn a_document_read_again_behind_the_job_is_embedded_before_it_ends() {
    let lib = Lib::new();
    lib.read_books(8, 0);
    lib.h.script.with(|s| s.delay = Duration::from_millis(150));
    lib.set_role("http://localhost:9/v1", DIMS);
    let model = lib.active_model();
    let c = lib.catalog();
    let first: String = c
        .query_row(
            "SELECT id FROM documents WHERE state = 'done' ORDER BY rowid LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    // Once the job is past the first document, it is "read again": its
    // vectors' record goes, as `commit_document` does.
    let past = || {
        lib.catalog()
            .query_row(
                "SELECT COUNT(*) FROM document_embeddings WHERE document_id = ?1 AND status = 'done'",
                [&first],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
            == 1
    };
    assert!(
        wait_until(LONG, past),
        "the job never reached the first book"
    );
    assert!(lib.overview().active.is_some(), "the job is still running");
    let (doc, m) = (first.clone(), model.clone());
    lib.root()
        .catalog()
        .call(move |c| {
            c.execute(
                "DELETE FROM document_embeddings WHERE document_id = ?1 AND model_id = ?2",
                [&doc, &m],
            )
        })
        .unwrap()
        .unwrap();
    lib.wait_idle();
    assert!(past(), "embedded again before the job ended");
    assert_eq!(lib.overview().counts.waiting, 0);
}
