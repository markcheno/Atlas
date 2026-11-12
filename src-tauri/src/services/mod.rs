//! Application services. Each module owns one responsibility from the
//! Phase 0 spec, section 4. Services are transport-independent: Tauri
//! commands call them today, and a future versioned HTTP API can wrap the
//! same functions.

pub mod answers;
pub mod catalog;
pub mod documents;
pub mod embedding;
pub mod export;
pub mod extraction;
pub mod inventory;
pub mod jobs;
pub mod knowledge;
pub mod library;
pub mod providers;
pub mod reader;
pub mod scheduler;
pub mod search;
pub mod source_open;
pub mod sources;
pub mod storage;
pub mod system;
pub mod workspace;
