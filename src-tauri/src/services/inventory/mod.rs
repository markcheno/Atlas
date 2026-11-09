//! Inventory: read-only traversal of library folders into the catalog
//! (phase0-spec §7, ADR 0005).
//!
//! Scans are the first job kind (ADR 0009): the scan runner of each open
//! data root walks one source root at a time with a [`scan::ScanProcessor`],
//! streaming each directory in bounded batches. Progress lives in the
//! catalog (`jobs` of kind `scan`, `scan_directories`), so a quit, crash, or
//! unplugged drive never loses completed work or duplicates rows. The
//! worker obeys [`crate::services::scheduler::policy::ResourcePolicy`] and
//! a token-bucket rate limit, and it never opens file contents.

pub mod classify;
pub mod control;
pub mod db;
pub mod rate;
pub mod scan;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::domain::dto::InventoryOverview;
use crate::domain::traits::InventorySource;
use crate::services::system::ResourceGovernor;

/// Debug builds only: lifts the metadata rate limit for benchmarking.
pub const UNTHROTTLED_ENV: &str = "ATLAS_INVENTORY_UNTHROTTLED";
/// Debug builds only: a fixed metadata rate (entries per second), so the
/// smoke test can pause cataloging before it finishes on a fast runner.
pub const RATE_ENV: &str = "ATLAS_INVENTORY_RATE";

#[derive(Debug, Clone)]
pub struct InventoryConfig {
    /// Entries written per transaction.
    pub batch_size: usize,
    /// No rate limit at all (tests and benchmarks).
    pub unthrottled: bool,
    /// Fixed operations per second instead of the policy's (tests).
    pub rate_override: Option<u32>,
    /// How often device and source-root conditions are re-checked.
    pub recheck_interval: Duration,
    /// How long to wait before re-checking while paused by policy.
    pub policy_wait: Duration,
    /// How long to sleep when nothing is scheduled (a wake cuts it short).
    pub idle_wait: Duration,
    /// Listing attempts per directory before it is given up.
    pub max_dir_attempts: u32,
    /// Minimum time between progress events.
    pub event_interval: Duration,
}

impl Default for InventoryConfig {
    fn default() -> Self {
        Self {
            batch_size: 100,
            unthrottled: false,
            rate_override: None,
            recheck_interval: Duration::from_secs(2),
            policy_wait: Duration::from_secs(5),
            idle_wait: Duration::from_secs(30),
            max_dir_attempts: 3,
            event_interval: Duration::from_millis(250),
        }
    }
}

impl InventoryConfig {
    pub fn from_env() -> Self {
        let unthrottled = cfg!(debug_assertions)
            && std::env::var(UNTHROTTLED_ENV).is_ok_and(|v| v == "1" || v == "true");
        let rate_override = cfg!(debug_assertions)
            .then(|| std::env::var(RATE_ENV).ok()?.parse::<u32>().ok())
            .flatten()
            .filter(|r| *r > 0);
        Self {
            unthrottled,
            rate_override,
            ..Self::default()
        }
    }
}

pub type InventoryEvents = Arc<dyn Fn(&InventoryOverview) + Send + Sync>;

#[derive(Clone)]
pub struct InventoryDeps {
    pub governor: Arc<Mutex<ResourceGovernor>>,
    pub source: Arc<dyn InventorySource>,
    pub events: InventoryEvents,
    pub config: InventoryConfig,
}
