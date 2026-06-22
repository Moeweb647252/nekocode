use std::sync::Arc;

use dashmap::Entry::{Occupied, Vacant};
use nekocode_core::agent::{Agent, AgentEvent};
use nekocode_entities::thread::Thread;
use nekocode_subthread::activator::{ActivationOutcome, ThreadActivator};
use tokio::sync::mpsc::UnboundedSender;

use crate::api::thread::{MiddlewareBuildContext, build_middlewares};

/// API-layer implementation of `ThreadActivator`. Builds a subthread's
/// `Agent` from its DB middlewares (mirroring the `activate_thread` API
/// endpoint's logic) and runs it to completion via `Agent::run_loop`.
///
/// This struct lives in the API crate because it depends on `Agent` and the
/// `active_threads` / `generate_states` maps, which are API-crate types. The
/// `nekocode-subthread` crate only knows the `ThreadActivator` trait, keeping
/// the dependency direction sound.
#[derive(Clone)]
pub struct ApiThreadActivator {
    pub db: toasty::Db,
    pub config: Arc<tokio::sync::RwLock<nekocode_types::config::Config>>,
    pub active_threads:
        Arc<dashmap::DashMap<u64, Arc<tokio::sync::RwLock<Agent>>>>,
    pub generate_states:
        Arc<dashmap::DashMap<u64, Arc<crate::api::generate::GenerateState>>>,
}

#[async_trait::async_trait]
impl ThreadActivator for ApiThreadActivator {
    async fn activate(&self, subthread_id: u64) -> Result<ActivationOutcome, anyhow::Error> {
        let thread = toasty::query!(Thread FILTER .id == #subthread_id)
            .include(Thread::fields().middlewares())
            .first()
            .exec(&mut self.db.clone())
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!("Subthread not found: {}", subthread_id)
            })?;
        let model_configs = {
            let config = self.config.read().await;
            config.models.clone()
        };
        let model_config = model_configs
            .into_iter()
            .find(|cfg| cfg.name == thread.model)
            .ok_or_else(|| {
                anyhow::anyhow!("Model config not found: {}", thread.model)
            })?;
        // Build the provider once and share it via Arc — both the
        // middleware-build context (for the subagent middleware) and the
        // Agent struct itself need the same provider instance.
        let provider: Arc<dyn nekocode_core::provider::Provider> =
            Arc::from(nekocode_provider::build_from_config(&model_config.data));

        let extensions = Arc::new(dashmap::DashMap::new());

        let ctx = MiddlewareBuildContext {
            db: self.db.clone(),
            config: self.config.clone(),
            extensions: extensions.clone(),
            thread_id: subthread_id,
            working_directory: thread.working_directory.clone(),
            subthread_activator: Arc::new(self.clone()),
            provider: provider.clone(),
        };
        let middlewares = build_middlewares(&ctx, &thread.middlewares.get()).await;

        match self.active_threads.entry(subthread_id) {
            Occupied(entry) => {
                // Already activated. Snapshot the cached agent under the read
                // lock and hand it back so `start_subthread` can still `run()`
                // it — having an activated entry is not an error; running
                // concurrently is. Agent is Clone (fields are Arcs) so the
                // snapshot is cheap. The completion callback's `deactivate`
                // normally keeps the slot free between runs, so an Occupied
                // hit here means a concurrent `start_subthread` raced past the
                // run-state check — accepting it is preferable to bailing.
                let agent = entry.get().read().await.clone();
                Ok(ActivationOutcome::AlreadyActivated(Arc::new(agent)))
            }
            Vacant(entry) => {
                let agent = Agent {
                    thread_id: subthread_id,
                    db: self.db.clone(),
                    middlewares: Arc::new(middlewares),
                    provider,
                    extensions,
                };
                // Cache behind the shared RwLock<Agent> the rest of the API
                // expects, and hand back an owned Arc<Agent> snapshot for the
                // background run_loop.
                entry.insert(Arc::new(tokio::sync::RwLock::new(agent.clone())));
                Ok(ActivationOutcome::Activated(Arc::new(agent)))
            }
        }
    }

    async fn deactivate(&self, subthread_id: u64) {
        self.active_threads.remove(&subthread_id);
        self.generate_states.remove(&subthread_id);
    }

    async fn run(
        &self,
        agent: Arc<Agent>,
        prompt: String,
        sender: UnboundedSender<AgentEvent>,
    ) -> Result<(), anyhow::Error> {
        // Agent::run_loop takes &self; deref the Arc for the call duration.
        let summary = (*agent).run_loop(prompt, sender).await?;
        tracing::debug!(
            "subthread run_loop finished; usage: {:?}",
            summary.usage
        );
        Ok(())
    }

    async fn delete_subthread(&self, subthread_id: u64) -> Result<(), anyhow::Error> {
        crate::api::thread::delete::delete_threads_cascade(
            &self.db,
            &self.active_threads,
            &self.generate_states,
            subthread_id,
        )
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))
    }
}
