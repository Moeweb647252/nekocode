mod agent_builder;
mod agents;
mod deletion;
mod execution;
pub(crate) mod generation;
mod subagent_factory;
mod subthread_controller;
mod thread_creation;
pub(crate) mod turn_store;

use std::sync::Arc;

use nekocode_types::config::Config;
use tokio::sync::{Mutex, MutexGuard, RwLock};

use self::agents::AgentRegistry;
use self::generation::GenerationRegistry;

#[derive(Debug, thiserror::Error)]
pub(crate) enum RuntimeError {
    #[error("thread generating")]
    ThreadGenerating,
    #[error("thread not activated")]
    ThreadNotActivated,
    #[error("thread already activated")]
    ThreadAlreadyActivated,
    #[error("item not found: {0}")]
    ItemNotFound(String),
    #[error("model config not found: {0}")]
    ModelNotFound(String),
    #[error("no active generation for thread {0}")]
    GenerationNotFound(u64),
    #[error("database error: {0}")]
    Database(#[from] toasty::Error),
    #[error("runtime error: {0}")]
    Other(String),
}

/// Internal owner for all live thread state. Transport adapters receive an
/// opaque `AppState` and interact through these high-level operations rather
/// than coordinating Maps and locks themselves.
pub(crate) struct ThreadRuntime {
    db: toasty::Db,
    config: Arc<RwLock<Config>>,
    lifecycle: Mutex<()>,
    agents: AgentRegistry,
    generations: Arc<GenerationRegistry>,
}

impl ThreadRuntime {
    pub(crate) fn new(db: toasty::Db, config: Config) -> Arc<Self> {
        Arc::new(Self {
            db,
            config: Arc::new(RwLock::new(config)),
            lifecycle: Mutex::new(()),
            agents: AgentRegistry::default(),
            generations: Arc::new(GenerationRegistry::default()),
        })
    }

    pub(crate) fn db(&self) -> toasty::Db {
        self.db.clone()
    }

    pub(crate) fn config(&self) -> Arc<RwLock<Config>> {
        self.config.clone()
    }

    pub(crate) fn is_active(&self, thread_id: u64) -> bool {
        self.agents.contains(thread_id)
    }

    pub(crate) fn active_agent(
        &self,
        thread_id: u64,
    ) -> Result<std::sync::Arc<nekocode_core::agent::Agent>, RuntimeError> {
        self.agents
            .get(thread_id)
            .ok_or(RuntimeError::ThreadNotActivated)
    }

    pub(crate) fn is_generating(&self, thread_id: u64) -> bool {
        self.generations.contains(thread_id)
    }

    pub(crate) async fn lifecycle_guard(&self) -> ThreadLifecycleGuard<'_> {
        ThreadLifecycleGuard {
            runtime: self,
            _guard: self.lifecycle.lock().await,
        }
    }
}

/// Capability issued while holding the shared lifecycle lock. Mutation routes
/// can ask it to verify idleness or evict an Agent, but cannot touch the raw
/// generation or agent registries.
pub(crate) struct ThreadLifecycleGuard<'a> {
    runtime: &'a ThreadRuntime,
    _guard: MutexGuard<'a, ()>,
}

impl ThreadLifecycleGuard<'_> {
    pub(crate) fn ensure_idle(&self, thread_id: u64) -> Result<(), RuntimeError> {
        if self.runtime.is_generating(thread_id) {
            Err(RuntimeError::ThreadGenerating)
        } else {
            Ok(())
        }
    }

    pub(crate) async fn remove_and_shutdown(&self, thread_id: u64) -> bool {
        self.runtime.agents.remove_and_shutdown(thread_id).await
    }
}
