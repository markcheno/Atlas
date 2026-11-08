// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // The extraction worker is this executable re-run with a hidden argument
    // (phase1-spec §7.2). It is checked first, so the worker starts no
    // window, plugin or workspace.
    if std::env::args_os()
        .nth(1)
        .is_some_and(|a| a == atlas_lib::services::extraction::WORKER_ARG)
    {
        std::process::exit(atlas_lib::services::extraction::worker_main::run());
    }
    atlas_lib::run();
}
