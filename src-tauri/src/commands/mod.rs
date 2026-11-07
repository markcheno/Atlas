//! Thin Tauri IPC adapters. Each command validates input, calls a service,
//! and maps the result to DTOs. No business logic lives here.
//!
//! Every command of the Phase 0 contract (spec section 9) is registered from
//! Slice 0.1 so the frontend facade is complete. Commands whose slice has
//! not shipped return `AtlasError::NotImplemented` with the planned slice,
//! never placeholder data.

pub mod app;
pub mod inventory;
pub mod library;
pub mod providers;
pub mod sources;
pub mod workspace;

/// Names of every registered command, in registration order. Kept in sync
/// with `invoke_handler` in `lib.rs` and the frontend facade by tests.
pub const COMMAND_NAMES: &[&str] = &[
    "get_app_status",
    "get_device_snapshot",
    "choose_data_root",
    "open_data_root",
    "list_source_roots",
    "add_source_root",
    "remove_source_root",
    "start_inventory",
    "pause_job",
    "resume_job",
    "cancel_job",
    "get_inventory_status",
    "list_documents",
    "open_original",
    "reveal_original",
    "list_provider_profiles",
    "save_provider_profile",
    "test_provider_connection",
];
