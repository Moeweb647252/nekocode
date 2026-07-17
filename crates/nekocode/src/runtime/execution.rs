use std::sync::Arc;

use nekocode_core::agent::Agent;
use nekocode_types::generate::MessageContent;
use tokio_util::sync::CancellationToken;

use super::generation::{GenerationLease, GenerationSubscription, GenerationTerminal};
use super::{RuntimeError, ThreadRuntime};

impl ThreadRuntime {
    pub(crate) async fn start_generation(
        self: &Arc<Self>,
        thread_id: u64,
        user_input: String,
    ) -> Result<GenerationSubscription, RuntimeError> {
        let (lease, agent) = {
            let _lifecycle = self.lifecycle.lock().await;
            let agent = self
                .agents
                .get(thread_id)
                .ok_or(RuntimeError::ThreadNotActivated)?;
            let lease = self
                .generations
                .reserve(thread_id, CancellationToken::new())?;
            (lease, agent)
        };
        let subscription = lease.subscribe();
        let runtime = self.clone();
        tokio::spawn(async move {
            let _ = runtime.execute_generation(lease, agent, user_input).await;
        });
        Ok(subscription)
    }

    pub(crate) fn subscribe_generation(
        &self,
        thread_id: u64,
    ) -> Result<GenerationSubscription, RuntimeError> {
        self.generations.subscribe(thread_id)
    }

    pub(crate) fn cancel_generation(&self, thread_id: u64) -> Result<(), RuntimeError> {
        self.generations.cancel(thread_id)
    }

    pub(crate) async fn run_subthread(
        self: &Arc<Self>,
        thread_id: u64,
        prompt: String,
        cancellation: CancellationToken,
    ) -> Result<(), RuntimeError> {
        let (lease, agent) = {
            let _lifecycle = self.lifecycle.lock().await;
            if self.generations.contains(thread_id) {
                return Err(RuntimeError::ThreadGenerating);
            }
            let agent = match self.agents.get(thread_id) {
                Some(agent) => agent,
                None => {
                    let agent = self.build_agent(thread_id).await?;
                    self.agents.activate_or_get(thread_id, agent)
                }
            };
            let lease = self.generations.reserve(thread_id, cancellation)?;
            (lease, agent)
        };
        let terminal = self.execute_generation(lease, agent, prompt).await;
        // Child Agents are short-lived by design. This also breaks the
        // middleware/controller/runtime ownership chain after each run.
        self.agents.remove_and_shutdown(thread_id).await;
        match terminal {
            GenerationTerminal::Finished(_) => Ok(()),
            GenerationTerminal::Interrupted => Err(RuntimeError::Other(
                "subthread generation interrupted".to_string(),
            )),
            GenerationTerminal::Error(error) => Err(RuntimeError::Other(error)),
        }
    }

    async fn execute_generation(
        &self,
        lease: GenerationLease,
        agent: Arc<Agent>,
        user_input: String,
    ) -> GenerationTerminal {
        let thread_id = lease.thread_id();
        let old_turns = match super::turn_store::load_turn_context(&self.db, thread_id).await {
            Ok(turns) => turns,
            Err(error) => {
                let terminal =
                    GenerationTerminal::Error(format!("error loading turn context: {error}"));
                lease.finish(terminal.clone());
                lease.release();
                return terminal;
            }
        };

        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let cancellation = lease.cancellation();
        let run_cancellation = cancellation.clone();
        let handle = tokio::spawn(async move {
            agent
                .run_loop_with_cancellation(
                    vec![MessageContent::Text {
                        content: user_input,
                    }],
                    old_turns,
                    nekocode_core::agent::AgentEventSink::new(sender),
                    run_cancellation,
                )
                .await
        });

        while let Some(event) = receiver.recv().await {
            lease.publish(event);
        }

        let terminal = match handle.await {
            Err(error) => GenerationTerminal::Error(format!("error joining agent task: {error}")),
            Ok(Ok(mut turn)) => {
                // A successful Agent result is authoritative even when a
                // cancellation request races just after completion.
                turn.finished = true;
                let usage = turn.usage.clone();
                match super::turn_store::persist_turn(&self.db, thread_id, turn).await {
                    Ok(()) => GenerationTerminal::Finished(usage),
                    Err(error) => GenerationTerminal::Error(format!(
                        "error persisting turn {thread_id}: {error}"
                    )),
                }
            }
            Ok(Err(mut partial)) => {
                partial.finished = false;
                match super::turn_store::persist_turn(&self.db, thread_id, partial).await {
                    Err(error) => GenerationTerminal::Error(format!(
                        "error persisting partial turn {thread_id}: {error}"
                    )),
                    Ok(()) if cancellation.is_cancelled() => GenerationTerminal::Interrupted,
                    Ok(()) => GenerationTerminal::Error("agent run failed".to_string()),
                }
            }
        };
        lease.finish(terminal.clone());
        lease.release();
        terminal
    }
}
