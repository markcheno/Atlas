//! Atlas application core.
//!
//! Single-process design: the Tauri runtime hosts the UI and these Rust
//! services in one executable. Tauri IPC is the only API; there is no HTTP
//! server, daemon, or database server. See `docs/architecture.md` and
//! `docs/adr/0001-local-ipc-no-server-single-data-root.md`.

pub mod commands;
pub mod domain;
pub mod error;
pub mod infrastructure;
pub mod services;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::app::get_app_status,
            commands::app::get_device_snapshot,
            commands::workspace::choose_data_root,
            commands::workspace::open_data_root,
            commands::sources::list_source_roots,
            commands::sources::add_source_root,
            commands::sources::remove_source_root,
            commands::inventory::start_inventory,
            commands::inventory::pause_job,
            commands::inventory::resume_job,
            commands::inventory::cancel_job,
            commands::inventory::get_inventory_status,
            commands::library::list_documents,
            commands::library::open_original,
            commands::library::reveal_original,
            commands::providers::list_provider_profiles,
            commands::providers::save_provider_profile,
            commands::providers::test_provider_connection,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Atlas");
}
