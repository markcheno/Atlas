//! The vec-sqlite engine, built against sqlite-vec 0.1.10-alpha.4 (DiskANN;
//! informational only: an alpha can't be adopted).
const BUILD: &str = "alpha";

mod sqlite_vec {
    pub const SIMD: &str = env!("SQLITE_VEC_SIMD");
    #[link(name = "sqlite_vec0")]
    unsafe extern "C" {
        pub fn sqlite3_vec_init();
    }
}

fn simd_label() -> &'static str {
    sqlite_vec::SIMD
}

include!("../../vec-sqlite/src/engine.rs");
