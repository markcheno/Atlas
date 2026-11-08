//! Scores extraction results against the corpus truth.
//!
//!   score --corpus <dir> --results <results.jsonl>
//!
//! Each input line: `{"candidate", "id", "outcome", "pages": [..]}` plus
//! anything else, which is passed through. Each output line adds the
//! metrics for that document (see xbench's docs for definitions).

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use serde_json::{Map, Value, json};
use xbench::{char_problems, gutenberg_body, page_chars, reading_order, strip_underscores, tokens, word_f1};

/// A page with fewer visible characters than this counts as empty.
const EMPTY_PAGE_CHARS: usize = 20;

fn main() {
    if let Err(e) = run() {
        eprintln!("score: {e}");
        std::process::exit(1);
    }
}

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == &format!("--{name}")).and_then(|i| args.get(i + 1)).cloned()
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let corpus = PathBuf::from(arg("corpus").ok_or("--corpus")?);
    let results = PathBuf::from(arg("results").ok_or("--results")?);
    let manifest: Value = serde_json::from_slice(&fs::read(corpus.join("manifest.json"))?)?;
    let docs: Map<String, Value> = manifest["documents"]
        .as_array()
        .ok_or("manifest documents")?
        .iter()
        .map(|d| (d["id"].as_str().unwrap_or_default().to_string(), d.clone()))
        .collect();

    for line in BufReader::new(fs::File::open(&results)?).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let mut r: Value = serde_json::from_str(&line)?;
        let id = r["id"].as_str().unwrap_or_default().to_string();
        let doc = docs.get(&id).ok_or_else(|| format!("unknown document {id}"))?;
        let scores = score(&corpus, doc, &r)?;
        let pages = r.as_object_mut().and_then(|o| o.remove("pages"));
        let obj = r.as_object_mut().ok_or("result is not an object")?;
        obj.insert("pageCount".into(), json!(pages.as_ref().and_then(|p| p.as_array()).map(|p| p.len())));
        obj.insert("scores".into(), scores);
        println!("{r}");
    }
    Ok(())
}

fn score(corpus: &std::path::Path, doc: &Value, r: &Value) -> Result<Value, Box<dyn std::error::Error>> {
    let expect = doc["expect"].as_str().unwrap_or("text");
    let outcome = r["outcome"].as_str().unwrap_or("missing");
    let pages: Vec<String> = r["pages"]
        .as_array()
        .map(|p| p.iter().map(|x| x.as_str().unwrap_or_default().to_string()).collect())
        .unwrap_or_default();
    let empty = pages.iter().filter(|p| page_chars(p) < EMPTY_PAGE_CHARS).count();
    // Garbled: glyph ids instead of text (fonts without a usable ToUnicode
    // map come out as control or private-use characters).
    let visible: usize = pages.iter().map(|p| page_chars(p)).sum();
    let bad: usize = pages
        .iter()
        .flat_map(|p| p.chars())
        .filter(|c| (c.is_control() && !c.is_whitespace()) || matches!(*c as u32, 0xE000..=0xF8FF | 0xFFFD))
        .count();
    let garbled = visible > 0 && bad * 10 >= visible;
    let needs_ocr =
        outcome == "ok" && !pages.is_empty() && (empty * 2 >= pages.len() || garbled) && doc["kind"] != "epub";
    let robust = !matches!(outcome, "panic" | "crash" | "timeout" | "missing");

    let mut s = json!({
        "expect": expect,
        "robust": robust,
        "emptyPages": empty,
        "garbled": garbled,
        "needsOcr": needs_ocr,
    });
    match expect {
        "needs-ocr" => {
            s["correct"] = json!(needs_ocr);
        }
        "encrypted" => {
            s["correct"] = json!(outcome == "encrypted");
        }
        "no-crash" => {
            s["correct"] = json!(robust);
        }
        _ => {
            s["correct"] = json!(outcome == "ok");
            s["ocrFalsePositive"] = json!(needs_ocr);
            if let Some(truth) = doc["truth"]["path"].as_str() {
                let truth_text = fs::read_to_string(corpus.join(truth))?;
                let gutenberg = doc["truth"]["kind"] == "gutenberg";
                let truth_text = if gutenberg { strip_underscores(gutenberg_body(&truth_text)) } else { truth_text };
                let joined = pages.join("\n");
                let candidate = if gutenberg { gutenberg_body(&joined).to_string() } else { joined.clone() };
                let t = tokens(&truth_text);
                let c = tokens(&candidate);
                let (p, rec, f1, common) = word_f1(&c, &t);
                s["truthKind"] = doc["truth"]["kind"].clone();
                s["truthTokens"] = json!(t.len());
                s["tokens"] = json!(c.len());
                s["precision"] = json!(p);
                s["recall"] = json!(rec);
                s["f1"] = json!(f1);
                // Order is only meaningful when a fair share was found.
                s["order"] = if rec >= 0.05 { json!(reading_order(&c, &t, common)) } else { Value::Null };
                s["chars"] = char_problems(&joined);
            }
        }
    }
    Ok(s)
}
