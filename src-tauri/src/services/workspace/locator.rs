//! The one file Atlas keeps outside the data root: where the root is.
//!
//! Stored in the OS per-user config folder so Atlas can find a data root
//! that lives anywhere, including an external drive. Losing it loses no
//! data; the user can reopen the root by choosing its folder.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AtlasError, AtlasResult};
use crate::infrastructure::fsutil::atomic_write;

pub const FILE: &str = "locator.json";
const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Locator {
    pub version: u32,
    pub data_root: String,
    pub root_id: String,
}

impl Locator {
    pub fn path(&self) -> PathBuf {
        PathBuf::from(&self.data_root)
    }
}

/// `None` when Atlas has never chosen a root, or the file is unreadable
/// (treated as first run; the root itself is untouched and can be reopened).
pub fn read(config_dir: &Path) -> Option<Locator> {
    let bytes = fs::read(config_dir.join(FILE)).ok()?;
    serde_json::from_slice::<Locator>(&bytes)
        .ok()
        .filter(|l| l.version == VERSION && !l.data_root.is_empty())
}

pub fn write(config_dir: &Path, data_root: &str, root_id: &str) -> AtlasResult<()> {
    fs::create_dir_all(config_dir).map_err(AtlasError::io("create Atlas's settings folder"))?;
    let locator = Locator {
        version: VERSION,
        data_root: data_root.to_owned(),
        root_id: root_id.to_owned(),
    };
    let mut bytes = serde_json::to_vec_pretty(&locator)
        .map_err(|_| AtlasError::Internal("locator could not be encoded"))?;
    bytes.push(b'\n');
    atomic_write(&config_dir.join(FILE), config_dir, &bytes)
        .map_err(AtlasError::io("remember the library location"))
}
