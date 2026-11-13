//! Scores every run against the corpus by ADR 0020's definitions.
//!
//! ```text
//! score --corpus <build/corpus> --results <dir> [--seed N] > scores.json
//! ```
//!
//! `<dir>` holds one `<variant>@<dpi>.jsonl` per run (the engines'
//! contract). The corpus folder holds `jobs.jsonl` (each page's id and
//! category), `truth/<id>.txt` and, for the poor-layer pages, the file's
//! own text layer in `layer/<id>.txt`, scored as the `text-layer` row.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ocrbench::score::{Difference, PageScore, bootstrap, page_mean, score};
use serde::Serialize;
use serde_json::{Value, json};

#[derive(serde::Deserialize)]
struct Job {
    id: String,
    category: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PageResult {
    id: String,
    category: String,
    outcome: String,
    #[serde(flatten)]
    score: Option<PageScore>,
    render_ms: Option<f64>,
    ocr_ms: Option<f64>,
    cpu_ms: Option<f64>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct Summary {
    pages: usize,
    failed: usize,
    over_60_s: usize,
    /// Mean of the pages' WERs (the rule's figure); a page that failed
    /// counts as every word missed.
    wer: f64,
    cer: f64,
    /// Over all words pooled.
    micro_wer: f64,
    unordered_word_error: f64,
    reading_order: f64,
    by_category: BTreeMap<String, Value>,
    /// Seconds per page, after the first.
    ocr_s_median: Option<f64>,
    ocr_s_p90: Option<f64>,
    render_s_median: Option<f64>,
    first_page_s: Option<f64>,
    cpu_per_wall: Option<f64>,
    init_ms: Option<f64>,
    peak_rss_mib: Option<f64>,
    rss_after_init_mib: Option<f64>,
    engine_info: Option<Value>,
}

fn quantile(values: &mut [f64], q: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    Some(values[((values.len() - 1) as f64 * q).round() as usize])
}

fn read_lines(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// One run's pages, scored, and its summary.
fn score_run(
    lines: &[Value],
    jobs: &[Job],
    truth: &BTreeMap<String, String>,
) -> (Vec<PageResult>, Summary) {
    let by_id: BTreeMap<&str, &Value> = lines
        .iter()
        .filter_map(|l| l.get("id").and_then(Value::as_str).map(|id| (id, l)))
        .collect();
    let mut pages = Vec::new();
    for job in jobs {
        let line = by_id.get(job.id.as_str());
        let outcome = line
            .and_then(|l| l.get("outcome"))
            .and_then(Value::as_str)
            .unwrap_or("missing")
            .to_owned();
        let text = line
            .and_then(|l| l.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let reference = truth.get(&job.id).map(String::as_str).unwrap_or("");
        let ms = |k: &str| line.and_then(|l| l.get(k)).and_then(Value::as_f64);
        pages.push(PageResult {
            id: job.id.clone(),
            category: job.category.clone(),
            score: Some(score(if outcome == "ok" { text } else { "" }, reference)),
            outcome,
            render_ms: ms("renderMs"),
            ocr_ms: ms("ocrMs"),
            cpu_ms: ms("cpuMs"),
        });
    }
    let mut s = Summary {
        pages: pages.len(),
        failed: pages.iter().filter(|p| p.outcome != "ok").count(),
        over_60_s: pages
            .iter()
            .filter(|p| p.ocr_ms.is_some_and(|ms| ms > 60_000.0))
            .count(),
        ..Summary::default()
    };
    let scores: Vec<&PageScore> = pages.iter().filter_map(|p| p.score.as_ref()).collect();
    s.wer = page_mean(&scores.iter().map(|p| p.wer).collect::<Vec<_>>());
    s.cer = page_mean(&scores.iter().map(|p| p.cer).collect::<Vec<_>>());
    s.unordered_word_error = page_mean(
        &scores
            .iter()
            .map(|p| p.unordered_word_error)
            .collect::<Vec<_>>(),
    );
    s.reading_order = page_mean(&scores.iter().map(|p| p.reading_order).collect::<Vec<_>>());
    let (edits, words) = scores
        .iter()
        .fold((0, 0), |(e, w), p| (e + p.word_edits, w + p.ref_words));
    s.micro_wer = edits as f64 / words.max(1) as f64;
    let mut categories: BTreeMap<String, Vec<&PageResult>> = BTreeMap::new();
    for p in &pages {
        categories.entry(p.category.clone()).or_default().push(p);
    }
    for (category, ps) in categories {
        let sc: Vec<&PageScore> = ps.iter().filter_map(|p| p.score.as_ref()).collect();
        s.by_category.insert(
            category,
            json!({
                "pages": ps.len(),
                "wer": page_mean(&sc.iter().map(|p| p.wer).collect::<Vec<_>>()),
                "cer": page_mean(&sc.iter().map(|p| p.cer).collect::<Vec<_>>()),
                "readingOrder": page_mean(&sc.iter().map(|p| p.reading_order).collect::<Vec<_>>()),
            }),
        );
    }
    // Timings: in the order the run read them, the first apart.
    let order: Vec<&Value> = lines.iter().filter(|l| l.get("id").is_some()).collect();
    let first = order
        .first()
        .and_then(|l| l.get("ocrMs"))
        .and_then(Value::as_f64);
    let mut ocr: Vec<f64> = order
        .iter()
        .skip(1)
        .filter_map(|l| l.get("ocrMs").and_then(Value::as_f64))
        .map(|ms| ms / 1e3)
        .collect();
    let mut render: Vec<f64> = order
        .iter()
        .skip(1)
        .filter_map(|l| l.get("renderMs").and_then(Value::as_f64))
        .map(|ms| ms / 1e3)
        .collect();
    let (cpu, wall) = order.iter().skip(1).fold((0.0, 0.0), |(c, w), l| {
        (
            c + l.get("cpuMs").and_then(Value::as_f64).unwrap_or(0.0),
            w + l.get("ocrMs").and_then(Value::as_f64).unwrap_or(0.0),
        )
    });
    s.ocr_s_median = quantile(&mut ocr, 0.5);
    s.ocr_s_p90 = quantile(&mut ocr, 0.9);
    s.render_s_median = quantile(&mut render, 0.5);
    s.first_page_s = first.map(|ms| ms / 1e3);
    s.cpu_per_wall = (wall > 0.0).then(|| cpu / wall);
    for l in lines {
        if l.get("ready").is_some() {
            s.init_ms = l.get("initMs").and_then(Value::as_f64);
        }
        if l.get("done").is_some() {
            let mib = |k: &str| l.get(k).and_then(Value::as_f64).map(|b| b / 1_048_576.0);
            s.peak_rss_mib = mib("peakRssBytes");
            s.rss_after_init_mib = mib("rssAfterInitBytes");
            s.engine_info = l.get("engineInfo").cloned();
        }
    }
    (pages, s)
}

/// `--pairwise <results dir>`: with no reference text (the owner's own
/// scans), how far apart each two runs' texts are, page by page: character
/// edits over the longer text, averaged over the pages both read.
fn pairwise(results: &Path) {
    let mut runs: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for entry in std::fs::read_dir(results)
        .expect("the results folder")
        .flatten()
    {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "jsonl") {
            continue;
        }
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let pages = read_lines(&path)
            .into_iter()
            .filter(|l| l.get("outcome").and_then(Value::as_str) == Some("ok"))
            .filter_map(|l| {
                Some((
                    l.get("id")?.as_str()?.to_owned(),
                    l.get("text")?.as_str()?.to_owned(),
                ))
            })
            .collect();
        runs.insert(name, pages);
    }
    let names: Vec<&String> = runs.keys().collect();
    let mut pairs = Vec::new();
    for (i, a) in names.iter().enumerate() {
        for b in &names[i + 1..] {
            let mut rates = Vec::new();
            for (id, ta) in &runs[*a] {
                let Some(tb) = runs[*b].get(id) else { continue };
                let ca: Vec<char> = ocrbench::score::normalise(ta).chars().collect();
                let cb: Vec<char> = ocrbench::score::normalise(tb).chars().collect();
                let longest = ca.len().max(cb.len());
                if longest > 0 {
                    rates.push(ocrbench::score::levenshtein(&ca, &cb) as f64 / longest as f64);
                }
            }
            pairs.push(json!({ "a": a, "b": b, "pages": rates.len(), "meanDisagreement": page_mean(&rates) }));
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({ "pairs": pairs })).expect("JSON")
    );
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--pairwise") {
        let dir = std::env::args().nth(2).expect("--pairwise <results dir>");
        pairwise(Path::new(&dir));
        return;
    }
    let mut corpus = None;
    let mut results = None;
    let mut seed = 20_261_112u64;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--corpus" => corpus = it.next().map(PathBuf::from),
            "--results" => results = it.next().map(PathBuf::from),
            "--seed" => seed = it.next().and_then(|s| s.parse().ok()).expect("--seed N"),
            _ => panic!("unknown argument {a}"),
        }
    }
    let corpus = corpus.expect("--corpus <dir>");
    let results = results.expect("--results <dir>");
    let jobs: Vec<Job> = read_lines(&corpus.join("jobs.jsonl"))
        .into_iter()
        .map(|v| serde_json::from_value(v).expect("a job"))
        .collect();
    let truth: BTreeMap<String, String> = jobs
        .iter()
        .map(|j| {
            let t = std::fs::read_to_string(corpus.join("truth").join(format!("{}.txt", j.id)))
                .unwrap_or_else(|_| panic!("no truth for {}", j.id));
            (j.id.clone(), t)
        })
        .collect();

    let mut runs: BTreeMap<String, (Vec<PageResult>, Summary)> = BTreeMap::new();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&results)
        .expect("the results folder")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        // Runs are named `<variant>@<dpi>`; the driver's job lists sit
        // beside them.
        .filter(|p| {
            p.file_stem()
                .is_some_and(|s| s.to_string_lossy().contains('@'))
        })
        .collect();
    files.sort();
    for file in files {
        let name = file.file_stem().unwrap().to_string_lossy().into_owned();
        runs.insert(name, score_run(&read_lines(&file), &jobs, &truth));
    }

    // The text-layer row: the poor-layer pages' own OCR layer.
    let layer_jobs: Vec<Job> = jobs
        .iter()
        .filter(|j| corpus.join("layer").join(format!("{}.txt", j.id)).exists())
        .map(|j| Job {
            id: j.id.clone(),
            category: j.category.clone(),
        })
        .collect();
    let layer_lines: Vec<Value> = layer_jobs
        .iter()
        .map(|j| {
            let t = std::fs::read_to_string(corpus.join("layer").join(format!("{}.txt", j.id)))
                .unwrap_or_default();
            json!({ "id": j.id, "outcome": "ok", "text": t })
        })
        .collect();
    let (layer_pages, layer) = score_run(&layer_lines, &layer_jobs, &truth);

    // Differences of WER between every two runs at the same DPI.
    let mut differences = Vec::new();
    let names: Vec<&String> = runs.keys().collect();
    for (i, a) in names.iter().enumerate() {
        for b in &names[i + 1..] {
            let (va, da) = a.rsplit_once('@').unwrap_or((a, ""));
            let (vb, db) = b.rsplit_once('@').unwrap_or((b, ""));
            if da != db {
                continue;
            }
            let pa: BTreeMap<&str, &PageResult> =
                runs[*a].0.iter().map(|p| (p.id.as_str(), p)).collect();
            let paired: Vec<(String, f64, f64)> = runs[*b]
                .0
                .iter()
                .filter_map(|pb| {
                    let pa = pa.get(pb.id.as_str())?;
                    Some((
                        pb.category.clone(),
                        pa.score.as_ref()?.wer,
                        pb.score.as_ref()?.wer,
                    ))
                })
                .collect();
            let d: Difference = bootstrap(&paired, 10_000, seed);
            differences.push(json!({ "dpi": da, "a": va, "b": vb, "werAMinusB": d }));
        }
    }

    let out = json!({
        "pagesInCorpus": jobs.len(),
        "runs": runs.iter().map(|(k, (_, s))| (k.clone(), json!(s))).collect::<BTreeMap<_, _>>(),
        "textLayer": layer,
        "differences": differences,
        "pages": runs.iter().map(|(k, (p, _))| (k.clone(), json!(p))).collect::<BTreeMap<_, _>>(),
        "textLayerPages": layer_pages,
    });
    println!("{}", serde_json::to_string_pretty(&out).expect("JSON"));
}
