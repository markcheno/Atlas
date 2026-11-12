//! The Atlas Data Root's on-disk layout (phase0-spec §6, ADR 0002).

use std::path::{Path, PathBuf};

/// Written last during creation; its presence makes a folder a valid root.
pub const MANIFEST: &str = "atlas.json";
/// Held with an exclusive OS lock while a process has the root open.
pub const LOCK: &str = "atlas.lock";
/// Present only while a root is being created; marks a resumable attempt.
pub const INIT_MARKER: &str = ".atlas-init";
pub const SETTINGS: &str = "config/settings.json";
pub const CATALOG: &str = "library/catalog.sqlite";
/// What the user creates (Phase 3, ADR 0016), apart from the catalog.
pub const KNOWLEDGE: &str = "library/knowledge.sqlite";
/// Staging area for atomic writes; emptied each time the root is opened.
pub const TMP: &str = "tmp";

/// Directory layout version recorded in the manifest.
pub const LAYOUT_VERSION: u32 = 1;

/// Every directory of layout version 1. Most are reserved for later phases.
pub const DIRECTORIES: &[&str] = &[
    "config",
    "library",
    "content/extracted",
    "indexes/vectors",
    "assets/thumbnails",
    "cache/previews",
    "jobs/checkpoints",
    "models",
    "exports",
    // Diagnostics reports the user saved (Slice 4.1).
    "exports/diagnostics",
    // The log (Slice 4.1). Never cleaned by Clean caches.
    "logs",
    TMP,
    "backups",
    // Snapshots of the knowledge store (Slice 3.2). Never cleaned.
    "backups/knowledge",
    "backups/knowledge/staging",
];

/// Offered when the user picks a folder that already holds other files.
pub const SUGGESTED_SUBFOLDER: &str = "Atlas Library";

/// OS-generated files that do not make a folder "non-empty".
pub const IGNORABLE_ENTRIES: &[&str] = &[".DS_Store", ".localized", "Thumbs.db", "desktop.ini"];

/// The suggested data root inside the OS's per-user local application data
/// folder (`~/Library/Application Support` on macOS, `%LOCALAPPDATA%` on
/// Windows, `$XDG_DATA_HOME` or `~/.local/share` on Linux).
pub fn default_data_root(local_data_dir: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        local_data_dir.join("com.altazlabs.atlas").join("Library")
    } else if cfg!(windows) {
        local_data_dir.join("Atlas").join("Library")
    } else {
        local_data_dir.join("atlas").join("library")
    }
}
