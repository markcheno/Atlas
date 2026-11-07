//! SQLite catalog: connection setup (WAL, foreign keys, busy timeout), the
//! single dedicated writer task, read-only connection pool, versioned
//! migrations, and the built-in `CatalogRepository` adapter.
//!
//! Not implemented in Slice 0.1. Schema and migrations arrive in Slice 0.2;
//! source roots in 0.3; scan sessions and the directory frontier in 0.5.
