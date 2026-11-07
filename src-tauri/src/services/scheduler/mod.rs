//! Durable single-worker scheduler for inventory scans: the job state
//! machine (Phase 0 spec, section 7.3), cooperative pause/resume/cancel,
//! and rate limiting driven by [`policy::ResourcePolicy`].
//!
//! Deliberately concrete: there is one job kind in Phase 0, so no
//! `JobProcessor` or `SchedulingPolicy` traits are defined.
//!
//! Slice 0.4 ships the resource policy; the job runner arrives in 0.5.

pub mod policy;
