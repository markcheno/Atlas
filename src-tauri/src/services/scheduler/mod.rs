//! Durable single-worker scheduler for inventory scans: the job state
//! machine (Phase 0 spec, section 7.3), cooperative pause/resume/cancel,
//! power and storage policy with battery hysteresis, and rate limiting.
//!
//! Deliberately concrete: there is one job kind in Phase 0, so no
//! `JobProcessor` or `SchedulingPolicy` traits are defined.
//!
//! Not implemented in Slice 0.1. Delivered by Slice 0.5 (policy inputs
//! from Slice 0.4).
