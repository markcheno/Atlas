//! What Atlas records about itself (Slice 4.1, phase4-spec §7.1, ADR 0021).
//!
//! An event is a code and numbers. Every field is an integer, a duration,
//! a boolean or a value of a closed enum; none can hold text made while
//! Atlas runs, so no path, title, search, question, answer or note can
//! reach the log. That is the privacy rule, kept by the type. The one
//! exception, a panic's source file, is a [`SourceFile`], which can only
//! be made from a location compiled into Atlas.

use serde::{Deserialize, Serialize};

use crate::domain::dto::{
    AskEnding, JobKind, JobState, KnowledgeState, PauseReason, PolicyMode, SemanticStatus,
    SnapshotOutcome, ThrottleReason,
};
use crate::error::ErrorCode;
use crate::services::extraction::supervisor::Crash;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case", deny_unknown_fields)]
pub enum Event {
    /// The first library this run of Atlas opened.
    AppStarted {
        major: u32,
        minor: u32,
        patch: u32,
    },
    LibraryOpened {
        catalog_schema: u32,
        /// The schema the catalog had, when this open migrated it.
        migrated_from: Option<u32>,
        knowledge: KnowledgeState,
        created: bool,
    },
    LibraryClosed {
        open_secs: u64,
    },
    JobStarted {
        kind: JobKind,
    },
    JobPaused {
        kind: JobKind,
        by: PauseBy,
        reason: Option<PauseReason>,
    },
    JobResumed {
        kind: JobKind,
        by: PauseBy,
    },
    /// A job's run ended: finished, paused, failed or stopped by closing.
    /// `items` is files found (scan), files read (read) or chunks embedded
    /// (embed).
    JobEnded {
        kind: JobKind,
        state: Option<JobState>,
        ms: u64,
        items: u64,
        errors: u64,
        /// Why a job that failed did (Slice 4.1.1): without it the log
        /// showed that indexing meaning stopped and not what stopped it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<FailureReason>,
    },
    WorkerStarted {
        role: WorkerRole,
    },
    WorkerCrashed {
        role: WorkerRole,
        crash: Crash,
    },
    /// A worker of another protocol: a mismatched install (Slice 4.1).
    WorkerRefused {
        role: WorkerRole,
        protocol: u32,
    },
    PolicyChanged {
        mode: PolicyMode,
        pause: Option<PauseReason>,
        throttle: Option<ThrottleReason>,
    },
    Snapshot {
        outcome: SnapshotOutcome,
        ms: u64,
    },
    /// An error a command returned to the window.
    ErrorReturned {
        code: ErrorCode,
    },
    Search {
        ms: u64,
        hits: u32,
        matched: u32,
        meaning: MeaningUse,
    },
    AskEnded {
        ms: u64,
        sources: u32,
        cited: u32,
        dropped: u32,
        ending: AskEnding,
    },
    /// The meaning index was loaded, or let go (P3-18).
    MeaningIndex {
        action: IndexAction,
        ms: u64,
        vectors: u64,
    },
    Panic {
        file: SourceFile,
        line: u32,
        column: u32,
    },
    /// Events lost because the writer fell behind.
    EventsDropped {
        count: u64,
    },
    ReportSaved {
        bytes: u64,
    },
    LogCleared,
}

/// Why a job failed: the codes jobs store as `last_error`. A code this
/// list doesn't know is `other`, so text can never ride in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureReason {
    ProviderUnreachable,
    ProviderAuth,
    ModelNotFound,
    ModelChanged,
    AddressRejected,
    IndexUnavailable,
    ExtractorUnavailable,
    WorkerProtocol,
    WorkerRestarts,
    NestedRoot,
    PathUndecodable,
    Database,
    Other,
}

impl FailureReason {
    pub fn of_code(code: &str) -> Self {
        match code {
            "provider_unreachable" => Self::ProviderUnreachable,
            "provider_auth" => Self::ProviderAuth,
            "model_not_found" => Self::ModelNotFound,
            "model_changed" => Self::ModelChanged,
            "address_rejected" => Self::AddressRejected,
            "index_unavailable" => Self::IndexUnavailable,
            "extractor_unavailable" => Self::ExtractorUnavailable,
            "worker_protocol" => Self::WorkerProtocol,
            "worker_restarts" => Self::WorkerRestarts,
            "nested_root" => Self::NestedRoot,
            "path_undecodable" => Self::PathUndecodable,
            "database" => Self::Database,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseBy {
    User,
    Policy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerRole {
    /// The extraction job's worker.
    Read,
    /// The page view's worker.
    Render,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexAction {
    Loaded,
    Dropped,
}

/// How a search used the meaning index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeaningUse {
    Off,
    Used,
    Unavailable,
}

impl From<&SemanticStatus> for MeaningUse {
    fn from(s: &SemanticStatus) -> Self {
        match s {
            SemanticStatus::Off => Self::Off,
            SemanticStatus::Ready { .. } => Self::Used,
            SemanticStatus::Unavailable { .. } => Self::Unavailable,
        }
    }
}

/// The source file of a panic, trimmed so a build machine's paths never
/// reach the log: Atlas's own files as `src/…`, anything else as
/// `dependency`. Made only from a location compiled into Atlas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SourceFile(String);

impl SourceFile {
    pub fn of_location(location: &std::panic::Location<'_>) -> Self {
        Self::trim(location.file())
    }

    fn trim(file: &str) -> Self {
        let file = file.replace('\\', "/");
        let own =
            !file.contains("/registry/") && !file.contains("/rustc/") && !file.starts_with('/');
        let own = own || file.starts_with(&env!("CARGO_MANIFEST_DIR").replace('\\', "/"));
        match file.rfind("src/") {
            Some(at) if own => Self(file[at..].to_owned()),
            _ => Self("dependency".to_owned()),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_files_never_carry_a_build_machine_path() {
        assert_eq!(
            SourceFile::trim("src/services/jobs/runner.rs").as_str(),
            "src/services/jobs/runner.rs"
        );
        let own = concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs");
        assert_eq!(SourceFile::trim(own).as_str(), "src/lib.rs");
        assert_eq!(
            SourceFile::trim("/home/someone/.cargo/registry/src/index/serde-1.0/src/de.rs")
                .as_str(),
            "dependency"
        );
        assert_eq!(
            SourceFile::trim("/rustc/abc/library/core/src/option.rs").as_str(),
            "dependency"
        );
        assert_eq!(
            SourceFile::of_location(std::panic::Location::caller()).as_str(),
            "src/services/diagnostics/event.rs"
        );
    }

    #[test]
    fn events_are_tagged_lines_of_codes_and_numbers() {
        let e = Event::JobEnded {
            kind: JobKind::Extract,
            state: Some(JobState::Completed),
            ms: 12,
            items: 3,
            errors: 0,
            reason: None,
        };
        let line = serde_json::to_string(&e).unwrap();
        assert_eq!(
            line,
            r#"{"event":"job_ended","kind":"extract","state":"completed","ms":12,"items":3,"errors":0}"#
        );
        assert_eq!(serde_json::from_str::<Event>(&line).unwrap(), e);
        assert!(
            serde_json::from_str::<Event>(r#"{"event":"job_started","kind":"scan","note":"x"}"#)
                .is_err()
        );
    }

    #[test]
    fn a_failed_job_says_why_in_a_code() {
        let e = Event::JobEnded {
            kind: JobKind::Embed,
            state: Some(JobState::FailedTerminal),
            ms: 5,
            items: 0,
            errors: 1,
            reason: Some(FailureReason::of_code("model_not_found")),
        };
        assert_eq!(
            serde_json::to_string(&e).unwrap(),
            r#"{"event":"job_ended","kind":"embed","state":"failed_terminal","ms":5,"items":0,"errors":1,"reason":"model_not_found"}"#
        );
        // Whatever a job stored, only a known code is written.
        assert_eq!(
            FailureReason::of_code("/Users/someone/Books"),
            FailureReason::Other
        );
    }
}
