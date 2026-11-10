//! How background work is paced: the resource policy every job kind obeys
//! ([`policy::ResourcePolicy`], ADR 0004) and the duty cycle of the jobs
//! that work in steps ([`duty`]). The job runners and state machine live
//! in `services::jobs` (ADR 0009).

pub mod duty;
pub mod policy;
