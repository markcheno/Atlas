// Builds the vendored sqlite-vec 0.1.10-alpha.4 C sources (sqlite-vec.c
// includes the DiskANN, IVF and rescore files) the way the published crate does
// (SQLITE_CORE, static), plus the SIMD paths the published crate leaves
// off. sqlite-vec's AVX code uses GCC/Clang attributes, so MSVC builds
// stay scalar.
fn main() {
    let include = std::env::var("DEP_SQLITE3_INCLUDE").expect("libsqlite3-sys include dir");
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let mut b = cc::Build::new();
    b.file("csrc/sqlite-vec.c")
        .include("csrc")
        .include(include)
        .define("SQLITE_CORE", None);
    let simd = match (arch.as_str(), env.as_str()) {
        ("aarch64", e) if e != "msvc" => {
            b.define("SQLITE_VEC_ENABLE_NEON", None);
            "neon"
        }
        ("x86_64", e) if e != "msvc" => {
            // The alpha's AVX path also uses AVX2 (Hamming distance).
            b.define("SQLITE_VEC_ENABLE_AVX", None).flag("-mavx").flag("-mavx2");
            "avx"
        }
        _ => "none",
    };
    println!("cargo:rustc-env=SQLITE_VEC_SIMD={simd}");
    println!("cargo:rerun-if-changed=csrc/sqlite-vec.c");
    b.compile("sqlite_vec0");
}
