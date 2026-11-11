//! Thin Tauri IPC adapters. Each command validates input, calls a service,
//! and maps the result to DTOs. No business logic lives here.
//!
//! Every command of the Phase 0 contract (spec section 9) is registered from
//! Slice 0.1 so the frontend facade is complete. Commands whose slice has
//! not shipped return `AtlasError::NotImplemented` with the planned slice,
//! never placeholder data.

pub mod answers;
pub mod app;
pub mod collections;
pub mod documents;
pub mod embedding;
pub mod extraction;
pub mod inventory;
pub mod knowledge;
pub mod library;
pub mod providers;
pub mod reader;
pub mod roles;
pub mod saved;
pub mod search;
pub mod settings;
pub mod sources;
pub mod storage;
pub mod trails;
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
    "start_embedding",
    "get_embedding_status",
    "search_documents",
    "get_document_detail",
    "list_duplicates",
    "get_related_documents",
    "get_storage_usage",
    "clean_caches",
    "list_documents",
    "open_original",
    "reveal_original",
    "list_provider_profiles",
    "save_provider_profile",
    "delete_provider_profile",
    "test_provider_connection",
    "detect_local_providers",
    "test_model_capability",
    "get_model_roles",
    "set_model_role",
    "clear_model_role",
    "get_model_recommendations",
    "ask_atlas",
    "cancel_answer",
    "acknowledge_disclosure",
    "get_knowledge_status",
    "record_open",
    "get_recording_status",
    "dismiss_trail_note",
    "list_trails",
    "get_trail",
    "start_trail",
    "continue_trail",
    "rename_trail",
    "pin_trail",
    "delete_trail",
    "delete_trail_step",
    "add_trail_note",
    "delete_all_trails",
    "save_bookmark",
    "save_finding",
    "update_saved_note",
    "delete_saved",
    "list_saved",
    "list_collections",
    "get_collection",
    "save_collection",
    "delete_collection",
    "add_to_collection",
    "remove_from_collection",
    "get_collection_membership",
    "get_suggestions",
    "dismiss_suggestion",
    "render_page",
];
