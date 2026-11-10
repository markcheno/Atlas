//! The embedding job in tests: a scripted backend in front of the fake
//! one, and a workspace with cataloging, extraction and embedding sharing
//! one fake device.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use atlas_lib::domain::dto::{EmbeddingOverview, JobKind};
use atlas_lib::domain::traits::{BackendInfo, EmbedError, EmbeddingBackend};
use atlas_lib::services::embedding::fake::HashedBagOfWordsBackend;
use atlas_lib::services::embedding::job::{self as embedding_job, EmbedConfig, EmbedDeps};
use atlas_lib::services::providers::secrets::SecretVault;
use atlas_lib::services::workspace::Workspace;

use super::{Env, ExtractionHarness, fast_extraction, worker};

/// What the scripted backend does, shared with the test.
#[derive(Default)]
pub struct Script {
    /// Errors returned by the next calls, in order (then answers again).
    pub fail_next: VecDeque<EmbedError>,
    /// Every call fails with this until cleared.
    pub fail_always: Option<EmbedError>,
    /// A request containing a text with this word is refused (400).
    pub reject_word: Option<String>,
    /// Answers with vectors of this length instead of the model's.
    pub wrong_dims: Option<usize>,
    /// Each call waits this long first.
    pub delay: Duration,
    /// Every text that got a vector, in order.
    pub answered: Vec<String>,
    /// Calls made, answered or not.
    pub calls: usize,
}

#[derive(Clone, Default)]
pub struct Scripted(pub Arc<Mutex<Script>>);

impl Scripted {
    pub fn with(&self, f: impl FnOnce(&mut Script)) {
        f(&mut self.0.lock().unwrap());
    }
    pub fn answered(&self) -> Vec<String> {
        self.0.lock().unwrap().answered.clone()
    }
    pub fn calls(&self) -> usize {
        self.0.lock().unwrap().calls
    }
}

struct Backend {
    script: Scripted,
    dims: usize,
    model: String,
}

impl EmbeddingBackend for Backend {
    fn embed(&self, inputs: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError> {
        let delay = {
            let mut s = self.script.0.lock().unwrap();
            s.calls += 1;
            s.delay
        };
        std::thread::sleep(delay);
        let mut s = self.script.0.lock().unwrap();
        if let Some(e) = s.fail_next.pop_front() {
            return Err(e);
        }
        if let Some(e) = &s.fail_always {
            return Err(e.clone());
        }
        if let Some(word) = &s.reject_word
            && inputs.iter().any(|t| t.contains(word.as_str()))
        {
            return Err(EmbedError::BadRequest { status: 400 });
        }
        let dims = s.wrong_dims.unwrap_or(self.dims);
        let fake = HashedBagOfWordsBackend::new(dims);
        s.answered.extend(inputs.iter().map(|t| (*t).to_owned()));
        Ok(inputs.iter().map(|t| fake.vector(t)).collect())
    }

    fn describe(&self) -> BackendInfo {
        BackendInfo {
            model: self.model.clone(),
            dims: self.dims,
        }
    }
}

pub fn fast_embedding() -> EmbedConfig {
    EmbedConfig {
        unthrottled: true,
        batch_size: 4,
        recheck_interval: Duration::from_millis(5),
        policy_wait: Duration::from_millis(20),
        idle_wait: Duration::from_millis(50),
        event_interval: Duration::ZERO,
        backoff_base: Duration::from_millis(5),
        backoff_max: Duration::from_millis(20),
        retry_base: Duration::from_millis(100),
        rebuild_threads: 2,
        ..EmbedConfig::default()
    }
}

/// Cataloging, extraction and embedding, as in the app.
pub struct EmbedHarness {
    pub ex: ExtractionHarness,
    pub deps: EmbedDeps,
    pub script: Scripted,
    pub events: Arc<Mutex<Vec<EmbeddingOverview>>>,
}

impl EmbedHarness {
    pub fn new(config: EmbedConfig) -> Self {
        let ex = ExtractionHarness::new(fast_extraction(), worker());
        let script = Scripted::default();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let backend_script = script.clone();
        let deps = EmbedDeps {
            governor: ex.inv.deps.governor.clone(),
            vault: Arc::new(SecretVault::new(None)),
            events: Arc::new(move |o: &EmbeddingOverview| sink.lock().unwrap().push(o.clone())),
            config,
            backends: Arc::new(move |s, _stop| {
                Box::new(Backend {
                    script: backend_script.clone(),
                    dims: s.dims,
                    model: s.model.clone(),
                })
            }),
        };
        Self {
            ex,
            deps,
            script,
            events,
        }
    }

    /// The real HTTP backend instead of the scripted one.
    pub fn over_http(mut self, config: atlas_lib::infrastructure::http_probe::ProbeConfig) -> Self {
        self.deps.backends = embedding_job::http_backends(config);
        self
    }
}

impl Env {
    pub fn workspace_with_embedding(&self, h: &EmbedHarness) -> Workspace {
        self.workspace_with_extraction(&h.ex)
            .with_job_processor(JobKind::Embed, embedding_job::factory(h.deps.clone()))
    }
}
