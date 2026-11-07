//! IPC contract tests.
//!
//! Every command of the Phase 0 contract is registered. Commands whose
//! slice has not shipped return an explicit `not_implemented` error that
//! names itself and its planned slice; nothing returns placeholder data.
//! Implemented commands are thin adapters, tested through their services
//! (see `workspace.rs`).

use atlas_lib::commands::{self, COMMAND_NAMES};
use atlas_lib::domain::dto::{
    ConnectionTestTarget, DocumentQuery, DocumentSort, OsKind, ProviderKind, ProviderProfileInput,
};
use atlas_lib::error::{AtlasError, ErrorCode};
use tauri::async_runtime::block_on;

fn assert_not_implemented<T: std::fmt::Debug>(
    result: Result<T, AtlasError>,
    command: &str,
    slice: &str,
) {
    let err = result.expect_err("stub must not return data");
    let payload = err.payload();
    assert_eq!(payload.code, ErrorCode::NotImplemented);
    assert_eq!(payload.command.as_deref(), Some(command));
    assert_eq!(payload.planned_slice.as_deref(), Some(slice));
    assert!(
        COMMAND_NAMES.contains(&command),
        "{command} missing from COMMAND_NAMES"
    );
}

#[test]
fn app_status_reports_only_true_facts() {
    let status = block_on(commands::app::get_app_status()).unwrap();
    assert_eq!(status.app_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(status.slice, "0.3");
    assert_eq!(status.arch, std::env::consts::ARCH);
    if cfg!(target_os = "macos") {
        assert_eq!(status.os, OsKind::Macos);
    }
    let implemented: Vec<&str> = status
        .features
        .iter()
        .filter(|f| f.implemented)
        .map(|f| f.id.as_str())
        .collect();
    assert_eq!(implemented, ["data_root", "source_folders"]);
    assert!(
        status
            .features
            .iter()
            .all(|f| f.implemented == f.planned_slice.is_none()),
        "unimplemented features name their slice; implemented ones do not"
    );
}

#[test]
fn every_stub_returns_not_implemented() {
    use commands::{app, inventory, library, providers};
    let s = || "x".to_owned();
    let query = DocumentQuery {
        text: None,
        source_root_id: None,
        file_type: None,
        sort: DocumentSort::NameAsc,
        page: 0,
        page_size: 50,
    };
    let candidate = ProviderProfileInput {
        id: None,
        kind: ProviderKind::Ollama,
        label: "Local".into(),
        base_url: "http://localhost:11434".into(),
        api_key: None,
        insecure_http_acknowledged: false,
    };

    block_on(async {
        assert_not_implemented(
            app::get_device_snapshot().await,
            "get_device_snapshot",
            "0.4",
        );
        assert_not_implemented(
            inventory::start_inventory(s()).await,
            "start_inventory",
            "0.5",
        );
        assert_not_implemented(inventory::pause_job(s()).await, "pause_job", "0.5");
        assert_not_implemented(inventory::resume_job(s()).await, "resume_job", "0.5");
        assert_not_implemented(inventory::cancel_job(s()).await, "cancel_job", "0.5");
        assert_not_implemented(
            inventory::get_inventory_status().await,
            "get_inventory_status",
            "0.5",
        );
        assert_not_implemented(
            library::list_documents(query).await,
            "list_documents",
            "0.6",
        );
        assert_not_implemented(library::open_original(s()).await, "open_original", "0.6");
        assert_not_implemented(
            library::reveal_original(s()).await,
            "reveal_original",
            "0.6",
        );
        assert_not_implemented(
            providers::list_provider_profiles().await,
            "list_provider_profiles",
            "0.7",
        );
        assert_not_implemented(
            providers::save_provider_profile(candidate.clone()).await,
            "save_provider_profile",
            "0.7",
        );
        assert_not_implemented(
            providers::test_provider_connection(ConnectionTestTarget::Candidate {
                profile: candidate,
            })
            .await,
            "test_provider_connection",
            "0.7",
        );
    });
}

/// `generate_handler!` cannot be introspected, so check its source text
/// against `COMMAND_NAMES` to catch a command added in one place only.
#[test]
fn registered_handlers_match_command_names() {
    let lib_rs = include_str!("../src/lib.rs");
    let start = lib_rs.find("generate_handler![").unwrap();
    let end = start + lib_rs[start..].find(']').unwrap();
    let registered: Vec<&str> = lib_rs[start..end]
        .split(',')
        .filter_map(|item| item.trim().rsplit("::").next())
        .filter(|name| !name.is_empty() && !name.contains('['))
        .collect();
    assert_eq!(registered, COMMAND_NAMES);
}

/// The frontend facade must expose every command, spelled identically.
#[test]
fn frontend_facade_covers_every_command() {
    let client_ts = include_str!("../../src/lib/api/client.ts");
    for name in COMMAND_NAMES {
        assert!(
            client_ts.contains(&format!("'{name}'")),
            "src/lib/api/client.ts does not invoke '{name}'"
        );
    }
}
