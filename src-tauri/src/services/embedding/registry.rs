//! The vector indexes of one open library (ADR 0013): opened lazily, one
//! per embedding model, shared by the embedding job and (Slice 2.4) search.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use super::index::{OpenReport, VectorIndex, VectorPaths, open_index};
use crate::error::AtlasResult;
use crate::services::catalog::writer::CatalogHandle;

pub struct VectorRegistry {
    paths: VectorPaths,
    indexes: Mutex<HashMap<String, Arc<VectorIndex>>>,
}

impl VectorRegistry {
    /// For the library at `data_root`. Saves left half-staged by a kill are
    /// removed: no save can be under way before the library opens.
    pub fn new(data_root: &Path) -> Self {
        let paths = VectorPaths::for_root(data_root);
        let _ = fs::remove_dir_all(&paths.tmp);
        Self {
            paths,
            indexes: Mutex::new(HashMap::new()),
        }
    }

    pub fn paths(&self) -> &VectorPaths {
        &self.paths
    }

    /// The model's index, opening (and reconciling or rebuilding) it on
    /// first use with up to `threads` threads.
    pub fn get_or_open(
        &self,
        catalog: &CatalogHandle,
        model_id: &str,
        threads: usize,
    ) -> AtlasResult<(Arc<VectorIndex>, Option<OpenReport>)> {
        let mut indexes = self.indexes.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(index) = indexes.get(model_id) {
            return Ok((Arc::clone(index), None));
        }
        let (index, report) = open_index(&self.paths, catalog, model_id, threads)?;
        let index = Arc::new(index);
        indexes.insert(model_id.to_owned(), Arc::clone(&index));
        Ok((index, Some(report)))
    }

    /// The model's index if it is open.
    pub fn loaded(&self, model_id: &str) -> Option<Arc<VectorIndex>> {
        self.indexes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(model_id)
            .cloned()
    }

    /// Drops a model's index (retired or failed).
    pub fn forget(&self, model_id: &str) {
        self.indexes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(model_id);
    }
}
