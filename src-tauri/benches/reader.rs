//! Phase 3 page view benchmark (phase3-spec §13; docs/benchmarks.md).
//!
//! Renders pages of the spike corpus's PDFs through the real reader worker,
//! as the page view does. It reports:
//! - the first render after the worker starts;
//! - renders at a width (1,200 px by default), p50 and p95, with and
//!   without a passage to find;
//! - a cache hit;
//! - the worker's peak memory.
//!
//! The target to report against is under 500 ms p95 at 1,200 px on the
//! reference machine. Never asserts it: shared CI runners are noisy.
//!
//! ```text
//! cargo bench --bench reader -- [--width 1200] [--label corpus] [--out results.json]
//! ```

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::json;

use atlas_lib::domain::dto::FileType;
use atlas_lib::services::extraction::supervisor::{
    DocumentProcessor, Extraction, WorkerCommand, WorkerStats,
};
use atlas_lib::services::reader::{Prepared, ReaderConfig, ReaderService};

struct Args {
    width: u32,
    label: String,
    out: Option<PathBuf>,
}

fn args() -> Args {
    let mut a = Args {
        width: 1200,
        label: "corpus".into(),
        out: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || it.next().expect("a value");
        match arg.as_str() {
            "--width" => a.width = value().parse().expect("a width"),
            "--label" => a.label = value(),
            "--out" => a.out = Some(PathBuf::from(value())),
            // cargo bench passes --bench.
            _ => {}
        }
    }
    a
}

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../spikes/extraction-bench/corpus")
}

/// The corpus's text PDFs: generated and real.
fn pdfs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in ["pdf/generated", "pdf/real"] {
        let Ok(entries) = fs::read_dir(corpus().join(dir)) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "pdf"))
            .collect();
        files.sort();
        out.extend(files);
    }
    out
}

fn millis(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

fn round(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

fn machine() -> serde_json::Value {
    json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "logicalCores": std::thread::available_parallelism().map(|n| n.get()).ok(),
        "ci": std::env::var("CI").is_ok(),
    })
}

fn summary(mut times: Vec<f64>) -> serde_json::Value {
    times.sort_by(f64::total_cmp);
    json!({
        "renders": times.len(),
        "p50Ms": round(percentile(&times, 0.5)),
        "p95Ms": round(percentile(&times, 0.95)),
        "maxMs": round(times.last().copied().unwrap_or(0.0)),
    })
}

fn prepared(path: &Path, tag: &str) -> Prepared {
    Prepared {
        // A key per file and run, so nothing comes from the cache unless
        // asked for.
        hash: blake3::hash(format!("{tag}:{}", path.display()).as_bytes())
            .to_hex()
            .to_string(),
        text: String::new(),
        path: Ok(path.to_path_buf()),
    }
}

fn main() {
    let a = args();
    let command = WorkerCommand::new(env!("CARGO_BIN_EXE_atlas"));
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let files = pdfs();
    assert!(!files.is_empty(), "the corpus's PDFs");

    // Each page's text, from the extraction worker, to look for.
    let mut extractor = atlas_lib::services::extraction::supervisor::Supervisor::new(
        command.clone(),
        Default::default(),
        std::sync::Arc::new(|| false),
        std::sync::Arc::new(WorkerStats::default()),
    );
    let mut pages: Vec<(PathBuf, u32, String)> = Vec::new();
    for f in &files {
        if let Ok(Extraction::Done { pieces, .. }) = extractor.extract(f, FileType::Pdf) {
            for p in pieces.iter().take(3) {
                pages.push((f.clone(), p.page_no.unwrap_or(1), p.text.clone()));
            }
        }
    }
    drop(extractor);

    let service = ReaderService::new(scratch.path(), Some(ReaderConfig::new(command)));
    // The first render pays for the worker's start and PDFium's load.
    let (first_file, first_page, _) = pages.first().cloned().expect("a page");
    let started = Instant::now();
    service
        .render(prepared(&first_file, "first"), first_page, a.width, None)
        .expect("the first page renders");
    let first = millis(started.elapsed());

    let mut plain = Vec::new();
    let mut with_find = Vec::new();
    let mut found = 0usize;
    let mut not_found: Vec<String> = Vec::new();
    for (i, (file, page, text)) in pages.iter().enumerate() {
        let t = Instant::now();
        if service
            .render(prepared(file, &format!("plain{i}")), *page, a.width, None)
            .is_ok()
        {
            plain.push(millis(t.elapsed()));
        }
        let excerpt: String = text.chars().skip(40).take(400).collect();
        let t = Instant::now();
        if let Ok(img) = service.render(
            prepared(file, &format!("find{i}")),
            *page,
            a.width,
            Some(&excerpt),
        ) {
            with_find.push(millis(t.elapsed()));
            found += usize::from(img.found);
            if !img.found {
                let name = file.file_name().unwrap_or_default().to_string_lossy();
                not_found.push(format!("{name} p{page}"));
            }
        }
    }

    // A cache hit: the same page again.
    let t = Instant::now();
    let hit = service
        .render(prepared(&first_file, "first"), first_page, a.width, None)
        .expect("a cached page");
    let cache_hit = millis(t.elapsed());
    assert!(hit.from_cache);

    let stats = service.stats();
    let peak = WorkerStats::get(&stats.peak_rss_bytes);
    let result = json!({
        "bench": "reader",
        "label": a.label,
        "machine": machine(),
        "width": a.width,
        "files": files.len(),
        "pages": pages.len(),
        "firstRenderMs": round(first),
        "render": summary(plain),
        "renderWithFind": summary(with_find),
        "found": found,
        // An excerpt not outlined: the file and page, to see why.
        "notFound": not_found,
        "cacheHitMs": round(cache_hit),
        "workerPeakRssBytes": peak,
        "spawns": WorkerStats::get(&stats.spawns),
        "kills": WorkerStats::get(&stats.kills),
    });
    println!("{}", serde_json::to_string_pretty(&result).unwrap());
    if let Some(out) = a.out {
        fs::write(out, serde_json::to_vec_pretty(&result).unwrap()).expect("writes the results");
    }
}
