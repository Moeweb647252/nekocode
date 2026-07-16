use std::sync::Arc;

use dashmap::Entry::{Occupied, Vacant};
use nekocode_core::agent::Agent;
use nekocode_core::extensions::Extensions;
use nekocode_entities::thread::Thread;
use nekocode_subthread::controller::{ActivationOutcome, ThreadController};
use nekocode_types::generate::MessageContent;

use crate::api::generate::turn_io;
use crate::api::thread::{MiddlewareBuildContext, build_middlewares};

/// API-layer implementation of `ThreadController`. Builds a subthread's
/// `Agent` from its DB middlewares (mirroring the `activate_thread` API
/// endpoint's logic) and runs it to completion via `Agent::run_loop`.
///
/// This struct lives in the API crate because it depends on `Agent` and the
/// `active_threads` / `generate_states` maps, which are API-crate types. The
/// `nekocode-subthread` crate only knows the `ThreadController` trait, keeping
/// the dependency direction sound.
#[derive(Clone)]
pub struct ApiThreadController {
    pub db: toasty::Db,
    pub config: Arc<tokio::sync::RwLock<nekocode_types::config::Config>>,
    pub active_threads: Arc<dashmap::DashMap<u64, Arc<tokio::sync::RwLock<Agent>>>>,
    pub generate_states: Arc<dashmap::DashMap<u64, Arc<crate::api::generate::GenerateState>>>,
    pub thread_lifecycle: Arc<tokio::sync::Mutex<()>>,
}

impl ApiThreadController {
    fn release_generation(
        &self,
        thread_id: u64,
        expected: &Arc<crate::api::generate::GenerateState>,
    ) {
        let should_remove = self
            .generate_states
            .get(&thread_id)
            .map(|current| Arc::ptr_eq(current.value(), expected))
            .unwrap_or(false);
        if should_remove {
            self.generate_states.remove(&thread_id);
        }
    }
}

#[async_trait::async_trait]
impl ThreadController for ApiThreadController {
    async fn activate(
        &self,
        subthread_id: u64,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<ActivationOutcome, anyhow::Error> {
        let generate_state = crate::api::generate::GenerateState::with_cancellation(cancellation);
        {
            let _lifecycle = self.thread_lifecycle.lock().await;
            if self.generate_states.contains_key(&subthread_id) {
                anyhow::bail!("subthread {subthread_id} is already generating");
            }
            self.generate_states
                .insert(subthread_id, generate_state.clone());
        }

        let result: Result<ActivationOutcome, anyhow::Error> = async {
            let thread = toasty::query!(Thread FILTER .id == #subthread_id)
                .include(Thread::fields().middlewares())
                .first()
                .exec(&mut self.db.clone())
                .await?
                .ok_or_else(|| anyhow::anyhow!("Subthread not found: {}", subthread_id))?;
            let model_configs = {
                let config = self.config.read().await;
                config.models.clone()
            };
            let model_config = model_configs
                .into_iter()
                .find(|cfg| cfg.name == thread.model)
                .ok_or_else(|| anyhow::anyhow!("Model config not found: {}", thread.model))?;
            // Build the provider once and share it via Arc — both the
            // middleware-build context (for the subthread and subagent middlewares)
            // and the Agent struct itself need the same provider instance.
            let provider: Arc<dyn nekocode_core::provider::Provider> =
                Arc::from(nekocode_provider::build_from_config(&model_config.data));

            let extensions = Extensions::new();

            let ctx = MiddlewareBuildContext {
                db: self.db.clone(),
                config: self.config.clone(),
                extensions: extensions.clone(),
                thread_id: subthread_id,
                working_directory: thread.working_directory.clone(),
                subthread_controller: Arc::new(self.clone()),
                provider: provider.clone(),
            };
            let middlewares = build_middlewares(&ctx, thread.middlewares.get()).await;

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
                        working_directory: thread.working_directory.clone(),
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
        .await;

        if let Err(error) = &result {
            generate_state.finish(crate::api::generate::StopReason {
                reason: crate::api::generate::Reason::Error,
                detail: error.to_string().into(),
            });
            self.release_generation(subthread_id, &generate_state);
        }
        result
    }

    async fn deactivate(&self, subthread_id: u64) {
        if let Some((_, agent)) = self.active_threads.remove(&subthread_id) {
            agent.read().await.shutdown().await;
        }
    }

    async fn run(&self, agent: Arc<Agent>, prompt: String) -> Result<(), anyhow::Error> {
        // Load history and persist results in the API layer; the agent itself
        // is storage-free. Agent::run_loop takes &self; deref the Arc for the
        // call duration.
        let thread_id = agent.thread_id;
        let generate_state = self
            .generate_states
            .get(&thread_id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| anyhow::anyhow!("subthread run reservation is missing"))?;

        let result: Result<(), anyhow::Error> = async {
            let old_turns = turn_io::load_turn_context(&self.db, thread_id).await?;
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            let cancellation = generate_state.cancellation_token.clone();
            let handle = tokio::spawn(async move {
                agent
                    .run_loop_with_cancellation(
                        vec![MessageContent::Text { content: prompt }],
                        old_turns,
                        nekocode_core::agent::AgentEventSink::new(tx),
                        cancellation,
                    )
                    .await
            });
            while let Some(event) = rx.recv().await {
                generate_state.publish(event);
            }
            match handle.await? {
                Ok(turn) => {
                    let usage = turn.usage.clone();
                    turn_io::persist_turn(&self.db, thread_id, turn).await?;
                    generate_state.finish(crate::api::generate::StopReason {
                        reason: crate::api::generate::Reason::Finished,
                        detail: serde_json::to_value(usage)?,
                    });
                    tracing::debug!("subthread run_loop finished");
                    Ok(())
                }
                // The agent already emitted the error as a stream event; discard the
                // partial turn (today nothing consumes unfinished turns) and
                // propagate the error so the subthread registry marks the run as
                // Error and wakes any waiters.
                Err(_partial) => Err(anyhow::anyhow!("subthread run_loop failed")),
            }
        }
        .await;

        if let Err(error) = &result {
            let reason = if generate_state.cancellation_token.is_cancelled() {
                crate::api::generate::Reason::Interrupted
            } else {
                crate::api::generate::Reason::Error
            };
            generate_state.finish(crate::api::generate::StopReason {
                reason,
                detail: error.to_string().into(),
            });
        }
        self.release_generation(thread_id, &generate_state);
        result
    }

    async fn delete_subthread(&self, subthread_id: u64) -> Result<(), anyhow::Error> {
        crate::api::thread::delete::delete_threads_cascade(
            &self.db,
            &self.active_threads,
            &self.generate_states,
            &self.thread_lifecycle,
            subthread_id,
        )
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))
    }
}
