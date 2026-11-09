//! IPC contract tests.
//!
//! Every command of the Phase 0 contract is registered, listed in
//! `COMMAND_NAMES`, and exposed by the frontend facade under the same name.
//! Since Slice 0.7 no command is a stub. Implemented commands are thin
//! adapters, tested through their services (see the other test files).

use atlas_lib::commands::{self, COMMAND_NAMES};
use atlas_lib::domain::dto::OsKind;
use tauri::async_runtime::block_on;

#[test]
fn app_status_reports_only_true_facts() {
    let status = block_on(commands::app::get_app_status()).unwrap();
    assert_eq!(status.app_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(status.slice, "1.4");
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
    assert_eq!(
        implemented,
        [
            "data_root",
            "source_folders",
            "device_check",
            "inventory",
            "library_browse",
            "provider_test",
            "extraction",
            "full_text_search"
        ]
    );
    assert!(
        status
            .features
            .iter()
            .all(|f| f.implemented == f.planned_slice.is_none()),
        "unimplemented features name their slice; implemented ones do not"
    );
}

/// No command returns `not_implemented` any more (Slice 0.7 shipped the
/// last ones). Checked on the source so a stub can't sneak back in.
#[test]
fn no_command_is_a_stub() {
    let sources = [
        include_str!("../src/commands/app.rs"),
        include_str!("../src/commands/workspace.rs"),
        include_str!("../src/commands/settings.rs"),
        include_str!("../src/commands/sources.rs"),
        include_str!("../src/commands/inventory.rs"),
        include_str!("../src/commands/extraction.rs"),
        include_str!("../src/commands/search.rs"),
        include_str!("../src/commands/library.rs"),
        include_str!("../src/commands/providers.rs"),
    ];
    for source in sources {
        assert!(!source.contains("not_implemented("));
    }
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
