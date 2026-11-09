//! Thin Tauri IPC adapters. Each command validates input, calls a service,
//! and maps the result to DTOs. No business logic lives here.
//!
//! Every command of the Phase 0 contract (spec section 9) is registered from
//! Slice 0.1 so the frontend facade is complete. Commands whose slice has
//! not shipped return `AtlasError::NotImplemented` with the planned slice,
//! never placeholder data.

pub mod app;
pub mod extraction;
pub mod inventory;
pub mod library;
pub mod providers;
pub mod settings;
pub mod sources;
pub mod workspace;

/// Names of every registered command, in registration order. Kept in sync
/// with `invoke_handler` in `lib.rs` and the frontend facade by tests.
pub const COMMAND_NAMES: &[&str] = &[
    "get_app_status",
    "get_device_snapshot",
    "get_device_check",
    "get_data_root_status",
    "get_default_data_root",
    "pick_folder",
    "inspect_data_root",
    "choose_data_root",
    "open_data_root",
    "get_settings",
    "update_settings",
    "list_source_roots",
    "add_source_root",
    "remove_source_root",
    "pick_and_add_source_roots",
    "start_inventory",
    "pause_job",
    "resume_job",
    "cancel_job",
    "get_inventory_status",
    "start_extraction",
    "get_extraction_status",
    "list_documents",
    "open_original",
    "reveal_original",
    "list_provider_profiles",
    "save_provider_profile",
    "delete_provider_profile",
    "test_provider_connection",
    "detect_local_providers",
];
