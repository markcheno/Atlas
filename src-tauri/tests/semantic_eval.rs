//! P2-17 (phase2-spec §10): both evaluation sets through hybrid search on
//! the spike corpus.
//!
//! - On every OS, with the fake backend: the plumbing. Every question of
//!   both sets gets a page with meaning search ready and no error, and
//!   Phase 1's set keeps at least 39 of its 40 (the fake's word-overlap
//!   "meaning" misses q40's long chunk, so partial matches fuse above it;
//!   with a real model the set must stay whole, below).
//! - On the reference machine, against a real model (`just eval-semantic`,
//!   ignored otherwise): every question's rank under text only, meaning
//!   only and hybrid, at the model's dimensions and at 512 (the same vectors
//!   truncated), and the gate: hybrid hit@5 ≥ 0.80 and meaning only above
//!   text only on the vague set. Set `ATLAS_EVAL_EMBED_URL` (the service's
//!   base, e.g. `http://localhost:8000/v1`), `ATLAS_EVAL_EMBED_MODEL`, and
//!   optionally `ATLAS_EVAL_EMBED_KEY`, `ATLAS_EVAL_EMBED_KIND`
//!   (`openai_compatible` or `ollama`) and `ATLAS_EVAL_EMBED_DIMS` (1024).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use atlas_lib::domain::dto::{ProviderKind, SearchQuery, SemanticStatus};
use atlas_lib::domain::traits::EmbeddingBackend;
use atlas_lib::infrastructure::http_probe::ProbeConfig;
use atlas_lib::services::embedding::chunk;
use atlas_lib::services::embedding::chunk_map::ChunkMap;
use atlas_lib::services::embedding::client::ProviderEmbeddingBackend;
use atlas_lib::services::embedding::db as edb;
use atlas_lib::services::embedding::index::VectorIndex;
use atlas_lib::services::embedding::query::QueryEmbedder;
use atlas_lib::services::embedding::vector::truncate_normalize;
use atlas_lib::services::embedding::{models, triggers};
use atlas_lib::services::extraction::db as extraction_db;
use atlas_lib::services::providers::recommend::query_profile;
use atlas_lib::services::providers::secrets::{Secret, SecretVault, account_for};
use atlas_lib::services::search::{self, MeaningContext, hybrid::Semantic};
use atlas_lib::services::workspace::{OpenRoot, Workspace};
use atlas_lib::services::{inventory, sources};
use serde_json::Value;

mod common;
use common::embedding::{EmbedHarness, fast_embedding};
use common::{Env, wait_until};

const LONG: Duration = Duration::from_secs(900);
const PASS: f64 = 0.80;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
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

struct Corpus {
    ws: Workspace,
    h: EmbedHarness,
    _env: Env,
    books: PathBuf,
    manifest: Value,
}

impl Corpus {
    /// The spike corpus read by the real worker, embedded through `h`, with
    /// an embedding role on `base` for `model` at `dims`.
    fn embedded(
        h: EmbedHarness,
        kind: &str,
        base: &str,
        model: &str,
        dims: i64,
        key: Option<&str>,
    ) -> Self {
        let corpus = repo().join("spikes/extraction-bench/corpus");
        let env = Env::new();
        let mut ws = env.workspace_with_embedding(&h);
        ws.choose(&env.base.join("AtlasData")).unwrap();
        let books = env.base.join("Books");
        for dir in ["epub", "pdf"] {
            copy_tree(&corpus.join(dir), &books.join(dir));
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
            inv.active.is_none()
                && ov.active.is_none()
                && ov.counts.total > 0
                && ov.counts.pending == 0
        }));
        if let Some(key) = key {
            h.deps.vault.put(&account_for("svc"), Secret::new(key));
        }
        let (kind, base, model) = (kind.to_owned(), base.to_owned(), model.to_owned());
        let storage = if key.is_some() {
            "'session_only'"
        } else {
            "NULL"
        };
        root.catalog()
            .call(move |c| {
                c.execute(
                    &format!(
                        "INSERT INTO provider_profiles (id, kind, label, base_url,
                             insecure_http_acknowledged, credential_storage, created_at, updated_at)
                         VALUES ('svc', ?1, 'Evaluation', ?2, 1, {storage}, 't', 't')"
                    ),
                    rusqlite::params![kind, base],
                )
                .unwrap();
                c.execute(
                    "INSERT INTO model_roles (role, provider_profile_id, model_id, dimensions,
                         tested_at, updated_at)
                     VALUES ('embedding', 'svc', ?1, ?2, 't', 't')",
                    rusqlite::params![model, dims],
                )
                .unwrap();
            })
            .unwrap();
        triggers::changed(root).unwrap();
        let books = dunce::canonicalize(&books).unwrap();
        let corpus = Self {
            ws,
            h,
            _env: env,
            books,
            manifest: json(&corpus.join("manifest.json")),
        };
        let started = Instant::now();
        assert!(
            wait_until(LONG, || corpus.embedded_all()),
            "embedding never finished"
        );
        eprintln!(
            "embedded the corpus in {:.1} s",
            started.elapsed().as_secs_f64()
        );
        corpus
    }

    fn root(&self) -> &OpenRoot {
        self.ws.require_open().unwrap()
    }

    fn embedded_all(&self) -> bool {
        self.root()
            .reader()
            .read(|c| {
                let Some(model) = models::serving(c)? else {
                    return Ok(false);
                };
                let waiting: i64 = c.query_row(
                    "SELECT COUNT(*) FROM documents d WHERE d.state = 'done' AND NOT EXISTS
                         (SELECT 1 FROM document_embeddings de
                          WHERE de.document_id = d.id AND de.model_id = ?1)",
                    [&model.id],
                    |r| r.get(0),
                )?;
                Ok::<_, rusqlite::Error>(
                    waiting == 0 && self.root().vectors().loaded(&model.id).is_some(),
                )
            })
            .unwrap()
            .unwrap()
    }

    fn relevant(&self, q: &Value) -> Vec<PathBuf> {
        q["relevant"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|id| {
                let doc = self.manifest["documents"]
                    .as_array()?
                    .iter()
                    .find(|d| d["id"] == *id)?;
                Some(self.books.join(doc["path"].as_str()?))
            })
            .collect()
    }

    /// The 1-based rank of the first relevant document among `docs`. Paths
    /// are compared as paths, so Windows separators don't matter.
    fn rank(&self, docs: &[String], relevant: &[PathBuf]) -> Option<usize> {
        docs.iter()
            .position(|doc| {
                let paths: Vec<String> = self
                    .root()
                    .reader()
                    .read(|c| {
                        c.prepare("SELECT display_path FROM file_instances WHERE document_id = ?1")?
                            .query_map([doc], |r| r.get::<_, String>(0))?
                            .collect::<rusqlite::Result<Vec<_>>>()
                    })
                    .unwrap()
                    .unwrap();
                paths
                    .iter()
                    .any(|p| relevant.iter().any(|r| Path::new(p) == r.as_path()))
            })
            .map(|r| r + 1)
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

fn questions(file: &str) -> Vec<Value> {
    json(&repo().join("fixtures/evaluation").join(file))["questions"]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn p2_17_both_sets_run_through_hybrid_search() {
    let h = EmbedHarness::new(fast_embedding());
    let corpus = Corpus::embedded(
        h,
        "openai_compatible",
        "http://localhost:9/v1",
        "test-embed",
        64,
        None,
    );
    let embedder = Arc::new(QueryEmbedder::new(corpus.h.query_backends()));
    let vault = SecretVault::new(None);
    let root = corpus.root();
    let catalog = root.catalog().handle();
    let ctx = MeaningContext {
        vectors: root.vectors(),
        catalog: &catalog,
        embedder: &embedder,
        vault: &vault,
        allow_remote: false,
    };
    for (file, must_pass_all) in [("questions.json", true), ("questions-vague.json", false)] {
        let list = questions(file);
        let mut passed = 0;
        let mut report = Vec::new();
        for q in &list {
            let text = q["question"].as_str().unwrap();
            let page =
                search::run_hybrid(root.reader(), root.root_status(), &ctx, &query(text)).unwrap();
            assert!(
                matches!(page.semantic, SemanticStatus::Ready { .. }),
                "{}: {:?}",
                q["id"],
                page.semantic
            );
            let docs: Vec<String> = page
                .hits
                .iter()
                .filter_map(|h| h.document_id.clone())
                .collect();
            let rank = corpus.rank(&docs, &corpus.relevant(q));
            passed += usize::from(rank.is_some_and(|r| r <= 5));
            report.push(format!(
                "{} {:>3} {:?} {text}",
                q["id"].as_str().unwrap(),
                rank.map_or("-".into(), |r| r.to_string()),
                rank.and_then(|r| page.hits.get(r - 1)).map(|h| h.why),
            ));
        }
        eprintln!("{file}\n{}", report.join("\n"));
        eprintln!(
            "{file}: hybrid hit@5 {passed}/{} (fake backend)",
            list.len()
        );
        if must_pass_all {
            assert!(
                passed + 1 >= list.len(),
                "Phase 1's set through hybrid search: {passed}/{}",
                list.len()
            );
        }
    }
}

/// One configuration's ranks: text only, meaning only, hybrid.
struct Ranks {
    text: Option<usize>,
    meaning: Option<usize>,
    hybrid: Option<usize>,
}

#[test]
#[ignore = "needs a real embedding model: just eval-semantic"]
fn p2_17_the_vague_set_against_a_real_model() {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let (Some(base), Some(model)) = (var("ATLAS_EVAL_EMBED_URL"), var("ATLAS_EVAL_EMBED_MODEL"))
    else {
        panic!("set ATLAS_EVAL_EMBED_URL and ATLAS_EVAL_EMBED_MODEL");
    };
    let key = var("ATLAS_EVAL_EMBED_KEY");
    let kind_text = var("ATLAS_EVAL_EMBED_KIND").unwrap_or_else(|| "openai_compatible".into());
    let kind = if kind_text == "ollama" {
        ProviderKind::Ollama
    } else {
        ProviderKind::OpenaiCompatible
    };
    let dims: usize = var("ATLAS_EVAL_EMBED_DIMS").map_or(1024, |d| d.parse().unwrap());
    let base = base.trim_end_matches('/').to_owned();

    let mut config = fast_embedding();
    config.unthrottled = true;
    let h = EmbedHarness::new(config).over_http(ProbeConfig::embedding());
    let corpus = Corpus::embedded(h, &kind_text, &base, &model, dims as i64, key.as_deref());
    let root = corpus.root();
    let serving = root
        .reader()
        .read(models::serving)
        .unwrap()
        .unwrap()
        .unwrap();
    let stored = root.vectors().loaded(&serving.id).unwrap();
    let profile = query_profile(&model);
    let backend = ProviderEmbeddingBackend {
        kind,
        base,
        key: key.map(Secret::new),
        model: model.clone(),
        dims,
        keep: dims,
        config: ProbeConfig::embedding(),
        stop: Arc::new(|| false),
    };
    eprintln!(
        "model {model} ({kind_text}): the service returns {dims} dimensions, Atlas stores {}; {} chunks; query prefix {:?}; similarity floor {}",
        serving.dims,
        stored.len(),
        profile.prefix,
        profile.min_similarity
    );

    // The comparison: the index at the model's full length and at 512.
    // Whichever Atlas stores is its own index; the other is built here by
    // embedding every chunk again.
    let chunk_texts: Vec<(u64, String)> = root
        .reader()
        .read(|c| {
            let mut stmt = c.prepare(
                "SELECT c.id, p.text, c.char_start, c.char_end FROM chunks c
                 JOIN passages p ON p.id = c.passage_id ORDER BY c.id",
            )?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, i64>(0)? as u64,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)? as usize,
                        r.get::<_, i64>(3)? as usize,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok::<_, rusqlite::Error>(
                rows.into_iter()
                    .filter_map(|(id, text, a, b)| {
                        chunk::slice(&text, chunk::Span { start: a, end: b })
                            .map(|t| (id, t.to_owned()))
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .unwrap()
        .unwrap();
    let build = |want: usize| -> VectorIndex {
        let index = VectorIndex::new("eval", want).unwrap();
        for batch in chunk_texts.chunks(16) {
            let texts: Vec<&str> = batch.iter().map(|(_, t)| t.as_str()).collect();
            let vectors = backend.embed(&texts).unwrap();
            let items: Vec<(u64, Vec<f32>)> = batch
                .iter()
                .zip(vectors)
                .filter_map(|((k, _), v)| truncate_normalize(&v, want).map(|t| (*k, t)))
                .collect();
            index.add_batch(&items, 4).unwrap();
        }
        index
    };
    let started = Instant::now();
    let built = build(if serving.dims < dims { dims } else { 512 });
    eprintln!(
        "built the comparison index in {:.1} s",
        started.elapsed().as_secs_f64()
    );
    let (full, short): (&VectorIndex, &VectorIndex) = if serving.dims < dims {
        (&built, &stored)
    } else {
        (&stored, &built)
    };
    let mut table = Vec::new();
    let mut sims: Vec<(f32, bool)> = Vec::new();
    let list = questions("questions-vague.json");
    let mut query_ms = Vec::new();
    for q in &list {
        let text = q["question"].as_str().unwrap();
        let relevant = corpus.relevant(q);
        let started = Instant::now();
        let input = format!("{}{text}", profile.prefix.as_deref().unwrap_or(""));
        let vector = backend.embed(&[input.as_str()]).unwrap().remove(0);
        query_ms.push(started.elapsed().as_millis());
        let short_vector = truncate_normalize(&vector, 512).unwrap();

        let text_page = root
            .reader()
            .read(|c| search::search(c, &query(text)))
            .unwrap()
            .unwrap();
        let text_docs: Vec<String> = text_page
            .hits
            .iter()
            .filter_map(|h| h.document_id.clone())
            .collect();
        let text_rank = corpus.rank(&text_docs, &relevant);

        let mut ranks = Vec::new();
        for (index, v) in [(full, &vector), (short, &short_vector)] {
            let mut map = ChunkMap::default();
            let unfloored = Semantic {
                vector: Box::new({
                    let v = v.clone();
                    move || Some(v)
                }),
                index,
                chunk_map: &mut map,
                min_similarity: 0.0,
            };
            let meaning = root
                .reader()
                .read(|c| search::meaning_ranking(c, &query(text), unfloored))
                .unwrap()
                .unwrap();
            let mut map2 = ChunkMap::default();
            let floored = Semantic {
                vector: Box::new({
                    let v = v.clone();
                    move || Some(v)
                }),
                index,
                chunk_map: &mut map2,
                min_similarity: profile.min_similarity,
            };
            let hybrid = root
                .reader()
                .read(|c| search::search_with(c, &query(text), Some(floored)))
                .unwrap()
                .unwrap();
            let hybrid_docs: Vec<String> = hybrid
                .hits
                .iter()
                .filter_map(|h| h.document_id.clone())
                .collect();
            ranks.push(Ranks {
                text: text_rank,
                meaning: corpus.rank(&meaning, &relevant),
                hybrid: corpus.rank(&hybrid_docs, &relevant),
            });
        }
        // Similarities of the best chunk per document, relevant or not, for
        // calibrating the floor.
        for (k, d) in full.search(&vector, 50).unwrap() {
            let doc = root
                .reader()
                .read(|c| edb::chunk_places(c, &[k]))
                .unwrap()
                .unwrap()
                .remove(&k)
                .map(|p| p.0);
            if let Some(doc) = doc {
                let is_relevant = corpus.rank(&[doc], &relevant).is_some();
                sims.push((1.0 - d, is_relevant));
            }
        }
        table.push((q["id"].as_str().unwrap().to_owned(), text.to_owned(), ranks));
    }

    let show = |r: Option<usize>| r.map_or("-".to_owned(), |r| r.to_string());
    eprintln!("\nid    text  meaning@{dims} hybrid@{dims}  meaning@512 hybrid@512  question");
    let hit = |r: Option<usize>| usize::from(r.is_some_and(|r| r <= 5));
    let mut totals = [0usize; 5];
    for (id, text, ranks) in &table {
        eprintln!(
            "{id:<5} {:>4}  {:>11} {:>10}  {:>11} {:>10}  {text}",
            show(ranks[0].text),
            show(ranks[0].meaning),
            show(ranks[0].hybrid),
            show(ranks[1].meaning),
            show(ranks[1].hybrid)
        );
        totals[0] += hit(ranks[0].text);
        totals[1] += hit(ranks[0].meaning);
        totals[2] += hit(ranks[0].hybrid);
        totals[3] += hit(ranks[1].meaning);
        totals[4] += hit(ranks[1].hybrid);
    }
    let n = table.len() as f64;
    let rate = |x: usize| x as f64 / n;
    eprintln!(
        "\nhit@5 of {}: text {:.3} · meaning@{dims} {:.3} · hybrid@{dims} {:.3} · meaning@512 {:.3} · hybrid@512 {:.3}",
        table.len(),
        rate(totals[0]),
        rate(totals[1]),
        rate(totals[2]),
        rate(totals[3]),
        rate(totals[4])
    );
    query_ms.sort();
    eprintln!(
        "query embedding: median {} ms, max {} ms",
        query_ms[query_ms.len() / 2],
        query_ms[query_ms.len() - 1]
    );
    let mean = |xs: Vec<f32>| xs.iter().sum::<f32>() / xs.len().max(1) as f32;
    let relevant: Vec<f32> = sims.iter().filter(|s| s.1).map(|s| s.0).collect();
    let other: Vec<f32> = sims.iter().filter(|s| !s.1).map(|s| s.0).collect();
    let low_relevant = relevant.iter().copied().fold(f32::MAX, f32::min);
    eprintln!(
        "similarity of chunks: relevant mean {:.3} (lowest {:.3}), other mean {:.3}",
        mean(relevant.clone()),
        low_relevant,
        mean(other)
    );
    // Phase 1's set through hybrid search with the real model: whole.
    let exact = questions("questions.json");
    let mut whole = 0;
    let mut missed = Vec::new();
    for q in &exact {
        let text = q["question"].as_str().unwrap();
        let input = format!("{}{text}", profile.prefix.as_deref().unwrap_or(""));
        let vector = backend.embed(&[input.as_str()]).unwrap().remove(0);
        // As Atlas searches: its own index, at the length it stores.
        let vector = truncate_normalize(&vector, serving.dims).unwrap();
        let mut map = ChunkMap::default();
        let semantic = Semantic {
            vector: Box::new(move || Some(vector)),
            index: &stored,
            chunk_map: &mut map,
            min_similarity: profile.min_similarity,
        };
        let page = root
            .reader()
            .read(|c| search::search_with(c, &query(text), Some(semantic)))
            .unwrap()
            .unwrap();
        let docs: Vec<String> = page
            .hits
            .iter()
            .filter_map(|h| h.document_id.clone())
            .collect();
        let rank = corpus.rank(&docs, &corpus.relevant(q));
        if rank.is_some_and(|r| r <= 5) {
            whole += 1;
        } else {
            missed.push(format!("{} {:?}", q["id"].as_str().unwrap(), rank));
        }
    }
    eprintln!(
        "Phase 1's set through hybrid at the stored {} dimensions: {whole}/{} {missed:?}",
        serving.dims,
        exact.len()
    );
    assert!(
        rate(totals[2]) >= PASS,
        "hybrid hit@5 {:.3} is below {PASS}",
        rate(totals[2])
    );
    assert_eq!(
        whole,
        exact.len(),
        "Phase 1's set must stay whole with a real model"
    );
    assert!(totals[1] > totals[0], "meaning alone must beat text alone");
}
