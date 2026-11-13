//! A read library of generated books whose texts are partly replaced by
//! sentences on chosen subjects, embedded with the fake backend: shared by
//! the meaning tests (Slices 2.4 and 2.5).

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use atlas_lib::domain::dto::{FileType, SearchPage, SearchQuery};
use atlas_lib::services::embedding::query::QueryEmbedder;
use atlas_lib::services::embedding::{models, triggers};
use atlas_lib::services::extraction::db as extraction_db;
use atlas_lib::services::providers::secrets::SecretVault;
use atlas_lib::services::search::{self, MeaningContext};
use atlas_lib::services::workspace::{OpenRoot, Workspace};
use atlas_lib::services::{inventory, sources};
use rusqlite::Connection;

use super::embedding::{EmbedHarness, fast_embedding};
use super::{Env, wait_until};

pub const LONG: Duration = Duration::from_secs(180);

pub const SKY: &str =
    "The voyage of the starship carried us past distant comets and glowing nebulae.";
pub const BREAD: &str =
    "Kneading sourdough bread and proofing the dough overnight in a warm kitchen.";
pub const SEA: &str = "Harbor ballads sung by old sailors beneath the swinging lantern light.";

pub struct Lib {
    pub ws: Workspace,
    pub h: EmbedHarness,
    pub env: Env,
    pub embedder: Arc<QueryEmbedder>,
    pub vault: SecretVault,
    /// The documents given a subject, in the order of the texts.
    pub docs: Vec<(String, FileType)>,
    pub sky: String,
    pub sky_type: FileType,
}

impl Lib {
    /// A read library of generated books, three of whose texts are replaced
    /// by sentences on distinct subjects; then embedded with the fake.
    pub fn new() -> Self {
        Self::with_texts(&[SKY, BREAD, SEA], false)
    }

    /// As [`Lib::new`], with one document per text (`copies_first`: the
    /// documents with copies first).
    pub fn with_texts(texts: &[&str], copies_first: bool) -> Self {
        Self::with_workspace(texts, copies_first, |ws| ws)
    }

    /// As [`Lib::with_texts`], with the workspace changed by `configure`
    /// before the library opens (a clock, trail events).
    pub fn with_workspace(
        texts: &[&str],
        copies_first: bool,
        configure: impl FnOnce(Workspace) -> Workspace,
    ) -> Self {
        Self::custom(texts, copies_first, 2, &[], configure)
    }

    /// As [`Lib::with_workspace`], with `copies` extra copies of files and
    /// `extra` files (such as spike-corpus books) put in the folder too.
    pub fn custom(
        texts: &[&str],
        copies_first: bool,
        copies: u32,
        extra: &[&Path],
        configure: impl FnOnce(Workspace) -> Workspace,
    ) -> Self {
        let copies = copies.to_string();
        Self::with_fixture(
            &["--files", "10", "--text-kb", "2", "--copies", &copies],
            texts,
            copies_first,
            extra,
            configure,
        )
    }

    /// As [`Lib::custom`], with the fixture generator's own arguments.
    pub fn with_fixture(
        fixture: &[&str],
        texts: &[&str],
        copies_first: bool,
        extra: &[&Path],
        configure: impl FnOnce(Workspace) -> Workspace,
    ) -> Self {
        let texts: Vec<String> = texts.iter().map(|t| (*t).to_owned()).collect();
        let order = if copies_first {
            "(SELECT COUNT(*) FROM file_instances f WHERE f.document_id = d.id) DESC, d.id"
        } else {
            "d.id"
        };
        let limit = texts.len();
        let h = EmbedHarness::new(fast_embedding());
        let env = Env::new();
        let mut ws = configure(env.workspace_with_embedding(&h));
        ws.choose(&env.base.join("AtlasData")).unwrap();
        let books = env.base.join("Books");
        let out = Command::new("node")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/generate-fixtures.mjs"))
            .arg(&books)
            .args(fixture)
            .output()
            .unwrap();
        assert!(out.status.success());
        if !extra.is_empty() {
            let dir = books.join("corpus");
            std::fs::create_dir_all(&dir).unwrap();
            for file in extra {
                std::fs::copy(file, dir.join(file.file_name().unwrap())).unwrap();
            }
        }
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
            .call(move |c| {
                let docs: Vec<(String, String)> = c
                    .prepare(&format!(
                            "SELECT d.id, (SELECT f.file_type FROM file_instances f
                                           WHERE f.document_id = d.id LIMIT 1)
                             FROM documents d WHERE d.state = 'done' ORDER BY {order} LIMIT {limit}"
                        ))
                    .unwrap()
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .unwrap()
                    .map(Result::unwrap)
                    .collect();
                for ((doc, _), text) in docs.iter().zip(&texts) {
                    c.execute(
                        "UPDATE passages SET text = ?2 WHERE document_id = ?1",
                        rusqlite::params![doc, text],
                    )
                    .unwrap();
                }
                docs
            })
            .unwrap();
        let docs: Vec<(String, FileType)> = docs
            .into_iter()
            .map(|(id, kind)| {
                let kind = if kind == "epub" {
                    FileType::Epub
                } else {
                    FileType::Pdf
                };
                (id, kind)
            })
            .collect();
        // No texts: no subject documents (the resume scenario).
        let sky_type = docs.first().map_or(FileType::Pdf, |d| d.1);
        let embedder = Arc::new(QueryEmbedder::new(h.query_backends()));
        let lib = Self {
            ws,
            embedder,
            vault: SecretVault::new(None),
            sky: docs.first().map(|d| d.0.clone()).unwrap_or_default(),
            sky_type,
            docs,
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

    pub fn root(&self) -> &OpenRoot {
        self.ws.require_open().unwrap()
    }

    pub fn catalog(&self) -> Connection {
        Connection::open(self.env.base.join("AtlasData/library/catalog.sqlite")).unwrap()
    }

    pub fn set_role(&self, base_url: &str) {
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

    pub fn indexed(&self) -> bool {
        let c = self.catalog();
        let Ok(Some(model)) = models::serving(&c) else {
            return false;
        };
        let waiting: i64 = c
            .query_row(
                // A copy kept out of a book in two formats is never indexed
                // (Slice 4.5.1).
                "SELECT COUNT(*) FROM documents d WHERE d.state = 'done' AND NOT EXISTS
                     (SELECT 1 FROM document_embeddings de
                      WHERE de.document_id = d.id AND de.model_id = ?1)
                   AND NOT EXISTS (SELECT 1 FROM book_pairs bp
                                   WHERE bp.hidden_document_id = d.id)",
                [&model.id],
                |r| r.get(0),
            )
            .unwrap();
        waiting == 0 && self.root().vectors().loaded(&model.id).is_some()
    }

    pub fn query(text: &str) -> SearchQuery {
        SearchQuery {
            text: text.into(),
            source_root_id: None,
            file_type: None,
            include_missing: false,
            page: 0,
            page_size: 20,
        }
    }

    pub fn hybrid(&self, q: &SearchQuery) -> SearchPage {
        let root = self.root();
        search::run_hybrid(
            root.reader(),
            root.root_status(),
            &MeaningContext {
                vectors: root.vectors(),
                catalog: &root.catalog().handle(),
                embedder: &self.embedder,
                vault: &self.vault,
                allow_remote: atlas_lib::services::providers::privacy::remote_embedding(
                    &root.settings().privacy,
                ),
            },
            q,
        )
        .unwrap()
    }

    pub fn text_only(&self, q: &SearchQuery) -> SearchPage {
        search::run(self.root().reader(), self.root().root_status(), q).unwrap()
    }

    /// "Means" the sky document without sharing a word with it.
    pub fn alias_to_sky(&self, query: &str) {
        let q = query.to_owned();
        self.h.script.with(|s| {
            s.alias.insert(q, SKY.into());
        });
    }
}
