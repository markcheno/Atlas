//! Text extraction (phase1-spec §7.2, ADR 0008, ADR 0009).
//!
//! PDFium never runs in the main process. The Atlas executable re-run with
//! [`WORKER_ARG`] is the extraction worker: `main.rs` checks for it before
//! anything else, so no window, plugin or workspace starts. Slice 1.2 ships
//! the worker skeleton: it finds and loads the bundled PDFium, proves it
//! works on a built-in PDF, and says `hello`. Requests arrive in Slice 1.3.

pub mod engine;
pub mod locate;
pub mod protocol;
pub mod worker_main;

/// The hidden argument that runs the executable as the extraction worker.
pub const WORKER_ARG: &str = "--extract-worker";

/// The PDFium build the app ships (`tools.lock.json`; a test keeps them in
/// sync). `pdfium-render`'s `pdfium_8125` feature must match it.
pub const PDFIUM_VERSION: &str = "chromium/8125";

/// Version of the JSON-lines protocol between the app and the worker.
pub const PROTOCOL_VERSION: u32 = 1;
