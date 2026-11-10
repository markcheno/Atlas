//! Embeddings (Phase 2, ADR 0013): chunks of the extracted text, their
//! vectors kept in the catalog, and the usearch index derived from them.
//!
//! - `chunk`: passages cut into chunks that keep their page or chapter.
//! - `vector`: unit vectors and their f16 encoding.
//! - `db`: the catalog rows (migration 0012).
//! - `index`: the vector index, saved atomically and rebuilt when needed.
//! - `fake`: a deterministic backend for tests and benchmarks.
//!
//! The embedding job that fills these (Slice 2.3) and hybrid search
//! (Slice 2.4) build on this module.

pub mod chunk;
pub mod client;
pub mod db;
pub mod fake;
pub mod index;
pub mod job;
pub mod models;
pub mod registry;
pub mod status;
pub mod triggers;
pub mod vector;
