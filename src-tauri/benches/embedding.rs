//! Phase 2 embedding benchmark (phase2-spec §13, P2-18): the embedding job
//! on a text fixture through the deterministic backend, with hybrid search
//! timed while it runs.
//!
//!   cargo bench --bench embedding -- --fixture <dir> [--label 1k-text]
//!       [--expect <files>] [--dims 1024] [--query-delay-ms 50]
//!       [--max-p95-ms 250] [--out results.json]
//!
//! The fixture is read first (unthrottled), then an embedding role is set
//! and the job runs at its Run duty cycle. Every 200 ms the search mix and
//! a few vague questions go through hybrid search, whose query embedding
//! waits `--query-delay-ms` as a local service would. The fake backend
//! itself answers at once, so the job's pace measures Atlas's own work
//! (chunking, hashing, writes, index adds), not a model's.
//! After the job, related documents are timed for up to 200 documents,
//! cold and then from the cache (Slice 2.5).
//! Measurements only, unless `--max-p95-ms` is given.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use atlas_lib::domain::dto::{EmbeddingOverview, JobKind, SearchQuery, SemanticStatus};
use atlas_lib::domain::traits::{BackendInfo, EmbedError, EmbeddingBackend};
use atlas_lib::infrastructure::local_source::LocalFsSource;
use atlas_lib::infrastructure::system_probe::{OsResourceMonitor, SimulatedMonitor, Simulation};
use atlas_lib::infrastructure::volume::OsVolumeInspector;
use atlas_lib::services::documents::related;
use atlas_lib::services::embedding::fake::HashedBagOfWordsBackend;
use atlas_lib::services::embedding::job::{
    self as embedding_job, BackendFactory, EmbedConfig, EmbedDeps,
};
use atlas_lib::services::embedding::query::QueryEmbedder;
use atlas_lib::services::embedding::{models, triggers};
use atlas_lib::services::extraction::db as extraction_db;
use atlas_lib::services::extraction::job::{
    self as extraction_job, ExtractionConfig, ExtractionDeps,
};
use atlas_lib::services::extraction::supervisor::WorkerCommand;
use atlas_lib::services::inventory::{InventoryConfig, InventoryDeps};
use atlas_lib::services::providers::secrets::SecretVault;
use atlas_lib::services::search::{self, MeaningContext};
use atlas_lib::services::system::ResourceGovernor;
use atlas_lib::services::workspace::{Workspace, WorkspaceConfig};
use atlas_lib::services::{inventory, sources};

const DEADLINE: Duration = Duration::from_secs(3 * 60 * 60);
/// Documents whose related documents are timed.
const RELATED_ASKS: usize = 200;
const MIX: [&str; 10] = [
    "orbital",
    "\"quiet harbour lights\"",
    "resonan*",
    "\"Astronomy marker 42\"",
    "river",
    "orbital banana",
    "a story about sailors far from home",
    "how bread rises overnight",
    "the motion of planets around a star",
    "an old song about the sea",
];

fn fail(msg: &str) -> ! {
    eprintln!("embedding bench: {msg}");
    std::process::exit(2)
}

struct Args {
    fixture: PathBuf,
    label: String,
    expect: Option<u64>,
    dims: usize,
    query_delay: Duration,
    max_p95_ms: Option<f64>,
    out: Option<PathBuf>,
}

fn parse_args() -> Args {
    let mut fixture = None;
    let mut a = Args {
        fixture: PathBuf::new(),
        label: "fixture".into(),
        expect: None,
        dims: 1024,
        query_delay: Duration::from_millis(50),
        max_p95_ms: None,
        out: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || {
            it.next()
                .unwrap_or_else(|| fail(&format!("{arg} needs a value")))
        };
        match arg.as_str() {
            "--bench" => {}
            "--fixture" => fixture = Some(PathBuf::from(value())),
            "--label" => a.label = value(),
            "--expect" => a.expect = Some(value().parse().unwrap_or_else(|_| fail("--expect"))),
            "--dims" => a.dims = value().parse().unwrap_or_else(|_| fail("--dims")),
            "--query-delay-ms" => {
                a.query_delay = Duration::from_millis(
                    value().parse().unwrap_or_else(|_| fail("--query-delay-ms")),
                );
            }
            "--max-p95-ms" => {
                a.max_p95_ms = Some(value().parse().unwrap_or_else(|_| fail("--max-p95-ms")));
            }
            "--out" => a.out = Some(PathBuf::from(value())),
            other => fail(&format!("unknown argument {other}")),
        }
    }
    a.fixture = fixture.unwrap_or_else(|| fail("--fixture <dir> is required"));
    a
}

/// The fake backend, waiting `delay` per call (a service's latency).
struct Delayed {
    inner: HashedBagOfWordsBackend,
    delay: Duration,
}

impl EmbeddingBackend for Delayed {
    fn embed(&self, inputs: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError> {
        std::thread::sleep(self.delay);
        self.inner.embed(inputs)
    }
    fn describe(&self) -> BackendInfo {
        self.inner.describe()
    }
}

fn backends(delay: Duration) -> BackendFactory {
    Arc::new(move |s, _stop| {
        Box::new(Delayed {
            inner: HashedBagOfWordsBackend::new(s.dims),
            delay,
        })
    })
}

fn workspace(config: &Path, events: Arc<AtomicU64>) -> Workspace {
    let volumes = Arc::new(OsVolumeInspector);
    let monitor = SimulatedMonitor {
        inner: OsResourceMonitor::new(volumes.clone()),
        simulation: Simulation::default(),
    };
    let governor = Arc::new(Mutex::new(ResourceGovernor::new(Box::new(monitor))));
    let inventory = InventoryDeps {
        governor: governor.clone(),
        source: Arc::new(LocalFsSource),
        events: Arc::new(|_| {}),
        config: InventoryConfig {
            unthrottled: true,
            ..InventoryConfig::default()
        },
    };
    let extraction = ExtractionDeps::new(
        governor.clone(),
        Arc::new(|_| {}),
        ExtractionConfig {
            unthrottled: true,
            ..ExtractionConfig::default()
        },
        WorkerCommand::new(env!("CARGO_BIN_EXE_atlas")),
    );
    let embedding = EmbedDeps {
        governor,
        vault: Arc::new(SecretVault::new(None)),
        events: Arc::new(move |_: &EmbeddingOverview| {
            events.fetch_add(1, Ordering::Relaxed);
        }),
        config: EmbedConfig::default(),
        backends: backends(Duration::ZERO),
    };
    Workspace::new(
        WorkspaceConfig {
            config_dir: config.to_path_buf(),
            default_root: config.join("Library"),
        },
        volumes,
    )
    .with_inventory(inventory)
    .with_job_processor(JobKind::Extract, extraction_job::factory(extraction))
    .with_job_processor(JobKind::Embed, embedding_job::factory(embedding))
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

fn percentiles(mut times: Vec<Duration>) -> Value {
    if times.is_empty() {
        return Value::Null;
    }
    times.sort();
    let ms = |d: Duration| (d.as_secs_f64() * 1000.0 * 100.0).round() / 100.0;
    let at = |p: usize| times[(times.len() * p / 100).min(times.len() - 1)];
    json!({ "count": times.len(), "p50Ms": ms(at(50)), "p95Ms": ms(at(95)), "maxMs": ms(*times.last().unwrap()) })
}

fn one(ws: &Workspace, sql: &str) -> i64 {
    let sql = sql.to_owned();
    ws.require_open()
        .unwrap()
        .reader()
        .read(move |c| c.query_row(&sql, [], |r| r.get(0)))
        .unwrap()
        .unwrap_or(-1)
}

fn reading_done(ws: &Workspace) -> bool {
    let root = ws.require_open().unwrap();
    let inv = inventory::control::overview(root).unwrap();
    let ov = root
        .reader()
        .read(|c| extraction_db::overview(c, None))
        .unwrap()
        .unwrap();
    inv.active.is_none() && ov.active.is_none() && ov.counts.total > 0 && ov.counts.pending == 0
}

fn file_bytes(p: &Path) -> u64 {
    fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

fn main() {
    let a = parse_args();
    let fixture = fs::canonicalize(&a.fixture).unwrap_or_else(|_| fail("fixture not found"));
    let tmp = tempfile::Builder::new()
        .prefix("atlas-bench-embedding-")
        .tempdir()
        .unwrap();
    let data = tmp.path().join("AtlasData");
    let events = Arc::new(AtomicU64::new(0));
    let mut ws = workspace(&tmp.path().join("config"), events.clone());
    ws.choose(&data).unwrap();
    let root = ws.require_open().unwrap();

    let start = Instant::now();
    let added = sources::add(root, &fixture).unwrap();
    inventory::control::enqueue(root, &added.id).unwrap();
    while !reading_done(&ws) {
        if start.elapsed() > DEADLINE {
            fail("reading did not finish");
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let read_secs = start.elapsed().as_secs_f64();
    let files = one(&ws, "SELECT COUNT(*) FROM file_instances");
    if let Some(expect) = a.expect
        && files as u64 != expect
    {
        fail(&format!("cataloged {files} files, expected {expect}"));
    }
    let catalog = data.join("library/catalog.sqlite");
    let catalog_before = file_bytes(&catalog);

    // Choose a model: the job starts.
    let dims = a.dims as i64;
    root.catalog()
        .call(move |c| {
            c.execute_batch(&format!(
                "INSERT INTO provider_profiles (id, kind, label, base_url, insecure_http_acknowledged,
                     created_at, updated_at)
                 VALUES ('svc', 'openai_compatible', 'Bench', 'http://localhost:9/v1', 1, 't', 't');
                 INSERT INTO model_roles (role, provider_profile_id, model_id, dimensions,
                     tested_at, updated_at)
                 VALUES ('embedding', 'svc', 'bench-embed', {dims}, 't', 't');"
            ))
        })
        .unwrap()
        .unwrap();
    triggers::changed(root).unwrap();
    let embedder = Arc::new(QueryEmbedder::new(backends(a.query_delay)));
    let vault = SecretVault::new(None);
    let catalog_handle = root.catalog().handle();

    let embed_start = Instant::now();
    let mut times = Vec::new();
    let mut ready = 0u32;
    let waiting = |ws: &Workspace| {
        one(
            ws,
            "SELECT COUNT(*) FROM documents d WHERE d.state = 'done' AND NOT EXISTS
                 (SELECT 1 FROM document_embeddings de WHERE de.document_id = d.id)",
        )
    };
    let docs = one(&ws, "SELECT COUNT(*) FROM documents WHERE state = 'done'");
    loop {
        if embed_start.elapsed() > DEADLINE {
            fail("embedding did not finish");
        }
        if waiting(&ws) == 0
            && one(
                &ws,
                "SELECT COUNT(*) FROM jobs WHERE kind = 'embed'
                 AND state IN ('queued', 'running', 'paused_policy')",
            ) == 0
        {
            break;
        }
        // Every query embedded afresh, as a new question would be.
        embedder.forget_queries();
        for text in MIX {
            let root = ws.require_open().unwrap();
            let t = Instant::now();
            let page = search::run_hybrid(
                root.reader(),
                root.root_status(),
                &MeaningContext {
                    vectors: root.vectors(),
                    catalog: &catalog_handle,
                    embedder: &embedder,
                    vault: &vault,
                    allow_remote: false,
                },
                &query(text),
            )
            .unwrap();
            times.push(t.elapsed());
            ready += u32::from(matches!(page.semantic, SemanticStatus::Ready { .. }));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let embed_secs = embed_start.elapsed().as_secs_f64();
    let model = root
        .reader()
        .read(models::serving)
        .unwrap()
        .unwrap()
        .unwrap();
    let chunks = one(&ws, "SELECT COUNT(*) FROM chunks");
    let vectors = one(&ws, "SELECT COUNT(*) FROM embeddings");
    let index = root.vectors().loaded(&model.id).expect("index open");
    let save = index.save(root.vectors().paths(), root.catalog()).unwrap();
    let raw = vectors as f64 * a.dims as f64 * 4.0;
    let searched = times.len();

    // Related documents (Slice 2.5): up to RELATED_ASKS documents spread
    // over the library, each asked twice: cold, then from the cache.
    let asked: Vec<String> = root
        .reader()
        .read(|c| {
            let mut stmt = c
                .prepare(
                    "SELECT document_id FROM document_embeddings WHERE status = 'done'
                     ORDER BY seq",
                )
                .unwrap();
            let all: Vec<String> = stmt
                .query_map([], |r| r.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            let step = (all.len() / RELATED_ASKS).max(1);
            all.into_iter().step_by(step).take(RELATED_ASKS).collect()
        })
        .unwrap();
    let (mut cold, mut warm, mut found) = (Vec::new(), Vec::new(), 0usize);
    for pass in 0..2 {
        for doc in &asked {
            let t = Instant::now();
            let page = related::run(
                root.reader(),
                root.root_status(),
                root.vectors(),
                &catalog_handle,
                doc,
                related::DEFAULT_LIMIT,
            )
            .unwrap();
            if pass == 0 {
                cold.push(t.elapsed());
                found += page.hits.len();
            } else {
                warm.push(t.elapsed());
            }
        }
    }
    let related_found = found as f64 / asked.len().max(1) as f64;
    let catalog_after = file_bytes(&catalog);
    let result = json!({
        "bench": "embedding",
        "label": a.label,
        "machine": {
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "logicalCores": std::thread::available_parallelism().map(|n| n.get()).ok(),
            "ci": std::env::var("CI").is_ok(),
        },
        "files": files,
        "documents": docs,
        "readSeconds": (read_secs * 10.0).round() / 10.0,
        "dims": a.dims,
        "queryDelayMs": a.query_delay.as_millis() as u64,
        "embedSeconds": (embed_secs * 10.0).round() / 10.0,
        "chunks": chunks,
        "vectors": vectors,
        "chunksPerSecond": (chunks as f64 / embed_secs).round(),
        "catalogGrowthPer1000Documents": ((catalog_after.saturating_sub(catalog_before)) as f64
            / (docs.max(1) as f64 / 1000.0)).round(),
        "catalogBytesPerVector": ((catalog_after.saturating_sub(catalog_before)) as f64
            / vectors.max(1) as f64).round(),
        "indexFileBytes": save.bytes,
        "indexFileToRawF32": (save.bytes as f64 / raw.max(1.0) * 1000.0).round() / 1000.0,
        "saveMs": save.ms,
        "hybridWhileEmbedding": percentiles(times),
        "related": {
            "cold": percentiles(cold),
            "warm": percentiles(warm),
            "meanHits": (related_found * 10.0).round() / 10.0,
        },
        "meaningReadyShare": (f64::from(ready) / searched.max(1) as f64 * 100.0).round() / 100.0,
        "eventsPerSecond": (events.load(Ordering::Relaxed) as f64 / embed_secs * 10.0).round() / 10.0,
    });
    let text = serde_json::to_string_pretty(&result).unwrap();
    match &a.out {
        Some(out) => fs::write(out, &text).unwrap(),
        None => println!("{text}"),
    }
    if let Some(max) = a.max_p95_ms {
        let p95 = result["hybridWhileEmbedding"]["p95Ms"]
            .as_f64()
            .unwrap_or(f64::MAX);
        if p95 > max {
            fail(&format!(
                "hybrid search p95 while embedding {p95} ms is above {max} ms"
            ));
        }
    }
}
