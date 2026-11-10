//! Background jobs of every kind (phase1-spec §7.1, ADR 0009).
//!
//! A job is a row in `jobs` with a `kind`, a state from the Phase 0 state
//! machine, and its source root. Each open data root runs one
//! [`runner::JobRunner`] thread per kind, which picks that kind's next
//! runnable job and hands it to the kind's [`JobProcessor`]. Scans are the
//! first kind ([`crate::services::inventory::scan::ScanProcessor`]); text
//! extraction is the second (Phase 1, Slice 1.3).

pub mod control;
pub mod db;
pub mod runner;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub use db::JobRow;
pub use runner::{JobRunner, JobRunners, JobWaker, Signal};

use crate::services::catalog::writer::CatalogHandle;
use crate::services::embedding::registry::VectorRegistry;

/// Whether a runner keeps going after a job returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Stop,
}

/// Runs jobs of one kind. Built per open data root, on its runner's thread.
pub trait JobProcessor: Send {
    /// How long the runner sleeps when no job of this kind is runnable (a
    /// wake cuts it short).
    fn idle_wait(&self) -> Duration;
    /// Called once when the runner starts, before the first job. Scans send
    /// their first status event here, as Phase 0's worker did.
    fn started(&mut self) {}
    /// Runs one job until it finishes, is paused or cancelled, or the root
    /// closes ([`Flow::Stop`]). The job's state in the catalog is the source
    /// of truth: a processor re-reads it rather than trusting `job.state`.
    fn run(&mut self, job: JobRow) -> Flow;
}

/// What a processor gets from the open data root.
pub struct ProcessorContext {
    pub catalog: CatalogHandle,
    pub data_root: PathBuf,
    /// Shared with the runner: stop requests and early wake-ups.
    pub signal: Arc<Signal>,
    /// Wakes every runner of this data root (other kinds included).
    pub waker: JobWaker,
    /// The library's vector indexes (Slice 2.3).
    pub vectors: Arc<VectorRegistry>,
}

/// Builds a kind's processor for each data root the workspace opens.
pub type ProcessorFactory = Arc<dyn Fn(ProcessorContext) -> Box<dyn JobProcessor> + Send + Sync>;
