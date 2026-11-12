//! Phase 3 knowledge store benchmark (phase3-spec §13; docs/benchmarks.md).
//!
//! Builds a small read library (the fixture generator's books, so anchors
//! resolve against a real catalog), then loads a generated store of
//! `--trails` trails and `--steps` steps through Import, and measures:
//! - the import: its dry run and applying it;
//! - the Trails list, with and without a filter (p50, p95);
//! - opening a 50-step trail with its anchors resolved;
//! - what recording adds to a search and to an answer;
//! - a snapshot, and the store's size per step;
//! - 1,000 notes as Markdown, and the whole store as JSON.
//!
//! Targets to report against (§13), never asserted: recording under 5 ms;
//! the list p95 under 100 ms at 100,000 steps; a 50-step trail in under
//! 200 ms; a snapshot under 2 s.
//!
//! ```text
//! cargo bench --bench knowledge -- [--trails 10000] [--steps 100000] [--label 100k] [--out results.json]
//! ```

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use atlas_lib::domain::dto::{
    AskEnding, AskQuery, ExportFormat, ExportScope, FileType, JobKind, LinkStyle, ProviderLocation,
    SearchIntent, SearchQuery, SemanticStatus, SourceUsed, TrailQuery,
};
use atlas_lib::infrastructure::local_source::LocalFsSource;
use atlas_lib::infrastructure::system_probe::{OsResourceMonitor, SimulatedMonitor, Simulation};
use atlas_lib::infrastructure::volume::OsVolumeInspector;
use atlas_lib::services::export::{self, Handles};
use atlas_lib::services::extraction::db as extraction_db;
use atlas_lib::services::extraction::job::{
    self as extraction_job, ExtractionConfig, ExtractionDeps,
};
use atlas_lib::services::extraction::supervisor::WorkerCommand;
use atlas_lib::services::inventory::{InventoryConfig, InventoryDeps};
use atlas_lib::services::knowledge::record::{AnswerOutcome, AskStart};
use atlas_lib::services::knowledge::snapshot;
use atlas_lib::services::search;
use atlas_lib::services::system::ResourceGovernor;
use atlas_lib::services::workspace::{Workspace, WorkspaceConfig};
use atlas_lib::services::{inventory, sources};

const RUNS: usize = 50;
const DEADLINE: Duration = Duration::from_secs(30 * 60);

struct Args {
    trails: u32,
    steps: u32,
    label: String,
    out: Option<PathBuf>,
}

fn args() -> Args {
    let mut a = Args {
        trails: 10_000,
        steps: 100_000,
        label: "100k".into(),
        out: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || it.next().expect("a value");
        match arg.as_str() {
            "--trails" => a.trails = value().parse().expect("a number"),
            "--steps" => a.steps = value().parse().expect("a number"),
            "--label" => a.label = value(),
            "--out" => a.out = Some(PathBuf::from(value())),
            _ => {}
        }
    }
    a
}

fn machine() -> Value {
    json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "logicalCores": std::thread::available_parallelism().map(|n| n.get()).ok(),
        "ci": std::env::var("CI").is_ok(),
    })
}

fn ms(d: Duration) -> f64 {
    (d.as_secs_f64() * 10_000.0).round() / 10.0
}

fn summary(mut times: Vec<f64>) -> Value {
    times.sort_by(f64::total_cmp);
    let at = |p: f64| times[((times.len() - 1) as f64 * p).round() as usize];
    json!({ "runs": times.len(), "p50Ms": at(0.5), "p95Ms": at(0.95), "maxMs": times[times.len() - 1] })
}

fn timed(runs: usize, mut f: impl FnMut(usize)) -> Value {
    summary(
        (0..runs)
            .map(|i| {
                let t = Instant::now();
                f(i);
                ms(t.elapsed())
            })
            .collect(),
    )
}

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

fn read_done(ws: &Workspace) -> bool {
    let root = ws.require_open().unwrap();
    let inv = inventory::control::overview(root).unwrap();
    let ov = root
        .reader()
        .read(|c| extraction_db::overview(c, None))
        .unwrap()
        .unwrap();
    inv.active.is_none() && ov.active.is_none() && ov.counts.total > 0 && ov.counts.pending == 0
}

/// This process's resident memory, sampled every 20 ms until stopped.
struct Sampler {
    stop: Arc<std::sync::atomic::AtomicBool>,
    before: u64,
    thread: std::thread::JoinHandle<u64>,
}

fn rss() -> u64 {
    let pid = sysinfo::get_current_pid().expect("a pid");
    let Ok(mut sys) = sysinfo::System::new() else {
        return 0;
    };
    sys.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&[pid]),
        true,
        sysinfo::ProcessRefreshKind::nothing().with_memory(),
    );
    sys.process(pid).map_or(0, sysinfo::Process::memory)
}

impl Sampler {
    fn start() -> Self {
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let before = rss();
        let flag = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            let mut peak = before;
            while !flag.load(std::sync::atomic::Ordering::SeqCst) {
                peak = peak.max(rss());
                std::thread::sleep(Duration::from_millis(20));
            }
            peak.max(rss())
        });
        Self {
            stop,
            before,
            thread,
        }
    }

    /// The memory before, and the peak.
    fn stop(self) -> (u64, u64) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        (self.before, self.thread.join().unwrap())
    }
}

fn node(script: &str, args: &[&std::ffi::OsStr]) -> String {
    let out = Command::new("node")
        .arg(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join(script),
        )
        .args(args)
        .output()
        .expect("node runs");
    assert!(
        out.status.success(),
        "{script}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn main() {
    let a = args();
    let tmp = tempfile::Builder::new()
        .prefix("atlas-bench-knowledge-")
        .tempdir()
        .unwrap();
    let books = tmp.path().join("Books");
    node(
        "scripts/generate-fixtures.mjs",
        &[
            books.as_os_str(),
            "--files".as_ref(),
            "60".as_ref(),
            "--text-kb".as_ref(),
            "4".as_ref(),
        ],
    );
    let mut ws = workspace(&tmp.path().join("config"));
    ws.choose(&tmp.path().join("AtlasData")).unwrap();
    let root = ws.require_open().unwrap();
    let added = sources::add(root, &books).unwrap();
    inventory::control::enqueue(root, &added.id).unwrap();
    let started = Instant::now();
    while !read_done(&ws) {
        assert!(started.elapsed() < DEADLINE, "reading did not finish");
        std::thread::sleep(Duration::from_millis(100));
    }
    let root = ws.require_open().unwrap();

    // The read documents, for the generator's anchors.
    let docs: Vec<Value> = root
        .reader()
        .read(|c| {
            let mut stmt = c.prepare(
                "SELECT d.id, lower(hex(d.content_hash)),
                        COALESCE(d.title, (SELECT display_name FROM file_instances f WHERE f.document_id = d.id LIMIT 1)),
                        (SELECT file_type FROM file_instances f WHERE f.document_id = d.id LIMIT 1),
                        (SELECT COALESCE(max(page_no), 1) FROM passages p WHERE p.document_id = d.id)
                 FROM documents d WHERE d.state = 'done'",
            )?;
            stmt.query_map([], |r| {
                let kind: String = r.get(3)?;
                Ok(json!({
                    "documentId": r.get::<_, String>(0)?,
                    "contentHash": r.get::<_, String>(1)?,
                    "title": r.get::<_, String>(2)?,
                    "fileType": if kind == "epub" { "epub" } else { "pdf" },
                    "pages": r.get::<_, i64>(4)?,
                }))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap()
        .unwrap();
    let docs_file = tmp.path().join("documents.json");
    fs::write(&docs_file, serde_json::to_vec(&docs).unwrap()).unwrap();
    let store_file = tmp.path().join("generated.json");
    let (trails, steps) = (a.trails.to_string(), a.steps.to_string());
    let generated: Value = serde_json::from_str(
        node(
            "scripts/generate-knowledge.mjs",
            &[
                store_file.as_os_str(),
                "--trails".as_ref(),
                trails.as_ref(),
                "--steps".as_ref(),
                steps.as_ref(),
                "--documents".as_ref(),
                docs_file.as_os_str(),
            ],
        )
        .trim(),
    )
    .unwrap();

    // The import: its dry run, then applying it (with its snapshot), with
    // this process's memory sampled throughout (Slice 3.9: the 1 GiB cap).
    let h = Handles::of(root).unwrap();
    let memory = Sampler::start();
    let t = Instant::now();
    let preview = export::preview(&h, &store_file).unwrap();
    let preview_ms = ms(t.elapsed());
    let t = Instant::now();
    export::apply(&h, &preview.token).unwrap();
    let apply_ms = ms(t.elapsed());
    let (rss_before, rss_peak) = memory.stop();

    let rec = root.recorder();
    let store = root.knowledge().shared();
    let one = |sql: &'static str| -> String {
        store
            .read(move |c| c.query_row(sql, [], |r| r.get::<_, String>(0)))
            .unwrap()
            .unwrap()
    };
    let count = |sql: &'static str| -> i64 {
        store
            .read(move |c| c.query_row(sql, [], |r| r.get::<_, i64>(0)))
            .unwrap()
            .unwrap()
    };
    let steps_here = count("SELECT count(*) FROM trail_steps");

    // The Trails list, newest first, and filtered.
    let list = timed(RUNS, |_| {
        rec.list(&TrailQuery {
            filter: String::new(),
            page: 0,
            page_size: 25,
        })
        .unwrap();
    });
    let filtered = timed(RUNS, |_| {
        rec.list(&TrailQuery {
            filter: "harbour".into(),
            page: 0,
            page_size: 25,
        })
        .unwrap();
    });

    // A trail of 50 steps, every anchor resolved.
    let fifty = one("SELECT id FROM trails WHERE step_count = 50 ORDER BY started_at LIMIT 1");
    let anchors = rec
        .get(&fifty)
        .unwrap()
        .steps
        .iter()
        .map(|s| s.anchors.len())
        .sum::<usize>();
    let open = timed(20, |_| {
        rec.get(&fifty).unwrap();
    });

    // What recording adds: a submitted search (its anchors built from the
    // catalog) and an answer, each a step of the current trail.
    let words = ["engine", "harbour", "orbital", "river", "keeper"];
    let queries: Vec<SearchQuery> = words
        .iter()
        .map(|w| SearchQuery {
            text: (*w).into(),
            source_root_id: None,
            file_type: None,
            include_missing: false,
            page: 0,
            page_size: 20,
        })
        .collect();
    let pages: Vec<_> = queries
        .iter()
        .map(|q| {
            root.reader()
                .read(|c| search::search(c, q))
                .unwrap()
                .unwrap()
        })
        .collect();
    let record_search = timed(RUNS, |i| {
        let k = i % queries.len();
        rec.search(&queries[k], SearchIntent::Submit, &pages[k]);
    });
    let sources: Vec<SourceUsed> = root
        .reader()
        .read(|c| {
            let mut stmt = c.prepare(
                "SELECT p.id, p.document_id, f.id, f.display_name, f.file_type, p.page_no,
                        substr(p.text, 1, 600)
                 FROM passages p JOIN file_instances f ON f.document_id = p.document_id
                 GROUP BY p.id ORDER BY p.id LIMIT 5",
            )?;
            stmt.query_map([], |r| {
                let kind: String = r.get(4)?;
                let page: Option<u32> = r.get(5)?;
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, kind, page, r.get(6)?))
            })?
            .collect::<rusqlite::Result<Vec<(i64, String, String, String, String, Option<u32>, String)>>>()
        })
        .unwrap()
        .unwrap()
        .into_iter()
        .enumerate()
        .map(|(i, (passage, doc, file, name, kind, page, excerpt))| SourceUsed {
            n: i as u32 + 1,
            document_id: doc,
            file_instance_id: file,
            title: name.clone(),
            display_name: name,
            file_type: if kind == "epub" {
                FileType::Epub
            } else {
                FileType::Pdf
            },
            location: page.map(|p| format!("Page {p}")),
            page_no: page,
            excerpt,
            passage: Some(passage),
        })
        .collect();
    let query = AskQuery {
        question: "how does the keeper log the tide".into(),
        source_root_id: None,
        file_type: None,
    };
    let record_answer = timed(RUNS, |i| {
        let id = format!("answer-{i}");
        let record = rec
            .ask_started(AskStart {
                answer_id: &id,
                query: &query,
                sources: &sources,
                weak: false,
                semantic: SemanticStatus::Off,
                model: "bench-chat",
                location: ProviderLocation::ThisComputer,
            })
            .expect("recording");
        rec.ask(
            record,
            AnswerOutcome {
                text: "The keeper logs the tide each night [1].".into(),
                ending: AskEnding::Done,
                cited: vec![1],
                dropped: 0,
            },
        );
    });

    // A snapshot of the whole store, and its size per step.
    let t = Instant::now();
    let outcome = snapshot::take(&store);
    let snapshot_ms = ms(t.elapsed());
    let bytes = count("SELECT page_count * page_size FROM pragma_page_count(), pragma_page_size()");

    // 1,000 notes as Markdown (the generator's first project), and the
    // whole store as one JSON file.
    let thousand = one("SELECT id FROM collections WHERE name = 'The thousand notes'");
    let md_dir = tmp.path().join("markdown");
    fs::create_dir_all(&md_dir).unwrap();
    let t = Instant::now();
    let md = export::export(
        &h,
        &ExportScope::Collection { id: thousand },
        ExportFormat::Markdown,
        LinkStyle::Obsidian,
        &md_dir,
    )
    .unwrap();
    let markdown_ms = ms(t.elapsed());
    let json_dir = tmp.path().join("json");
    fs::create_dir_all(&json_dir).unwrap();
    let t = Instant::now();
    export::export(
        &h,
        &ExportScope::Everything,
        ExportFormat::Json,
        LinkStyle::Obsidian,
        &json_dir,
    )
    .unwrap();
    let json_ms = ms(t.elapsed());
    let json_bytes = fs::read_dir(json_dir.join("Atlas"))
        .unwrap()
        .flatten()
        .map(|e| e.metadata().map(|m| m.len()).unwrap_or(0))
        .sum::<u64>();

    let result = json!({
        "bench": "knowledge",
        "label": a.label,
        "machine": machine(),
        "generated": generated,
        "documents": docs.len(),
        "steps": steps_here,
        "import": {
            "previewMs": preview_ms,
            "applyMs": apply_ms,
            "rssBeforeBytes": rss_before,
            "peakRssDeltaBytes": rss_peak.saturating_sub(rss_before),
        },
        "list": list,
        "listFiltered": filtered,
        "openFifty": { "anchors": anchors, "times": open },
        "recordSearch": record_search,
        "recordAnswer": record_answer,
        "snapshot": { "ms": snapshot_ms, "outcome": format!("{outcome:?}") },
        "storeBytes": bytes,
        "bytesPerStep": bytes / steps_here.max(1),
        "markdown": { "notes": md.written + md.replaced, "ms": markdown_ms },
        "json": { "bytes": json_bytes, "ms": json_ms },
    });
    println!("{}", serde_json::to_string_pretty(&result).unwrap());
    if let Some(out) = a.out {
        fs::write(out, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
    }
    drop(ws);
}
