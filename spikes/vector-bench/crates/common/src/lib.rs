//! Shared code for the vector-engine spike: the data files written by
//! `datagen`, an `Engine` trait each candidate implements, and one scenario
//! runner so every engine is measured the same way.
//!
//! Files in a data directory (all little-endian):
//! - `base.fvecs`, `queries.fvecs`: per vector an `i32` dimension, then the
//!   `f32` components. Vectors are unit length, so cosine = dot product.
//! - `roots.u16`: the source-root id of each base vector.
//! - `gt.ivecs`, `gt-root<r>.ivecs`: exact top-10 ids per query, unfiltered
//!   and restricted to root `r`.
//! - `meta.json`: sizes, seed, and the filter roots with their selectivity.

use std::error::Error;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

pub type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

pub const K: usize = 10;

// ---------------------------------------------------------------- files

pub fn write_fvecs(path: &Path, dim: usize, data: &[f32]) -> Result<()> {
    let mut w = BufWriter::with_capacity(1 << 20, File::create(path)?);
    for v in data.chunks_exact(dim) {
        w.write_all(&(dim as i32).to_le_bytes())?;
        for x in v {
            w.write_all(&x.to_le_bytes())?;
        }
    }
    w.flush()?;
    Ok(())
}

/// Reads an `.fvecs` file into one contiguous buffer.
pub fn read_fvecs(path: &Path) -> Result<(usize, Vec<f32>)> {
    let mut r = BufReader::with_capacity(1 << 20, File::open(path)?);
    let len = fs::metadata(path)?.len() as usize;
    let mut head = [0u8; 4];
    r.read_exact(&mut head)?;
    let dim = i32::from_le_bytes(head) as usize;
    let rows = len / (4 + 4 * dim);
    let mut out = Vec::with_capacity(rows * dim);
    let mut row = vec![0u8; 4 * dim];
    for i in 0..rows {
        if i > 0 {
            r.read_exact(&mut head)?;
        }
        r.read_exact(&mut row)?;
        out.extend(
            row.chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        );
    }
    Ok((dim, out))
}

pub fn write_ivecs(path: &Path, rows: &[Vec<u32>]) -> Result<()> {
    let mut w = BufWriter::new(File::create(path)?);
    for row in rows {
        w.write_all(&(row.len() as i32).to_le_bytes())?;
        for x in row {
            w.write_all(&x.to_le_bytes())?;
        }
    }
    w.flush()?;
    Ok(())
}

pub fn read_ivecs(path: &Path) -> Result<Vec<Vec<u32>>> {
    let bytes = fs::read(path)?;
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let n = i32::from_le_bytes(bytes[at..at + 4].try_into()?) as usize;
        at += 4;
        out.push(
            bytes[at..at + 4 * n]
                .chunks_exact(4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect(),
        );
        at += 4 * n;
    }
    Ok(out)
}

/// A generated data set, loaded into memory.
pub struct Data {
    pub dir: PathBuf,
    pub dim: usize,
    pub base: Vec<f32>,
    pub queries: Vec<f32>,
    pub roots: Vec<u16>,
    pub gt: Vec<Vec<u32>>,
    /// (root, selectivity, ground truth restricted to that root)
    pub filters: Vec<(u16, f64, Vec<Vec<u32>>)>,
    pub meta: Value,
}

impl Data {
    pub fn load(dir: &Path) -> Result<Data> {
        let meta: Value = serde_json::from_slice(&fs::read(dir.join("meta.json"))?)?;
        let (dim, base) = read_fvecs(&dir.join("base.fvecs"))?;
        let (_, queries) = read_fvecs(&dir.join("queries.fvecs"))?;
        let roots = fs::read(dir.join("roots.u16"))?
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        let gt = read_ivecs(&dir.join("gt.ivecs"))?;
        let mut filters = Vec::new();
        for f in meta["filters"].as_array().ok_or("meta.json: no filters")? {
            let root = f["root"].as_u64().ok_or("filter root")? as u16;
            let sel = f["selectivity"].as_f64().ok_or("filter selectivity")?;
            filters.push((
                root,
                sel,
                read_ivecs(&dir.join(format!("gt-root{root}.ivecs")))?,
            ));
        }
        Ok(Data {
            dir: dir.to_path_buf(),
            dim,
            base,
            queries,
            roots,
            gt,
            filters,
            meta,
        })
    }

    pub fn n(&self) -> usize {
        self.base.len() / self.dim
    }

    pub fn vector(&self, id: usize) -> &[f32] {
        &self.base[id * self.dim..(id + 1) * self.dim]
    }

    pub fn query(&self, i: usize) -> &[f32] {
        &self.queries[i * self.dim..(i + 1) * self.dim]
    }

    pub fn query_count(&self) -> usize {
        self.queries.len() / self.dim
    }
}

// ---------------------------------------------------------------- metrics

pub fn recall(found: &[u64], truth: &[u32]) -> f64 {
    let want = truth.len().min(K);
    if want == 0 {
        return 1.0;
    }
    let hit = truth[..want]
        .iter()
        .filter(|t| found.contains(&(**t as u64)))
        .count();
    hit as f64 / want as f64
}

pub fn ms(d: Duration) -> f64 {
    (d.as_secs_f64() * 1e6).round() / 1e3
}

/// p50, p95, max (milliseconds) of a set of durations.
pub fn latency(mut v: Vec<Duration>) -> Value {
    v.sort();
    let at = |p: f64| v[((v.len() as f64 * p).ceil() as usize).clamp(1, v.len()) - 1];
    json!({
        "count": v.len(),
        "p50Ms": ms(at(0.50)),
        "p95Ms": ms(at(0.95)),
        "maxMs": ms(*v.last().unwrap_or(&Duration::ZERO)),
    })
}

/// Peak resident memory of this process, from the OS (exact, not sampled).
pub fn peak_rss_bytes() -> u64 {
    #[cfg(unix)]
    unsafe {
        let mut u: libc::rusage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_SELF, &mut u);
        // ru_maxrss is bytes on macOS and KiB on Linux.
        if cfg!(target_os = "macos") {
            u.ru_maxrss as u64
        } else {
            u.ru_maxrss as u64 * 1024
        }
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::ProcessStatus::{
            GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
        };
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        let mut c: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb);
        c.PeakWorkingSetSize as u64
    }
}

pub fn dir_bytes(path: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.is_file() {
        return meta.len();
    }
    fs::read_dir(path)
        .map(|it| it.flatten().map(|e| dir_bytes(&e.path())).sum())
        .unwrap_or(0)
}

// ---------------------------------------------------------------- CLI

pub struct Args(Vec<String>);

impl Args {
    pub fn from_env() -> Args {
        Args(std::env::args().skip(1).collect())
    }
    pub fn get(&self, name: &str) -> Option<&str> {
        let flag = format!("--{name}");
        self.0
            .iter()
            .position(|a| *a == flag)
            .and_then(|i| self.0.get(i + 1))
            .map(|s| s.as_str())
    }
    pub fn path(&self, name: &str) -> Result<PathBuf> {
        Ok(PathBuf::from(
            self.get(name).ok_or_else(|| format!("missing --{name}"))?,
        ))
    }
    pub fn num(&self, name: &str, default: usize) -> Result<usize> {
        match self.get(name) {
            Some(v) => Ok(v.parse()?),
            None => Ok(default),
        }
    }
    pub fn mode(&self) -> &str {
        self.0.first().map(|s| s.as_str()).unwrap_or("bench")
    }
}

// ---------------------------------------------------------------- engines

/// One search setting to sweep (for example HNSW `ef`), from fast to exact.
#[derive(Clone, Debug)]
pub struct Knob {
    pub label: String,
    pub value: f64,
}

pub fn knob(label: &str, value: f64) -> Knob {
    Knob {
        label: label.into(),
        value,
    }
}

pub trait Engine {
    /// Insert every base vector (id = index) with its root and build any
    /// index. The runner times the whole call.
    fn build(&mut self, data: &Data) -> Result<()>;
    /// Drop everything in memory and open the index from disk again.
    fn reopen(&mut self) -> Result<()>;
    /// Settings to sweep, fastest first; empty for exact engines.
    fn knobs(&self) -> Vec<Knob>;
    fn set_knob(&mut self, k: &Knob) -> Result<()>;
    fn search(&mut self, q: &[f32], k: usize, root: Option<u16>) -> Result<Vec<u64>>;
    fn delete(&mut self, ids: &[u64]) -> Result<()>;
    /// Optional maintenance after deletes (compaction, cleanup).
    fn compact(&mut self) -> Result<bool> {
        Ok(false)
    }
    /// Index files on disk.
    fn disk_bytes(&self) -> u64;
    /// Engine-specific facts for the report (versions, settings).
    fn describe(&self) -> Value;
    /// Crash test: keep inserting `data` in batches, printing
    /// `committed <n>` after each durable step, until killed.
    fn crash_insert(&mut self, data: &Data) -> Result<()>;
    /// After a crash: open what is on disk and report what survived.
    fn crash_verify(&mut self, data: &Data) -> Result<Value>;
}

/// Runs the standard scenario and returns the report.
pub fn run(engine: &mut dyn Engine, data: &Data, queries: usize) -> Result<Value> {
    let nq = queries.min(data.query_count());
    let raw = (data.n() * data.dim * 4) as u64;

    let t = Instant::now();
    engine.build(data)?;
    let build = t.elapsed();
    let disk = engine.disk_bytes();
    eprintln!(
        "vbench: built in {:.1} s, {:.1} MiB on disk",
        build.as_secs_f64(),
        disk as f64 / 1048576.0
    );

    let t = Instant::now();
    engine.reopen()?;
    let opened = t.elapsed();
    let knobs = engine.knobs();
    if let Some(first) = knobs.last() {
        engine.set_knob(first)?;
    }
    let t = Instant::now();
    engine.search(data.query(0), K, None)?;
    let first_query = t.elapsed();

    // Recall/latency sweep, single thread, unfiltered.
    let settings = if knobs.is_empty() {
        vec![knob("exact", 0.0)]
    } else {
        knobs.clone()
    };
    let mut sweep = Vec::new();
    for s in &settings {
        if !knobs.is_empty() {
            engine.set_knob(s)?;
        }
        // Warm-up so the first point isn't charged for page faults.
        for i in 0..nq.min(20) {
            engine.search(data.query(i), K, None)?;
        }
        let mut times = Vec::with_capacity(nq);
        let mut rec = 0.0;
        for i in 0..nq {
            let t = Instant::now();
            let found = engine.search(data.query(i), K, None)?;
            times.push(t.elapsed());
            rec += recall(&found, &data.gt[i]);
        }
        let point = json!({ "knob": s.label, "value": s.value, "recall": rec / nq as f64, "latency": latency(times) });
        eprintln!(
            "vbench: {} {} recall {:.4} p95 {} ms",
            s.label, s.value, point["recall"], point["latency"]["p95Ms"]
        );
        sweep.push(point);
    }

    // Filtered searches at the most accurate setting that reaches 0.95
    // recall (or the most accurate one, if none does).
    let chosen = sweep
        .iter()
        .position(|p| p["recall"].as_f64().unwrap_or(0.0) >= 0.95)
        .unwrap_or(settings.len() - 1);
    if !knobs.is_empty() {
        engine.set_knob(&settings[chosen])?;
    }
    let mut filtered = Vec::new();
    for (root, sel, gt) in &data.filters {
        let mut times = Vec::with_capacity(nq);
        let mut rec = 0.0;
        let mut wrong_root = 0;
        for i in 0..nq {
            let t = Instant::now();
            let found = engine.search(data.query(i), K, Some(*root))?;
            times.push(t.elapsed());
            rec += recall(&found, &gt[i]);
            wrong_root += found
                .iter()
                .filter(|id| data.roots[**id as usize] != *root)
                .count();
        }
        filtered.push(json!({
            "root": root, "selectivity": sel, "knob": settings[chosen].label, "value": settings[chosen].value,
            "recall": rec / nq as f64, "latency": latency(times), "resultsFromOtherRoots": wrong_root,
        }));
    }

    // Delete 1% (every id divisible by 100), then check none come back.
    let doomed: Vec<u64> = (0..data.n() as u64).filter(|i| i % 100 == 0).collect();
    let t = Instant::now();
    engine.delete(&doomed)?;
    let delete = t.elapsed();
    let mut resurrected = 0;
    for i in 0..nq.min(200) {
        resurrected += engine
            .search(data.query(i), K, None)?
            .iter()
            .filter(|id| *id % 100 == 0)
            .count();
    }
    let disk_after_delete = engine.disk_bytes();
    let t = Instant::now();
    let compacted = engine.compact()?;
    let compact = t.elapsed();

    Ok(json!({
        "engine": engine.describe(),
        "appLibraries": app_libraries(),
        "n": data.n(),
        "dim": data.dim,
        "queries": nq,
        "buildSeconds": build.as_secs_f64(),
        "diskBytes": disk,
        "rawBytes": raw,
        "diskOverRaw": disk as f64 / raw as f64,
        "reopenMs": ms(opened),
        "firstQueryMs": ms(first_query),
        "openToFirstAnswerMs": ms(opened + first_query),
        "sweep": sweep,
        "filtered": filtered,
        "delete": {
            "ids": doomed.len(),
            "ms": ms(delete),
            "deletedIdsReturned": resurrected,
            "diskBytesAfter": disk_after_delete,
            "compacted": compacted,
            "compactMs": if compacted { Some(ms(compact)) } else { None },
            "diskBytesAfterCompact": if compacted { Some(engine.disk_bytes()) } else { None },
        },
        "peakRssBytes": peak_rss_bytes(),
    }))
}

/// Touches the libraries the app links, so every binary contains them.
pub fn app_libraries() -> Value {
    let sqlite = rusqlite::Connection::open_in_memory()
        .and_then(|c| c.query_row("select sqlite_version()", [], |r| r.get::<_, String>(0)))
        .unwrap_or_default();
    let tokio = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .build()
        .is_ok();
    json!({ "sqlite": sqlite, "tokio": tokio })
}

/// Entry point shared by every engine binary.
///
///   <engine> bench --data <dir> --index <dir> [--queries N] [--variant V]
///   <engine> crash-insert --data <dir> --index <dir> [--variant V]
///   <engine> crash-verify --data <dir> --index <dir> [--variant V]
pub fn main_with(make: impl Fn(&Path, Option<&str>) -> Result<Box<dyn Engine>>) {
    if let Err(e) = main_inner(make) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn main_inner(make: impl Fn(&Path, Option<&str>) -> Result<Box<dyn Engine>>) -> Result<()> {
    let args = Args::from_env();
    let data_dir = args.path("data")?;
    let index = args.path("index")?;
    let variant = args.get("variant");
    let t = Instant::now();
    let data = Data::load(&data_dir)?;
    eprintln!(
        "vbench: loaded {} × {} in {:.1} s",
        data.n(),
        data.dim,
        t.elapsed().as_secs_f64()
    );
    match args.mode() {
        "bench" => {
            if index.exists() {
                fs::remove_dir_all(&index)?;
            }
            fs::create_dir_all(&index)?;
            let mut engine = make(&index, variant)?;
            let report = run(engine.as_mut(), &data, args.num("queries", 1000)?)?;
            println!("{report}");
        }
        "crash-insert" => {
            if index.exists() {
                fs::remove_dir_all(&index)?;
            }
            fs::create_dir_all(&index)?;
            make(&index, variant)?.crash_insert(&data)?;
        }
        "crash-verify" => {
            let report = make(&index, variant)?.crash_verify(&data)?;
            println!("{report}");
        }
        other => return Err(format!("unknown mode {other}").into()),
    }
    Ok(())
}

/// Prints a progress line the crash test parent waits for.
pub fn committed(n: usize) {
    println!("committed {n}");
    let _ = std::io::stdout().flush();
}
