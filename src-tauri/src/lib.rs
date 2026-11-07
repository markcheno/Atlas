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
pub mod state;

use std::path::PathBuf;
use std::sync::Arc;

use tauri::Manager;

use infrastructure::volume::OsVolumeInspector;
use services::workspace::{Workspace, WorkspaceConfig, layout};
use state::AppState;

/// Debug builds only: redirects the locator (and the suggested data root)
/// so tests and smoke runs never touch the real per-user configuration.
pub const CONFIG_DIR_OVERRIDE_ENV: &str = "ATLAS_CONFIG_DIR";

fn workspace_config(app: &tauri::AppHandle) -> tauri::Result<WorkspaceConfig> {
    if cfg!(debug_assertions)
        && let Some(dir) = std::env::var_os(CONFIG_DIR_OVERRIDE_ENV).filter(|d| !d.is_empty())
    {
        let dir = PathBuf::from(dir);
        return Ok(WorkspaceConfig {
            default_root: dir.join("Library"),
            config_dir: dir,
        });
    }
    Ok(WorkspaceConfig {
        config_dir: app.path().app_config_dir()?,
        default_root: layout::default_data_root(&app.path().local_data_dir()?),
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let config = workspace_config(app.handle())?;
            let mut workspace = Workspace::new(config, Arc::new(OsVolumeInspector));
            // Reopens the remembered root; problems become status, never a crash.
            workspace.startup();
            app.manage(AppState::new(workspace));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app::get_app_status,
            commands::app::get_device_snapshot,
            commands::workspace::get_data_root_status,
            commands::workspace::get_default_data_root,
            commands::workspace::pick_folder,
            commands::workspace::inspect_data_root,
            commands::workspace::choose_data_root,
            commands::workspace::open_data_root,
            commands::settings::get_settings,
            commands::settings::update_settings,
            commands::sources::list_source_roots,
            commands::sources::add_source_root,
            commands::sources::remove_source_root,
            commands::sources::pick_and_add_source_roots,
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
