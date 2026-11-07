//! Concrete OS, database and HTTP adapters that implement the traits in
//! `domain::traits`, plus low-level filesystem and clock helpers.

pub mod clock;
pub mod fsutil;
pub mod native_path;
pub mod system_probe;
pub mod volume;
