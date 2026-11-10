//! Embedding search queries (Slice 2.4, phase2-spec §7.4): with the model
//! search uses (the serving one), its query prefix, a small cache, and a
//! short wait.
//!
//! A query is never throttled, but search waits for its vector at most
//! [`QUERY_WAIT`]. A request still running then finishes in the
//! background and fills the cache, so asking again (or a model that was
//! loading) answers at once.

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rusqlite::{Connection, OptionalExtension};

use super::job::{BackendFactory, ResolvedService};
use super::{db::EmbeddingModel, models};
use crate::domain::dto::{ProviderLocation, SemanticUnavailable};
use crate::domain::traits::EmbedError;
use crate::services::providers::profiles::{self, Prepared};
use crate::services::providers::recommend::{QueryProfile, query_profile};
use crate::services::providers::secrets::SecretVault;
use crate::services::providers::url_policy;

/// The longest search waits for a query's vector.
pub const QUERY_WAIT: Duration = Duration::from_millis(1_500);
/// Queries kept per model.
pub const CACHE_SIZE: usize = 256;
/// How long a resolved service (address and key) is reused.
const SERVICE_TTL: Duration = Duration::from_secs(60);

/// The model search uses and how to reach its service.
#[derive(Clone)]
pub struct QueryTarget {
    pub model: EmbeddingModel,
    pub profile: QueryProfile,
    service: Arc<ResolvedService>,
}

/// Why there is no target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoTarget {
    /// No embedding role, or no model filled yet: search is text only.
    Off,
    Unavailable(SemanticUnavailable),
}

/// Per model: queries in the order they were added, and their vectors.
type Cache = Mutex<HashMap<String, (VecDeque<String>, HashMap<String, Vec<f32>>)>>;

pub struct QueryEmbedder {
    backends: BackendFactory,
    cache: Cache,
    services: Mutex<HashMap<String, (Instant, Arc<ResolvedService>)>>,
}

impl QueryEmbedder {
    pub fn new(backends: BackendFactory) -> Self {
        Self {
            backends,
            cache: Mutex::new(HashMap::new()),
            services: Mutex::new(HashMap::new()),
        }
    }

    /// The serving model and its service, if search may use them now.
    /// `allow_remote` is the library's privacy setting.
    pub fn target(
        &self,
        conn: &Connection,
        vault: &SecretVault,
        allow_remote: bool,
    ) -> Result<QueryTarget, NoTarget> {
        let failed = |_| NoTarget::Unavailable(SemanticUnavailable::Failed);
        if models::role(conn).map_err(failed)?.is_none() {
            return Err(NoTarget::Off);
        }
        let Some(model) = models::serving(conn).map_err(failed)? else {
            return Err(NoTarget::Off);
        };
        let Some(profile_id) = model.provider_profile_id.clone() else {
            // Its service was removed.
            return Err(NoTarget::Unavailable(SemanticUnavailable::Unreachable));
        };
        let base: Option<String> = conn
            .query_row(
                "SELECT base_url FROM provider_profiles WHERE id = ?1",
                [&profile_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(failed)?;
        let Some(base) = base else {
            return Err(NoTarget::Unavailable(SemanticUnavailable::Unreachable));
        };
        if url_policy::location_of(&base) == ProviderLocation::Internet && !allow_remote {
            return Err(NoTarget::Unavailable(SemanticUnavailable::NotPermitted));
        }
        let service = self.service(conn, vault, &model, &profile_id)?;
        Ok(QueryTarget {
            profile: query_profile(&model.model_name),
            model,
            service,
        })
    }

    fn service(
        &self,
        conn: &Connection,
        vault: &SecretVault,
        model: &EmbeddingModel,
        profile_id: &str,
    ) -> Result<Arc<ResolvedService>, NoTarget> {
        let key = format!("{}|{profile_id}", model.id);
        if let Some((at, s)) = self
            .services
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&key)
            && at.elapsed() < SERVICE_TTL
        {
            return Ok(Arc::clone(s));
        }
        let resolved = match profiles::resolve_saved(conn, vault, profile_id) {
            Ok(Prepared::Ready {
                kind,
                endpoint,
                key: secret,
            }) => ResolvedService {
                kind,
                base: endpoint.base,
                key: secret,
                model: model.model_name.clone(),
                dims: model.dims,
            },
            Ok(Prepared::Blocked(_)) => {
                return Err(NoTarget::Unavailable(SemanticUnavailable::NotPermitted));
            }
            Err(_) => return Err(NoTarget::Unavailable(SemanticUnavailable::Unreachable)),
        };
        let resolved = Arc::new(resolved);
        self.services
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(key, (Instant::now(), Arc::clone(&resolved)));
        Ok(resolved)
    }

    fn cached(&self, model: &str, text: &str) -> Option<Vec<f32>> {
        let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache.get(model).and_then(|(_, m)| m.get(text).cloned())
    }

    fn remember(cache: &Cache, model: &str, text: &str, v: Vec<f32>) {
        let mut cache = cache.lock().unwrap_or_else(|p| p.into_inner());
        let (order, map) = cache.entry(model.to_owned()).or_default();
        if map.insert(text.to_owned(), v).is_none() {
            order.push_back(text.to_owned());
            while order.len() > CACHE_SIZE {
                if let Some(old) = order.pop_front() {
                    map.remove(&old);
                }
            }
        }
    }

    /// Starts embedding `text` for `target`; [`PendingQuery::wait`] gives the
    /// vector within [`QUERY_WAIT`] of the start.
    pub fn start(self: &Arc<Self>, target: &QueryTarget, text: &str) -> PendingQuery {
        let started = Instant::now();
        let model = target.model.id.clone();
        if let Some(v) = self.cached(&model, text) {
            return PendingQuery {
                started,
                ready: Some(Ok(v)),
                rx: None,
            };
        }
        let input = format!("{}{text}", target.profile.prefix.as_deref().unwrap_or(""));
        let (tx, rx) = mpsc::channel();
        let me = Arc::clone(self);
        let service = Arc::clone(&target.service);
        let text = text.to_owned();
        let _ = std::thread::Builder::new()
            .name("atlas-query-embed".into())
            .spawn(move || {
                let backend = (me.backends)(&service, Arc::new(|| false));
                let result = backend
                    .embed(&[input.as_str()])
                    .and_then(|mut v| v.pop().ok_or(EmbedError::WrongCount { sent: 1, got: 0 }));
                if let Ok(v) = &result {
                    Self::remember(&me.cache, &model, &text, v.clone());
                }
                let _ = tx.send(result);
            });
        PendingQuery {
            started,
            ready: None,
            rx: Some(rx),
        }
    }
}

/// A query being embedded.
pub struct PendingQuery {
    started: Instant,
    ready: Option<Result<Vec<f32>, EmbedError>>,
    rx: Option<mpsc::Receiver<Result<Vec<f32>, EmbedError>>>,
}

impl PendingQuery {
    /// The vector, or why there is none, waiting no longer than
    /// [`QUERY_WAIT`] from the start.
    pub fn wait(self) -> Result<Vec<f32>, SemanticUnavailable> {
        let result = match (self.ready, self.rx) {
            (Some(r), _) => r,
            (None, Some(rx)) => {
                let left = QUERY_WAIT.saturating_sub(self.started.elapsed());
                match rx.recv_timeout(left) {
                    Ok(r) => r,
                    Err(_) => return Err(SemanticUnavailable::Timeout),
                }
            }
            (None, None) => return Err(SemanticUnavailable::Failed),
        };
        result.map_err(|e| match e {
            EmbedError::Network | EmbedError::Unavailable => SemanticUnavailable::Unreachable,
            EmbedError::Timeout => SemanticUnavailable::Timeout,
            _ => SemanticUnavailable::Failed,
        })
    }
}
