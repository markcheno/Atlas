//! Same entry point as the `sqlite-vec` crate, from the SIMD build.

/// Which SIMD path this build enabled: "neon", "avx" or "none".
pub const SIMD: &str = env!("SQLITE_VEC_SIMD");

#[link(name = "sqlite_vec0")]
unsafe extern "C" {
    pub fn sqlite3_vec_init();
}
