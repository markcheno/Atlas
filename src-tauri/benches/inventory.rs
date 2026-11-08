//! Phase 0 benchmark harness (phase0-spec §13; docs/benchmarks.md).
//!
//! Catalogs a fixture made by `scripts/generate-fixtures.mjs` through the
//! real services: the local filesystem source, SQLite catalog, OS volume
//! inspector and OS resource monitor (power can be simulated so AC and
//! battery runs are repeatable). Records measurements; never asserts a
//! performance target, because shared CI runners are noisy.
//!
//! ```text
//! cargo bench --bench inventory -- --fixture <dir> [--label 1k]
//!     [--rates policy,100,250,unthrottled] [--power ac|battery:50]
//!     [--browse <queries per second>] [--pause] [--recovery]
//!     [--expect <files>] [--out results.json]
//! ```

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

use atlas_lib::domain::dto::{DocumentQuery, DocumentSort, FileType, InventoryOverview, JobState};
use atlas_lib::infrastructure::local_source::LocalFsSource;
use atlas_lib::infrastructure::system_probe::{
    OsResourceMonitor, SimulatedMonitor, SimulatedPower, Simulation,
};
use atlas_lib::infrastructure::volume::OsVolumeInspector;
use atlas_lib::services::inventory::{InventoryConfig, InventoryDeps, control};
use atlas_lib::services::library;
use atlas_lib::services::scheduler::policy::{AC_OPS_PER_SECOND, SLOW_OPS_PER_SECOND};
use atlas_lib::services::sources;
use atlas_lib::services::system::ResourceGovernor;
use atlas_lib::services::workspace::{Workspace, WorkspaceConfig, layout};

const DEADLINE: Duration = Duration::from_secs(60 * 60);

#[derive(Clone, Copy, Debug)]
enum Rate {
    /// Whatever the resource policy allows for the simulated power state.
    Policy,
    Fixed(u32),
    Unthrottled,
}

impl Rate {
    fn parse(s: &str) -> Rate {
        match s {
            "policy" => Rate::Policy,
            "unthrottled" => Rate::Unthrottled,
            n => Rate::Fixed(n.parse().unwrap_or_else(|_| fail(&format!("bad rate {n}")))),
        }
    }
    fn label(self) -> String {
        match self {
            Rate::Policy => "policy".into(),
            Rate::Fixed(n) => n.to_string(),
            Rate::Unthrottled => "unthrottled".into(),
        }
    }
}

struct Args {
    fixture: PathBuf,
    label: String,
    rates: Vec<Rate>,
    power: SimulatedPower,
    power_label: String,
    browse_per_sec: u32,
    pause: bool,
    recovery: bool,
    expect: Option<u64>,
    out: Option<PathBuf>,
    child_config: Option<PathBuf>,
}

fn fail(msg: &str) -> ! {
    eprintln!("bench: {msg}");
    std::process::exit(2)
}

fn parse_args() -> Args {
    let mut a = Args {
        fixture: PathBuf::new(),
        label: "fixture".into(),
        rates: vec![Rate::Policy],
        power: SimulatedPower::Ac,
        power_label: "ac".into(),
        browse_per_sec: 0,
        pause: false,
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
            // `cargo bench` passes this to every bench binary.
            "--bench" => {}
            "--fixture" => a.fixture = PathBuf::from(value()),
            "--label" => a.label = value(),
            "--rates" => a.rates = value().split(',').map(Rate::parse).collect(),
            "--power" => {
                a.power_label = value();
                a.power = Simulation::parse_power(&a.power_label)
                    .unwrap_or_else(|| fail("--power is ac, battery:<pct>, unknown or desktop"));
            }
            "--browse" => a.browse_per_sec = value().parse().unwrap_or_else(|_| fail("--browse")),
            "--pause" => a.pause = true,
            "--recovery" => a.recovery = true,
            "--expect" => a.expect = Some(value().parse().unwrap_or_else(|_| fail("--expect"))),
            "--out" => a.out = Some(PathBuf::from(value())),
            "--child-scan" => a.child_config = Some(PathBuf::from(value())),
            other => fail(&format!("unknown argument {other}")),
        }
    }
    if a.fixture.as_os_str().is_empty() {
        fail("--fixture <dir> is required (see scripts/generate-fixtures.mjs)");
    }
    a.fixture = fs::canonicalize(&a.fixture).unwrap_or_else(|_| fail("fixture not found"));
    a
}

fn main() {
    let args = parse_args();
    if let Some(config) = &args.child_config {
        child_scan(&args, config);
    }

    let mut runs = Vec::new();
    for &rate in &args.rates {
        eprintln!(
            "bench: {} at rate {} on {}",
            args.label,
            rate.label(),
            args.power_label
        );
        runs.push(scan_run(&args, rate));
    }
    let recovery = args.recovery.then(|| recovery_run(&args));

    let report = json!({
        "fixture": args.label,
        "fixtureEntries": count_entries(&args.fixture),
        "power": args.power_label,
        "policyRates": { "ac": AC_OPS_PER_SECOND, "slow": SLOW_OPS_PER_SECOND },
        "machine": machine(),
        "runs": runs,
        "recovery": recovery,
    });
    let text = serde_json::to_string_pretty(&report).unwrap();
    println!("{text}");
    if let Some(out) = &args.out {
        fs::write(out, &text).unwrap_or_else(|e| fail(&format!("write {}: {e}", out.display())));
    }
}

// ----------------------------------------------------------------- setup

struct Scratch {
    _tmp: tempfile::TempDir,
    config: PathBuf,
    data: PathBuf,
}

fn scratch() -> Scratch {
    let tmp = tempfile::Builder::new()
        .prefix("atlas-bench-")
        .tempdir()
        .unwrap();
    let config = tmp.path().join("config");
    let data = tmp.path().join("AtlasData");
    Scratch {
        config,
        data,
        _tmp: tmp,
    }
}

/// What the runner reports through its progress events, which is how the
/// harness follows a scan without adding queries of its own.
#[derive(Default)]
struct Progress {
    events: AtomicU64,
    last: Mutex<Option<InventoryOverview>>,
}

fn workspace(
    config: &Path,
    power: SimulatedPower,
    rate: Rate,
    progress: Arc<Progress>,
) -> Workspace {
    let volumes = Arc::new(OsVolumeInspector);
    let monitor = SimulatedMonitor {
        inner: OsResourceMonitor::new(volumes.clone()),
        simulation: Simulation {
            power: Some(power),
            free_bytes: None,
        },
    };
    let deps = InventoryDeps {
        governor: Arc::new(Mutex::new(ResourceGovernor::new(Box::new(monitor)))),
        source: Arc::new(LocalFsSource),
        events: Arc::new(move |o: &InventoryOverview| {
            progress.events.fetch_add(1, Ordering::Relaxed);
            *progress.last.lock().unwrap() = Some(o.clone());
        }),
        config: InventoryConfig {
            unthrottled: matches!(rate, Rate::Unthrottled),
            rate_override: match rate {
                Rate::Fixed(n) => Some(n),
                _ => None,
            },
            ..InventoryConfig::default()
        },
    };
    Workspace::new(
        WorkspaceConfig {
            config_dir: config.to_path_buf(),
            default_root: config.join("Library"),
        },
        volumes,
    )
    .with_inventory(deps)
}

fn query() -> DocumentQuery {
    DocumentQuery {
        text: None,
        source_root_id: None,
        file_type: None,
        sort: DocumentSort::NameAsc,
        page: 0,
        page_size: 200,
        include_missing: false,
    }
}

/// The browse mix: first page, search, a deep sorted page, a filtered
/// search, a search with no matches, and the indexing status.
fn browse_mix(root_id: &str) -> Vec<Option<DocumentQuery>> {
    let search = |t: &str| DocumentQuery {
        text: Some(t.into()),
        ..query()
    };
    vec![
        Some(query()),
        Some(search("volume 12")),
        Some(DocumentQuery {
            sort: DocumentSort::ModifiedDesc,
            page: 20,
            page_size: 50,
            ..query()
        }),
        Some(DocumentQuery {
            source_root_id: Some(root_id.into()),
            file_type: Some(FileType::Pdf),
            ..search("book 1")
        }),
        Some(search("no such file anywhere")),
        None, // get_inventory_status
    ]
}

fn run_query(ws: &Workspace, q: &Option<DocumentQuery>) {
    let root = ws.require_open().unwrap();
    match q {
        Some(q) => {
            library::list(root.reader(), root.root_status(), q).unwrap();
        }
        None => {
            control::overview(root).unwrap();
        }
    }
}

fn file_count(ws: &Workspace) -> u64 {
    let root = ws.require_open().unwrap();
    let q = DocumentQuery {
        page_size: 1,
        ..query()
    };
    library::list(root.reader(), root.root_status(), &q)
        .unwrap()
        .matched_so_far as u64
}

fn idle(ws: &Workspace) -> bool {
    control::overview(ws.require_open().unwrap())
        .is_ok_and(|o| o.active.is_none() && o.queued_sessions == 0)
}

fn wait(mut f: impl FnMut() -> bool) {
    let start = Instant::now();
    while !f() {
        if start.elapsed() > DEADLINE {
            fail("timed out");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

// ------------------------------------------------------------ resources

/// Samples this process's CPU and resident memory every 100 ms.
struct Sampler {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<(u64, f32)>>,
    cpu_start_ms: u64,
}

fn process_now(sys: &mut System, pid: Pid) -> (u64, u64, f32) {
    sys.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        false,
        ProcessRefreshKind::nothing().with_cpu().with_memory(),
    );
    let p = sys.process(pid).expect("own process");
    (p.accumulated_cpu_time(), p.memory(), p.cpu_usage())
}

impl Sampler {
    fn start() -> Self {
        let pid = sysinfo::get_current_pid().unwrap();
        let mut sys = new_system();
        let (cpu_start_ms, _, _) = process_now(&mut sys, pid);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let handle = std::thread::spawn(move || {
            let (mut peak_rss, mut peak_cpu) = (0u64, 0f32);
            while !flag.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(100));
                let (_, rss, cpu) = process_now(&mut sys, pid);
                peak_rss = peak_rss.max(rss);
                peak_cpu = peak_cpu.max(cpu);
            }
            (peak_rss, peak_cpu)
        });
        Self {
            stop,
            handle: Some(handle),
            cpu_start_ms,
        }
    }

    /// Returns (CPU milliseconds used, sampled peak RSS, sampled peak CPU %).
    fn finish(mut self) -> (u64, u64, f32) {
        self.stop.store(true, Ordering::Relaxed);
        let (peak_rss, peak_cpu) = self.handle.take().unwrap().join().unwrap();
        let pid = sysinfo::get_current_pid().unwrap();
        let (cpu_end_ms, _, _) = process_now(&mut new_system(), pid);
        (
            cpu_end_ms.saturating_sub(self.cpu_start_ms),
            peak_rss,
            peak_cpu,
        )
    }
}

fn dir_bytes(path: &Path) -> u64 {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_bytes(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

fn file_bytes(path: &Path) -> u64 {
    fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn catalog_sizes(data: &Path) -> Value {
    let db = data.join(layout::CATALOG);
    let wal = PathBuf::from(format!("{}-wal", db.display()));
    json!({ "dbBytes": file_bytes(&db), "walBytes": file_bytes(&wal), "dataRootBytes": dir_bytes(data) })
}

fn count_entries(path: &Path) -> u64 {
    fs::read_dir(path)
        .map(|it| {
            it.flatten()
                .map(|e| {
                    1 + match e.file_type() {
                        Ok(t) if t.is_dir() => count_entries(&e.path()),
                        _ => 0,
                    }
                })
                .sum()
        })
        .unwrap_or(0)
}

fn percentiles(mut times: Vec<Duration>) -> Value {
    if times.is_empty() {
        return Value::Null;
    }
    times.sort();
    let ms = |d: Duration| (d.as_secs_f64() * 1000.0 * 100.0).round() / 100.0;
    let at = |p: usize| times[(times.len() * p / 100).min(times.len() - 1)];
    json!({
        "count": times.len(),
        "p50Ms": ms(at(50)),
        "p95Ms": ms(at(95)),
        "maxMs": ms(*times.last().unwrap()),
    })
}

fn new_system() -> System {
    System::new().unwrap_or_else(|e| fail(&format!("sysinfo: {e}")))
}

fn machine() -> Value {
    let mut sys = new_system();
    sys.refresh_memory();
    json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "osVersion": System::long_os_version().ok(),
        "cpu": cpu_brand(),
        "logicalCores": std::thread::available_parallelism().map(|n| n.get()).ok(),
        "totalMemoryBytes": sys.total_memory(),
        "ci": std::env::var("CI").is_ok(),
    })
}

fn cpu_brand() -> Option<String> {
    let mut sys = new_system();
    sys.refresh_cpu_all();
    sys.cpus()
        .first()
        .map(|c| c.brand().trim().to_string())
        .filter(|b| !b.is_empty())
}

// ------------------------------------------------------------------ runs

fn scan_run(args: &Args, rate: Rate) -> Value {
    let s = scratch();
    let progress = Arc::new(Progress::default());
    let ws = Arc::new(Mutex::new(workspace(
        &s.config,
        args.power,
        rate,
        progress.clone(),
    )));
    let open_started = Instant::now();
    ws.lock().unwrap().choose(&s.data).unwrap();
    let create_ms = open_started.elapsed().as_millis();
    let empty = catalog_sizes(&s.data);

    // What the harness itself costs (mostly the CPU sampler), subtracted
    // from the scan's CPU below.
    let baseline = Sampler::start();
    let t = Instant::now();
    std::thread::sleep(Duration::from_secs(2));
    let baseline_per_sec = baseline.finish().0 as f64 / t.elapsed().as_secs_f64();

    let sampler = Sampler::start();
    let started = Instant::now();
    let root_id = {
        let w = ws.lock().unwrap();
        let root = w.require_open().unwrap();
        let added = sources::add(root, &args.fixture).unwrap();
        control::enqueue(root, &added.id).unwrap();
        added.id
    };

    // Concurrent browsing: one query every 1/n seconds while scanning.
    let scanning = Arc::new(AtomicBool::new(true));
    let browse = (args.browse_per_sec > 0).then(|| {
        let (ws, scanning, mix) = (ws.clone(), scanning.clone(), browse_mix(&root_id));
        let gap = Duration::from_secs_f64(1.0 / args.browse_per_sec as f64);
        std::thread::spawn(move || {
            let mut times = Vec::new();
            let mut i = 0;
            while scanning.load(Ordering::Relaxed) {
                let started = Instant::now();
                {
                    // Time the command itself, not waiting for the lock
                    // this harness shares with its own polling loop.
                    let w = ws.lock().unwrap();
                    let t = Instant::now();
                    run_query(&w, &mix[i % mix.len()]);
                    times.push(t.elapsed());
                }
                let t = started;
                i += 1;
                std::thread::sleep(gap.saturating_sub(t.elapsed()));
            }
            times
        })
    });

    // Poll for the first visible rows, then follow the runner's progress
    // events: the harness adds no queries while the scan runs, except a
    // status check every 2 s in case the final event is missed.
    let mut first_rows = None;
    let mut pause = None;
    let mut last_check = Instant::now();
    loop {
        if first_rows.is_none() {
            if file_count(&ws.lock().unwrap()) > 0 {
                first_rows = Some(started.elapsed());
            }
        } else {
            let last = progress.last.lock().unwrap().clone();
            if args.pause
                && pause.is_none()
                && let (Some(expect), Some(a)) =
                    (args.expect, last.as_ref().and_then(|o| o.active.as_ref()))
                && a.discovered_files as u64 * 10 >= expect * 3
            {
                pause = Some(pause_resume(&ws.lock().unwrap()));
            }
            let reported_idle = last.is_some_and(|o| o.active.is_none() && o.queued_sessions == 0);
            if (reported_idle || last_check.elapsed() > Duration::from_secs(2))
                && idle(&ws.lock().unwrap())
            {
                break;
            }
            if last_check.elapsed() > Duration::from_secs(2) {
                last_check = Instant::now();
            }
        }
        if started.elapsed() > DEADLINE {
            fail("timed out");
        }
        let gap = if first_rows.is_none() { 2 } else { 50 };
        std::thread::sleep(Duration::from_millis(gap));
    }
    let wall = started.elapsed();
    scanning.store(false, Ordering::Relaxed);
    let browse_times = browse.map(|h| h.join().unwrap()).unwrap_or_default();
    let (cpu_ms, peak_rss, peak_cpu) = sampler.finish();

    let files = file_count(&ws.lock().unwrap());
    if let Some(expect) = args.expect
        && files != expect
    {
        fail(&format!("cataloged {files} files, expected {expect}"));
    }
    let scanned = catalog_sizes(&s.data);

    // Warm queries on the finished library.
    let mix = browse_mix(&root_id);
    let warm: Vec<Duration> = (0..60)
        .map(|i| {
            let t = Instant::now();
            run_query(&ws.lock().unwrap(), &mix[i % mix.len()]);
            t.elapsed()
        })
        .collect();

    // The status command (get_inventory_status) on the finished library.
    let status: Vec<Duration> = (0..30)
        .map(|_| {
            let t = Instant::now();
            run_query(&ws.lock().unwrap(), &None);
            t.elapsed()
        })
        .collect();

    // Cold: close everything, reopen the library, time the first page.
    ws.lock().unwrap().shutdown();
    drop(ws);
    let closed = catalog_sizes(&s.data);
    let reopen_started = Instant::now();
    let mut again = workspace(&s.config, args.power, rate, Arc::default());
    again.startup();
    let reopen_ms = reopen_started.elapsed().as_secs_f64() * 1000.0;
    let cold_started = Instant::now();
    run_query(&again, &Some(query()));
    let cold_ms = cold_started.elapsed().as_secs_f64() * 1000.0;
    again.shutdown();

    let secs = wall.as_secs_f64();
    json!({
        "rate": rate.label(),
        "files": files,
        "wallSeconds": round(secs, 3),
        "filesPerSecond": round(files as f64 / secs, 1),
        "entriesPerSecond": round(count_entries(&args.fixture) as f64 / secs, 1),
        "cpuMs": cpu_ms,
        "avgCpuPercentOfOneCore": round(cpu_ms as f64 / 10.0 / secs, 1),
        "harnessCpuMsPerSecond": round(baseline_per_sec, 1),
        "scanCpuMs": round((cpu_ms as f64 - baseline_per_sec * secs).max(0.0), 0),
        "scanCpuPercentOfOneCore": round(
            (cpu_ms as f64 - baseline_per_sec * secs).max(0.0) / 10.0 / secs,
            1
        ),
        "sampledPeakCpuPercent": round(peak_cpu as f64, 1),
        "sampledPeakRssBytes": peak_rss,
        "progressEvents": progress.events.load(Ordering::Relaxed),
        "createLibraryMs": create_ms,
        "firstRowsMs": first_rows.map(|d| round(d.as_secs_f64() * 1000.0, 1)),
        "browseWhileScanning": percentiles(browse_times),
        "warmQueries": percentiles(warm),
        "statusQuery": percentiles(status),
        "reopenMs": round(reopen_ms, 1),
        "coldFirstPageMs": round(cold_ms, 2),
        "catalogEmpty": empty,
        "catalogScanned": scanned,
        "catalogClosed": closed,
        "pauseResume": pause,
    })
}

/// Pauses mid-scan, checks nothing more is written, resumes.
fn pause_resume(ws: &Workspace) -> Value {
    let root = ws.require_open().unwrap();
    let job = control::overview(root).unwrap().active.unwrap().job_id;
    let t = Instant::now();
    control::pause(root, &job).unwrap();
    let pause_ms = t.elapsed().as_secs_f64() * 1000.0;
    let at_pause = file_count(ws);
    std::thread::sleep(Duration::from_millis(500));
    let late_writes = file_count(ws).saturating_sub(at_pause);
    let t = Instant::now();
    control::resume(root, &job).unwrap();
    wait(|| {
        file_count(ws) > at_pause
            || control::overview(root)
                .is_ok_and(|o| o.active.is_none_or(|a| a.state == JobState::Completed))
    });
    json!({
        "pauseCallMs": round(pause_ms, 2),
        "filesWrittenAfterPause": late_writes,
        "resumeToNextWriteMs": round(t.elapsed().as_secs_f64() * 1000.0, 1),
    })
}

/// Kills a child process mid-scan, then reopens and finishes (P0-08).
fn recovery_run(args: &Args) -> Value {
    let s = scratch();
    {
        let mut ws = workspace(&s.config, args.power, Rate::Unthrottled, Arc::default());
        ws.choose(&s.data).unwrap();
        ws.shutdown();
    }
    let mut child: Child = Command::new(std::env::current_exe().unwrap())
        .args(["--fixture".as_ref(), args.fixture.as_os_str()])
        .args(["--child-scan".as_ref(), s.config.as_os_str()])
        .spawn()
        .unwrap();
    let db = s.data.join(layout::CATALOG);
    let target = args.expect.unwrap_or(1000) / 3;
    wait(|| rows_read_only(&db).is_some_and(|n| n >= target));
    child.kill().unwrap();
    child.wait().unwrap();
    let at_kill = rows_read_only(&db).unwrap_or(0);

    let t = Instant::now();
    let mut ws = workspace(&s.config, args.power, Rate::Unthrottled, Arc::default());
    ws.startup();
    let reopen_ms = t.elapsed().as_secs_f64() * 1000.0;
    wait(|| idle(&ws));
    let finish = t.elapsed();
    let files = file_count(&ws);
    ws.shutdown();
    let distinct = distinct_read_only(&db);
    json!({
        "filesAtKill": at_kill,
        "reopenMs": round(reopen_ms, 1),
        "resumeToCompleteSeconds": round(finish.as_secs_f64(), 3),
        "files": files,
        "duplicateRows": files.saturating_sub(distinct),
    })
}

/// Child mode: open the library, catalog the fixture, run until killed.
fn child_scan(args: &Args, config: &Path) -> ! {
    let mut ws = workspace(config, args.power, Rate::Unthrottled, Arc::default());
    ws.startup();
    let root = ws.require_open().unwrap();
    let added = sources::add(root, &args.fixture).unwrap();
    control::enqueue(root, &added.id).unwrap();
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn rows_read_only(db: &Path) -> Option<u64> {
    let c = rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .ok()?;
    c.busy_timeout(Duration::from_secs(1)).ok()?;
    c.query_row("SELECT COUNT(*) FROM file_instances", [], |r| {
        r.get::<_, i64>(0)
    })
    .ok()
    .map(|n| n as u64)
}

fn distinct_read_only(db: &Path) -> u64 {
    let c = rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .unwrap();
    c.query_row(
        "SELECT COUNT(*) FROM (SELECT DISTINCT source_root_id, rel_path FROM file_instances)",
        [],
        |r| r.get::<_, i64>(0),
    )
    .unwrap() as u64
}

fn round(v: f64, places: i32) -> f64 {
    let f = 10f64.powi(places);
    (v * f).round() / f
}
