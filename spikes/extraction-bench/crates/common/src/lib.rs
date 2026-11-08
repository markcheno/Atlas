//! Shared code for the extraction spike.
//!
//! **Contract.** Every candidate binary takes files on the command line and
//! writes one JSON line per file (to the file named by `XBENCH_OUT`, else to
//! stdout; some libraries print to stdout themselves):
//! `{"file", "outcome", "pages": [text…], "ms", "error"?, "meta"?}` and, at
//! the end, `{"done": true, "peakRssBytes": …}`. `outcome` is one of `ok`,
//! `encrypted`, `malformed`, `unsupported` or `panic`; a crash or timeout is
//! recorded by the runner, which runs each file in its own process for the
//! robustness pass.
//!
//! **Metrics** (used by the `score` binary):
//! - word F1 over normalized tokens (NFKC, lower case, CJK characters as
//!   single tokens, line-end hyphenation joined);
//! - reading order: of the tokens both texts share, the fraction that also
//!   appear in the same order (longest common subsequence via Myers diff);
//! - character problems: unexpanded ligatures, U+FFFD, private-use code
//!   points.

use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::time::Instant;

use serde_json::{Value, json};
use unicode_normalization::UnicodeNormalization;

// ---------------------------------------------------------------- contract

pub struct Extracted {
    pub pages: Vec<String>,
    pub meta: Value,
}

#[derive(Debug)]
pub enum Failure {
    Encrypted(String),
    Malformed(String),
    Unsupported(String),
}

pub type Extract = fn(&Path) -> Result<Extracted, Failure>;

/// Runs `extract` over every file argument, one JSON line each.
pub fn main_with(name: &str, version: &str, extract: Extract) {
    // Keep panic messages out of stdout; they go into the JSON instead.
    std::panic::set_hook(Box::new(|_| {}));
    let files: Vec<String> = std::env::args().skip(1).collect();
    let mut out: Box<dyn Write> = match std::env::var_os("XBENCH_OUT") {
        Some(p) => Box::new(std::fs::File::create(p).expect("XBENCH_OUT")),
        None => Box::new(std::io::stdout()),
    };
    for file in &files {
        let started = Instant::now();
        let result = catch_unwind(AssertUnwindSafe(|| extract(Path::new(file))));
        let ms = started.elapsed().as_secs_f64() * 1e3;
        let line = match result {
            Ok(Ok(x)) => {
                json!({ "file": file, "outcome": "ok", "pages": x.pages, "meta": x.meta, "ms": ms })
            }
            Ok(Err(Failure::Encrypted(e))) => {
                json!({ "file": file, "outcome": "encrypted", "error": e, "ms": ms })
            }
            Ok(Err(Failure::Malformed(e))) => {
                json!({ "file": file, "outcome": "malformed", "error": e, "ms": ms })
            }
            Ok(Err(Failure::Unsupported(e))) => {
                json!({ "file": file, "outcome": "unsupported", "error": e, "ms": ms })
            }
            Err(p) => {
                let msg = p
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "panic".into());
                json!({ "file": file, "outcome": "panic", "error": msg, "ms": ms })
            }
        };
        writeln!(out, "{line}").expect("write result");
        out.flush().expect("flush result");
    }
    let done = json!({ "done": true, "extractor": name, "version": version, "peakRssBytes": peak_rss_bytes(), "appLibraries": app_libraries() });
    writeln!(out, "{done}").expect("write result");
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
    let nfkc: String = "ﬁ".nfkc().collect();
    json!({ "sqlite": sqlite, "tokio": tokio, "unicodeNormalization": nfkc == "fi" })
}

/// Peak resident memory of this process, from the OS (exact, not sampled).
pub fn peak_rss_bytes() -> u64 {
    #[cfg(unix)]
    unsafe {
        let mut u: libc::rusage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_SELF, &mut u);
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

// ---------------------------------------------------------------- metrics

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF   // Hiragana, Katakana
        | 0x3400..=0x4DBF // CJK Extension A
        | 0x4E00..=0x9FFF // CJK Unified Ideographs
        | 0xAC00..=0xD7AF // Hangul syllables
        | 0xF900..=0xFAFF // CJK Compatibility Ideographs
        | 0x20000..=0x2FFFF)
}

/// Joins words broken across lines with a hyphen ("exam-\nple").
pub fn dehyphenate(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '-' && i > 0 && chars[i - 1].is_alphabetic() {
            let mut j = i + 1;
            while j < chars.len() && (chars[j] == ' ' || chars[j] == '\t' || chars[j] == '\r') {
                j += 1;
            }
            if j < chars.len() && chars[j] == '\n' {
                let mut k = j + 1;
                while k < chars.len() && chars[k].is_whitespace() {
                    k += 1;
                }
                if k < chars.len() && chars[k].is_lowercase() {
                    i = k;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Normalized word tokens: NFKC, lower case, alphanumeric runs, each CJK
/// character on its own.
pub fn tokens(text: &str) -> Vec<String> {
    let norm: String = dehyphenate(text).nfkc().collect::<String>().to_lowercase();
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in norm.chars() {
        if is_cjk(c) {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            out.push(c.to_string());
        } else if c.is_alphanumeric() {
            cur.push(c);
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Precision, recall and F1 of the candidate's tokens against the truth,
/// as multisets.
pub fn word_f1(candidate: &[String], truth: &[String]) -> (f64, f64, f64, usize) {
    use std::collections::HashMap;
    let mut counts: HashMap<&str, i64> = HashMap::new();
    for t in truth {
        *counts.entry(t).or_default() += 1;
    }
    let mut common = 0usize;
    for c in candidate {
        if let Some(n) = counts.get_mut(c.as_str())
            && *n > 0
        {
            *n -= 1;
            common += 1;
        }
    }
    if candidate.is_empty() || truth.is_empty() || common == 0 {
        return (0.0, 0.0, 0.0, common);
    }
    let p = common as f64 / candidate.len() as f64;
    let r = common as f64 / truth.len() as f64;
    (p, r, 2.0 * p * r / (p + r), common)
}

/// Of the tokens the two texts share, the fraction in the same order:
/// length of the longest common subsequence ÷ shared tokens. 1.0 means
/// everything found is in reading order; 0.0 when nothing is shared.
pub fn reading_order(candidate: &[String], truth: &[String], common: usize) -> f64 {
    if common == 0 {
        return 0.0;
    }
    let lcs = lcs_len(candidate, truth);
    (lcs as f64 / common as f64).min(1.0)
}

/// Longest common subsequence length via Myers diff, with a time budget;
/// past the budget the diff falls back to a coarser (shorter) match, which
/// only makes the score stricter.
pub fn lcs_len(a: &[String], b: &[String]) -> usize {
    use similar::{Algorithm, DiffOp, capture_diff_slices_deadline};
    let deadline = Instant::now() + std::time::Duration::from_secs(20);
    capture_diff_slices_deadline(Algorithm::Myers, a, b, Some(deadline))
        .iter()
        .map(|op| match op {
            DiffOp::Equal { len, .. } => *len,
            _ => 0,
        })
        .sum()
}

/// Characters that show an extraction problem.
pub fn char_problems(text: &str) -> Value {
    let mut ligatures = 0;
    let mut replacement = 0;
    let mut private_use = 0;
    let mut control = 0;
    for c in text.chars() {
        match c as u32 {
            0xFB00..=0xFB06 => ligatures += 1,
            0xFFFD => replacement += 1,
            0xE000..=0xF8FF | 0xF0000..=0x10FFFF => private_use += 1,
            _ if c.is_control() && !c.is_whitespace() => control += 1,
            _ => {}
        }
    }
    json!({ "ligatures": ligatures, "replacement": replacement, "privateUse": private_use, "control": control })
}

/// Text between Project Gutenberg's START and END markers, if present.
pub fn gutenberg_body(text: &str) -> &str {
    let start = text
        .find("*** START OF")
        .and_then(|i| text[i..].find('\n').map(|j| i + j + 1))
        .unwrap_or(0);
    let end = text[start..]
        .find("*** END OF")
        .map(|i| start + i)
        .unwrap_or(text.len());
    &text[start..end]
}

/// Plain-text editions mark italics with underscores; drop them.
pub fn strip_underscores(text: &str) -> String {
    text.replace('_', " ")
}

/// Non-whitespace characters on a page.
pub fn page_chars(page: &str) -> usize {
    page.chars().filter(|c| !c.is_whitespace()).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Vec<String> {
        tokens(s)
    }

    #[test]
    fn identical_text_scores_one() {
        let a = t("The quick brown fox jumps over the lazy dog.");
        let (p, r, f, common) = word_f1(&a, &a);
        assert_eq!((p, r, f), (1.0, 1.0, 1.0));
        assert_eq!(reading_order(&a, &a, common), 1.0);
    }

    #[test]
    fn swapped_columns_keep_f1_but_lose_order() {
        let truth = t("alpha beta gamma delta epsilon zeta eta theta iota kappa");
        let swapped = t("zeta eta theta iota kappa alpha beta gamma delta epsilon");
        let (_, _, f, common) = word_f1(&swapped, &truth);
        assert_eq!(f, 1.0);
        let order = reading_order(&swapped, &truth, common);
        assert!(order <= 0.5, "order {order}");
    }

    #[test]
    fn empty_output_scores_zero() {
        let truth = t("some words");
        let (_, _, f, common) = word_f1(&[], &truth);
        assert_eq!(f, 0.0);
        assert_eq!(reading_order(&[], &truth, common), 0.0);
    }

    #[test]
    fn ligatures_accents_and_case_normalize() {
        assert_eq!(t("Ofﬁce CAFÉ"), t("office café"));
        assert_eq!(t("Cafe\u{301}"), t("café"));
    }

    #[test]
    fn line_end_hyphenation_is_joined() {
        assert_eq!(t("an exam-\nple of it"), t("an example of it"));
        // A real hyphen before a capital stays two words.
        assert_eq!(t("North-\nWest"), vec!["north", "west"]);
    }

    #[test]
    fn cjk_characters_are_tokens() {
        assert_eq!(
            t("道可道，非常道。"),
            vec!["道", "可", "道", "非", "常", "道"]
        );
    }

    #[test]
    fn gutenberg_markers_are_stripped() {
        let txt = "Header\n*** START OF THE PROJECT GUTENBERG EBOOK X ***\nBody text\n*** END OF THE PROJECT GUTENBERG EBOOK X ***\nLicense";
        assert_eq!(gutenberg_body(txt).trim(), "Body text");
        assert_eq!(gutenberg_body("no markers"), "no markers");
    }

    #[test]
    fn character_problems_are_counted() {
        let v = char_problems("ﬁne \u{FFFD} \u{E001}");
        assert_eq!(v["ligatures"], 1);
        assert_eq!(v["replacement"], 1);
        assert_eq!(v["privateUse"], 1);
    }
}
