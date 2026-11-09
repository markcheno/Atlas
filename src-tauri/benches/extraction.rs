//! Phase 1 extraction benchmark (phase1-spec §13; docs/benchmarks.md).
//!
//! Catalogs a text fixture (`scripts/generate-fixtures.mjs --text-kb N`)
//! unthrottled, then reads it through the real job, supervisor and worker
//! at a chosen duty cycle: the Run duty cycle on AC, the Slow one on a
//! simulated battery, or unthrottled. Records files, pages and passages per
//! second, CPU and peak memory of the app and of the worker, worker
//! restarts, the largest write-ahead log, the time to the first searchable
//! document and the final index merge, and how the catalog grows against
//! the text (`dbstat`). `--recovery` kills a child process mid-read and
//! reports what the kill left: partial documents, duplicates, and worker
//! processes that outlived it (P1-12, P1-22). Never asserts a target;
//! shared CI runners are noisy.
//!
//! ```text
//! cargo bench --bench extraction -- --fixture <dir> [--label 1k-text]
//!     [--duty run,slow,unthrottled] [--recovery] [--expect <files>]
//!     [--out results.json]
//! ```

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System};

use atlas_lib::domain::dto::JobKind;
use atlas_lib::infrastructure::local_source::LocalFsSource;
use atlas_lib::infrastructure::system_probe::{
    OsResourceMonitor, SimulatedMonitor, SimulatedPower, Simulation,
};
use atlas_lib::infrastructure::volume::OsVolumeInspector;
use atlas_lib::services::extraction::db as extraction_db;
use atlas_lib::services::extraction::job::{
    self as extraction_job, ExtractionConfig, ExtractionDeps,
};
use atlas_lib::services::extraction::supervisor::{WorkerCommand, WorkerStats};
use atlas_lib::services::inventory::{InventoryConfig, InventoryDeps};
use atlas_lib::services::system::ResourceGovernor;
use atlas_lib::services::workspace::{Workspace, WorkspaceConfig, layout};
use atlas_lib::services::{inventory, sources, storage};

const DEADLINE: Duration = Duration::from_secs(2 * 60 * 60);
const SAMPLE: Duration = Duration::from_millis(100);
/// How often the harness reads the library's status once text exists.
const STATUS_POLL: Duration = Duration::from_millis(500);
/// A worker must be gone this soon after its app is killed (P1-22).
const WORKER_GONE: Duration = Duration::from_secs(5);

fn fail(msg: &str) -> ! {
    eprintln!("bench-extraction: {msg}");
    std::process::exit(2)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Duty {
    /// The Run duty cycle (AC power).
    Run,
    /// The Slow duty cycle (a simulated battery at 50%).
    Slow,
    Unthrottled,
}

impl Duty {
    fn parse(s: &str) -> Duty {
        match s {
            "run" => Duty::Run,
            "slow" => Duty::Slow,
            "unthrottled" => Duty::Unthrottled,
            other => fail(&format!("bad duty {other} (run, slow, unthrottled)")),
        }
    }
    fn label(self) -> &'static str {
        match self {
            Duty::Run => "run",
            Duty::Slow => "slow",
            Duty::Unthrottled => "unthrottled",
        }
    }
    fn power(self) -> SimulatedPower {
        match self {
            Duty::Slow => SimulatedPower::Battery(50.0),
            _ => SimulatedPower::Ac,
        }
    }
}

struct Args {
    fixture: PathBuf,
    label: String,
    duties: Vec<Duty>,
    recovery: bool,
    expect: Option<u64>,
    out: Option<PathBuf>,
    child_config: Option<PathBuf>,
}

fn parse_args() -> Args {
    let mut fixture = None;
    let mut a = Args {
        fixture: PathBuf::new(),
        label: "fixture".into(),
        duties: vec![Duty::Run],
        recovery: false,
        expect: None,
        out: None,
        child_config: None,
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
            "--duty" => a.duties = value().split(',').map(Duty::parse).collect(),
            "--recovery" => a.recovery = true,
            "--expect" => a.expect = Some(value().parse().unwrap_or_else(|_| fail("--expect"))),
            "--out" => a.out = Some(PathBuf::from(value())),
            "--child-read" => a.child_config = Some(PathBuf::from(value())),
            other => fail(&format!("unknown argument {other}")),
        }
    }
    let fixture = fixture.unwrap_or_else(|| fail("--fixture <dir> is required"));
    a.fixture = fs::canonicalize(fixture).unwrap_or_else(|_| fail("fixture not found"));
    a
}

fn main() {
    let args = parse_args();
    if let Some(config) = &args.child_config {
        child_read(&args, config);
    }
    let runs: Vec<Value> = args.duties.iter().map(|d| read_run(&args, *d)).collect();
    let mut report = json!({
        "label": args.label,
        "fixture": args.fixture.file_name().map(|n| n.to_string_lossy().into_owned()),
        "machine": machine(),
        "runs": runs,
    });
    if args.recovery {
        report["recovery"] = recovery_run(&args);
    }
    let text = serde_json::to_string_pretty(&report).unwrap();
    match &args.out {
        Some(out) => fs::write(out, &text).unwrap(),
        None => println!("{text}"),
    }
}

// ----------------------------------------------------------------- setup

fn workspace(config: &Path, duty: Duty) -> (Workspace, Arc<WorkerStats>) {
    let volumes = Arc::new(OsVolumeInspector);
    let monitor = SimulatedMonitor {
        inner: OsResourceMonitor::new(volumes.clone()),
        simulation: Simulation {
            power: Some(duty.power()),
            free_bytes: None,
        },
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
            unthrottled: duty == Duty::Unthrottled,
            ..ExtractionConfig::default()
        },
        WorkerCommand::new(env!("CARGO_BIN_EXE_atlas")),
    );
    let stats = Arc::clone(&extraction.stats);
    let ws = Workspace::new(
        WorkspaceConfig {
            config_dir: config.to_path_buf(),
            default_root: config.join("Library"),
        },
        volumes,
    )
    .with_inventory(inventory)
    .with_job_processor(JobKind::Extract, extraction_job::factory(extraction));
    (ws, stats)
}

struct Scratch {
    _tmp: tempfile::TempDir,
    config: PathBuf,
    data: PathBuf,
}

fn scratch() -> Scratch {
    let tmp = tempfile::Builder::new()
        .prefix("atlas-bench-extraction-")
        .tempdir()
        .unwrap();
    Scratch {
        config: tmp.path().join("config"),
        data: tmp.path().join("AtlasData"),
        _tmp: tmp,
    }
}

fn one(ws: &Workspace, sql: &'static str) -> i64 {
    ws.require_open()
        .unwrap()
        .reader()
        .read(|c| c.query_row(sql, [], |r| r.get(0)))
        .unwrap()
        .unwrap_or(-1)
}

/// (cataloging finished, files still waiting for text, a reading job left,
/// what the reading job is doing: "running", "slow:<reason>", "paused:<reason>")
fn progress(ws: &Workspace) -> (bool, u32, bool, Option<String>) {
    let root = ws.require_open().unwrap();
    let inv = inventory::control::overview(root).unwrap();
    let ov = root
        .reader()
        .read(|c| extraction_db::overview(c, None))
        .unwrap()
        .unwrap();
    let doing = ov.active.as_ref().map(|a| {
        if let Some(p) = a.pause_reason {
            format!("paused:{p:?}")
        } else if let Some(t) = a.throttle_reason {
            format!("slow:{t:?}")
        } else {
            format!("{:?}", a.state)
        }
    });
    (
        inv.active.is_none() && ov.counts.total > 0,
        ov.counts.pending,
        ov.active.is_some() || ov.queued_jobs > 0,
        doing,
    )
}

fn file_bytes(path: &Path) -> u64 {
    fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

// ------------------------------------------------------------ resources

/// One process's CPU time and peak memory over a sampling window.
#[derive(Default, Clone, Copy)]
struct Usage {
    cpu_ms: u64,
    peak_rss: u64,
}

/// Samples the app (this process) and every extraction worker it starts,
/// every 100 ms. A restarted worker is a new process; their CPU adds up.
struct Sampler {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<(Usage, Usage)>>,
}

fn children_of(sys: &mut System, parent: Pid) -> Vec<Pid> {
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    sys.processes()
        .iter()
        // On Linux sysinfo lists threads too, with their process as parent.
        .filter(|(_, p)| {
            p.parent() == Some(parent)
                && p.thread_kind().is_none()
                && p.status() != ProcessStatus::Zombie
        })
        .map(|(pid, _)| *pid)
        .collect()
}

fn cpu_and_rss(sys: &mut System, pid: Pid) -> Option<(u64, u64)> {
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing().with_cpu().with_memory(),
    );
    sys.process(pid)
        .map(|p| (p.accumulated_cpu_time(), p.memory()))
}

impl Sampler {
    fn start() -> Self {
        let me = sysinfo::get_current_pid().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let handle = std::thread::spawn(move || {
            let mut sys = System::new().unwrap_or_else(|e| fail(&format!("sysinfo: {e}")));
            let app_start = cpu_and_rss(&mut sys, me).map_or(0, |u| u.0);
            let mut app = Usage::default();
            let mut worker_peak = 0u64;
            // Last CPU time seen per worker process.
            let mut workers: HashMap<Pid, u64> = HashMap::new();
            let mut known: Vec<Pid> = Vec::new();
            let mut tick = 0u32;
            loop {
                let last = flag.load(Ordering::Relaxed);
                if let Some((cpu, rss)) = cpu_and_rss(&mut sys, me) {
                    app.cpu_ms = cpu.saturating_sub(app_start);
                    app.peak_rss = app.peak_rss.max(rss);
                }
                // Look for new workers each second, or when one is gone.
                let alive: Vec<Pid> = known
                    .iter()
                    .copied()
                    .filter(|pid| {
                        if let Some((cpu, rss)) = cpu_and_rss(&mut sys, *pid) {
                            workers.insert(*pid, cpu);
                            worker_peak = worker_peak.max(rss);
                            true
                        } else {
                            false
                        }
                    })
                    .collect();
                if alive.len() != known.len() || known.is_empty() || tick.is_multiple_of(10) {
                    known = children_of(&mut sys, me);
                } else {
                    known = alive;
                }
                tick += 1;
                if last {
                    break;
                }
                std::thread::sleep(SAMPLE);
            }
            (
                app,
                Usage {
                    cpu_ms: workers.values().sum(),
                    peak_rss: worker_peak,
                },
            )
        });
        Self {
            stop,
            handle: Some(handle),
        }
    }

    /// Returns (app, workers).
    fn finish(mut self) -> (Usage, Usage) {
        self.stop.store(true, Ordering::Relaxed);
        self.handle.take().unwrap().join().unwrap()
    }
}

fn running(pid: Pid) -> bool {
    let mut sys = System::new().unwrap_or_else(|e| fail(&format!("sysinfo: {e}")));
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing(),
    );
    sys.process(pid)
        .is_some_and(|p| p.status() != ProcessStatus::Zombie)
}

// ------------------------------------------------------------------ runs

fn read_run(args: &Args, duty: Duty) -> Value {
    eprintln!("bench-extraction: {} at {}", args.label, duty.label());
    let s = scratch();
    let (mut ws, stats) = workspace(&s.config, duty);
    ws.choose(&s.data).unwrap();
    let wal = PathBuf::from(format!("{}-wal", s.data.join(layout::CATALOG).display()));
    let root = ws.require_open().unwrap();
    let added = sources::add(root, &args.fixture).unwrap();
    inventory::control::enqueue(root, &added.id).unwrap();

    // Reading starts when cataloging ends; time it from there.
    let begun = Instant::now();
    while !progress(&ws).0 {
        if begun.elapsed() > DEADLINE {
            fail("cataloging did not finish");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let catalog_secs = begun.elapsed().as_secs_f64();
    let sampler = Sampler::start();
    let start = Instant::now();
    let mut first_searchable = None;
    let mut all_read = None;
    let mut peak_wal = 0u64;
    // How often each policy state was seen while reading.
    let mut seen: HashMap<String, u32> = HashMap::new();
    loop {
        if start.elapsed() > DEADLINE {
            fail("reading did not finish within two hours");
        }
        let (_, pending, active, doing) = progress(&ws);
        if let Some(doing) = doing {
            *seen.entry(doing).or_default() += 1;
        }
        peak_wal = peak_wal.max(file_bytes(&wal));
        if first_searchable.is_none() && one(&ws, "SELECT EXISTS (SELECT 1 FROM passages)") == 1 {
            first_searchable = Some(start.elapsed());
        }
        if pending == 0 {
            all_read.get_or_insert(start.elapsed());
            if !active {
                break;
            }
        }
        // Fine-grained until the first document is searchable, then
        // sparse: at 30k files each status read costs milliseconds of the
        // app's CPU, which would otherwise be counted as reading.
        std::thread::sleep(if first_searchable.is_some() {
            STATUS_POLL
        } else {
            SAMPLE
        });
    }
    let read = start.elapsed();
    let (app, worker) = sampler.finish();
    let all_read = all_read.unwrap_or(read);

    let files = one(&ws, "SELECT COUNT(*) FROM file_instances");
    if let Some(expect) = args.expect
        && files as u64 != expect
    {
        fail(&format!("cataloged {files} files, expected {expect}"));
    }
    let documents = one(&ws, "SELECT COUNT(*) FROM documents WHERE state = 'done'");
    let pdf_pages = one(
        &ws,
        "SELECT COUNT(*) FROM passages WHERE page_no IS NOT NULL",
    );
    let passages = one(&ws, "SELECT COUNT(*) FROM passages");
    let text_chars = one(&ws, "SELECT COALESCE(SUM(length(text)), 0) FROM passages");
    let text_stored = one(
        &ws,
        "SELECT COALESCE(SUM(length(CAST(text AS BLOB))), 0) FROM passages",
    );
    let page_size = one(&ws, "PRAGMA page_size");

    // Growth, measured as the catalog stands after a checkpoint.
    ws.require_open()
        .unwrap()
        .catalog()
        .call(|c| c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())))
        .unwrap()
        .unwrap();
    let usage = storage::usage(&s.data, &OsVolumeInspector).unwrap();
    ws.shutdown();

    let secs = read.as_secs_f64();
    let per_sec = |n: i64| round(n as f64 / secs.max(1e-9), 1);
    let pct = |ms: u64| round(ms as f64 / (secs * 1000.0).max(1e-9) * 100.0, 1);
    let mib = |b: u64| round(b as f64 / (1024.0 * 1024.0), 1);
    json!({
        "duty": duty.label(),
        "files": files,
        "documents": documents,
        "pdfPages": pdf_pages,
        "passages": passages,
        "textChars": text_chars,
        "catalogSeconds": round(catalog_secs, 2),
        "readSeconds": round(secs, 2),
        "firstSearchableSeconds": first_searchable.map(|d| round(d.as_secs_f64(), 2)),
        "finalMergeSeconds": round((read - all_read).as_secs_f64(), 2),
        "filesPerSecond": per_sec(files),
        "pagesPerSecond": per_sec(pdf_pages),
        "passagesPerSecond": per_sec(passages),
        "appCpuPercentOfOneCore": pct(app.cpu_ms),
        "workerCpuPercentOfOneCore": pct(worker.cpu_ms),
        "appPeakRssMiB": mib(app.peak_rss),
        "workerPeakRssMiB": mib(worker.peak_rss),
        "workerSpawns": WorkerStats::get(&stats.spawns),
        "workerKills": WorkerStats::get(&stats.kills),
        "peakWalMiB": mib(peak_wal),
        "policySeen": seen,
        "growth": {
            "pageSize": page_size,
            "textStoredBytes": text_stored,
            "catalogBytes": usage.catalog_bytes,
            "textBytes": usage.text_bytes,
            "indexBytes": usage.index_bytes,
            "otherBytes": usage.other_catalog_bytes,
            "catalogPerText": round(usage.catalog_bytes as f64 / (text_stored.max(1) as f64), 2),
            "textPagesPerText": round(usage.text_bytes as f64 / (text_stored.max(1) as f64), 2),
            "indexPerText": round(usage.index_bytes as f64 / (text_stored.max(1) as f64), 2),
        },
    })
}

/// Kills a child process mid-read, then reopens and finishes (P1-12,
/// P1-22).
fn recovery_run(args: &Args) -> Value {
    eprintln!("bench-extraction: {} recovery", args.label);
    let s = scratch();
    {
        let (mut ws, _) = workspace(&s.config, Duty::Run);
        ws.choose(&s.data).unwrap();
        ws.shutdown();
    }
    let mut child: Child = Command::new(std::env::current_exe().unwrap())
        .args(["--fixture".as_ref(), args.fixture.as_os_str()])
        .args(["--child-read".as_ref(), s.config.as_os_str()])
        .spawn()
        .unwrap();
    let db = s.data.join(layout::CATALOG);
    let target = (args.expect.unwrap_or(1000) / 5).max(10) as i64;
    let started = Instant::now();
    while count_read_only(&db, "SELECT COUNT(*) FROM documents").unwrap_or(0) < target {
        if started.elapsed() > DEADLINE {
            let _ = child.kill();
            fail("the child never read enough to kill it");
        }
        std::thread::sleep(SAMPLE);
    }
    let mut sys = System::new().unwrap_or_else(|e| fail(&format!("sysinfo: {e}")));
    let workers = children_of(&mut sys, Pid::from_u32(child.id()));
    child.kill().unwrap();
    child.wait().unwrap();
    let killed = Instant::now();
    while workers.iter().any(|p| running(*p)) && killed.elapsed() < WORKER_GONE {
        std::thread::sleep(Duration::from_millis(50));
    }
    let workers_gone_ms = killed.elapsed().as_secs_f64() * 1000.0;
    let survivors: Vec<Pid> = workers.iter().copied().filter(|p| running(*p)).collect();
    for pid in &survivors {
        sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[*pid]),
            true,
            ProcessRefreshKind::nothing(),
        );
        if let Some(p) = sys.process(*pid) {
            p.kill();
        }
    }
    let at_kill = count_read_only(&db, "SELECT COUNT(*) FROM documents").unwrap_or(0);
    let partial = count_read_only(
        &db,
        "SELECT COUNT(*) FROM documents d
         JOIN file_instances f ON f.document_id = d.id
         WHERE d.state = 'done' AND f.file_type = 'pdf'
           AND d.page_count IS NOT (SELECT COUNT(*) FROM passages p WHERE p.document_id = d.id)",
    )
    .unwrap_or(-1);

    let t = Instant::now();
    let (mut ws, _) = workspace(&s.config, Duty::Unthrottled);
    ws.startup();
    let reopen_ms = t.elapsed().as_secs_f64() * 1000.0;
    loop {
        if t.elapsed() > DEADLINE {
            fail("reading did not finish after the kill");
        }
        let (catalogued, pending, active, _) = progress(&ws);
        if catalogued && pending == 0 && !active {
            break;
        }
        std::thread::sleep(SAMPLE);
    }
    let finish = t.elapsed().as_secs_f64();
    let duplicates = one(
        &ws,
        "SELECT COUNT(*) FROM (SELECT content_hash FROM documents GROUP BY content_hash HAVING COUNT(*) > 1)",
    );
    let pending = one(
        &ws,
        "SELECT COUNT(*) FROM file_instances WHERE extract_state = 'pending'",
    );
    ws.shutdown();
    json!({
        "documentsAtKill": at_kill,
        "workersAtKill": workers.len(),
        "workersLeftAfterKill": survivors.len(),
        "workersGoneMs": round(workers_gone_ms, 0),
        "partialDocumentsAtKill": partial,
        "reopenMs": round(reopen_ms, 1),
        "resumeToCompleteSeconds": round(finish, 2),
        "duplicateDocuments": duplicates,
        "pendingAfterResume": pending,
    })
}

/// Child mode: open the library, catalog the fixture, read at the Run duty
/// cycle until killed.
fn child_read(args: &Args, config: &Path) -> ! {
    let (mut ws, _) = workspace(config, Duty::Run);
    ws.startup();
    let root = ws.require_open().unwrap();
    let added = sources::add(root, &args.fixture).unwrap();
    inventory::control::enqueue(root, &added.id).unwrap();
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn count_read_only(db: &Path, sql: &str) -> Option<i64> {
    let c = rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .ok()?;
    c.busy_timeout(Duration::from_secs(1)).ok()?;
    c.query_row(sql, [], |r| r.get(0)).ok()
}

fn machine() -> Value {
    let mut sys = System::new().unwrap_or_else(|e| fail(&format!("sysinfo: {e}")));
    sys.refresh_memory();
    sys.refresh_cpu_all();
    json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "osVersion": System::long_os_version().ok(),
        "cpu": sys.cpus().first().map(|c| c.brand().trim().to_string()).filter(|b| !b.is_empty()),
        "logicalCores": std::thread::available_parallelism().map(|n| n.get()).ok(),
        "totalMemoryBytes": sys.total_memory(),
        "ci": std::env::var("CI").is_ok(),
    })
}

fn round(v: f64, places: i32) -> f64 {
    let f = 10f64.powi(places);
    (v * f).round() / f
}
