//! `atlas.json`: identity and versions of a data root. Contains no secrets.

use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::layout::{LAYOUT_VERSION, MANIFEST, TMP};
use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::fsutil::atomic_write;

pub const FORMAT: &str = "atlas-data-root";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub format: String,
    pub root_id: String,
    pub layout_version: u32,
    /// Advisory copy; the catalog's `schema_migrations` is authoritative.
    pub catalog_schema_version: u32,
    pub created_at: String,
    pub created_by_app_version: String,
    pub last_written_by_app_version: String,
}

impl Manifest {
    pub fn new(root_id: String, catalog_schema_version: u32, now: String) -> Self {
        Self {
            format: FORMAT.to_owned(),
            root_id,
            layout_version: LAYOUT_VERSION,
            catalog_schema_version,
            created_at: now,
            created_by_app_version: APP_VERSION.to_owned(),
            last_written_by_app_version: APP_VERSION.to_owned(),
        }
    }
}

/// Reads and validates the manifest. A layout newer than this build
/// supports is `DataRootIncompatible`; anything malformed is `DataRootInvalid`.
pub fn read(root: &Path) -> AtlasResult<Manifest> {
    let bytes = match fs::read(root.join(MANIFEST)) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Err(AtlasError::DataRootInvalid {
                reason: "it has no Atlas manifest",
            });
        }
        Err(err) => return Err(AtlasError::io("read the library manifest")(err)),
    };
    let manifest: Manifest =
        serde_json::from_slice(&bytes).map_err(|_| AtlasError::DataRootInvalid {
            reason: "its manifest is unreadable",
        })?;
    if manifest.format != FORMAT {
        return Err(AtlasError::DataRootInvalid {
            reason: "its manifest is not an Atlas manifest",
        });
    }
    if uuid::Uuid::parse_str(&manifest.root_id).is_err() {
        return Err(AtlasError::DataRootInvalid {
            reason: "its manifest has an invalid library ID",
        });
    }
    if manifest.layout_version > LAYOUT_VERSION {
        return Err(AtlasError::DataRootIncompatible {
            found: manifest.layout_version,
            supported: LAYOUT_VERSION,
        });
    }
    Ok(manifest)
}

pub fn write(root: &Path, manifest: &Manifest) -> AtlasResult<()> {
    let mut bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|_| AtlasError::Internal("manifest could not be encoded"))?;
    bytes.push(b'\n');
    atomic_write(&root.join(MANIFEST), &root.join(TMP), &bytes)
        .map_err(AtlasError::io("write the library manifest"))
}
