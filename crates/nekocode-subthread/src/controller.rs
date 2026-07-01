use std::sync::Arc;

use nekocode_core::agent::{Agent, AgentEvent};
use tokio::sync::mpsc::UnboundedSender;

/// Outcome of activating a subthread for background execution.
pub enum ActivationOutcome {
    /// The subthread was freshly activated and is ready to run. The caller
    /// spawns the background task using this agent.
    Activated(Arc<Agent>),
    /// The subthread was already activated (e.g. a prior run left it in
    /// `active_threads`, or a concurrent activation raced). The agent handle
    /// is returned so the caller can still `run()` it — `start_subthread`
    /// only refuses when the subthread is *already running*, not merely
    /// activated.
    AlreadyActivated(Arc<Agent>),
}

/// Abstraction over the API layer's thread control machinery (activate, run,
/// deactivate, delete).
///
/// The `nekocode-subthread` crate cannot depend on the `nekocode` API crate
/// (that would be a cycle), so the API crate implements this trait and
/// injects it into `SubthreadMiddleware`. This keeps the dependency direction
/// sound: the subthread crate defines what it needs, the API crate provides
/// it.
#[async_trait::async_trait]
pub trait ThreadController: Send + Sync {
    /// Activate `subthread_id` (build its `Agent` from its DB middlewares and
    /// insert into `active_threads`), returning the agent if newly activated.
    /// Mirrors the `activate_thread` API endpoint but programmatic.
    async fn activate(&self, subthread_id: u64) -> Result<ActivationOutcome, anyhow::Error>;

    /// Remove `subthread_id` from `active_threads` (and `generate_states`).
    /// Called when the background run completes or errors.
    async fn deactivate(&self, subthread_id: u64);

    /// Run `agent.run_loop(prompt, sender)` to completion. The API layer owns
    /// the `Agent` type, so it owns the call site; the subthread crate just
    /// needs to await the result and react to Ok/Err. The `sender` is provided
    /// by the caller (subthread crate) so it can discard events.
    async fn run(
        &self,
        agent: Arc<Agent>,
        prompt: String,
        sender: UnboundedSender<AgentEvent>,
    ) -> Result<(), anyhow::Error>;

    /// Delete `subthread_id` and all of its descendants recursively: abort any
    /// in-flight background tasks they own (via their per-parent
    /// `SubthreadRegistry` in `Agent.extensions`), evict them from
    /// `active_threads`/`generate_states`, then delete their messages → turns
    /// → middlewares → thread rows in one transaction. Mirrors the
    /// `delete_thread` API endpoint's cascade but scoped to a subthread.
    ///
    /// Refuses if the subthread (or any descendant) is mid-generation.
    async fn delete_subthread(&self, subthread_id: u64) -> Result<(), anyhow::Error>;
}
