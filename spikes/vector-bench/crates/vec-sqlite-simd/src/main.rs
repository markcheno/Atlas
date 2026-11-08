//! The vec-sqlite engine, built against sqlite-vec 0.1.9 compiled with SIMD (crates/sqlite-vec-simd).
const BUILD: &str = "simd";
fn simd_label() -> &'static str {
    sqlite_vec::SIMD
}
include!("../../vec-sqlite/src/engine.rs");
