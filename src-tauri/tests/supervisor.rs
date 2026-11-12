//! Slice 1.3: the worker supervisor against the real `atlas --extract-worker`
//! (phase1-spec §7.2; the process side of P1-11). Faults are injected with
//! the worker's debug-only `ATLAS_WORKER_TEST_FAULT`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use atlas_lib::domain::dto::FileType;
use atlas_lib::services::extraction::engine::Outcome;
use atlas_lib::services::extraction::engine::hash::HashError;
use atlas_lib::services::extraction::locate::{LIBRARY_ENV, Unavailable};
use atlas_lib::services::extraction::supervisor::PageCounts;
use atlas_lib::services::extraction::supervisor::{
    Crash, DocumentProcessor, Extraction, Supervisor, SupervisorConfig, WorkerCommand, WorkerError,
    WorkerStats,
};
use atlas_lib::services::extraction::worker_main::{TEST_FAULT_ENV, TEST_PROTOCOL_ENV};

fn corpus(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../spikes/extraction-bench/corpus")
        .join(rel)
}

fn command() -> WorkerCommand {
    WorkerCommand::new(env!("CARGO_BIN_EXE_atlas"))
}

fn supervisor(command: WorkerCommand, config: SupervisorConfig) -> (Supervisor, Arc<WorkerStats>) {
    let stats = Arc::new(WorkerStats::default());
    let s = Supervisor::new(command, config, Arc::new(|| false), Arc::clone(&stats));
    (s, stats)
}

/// A copy of a corpus file under a name the fault hook can match.
fn copy_as(tmp: &Path, rel: &str, name: &str) -> PathBuf {
    let to = tmp.join(name);
    std::fs::copy(corpus(rel), &to).unwrap();
    to
}

#[test]
fn hashes_and_extracts_through_the_worker() {
    let (mut s, stats) = supervisor(command(), SupervisorConfig::default());
    let book = corpus("pdf/generated/book.pdf");
    let hashed = s.hash(&book).unwrap().unwrap();
    let bytes = std::fs::read(&book).unwrap();
    assert_eq!(hashed.hash, blake3::hash(&bytes).to_hex().to_string());
    assert_eq!(hashed.size, bytes.len() as u64);

    let Extraction::Done { summary, pieces } = s.extract(&book, FileType::Pdf).unwrap() else {
        panic!("book.pdf has text");
    };
    assert_eq!(summary.page_count, 44);
    assert_eq!(pieces.len(), 44);
    assert_eq!(pieces[0].location, "Page 1");
    assert_eq!(pieces[43].page_no, Some(44));
    assert!(summary.char_count > 100_000);

    let drm = s
        .extract(&corpus("malformed/drm.epub"), FileType::Epub)
        .unwrap();
    assert_eq!(
        drm,
        Extraction::Failed {
            outcome: Outcome::Encrypted,
            reason: "drm".into(),
            pages: None,
        }
    );
    let missing = s.hash(Path::new("/no/such/file.pdf")).unwrap();
    assert_eq!(missing, Err(HashError::Failed));
    // One worker served every request.
    assert_eq!(WorkerStats::get(&stats.spawns), 1);
}

#[test]
fn a_worker_that_exits_mid_document_is_replaced() {
    let tmp = tempfile::tempdir().unwrap();
    let bad = copy_as(tmp.path(), "pdf/generated/book.pdf", "crash-exit.pdf");
    let good = corpus("pdf/generated/one-column.pdf");
    let (mut s, stats) = supervisor(
        command().env(TEST_FAULT_ENV, "crash-exit:exit"),
        SupervisorConfig::default(),
    );
    assert_eq!(
        s.extract(&bad, FileType::Pdf),
        Err(WorkerError::Crashed(Crash::Exited))
    );
    assert!(matches!(
        s.extract(&good, FileType::Pdf),
        Ok(Extraction::Done { .. })
    ));
    assert_eq!(WorkerStats::get(&stats.spawns), 2);
}

#[test]
fn a_hung_worker_is_killed_after_the_inactivity_timeout() {
    let tmp = tempfile::tempdir().unwrap();
    let bad = copy_as(tmp.path(), "pdf/generated/book.pdf", "crash-hang.pdf");
    let config = SupervisorConfig {
        inactivity: Duration::from_secs(1),
        ..SupervisorConfig::default()
    };
    let (mut s, stats) = supervisor(command().env(TEST_FAULT_ENV, "crash-hang:hang"), config);
    let start = Instant::now();
    assert_eq!(
        s.extract(&bad, FileType::Pdf),
        Err(WorkerError::Crashed(Crash::Timeout))
    );
    assert!(start.elapsed() < Duration::from_secs(10));
    assert_eq!(WorkerStats::get(&stats.kills), 1);
}

#[test]
fn a_worker_over_the_memory_cap_is_killed() {
    let tmp = tempfile::tempdir().unwrap();
    let bad = copy_as(tmp.path(), "pdf/generated/book.pdf", "crash-bloat.pdf");
    let config = SupervisorConfig {
        rss_cap_bytes: 300 * 1024 * 1024,
        poll: Duration::from_millis(50),
        ..SupervisorConfig::default()
    };
    let (mut s, _) = supervisor(command().env(TEST_FAULT_ENV, "crash-bloat:bloat"), config);
    assert_eq!(
        s.extract(&bad, FileType::Pdf),
        Err(WorkerError::Crashed(Crash::Memory))
    );
}

#[test]
fn a_stop_request_ends_a_wait_and_kills_the_worker() {
    let tmp = tempfile::tempdir().unwrap();
    let bad = copy_as(tmp.path(), "pdf/generated/book.pdf", "crash-hang.pdf");
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let stats = Arc::new(WorkerStats::default());
    let mut s = Supervisor::new(
        command().env(TEST_FAULT_ENV, "crash-hang:hang"),
        SupervisorConfig::default(),
        Arc::new(move || flag.load(Ordering::SeqCst)),
        Arc::clone(&stats),
    );
    let setter = Arc::clone(&stop);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        setter.store(true, Ordering::SeqCst);
    });
    let start = Instant::now();
    assert_eq!(s.extract(&bad, FileType::Pdf), Err(WorkerError::Stopped));
    assert!(start.elapsed() < Duration::from_secs(5));
    assert_eq!(WorkerStats::get(&stats.kills), 1);
}

#[test]
fn a_missing_pdfium_is_reported_as_unavailable() {
    let (mut s, _) = supervisor(
        command().env(LIBRARY_ENV, "/no/such/libpdfium"),
        SupervisorConfig::default(),
    );
    assert_eq!(
        s.hash(&corpus("pdf/generated/book.pdf")),
        Err(WorkerError::Unavailable(Unavailable::NotFound))
    );
}

/// P4-04: a worker that speaks another protocol is a mismatched install.
/// It is refused at hello and not restarted: every request says so at once.
#[test]
fn p4_04_a_worker_with_another_protocol_is_refused() {
    let (mut s, stats) = supervisor(
        command().env(TEST_PROTOCOL_ENV, "1"),
        SupervisorConfig::default(),
    );
    let book = corpus("pdf/generated/book.pdf");
    for _ in 0..3 {
        assert_eq!(
            s.extract(&book, FileType::Pdf),
            Err(WorkerError::Unavailable(Unavailable::ProtocolMismatch))
        );
        assert!(!s.is_running());
    }
    assert_eq!(s.refused_protocol(), Some(1));
    assert_eq!(
        WorkerStats::get(&stats.extracts),
        0,
        "nothing was asked of it"
    );
}

/// P4-04: a PDF that needs OCR says how many pages it has, and how many
/// have usable text. The two scans have none; their pages are known.
#[test]
fn p4_04_scanned_pdfs_report_their_pages() {
    let (mut s, _) = supervisor(command(), SupervisorConfig::default());
    for (rel, page_count) in [
        ("pdf/scanned/ia-alice-1894.pdf", 4),
        ("pdf/scanned/synthetic-scan.pdf", 3),
    ] {
        let Extraction::Failed {
            outcome,
            reason,
            pages,
        } = s.extract(&corpus(rel), FileType::Pdf).unwrap()
        else {
            panic!("{rel} needs OCR");
        };
        assert_eq!(
            (outcome, reason.as_str()),
            (Outcome::NeedsOcr, "no_text_layer")
        );
        assert_eq!(
            pages,
            Some(PageCounts {
                page_count,
                text_pages: 0
            }),
            "{rel}"
        );
    }
    let Extraction::Failed { pages, reason, .. } = s
        .extract(&corpus("pdf/generated/no-tounicode.pdf"), FileType::Pdf)
        .unwrap()
    else {
        panic!("no-tounicode.pdf needs OCR");
    };
    assert_eq!(reason, "unreadable_text");
    assert_eq!(
        pages,
        Some(PageCounts {
            page_count: 3,
            text_pages: 0
        }),
        "its text is garbled on every page"
    );
}
