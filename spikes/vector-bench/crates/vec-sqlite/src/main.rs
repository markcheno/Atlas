//! The vec-sqlite engine with the published sqlite-vec 0.1.9 crate, as built by default.
const BUILD: &str = "crate";
fn simd_label() -> &'static str {
    "none (the published crate enables no SIMD)"
}
include!("engine.rs");
