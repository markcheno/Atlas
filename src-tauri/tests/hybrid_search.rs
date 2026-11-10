//! Slice 2.4: hybrid search (phase2-spec §7.4).
//!
//! - P2-10: a question that shares no words with a document still finds it
//!   by meaning; a quoted phrase finds by text; a document found both ways
//!   says so and ranks first; folder and type filters apply to meaning
//!   matches; nonsense finds nothing.
//! - P2-11: without a model, with the service down, slow or not allowed,
//!   or with the index still loading, the page is exactly Phase 1's plus a
//!   status, never an error.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use atlas_lib::domain::dto::{
    FileType, MatchReason, SearchPage, SearchQuery, SemanticStatus, SemanticUnavailable,
};
use atlas_lib::services::embedding::query::QueryEmbedder;
use atlas_lib::services::embedding::{models, triggers};
use atlas_lib::services::extraction::db as extraction_db;
use atlas_lib::services::providers::secrets::SecretVault;
use atlas_lib::services::search::{self, MeaningContext};
use atlas_lib::services::workspace::{OpenRoot, Workspace};
use atlas_lib::services::{inventory, sources};
use rusqlite::Connection;

mod common;
use common::embedding::{EmbedHarness, fast_embedding};
use common::{Env, wait_until};

const LONG: Duration = Duration::from_secs(180);

const SKY: &str = "The voyage of the starship carried us past distant comets and glowing nebulae.";
const BREAD: &str = "Kneading sourdough bread and proofing the dough overnight in a warm kitchen.";
const SEA: &str = "Harbor ballads sung by old sailors beneath the swinging lantern light.";

struct Lib {
    ws: Workspace,
    h: EmbedHarness,
    env: Env,
    embedder: Arc<QueryEmbedder>,
    vault: SecretVault,
    sky: String,
    sky_type: FileType,
}

impl Lib {
    /// A read library of generated books, three of whose texts are replaced
    /// by sentences on distinct subjects; then embedded with the fake.
    fn new() -> Self {
        let h = EmbedHarness::new(fast_embedding());
        let env = Env::new();
        let mut ws = env.workspace_with_embedding(&h);
        ws.choose(&env.base.join("AtlasData")).unwrap();
        let books = env.base.join("Books");
        let out = Command::new("node")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/generate-fixtures.mjs"))
            .arg(&books)
            .args(["--files", "10", "--text-kb", "2", "--copies", "2"])
            .output()
            .unwrap();
        assert!(out.status.success());
        let root = ws.require_open().unwrap();
        let added = sources::add(root, &books).unwrap();
        inventory::control::enqueue(root, &added.id).unwrap();
        assert!(wait_until(LONG, || {
            let inv = inventory::control::overview(root).unwrap();
            let ov = root
                .reader()
                .read(|c| extraction_db::overview(c, None))
                .unwrap()
                .unwrap();
            inv.active.is_none() && ov.active.is_none() && ov.counts.pending == 0
        }));
        // Three documents get one subject each.
        let docs: Vec<(String, String)> = root
            .catalog()
            .call(|c| {
                let docs: Vec<(String, String)> = c
                    .prepare(
                        "SELECT d.id, (SELECT f.file_type FROM file_instances f
                                       WHERE f.document_id = d.id LIMIT 1)
                         FROM documents d WHERE d.state = 'done' ORDER BY d.id LIMIT 3",
                    )
                    .unwrap()
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .unwrap()
                    .map(Result::unwrap)
                    .collect();
                for ((doc, _), text) in docs.iter().zip([SKY, BREAD, SEA]) {
                    c.execute(
                        "UPDATE passages SET text = ?2 WHERE document_id = ?1",
                        rusqlite::params![doc, text],
                    )
                    .unwrap();
                }
                docs
            })
            .unwrap();
        let sky_type = if docs[0].1 == "epub" {
            FileType::Epub
        } else {
            FileType::Pdf
        };
        let embedder = Arc::new(QueryEmbedder::new(h.query_backends()));
        let lib = Self {
            ws,
            embedder,
            vault: SecretVault::new(None),
            sky: docs[0].0.clone(),
            sky_type,
            h,
            env,
        };
        lib.set_role("http://localhost:9/v1");
        assert!(
            wait_until(LONG, || lib.indexed()),
            "embedding never finished"
        );
        lib
    }

    fn root(&self) -> &OpenRoot {
        self.ws.require_open().unwrap()
    }

    fn catalog(&self) -> Connection {
        Connection::open(self.env.base.join("AtlasData/library/catalog.sqlite")).unwrap()
    }

    fn set_role(&self, base_url: &str) {
        let base = base_url.to_owned();
        self.root()
            .catalog()
            .call(move |c| {
                c.execute(
                    "INSERT INTO provider_profiles (id, kind, label, base_url,
                         insecure_http_acknowledged, created_at, updated_at)
                     VALUES ('svc', 'openai_compatible', 'Test', ?1, 1, 't', 't')
                     ON CONFLICT (id) DO UPDATE SET base_url = excluded.base_url",
                    [&base],
                )
                .unwrap();
                c.execute_batch(
                    "INSERT OR IGNORE INTO model_roles (role, provider_profile_id, model_id,
                         dimensions, tested_at, updated_at)
                     VALUES ('embedding', 'svc', 'test-embed', 64, 't', 't')",
                )
                .unwrap();
            })
            .unwrap();
        triggers::changed(self.root()).unwrap();
    }

    fn indexed(&self) -> bool {
        let c = self.catalog();
        let Ok(Some(model)) = models::serving(&c) else {
            return false;
        };
        let waiting: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM documents d WHERE d.state = 'done' AND NOT EXISTS
                     (SELECT 1 FROM document_embeddings de
                      WHERE de.document_id = d.id AND de.model_id = ?1)",
                [&model.id],
                |r| r.get(0),
            )
            .unwrap();
        waiting == 0 && self.root().vectors().loaded(&model.id).is_some()
    }

    fn query(text: &str) -> SearchQuery {
        SearchQuery {
            text: text.into(),
            source_root_id: None,
            file_type: None,
            include_missing: false,
            page: 0,
            page_size: 20,
        }
    }

    fn hybrid(&self, q: &SearchQuery) -> SearchPage {
        let root = self.root();
        search::run_hybrid(
            root.reader(),
            root.root_status(),
            &MeaningContext {
                vectors: root.vectors(),
                catalog: &root.catalog().handle(),
                embedder: &self.embedder,
                vault: &self.vault,
                allow_remote: root.settings().privacy.allow_remote_embedding,
            },
            q,
        )
        .unwrap()
    }

    fn text_only(&self, q: &SearchQuery) -> SearchPage {
        search::run(self.root().reader(), self.root().root_status(), q).unwrap()
    }

    /// "Means" the sky document without sharing a word with it.
    fn alias_to_sky(&self, query: &str) {
        let q = query.to_owned();
        self.h.script.with(|s| {
            s.alias.insert(q, SKY.into());
        });
    }
}

fn ids(page: &SearchPage) -> Vec<(Option<String>, MatchReason)> {
    page.hits
        .iter()
        .map(|h| (h.document_id.clone(), h.why))
        .collect()
}

#[test]
fn p2_10_meaning_finds_what_the_words_miss_and_fusion_ranks_both_first() {
    let lib = Lib::new();

    // Shares no word with the document: found by meaning only.
    lib.alias_to_sky("interstellar odyssey");
    let page = lib.hybrid(&Lib::query("interstellar odyssey"));
    assert!(matches!(
        page.semantic,
        SemanticStatus::Ready {
            coverage_percent: 100
        }
    ));
    assert!(
        lib.text_only(&Lib::query("interstellar odyssey"))
            .hits
            .is_empty()
    );
    let first = &page.hits[0];
    assert_eq!(first.document_id.as_deref(), Some(lib.sky.as_str()));
    assert_eq!(first.why, MatchReason::Semantic);
    assert!(first.all_words, "never 'some of your words'");
    let snippet: String = first.snippet.iter().map(|s| s.text.as_str()).collect();
    assert!(snippet.contains("starship"), "{snippet}");
    assert!(page.hits.len() <= search::hybrid::MAX_MEANING_ONLY);

    // Found both ways: says so, and comes first.
    lib.alias_to_sky("distant comets");
    let page = lib.hybrid(&Lib::query("distant comets"));
    assert_eq!(page.hits[0].document_id.as_deref(), Some(lib.sky.as_str()));
    assert_eq!(page.hits[0].why, MatchReason::TextSemantic);

    // A quoted phrase finds its document by text.
    let page = lib.hybrid(&Lib::query("\"sourdough bread\""));
    assert!(matches!(
        page.hits[0].why,
        MatchReason::Text | MatchReason::TextSemantic
    ));
    let bread: String = page.hits[0]
        .snippet
        .iter()
        .map(|s| s.text.as_str())
        .collect();
    assert!(bread.to_lowercase().contains("sourdough"));

    // Filters apply to meaning matches: the other file type excludes it.
    let mut other = Lib::query("interstellar odyssey");
    other.file_type = Some(match lib.sky_type {
        FileType::Pdf => FileType::Epub,
        FileType::Epub => FileType::Pdf,
    });
    let page = lib.hybrid(&other);
    assert!(
        page.hits
            .iter()
            .all(|h| h.document_id.as_deref() != Some(lib.sky.as_str())),
        "{:?}",
        ids(&page)
    );
    let mut same = Lib::query("interstellar odyssey");
    same.file_type = Some(lib.sky_type);
    assert_eq!(
        lib.hybrid(&same).hits[0].document_id.as_deref(),
        Some(lib.sky.as_str())
    );
    let mut nowhere = Lib::query("interstellar odyssey");
    nowhere.source_root_id = Some("no-such-folder".into());
    assert!(lib.hybrid(&nowhere).hits.is_empty());

    // Copies are one result.
    let page = lib.hybrid(&Lib::query("book"));
    let docs: Vec<_> = page
        .hits
        .iter()
        .filter_map(|h| h.document_id.clone())
        .collect();
    let unique: std::collections::BTreeSet<_> = docs.iter().collect();
    assert_eq!(docs.len(), unique.len());

    // Nonsense still finds nothing.
    let page = lib.hybrid(&Lib::query("zqxjv plorbix"));
    assert!(page.hits.is_empty(), "{:?}", ids(&page));

    // A repeated question is not sent again.
    let calls = lib.h.script.calls();
    lib.hybrid(&Lib::query("interstellar odyssey"));
    assert_eq!(lib.h.script.calls(), calls, "answered from the cache");
}

#[test]
fn p2_11_without_meaning_the_page_is_phase_1s_plus_a_status() {
    let lib = Lib::new();
    let q = Lib::query("comets bread");
    let phase1 = ids(&lib.text_only(&q));
    assert!(!phase1.is_empty());

    // The service is down.
    lib.h
        .script
        .with(|s| s.fail_always = Some(atlas_lib::domain::traits::EmbedError::Network));
    let page = lib.hybrid(&Lib::query("comets bread"));
    assert_eq!(
        page.semantic,
        SemanticStatus::Unavailable {
            reason: SemanticUnavailable::Unreachable
        }
    );
    assert_eq!(ids(&page), phase1);

    // Too slow: Phase 1's page now, and the answer fills the cache.
    lib.h.script.with(|s| {
        s.fail_always = None;
        s.delay = Duration::from_millis(2_500);
    });
    let page = lib.hybrid(&Lib::query("comets bread"));
    assert_eq!(
        page.semantic,
        SemanticStatus::Unavailable {
            reason: SemanticUnavailable::Timeout
        }
    );
    assert_eq!(ids(&page), phase1);
    std::thread::sleep(Duration::from_millis(1_500));
    let page = lib.hybrid(&Lib::query("comets bread"));
    assert!(
        matches!(page.semantic, SemanticStatus::Ready { .. }),
        "{:?}",
        page.semantic
    );
    lib.h.script.with(|s| s.delay = Duration::ZERO);

    // The index isn't open yet: it opens in the background.
    let model = models::serving(&lib.catalog()).unwrap().unwrap();
    lib.root().vectors().forget(&model.id);
    let page = lib.hybrid(&Lib::query("comets bread"));
    assert_eq!(
        page.semantic,
        SemanticStatus::Unavailable {
            reason: SemanticUnavailable::Loading
        }
    );
    assert_eq!(ids(&page), phase1);
    assert!(wait_until(LONG, || lib
        .root()
        .vectors()
        .loaded(&model.id)
        .is_some()));
    assert!(matches!(
        lib.hybrid(&Lib::query("comets bread")).semantic,
        SemanticStatus::Ready { .. }
    ));

    // An internet service without permission: nothing is sent.
    lib.set_role("https://embeddings.invalid/v1");
    let calls = lib.h.script.calls();
    let page = lib.hybrid(&Lib::query("a fresh question"));
    assert_eq!(
        page.semantic,
        SemanticStatus::Unavailable {
            reason: SemanticUnavailable::NotPermitted
        }
    );
    assert_eq!(lib.h.script.calls(), calls);

    // No embedding model at all: Off.
    lib.root()
        .catalog()
        .call(|c| c.execute_batch("DELETE FROM model_roles"))
        .unwrap()
        .unwrap();
    let page = lib.hybrid(&Lib::query("comets bread"));
    assert_eq!(page.semantic, SemanticStatus::Off);
    assert_eq!(ids(&page), phase1);
}
