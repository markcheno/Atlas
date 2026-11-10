//! Phase 1 search benchmark (phase1-spec §13, P1-19; docs/benchmarks.md).
//!
//! Catalogs and reads a text fixture (`scripts/generate-fixtures.mjs
//! --text-kb N`) through the real services and worker, then measures
//! `search_documents` for a query mix while text is being read, warm when
//! idle, and cold after reopening. Also records index size and the time of
//! a full index rebuild. Records measurements; asserts only with
//! `--max-p95-ms`, which `just bench-search` passes on the reference
//! machine (CI runners are noisy).
//!
//! ```text
//! cargo bench --bench search -- --fixture <dir> [--label 30k-text]
//!     [--expect <files>] [--out results.json] [--max-p95-ms 250]
//! cargo bench --bench search -- --eval <questions.json> --data-root <dir>
//!     [--manifest <corpus manifest.json>]
//! ```
//!
//! `--eval` opens an existing library's catalog **read-only** (no lock, no
//! migrations, nothing written) and prints each question's rank. Relevant
//! entries are file paths relative to a library folder, or, with
//! `--manifest`, the corpus's document ids.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use atlas_lib::domain::dto::{DuplicateQuery, JobKind, ProviderKind, SearchQuery};
use atlas_lib::domain::traits::EmbeddingBackend;
use atlas_lib::infrastructure::http_probe::ProbeConfig;
use atlas_lib::infrastructure::keychain::LazyKeychain;
use atlas_lib::infrastructure::local_source::LocalFsSource;
use atlas_lib::infrastructure::system_probe::{OsResourceMonitor, SimulatedMonitor, Simulation};
use atlas_lib::infrastructure::volume::OsVolumeInspector;
use atlas_lib::services::documents::duplicates;
use atlas_lib::services::embedding::chunk_map::ChunkMap;
use atlas_lib::services::embedding::client::ProviderEmbeddingBackend;
use atlas_lib::services::embedding::index::{VectorIndex, VectorPaths, open_readonly};
use atlas_lib::services::embedding::models;
use atlas_lib::services::extraction::db as extraction_db;
use atlas_lib::services::extraction::job::{
    self as extraction_job, ExtractionConfig, ExtractionDeps,
};
use atlas_lib::services::extraction::supervisor::WorkerCommand;
use atlas_lib::services::inventory::{InventoryConfig, InventoryDeps};
use atlas_lib::services::providers::recommend::{QueryProfile, query_profile};
use atlas_lib::services::providers::secrets::{Secret, SecretStore, account_for};
use atlas_lib::services::search::hybrid::Semantic;
use atlas_lib::services::system::ResourceGovernor;
use atlas_lib::services::workspace::{Workspace, WorkspaceConfig};
use atlas_lib::services::{inventory, search, sources, storage};

const DEADLINE: Duration = Duration::from_secs(60 * 60);

/// (name, query): words from the fixture generator's vocabulary.
const MIX: [(&str, &str); 7] = [
    ("single term", "orbital"),
    ("phrase", "\"quiet harbour lights\""),
    ("prefix", "resonan*"),
    ("rare term", "\"Astronomy marker 42\""),
    ("common term", "river"),
    ("any words", "orbital banana"),
    ("name", "book 000042"),
];

fn fail(msg: &str) -> ! {
    eprintln!("bench-search: {msg}");
    std::process::exit(2)
}

#[derive(Default)]
struct Args {
    fixture: Option<PathBuf>,
    label: String,
    expect: Option<u64>,
    out: Option<PathBuf>,
    max_p95_ms: Option<f64>,
    eval: Option<PathBuf>,
    data_root: Option<PathBuf>,
    manifest: Option<PathBuf>,
    semantic: bool,
}

fn parse_args() -> Args {
    let mut a = Args {
        label: "fixture".into(),
        ..Args::default()
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || {
            it.next()
                .unwrap_or_else(|| fail(&format!("{arg} needs a value")))
        };
        match arg.as_str() {
            "--bench" => {}
            "--fixture" => a.fixture = Some(PathBuf::from(value())),
            "--label" => a.label = value(),
            "--expect" => a.expect = Some(value().parse().unwrap_or_else(|_| fail("--expect"))),
            "--out" => a.out = Some(PathBuf::from(value())),
            "--max-p95-ms" => {
                a.max_p95_ms = Some(value().parse().unwrap_or_else(|_| fail("--max-p95-ms")));
            }
            "--eval" => a.eval = Some(PathBuf::from(value())),
            "--data-root" => a.data_root = Some(PathBuf::from(value())),
            "--manifest" => a.manifest = Some(PathBuf::from(value())),
            "--semantic" => a.semantic = true,
            other => fail(&format!("unknown argument {other}")),
        }
    }
    a
}

fn main() {
    let args = parse_args();
    if let Some(questions) = &args.eval {
        let data_root = args
            .data_root
            .as_deref()
            .unwrap_or_else(|| fail("--eval needs --data-root <library folder>"));
        eval(
            questions,
            data_root,
            args.manifest.as_deref(),
            args.semantic,
        );
        return;
    }
    let fixture = args
        .fixture
        .clone()
        .unwrap_or_else(|| fail("--fixture <dir> is required"));
    let fixture = fs::canonicalize(fixture).unwrap_or_else(|_| fail("fixture not found"));
    let report = run(&args, &fixture);
    let text = serde_json::to_string_pretty(&report).unwrap();
    match &args.out {
        Some(out) => fs::write(out, &text).unwrap(),
        None => println!("{text}"),
    }
    if let Some(max) = args.max_p95_ms {
        let p95 = report["whileReading"]["all"]["p95Ms"]
            .as_f64()
            .unwrap_or(f64::MAX);
        if p95 > max {
            fail(&format!("p95 while reading {p95} ms is above {max} ms"));
        }
    }
}

// ----------------------------------------------------------------- setup

fn workspace(config: &Path) -> Workspace {
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
        governor,
        Arc::new(|_| {}),
        ExtractionConfig {
            unthrottled: true,
            ..ExtractionConfig::default()
        },
        WorkerCommand::new(env!("CARGO_BIN_EXE_atlas")),
    );
    Workspace::new(
        WorkspaceConfig {
            config_dir: config.to_path_buf(),
            default_root: config.join("Library"),
        },
        volumes,
    )
    .with_inventory(inventory)
    .with_job_processor(JobKind::Extract, extraction_job::factory(extraction))
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

fn timed(ws: &Workspace, text: &str) -> Duration {
    let root = ws.require_open().unwrap();
    let start = Instant::now();
    search::run(root.reader(), root.root_status(), &query(text)).unwrap();
    start.elapsed()
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

fn one(ws: &Workspace, sql: &'static str) -> i64 {
    ws.require_open()
        .unwrap()
        .reader()
        .read(|c| c.query_row(sql, [], |r| r.get(0)))
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

fn run(args: &Args, fixture: &Path) -> Value {
    let tmp = tempfile::Builder::new()
        .prefix("atlas-bench-search-")
        .tempdir()
        .unwrap();
    let config = tmp.path().join("config");
    let data = tmp.path().join("AtlasData");
    let mut ws = workspace(&config);
    ws.choose(&data).unwrap();
    let root = ws.require_open().unwrap();
    let start = Instant::now();
    let added = sources::add(root, fixture).unwrap();
    inventory::control::enqueue(root, &added.id).unwrap();

    // While reading: the whole mix every 200 ms, once text exists.
    let mut by_kind: Vec<Vec<Duration>> = vec![Vec::new(); MIX.len()];
    let mut first_searchable: Option<f64> = None;
    loop {
        if start.elapsed() > DEADLINE {
            fail("reading did not finish within an hour");
        }
        if reading_done(&ws) {
            break;
        }
        if one(&ws, "SELECT COUNT(*) FROM passages") > 0 {
            first_searchable.get_or_insert(start.elapsed().as_secs_f64());
            for (i, (_, q)) in MIX.iter().enumerate() {
                by_kind[i].push(timed(&ws, q));
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let read_secs = start.elapsed().as_secs_f64();
    let files = one(&ws, "SELECT COUNT(*) FROM file_instances");
    if let Some(expect) = args.expect
        && files as u64 != expect
    {
        fail(&format!("cataloged {files} files, expected {expect}"));
    }
    let passages = one(&ws, "SELECT COUNT(*) FROM passages");
    let text_chars = one(&ws, "SELECT COALESCE(SUM(length(text)), 0) FROM passages");
    let index_bytes = one(
        &ws,
        "SELECT COALESCE(SUM(pgsize), -1) FROM dbstat WHERE name LIKE 'passages_fts%'",
    );

    let while_reading = summarize(&by_kind);

    // Warm and idle.
    let mut warm: Vec<Vec<Duration>> = vec![Vec::new(); MIX.len()];
    for _ in 0..20 {
        for (i, (_, q)) in MIX.iter().enumerate() {
            warm[i].push(timed(&ws, q));
        }
    }
    let idle = summarize(&warm);

    // Library › Duplicates and Settings › Storage, idle (Slice 1.6).
    let root = ws.require_open().unwrap();
    let dup_query = DuplicateQuery {
        source_root_id: None,
        file_type: None,
        include_missing: false,
        page: 0,
        page_size: 20,
    };
    let mut dup_times = Vec::new();
    let mut groups = 0;
    for _ in 0..20 {
        let t = Instant::now();
        let page = duplicates::list(root.reader(), root.root_status(), &dup_query).unwrap();
        dup_times.push(t.elapsed());
        groups = page.total_groups;
    }
    let mut storage_times = Vec::new();
    let mut usage = None;
    for _ in 0..5 {
        let t = Instant::now();
        usage = Some(storage::usage(&data, &OsVolumeInspector).unwrap());
        storage_times.push(t.elapsed());
    }
    let usage = usage.unwrap();

    // A full rebuild of the index (what migration 0009 does once).
    let rebuild = {
        let t = Instant::now();
        ws.require_open()
            .unwrap()
            .catalog()
            .call(|c| c.execute_batch("INSERT INTO passages_fts (passages_fts) VALUES ('rebuild')"))
            .unwrap()
            .unwrap();
        t.elapsed().as_secs_f64()
    };

    // Cold: the first query of each kind after reopening the library.
    ws.shutdown();
    let mut ws = workspace(&config);
    ws.startup();
    let cold: Vec<Value> = MIX
        .iter()
        .map(|(name, q)| {
            json!({ "query": name, "ms": (timed(&ws, q).as_secs_f64() * 100_000.0).round() / 100.0 })
        })
        .collect();
    ws.shutdown();

    let catalog_bytes = ["catalog.sqlite", "catalog.sqlite-wal"]
        .iter()
        .filter_map(|f| fs::metadata(data.join("library").join(f)).ok())
        .map(|m| m.len())
        .sum::<u64>();
    json!({
        "label": args.label,
        "machine": machine(),
        "files": files,
        "passages": passages,
        "textChars": text_chars,
        "readSeconds": (read_secs * 10.0).round() / 10.0,
        "firstSearchableSeconds": first_searchable.map(|s| (s * 10.0).round() / 10.0),
        "catalogBytes": catalog_bytes,
        "indexBytes": if index_bytes < 0 { Value::Null } else { json!(index_bytes) },
        "rebuildSeconds": (rebuild * 100.0).round() / 100.0,
        "whileReading": while_reading,
        "idle": idle,
        "duplicates": { "groups": groups, "latency": percentiles(dup_times) },
        "storage": {
            "latency": percentiles(storage_times),
            "textBytes": usage.text_bytes,
            "indexBytes": usage.index_bytes,
            "catalogBytes": usage.catalog_bytes,
        },
        "cold": cold,
    })
}

fn summarize(by_kind: &[Vec<Duration>]) -> Value {
    let mut all = Vec::new();
    let mut kinds = serde_json::Map::new();
    for (i, (name, _)) in MIX.iter().enumerate() {
        all.extend(by_kind[i].iter().copied());
        kinds.insert((*name).to_owned(), percentiles(by_kind[i].clone()));
    }
    json!({ "all": percentiles(all), "byQuery": kinds })
}

fn machine() -> Value {
    json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "logicalCores": std::thread::available_parallelism().map(|n| n.get()).ok(),
        "ci": std::env::var("CI").is_ok(),
    })
}

// ------------------------------------------------------------------ eval

/// Prints each question's rank against an existing library, read-only.
fn eval(questions: &Path, data_root: &Path, manifest: Option<&Path>, semantic: bool) {
    let db = data_root.join("library").join("catalog.sqlite");
    if !db.is_file() {
        fail(&format!("no catalog at {}", db.display()));
    }
    // Read-only: with the app closed (no WAL), open the file as immutable,
    // which never writes or creates anything beside it.
    let wal = data_root.join("library").join("catalog.sqlite-wal");
    let uri = if wal.exists() {
        format!("file:{}?mode=ro", db.display())
    } else {
        format!("file:{}?immutable=1", db.display())
    };
    let conn = rusqlite::Connection::open_with_flags(
        uri,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .unwrap_or_else(|e| fail(&format!("open the catalog read-only: {e}")));
    let version: u32 = conn
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| {
            r.get(0)
        })
        .unwrap_or(0);
    if version < 9 {
        fail(&format!(
            "this library is at schema {version}; search needs 9 (open it once with Atlas 1.4 or later)"
        ));
    }
    let questions: Value = serde_json::from_str(&fs::read_to_string(questions).unwrap())
        .unwrap_or_else(|_| fail("questions.json is not valid JSON"));
    let ids: Option<Value> =
        manifest.map(|m| serde_json::from_str(&fs::read_to_string(m).unwrap()).unwrap());
    let path_for = |r: &str| -> String {
        ids.as_ref()
            .and_then(|m| m["documents"].as_array())
            .and_then(|docs| docs.iter().find(|d| d["id"] == r))
            .and_then(|d| d["path"].as_str())
            .unwrap_or(r)
            .replace('\\', "/")
    };
    let mut passed = 0;
    let list = questions["questions"]
        .as_array()
        .unwrap_or_else(|| fail("no questions"));
    let meaning = semantic.then(|| meaning_for(&conn, data_root));
    let mut map = ChunkMap::default();
    for q in list {
        let text = q["question"].as_str().unwrap_or_default();
        let relevant: Vec<String> = q["relevant"]
            .as_array()
            .map(|r| r.iter().filter_map(|v| v.as_str()).map(path_for).collect())
            .unwrap_or_default();
        let page = match &meaning {
            None => search::search(&conn, &query(text)),
            Some((index, backend, profile)) => {
                let input = format!("{}{text}", profile.prefix.as_deref().unwrap_or(""));
                let vector = backend
                    .embed(&[input.as_str()])
                    .unwrap_or_else(|e| fail(&format!("embed the question: {e}")))
                    .remove(0);
                search::search_with(
                    &conn,
                    &query(text),
                    Some(Semantic {
                        vector: Box::new(move || Some(vector)),
                        index,
                        chunk_map: &mut map,
                        min_similarity: profile.min_similarity,
                    }),
                )
            }
        }
        .unwrap_or_else(|e| fail(&format!("search: {e}")));
        let rank = page.hits.iter().position(|hit| {
            let mut paths = vec![hit.display_path.replace('\\', "/")];
            if let Some(doc) = &hit.document_id {
                let mut stmt = conn
                    .prepare_cached(
                        "SELECT display_path FROM file_instances WHERE document_id = ?1",
                    )
                    .unwrap();
                paths.extend(
                    stmt.query_map([doc], |r| r.get::<_, String>(0))
                        .unwrap()
                        .filter_map(Result::ok)
                        .map(|p| p.replace('\\', "/")),
                );
            }
            paths.iter().any(|p| {
                relevant
                    .iter()
                    .any(|r| p.ends_with(&format!("/{r}")) || p == r)
            })
        });
        if rank.is_some_and(|r| r < 5) {
            passed += 1;
        }
        println!(
            "{:<6} {:<5} {}",
            q["id"].as_str().unwrap_or("?"),
            rank.map_or("-".to_owned(), |r| (r + 1).to_string()),
            text
        );
    }
    println!(
        "hit@5: {passed}/{} = {:.3}",
        list.len(),
        f64::from(passed) / list.len().max(1) as f64
    );
}

/// The library's meaning index, opened read-only, and its embedding
/// service: the key from `ATLAS_EVAL_EMBED_KEY`, else the keychain.
fn meaning_for(
    conn: &rusqlite::Connection,
    data_root: &Path,
) -> (VectorIndex, ProviderEmbeddingBackend, QueryProfile) {
    let model = models::serving(conn)
        .ok()
        .flatten()
        .unwrap_or_else(|| fail("this library has no meaning index yet"));
    let paths = VectorPaths::for_root(data_root);
    let index = open_readonly(&paths, conn, &model.id).unwrap_or_else(|e| {
        fail(&format!(
            "{e}; open the library in Atlas once, then try again"
        ))
    });
    let profile_id = model
        .provider_profile_id
        .clone()
        .unwrap_or_else(|| fail("the meaning index's AI service was removed"));
    let (kind, base, storage): (String, String, Option<String>) = conn
        .query_row(
            "SELECT kind, base_url, credential_storage FROM provider_profiles WHERE id = ?1",
            [&profile_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap_or_else(|_| fail("the meaning index's AI service was removed"));
    let key = std::env::var("ATLAS_EVAL_EMBED_KEY")
        .ok()
        .filter(|k| !k.is_empty())
        .or_else(|| {
            (storage.as_deref() == Some("keychain"))
                .then(|| {
                    LazyKeychain::default()
                        .get(&account_for(&profile_id))
                        .ok()
                        .flatten()
                        .map(|s| s.expose().to_owned())
                })
                .flatten()
        });
    let backend = ProviderEmbeddingBackend {
        kind: if kind == "ollama" {
            ProviderKind::Ollama
        } else {
            ProviderKind::OpenaiCompatible
        },
        base: base.trim_end_matches('/').to_owned(),
        key: key.map(Secret::new),
        model: model.model_name.clone(),
        // The service returns the role's tested length; the index keeps
        // the model row's (fewer when the model is cut short).
        dims: conn
            .query_row(
                "SELECT dimensions FROM model_roles WHERE role = 'embedding' AND model_id = ?1",
                [&model.model_name],
                |r| r.get::<_, Option<i64>>(0),
            )
            .ok()
            .flatten()
            .map_or(model.dims, |d| (d as usize).max(model.dims)),
        keep: model.dims,
        config: ProbeConfig::embedding(),
        stop: Arc::new(|| false),
    };
    eprintln!(
        "meaning search: {} ({} dimensions, {} vectors)",
        model.model_name,
        model.dims,
        index.len()
    );
    (index, backend, query_profile(&model.model_name))
}
