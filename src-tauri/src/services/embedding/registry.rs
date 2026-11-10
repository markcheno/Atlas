//! The vector indexes of one open library (ADR 0013): opened lazily, one
//! per embedding model, shared by the embedding job and search (Slice 2.4).
//!
//! Each model has a slot of its own. Opening an index can take a minute
//! (a rebuild), and holds only that model's slot: search asks with
//! [`VectorRegistry::loaded`], which never waits, and can start an open in
//! the background with [`VectorRegistry::open_in_background`].

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use super::chunk_map::ChunkMap;
use super::index::{OpenReport, VectorIndex, VectorPaths, open_index};
use crate::error::AtlasResult;
use crate::services::catalog::writer::CatalogHandle;
use crate::services::documents::related::RelatedCache;

type Slot = Arc<Mutex<Option<Arc<VectorIndex>>>>;

pub struct VectorRegistry {
    paths: VectorPaths,
    slots: Mutex<HashMap<String, Slot>>,
    /// Chunk id → document, for search's filters (shared by every model).
    chunk_map: Mutex<ChunkMap>,
    /// Related documents' candidates, by document (Slice 2.5).
    related: Mutex<RelatedCache>,
}

impl VectorRegistry {
    /// For the library at `data_root`. Saves left half-staged by a kill are
    /// removed: no save can be under way before the library opens.
    pub fn new(data_root: &Path) -> Self {
        let paths = VectorPaths::for_root(data_root);
        let _ = fs::remove_dir_all(&paths.tmp);
        Self {
            paths,
            slots: Mutex::new(HashMap::new()),
            chunk_map: Mutex::new(ChunkMap::default()),
            related: Mutex::new(RelatedCache::default()),
        }
    }

    pub fn paths(&self) -> &VectorPaths {
        &self.paths
    }

    fn slot(&self, model_id: &str) -> Slot {
        let mut slots = self.slots.lock().unwrap_or_else(|p| p.into_inner());
        Arc::clone(slots.entry(model_id.to_owned()).or_default())
    }

    /// The model's index, opening (and reconciling or rebuilding) it on
    /// first use with up to `threads` threads. Waits for an open already
    /// under way.
    pub fn get_or_open(
        &self,
        catalog: &CatalogHandle,
        model_id: &str,
        threads: usize,
    ) -> AtlasResult<(Arc<VectorIndex>, Option<OpenReport>)> {
        let slot = self.slot(model_id);
        let mut held = slot.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(index) = held.as_ref() {
            return Ok((Arc::clone(index), None));
        }
        let (index, report) = open_index(&self.paths, catalog, model_id, threads)?;
        let index = Arc::new(index);
        *held = Some(Arc::clone(&index));
        Ok((index, Some(report)))
    }

    /// The model's index if it is open. Never waits: `None` while it is
    /// being opened.
    pub fn loaded(&self, model_id: &str) -> Option<Arc<VectorIndex>> {
        let slot = self.slot(model_id);
        let held = slot.try_lock().ok()?;
        held.as_ref().cloned()
    }

    /// Whether the model's index is being opened right now.
    pub fn opening(&self, model_id: &str) -> bool {
        self.slot(model_id).try_lock().is_err()
    }

    /// Starts opening the model's index on a thread of its own, unless it is
    /// open or opening already.
    pub fn open_in_background(
        self: &Arc<Self>,
        catalog: &CatalogHandle,
        model_id: &str,
        threads: usize,
    ) {
        if self.loaded(model_id).is_some() || self.opening(model_id) {
            return;
        }
        let (me, catalog, model) = (Arc::clone(self), catalog.clone(), model_id.to_owned());
        let _ = std::thread::Builder::new()
            .name("atlas-index-open".into())
            .spawn(move || {
                let _ = me.get_or_open(&catalog, &model, threads);
            });
    }

    /// Drops a model's index (retired or failed).
    pub fn forget(&self, model_id: &str) {
        self.slots
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(model_id);
        self.related_cache().forget(model_id);
    }

    /// The chunk → document map, topped up to the catalog's latest chunk.
    pub fn chunk_map(&self) -> MutexGuard<'_, ChunkMap> {
        self.chunk_map.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Related documents' cache (Slice 2.5).
    pub fn related_cache(&self) -> MutexGuard<'_, RelatedCache> {
        self.related.lock().unwrap_or_else(|p| p.into_inner())
    }
}
