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
        panic!(
            "just eval-semantic needs, for example:\n\
             ATLAS_EVAL_EMBED_URL=http://localhost:8000/v1\n\
             ATLAS_EVAL_EMBED_MODEL=Qwen3-Embedding-0.6B-8bit\n\
             and, if the service needs one, ATLAS_EVAL_EMBED_KEY=<key>. Optional:\n\
             ATLAS_EVAL_EMBED_KIND (openai_compatible or ollama), ATLAS_EVAL_EMBED_DIMS (1024).\n\
             Set them in this shell, or on the same line as the command."
        );
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
                weak_margin: atlas_lib::services::providers::recommend::DEFAULT_WEAK_MARGIN,
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
                weak_margin: profile.weak_margin,
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
            weak_margin: profile.weak_margin,
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

/// Embedding throughput, index recall and the meaning-only share against a
/// real model (phase2-spec §13; Slice 2.8), run by its owner with
/// `just eval-embed` and the `ATLAS_EVAL_EMBED_*` variables of
/// `eval-semantic`:
///
/// - **Throughput:** every chunk of the spike corpus embedded again in the
///   job's batches (at most 16 chunks and 16,000 characters), timed at the
///   service. Tokens are the service's own count (`usage.prompt_tokens`,
///   Ollama's `prompt_eval_count`) when it reports one; otherwise
///   characters ÷ 4, said to be an estimate. The service always answers at
///   its full length; 512 is Atlas truncating, so its pace is the same.
/// - **Recall@10 of the index on real vectors**, at the full length and at
///   512: the vague questions (with the model's query instruction) and 200
///   chunks as queries, the index's top 10 against an exact cosine scan.
/// - **The meaning-only share:** vague questions with a relevant document in
///   the top 5 by hybrid search (as Atlas searches, its own index) but not
///   by text alone.
#[test]
#[ignore = "needs a real embedding model: just eval-embed"]
fn embedding_throughput_and_recall_against_a_real_model() {
    use atlas_lib::infrastructure::http_probe::{
        EmbedReply, client, embedding_vectors, embeddings_request, post_embeddings,
    };
    use std::collections::HashSet;

    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let (Some(base), Some(model)) = (var("ATLAS_EVAL_EMBED_URL"), var("ATLAS_EVAL_EMBED_MODEL"))
    else {
        panic!(
            "just eval-embed needs, for example:\n\
             ATLAS_EVAL_EMBED_URL=http://localhost:8000/v1\n\
             ATLAS_EVAL_EMBED_MODEL=Qwen3-Embedding-0.6B-8bit\n\
             and, if the service needs one, ATLAS_EVAL_EMBED_KEY=<key>. Optional:\n\
             ATLAS_EVAL_EMBED_KIND (openai_compatible or ollama), ATLAS_EVAL_EMBED_DIMS (1024)."
        );
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

    // The job's batches: at most 16 chunks and 16,000 characters.
    let mut batches: Vec<Vec<usize>> = Vec::new();
    let (mut current, mut chars) = (Vec::new(), 0usize);
    for (i, (_, t)) in chunk_texts.iter().enumerate() {
        let n = t.chars().count();
        if !current.is_empty() && (current.len() == 16 || chars + n > 16_000) {
            batches.push(std::mem::take(&mut current));
            chars = 0;
        }
        current.push(i);
        chars += n;
    }
    if !current.is_empty() {
        batches.push(current);
    }

    let http = client(&ProbeConfig::embedding());
    let cap = ProbeConfig::embedding().max_body_bytes;
    let embed = |inputs: &[&str]| -> (Vec<Vec<f32>>, Option<u64>) {
        let (path, body) = embeddings_request(kind, &model, inputs);
        let url = format!("{base}{path}");
        let reply = tauri::async_runtime::block_on(post_embeddings(
            &http,
            &url,
            key.as_deref(),
            &body,
            cap,
        ));
        let EmbedReply::Ok(answer) = reply else {
            panic!("the service refused an embedding request: {reply:?}");
        };
        let tokens = answer
            .pointer("/usage/prompt_tokens")
            .or_else(|| answer.pointer("/usage/total_tokens"))
            .or_else(|| answer.get("prompt_eval_count"))
            .and_then(Value::as_u64);
        let mut vectors = embedding_vectors(kind, &answer).expect("vectors");
        for v in &mut vectors {
            let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            v.iter_mut().for_each(|x| *x /= norm);
        }
        (vectors, tokens)
    };

    let started = Instant::now();
    let (mut full, mut tokens, mut counted) = (Vec::new(), 0u64, true);
    let total_chars: usize = chunk_texts.iter().map(|(_, t)| t.chars().count()).sum();
    for batch in &batches {
        let texts: Vec<&str> = batch.iter().map(|&i| chunk_texts[i].1.as_str()).collect();
        let (vectors, used) = embed(&texts);
        assert_eq!(vectors.len(), texts.len());
        match used {
            Some(n) => tokens += n,
            None => counted = false,
        }
        full.extend(vectors);
    }
    let secs = started.elapsed().as_secs_f64();
    let (tokens, how) = if counted {
        (tokens as f64, "counted by the service")
    } else {
        (total_chars as f64 / 4.0, "estimated as characters ÷ 4")
    };
    eprintln!(
        "model {model} ({kind_text}): {} chunks in {} requests, {:.1} s: {:.1} chunks/s, {:.0} tokens/s ({how}), {:.0} characters/s; the service returns {} dimensions, Atlas stores {}",
        full.len(),
        batches.len(),
        secs,
        full.len() as f64 / secs,
        tokens / secs,
        total_chars as f64 / secs,
        full[0].len(),
        serving.dims,
    );

    // Recall@10 of the index against an exact scan, on these vectors.
    let keys: Vec<u64> = chunk_texts.iter().map(|(k, _)| *k).collect();
    let questions_list = questions("questions-vague.json");
    let query_texts: Vec<String> = questions_list
        .iter()
        .map(|q| {
            format!(
                "{}{}",
                profile.prefix.as_deref().unwrap_or(""),
                q["question"].as_str().unwrap()
            )
        })
        .collect();
    let question_vectors: Vec<Vec<f32>> = query_texts
        .iter()
        .flat_map(|t| embed(&[t.as_str()]).0)
        .collect();
    let step = (full.len() / 200).max(1);
    let lengths: Vec<usize> = [full[0].len(), 512]
        .into_iter()
        .filter(|&w| w <= full[0].len())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .rev()
        .collect();
    for want in lengths {
        let vectors: Vec<Vec<f32>> = full
            .iter()
            .map(|v| truncate_normalize(v, want).unwrap())
            .collect();
        let index = VectorIndex::new("recall", want).unwrap();
        let items: Vec<(u64, Vec<f32>)> = keys.iter().copied().zip(vectors.clone()).collect();
        index.add_batch(&items, 4).unwrap();
        let mut queries: Vec<Vec<f32>> = question_vectors
            .iter()
            .map(|v| truncate_normalize(v, want).unwrap())
            .collect();
        queries.extend(vectors.iter().step_by(step).take(200).cloned());
        let mut found = 0usize;
        for q in &queries {
            let mut exact: Vec<(f32, u64)> = vectors
                .iter()
                .zip(&keys)
                .map(|(v, k)| (v.iter().zip(q).map(|(a, b)| a * b).sum::<f32>(), *k))
                .collect();
            exact.sort_by(|a, b| b.0.total_cmp(&a.0));
            let truth: HashSet<u64> = exact.iter().take(10).map(|e| e.1).collect();
            let got = index.search(q, 10).unwrap();
            found += got.iter().filter(|(k, _)| truth.contains(k)).count();
        }
        eprintln!(
            "recall@10 of the index at {want} dimensions: {:.4} over {} queries ({} questions, {} chunks)",
            found as f64 / (queries.len() * 10) as f64,
            queries.len(),
            question_vectors.len(),
            queries.len() - question_vectors.len(),
        );
    }

    // The meaning-only share: as Atlas searches, its own index and floor.
    let (mut text_hits, mut hybrid_hits, mut only_meaning) = (0, 0, Vec::new());
    for (q, vector) in questions_list.iter().zip(&question_vectors) {
        let text = q["question"].as_str().unwrap();
        let relevant = corpus.relevant(q);
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
        let by_text = corpus.rank(&text_docs, &relevant).is_some_and(|r| r <= 5);
        let vector = truncate_normalize(vector, serving.dims).unwrap();
        let mut map = ChunkMap::default();
        let semantic = Semantic {
            vector: Box::new(move || Some(vector)),
            index: &stored,
            chunk_map: &mut map,
            min_similarity: profile.min_similarity,
            weak_margin: profile.weak_margin,
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
        let by_hybrid = corpus.rank(&docs, &relevant).is_some_and(|r| r <= 5);
        text_hits += usize::from(by_text);
        hybrid_hits += usize::from(by_hybrid);
        if by_hybrid && !by_text {
            only_meaning.push(q["id"].as_str().unwrap().to_owned());
        }
    }
    let n = questions_list.len();
    eprintln!(
        "vague set at the stored {} dimensions: text alone {text_hits}/{n}, hybrid {hybrid_hits}/{n}; found only with meaning: {}/{n} ({:.0}%) {only_meaning:?}",
        serving.dims,
        only_meaning.len(),
        100.0 * only_meaning.len() as f64 / n as f64,
    );
}

/// Ask Atlas against real models (Slice 2.6, phase2-spec §13), run by its
/// owner with `just eval-ask`: the corpus embedded as above, then every
/// vague question asked through the chat model. Reports, per question and
/// in all, the time to the first word and to the whole answer, the
/// sources sent, cited and invented, and whether a relevant document was
/// among the sources. Asserts only that no invented citation survives.
/// Set the `ATLAS_EVAL_EMBED_*` variables as for `eval-semantic`, plus
/// `ATLAS_EVAL_CHAT_URL`, `ATLAS_EVAL_CHAT_MODEL`, and optionally
/// `ATLAS_EVAL_CHAT_KEY` and `ATLAS_EVAL_CHAT_KIND`.
#[test]
#[ignore = "needs real embedding and chat models: just eval-ask"]
fn ask_atlas_against_real_models() {
    use atlas_lib::domain::dto::{AnswerErrorCode, AnswerEvent, AnswerStep, AskQuery, AskReply};
    use atlas_lib::services::answers::stream::ChatLimits;
    use atlas_lib::services::answers::{self, AnswerEvents, AskDeps};
    use atlas_lib::services::embedding::job::http_backends;
    use atlas_lib::services::providers::{privacy, url_policy};
    use std::sync::Mutex;

    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let (Some(base), Some(model), Some(chat_model)) = (
        var("ATLAS_EVAL_EMBED_URL"),
        var("ATLAS_EVAL_EMBED_MODEL"),
        var("ATLAS_EVAL_CHAT_MODEL"),
    ) else {
        panic!(
            "just eval-ask needs, for example:\n\
             ATLAS_EVAL_EMBED_URL=http://localhost:8000/v1\n\
             ATLAS_EVAL_EMBED_MODEL=Qwen3-Embedding-0.6B-8bit\n\
             ATLAS_EVAL_CHAT_MODEL=<a chat model the service has>\n\
             and, if the service needs one, ATLAS_EVAL_EMBED_KEY=<key>.\n\
             The chat model uses the same service and key unless ATLAS_EVAL_CHAT_URL\n\
             and ATLAS_EVAL_CHAT_KEY say otherwise. Optional: ATLAS_EVAL_EMBED_KIND,\n\
             ATLAS_EVAL_CHAT_KIND (openai_compatible or ollama), ATLAS_EVAL_EMBED_DIMS."
        );
    };
    let chat_base = var("ATLAS_EVAL_CHAT_URL").unwrap_or_else(|| base.clone());
    let kind_text = var("ATLAS_EVAL_EMBED_KIND").unwrap_or_else(|| "openai_compatible".into());
    let chat_kind = var("ATLAS_EVAL_CHAT_KIND").unwrap_or_else(|| "openai_compatible".into());
    let dims: i64 = var("ATLAS_EVAL_EMBED_DIMS").map_or(1024, |d| d.parse().unwrap());
    let base = base.trim_end_matches('/').to_owned();
    let chat_base = chat_base.trim_end_matches('/').to_owned();

    let mut config = fast_embedding();
    config.unthrottled = true;
    let h = EmbedHarness::new(config).over_http(ProbeConfig::embedding());
    let vault = Arc::clone(&h.deps.vault);
    let corpus = Corpus::embedded(
        h,
        &kind_text,
        &base,
        &model,
        dims,
        var("ATLAS_EVAL_EMBED_KEY").as_deref(),
    );
    let root = corpus.root();
    let chat_key = var("ATLAS_EVAL_CHAT_KEY").or_else(|| {
        var("ATLAS_EVAL_CHAT_URL")
            .is_none()
            .then(|| var("ATLAS_EVAL_EMBED_KEY"))
            .flatten()
    });
    if let Some(key) = &chat_key {
        vault.put(&account_for("chat"), Secret::new(key.clone()));
    }
    let storage = if chat_key.is_some() {
        "'session_only'"
    } else {
        "NULL"
    };
    let (kind, url, name) = (chat_kind.clone(), chat_base.clone(), chat_model.clone());
    root.catalog()
        .call(move |c| {
            c.execute(
                &format!(
                    "INSERT INTO provider_profiles (id, kind, label, base_url,
                         insecure_http_acknowledged, credential_storage, created_at, updated_at)
                     VALUES ('chat', ?1, 'Evaluation chat', ?2, 1, {storage}, 't', 't')"
                ),
                rusqlite::params![kind, url],
            )
            .unwrap();
            c.execute(
                "INSERT INTO model_roles (role, provider_profile_id, model_id, dimensions,
                     tested_at, updated_at)
                 VALUES ('chat', 'chat', ?1, NULL, 't', 't')",
                [&name],
            )
            .unwrap();
        })
        .unwrap();

    let embedder = Arc::new(QueryEmbedder::new(http_backends(ProbeConfig::embedding())));
    let log: Arc<Mutex<Vec<(Instant, AnswerEvent)>>> = Arc::default();
    let sink = Arc::clone(&log);
    let events: AnswerEvents =
        Arc::new(move |e: &AnswerEvent| sink.lock().unwrap().push((Instant::now(), e.clone())));
    let privacy_settings = root.settings().privacy.clone();
    let ask = |question: &str| {
        answers::ask(
            &AskDeps {
                reader: root.reader(),
                meaning: MeaningContext {
                    vectors: root.vectors(),
                    catalog: &root.catalog().handle(),
                    embedder: &embedder,
                    vault: &vault,
                    allow_remote: privacy::remote_embedding(&privacy_settings),
                },
                vault: &vault,
                privacy: &privacy_settings,
                locate: url_policy::location_of,
                limits: ChatLimits::default(),
                events: Arc::clone(&events),
                answers: root.answers(),
                recorder: None,
            },
            &AskQuery {
                question: question.into(),
                source_root_id: None,
                file_type: None,
            },
        )
        .unwrap()
    };
    // How far the best meaning match is above the model's floor: what the
    // weak-retrieval margin is tuned from.
    let margin = |question: &str| -> Option<f32> {
        let query = atlas_lib::domain::dto::SearchQuery {
            text: question.into(),
            source_root_id: None,
            file_type: None,
            include_missing: false,
            page: 0,
            page_size: answers::retrieve::DOCUMENTS as u32,
        };
        let meaning = MeaningContext {
            vectors: root.vectors(),
            catalog: &root.catalog().handle(),
            embedder: &embedder,
            vault: &vault,
            allow_remote: privacy::remote_embedding(&privacy_settings),
        };
        let r = root
            .reader()
            .read(|c| {
                atlas_lib::services::search::retrieve(
                    c,
                    &meaning,
                    &query,
                    answers::retrieve::DOCUMENTS,
                )
            })
            .unwrap()
            .unwrap();
        Some(r.best_similarity? - r.min_similarity?)
    };
    // The first query embedding loads the model; don't time it.
    let _ = corpus.root().reader().read(|c| {
        embedder
            .target(c, &vault, true)
            .map(|t| embedder.start(&t, "warm up"))
    });
    std::thread::sleep(Duration::from_secs(5));
    eprintln!("chat model {chat_model} ({chat_kind}) at {chat_base}");

    /// One set's measures: the vague set (the library covers each) or the
    /// off set (it covers none).
    #[derive(Default)]
    struct Tally {
        asked: usize,
        nothing_found: usize,
        firsts: Vec<f64>,
        wholes: Vec<f64>,
        supplied: usize,
        cited: usize,
        invented: usize,
        relevant: usize,
        weak: usize,
        errors: usize,
        no_answer: usize,
        /// Answers saying the excerpts don't cover the question.
        not_covered: usize,
        /// … and of those, how many cite nothing, as the prompt asks.
        not_covered_uncited: usize,
        margins: Vec<f32>,
    }
    // An answer opening by saying the excerpts don't cover the question.
    let says_not_covered = |answer: &str| {
        let first = answer
            .trim()
            .split(['.', '\n'])
            .next()
            .unwrap_or("")
            .to_lowercase();
        [
            "do not",
            "don't",
            "does not",
            "doesn't",
            "no excerpt",
            "none of",
            // Not "nothing": "ruling by doing nothing" is a question's words.
        ]
        .iter()
        .any(|p| first.contains(p))
            && ["excerpt", "source", "librar"]
                .iter()
                .any(|p| first.contains(p))
    };
    let mut transcript = String::new();
    let mut tallies = Vec::new();
    for (set, file) in [
        ("vague", "questions-vague.json"),
        ("off", "questions-off.json"),
    ] {
        let mut t = Tally::default();
        for q in questions(file) {
            let qid = q["id"].as_str().unwrap();
            let text = q["question"].as_str().unwrap();
            let close = margin(text);
            t.margins.extend(close);
            let start = Instant::now();
            let started = match ask(text) {
                AskReply::Started(s) => s,
                AskReply::NothingFound { .. } => {
                    t.nothing_found += 1;
                    eprintln!("{qid} nothing found · margin {close:?}");
                    transcript.push_str(&format!(
                        "## {qid} · margin {} · nothing found\n\n{text}\n\n",
                        close.map_or("—".into(), |m| format!("{m:.2}"))
                    ));
                    continue;
                }
                AskReply::NeedsDisclosure(_) => {
                    panic!("the evaluation doesn't send to an internet service")
                }
            };
            t.asked += 1;
            let id = started.answer_id.clone();
            let steps = |log: &Mutex<Vec<(Instant, AnswerEvent)>>| -> Vec<(Instant, AnswerStep)> {
                log.lock()
                    .unwrap()
                    .iter()
                    .filter(|(_, e)| e.answer_id == id)
                    .map(|(t, e)| (*t, e.step.clone()))
                    .collect()
            };
            let ended = |s: &AnswerStep| {
                matches!(
                    s,
                    AnswerStep::Done { .. } | AnswerStep::Stopped | AnswerStep::Error { .. }
                )
            };
            assert!(
                wait_until(Duration::from_secs(600), || steps(&log)
                    .iter()
                    .any(|(_, s)| ended(s))),
                "no answer within ten minutes"
            );
            let steps = steps(&log);
            let first = steps
                .iter()
                .find(|(_, s)| matches!(s, AnswerStep::Delta { .. }))
                .map(|(t, _)| t.duration_since(start));
            let (end_at, last) = steps.last().unwrap();
            let answer: String = steps
                .iter()
                .filter_map(|(_, s)| match s {
                    AnswerStep::Delta { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            // No invented citation survives.
            for n in answer
                .split('[')
                .skip(1)
                .filter_map(|rest| rest.split(']').next()?.parse::<u32>().ok())
            {
                assert!(
                    (1..=started.sources.len() as u32).contains(&n),
                    "[{n}] in {answer:?}"
                );
            }
            let (cited, dropped) = match last {
                AnswerStep::Done { cited, dropped } | AnswerStep::Error { cited, dropped, .. } => {
                    (cited.len(), *dropped)
                }
                _ => (0, 0),
            };
            let relevant = corpus.relevant(&q);
            let hit = if relevant.is_empty() {
                None
            } else {
                let docs: Vec<String> = started
                    .sources
                    .iter()
                    .map(|s| s.document_id.clone())
                    .collect();
                Some(corpus.rank(&docs, &relevant).is_some())
            };
            t.relevant += usize::from(hit == Some(true));
            t.supplied += started.sources.len();
            t.cited += cited;
            t.invented += dropped as usize;
            t.weak += usize::from(started.weak);
            let not_covered = match last {
                AnswerStep::Error { code, .. } => {
                    t.errors += 1;
                    t.no_answer += usize::from(*code == AnswerErrorCode::NoAnswer);
                    false
                }
                _ => says_not_covered(&answer),
            };
            t.not_covered += usize::from(not_covered);
            t.not_covered_uncited += usize::from(not_covered && cited == 0);
            if let Some(f) = first {
                t.firsts.push(f.as_secs_f64());
            }
            t.wholes.push(end_at.duration_since(start).as_secs_f64());
            transcript.push_str(&format!(
                "## {qid} · margin {} · {}{}\n\n{text}\n\n{}\n\n",
                close.map_or("—".into(), |m| format!("{m:.2}")),
                if started.weak { "weak" } else { "not weak" },
                if not_covered { " · not covered" } else { "" },
                match last {
                    AnswerStep::Error { code, message, .. } => {
                        format!("error {code:?}: {message}")
                    }
                    _ => answer.clone(),
                }
            ));
            eprintln!(
                "{qid} first {:>6} whole {:>6.1} s · margin {} · {} sources, {cited} cited, {dropped} invented{}{} · relevant source: {} · {}",
                first.map_or("—".into(), |f| format!("{:.1} s", f.as_secs_f64())),
                end_at.duration_since(start).as_secs_f64(),
                close.map_or("—".into(), |m| format!("{m:.2}")),
                started.sources.len(),
                if started.weak { " · weak" } else { "" },
                if not_covered { " · not covered" } else { "" },
                match hit {
                    Some(true) => "yes",
                    Some(false) => "no",
                    None => "n/a",
                },
                match last {
                    AnswerStep::Error { code, .. } => format!("error {code:?}"),
                    _ => answer
                        .chars()
                        .take(80)
                        .collect::<String>()
                        .replace('\n', " "),
                }
            );
        }
        tallies.push((set, t));
    }
    let pct = |v: &[f64], p: f64| -> f64 {
        let mut v = v.to_vec();
        if v.is_empty() {
            return f64::NAN;
        }
        v.sort_by(f64::total_cmp);
        v[((v.len() as f64 - 1.0) * p).round() as usize]
    };
    let spread = |v: &[f32]| -> String {
        let mut v = v.to_vec();
        if v.is_empty() {
            return "—".into();
        }
        v.sort_by(f32::total_cmp);
        format!(
            "{:.2} / {:.2} / {:.2}",
            v[0],
            v[v.len() / 2],
            v[v.len() - 1]
        )
    };
    for (set, t) in &tallies {
        let total = t.asked + t.nothing_found;
        eprintln!(
            "Ask Atlas, {set} set, {total} questions: nothing found {}; first word p50 {:.1} s / p95 {:.1} s; whole answer p50 {:.1} s / p95 {:.1} s; a relevant document among the sources {}/{}; citation rate {:.2} (cited ÷ supplied, {}/{}); invented citations {} ({:.2} per answer); weak retrieval {}; not covered {} (citing nothing {}); errors {} (no answer {}); margin above the floor min / median / max {}",
            t.nothing_found,
            pct(&t.firsts, 0.5),
            pct(&t.firsts, 0.95),
            pct(&t.wholes, 0.5),
            pct(&t.wholes, 0.95),
            t.relevant,
            if *set == "off" { 0 } else { t.asked },
            t.cited as f64 / t.supplied.max(1) as f64,
            t.cited,
            t.supplied,
            t.invented,
            t.invented as f64 / t.asked.max(1) as f64,
            t.weak,
            t.not_covered,
            t.not_covered_uncited,
            t.errors,
            t.no_answer,
            spread(&t.margins),
        );
    }
    eprintln!(
        "weak retrieval is a margin below {:.2} (the model's, from the recommendation catalog)",
        atlas_lib::services::providers::recommend::query_profile(&model).weak_margin
    );
    let out = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("eval-ask-answers.md");
    std::fs::write(&out, transcript).unwrap();
    eprintln!("the whole answers: {}", out.display());
}
