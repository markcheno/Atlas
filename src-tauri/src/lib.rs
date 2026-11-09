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
use std::sync::{Arc, Mutex};

use tauri::{Emitter, Manager};

use domain::dto::{ExtractionOverview, InventoryOverview, JobKind};
use infrastructure::keychain::LazyKeychain;
use infrastructure::local_source::LocalFsSource;
use infrastructure::system_probe::{OsResourceMonitor, SimulatedMonitor, Simulation};
use infrastructure::volume::OsVolumeInspector;
use services::extraction::job::{self as extraction_job, ExtractionConfig, ExtractionDeps};
use services::extraction::supervisor::WorkerCommand;
use services::inventory::{InventoryConfig, InventoryDeps};
use services::providers::secrets::{SecretStore, SecretVault};
use services::system::ResourceGovernor;
use services::workspace::{Workspace, WorkspaceConfig, layout};
use state::AppState;

/// Tauri event carrying an `InventoryOverview` whenever cataloging
/// progresses or changes state.
pub const INVENTORY_EVENT: &str = "inventory-status";

/// Tauri event carrying an `ExtractionOverview` whenever reading text makes
/// progress or changes state (at most four per second, plus every change).
pub const EXTRACTION_EVENT: &str = "extraction-status";

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

/// Creates the main window from its `tauri.conf.json` entry (which has
/// `"create": false`), after the app state exists.
fn main_window(app: &tauri::App) -> tauri::Result<()> {
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == "main")
        .cloned()
        .ok_or_else(|| tauri::Error::WindowNotFound)?;
    #[allow(unused_mut)]
    let mut builder = tauri::WebviewWindowBuilder::from_config(app.handle(), &config)?;
    #[cfg(windows)]
    if let Some(args) = infrastructure::e2e::webview2_driver_args() {
        builder = builder.additional_browser_args(&args);
    }
    builder.build()?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let config = workspace_config(app.handle())?;
            let volumes = Arc::new(OsVolumeInspector);
            // Debug builds may simulate power and free space for demos.
            let monitor = SimulatedMonitor {
                inner: OsResourceMonitor::new(volumes.clone()),
                simulation: Simulation::from_env(),
            };
            let governor = Arc::new(Mutex::new(ResourceGovernor::new(Box::new(monitor))));
            let emitter = app.handle().clone();
            let deps = InventoryDeps {
                governor: governor.clone(),
                source: Arc::new(LocalFsSource),
                events: Arc::new(move |overview: &InventoryOverview| {
                    let _ = emitter.emit(INVENTORY_EVENT, overview);
                }),
                config: InventoryConfig::from_env(),
            };
            let extraction_emitter = app.handle().clone();
            // The worker is this executable started with `--extract-worker`.
            let worker = WorkerCommand::current_exe()?;
            let extraction = ExtractionDeps::new(
                governor.clone(),
                Arc::new(move |overview: &ExtractionOverview| {
                    let _ = extraction_emitter.emit(EXTRACTION_EVENT, overview);
                }),
                ExtractionConfig::from_env(),
                worker,
            );
            let mut workspace = Workspace::new(config, volumes)
                .with_inventory(deps)
                .with_job_processor(JobKind::Extract, extraction_job::factory(extraction));
            // Reopens the remembered root and resumes interrupted cataloging;
            // problems become status, never a crash.
            workspace.startup();
            // Keys go to the OS keychain, opened on first use so startup
            // never waits for it. If it's unavailable, keys are kept for
            // this session only and never written to disk.
            let keychain: Arc<dyn SecretStore> = Arc::new(LazyKeychain::default());
            let vault = Arc::new(SecretVault::new(Some(keychain)));
            app.manage(AppState::new(
                workspace,
                governor,
                // Debug builds may record opens instead (smoke tests).
                infrastructure::e2e::opener(),
                vault,
            ));
            main_window(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app::get_app_status,
            commands::app::get_device_snapshot,
            commands::app::get_device_check,
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
            commands::extraction::start_extraction,
            commands::extraction::get_extraction_status,
            commands::search::search_documents,
            commands::documents::get_document_detail,
            commands::documents::list_duplicates,
            commands::library::list_documents,
            commands::library::open_original,
            commands::library::reveal_original,
            commands::providers::list_provider_profiles,
            commands::providers::save_provider_profile,
            commands::providers::delete_provider_profile,
            commands::providers::test_provider_connection,
            commands::providers::detect_local_providers,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Atlas")
        .run(|app, event| {
            // Stop the inventory worker cleanly; no scanner outlives the app.
            if let tauri::RunEvent::Exit = event
                && let Some(state) = app.try_state::<AppState>()
                && let Ok(mut workspace) = state.workspace.lock()
            {
                workspace.shutdown();
            }
        });
}
